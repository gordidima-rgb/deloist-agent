//! Работа команды без ожидания: `assign_task` отдаёт задачу участнику и сразу возвращает номер, `task_status` и
//! `team_status` показывают, кто чем занят, `share_file` кладёт файл из своей папки в папку участника.
//!
//! Правила: любому участнику своей команды, вне команды — только ведущий ведущему; не больше трёх незакрытых задач
//! на участника от одного бота; повтор того же текста, пока задача идёт, возвращает тот же номер. Ход участника
//! запускает тот же `delegate_to_bot`, что и у `delegate`, с теми же правилами согласий. Задачи лежат в
//! `team_tasks.json`: после перезапуска то, что шло или ждало, ставится в очередь и идёт заново. Идёт ли прерванный
//! ход с того места или ждёт человека, решает журнал запусков (`runs`). Конец задачи — короткая строка в ленте
//! ведущего; его собственный ход она не запускает. Новых адресов API нет: инструменты идут только через ход бота.

use std::path::{Component, Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::store::{now, uid, Bot, Room};
use crate::App;

pub const TOOLS: [&str; 4] = ["assign_task", "task_status", "team_status", "share_file"];

pub fn is_tool(name: &str) -> bool {
    TOOLS.contains(&name)
}

pub fn label(name: &str) -> &'static str {
    match name {
        "assign_task" => "задача участнику",
        "task_status" => "состояние задач",
        "team_status" => "состояние команды",
        _ => "передача файла",
    }
}

/// Незакрытых задач у одного участника от одного бота.
pub const MAX_OPEN: usize = 3;
const MAX_TEXT: usize = 6_000;
const TEXT_TOO_LONG: &str = "Задача длиннее 6000 знаков: сократи до сути — цель, входные данные, что вернуть";
const MAX_FILES: usize = 5;
/// Файл для передачи не больше этого.
pub const MAX_SHARE_BYTES: u64 = 20 * 1024 * 1024;
const SUMMARY_CHARS: usize = 1_500;
const LINE_CHARS: usize = 300;
const NOTE_CHARS: usize = 300;
const KEEP_MS: i64 = 24 * 3600 * 1000;
const KEEP_OLD_MS: i64 = 7 * 24 * 3600 * 1000;
const KEEP_TOTAL: usize = 300;
/// Сколько задача ждёт, пока занятый получатель освободится.
const QUEUE_WAIT: Duration = Duration::from_secs(30 * 60);
const RESULT_FILES: usize = 8;
const NO_REPLY: &str = crate::turn::NO_REPLY;

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Task {
    pub id: String,
    pub from: String,
    pub from_name: String,
    pub to: String,
    pub to_name: String,
    pub text: String,
    /// Файлы, переданные вместе с задачей: пути внутри папки получателя.
    pub files: Vec<String>,
    /// queued, running, done, error, interrupted.
    pub state: String,
    pub created: i64,
    pub started: i64,
    pub finished: i64,
    pub summary: String,
    /// Файлы, появившиеся в папке получателя за время задачи.
    pub result_files: Vec<String>,
    /// Режим подтверждений отправителя в момент выдачи (с цепочкой передач): к запуску его ход уже кончился.
    pub floor: String,
    /// Отправитель работал без человека.
    pub unattended: bool,
    /// Глубина цепочки передач отправителя в момент выдачи.
    pub depth: u32,
    /// Группа, из чата которой выдана задача: строка о конце уйдёт туда. Пусто — в личный чат отправителя.
    pub room: String,
    /// Ведущему уже сказали об итоге в его следующем ходу (системная заметка).
    pub noted: bool,
    /// Первое сообщение продолжения (решение человека по карточке или заметка об обрыве): уходит исполнителю после брифа.
    pub carry: String,
}

impl Task {
    pub fn is_open(&self) -> bool {
        matches!(self.state.as_str(), "queued" | "running")
    }
}

pub struct Tasks {
    path: PathBuf,
    items: Mutex<Vec<Task>>,
}

impl Tasks {
    /// Читает сохранённое. Процесс, который вёл задачи, умер: ждавшая и шедшая встают в очередь. Запускает их `resume`.
    pub fn open(dir: &Path) -> Self {
        let path = dir.join("team_tasks.json");
        let mut items: Vec<Task> = std::fs::read_to_string(&path).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default();
        let at = now();
        let mut changed = false;
        for task in items.iter_mut().filter(|t| t.is_open()) {
            changed = true;
            task.state = "queued".into();
            task.started = 0;
        }
        let before = items.len();
        items.retain(|t| at - t.created < KEEP_OLD_MS);
        let tasks = Self { path, items: Mutex::new(items) };
        if changed || before != tasks.items.lock().unwrap().len() {
            tasks.save(&tasks.items.lock().unwrap());
        }
        tasks
    }

    fn save(&self, items: &[Task]) {
        let Ok(text) = serde_json::to_string(items) else { return };
        let tmp = self.path.with_extension("json.tmp");
        if std::fs::write(&tmp, text).and_then(|_| std::fs::rename(&tmp, &self.path)).is_err() {
            eprintln!("[team_tasks] не смог записать team_tasks.json");
        }
    }

    /// Новая задача или уже идущая такая же. `true` — создана новая.
    fn begin(&self, task: Task) -> Result<(Task, bool), String> {
        let mut items = self.items.lock().unwrap();
        let same = |t: &&Task| t.is_open() && t.from == task.from && t.to == task.to;
        if let Some(found) = items.iter().filter(same).find(|t| squash(&t.text) == squash(&task.text)) {
            return Ok((found.clone(), false));
        }
        if items.iter().filter(same).count() >= MAX_OPEN {
            return Err(format!(
                "у «{}» уже {MAX_OPEN} незакрытые задачи от тебя: дождись итога (task_status) и давай следующую",
                task.to_name
            ));
        }
        items.push(task.clone());
        if items.len() > KEEP_TOTAL {
            if let Some(index) = items.iter().position(|t| !t.is_open()) {
                items.remove(index);
            }
        }
        self.save(&items);
        Ok((task, true))
    }

