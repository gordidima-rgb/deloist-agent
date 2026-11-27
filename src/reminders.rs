//! Напоминания без модели: срок стреляет таймером, ход бота не начинается и модель не вызывается.
//! Свои поля: `leadMinutes` (0, 5, 15 или 60: за сколько до события слать), `channels` (["windows"] по умолчанию,
//! или "telegram"), `onceAt` (разовое напоминание, мс; нет — повтор по `at`/`days`). Текст — поле `prompt`, название — `title`.

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use serde_json::{json, Value};

use serde::{Deserialize, Serialize};

use crate::schedule::{self, Timing};
use crate::store::now;
use crate::App;

/// Значение `kind` в запросе и в записи: так интерфейс отличает напоминание.
pub const KIND: &str = "reminder";
pub const LEADS: [i64; 4] = [0, 5, 15, 60];
pub const CHANNELS: [&str; 2] = ["windows", "telegram"];
const MAX_TEXT_CHARS: usize = 500;
const MINUTE_MS: i64 = 60_000;
/// Если компьютер был выключен дольше этого, напоминание не шлём: «в 9» в одиннадцать вечера уже не напоминание.
pub const OVERDUE_MS: i64 = 12 * 3_600_000;

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Reminder {
    pub id: String,
    pub kind: String,
    pub title: String,
    /// Текст напоминания.
    pub prompt: String,
    pub enabled: bool,
    #[serde(flatten)]
    pub when: Timing,
    /// За сколько минут до события слать: 0, 5, 15 или 60.
    pub lead_minutes: i64,
    /// Каналы доставки; пусто — уведомление Windows.
    pub channels: Vec<String>,
    pub last_run: i64,
    pub next_run: i64,
    pub created_at: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_status: Option<String>,
    /// Подробности последней отправки по каналам.
    #[serde(skip_serializing_if = "Value::is_null")]
    pub last_delivery: Value,
}

impl Default for Reminder {
    fn default() -> Self {
        Self {
            id: String::new(),
            kind: KIND.into(),
            title: String::new(),
            prompt: String::new(),
            enabled: true,
            when: Timing::default(),
            lead_minutes: 0,
            channels: Vec::new(),
            last_run: 0,
            next_run: 0,
            created_at: 0,
            last_status: None,
            last_delivery: Value::Null,
        }
    }
}

fn lead_ms(reminder: &Reminder) -> i64 {
    reminder.lead_minutes.max(0) * MINUTE_MS
}

pub fn channels_of(reminder: &Reminder) -> Vec<String> {
    or_windows(reminder.channels.clone())
}

/// Пустой список каналов означает уведомление Windows: так решено для рутин и для привычек.
pub fn or_windows(list: Vec<String>) -> Vec<String> {
    if list.is_empty() { vec!["windows".to_string()] } else { list }
}

/// Срок отправки, если считать от `now_ms`: момент события минус опережение.
/// Разовое и повторяющееся считаются через schedule::next_run_ms (одна дата).
pub fn schedule_from(reminder: &Reminder, now_ms: i64) -> i64 {
    schedule::next_run_ms(&reminder.when, now_ms) - lead_ms(reminder)
}

/// Срок пропущен больше чем на 12 часов: компьютер был выключен, напоминание уже не к месту.
pub fn overdue(fire_at: i64, now_ms: i64) -> bool {
    now_ms - fire_at > OVERDUE_MS
}

/// Поля напоминания из запроса: только переданные. При создании сюда приходят все поля запроса.
pub fn apply(reminder: &mut Reminder, input: &Value) {
    if let Some(lead) = input.get("leadMinutes").and_then(Value::as_i64) {
        reminder.lead_minutes = lead;
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
        reminder.channels = wanted;
    }
    if let Some(value) = input.get("onceAt") {
        reminder.when.once_at = value.as_i64().filter(|ms| *ms > 0).unwrap_or(0);
    }
}

