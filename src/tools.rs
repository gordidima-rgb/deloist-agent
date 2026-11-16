//! Инструменты бота. Каждый — обычная функция, которую вызывает модель.
//! Риск: high — всегда спрашиваем человека.

use std::path::Path;
use std::process::Stdio;
use std::sync::Arc;
use std::time::Duration;

use regex::Regex;
use serde_json::{json, Value};
use tokio_util::sync::CancellationToken;

use crate::fetch::{fetch_page, is_public_ip};
use crate::process::{keep_safe_env, shell_command};
use crate::store::{uid, Bot};
use crate::App;

const MAX_OUTPUT: usize = 8000;
const MAX_FILE: u64 = 200_000;
/// Бриф делегата — короткая постановка, а не вся переписка: длинный текст жжёт чужое окно.
const MAX_BRIEF_CHARS: usize = 6000;
const BRIEF_TOO_LONG: &str = "Бриф длиннее 6000 знаков: сократи до сути — цель, входные данные, что вернуть";

pub fn clamp(s: &str, limit: usize) -> String {
    let count = s.chars().count();
    if count > limit {
        let head: String = s.chars().take(limit).collect();
        format!("{head}\n… обрезано ({count} символов)")
    } else {
        s.to_string()
    }
}

// ---------------------------------------------------------------------------
// Схемы для модели
// ---------------------------------------------------------------------------

/// Командные инструменты, которые получает и агент ACP через MCP-сервер своего бота.
pub const TEAM_TOOLS: [&str; 11] = ["delegate", "offer_specialist", "list_bots", "remember", "recall", "telegram_posts", "ask_user", "propose_team", "search_catalog", "propose_bot_change", "propose_routine"];

/// Всё, что MCP-процесс агента ACP вправе просить у приложения: браузер бота, команда и рутины.
/// Рутины идут теми же правилами, что у API-ботов: создание и правка спрашивают человека по режиму бота, удаление всегда.
pub fn is_bridge_tool(name: &str) -> bool {
    TEAM_TOOLS.contains(&name) || crate::team_tasks::is_tool(name) || crate::routine_tools::is_routine_tool(name) || crate::browser_tools::tool_defs().iter().any(|def| def["function"]["name"] == name)
}

pub fn team_tool_defs() -> Vec<Value> {
    let mut defs: Vec<Value> = base_tool_defs().as_array().cloned().unwrap_or_default().into_iter()
        .filter(|def| def["function"]["name"].as_str().is_some_and(|n| TEAM_TOOLS.contains(&n)))
        .collect();
    defs.extend(crate::team_tasks::defs());
    defs.extend(crate::routine_tools::defs());
    defs
}

pub fn tool_defs() -> Value {
    let mut all = base_tool_defs();
    if let Some(list) = all.as_array_mut() {
        list.extend(crate::browser_tools::tool_defs());
        list.extend(crate::team_tasks::defs());
        list.extend(crate::routine_tools::defs());
        list.extend(crate::workflow_tools::defs());
        list.extend(crate::goal_tools::defs());
        list.extend(crate::scripts::defs());
        list.extend(crate::habits::bot_tool_defs());
        list.extend(crate::todos::bot_tool_defs());
        // В облаке у ботов нет консоли: инструмент не предлагается модели.
        if crate::cloud::active() {
            list.retain(|def| def["function"]["name"] != "run_command");
        }
    }
    add_purpose(&mut all);
    all
}

/// Инструменты, которые могут вызвать карточку: у них есть необязательное `purpose` — одна фраза бота человеку.
/// Её показывают отдельной строкой «Бот объясняет»; на решение она не влияет (см. `describe`).
const PURPOSE_TOOLS: &[&str] = &["run_command", "write_file", "web_fetch", "create_bot", "update_bot", "generate_image", "create_routine", "update_routine", "delete_routine"];

fn add_purpose(all: &mut Value) {
    let Some(list) = all.as_array_mut() else { return };
    for def in list {
        let name = def["function"]["name"].as_str().unwrap_or_default().to_string();
        if !PURPOSE_TOOLS.contains(&name.as_str()) {
            continue;
        }
        if let Some(props) = def["function"]["parameters"]["properties"].as_object_mut() {
            props.entry("purpose").or_insert_with(|| json!({
                "type": "string",
                "description": "Необязательно: одна фраза простым текстом для человека (до 200 знаков) — зачем это действие. Показывается в карточке разрешения как твои слова."
            }));
        }
    }
}