    pub fn get(&self, id: &str) -> Option<Task> {
        self.items.lock().unwrap().iter().find(|t| t.id == id).cloned()
    }

    fn update(&self, id: &str, change: impl FnOnce(&mut Task)) -> Option<Task> {
        let mut items = self.items.lock().unwrap();
        let task = items.iter_mut().find(|t| t.id == id)?;
        change(task);
        let task = task.clone();
        self.save(&items);
        Some(task)
    }

    /// Закрытые задачи этого отправителя, о которых модели ещё не сказали; помечает их сказанными.
    pub fn take_unnoted(&self, from: &str) -> Vec<Task> {
        let mut items = self.items.lock().unwrap();
        let mut found: Vec<Task> = Vec::new();
        for task in items.iter_mut().filter(|t| t.from == from && !t.is_open() && !t.noted) {
            task.noted = true;
            found.push(task.clone());
        }
        if !found.is_empty() {
            self.save(&items);
        }
        found.sort_by_key(|t| t.finished);
        found
    }

    /// Номера задач, которые ждут запуска.
    fn queued_ids(&self) -> Vec<String> {
        self.items.lock().unwrap().iter().filter(|t| t.state == "queued").map(|t| t.id.clone()).collect()
    }

    pub fn all(&self) -> Vec<Task> {
        self.items.lock().unwrap().clone()
    }
}

fn squash(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ").to_lowercase()
}

fn clip(text: &str, limit: usize) -> String {
    let flat = text.trim();
    if flat.chars().count() <= limit {
        return flat.to_string();
    }
    format!("{}…", flat.chars().take(limit).collect::<String>().trim_end())
}

fn one_line(text: &str, limit: usize) -> String {
    clip(&text.split_whitespace().collect::<Vec<_>>().join(" "), limit)
}

// ---------------------------------------------------------------------------
// Кто в одной команде
// ---------------------------------------------------------------------------

/// Ведущий команды: назначенный в ней, а если не назначен, — единственный бот ранга «ведущий».
fn lead_of(app: &App, room: &Room) -> Option<String> {
    if !room.lead.is_empty() && room.bot_ids.contains(&room.lead) {
        return Some(room.lead.clone());
    }
    let mut leads = room.bot_ids.iter().filter(|id| app.store.bot(id).is_some_and(|b| b.rank == "lead"));
    let first = leads.next()?;
    leads.next().is_none().then(|| first.clone())
}

fn rooms_of(app: &App, bot_id: &str) -> Vec<Room> {
    app.store.rooms().into_iter().filter(|r| !r.archived && r.bot_ids.iter().any(|b| b == bot_id)).collect()
}

/// Те, кому `from` вправе давать задачи и файлы (владелец, 16.11.2026): внутри команды — любому
/// участнику, вне команды общаются только ведущие (оркестраторы) — ведущий с ведущими других команд.
pub fn teammates(app: &App, from: &Bot) -> Vec<Bot> {
    let mut ids: Vec<String> = Vec::new();
    let mut push = |id: &String| {
        if *id != from.id && !ids.contains(id) {
            ids.push(id.clone());
        }
    };
    let rooms: Vec<Room> = app.store.rooms().into_iter().filter(|r| !r.archived).collect();
    let mut leads_somewhere = false;
    for room in rooms.iter().filter(|r| r.bot_ids.iter().any(|b| *b == from.id)) {
        leads_somewhere |= lead_of(app, room).as_deref() == Some(from.id.as_str());
        room.bot_ids.iter().for_each(&mut push);
    }
    if leads_somewhere {
        for lead in rooms.iter().filter_map(|room| lead_of(app, room)) {
            push(&lead);
        }
    }
    ids.iter().filter_map(|id| app.store.bot(id)).collect()
}

/// Кому `delegate` может поручить: команда по правилу `teammates`, плюс боты, которых человек
/// упомянул в этом ходе («@Бета» — его согласие), и специалисты, уже приглашённые карточкой (гости треда).
pub fn delegate_targets(app: &App, from: &Bot) -> Vec<Bot> {
    let mut bots = teammates(app, from);
    let mut extra: Vec<String> = app.turns.mentioned_for(&from.id);
    extra.extend(
        crate::store::bot_guests(from).iter().filter_map(|g| g.get("botId").and_then(Value::as_str)).map(str::to_string),
    );
    for id in extra {
        if id != from.id && !bots.iter().any(|b| b.id == id) {
            if let Some(bot) = app.store.bot(&id) {
                bots.push(bot);
            }
        }
    }
    bots
}

/// Бот по имени среди разрешённых: точное имя, иначе единственное совпадение по началу.
pub fn pick_by_name(bots: &[Bot], name: &str) -> Option<Bot> {
    let wanted = name.trim().to_lowercase();
    if wanted.is_empty() {
        return None;
    }
    let exact: Vec<&Bot> = bots.iter().filter(|b| b.name.to_lowercase() == wanted).collect();
    if !exact.is_empty() {
        return (exact.len() == 1).then(|| exact[0].clone());
    }
    let prefix: Vec<&Bot> = bots.iter().filter(|b| b.name.to_lowercase().starts_with(&wanted)).collect();
    (prefix.len() == 1).then(|| prefix[0].clone())
}

/// Отказ с подсказкой, если адресат вне разрешённых.
pub const OUTSIDE_TEAM: &str = "не в твоей команде: задачи можно передавать участникам своей команды, а вне команды общаются только ведущие команд. Нужен другой бот — предложи человеку собрать команду через propose_team или специалиста через offer_specialist.";

