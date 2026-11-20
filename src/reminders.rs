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
    if let Err(e) = app.store.finish_reminder(&routine.id, status, detail, next, once).await {
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