fn base_tool_defs() -> Value {
    let function = |name: &str, description: &str, properties: Value, required: &[&str]| {
        let mut parameters = json!({ "type": "object", "properties": properties });
        if !required.is_empty() {
            parameters["required"] = json!(required);
        }
        json!({
            "type": "function",
            "function": { "name": name, "description": description, "parameters": parameters },
        })
    };
    json!([
        function(
            "list_files",
            "Показать файлы и папки. Без подтверждения: своя папка и любая папка на компьютере, кроме секретов.",
            json!({ "path": { "type": "string", "description": "Относительный путь — в своей папке (по умолчанию её корень), абсолютный (C:\\…) — где угодно." } }),
            &[],
        ),
        function(
            "read_file",
            "Прочитать текстовый файл. Без подтверждения: своя папка и любой файл на компьютере, кроме секретов.",
            json!({ "path": { "type": "string", "description": "Относительный путь — в своей папке, абсолютный (C:\\…) — где угодно." } }),
            &["path"],
        ),
        function(
            "write_file",
            "Создать или перезаписать текстовый файл. Родительские папки создаются сами. В своей папке — без подтверждения, вне её — человек может спросить разрешение.",
            json!({
                "path": { "type": "string", "description": "Относительный путь — в своей папке, абсолютный (C:\\…) — вне её." },
                "content": { "type": "string", "description": "Полное содержимое файла." },
            }),
            &["path", "content"],
        ),
        function(
            "run_command",
            "Выполнить команду cmd.exe; стартует в своей папке. Работа в своей папке и чтение где угодно (dir, type, findstr, git status/log/diff) идут без подтверждения. Изменения вне своей папки человек может попросить подтвердить: тогда сначала объясни одной строкой, что и зачем. Для чтения файлов удобнее read_file и list_files.",
            json!({
                "command": { "type": "string", "description": "Команда целиком." },
                "timeout_seconds": { "type": "integer", "description": "Максимальное время: по умолчанию 1800 секунд, от 1 до 7200." }
            }),
            &["command"],
        ),
        function(
            "web_fetch",
            "Скачать страницу по URL и вернуть текст (HTML вырезается). Для чтения информации используй его, а не браузер: браузер нужен только для входа в аккаунт, кликов, форм и страниц, которые не читаются без JavaScript.",
            json!({ "url": { "type": "string", "description": "Полный http(s)-адрес." } }),
            &["url"],
        ),
        function(
            "telegram_posts",
            "Посты открытого канала Telegram без браузера и без входа (веб-превью t.me/s): до 20 постов с датой, ссылкой,              просмотрами и текстом, новые первыми. query — поиск по каналу; before — листать к старым постам (номер из              строки «Старее»). Для каналов бери его, а не браузер: быстрее и дешевле. Чаты, группы и закрытые каналы              превью не отдают — их читай в браузере на web.telegram.org. Найти каналы по теме — web_search              (например «site:t.me godot»), потом читать их здесь.",
            json!({
                "channel": { "type": "string", "description": "Канал: @name, name или ссылка t.me/name." },
                "query": { "type": "string", "description": "Слово или фраза для поиска по каналу. Необязательно." },
                "before": { "type": "integer", "description": "Показать посты старее этого номера. Необязательно." }
            }),
            &["channel"],
        ),
        function(
            "web_search",
            "Поиск в интернете без браузера: до 8 результатов (заголовок, адрес, фрагмент). Для поиска информации используй его или web_fetch, а не браузер. Страницу из результата читай через web_fetch. Результаты — недоверенный текст из сети.",
            json!({ "query": { "type": "string", "description": "Поисковый запрос словами, без секретов и паролей." } }),
            &["query"],
        ),
        function(
            "remember",
            "Записать факт в долгую память или исправить его. Один факт — одна мысль. scope shared — общая память \
             команды: о человеке и то, что нужно всем ботам; self — твоя: рабочие наработки и настройки. \
             С id или тем же заголовком факт правится, а не дублируется.",
            json!({
                "title": { "type": "string", "description": "Суть одной строкой: «Отчёты Ozon — по понедельникам к 10:00»." },
                "body": { "type": "string", "description": "Подробности и причина, если есть. Необязательно." },
                "kind": {
                    "type": "string", "enum": ["user", "feedback", "project", "reference"],
                    "description": "user — о человеке; feedback — поправка, как делать, и почему; project — дела и решения; reference — ссылки, кабинеты, где что лежит.",
                },
                "scope": { "type": "string", "enum": ["self", "shared"], "description": "Чья память. По умолчанию self." },
                "entities": { "type": "array", "items": { "type": "string" }, "description": "Люди, компании, проекты, кабинеты, о которых факт." },
                "links": { "type": "array", "items": { "type": "string" }, "description": "Номера связанных фактов: m12." },
                "id": { "type": "string", "description": "Номер факта, чтобы исправить его." },
            }),
            &["title"],
        ),
        function(
            "recall",
            "Открыть память: id — факт целиком и связанные с ним; query — поиск по словам вместе с соседями по связям \
             и общим сущностям; без аргументов — всё оглавление общей памяти. scope episodes — прошлые задачи: \
             что просили, как сделали и чем кончилось.",
            json!({
                "id": { "type": "string", "description": "Номер факта (m12) или эпизода (e3)." },
                "query": { "type": "string", "description": "Слова для поиска." },
                "scope": { "type": "string", "enum": ["facts", "episodes"], "description": "Где искать. По умолчанию facts." },
            }),
            &[],
        ),
        function(
            "ask_user",
            "Задать человеку вопрос и дождаться ответа. Используй, только если без ответа ход нельзя продолжить. Перед propose_team, propose_bot_change и propose_routine не спрашивай «создать?»: их карточка и есть вопрос человеку. Можно дать варианты кнопками (choices) или простую форму (form): ответ формы придёт JSON. Если человек не ответит сразу, ответ придёт позже отдельным сообщением.",
            json!({
                "question": { "type": "string", "description": "Вопрос." },
                "choices": { "type": "array", "items": { "type": "string" }, "description": "Варианты ответа кнопками, до 8." },
                "form": {
                    "type": "array",
                    "description": "Поля формы, до 10. Ответ — JSON {name: значение}.",
                    "items": { "type": "object", "properties": {
                        "name": { "type": "string", "description": "Ключ в ответе: латиница, цифры, _." },
                        "label": { "type": "string", "description": "Подпись поля." },
                        "type": { "type": "string", "enum": ["text", "number", "date", "choice"] },
                        "options": { "type": "array", "items": { "type": "string" }, "description": "Варианты для type=choice." },
                    }, "required": ["name", "label", "type"] },
                },
            }),
            &["question"],
        ),
        function(
            "list_bots",
            "Показать команду и настройки ботов, чтобы выбрать бота для изменения или делегирования.",
            json!({}), &[],
        ),
        function(
            "create_bot",
            "Создать нового бота в команде. Передай его имя, роль и полную инструкцию. Вернёт ID; служебные файлы менять не нужно.",
            json!({
                "name": { "type": "string", "description": "Имя нового бота" },
                "role": { "type": "string", "description": "Короткое описание роли" },
                "persona": { "type": "string", "description": "Полная инструкция и характер бота" },
                "driver": { "type": "string", "description": "Драйвер модели, пустая строка — общий" },
                "model": { "type": "string", "description": "Модель, пустая строка — общая" },
                "color": { "type": "string", "description": "Цвет аватара в формате #RRGGBB" }
            }), &["name"],
        ),
        function(
            "update_bot",
            "Изменить настройки существующего бота по ID из list_bots. Передавай только поля, которые нужно изменить; persona заменяет инструкцию целиком.",
            json!({
                "bot_id": { "type": "string", "description": "ID бота из list_bots" },
                "name": { "type": "string", "description": "Имя" },
                "role": { "type": "string", "description": "Роль" },
                "persona": { "type": "string", "description": "Полная инструкция" },
                "driver": { "type": "string", "description": "Драйвер модели" },
                "model": { "type": "string", "description": "Модель" },
                "color": { "type": "string", "description": "Цвет аватара #RRGGBB" }
            }), &["bot_id"],
        ),
        function(
            "delegate",
            "Передать задачу боту своей команды (ведущий — ещё и ведущему другой команды) и получить его ответ. Работает в одну сторону, без цепочек. Бот-получатель не видит ни этой переписки, ни прошлых ходов: задача должна быть самодостаточным брифом.",
            json!({
                "bot": { "type": "string", "description": "Имя бота из команды." },
                "task": { "type": "string", "description": "Самодостаточный бриф: цель, входные данные и пути, ограничения, что вернуть. До ~1500 токенов, без истории переписки и «см. выше»." },
            }),
            &["bot", "task"],
        ),
        function(
            "offer_specialist",
            "Показать человеку карточку «Подключу специалиста», когда задача вне твоего профиля. Вызывай для специалиста из списка команды, а не delegate: работу начнёт специалист только после согласия человека.",
            json!({
                "bot": { "type": "string", "description": "Имя специалиста из команды." },
                "reason": { "type": "string", "description": "Почему эта задача по его профилю: одна короткая фраза." },
                "task": { "type": "string", "description": "Что именно передать специалисту. Полная постановка, без «см. выше»." },
            }),
            &["bot", "reason", "task"],
        ),
        function(
            "propose_team",
            "Предложить человеку собрать команду: показывает карточку «Создать / Не надо» и ждёт решения, сам ничего не создаёт. Карточка и есть вопрос: перед ней не переспрашивай через ask_user. Так собирай ботов и команды: API программы скриптами не вызывай. Новые боты получат режим вопросов «ask», без сайтов и без предела токенов; модель — как у тебя. Права и папки в аргументах не читаются.",
            json!({
                "newBots": { "type": "array", "description": "Новые боты, вместе с catalogBots не больше 5.", "items": { "type": "object", "properties": {
                    "name": { "type": "string", "description": "Имя до 40 знаков, не совпадает с существующим ботом." },
                    "role": { "type": "string", "description": "Роль одной строкой до 80 знаков." },
                    "instructions": { "type": "string", "description": "Инструкции бота, до 4000 знаков." },
                }, "required": ["name", "role", "instructions"] } },
                "catalogBots": { "type": "array", "items": { "type": "string" }, "description": "Готовые боты каталога: id из search_catalog (kind agent). Человек увидит, что им нужно; установятся с теми же безопасными умолчаниями." },
                "addBots": { "type": "array", "items": { "type": "string" }, "description": "Имена уже существующих ботов, которых добавить в команду." },
                "room": { "type": "string", "description": "Имя существующей команды; для новой команды — «new:Имя»." },
                "lead": { "type": "string", "description": "Имя ведущего команды (из участников)." },
            }), &[],
        ),
        function(
            "propose_bot_change",
            "Предложить человеку поправить участника своей команды: роль, инструкции или убрать из команды. Показывает карточку «было → стало» с кнопками «Применить / Не надо», ждёт решения и сам ничего не меняет. Права, режим подтверждений, сайты, драйвер, модель, команды запуска, папки и лимиты этим инструментом не меняются и в аргументах не читаются. Себя править нельзя.",
            json!({
                "bot": { "type": "string", "description": "Имя участника твоей команды." },
                "role": { "type": "string", "description": "Новая роль одной строкой до 80 знаков." },
                "instructions": { "type": "string", "description": "Новые инструкции целиком (заменяют прежние), до 4000 знаков." },
                "removeFromRoom": { "type": "string", "description": "Имя команды, где вы вместе: убрать участника из неё." },
            }), &["bot"],
        ),
        function(
            "propose_routine",
            "Предложить человеку расписание для участника команды (или для себя): рутина создаётся включённой только после «Создать» в карточке, где видны, кто, когда и текст задачи. Расписание — объект: {at:\"09:00\", days:[1,2,3,4,5]} или {every_minutes:60, window_from, window_to}. Неверное расписание вернётся ошибкой до карточки.",
            json!({
                "bot": { "type": "string", "description": "Имя участника твоей команды; без него — ты сам." },
                "title": { "type": "string", "description": "Короткое название, до 80 знаков." },
                "prompt": { "type": "string", "description": "Полная постановка на каждый запуск: рутина проснётся без этого разговора, «см. выше» не сработает." },
                "schedule": { "type": "object", "description": "Когда: at (ЧЧ:ММ) с days (1 — пн … 7 — вс) либо every_minutes (не реже 5) с window_from/window_to.", "properties": {
                    "at": { "type": "string" },
                    "days": { "type": "array", "items": { "type": "integer" } },
                    "every_minutes": { "type": "integer" },
                    "window_from": { "type": "string" },
                    "window_to": { "type": "string" },
                } },
            }), &["title", "prompt", "schedule"],
        ),
        function(
            "search_catalog",
            "Найти в каталоге готовых ботов и навыков по словам задачи (до 10): id, имя, роль, что нужно, установлен ли. Только чтение. Нужного бота предложи человеку через propose_team (catalogBots).",
            json!({
                "query": { "type": "string", "description": "Слова задачи, например «таблицы отчёты»." },
                "kind": { "type": "string", "enum": ["agent", "skill"], "description": "Только агенты или только навыки; по умолчанию оба." },
            }),
            &["query"],
        ),
    ])
}

/// Инструменты, на которые нельзя отвечать «разрешить на этот разговор» или «всегда»: каждый вызов — отдельное решение.
/// Ход самопроверки и ход без человека спрашивают инструменты рутин и целей каждый раз (`grants::scope_for`).
pub fn never_remember(tool: &str) -> bool {
    tool == "delete_routine" || crate::workflow_tools::is_mutating(tool)
}

pub fn is_high_risk(name: &str) -> bool {
    matches!(name, "run_command" | "create_bot" | "update_bot" | "create_routine" | "update_routine" | "delete_routine" | "edit_workflow" | "run_workflow" | "delete_workflow")
}