fn find_teammate(app: &App, from: &Bot, name: &str) -> Result<Bot, String> {
    let name = name.trim();
    if name.is_empty() {
        return Err("не указан бот".into());
    }
    pick_by_name(&teammates(app, from), name).ok_or_else(|| format!("«{name}» {OUTSIDE_TEAM}"))
}

// ---------------------------------------------------------------------------
// Схемы
// ---------------------------------------------------------------------------

pub fn defs() -> Vec<Value> {
    let function = |name: &str, description: &str, properties: Value, required: &[&str]| {
        let mut parameters = json!({ "type": "object", "properties": properties });
        if !required.is_empty() {
            parameters["required"] = json!(required);
        }
        json!({ "type": "function", "function": { "name": name, "description": description, "parameters": parameters } })
    };
    vec![
        function(
            "assign_task",
            "Дать задачу участнику своей команды БЕЗ ожидания: вернётся taskId, а участник работает параллельно. Так можно пустить нескольких участников сразу. Когда задача закончится, в твоей ленте появится строка; свой ход она не запускает — результат смотри через task_status. Не больше 3 незакрытых задач на участника; тот же текст тому же боту, пока задача идёт, вернёт тот же taskId. Участник не видит ни твою переписку, ни прошлые ходы: задача — самодостаточный бриф.",
            json!({
                "bot": { "type": "string", "description": "Имя участника твоей команды; ведущий может дать задачу и ведущему другой команды." },
                "text": { "type": "string", "description": "Самодостаточный бриф: цель, входные данные, ограничения, что вернуть. До 6000 знаков." },
                "files": { "type": "array", "items": { "type": "string" }, "description": "Файлы из твоей папки, которые нужны участнику (до 5, по 20 МБ). Копии лягут в его папку attachments/." },
            }),
            &["bot", "text"],
        ),
        function(
            "task_status",
            "Состояние выданных тобой задач: ждёт, в работе, готово, ошибка или прервана перезапуском; итог (до 1500 знаков) и файлы результата. Без taskId — все незакрытые и закрытые за последние сутки.",
            json!({ "taskId": { "type": "string", "description": "Номер из assign_task." } }),
            &[],
        ),
        function(
            "team_status",
            "Кто в твоей команде, кто ведёт, кто занят и чем (одна строка), последний итог участника и когда он был.",
            json!({ "room": { "type": "string", "description": "Имя команды; без него — все твои команды." } }),
            &[],
        ),
        function(
            "share_file",
            "Передать файл из своей папки участнику своей команды: копия ляжет в его папку attachments/ с датой в имени. Только свои файлы (без «..», ссылок, .env и ключей), до 20 МБ. Для задачи проще указать files в assign_task.",
            json!({
                "bot": { "type": "string", "description": "Имя участника твоей команды." },
                "path": { "type": "string", "description": "Файл в твоей рабочей папке: относительный путь или полный путь внутри неё." },
            }),
            &["bot", "path"],
        ),
    ]
}

// ---------------------------------------------------------------------------
// Исполнение
// ---------------------------------------------------------------------------

pub async fn exec(app: &Arc<App>, bot: &Bot, name: &str, args: &Value, depth: u32) -> Result<String, String> {
    let text = |key: &str| args.get(key).and_then(Value::as_str).unwrap_or_default().trim().to_string();
    match name {
        "assign_task" => assign(app, bot, &text("bot"), &text("text"), args.get("files"), depth).await,
        "task_status" => Ok(status(app, bot, &text("taskId"))),
        "team_status" => team_status(app, bot, &text("room")),
        _ => {
            let target = find_teammate(app, bot, &text("bot"))?;
            let copied = share(app, bot, &target, &text("path")).await?;
            Ok(format!("Файл передан «{}»: в его папке {copied}. Сошлись на этот путь в задаче.", target.name))
        }
    }
}

async fn assign(app: &Arc<App>, from: &Bot, to_name: &str, text: &str, files: Option<&Value>, depth: u32) -> Result<String, String> {
    let target = find_teammate(app, from, to_name)?;
    // Глубина цепочки: настоящую берём и из хода отправителя (мост ACP зовёт с нулём). Лимит проверяется сразу, а не при запуске.
    let start_depth = depth.max(app.turns.delegation_depth_of(&from.id));
    if start_depth >= 2 {
        return Err("слишком глубокая цепочка делегирования".into());
    }
    if text.is_empty() {
        return Err("пустая задача".into());
    }
    if text.chars().count() > MAX_TEXT {
        return Err(TEXT_TOO_LONG.into());
    }
    let paths: Vec<String> = match files {
        None | Some(Value::Null) => Vec::new(),
        Some(Value::Array(list)) => list.iter().map(|v| v.as_str().unwrap_or_default().trim().to_string()).collect(),
        Some(_) => return Err("files — список путей".into()),
    };
    if paths.len() > MAX_FILES {
        return Err(format!("файлов не больше {MAX_FILES}"));
    }
    // Файлы копируем ДО выдачи номера: отказ при копировании не оставляет номера, которого нет.
    let mut copied = Vec::new();
    for raw in &paths {
        match share(app, from, &target, raw).await {
            Ok(path) => copied.push(path),
            Err(e) => {
                remove_copies(app, &target, &copied);
                return Err(format!("задача не выдана: {e}"));
            }
        }
    }
    // Что имел отправитель сейчас: пол осторожности с цепочкой передач, «без человека», глубина и откуда выдано.
    let fresh = app.store.bot(&from.id).unwrap_or_else(|| from.clone());
    let task = Task {
        id: uid("task"),
        from: from.id.clone(),
        from_name: from.name.clone(),
        to: target.id.clone(),
        to_name: target.name.clone(),
        text: text.to_string(),
        files: copied.clone(),
        state: "queued".into(),
        created: now(),
        floor: app.turns.effective_approval(&app.store, &from.id, &fresh.approval),
        unattended: app.turns.is_unattended(&from.id),
        depth: start_depth,
        room: app.turns.conv_of(&from.id).and_then(|c| c.strip_prefix("room:").map(str::to_string)).unwrap_or_default(),
        ..Task::default()
    };
    let (task, created) = match app.tasks.begin(task) {
        Ok(pair) => pair,
        Err(e) => {
            remove_copies(app, &target, &copied);
            return Err(e);
        }
    };
    if !created {
        remove_copies(app, &target, &copied);
        return Ok(format!("Такая задача для «{}» уже идёт: taskId {}. Состояние — task_status.", target.name, task.id));
    }
    tokio::spawn(run(app.clone(), task.id.clone()));
    Ok(format!(
        "Задача отдана «{}», taskId {}. Она идёт параллельно, ждать её не нужно: когда закончится, в ленте появится строка. Состояние и итог — task_status.",
        target.name, task.id
    ))
}

