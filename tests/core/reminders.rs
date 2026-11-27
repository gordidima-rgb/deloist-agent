//! Напоминания: срабатывают таймером без модели, опережение сдвигает срок, разовое выключается само,
//! пропуск больше чем на 12 часов не шлётся, Telegram шлёт в привязанный чат или честно говорит, что не подключён.
//! Уведомление Windows в тестах пишется в файл (`CREW_TEST_REMINDER_SINK`), а не на экран.

use crate::common::{wait_until, Crew};

use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use axum::body::Bytes;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::post;
use axum::{Json, Router};
use serde_json::{json, Value};

const TOKEN: &str = "123456:TEST-token";

/// Модель, которая считает запросы: напоминание не должно её трогать.
#[derive(Clone, Default)]
struct Model {
    calls: Arc<Mutex<usize>>,
}

async fn completions(State(model): State<Model>, _body: Bytes) -> Json<Value> {
    *model.calls.lock().unwrap() += 1;
    Json(json!({ "choices": [{ "message": { "role": "assistant", "content": "готово" } }] }))
}

async fn start_model() -> (u16, Model) {
    let model = Model::default();
    let router = Router::new().route("/v1/chat/completions", post(completions)).with_state(model.clone());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
    (port, model)
}

/// Маленький Telegram: помнит отправленные сообщения и отдаёт обновления для привязки чата.
#[derive(Clone, Default)]
struct FakeTg {
    updates: Arc<Mutex<Vec<Value>>>,
    sent: Arc<Mutex<Vec<Value>>>,
}

async fn telegram(State(fake): State<FakeTg>, Path((token, method)): Path<(String, String)>, body: Bytes) -> Response {
    if token != format!("bot{TOKEN}") {
        return (StatusCode::UNAUTHORIZED, Json(json!({ "ok": false }))).into_response();
    }
    match method.as_str() {
        "getMe" => Json(json!({ "ok": true, "result": { "id": 1, "is_bot": true, "first_name": "Deloist", "username": "deloist_test_bot" } })).into_response(),
        "getUpdates" => {
            let request: Value = serde_json::from_slice(&body).unwrap_or_else(|_| json!({}));
            let offset = request["offset"].as_i64().unwrap_or(0);
            let updates: Vec<Value> = fake.updates.lock().unwrap().iter().filter(|u| u["update_id"].as_i64().unwrap_or(0) >= offset).cloned().collect();
            Json(json!({ "ok": true, "result": updates })).into_response()
        }
        "sendMessage" => {
            let request: Value = serde_json::from_slice(&body).unwrap_or_else(|_| json!({}));
            fake.sent.lock().unwrap().push(request);
            Json(json!({ "ok": true, "result": { "message_id": 1 } })).into_response()
        }
        _ => (StatusCode::NOT_FOUND, Json(json!({ "ok": false }))).into_response(),
    }
}

async fn fake_telegram() -> (String, FakeTg) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let fake = FakeTg::default();
    let router = Router::new().route("/{token}/{method}", post(telegram)).with_state(fake.clone());
    tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
    (format!("http://127.0.0.1:{port}"), fake)
}

fn now_ms() -> i64 {
    chrono::Utc::now().timestamp_millis()
}

/// Запуск программы: модель-счётчик, файл-заглушка Windows и (по желанию) фальшивый Telegram.
async fn setup(name: &str, telegram_api: Option<&str>) -> (Crew, Model, PathBuf) {
    let (port, model) = start_model().await;
    let sink = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(format!("{name}-sink.jsonl"));
    let _ = std::fs::remove_file(&sink);
    let sink_text = sink.display().to_string();
    let mut env: Vec<(&str, &str)> = vec![("CREW_TEST_TICK_MS", "100"), ("CREW_TEST_REMINDER_SINK", &sink_text)];
    if let Some(api) = telegram_api {
        env.push(("CREW_TEST_TELEGRAM_API", api));
    }
    let crew = Crew::start_with_env(name, &env).await;
    crew.ok("PUT", "/api/settings", Some(json!({ "driver": "api", "baseUrl": format!("http://127.0.0.1:{port}/v1"), "apiKey": "k" }))).await;
    crew.listen().await;
    (crew, model, sink)
}

fn sink_lines(sink: &PathBuf) -> Vec<Value> {
    std::fs::read_to_string(sink).unwrap_or_default().lines().filter_map(|l| serde_json::from_str(l).ok()).collect()
}

async fn routine(crew: &Crew, id: &str) -> Value {
    let state = crew.ok("GET", "/api/state", None).await;
    state["reminders"].as_array().unwrap().iter().find(|r| r["id"] == id).cloned().unwrap_or(Value::Null)
}

async fn create_reminder(crew: &Crew, body: Value) -> String {
    let (status, data) = crew.send("POST", "/api/reminders", Some(body)).await;
    assert_eq!(status, 201, "{data}");
    data["reminder"]["id"].as_str().unwrap().to_string()
}

