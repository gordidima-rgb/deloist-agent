//! Хранилище. Всё лежит на диске: одно состояние, NDJSON-треды, память, воркспейсы ботов.
//! Формат файлов тот же, что у прежней Node-версии, поэтому старая папка `data/` подходит как есть.

use std::collections::{BTreeMap, HashMap};
use std::path::{Component, Path, PathBuf};
use std::sync::Mutex;

use rand::RngCore;
use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};
use tokio::io::AsyncWriteExt;

use crate::reminders;
use crate::schedule;

/// Следующий срок рутины: у напоминания — свой расчёт (опережение, разовая дата), у обычной рутины — как раньше.
fn next_run_for(routine: &Routine, now_ms: i64) -> i64 {
    if reminders::is_reminder(routine) {
        reminders::schedule_from(routine, now_ms)
    } else {
        schedule::next_run_ms(routine, now_ms)
    }
}

pub fn uid(prefix: &str) -> String {
    let mut bytes = [0u8; 8];
    rand::thread_rng().fill_bytes(&mut bytes);
    format!("{prefix}_{}", hex(&bytes))
}

pub fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

pub fn now() -> i64 {
    chrono::Utc::now().timestamp_millis()
}

/// Индекс первой записи, которую видит модель: сразу после последнего якоря окна.
fn last_anchor(all: &[Value]) -> usize {
    all.iter()
        .rev()
        .find(|m| m["role"] == "anchor")
        .and_then(|m| m["from"].as_u64())
        .map_or(0, |from| (from as usize).min(all.len()))
}

/// Индексы записей, которые видит модель: после якоря, без тихих ходов и служебных меток.
fn model_visible(all: &[Value]) -> Vec<usize> {
    let from = last_anchor(all);
    let quiet_turns: std::collections::HashSet<&str> = all[from..]
        .iter()
        .filter(|m| m["role"] == "quiet")
        .filter_map(|m| m["turnId"].as_str())
        .collect();
    (from..all.len())
        .filter(|&i| {
            let m = &all[i];
            m["role"] != "quiet" && m["role"] != "anchor" && m["role"] != "task" && m["turnId"].as_str().map_or(true, |t| !quiet_turns.contains(t))
        })
        .collect()
}

fn model_window(all: &[Value]) -> Vec<Value> {
    model_visible(all).into_iter().map(|i| all[i].clone()).collect()
}

/// Когда модель последний раз видела часы: часы вне окна или из тихого хода не считаются.
fn last_clock(all: &[Value]) -> Option<i64> {
    model_visible(all).into_iter().rev().find(|&i| all[i]["role"] == "time").and_then(|i| all[i]["at"].as_i64())
}

/// Куда поставить новый якорь, если окно переполнилось. Считаем только то, что видит модель:
/// иначе ночь тихих рутин вытеснила бы из окна живую переписку.
fn anchor_position(all: &[Value], keep: usize, retain: usize) -> Option<usize> {
    let visible = model_visible(all);
    let keep = keep.max(1);
    if visible.len() <= keep {
        return None;
    }
    let tail = &visible[visible.len() - retain.clamp(1, keep)..];
    // Окно начинается с реплики человека: строгие провайдеры не принимают историю с ответа
    // бота, а после якоря такое начало держалось бы десятки ходов.
    let start = tail.iter().copied().find(|&i| matches!(all[i]["role"].as_str(), Some("user" | "time")));
    Some(start.unwrap_or(tail[0]))
}

fn is_user_like(entry: &Value) -> bool {
    matches!(entry["role"].as_str(), Some("user" | "time"))
}

/// Сводка, которую модель видит вместо истории до последнего якоря.
fn last_summary(all: &[Value]) -> Option<String> {
    all.iter()
        .rev()
        .find(|m| m["role"] == "anchor")
        .and_then(|m| m["summary"].as_str())
        .filter(|s| !s.trim().is_empty())
        .map(str::to_string)
}

/// Примерный размер записи в знаках. Результат инструмента модель видит обрезанным, так и считаем.
fn entry_chars(entry: &Value) -> usize {
    let mut chars = entry["text"].as_str().map_or(0, |t| t.chars().count());
    chars += entry[crate::memory::RECALL_FIELD].as_str().map_or(0, |t| t.chars().count());
    chars += entry[crate::dates_ru::NOTE_FIELD].as_str().map_or(0, |t| t.chars().count());
    for call in entry["toolCalls"].as_array().into_iter().flatten() {
        chars += call["args"].to_string().chars().count();
        chars += call["result"].as_str().map_or(0, |r| r.chars().count().min(4_000));
    }
    chars
}

/// Сколько знаков видит модель: по ним окно сжимается раньше счётчика записей.
fn window_chars(all: &[Value]) -> usize {
    model_visible(all).into_iter().map(|i| entry_chars(&all[i])).sum()
}

/// Что свернуть в сводку: с какой записи начнётся новое окно, что из него выпадет и прежняя сводка.
#[derive(Debug)]
pub struct CompactPlan {
    pub start: usize,
    pub dropped: Vec<Value>,
    pub summary: Option<String>,
}

/// По счётчику окно двигается как раньше: больше `keep` — остаётся около `retain`.
/// По размеру или по просьбе остаётся последняя треть того, что видит модель.
/// Новое окно всегда начинается с реплики человека.
fn compact_plan(all: &[Value], keep: usize, retain: usize, force: bool, budget: usize) -> Option<CompactPlan> {
    let visible = model_visible(all);
    let by_count = keep > 0 && visible.len() > keep;
    let by_size = budget > 0 && window_chars(all) > budget;
    let start = if force || by_size {
        let tail_from = visible.len() - (visible.len() / 3).max(1).min(visible.len());
        visible[tail_from..]
            .iter()
            .copied()
            .find(|&i| is_user_like(&all[i]))
            .or_else(|| visible.iter().copied().rev().find(|&i| is_user_like(&all[i])))?
    } else if by_count {
        anchor_position(all, keep, retain)?
    } else {
        return None;
    };
    let dropped: Vec<Value> = visible.iter().copied().filter(|&i| i < start).map(|i| all[i].clone()).collect();
    if dropped.is_empty() {
        return None;
    }
    Some(CompactPlan { start, dropped, summary: last_summary(all) })
}

/// Тред, о котором речь: личный тред бота или группа.
#[derive(Clone, Copy, Debug)]
pub enum Thread<'a> {
    Bot(&'a str),
    Room(&'a str),
}

/// Для человека якорь со сводкой — это разделитель «раньше было вот что», остальные метки не нужны.
fn human_view(all: Vec<Value>) -> Vec<Value> {
    all.into_iter()
        .filter(|m| m["role"] != "time")
        .filter_map(|m| {
            if m["role"] != "anchor" {
                return Some(m);
            }
            let summary = m["summary"].as_str().filter(|s| !s.trim().is_empty())?;
            Some(json!({
                "id": m.get("id").cloned().unwrap_or_else(|| json!(format!("anchor-{}", m["at"]))),
                "role": "compacted", "text": summary, "dropped": m["dropped"], "at": m["at"],
            }))
        })
        .collect()
}

// ---------------------------------------------------------------------------
// Модель
// ---------------------------------------------------------------------------

/// Мост в Telegram: выбранный бот команды, привязанный чат и место в потоке обновлений.
/// Сам токен бота лежит только в `secrets.json`, сюда он не попадает.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct TelegramSettings {
    pub bot_id: String,
    pub chat_id: String,
    pub enabled: bool,
    /// `update_id + 1` последнего разобранного обновления: перезапуск не повторяет старые.
    pub offset: i64,
}

/// Мост в Discord: выбранный бот команды, привязанный человек и его личный канал (DM).
/// Сам токен бота лежит только в `secrets.json`, сюда он не попадает.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct DiscordSettings {
    pub bot_id: String,
    pub user_id: String,
    pub channel_id: String,
    pub enabled: bool,
}

/// Мост в Slack: выбранный бот команды, привязанный человек, его личный канал (DM) и рабочее
/// пространство. Сами токены лежат только в `secrets.json`, сюда они не попадают.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct SlackSettings {
    pub bot_id: String,
    pub user_id: String,
    pub channel_id: String,
    pub enabled: bool,
    pub team: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Settings {
    pub driver: String,
    pub model: String,
    pub base_url: String,
    pub api_key_env: String,
    pub cli_command: String,
    /// Запасная модель на случай лимита или баланса у основной: "" — нет, "api" или "cli".
    pub fallback_driver: String,
    pub fallback_model: String,
    pub fallback_base_url: String,
    pub fallback_cli_command: String,
    /// Внешний агент ACP: полная команда запуска, например `npx -y @agentclientprotocol/claude-agent-acp`.
    pub acp_command: String,
    /// Переменные окружения для агента ACP поверх обычного окружения.
    pub acp_env: std::collections::BTreeMap<String, String>,
    /// Режим сессии агента ACP (`read-only`, `default` и т.п.). Пусто — как решит агент.
    pub acp_mode: String,
    /// Свои адреса серверов моделей (OpenAI-совместимых) для поиска на компьютере и в сети.
    pub model_servers: Vec<String>,
    pub temperature: f64,
    pub history_limit: i64,
    /// Модель понимает картинки: снимки экрана уходят ей, а не только в файл.
    pub vision: bool,
    /// Сервис картинок: адрес OpenAI-совместимого `/images/generations`. Пусто — выключено.
    pub image_base_url: String,
    pub image_model: String,
    /// Сколько картинок в день можно нарисовать. 0 — без предела.
    pub image_daily: i64,
    pub browser_path: String,
    /// Движок браузера бота: пусто или chrome — настоящий Chrome/Edge, obscura — лёгкий Obscura, moli — лёгкий Moli.
    /// В облаке (`CREW_CLOUD=1`) пусто значит Moli.
    pub browser_engine: String,
    /// Язык интерфейса: ru или en. Пусто значит «ещё не выбран».
    pub ui_lang: String,
    /// Шрифт интерфейса: system, onest или golos.
    pub ui_font: String,
    /// Сама проверять, не вышла ли новая версия.
    pub check_updates: bool,
    /// Ставить найденную версию сама, в фоне: перезапуск только по кнопке или при следующем запуске.
    pub auto_update: bool,
    /// Порт окна. Один и тот же между запусками: на нём держатся адреса вебхуков и настройки браузера.
    pub local_port: i64,
    /// Ярлык на рабочем столе уже предлагали: удалённый человеком ярлык заново не появляется.
    pub desktop_shortcut_done: bool,
    /// Экран «Собери свою ИИ-команду» уже пройден: подписка выбрана.
    pub team_onboarded: bool,
    /// Где живут питомцы: off, desktop (под окнами) или top (поверх всех).
    pub pets_mode: String,
    /// Док агентов у края экрана: аватары ботов и кнопка нового.
    pub dock_enabled: bool,
    /// Слой дока: top (поверх других приложений) или normal (обычное окно).
    pub dock_layer: String,
    /// Дисплей дока: пусто — где окно программы, иначе имя монитора Windows (`\.\DISPLAY2`).
    pub dock_display: String,
    /// Край экрана: right или left.
    pub dock_edge: String,
    /// Где док стоит вдоль края: 0 — вверху, 1 — внизу.
    pub dock_offset: f64,
    /// Уходит за край, пока указатель не подведут к краю.
    pub dock_autohide: bool,
    /// Ответы ботов ненадолго появляются рядом с доком.
    pub dock_results: bool,
    /// Пока бот работает, рядом с его аватаром видно, чем он занят.
    pub dock_activity: bool,
    /// Кому уходит голос с микрофона дока и горячей клавиши: пусто — последнему, с кем говорил, иначе id бота.
    pub voice_target: String,
    /// Тема окна, чтобы док и питомцы совпадали с ним: light, dark или пусто (как в системе).
    pub ui_theme: String,
    /// Сколько токенов в сутки могут потратить рутины. 0 — без предела.
    pub daily_token_limit: i64,
    /// Запускать вместе с Windows, в трее.
    pub autostart: bool,
    /// Не давать компьютеру засыпать, пока есть включённые рутины.
    pub keep_awake: bool,
    /// Режим сервера: автозапуск без окна и запрет сна, пока программа работает. Меняется только отдельным адресом.
    pub server_mode: bool,
    /// Записи журнала старше этого человек уже видел.
    pub journal_seen_at: i64,
    /// Бот сам сохраняет навыки: `auto` — сразу, с отменой; `ask` — через карточку; `off` — нет.
    pub skill_learning: String,
    /// Последняя версия программы, которую человек уже видел в диалоге «Что нового».
    #[serde(default)]
    pub last_seen_version: String,
    /// Мост в Telegram: без токена из `secrets.json` ничего не делает.
    pub telegram: TelegramSettings,
    /// Мост в Discord: без токена из `secrets.json` ничего не делает.
    pub discord: DiscordSettings,
    /// Мост в Slack: без пары токенов из `secrets.json` ничего не делает.
    pub slack: SlackSettings,
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            driver: "demo".into(),
            model: "gpt-4o-mini".into(),
            base_url: "https://api.openai.com/v1".into(),
            api_key_env: "OPENAI_API_KEY".into(),
            cli_command: "claude -p".into(),
            fallback_driver: String::new(),
            fallback_model: String::new(),
            fallback_base_url: String::new(),
            fallback_cli_command: String::new(),
            acp_command: String::new(),
            acp_env: std::collections::BTreeMap::new(),
            acp_mode: String::new(),
            model_servers: Vec::new(),
            temperature: 0.7,
            history_limit: 30,
            vision: false,
            image_base_url: String::new(),
            image_model: String::new(),
            image_daily: 50,
            browser_path: String::new(),
            browser_engine: String::new(),
            ui_lang: String::new(),
            ui_font: "system".into(),
            check_updates: true,
            auto_update: true,
            local_port: 0,
            desktop_shortcut_done: false,
            team_onboarded: false,
            pets_mode: "off".into(),
            dock_enabled: true,
            dock_layer: "top".into(),
            dock_display: String::new(),
            dock_edge: "right".into(),
            dock_offset: 0.4,
            dock_autohide: false,
            dock_results: true,
            dock_activity: true,
            voice_target: String::new(),
            ui_theme: String::new(),
            daily_token_limit: 1_000_000,
            autostart: false,
            keep_awake: false,
            server_mode: false,
            journal_seen_at: 0,
            skill_learning: "auto".into(),
            last_seen_version: String::new(),
            telegram: TelegramSettings::default(),
            discord: DiscordSettings::default(),
            slack: SlackSettings::default(),
            extra: Map::new(),
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Bot {
    pub id: String,
    pub name: String,
    pub role: String,
    pub emoji: String,
    pub color: String,
    pub persona: String,
    /// Одобренные правила от человека (`brain.md`, см. `brain.rs`). В ход идёт эта копия, а не файл в папке бота.
    /// Меняется только карточкой «Применить» или в ящике бота: в общем PATCH бота этого поля нет.
    pub brain: String,
    pub driver: String,
    pub model: String,
    /// `lead` или `specialist`: кого показывать в боковой панели и кого звать гостем.
    pub rank: String,
    pub pinned: bool,
    /// Необязательная метка команды, до 40 символов.
    pub team: String,
    /// Через сколько дней бездействия специалист уходит в резерв. 0 — никогда.
    pub reserve_after_days: i64,
    /// Когда бот последний раз закончил ход, мс. 0 — активности ещё не было.
    pub last_active_at: i64,
    pub created_at: i64,
    /// Сайты, где человек разрешил боту нажимать и вводить текст.
    pub browser_sites: Vec<String>,
    /// Политика подтверждений: `ask` (по умолчанию), `auto` или `strict`.
    pub approval: String,
    /// Виды действий, которые бот спрашивает всегда, в любом режиме и без «запомнить» (`tools::ALWAYS_ASK_KINDS`). Меняет только человек в окне.
    pub always_ask: Vec<String>,
    /// Подключённые навыки: идентификаторы из state.skills.
    pub skills: Vec<String>,
    /// Какой из двенадцати стеклянных шаров (web/avatar.js, ORBS) стоит у бота. -1 — ещё не назначен: при загрузке получает шар по id.
    pub skin: i64,
    /// Показывать бота питомцем на рабочем столе.
    pub pet: bool,
    /// Куда человек его поставил, в пикселях экрана. Пока не двигали, места нет.
    pub pet_x: Option<i32>,
    pub pet_y: Option<i32>,
    /// Показывать бота в доке агентов. Новые боты там сразу.
    pub dock: bool,
    /// Читать ответы вслух (голос 0.3.1). Переключатель в меню мини-чата дока.
    pub read_aloud: bool,
    /// Предел токенов бота за календарный месяц. 0 — без предела (см. `limit::month_blocked`).
    pub monthly_token_limit: i64,
    /// Бот-оркестратор: расспрашивает человека и собирает команду (`propose_team`). Его нельзя убрать, пока жива команда.
    pub orchestrator: bool,
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

impl Default for Bot {
    fn default() -> Self {
        Self {
            id: String::new(),
            name: String::new(),
            role: String::new(),
            emoji: String::new(),
            color: "#7a8595".into(),
            persona: String::new(),
            brain: String::new(),
            driver: String::new(),
            model: String::new(),
            rank: "lead".into(),
            pinned: false,
            team: String::new(),
            reserve_after_days: 7,
            last_active_at: 0,
            created_at: 0,
            browser_sites: Vec::new(),
            approval: "ask".into(),
            always_ask: Vec::new(),
            skills: Vec::new(),
            skin: -1,
            pet: false,
            pet_x: None,
            pet_y: None,
            dock: true,
            read_aloud: false,
            monthly_token_limit: 0,
            orchestrator: false,
            extra: Map::new(),
        }
    }
}

impl Bot {
    /// Специалист без закрепления, который давно не работал, уходит в резерв.
    /// Пока активности не записано (`lastActiveAt == 0`), в резерв не отправляем.
    pub fn in_reserve(&self, at: i64) -> bool {
        self.rank == "specialist" && !self.pinned && inactive_for_reserve(self.reserve_after_days, self.last_active_at, at)
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Routine {
    pub id: String,
    pub bot_id: String,
    pub title: String,
    pub prompt: String,
    pub every_minutes: i64,
    /// Верх случайного интервала: срок выпадает между `every_minutes` и этим числом, чтобы запуски
    /// не шли ровной сеткой. 0 или не больше `every_minutes` — интервал ровный.
    pub every_max_minutes: i64,
    /// «09:00»: запускать в это время, а не каждые N минут.
    pub at: String,
    /// Cron-выражение (5 полей). Если задано, заменяет every/at/days/window.
    pub cron: String,
    /// Часовой пояс IANA для cron. Пусто — местное время ПК.
    pub tz: String,
    /// Дни недели, 1 — понедельник. Пусто — каждый день.
    pub days: Vec<i64>,
    /// Повтор по тому же правилу, что у задач (dates_ru::Recurrence). Пусто — нет правила.
    #[serde(default)]
    pub recurrence: Option<crate::dates_ru::Recurrence>,
    /// Не раньше этой даты (YYYY-MM-DD, местное время), пусто — без ограничения.
    #[serde(default, rename = "startOn")]
    pub start_on: String,
    /// Разовый запуск в миллисекундах; 0 — повторяющийся.
    #[serde(default, rename = "onceAt")]
    pub once_at: i64,
    /// Окно работы, может переходить через полночь: «22:00»–«07:00». Пусто — всегда.
    pub window_from: String,
    pub window_to: String,
    pub enabled: bool,
    /// Писать в тред и журнал, только если есть что сказать. По умолчанию так и есть.
    pub quiet: bool,
    pub last_run: i64,
    pub next_run: i64,
    pub created_at: i64,
    /// Запускается не по расписанию, а вызовом вебхука.
    pub hook: bool,
    /// Секрет в адресе вебхука. Пусто у обычной рутины.
    pub hook_token: String,
    /// События GitHub, на которые реагирует вебхук. Пусто — на любое.
    #[serde(default)]
    pub hook_events: Vec<String>,
    /// Цель, которую запускает рутина вместо обычного задания. Пусто — рутина как раньше.
    pub goal_id: String,
    /// Осторожность создателя, если рутину завёл другой бот: запуски не смелее её.
    pub approval_floor: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_status: Option<String>,
    /// Сколько запусков подряд кончились ошибкой: после 3 рутина идёт не чаще раза в час, после 6 — раза в 4 часа.
    #[serde(skip_serializing_if = "is_zero_u32")]
    pub failures: u32,
    /// Причина последней ошибки: подпись в строке рутины и в истории.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stop_reason: Option<crate::runs::StopReason>,
    /// Три «вечные» ошибки подряд (вход, проверка, неизвестная): рутина на паузе, пока человек не нажмёт «Возобновить».
    #[serde(skip_serializing_if = "is_false")]
    pub error_paused: bool,
    /// С какого момента расписание ждёт занятого бота (0 — не ждёт). Только в памяти.
    #[serde(skip)]
    pub busy_since: i64,
    /// Что делать с пропущенными запусками: "last" — выполнить последний, "skip" — пропустить.
    #[serde(default = "default_catch_up")]
    pub catch_up: String,
    /// Заметка с прошлого запуска (finish_turn note), обрезанная до 8000 знаков, хранится хвост.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub progress_note: String,
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

fn default_catch_up() -> String {
    "last".to_string()
}

fn is_zero_u32(v: &u32) -> bool {
    *v == 0
}

fn is_false(v: &bool) -> bool {
    !*v
}

/// Чем кончился запуск рутины для расписания.
#[derive(Debug, Clone, PartialEq)]
pub enum RoutineOutcome {
    /// Не ошибка (выполнена, тихо, остановил человек, пауза): серия не растёт.
    Done,
    /// Ошибка, серия подряд — `failures`.
    Failed { failures: u32 },
    /// Третья «вечная» ошибка подряд: рутина встала на паузу, человеку сообщаем один раз.
    PausedByErrors { failures: u32 },
}

impl Default for Routine {
    fn default() -> Self {
        Self {
            id: String::new(),
            bot_id: String::new(),
            title: String::new(),
            prompt: String::new(),
            every_minutes: 60,
            every_max_minutes: 0,
            at: String::new(),
            cron: String::new(),
            tz: String::new(),
            days: Vec::new(),
            recurrence: None,
            start_on: String::new(),
            once_at: 0,
            window_from: String::new(),
            window_to: String::new(),
            enabled: true,
            quiet: true,
            last_run: 0,
            next_run: 0,
            created_at: 0,
            hook: false,
            hook_token: String::new(),
            hook_events: Vec::new(),
            goal_id: String::new(),
            approval_floor: String::new(),
            last_status: None,
            failures: 0,
            stop_reason: None,
            error_paused: false,
            busy_since: 0,
            catch_up: "last".to_string(),
            progress_note: String::new(),
            extra: Map::new(),
        }
    }
}

/// Навык — блок инструкций, который бот получает целиком и только когда он ему подключён.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Skill {
    pub id: String,
    pub name: String,
    pub description: String,
    pub body: String,
    pub created_at: i64,
    /// Пусто — навык написал человек; `learned` — бот сохранил его сам.
    pub origin: String,
    /// Кто выучил навык.
    pub bot_id: String,
    pub updated_at: i64,
    /// Прошлые версии для отмены, новые в конце.
    pub previous: Vec<SkillVersion>,
    /// Навык из каталога: его идентификатор там (`skill.check-a-claim`). Пусто — не из каталога.
    pub catalog_id: String,
    /// Версия каталога, которую человек принял последней.
    pub catalog_version: String,
    /// Хэш текста в момент установки или обновления: другой хэш значит, что навык правили.
    pub catalog_hash: String,
}

/// Версия навыка до правки ботом.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct SkillVersion {
    pub name: String,
    pub description: String,
    pub body: String,
    pub at: i64,
}

impl Default for Skill {
    fn default() -> Self {
        Self {
            id: String::new(),
            name: String::new(),
            description: String::new(),
            body: String::new(),
            created_at: 0,
            origin: String::new(),
            bot_id: String::new(),
            updated_at: 0,
            previous: Vec::new(),
            catalog_id: String::new(),
            catalog_version: String::new(),
            catalog_hash: String::new(),
        }
    }
}

/// Сколько прошлых версий навыка хранить для отмены.
const SKILL_VERSIONS: usize = 5;

/// Предел картинки команды, байт.
pub const ROOM_PICTURE_MAX: usize = 4 * 1024 * 1024;

/// Расширение по первым байтам: png, jpeg или webp; иначе `None`.
pub fn picture_ext(bytes: &[u8]) -> Option<&'static str> {
    if bytes.starts_with(&[0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a]) {
        Some("png")
    } else if bytes.starts_with(&[0xff, 0xd8, 0xff]) {
        Some("jpg")
    } else if bytes.len() > 12 && &bytes[0..4] == b"RIFF" && &bytes[8..12] == b"WEBP" {
        Some("webp")
    } else {
        None
    }
}

/// Группа: несколько ботов в одном треде со своими правилами, кто отвечает.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Room {
    pub id: String,
    pub name: String,
    pub emoji: String,
    pub color: String,
    /// Общие инструкции группы: подмешиваются каждому её боту.
    pub instructions: String,
    pub bot_ids: Vec<String>,
    /// Кто отвечает, если не выбрала модель и нет упоминания.
    pub lead: String,
    /// `lead` — всегда ведущий, `auto` — модель выбирает подходящего.
    pub mode: String,
    /// Группа закончена и убрана из списка, но её история осталась.
    pub archived: bool,
    /// Папки, доступные каждому боту группы, но только внутри разговора этой группы.
    /// Проверены при записи (`rooms::check_folders`): абсолютные, существующие, не секретные.
    pub folders: Vec<String>,
    /// Из `folders`: папки, в которых лежит сама программа и которые человек подтвердил явно в настройках
    /// команды (проект целиком, например `C:\Projects\deloist`). Ставится только человеческим PATCH.
    pub program_folders: Vec<String>,
    /// Навыки, которые получает каждый бот группы, но только внутри этой группы.
    pub skills: Vec<String>,
    /// Расширение файла картинки команды (`png`, `jpg`, `webp`) или пусто. Сам файл — `rooms/<id>/picture.<ext>`.
    pub picture: String,
    /// Когда картинку сменили: для адреса с версией, чтобы окно не держало старую.
    pub picture_at: i64,
    /// `interview` — оркестратор расспрашивает человека, `ready` — команда собрана (по умолчанию, у старых команд так).
    pub stage: String,
    pub created_at: i64,
}

impl Default for Room {
    fn default() -> Self {
        Self {
            id: String::new(),
            name: String::new(),
            emoji: String::new(),
            color: "#7a8595".into(),
            instructions: String::new(),
            bot_ids: Vec::new(),
            lead: String::new(),
            mode: "lead".into(),
            archived: false,
            folders: Vec::new(),
            program_folders: Vec::new(),
            skills: Vec::new(),
            picture: String::new(),
            picture_at: 0,
            stage: STAGE_READY.into(),
            created_at: 0,
        }
    }
}

/// Подключённый MCP-сервер: свои инструменты без кода.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct McpServer {
    pub id: String,
    pub name: String,
    pub command: String,
    /// Адрес HTTP-сервера (MCP Streamable HTTP). Пусто — запускаем команду по stdio.
    /// Поле новое: старые state.json без него читаются как stdio.
    #[serde(default)]
    pub url: String,
    pub args: Vec<String>,
    pub enabled: bool,
    /// Не спрашивать человека на каждый вызов инструментов этого сервера.
    pub auto_approve: bool,
    pub created_at: i64,
    /// Окружение процесса stdio. Значения — шаблоны: `{{secret:NAME}}` подставляется при запуске
    /// из хранилища секретов, в state.json лежит только ссылка.
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    pub env: BTreeMap<String, String>,
    /// Заголовки HTTP-сервера, тоже шаблоны.
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    pub headers: BTreeMap<String, String>,
    /// Сервер поставлен из манифеста каталога: закреплённая установка, согласие, вход.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub connector: Option<crate::connectors::Meta>,
    /// Свой HTTP-сервер (не из каталога), которому нужен вход: адреса сервера авторизации и client id.
    /// Токены лежат в хранилище секретов, здесь их нет.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub login: Option<Login>,
}

