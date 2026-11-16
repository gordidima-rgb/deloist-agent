//! Запасная модель: лимит у основной, память о сбое на 15 минут, секрет запасной и переход
//! посреди хода с уже выполненными инструментами.

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

/// Что поддельная модель ответит на следующий запрос.
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
    usage: Arc<Mutex<Option<(i64, i64)>>>,
}

fn rand_id() -> u128 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()
}

async fn completions(State(fake): State<Fake>, body: Bytes) -> Response {
    let request: Value = serde_json::from_slice(&body).unwrap_or_else(|_| json!({}));
    fake.requests.lock().unwrap().push(request);
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
            "role": "assistant",
            "content": "",
            "tool_calls": [{
                "id": format!("c{}", rand_id()),
                "type": "function",
                "function": { "name": name, "arguments": args.to_string() },
            }],
        }),
    };
    let mut reply = json!({ "choices": [{ "message": message }] });
    if let Some((prompt, completion)) = *fake.usage.lock().unwrap() {
        reply["usage"] = json!({ "prompt_tokens": prompt, "completion_tokens": completion });
    }
    Json(reply).into_response()
}

async fn serve(fake: Fake) -> u16 {
    let router = Router::new().route("/v1/chat/completions", post(completions)).with_state(fake);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
    port
}

async fn setup(name: &str, env: &[(&str, &str)], with_fallback: bool) -> (Crew, Fake, Fake, String) {
    let main = Fake::default();
    let spare = Fake::default();
    let main_port = serve(main.clone()).await;
    let spare_port = serve(spare.clone()).await;

    let crew = Crew::start_with_env(name, env).await;
    let mut body = json!({
        "driver": "api",
        "model": "main-model",
        "baseUrl": format!("http://127.0.0.1:{main_port}/v1"),
        "apiKey": "main-key",
    });
    if with_fallback {
        body["fallbackDriver"] = json!("api");
        body["fallbackModel"] = json!("backup-model");
        body["fallbackBaseUrl"] = json!(format!("http://127.0.0.1:{spare_port}/v1"));
        body["fallbackApiKey"] = json!("backup-key");
    }
    crew.ok("PUT", "/api/settings", Some(body)).await;
    crew.listen().await;
    let state = crew.ok("GET", "/api/state", None).await;
    let bot = state["bots"][0]["id"].as_str().unwrap().to_string();
    (crew, main, spare, bot)
}

/// Запустить рутину и дождаться её записи в журнале.
async fn run_routine(crew: &Crew, bot: &str) -> Value {
    let created = crew
        .ok(
            "POST",
            "/api/routines",
            Some(json!({ "botId": bot, "title": "Проверка", "prompt": "сделай работу", "everyMinutes": 60 })),
        )
        .await;
    let id = created["routine"]["id"].as_str().unwrap().to_string();
    crew.ok("POST", &format!("/api/routines/{id}/run"), Some(json!({}))).await;
    let event = crew.wait_event("журнал", |e| e["type"] == "journal").await;
    event["entry"].clone()
}

#[tokio::test(flavor = "multi_thread")]
async fn a_limit_on_the_main_model_finishes_the_run_on_the_fallback() {
    let (crew, main, spare, bot) = setup("fallback-switch", &[], true).await;
    *main.script.lock().unwrap() = vec![Step::Fail(429, "usage limit reached".into())];
    *spare.script.lock().unwrap() = vec![Step::Say("сделал на запасной".into())];
    *spare.usage.lock().unwrap() = Some((300, 100));

    let entry = run_routine(&crew, &bot).await;
    assert_eq!(entry["status"], "ok", "{entry}");
    assert_eq!(entry["text"], "сделал на запасной");
    assert_eq!(entry["fallback"], "backup-model");
    assert_eq!(entry["tokens"], 400, "токены запасной попали в расход: {entry}");

    let switched = crew.find_event(|e| e["type"] == "model_switched").expect("нет события переключения");
    assert_eq!(switched["from"], "main-model");
    assert_eq!(switched["to"], "backup-model");
    assert!(switched["reason"].as_str().unwrap().chars().count() <= 200, "{switched}");

    assert_eq!(main.requests.lock().unwrap().len(), 1, "основная спрошена один раз");
    let requests = spare.requests.lock().unwrap();
    assert_eq!(requests.len(), 1, "запасная получила запрос");
    assert_eq!(requests[0]["model"], "backup-model");
}

#[tokio::test(flavor = "multi_thread")]
async fn the_main_key_never_goes_to_another_address() {
    let (crew, main, _, bot) = setup("fallback-no-key", &[], false).await;
    let other = Fake::default();
    let port = serve(other.clone()).await;
    crew.ok(
        "PUT",
        "/api/settings",
        Some(json!({ "fallbackDriver": "api", "fallbackModel": "backup-model", "fallbackBaseUrl": format!("http://127.0.0.1:{port}/v1") })),
    )
    .await;
    *main.script.lock().unwrap() = vec![Step::Fail(429, "usage limit reached".into())];

    let entry = run_routine(&crew, &bot).await;
    assert_eq!(entry["status"], "failed", "{entry}");
    assert!(other.requests.lock().unwrap().is_empty(), "чужой адрес без своего ключа не вызывается");
}