/// Инструменты, которые только читают: их не спрашиваем даже в строгом режиме.
pub fn is_read_only(name: &str) -> bool {
    // `ask_user` и так обращается к человеку. `remember` меняет память бота, это уже запись.
    // `save_script` и `run_script` сами ничего не делают: сохранять можно только под согласие человека, а каждый шаг
    // скрипта идёт через обычную проверку и карточки (`turn::execute_call_in`).
    matches!(name, "list_files" | "read_file" | "list_bots" | "browser_page" | "browser_show_to_human" | "browser_logins" | "ask_user" | "recall" | "read_skill" | "workspace_graph" | "list_routines" | "list_workflows" | "read_workflow" | "read_workflow_runs" | "list_habits" | "list_todos" | "offer_specialist" | "propose_team" | "propose_bot_change" | "propose_routine" | "search_catalog" | "save_script" | "run_script")
}

/// Какой из двух режимов осторожнее. Передача задачи не должна снижать осторожность:
/// иначе бот, который спрашивает, обошёл бы вопрос через бота, который не спрашивает.
pub fn stricter(a: &str, b: &str) -> String {
    let rank = |mode: &str| match mode {
        "strict" => 2,
        "auto" => 0,
        _ => 1,
    };
    let winner = if rank(a) >= rank(b) { a } else { b };
    match winner {
        "strict" | "auto" => winner.to_string(),
        _ => "ask".to_string(),
    }
}

pub(crate) fn policy_needs_approval(policy: &str, name: &str, mcp: bool, server_auto: bool) -> bool {
    if mcp {
        // Чужие инструменты могут делать что угодно: снять вопрос может только настройка самого сервера,
        // а строгий режим спрашивает всегда.
        return policy == "strict" || !server_auto;
    }
    match policy {
        // Человек сказал «решай сам»: карточек нет.
        "auto" => false,
        // Строгий: спрашиваем всё, что хоть что-то меняет.
        "strict" => !is_read_only(name),
        _ => is_high_risk(name),
    }
}

/// Все схемы: свои, картинки (если сервис настроен) и инструменты подключённых MCP-серверов.
/// Порядок — по имени, а не по порядку подключения: порядок зависит от загрузки,
/// а любой сдвиг схем меняет префикс запроса и обнуляет кэш провайдера.
pub async fn tool_defs_with_mcp(app: &Arc<App>) -> Value {
    let mut all = tool_defs();
    let mut extra: Vec<Value> = vec![crate::skills::read_skill_def(), crate::graph::tool_def()];
    if crate::skills::learning_on(&app.store.settings()) {
        extra.push(crate::skills::save_skill_def());
    }
    if crate::image::ready(app) {
        extra.push(crate::image::tool_def());
    }
    extra.extend(crate::mcp::tool_defs(app).await);
    if let Some(list) = all.as_array_mut() {
        list.extend(extra);
        list.sort_by(|a, b| {
            a["function"]["name"].as_str().unwrap_or_default().cmp(b["function"]["name"].as_str().unwrap_or_default())
        });
    }
    all
}

/// Нужно ли разрешение человека на этот вызов с учётом политики бота.
pub fn needs_approval(app: &App, bot: &Bot, name: &str) -> bool {
    // Передача задачи другому боту карточки не требует: получатель работает по своим правилам
    // (его режим не мягче режима отправителя, см. `stricter`), а его собственные опасные шаги спрашиваются у него.
    // Копия файла в чужую папку — запись вне своей: без «Свободы действий» спрашивается, как другие такие записи.
    if name == "share_file" {
        return bot.approval != "auto";
    }
    if name == "delegate" || crate::team_tasks::is_tool(name) {
        return false;
    }
    // Привычки: бот читает их и отмечает своим инструментом без карточки (см. `habits::exec_bot_tool`).
    // Задачи человека — то же самое: бот по просьбе человека добавляет и отмечает их (см. `todos::exec_bot_tool`).
    if crate::habits::is_bot_tool(name) || crate::todos::is_bot_tool(name) {
        return false;
    }
    // Картинка тратит деньги у платного сервиса: спрашиваем, если человек не сказал «решай сам».
    if name == "generate_image" {
        return bot.approval != "auto";
    }
    if name == "save_skill" {
        return crate::skills::save_needs_approval(&app.store.settings(), bot);
    }
    if crate::mcp::split_name(name).is_some() {
        let servers = app.store.mcp_servers();
        let auto = crate::mcp::server_of(&servers, name).map(|s| s.auto_approve).unwrap_or(false);
        return policy_needs_approval(&bot.approval, name, true, auto);
    }
    policy_needs_approval(&bot.approval, name, false, false)
}

/// То же с учётом аргументов. В режиме «спрашивать» своя папка и чтение где угодно идут
/// без вопроса, а изменения вне своей папки спрашиваются (см. `reach`).
pub fn needs_approval_for(app: &App, bot: &Bot, name: &str, args: &Value) -> bool {
    if name == "write_file" && special_write_guard(app, &bot.id, arg(args, "path")) {
        return true;
    }
    // Передача файла участнику (`share_file` и `files` у `assign_task`) пишет в чужую папку: карточка, если бот не в режиме «решай сам».
    if name == "share_file" || (name == "assign_task" && args.get("files").and_then(Value::as_array).is_some_and(|f| !f.is_empty())) {
        return bot.approval != "auto";
    }
    if name == "delegate" || crate::team_tasks::is_tool(name) {
        return false;
    }
    if crate::habits::is_bot_tool(name) || crate::todos::is_bot_tool(name) {
        return false;
    }
    // Вход сохранённым паролем спрашивает сам инструмент (карточка с сайтом и именем входа).
    // Общая карточка поверх неё спросила бы второй раз.
    if name == "browser_login" {
        return false;
    }
    if name == "run_command" {
        let command = arg(args, "command");
        // Отказ при выполнении будет всё равно: карточка на него только сбила бы человека.
        if command_refused(app, &bot.id, command).is_some() {
            return false;
        }
        // Выпуск наружу (push, выпуск, зеркало) спрашивается всегда, даже в «Свободе действий».
        if crate::reach::release_command(command).is_some() {
            return true;
        }
    }
    // Файл с ключами и ключ SSH — карточка при любом режиме, даже «решай сам».
    if crate::keys::call_touch(name, args).is_some() {
        // Путь, который чтение всё равно отобьёт (секреты программы), карточкой не спрашивается.
        // Сверяем с той же проверкой, что и чтение файла с ключами: `.env` по абсолютному пути
        // читать можно, но только после карточки.
        if name == "read_file" && key_file_target(&app.store, &bot.id, arg(args, "path")).is_err() {
            return false;
        }
        // То же для команды, которая лезет в служебные файлы программы: её отбивает выполнение.
        if name == "run_command"
            && crate::reach::command_secret(arg(args, "command"), &app.store.workspace(&bot.id), app.store.data_dir())
                .is_some_and(|(_, reason)| reason.starts_with("служебные файлы программы"))
        {
            return false;
        }
        return true;
    }
    if crate::routine_tools::is_routine_tool(name) {
        return crate::routine_tools::needs_approval(app, bot, name, args);
    }
    // Рабочие процессы: чтение свободно, правка, запуск и удаление спрашивают всегда, в любом режиме.
    if crate::workflow_tools::is_tool(name) {
        return crate::workflow_tools::is_mutating(name);
    }
    if crate::goal_tools::is_goal_tool(name) {
        return crate::goal_tools::needs_approval(app, bot, name, args);
    }
    let store = &app.store;
    // Секреты отбиваются при выполнении в любом режиме, карточка тут не нужна: разрешить их нельзя.
    if name == "run_command"
        && crate::reach::command_secret(arg(args, "command"), &store.workspace(&bot.id), store.data_dir()).is_some()
    {
        return false;
    }
    if bot.approval != "ask" {
        return needs_approval(app, bot, name);
    }
    match name {
        "run_command" => {
            // Скрипт PowerShell, который по строгому разбору только читает и пишет файлы в своих папках, идёт без карточки.
            // Всё, что разобрать не удалось, спрашивается, как раньше.
            if let Some(script) = crate::shellplan::script_from_cmdline(arg(args, "command")) {
                if crate::describe::own_space_script(app, &bot.id, &script) {
                    return false;
                }
            }
            let workspace = store.workspace(&bot.id);
            let projects = homes(app, &bot.id);
            let places = crate::reach::Places { workspace: &workspace, projects: &projects, data_dir: store.data_dir(), install: None };
            crate::reach::command_needs_approval_in(arg(args, "command"), &places)
        }
        // Секреты отбиваются при выполнении, без карточки: разрешить их нельзя.
        "write_file" => matches!(write_target(app, &bot.id, arg(args, "path")), Ok((_, true))),
        _ => needs_approval(app, bot, name),
    }
}

