//! Напоминания без модели. Рутина-напоминание (`extra.kind == "reminder"`) стреляет таймером:
//! ход бота не начинается и модель не вызывается. Бот подключается только к обычным рутинам с заданием.
//!
//! Поля в `extra` рутины: `leadMinutes` (0, 5, 15 или 60: за сколько до события слать),
//! `channels` (["windows"] по умолчанию, или "telegram"), `onceAt` (разовое напоминание, мс; нет — повтор
//! по `at`/`days`). Текст напоминания — поле `prompt`, название — `title`.

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use serde_json::{json, Value};

use crate::schedule;
use crate::store::{now, Routine};
use crate::App;

/// Значение `extra.kind` у рутины-напоминания. У обычных рутин этого поля нет.
pub const KIND: &str = "reminder";
pub const LEADS: [i64; 4] = [0, 5, 15, 60];
pub const CHANNELS: [&str; 2] = ["windows", "telegram"];
const MAX_TEXT_CHARS: usize = 500;
const MINUTE_MS: i64 = 60_000;
/// Если компьютер был выключен дольше этого, напоминание не шлём: «в 9» в одиннадцать вечера уже не напоминание.
pub const OVERDUE_MS: i64 = 12 * 3_600_000;

pub fn is_reminder(routine: &Routine) -> bool {
    routine.extra.get("kind").and_then(Value::as_str) == Some(KIND)
}

/// Запрос создаёт напоминание, если у него `kind: "reminder"`.
pub fn wants(input: &Value) -> bool {
    input.get("kind").and_then(Value::as_str) == Some(KIND)
}

fn lead_ms(routine: &Routine) -> i64 {
    routine.extra.get("leadMinutes").and_then(Value::as_i64).unwrap_or(0).max(0) * MINUTE_MS
}

/// Момент разового напоминания в мс; 0 — разового нет.
fn once_at(routine: &Routine) -> i64 {
    routine.once_at
}

pub fn channels_of(routine: &Routine) -> Vec<String> {
    let list: Vec<String> = routine
        .extra
        .get("channels")
        .and_then(Value::as_array)
        .map(|items| items.iter().filter_map(Value::as_str).map(str::to_string).collect())
        .unwrap_or_default();
    or_windows(list)
}

/// Пустой список каналов означает уведомление Windows: так решено для рутин и для привычек.
pub fn or_windows(list: Vec<String>) -> Vec<String> {
    if list.is_empty() { vec!["windows".to_string()] } else { list }
}

/// Срок отправки, если считать от `now_ms`: момент события минус опережение.
/// Разовое и повторяющееся считаются через schedule::next_run_ms (одна дата).
pub fn schedule_from(routine: &Routine, now_ms: i64) -> i64 {
    schedule::next_run_ms(routine, now_ms) - lead_ms(routine)
}

/// Срок пропущен больше чем на 12 часов: компьютер был выключен, напоминание уже не к месту.
pub fn overdue(fire_at: i64, now_ms: i64) -> bool {
    now_ms - fire_at > OVERDUE_MS
}

/// Поля напоминания из запроса: только переданные. При создании сюда приходят все поля запроса.
pub fn apply(routine: &mut Routine, input: &Value) {
    if let Some(lead) = input.get("leadMinutes").and_then(Value::as_i64) {
        routine.extra.insert("leadMinutes".into(), json!(lead));
    }
    if let Some(list) = input.get("channels").and_then(Value::as_array) {
        let mut wanted: Vec<String> = Vec::new();
        for channel in list.iter().filter_map(Value::as_str).filter(|c| CHANNELS.contains(c)) {
            if !wanted.iter().any(|c| c == channel) {
                wanted.push(channel.to_string());
            }
        }
        if wanted.is_empty() {
            wanted.push("windows".to_string());
        }
        routine.extra.insert("channels".into(), json!(wanted));
    }
    if let Some(value) = input.get("onceAt") {
        routine.once_at = value.as_i64().filter(|ms| *ms > 0).unwrap_or(0);
    }
}

