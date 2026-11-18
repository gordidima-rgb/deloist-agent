//! Привычки: отметки, серия, предел, отмена, инструменты бота.

use crate::common;

use std::sync::{Arc, Mutex};

use axum::body::Bytes;
use axum::extract::State;
use axum::response::{IntoResponse, Response};
use axum::routing::post;
use axum::{Json, Router};
use chrono::{Datelike, Duration, Local, NaiveDate};
use common::Crew;
use serde_json::{json, Value};

fn today() -> NaiveDate {
    Local::now().date_naive()
}

fn day_before(n: i64) -> String {
    (today() - Duration::days(n)).format("%Y-%m-%d").to_string()
}

async fn make(crew: &Crew, title: &str, days_mask: u32) -> String {
    let made = crew.ok("POST", "/api/habits", Some(json!({ "title": title, "icon": "drop", "days_mask": days_mask }))).await;
    assert_eq!(made["days_mask"], days_mask);
    made["id"].as_str().unwrap().to_string()
}

async fn check(crew: &Crew, id: &str, date: Option<&str>) -> Value {
    let body = match date {
        Some(day) => json!({ "date": day }),
        None => json!({}),
    };
    crew.ok("POST", &format!("/api/habits/{id}/check"), Some(body)).await
}

#[tokio::test(flavor = "multi_thread")]
async fn create_check_twice_and_the_streak_counts_one() {
    let crew = Crew::start("habits-basic").await;
    let id = make(&crew, "Выпить воды", 127).await;

    let first = check(&crew, &id, None).await;
    assert_eq!(first["today_done"], true);
    assert_eq!(first["streak"], 1);
    assert_eq!(first["week"].as_array().unwrap().len(), 7);

    let again = check(&crew, &id, None).await;
    assert_eq!(again["total_done"], 1, "повторная отметка того же дня не дублирует");
    assert_eq!(again["streak"], 1);

    let list = crew.ok("GET", "/api/habits", None).await;
    assert_eq!(list.as_array().unwrap().len(), 1);
    assert_eq!(list[0]["today_done"], true);
}

#[tokio::test(flavor = "multi_thread")]
async fn an_unscheduled_day_does_not_break_the_streak() {
    let crew = Crew::start("habits-skip").await;
    // Каждый день, кроме вчерашнего дня недели: вчера не расписано.
    let yesterday = today() - Duration::days(1);
    let skipped = yesterday.weekday().num_days_from_monday();
    let mask = 127u32 & !(1u32 << skipped);
    let id = make(&crew, "Чтение", mask).await;

    check(&crew, &id, None).await;
    check(&crew, &id, Some(&day_before(2))).await;
    let streak = check(&crew, &id, Some(&day_before(3))).await;
    assert_eq!(streak["streak"], 3, "сегодня, позавчера, третьего дня; вчера не расписано и серию не рвёт");

    // Отмена сегодняшней отметки: сегодня открыт, серия идёт с позавчера.
    let (status, undone) = crew.send("DELETE", &format!("/api/habits/{id}/check?date={}", today().format("%Y-%m-%d")), None).await;
    assert_eq!(status, 200);
    assert_eq!(undone["today_done"], false);
    assert_eq!(undone["streak"], 2);
}

#[tokio::test(flavor = "multi_thread")]
async fn the_fifty_first_habit_is_refused_and_a_future_day_too() {
    let crew = Crew::start("habits-limit").await;
    let mut first = String::new();
    for n in 0..50 {
        let id = make(&crew, &format!("Привычка {n}"), 127).await;
        if n == 0 {
            first = id;
        }
    }
    let (status, body) = crew.send("POST", "/api/habits", Some(json!({ "title": "Пятьдесят первая", "icon": "drop" }))).await;
    assert_eq!(status, 409, "{body}");
    assert_eq!(body["error"], "habit_limit");

    // Архив освобождает место, отметка будущего дня отказывает.
    let (archived, _) = crew.send("DELETE", &format!("/api/habits/{first}"), None).await;
    assert_eq!(archived, 204);
    let (created, _) = crew.send("POST", "/api/habits", Some(json!({ "title": "Новая", "icon": "drop" }))).await;
    assert_eq!(created, 201);

    let tomorrow = (today() + Duration::days(1)).format("%Y-%m-%d").to_string();
    let (future, body) = crew.send("POST", &format!("/api/habits/{first}/check"), Some(json!({ "date": tomorrow }))).await;
    assert_eq!(future, 422, "{body}");
}