/// Protected files require a card even in auto; ordinary paths outside own homes do not.
pub(crate) fn special_write_guard(app: &App, bot_id: &str, raw: &str) -> bool {
    // Invalid or forbidden targets are rejected by the file operation itself.
    let Ok((path, _)) = write_target(app, bot_id, raw) else { return false };
    let real = crate::reach::real_path(&path).unwrap_or(path);
    let project = homes(app, bot_id).iter().any(|p| real.starts_with(std::fs::canonicalize(p).unwrap_or_else(|_| p.clone())));
    crate::reach::auto_exec_reason(&real).is_some() || !crate::reach::single_link_or_missing(&real)
        || (project && crate::reach::project_guard_reason(&real).is_some())
}

/// Варианты и поля формы для `ask_user`, проверенные: модель может прислать что угодно.
pub(crate) fn ask_shape(args: &Value) -> Result<(Vec<String>, Vec<crate::turn::FormField>), String> {
    let short = |v: &Value, limit: usize| v.as_str().map(|s| s.trim().chars().take(limit).collect::<String>()).filter(|s| !s.is_empty());
    let mut choices: Vec<String> = Vec::new();
    for item in args.get("choices").and_then(Value::as_array).into_iter().flatten() {
        if let Some(text) = short(item, 80) {
            if !choices.contains(&text) {
                choices.push(text);
            }
        }
    }
    if choices.len() > 8 {
        return Err("вариантов больше восьми".into());
    }
    let mut form = Vec::new();
    for item in args.get("form").and_then(Value::as_array).into_iter().flatten() {
        let name = item["name"].as_str().unwrap_or_default().trim().to_string();
        if name.is_empty() || name.len() > 40 || !name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
            return Err(format!("поле формы «{name}»: имя из латиницы, цифр и _"));
        }
        if form.iter().any(|f: &crate::turn::FormField| f.name == name) {
            return Err(format!("поле формы «{name}» повторяется"));
        }
        let kind = item["type"].as_str().unwrap_or("text").to_string();
        if !matches!(kind.as_str(), "text" | "number" | "date" | "choice") {
            return Err(format!("поле «{name}»: тип text, number, date или choice"));
        }
        let options: Vec<String> = item["options"].as_array().into_iter().flatten().filter_map(|o| short(o, 80)).take(12).collect();
        if kind == "choice" && options.is_empty() {
            return Err(format!("поле «{name}»: у выбора нужны варианты"));
        }
        let label = short(&item["label"], 120).unwrap_or_else(|| name.clone());
        form.push(crate::turn::FormField { name, label, kind, options });
    }
    if form.len() > 10 {
        return Err("полей формы больше десяти".into());
    }
    if !form.is_empty() && !choices.is_empty() {
        return Err("либо варианты, либо форма".into());
    }
    Ok((choices, form))
}

/// Команду нельзя выполнять ни в каком режиме: она меняет запущенную программу, а у бота-разработчика —
/// ещё и уводит сборку из checkout.
pub(crate) fn command_refused(app: &App, bot_id: &str, command: &str) -> Option<&'static str> {
    let store = &app.store;
    let workspace = store.workspace(bot_id);
    let dev_projects = crate::selfdev::projects(store, bot_id);
    let projects = homes(app, bot_id);
    let install = crate::selfdev::install_dir();
    let places = crate::reach::Places { workspace: &workspace, projects: &projects, data_dir: store.data_dir(), install: install.as_deref() };
    crate::reach::command_refusal(command, &places, !dev_projects.is_empty())
}

/// Облако: любой путь только внутри рабочей папки этого бота. Абсолютный путь принимается, если он
/// ведёт в неё; всё остальное (`/proc`, `/etc`, чужие папки, секреты) отклоняется. Дальше работает
/// `safe_path`: он убирает `..` и проверяет настоящий путь, поэтому ссылка наружу не проходит.
pub(crate) fn cloud_target(store: &crate::store::Store, bot_id: &str, raw: &str) -> Result<std::path::PathBuf, String> {
    let outside = || format!("{raw}: {}", crate::cloud::OUTSIDE_WORKSPACE);
    if !crate::reach::is_absolute_like(raw) {
        return store.safe_path(bot_id, raw).map_err(|_| outside());
    }
    let workspace = store.workspace(bot_id);
    let _ = std::fs::create_dir_all(&workspace);
    let mut roots = vec![workspace.clone()];
    if let Ok(real) = std::fs::canonicalize(&workspace) {
        roots.push(real);
    }
    for root in roots {
        if let Ok(rest) = Path::new(raw).strip_prefix(&root) {
            return store.safe_path(bot_id, &rest.to_string_lossy()).map_err(|_| outside());
        }
    }
    Err(outside())
}

/// Путь для чтения: относительный — в своей папке, абсолютный — где угодно, кроме секретов.
pub(crate) fn read_target(store: &crate::store::Store, bot_id: &str, raw: &str) -> Result<std::path::PathBuf, String> {
    let real = if crate::cloud::active() {
        cloud_target(store, bot_id, raw)?
    } else if !crate::reach::is_absolute_like(raw) {
        store.safe_path(bot_id, raw)?
    } else { std::fs::canonicalize(raw).map_err(|e| format!("{raw}: {e}"))? };
    if let Some(reason) = crate::reach::secret_reason(&real, store.data_dir(), &store.workspace(bot_id)) {
        return Err(format!("{raw}: {reason}, сюда боту нельзя"));
    }
    if !crate::reach::safe_read_links(&real) {
        return Err(format!("{raw}: нельзя проверить, не связан ли файл с закрытыми данными"));
    }
    Ok(real)
}

/// `~` в начале пути — домашняя папка пользователя.
fn expand_home(raw: &str) -> std::path::PathBuf {
    match raw.strip_prefix("~/").or_else(|| raw.strip_prefix("~\\")) {
        Some(rest) => dirs_home().join(rest),
        None => std::path::PathBuf::from(raw),
    }
}

fn dirs_home() -> std::path::PathBuf {
    std::env::var_os("USERPROFILE").or_else(|| std::env::var_os("HOME")).map(std::path::PathBuf::from).unwrap_or_default()
}

/// Файл с ключами (`.env`, `secrets.json` проекта): после карточки читать можно. Служебная
/// папка программы, `.ssh`, профили браузеров и хранилище Windows закрыты и с карточкой.
pub(crate) fn key_file_target(store: &crate::store::Store, bot_id: &str, raw: &str) -> Result<std::path::PathBuf, String> {
    if crate::cloud::active() {
        let path = cloud_target(store, bot_id, raw)?;
        return if crate::reach::safe_read_links(&path) { Ok(path) } else { Err(format!("{raw}: нельзя проверить ссылки файла")) };
    }
    let path = if crate::reach::is_absolute_like(raw) {
        std::fs::canonicalize(raw).map_err(|e| format!("{raw}: {e}"))?
    } else {
        store.safe_path(bot_id, raw)?
    };
    if !crate::reach::safe_read_links(&path) { return Err(format!("{raw}: нельзя проверить ссылки файла")); }
    match crate::reach::secret_reason(&path, store.data_dir(), &store.workspace(bot_id)) {
        None | Some(crate::reach::KEY_FILE) => Ok(path),
        Some(reason) => Err(format!("{raw}: {reason}, сюда боту нельзя")),
    }
}

/// Папки, где бот хозяин: checkout режима разработки и общие папки команды (только в её разговоре).
pub(crate) fn homes(app: &App, bot_id: &str) -> Vec<std::path::PathBuf> {
    let mut out = crate::rooms::projects_for_turn(app, bot_id);
    out.extend(crate::rooms::room_homes(app, bot_id));
    out
}

/// Файл автозапуска, внутренность `.git` или запускаемый файл (`.exe`, `.bat`, `.js`, `.lnk`…): запись туда нужна только
/// с карточкой и в своей папке (кроме режима «решай сам»).
/// Путь сначала приводится к настоящему виду: Windows читает `file::$DATA` и `file:поток` как тот же файл, короткие имена
/// 8.3 (`GIT~1`, `CLAUDE~1`) как длинные, хвостовые точки и пробелы отбрасывает. Любое имя с `~цифра` подозрительно само.
pub(crate) fn autorun_path(path: &Path) -> bool {
    let cleaned = clean_spelling(path);
    // Короткое имя, которое не удалось раскрыть (файла ещё нет): прячет что угодно, спрашиваем. Существующие имена
    // сначала раскрываются до длинных: короткое имя папки данных в пути не должно считаться подозрительным.
    let resolved = crate::reach::real_path(&cleaned).unwrap_or_else(|| cleaned.clone());
    if resolved.components().any(|c| {
        let name = c.as_os_str().to_string_lossy();
        name.find('~').is_some_and(|at| name[at + 1..].chars().next().is_some_and(|d| d.is_ascii_digit()))
    }) {
        return true;
    }
    let guarded = |p: &Path| {
        crate::reach::auto_exec_reason(p).is_some()
            || crate::reach::planted_executable(p)
            || p.components().any(|c| c.as_os_str().eq_ignore_ascii_case(".git"))
    };
    guarded(path)
        || guarded(&cleaned)
        || crate::reach::real_path(path).is_some_and(|real| guarded(&real))
        || crate::reach::real_path(&cleaned).is_some_and(|real| guarded(&real))
}