/// Проверка напоминания. `existing` — сохранённое напоминание при правке: недостающие поля берутся из него.
pub fn validate(input: &Value, existing: Option<&Routine>) -> Result<(), String> {
    match input.get("prompt").filter(|v| !v.is_null()) {
        Some(text) => {
            let text = text.as_str().unwrap_or_default().trim();
            if text.is_empty() {
                return Err("нужен текст напоминания".into());
            }
            if text.chars().count() > MAX_TEXT_CHARS {
                return Err("текст напоминания не длиннее 500 знаков".into());
            }
        }
        None if existing.is_none() => return Err("нужен текст напоминания".into()),
        None => {}
    }
    if let Some(lead) = input.get("leadMinutes").filter(|v| !v.is_null()) {
        if !lead.as_i64().is_some_and(|m| LEADS.contains(&m)) {
            return Err("опережение: 0, 5, 15 или 60 минут".into());
        }
    }
    if let Some(list) = input.get("channels").filter(|v| !v.is_null()) {
        let known = list
            .as_array()
            .is_some_and(|items| !items.is_empty() && items.iter().all(|c| c.as_str().is_some_and(|c| CHANNELS.contains(&c))));
        if !known {
            return Err("каналы: windows или telegram".into());
        }
    }
    let once = match input.get("onceAt") {
        Some(value) if !value.is_null() => {
            if !value.as_i64().is_some_and(|ms| ms > 0) {
                return Err("дата напоминания — число миллисекунд больше нуля".into());
            }
            true
        }
        // Пустая дата снимает разовость: остаётся повтор по времени.
        Some(_) => false,
        None => existing.is_some_and(|r| once_at(r) > 0),
    };
    if !once {
        let at = match input.get("at") {
            Some(value) => value.as_str().unwrap_or_default().trim().to_string(),
            None => existing.map(|r| r.at.clone()).unwrap_or_default(),
        };
        if schedule::parse_time(&at).is_none() {
            return Err("нужны дата или время напоминания ЧЧ:ММ".into());
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Каналы доставки
// ---------------------------------------------------------------------------

/// Канал доставки. Будущие каналы добавляются реализацией, а не веткой в общем коде.
pub trait Channel: Send + Sync {
    fn id(&self) -> &'static str;
    fn send<'a>(&'a self, app: &'a Arc<App>, title: &'a str, body: &'a str) -> Pin<Box<dyn Future<Output = Result<(), String>> + Send + 'a>>;
}

/// Уведомление Windows. В тестах (сборка для отладки с `CREW_TEST_REMINDER_SINK`) пишет строку в файл, а не на экран.
pub struct Windows;

impl Channel for Windows {
    fn id(&self) -> &'static str {
        "windows"
    }

    fn send<'a>(&'a self, _app: &'a Arc<App>, title: &'a str, body: &'a str) -> Pin<Box<dyn Future<Output = Result<(), String>> + Send + 'a>> {
        Box::pin(async move {
            if cfg!(debug_assertions) {
                if let Ok(path) = std::env::var("CREW_TEST_REMINDER_SINK") {
                    use std::io::Write;
                    let line = json!({ "channel": "windows", "title": title, "body": body }).to_string();
                    let mut file = std::fs::OpenOptions::new().create(true).append(true).open(&path).map_err(|e| e.to_string())?;
                    return writeln!(file, "{line}").map_err(|e| e.to_string());
                }
            }
            let (title, body) = (title.to_string(), body.to_string());
            tokio::task::spawn_blocking(move || show_toast(&title, &body)).await.map_err(|e| e.to_string())?
        })
    }
}

/// Сообщение в привязанный чат Telegram. Без токена или чата — понятная причина, а не тишина.
pub struct Telegram;

impl Channel for Telegram {
    fn id(&self) -> &'static str {
        "telegram"
    }

    fn send<'a>(&'a self, app: &'a Arc<App>, title: &'a str, body: &'a str) -> Pin<Box<dyn Future<Output = Result<(), String>> + Send + 'a>> {
        Box::pin(async move { crate::telegram::push_text(app, &format!("{title}: {body}")).await })
    }
}

fn channel_for(id: &str) -> Option<Box<dyn Channel>> {
    match id {
        "windows" => Some(Box::new(Windows)),
        "telegram" => Some(Box::new(Telegram)),
        _ => None,
    }
}

#[derive(Debug, Clone)]
pub struct Delivery {
    pub channel: String,
    pub error: Option<String>,
}

/// Отправить напоминание по всем его каналам. Открытое окно получает событие `reminder` в любом случае.
pub async fn deliver(app: &Arc<App>, routine: &Routine) -> Vec<Delivery> {
    app.turns.emit(json!({ "type": "reminder", "routineId": routine.id, "title": routine.title, "text": routine.prompt }));
    send_text(app, "Напоминание", &routine.prompt, &channels_of(routine)).await
}

/// Отправить текст по списку каналов. Ошибка одного канала не мешает остальным.
/// Рутины-напоминания и привычки шлют через неё: каналы одни и те же.
pub async fn send_text(app: &Arc<App>, title: &str, body: &str, channels: &[String]) -> Vec<Delivery> {
    let mut out = Vec::new();
    for id in channels {
        let (name, result) = match channel_for(id) {
            Some(channel) => (channel.id().to_string(), channel.send(app, title, body).await),
            None => (id.clone(), Err(format!("неизвестный канал: {id}"))),
        };
        out.push(Delivery { channel: name, error: result.err() });
    }
    out
}

pub fn deliveries_json(list: &[Delivery]) -> Value {
    Value::Array(
        list.iter()
            .map(|d| json!({ "channel": d.channel, "ok": d.error.is_none(), "error": d.error }))
            .collect(),
    )
}

fn status_of(list: &[Delivery]) -> &'static str {
    let failed = list.iter().filter(|d| d.error.is_some()).count();
    match (failed, list.len()) {
        (0, _) => "ok",
        (f, n) if f == n => "error",
        _ => "partial",
    }
}