#[tokio::test(flavor = "multi_thread")]
async fn templates_come_in_four_groups_with_english_names() {
    let crew = Crew::start("habits-templates").await;
    let data = crew.ok("GET", "/api/habits/templates", None).await;
    assert_eq!(data["groups"].as_array().unwrap().len(), 4);
    let templates = data["templates"].as_array().unwrap();
    assert!(templates.len() >= 12);
    for template in templates {
        assert!(template["en"].as_str().is_some_and(|s| !s.is_empty()), "{template}");
    }
}

// --- Инструменты бота: сценарий модели по шагам, как в tests/core/routines_chat.rs ---

#[derive(Clone, Default)]
struct Fake {
    script: Arc<Mutex<Vec<Value>>>,
}

fn say(text: &str) -> Value {
    json!({ "role": "assistant", "content": text })
}

fn tool(name: &str, args: Value) -> Value {
    let id = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos();
    json!({ "role": "assistant", "content": "", "tool_calls": [{ "id": format!("c{id}"), "type": "function", "function": { "name": name, "arguments": args.to_string() } }] })
}

async fn completions(State(fake): State<Fake>, _body: Bytes) -> Response {
    let message = {
        let mut script = fake.script.lock().unwrap();
        if script.is_empty() {
            say("готово")
        } else {
            script.remove(0)
        }
    };
    Json(json!({ "choices": [{ "message": message }] })).into_response()
}

async fn setup(name: &str) -> (Crew, Fake, String) {
    let fake = Fake::default();
    let router = Router::new().route("/v1/chat/completions", post(completions)).with_state(fake.clone());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });

    let crew = Crew::start_with_env(name, &[]).await;
    crew.ok("PUT", "/api/settings", Some(json!({ "driver": "api", "baseUrl": format!("http://127.0.0.1:{port}/v1"), "apiKey": "k" }))).await;
    crew.listen().await;
    let state = crew.ok("GET", "/api/state", None).await;
    let bot = state["bots"][0]["id"].as_str().unwrap().to_string();
    (crew, fake, bot)
}

/// Сказать боту и дождаться конца хода; вернуть результат инструмента (статус и текст).
async fn ask_and_result(crew: &Crew, bot: &str, text: &str) -> (String, String) {
    crew.clear_events();
    crew.say(bot, text).await;
    crew.wait_event("конец хода", |e| e["type"] == "turn_end").await;
    let update = crew.find_event(|e| e["type"] == "tool_update" && e["status"] != "running").expect("инструмент не отвечал");
    (update["status"].as_str().unwrap().to_string(), update["result"].as_str().unwrap_or_default().to_string())
}

#[tokio::test(flavor = "multi_thread")]
async fn a_bot_checks_today_but_not_three_days_ago() {
    let (crew, fake, bot) = setup("habits-bot").await;
    let id = make(&crew, "Прогулка", 127).await;

    *fake.script.lock().unwrap() = vec![tool("check_habit", json!({ "habit": id })), say("Отметил")];
    let (status, result) = ask_and_result(&crew, &bot, "отметь прогулку").await;
    assert_eq!(status, "ok", "{result}");
    assert!(result.contains("Прогулка"), "{result}");
    let list = crew.ok("GET", "/api/habits", None).await;
    assert_eq!(list[0]["today_done"], true, "бот отметил сегодня");

    *fake.script.lock().unwrap() = vec![tool("check_habit", json!({ "habit": "Прогулка", "date": day_before(3) })), say("Не получилось")];
    let (status, result) = ask_and_result(&crew, &bot, "отметь прогулку за позавчера и ещё раньше").await;
    assert_ne!(status, "ok", "{result}");
    assert!(result.contains("сегодня или вчера"), "{result}");
    let list = crew.ok("GET", "/api/habits", None).await;
    assert_eq!(list[0]["total_done"], 1, "третий день не записан");
}

// --- Напоминания о привычках: тик планировщика, отправка через тестовый файл Windows ---

fn sink_lines(sink: &std::path::Path) -> Vec<Value> {
    std::fs::read_to_string(sink).unwrap_or_default().lines().filter_map(|l| serde_json::from_str(l).ok()).collect()
}