/// Проверка напоминания. `existing` — сохранённое напоминание при правке: недостающие поля берутся из него.
pub fn validate(input: &Value, existing: Option<&Reminder>) -> Result<(), String> {
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
        None => existing.is_some_and(|r| r.when.once_at > 0),
    };
    if !once {
        let at = match input.get("at") {
            Some(value) => value.as_str().unwrap_or_default().trim().to_string(),
            None => existing.map(|r| r.when.at.clone()).unwrap_or_default(),
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
pub async fn deliver(app: &Arc<App>, reminder: &Reminder) -> Vec<Delivery> {
    app.turns.emit(json!({ "type": "reminder", "routineId": reminder.id, "title": reminder.title, "text": reminder.prompt }));
    send_text(app, "Напоминание", &reminder.prompt, &channels_of(reminder)).await
}

/// Отправить текст по списку каналов. Ошибка одного канала не мешает остальным.
/// Напоминания и привычки шлют через неё: каналы одни и те же.
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
pub async fn fire(app: &Arc<App>, reminder: &Reminder) {
    let at = now();
    let (status, detail) = if overdue(reminder.next_run, at) {
        ("missed", json!([{ "channel": null, "ok": false, "error": "пропущено: компьютер был выключен больше 12 часов" }]))
    } else {
        let list = deliver(app, reminder).await;
        (status_of(&list), deliveries_json(&list))
    };
    let once = reminder.when.once_at > 0;
    // Следующий срок — первый, что ещё не прошёл: после долгого простоя пропущенные не копятся.
    let mut next = if once { reminder.next_run } else { schedule_from(reminder, reminder.next_run + lead_ms(reminder)) };
    let mut guard = 0;
    while !once && next <= at && guard < 400 {
        next = schedule_from(reminder, next + lead_ms(reminder));
        guard += 1;
    }
    if let Err(e) = app.reminders.finish(&reminder.id, status, detail, next, once) {
        eprintln!("[reminder] {}: {e}", reminder.title);
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
    items: Mutex<Vec<Reminder>>,
    broken: AtomicBool,
}

impl Reminders {
    pub fn open(data_dir: std::path::PathBuf) -> Self {
        let path = data_dir.join("reminders.json");
        let mut broken = false;
        let saved: Vec<Reminder> = match std::fs::read_to_string(&path) {
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

    pub fn list(&self) -> Vec<Reminder> {
        self.inner.items.lock().unwrap().clone()
    }

    pub fn get(&self, id: &str) -> Option<Reminder> {
        self.inner.items.lock().unwrap().iter().find(|r| r.id == id).cloned()
    }

    pub fn add_reminder(&self, input: &Value) -> std::io::Result<Reminder> {
        if !self.writable() {
            return Err(std::io::Error::other("reminders store broken"));
        }
        let mut reminder = Reminder {
            id: crate::store::uid("rt"),
            title: crate::store::take_chars(&crate::store::text_or(input, "title", ""), 80),
            prompt: crate::store::text_or(input, "prompt", ""),
            enabled: input.get("enabled").map_or(true, |v| v != &Value::Bool(false)),
            created_at: crate::store::now(),
            ..Reminder::default()
        };
        apply(&mut reminder, input);
        if reminder.title.is_empty() {
            reminder.title = crate::store::take_chars(&reminder.prompt, 80);
        }
        reminder.when.every_minutes = crate::store::minutes(input.get("everyMinutes"));
        reminder.when.apply(input).map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidInput, e))?;
        reminder.next_run = schedule_from(&reminder, crate::store::now());
        self.inner.items.lock().unwrap().push(reminder.clone());
        self.save();
        Ok(reminder)
    }

    pub fn update_reminder(&self, id: &str, patch: &Value) -> std::io::Result<Option<Reminder>> {
        if !self.writable() {
            return Err(std::io::Error::other("reminders store broken"));
        }
        let updated = {
            let mut items = self.inner.items.lock().unwrap();
            let Some(r) = items.iter_mut().find(|r| r.id == id) else { return Ok(None); };
            if let Some(v) = patch.get("enabled").filter(|v| !v.is_null()) { r.enabled = crate::store::truthy(v); }
            if let Some(v) = patch.get("prompt").filter(|v| !v.is_null()) { r.prompt = crate::store::as_string(v); }
            if let Some(v) = patch.get("title").filter(|v| !v.is_null()) { r.title = crate::store::as_string(v); }
            if patch.get("everyMinutes").is_some_and(|v| !v.is_null()) { r.when.every_minutes = crate::store::minutes(patch.get("everyMinutes")); }
            r.when.apply(patch).map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidInput, e))?;
            apply(r, patch);
            let rescheduled = ["everyMinutes", "everyMaxMinutes", "at", "days", "windowFrom", "windowTo", "leadMinutes", "recurrence", "startOn", "onceAt", "cron", "tz"].iter().any(|key| patch.get(*key).is_some_and(|v| !v.is_null()));
            if rescheduled {
                r.next_run = schedule_from(r, crate::store::now());
            }
            // Только для проверок: приблизить срок, не дожидаясь его.
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
            r.last_delivery = detail;
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

    fn reminder(at: &str, lead: i64) -> Reminder {
        Reminder { when: Timing { at: at.into(), ..Timing::default() }, prompt: "позвонить маме".into(), lead_minutes: lead, ..Reminder::default() }
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
        r.when.once_at = ms(5, 12, 0);
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