/// Путь, запись в который идёт с карточкой: файл автозапуска или подложенный исполняемый файл (`autorun_path`)
/// либо существующий файл с несколькими жёсткими ссылками (по имени его не опознать).
pub(crate) fn flagged_target(path: &Path) -> bool {
    autorun_path(path) || !crate::reach::single_link_or_missing(&crate::reach::real_path(path).unwrap_or_else(|| path.to_path_buf()))
}

/// Путь без потоков (`::$DATA`, `:поток`), хвостовых точек и пробелов в именах.
fn clean_spelling(path: &Path) -> std::path::PathBuf {
    let mut out = std::path::PathBuf::new();
    for component in path.components() {
        match component {
            std::path::Component::Normal(name) => {
                let text = name.to_string_lossy();
                let head = text.split(':').next().unwrap_or_default().trim_end_matches(['.', ' ']);
                out.push(if head.is_empty() { text.as_ref() } else { head });
            }
            other => out.push(other.as_os_str()),
        }
    }
    out
}

/// Двоеточие вне префикса диска (потоки файла) и имена, которые Windows читает по-особому, в пути для записи не принимаются.
fn plain_components(raw: &str) -> bool {
    Path::new(raw).components().all(|c| match c {
        std::path::Component::Normal(name) => crate::store::plain_file_name(&name.to_string_lossy()),
        _ => true,
    })
}

/// Путь для записи и признак «вне своей папки» (или файл автозапуска).
pub(crate) fn write_target(app: &App, bot_id: &str, raw: &str) -> Result<(std::path::PathBuf, bool), String> {
    let store = &app.store;
    if crate::cloud::active() {
        return cloud_target(store, bot_id, raw).map(|path| {
            let real = crate::reach::real_path(&path).unwrap_or_else(|| path.clone());
            let flagged = flagged_target(&real);
            (path, flagged)
        });
    }
    if !crate::reach::is_absolute_like(raw) {
        return store.safe_path(bot_id, raw).map(|path| { let flagged = flagged_target(&path); (path, flagged) });
    }
    if !plain_components(raw) {
        return Err(format!("{raw}: потоки файла, двоеточие и имена устройств в пути не принимаются"));
    }
    let real = crate::reach::real_path(Path::new(raw)).ok_or_else(|| format!("{raw}: такого диска или папки нет"))?;
    let workspace = store.workspace(bot_id);
    if let Some(reason) = crate::reach::secret_reason(&real, store.data_dir(), &workspace) {
        return Err(format!("{raw}: {reason}, сюда боту нельзя"));
    }
    let projects = homes(app, bot_id);
    let install = crate::selfdev::install_dir();
    let places = crate::reach::Places { workspace: &workspace, projects: &projects, data_dir: store.data_dir(), install: install.as_deref() };
    if let Some(reason) = crate::reach::install_reason(&real, &places) {
        return Err(format!("{raw}: {reason}"));
    }
    let home = std::fs::canonicalize(&workspace).unwrap_or(workspace);
    if real.starts_with(&home) {
        let flagged = flagged_target(&real);
        return Ok((real, flagged));
    }
    // Папки проектов (checkout, общие папки команд) хранятся без `\?\`, а `real` канонический: сравниваем канонические.
    let in_project = projects.iter().any(|p| real.starts_with(std::fs::canonicalize(p).unwrap_or_else(|_| p.clone())));
    if !in_project {
        return Ok((real, true));
    }
    // Внутри проекта бот хозяин, кроме папки программы и файлов, которые исполняются сами (хуки git, настройки Claude Code…).
    if let Some(reason) = crate::reach::program_roots().iter().any(|r| real.starts_with(r)).then_some(crate::reach::INSTALL_REFUSAL) {
        return Err(format!("{raw}: {reason}"));
    }
    Ok((real.clone(), crate::reach::project_guard_reason(&real).is_some() || !crate::reach::single_link_or_missing(&real)))
}

/// Список чужой папки: полные пути, чтобы модель могла идти дальше.
async fn list_outside(store: &crate::store::Store, bot_id: &str, raw: &str) -> Result<String, String> {
    if crate::cloud::active() {
        return Err(format!("{raw}: {}", crate::cloud::OUTSIDE_WORKSPACE));
    }
    const LIMIT: usize = 500;
    let real = read_target(store, bot_id, raw)?;
    let mut dir = tokio::fs::read_dir(&real).await.map_err(|e| format!("{raw}: {e}"))?;
    let mut items: Vec<(bool, String)> = Vec::new();
    while let Some(entry) = dir.next_entry().await.map_err(|e| e.to_string())? {
        let is_dir = entry.file_type().await.map(|t| t.is_dir()).unwrap_or(false);
        items.push((is_dir, crate::reach::display(&entry.path())));
    }
    let shown = crate::reach::display(&real);
    if items.is_empty() {
        return Ok(format!("Папка «{shown}» пуста."));
    }
    items.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.to_lowercase().cmp(&b.1.to_lowercase())));
    let total = items.len();
    let mut lines: Vec<String> = items
        .into_iter()
        .take(LIMIT)
        .map(|(is_dir, path)| format!("{}: {path}", if is_dir { "папка" } else { "файл" }))
        .collect();
    if total > LIMIT {
        lines.push(format!("… и ещё {} (показаны первые {LIMIT})", total - LIMIT));
    }
    Ok(format!("Содержимое «{shown}»:\n{}", lines.join("\n")))
}

#[cfg(test)]
mod approval_tests {
    use super::{policy_needs_approval as needs, stricter};

    #[test]
    fn mcp_asks_unless_the_server_is_trusted_and_strict_always_asks() {
        assert!(needs("ask", "mcp__a__b", true, false));
        assert!(!needs("ask", "mcp__a__b", true, true));
        assert!(needs("auto", "mcp__a__b", true, false), "README обещает вопрос и в режиме auto");
        assert!(!needs("auto", "mcp__a__b", true, true));
        assert!(needs("strict", "mcp__a__b", true, true), "строгий режим спрашивает всегда");
    }

    #[test]
    fn offering_a_specialist_does_not_ask_the_person() {
        // Карточку согласия создаёт только сам инструмент: он ничего не делает без человека.
        assert!(super::is_read_only("offer_specialist"));
        for mode in ["ask", "auto", "strict"] {
            assert!(!needs(mode, "offer_specialist", false, false), "{mode}");
        }
    }

    #[test]
    fn strict_asks_for_every_change_including_memory() {
        for tool in ["write_file", "run_command", "web_fetch", "web_search", "remember", "browser_nav"] {
            assert!(needs("strict", tool, false, false), "{tool}");
        }
        for mode in ["ask", "auto"] {
            assert!(!needs(mode, "web_search", false, false), "web_search в режиме {mode} без карточки");
        }
        for tool in ["list_files", "read_file", "browser_page", "browser_show_to_human", "browser_logins", "ask_user"] {
            assert!(!needs("strict", tool, false, false), "{tool}");
        }
    }

    #[test]
    fn managing_bots_respects_approval_policy_and_fields() {
        for name in ["create_bot", "update_bot"] {
            assert!(needs("ask", name, false, false));
            assert!(needs("strict", name, false, false));
            assert!(!needs("auto", name, false, false));
        }
        assert!(!needs("strict", "list_bots", false, false));
        assert!(super::bot_profile_args(&serde_json::json!({"name":"X", "approval":"auto"}), false).is_err());
        assert!(super::bot_profile_args(&serde_json::json!({"name":"", "role":"test"}), false).is_err());
        assert!(super::bot_profile_args(&serde_json::json!({"name":"X", "color":"#BAD"}), false).is_err());
        assert!(super::bot_profile_args(&serde_json::json!({"bot_id":"bot_1", "role":"test"}), true).is_ok());
    }

    #[test]
    fn handing_a_task_over_never_lowers_caution() {
        assert_eq!(stricter("auto", "ask"), "ask");
        assert_eq!(stricter("ask", "auto"), "ask");
        assert_eq!(stricter("auto", "auto"), "auto");
        assert_eq!(stricter("auto", "strict"), "strict");
        assert_eq!(stricter("strict", "ask"), "strict");
        assert_eq!(stricter("чепуха", "auto"), "ask", "неизвестное значение как «спрашивать»");
    }
}