#[tokio::test(flavor = "multi_thread")]
async fn a_reminder_fires_on_time_and_never_calls_the_model() {
    let (crew, model, sink) = setup("rem-fire", None).await;
    let id = create_reminder(&crew, json!({ "kind": "reminder", "prompt": "позвонить маме", "onceAt": now_ms() + 1500 })).await;

    let event = crew.wait_event("напоминание в окне", |e| e["type"] == "reminder").await;
    assert_eq!(event["text"], "позвонить маме");
    wait_until("запись в уведомлениях", 20, || async { !sink_lines(&sink).is_empty() }).await;

    let lines = sink_lines(&sink);
    assert_eq!(lines[0]["channel"], "windows");
    assert_eq!(lines[0]["body"], "позвонить маме");
    assert_eq!(*model.calls.lock().unwrap(), 0, "напоминание вызвало модель");

    // Разовое выключается само и отмечено как отправленное.
    wait_until("статус отправки", 20, || async { routine(&crew, &id).await["lastStatus"] == "ok" }).await;
    let after = routine(&crew, &id).await;
    assert_eq!(after["enabled"], false, "разовое напоминание не выключилось: {after}");
    assert_eq!(after["lastDelivery"][0]["ok"], true);
    assert_eq!(lines.len(), 1, "разовое ушло дважды");
}