/// Убрать копии, сделанные для задачи, которой не будет.
fn remove_copies(app: &App, target: &Bot, copied: &[String]) {
    for rel in copied {
        if let Ok(path) = app.store.safe_path(&target.id, rel) {
            if !is_link(&path) {
                let _ = std::fs::remove_file(path);
            }
        }
    }
}

/// После запуска программы: ведёт заново задачи, которые перезапуск оставил в очереди. Зовётся один раз из `main`.
pub fn resume(app: &Arc<App>) {
    for id in app.tasks.queued_ids() {
        tokio::spawn(run(app.clone(), id));
    }
}

/// Бриф получателю: задача, переданные файлы и кто их дал.
fn brief(task: &Task) -> String {
    let mut text = task.text.clone();
    if !task.files.is_empty() {
        text.push_str(&format!("\n\nФайлы от «{}» лежат в твоей папке: {}.", task.from_name, task.files.join(", ")));
    }
    text
}

/// Ведёт задачу: ждёт, пока получатель свободен, запускает его ход и записывает итог.
async fn run(app: Arc<App>, id: String) {
    let deadline = Instant::now() + QUEUE_WAIT;
    loop {
        let Some(task) = app.tasks.get(&id) else { return };
        let (Some(from), Some(target)) = (app.store.bot(&task.from), app.store.bot(&task.to)) else {
            finish(&app, &id, "error", "бота больше нет в команде".into(), Vec::new()).await;
            return;
        };
        // Пока задача ждала, бота могли убрать из команды: права на задачу давала именно команда.
        if !teammates(&app, &from).iter().any(|b| b.id == target.id) {
            finish(&app, &id, "error", format!("«{}» не в команде: задача не начата", target.name), Vec::new()).await;
            return;
        }
        if app.turns.is_busy(&target.id) {
            if Instant::now() > deadline {
                finish(&app, &id, "error", format!("«{}» был занят больше 30 минут, задача не начата", target.name), Vec::new()).await;
                return;
            }
            tokio::time::sleep(Duration::from_secs(1)).await;
            continue;
        }
        let started = now();
        app.tasks.update(&id, |t| {
            t.state = "running".into();
            t.started = started;
        });
        app.turns.emit(json!({ "type": "task_update", "botId": task.from, "taskId": id, "state": "running" }));
        // Ход, который оборвал перезапуск: продолжаем его запись, а не начинаем задачу с нуля.
        let pending = app.runs.pending_for_task(&id);
        let mut text = brief(&task);
        // Решение человека или заметка об обрыве; без них (старт программы) заметку собирает сама запись.
        if !task.carry.is_empty() {
            text = format!("{text}\n\n{}", task.carry);
        } else if let Some(run) = &pending {
            text = format!("{text}\n\n{}", run.continuation_note());
        }
        let carried = crate::turn::Carried { floor: task.floor.clone(), unattended: task.unattended, task_id: id.clone(), resume: pending.map(|r| r.id) };
        match crate::turn::delegate_to_bot_with(app.clone(), from, target.clone(), text, task.depth, Some(carried)).await {
            Err(e) if e.ends_with(crate::turn::BUSY_SUFFIX) => {
                app.tasks.update(&id, |t| t.state = "queued".into());
                tokio::time::sleep(Duration::from_secs(1)).await;
            }
            Err(e) => {
                finish(&app, &id, "error", e, Vec::new()).await;
                return;
            }
            // Ход встал на паузу (лимит, предел): задача не закрыта, её продолжит журнал запусков через `requeue`.
            Ok(_) if app.runs.paused().iter().any(|r| matches!(&r.origin, crate::runs::Origin::Team { task_id } if *task_id == id)) => {
                app.tasks.update(&id, |t| t.state = "paused".into());
                app.turns.emit(json!({ "type": "task_update", "botId": task.from, "taskId": id, "state": "paused" }));
                return;
            }
            Ok(_) if app.runs.waiting_for_task(&id).is_some() => {
                // Исполнитель ждёт человека: задача не закончена, её продолжит решение по карточке.
                let wait = app.runs.waiting_for_task(&id).map(|r| r.wait_text()).unwrap_or_default();
                finish(&app, &id, "interrupted", wait, Vec::new()).await;
                return;
            }
            Ok(text) => {
                let files = fresh_files(&app.store, &target.id, started);
                let (state, summary) = if text.trim() == NO_REPLY { ("error", text) } else { ("done", text) };
                finish(&app, &id, state, summary, files).await;
                return;
            }
        }
    }
}