pub fn label(name: &str) -> &str {
    match name {
        n if crate::team_tasks::is_tool(n) => crate::team_tasks::label(n),
        "run_command" => "команда в оболочке",
        "ask_user" => "вопрос человеку",
        "write_file" => "запись файла",
        "read_file" => "чтение файла",
        "list_files" => "список файлов",
        "web_fetch" => "запрос в сеть",
        "web_search" => "поиск в интернете",
        "telegram_posts" => "чтение канала Telegram",
        "remember" => "запись в память",
        "recall" => "поиск в памяти",
        "save_skill" => "сохранение навыка",
        "read_skill" => "чтение навыка",
        "workspace_graph" => "граф файлов",
        "delegate" => "делегирование",
        "offer_specialist" => "предложение специалиста",
        "propose_team" => "предложение команды",
        "propose_bot_change" => "предложение изменить бота",
        "propose_routine" => "предложение рутины",
        "search_catalog" => "поиск в каталоге",
        "list_bots" => "просмотр команды",
        "create_bot" => "создание бота",
        "update_bot" => "настройка бота",
        "list_routines" => "просмотр рутин",
        "list_workflows" => "просмотр процессов",
        "read_workflow" => "чтение процесса",
        "read_workflow_runs" => "запуски процесса",
        "edit_workflow" => "правка процесса",
        "run_workflow" => "запуск процесса",
        "delete_workflow" => "удаление процесса",
        "list_habits" => "просмотр привычек",
        "save_script" => "сохранение скрипта",
        "run_script" => "запуск скрипта",
        "check_habit" => "отметка привычки",
        "list_todos" => "просмотр задач",
        "add_todo" => "новая задача",
        "complete_todo" => "задача выполнена",
        "create_routine" => "создание рутины",
        "update_routine" => "изменение рутины",
        "delete_routine" => "удаление рутины",
        "create_goal" => "новая цель",
        "list_goals" => "просмотр целей",
        "update_goal" => "изменение цели",
        "run_goal" => "запуск цели",
        "pause_goal" => "пауза цели",
        "resume_goal" => "возобновление цели",
        "complete_goal" => "завершение цели",
        "report_goal_progress" => "запись прогресса цели",
        "generate_image" => "платная картинка",
        "browser_nav" => "браузер: адрес и вкладки",
        "browser_page" => "браузер: страница",
        "browser_mouse" => "браузер: мышь",
        "browser_keyboard" => "браузер: клавиатура",
        "browser_devtools" => "браузер: отладчик",
        "browser_human" => "браузер: передача человеку",
        "browser_show_to_human" => "браузер: показать экран человеку",
        "browser_logins" => "браузер: входы из менеджера паролей",
        "browser_login" => "браузер: вход сохранённым паролем",
        other => other,
    }
}

// ---------------------------------------------------------------------------
// Вспомогательное
// ---------------------------------------------------------------------------

fn strip_html(html: &str) -> String {
    let script = Regex::new(r"(?is)<script.*?</script>").unwrap();
    let style = Regex::new(r"(?is)<style.*?</style>").unwrap();
    let tag = Regex::new(r"(?s)<[^>]+>").unwrap();
    let spaces = Regex::new(r"[ \t]+").unwrap();
    let blank = Regex::new(r"\n{3,}").unwrap();
    let text = script.replace_all(html, " ");
    let text = style.replace_all(&text, " ");
    let text = tag.replace_all(&text, " ");
    let text = text
        .replace("&nbsp;", " ")
        .replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"");
    let text = spaces.replace_all(&text, " ");
    blank.replace_all(&text, "\n\n").trim().to_string()
}

struct ShellResult {
    code: i32,
    out: String,
    err: String,
    timed_out: bool,
}

/// Команда бота. Песочница подставляет изоляцию и текстовый фильтр, но при любом отказе
/// выполнение уходит на прежний путь: вывод и код остаются такими же, как раньше.
async fn run_shell(command: &str, cwd: &Path, tmp: &Path, timeout: Duration, cancel: &CancellationToken, extra_env: &[(String, String)]) -> ShellResult {
    // Второй замок политики облака: даже если проверку в `exec_tool` обойдут, процесс не стартует.
    if crate::cloud::active() {
        return ShellResult { code: 1, timed_out: false, out: String::new(), err: crate::cloud::NO_CONSOLE.to_string() };
    }
    // Вне Windows песочницы нет: команда не запускается, пока человек явно не выключил её (`CREW_SANDBOX=off`).
    #[cfg(not(windows))]
    if let Some(reason) = crate::sandbox::refusal() {
        return ShellResult { code: 1, timed_out: false, out: String::new(), err: format!("Команда не выполнена: {reason}.") };
    }
    // Первая самопроверка запускает канарейку и ждёт её: на рабочем потоке рантайма так нельзя.
    let status = tokio::task::spawn_blocking(crate::sandbox::status).await;
    let level = status.map(|status| status.level).unwrap_or(crate::sandbox::Level::Off);
    // `strict` — это обещание: если клетка не поднялась, выполнять команду без неё нельзя.
    if crate::sandbox::mode() == crate::sandbox::Mode::Strict && level != crate::sandbox::Level::AppContainer {
        return ShellResult {
            code: 1,
            timed_out: false,
            out: String::new(),
            err: format!("Команда не выполнена: песочница strict недоступна ({}).", crate::sandbox::status().detail),
        };
    }
    // Текстовый слой нужен только там, где файловой изоляции нет: в AppContainer запрет
    // держит сама система, а лишний отказ ломал бы нормальные команды.
    if level != crate::sandbox::Level::AppContainer {
        if let Some(reason) = crate::sandbox::looks_protected(command) {
            return ShellResult {
                code: 1,
                timed_out: false,
                out: String::new(),
                err: format!("Команда не выполнена: {reason}. Песочница: {}.", level.name()),
            };
        }
    }
    if level != crate::sandbox::Level::Off {
        match run_shell_sandboxed(command, cwd, tmp, timeout, cancel, extra_env).await {
            Ok(result) => return result,
            // `strict` — это обещание: если клетка для этой команды не поднялась, без неё не выполняем.
            Err(reason) if crate::sandbox::mode() == crate::sandbox::Mode::Strict => {
                return ShellResult {
                    code: 1,
                    timed_out: false,
                    out: String::new(),
                    err: format!("Команда не выполнена: песочница strict не смогла её запустить ({reason})."),
                };
            }
            Err(_) => {}
        }
    }
    run_shell_plain(command, cwd, tmp, timeout, cancel, extra_env).await
}

/// Запуск через песочницу. `Err` — она не справилась и команда ещё не запускалась.
async fn run_shell_sandboxed(command: &str, cwd: &Path, tmp: &Path, timeout: Duration, cancel: &CancellationToken, extra_env: &[(String, String)]) -> Result<ShellResult, String> {
    let stamp = uid("crew");
    let out_file = tmp.join(format!("{stamp}.out"));
    let err_file = tmp.join(format!("{stamp}.err"));
    let line = crate::process::shell_command_line(command);
    let mut env = crate::process::safe_env_pairs();
    env.extend(extra_env.iter().map(|(k, v)| (k.into(), v.into())));
    let (spawn_out, spawn_err) = (out_file.clone(), err_file.clone());
    let spawn_cwd = cwd.to_path_buf();
    let cancel = cancel.clone();
    // Пока команда идёт, её вывод читается в экран бота (только для run_command).
    let tail = crate::screen::tail_files(&out_file, &err_file);
    // Внутри песочницы ожидание блокирующее (WaitForSingleObject), поэтому уводим его с рантайма.
    let started = tokio::task::spawn_blocking(move || {
        crate::sandbox::spawn_ex_cancellable(&line, &spawn_cwd, &env, timeout, &spawn_out, &spawn_err, Some(&cancel))
    })
    .await;
    let outcome = match started {
        Ok(Ok(outcome)) => outcome,
        // Команда ещё не запускалась: повтор без песочницы не сделает её дважды.
        failed => {
            let _ = tokio::fs::remove_file(&out_file).await;
            let _ = tokio::fs::remove_file(&err_file).await;
            return Err(match failed {
                Ok(Err(reason)) => reason,
                _ => "запуск в песочнице оборвался".to_string(),
            });
        }
    };
    drop(tail);

    let read = |path: std::path::PathBuf| async move {
        let bytes = tokio::fs::read(&path).await.unwrap_or_default();
        let _ = tokio::fs::remove_file(&path).await;
        String::from_utf8_lossy(&bytes).into_owned()
    };
    let mut out = read(out_file).await;
    let err = read(err_file).await;
    if outcome.timed_out {
        out = format!("{out}\n[таймаут {} с: процесс убит]", timeout.as_secs());
    } else if outcome.cancelled {
        out = format!("{out}\n[команда остановлена пользователем]");
    }
    Ok(ShellResult { code: outcome.code, out, err, timed_out: outcome.timed_out })
}