#[tokio::test(flavor = "multi_thread")]
async fn an_empty_fallback_address_reuses_the_main_provider_and_key() {
    let (crew, main, _, bot) = setup("fallback-same", &[], false).await;
    crew.ok("PUT", "/api/settings", Some(json!({ "fallbackDriver": "api", "fallbackModel": "cheap-model" }))).await;
    *main.script.lock().unwrap() = vec![Step::Fail(429, "usage limit reached".into()), Step::Say("сделал".into())];

    let entry = run_routine(&crew, &bot).await;
    assert_eq!(entry["status"], "ok", "{entry}");
    assert_eq!(entry["fallback"], "cheap-model");
    let requests = main.requests.lock().unwrap();
    assert_eq!(requests.len(), 2);
    assert_eq!(requests[1]["model"], "cheap-model");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_bad_request_does_not_switch_the_model() {
    let (crew, main, spare, bot) = setup("fallback-400", &[], true).await;
    *main.script.lock().unwrap() = vec![Step::Fail(400, "invalid request".into())];

    let entry = run_routine(&crew, &bot).await;
    assert_eq!(entry["status"], "failed", "{entry}");
    assert!(entry.get("fallback").is_none(), "переключения не было: {entry}");
    assert_eq!(main.requests.lock().unwrap().len(), 1);
    assert!(spare.requests.lock().unwrap().is_empty(), "запасная не вызывается");
    assert!(crew.find_event(|e| e["type"] == "model_switched").is_none());
}

#[tokio::test(flavor = "multi_thread")]
async fn without_a_fallback_the_turn_fails_as_before() {
    let (crew, main, spare, bot) = setup("fallback-none", &[], false).await;
    *main.script.lock().unwrap() = vec![Step::Fail(429, "usage limit".into())];

    let entry = run_routine(&crew, &bot).await;
    assert_eq!(entry["status"], "failed", "{entry}");
    assert!(entry.get("fallback").is_none(), "{entry}");
    assert_eq!(main.requests.lock().unwrap().len(), 1);
    assert!(spare.requests.lock().unwrap().is_empty());
    assert!(crew.find_event(|e| e["type"] == "model_switched").is_none());
}

/// Что значок на аватарке узнаёт о боте из общего состояния.
async fn preview(crew: &Crew, bot: &str) -> Value {
    let state = crew.ok("GET", "/api/state", None).await;
    state["bots"].as_array().unwrap().iter().find(|b| b["id"] == bot).unwrap()["preview"].clone()
}

#[tokio::test(flavor = "multi_thread")]
async fn a_failed_turn_shows_on_the_bot_until_the_next_good_one() {
    let (crew, main, _spare, bot) = setup("preview-error", &[], false).await;
    *main.script.lock().unwrap() = vec![Step::Fail(400, "плохой запрос".into()), Step::Say("теперь готово".into())];

    crew.say(&bot, "сделай").await;
    crew.wait_event("конец хода", |e| e["type"] == "turn_end").await;
    let failed = preview(&crew, &bot).await;
    assert_eq!(failed["error"], true, "{failed}");

    crew.clear_events();
    crew.say(&bot, "ещё раз").await;
    crew.wait_event("конец хода", |e| e["type"] == "turn_end").await;
    let fixed = preview(&crew, &bot).await;
    assert_eq!(fixed["error"], false, "{fixed}");
    assert_eq!(fixed["text"], "теперь готово", "{fixed}");
}

#[tokio::test(flavor = "multi_thread")]
async fn stopping_a_turn_is_not_an_error_on_the_bot() {
    let (crew, main, _spare, bot) = setup("preview-stop", &[], false).await;
    *main.script.lock().unwrap() = vec![Step::Tool("run_command", json!({ "command": "echo ок > C:\\Windows\\deloist-probe.txt" }))];

    crew.say(&bot, "сделай").await;
    crew.wait_event("карточка", |e| e["type"] == "approval").await;
    crew.ok("POST", &format!("/api/bots/{bot}/stop"), None).await;
    crew.wait_event("конец хода", |e| e["type"] == "turn_end").await;
    let stopped = preview(&crew, &bot).await;
    assert_eq!(stopped["error"], false, "человек сам нажал стоп: {stopped}");
}

#[tokio::test(flavor = "multi_thread")]
async fn when_the_fallback_fails_too_the_turn_stops_with_its_error() {
    let (crew, main, spare, bot) = setup("fallback-both", &[], true).await;
    *main.script.lock().unwrap() = vec![Step::Fail(429, "main is out of balance".into())];
    *spare.script.lock().unwrap() = vec![Step::Fail(429, "backup also out of balance".into())];

    let entry = run_routine(&crew, &bot).await;
    assert_eq!(entry["status"], "failed", "{entry}");
    assert!(entry["text"].as_str().unwrap().contains("backup also out of balance"), "{entry}");
    assert_eq!(main.requests.lock().unwrap().len(), 1, "по кругу не повторяем");
    assert_eq!(spare.requests.lock().unwrap().len(), 1, "запасная спрошена один раз");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_recent_limit_sends_new_turns_straight_to_the_fallback() {
    let (crew, main, spare, bot) =
        setup("fallback-cooldown", &[("CREW_TEST_FALLBACK_COOLDOWN_MS", "700")], true).await;
    *main.script.lock().unwrap() = vec![Step::Fail(429, "usage limit".into())];
    *spare.script.lock().unwrap() = vec![Step::Say("первый".into())];

    let entry = run_routine(&crew, &bot).await;
    assert_eq!(entry["status"], "ok", "{entry}");
    assert_eq!(main.requests.lock().unwrap().len(), 1);

    // Сразу второй ход: основную даже не спрашиваем.
    crew.clear_events();
    *spare.script.lock().unwrap() = vec![Step::Say("второй".into())];
    crew.say(&bot, "ещё раз").await;
    let end = crew.wait_event("второй ход", |e| e["type"] == "turn_end").await;
    assert_eq!(end["failed"], false, "{end}");
    assert_eq!(main.requests.lock().unwrap().len(), 1, "основная не вызывалась");
    assert_eq!(spare.requests.lock().unwrap().len(), 2);
    let switched = crew.find_event(|e| e["type"] == "model_switched").expect("нет переключения");
    assert_eq!(switched["reason"], "основная модель недавно упёрлась в лимит");

    // Срок памяти вышел: снова основная.
    tokio::time::sleep(Duration::from_millis(900)).await;
    *main.script.lock().unwrap() = vec![Step::Say("основная снова тут".into())];
    crew.clear_events();
    crew.say(&bot, "и ещё").await;
    let end = crew.wait_event("третий ход", |e| e["type"] == "turn_end").await;
    assert_eq!(end["failed"], false, "{end}");
    assert_eq!(end["text"], "основная снова тут");
    assert_eq!(main.requests.lock().unwrap().len(), 2);
    assert_eq!(spare.requests.lock().unwrap().len(), 2, "запасная больше не нужна");
}

#[tokio::test(flavor = "multi_thread")]
async fn the_fallback_key_is_never_echoed_back() {
    let crew = Crew::start("fallback-secret").await;
    let put = crew
        .ok(
            "PUT",
            "/api/settings",
            Some(json!({
                "fallbackDriver": "api",
                "fallbackModel": "backup-model",
                "fallbackBaseUrl": "http://127.0.0.1:9/v1",
                "fallbackApiKey": "super-secret-backup",
            })),
        )
        .await;
    let text = put.to_string();
    assert!(!text.contains("super-secret-backup"), "ключ в ответе на сохранение: {text}");
    assert_eq!(put["settings"]["hasFallbackKey"], true, "{put}");

    let state = crew.ok("GET", "/api/state", None).await;
    let text = state.to_string();
    assert!(!text.contains("super-secret-backup"), "ключ в состоянии: {text}");
    assert_eq!(state["settings"]["hasFallbackKey"], true);

    // Пустое поле в интерфейсе не затирает сохранённый ключ.
    crew.ok("PUT", "/api/settings", Some(json!({ "fallbackApiKey": "", "fallbackModel": "другая" }))).await;
    let state = crew.ok("GET", "/api/state", None).await;
    assert_eq!(state["settings"]["hasFallbackKey"], true);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_limit_mid_turn_keeps_the_tool_history_for_the_fallback() {
    let (crew, main, spare, bot) = setup("fallback-midturn", &[], true).await;
    *main.script.lock().unwrap() = vec![Step::Tool("list_files", json!({})), Step::Fail(429, "rate limit".into())];
    *spare.script.lock().unwrap() = vec![Step::Say("закончил на запасной".into())];

    let entry = run_routine(&crew, &bot).await;
    assert_eq!(entry["status"], "ok", "{entry}");
    assert_eq!(entry["text"], "закончил на запасной");
    assert_eq!(entry["fallback"], "backup-model");
    assert_eq!(main.requests.lock().unwrap().len(), 2, "два шага на основной");
    assert_eq!(spare.requests.lock().unwrap().len(), 1);

    let request = spare.requests.lock().unwrap()[0].clone();
    let messages = request["messages"].as_array().unwrap();
    assert!(
        messages.iter().any(|m| m["role"] == "tool" && !m["content"].as_str().unwrap_or("").is_empty()),
        "запасная не получила результат инструмента: {request}"
    );
    assert!(
        messages.iter().any(|m| m["role"] == "assistant" && m["tool_calls"].is_array()),
        "запасная не получила историю вызова: {request}"
    );
}