/// Поставить в очередь задачу, ход которой человек решил продолжить (она закрыта как прерванная).
pub fn requeue(app: &Arc<App>, id: &str, note: &str) -> Result<(), String> {
    let task = app.tasks.get(id).ok_or("задачи уже нет")?;
    // На старте задача уже в очереди: её возьмёт `resume`.
    if task.state == "queued" {
        return Ok(());
    }
    app.tasks.update(id, |t| {
        t.state = "queued".into();
        t.finished = 0;
        t.carry = note.to_string();
    });
    tokio::spawn(run(app.clone(), id.to_string()));
    Ok(())
}

/// Ход задачи ждёт человека: задача закрыта как прервана, ведущий узнает об этом в своей ленте.
pub async fn finish_interrupted(app: &Arc<App>, id: &str, text: &str) {
    finish(app, id, "interrupted", text.to_string(), Vec::new()).await;
}

async fn finish(app: &Arc<App>, id: &str, state: &str, summary: String, files: Vec<String>) {
    let Some(task) = app.tasks.update(id, |t| {
        t.state = state.into();
        t.finished = now();
        t.summary = clip(&summary, SUMMARY_CHARS);
        t.result_files = files;
        t.carry.clear();
    }) else {
        return;
    };
    // Строка в ленте ведущего: сам он не просыпается, итог ему отдаёт task_status.
    let entry = json!({
        "role": "task", "taskId": task.id, "state": task.state, "to": task.to, "toName": task.to_name,
        "text": one_line(&task.summary, LINE_CHARS), "files": task.result_files,
    });
    // Задача выдана из чата команды: строка о конце ложится туда, где человек её и ждёт.
    if !task.room.is_empty() && app.store.room(&task.room).is_some_and(|r| r.bot_ids.contains(&task.from)) {
        match app.store.append_room_message(&task.room, entry.clone()).await {
            Ok(saved) => app.turns.emit(json!({ "type": "room_message", "roomId": task.room, "message": saved })),
            Err(e) => eprintln!("[team_tasks] не смог записать строку в ленту группы: {e}"),
        }
        return;
    }
    match app.store.append_message(&task.from, entry).await {
        Ok(saved) => app.turns.emit(json!({ "type": "task_done", "botId": task.from, "taskId": task.id, "message": saved })),
        Err(e) => eprintln!("[team_tasks] не смог записать строку в ленту: {e}"),
    }
}

/// Заметка модели ведущего в начале его хода: какие задачи закрылись с тех пор, как он о них слышал. Раз на задачу.
/// Строку в ленте модель не видит (роль `task` скрыта), поэтому итог ей приходит только так или через task_status.
pub fn closed_note(app: &App, bot_id: &str) -> Option<String> {
    let found = app.tasks.take_unnoted(bot_id);
    if found.is_empty() {
        return None;
    }
    let mut lines = vec!["С прошлого хода закрылись задачи, которые ты выдал:".to_string()];
    for task in found {
        let mut line = format!("- «{}», {} (taskId {})", task.to_name, state_ru(&task.state), task.id);
        if !task.summary.is_empty() {
            line.push_str(&format!(": {}", one_line(&task.summary, NOTE_CHARS)));
        }
        if !task.result_files.is_empty() {
            line.push_str(&format!(". Файлы: {}", task.result_files.join("; ")));
        }
        lines.push(line);
    }
    lines.push("Полный итог — task_status.".into());
    Some(lines.join("\n"))
}

/// Файлы, изменённые в папке получателя с начала задачи: пути результата. Ссылки, скрытые и секретные пропускаем.
fn fresh_files(store: &crate::store::Store, bot_id: &str, since_ms: i64) -> Vec<String> {
    let workspace = store.workspace(bot_id);
    let Ok(root) = std::fs::canonicalize(&workspace) else { return Vec::new() };
    let mut found: Vec<(i64, PathBuf)> = Vec::new();
    let mut visited = 0usize;
    let mut stack = vec![(root.clone(), 0u8)];
    while let Some((dir, level)) = stack.pop() {
        let Ok(read) = std::fs::read_dir(&dir) else { continue };
        for entry in read.flatten() {
            visited += 1;
            if visited > 2_000 {
                break;
            }
            let path = entry.path();
            let name = entry.file_name().to_string_lossy().to_string();
            if name.starts_with('.') || is_link(&path) {
                continue;
            }
            let Ok(meta) = entry.metadata() else { continue };
            if meta.is_dir() {
                if level < 3 && !(level == 0 && name.eq_ignore_ascii_case("attachments")) {
                    stack.push((path, level + 1));
                }
            } else if let Some(modified) = meta.modified().ok().and_then(|m| m.duration_since(std::time::UNIX_EPOCH).ok()) {
                let at = modified.as_millis() as i64;
                if at >= since_ms - 1_000 {
                    found.push((at, path));
                }
            }
        }
    }
    found.sort_by(|a, b| b.0.cmp(&a.0));
    found
        .into_iter()
        .filter_map(|(_, p)| std::fs::canonicalize(&p).ok())
        .filter(|real| real.starts_with(&root))
        .filter(|real| crate::reach::secret_reason(real, store.data_dir(), &root).is_none() && crate::reach::safe_read_links(real))
        .take(RESULT_FILES)
        .map(|p| crate::reach::display(&p))
        .collect()
}

// ---------------------------------------------------------------------------
// Состояние
// ---------------------------------------------------------------------------

fn state_ru(state: &str) -> &'static str {
    match state {
        "queued" => "ждёт",
        "running" => "в работе",
        "done" => "готово",
        "error" => "ошибка",
        _ => "прервана перезапуском программы",
    }
}

fn ago(since: i64) -> String {
    let minutes = ((now() - since) / 60_000).max(0);
    match minutes {
        0 => "только что".into(),
        1..=59 => format!("{minutes} мин назад"),
        60..=1439 => format!("{} ч назад", minutes / 60),
        _ => format!("{} дн назад", minutes / 1440),
    }
}