/// Вход в свой MCP-сервер по OAuth.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", default)]
pub struct Login {
    pub oauth: crate::oauth::Config,
    /// Сервер ответил 401 или токен отозван: нужна кнопка «Войти».
    pub needs_reauth: bool,
}

impl Default for McpServer {
    fn default() -> Self {
        Self {
            id: String::new(),
            name: String::new(),
            command: String::new(),
            url: String::new(),
            args: Vec::new(),
            enabled: true,
            auto_approve: false,
            created_at: 0,
            env: BTreeMap::new(),
            headers: BTreeMap::new(),
            connector: None,
            login: None,
        }
    }
}

impl McpServer {
    /// Транспорт: непустой адрес — HTTP, иначе stdio.
    pub fn is_http(&self) -> bool {
        !self.url.trim().is_empty()
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub(crate) struct State {
    version: i64,
    created_at: i64,
    settings: Settings,
    bots: Vec<Bot>,
    routines: Vec<Routine>,
    skills: Vec<Skill>,
    rooms: Vec<Room>,
    mcp: Vec<McpServer>,
}

impl Default for State {
    fn default() -> Self {
        Self {
            version: 1,
            created_at: 0,
            settings: Settings::default(),
            bots: Vec::new(),
            routines: Vec::new(),
            skills: Vec::new(),
            rooms: Vec::new(),
            mcp: Vec::new(),
        }
    }
}

#[derive(Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
struct Secrets {
    api_key: String,
    /// Ключ запасной модели: отдельный, чтобы основная и запасная могли быть у разных провайдеров.
    fallback_api_key: String,
    /// Ключ сервиса картинок: у него может быть свой провайдер.
    image_api_key: String,
    /// Ключи нативных драйверов: у каждого провайдера свой, чужому он не уходит.
    anthropic_api_key: String,
    gemini_api_key: String,
    /// Вход в аккаунт Deloist: токен устройства от сайта и кому он выдан.
    account_token: String,
    account_email: String,
    account_name: String,
    /// Ключи провайдеров из окна «Подключить свою ИИ»: по идентификатору провайдера.
    provider_keys: std::collections::BTreeMap<String, String>,
    /// Токен своего бота Telegram: только здесь, в state.json и ответах его нет.
    telegram_bot_token: String,
    /// Секрет вебхука Telegram (`secret_token` у `setWebhook`): только здесь; меняется вместе с токеном бота.
    telegram_webhook_secret: String,
    /// Токен своего бота Discord: только здесь, в state.json и ответах его нет.
    discord_bot_token: String,
    /// Токен бота Slack (xoxb-…): только здесь, в state.json и ответах его нет.
    slack_bot_token: String,
    /// Токен приложения Slack (xapp-…) для Socket Mode: тоже только здесь.
    slack_app_token: String,
}

/// Стартовая команда на двух языках: (имя, роль, характер) по-русски и по-английски.
type Starter = [(&'static str, &'static str, &'static str); 2];

const STARTERS: [Starter; 3] = [
    [
        (
            "Аналитик",
            "разбирает тренды и ищет угол",
            "Ты — аналитик. Твоя работа: разобрать тему, найти неочевидный угол и сказать, что в ней важно.\nТы не пересказываешь новость, а отвечаешь на вопрос «и что с этого?».\nВсегда отделяй факт от гипотезы и прямо называй, где ты не уверен.\nЕсли данных не хватает — скажи, каких именно, и предложи самый дешёвый способ их достать.",
        ),
        (
            "Analyst",
            "digs into trends and finds the angle",
            "You are an analyst. Your job is to take a topic apart, find the non-obvious angle and say what matters in it.\nYou don't retell the news, you answer the question \"so what?\".\nAlways separate fact from guess and say plainly where you are not sure.\nIf data is missing, say exactly what is missing and suggest the cheapest way to get it.\nReply in English unless asked otherwise.",
        ),
    ],
    [
        (
            "Редактор",
            "пишет тексты твоим голосом",
            "Ты — редактор. Ты пишешь так, как пишет автор, а не «как ИИ».\nКороткие абзацы, конкретика, никаких «в современном мире» и «важно отметить».\nОдин текст = одна мысль. Первая строка должна работать как крючок.\nТы сохраняешь черновики в файлы воркспейса через write_file и называешь их понятно.",
        ),
        (
            "Editor",
            "writes in your voice",
            "You are an editor. You write the way the author writes, not the way AI writes.\nShort paragraphs, concrete details, no \"in today's world\" and no \"it's important to note\".\nOne text, one idea. The first line has to work as a hook.\nYou save drafts to workspace files with write_file and give them clear names.\nReply in English unless asked otherwise.",
        ),
    ],
    [
        (
            "Инженер",
            "код, файлы, проверки",
            "Ты — инженер. Ты работаешь в своём воркспейсе: читаешь файлы, пишешь код, запускаешь команды.\nПрежде чем запускать команду, объясни одной строкой, что она делает и зачем.\nНе выдумывай вывод команд: если не запускал — так и скажи.\nПосле правки файла коротко перечисли, что изменил.",
        ),
        (
            "Engineer",
            "code, files, checks",
            "You are an engineer. You work in your workspace: you read files, write code and run commands.\nBefore running a command, explain in one line what it does and why.\nNever make up command output: if you didn't run it, say so.\nAfter editing a file, briefly list what you changed.\nReply in English unless asked otherwise.",
        ),
    ],
];

pub const STAGE_INTERVIEW: &str = "interview";
pub const STAGE_READY: &str = "ready";
/// Шар «Лава» — внешность оркестратора.
pub const ORCHESTRATOR_SKIN: i64 = 13;

/// Бот-оркестратор на двух языках: расспрашивает человека и собирает минимальную команду под задачу.
const ORCHESTRATOR: Starter = [
    (
        "Оркестратор",
        "собирает команду под задачу",
        "Ты — оркестратор. Человек приходит с задачей, а ты собираешь под неё МИНИМАЛЬНУЮ команду ботов. Один бот, если хватает одного. Больше трёх — только если можешь обосновать каждого отдельно.\n\
\n\
Как ты ведёшь разговор:\n\
- Расспрашивай по одному вопросу за раз через инструмент ask_user. К каждому вопросу дай 2–4 готовых варианта ответа и пометь лучший словами «(рекомендую)». Человек может ответить своими словами. Карточка и есть вопрос: не повторяй его текстом и не пиши «жду ответа».\n\
- Не задавай вопрос, ответ на который уже есть в разговоре. Не устраивай допрос: когда картина ясна, переходи к предложению. Если человек просит «хватит, предлагай» — предлагай сразу.\n\
- Пока идёт расспрос, выясни: (1) какой результат нужен и как понять, что он готов; (2) какие входы есть — файлы, сайты, аккаунты, данные; (3) как часто это нужно — разово или по расписанию; (4) ограничения и чего делать нельзя; (5) кто проверяет результат и в какой момент.\n\
- Говори простыми словами, без жаргона. Не рассказывай человеку про инструменты и устройство программы, пока он сам не спросил.\n\
\n\
Как ты собираешь состав:\n\
- Сначала загляни в каталог готовых ботов инструментом search_catalog: подходящего готового бота лучше взять, чем придумывать нового (поле catalogBots). Новых описывай в newBots: имя, короткая роль и инструкции на несколько строк — что делает, чего не делает, как отчитывается.\n\
- Ведущий команды — ты сам: поле lead не заполняй. Правила, общие для всей команды, запиши в instructions.\n\
- Ничего не создавай сам и не обещай, что «уже создал»: состав появляется только после того, как человек нажмёт «Создать» на карточке.\n\
\n\
Как ты завершаешь расспрос:\n\
- Итог — ровно один вызов propose_team: роли, правила команды и коротко, почему именно такой состав. Не спрашивай «создать?» перед карточкой и не пересказывай состав текстом до неё: решает карточка.\n\
- Если человек отказался или попросил поправить — учти и предложи заново, но не повторяй то же без новой причины.\n\
\n\
После того как команда создана: коротко скажи, что получилось и с чего начать, и дальше работай ведущим — принимай задачи, отдавай их нужным ботам инструментом assign_task и собирай итог. Если человеку нужен другой состав, снова расспроси его и предложи новый через propose_team.",
    ),
    (
        "Orchestrator",
        "builds a team for the task",
        "You are the orchestrator. A person comes with a task and you assemble the MINIMAL team of bots for it. One bot if one is enough. More than three only if you can justify each one separately.\n\
\n\
How you run the conversation:\n\
- Ask one question at a time with the ask_user tool. Give each question 2-4 ready-made options and mark the best one with \"(recommended)\". The person may answer in their own words. The card is the question: do not repeat it in text and do not write \"waiting for your answer\".\n\
- Never ask what the conversation already answers. Don't interrogate: once the picture is clear, move on to the proposal. If the person says \"enough, propose\", propose right away.\n\
- During the interview find out: (1) what result is needed and how we will know it is done; (2) what inputs exist - files, sites, accounts, data; (3) how often it is needed - once or on a schedule; (4) limits and what must not be done; (5) who checks the result and when.\n\
- Use plain words, no jargon. Don't explain tools or how the program works unless asked.\n\
\n\
How you pick the team:\n\
- First look through the catalog of ready-made bots with search_catalog: a suitable ready bot beats inventing a new one (field catalogBots). Describe new ones in newBots: a name, a short role and a few lines of instructions - what it does, what it does not do, how it reports.\n\
- You are the team lead yourself: leave the lead field empty. Put rules shared by the whole team into instructions.\n\
- Never create anything yourself and never claim you already did: the team appears only after the person presses \"Create\" on the card.\n\
\n\
How you finish the interview:\n\
- The result is exactly one propose_team call: roles, team rules and a short reason for this exact lineup. Don't ask \"create it?\" before the card and don't retell the lineup in text first: the card decides.\n\
- If the person declines or asks for changes, take it into account and propose again, but don't repeat the same without a new reason.\n\
\n\
Once the team is created: say briefly what you got and where to start, then work as the lead - take tasks, hand them to the right bots with the assign_task tool and gather the result. If the person needs a different lineup, interview them again and propose a new one with propose_team.\n\
Reply in English unless asked otherwise.",
    ),
];

/// Бот-оркестратор. Драйвер и модель пустые: он берёт настройки программы (подписка Deloist или своя ИИ), как стартовые боты.
fn new_orchestrator(taken: &[Bot]) -> Bot {
    let (name, role, persona) = ORCHESTRATOR[0];
    let mut unique = name.to_string();
    let mut n = 2;
    while taken.iter().any(|b| b.name.eq_ignore_ascii_case(&unique)) {
        unique = format!("{name} {n}");
        n += 1;
    }
    Bot {
        id: uid("bot"),
        name: unique,
        role: role.into(),
        color: "#e8643c".into(),
        persona: persona.into(),
        pinned: false, // виден под своей командой; закреплённый дублировал бы его в боковой панели
        created_at: now(),
        skin: ORCHESTRATOR_SKIN,
        orchestrator: true,
        ..Bot::default()
    }
}

/// Название первой команды новой установки.
const MAIN_ROOM: [&str; 2] = ["Главная команда", "Main team"];

/// Смена языка переводит только нетронутых стартовых ботов: чужие правки важнее языка.
fn translate_starters(bots: &mut [Bot], lang: &str) {
    let (from, to) = if lang == "en" { (0, 1) } else { (1, 0) };
    for bot in bots.iter_mut() {
        let found = STARTERS.iter().chain(std::iter::once(&ORCHESTRATOR)).find(|starter| {
            let (name, role, persona) = starter[from];
            bot.name == name && bot.role == role && bot.persona == persona
        });
        if let Some(starter) = found {
            let (name, role, persona) = starter[to];
            bot.name = name.into();
            bot.role = role.into();
            bot.persona = persona.into();
        }
    }
}

fn seed() -> State {
    // Для проверок и старых сценариев: прежний набор стартовых ботов вместо оркестратора.
    if std::env::var("CREW_LEGACY_STARTERS").is_ok_and(|v| v == "1") {
        return seed_starters();
    }
    let at = now();
    let bot = new_orchestrator(&[]);
    let room = Room {
        id: uid("room"),
        name: MAIN_ROOM[0].into(),
        bot_ids: vec![bot.id.clone()],
        lead: bot.id.clone(),
        stage: STAGE_INTERVIEW.into(),
        created_at: at,
        ..Room::default()
    };
    State { version: 1, created_at: at, settings: Settings::default(), bots: vec![bot], routines: Vec::new(), skills: Vec::new(), rooms: vec![room], mcp: Vec::new() }
}

/// Прежняя новая установка: три стартовых бота без команды.
fn seed_starters() -> State {
    let at = now();
    let colors = ["#4f9ce6", "#8e6ac9", "#f2a93b"];
    State {
        version: 1,
        created_at: at,
        settings: Settings::default(),
        bots: STARTERS
            .iter()
            .enumerate()
            .map(|(i, starter)| Bot {
                id: uid("bot"),
                name: starter[0].0.into(),
                role: starter[0].1.into(),
                color: colors[i].into(),
                persona: starter[0].2.into(),
                pinned: i == 0,
                created_at: at,
                skin: [0, 5, 10][i],
                ..Bot::default()
            })
            .collect(),
        routines: Vec::new(),
        skills: Vec::new(),
        rooms: Vec::new(),
        mcp: Vec::new(),
    }
}

// ---------------------------------------------------------------------------
// Приведение значений: API принимает JSON от интерфейса и ведёт себя как раньше
// ---------------------------------------------------------------------------

fn as_string(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        Value::Null => String::new(),
        other => other.to_string(),
    }
}

/// Последнего питомца убрали: выключаем режим совсем. Так «включено» всегда значит, что кто-то показан,
/// и следующее включение выберет бота заново, а не оставит пустоту.
fn calm_pets(state: &mut State) {
    if !state.bots.iter().any(|bot| bot.pet) {
        state.settings.pets_mode = "off".into();
    }
}

pub(crate) fn truthy(v: &Value) -> bool {
    match v {
        Value::Null => false,
        Value::Bool(b) => *b,
        Value::Number(n) => n.as_f64().is_some_and(|x| x != 0.0 && !x.is_nan()),
        Value::String(s) => !s.is_empty(),
        _ => true,
    }
}

fn text_or(input: &Value, key: &str, default: &str) -> String {
    match input.get(key) {
        Some(v) if truthy(v) => as_string(v),
        _ => default.to_string(),
    }
}

fn take_chars(s: &str, n: usize) -> String {
    s.chars().take(n).collect()
}

/// Сколько стеклянных шаров в web/avatar.js (ORBS) и в атласе web/orbs.webp.
pub const ORB_COUNT: i64 = 20;
const MAX_SKIN: i64 = ORB_COUNT - 1;
/// Сколько специалистов-гостей помнит тред: старые вытесняются.
pub const MAX_GUESTS: usize = 8;
const DAY_MS: i64 = 24 * 60 * 60 * 1000;

pub const BOT_RANKS: [&str; 2] = ["lead", "specialist"];

/// Проверка полей бота, которые пришли в PATCH: неизвестное значение — 400, а не тихая замена.
pub fn check_bot_patch(patch: &Value) -> Result<(), String> {
    if let Some(v) = patch.get("rank").filter(|v| !v.is_null()) {
        if !v.as_str().is_some_and(|rank| BOT_RANKS.contains(&rank)) {
            return Err("rank: только lead или specialist".into());
        }
    }
    if let Some(v) = patch.get("pinned").filter(|v| !v.is_null()) {
        if !v.is_boolean() {
            return Err("pinned: нужно true или false".into());
        }
    }
    if let Some(v) = patch.get("team").filter(|v| !v.is_null()) {
        let Some(team) = v.as_str() else {
            return Err("team: нужна строка до 40 символов".into());
        };
        if team.chars().count() > 40 {
            return Err("team: не длиннее 40 символов".into());
        }
    }
    if let Some(v) = patch.get("reserveAfterDays").filter(|v| !v.is_null()) {
        if !v.as_i64().is_some_and(|days| days >= 0) {
            return Err("reserveAfterDays: целое число дней, 0 — никогда".into());
        }
    }
    if let Some(v) = patch.get("monthlyTokenLimit").filter(|v| !v.is_null()) {
        if !v.as_i64().is_some_and(|n| (0..=1_000_000_000_000).contains(&n)) {
            return Err("monthlyTokenLimit: целое число токенов, 0 или пусто — без предела".into());
        }
    }
    if let Some(v) = patch.get("alwaysAsk").filter(|v| !v.is_null()) {
        let known = v.as_array().is_some_and(|list| list.iter().all(|k| k.as_str().is_some_and(|k| crate::tools::ALWAYS_ASK_KINDS.iter().any(|(id, _)| *id == k))));
        if !known {
            return Err("alwaysAsk: список из command, send, publish, pay, delete, files, bots".into());
        }
    }
    if let Some(v) = patch.get("removeGuest").filter(|v| !v.is_null()) {
        if !v.as_str().is_some_and(|id| !id.trim().is_empty()) {
            return Err("removeGuest: нужен id бота".into());
        }
    }
    Ok(())
}

/// Давно ли бот бездействовал: `days == 0` значит «никогда не уходит в резерв».
fn inactive_for_reserve(days: i64, last_active_at: i64, at: i64) -> bool {
    days > 0 && last_active_at > 0 && at.saturating_sub(last_active_at) > days.saturating_mul(DAY_MS)
}

/// Гости треда лежат в `extra.guests` вызывающего бота: `[{ botId, since }]`.
pub fn bot_guests(bot: &Bot) -> Vec<Value> {
    bot.extra
        .get("guests")
        .and_then(Value::as_array)
        .map(|list| {
            list.iter()
                .filter(|guest| guest.get("botId").and_then(Value::as_str).is_some_and(|id| !id.is_empty()))
                .take(MAX_GUESTS)
                .cloned()
                .collect()
        })
        .unwrap_or_default()
}

fn set_bot_guests(bot: &mut Bot, guests: Vec<Value>) {
    if guests.is_empty() {
        bot.extra.remove("guests");
    } else {
        bot.extra.insert("guests".into(), Value::Array(guests));
    }
}

/// Откуда считать шар боту без шара: от его id, один и тот же при каждом запуске.
fn skin_start(id: &str) -> usize {
    let hash = id.bytes().fold(0xcbf2_9ce4_8422_2325_u64, |h, b| (h ^ u64::from(b)).wrapping_mul(0x0100_0000_01b3));
    (hash % ORB_COUNT as u64) as usize
}

/// Шар, который у команды сейчас встречается реже всех; при равенстве — первый от `start`.
fn free_skin(bots: &[Bot], start: usize) -> i64 {
    let mut used = [0usize; ORB_COUNT as usize];
    for bot in bots {
        if (0..ORB_COUNT).contains(&bot.skin) {
            used[bot.skin as usize] += 1;
        }
    }
    let least = used.iter().copied().min().unwrap_or(0);
    (0..ORB_COUNT as usize).map(|k| (start + k) % ORB_COUNT as usize).find(|&i| used[i] == least).unwrap_or(0) as i64
}

/// Старые боты (с формой, ушами и материалом вместо шара) получают шар по id; прежние поля остаются в файле нетронутыми.
fn assign_skins(bots: &mut [Bot]) -> bool {
    let mut changed = false;
    for i in 0..bots.len() {
        if !(0..ORB_COUNT).contains(&bots[i].skin) {
            let skin = free_skin(bots, skin_start(&bots[i].id));
            bots[i].skin = skin;
            changed = true;
        }
    }
    changed
}

/// Номер варианта внешности в допустимых границах.
fn look_index(v: Option<&Value>, max: i64) -> Option<i64> {
    number(v).map(|x| (x as i64).clamp(0, max))
}

fn number(v: Option<&Value>) -> Option<f64> {
    match v? {
        Value::Number(n) => n.as_f64(),
        Value::String(s) => s.trim().parse().ok(),
        _ => None,
    }
}

/// Политика подтверждений бота. Неизвестное значение — как раньше, «спрашивать».
fn approval_mode(v: Option<&Value>) -> String {
    match v.and_then(Value::as_str) {
        Some("auto") => "auto".into(),
        Some("strict") => "strict".into(),
        _ => "ask".into(),
    }
}

fn string_list(v: Option<&Value>) -> Vec<String> {
    let mut out: Vec<String> = v
        .and_then(Value::as_array)
        .map(|items| items.iter().filter_map(Value::as_str).filter(|s| !s.trim().is_empty()).map(str::to_string).collect())
        .unwrap_or_default();
    out.dedup();
    out.truncate(64);
    out
}

/// Объект строк `{имя: значение}` из запроса: env и заголовки MCP-сервера.
pub(crate) fn string_map(v: Option<&Value>) -> BTreeMap<String, String> {
    v.and_then(Value::as_object)
        .map(|map| {
            map.iter()
                .filter_map(|(key, value)| value.as_str().map(|text| (key.trim().to_string(), text.to_string())))
                .filter(|(key, _)| !key.is_empty())
                .take(64)
                .collect()
        })
        .unwrap_or_default()
}

/// Поля расписания из запроса. Формат времени проверяет сервер, пустая строка снимает значение.
fn apply_schedule(routine: &mut Routine, input: &Value) -> Result<(), String> {
    if let Some(max) = input.get("everyMaxMinutes") {
        routine.every_max_minutes = number(Some(max)).filter(|x| x.is_finite() && *x > 0.0).map_or(0, |x| x as i64);
    }
    if let Some(at) = input.get("at").and_then(Value::as_str) {
        routine.at = at.trim().to_string();
    }
    if let Some(days) = input.get("days").and_then(Value::as_array) {
        let mut wanted: Vec<i64> = days.iter().filter_map(Value::as_i64).filter(|d| (1..=7).contains(d)).collect();
        wanted.sort_unstable();
        wanted.dedup();
        routine.days = wanted;
    }
    if let Some(from) = input.get("windowFrom").and_then(Value::as_str) {
        routine.window_from = from.trim().to_string();
    }
    if let Some(to) = input.get("windowTo").and_then(Value::as_str) {
        routine.window_to = to.trim().to_string();
    }
    if let Some(v) = input.get("recurrence") {
        if v.is_null() {
            routine.recurrence = None;
        } else if let Ok(rec) = serde_json::from_value(v.clone()) {
            routine.recurrence = Some(rec);
        }
    }
    if let Some(v) = input.get("startOn").or_else(|| input.get("start_on")) {
        routine.start_on = v.as_str().unwrap_or_default().trim().to_string();
    }
    if let Some(v) = input.get("onceAt").or_else(|| input.get("once_at")) {
        routine.once_at = v.as_i64().unwrap_or(0);
    }
    if let Some(v) = input.get("cron") {
        let s = v.as_str().ok_or("поле cron должно быть строкой")?.trim().to_string();
        if !s.is_empty() {
            crate::schedule::parse_cron(&s).map_err(|e| format!("поле cron: {e}"))?;
            if input.get("hook").is_some_and(truthy) {
                return Err("cron несовместим с вебхуком".into());
            }
        }
        routine.cron = s;
    }
    if let Some(v) = input.get("tz") {
        let s = v.as_str().ok_or("поле tz должно быть строкой")?.trim().to_string();
        if !s.is_empty() && !crate::schedule::tz_valid(&s) {
            return Err("поле tz: неизвестный часовой пояс".into());
        }
        routine.tz = s;
    }
    if !routine.cron.is_empty() && routine.hook {
        return Err("cron несовместим с вебхуком".into());
    }
    if !routine.cron.is_empty() && routine.once_at > 0 {
        return Err("cron несовместим с разовым запуском (onceAt)".into());
    }
    Ok(())
}

fn minutes(v: Option<&Value>) -> i64 {
    let raw = number(v).filter(|x| *x != 0.0 && x.is_finite()).unwrap_or(60.0);
    (raw as i64).max(5)
}

/// Windows по-особому читает двоеточия (потоки файла), хвостовые точки и пробелы («.. » это «..»)
/// и имена устройств. В воркспейсе такие имена не нужны.
pub(crate) fn plain_file_name(name: &str) -> bool {
    if name.contains(':') || name.ends_with('.') || name.ends_with(' ') {
        return false;
    }
    let stem = name.split('.').next().unwrap_or_default().to_uppercase();
    let device = matches!(stem.as_str(), "CON" | "PRN" | "AUX" | "NUL")
        || (stem.len() == 4
            && (stem.starts_with("COM") || stem.starts_with("LPT"))
            && stem.as_bytes()[3].is_ascii_digit());
    !device
}

// ---------------------------------------------------------------------------
// Хранилище
// ---------------------------------------------------------------------------

pub struct Store {
    data_dir: PathBuf,
    state: Mutex<State>,
    secrets: Mutex<Secrets>,
    env: Mutex<HashMap<String, String>>,
    /// Записи state.json идут строго по очереди, иначе старый снимок мог бы затереть новый.
    write_gate: tokio::sync::Mutex<()>,
    /// То же для `secrets.json`.
    secrets_gate: tokio::sync::Mutex<()>,
    /// `secrets.json` не удалось прочитать: в памяти пусто, запись ключей отказывает,
    /// пока файл не прочитается при следующем запуске.
    secrets_unavailable: std::sync::atomic::AtomicBool,
    /// Команда → состав, для которого общая папка уже собрана (`build_team_folders`).
    team_built: Mutex<HashMap<String, Vec<String>>>,
}

/// Временный файл рядом и замена: читатель видит либо старый файл, либо новый целиком.
async fn write_replace(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    let tmp = path.with_extension("json.tmp");
    let mut options = tokio::fs::OpenOptions::new();
    options.write(true).create(true).truncate(true);
    #[cfg(unix)]
    options.mode(0o600);
    let mut file = options.open(&tmp).await?;
    file.write_all(bytes).await?;
    file.flush().await?;
    drop(file);
    tokio::fs::rename(&tmp, path).await
}

/// Отложить нечитаемый `secrets.json` в сторону, а не перезаписать: его ещё можно вернуть
/// (например, файл зашифровал другой пользователь Windows). В журнал идёт только причина.
fn set_aside(path: &Path, why: &str) {
    // Вне Windows открытый остаток прежней версии (без шифрования) затираем нулями, а не храним.
    #[cfg(not(windows))]
    if std::fs::read(path).is_ok_and(|bytes| crate::secrets::is_plaintext_leftover(&bytes)) {
        eprintln!("[secrets] secrets.json открытый и не принят ({why}); затёрт: {}", crate::secrets::shred_file(path));
        return;
    }
    let aside = path.with_extension(format!("json.bad-{}", now()));
    let moved = std::fs::rename(path, &aside).is_ok();
    eprintln!("[secrets] secrets.json не прочитан ({why}); отложен: {moved}");
    if let Some(dir) = path.parent() {
        prune_aside(dir, 3);
    }
}

/// Оставить только `keep` самых новых `secrets.json.bad-*`.
fn prune_aside(dir: &Path, keep: usize) {
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    let mut found: Vec<(u64, PathBuf)> = entries
        .flatten()
        .filter_map(|e| {
            let name = e.file_name().to_string_lossy().to_string();
            let stamp = name.strip_prefix("secrets.json.bad-")?.parse::<u64>().ok()?;
            Some((stamp, e.path()))
        })
        .collect();
    found.sort_by(|a, b| b.0.cmp(&a.0));
    for (_, old) in found.into_iter().skip(keep) {
        match std::fs::remove_file(&old) {
            Ok(()) => eprintln!("[secrets] старый отложенный файл удалён"),
            Err(e) => eprintln!("[secrets] старый отложенный файл не удалён ({})", e.kind()),
        }
    }
}

/// Убрать копию `secrets.json.migrating`; неудача попадает в журнал, повтор — при следующей записи.
fn remove_migrating(data_dir: &Path) {
    // Вне Windows копия открытая: затираем нулями, потом удаляем.
    #[cfg(not(windows))]
    {
        let copy = data_dir.join("secrets.json.migrating");
        if copy.exists() && !crate::secrets::shred_file(&copy) {
            eprintln!("[secrets] копия secrets.json.migrating не удалена; повторю при следующей записи");
        }
        return;
    }
    #[cfg(windows)]
    match std::fs::remove_file(data_dir.join("secrets.json.migrating")) {
        Ok(()) => {}
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => eprintln!("[secrets] копия secrets.json.migrating не удалена ({}); повторю при следующей записи", e.kind()),
    }
}

/// Прочитать файл: «нет файла» — `Ok(None)`; любая другая ошибка (чужая блокировка, антивирус) —
/// три попытки с короткой паузой, потом ошибка.
fn read_secrets_file(path: &Path) -> Result<Option<Vec<u8>>, std::io::ErrorKind> {
    let mut last = std::io::ErrorKind::Other;
    for attempt in 0..3 {
        match std::fs::read(path) {
            Ok(bytes) => return Ok(Some(bytes)),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(e) => last = e.kind(),
        }
        if attempt < 2 {
            std::thread::sleep(std::time::Duration::from_millis(100));
        }
    }
    Err(last)
}

/// Прочитать `secrets.json`. Второе значение — «ключи недоступны»: файл есть, но не читается;
/// тогда в памяти пусто, а запись `persist_secrets` отказывает, чтобы не затереть хороший файл.
/// Старый открытый файл перешифровывается при первом чтении:
/// 1) копия открытого файла `secrets.json.migrating`; 2) зашифрованный файл пишется рядом и
/// проверяется расшифровкой; 3) подменяет открытый; 4) проверка с диска и копия удаляется.
/// Сбой на любом шаге оставляет открытый файл как был (до следующего запуска), а оборванный
/// посреди запуск восстанавливается из копии. Исходные байты остаются в `secrets.json.v1`
/// (для возврата на прежнюю версию). В журнал попадает только причина, не значения.
fn load_secrets(data_dir: &Path) -> (Secrets, bool) {
    use crate::secrets::{open_file, FileContent};
    prune_aside(data_dir, 3);
    let path = data_dir.join("secrets.json");
    let backup = data_dir.join("secrets.json.migrating");
    let from_backup = || {
        let bytes = std::fs::read(&backup).ok()?;
        match open_file(&bytes) {
            Ok(FileContent::Plain(plain)) => Some((plain, bytes)),
            _ => None,
        }
    };
    let (plain, original) = match read_secrets_file(&path) {
        Ok(Some(bytes)) => match open_file(&bytes) {
            Ok(FileContent::Sealed(plain)) => {
                // Прошлый запуск зашифровал файл, но не успел убрать копию.
                remove_migrating(data_dir);
                return (
                    serde_json::from_slice(&plain).unwrap_or_else(|_| {
                        set_aside(&path, "внутри не JSON");
                        Secrets::default()
                    }),
                    false,
                );
            }
            Ok(FileContent::Plain(plain)) => (plain, bytes),
            Err(why) => match from_backup() {
                Some(found) => found,
                None => {
                    // Зашифрованный файл не открылся (нет ключа, чужой пользователь Windows, другой
                    // ключ): он может ещё пригодиться, поэтому ничего не откладываем и не удаляем.
                    // Откладывает его только человек кнопкой «Начать с пустыми ключами».
                    if bytes.starts_with(crate::secrets::SEALED_MAGIC) {
                        eprintln!("[secrets] secrets.json зашифрован и не открывается ({why}); файл не трогаю, запись ключей закрыта");
                        return (Secrets::default(), true);
                    }
                    set_aside(&path, &why);
                    return (Secrets::default(), false);
                }
            },
        },
        Ok(None) => match from_backup() {
            Some(found) => found,
            None => return (Secrets::default(), false),
        },
        Err(kind) => {
            eprintln!("[secrets] secrets.json не читается ({kind}); ключи недоступны, файл не трогаю");
            return (Secrets::default(), true);
        }
    };
    let Ok(secrets) = serde_json::from_slice::<Secrets>(&plain) else {
        set_aside(&path, "испорчен JSON");
        remove_migrating(data_dir);
        return (Secrets::default(), false);
    };
    if let Err(why) = migrate_secrets(&path, &backup, &original, &secrets) {
        eprintln!("[secrets] secrets.json пока не зашифрован ({why}); попробую при следующей записи");
    }
    (secrets, false)
}

/// `original` — исходные байты открытого файла: в `.migrating` и `.v1` уходят они, а не
/// заново собранные `Secrets`, чтобы неизвестные поля не терялись.
fn migrate_secrets(path: &Path, backup: &Path, original: &[u8], secrets: &Secrets) -> Result<(), String> {
    use crate::secrets::{open_file, seal_file, FileContent};
    let plain = original;
    let wanted = serde_json::to_vec(secrets).map_err(|e| e.to_string())?;
    let sealed = seal_file(&wanted)?;
    let reads_back = |bytes: &[u8]| matches!(open_file(bytes), Ok(FileContent::Sealed(p)) if p == wanted);
    if !reads_back(&sealed) {
        return Err("зашифрованное не расшифровалось обратно".into());
    }
    // Копия для возврата на 0.3.4; существующую не затираем.
    let v1 = path.with_extension("json.v1");
    // Вне Windows открытая копия `.v1` не нужна: прежняя версия там не шифровала вовсе.
    if cfg!(windows) && !v1.exists() {
        std::fs::write(&v1, plain).map_err(|e| format!("копия v1: {}", e.kind()))?;
    }
    std::fs::write(backup, plain).map_err(|e| format!("копия: {e}"))?;
    let tmp = path.with_extension("json.tmp");
    let done = std::fs::write(&tmp, &sealed)
        .and_then(|()| std::fs::rename(&tmp, path))
        .map_err(|e| format!("запись: {e}"))
        .and_then(|()| match std::fs::read(path) {
            Ok(bytes) if reads_back(&bytes) => Ok(()),
            _ => Err("проверка с диска не прошла".to_string()),
        });
    match done {
        Ok(()) => {
            if let Some(dir) = path.parent() {
                remove_migrating(dir);
            }
            Ok(())
        }
        Err(why) => {
            // Вернуть открытый файл из копии: так ключи не пропадут.
            let _ = std::fs::remove_file(&tmp);
            if std::fs::rename(backup, path).is_err() {
                let _ = std::fs::write(path, plain);
                let _ = std::fs::remove_file(backup);
            }
            Err(why)
        }
    }
}

/// Если ведущий команды больше не входит в её состав (например, его убрали) — назначить первого участника.
/// Тот же код, что использует `update_room`, чтобы логика не расходилась.
fn fix_room_lead(room: &mut Room) {
    if !room.bot_ids.contains(&room.lead) {
        room.lead = room.bot_ids.first().cloned().unwrap_or_default();
    }
}

/// Старые расписания переводим на общее правило (at+days -> recurrence, extra.onceAt -> once_at).
/// Возвращает, менялось ли что-то: по этому флагу состояние дописывается на диск.
fn migrate_routines(state: &mut State) -> bool {
    let mut changed = false;
    for r in &mut state.routines {
        if !r.at.is_empty() && r.recurrence.is_none() {
            r.recurrence = Some(if r.days.is_empty() {
                crate::dates_ru::Recurrence {
                    freq: crate::dates_ru::Freq::Daily,
                    interval: 1,
                    weekdays: vec![],
                    monthday: None,
                    nth: None,
                    month: None,
                }
            } else {
                crate::dates_ru::Recurrence {
                    freq: crate::dates_ru::Freq::Weekly,
                    interval: 1,
                    weekdays: r.days.iter().map(|d| *d as u8).collect(),
                    monthday: None,
                    nth: None,
                    month: None,
                }
            });
            r.days.clear();
            changed = true;
        }
        if let Some(ms) = r.extra.get("onceAt").and_then(Value::as_i64) {
            r.once_at = ms;
            r.extra.remove("onceAt");
            changed = true;
        }
    }
    changed
}

impl Store {
    pub async fn open(data_dir: PathBuf) -> std::io::Result<Self> {
        for sub in ["threads", "memory", "workspaces", "tmp", "rooms"] {
            tokio::fs::create_dir_all(data_dir.join(sub)).await?;
        }
        let state_file = data_dir.join("state.json");
        let mut must_write = false;
        let mut state = match tokio::fs::read_to_string(&state_file).await {
            Ok(text) => match serde_json::from_str::<State>(&text) {
                Ok(state) => state,
                Err(_) => {
                    // Битый state не должен убивать запуск: отводим его в сторону и начинаем заново.
                    let aside = data_dir.join(format!("state.json.broken-{}", now()));
                    tokio::fs::rename(&state_file, aside).await?;
                    must_write = true;
                    seed()
                }
            },
            Err(_) => {
                must_write = true;
                seed()
            }
        };
        if state.version == 0 {
            state.version = 1;
        }
        if migrate_routines(&mut state) {
            must_write = true;
        }
        // Модель уже настроена руками до появления экрана «Собери команду»: человеку он не нужен.
        if !state.settings.team_onboarded && state.settings.driver != "demo" {
            state.settings.team_onboarded = true;
            must_write = true;
        }
        if assign_skins(&mut state.bots) {
            must_write = true;
        }
        // До выбора по одному питомцы показывали первых шестерых: включённое раньше не должно пропасть.
        if state.settings.pets_mode != "off" && !state.bots.iter().any(|bot| bot.pet) {
            for bot in state.bots.iter_mut().take(6) {
                bot.pet = true;
            }
        }

        crate::secrets::note_data_dir(&data_dir);
        let (secrets, secrets_unavailable) = load_secrets(&data_dir);

        for bot in &state.bots {
            tokio::fs::create_dir_all(data_dir.join("workspaces").join(&bot.id)).await?;
        }

        let store = Self {
            data_dir,
            state: Mutex::new(state),
            secrets: Mutex::new(secrets),
            env: Mutex::new(HashMap::new()),
            write_gate: tokio::sync::Mutex::new(()),
            secrets_gate: tokio::sync::Mutex::new(()),
            secrets_unavailable: std::sync::atomic::AtomicBool::new(secrets_unavailable),
            team_built: Mutex::new(HashMap::new()),
        };
        if must_write {
            store.persist().await?;
        }
        store.migrate_one_bot_one_team().await;
        store.build_team_folders().await;
        Ok(store)
    }

    pub fn data_dir(&self) -> &Path {
        &self.data_dir
    }

    /// Временные файлы живут в папке данных: системный Temp бывает закрыт для записи,
    /// например у процессов с пониженным уровнем целостности.
    pub fn tmp_dir(&self) -> PathBuf {
        self.data_dir.join("tmp")
    }

    /// Значения из `.env`: переменные процесса важнее файла.
    pub fn set_env_file(&self, values: HashMap<String, String>) {
        *self.env.lock().unwrap() = values;
    }

    fn lookup_env(&self, key: &str) -> Option<String> {
        std::env::var(key)
            .ok()
            .or_else(|| self.env.lock().unwrap().get(key).cloned())
            .filter(|v| !v.is_empty())
    }

    async fn persist(&self) -> std::io::Result<()> {
        {
            let _gate = self.write_gate.lock().await;
            let text = {
                let state = self.state.lock().unwrap();
                serde_json::to_string_pretty(&*state).map_err(std::io::Error::other)?
            };
            let target = self.data_dir.join("state.json");
            let tmp = self.data_dir.join("state.json.tmp");
            tokio::fs::write(&tmp, text).await?;
            tokio::fs::rename(&tmp, &target).await?;
        }
        // Сборка общих папок идёт уже без замка записи: другие записи состояния не ждут копирования файлов.
        self.build_team_folders().await;
        Ok(())
    }

    /// `secrets.json` пишется только зашифрованным (DPAPI текущего пользователя Windows) и целиком
    /// через временный файл: оборванная запись не теряет прежние ключи.
    async fn persist_secrets(&self) -> std::io::Result<()> {
        let _gate = self.secrets_gate.lock().await;
        if self.secrets_unavailable.load(std::sync::atomic::Ordering::SeqCst) {
            eprintln!("[secrets] запись отказана: secrets.json не был прочитан при запуске");
            return Err(std::io::Error::other(
                "Ключи сейчас недоступны: файл secrets.json не прочитался. Перезапусти Deloist, прежний файл цел",
            ));
        }
        let plain = {
            let secrets = self.secrets.lock().unwrap();
            serde_json::to_vec(&*secrets).map_err(std::io::Error::other)?
        };
        let sealed = crate::secrets::seal_file(&plain).map_err(std::io::Error::other)?;
        let path = self.data_dir.join("secrets.json");
        write_replace(&path, &sealed).await?;
        remove_migrating(&self.data_dir);
        Ok(())
    }

    /// Ключи не прочитались при запуске: запись закрыта, файл на диске цел.
    pub fn secrets_unavailable(&self) -> bool {
        self.secrets_unavailable.load(std::sync::atomic::Ordering::SeqCst)
    }

    /// Единственный путь выхода из «ключи недоступны»: нечитаемый `secrets.json` откладывается в
    /// `secrets.json.bad-reset-N` (имя, которое чистка старых копий не трогает), запись открывается.
    /// Вызывает только человек (`guard::human_only`). Возвращает, было ли что откладывать.
    pub async fn reset_unavailable_secrets(&self) -> std::io::Result<bool> {
        let _gate = self.secrets_gate.lock().await;
        if !self.secrets_unavailable() {
            return Ok(false);
        }
        let path = self.data_dir.join("secrets.json");
        let mut moved = false;
        if std::fs::symlink_metadata(&path).is_ok() {
            let aside = path.with_extension(format!("json.bad-reset-{}", now()));
            std::fs::rename(&path, &aside)?;
            moved = true;
            eprintln!("[secrets] по просьбе человека нечитаемый secrets.json отложен; ключи начаты заново");
        }
        self.secrets_unavailable.store(false, std::sync::atomic::Ordering::SeqCst);
        Ok(moved)
    }

    // --- настройки ---------------------------------------------------------

    pub fn settings(&self) -> Settings {
        self.state.lock().unwrap().settings.clone()
    }

    pub fn telegram(&self) -> TelegramSettings {
        self.state.lock().unwrap().settings.telegram.clone()
    }

    /// Меняет только поля моста: остальные настройки не перезаписываются.
    pub async fn update_telegram(&self, change: impl FnOnce(&mut TelegramSettings)) -> std::io::Result<TelegramSettings> {
        let telegram = {
            let mut state = self.state.lock().unwrap();
            change(&mut state.settings.telegram);
            state.settings.telegram.clone()
        };
        self.persist().await?;
        Ok(telegram)
    }

    /// Токен живёт только в `secrets.json`.
    pub fn telegram_token(&self) -> String {
        self.secrets.lock().unwrap().telegram_bot_token.clone()
    }

    /// Пустая строка забывает токен.
    pub async fn set_telegram_token(&self, token: &str) -> std::io::Result<()> {
        {
            let mut secrets = self.secrets.lock().unwrap();
            secrets.telegram_bot_token = token.trim().to_string();
            // Новый токен — новый секрет вебхука; пустой токен секрет забывает.
            secrets.telegram_webhook_secret = if secrets.telegram_bot_token.is_empty() { String::new() } else { crate::guard::random_token() };
        }
        self.persist_secrets().await
    }

    /// Секрет вебхука Telegram; пустая строка — ещё не создан.
    pub fn telegram_webhook_secret(&self) -> String {
        self.secrets.lock().unwrap().telegram_webhook_secret.clone()
    }

    /// Секрет вебхука: есть — отдаёт, нет (старые данные) — создаёт и сохраняет.
    /// Проверка и создание — в одной секции замка, поэтому два вызова не получат разные секреты.
    pub async fn ensure_telegram_webhook_secret(&self) -> std::io::Result<String> {
        let (secret, created) = {
            let mut secrets = self.secrets.lock().unwrap();
            if secrets.telegram_webhook_secret.is_empty() {
                secrets.telegram_webhook_secret = crate::guard::random_token();
                (secrets.telegram_webhook_secret.clone(), true)
            } else {
                (secrets.telegram_webhook_secret.clone(), false)
            }
        };
        if created {
            self.persist_secrets().await?;
        }
        Ok(secret)
    }

    pub fn discord(&self) -> DiscordSettings {
        self.state.lock().unwrap().settings.discord.clone()
    }

    /// Меняет только поля моста: остальные настройки не перезаписываются.
    pub async fn update_discord(&self, change: impl FnOnce(&mut DiscordSettings)) -> std::io::Result<DiscordSettings> {
        let discord = {
            let mut state = self.state.lock().unwrap();
            change(&mut state.settings.discord);
            state.settings.discord.clone()
        };
        self.persist().await?;
        Ok(discord)
    }

    /// Токен живёт только в `secrets.json`.
    pub fn discord_token(&self) -> String {
        self.secrets.lock().unwrap().discord_bot_token.clone()
    }

    /// Пустая строка забывает токен.
    pub async fn set_discord_token(&self, token: &str) -> std::io::Result<()> {
        self.secrets.lock().unwrap().discord_bot_token = token.trim().to_string();
        self.persist_secrets().await
    }

    pub fn slack(&self) -> SlackSettings {
        self.state.lock().unwrap().settings.slack.clone()
    }

    /// Меняет только поля моста: остальные настройки не перезаписываются.
    pub async fn update_slack(&self, change: impl FnOnce(&mut SlackSettings)) -> std::io::Result<SlackSettings> {
        let slack = {
            let mut state = self.state.lock().unwrap();
            change(&mut state.settings.slack);
            state.settings.slack.clone()
        };
        self.persist().await?;
        Ok(slack)
    }

    /// Оба токена живут только в `secrets.json`.
    pub fn slack_bot_token(&self) -> String {
        self.secrets.lock().unwrap().slack_bot_token.clone()
    }

    pub fn slack_app_token(&self) -> String {
        self.secrets.lock().unwrap().slack_app_token.clone()
    }

    /// Пустые строки забывают оба токена.
    pub async fn set_slack_tokens(&self, bot_token: &str, app_token: &str) -> std::io::Result<()> {
        {
            let mut secrets = self.secrets.lock().unwrap();
            secrets.slack_bot_token = bot_token.trim().to_string();
            secrets.slack_app_token = app_token.trim().to_string();
        }
        self.persist_secrets().await
    }

    /// Настройка без своего поля (горячие клавиши): лежит в state.json вместе с остальными, проверяет её владелец.
    pub async fn set_setting_extra(&self, key: &str, value: Value) -> std::io::Result<()> {
        self.state.lock().unwrap().settings.extra.insert(key.to_string(), value);
        self.persist().await
    }

    /// Отметить, что ярлык уже создавали. Возвращает true только в первый раз.
    pub async fn mark_team_onboarded(&self) -> std::io::Result<()> {
        self.state.lock().unwrap().settings.team_onboarded = true;
        self.persist().await
    }

    /// Настройка, которой нет среди полей (например, checkout режима разработки). Меняет её только
    /// свой маршрут сервера, а не общий `PUT /api/settings`.
    pub async fn set_settings_extra(&self, key: &str, value: Value) -> std::io::Result<()> {
        self.state.lock().unwrap().settings.extra.insert(key.to_string(), value);
        self.persist().await
    }

    pub async fn take_first_shortcut(&self) -> bool {
        {
            let mut state = self.state.lock().unwrap();
            if state.settings.desktop_shortcut_done {
                return false;
            }
            state.settings.desktop_shortcut_done = true;
        }
        let _ = self.persist().await;
        true
    }

    /// Запомнить порт окна. Из API его не поменять: это не настройка, а след прошлого запуска.
    pub async fn remember_port(&self, port: u16) -> std::io::Result<()> {
        {
            let mut state = self.state.lock().unwrap();
            if state.settings.local_port == i64::from(port) {
                return Ok(());
            }
            state.settings.local_port = i64::from(port);
        }
        self.persist().await
    }

    pub async fn update_settings(&self, patch: &Value) -> std::io::Result<Settings> {
        {
            let mut state = self.state.lock().unwrap();
            if let Some(lang) = patch.get("uiLang").and_then(Value::as_str).filter(|l| *l == "ru" || *l == "en") {
                translate_starters(&mut state.bots, lang);
                // Первую команду переименовываем, пока человек её не трогал.
                let (from, to) = if lang == "en" { (0, 1) } else { (1, 0) };
                for room in state.rooms.iter_mut().filter(|r| r.name == MAIN_ROOM[from]) {
                    room.name = MAIN_ROOM[to].into();
                }
            }
            let s = &mut state.settings;
            if let Some(v) = patch.get("driver").filter(|v| !v.is_null()) {
                s.driver = as_string(v);
            }
            if let Some(v) = patch.get("model").filter(|v| !v.is_null()) {
                s.model = as_string(v);
            }
            if let Some(v) = patch.get("baseUrl").filter(|v| !v.is_null()) {
                s.base_url = as_string(v);
            }
            if let Some(v) = patch.get("apiKeyEnv").filter(|v| !v.is_null()) {
                s.api_key_env = as_string(v);
            }
            if let Some(v) = patch.get("cliCommand").filter(|v| !v.is_null()) {
                s.cli_command = as_string(v);
            }
            if let Some(v) = patch.get("fallbackDriver").filter(|v| !v.is_null()) {
                s.fallback_driver = as_string(v);
            }
            if let Some(v) = patch.get("fallbackModel").filter(|v| !v.is_null()) {
                s.fallback_model = as_string(v);
            }
            if let Some(v) = patch.get("fallbackBaseUrl").filter(|v| !v.is_null()) {
                s.fallback_base_url = as_string(v);
            }
            if let Some(v) = patch.get("fallbackCliCommand").filter(|v| !v.is_null()) {
                s.fallback_cli_command = as_string(v);
            }
            if let Some(list) = patch.get("modelServers").and_then(Value::as_array) {
                s.model_servers = list
                    .iter()
                    .filter_map(Value::as_str)
                    .map(|u| u.trim().trim_end_matches('/').to_string())
                    .filter(|u| (u.starts_with("http://") || u.starts_with("https://")) && !u.contains(char::is_whitespace))
                    .take(20)
                    .collect();
            }
            if let Some(v) = patch.get("acpMode").filter(|v| !v.is_null()) {
                s.acp_mode = as_string(v);
            }
            if let Some(v) = patch.get("acpCommand").filter(|v| !v.is_null()) {
                s.acp_command = as_string(v);
            }
            if let Some(env) = patch.get("acpEnv").and_then(Value::as_object) {
                s.acp_env = env
                    .iter()
                    .filter(|(name, _)| !name.trim().is_empty() && !name.to_uppercase().starts_with("CREW_"))
                    .map(|(name, value)| (name.trim().to_string(), as_string(value)))
                    .collect();
            }
            if let Some(x) = number(patch.get("temperature")) {
                s.temperature = x;
            }
            if let Some(x) = number(patch.get("historyLimit")) {
                s.history_limit = x as i64;
            }
            if let Some(v) = patch.get("vision").filter(|v| !v.is_null()) {
                s.vision = truthy(v);
            }
            if let Some(v) = patch.get("imageBaseUrl").filter(|v| !v.is_null()) {
                s.image_base_url = as_string(v);
            }
            if let Some(v) = patch.get("imageModel").filter(|v| !v.is_null()) {
                s.image_model = as_string(v);
            }
            if let Some(x) = number(patch.get("imageDaily")) {
                s.image_daily = (x as i64).max(0);
            }
            if let Some(v) = patch.get("browserPath").filter(|v| !v.is_null()) {
                s.browser_path = as_string(v);
            }
            // Неизвестное значение не сохраняем: сервер отвечает на него ошибкой до записи.
            if let Some(engine) = patch.get("browserEngine").and_then(Value::as_str) {
                if matches!(engine, "chrome" | "obscura" | "moli") {
                    s.browser_engine = engine.into();
                }
            }
            if let Some(lang) = patch.get("uiLang").and_then(Value::as_str) {
                if lang == "ru" || lang == "en" {
                    s.ui_lang = lang.into();
                }
            }
            if let Some(font) = patch.get("uiFont").and_then(Value::as_str) {
                if matches!(font, "system" | "onest" | "golos") {
                    s.ui_font = font.into();
                }
            }
            if let Some(v) = patch.get("checkUpdates").filter(|v| !v.is_null()) {
                s.check_updates = truthy(v);
            }
            if let Some(v) = patch.get("autoUpdate").filter(|v| !v.is_null()) {
                s.auto_update = truthy(v);
            }
            if let Some(v) = patch.get("lastSeenVersion").and_then(Value::as_str) {
                s.last_seen_version = v.chars().take(40).collect();
            }
            if let Some(mode) = patch.get("petsMode").and_then(Value::as_str) {
                if mode == "off" || mode == "desktop" || mode == "top" {
                    s.pets_mode = mode.into();
                }
            }
            if let Some(v) = patch.get("dockEnabled").filter(|v| !v.is_null()) {
                s.dock_enabled = truthy(v);
            }
            if let Some(layer) = patch.get("dockLayer").and_then(Value::as_str) {
                if layer == "top" || layer == "normal" {
                    s.dock_layer = layer.into();
                }
            }
            if let Some(display) = patch.get("dockDisplay").and_then(Value::as_str) {
                // Имя монитора из Windows короткое; чужой текст длиннее не нужен.
                s.dock_display = take_chars(display.trim(), 64);
            }
            if let Some(edge) = patch.get("dockEdge").and_then(Value::as_str) {
                if edge == "right" || edge == "left" {
                    s.dock_edge = edge.into();
                }
            }
            if let Some(x) = number(patch.get("dockOffset")).filter(|x| x.is_finite()) {
                s.dock_offset = x.clamp(0.0, 1.0);
            }
            if let Some(v) = patch.get("dockAutohide").filter(|v| !v.is_null()) {
                s.dock_autohide = truthy(v);
            }
            if let Some(v) = patch.get("dockResults").filter(|v| !v.is_null()) {
                s.dock_results = truthy(v);
            }
            if let Some(v) = patch.get("dockActivity").filter(|v| !v.is_null()) {
                s.dock_activity = truthy(v);
            }
            if let Some(target) = patch.get("voiceTarget").and_then(Value::as_str) {
                // «last» и пусто — последнему собеседнику; иначе id бота, он короткий.
                let target = target.trim();
                s.voice_target = if target == "last" { String::new() } else { take_chars(target, 64) };
            }
            if let Some(theme) = patch.get("uiTheme").and_then(Value::as_str) {
                if matches!(theme, "" | "light" | "dark") {
                    s.ui_theme = theme.into();
                }
            }
            if let Some(x) = number(patch.get("dailyTokenLimit")) {
                s.daily_token_limit = (x as i64).max(0);
            }
            if let Some(v) = patch.get("autostart").filter(|v| !v.is_null()) {
                s.autostart = truthy(v);
            }
            if let Some(v) = patch.get("keepAwake").filter(|v| !v.is_null()) {
                s.keep_awake = truthy(v);
            }
            if let Some(v) = patch.get("serverMode").filter(|v| !v.is_null()) {
                s.server_mode = truthy(v);
            }
            if let Some(x) = number(patch.get("journalSeenAt")) {
                s.journal_seen_at = x as i64;
            }
            if let Some(mode) = patch.get("skillLearning").and_then(Value::as_str) {
                if matches!(mode, "auto" | "ask" | "off") {
                    s.skill_learning = mode.into();
                }
            }
            // Включили питомцев, а выбранных нет: показываем первого, иначе включение выглядело бы поломкой.
            if state.settings.pets_mode != "off" && !state.bots.iter().any(|bot| bot.pet) {
                if let Some(first) = state.bots.first_mut() {
                    first.pet = true;
                }
            }
        }
        if let Some(v) = patch.get("apiKey").filter(|v| !v.is_null()) {
            self.secrets.lock().unwrap().api_key = as_string(v);
            self.persist_secrets().await?;
        }
        // Пустое поле в интерфейсе значит «оставить сохранённый ключ».
        if let Some(v) = patch.get("fallbackApiKey").and_then(Value::as_str).filter(|v| !v.trim().is_empty()) {
            self.secrets.lock().unwrap().fallback_api_key = v.to_string();
            self.persist_secrets().await?;
        }
        if let Some(v) = patch.get("imageApiKey").and_then(Value::as_str).filter(|v| !v.trim().is_empty()) {
            self.secrets.lock().unwrap().image_api_key = v.trim().to_string();
            self.persist_secrets().await?;
        }
        if let Some(v) = patch.get("anthropicApiKey").and_then(Value::as_str).filter(|v| !v.trim().is_empty()) {
            self.secrets.lock().unwrap().anthropic_api_key = v.trim().to_string();
            self.persist_secrets().await?;
        }
        if let Some(v) = patch.get("geminiApiKey").and_then(Value::as_str).filter(|v| !v.trim().is_empty()) {
            self.secrets.lock().unwrap().gemini_api_key = v.trim().to_string();
            self.persist_secrets().await?;
        }
        self.persist().await?;
        Ok(self.settings())
    }

    pub fn api_key(&self) -> String {
        let stored = self.secrets.lock().unwrap().api_key.clone();
        if !stored.is_empty() {
            return stored;
        }
        let name = self.settings().api_key_env;
        let name = if name.is_empty() { "OPENAI_API_KEY".to_string() } else { name };
        self.lookup_env(&name)
            .or_else(|| self.lookup_env("OPENAI_API_KEY"))
            .unwrap_or_default()
    }

    pub fn has_api_key(&self) -> bool {
        !self.api_key().is_empty()
    }

    /// Ключ Anthropic или Gemini: свой из настроек, иначе из окружения или `.env`.
    pub fn provider_key(&self, driver: &str) -> String {
        let (stored, names): (String, &[&str]) = {
            let secrets = self.secrets.lock().unwrap();
            match driver {
                "anthropic" => (secrets.anthropic_api_key.clone(), &["ANTHROPIC_API_KEY"]),
                "gemini" => (secrets.gemini_api_key.clone(), &["GEMINI_API_KEY", "GOOGLE_API_KEY"]),
                _ => return String::new(),
            }
        };
        if !stored.is_empty() {
            return stored;
        }
        names.iter().find_map(|name| self.lookup_env(name)).unwrap_or_default()
    }

    /// Свой ключ запасной модели. Если его нет, запасная на `api` пользуется основным.
    pub fn fallback_api_key(&self) -> String {
        self.secrets.lock().unwrap().fallback_api_key.clone()
    }

    pub fn has_fallback_key(&self) -> bool {
        !self.fallback_api_key().is_empty()
    }

    /// Свой ключ сервиса картинок. Когда можно взять основной, решает `image::api_key`.
    pub fn image_api_key(&self) -> String {
        self.secrets.lock().unwrap().image_api_key.clone()
    }

    pub fn has_image_key(&self) -> bool {
        !self.image_api_key().is_empty()
    }

    // --- аккаунт Deloist ----------------------------------------------------

    pub fn account_token(&self) -> Option<String> {
        Some(self.secrets.lock().unwrap().account_token.clone()).filter(|t| !t.is_empty())
    }

    /// Почта и имя, если вход выполнен.
    pub fn account(&self) -> Option<(String, String)> {
        let secrets = self.secrets.lock().unwrap();
        (!secrets.account_token.is_empty() && !secrets.account_email.is_empty())
            .then(|| (secrets.account_email.clone(), secrets.account_name.clone()))
    }

    /// Ключ провайдера из окна «Подключить свою ИИ». Уходит только этому провайдеру.
    pub fn own_key(&self, id: &str) -> String {
        self.secrets.lock().unwrap().provider_keys.get(id).cloned().unwrap_or_default()
    }

    /// Пустой ключ стирает сохранённый.
    pub async fn set_own_key(&self, id: &str, key: &str) -> std::io::Result<()> {
        {
            let mut secrets = self.secrets.lock().unwrap();
            if key.trim().is_empty() {
                secrets.provider_keys.remove(id);
            } else {
                secrets.provider_keys.insert(id.to_string(), key.trim().to_string());
            }
        }
        self.persist_secrets().await
    }

    pub async fn set_account(&self, token: &str, email: &str, name: &str) -> std::io::Result<()> {
        {
            let mut secrets = self.secrets.lock().unwrap();
            secrets.account_token = token.to_string();
            secrets.account_email = email.to_string();
            secrets.account_name = name.to_string();
        }
        self.persist_secrets().await
    }

    pub async fn clear_account(&self) -> std::io::Result<()> {
        self.set_account("", "", "").await
    }

    // --- боты --------------------------------------------------------------

    pub fn bots(&self) -> Vec<Bot> {
        self.state.lock().unwrap().bots.clone()
    }

    pub fn bot(&self, id: &str) -> Option<Bot> {
        self.state.lock().unwrap().bots.iter().find(|b| b.id == id).cloned()
    }

    pub async fn add_bot(&self, input: &Value) -> std::io::Result<Bot> {
        // Уровень модели и усилие лежат в `extra`: новый бот получает их так же, как из PATCH.
        let mut extra = Map::new();
        if let Some(level) = input.get("modelLevel").and_then(Value::as_str).and_then(crate::providers::ModelLevel::parse) {
            extra.insert("modelLevel".into(), json!(level.key()));
        }
        if let Some(effort) = input.get("effort").and_then(Value::as_str).filter(|e| valid_id(e, 40, false)) {
            extra.insert("effort".into(), json!(effort));
        }
        let bot = Bot {
            id: uid("bot"),
            name: take_chars(&text_or(input, "name", "Новый бот"), 40),
            role: take_chars(&text_or(input, "role", "без роли"), 80),
            emoji: take_chars(&text_or(input, "emoji", ""), 4),
            color: text_or(input, "color", "#7a8595"),
            persona: text_or(input, "persona", "Ты — полезный ассистент. Отвечай коротко и по делу."),
            brain: String::new(),
            driver: text_or(input, "driver", ""),
            model: text_or(input, "model", ""),
            rank: match input.get("rank").and_then(Value::as_str) {
                Some(rank) if BOT_RANKS.contains(&rank) => rank.into(),
                _ => "lead".into(),
            },
            pinned: input.get("pinned").is_some_and(truthy),
            team: take_chars(text_or(input, "team", "").trim(), 40),
            reserve_after_days: input.get("reserveAfterDays").and_then(Value::as_i64).filter(|days| *days >= 0).unwrap_or(7),
            last_active_at: 0,
            created_at: now(),
            browser_sites: Vec::new(),
            approval: approval_mode(input.get("approval")),
            always_ask: Vec::new(),
            skills: string_list(input.get("skills")),
            skin: look_index(input.get("skin"), MAX_SKIN).unwrap_or(-1),
            pet: false,
            pet_x: None,
            pet_y: None,
            dock: true,
            read_aloud: false,
            monthly_token_limit: 0,
            orchestrator: false,
            extra,
        };
        let bot = {
            let mut state = self.state.lock().unwrap();
            let mut bot = bot;
            if bot.skin < 0 {
                // Шар, которого у команды меньше всего: соседи не повторяются, пока шаров хватает.
                bot.skin = free_skin(&state.bots, skin_start(&bot.id));
            }
            state.bots.push(bot.clone());
            bot
        };
        tokio::fs::create_dir_all(self.workspace(&bot.id)).await?;
        self.persist().await?;
        Ok(bot)
    }

    pub async fn update_bot(&self, id: &str, patch: &Value) -> std::io::Result<Option<Bot>> {
        let updated = {
            let mut state = self.state.lock().unwrap();
            let Some(bot) = state.bots.iter_mut().find(|b| b.id == id) else {
                return Ok(None);
            };
            let fields: [(&str, &mut String); 7] = [
                ("name", &mut bot.name),
                ("role", &mut bot.role),
                ("emoji", &mut bot.emoji),
                ("color", &mut bot.color),
                ("persona", &mut bot.persona),
                ("driver", &mut bot.driver),
                ("model", &mut bot.model),
            ];
            for (key, slot) in fields {
                if let Some(v) = patch.get(key).filter(|v| !v.is_null()) {
                    *slot = as_string(v);
                }
            }
            if let Some(v) = patch.get("pinned").filter(|v| !v.is_null()) {
                bot.pinned = truthy(v);
            }
            if let Some(v) = patch.get("rank").and_then(Value::as_str).filter(|rank| BOT_RANKS.contains(rank)) {
                bot.rank = v.to_string();
            }
            if let Some(v) = patch.get("team").and_then(Value::as_str) {
                bot.team = take_chars(v.trim(), 40);
            }
            if let Some(v) = patch.get("reserveAfterDays").and_then(Value::as_i64).filter(|days| *days >= 0) {
                bot.reserve_after_days = v;
            }
            // Гость уходит из треда по просьбе человека: `PATCH { "removeGuest": "<botId>" }`.
            if let Some(guest) = patch.get("removeGuest").and_then(Value::as_str) {
                let mut guests = bot_guests(bot);
                let before = guests.len();
                guests.retain(|entry| entry["botId"] != guest);
                if guests.len() != before {
                    set_bot_guests(bot, guests);
                }
            }
            if let Some(skin) = look_index(patch.get("skin"), MAX_SKIN) {
                bot.skin = skin;
            }
            if let Some(sites) = patch.get("browserSites").and_then(Value::as_array) {
                bot.browser_sites = sites.iter().filter_map(Value::as_str).map(str::to_string).collect();
            }
            if let Some(v) = patch.get("dock").filter(|v| !v.is_null()) {
                bot.dock = truthy(v);
            }
            if let Some(v) = patch.get("readAloud").filter(|v| !v.is_null()) {
                bot.read_aloud = truthy(v);
            }
            let pet = patch.get("pet").filter(|v| !v.is_null()).map(truthy);
            if let Some(pet) = pet {
                bot.pet = pet;
            }
            // Месячный предел: пусто или null снимает, число — токены. Проверка вида — в `check_bot_patch`.
            if let Some(v) = patch.get("monthlyTokenLimit") {
                bot.monthly_token_limit = v.as_i64().filter(|n| *n > 0).unwrap_or(0);
            }
            if let Some(v) = patch.get("approval").filter(|v| !v.is_null()) {
                bot.approval = approval_mode(Some(v));
            }
            if let Some(v) = patch.get("alwaysAsk") {
                bot.always_ask = string_list(Some(v));
            }
            if let Some(v) = patch.get("skills").filter(|v| !v.is_null()) {
                bot.skills = string_list(Some(v));
            }
            // Память разговора: «обычная», «большая» или «максимум модели» (см. context.rs).
            if let Some(v) = patch.get("contextMode").and_then(Value::as_str) {
                bot.extra.insert("contextMode".into(), json!(crate::context::Memory::parse(v).key()));
            }
            // Порог сжатия в процентах окна: целое число, прижатое к границам ползунка; null убирает его
            // (снова старый режим), всё остальное - мусор: не записывается и не стирает сохранённое.
            if let Some(v) = patch.get("compactAtPercent") {
                if v.is_null() {
                    bot.extra.remove("compactAtPercent");
                } else if let Some(n) = v.as_i64().or_else(|| v.as_f64().filter(|f| f.fract() == 0.0 && f.abs() < 1e9).map(|f| f as i64)) {
                    bot.extra.insert("compactAtPercent".into(), json!(crate::context::clamp_percent(n)));
                }
            }
            // Уровень модели и усилие: пустая строка или null убирает поле, неизвестное значение не пишем.
            if let Some(v) = patch.get("modelLevel") {
                match v.as_str().and_then(crate::providers::ModelLevel::parse) {
                    Some(level) => bot.extra.insert("modelLevel".into(), json!(level.key())),
                    None => bot.extra.remove("modelLevel"),
                };
            }
            if let Some(v) = patch.get("effort") {
                match v.as_str().filter(|e| valid_id(e, 40, false)) {
                    Some(effort) => bot.extra.insert("effort".into(), json!(effort)),
                    None => bot.extra.remove("effort"),
                };
            }
            if let Some(v) = patch.get("agentOptions") {
                let clean: serde_json::Map<String, Value> = v
                    .as_object()
                    .map(|o| {
                        o.iter()
                            .filter(|(k, v)| valid_id(k, 64, true) && v.as_str().is_some_and(|v| valid_id(v, 64, true)))
                            .take(8)
                            .map(|(k, v)| (k.clone(), v.clone()))
                            .collect()
                    })
                    .unwrap_or_default();
                if clean.is_empty() {
                    bot.extra.remove("agentOptions");
                } else {
                    bot.extra.insert("agentOptions".into(), Value::Object(clean));
                }
            }
            let updated = bot.clone();
            match pet {
                // Выбрали бота при выключенных питомцах: человек хочет его видеть.
                Some(true) if state.settings.pets_mode == "off" => state.settings.pets_mode = "desktop".into(),
                Some(false) => calm_pets(&mut state),
                _ => {}
            }
            updated
        };
        self.persist().await?;
        Ok(Some(updated))
    }

    /// Бот закончил ход: запоминаем время последней активности для резерва.
    pub async fn note_bot_active(&self, id: &str, at: i64) -> std::io::Result<bool> {
        Ok(self.edit_bot(id, |bot| bot.last_active_at = at).await?.is_some())
    }

    /// Специалист пришёл в тред вызывающего бота гостем: `extra.guests` на боте-хозяине.
    pub async fn add_guest(&self, host_id: &str, guest_id: &str) -> std::io::Result<bool> {
        if host_id == guest_id {
            return Ok(false);
        }
        {
            let mut state = self.state.lock().unwrap();
            let at = now();
            let known: Vec<(String, i64, i64)> =
                state.bots.iter().map(|bot| (bot.id.clone(), bot.reserve_after_days, bot.last_active_at)).collect();
            if !known.iter().any(|(id, _, _)| id == guest_id) {
                return Ok(false);
            }
            let Some(host) = state.bots.iter_mut().find(|bot| bot.id == host_id) else {
                return Ok(false);
            };
            let mut guests = bot_guests(host);
            // Просроченные гости не переезжают в новую запись.
            guests.retain(|guest| {
                let id = guest["botId"].as_str().unwrap_or_default();
                known
                    .iter()
                    .find(|(known_id, _, _)| known_id == id)
                    .is_some_and(|(_, days, last)| !inactive_for_reserve(*days, *last, at))
            });
            // Повторный вызов обновляет `since` и поднимает гостя в конец: старые вытесняются первыми.
            guests.retain(|guest| guest["botId"] != guest_id);
            guests.push(json!({ "botId": guest_id, "since": at }));
            if guests.len() > MAX_GUESTS {
                guests.drain(..guests.len() - MAX_GUESTS);
            }
            set_bot_guests(host, guests);
        }
        self.persist().await?;
        Ok(true)
    }

    /// Убирает гостей, которые давно не работали или чьих ботов уже нет.
    pub async fn prune_guests(&self) -> std::io::Result<()> {
        let changed = {
            let mut state = self.state.lock().unwrap();
            let at = now();
            let known: Vec<(String, i64, i64)> =
                state.bots.iter().map(|bot| (bot.id.clone(), bot.reserve_after_days, bot.last_active_at)).collect();
            let mut changed = false;
            for bot in &mut state.bots {
                let Some(list) = bot.extra.get("guests").and_then(Value::as_array) else {
                    continue;
                };
                let raw_len = list.len();
                let mut guests = bot_guests(bot);
                guests.retain(|guest| {
                    let id = guest["botId"].as_str().unwrap_or_default();
                    known
                        .iter()
                        .find(|(known_id, _, _)| known_id == id)
                        .is_some_and(|(_, days, last)| !inactive_for_reserve(*days, *last, at))
                });
                if guests.len() != raw_len {
                    set_bot_guests(bot, guests);
                    changed = true;
                }
            }
            changed
        };
        if changed {
            self.persist().await?;
        }
        Ok(())
    }

    /// Служебное поле бота, которое не правится из профиля (например, знакомство с ним).
    pub async fn set_bot_extra(&self, id: &str, key: &str, value: Value) -> std::io::Result<bool> {
        {
            let mut state = self.state.lock().unwrap();
            let Some(bot) = state.bots.iter_mut().find(|b| b.id == id) else {
                return Ok(false);
            };
            bot.extra.insert(key.to_string(), value);
        }
        self.persist().await?;
        Ok(true)
    }

    /// Ещё один питомец поместится. Уже показанного снова включить можно всегда.
    pub fn pet_fits(&self, id: &str) -> bool {
        let state = self.state.lock().unwrap();
        state.bots.iter().filter(|bot| bot.pet && bot.id != id).count() < crate::pets::MAX_PETS
    }

    /// Человек поставил питомца на новое место. Ни окна, ни страницы это не меняет.
    pub async fn place_pet(&self, id: &str, x: i32, y: i32) -> std::io::Result<bool> {
        {
            let mut state = self.state.lock().unwrap();
            let Some(bot) = state.bots.iter_mut().find(|b| b.id == id) else {
                return Ok(false);
            };
            bot.pet_x = Some(x);
            bot.pet_y = Some(y);
        }
        self.persist().await?;
        Ok(true)
    }

    /// Старое разрешение сайта снято со страницы «Правила разрешений». Новые живут в `grants`.
    pub async fn forget_site(&self, bot_id: &str, host: &str) -> std::io::Result<bool> {
        {
            let mut state = self.state.lock().unwrap();
            let Some(bot) = state.bots.iter_mut().find(|b| b.id == bot_id) else {
                return Ok(false);
            };
            let before = bot.browser_sites.len();
            bot.browser_sites.retain(|site| site != host);
            if bot.browser_sites.len() == before {
                return Ok(false);
            }
        }
        self.persist().await.map(|_| true)
    }

    /// Профиль браузера бота: в нём живут входы в кабинеты.
    pub fn browser_profile(&self, bot_id: &str) -> PathBuf {
        self.data_dir.join("browser").join(bot_id)
    }

    /// Cookies и localStorage бота в Obscura. Отдельно от профиля Chrome: форматы разные.
    pub fn obscura_profile(&self, bot_id: &str) -> PathBuf {
        self.data_dir.join("browser-obscura").join(bot_id)
    }

    /// Хранилище бота в Moli (`--profile-dir`). Отдельно от Chrome и Obscura: форматы разные.
    pub fn moli_profile(&self, bot_id: &str) -> PathBuf {
        self.data_dir.join("browser-moli").join(bot_id)
    }

    pub async fn remove_bot(&self, id: &str) -> std::io::Result<bool> {
        {
            let mut state = self.state.lock().unwrap();
            let Some(idx) = state.bots.iter().position(|b| b.id == id) else {
                return Ok(false);
            };
            state.bots.remove(idx);
            state.routines.retain(|r| r.bot_id != id);
            calm_pets(&mut state);
        }
        self.persist().await?;
        let _ = tokio::fs::remove_file(self.thread_file(id)).await;
        // Бота нет, и входы в его кабинеты хранить незачем.
        let _ = tokio::fs::remove_dir_all(self.browser_profile(id)).await;
        let _ = tokio::fs::remove_dir_all(self.obscura_profile(id)).await;
        let _ = tokio::fs::remove_dir_all(self.moli_profile(id)).await;
        Ok(true)
    }

    // --- треды -------------------------------------------------------------

    fn thread_file(&self, bot_id: &str) -> PathBuf {
        self.data_dir.join("threads").join(format!("{bot_id}.jsonl"))
    }

    pub async fn append_message(&self, bot_id: &str, msg: Value) -> std::io::Result<Value> {
        let mut entry = Map::new();
        let source = msg.as_object().cloned().unwrap_or_default();
        entry.insert(
            "id".into(),
            source.get("id").filter(|v| truthy(v)).cloned().unwrap_or_else(|| json!(uid("msg"))),
        );
        entry.insert(
            "at".into(),
            source.get("at").filter(|v| truthy(v)).cloned().unwrap_or_else(|| json!(now())),
        );
        for (k, v) in source {
            entry.insert(k, v);
        }
        let entry = Value::Object(entry);
        let mut line = serde_json::to_string(&entry).map_err(std::io::Error::other)?;
        line.push('\n');
        let mut file = tokio::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(self.thread_file(bot_id))
            .await?;
        file.write_all(line.as_bytes()).await?;
        file.flush().await?;
        Ok(entry)
    }

    /// Все записи треда как есть.
    async fn load_thread(&self, bot_id: &str) -> Vec<Value> {
        let Ok(raw) = tokio::fs::read_to_string(self.thread_file(bot_id)).await else {
            return Vec::new();
        };
        // Обрезанная строка после падения просто пропускается.
        raw.split('\n')
            .filter(|l| !l.is_empty())
            .filter_map(|l| serde_json::from_str(l).ok())
            .collect()
    }

    /// Тихие ходы не показываем ни человеку, ни модели: в файле они остаются для отладки.
    fn drop_quiet(all: &[Value]) -> Vec<Value> {
        let quiet_turns: std::collections::HashSet<String> = all
            .iter()
            .filter(|m| m["role"] == "quiet")
            .filter_map(|m| m["turnId"].as_str().map(str::to_string))
            .collect();
        all.iter()
            .filter(|m| m["role"] != "quiet")
            .filter(|m| m["turnId"].as_str().map_or(true, |t| !quiet_turns.contains(t)))
            .cloned()
            .collect()
    }

    /// Тред для человека: служебные метки (часы, якорь окна) ему не нужны.
    pub async fn read_thread(&self, bot_id: &str, limit: i64) -> Vec<Value> {
        let all = self.load_thread(bot_id).await;
        let visible: Vec<Value> = human_view(Self::drop_quiet(&all));
        // Лимит считаем уже после отсева, иначе тихие записи вытеснят важные.
        let start = if limit > 0 { visible.len().saturating_sub(limit as usize) } else { 0 };
        visible[start..].to_vec()
    }

    /// Тред для модели: с якорем окна и с часами, которых не видит человек.
    /// Окно двигается редкими рывками, а не на одно сообщение за ход: иначе начало
    /// запроса меняется каждый ход и кэш провайдера не попадает.
    pub async fn read_model_thread(&self, bot_id: &str) -> Vec<Value> {
        model_window(&self.load_thread(bot_id).await)
    }

    /// Когда модель последний раз видела часы, если они ещё в её окне.
    pub async fn last_time_context(&self, bot_id: &str) -> Option<i64> {
        last_clock(&self.load_thread(bot_id).await)
    }

    /// Сдвинуть окно истории целиком: оставить около `retain` последних записей, если модель
    /// видит больше `keep`. Возвращает, поставили ли новый якорь.
    #[cfg(test)]
    pub async fn anchor_thread(&self, bot_id: &str, keep: usize, retain: usize) -> bool {
        let Some(plan) = self.plan_compaction(Thread::Bot(bot_id), keep, retain, false, 0).await else {
            return false;
        };
        self.write_anchor(Thread::Bot(bot_id), plan.start, None, plan.dropped.len()).await
    }

    async fn load_any(&self, thread: Thread<'_>) -> Vec<Value> {
        match thread {
            Thread::Bot(id) => self.load_thread(id).await,
            Thread::Room(id) => self.load_room_thread(id).await,
        }
    }

    /// Что свернуть в сводку, если окно переполнилось по счётчику или по размеру, или если просят.
    pub async fn plan_compaction(&self, thread: Thread<'_>, keep: usize, retain: usize, force: bool, budget: usize) -> Option<CompactPlan> {
        compact_plan(&self.load_any(thread).await, keep, retain, force, budget)
    }

    /// Поставить якорь окна со сводкой всего, что до него. Без сводки якорь как прежде просто сдвигает окно.
    pub async fn write_anchor(&self, thread: Thread<'_>, start: usize, summary: Option<&str>, dropped: usize) -> bool {
        let mut marker = json!({ "id": uid("anchor"), "role": "anchor", "from": start, "at": now(), "dropped": dropped });
        if let Some(text) = summary.filter(|s| !s.trim().is_empty()) {
            marker["summary"] = json!(text);
        }
        match thread {
            Thread::Bot(id) => self.append_message(id, marker).await.is_ok(),
            Thread::Room(id) => self.append_room_message(id, marker).await.is_ok(),
        }
    }

    /// Сводка истории до окна, которую модель видит в системном промпте.
    pub async fn thread_summary(&self, thread: Thread<'_>) -> Option<String> {
        last_summary(&self.load_any(thread).await)
    }

    /// Вся лента без тихих ходов, вместе с тем, что уже вне окна: по ней пишутся эпизоды и извлечение.
    pub async fn thread_entries(&self, thread: Thread<'_>) -> Vec<Value> {
        Self::drop_quiet(&self.load_any(thread).await)
    }

    /// Записи треда до сообщения `message_id` включительно, как они лежат в файле.
    /// Якорь окна остаётся на своём месте: его `from` считает записи этого же файла.
    pub async fn thread_prefix(&self, bot_id: &str, message_id: &str) -> Option<Vec<Value>> {
        let all = self.load_thread(bot_id).await;
        let index = all.iter().position(|m| m["id"].as_str() == Some(message_id))?;
        Some(all[..=index].to_vec())
    }

    /// Сколько знаков окна видит модель: по этому счёту окно и сжимается.
    pub async fn window_size(&self, thread: Thread<'_>) -> usize {
        window_chars(&self.load_any(thread).await)
    }

    pub async fn clear_thread(&self, bot_id: &str) -> std::io::Result<()> {
        tokio::fs::write(self.thread_file(bot_id), "").await
    }

    pub async fn last_preview(&self, bot_id: &str) -> Value {
        for m in self.read_thread(bot_id, 12).await.iter().rev() {
            let role = m.get("role").and_then(Value::as_str).unwrap_or_default();
            if role == "user" || role == "assistant" {
                let text = m.get("text").map(as_string).unwrap_or_default();
                let text = text.split_whitespace().collect::<Vec<_>>().join(" ");
                if !text.is_empty() {
                    let error = m.get("error").and_then(Value::as_bool).unwrap_or(false);
                    return json!({ "text": take_chars(&text, 120), "at": m.get("at"), "role": role, "error": error });
                }
            }
        }
        Value::Null
    }

    /// Папка файлов бота: у бота в команде — общая папка команды `workspaces/team_<id>`, иначе личная.
    /// Покинул команду или команда в архиве — вернулся в личную; то, что он сделал в общей, остаётся там.
    pub fn workspace(&self, bot_id: &str) -> PathBuf {
        match self.team_of(bot_id) {
            Some(room) => self.team_workspace(&room),
            None => self.personal_workspace(bot_id),
        }
    }

    /// Личная папка бота. Здесь лежит то, что у бота своё даже в команде: `memory.md`, `brain.md`, `MAIN.md`.
    pub fn personal_workspace(&self, bot_id: &str) -> PathBuf {
        self.data_dir.join("workspaces").join(bot_id)
    }

    /// Общая папка команды: только путь, без работы с диском. Папку и файлы участников готовит
    /// `build_team_folders` при каждом изменении состава (в `persist`) и при запуске.
    fn team_workspace(&self, room: &Room) -> PathBuf {
        self.data_dir.join("workspaces").join(format!("team_{}", room.id))
    }

    /// Собирает общие папки команд: файлы новых участников попадают в папку один раз (копией небольшого файла,
    /// без перезаписи и без жёстких ссылок: файл с несколькими ссылками `tools::flagged_target` считает подозрительным).
    /// Тяжёлая работа с диском идёт в отдельном потоке, и только если состав команды изменился.
    pub async fn build_team_folders(&self) {
        let jobs: Vec<(Room, Vec<(String, String)>)> = {
            let state = self.state.lock().unwrap();
            let mut built = self.team_built.lock().unwrap();
            state
                .rooms
                .iter()
                .filter(|r| !r.archived)
                .filter_map(|r| {
                    if built.get(&r.id) == Some(&r.bot_ids) {
                        return None;
                    }
                    built.insert(r.id.clone(), r.bot_ids.clone());
                    let names = r.bot_ids.iter().map(|id| (id.clone(), state.bots.iter().find(|b| &b.id == id).map(|b| b.name.clone()).unwrap_or_default())).collect();
                    Some((r.clone(), names))
                })
                .collect()
        };
        for (room, names) in jobs {
            let root = self.data_dir.join("workspaces");
            let _ = tokio::task::spawn_blocking(move || build_team_folder(&root, &room, &names)).await;
        }
    }

    /// Владелец браузера команды: бот, чей профиль держит общий браузер (там уже есть входы в кабинеты).
    /// Выбор запоминается в `browser/team_<id>.owner`, чтобы не менялся от того, кто сегодня активнее.
    /// Входы принадлежат команде: вышедший участник возвращается к своему профилю и общих входов не уносит.
    pub fn browser_owner(&self, bot_id: &str) -> String {
        match self.team_of(bot_id) {
            Some(room) => self.team_browser_owner(&room).unwrap_or_else(|| bot_id.to_string()),
            None => bot_id.to_string(),
        }
    }

    /// Владелец браузера команды. Вышел из команды (или удалён) — выбирается новый из оставшихся, файл переписывается.
    pub fn team_browser_owner(&self, room: &Room) -> Option<String> {
        let marker = self.data_dir.join("browser").join(format!("team_{}.owner", room.id));
        if let Ok(text) = std::fs::read_to_string(&marker) {
            let id = text.trim();
            if room.bot_ids.iter().any(|b| b == id) && self.bot(id).is_some() {
                return Some(id.to_string());
            }
        }
        // Первым берём профиль, которым пользовались последним: в нём свежие входы.
        let pick = room
            .bot_ids
            .iter()
            .filter(|id| self.bot(id).is_some())
            .map(|id| (std::fs::metadata(self.browser_profile(id).join("Default")).and_then(|m| m.modified()).ok(), id.clone()))
            .max_by_key(|(touched, _)| *touched)
            .map(|(_, id)| id)?;
        let _ = std::fs::create_dir_all(self.data_dir.join("browser"));
        let _ = std::fs::write(&marker, &pick);
        Some(pick)
    }

    /// Путь внутри воркспейса бота. Всё, что пытается выйти наружу, — ошибка.
    pub fn safe_path(&self, bot_id: &str, rel: &str) -> Result<PathBuf, String> {
        let base = self.workspace(bot_id);
        let outside = || format!("путь вне воркспейса: {rel}");
        // Полный путь к своему же файлу бот берёт из ответов инструментов: внутри своей папки это тот же файл.
        let given = Path::new(rel);
        let inner = given.strip_prefix(&base).unwrap_or(given);
        let mut parts: Vec<std::ffi::OsString> = Vec::new();
        for component in inner.components() {
            match component {
                Component::Normal(part) => {
                    if !plain_file_name(&part.to_string_lossy()) {
                        return Err(outside());
                    }
                    parts.push(part.to_os_string());
                }
                Component::CurDir => {}
                Component::ParentDir => {
                    parts.pop().ok_or_else(outside)?;
                }
                Component::RootDir | Component::Prefix(_) => return Err(outside()),
            }
        }
        let mut abs = base.clone();
        for part in parts {
            abs.push(part);
        }

        // Текст пути мог быть чистым, а внутри воркспейса лежать ссылка наружу:
        // смотрим, куда на самом деле ведёт самая глубокая существующая часть.
        let _ = std::fs::create_dir_all(&base);
        let root = std::fs::canonicalize(&base).map_err(|e| e.to_string())?;
        let mut existing = abs.as_path();
        while std::fs::symlink_metadata(existing).is_err() {
            existing = existing.parent().ok_or_else(outside)?;
        }
        // Битая ссылка canonicalize не проходит и тоже отбивается здесь.
        let real = std::fs::canonicalize(existing).map_err(|_| outside())?;
        if !real.starts_with(&root) {
            return Err(outside());
        }
        Ok(abs)
    }

    pub async fn list_workspace(&self, bot_id: &str, rel: &str) -> Result<Vec<Value>, String> {
        let abs = self.safe_path(bot_id, rel)?;
        let mut dir = tokio::fs::read_dir(&abs).await.map_err(|e| e.to_string())?;
        let prefix = rel.replace('\\', "/");
        let mut items: Vec<(bool, String, String, i64)> = Vec::new();
        while let Some(entry) = dir.next_entry().await.map_err(|e| e.to_string())? {
            let name = entry.file_name().to_string_lossy().into_owned();
            if name.starts_with('.') {
                continue;
            }
            let is_dir = entry.file_type().await.map(|t| t.is_dir()).unwrap_or(false);
            // Время правки нужно командной строке: `--export` забирает только то, что бот изменил.
            let modified = entry
                .metadata()
                .await
                .ok()
                .and_then(|m| m.modified().ok())
                .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                .map(|d| d.as_millis() as i64)
                .unwrap_or(0);
            let trimmed = prefix.trim_end_matches('/');
            let path = if trimmed.is_empty() || trimmed == "." {
                name.clone()
            } else {
                format!("{trimmed}/{name}")
            };
            items.push((is_dir, name, path, modified));
        }
        items.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.cmp(&b.1)));
        Ok(items
            .into_iter()
            .map(|(dir, name, path, modified)| json!({ "name": name, "dir": dir, "path": path, "modified": modified }))
            .collect())
    }

    // --- рутины ------------------------------------------------------------

    pub fn routines(&self) -> Vec<Routine> {
        self.state.lock().unwrap().routines.clone()
    }

    /// Обычная рутина такой, какой её запишет `add_routine_capped` по тому же вводу, но без записи:
    /// карточка `propose_routine` показывает расписание и ближайший запуск из неё.
    pub fn preview_routine(input: &Value) -> Routine {
        let mut routine = Routine {
            bot_id: text_or(input, "botId", ""),
            title: take_chars(&text_or(input, "title", "Рутина"), 80),
            prompt: text_or(input, "prompt", ""),
            every_minutes: minutes(input.get("everyMinutes")),
            enabled: input.get("enabled").map_or(true, |v| v != &Value::Bool(false)),
            quiet: input.get("quiet").map_or(true, truthy),
            ..Routine::default()
        };
        apply_schedule(&mut routine, input).ok();
        routine.next_run = next_run_for(&routine, now());
        routine
    }

    pub async fn add_routine(&self, input: &Value) -> std::io::Result<Routine> {
        Ok(self.add_routine_capped(input, usize::MAX).await?.expect("без предела рутина добавляется всегда"))
    }

    /// То же с пределом: счёт и вставка под одним замком. `None` — предел достигнут.
    pub async fn add_routine_capped(&self, input: &Value, cap: usize) -> std::io::Result<Option<Routine>> {
        // Напоминание не привязано к боту: его шлёт таймер, а название по умолчанию — текст напоминания.
        let reminder = reminders::wants(input);
        let mut routine = Routine {
            id: uid("rt"),
            bot_id: if reminder { String::new() } else { text_or(input, "botId", "") },
            title: take_chars(&text_or(input, "title", if reminder { "" } else { "Рутина" }), 80),
            prompt: text_or(input, "prompt", ""),
            every_minutes: minutes(input.get("everyMinutes")),
            enabled: input.get("enabled").map_or(true, |v| v != &Value::Bool(false)),
            quiet: input.get("quiet").map_or(true, truthy),
            catch_up: text_or(input, "catchUp", "last"),
            created_at: now(),
            ..Routine::default()
        };
        if reminder {
            routine.extra.insert("kind".into(), json!(reminders::KIND));
            reminders::apply(&mut routine, input);
            if routine.title.is_empty() {
                routine.title = take_chars(&routine.prompt, 80);
            }
        }
        apply_schedule(&mut routine, input).map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidInput, e))?;
        routine.hook = input.get("hook").is_some_and(truthy);
        routine.goal_id = text_or(input, "goalId", "");
        routine.approval_floor = text_or(input, "approvalFloor", "");
        if let Some(v) = input.get("hookEvents").filter(|v| !v.is_null()) {
            routine.hook_events = validate_hook_events(v).map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
        }
        if routine.hook {
            routine.hook_token = crate::guard::random_token();
            // Вебхук не по расписанию: тикер его не трогает.
            routine.next_run = 0;
        } else {
            routine.next_run = next_run_for(&routine, now());
        }
        {
            let mut state = self.state.lock().unwrap();
            if state.routines.len() >= cap {
                return Ok(None);
            }
            state.routines.push(routine.clone());
        }
        self.persist().await?;
        Ok(Some(routine))
    }

    pub async fn update_routine(&self, id: &str, patch: &Value) -> std::io::Result<Option<Routine>> {
        let updated = {
            let mut state = self.state.lock().unwrap();
            let Some(r) = state.routines.iter_mut().find(|r| r.id == id) else {
                return Ok(None);
            };
            if let Some(v) = patch.get("enabled").filter(|v| !v.is_null()) {
                r.enabled = truthy(v);
            }
            if let Some(v) = patch.get("quiet").filter(|v| !v.is_null()) {
                r.quiet = truthy(v);
            }
            if let Some(v) = patch.get("prompt").filter(|v| !v.is_null()) {
                r.prompt = as_string(v);
            }
            if let Some(v) = patch.get("title").filter(|v| !v.is_null()) {
                r.title = as_string(v);
            }
            if let Some(v) = patch.get("goalId").filter(|v| !v.is_null()) {
                r.goal_id = as_string(v);
            }
            if let Some(v) = patch.get("approvalFloor").filter(|v| !v.is_null()) {
                r.approval_floor = as_string(v);
            }
            // «Возобновить» после паузы из-за ошибок.
            if let Some(v) = patch.get("errorPaused").filter(|v| !v.is_null()) {
                r.error_paused = truthy(v);
                if !r.error_paused {
                    r.failures = 0;
                    r.stop_reason = None;
                }
            }
            if let Some(v) = patch.get("catchUp").filter(|v| !v.is_null()) {
                r.catch_up = as_string(v);
            }
            if let Some(v) = patch.get("progressNote").filter(|v| !v.is_null()) {
                r.progress_note = as_string(v);
            }
            if patch.get("everyMinutes").is_some_and(|v| !v.is_null()) {
                r.every_minutes = minutes(patch.get("everyMinutes"));
            }
            if let Some(v) = patch.get("hook").filter(|v| !v.is_null()) {
                let want = truthy(v);
                if want && !r.hook {
                    r.hook_token = crate::guard::random_token();
                }
                if !want {
                    r.hook_token.clear();
                }
                r.hook = want;
            }
            if let Some(v) = patch.get("hookEvents").filter(|v| !v.is_null()) {
                r.hook_events = validate_hook_events(v).map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
            }
            apply_schedule(r, patch).map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidInput, e))?;
            if reminders::is_reminder(r) {
                reminders::apply(r, patch);
            }
            let rescheduled = ["everyMinutes", "everyMaxMinutes", "at", "days", "windowFrom", "windowTo", "hook", "leadMinutes", "recurrence", "startOn", "onceAt", "cron", "tz"].iter().any(|key| patch.get(*key).is_some_and(|v| !v.is_null()));
            // Человек выключил рутину или поменял расписание: прежняя серия ошибок больше не нужна.
            if rescheduled || !r.enabled || r.hook {
                r.failures = 0;
                r.stop_reason = None;
            }
            if r.hook {
                r.next_run = 0;
            } else if rescheduled {
                r.next_run = next_run_for(r, now());
            }
            // Только для проверок: приблизить срок, не дожидаясь его.
            if cfg!(debug_assertions) {
                if let Some(at) = number(patch.get("nextRun")) {
                    r.next_run = at as i64;
                }
            }
            r.clone()
        };
        self.persist().await?;
        Ok(Some(updated))
    }

    pub async fn remove_routine(&self, id: &str) -> std::io::Result<bool> {
        {
            let mut state = self.state.lock().unwrap();
            let Some(idx) = state.routines.iter().position(|r| r.id == id) else {
                return Ok(false);
            };
            state.routines.remove(idx);
        }
        self.persist().await?;
        Ok(true)
    }

    pub async fn mark_routine_run(&self, id: &str, ok: bool) -> std::io::Result<()> {
        {
            let mut state = self.state.lock().unwrap();
            let Some(r) = state.routines.iter_mut().find(|r| r.id == id) else {
                return Ok(());
            };
            r.last_run = now();
            r.busy_since = 0;
            // «Готово» ставит только конец хода: до него рутина «идёт», а обрыв перезапуском виден при старте.
            r.last_status = Some(if ok { "running" } else { "error" }.to_string());
            schedule::finish(r, now());
        }
        self.persist().await
    }

    /// Ход рутины кончился со статусом `status`. Выполнено — серия ошибок сброшена. Ошибка (`reason`) растит серию:
    /// от 3 подряд рутина идёт реже, а три «вечные» ошибки подряд ставят её на паузу. Выключить её сбои не могут.
    pub async fn finish_routine_run(&self, id: &str, status: &str, reason: Option<crate::runs::StopReason>) -> std::io::Result<RoutineOutcome> {
        let outcome = {
            let mut state = self.state.lock().unwrap();
            let Some(r) = state.routines.iter_mut().find(|r| r.id == id) else {
                return Ok(RoutineOutcome::Done);
            };
            // Цель ведёт свои запуски сама, вебхук без расписания.
            let counted = r.goal_id.is_empty() && !r.hook;
            let outcome = match reason.filter(|r| counted && r.is_failure() && matches!(status, "failed" | "needs")) {
                Some(reason) => {
                    r.failures += 1;
                    r.stop_reason = Some(reason);
                    if r.enabled {
                        r.next_run = r.next_run.max(r.last_run + schedule::failure_gap_ms(r.failures));
                    }
                    if reason.is_lasting() && r.failures >= 3 && !r.error_paused {
                        r.error_paused = true;
                        RoutineOutcome::PausedByErrors { failures: r.failures }
                    } else {
                        RoutineOutcome::Failed { failures: r.failures }
                    }
                }
                None => {
                    if counted && matches!(status, "ok" | "quiet") {
                        r.failures = 0;
                        r.stop_reason = None;
                        r.error_paused = false;
                    }
                    RoutineOutcome::Done
                }
            };
            r.last_status = Some(status.to_string());
            outcome
        };
        self.persist().await?;
        Ok(outcome)
    }

    /// Запуск не состоялся (не то время, предел токенов): просто следующий срок.
    pub async fn reschedule_routine(&self, id: &str, status: &str) -> std::io::Result<()> {
        {
            let mut state = self.state.lock().unwrap();
            let Some(r) = state.routines.iter_mut().find(|r| r.id == id) else {
                return Ok(());
            };
            r.last_status = Some(status.to_string());
            schedule::finish(r, now());
        }
        self.persist().await
    }

    /// Напоминание отработало (или пропущено): статус, подробности по каналам, следующий срок.
    /// Разовое после срока выключается, чтобы не сработать второй раз.
    pub async fn finish_reminder(&self, id: &str, status: &str, detail: Value, next_run: i64, disable: bool) -> std::io::Result<()> {
        {
            let mut state = self.state.lock().unwrap();
            let Some(r) = state.routines.iter_mut().find(|r| r.id == id) else {
                return Ok(());
            };
            r.last_run = now();
            r.last_status = Some(status.to_string());
            r.extra.insert("lastDelivery".into(), detail);
            r.next_run = next_run;
            if disable {
                r.enabled = false;
            }
        }
        self.persist().await
    }

    pub async fn set_routine_status(&self, id: &str, status: &str) -> std::io::Result<()> {
        {
            let mut state = self.state.lock().unwrap();
            let Some(r) = state.routines.iter_mut().find(|r| r.id == id) else {
                return Ok(());
            };
            r.last_status = Some(status.to_string());
        }
        self.persist().await
    }

    pub fn routine(&self, id: &str) -> Option<Routine> {
        self.state.lock().unwrap().routines.iter().find(|r| r.id == id).cloned()
    }

    /// Бот занят: рутина ждёт его по 5 минут, очередь не копит, но не дольше часа. `false` — час прошёл, запуск пропущен.
    pub fn wait_for_bot(&self, id: &str) -> bool {
        let mut state = self.state.lock().unwrap();
        let Some(r) = state.routines.iter_mut().find(|r| r.id == id) else { return true };
        let t = now();
        if r.busy_since == 0 {
            r.busy_since = t;
        }
        if t - r.busy_since >= schedule::hour_ms() {
            r.busy_since = 0;
            return false;
        }
        r.next_run = t + schedule::hour_ms() / 12;
        true
    }

    // --- навыки ------------------------------------------------------------

    pub fn skills(&self) -> Vec<Skill> {
        self.state.lock().unwrap().skills.clone()
    }

    pub fn skill(&self, id: &str) -> Option<Skill> {
        self.state.lock().unwrap().skills.iter().find(|s| s.id == id).cloned()
    }

    /// Навыки, подключённые боту, в порядке подключения.
    pub fn skills_for_bot(&self, bot_id: &str) -> Vec<Skill> {
        let state = self.state.lock().unwrap();
        let Some(bot) = state.bots.iter().find(|b| b.id == bot_id) else {
            return Vec::new();
        };
        bot.skills
            .iter()
            .filter_map(|id| state.skills.iter().find(|s| &s.id == id).cloned())
            .collect()
    }

    /// Навыки бота внутри группы: свои и общие группы (только пока бот в её составе).
    pub fn skills_for_bot_in_room(&self, bot_id: &str, room_id: &str) -> Vec<Skill> {
        let state = self.state.lock().unwrap();
        let Some(bot) = state.bots.iter().find(|b| b.id == bot_id) else {
            return Vec::new();
        };
        let shared: &[String] = match state.rooms.iter().find(|r| r.id == room_id && r.bot_ids.iter().any(|b| b == bot_id)) {
            Some(room) => &room.skills,
            None => &[],
        };
        let mut ids: Vec<&String> = bot.skills.iter().collect();
        ids.extend(shared.iter().filter(|id| !bot.skills.contains(id)));
        ids.into_iter().filter_map(|id| state.skills.iter().find(|s| &s.id == id).cloned()).collect()
    }

    pub async fn add_skill(&self, input: &Value) -> std::io::Result<Skill> {
        let skill = Skill {
            id: uid("sk"),
            name: take_chars(&text_or(input, "name", "Новый навык"), 60),
            description: take_chars(&text_or(input, "description", ""), 160),
            body: text_or(input, "body", ""),
            created_at: now(),
            ..Skill::default()
        };
        // Навык попадает в каждый запрос бота целиком.
        let skill = Skill { body: take_chars(&skill.body, 20_000), ..skill };
        self.state.lock().unwrap().skills.push(skill.clone());
        self.persist().await?;
        Ok(skill)
    }

    pub async fn update_skill(&self, id: &str, patch: &Value) -> std::io::Result<Option<Skill>> {
        let updated = {
            let mut state = self.state.lock().unwrap();
            let Some(skill) = state.skills.iter_mut().find(|s| s.id == id) else {
                return Ok(None);
            };
            for (key, slot) in [("name", &mut skill.name), ("description", &mut skill.description), ("body", &mut skill.body)] {
                if let Some(v) = patch.get(key).filter(|v| !v.is_null()) {
                    *slot = as_string(v);
                }
            }
            skill.clone()
        };
        self.persist().await?;
        Ok(Some(updated))
    }

    pub async fn remove_skill(&self, id: &str) -> std::io::Result<bool> {
        {
            let mut state = self.state.lock().unwrap();
            let Some(idx) = state.skills.iter().position(|s| s.id == id) else {
                return Ok(false);
            };
            state.skills.remove(idx);
            // Навыка нет — его не должно остаться и в профилях ботов.
            for bot in &mut state.bots {
                bot.skills.retain(|skill| skill != id);
            }
            for room in &mut state.rooms {
                room.skills.retain(|skill| skill != id);
            }
        }
        self.persist().await?;
        Ok(true)
    }

    /// Положить готовый навык (каталог собирает его сам). Длины режутся так же, как у человека.
    pub async fn push_skill(&self, mut skill: Skill) -> std::io::Result<Skill> {
        skill.name = take_chars(&skill.name, 60);
        skill.description = take_chars(&skill.description, 160);
        skill.body = take_chars(&skill.body, 20_000);
        self.state.lock().unwrap().skills.push(skill.clone());
        self.persist().await?;
        Ok(skill)
    }

    /// Правка навыка целиком: каталог меняет текст и свои отметки одним шагом.
    pub async fn edit_skill(&self, id: &str, edit: impl FnOnce(&mut Skill)) -> std::io::Result<Option<Skill>> {
        let updated = {
            let mut state = self.state.lock().unwrap();
            let Some(skill) = state.skills.iter_mut().find(|s| s.id == id) else {
                return Ok(None);
            };
            edit(skill);
            skill.body = take_chars(&skill.body, 20_000);
            if skill.previous.len() > SKILL_VERSIONS {
                let extra = skill.previous.len() - SKILL_VERSIONS;
                skill.previous.drain(..extra);
            }
            skill.clone()
        };
        self.persist().await?;
        Ok(Some(updated))
    }

    /// Правка бота целиком, для полей вне профиля (отметки каталога в `extra`).
    pub async fn edit_bot(&self, id: &str, edit: impl FnOnce(&mut Bot)) -> std::io::Result<Option<Bot>> {
        let updated = {
            let mut state = self.state.lock().unwrap();
            let Some(bot) = state.bots.iter_mut().find(|b| b.id == id) else {
                return Ok(None);
            };
            edit(bot);
            bot.clone()
        };
        self.persist().await?;
        Ok(Some(updated))
    }

    /// Бот сохранил способ работы. Навык с тем же названием, подключённый ему, правится (прошлая
    /// версия уходит в историю), иначе создаётся новый и сразу подключается этому боту.
    /// Чужой навык с тем же названием бот не трогает: он меняет поведение других ботов.
    pub async fn learn_skill(&self, bot_id: &str, name: &str, description: &str, body: &str) -> Result<(Skill, bool), String> {
        let name = take_chars(name.trim(), 60);
        let body = take_chars(body.trim(), 20_000);
        if name.is_empty() || body.is_empty() {
            return Err("у навыка должны быть название и шаги".into());
        }
        let description = take_chars(description.trim(), 160);
        let outcome = {
            let mut state = self.state.lock().unwrap();
            let Some(bot_index) = state.bots.iter().position(|b| b.id == bot_id) else {
                return Err("бот не найден".into());
            };
            let attached = state.bots[bot_index].skills.clone();
            let wanted = name.to_lowercase();
            match state.skills.iter().position(|s| s.name.to_lowercase() == wanted) {
                Some(index) if attached.contains(&state.skills[index].id) => {
                    let skill = &mut state.skills[index];
                    skill.previous.push(SkillVersion {
                        name: skill.name.clone(),
                        description: skill.description.clone(),
                        body: skill.body.clone(),
                        at: now(),
                    });
                    if skill.previous.len() > SKILL_VERSIONS {
                        let extra = skill.previous.len() - SKILL_VERSIONS;
                        skill.previous.drain(..extra);
                    }
                    skill.body = body;
                    if !description.is_empty() {
                        skill.description = description;
                    }
                    skill.updated_at = now();
                    (skill.clone(), false)
                }
                Some(_) => return Err(format!("навык «{name}» уже есть у других ботов: выбери другое название")),
                None => {
                    let skill = Skill {
                        id: uid("sk"),
                        name,
                        description,
                        body,
                        created_at: now(),
                        origin: "learned".into(),
                        bot_id: bot_id.to_string(),
                        updated_at: now(),
                        ..Skill::default()
                    };
                    state.skills.push(skill.clone());
                    state.bots[bot_index].skills.push(skill.id.clone());
                    (skill, true)
                }
            }
        };
        self.persist().await.map_err(|e| e.to_string())?;
        Ok(outcome)
    }

    /// Отменить последнюю правку навыка. Нет прошлой версии у выученного навыка — он удаляется,
    /// у навыка человека отменять нечего. Возвращает, что стало: `restored` или `removed`.
    pub async fn undo_skill(&self, id: &str) -> Result<&'static str, String> {
        let removed = {
            let mut state = self.state.lock().unwrap();
            let Some(skill) = state.skills.iter_mut().find(|s| s.id == id) else {
                return Err("навык не найден".into());
            };
            match skill.previous.pop() {
                Some(version) => {
                    skill.name = version.name;
                    skill.description = version.description;
                    skill.body = version.body;
                    skill.updated_at = now();
                    false
                }
                None if skill.origin == "learned" => true,
                None => return Err("у этого навыка нет прошлой версии".into()),
            }
        };
        if removed {
            self.remove_skill(id).await.map_err(|e| e.to_string())?;
            return Ok("removed");
        }
        self.persist().await.map_err(|e| e.to_string())?;
        Ok("restored")
    }

    // --- группы ------------------------------------------------------------

    pub fn rooms(&self) -> Vec<Room> {
        self.state.lock().unwrap().rooms.clone()
    }

    pub fn room(&self, id: &str) -> Option<Room> {
        self.state.lock().unwrap().rooms.iter().find(|r| r.id == id).cloned()
    }

    /// Незаблокированная (не архивная) команда, в которой состоит бот. Бот может быть только в одной такой команде.
    pub fn team_of(&self, bot_id: &str) -> Option<Room> {
        self.state.lock().unwrap().rooms.iter().find(|r| !r.archived && r.bot_ids.iter().any(|b| b == bot_id)).cloned()
    }

    /// Проверка «один бот — одна команда»: ни один из `bot_ids` не должен уже состоять в другой
    /// незаблокированной команде, кроме, возможно, `room_id` (та же команда, в которую их добавляют).
    /// Возвращает текст ошибки, пригодный для отдачи клиенту.
    pub fn check_single_team(&self, bot_ids: &[String], room_id: Option<&str>) -> Result<(), String> {
        let bots = self.bots();
        for id in bot_ids {
            if let Some(room) = self.team_of(id) {
                if room_id.map(|r| r != room.id).unwrap_or(true) {
                    let name = bots.iter().find(|b| &b.id == id).map(|b| b.name.clone()).unwrap_or_else(|| id.clone());
                    return Err(format!("«{name}» уже в команде «{}». Бот может быть только в одной команде — сначала убери его оттуда.", room.name));
                }
            }
        }
        Ok(())
    }

    pub async fn add_room(&self, input: &Value) -> std::io::Result<Room> {
        let bots = self.bots();
        let wanted = string_list(input.get("botIds"));
        let members: Vec<String> = wanted.into_iter().filter(|id| bots.iter().any(|b| &b.id == id)).collect();
        if let Err(msg) = self.check_single_team(&members, None) {
            return Err(std::io::Error::new(std::io::ErrorKind::InvalidData, msg));
        }
        let lead = text_or(input, "lead", "");
        let lead = if members.contains(&lead) { lead } else { members.first().cloned().unwrap_or_default() };
        let room = Room {
            id: uid("room"),
            name: take_chars(&text_or(input, "name", "Новая группа"), 40),
            emoji: String::new(),
            color: text_or(input, "color", "#7a8595"),
            instructions: text_or(input, "instructions", ""),
            bot_ids: members,
            lead,
            mode: if text_or(input, "mode", "lead") == "auto" { "auto".into() } else { "lead".into() },
            archived: false,
            folders: Vec::new(),
            program_folders: Vec::new(),
            skills: Vec::new(),
            picture: String::new(),
            picture_at: 0,
            stage: STAGE_READY.into(),
            created_at: now(),
        };
        self.state.lock().unwrap().rooms.push(room.clone());
        self.persist().await?;
        Ok(room)
    }

    /// Новая команда с оркестратором во главе: расспрос идёт в ней (`stage = interview`).
    pub async fn add_orchestrator_room(&self, input: &Value) -> std::io::Result<(Room, Bot)> {
        let bot = {
            let mut state = self.state.lock().unwrap();
            let bot = new_orchestrator(&state.bots);
            state.bots.push(bot.clone());
            bot
        };
        tokio::fs::create_dir_all(self.workspace(&bot.id)).await?;
        let room = Room {
            id: uid("room"),
            name: take_chars(&text_or(input, "name", "Новая группа"), 40),
            color: text_or(input, "color", "#7a8595"),
            instructions: text_or(input, "instructions", ""),
            bot_ids: vec![bot.id.clone()],
            lead: bot.id.clone(),
            stage: STAGE_INTERVIEW.into(),
            created_at: now(),
            ..Room::default()
        };
        self.state.lock().unwrap().rooms.push(room.clone());
        self.persist().await?;
        Ok((room, bot))
    }

    /// Оркестратор команды, если он в ней есть.
    pub fn room_orchestrator(&self, room_id: &str) -> Option<Bot> {
        let state = self.state.lock().unwrap();
        let room = state.rooms.iter().find(|r| r.id == room_id)?;
        room.bot_ids.iter().find_map(|id| state.bots.iter().find(|b| &b.id == id && b.orchestrator)).cloned()
    }

    /// Команда, в которой этот оркестратор собирает состав: сначала та, где идёт расспрос.
    pub fn orchestrator_room(&self, bot_id: &str) -> Option<Room> {
        let state = self.state.lock().unwrap();
        let mut mine = state.rooms.iter().filter(|r| !r.archived && r.bot_ids.iter().any(|id| id == bot_id));
        let first = mine.clone().find(|r| r.stage == STAGE_INTERVIEW);
        first.or_else(|| mine.next()).cloned()
    }

    /// Одобренные правила бота (`brain.md`). Вызывает только `brain::set`.
    pub async fn set_bot_brain(&self, id: &str, text: &str) -> std::io::Result<Option<Bot>> {
        let bot = {
            let mut state = self.state.lock().unwrap();
            let Some(bot) = state.bots.iter_mut().find(|b| b.id == id) else {
                return Ok(None);
            };
            bot.brain = text.to_string();
            bot.clone()
        };
        self.persist().await?;
        Ok(Some(bot))
    }

    pub async fn set_room_stage(&self, id: &str, stage: &str) -> std::io::Result<Option<Room>> {
        let room = {
            let mut state = self.state.lock().unwrap();
            let Some(room) = state.rooms.iter_mut().find(|r| r.id == id) else {
                return Ok(None);
            };
            room.stage = if stage == STAGE_INTERVIEW { STAGE_INTERVIEW.into() } else { STAGE_READY.into() };
            room.clone()
        };
        self.persist().await?;
        Ok(Some(room))
    }

    /// «Перестроить команду»: расспрос заново, ведущий — оркестратор (у старой команды без него он появляется).
    pub async fn rebuild_room(&self, id: &str) -> std::io::Result<Option<(Room, Bot)>> {
        let created = {
            let mut state = self.state.lock().unwrap();
            let Some(pos) = state.rooms.iter().position(|r| r.id == id) else {
                return Ok(None);
            };
            let existing = state.rooms[pos].bot_ids.iter().find_map(|b| state.bots.iter().find(|x| &x.id == b && x.orchestrator)).cloned();
            let (bot, fresh) = match existing {
                Some(bot) => (bot, false),
                None => {
                    let bot = new_orchestrator(&state.bots);
                    state.bots.push(bot.clone());
                    state.rooms[pos].bot_ids.push(bot.id.clone());
                    (bot, true)
                }
            };
            let room = &mut state.rooms[pos];
            room.lead = bot.id.clone();
            room.stage = STAGE_INTERVIEW.into();
            (room.clone(), bot, fresh)
        };
        if created.2 {
            tokio::fs::create_dir_all(self.workspace(&created.1.id)).await?;
        }
        self.persist().await?;
        Ok(Some((created.0, created.1)))
    }

    pub async fn update_room(&self, id: &str, patch: &Value) -> std::io::Result<Option<Room>> {
        if self.room(id).is_none() {
            return Ok(None);
        }
        let bots = self.bots();
        // Проверка одиночной команды: добавляемые боты не должны состоять в другой незаблокированной команде.
        if let Some(v) = patch.get("botIds").filter(|v| !v.is_null()) {
            let members: Vec<String> = string_list(Some(v)).into_iter().filter(|bid| bots.iter().any(|b| &b.id == bid)).collect();
            if let Err(msg) = self.check_single_team(&members, Some(id)) {
                return Err(std::io::Error::new(std::io::ErrorKind::InvalidData, msg));
            }
        }
        // Разблокировка команды: её участники не должны уже состоять в другой незаблокированной команде.
        if patch.get("archived").and_then(Value::as_bool) == Some(false) {
            let final_members = if let Some(v) = patch.get("botIds").filter(|v| !v.is_null()) {
                string_list(Some(v)).into_iter().filter(|bid| bots.iter().any(|b| &b.id == bid)).collect()
            } else {
                self.room(id).map(|r| r.bot_ids).unwrap_or_default()
            };
            if let Err(msg) = self.check_single_team(&final_members, Some(id)) {
                return Err(std::io::Error::new(std::io::ErrorKind::InvalidData, msg));
            }
        }
        let updated = {
            let mut state = self.state.lock().unwrap();
            let Some(room) = state.rooms.iter_mut().find(|r| r.id == id) else {
                return Ok(None);
            };
            for (key, slot) in [("name", &mut room.name), ("emoji", &mut room.emoji), ("color", &mut room.color), ("instructions", &mut room.instructions)] {
                if let Some(v) = patch.get(key).filter(|v| !v.is_null()) {
                    *slot = as_string(v);
                }
            }
            if let Some(v) = patch.get("botIds").filter(|v| !v.is_null()) {
                room.bot_ids = string_list(Some(v)).into_iter().filter(|id| bots.iter().any(|b| &b.id == id)).collect();
            }
            if let Some(v) = patch.get("lead").filter(|v| !v.is_null()) {
                room.lead = as_string(v);
            }
            fix_room_lead(room);
            if let Some(v) = patch.get("mode").and_then(Value::as_str) {
                room.mode = if v == "auto" { "auto".into() } else { "lead".into() };
            }
            if let Some(v) = patch.get("archived").and_then(Value::as_bool) {
                room.archived = v;
            }
            // Уже проверенные сервером списки (`rooms::check_folders`, `rooms::check_skills`).
            if let Some(v) = patch.get("folders").filter(|v| v.is_array()) {
                room.folders = string_list(Some(v));
                room.program_folders.retain(|p| room.folders.iter().any(|f| f.eq_ignore_ascii_case(p)));
            }
            if let Some(v) = patch.get("programFolders").filter(|v| v.is_array()) {
                let wanted = string_list(Some(v));
                room.program_folders = room.folders.iter().filter(|f| wanted.iter().any(|w| w.eq_ignore_ascii_case(f))).cloned().collect();
            }
            if let Some(v) = patch.get("skills").filter(|v| v.is_array()) {
                room.skills = string_list(Some(v));
            }
            room.clone()
        };
        self.persist().await?;
        Ok(Some(updated))
    }

    /// Миграция «один бот — одна команда»: при загрузке состояния бот, состоящий в нескольких незаблокированных
    /// командах, остаётся в одной (где он ведущий; среди равных — с большим составом, потом старейшая) и удаляется из остальных.
    /// Для каждой покинутой команды в её тред пишется служебная строка. Идемпотентно.
    pub async fn migrate_one_bot_one_team(&self) -> bool {
        #[derive(Clone)]
        struct Act { left_room: String, bot_name: String, keep_room_name: String }
        let mut actions: Vec<Act> = Vec::new();
        let mut changed = false;
        {
            let mut state = self.state.lock().unwrap();
            let bot_ids: Vec<String> = state.bots.iter().map(|b| b.id.clone()).collect();
            for bot_id in bot_ids {
                let rooms: Vec<usize> = state.rooms.iter().enumerate().filter(|(_, r)| !r.archived && r.bot_ids.iter().any(|b| b == &bot_id)).map(|(i, _)| i).collect();
                if rooms.len() <= 1 { continue; }
                // Остаётся команда, где бот ведущий; среди равных — с большим составом, потом старейшая.
                let keep_idx = *rooms
                    .iter()
                    .min_by_key(|&&i| (state.rooms[i].lead != bot_id, std::cmp::Reverse(state.rooms[i].bot_ids.len()), state.rooms[i].created_at))
                    .unwrap();
                let keep_room_name = state.rooms[keep_idx].name.clone();
                let bot_name = state.bots.iter().find(|b| b.id == bot_id).map(|b| b.name.clone()).unwrap_or_else(|| bot_id.clone());
                for &i in &rooms {
                    if i == keep_idx { continue; }
                    let room = &mut state.rooms[i];
                    room.bot_ids.retain(|b| b != &bot_id);
                    fix_room_lead(room);
                    actions.push(Act { left_room: room.id.clone(), bot_name: bot_name.clone(), keep_room_name: keep_room_name.clone() });
                    changed = true;
                }
            }
        }
        if !changed {
            return false;
        }
        let _ = self.persist().await;
        for a in &actions {
            let line = format!("{} перешёл только в команду «{}»: бот может быть только в одной команде.", a.bot_name, a.keep_room_name);
            let entry = serde_json::json!({ "role": "system", "text": line });
            let _ = self.append_room_message(&a.left_room, entry).await;
        }
        true
    }

    pub async fn remove_room(&self, id: &str) -> std::io::Result<bool> {
        {
            let mut state = self.state.lock().unwrap();
            let Some(idx) = state.rooms.iter().position(|r| r.id == id) else {
                return Ok(false);
            };
            state.rooms.remove(idx);
        }
        self.persist().await?;
        let _ = tokio::fs::remove_file(self.room_file(id)).await;
        let _ = tokio::fs::remove_dir_all(self.room_dir(id)).await;
        Ok(true)
    }

    /// Папка файлов команды. Имя берётся только из id существующей команды, не из запроса.
    fn room_dir(&self, room_id: &str) -> PathBuf {
        self.data_dir.join("rooms").join(room_id)
    }

    /// Положить картинку команды. Формат узнаётся по первым байтам, а не по имени или заголовку.
    pub async fn set_room_picture(&self, id: &str, bytes: &[u8]) -> Result<Option<Room>, String> {
        let Some(ext) = picture_ext(bytes) else {
            return Err("нужна картинка png, jpeg или webp".into());
        };
        if bytes.len() > ROOM_PICTURE_MAX {
            return Err("картинка больше 4 МБ".into());
        }
        if self.room(id).is_none() {
            return Ok(None);
        }
        let dir = self.room_dir(id);
        tokio::fs::create_dir_all(&dir).await.map_err(|e| e.to_string())?;
        for other in ["png", "jpg", "webp"] {
            let _ = tokio::fs::remove_file(dir.join(format!("picture.{other}"))).await;
        }
        tokio::fs::write(dir.join(format!("picture.{ext}")), bytes).await.map_err(|e| e.to_string())?;
        let room = {
            let mut state = self.state.lock().unwrap();
            let Some(room) = state.rooms.iter_mut().find(|r| r.id == id) else { return Ok(None) };
            room.picture = ext.into();
            room.picture_at = now();
            room.clone()
        };
        self.persist().await.map_err(|e| e.to_string())?;
        Ok(Some(room))
    }

    pub async fn clear_room_picture(&self, id: &str) -> std::io::Result<Option<Room>> {
        let room = {
            let mut state = self.state.lock().unwrap();
            let Some(room) = state.rooms.iter_mut().find(|r| r.id == id) else { return Ok(None) };
            room.picture.clear();
            room.picture_at = now();
            room.clone()
        };
        let dir = self.room_dir(id);
        for ext in ["png", "jpg", "webp"] {
            let _ = tokio::fs::remove_file(dir.join(format!("picture.{ext}"))).await;
        }
        self.persist().await?;
        Ok(Some(room))
    }

    /// Файл картинки команды и его тип.
    pub async fn room_picture(&self, id: &str) -> Option<(Vec<u8>, &'static str)> {
        let room = self.room(id)?;
        let (ext, mime) = match room.picture.as_str() {
            "png" => ("png", "image/png"),
            "jpg" => ("jpg", "image/jpeg"),
            "webp" => ("webp", "image/webp"),
            _ => return None,
        };
        let bytes = tokio::fs::read(self.room_dir(id).join(format!("picture.{ext}"))).await.ok()?;
        Some((bytes, mime))
    }

    fn room_file(&self, room_id: &str) -> PathBuf {
        self.data_dir.join("rooms").join(format!("{room_id}.jsonl"))
    }

    pub async fn append_room_message(&self, room_id: &str, msg: Value) -> std::io::Result<Value> {
        let mut entry = msg.as_object().cloned().unwrap_or_default();
        entry.entry("id").or_insert_with(|| json!(uid("msg")));
        entry.entry("at").or_insert_with(|| json!(now()));
        let entry = Value::Object(entry);
        let mut line = serde_json::to_string(&entry).map_err(std::io::Error::other)?;
        line.push('\n');
        let mut file = tokio::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(self.room_file(room_id))
            .await?;
        file.write_all(line.as_bytes()).await?;
        file.flush().await?;
        Ok(entry)
    }

    async fn load_room_thread(&self, room_id: &str) -> Vec<Value> {
        let Ok(raw) = tokio::fs::read_to_string(self.room_file(room_id)).await else {
            return Vec::new();
        };
        raw.split('\n').filter(|l| !l.is_empty()).filter_map(|l| serde_json::from_str(l).ok()).collect()
    }

    /// Тред группы для человека: служебные метки (часы, якорь окна) ему не нужны.
    pub async fn read_room_thread(&self, room_id: &str, limit: i64) -> Vec<Value> {
        let all: Vec<Value> = human_view(self.load_room_thread(room_id).await);
        let start = if limit > 0 { all.len().saturating_sub(limit as usize) } else { 0 };
        all[start..].to_vec()
    }

    /// Тред группы для модели: с часами и с тем же якорем окна, что у личного треда.
    pub async fn read_room_model_thread(&self, room_id: &str) -> Vec<Value> {
        model_window(&self.load_room_thread(room_id).await)
    }

    /// Когда модель последний раз видела часы в этой группе, если они ещё в её окне.
    pub async fn last_room_time_context(&self, room_id: &str) -> Option<i64> {
        last_clock(&self.load_room_thread(room_id).await)
    }

    /// Сдвинуть окно истории группы целиком. Смысл тот же, что у `anchor_thread`.
    #[cfg(test)]
    pub async fn anchor_room_thread(&self, room_id: &str, keep: usize, retain: usize) -> bool {
        let Some(plan) = self.plan_compaction(Thread::Room(room_id), keep, retain, false, 0).await else {
            return false;
        };
        self.write_anchor(Thread::Room(room_id), plan.start, None, plan.dropped.len()).await
    }

    pub async fn room_preview(&self, room_id: &str) -> Value {
        for m in self.read_room_thread(room_id, 12).await.iter().rev() {
            let text = m["text"].as_str().unwrap_or_default();
            let text = text.split_whitespace().collect::<Vec<_>>().join(" ");
            if !text.is_empty() {
                let who = m["name"].as_str().unwrap_or_default();
                return json!({ "text": take_chars(&text, 120), "at": m["at"], "role": m["role"], "name": who });
            }
        }
        Value::Null
    }

    // --- MCP ---------------------------------------------------------------

    pub fn mcp_servers(&self) -> Vec<McpServer> {
        self.state.lock().unwrap().mcp.clone()
    }

    pub fn mcp_server(&self, id: &str) -> Option<McpServer> {
        self.state.lock().unwrap().mcp.iter().find(|s| s.id == id).cloned()
    }

    pub async fn add_mcp(&self, input: &Value) -> std::io::Result<McpServer> {
        let command = text_or(input, "command", "").trim().to_string();
        if !command.is_empty() {
            crate::cloud::deny(crate::cloud::NO_STDIO).map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidInput, e))?;
        }
        let url = text_or(input, "url", "").trim().to_string();
        // Без названия показываем то, по чему сервер узнают: хост или команду. Весь адрес нельзя:
        // имя уходит модели и в журнал, а в адресе бывает ключ.
        let title = if url.is_empty() { command.clone() } else { crate::mcp::default_name(&url) };
        let server = McpServer {
            id: uid("mcp"),
            name: take_chars(&text_or(input, "name", &title), 40),
            command,
            url,
            args: string_list(input.get("args")),
            enabled: input.get("enabled").map_or(true, truthy),
            auto_approve: input.get("autoApprove").is_some_and(truthy),
            created_at: now(),
            env: string_map(input.get("env")),
            headers: string_map(input.get("headers")),
            connector: None,
            login: None,
        };
        self.state.lock().unwrap().mcp.push(server.clone());
        self.persist().await?;
        Ok(server)
    }

    /// Положить сервер целиком: новый добавляется, тот же id заменяется на месте.
    pub async fn put_mcp(&self, server: McpServer) -> std::io::Result<()> {
        if !server.command.trim().is_empty() {
            crate::cloud::deny(crate::cloud::NO_STDIO).map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidInput, e))?;
        }
        {
            let mut state = self.state.lock().unwrap();
            match state.mcp.iter_mut().find(|s| s.id == server.id) {
                Some(slot) => *slot = server,
                None => state.mcp.push(server),
            }
        }
        self.persist().await
    }

    /// Поправить сервер на месте. None — такого нет.
    pub async fn edit_mcp(&self, id: &str, change: impl FnOnce(&mut McpServer)) -> std::io::Result<Option<McpServer>> {
        let updated = {
            let mut state = self.state.lock().unwrap();
            let Some(server) = state.mcp.iter_mut().find(|s| s.id == id) else {
                return Ok(None);
            };
            change(server);
            server.clone()
        };
        self.persist().await?;
        Ok(Some(updated))
    }

    pub async fn update_mcp(&self, id: &str, patch: &Value) -> std::io::Result<Option<McpServer>> {
        let updated = {
            let mut state = self.state.lock().unwrap();
            let Some(server) = state.mcp.iter_mut().find(|s| s.id == id) else {
                return Ok(None);
            };
            if let Some(v) = patch.get("name").filter(|v| !v.is_null()) {
                server.name = as_string(v);
            }
            // Смена транспорта одним полем: непустой адрес убирает команду и наоборот.
            // Оба сразу не придут: такую комбинацию отклоняет API.
            if let Some(v) = patch.get("url").filter(|v| !v.is_null()) {
                let url = as_string(v).trim().to_string();
                if url.is_empty() {
                    server.url.clear();
                } else {
                    server.url = url;
                    server.command.clear();
                    server.args.clear();
                }
            }
            if let Some(v) = patch.get("command").filter(|v| !v.is_null()) {
                let command = as_string(v).trim().to_string();
                if command.is_empty() {
                    server.command.clear();
                } else {
                    server.command = command;
                    server.url.clear();
                }
            }
            if let Some(v) = patch.get("args").filter(|v| !v.is_null()) {
                server.args = string_list(Some(v));
            }
            if let Some(v) = patch.get("enabled").filter(|v| !v.is_null()) {
                server.enabled = truthy(v);
            }
            if let Some(v) = patch.get("autoApprove").filter(|v| !v.is_null()) {
                server.auto_approve = truthy(v);
            }
            if let Some(v) = patch.get("env").filter(|v| !v.is_null()) {
                server.env = string_map(Some(v));
            }
            if let Some(v) = patch.get("headers").filter(|v| !v.is_null()) {
                server.headers = string_map(Some(v));
            }
            server.clone()
        };
        self.persist().await?;
        Ok(Some(updated))
    }

    pub async fn remove_mcp(&self, id: &str) -> std::io::Result<bool> {
        {
            let mut state = self.state.lock().unwrap();
            let Some(idx) = state.mcp.iter().position(|s| s.id == id) else {
                return Ok(false);
            };
            state.mcp.remove(idx);
        }
        self.persist().await?;
        Ok(true)
    }

    /// Рутина по идентификатору и секрету из адреса вебхука.
    pub fn routine_by_hook(&self, id: &str, token: &str) -> Option<Routine> {
        let state = self.state.lock().unwrap();
        state
            .routines
            .iter()
            .find(|r| r.id == id && r.hook && !r.hook_token.is_empty() && constant_time_eq(&r.hook_token, token))
            .cloned()
    }
}