/// Срок напоминания подошёл: шлём (или отмечаем пропуск) и ставим следующий срок. Модель не вызывается.
pub async fn fire(app: &Arc<App>, routine: &Routine) {
    let at = now();
    let (status, detail) = if overdue(routine.next_run, at) {
        ("missed", json!([{ "channel": null, "ok": false, "error": "пропущено: компьютер был выключен больше 12 часов" }]))
    } else {
        let list = deliver(app, routine).await;
        (status_of(&list), deliveries_json(&list))
    };
    let once = once_at(routine) > 0;
    // Следующий срок — первый, что ещё не прошёл: после долгого простоя пропущенные не копятся.
    let mut next = if once { routine.next_run } else { schedule_from(routine, routine.next_run + lead_ms(routine)) };
    let mut guard = 0;
    while !once && next <= at && guard < 400 {
        next = schedule_from(routine, next + lead_ms(routine));
        guard += 1;
    }
    if let Err(e) = app.reminders.finish(&routine.id, status, detail, next, once) {
        eprintln!("[reminder] {}: {e}", routine.title);
    }
}

// ---------------------------------------------------------------------------
// Уведомление Windows
// ---------------------------------------------------------------------------

#[cfg(windows)]
fn show_toast(title: &str, body: &str) -> Result<(), String> {
    use windows::core::HSTRING;
    use windows::Data::Xml::Dom::XmlDocument;
    use windows::UI::Notifications::{ToastNotification, ToastNotificationManager};
    use windows::Win32::System::WinRT::{RoInitialize, RO_INIT_MULTITHREADED};

    fn err(e: windows::core::Error) -> String {
        e.to_string()
    }
    // Потоку пула может не хватить WinRT-инициализации; если уже есть другая — код вернёт, это нормально.
    let _ = unsafe { RoInitialize(RO_INIT_MULTITHREADED) };
    let xml = XmlDocument::new().map_err(err)?;
    let markup = format!(
        "<toast><visual><binding template=\"ToastGeneric\"><text>{}</text><text>{}</text></binding></visual></toast>",
        xml_escape(title),
        xml_escape(body)
    );
    xml.LoadXml(&HSTRING::from(markup)).map_err(err)?;
    let toast = ToastNotification::CreateToastNotification(&xml).map_err(err)?;
    let app_id = HSTRING::from(crate::aumid::notification_app_id()?);
    let notifier = ToastNotificationManager::CreateToastNotifierWithId(&app_id).map_err(err)?;
    notifier.Show(&toast).map_err(err)
}

#[cfg(not(windows))]
fn show_toast(_title: &str, _body: &str) -> Result<(), String> {
    Ok(())
}

pub(crate) fn xml_escape(text: &str) -> String {
    text.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;").replace('\'', "&apos;")
}

// ---------------------------------------------------------------------------
// Своё хранилище напоминаний: reminders.json в папке данных.
// ---------------------------------------------------------------------------

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;

#[derive(Clone)]
pub struct Reminders {
    inner: Arc<RemindersInner>,
}