fn describe(task: &Task) -> String {
    let mut line = format!("{} · {} · {}", task.id, task.to_name, state_ru(&task.state));
    if task.is_open() {
        line.push_str(&format!(" · дана {}", ago(task.created)));
    } else if task.finished > 0 {
        line.push_str(&format!(" · {}", ago(task.finished)));
    }
    line.push_str(&format!("\n  Задача: {}", one_line(&task.text, 120)));
    if !task.summary.is_empty() {
        line.push_str(&format!("\n  Итог: {}", task.summary));
    }
    if !task.result_files.is_empty() {
        line.push_str(&format!("\n  Файлы: {}", task.result_files.join("; ")));
    }
    line
}

fn status(app: &App, bot: &Bot, id: &str) -> String {
    let mine: Vec<Task> = app.tasks.all().into_iter().filter(|t| t.from == bot.id).collect();
    if !id.is_empty() {
        // Чужой и несуществующий номер отвечают одинаково: по ответу не узнать, есть ли он у другого бота.
        return mine.iter().find(|t| t.id == id).map(describe).unwrap_or_else(|| format!("Задачи {id} у тебя нет."));
    }
    let at = now();
    let mut shown: Vec<&Task> = mine.iter().filter(|t| t.is_open() || at - t.finished < KEEP_MS).collect();
    shown.sort_by_key(|t| std::cmp::Reverse(t.created));
    if shown.is_empty() {
        return "Выданных задач нет: ни идущих, ни закрытых за сутки.".into();
    }
    shown.iter().map(|t| describe(t)).collect::<Vec<_>>().join("\n")
}

fn team_status(app: &App, bot: &Bot, room_name: &str) -> Result<String, String> {
    let mut rooms = rooms_of(app, &bot.id);
    if !room_name.is_empty() {
        let wanted = room_name.to_lowercase();
        rooms.retain(|r| r.name.to_lowercase().contains(&wanted));
    }
    if rooms.is_empty() {
        return Err(if room_name.is_empty() { "ты не состоишь ни в одной команде".into() } else { format!("в твоих командах нет «{room_name}»") });
    }
    let tasks = app.tasks.all();
    let mut out: Vec<String> = Vec::new();
    for room in rooms {
        let lead = lead_of(app, &room);
        // Тексты задач и итоги чужих участников видит только ведущий; участник — свои.
        let is_lead = lead.as_deref() == Some(bot.id.as_str());
        let lead_name = lead.as_ref().and_then(|id| app.store.bot(id)).map(|b| b.name).unwrap_or_else(|| "не назначен".into());
        out.push(format!("Команда «{}», ведёт: {lead_name}", room.name));
        for id in &room.bot_ids {
            let Some(member) = app.store.bot(id) else { continue };
            let current = tasks.iter().filter(|t| t.to == member.id && t.is_open() && room.bot_ids.contains(&t.from)).max_by_key(|t| t.created);
            let sees = is_lead || member.id == bot.id;
            let busy = match current {
                Some(task) if sees => format!("занят: {}", one_line(&task.text, 80)),
                Some(_) => "занят".into(),
                None if app.turns.is_busy(&member.id) => "занят разговором".into(),
                None => "свободен".into(),
            };
            let last = tasks.iter().filter(|t| t.to == member.id && !t.is_open() && room.bot_ids.contains(&t.from)).max_by_key(|t| t.finished);
            let last = last
                .filter(|_| sees)
                .map(|t| format!(". Последний итог ({}): {}", ago(t.finished), if t.summary.is_empty() { state_ru(&t.state).to_string() } else { one_line(&t.summary, 160) }))
                .unwrap_or_default();
            let who = if member.id == bot.id { " (ты)" } else { "" };
            let role = if member.role.trim().is_empty() { String::new() } else { format!(", {}", one_line(&member.role, 60)) };
            out.push(format!("- {}{who}{role}: {busy}{last}", member.name));
        }
    }
    Ok(out.join("\n"))
}

// ---------------------------------------------------------------------------
// Передача файла
// ---------------------------------------------------------------------------

/// Ссылка (symlink, junction и любая точка повторной обработки).
fn is_link(path: &Path) -> bool {
    let Ok(meta) = std::fs::symlink_metadata(path) else { return false };
    if meta.file_type().is_symlink() {
        return true;
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        return meta.file_attributes() & 0x400 != 0;
    }
    #[cfg(not(windows))]
    false
}

/// Путь внутри папки бота: относительный как есть, полный — только если он лежит в этой папке.
fn relative_in_workspace(store: &crate::store::Store, bot_id: &str, raw: &str) -> Result<PathBuf, String> {
    if raw.is_empty() {
        return Err("не указан файл".into());
    }
    if raw.contains('\0') || raw.chars().count() > 300 {
        return Err("путь файла пустой или слишком длинный".into());
    }
    let path = Path::new(raw);
    if path.components().any(|c| matches!(c, Component::ParentDir)) {
        return Err(format!("{raw}: «..» в пути не допускается"));
    }
    if !crate::reach::is_absolute_like(raw) {
        return Ok(path.to_path_buf());
    }
    let norm = |p: &Path| crate::reach::display(p).replace('/', "\\").trim_end_matches('\\').to_lowercase();
    let wanted = norm(path);
    let own = store.workspace(bot_id);
    let mut bases = vec![own.clone()];
    if let Ok(real) = std::fs::canonicalize(&own) {
        bases.push(real);
    }
    for base in bases {
        let base = norm(&base);
        if let Some(rest) = wanted.strip_prefix(&format!("{base}\\")) {
            // Регистр букв нужен для самой ФС лишь на других системах: берём хвост из исходной строки.
            let cut = raw.len() - rest.len();
            let tail = raw.get(cut..).unwrap_or(rest);
            return Ok(PathBuf::from(tail.replace('/', "\\")));
        }
    }
    Err(format!("{raw}: передавать можно только файлы из своей рабочей папки"))
}

