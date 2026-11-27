//! Лимит — пауза до сброса, а не провал: срок сброса, общий «отдых», продолжение, `await_event`.

use crate::common;

use std::sync::{Arc, Mutex};
use std::time::Duration;

use axum::body::Bytes;
use axum::extract::State;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::post;
use axum::{Json, Router};
use common::Crew;
use serde_json::{json, Value};

fn now_ms() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_millis() as i64
}

#[derive(Clone)]
enum Step {
    Say(String),
    Tool(&'static str, Value),
    Fail(u16, String),
}

#[derive(Clone, Default)]
struct Fake {
    script: Arc<Mutex<Vec<Step>>>,
    requests: Arc<Mutex<Vec<Value>>>,
}

impl Fake {
    fn calls(&self) -> usize {
        self.requests.lock().unwrap().len()
    }
}

async fn completions(State(fake): State<Fake>, body: Bytes) -> Response {
    fake.requests.lock().unwrap().push(serde_json::from_slice(&body).unwrap_or_else(|_| json!({})));
    let step = {
        let mut script = fake.script.lock().unwrap();
        if script.is_empty() {
            Step::Say("готово".into())
        } else {
            script.remove(0)
        }
    };
    let message = match step {
        Step::Fail(status, text) => return (StatusCode::from_u16(status).unwrap(), text).into_response(),
        Step::Say(text) => json!({ "role": "assistant", "content": text }),
        Step::Tool(name, args) => json!({
            "role": "assistant", "content": "",
            "tool_calls": [{ "id": "call_1", "type": "function", "function": { "name": name, "arguments": args.to_string() } }],
        }),
    };
    Json(json!({ "choices": [{ "message": message }] })).into_response()
}

async fn fake_server() -> (Fake, u16) {
    let fake = Fake::default();
    let router = Router::new().route("/v1/chat/completions", post(completions)).with_state(fake.clone());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
    (fake, port)
}

async fn configure(crew: &Crew, port: u16) -> String {
    crew.ok(
        "PUT",
        "/api/settings",
        Some(json!({ "driver": "api", "model": "test-model", "baseUrl": format!("http://127.0.0.1:{port}/v1"), "apiKey": "test-key" })),
    )
    .await;
    crew.listen().await;
    let state = crew.ok("GET", "/api/state", None).await;
    state["bots"][0]["id"].as_str().unwrap().to_string()
}

async fn setup(name: &str, env: &[(&str, &str)]) -> (Crew, Fake, String) {
    let (fake, port) = fake_server().await;
    let crew = Crew::start_with_env(name, env).await;
    let bot = configure(&crew, port).await;
    (crew, fake, bot)
}

fn runs_file(crew: &Crew) -> Vec<Value> {
    serde_json::from_str(&std::fs::read_to_string(crew.data_dir.join("runs.json")).unwrap_or_else(|_| "[]".into())).unwrap()
}

async fn thread(crew: &Crew, bot: &str) -> Vec<Value> {
    crew.ok("GET", &format!("/api/bots/{bot}/thread?limit=100"), None).await["messages"].as_array().cloned().unwrap_or_default()
}

fn said(messages: &[Value], text: &str) -> bool {
    messages.iter().any(|m| m["text"].as_str().is_some_and(|t| t.contains(text)))
}

async fn wait_thread(crew: &Crew, bot: &str, text: &str) {
    common::wait_until(text, 30, || async { said(&thread(crew, bot).await, text) }).await;
}

async fn wait_state(crew: &Crew, kind: &str) -> Value {
    common::wait_until(kind, 30, || async { runs_file(crew).first().is_some_and(|r| r["state"]["kind"] == kind) }).await;
    runs_file(crew)[0].clone()
}

async fn add_routine(crew: &Crew, bot: &str, title: &str, extra: Value) -> String {
    let mut body = json!({ "botId": bot, "title": title, "prompt": "сделай работу", "everyMinutes": 600 });
    body.as_object_mut().unwrap().extend(extra.as_object().unwrap().clone());
    crew.ok("POST", "/api/routines", Some(body)).await["routine"]["id"].as_str().unwrap().to_string()
}

async fn run_unattended(crew: &Crew, id: &str) -> Value {
    crew.clear_events();
    crew.ok("POST", &format!("/api/routines/{id}/run"), Some(json!({ "attended": false }))).await;
    crew.wait_event("журнал", |e| e["type"] == "journal").await["entry"].clone()
}

fn local_ms(day_offset: u64, hour: u32, minute: u32) -> i64 {
    use chrono::{Days, Local, NaiveTime, TimeZone};
    let date = Local::now().date_naive().checked_add_days(Days::new(day_offset)).unwrap();
    Local.from_local_datetime(&date.and_time(NaiveTime::from_hms_opt(hour, minute, 0).unwrap())).earliest().unwrap().timestamp_millis()
}

#[tokio::test(flavor = "multi_thread")]
async fn a_limit_pauses_the_run_then_one_resume_finishes_it() {
    let sink = common::temp_data_dir("limits-sink").join("toasts.jsonl");
    std::fs::create_dir_all(sink.parent().unwrap()).unwrap();
    let (crew, fake, bot) =
        setup("limits-resume", &[("CREW_TEST_TICK_MS", "200"), ("CREW_TEST_PAUSE_BASE_MS", "3500"), ("CREW_TEST_REMINDER_SINK", sink.to_str().unwrap())]).await;
    *fake.script.lock().unwrap() = vec![Step::Fail(429, "usage limit reached".into())];
    crew.say(&bot, "сделай работу").await;
    let end = crew.wait_event("конец хода", |e| e["type"] == "turn_end").await;
    assert_eq!(end["failed"], false, "лимит не провал: {end}");

    let run = runs_file(&crew)[0].clone();
    assert_eq!(run["state"]["kind"], "paused", "{run}");
    assert_eq!(run["state"]["reason"], "limit", "{run}");
    assert!(run["state"]["until_ms"].as_i64().unwrap() > now_ms(), "{run}");
    assert!(said(&thread(&crew, &bot).await, "продолжу в"), "в чате нет строки о паузе");
    let needs = crew.ok("GET", "/api/needs", None).await;
    let item = needs["items"].as_array().unwrap().iter().find(|i| i["paused"] == true).unwrap_or_else(|| panic!("пауза не видна в «Нужен ты»: {needs}")).clone();
    assert_eq!((item["reason"].as_str(), item["until"].as_i64()), (Some("limit"), run["state"]["until_ms"].as_i64()), "{item}");

    // Срок вышел: тик продолжает ход один раз, первым сообщением идёт «Лимит сбросился».
    wait_thread(&crew, &bot, "готово").await;
    let done = wait_state(&crew, "done").await;
    assert_eq!(done["pauses"], 1, "{done}");
    assert_eq!(done["attempts"], 0, "пауза — не обрыв, попытка не растёт: {done}");
    assert!(said(&thread(&crew, &bot).await, "Лимит сбросился — продолжай задание с того места, где остановился."));
    tokio::time::sleep(Duration::from_millis(800)).await;
    assert_eq!(fake.calls(), 2, "один сбой и одно продолжение");
    assert_eq!(runs_file(&crew).len(), 1);

    // Одно уведомление на окно сброса, а не по тику.
    let toasts = std::fs::read_to_string(&sink).unwrap_or_default();
    assert_eq!(toasts.lines().count(), 1, "{toasts}");
    assert!(toasts.contains("ждут ботов: 1"), "{toasts}");
}

#[tokio::test(flavor = "multi_thread")]
async fn the_reset_named_in_the_error_is_the_pause_deadline() {
    let (crew, fake, bot) = setup("limits-reset-text", &[]).await;
    *fake.script.lock().unwrap() = vec![Step::Fail(429, "usage limit reached, resets 11:59pm".into())];
    crew.say(&bot, "сделай").await;
    crew.wait_event("конец хода", |e| e["type"] == "turn_end").await;
    let run = wait_state(&crew, "paused").await;
    let today = local_ms(0, 23, 59);
    let expected = if today > now_ms() { today } else { local_ms(1, 23, 59) };
    assert_eq!(run["state"]["until_ms"].as_i64(), Some(expected), "{run}");
    assert_eq!(fake.calls(), 1, "до срока модель не зовут");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_limit_hit_by_one_bot_makes_another_unattended_run_wait_without_a_model_call() {
    let (crew, fake, bot) = setup("limits-rest", &[]).await;
    let second = crew.ok("POST", "/api/bots", Some(json!({ "name": "Второй", "role": "тест" }))).await["bot"]["id"].as_str().unwrap().to_string();
    *fake.script.lock().unwrap() = vec![Step::Fail(429, "usage limit reached".into())];
    crew.say(&bot, "сделай").await;
    crew.wait_event("конец хода", |e| e["type"] == "turn_end").await;
    let rest: Value = serde_json::from_str(&std::fs::read_to_string(crew.data_dir.join("limits-rest.json")).unwrap()).unwrap();
    let until = rest.as_object().unwrap().values().next().unwrap().as_i64().unwrap();
    assert!(until > now_ms(), "{rest}");

    // Второй бот на той же модели: ход без человека ждёт, модель не вызывается.
    let id = add_routine(&crew, &second, "Ночная", json!({})).await;
    let entry = run_unattended(&crew, &id).await;
    assert_eq!(entry["status"], "paused", "{entry}");
    assert_eq!(fake.calls(), 1, "модель не вызвана: {entry}");
    let paused = runs_file(&crew).into_iter().find(|r| r["bot_id"] == second.as_str()).unwrap();
    assert_eq!(paused["state"]["until_ms"].as_i64(), Some(until), "срок общий: {paused}");

    // Человек с тем же ботом пишет сам: пробуем, решает он.
    crew.clear_events();
    *fake.script.lock().unwrap() = vec![Step::Say("человеку отвечено".into())];
    crew.say(&second, "привет").await;
    let end = crew.wait_event("ответ человеку", |e| e["type"] == "turn_end" && e["botId"] == second.as_str()).await;
    assert_eq!(end["failed"], false, "{end}");
    assert_eq!(fake.calls(), 2);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_nearly_empty_subscription_window_blocks_unattended_work_but_not_chat() {
    let dir = common::temp_data_dir("limits-window");
    std::fs::create_dir_all(&dir).unwrap();
    let reset = now_ms() + 3 * 3_600_000;
    std::fs::write(dir.join("providers.json"), json!({ "codex-acp": { "limits": [{ "left": 3, "resetsAt": reset }] } }).to_string()).unwrap();
    let crew = Crew::start_at(dir, &[]).await;
    crew.ok("PUT", "/api/settings", Some(json!({ "driver": "acp:codex-acp", "model": "" }))).await;
    crew.listen().await;
    let bot = crew.ok("GET", "/api/state", None).await["bots"][0]["id"].as_str().unwrap().to_string();

    let id = add_routine(&crew, &bot, "Ночная", json!({})).await;
    let entry = run_unattended(&crew, &id).await;
    assert_eq!(entry["status"], "paused", "{entry}");
    let run = runs_file(&crew)[0].clone();
    assert_eq!((run["state"]["reason"].as_str(), run["state"]["until_ms"].as_i64()), (Some("limit"), Some(reset)), "{run}");

    // Живой разговор правилом не блокируется: агента здесь нет, и ход падает на нём, а не ждёт сброса.
    crew.clear_events();
    crew.say(&bot, "привет").await;
    crew.wait_event("живой ход", |e| e["type"] == "turn_end").await;
    let messages = thread(&crew, &bot).await;
    let last = messages.last().unwrap()["text"].as_str().unwrap();
    assert!(last.starts_with("Не смог ответить"), "человека правило не останавливает: {last}");
}

#[tokio::test(flavor = "multi_thread")]
async fn the_monthly_bot_limit_pauses_unattended_work_until_next_month() {
    let (crew, fake, bot) = setup("limits-month", &[]).await;
    // Расход уже есть: один обычный ход, потом предел бота ниже него.
    crew.say(&bot, "привет").await;
    crew.wait_event("первый ход", |e| e["type"] == "turn_end").await;
    crew.ok("PATCH", &format!("/api/bots/{bot}"), Some(json!({ "monthlyTokenLimit": 1 }))).await;
    let id = add_routine(&crew, &bot, "Ночная", json!({})).await;
    let before = fake.calls();
    let entry = run_unattended(&crew, &id).await;
    assert_eq!(entry["status"], "paused", "{entry}");
    assert_eq!(fake.calls(), before, "модель не вызвана");
    let run = runs_file(&crew).into_iter().find(|r| r["origin"]["process"].is_object()).unwrap();
    assert_eq!(run["state"]["reason"], "budget", "{run}");
    use chrono::{Datelike, Local};
    let now = Local::now();
    let (year, month) = if now.month() == 12 { (now.year() + 1, 1) } else { (now.year(), now.month() + 1) };
    let first = chrono::NaiveDate::from_ymd_opt(year, month, 1).unwrap().and_time(chrono::NaiveTime::MIN);
    let expected = chrono::TimeZone::from_local_datetime(&Local, &first).earliest().unwrap().timestamp_millis();
    assert_eq!(run["state"]["until_ms"].as_i64(), Some(expected), "{run}");
}

#[tokio::test(flavor = "multi_thread")]
async fn await_event_for_minutes_pauses_the_run_and_the_button_continues_it() {
    let (crew, fake, bot) = setup("limits-await", &[]).await;
    *fake.script.lock().unwrap() = vec![Step::Tool("await_event", json!({ "in_minutes": 30, "note": "жду ответа клиента" }))];
    crew.say(&bot, "проверь ответ").await;
    let end = crew.wait_event("конец хода", |e| e["type"] == "turn_end").await;
    assert_eq!(end["failed"], false, "{end}");
    let run = wait_state(&crew, "paused").await;
    assert_eq!(run["state"]["reason"], "event", "{run}");
    let until = run["state"]["until_ms"].as_i64().unwrap();
    assert!((until - now_ms() - 30 * 60_000).abs() < 60_000, "{run}");
    assert_eq!(run["state"]["note"], "жду ответа клиента");
    assert!(said(&thread(&crew, &bot).await, "Жду: жду ответа клиента"), "в чате нет строки ожидания");
    assert_eq!(fake.calls(), 1, "после await_event модель больше не зовут");

    // Срок далеко: срабатывает кнопка «Продолжить сейчас».
    crew.ok("POST", &format!("/api/runs/{}/resume", run["id"].as_str().unwrap()), None).await;
    wait_thread(&crew, &bot, "готово").await;
    wait_state(&crew, "done").await;
    assert!(said(&thread(&crew, &bot).await, "Срок ожидания вышел"));
    let again = crew.client.post(format!("{}/api/runs/{}/resume", crew.base, run["id"].as_str().unwrap())).header("cookie", &crew.cookie).send().await.unwrap();
    assert_eq!(again.status().as_u16(), 409, "вторая запись об одной паузе не продолжается");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_webhook_wakes_the_waiting_run_with_its_data() {
    let (crew, fake, bot) = setup("limits-webhook", &[]).await;
    let (id, token) = crew.webhook_process(&bot, "Ответ клиента", "Разбери ответ", &[]).await;
    *fake.script.lock().unwrap() = vec![Step::Tool("await_event", json!({ "webhook": id, "note": "ответ клиента" }))];
    crew.say(&bot, "жди ответа").await;
    let run = wait_state(&crew, "paused").await;
    assert_eq!(run["state"]["webhook"], id.as_str(), "{run}");

    let res = crew.client.post(format!("{}/hooks/{id}/{token}", crew.base)).header("content-type", "application/json").body(json!({ "answer": "согласен" }).to_string()).send().await.unwrap();
    assert_eq!(res.status().as_u16(), 202);
    wait_thread(&crew, &bot, "Сработал вебхук, которого ты ждал").await;
    wait_state(&crew, "done").await;
    let messages = thread(&crew, &bot).await;
    assert!(said(&messages, "согласен"), "данные вебхука дошли до хода");
    assert_eq!(runs_file(&crew).len(), 1, "нового запуска по процессу нет: ждавший продолжился");
    assert_eq!(fake.calls(), 2);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_paused_run_survives_a_restart_and_the_fifth_limit_in_a_row_calls_the_human() {
    let (fake, port) = fake_server().await;
    let mut first = Crew::start_with_env("limits-restart", &[]).await;
    let bot = configure(&first, port).await;
    *fake.script.lock().unwrap() = vec![Step::Fail(429, "usage limit reached".into())];
    first.say(&bot, "сделай").await;
    first.wait_event("конец хода", |e| e["type"] == "turn_end").await;
    wait_state(&first, "paused").await;
    let dir = first.data_dir.clone();
    first.keep_data();
    drop(first);

    // После перезапуска запись на месте и не продолжена досрочно: до срока модель молчит.
    let second = Crew::start_at(dir.clone(), &[]).await;
    second.listen().await;
    tokio::time::sleep(Duration::from_millis(500)).await;
    let runs = runs_file(&second);
    assert_eq!((runs.len(), runs[0]["state"]["kind"].as_str()), (1, Some("paused")), "{runs:?}");
    assert_eq!(fake.calls(), 1, "модель не звали");
    let mut second = second;
    second.keep_data();
    drop(second);

    // Пять пауз подряд уже было, срок вышел: ход упирается в лимит снова и зовёт человека.
    let mut runs = runs_file_at(&dir);
    runs[0]["pauses"] = json!(5);
    runs[0]["state"]["until_ms"] = json!(1);
    std::fs::write(dir.join("runs.json"), Value::Array(runs).to_string()).unwrap();
    let _ = std::fs::remove_file(dir.join("limits-rest.json"));
    *fake.script.lock().unwrap() = vec![Step::Fail(429, "usage limit reached".into())];
    let third = Crew::start_at(dir.clone(), &[("CREW_TEST_TICK_MS", "200")]).await;
    third.listen().await;
    let run = wait_state(&third, "needs_human").await;
    assert_eq!(run["state"]["reason"], "limit", "{run}");
    assert!(run["state"]["text"].as_str().unwrap().contains("не отпускает"), "{run}");
    let needs = third.ok("GET", "/api/needs", None).await;
    assert!(needs["items"].as_array().unwrap().iter().any(|i| i["kind"] == "run"), "в «Нужен ты» нет записи: {needs}");
}

fn runs_file_at(dir: &std::path::Path) -> Vec<Value> {
    serde_json::from_str(&std::fs::read_to_string(dir.join("runs.json")).unwrap()).unwrap()
}