#[tokio::test(flavor = "multi_thread")]
async fn lead_time_moves_the_moment_back() {
    let (crew, model, sink) = setup("rem-lead", None).await;
    let once = now_ms() + 90 * 60_000;
    let id = create_reminder(&crew, json!({ "kind": "reminder", "prompt": "собрать вещи", "onceAt": once, "leadMinutes": 60 })).await;

    let next = routine(&crew, &id).await["nextRun"].as_i64().unwrap();
    let expected = once - 60 * 60_000;
    assert!((next - expected).abs() < 5_000, "срок {next}, ждали около {expected}");
    // До срока с опережением ничего не уходит и модель не просыпается.
    tokio::time::sleep(std::time::Duration::from_millis(400)).await;
    assert!(sink_lines(&sink).is_empty());
    assert_eq!(*model.calls.lock().unwrap(), 0);
    assert_eq!(routine(&crew, &id).await["enabled"], true);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_reminder_overdue_by_more_than_twelve_hours_is_marked_missed_not_sent() {
    let (crew, _model, sink) = setup("rem-missed", None).await;
    let id = create_reminder(&crew, json!({ "kind": "reminder", "prompt": "старое", "onceAt": now_ms() - 13 * 3_600_000 })).await;

    wait_until("пропуск отмечен", 20, || async { routine(&crew, &id).await["lastStatus"] == "missed" }).await;
    assert!(sink_lines(&sink).is_empty(), "просроченное напоминание всё же отправили");
    let after = routine(&crew, &id).await;
    assert_eq!(after["enabled"], false);
    assert!(after["lastDelivery"][0]["error"].as_str().unwrap().contains("пропущено"));
}

#[tokio::test(flavor = "multi_thread")]
async fn telegram_says_when_it_is_not_connected_and_posts_once_paired() {
    let (base, fake) = fake_telegram().await;
    let (crew, model, _sink) = setup("rem-telegram", Some(&base)).await;

    // Telegram ещё не подключён: напоминание честно записывает причину.
    let first = create_reminder(&crew, json!({ "kind": "reminder", "prompt": "первое", "onceAt": now_ms() + 1000, "channels": ["telegram"] })).await;
    wait_until("причина записана", 20, || async { routine(&crew, &first).await["lastStatus"] == "error" }).await;
    let detail = routine(&crew, &first).await["lastDelivery"][0]["error"].clone();
    assert_eq!(detail, "Telegram не подключён");

    // Подключаем бота и привязываем чат кодом, как в обычном мосте.
    let state = crew.ok("GET", "/api/state", None).await;
    let bot = state["bots"][0]["id"].as_str().unwrap().to_string();
    crew.ok("PUT", "/api/telegram", Some(json!({ "token": TOKEN, "botId": bot }))).await;
    let pair = crew.ok("POST", "/api/telegram/pair", None).await;
    fake.updates.lock().unwrap().push(json!({
        "update_id": 1,
        "message": { "message_id": 1, "date": 1, "chat": { "id": 555, "type": "private" }, "from": { "id": 555, "is_bot": false }, "text": format!("/start {}", pair["code"].as_str().unwrap()) },
    }));
    wait_until("чат привязан", 20, || async { crew.ok("GET", "/api/telegram", None).await["chatPaired"] == true }).await;

    let second = create_reminder(&crew, json!({ "kind": "reminder", "prompt": "позвонить врачу", "onceAt": now_ms() + 1000, "channels": ["telegram"] })).await;
    wait_until("сообщение в Telegram", 20, || async {
        fake.sent.lock().unwrap().iter().any(|m| m["text"].as_str().is_some_and(|t| t.contains("позвонить врачу")))
    })
    .await;
    assert_eq!(routine(&crew, &second).await["lastStatus"], "ok");
    assert_eq!(*model.calls.lock().unwrap(), 0, "напоминание в Telegram вызвало модель");
}

#[tokio::test(flavor = "multi_thread")]
async fn the_test_route_sends_now_and_leaves_the_schedule_alone() {
    let (crew, _model, sink) = setup("rem-test-route", None).await;
    let future = now_ms() + 3 * 3_600_000;
    let id = create_reminder(&crew, json!({ "kind": "reminder", "prompt": "проверка", "onceAt": future })).await;

    let data = crew.ok("POST", &format!("/api/reminders/{id}/test"), None).await;
    assert_eq!(data["ok"], true, "{data}");
    assert_eq!(sink_lines(&sink).len(), 1);
    let after = routine(&crew, &id).await;
    assert_eq!(after["enabled"], true);
    assert!((after["nextRun"].as_i64().unwrap() - future).abs() < 5_000, "проверка сдвинула срок");
}

#[tokio::test(flavor = "multi_thread")]
async fn reminder_crud_through_its_own_api_keeps_json_shape() {
    let (crew, _model, _sink) = setup("rem-crud", None).await;
    let (status, data) = crew.send("POST", "/api/reminders", Some(json!({ "kind": "reminder", "prompt": "позвонить", "at": "09:00", "channels": ["windows"] }))).await;
    assert_eq!(status, 201, "{data}");
    let id = data["reminder"]["id"].as_str().unwrap().to_string();
    assert_eq!(data["reminder"]["prompt"], "позвонить");
    assert_eq!(data["reminder"]["kind"], "reminder");
    assert!(data["reminder"]["nextRun"].is_number());
    let listed = crew.ok("GET", "/api/state", None).await;
    assert!(listed["reminders"].as_array().unwrap().iter().any(|r| r["id"] == id.as_str()), "список напоминаний в /api/state");
    assert!(listed["processes"].as_array().unwrap().iter().all(|r| r["id"] != id.as_str()), "напоминание — не процесс");

    let ics = crew.client.get(format!("{}/api/tasks.ics", crew.base)).header("cookie", &crew.cookie).send().await.unwrap().text().await.unwrap();
    assert!(ics.contains(&id), "ics не содержит напоминание");

    let (status, data) = crew.send("PATCH", &format!("/api/reminders/{id}"), Some(json!({ "prompt": "обновлено", "enabled": false }))).await;
    assert_eq!(status, 200, "{data}");
    assert_eq!(data["reminder"]["prompt"], "обновлено");
    assert_eq!(data["reminder"]["enabled"], false);

    let (status, data) = crew.send("DELETE", &format!("/api/reminders/{id}"), None).await;
    assert_eq!(status, 200, "{data}");
    assert_eq!(data["ok"], true);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_seeded_reminder_in_reminders_json_fires() {
    let data_dir = crate::common::temp_data_dir("rem-seeded");
    std::fs::create_dir_all(&data_dir).unwrap();
    let sink = std::path::PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("rem-seeded-sink.jsonl");
    let _ = std::fs::remove_file(&sink);
    let sink_text = sink.display().to_string();
    let now = now_ms();
    let rem = json!({
        "reminders": [{
            "id": "rt_seeded",
            "title": "seeded",
            "prompt": "seeded prompt",
            "everyMinutes": 60,
            "enabled": true,
            "quiet": true,
            "nextRun": now + 5000,
            "createdAt": now,
            "extra": { "kind": "reminder", "leadMinutes": 0, "channels": ["windows"] }
        }]
    });
    std::fs::write(data_dir.join("reminders.json"), serde_json::to_string_pretty(&rem).unwrap()).unwrap();

    let env = [("CREW_TEST_TICK_MS", "100"), ("CREW_TEST_REMINDER_SINK", sink_text.as_str())];
    let crew = Crew::start_at(data_dir, &env).await;
    crew.listen().await;
    let event = crew.wait_event("напоминание в окне", |e| e["type"] == "reminder").await;
    assert_eq!(event["text"], "seeded prompt");
    let _ = std::fs::remove_file(&sink);
}

#[tokio::test(flavor = "multi_thread")]
async fn ordinary_routines_still_start_a_turn() {
    let (crew, model, _sink) = setup("rem-ordinary", None).await;
    let state = crew.ok("GET", "/api/state", None).await;
    let bot = state["bots"][0]["id"].as_str().unwrap().to_string();
    let (status, data) = crew.send("POST", "/api/workflows", Some(json!({ "botId": bot, "title": "Сводка", "prompt": "сделай сводку", "everyMinutes": 60 }))).await;
    assert_eq!(status, 201, "{data}");
    let id = data["process"]["id"].as_str().unwrap().to_string();

    crew.ok("POST", &format!("/api/workflows/{id}/run"), Some(json!({}))).await;
    crew.wait_event("конец хода рутины", |e| e["type"] == "turn_end").await;
    assert!(*model.calls.lock().unwrap() >= 1, "обычная рутина не начала ход");
    assert!(routine(&crew, &id).await.is_null(), "обычная рутина не попала в напоминания");
}