/// Прежний путь: tokio-процесс, вывод в файлы. Работает для `CREW_SANDBOX=off` и как запасной.
async fn run_shell_plain(command: &str, cwd: &Path, tmp: &Path, timeout: Duration, cancel: &CancellationToken, extra_env: &[(String, String)]) -> ShellResult {
    // Вывод пишем в файлы, а не в пайпы: команда не упирается в лимиты буфера.
    let stamp = uid("crew");
    let out_file = tmp.join(format!("{stamp}.out"));
    let err_file = tmp.join(format!("{stamp}.err"));

    let opened = (std::fs::File::create(&out_file), std::fs::File::create(&err_file));
    let (Ok(fd_out), Ok(fd_err)) = opened else {
        return ShellResult { code: -1, out: String::new(), err: "не смог открыть файлы вывода".into(), timed_out: false };
    };
    let tail = crate::screen::tail_files(&out_file, &err_file);

    let mut cmd = shell_command(command);
    keep_safe_env(&mut cmd);
    cmd.envs(extra_env.iter().map(|(k, v)| (k, v)));
    cmd.current_dir(cwd)
        .stdin(Stdio::null())
        .stdout(Stdio::from(fd_out))
        .stderr(Stdio::from(fd_err))
        .kill_on_drop(true);
    // Своя группа процессов: по таймауту убиваем и дочерние.
    #[cfg(unix)]
    cmd.process_group(0);

    let mut code = -1;
    let mut timed_out = false;
    let mut cancelled = false;
    let mut spawn_error = String::new();
    match cmd.spawn() {
        Ok(mut child) => {
            tokio::select! {
                result = child.wait() => match result {
                    Ok(status) => code = status.code().unwrap_or(-1),
                    Err(e) => spawn_error = e.to_string(),
                },
                _ = tokio::time::sleep(timeout) => { timed_out = true; },
                _ = cancel.cancelled() => { cancelled = true; },
            }
            if timed_out || cancelled {
                // cmd.exe запускает дочерние процессы: завершаем и их, а не только оболочку.
                if let Some(pid) = child.id() {
                    #[cfg(windows)]
                    let mut kill = tokio::process::Command::new("taskkill");
                    #[cfg(windows)]
                    kill.args(["/PID", &pid.to_string(), "/T", "/F"]);
                    // Вне Windows оболочка ведёт свою группу процессов: сигнал уходит всей группе.
                    #[cfg(not(windows))]
                    let mut kill = tokio::process::Command::new("kill");
                    #[cfg(not(windows))]
                    kill.args(["-KILL", "--", &format!("-{pid}")]);
                    kill.stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null());
                    #[cfg(windows)]
                    kill.creation_flags(0x0800_0000);
                    let _ = kill.status().await;
                }
                let _ = child.kill().await;
                let _ = child.wait().await;
            }
        },
        Err(e) => spawn_error = e.to_string(),
    }
    drop(tail);

    let read = |path: std::path::PathBuf| async move {
        let bytes = tokio::fs::read(&path).await.unwrap_or_default();
        let _ = tokio::fs::remove_file(&path).await;
        String::from_utf8_lossy(&bytes).into_owned()
    };
    let mut out = read(out_file).await;
    let mut err = read(err_file).await;
    if timed_out {
        out = format!("{out}\n[таймаут {} с: процесс убит]", timeout.as_secs());
    } else if cancelled {
        out = format!("{out}\n[команда остановлена пользователем]");
    }
    if err.is_empty() && !spawn_error.is_empty() {
        err = spawn_error;
    }
    ShellResult { code, out, err, timed_out }
}

fn arg<'a>(args: &'a Value, key: &str) -> &'a str {
    args.get(key).and_then(Value::as_str).unwrap_or_default()
}

// Разрешённые поля профиля: разрешения и доступы меняет только человек в настройках.
fn bot_profile_args(args: &Value, editing: bool) -> Result<Value, String> {
    let object = args.as_object().ok_or("ожидался объект настроек")?;
    let mut profile = serde_json::Map::new();
    for (key, value) in object {
        if editing && key == "bot_id" { continue; }
        if !matches!(key.as_str(), "name" | "role" | "persona" | "driver" | "model" | "color") {
            return Err(format!("поле {key} нельзя изменить этим инструментом"));
        }
        let text = value.as_str().ok_or_else(|| format!("поле {key} должно быть строкой"))?;
        if key == "name" && (text.trim().is_empty() || text.chars().count() > 40) {
            return Err("имя должно содержать от 1 до 40 символов".into());
        }
        if key == "role" && text.chars().count() > 80 { return Err("роль длиннее 80 символов".into()); }
        if text.len() > 100_000 { return Err(format!("поле {key} слишком длинное")); }
        if key == "color" && !(text.len() == 7 && text.starts_with('#') && text[1..].bytes().all(|b| b.is_ascii_hexdigit())) {
            return Err("цвет должен быть в формате #RRGGBB".into());
        }
        profile.insert(key.clone(), value.clone());
    }
    if !editing && !profile.contains_key("name") { return Err("нужно имя бота".into()); }
    if editing && profile.is_empty() { return Err("не указаны поля для изменения".into()); }
    Ok(Value::Object(profile))
}

// ---------------------------------------------------------------------------
// Исполнение
// ---------------------------------------------------------------------------