struct RemindersInner {
    path: std::path::PathBuf,
    items: Mutex<Vec<Routine>>,
    broken: AtomicBool,
}

impl Reminders {
    pub fn open(data_dir: std::path::PathBuf) -> Self {
        let path = data_dir.join("reminders.json");
        let mut broken = false;
        let saved: Vec<Routine> = match std::fs::read_to_string(&path) {
            Ok(text) => {
                // Формат: { "reminders": [...] } или голый массив (на всякий случай).
                let parsed: Result<serde_json::Value, _> = serde_json::from_str(&text);
                match parsed {
                    Ok(value) => {
                        if let Some(arr) = value.get("reminders").and_then(|v| v.as_array()) {
                            arr.iter().filter_map(|v| serde_json::from_value(v.clone()).ok()).collect()
                        } else if let Some(arr) = value.as_array() {
                            arr.iter().filter_map(|v| serde_json::from_value(v.clone()).ok()).collect()
                        } else {
                            broken = true;
                            Vec::new()
                        }
                    }
                    Err(_) => {
                        broken = true;
                        Vec::new()
                    }
                }
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Vec::new(),
            Err(_) => {
                broken = true;
                Vec::new()
            }
        };
        if broken {
            let _ = std::fs::copy(&path, path.with_extension("json.bak"));
        }
        Self { inner: Arc::new(RemindersInner { path, items: Mutex::new(saved), broken: AtomicBool::new(broken) }) }
    }

    fn writable(&self) -> bool {
        !self.inner.broken.load(Ordering::Relaxed)
    }

    fn save(&self) {
        if !self.writable() { return; }
        let items = self.inner.items.lock().unwrap().clone();
        let text = match serde_json::to_string_pretty(&serde_json::json!({ "reminders": items })) {
            Ok(t) => t,
            Err(_) => return,
        };
        let tmp = self.inner.path.with_extension("json.tmp");
        if std::fs::write(&tmp, text).is_ok() {
            let _ = std::fs::rename(&tmp, &self.inner.path);
        }
    }

    pub fn list(&self) -> Vec<Routine> {
        self.inner.items.lock().unwrap().clone()
    }

    pub fn get(&self, id: &str) -> Option<Routine> {
        self.inner.items.lock().unwrap().iter().find(|r| r.id == id).cloned()
    }

    pub fn add_reminder(&self, input: &Value, human: bool) -> std::io::Result<Routine> {
        if !self.writable() {
            return Err(std::io::Error::other("reminders store broken"));
        }
        let reminder = wants(input);
        let mut routine = Routine {
            id: crate::store::uid("rt"),
            bot_id: if reminder { String::new() } else { crate::store::text_or(input, "botId", "") },
            title: crate::store::take_chars(&crate::store::text_or(input, "title", if reminder { "" } else { "Рутина" }), 80),
            prompt: crate::store::text_or(input, "prompt", ""),
            every_minutes: crate::store::minutes(input.get("everyMinutes")),
            enabled: input.get("enabled").map_or(true, |v| v != &serde_json::Value::Bool(false)),
            quiet: input.get("quiet").map_or(true, crate::store::truthy),
            catch_up: crate::store::text_or(input, "catchUp", "last"),
            created_at: crate::store::now(),
            ..Routine::default()
        };
        if reminder {
            routine.extra.insert("kind".into(), serde_json::json!(KIND));
            apply(&mut routine, input);
            if routine.title.is_empty() {
                routine.title = crate::store::take_chars(&routine.prompt, 80);
            }
        }
        crate::store::apply_schedule(&mut routine, input).map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidInput, e))?;
        routine.hook = input.get("hook").is_some_and(crate::store::truthy);
        routine.goal_id = crate::store::text_or(input, "goalId", "");
        routine.approval_floor = crate::store::text_or(input, "approvalFloor", "");
        if human {
            routine.check = crate::store::text_or(input, "check", "");
        }
        if let Some(v) = input.get("hookEvents").filter(|v| !v.is_null()) {
            routine.hook_events = crate::store::validate_hook_events(v).map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
        }
        if routine.hook {
            routine.hook_token = crate::guard::random_token();
            routine.next_run = 0;
        } else {
            routine.next_run = schedule_from(&routine, crate::store::now());
        }
        {
            let mut items = self.inner.items.lock().unwrap();
            items.push(routine.clone());
        }
        self.save();
        Ok(routine)
    }

    pub fn update_reminder(&self, id: &str, patch: &Value, human: bool) -> std::io::Result<Option<Routine>> {
        if !self.writable() {
            return Err(std::io::Error::other("reminders store broken"));
        }
        let updated = {
            let mut items = self.inner.items.lock().unwrap();
            let Some(r) = items.iter_mut().find(|r| r.id == id) else { return Ok(None); };
            if let Some(v) = patch.get("enabled").filter(|v| !v.is_null()) { r.enabled = crate::store::truthy(v); }
            if let Some(v) = patch.get("quiet").filter(|v| !v.is_null()) { r.quiet = crate::store::truthy(v); }
            if let Some(v) = patch.get("prompt").filter(|v| !v.is_null()) { r.prompt = crate::store::as_string(v); }
            if let Some(v) = patch.get("title").filter(|v| !v.is_null()) { r.title = crate::store::as_string(v); }
            if let Some(v) = patch.get("goalId").filter(|v| !v.is_null()) { r.goal_id = crate::store::as_string(v); }
            if let Some(v) = patch.get("approvalFloor").filter(|v| !v.is_null()) { r.approval_floor = crate::store::as_string(v); }
            if let Some(v) = patch.get("errorPaused").filter(|v| !v.is_null()) {
                r.error_paused = crate::store::truthy(v);
                if !r.error_paused { r.failures = 0; r.stop_reason = None; }
            }
            if let Some(v) = patch.get("catchUp").filter(|v| !v.is_null()) { r.catch_up = crate::store::as_string(v); }
            if let Some(v) = patch.get("progressNote").filter(|v| !v.is_null()) { r.progress_note = crate::store::as_string(v); }
            if human {
                if let Some(v) = patch.get("check").filter(|v| !v.is_null()) { r.check = crate::store::as_string(v); }
            }
            if patch.get("everyMinutes").is_some_and(|v| !v.is_null()) { r.every_minutes = crate::store::minutes(patch.get("everyMinutes")); }
            if let Some(v) = patch.get("hook").filter(|v| !v.is_null()) {
                let want = crate::store::truthy(v);
                if want && !r.hook { r.hook_token = crate::guard::random_token(); }
                if !want { r.hook_token.clear(); }
                r.hook = want;
            }
            if let Some(v) = patch.get("hookEvents").filter(|v| !v.is_null()) {
                r.hook_events = crate::store::validate_hook_events(v).map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
            }
            crate::store::apply_schedule(r, patch).map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidInput, e))?;
            if is_reminder(r) { apply(r, patch); }
            let rescheduled = ["everyMinutes","everyMaxMinutes","at","days","windowFrom","windowTo","hook","leadMinutes","recurrence","startOn","onceAt","cron","tz"].iter().any(|key| patch.get(*key).is_some_and(|v| !v.is_null()));
            if rescheduled || !r.enabled || r.hook { r.failures = 0; r.stop_reason = None; }
            if r.hook { r.next_run = 0; } else if rescheduled { r.next_run = schedule_from(r, crate::store::now()); }
            if cfg!(debug_assertions) {
                if let Some(at) = crate::store::number(patch.get("nextRun")) { r.next_run = at as i64; }
            }
            r.clone()
        };
        self.save();
        Ok(Some(updated))
    }

    pub fn remove_reminder(&self, id: &str) -> std::io::Result<bool> {
        let mut items = self.inner.items.lock().unwrap();
        let Some(idx) = items.iter().position(|r| r.id == id) else { return Ok(false); };
        items.remove(idx);
        drop(items);
        self.save();
        Ok(true)
    }

    pub fn finish(&self, id: &str, status: &str, detail: serde_json::Value, next_run: i64, disable: bool) -> std::io::Result<()> {
        let mut items = self.inner.items.lock().unwrap();
        if let Some(r) = items.iter_mut().find(|r| r.id == id) {
            r.last_run = crate::store::now();
            r.last_status = Some(status.to_string());
            r.extra.insert("lastDelivery".into(), detail);
            r.next_run = next_run;
            if disable { r.enabled = false; }
        }
        drop(items);
        self.save();
        Ok(())
    }

    pub fn is_broken(&self) -> bool { self.inner.broken.load(Ordering::Relaxed) }
}