/// Файл для передачи: проверен целиком. Возвращает настоящий путь.
fn source_file(store: &crate::store::Store, bot_id: &str, raw: &str) -> Result<PathBuf, String> {
    let rel = relative_in_workspace(store, bot_id, raw)?;
    let rel_text = rel.to_string_lossy().to_string();
    let abs = store.safe_path(bot_id, &rel_text)?;
    // Ссылку на любой части пути не передаём, даже если она ведёт внутрь папки.
    let mut walk = store.workspace(bot_id);
    for part in rel.components() {
        if let Component::Normal(p) = part {
            walk.push(p);
            if is_link(&walk) {
                return Err(format!("{raw}: ссылки и junction передавать нельзя"));
            }
        }
    }
    let meta = std::fs::metadata(&abs).map_err(|_| format!("{raw}: такого файла нет"))?;
    if !meta.is_file() {
        return Err(format!("{raw}: это не файл"));
    }
    if meta.len() > MAX_SHARE_BYTES {
        return Err(format!("{raw}: файл больше 20 МБ ({} МБ)", meta.len() / 1024 / 1024));
    }
    let real = std::fs::canonicalize(&abs).map_err(|e| format!("{raw}: {e}"))?;
    if let Some(reason) = crate::reach::secret_reason(&real, store.data_dir(), &store.workspace(bot_id)) {
        return Err(format!("{raw}: {reason}, такой файл не передаём"));
    }
    if !crate::reach::safe_read_links(&real) {
        return Err(format!("{raw}: нельзя проверить, не связан ли файл с закрытыми данными"));
    }
    // Запускаемые и автозапускаемые файлы (.bat, .ps1, .js, .lnk, .exe…), потоки и короткие имена другому боту не передаём:
    // получатель мог бы запустить их или человек открыл бы двойным щелчком. Те же правила, что у записи файла.
    if crate::tools::flagged_target(&real) || crate::tools::flagged_target(&abs) {
        return Err(format!("{raw}: запускаемые файлы и файлы автозапуска другому боту не передаём"));
    }
    Ok(real)
}

/// Копирует не больше `max` байт в новый файл `dest` (`create_new`: чужой и ссылку не затираем). Больше — файл удаляется.
/// Длине из метаданных не верим: файл могли дописать, пока копируем.
fn copy_capped(mut from: impl std::io::Read, dest: &Path, max: u64) -> Result<u64, String> {
    use std::io::Read;
    let mut out = std::fs::OpenOptions::new().write(true).create_new(true).open(dest).map_err(|e| format!("не смог создать копию: {e}"))?;
    let result = std::io::copy(&mut (&mut from).take(max + 1), &mut out).map_err(|e| format!("не смог скопировать: {e}"));
    drop(out);
    match result {
        Ok(n) if n <= max => Ok(n),
        Ok(_) => {
            let _ = std::fs::remove_file(dest);
            Err(format!("файл больше {} МБ", max / 1024 / 1024))
        }
        Err(e) => {
            let _ = std::fs::remove_file(dest);
            Err(e)
        }
    }
}