/// Сравнение секретов без раннего выхода: время не подсказывает, сколько символов совпало.
fn constant_time_eq(a: &str, b: &str) -> bool {
    a.len() == b.len() && a.bytes().zip(b.bytes()).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

/// Проверка имени события GitHub: `push` или `pull_request.opened`.
pub(crate) fn valid_hook_event(name: &str) -> bool {
    let mut parts = name.split('.');
    let Some(first) = parts.next() else { return false; };
    if first.is_empty() || !first.bytes().all(|b| b.is_ascii_lowercase() || b == b'_') {
        return false;
    }
    if let Some(second) = parts.next() {
        if second.is_empty() || !second.bytes().all(|b| b.is_ascii_lowercase() || b == b'_') {
            return false;
        }
        if parts.next().is_some() {
            return false;
        }
    }
    true
}

pub(crate) fn validate_hook_events(value: &Value) -> Result<Vec<String>, String> {
    let items = value.as_array().ok_or("hookEvents должен быть списком строк")?;
    if items.len() > 20 {
        return Err("hookEvents: не больше 20 событий".into());
    }
    let mut out = Vec::with_capacity(items.len());
    for item in items {
        let name = item.as_str().ok_or("hookEvents: каждое событие — строка")?;
        if !valid_hook_event(name) {
            return Err(format!("hookEvents: недопустимое имя события «{name}»"));
        }
        out.push(name.to_string());
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(name: &str) -> PathBuf {
        let stamp = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos();
        std::env::temp_dir().join(format!("deloist-{name}-{stamp}"))
    }

    // Окно истории должно двигаться рывком и редко: сдвиг на сообщение за ход
    // меняет начало запроса и выбрасывает кэш провайдера на каждом ходе.
    #[tokio::test]
    async fn the_history_window_moves_in_one_jump_and_keeps_the_tail() {
        let dir = temp_dir("anchor");
        let store = Store::open(dir.clone()).await.unwrap();
        for i in 0..5 {
            store.append_message("bot", json!({ "role": "user", "text": format!("m{i}") })).await.unwrap();
        }
        assert!(!store.anchor_thread("bot", 8, 4).await, "пока не перебрали — якорь не нужен");

        for i in 5..9 {
            store.append_message("bot", json!({ "role": "user", "text": format!("m{i}") })).await.unwrap();
        }
        assert!(store.anchor_thread("bot", 8, 4).await, "перебрали окно — пора сдвинуть");

        let model = store.read_model_thread("bot").await;
        assert_eq!(model.len(), 4, "в окно остаются последние четыре записи");
        assert_eq!(model[0]["text"], "m5");
        assert_eq!(model[3]["text"], "m8");

        // Человек по-прежнему видит весь тред: якорь — только для модели.
        let shown = store.read_thread("bot", 100).await;
        assert_eq!(shown.len(), 9);
        assert!(shown.iter().all(|m| m["role"] != "anchor"), "якорь не показывается");

        // Второй якорь не появляется, пока окно снова не переполнится.
        assert!(!store.anchor_thread("bot", 8, 4).await);
        let _ = tokio::fs::remove_dir_all(dir).await;
    }

    #[tokio::test]
    async fn a_full_path_inside_the_own_workspace_is_the_same_file() {
        let dir = temp_dir("safe-abs");
        let store = Store::open(dir.clone()).await.unwrap();
        let own = store.workspace("bot_a").join("pic.png");
        assert_eq!(store.safe_path("bot_a", &own.display().to_string()).unwrap().file_name(), own.file_name());
        let foreign = store.workspace("bot_b").join("pic.png").display().to_string();
        assert!(store.safe_path("bot_a", &foreign).is_err(), "чужая папка остаётся снаружи");
        let escape = store.workspace("bot_a").join("..").join("bot_b").join("pic.png").display().to_string();
        assert!(store.safe_path("bot_a", &escape).is_err(), "выход через .. из своей папки");
        let _ = tokio::fs::remove_dir_all(dir).await;
    }

    // Часы — служебная запись: модель их видит, человек нет.
    #[tokio::test]
    async fn the_clock_entry_is_visible_only_to_the_model() {
        let dir = temp_dir("clock");
        let store = Store::open(dir.clone()).await.unwrap();
        store.append_message("bot", json!({ "role": "time", "text": "Время подготовки хода: 2026-11-07 10:00 +07:00.", "at": 1 })).await.unwrap();
        store.append_message("bot", json!({ "role": "user", "text": "привет" })).await.unwrap();

        assert_eq!(store.read_thread("bot", 10).await.len(), 1, "часы не показываем человеку");
        assert_eq!(store.read_model_thread("bot").await.len(), 2, "модель получает часы");
        assert_eq!(store.last_time_context("bot").await, Some(1));
        let _ = tokio::fs::remove_dir_all(dir).await;
    }

    // Серия ошибок: после 3 рутина идёт реже, три «вечные» ставят её на паузу, один успех возвращает всё.
    #[tokio::test]
    async fn failed_runs_slow_a_routine_and_lasting_ones_pause_it() {
        use crate::runs::StopReason::{Check, Login, Network};
        let dir = temp_dir("routine-failures");
        let store = Store::open(dir.clone()).await.unwrap();
        let r = store.add_routine(&json!({ "botId": "bot", "title": "Срез", "prompt": "собери", "everyMinutes": 10 })).await.unwrap();

        store.mark_routine_run(&r.id, true).await.unwrap();
        assert_eq!(store.routine(&r.id).unwrap().last_status.as_deref(), Some("running"), "до конца хода — не «готово»");

        for n in 1..=2 {
            assert_eq!(store.finish_routine_run(&r.id, "failed", Some(Network)).await.unwrap(), RoutineOutcome::Failed { failures: n });
        }
        let saved = store.routine(&r.id).unwrap();
        assert!(saved.next_run < saved.last_run + 20 * 60_000, "две ошибки расписание не меняют");
        assert_eq!(store.finish_routine_run(&r.id, "failed", Some(Network)).await.unwrap(), RoutineOutcome::Failed { failures: 3 });
        let saved = store.routine(&r.id).unwrap();
        assert!(saved.next_run >= saved.last_run + 3_600_000, "после трёх — не чаще раза в час");
        assert!(!saved.error_paused, "сеть пройдёт сама: паузы нет");
        for _ in 0..3 {
            store.finish_routine_run(&r.id, "failed", Some(Network)).await.unwrap();
        }
        let saved = store.routine(&r.id).unwrap();
        assert!(saved.next_run >= saved.last_run + 4 * 3_600_000, "после шести — раз в 4 часа");

        // Пауза и «стоп» человеком серию не трогают, успех сбрасывает.
        assert_eq!(store.finish_routine_run(&r.id, "paused", Some(Network)).await.unwrap(), RoutineOutcome::Done);
        assert_eq!(store.finish_routine_run(&r.id, "needs", Some(crate::runs::StopReason::Limit)).await.unwrap(), RoutineOutcome::Done);
        assert_eq!(store.finish_routine_run(&r.id, "stopped", None).await.unwrap(), RoutineOutcome::Done);
        assert_eq!(store.routine(&r.id).unwrap().failures, 6);
        assert_eq!(store.finish_routine_run(&r.id, "ok", None).await.unwrap(), RoutineOutcome::Done);
        let saved = store.routine(&r.id).unwrap();
        assert_eq!((saved.failures, saved.stop_reason), (0, None), "выполнено — серия сброшена");

        // Вход и проверка — «вечные»: третья подряд ставит на паузу, сообщение одно.
        assert_eq!(store.finish_routine_run(&r.id, "needs", Some(Login)).await.unwrap(), RoutineOutcome::Failed { failures: 1 });
        assert_eq!(store.finish_routine_run(&r.id, "failed", Some(Check)).await.unwrap(), RoutineOutcome::Failed { failures: 2 });
        assert_eq!(store.finish_routine_run(&r.id, "failed", Some(Check)).await.unwrap(), RoutineOutcome::PausedByErrors { failures: 3 });
        assert_eq!(store.finish_routine_run(&r.id, "failed", Some(Check)).await.unwrap(), RoutineOutcome::Failed { failures: 4 });
        let saved = store.routine(&r.id).unwrap();
        assert!(saved.error_paused && saved.enabled, "рутина на паузе, но не выключена");
        let resumed = store.update_routine(&r.id, &json!({ "errorPaused": false })).await.unwrap().unwrap();
        assert_eq!((resumed.error_paused, resumed.failures), (false, 0));
        let _ = std::fs::remove_dir_all(dir);
    }

    // Ночь тихих рутин не должна стоить боту памяти о живом разговоре.
    #[tokio::test]
    async fn quiet_routines_do_not_push_the_conversation_out_of_the_window() {
        let dir = temp_dir("quiet-window");
        let store = Store::open(dir.clone()).await.unwrap();
        for i in 0..4 {
            store.append_message("bot", json!({ "role": "user", "text": format!("m{i}") })).await.unwrap();
        }
        for n in 0..50 {
            let turn = format!("turn{n}");
            store.append_message("bot", json!({ "role": "time", "text": "часы", "turnId": turn, "at": 5 })).await.unwrap();
            store.append_message("bot", json!({ "role": "user", "text": "рутина", "turnId": turn })).await.unwrap();
            store.append_message("bot", json!({ "role": "assistant", "text": "[ТИХО]", "turnId": turn })).await.unwrap();
            store.append_message("bot", json!({ "role": "quiet", "turnId": turn })).await.unwrap();
        }
        assert!(!store.anchor_thread("bot", 8, 4).await, "тихие записи окно не переполняют");
        let model = store.read_model_thread("bot").await;
        assert_eq!(model.len(), 4, "разговор на месте: {model:?}");
        assert_eq!(store.last_time_context("bot").await, None, "часы тихого хода модель не видела");

        // И когда окно всё же сдвигается, в нём остаются живые записи, а не тихие.
        for i in 4..9 {
            store.append_message("bot", json!({ "role": "user", "text": format!("m{i}") })).await.unwrap();
        }
        assert!(store.anchor_thread("bot", 8, 4).await);
        let texts: Vec<Value> = store.read_model_thread("bot").await.iter().map(|m| m["text"].clone()).collect();
        assert_eq!(texts, vec![json!("m5"), json!("m6"), json!("m7"), json!("m8")]);
        let _ = tokio::fs::remove_dir_all(dir).await;
    }

    // После якоря такое начало держалось бы десятки ходов, поэтому окно начинается с человека.
    #[tokio::test]
    async fn the_window_starts_with_a_human_message_when_it_can() {
        let dir = temp_dir("anchor-start");
        let store = Store::open(dir.clone()).await.unwrap();
        for i in 0..5 {
            store.append_message("bot", json!({ "role": "user", "text": format!("q{i}") })).await.unwrap();
            store.append_message("bot", json!({ "role": "assistant", "text": format!("a{i}") })).await.unwrap();
        }
        assert!(store.anchor_thread("bot", 8, 4).await);
        let model = store.read_model_thread("bot").await;
        assert_eq!(model[0]["text"], "q3", "первая запись окна — реплика человека: {model:?}");
        let _ = tokio::fs::remove_dir_all(dir).await;
    }

    // Часы, выпавшие из окна, модель уже не видит: их надо показать заново.
    #[tokio::test]
    async fn a_clock_outside_the_window_does_not_count() {
        let dir = temp_dir("anchor-clock");
        let store = Store::open(dir.clone()).await.unwrap();
        store.append_message("bot", json!({ "role": "time", "text": "часы", "at": 7 })).await.unwrap();
        for i in 0..9 {
            store.append_message("bot", json!({ "role": "user", "text": format!("m{i}") })).await.unwrap();
        }
        assert_eq!(store.last_time_context("bot").await, Some(7));
        assert!(store.anchor_thread("bot", 8, 4).await);
        assert_eq!(store.last_time_context("bot").await, None);
        let _ = tokio::fs::remove_dir_all(dir).await;
    }

    // Сводка живёт в якоре: модель видит её вместо выпавшего, человек — разделителем.
    #[tokio::test]
    async fn a_summary_rides_on_the_anchor_and_carries_over() {
        let dir = temp_dir("summary");
        let store = Store::open(dir.clone()).await.unwrap();
        for i in 0..5 {
            store.append_message("bot", json!({ "role": "user", "text": format!("q{i}") })).await.unwrap();
            store.append_message("bot", json!({ "role": "assistant", "text": format!("a{i}") })).await.unwrap();
        }
        let plan = store.plan_compaction(Thread::Bot("bot"), 8, 4, false, 0).await.expect("окно переполнено");
        assert_eq!(plan.summary, None, "сводки ещё не было");
        assert_eq!(plan.dropped.len(), 6, "выпадает всё до q3: {:?}", plan.dropped);
        assert!(store.write_anchor(Thread::Bot("bot"), plan.start, Some("сводка-1"), plan.dropped.len()).await);
        assert_eq!(store.thread_summary(Thread::Bot("bot")).await.as_deref(), Some("сводка-1"));
        assert_eq!(store.read_model_thread("bot").await[0]["text"], "q3");

        let shown = store.read_thread("bot", 100).await;
        let divider: Vec<&Value> = shown.iter().filter(|m| m["role"] == "compacted").collect();
        assert_eq!(divider.len(), 1, "человек видит разделитель");
        assert_eq!(divider[0]["text"], "сводка-1");
        assert_eq!(divider[0]["dropped"], 6);

        // Следующий сдвиг получает прежнюю сводку, чтобы свернуть её вместе с новым куском.
        for i in 5..9 {
            store.append_message("bot", json!({ "role": "user", "text": format!("q{i}") })).await.unwrap();
            store.append_message("bot", json!({ "role": "assistant", "text": format!("a{i}") })).await.unwrap();
        }
        let next = store.plan_compaction(Thread::Bot("bot"), 8, 4, false, 0).await.unwrap();
        assert_eq!(next.summary.as_deref(), Some("сводка-1"));
        assert!(next.dropped.iter().all(|m| m["text"] != "q0"), "выпавшее раньше второй раз не сворачиваем");
        let _ = tokio::fs::remove_dir_all(dir).await;
    }

    // Длинные ответы переполняют окно раньше счётчика записей; по просьбе сжимаем и без переполнения.
    #[tokio::test]
    async fn size_and_request_trigger_compaction_and_keep_the_last_human_turn() {
        let dir = temp_dir("summary-size");
        let store = Store::open(dir.clone()).await.unwrap();
        for i in 0..3 {
            store.append_message("bot", json!({ "role": "user", "text": format!("q{i}") })).await.unwrap();
            store.append_message("bot", json!({ "role": "assistant", "text": "x".repeat(1000) })).await.unwrap();
        }
        assert!(store.plan_compaction(Thread::Bot("bot"), 90, 60, false, 10_000).await.is_none(), "влезает");
        let plan = store.plan_compaction(Thread::Bot("bot"), 90, 60, false, 2_000).await.expect("не влезает");
        let start = &store.load_thread("bot").await[plan.start];
        assert_eq!(start["role"], "user", "окно начинается с человека");
        assert_eq!(start["text"], "q2");
        assert!(store.plan_compaction(Thread::Bot("bot"), 0, 0, true, 0).await.is_some(), "по просьбе");

        // Одна реплика человека — сжимать нечего.
        let lone = temp_dir("summary-lone");
        let other = Store::open(lone.clone()).await.unwrap();
        other.append_message("bot", json!({ "role": "user", "text": "привет" })).await.unwrap();
        assert!(other.plan_compaction(Thread::Bot("bot"), 0, 0, true, 0).await.is_none());
        let _ = tokio::fs::remove_dir_all(dir).await;
        let _ = tokio::fs::remove_dir_all(lone).await;
    }

    #[tokio::test]
    async fn a_room_window_moves_by_anchor_and_hides_it_from_the_human() {
        let dir = temp_dir("room-anchor");
        let store = Store::open(dir.clone()).await.unwrap();
        for i in 0..9 {
            store.append_room_message("room", json!({ "role": "user", "text": format!("m{i}") })).await.unwrap();
        }
        assert!(store.anchor_room_thread("room", 8, 4).await);
        assert!(!store.anchor_room_thread("room", 8, 4).await, "второй якорь сразу не нужен");
        assert_eq!(store.read_room_model_thread("room").await.len(), 4);
        let shown = store.read_room_thread("room", 100).await;
        assert_eq!(shown.len(), 9);
        assert!(shown.iter().all(|m| m["role"] != "anchor"), "якорь не показывается");
        let _ = tokio::fs::remove_dir_all(dir).await;
    }

    fn contains(hay: &[u8], needle: &str) -> bool {
        hay.windows(needle.len()).any(|w| w == needle.as_bytes())
    }

    #[cfg(windows)]
    #[tokio::test]
    async fn a_plain_secrets_file_is_sealed_on_first_load_and_keeps_every_key() {
        let dir = temp_dir("secrets-migrate");
        std::fs::create_dir_all(&dir).unwrap();
        let plain = json!({ "accountToken": "tok-PLAIN-42", "accountEmail": "me@deloist.ru", "providerKeys": { "openrouter": "sk-or-PLAIN" } });
        std::fs::write(dir.join("secrets.json"), plain.to_string()).unwrap();

        let store = Store::open(dir.clone()).await.unwrap();
        assert_eq!(store.account_token().as_deref(), Some("tok-PLAIN-42"));
        let raw = std::fs::read(dir.join("secrets.json")).unwrap();
        assert!(raw.starts_with(crate::secrets::SEALED_MAGIC), "файл зашифрован");
        assert!(!contains(&raw, "tok-PLAIN-42") && !contains(&raw, "sk-or-PLAIN") && !contains(&raw, "deloist.ru"), "на диске нет открытых значений");
        assert!(!dir.join("secrets.json.migrating").exists(), "копия убрана после успешного перешифрования");
        assert!(!dir.join("secrets.json.tmp").exists());
        drop(store);

        // Повторный запуск читает зашифрованный файл, запись тоже идёт зашифрованной.
        let store = Store::open(dir.clone()).await.unwrap();
        assert_eq!(store.account(), Some(("me@deloist.ru".to_string(), String::new())));
        store.set_own_key("openrouter", "sk-or-NEW").await.unwrap();
        let raw = std::fs::read(dir.join("secrets.json")).unwrap();
        assert!(raw.starts_with(crate::secrets::SEALED_MAGIC) && !contains(&raw, "sk-or-NEW"));
        drop(store);
        let store = Store::open(dir.clone()).await.unwrap();
        assert_eq!(store.secrets.lock().unwrap().provider_keys.get("openrouter").map(String::as_str), Some("sk-or-NEW"));
        let _ = tokio::fs::remove_dir_all(dir).await;
    }

    #[cfg(windows)]
    #[tokio::test]
    async fn a_run_cut_short_mid_migration_recovers_from_the_copy() {
        let dir = temp_dir("secrets-recover");
        std::fs::create_dir_all(&dir).unwrap();
        let plain = json!({ "accountToken": "tok-COPY" }).to_string();
        // Обрыв: копия уже есть, а на месте основного — мусор от прерванной записи.
        std::fs::write(dir.join("secrets.json.migrating"), &plain).unwrap();
        std::fs::write(dir.join("secrets.json"), b"\x00\x01garbage").unwrap();
        let store = Store::open(dir.clone()).await.unwrap();
        assert_eq!(store.account_token().as_deref(), Some("tok-COPY"));
        let raw = std::fs::read(dir.join("secrets.json")).unwrap();
        assert!(raw.starts_with(crate::secrets::SEALED_MAGIC));
        assert!(!dir.join("secrets.json.migrating").exists());
        let _ = tokio::fs::remove_dir_all(dir).await;
    }

    #[tokio::test]
    async fn an_undecryptable_sealed_file_stays_unchanged_blocks_saves_and_shows_unavailable() {
        let dir = temp_dir("secrets-foreign");
        std::fs::create_dir_all(&dir).unwrap();
        // Блоб с чужой добавкой: расшифровка не пройдёт — как файл другого пользователя Windows.
        let mut foreign = crate::secrets::SEALED_MAGIC.to_vec();
        foreign.extend_from_slice(b"not-a-dpapi-blob");
        std::fs::write(dir.join("secrets.json"), &foreign).unwrap();
        let store = Store::open(dir.clone()).await.unwrap();
        assert!(store.account_token().is_none());
        assert!(store.secrets_unavailable(), "состояние «ключи недоступны»");
        assert_eq!(std::fs::read(dir.join("secrets.json")).unwrap(), foreign, "файл на диске без изменений");
        assert!(store.set_own_key("openrouter", "sk-or-LOST").await.is_err(), "запись отказана");
        let names: Vec<String> = std::fs::read_dir(&dir).unwrap().flatten().map(|e| e.file_name().to_string_lossy().to_string()).collect();
        assert!(!names.iter().any(|n| n.starts_with("secrets.json.")), "ничего не отложено и не добавлено: {names:?}");
        drop(store);
        // Повторный запуск тоже ничего не откладывает.
        let again = Store::open(dir.clone()).await.unwrap();
        assert!(again.secrets_unavailable());
        assert_eq!(std::fs::read(dir.join("secrets.json")).unwrap(), foreign);
        let _ = tokio::fs::remove_dir_all(dir).await;
    }

    #[tokio::test]
    async fn only_the_reset_moves_the_unreadable_file_aside_and_keeps_the_copy() {
        let dir = temp_dir("secrets-reset");
        std::fs::create_dir_all(&dir).unwrap();
        let mut foreign = crate::secrets::SEALED_MAGIC.to_vec();
        foreign.extend_from_slice(b"not-a-dpapi-blob");
        std::fs::write(dir.join("secrets.json"), &foreign).unwrap();
        // Старые отложенные копии: чистка при запуске их урезает, а копию после сброса — нет.
        let store = Store::open(dir.clone()).await.unwrap();
        assert!(store.reset_unavailable_secrets().await.unwrap(), "файл отложен");
        assert!(!store.secrets_unavailable());
        assert!(!dir.join("secrets.json").exists());
        let aside: Vec<_> = std::fs::read_dir(&dir).unwrap().flatten().filter(|e| e.file_name().to_string_lossy().starts_with("secrets.json.bad-")).collect();
        assert_eq!(aside.len(), 1);
        assert_eq!(std::fs::read(aside[0].path()).unwrap(), foreign, "копия цела");
        store.set_own_key("openrouter", "sk-or-NEW").await.unwrap();
        assert!(!store.reset_unavailable_secrets().await.unwrap(), "второй сброс ничего не делает");
        drop(store);
        // Следующий запуск (с чисткой старых копий) копию сохраняет.
        let _ = Store::open(dir.clone()).await.unwrap();
        assert!(aside[0].path().exists(), "чистка не тронула копию");
        let _ = tokio::fs::remove_dir_all(dir).await;
    }

    #[tokio::test]
    async fn an_unreadable_secrets_file_blocks_writes_and_survives() {
        let dir = temp_dir("secrets-unreadable");
        std::fs::create_dir_all(dir.join("secrets.json")).unwrap();
        std::fs::write(dir.join("secrets.json").join("marker"), b"x").unwrap();
        let store = Store::open(dir.clone()).await.unwrap();
        assert!(store.account_token().is_none(), "в памяти пусто");
        assert!(store.set_own_key("openrouter", "sk-or-LOST").await.is_err(), "запись отказана");
        assert!(dir.join("secrets.json").is_dir() && dir.join("secrets.json").join("marker").exists(), "файл на месте");
        let aside = std::fs::read_dir(&dir).unwrap().flatten().filter(|e| e.file_name().to_string_lossy().starts_with("secrets.json.bad-")).count();
        assert_eq!(aside, 0, "нечитаемое не откладывается и не затирается");
        let _ = tokio::fs::remove_dir_all(dir).await;
    }

    #[tokio::test]
    async fn a_missing_secrets_file_is_just_empty_and_writable() {
        let dir = temp_dir("secrets-missing");
        let store = Store::open(dir.clone()).await.unwrap();
        assert!(store.account_token().is_none());
        assert!(!store.secrets_unavailable.load(std::sync::atomic::Ordering::SeqCst));
        let _ = tokio::fs::remove_dir_all(dir).await;
    }

    #[test]
    fn only_the_three_newest_aside_files_are_kept() {
        let dir = temp_dir("secrets-prune");
        std::fs::create_dir_all(&dir).unwrap();
        for stamp in [100, 200, 300, 400, 500] {
            std::fs::write(dir.join(format!("secrets.json.bad-{stamp}")), b"x").unwrap();
        }
        std::fs::write(dir.join("other.txt"), b"x").unwrap();
        let (_, unavailable) = load_secrets(&dir);
        assert!(!unavailable);
        let mut left: Vec<String> = std::fs::read_dir(&dir).unwrap().flatten().map(|e| e.file_name().to_string_lossy().to_string()).collect();
        left.sort();
        assert_eq!(left, ["other.txt", "secrets.json.bad-300", "secrets.json.bad-400", "secrets.json.bad-500"]);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[cfg(windows)]
    #[tokio::test]
    async fn migration_keeps_the_original_bytes_in_the_v1_copy() {
        let dir = temp_dir("secrets-v1");
        std::fs::create_dir_all(&dir).unwrap();
        let original = br#"{ "accountToken": "tok-V1", "futureField": "kept-here" }"#;
        std::fs::write(dir.join("secrets.json"), original).unwrap();
        let store = Store::open(dir.clone()).await.unwrap();
        assert_eq!(store.account_token().as_deref(), Some("tok-V1"));
        assert_eq!(std::fs::read(dir.join("secrets.json.v1")).unwrap(), original, "байты в точности прежние");
        assert!(std::fs::read(dir.join("secrets.json")).unwrap().starts_with(crate::secrets::SEALED_MAGIC));
        drop(store);
        // Повторный запуск и запись не трогают копию.
        let store = Store::open(dir.clone()).await.unwrap();
        store.set_own_key("openrouter", "sk-or-2").await.unwrap();
        assert_eq!(std::fs::read(dir.join("secrets.json.v1")).unwrap(), original);
        let _ = tokio::fs::remove_dir_all(dir).await;
    }

    #[cfg(windows)]
    #[tokio::test]
    async fn a_leftover_migrating_copy_is_removed_on_the_next_write() {
        let dir = temp_dir("secrets-leftover");
        std::fs::create_dir_all(&dir).unwrap();
        let store = Store::open(dir.clone()).await.unwrap();
        std::fs::write(dir.join("secrets.json.migrating"), b"{}").unwrap();
        store.set_own_key("openrouter", "sk-or-3").await.unwrap();
        assert!(!dir.join("secrets.json.migrating").exists());
        let _ = tokio::fs::remove_dir_all(dir).await;
    }

    #[tokio::test]
    async fn old_state_without_last_seen_version_loads() {
        let dir = temp_dir("old-state");
        std::fs::create_dir_all(&dir).unwrap();
        let state = json!({ "settings": { "driver": "demo" } });
        std::fs::write(dir.join("state.json"), serde_json::to_string(&state).unwrap()).unwrap();
        let store = Store::open(dir.clone()).await.unwrap();
        assert_eq!(store.settings().last_seen_version, "");
        store.update_settings(&json!({ "lastSeenVersion": "0.3.1" })).await.unwrap();
        assert_eq!(store.settings().last_seen_version, "0.3.1");
        let _ = tokio::fs::remove_dir_all(dir).await;
    }

    #[tokio::test]
    async fn an_old_bot_without_level_or_effort_loads_unchanged() {
        let dir = temp_dir("old-bot-level");
        std::fs::create_dir_all(&dir).unwrap();
        let state = json!({
            "bots": [{ "id": "bot_old", "name": "Аня", "driver": "api", "model": "old-model" }],
        });
        std::fs::write(dir.join("state.json"), serde_json::to_string(&state).unwrap()).unwrap();
        let store = Store::open(dir.clone()).await.unwrap();
        let bot = store.bot("bot_old").expect("старый бот загрузился");
        assert_eq!(bot.model, "old-model");
        assert!(bot.extra.get("modelLevel").is_none(), "нового поля нет — и не появляется");
        assert!(bot.extra.get("effort").is_none());
        let _ = tokio::fs::remove_dir_all(dir).await;
    }

    // Старый state.json без полей ранга и резерва читается с прежними значениями.
    #[tokio::test]
    async fn an_old_bot_gets_lead_rank_and_default_preferences() {
        let dir = temp_dir("old-bot-rank");
        std::fs::create_dir_all(&dir).unwrap();
        let state = json!({
            "bots": [{ "id": "bot_old", "name": "Аня", "role": "аналитик" }],
        });
        std::fs::write(dir.join("state.json"), serde_json::to_string(&state).unwrap()).unwrap();
        let store = Store::open(dir.clone()).await.unwrap();
        let bot = store.bot("bot_old").expect("старый бот загрузился");
        assert_eq!(bot.rank, "lead");
        assert!(!bot.pinned);
        assert_eq!(bot.team, "");
        assert_eq!(bot.reserve_after_days, 7);
        assert_eq!(bot.last_active_at, 0);
        assert_eq!(bot_guests(&bot), Vec::<Value>::new());
        assert!(!bot.in_reserve(now()));
        let _ = tokio::fs::remove_dir_all(dir).await;
    }

    // Старый state.json без поля архива читается как обычная, живая группа.
    #[test]
    fn an_old_room_without_archived_loads_as_active() {
        let room: Room = serde_json::from_str(
            r##"{"id":"room_old","name":"Совет","emoji":"","color":"#7a8595","instructions":"","botIds":[],"lead":"","mode":"lead","createdAt":1}"##,
        )
        .expect("старая группа загрузилась");
        assert!(!room.archived, "старая группа не в архиве");
    }

    #[test]
    fn bot_preferences_are_validated_before_saving() {
        for bad in [
            json!({ "rank": "boss" }),
            json!({ "rank": 3 }),
            json!({ "pinned": "yes" }),
            json!({ "team": "я".repeat(41) }),
            json!({ "reserveAfterDays": -1 }),
            json!({ "reserveAfterDays": "7" }),
            json!({ "removeGuest": "" }),
        ] {
            let message = check_bot_patch(&bad).expect_err("плохое значение прошло проверку");
            assert!(message.chars().any(|c| c.is_ascii_alphabetic()), "сообщение без имени поля: {message}");
        }
        assert!(check_bot_patch(&json!({
            "rank": "specialist", "pinned": true, "team": "Разработка", "reserveAfterDays": 0, "removeGuest": "bot_1"
        }))
        .is_ok());
    }

    #[test]
    fn in_reserve_needs_a_specialist_that_is_not_pinned_and_has_been_idle() {
        let at = now();
        let mut bot = Bot { rank: "specialist".into(), last_active_at: at - 8 * DAY_MS, ..Bot::default() };
        assert!(bot.in_reserve(at));
        bot.pinned = true;
        assert!(!bot.in_reserve(at), "закреплённый не уходит в резерв");
        bot.pinned = false;
        bot.reserve_after_days = 0;
        assert!(!bot.in_reserve(at), "0 — никогда");
        bot.reserve_after_days = 7;
        bot.last_active_at = at - DAY_MS;
        assert!(!bot.in_reserve(at), "свежая активность держит в строю");
        bot.rank = "lead".into();
        bot.last_active_at = at - 8 * DAY_MS;
        assert!(!bot.in_reserve(at), "лид не уходит в резерв");
    }

    // Гость живёт в extra.guests хозяина, повторный вызов обновляет since, старые вытесняются.
    #[tokio::test]
    async fn guests_are_capped_expire_and_can_be_removed_by_patch() {
        let dir = temp_dir("guests");
        let store = Store::open(dir.clone()).await.unwrap();
        let host = store.add_bot(&json!({ "name": "Хозяин" })).await.unwrap();
        let mut guests = Vec::new();
        for i in 0..9 {
            guests.push(store.add_bot(&json!({ "name": format!("Гость {i}") })).await.unwrap());
        }
        for guest in &guests {
            assert!(store.add_guest(&host.id, &guest.id).await.unwrap());
        }
        let kept = bot_guests(&store.bot(&host.id).unwrap());
        assert_eq!(kept.len(), MAX_GUESTS, "больше восьми гостей не держим");
        assert_eq!(kept[0]["botId"], guests[1].id.as_str(), "самый старый вытеснен");
        assert_eq!(kept[MAX_GUESTS - 1]["botId"], guests[8].id.as_str());

        // Простой дольше reserveAfterDays убирает гостя.
        let idle = guests[8].id.clone();
        let old = now() - 8 * DAY_MS;
        store.edit_bot(&idle, |bot| bot.last_active_at = old).await.unwrap();
        store.prune_guests().await.unwrap();
        let pruned = bot_guests(&store.bot(&host.id).unwrap());
        assert_eq!(pruned.len(), MAX_GUESTS - 1);
        assert!(pruned.iter().all(|guest| guest["botId"] != idle.as_str()), "просроченный гость остался");

        // removeGuest убирает гостя сразу.
        let gone = guests[7].id.clone();
        let updated = store.update_bot(&host.id, &json!({ "removeGuest": gone })).await.unwrap().unwrap();
        assert!(bot_guests(&updated).iter().all(|guest| guest["botId"] != gone.as_str()));
        let _ = tokio::fs::remove_dir_all(dir).await;
    }

    #[tokio::test]
    async fn check_single_team_rejects_busy_bot() {
        let dir = temp_dir("check-single-team");
        let store = Store::open(dir.clone()).await.unwrap();
        let b1 = store.add_bot(&json!({ "name": "Аня" })).await.unwrap();
        let b2 = store.add_bot(&json!({ "name": "Боря" })).await.unwrap();
        let r1 = store.add_room(&json!({ "name": "R1", "botIds": [b1.id.clone()], "lead": b1.id.clone() })).await.unwrap();
        // b1 уже в команде
        assert!(store.check_single_team(&[b1.id.clone()], None).is_err());
        // добавление его во вторую команду не проходит
        let res = store.add_room(&json!({ "name": "R2", "botIds": [b1.id.clone()], "lead": b1.id.clone() })).await;
        assert!(res.is_err(), "занятый бот не должен создать вторую команду");
        // свободный бот проходит
        assert!(store.check_single_team(&[b2.id.clone()], None).is_ok());
        // тот же id в той же команде допустим
        assert!(store.check_single_team(&[b1.id.clone()], Some(&r1.id)).is_ok());
        let _ = tokio::fs::remove_dir_all(dir).await;
    }

    #[tokio::test]
    async fn migrate_one_bot_one_team_leaves_one_room() {
        let dir = temp_dir("migrate-one-bot");
        std::fs::create_dir_all(&dir).unwrap();
        let state = json!({
            "bots": [{ "id": "b1", "name": "Бот", "driver": "", "model": "" }],
            "rooms": [
                { "id": "r_lead", "name": "Лид", "emoji": "", "color": "#7a8595", "instructions": "", "botIds": ["b1"], "lead": "b1", "mode": "lead", "createdAt": 1 },
                { "id": "r_other", "name": "Другая", "emoji": "", "color": "#7a8595", "instructions": "", "botIds": ["b1"], "lead": "", "mode": "lead", "createdAt": 2 }
            ]
        });
        std::fs::write(dir.join("state.json"), serde_json::to_string(&state).unwrap()).unwrap();
        let store = Store::open(dir.clone()).await.unwrap();
        // бот остался в команде, где он ведущий
        assert!(store.room("r_lead").unwrap().bot_ids.contains(&"b1".to_string()));
        assert!(!store.room("r_other").unwrap().bot_ids.contains(&"b1".to_string()));
        // ведущий покинутой команды исправлен
        assert_eq!(store.room("r_other").unwrap().lead, "");
        // в треде покинутой команды — строка о переходе
        let thread = store.read_room_thread("r_other", 50).await;
        assert!(thread.iter().any(|m| m["text"].as_str().is_some_and(|t| t.contains("перешёл только в команду «Лид»"))), "{thread:?}");
        // повторный прогон ничего не меняет
        assert!(!store.migrate_one_bot_one_team().await);
        let _ = tokio::fs::remove_dir_all(dir).await;
    }

    #[tokio::test]
    async fn migration_keeps_the_bigger_team_among_teams_the_bot_leads() {
        // Как у владельца 18.11: «Редактор» ведёт команду из одного себя и команду из восьми.
        let dir = temp_dir("migrate-bigger");
        std::fs::create_dir_all(&dir).unwrap();
        let state = json!({
            "bots": [{ "id": "b1", "name": "Редактор" }, { "id": "b2", "name": "Автор" }],
            "rooms": [
                { "id": "r_small", "name": "Команда 2", "botIds": ["b1"], "lead": "b1", "createdAt": 1 },
                { "id": "r_big", "name": "Соцсети", "botIds": ["b1", "b2"], "lead": "b1", "createdAt": 2 }
            ]
        });
        std::fs::write(dir.join("state.json"), serde_json::to_string(&state).unwrap()).unwrap();
        let store = Store::open(dir.clone()).await.unwrap();
        assert_eq!(store.team_of("b1").unwrap().id, "r_big");
        assert!(store.room("r_small").unwrap().bot_ids.is_empty());
        assert_eq!(store.team_of("b2").unwrap().id, "r_big");
        let _ = tokio::fs::remove_dir_all(dir).await;
    }

    impl Store {
        /// Тест подложил файлы после запуска: забываем собранное и собираем снова.
        async fn build_team_folders_again(&self) {
            self.team_built.lock().unwrap().clear();
            self.build_team_folders().await;
        }
    }

    async fn team_store(name: &str) -> (Store, PathBuf) {
        let dir = temp_dir(name);
        std::fs::create_dir_all(&dir).unwrap();
        let state = json!({
            "bots": [{ "id": "b1", "name": "Автор" }, { "id": "b2", "name": "Редактор" }, { "id": "b3", "name": "Одиночка" }],
            "rooms": [{ "id": "r1", "name": "Соцсети", "botIds": ["b1", "b2"], "lead": "b1", "createdAt": 1 }]
        });
        std::fs::write(dir.join("state.json"), serde_json::to_string(&state).unwrap()).unwrap();
        for id in ["b1", "b2", "b3"] {
            std::fs::create_dir_all(dir.join("workspaces").join(id)).unwrap();
        }
        (Store::open(dir.clone()).await.unwrap(), dir)
    }

    #[tokio::test]
    async fn a_team_member_works_in_the_team_folder_built_once_without_overwriting() {
        let (store, dir) = team_store("team-ws").await;
        let ws = dir.join("workspaces");
        std::fs::create_dir_all(ws.join("b1").join("sub")).unwrap();
        std::fs::write(ws.join("b1").join("same.csv"), "from b1").unwrap();
        std::fs::write(ws.join("b1").join("sub").join("x.txt"), "x").unwrap();
        std::fs::write(ws.join("b1").join("memory.md"), "private").unwrap();
        std::fs::write(ws.join("b1").join("brain.md"), "private").unwrap();
        std::fs::write(ws.join("b1").join("run.bat"), "calc").unwrap();
        std::fs::write(ws.join("b2").join("same.csv"), "from b2").unwrap();
        std::fs::write(ws.join("b2").join("dup.txt"), "d").unwrap();
        std::fs::write(ws.join("b1").join("dup.txt"), "d").unwrap();
        // Собранные программой файлы (метка в начале) и временные файлы записи в общую папку не идут.
        std::fs::write(ws.join("b1").join("AGENTS.md"), format!("{}\nСекрет-личной-памяти", crate::agents_md::MARK)).unwrap();
        std::fs::write(ws.join("b1").join("CLAUDE.md"), "x").unwrap();
        std::fs::write(ws.join("b1").join(".memory.md.tmp"), "x").unwrap();
        std::fs::write(ws.join("b1").join("sub").join("copy.md"), format!("{}\nСекрет-личной-памяти", crate::agents_md::MARK)).unwrap();
        std::fs::remove_file(ws.join("team_r1.members")).unwrap();
        store.build_team_folders_again().await;
        let team = store.workspace("b1");
        assert_eq!(team, ws.join("team_r1"));
        assert_eq!(store.workspace("b2"), team, "оба участника в одной папке");
        assert_eq!(store.workspace("b3"), ws.join("b3"), "бот вне команды — в личной");
        assert_eq!(std::fs::read_to_string(team.join("same.csv")).unwrap(), "from b1");
        assert_eq!(std::fs::read_to_string(team.join("same__Редактор.csv")).unwrap(), "from b2");
        assert!(team.join("sub").join("x.txt").is_file() && team.join("dup.txt").is_file());
        for private in ["memory.md", "brain.md", "run.bat", "AGENTS.md", "CLAUDE.md", ".memory.md.tmp", "sub/copy.md"] {
            assert!(!team.join(private).exists(), "{private} в общую папку не идёт");
        }
        assert!(ws.join("b1").join("memory.md").is_file(), "личная папка осталась как была");
        // Повторный вызов ничего не меняет, а файл, дописанный в команде, не затирается.
        std::fs::write(team.join("same.csv"), "edited").unwrap();
        store.build_team_folders_again().await;
        assert_eq!(std::fs::read_to_string(team.join("same.csv")).unwrap(), "edited");
        assert_eq!(std::fs::read_dir(&team).unwrap().count(), 4, "same, same__Редактор, sub, dup");
        let _ = tokio::fs::remove_dir_all(dir).await;
    }

    #[tokio::test]
    async fn building_the_team_folder_does_not_follow_a_link_in_the_destination_or_go_deeper_than_the_limit() {
        let (store, dir) = team_store("team-ws-links").await;
        let ws = dir.join("workspaces");
        let outside = dir.join("outside");
        std::fs::create_dir_all(&outside).unwrap();
        std::fs::create_dir_all(ws.join("b1").join("sub")).unwrap();
        std::fs::write(ws.join("b1").join("sub").join("x.txt"), "x").unwrap();
        let mut deep = ws.join("b1");
        for i in 0..25 {
            deep.push(format!("d{i}"));
        }
        std::fs::create_dir_all(&deep).unwrap();
        std::fs::write(deep.join("deep.txt"), "d").unwrap();
        // В общей папке уже лежит junction с именем папки участника: он ведёт наружу.
        let team = ws.join("team_r1");
        let made = std::process::Command::new("cmd").args(["/c", "mklink", "/J"]).arg(team.join("sub")).arg(&outside).output().map(|o| o.status.success()).unwrap_or(false);
        std::fs::remove_file(ws.join("team_r1.members")).unwrap();
        store.build_team_folders_again().await;
        if made {
            assert!(std::fs::read_dir(&outside).unwrap().next().is_none(), "через junction ничего не записано наружу");
            let _ = std::process::Command::new("cmd").args(["/c", "rmdir"]).arg(team.join("sub")).output();
        }
        let mut seen = team.clone();
        let mut levels = 0;
        while let Some(next) = std::fs::read_dir(&seen).ok().and_then(|mut d| d.find_map(|e| e.ok()).filter(|e| e.path().is_dir()).map(|e| e.path())) {
            seen = next;
            levels += 1;
        }
        assert!(levels <= MERGE_DEPTH, "глубже {MERGE_DEPTH} уровней не переносится: {levels}");
        assert!(!seen.join("deep.txt").exists());
        let _ = tokio::fs::remove_dir_all(dir).await;
    }

    #[tokio::test]
    async fn when_the_browser_owner_leaves_the_team_another_member_takes_over_and_the_file_is_rewritten() {
        let (store, dir) = team_store("team-owner-leaves").await;
        std::fs::create_dir_all(store.browser_profile("b1").join("Default")).unwrap();
        let room = store.room("r1").unwrap();
        assert_eq!(store.team_browser_owner(&room).as_deref(), Some("b1"));
        store.update_room("r1", &json!({ "botIds": ["b2"] })).await.unwrap();
        let room = store.room("r1").unwrap();
        assert_eq!(store.team_browser_owner(&room).as_deref(), Some("b2"), "владелец вышел: общий браузер у оставшегося");
        assert_eq!(std::fs::read_to_string(dir.join("browser").join("team_r1.owner")).unwrap(), "b2");
        assert_eq!(store.browser_owner("b1"), "b1", "вышедший снова со своим профилем");
        let _ = tokio::fs::remove_dir_all(dir).await;
    }

    #[tokio::test]
    async fn a_member_leaving_the_team_returns_to_the_personal_folder_and_a_newcomer_adds_files() {
        let (store, dir) = team_store("team-ws-leave").await;
        let team = store.workspace("b2");
        assert!(team.is_dir(), "папка собрана при запуске");
        std::fs::write(team.join("made-in-team.md"), "a").unwrap();
        store.update_room("r1", &json!({ "botIds": ["b1"] })).await.unwrap();
        assert_eq!(store.workspace("b2"), dir.join("workspaces").join("b2"));
        assert!(team.join("made-in-team.md").is_file(), "файлы команды остались в команде");
        std::fs::write(dir.join("workspaces").join("b3").join("late.txt"), "l").unwrap();
        store.update_room("r1", &json!({ "botIds": ["b1", "b3"] })).await.unwrap();
        assert!(store.workspace("b3").join("late.txt").is_file(), "новичок принёс файлы в уже собранную папку");
        let _ = tokio::fs::remove_dir_all(dir).await;
    }

    #[tokio::test]
    async fn the_team_browser_owner_is_remembered_even_when_others_touch_their_profiles() {
        let (store, dir) = team_store("team-owner").await;
        std::fs::create_dir_all(store.browser_profile("b2").join("Default")).unwrap();
        assert_eq!(store.browser_owner("b1"), "b2", "свежий профиль — владелец");
        std::thread::sleep(std::time::Duration::from_millis(20));
        std::fs::create_dir_all(store.browser_profile("b1").join("Default")).unwrap();
        std::fs::write(store.browser_profile("b1").join("Default").join("x"), "1").unwrap();
        assert_eq!(store.browser_owner("b1"), "b2", "выбор запомнен");
        assert_eq!(store.browser_owner("b2"), "b2");
        assert_eq!(store.browser_owner("b3"), "b3", "бот вне команды — свой браузер");
        assert!(dir.join("browser").join("team_r1.owner").is_file());
        let _ = tokio::fs::remove_dir_all(dir).await;
    }

    #[tokio::test]
    async fn load_migrates_old_routine_schedules_once() {
        let dir = temp_dir("routine-migrate");
        let legacy = json!({
            "version": 1,
            "routines": [
                { "id": "r1", "botId": "b1", "title": "Срез", "prompt": "собери", "at": "09:00", "days": [3] },
                { "id": "r2", "botId": "b1", "title": "Звонок", "prompt": "позвони", "onceAt": 1_800_000_000_000_i64 }
            ]
        });
        tokio::fs::create_dir_all(&dir).await.unwrap();
        tokio::fs::write(dir.join("state.json"), legacy.to_string()).await.unwrap();
        let store = Store::open(dir.clone()).await.unwrap();
        let migrated = store.routine("r1").unwrap();
        assert_eq!(migrated.recurrence, Some(crate::dates_ru::Recurrence {
            freq: crate::dates_ru::Freq::Weekly,
            interval: 1,
            weekdays: vec![3],
            monthday: None,
            nth: None,
            month: None,
        }));
        assert!(migrated.days.is_empty());
        let loaded_once = store.routine("r2").unwrap();
        assert_eq!(loaded_once.once_at, 1_800_000_000_000);
        assert!(!loaded_once.extra.contains_key("onceAt"));

        let mut state = State::default();
        let mut reminder = Routine { id: "r2".into(), ..Routine::default() };
        reminder.extra.insert("onceAt".into(), json!(1_800_000_000_000_i64));
        state.routines.push(reminder);
        assert!(migrate_routines(&mut state));
        assert_eq!(state.routines[0].once_at, 1_800_000_000_000);
        assert!(!state.routines[0].extra.contains_key("onceAt"));
        assert!(!migrate_routines(&mut state), "вторая миграция ничего не меняет");

        let saved = tokio::fs::read_to_string(dir.join("state.json")).await.unwrap();
        let _ = Store::open(dir.clone()).await.unwrap();
        assert_eq!(tokio::fs::read_to_string(dir.join("state.json")).await.unwrap(), saved);
        let _ = tokio::fs::remove_dir_all(dir).await;
    }
}

/// Короткий безопасный id: до `max` символов; `wide` разрешает заглавные буквы и точку.
fn valid_id(s: &str, max: usize, wide: bool) -> bool {
    !s.is_empty()
        && s.len() <= max
        && s.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_' || c == '-' || (wide && (c.is_ascii_uppercase() || c == '.')))
}

/// Готовит общую папку команды: файлы участников, которых в ней ещё нет (список — `team_<id>.members` рядом с папкой).
/// Участник попадает в список один раз: вышел и вернулся — файлы второй раз не переносятся.
fn build_team_folder(root: &Path, room: &Room, names: &[(String, String)]) {
    static BUILD: Mutex<()> = Mutex::new(());
    let _build = BUILD.lock().unwrap_or_else(|e| e.into_inner());
    let dir = root.join(format!("team_{}", room.id));
    let marker = root.join(format!("team_{}.members", room.id));
    let mut done = std::fs::read_to_string(&marker).unwrap_or_default();
    if std::fs::create_dir_all(&dir).is_err() {
        return;
    }
    let Ok(team_root) = std::fs::canonicalize(&dir) else { return };
    let fresh: Vec<&(String, String)> = names.iter().filter(|(id, _)| !done.lines().any(|l| l == id)).collect();
    for (member, name) in fresh {
        let tag: String = name.chars().filter(|c| c.is_alphanumeric()).take(16).collect();
        merge_files(&root.join(member), &dir, &team_root, Path::new(""), &tag, &mut MergeBudget::default());
        done.push_str(&format!("{member}\n"));
    }
    let _ = std::fs::write(&marker, done);
}

/// Сколько личной папки одного участника переносится в общую: записей (файлов и папок), байт и глубина.
struct MergeBudget {
    entries: usize,
    bytes: u64,
}

impl Default for MergeBudget {
    fn default() -> Self {
        Self { entries: 5000, bytes: 500 * 1024 * 1024 }
    }
}

const MERGE_FILE_LIMIT: u64 = 20 * 1024 * 1024;
const MERGE_DEPTH: usize = 20;

/// Файлы, которые у бота остаются своими и в общую папку не идут: память, правила от человека, старый MAIN.md,
/// собранные программой `AGENTS.md` и `CLAUDE.md` (в команде их пишет программа сама), временные файлы записи
/// с точкой в начале имени. Только в корне личной папки.
fn private_file(rel: &Path, name: &str) -> bool {
    let name = name.trim_start_matches('.').to_lowercase();
    let private = [crate::memory::bot_file::FILE, crate::brain::FILE, crate::turn::MAIN_FILE, "AGENTS.md", "CLAUDE.md"];
    rel.as_os_str().is_empty() && private.iter().any(|f| name.starts_with(&f.to_lowercase()))
}

/// Путь внутри общей папки, куда можно писать: ни один существующий кусок не ссылка и не junction,
/// настоящий родитель лежит внутри папки команды.
fn merge_dest(team_root: &Path, to_root: &Path, rel: &Path) -> Option<PathBuf> {
    let mut walk = to_root.to_path_buf();
    for part in rel.components() {
        walk.push(part);
        if crate::attach::is_link(&walk) {
            return None;
        }
    }
    let real_parent = std::fs::canonicalize(walk.parent()?).ok()?;
    real_parent.starts_with(team_root).then_some(walk)
}

/// Копирует файлы личной папки бота в общую папку команды. Существующее имя не трогается: файл с тем же именем
/// и другим содержимым получает метку с именем бота (одинаковый — пропускается). Ссылки и junction (в источнике и
/// в назначении), личные файлы (`private_file`), файлы, начинающиеся с метки сборки Deloist, и всё, что
/// `tools::flagged_target` считает запускаемым или многоссылочным, не переносятся. Источник открывается один раз,
/// и копия идёт из этого же открытого файла. Личная папка остаётся как была.
fn merge_files(from: &Path, to_root: &Path, team_root: &Path, rel: &Path, tag: &str, budget: &mut MergeBudget) {
    if rel.components().count() >= MERGE_DEPTH {
        return;
    }
    let Ok(entries) = std::fs::read_dir(from.join(rel)) else { return };
    for entry in entries.flatten() {
        if budget.entries == 0 || budget.bytes == 0 {
            return;
        }
        let name = entry.file_name();
        let rel_path = rel.join(&name);
        let source = entry.path();
        if crate::attach::is_link(&source) || private_file(rel, &name.to_string_lossy()) || crate::reach::project_guard_reason(&rel_path).is_some() {
            continue;
        }
        let Ok(meta) = entry.metadata() else { continue };
        if meta.is_dir() {
            let Some(dest) = merge_dest(team_root, to_root, &rel_path) else { continue };
            if (dest.is_dir() || std::fs::create_dir(&dest).is_ok()) && !crate::attach::is_link(&dest) {
                budget.entries -= 1;
                merge_files(from, to_root, team_root, &rel_path, tag, budget);
            }
            continue;
        }
        if !meta.is_file() || meta.len() > MERGE_FILE_LIMIT || crate::tools::flagged_target(&source) {
            continue;
        }
        let Some(mut target) = merge_dest(team_root, to_root, &rel_path) else { continue };
        let Ok(mut file) = std::fs::File::open(&source) else { continue };
        if !file.metadata().is_ok_and(|m| m.is_file()) || starts_with_mark(&mut file) {
            continue;
        }
        if std::fs::symlink_metadata(&target).is_ok() {
            if same_content(&mut file, &target) {
                continue;
            }
            let stem = Path::new(&name).file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
            let ext = Path::new(&name).extension().map(|e| format!(".{}", e.to_string_lossy())).unwrap_or_default();
            target = to_root.join(rel).join(format!("{stem}__{tag}{ext}"));
            if merge_dest(team_root, to_root, &rel.join(target.file_name().unwrap_or_default())).is_none() {
                continue;
            }
        }
        // `create_new`: существующее имя (в том числе ссылку) не открываем и не перезаписываем.
        let Ok(mut out) = std::fs::OpenOptions::new().write(true).create_new(true).open(&target) else { continue };
        use std::io::{Read as _, Seek as _};
        if file.rewind().is_ok() && std::io::copy(&mut file.by_ref().take(MERGE_FILE_LIMIT), &mut out).is_ok() {
            budget.entries -= 1;
            budget.bytes = budget.bytes.saturating_sub(meta.len());
        }
    }
}

/// Файл начинается с метки «собрано Deloist» (`agents_md::MARK`): такие файлы пишет программа для одного бота.
fn starts_with_mark(file: &mut std::fs::File) -> bool {
    use std::io::{Read as _, Seek as _};
    let mark = crate::agents_md::MARK.as_bytes();
    let mut head = vec![0u8; mark.len()];
    let found = file.read_exact(&mut head).is_ok() && head == mark;
    let _ = file.rewind();
    found
}

/// Содержимое открытого файла совпадает с файлом `target` (обычным, не ссылкой).
fn same_content(file: &mut std::fs::File, target: &Path) -> bool {
    use std::io::{Read as _, Seek as _};
    if crate::attach::is_link(target) {
        return false;
    }
    let (mut mine, _) = (Vec::new(), file.rewind());
    let read = file.by_ref().take(MERGE_FILE_LIMIT).read_to_end(&mut mine).is_ok();
    let _ = file.rewind();
    read && std::fs::read(target).is_ok_and(|theirs| theirs == mine)
}