// crate::store::number may be private; add accessor? We'll implement number parsing here if needed.
// Actually used crate::store::number above; ensure pub(crate).
#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{Local, TimeZone};

    /// Ноябрь 2026, местное время компьютера: расчёт идёт в местных часах, поэтому и дату задаём так же.
    fn ms(day: u32, hour: u32, minute: u32) -> i64 {
        Local.with_ymd_and_hms(2026, 11, day, hour, minute, 0).unwrap().timestamp_millis()
    }

    fn reminder(at: &str, lead: i64) -> Routine {
        let mut r = Routine { at: at.into(), prompt: "позвонить маме".into(), ..Routine::default() };
        r.extra.insert("kind".into(), json!(KIND));
        r.extra.insert("leadMinutes".into(), json!(lead));
        r
    }

    #[test]
    fn lead_time_moves_the_moment_back() {
        let r = reminder("09:00", 15);
        assert_eq!(schedule_from(&r, ms(2, 8, 0)), ms(2, 8, 45));
        // В 8:50 срок уже позади (8:45): напоминание уйдёт сразу. После отправки следующий срок — завтра.
        assert_eq!(schedule_from(&r, ms(2, 8, 50)), ms(2, 8, 45));
        assert_eq!(schedule_from(&r, ms(2, 9, 0) + 15 * MINUTE_MS), ms(3, 8, 45));
    }

    #[test]
    fn no_lead_fires_at_the_moment() {
        let r = reminder("09:00", 0);
        assert_eq!(schedule_from(&r, ms(2, 8, 0)), ms(2, 9, 0));
    }

    #[test]
    fn one_off_uses_its_date_minus_lead() {
        let mut r = reminder("", 60);
        r.once_at = ms(5, 12, 0);
        assert_eq!(schedule_from(&r, ms(2, 8, 0)), ms(5, 11, 0));
    }

    #[test]
    fn overdue_is_more_than_twelve_hours() {
        let fire = ms(2, 9, 0);
        assert!(!overdue(fire, fire + OVERDUE_MS));
        assert!(overdue(fire, fire + OVERDUE_MS + 1));
    }

    #[test]
    fn channels_default_to_windows_and_drop_unknown() {
        let mut r = reminder("09:00", 0);
        assert_eq!(channels_of(&r), vec!["windows".to_string()]);
        apply(&mut r, &json!({ "channels": ["telegram", "sms", "telegram"] }));
        assert_eq!(channels_of(&r), vec!["telegram".to_string()]);
    }

    #[test]
    fn validation_rejects_bad_fields() {
        let ok = json!({ "kind": "reminder", "prompt": "вынести мусор", "at": "09:00", "leadMinutes": 5, "channels": ["windows"] });
        assert!(validate(&ok, None).is_ok());
        assert!(validate(&json!({ "kind": "reminder", "prompt": "  ", "at": "09:00" }), None).is_err());
        assert!(validate(&json!({ "kind": "reminder", "prompt": "x".repeat(501), "at": "09:00" }), None).is_err());
        assert!(validate(&json!({ "kind": "reminder", "prompt": "x", "at": "09:00", "leadMinutes": 7 }), None).is_err());
        assert!(validate(&json!({ "kind": "reminder", "prompt": "x", "at": "09:00", "channels": ["sms"] }), None).is_err());
        assert!(validate(&json!({ "kind": "reminder", "prompt": "x", "onceAt": -1 }), None).is_err());
        assert!(validate(&json!({ "kind": "reminder", "prompt": "x" }), None).is_err());
    }

    #[test]
    fn a_patch_checks_only_what_it_changes() {
        let r = reminder("09:00", 0);
        assert!(validate(&json!({ "leadMinutes": 15 }), Some(&r)).is_ok());
        // Убрать время у повторяющегося напоминания без разовой даты нельзя.
        assert!(validate(&json!({ "at": "" }), Some(&r)).is_err());
    }

    #[test]
    fn status_reflects_channel_results() {
        let ok = Delivery { channel: "windows".into(), error: None };
        let bad = Delivery { channel: "telegram".into(), error: Some("Telegram не подключён".into()) };
        assert_eq!(status_of(&[ok.clone()]), "ok");
        assert_eq!(status_of(&[bad.clone()]), "error");
        assert_eq!(status_of(&[ok, bad]), "partial");
    }
}