pub async fn exec_tool(app: &Arc<App>, bot: &Bot, name: &str, args: &Value, depth: u32, cancel: &CancellationToken) -> Result<String, String> {
    let store = &app.store;
    // Облако: консоли нет, команда не запускается ни при каком вызове.
    if name == "run_command" {
        crate::cloud::deny(crate::cloud::NO_CONSOLE)?;
    }
    match name {
        "list_files" => {
            let mut rel = arg(args, "path").to_string();
            if crate::cloud::active() {
                // Абсолютный путь внутри своей папки превращается в путь относительно неё.
                let abs = cloud_target(store, &bot.id, &rel)?;
                let root = std::fs::canonicalize(store.workspace(&bot.id)).map_err(|e| e.to_string())?;
                let real = std::fs::canonicalize(&abs).map_err(|e| format!("{rel}: {e}"))?;
                let inner = real.strip_prefix(&root).map_err(|_| format!("{rel}: {}", crate::cloud::OUTSIDE_WORKSPACE))?;
                rel = inner.to_string_lossy().replace(std::path::MAIN_SEPARATOR, "/");
            }
            let rel = rel.as_str();
            if crate::reach::is_absolute_like(rel) {
                return list_outside(store, &bot.id, rel).await;
            }
            let items = store.list_workspace(&bot.id, rel).await?;
            let shown = if rel.is_empty() { "." } else { rel };
            if items.is_empty() {
                return Ok(format!("Папка «{shown}» пуста."));
            }
            let lines: Vec<String> = items
                .iter()
                .map(|i| {
                    let kind = if i["dir"] == json!(true) { "папка" } else { "файл" };
                    format!("{kind}: {}", i["path"].as_str().unwrap_or_default())
                })
                .collect();
            Ok(format!("Содержимое «{shown}»:\n{}", lines.join("\n")))
        }

        "read_file" => {
            let raw = arg(args, "path");
            // `~/.ssh/config`: только имена хостов, без адресов и путей к ключам.
            if !crate::cloud::active() && crate::keys::is_ssh_config(raw) {
                let path = if raw.starts_with('~') || crate::reach::is_absolute_like(raw) { expand_home(raw) } else { store.safe_path(&bot.id, raw)? };
                let text = tokio::fs::read_to_string(&path).await.map_err(|e| format!("{raw}: {e}"))?;
                return Ok(crate::keys::ssh_config_hosts(&text));
            }
            // Файл с ключами: сюда доходят только после карточки `secret`. Значения уходят в
            // хранилище, модели — ссылки `{{secret:ИМЯ}}`.
            if crate::keys::path_touch(raw).is_some() {
                let abs = key_file_target(store, &bot.id, raw)?;
                let bytes = tokio::fs::read(&abs).await.map_err(|e| e.to_string())?;
                let (shown, count) = crate::keys::take_values(app, &bot.id, &String::from_utf8_lossy(&bytes))?;
                let shown = crate::sensitive::mask(&shown, false);
                return Ok(clamp(&format!("{shown}\n\n{}", crate::keys::saved_note(count)), MAX_OUTPUT));
            }
            let abs = read_target(store, &bot.id, raw)?;
            let meta = tokio::fs::metadata(&abs).await.map_err(|e| e.to_string())?;
            if meta.len() > MAX_FILE {
                return Err(format!("файл больше {MAX_FILE} байт: {}", meta.len()));
            }
            let bytes = tokio::fs::read(&abs).await.map_err(|e| e.to_string())?;
            Ok(clamp(&String::from_utf8_lossy(&bytes), MAX_OUTPUT))
        }

        "write_file" => {
            let rel = arg(args, "path");
            if rel.is_empty() {
                return Err("не указан путь".into());
            }
            let (abs, _) = write_target(app, &bot.id, rel)?;
            if let Some(parent) = abs.parent() {
                tokio::fs::create_dir_all(parent).await.map_err(|e| e.to_string())?;
            }
            let content = args.get("content").and_then(Value::as_str).unwrap_or_default();
            tokio::fs::write(&abs, content).await.map_err(|e| e.to_string())?;
            Ok(format!("Записано {} символов в {rel}.", content.chars().count()))
        }

        "run_command" => {
            let command = arg(args, "command").trim();
            if command.is_empty() {
                return Err("пустая команда".into());
            }
            let cwd = store.workspace(&bot.id);
            // Секреты закрыты в любом режиме: карточкой их не открыть, отказ тот же, что у read_file.
            if let Some((raw, reason)) = crate::reach::command_secret(command, &cwd, store.data_dir()) {
                return Err(format!("{raw}: {reason}, сюда боту нельзя"));
            }
            tokio::fs::create_dir_all(&cwd).await.map_err(|e| e.to_string())?;
            if let Some(reason) = command_refused(app, &bot.id, command) {
                return Err(format!("команда не выполнена: {reason}"));
            }
            let seconds = match args.get("timeout_seconds") {
                None => 1800,
                Some(value) => value.as_u64().filter(|seconds| (1..=7200).contains(seconds))
                    .ok_or("timeout_seconds должен быть целым числом от 1 до 7200")?,
            };
            // `{{secret:ИМЯ}}` → переменная окружения процесса: в тексте команды значения нет.
            let known = crate::keys::known(app, &bot.id);
            let lookup = |name: &str| known.iter().find(|(n, _)| n == name).map(|(_, v)| v.clone());
            let (prepared, mut secret_env) = crate::keys::prepare_command(command, lookup)?;
            // Голое имя программы cmd ищет сначала в текущей папке: подложенный в папке бота `pwsh.bat` или `git.exe` запустился бы
            // вместо настоящего. Для всех команд бота поиск в текущей папке выключен (явный путь `.\x.exe` по-прежнему работает).
            secret_env.push(("NoDefaultCurrentDirectoryInExePath".to_string(), "1".to_string()));
            // Вывод идёт в экран бота, пока команда работает. «Стоп» там гасит только команду, не ход.
            let stop = cancel.child_token();
            let live = crate::screen::Shell::begin(app, &bot.id, command, stop.clone());
            let mut res = crate::screen::SHELL
                .scope(live.clone(), run_shell(&prepared, &cwd, &store.tmp_dir(), Duration::from_secs(seconds), &stop, &secret_env))
                .await;
            live.end(res.code, stop.is_cancelled() && !cancel.is_cancelled());
            // Команда читала ключи (после карточки `secret`): значения — в хранилище, модели — ссылки.
            let mut note = String::new();
            if crate::keys::command_touch(command).is_some() {
                let (out, count) = crate::keys::take_values(app, &bot.id, &res.out)?;
                res.out = crate::sensitive::mask(&out, false);
                res.err = crate::sensitive::mask(&res.err, false);
                note = crate::keys::saved_note(count);
            }
            let known = crate::keys::known(app, &bot.id);
            res.out = crate::keys::scrub(&res.out, &known);
            res.err = crate::keys::scrub(&res.err, &known);
            let mut parts = vec![format!("$ {command}")];
            if !note.is_empty() {
                parts.push(note);
            }
            if !res.out.is_empty() {
                parts.push(format!("stdout:\n{}", clamp(&res.out, 4000)));
            }
            if !res.err.is_empty() {
                parts.push(format!("stderr:\n{}", clamp(&res.err, 3000)));
            }
            parts.push(format!("код выхода: {}", res.code));
            if res.timed_out {
                Err(format!("Команда превысила лимит {seconds} с и остановлена вместе с дочерними процессами. Используй вывод ниже, реши, как продолжить, и при необходимости задай больший timeout_seconds.\n{}", parts.join("\n")))
            } else {
                Ok(parts.join("\n"))
            }
        }

        "web_fetch" => {
            let page = fetch_page(arg(args, "url"), &is_public_ip).await?;
            let text = if page.kind.contains("html") { strip_html(&page.body) } else { page.body };
            Ok(clamp(&format!("HTTP {} · {}\n\n{text}", page.status, page.kind), MAX_OUTPUT))
        }

        "telegram_posts" => {
            let name = crate::tg_channels::channel_name(arg(args, "channel"))?;
            let before = args["before"].as_u64().or_else(|| args["before"].as_f64().map(|n| n as u64));
            let query = arg(args, "query");
            let page = fetch_page(&crate::tg_channels::preview_url(&name, query, before), &is_public_ip).await?;
            Ok(crate::tg_channels::render(&page.body, &name, query).unwrap_or_else(|| crate::tg_channels::no_preview(&name)))
        }

        "web_search" => Ok(clamp(&crate::web_search::search(arg(args, "query")).await?, MAX_OUTPUT)),

        "remember" => crate::memory::remember(app, bot, args).await,

        "recall" => Ok(crate::memory::recall(app, &bot.id, args).await),

        "save_skill" => crate::skills::save(app, bot, args).await,

        "read_skill" => crate::skills::read(app, bot, args),

        "workspace_graph" => crate::graph::exec(app, &bot.id, args).await,

        "ask_user" => {
            let question = arg(args, "question").trim();
            if question.is_empty() {
                return Err("пустой вопрос".into());
            }
            if question.chars().count() > crate::turn::QUESTION_LIMIT {
                return Err(format!("вопрос длиннее {} знаков: спроси короче", crate::turn::QUESTION_LIMIT));
            }
            let (choices, form) = ask_shape(args)?;
            let is_form = !form.is_empty();
            let answer = crate::turn::request_answer(app, &bot.id, question, choices, form).await;
            if is_form && answer.starts_with('{') {
                Ok(format!("Ответ человека (JSON): {answer}"))
            } else {
                Ok(format!("Ответ человека: {answer}"))
            }
        }

        "generate_image" => crate::image::generate(app, bot, args).await,

        "list_bots" => {
            let bots: Vec<Value> = store.bots().into_iter().map(|b| json!({
                "id": b.id, "name": b.name, "role": b.role, "persona": b.persona,
                "driver": b.driver, "model": b.model, "color": b.color
            })).collect();
            Ok(json!({ "bots": bots }).to_string())
        }
        "create_bot" => {
            let profile = bot_profile_args(args, false)?;
            let created = store.add_bot(&profile).await.map_err(|e| e.to_string())?;
            app.turns.emit(json!({ "type": "state_changed" }));
            Ok(json!({ "id": created.id, "name": created.name, "role": created.role }).to_string())
        }
        "update_bot" => {
            let id = arg(args, "bot_id").trim();
            if id.is_empty() { return Err("нужен ID бота из list_bots".into()); }
            let profile = bot_profile_args(args, true)?;
            let updated = store.update_bot(id, &profile).await.map_err(|e| e.to_string())?
                .ok_or("бот с таким ID не найден")?;
            app.turns.emit(json!({ "type": "state_changed" }));
            Ok(json!({ "id": updated.id, "name": updated.name, "role": updated.role }).to_string())
        }

        "delegate" => {
            let target = arg(args, "bot").trim();
            let task = arg(args, "task").trim();
            if target.is_empty() {
                return Err("не указан бот".into());
            }
            if task.is_empty() {
                return Err("пустая задача".into());
            }
            if task.chars().count() > MAX_BRIEF_CHARS {
                return Err(BRIEF_TOO_LONG.into());
            }
            let result = crate::turn::delegate_to(app.clone(), bot.clone(), target.to_string(), task.to_string(), depth).await?;
            Ok(result)
        }

        "offer_specialist" => {
            // Предлагает только ведущий: специалисту нечего передавать дальше.
            if bot.rank != "lead" {
                return Err("предложить специалиста может только ведущий бот".into());
            }
            let target_name = arg(args, "bot").trim();
            let reason = arg(args, "reason").trim();
            let task = arg(args, "task").trim();
            if target_name.is_empty() {
                return Err("не указан специалист".into());
            }
            if reason.is_empty() {
                return Err("не указана причина".into());
            }
            if task.is_empty() {
                return Err("пустая задача".into());
            }
            let wanted = target_name.to_lowercase();
            let bots = store.bots();
            let target = bots
                .iter()
                .find(|b| b.name.to_lowercase() == wanted)
                .or_else(|| bots.iter().find(|b| b.name.to_lowercase().starts_with(&wanted)))
                .cloned()
                .ok_or_else(|| format!("в команде нет бота «{target_name}»"))?;
            if target.id == bot.id {
                return Err("нельзя предложить самого себя".into());
            }
            if target.rank != "specialist" {
                return Err(format!("«{}» не специалист: предложить можно только специалиста", target.name));
            }
            let offer = crate::offers::Offer {
                id: uid("offer"),
                host_bot_id: bot.id.clone(),
                specialist_id: target.id.clone(),
                reason: reason.chars().take(300).collect(),
                task: task.chars().take(4_000).collect(),
                at: crate::store::now(),
                team: None,
                change: None,
            };
            app.offers.add(offer.clone());
            app.turns.emit(json!({ "type": "specialist_offer", "botId": bot.id, "offer": offer.event_json(&target.name) }));
            Ok("Карточка показана человеку. Закончи ход одной фразой: ждёшь его решения.".into())
        }

        "propose_team" => crate::team_proposal::propose(app, bot, args, cancel).await,

        "propose_bot_change" => crate::team_changes::propose_bot_change(app, bot, args, cancel).await,

        "propose_routine" => crate::team_changes::propose_routine(app, bot, args, cancel).await,

        "search_catalog" => crate::team_proposal::search_catalog(app, args).await,

        "list_habits" | "check_habit" => crate::habits::exec_bot_tool(app, name, args),

        "list_todos" | "add_todo" | "complete_todo" => crate::todos::exec_bot_tool(app, name, args),

        script if crate::scripts::is_script_tool(script) => crate::scripts::exec(app, bot, script, args, depth, cancel).await,

        goal if crate::goal_tools::is_goal_tool(goal) => crate::goal_tools::exec(app, bot, goal, args).await,

        team if crate::team_tasks::is_tool(team) => crate::team_tasks::exec(app, bot, team, args, depth).await,

        routine if crate::routine_tools::is_routine_tool(routine) => crate::routine_tools::exec(app, bot, routine, args).await,

        flow if crate::workflow_tools::is_tool(flow) => crate::workflow_tools::exec(app, bot, flow, args).await,

        browser if browser.starts_with("browser_") => crate::browser_tools::exec(app, bot, browser, args).await,

        other => Err(format!("неизвестный инструмент: {other}")),
    }
}