/// Копирует файл из папки `from` в `attachments/` получателя. Возвращает путь внутри папки получателя.
pub async fn share(app: &App, from: &Bot, target: &Bot, raw: &str) -> Result<String, String> {
    let store = &app.store;
    let source = source_file(store, &from.id, raw)?;
    let name = source.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
    if name.is_empty() || !crate::store::plain_file_name(&name) || name.chars().count() > 120 {
        return Err(format!("{raw}: имя файла не годится"));
    }
    let date = chrono::Local::now().format("%Y-%m-%d");
    let mut rel = format!("attachments/{date}-{name}");
    let mut copy = 2;
    // Чужой файл с таким именем не затираем: у копий нумерация.
    while store.safe_path(&target.id, &rel).is_ok_and(|p| std::fs::symlink_metadata(&p).is_ok()) {
        rel = format!("attachments/{date}-{copy}-{name}");
        copy += 1;
    }
    let dest = store.safe_path(&target.id, &rel)?;
    if crate::tools::flagged_target(&dest) {
        return Err(format!("{raw}: имя копии похоже на запускаемый файл или короткое имя, не передаём"));
    }
    let parent = dest.parent().map(Path::to_path_buf).ok_or("нет папки для копии")?;
    tokio::fs::create_dir_all(&parent).await.map_err(|e| e.to_string())?;
    let workspace = std::fs::canonicalize(store.workspace(&target.id)).map_err(|e| e.to_string())?;
    let source_for_copy = source.clone();
    let dest_for_copy = dest.clone();
    // Всё, что чувствительно к гонке, — в одном блокирующем куске: открыли источник один раз, проверили ссылки прямо перед записью.
    tokio::task::spawn_blocking(move || -> Result<(), String> {
        // Папка вложений могла стать ссылкой наружу уже после создания: проверяем по настоящему пути прямо перед записью.
        if is_link(&parent) || !std::fs::canonicalize(&parent).is_ok_and(|real| real.starts_with(&workspace)) {
            return Err("папка вложений получателя — ссылка или вне его папки, передавать нельзя".into());
        }
        let file = std::fs::File::open(&source_for_copy).map_err(|e| format!("не смог открыть файл: {e}"))?;
        let meta = file.metadata().map_err(|e| e.to_string())?;
        if !meta.is_file() || meta.len() > MAX_SHARE_BYTES {
            return Err("файл не годится для передачи".into());
        }
        copy_capped(file, &dest_for_copy, MAX_SHARE_BYTES)?;
        // Копия должна лежать внутри папки получателя и быть обычным файлом: иначе стираем.
        let inside = std::fs::canonicalize(&dest_for_copy).is_ok_and(|real| real.starts_with(&workspace));
        if !inside || is_link(&dest_for_copy) {
            let _ = std::fs::remove_file(&dest_for_copy);
            return Err("копия оказалась вне папки получателя".into());
        }
        Ok(())
    })
    .await
    .map_err(|e| e.to_string())??;
    Ok(rel)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn task(id: &str, from: &str, to: &str, text: &str) -> Task {
        Task { id: id.into(), from: from.into(), to: to.into(), to_name: to.into(), text: text.into(), state: "queued".into(), created: now(), ..Task::default() }
    }

    fn temp(name: &str) -> PathBuf {
        let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("target").join("test-tmp").join(format!("team-tasks-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn the_same_text_to_the_same_bot_gives_the_same_task() {
        let dir = temp("dedupe");
        let tasks = Tasks::open(&dir);
        let (first, created) = tasks.begin(task("a", "lead", "pic", "Нарисуй   кота")).unwrap();
        assert!(created);
        let (again, created) = tasks.begin(task("b", "lead", "pic", "нарисуй кота")).unwrap();
        assert!(!created);
        assert_eq!(again.id, first.id);
        // Другому боту или от другого бота — отдельная задача.
        assert!(tasks.begin(task("c", "lead", "post", "нарисуй кота")).unwrap().1);
        assert!(tasks.begin(task("d", "other", "pic", "нарисуй кота")).unwrap().1);
        // Закрытая задача не мешает повторить.
        tasks.update("a", |t| t.state = "done".into());
        assert!(tasks.begin(task("e", "lead", "pic", "Нарисуй кота")).unwrap().1);
    }

    #[test]
    fn three_open_tasks_per_assignee_from_one_bot_is_the_limit() {
        let dir = temp("cap");
        let tasks = Tasks::open(&dir);
        for n in 0..MAX_OPEN {
            tasks.begin(task(&format!("t{n}"), "lead", "pic", &format!("задача {n}"))).unwrap();
        }
        let refused = tasks.begin(task("t9", "lead", "pic", "ещё одна")).unwrap_err();
        assert!(refused.contains("task_status"), "{refused}");
        assert!(tasks.begin(task("x", "other", "pic", "ещё одна")).is_ok(), "лимит у каждого отправителя свой");
        tasks.update("t0", |t| t.state = "error".into());
        assert!(tasks.begin(task("t10", "lead", "pic", "ещё одна")).is_ok(), "закрытая освобождает место");
    }

    #[test]
    fn a_restart_requeues_open_tasks_whatever_their_age() {
        let dir = temp("restart");
        let tasks = Tasks::open(&dir);
        tasks.begin(task("a", "lead", "pic", "ждёт")).unwrap();
        tasks.begin(task("b", "lead", "post", "идёт")).unwrap();
        tasks.update("b", |t| t.state = "running".into());
        tasks.begin(task("c", "lead", "doc", "готова")).unwrap();
        tasks.update("c", |t| t.state = "done".into());
        drop(tasks);
        let after = Tasks::open(&dir);
        assert_eq!(after.get("a").unwrap().state, "queued", "ждавшая остаётся в очереди");
        assert_eq!(after.get("b").unwrap().state, "queued", "шедшая встаёт в очередь заново");
        assert_eq!(after.get("c").unwrap().state, "done");
        let mut ids = after.queued_ids();
        ids.sort();
        assert_eq!(ids, vec!["a", "b"]);
        // Возраст не мешает: после перезапуска задача жива, пока её не закрыл журнал запусков.
        after.update("a", |t| t.created = now() - 24 * 3600 * 1000);
        after.update("b", |t| t.state = "running".into());
        drop(after);
        let again = Tasks::open(&dir);
        assert_eq!(again.get("a").unwrap().state, "queued");
        assert_eq!(again.get("b").unwrap().state, "queued");
    }

    #[test]
    fn share_stops_at_20mb_of_real_bytes() {
        use std::io::Read;
        assert_eq!(MAX_SHARE_BYTES, 20 * 1024 * 1024);
        let dir = temp("copy-cap");
        // Метаданные врут: читается больше, чем разрешено. Копия не остаётся.
        let dest = dir.join("big.bin");
        let err = copy_capped(std::io::repeat(7).take(5_000), &dest, 1_000).unwrap_err();
        assert!(err.contains("больше"), "{err}");
        assert!(!dest.exists(), "переполненная копия не удалена");
        // Ровно по границе проходит.
        let fine = dir.join("fine.bin");
        assert_eq!(copy_capped(std::io::repeat(7).take(1_000), &fine, 1_000).unwrap(), 1_000);
        assert_eq!(std::fs::metadata(&fine).unwrap().len(), 1_000);
        // Чужой файл с этим именем не затирается.
        assert!(copy_capped(std::io::repeat(1).take(10), &fine, 1_000).is_err());
        assert_eq!(std::fs::metadata(&fine).unwrap().len(), 1_000, "существующий файл перезаписан");
        // Настоящий предел — 20 МБ: на нём копия обрывается и стирается.
        let huge = dir.join("huge.bin");
        assert!(copy_capped(std::io::repeat(0).take(MAX_SHARE_BYTES + 10), &huge, MAX_SHARE_BYTES).is_err());
        assert!(!huge.exists());
    }

    #[test]
    fn the_status_text_shows_only_the_askers_tasks_and_hides_foreign_ids() {
        let mine = Task { from: "me".into(), ..task("task_1", "me", "pic", "моя") };
        assert!(describe(&mine).contains("task_1") && describe(&mine).contains("ждёт"));
        assert_eq!(state_ru("interrupted"), "прервана перезапуском программы");
    }
}