#[tokio::test(flavor = "multi_thread")]
async fn a_habit_reminder_fires_once_and_only_when_due() {
    let sink = std::path::PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("habits-remind-sink.jsonl");
    let _ = std::fs::remove_file(&sink);
    let sink_text = sink.display().to_string();
    let crew = Crew::start_with_env("habits-remind", &[("CREW_TEST_TICK_MS", "100"), ("CREW_TEST_REMINDER_SINK", &sink_text)]).await;

    // Время «сейчас» (ЧЧ:ММ) уже прошло: напоминание должно уйти на ближайшем тике.
    let now = Local::now();
    let at = now.format("%H:%M").to_string();
    let (status, due) = crew
        .send("POST", "/api/habits", Some(json!({ "title": "Выпить воды", "icon": "drop", "days_mask": 127, "remind_at": at, "remind_channels": ["windows"] })))
        .await;
    assert_eq!(status, 201, "{due}");
    assert_eq!(due["remind_at"], at.as_str());
    assert_eq!(due["remind_channels"], json!(["windows"]));
    let due_id = due["id"].as_str().unwrap().to_string();

    // Сегодня не расписано: напоминания быть не должно.
    let weekday_bit = 1u32 << now.weekday().num_days_from_monday();
    let (status, _) = crew
        .send("POST", "/api/habits", Some(json!({ "title": "Не сегодня", "icon": "drop", "days_mask": 127 & !weekday_bit, "remind_at": at, "remind_channels": ["windows"] })))
        .await;
    assert_eq!(status, 201);

    // Отмечена сегодня до времени напоминания: тоже молчим.
    let done = make(&crew, "Уже сделана", 127).await;
    check(&crew, &done, None).await;
    let (status, _) = crew.send("PATCH", &format!("/api/habits/{done}"), Some(json!({ "remind_at": at }))).await;
    assert_eq!(status, 200);

    common::wait_until("напоминание о привычке", 20, || async { !sink_lines(&sink).is_empty() }).await;
    // Несколько тиков спустя второго напоминания быть не должно.
    tokio::time::sleep(std::time::Duration::from_millis(800)).await;
    let lines = sink_lines(&sink);
    assert_eq!(lines.len(), 1, "{lines:?}");
    assert_eq!(lines[0]["channel"], "windows");
    assert_eq!(lines[0]["title"], "Напоминание");
    assert_eq!(lines[0]["body"], "Выпить воды");

    // Неверное время отказывает, пустая строка снимает напоминание.
    let (bad, body) = crew.send("PATCH", &format!("/api/habits/{due_id}"), Some(json!({ "remind_at": "25:00" }))).await;
    assert_eq!(bad, 422, "{body}");
    let cleared = crew.ok("PATCH", &format!("/api/habits/{due_id}"), Some(json!({ "remind_at": "" }))).await;
    assert_eq!(cleared["remind_at"], Value::Null);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_bot_manages_a_habit_freely_but_archiving_it_always_asks() {
    let (crew, fake, bot) = setup("habits-manage").await;

    *fake.script.lock().unwrap() = vec![tool("create_habit", json!({ "title": "Вода", "icon": "drop", "days_mask": 31 })), say("Завёл")];
    let (status, result) = ask_and_result(&crew, &bot, "заведи привычку").await;
    assert_eq!(status, "ok", "{result}");
    let list = crew.ok("GET", "/api/habits", None).await;
    assert_eq!(list[0]["days_mask"], 31);
    let id = list[0]["id"].as_str().unwrap().to_string();

    *fake.script.lock().unwrap() = vec![tool("create_habit", json!({ "title": "Без значка" })), say("Не вышло")];
    let (status, _) = ask_and_result(&crew, &bot, "заведи привычку без значка").await;
    assert_ne!(status, "ok", "значок обязателен, как и у кнопки в окне");

    *fake.script.lock().unwrap() = vec![tool("update_habit", json!({ "habit": "Вода", "title": "Стакан воды", "remind_at": "12:00" })), say("Изменил")];
    let (status, result) = ask_and_result(&crew, &bot, "переименуй").await;
    assert_eq!(status, "ok", "{result}");
    let list = crew.ok("GET", "/api/habits", None).await;
    assert_eq!(list[0]["title"], "Стакан воды");

    *fake.script.lock().unwrap() = vec![tool("update_habit", json!({ "habit": "нет-такой", "title": "x" })), say("Не вышло")];
    let (status, _) = ask_and_result(&crew, &bot, "измени несуществующую").await;
    assert_ne!(status, "ok");

    *fake.script.lock().unwrap() = vec![tool("archive_habit", json!({ "habit": id })), say("Убрал")];
    crew.clear_events();
    crew.say(&bot, "убери привычку").await;
    let card = crew.wait_event("карточка", |e| e["type"] == "approval").await;
    assert_eq!(card["tool"], "archive_habit");
    crew.ok("POST", &format!("/api/approvals/{}", card["id"].as_str().unwrap()), Some(json!({ "allow": true }))).await;
    crew.wait_event("конец хода", |e| e["type"] == "turn_end").await;
    assert!(crew.ok("GET", "/api/habits", None).await.as_array().unwrap().is_empty());
}
