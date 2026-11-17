//! Очередь сообщений: занятый бот принимает текст, очередь правится, стоп её ставит на паузу.

use crate::common;

use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use axum::body::Bytes;
use axum::extract::State;
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
    /// Ответ висит, пока тест не отпустит: так видно, что модель ещё не вызывали.
    Hold(Arc<tokio::sync::Notify>),
}

#[derive(Clone, Default)]
struct Fake {
    script: Arc<Mutex<Vec<Step>>>,
    requests: Arc<Mutex<Vec<Value>>>,
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
        Step::Hold(gate) => {
            gate.notified().await;
            json!({ "role": "assistant", "content": "готово" })
        }
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
        Some(json!({
            "driver": "api",
            "model": "test-model",
            "baseUrl": format!("http://127.0.0.1:{port}/v1"),
            "apiKey": "test-key",
        })),
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

fn thread_messages(state: &Value) -> Vec<Value> {
    state["messages"].as_array().cloned().unwrap_or_default()
}

async fn thread(crew: &Crew, bot: &str) -> Vec<Value> {
    let state = crew.ok("GET", &format!("/api/bots/{bot}/thread?limit=100"), None).await;
    thread_messages(&state)
}

async fn wait_queue(crew: &Crew, bot: &str, len: usize) -> Value {
    let started = Instant::now();
    loop {
        let queue = crew.ok("GET", &format!("/api/bots/{bot}/queue"), None).await;
        if queue["items"].as_array().is_some_and(|items| items.len() == len) {
            return queue;
        }
        if started.elapsed() > Duration::from_secs(20) {
            panic!("очередь не стала длины {len}: {queue}");
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

async fn wait_paused(crew: &Crew, bot: &str) -> Value {
    let started = Instant::now();
    loop {
        let queue = crew.ok("GET", &format!("/api/bots/{bot}/queue"), None).await;
        if queue["paused"] == true {
            return queue;
        }
        if started.elapsed() > Duration::from_secs(20) {
            panic!("очередь не встала на паузу: {queue}");
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

async fn wait_thread_text(crew: &Crew, bot: &str, text: &str) {
    let started = Instant::now();
    loop {
        let messages = thread(crew, bot).await;
        if messages.iter().any(|m| m["text"] == text) {
            return;
        }
        if started.elapsed() > Duration::from_secs(20) {
            panic!("нет «{text}» в треде: {messages:?}");
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

async fn wait_requests(fake: &Fake, count: usize) {
    let started = Instant::now();
    while started.elapsed() < Duration::from_secs(10) {
        if fake.requests.lock().unwrap().len() >= count {
            return;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    panic!("модель не получила запрос №{count}");
}

fn queue_texts(queue: &Value) -> Vec<String> {
    queue["items"]
        .as_array()
        .map(|items| items.iter().map(|item| item["text"].as_str().unwrap_or_default().to_string()).collect())
        .unwrap_or_default()
}

fn queue_ids(queue: &Value) -> Vec<String> {
    queue["items"]
        .as_array()
        .map(|items| items.iter().map(|item| item["id"].as_str().unwrap_or_default().to_string()).collect())
        .unwrap_or_default()
}

async fn queued(crew: &Crew, bot: &str, text: &str) -> String {
    let (status, body) = crew
        .send("POST", &format!("/api/bots/{bot}/messages"), Some(json!({ "text": text })))
        .await;
    assert_eq!(status, 202, "{body}");
    assert_eq!(body["queued"], true, "{body}");
    body["item"]["id"].as_str().unwrap().to_string()
}

#[tokio::test(flavor = "multi_thread")]
async fn a_message_for_a_busy_bot_is_queued_and_starts_after_the_turn() {
    let (crew, fake, bot) = setup("queue-busy", &[]).await;
    *fake.script.lock().unwrap() = vec![
        Step::Tool("run_command", json!({ "command": "echo один > C:\\Windows\\deloist-probe.txt" })),
        Step::Say("первый".into()),
        Step::Say("второй".into()),
    ];

    crew.clear_events();
    crew.say(&bot, "раз").await;
    let approval = crew.wait_event("карточка", |e| e["type"] == "approval").await;
    let id = approval["id"].as_str().unwrap().to_string();

    let (status, body) = crew
        .send("POST", &format!("/api/bots/{bot}/messages"), Some(json!({ "text": "два" })))
        .await;
    assert_eq!(status, 202, "{body}");
    assert_eq!(body["queued"], true, "{body}");
    assert_eq!(body["item"]["text"], "два");
    assert!(body["item"]["id"].as_str().unwrap().starts_with("q_"));

    let queue = wait_queue(&crew, &bot, 1).await;
    assert_eq!(queue["paused"], false, "{queue}");
    assert_eq!(queue["items"][0]["text"], "два");

    crew.ok("POST", &format!("/api/approvals/{id}"), Some(json!({ "allow": true }))).await;
    let first_end = crew
        .wait_event("конец первого хода", |e| e["type"] == "turn_end" && e["botId"] == bot.as_str())
        .await;
    assert_eq!(first_end["failed"], false, "{first_end}");
    wait_thread_text(&crew, &bot, "второй").await;

    let messages = thread(&crew, &bot).await;
    let at = |text: &str| messages.iter().position(|m| m["text"] == text);
    let first = at("первый").expect("нет ответа на первое сообщение");
    let second = at("второй").expect("нет ответа на второе сообщение");
    let question = messages
        .iter()
        .position(|m| m["role"] == "user" && m["text"] == "два")
        .expect("нет второго вопроса в треде");
    assert!(first < question && question < second, "порядок треда сломан: {messages:?}");
    assert_eq!(fake.requests.lock().unwrap().len(), 3, "два хода по одному шагу модели");
}

#[tokio::test(flavor = "multi_thread")]
async fn three_queued_messages_go_strictly_in_order() {
    let (crew, fake, bot) = setup("queue-order", &[]).await;
    *fake.script.lock().unwrap() = vec![
        Step::Tool("run_command", json!({ "command": "echo один > C:\\Windows\\deloist-probe.txt" })),
        Step::Say("ответ 1".into()),
        Step::Say("ответ 2".into()),
        Step::Say("ответ 3".into()),
        Step::Say("ответ 4".into()),
    ];

    crew.say(&bot, "старт").await;
    let approval = crew.wait_event("карточка", |e| e["type"] == "approval").await;
    let id = approval["id"].as_str().unwrap().to_string();

    for text in ["первое", "второе", "третье"] {
        let (status, body) = crew
            .send("POST", &format!("/api/bots/{bot}/messages"), Some(json!({ "text": text })))
            .await;
        assert_eq!(status, 202, "{body}");
        assert_eq!(body["queued"], true, "{body}");
    }
    wait_queue(&crew, &bot, 3).await;

    crew.ok("POST", &format!("/api/approvals/{id}"), Some(json!({ "allow": true }))).await;
    wait_thread_text(&crew, &bot, "ответ 4").await;

    let messages = thread(&crew, &bot).await;
    let positions = |texts: &[&str]| -> Vec<usize> {
        texts.iter().map(|text| messages.iter().position(|m| m["text"] == *text).unwrap_or_else(|| panic!("нет «{text}»: {messages:?}"))).collect()
    };
    let users = positions(&["старт", "первое", "второе", "третье"]);
    assert!(users.windows(2).all(|w| w[0] < w[1]), "вопросы не по порядку: {users:?}");
    let answers = positions(&["ответ 1", "ответ 2", "ответ 3", "ответ 4"]);
    assert!(answers.windows(2).all(|w| w[0] < w[1]), "ответы не по порядку: {answers:?}");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_queued_message_can_be_edited_and_deleted() {
    let (crew, fake, bot) = setup("queue-edit", &[]).await;
    *fake.script.lock().unwrap() = vec![
        Step::Tool("run_command", json!({ "command": "echo один > C:\\Windows\\deloist-probe.txt" })),
        Step::Say("первый".into()),
        Step::Say("новый".into()),
    ];

    crew.say(&bot, "старт").await;
    let approval = crew.wait_event("карточка", |e| e["type"] == "approval").await;
    let id = approval["id"].as_str().unwrap().to_string();

    let (_, first) = crew
        .send("POST", &format!("/api/bots/{bot}/messages"), Some(json!({ "text": "старый" })))
        .await;
    let (_, second) = crew
        .send("POST", &format!("/api/bots/{bot}/messages"), Some(json!({ "text": "удалить" })))
        .await;
    let first_id = first["item"]["id"].as_str().unwrap().to_string();
    let second_id = second["item"]["id"].as_str().unwrap().to_string();
    wait_queue(&crew, &bot, 2).await;

    let patched = crew
        .ok(
            "PATCH",
            &format!("/api/bots/{bot}/queue/{first_id}"),
            Some(json!({ "text": "исправлено" })),
        )
        .await;
    assert_eq!(patched["item"]["text"], "исправлено");
    let queue = crew.ok("GET", &format!("/api/bots/{bot}/queue"), None).await;
    assert_eq!(queue["items"][0]["text"], "исправлено", "{queue}");

    let (status, _) = crew
        .send(
            "PATCH",
            &format!("/api/bots/{bot}/queue/{first_id}"),
            Some(json!({ "text": "   " })),
        )
        .await;
    assert_eq!(status, 400, "пустой текст");
    let (status, _) = crew
        .send("PATCH", &format!("/api/bots/{bot}/queue/q_none"), Some(json!({ "text": "x" })))
        .await;
    assert_eq!(status, 404, "чужой id");
    let (status, _) = crew.send("DELETE", &format!("/api/bots/{bot}/queue/q_none"), None).await;
    assert_eq!(status, 404, "чужой id");

    crew.ok("DELETE", &format!("/api/bots/{bot}/queue/{second_id}"), None).await;
    let queue = wait_queue(&crew, &bot, 1).await;
    assert_eq!(queue["items"][0]["text"], "исправлено");

    crew.ok("POST", &format!("/api/approvals/{id}"), Some(json!({ "allow": true }))).await;
    wait_thread_text(&crew, &bot, "новый").await;

    {
        let requests = fake.requests.lock().unwrap();
        assert_eq!(requests.len(), 3);
        let sent = requests[2].to_string();
        assert!(sent.contains("исправлено"), "модель не увидела новый текст: {sent}");
        assert!(!sent.contains("удалить"), "модель увидела удалённое сообщение: {sent}");
    }

    let messages = thread(&crew, &bot).await;
    assert!(
        !messages.iter().any(|m| m["text"] == "удалить"),
        "удалённое сообщение попало в тред: {messages:?}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn the_queue_holds_at_most_fifty_messages() {
    let (crew, fake, bot) = setup("queue-limit", &[]).await;
    *fake.script.lock().unwrap() = vec![Step::Tool("run_command", json!({ "command": "echo один > C:\\Windows\\deloist-probe.txt" }))];
    crew.say(&bot, "старт").await;
    // Модель отвечает инструментом без второго шага: ход висит на карточке.
    let approval = crew.wait_event("карточка", |e| e["type"] == "approval").await;
    assert_eq!(approval["tool"], "run_command");

    for index in 0..50 {
        let (status, body) = crew
            .send(
                "POST",
                &format!("/api/bots/{bot}/messages"),
                Some(json!({ "text": format!("сообщение {index}") })),
            )
            .await;
        assert_eq!(status, 202, "{index}: {body}");
    }
    wait_queue(&crew, &bot, 50).await;

    let (status, body) = crew
        .send("POST", &format!("/api/bots/{bot}/messages"), Some(json!({ "text": "лишнее" })))
        .await;
    assert_eq!(status, 429, "{body}");
    assert_eq!(body["error"], "В очереди уже 50 сообщений, подожди немного");
}

#[tokio::test(flavor = "multi_thread")]
async fn stop_pauses_the_queue_and_resume_starts_it() {
    let (crew, fake, bot) = setup("queue-stop", &[]).await;
    *fake.script.lock().unwrap() = vec![
        Step::Tool("run_command", json!({ "command": "echo один > C:\\Windows\\deloist-probe.txt" })),
        Step::Say("после паузы".into()),
    ];

    crew.clear_events();
    crew.say(&bot, "старт").await;
    crew.wait_event("карточка", |e| e["type"] == "approval").await;
    crew.say(&bot, "в очереди").await;
    wait_queue(&crew, &bot, 1).await;

    crew.ok("POST", &format!("/api/bots/{bot}/stop"), None).await;
    crew.wait_event("конец хода", |e| e["type"] == "turn_end").await;
    let paused = wait_paused(&crew, &bot).await;
    assert_eq!(paused["items"].as_array().unwrap().len(), 1, "{paused}");
    assert_eq!(paused["pauseReason"], "manual", "{paused}");
    let event = crew.wait_event("причина паузы", |e| e["type"] == "queue_updated" && e["pauseReason"] == "manual").await;
    assert_eq!(event["paused"], true, "{event}");

    tokio::time::sleep(Duration::from_millis(300)).await;
    assert_eq!(fake.requests.lock().unwrap().len(), 1, "после стопа модель не звали");

    crew.ok("POST", &format!("/api/bots/{bot}/queue/resume"), None).await;
    wait_thread_text(&crew, &bot, "после паузы").await;
    let queue = wait_queue(&crew, &bot, 0).await;
    assert_eq!(queue["paused"], false, "{queue}");
    assert!(queue["pauseReason"].is_null(), "{queue}");
    assert_eq!(fake.requests.lock().unwrap().len(), 2);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_new_message_while_paused_runs_the_old_one_first() {
    let (crew, fake, bot) = setup("queue-resume", &[]).await;
    *fake.script.lock().unwrap() = vec![
        Step::Tool("run_command", json!({ "command": "echo один > C:\\Windows\\deloist-probe.txt" })),
        Step::Say("старое".into()),
        Step::Say("новое".into()),
    ];

    crew.say(&bot, "старт").await;
    crew.wait_event("карточка", |e| e["type"] == "approval").await;
    crew.say(&bot, "старое сообщение").await;
    wait_queue(&crew, &bot, 1).await;
    crew.ok("POST", &format!("/api/bots/{bot}/stop"), None).await;
    crew.wait_event("конец хода", |e| e["type"] == "turn_end").await;
    wait_paused(&crew, &bot).await;

    let (status, body) = crew
        .send("POST", &format!("/api/bots/{bot}/messages"), Some(json!({ "text": "новое сообщение" })))
        .await;
    assert_eq!(status, 202, "{body}");
    assert_eq!(body["queued"], true, "{body}");
    let queue = crew.ok("GET", &format!("/api/bots/{bot}/queue"), None).await;
    assert!(queue["pauseReason"].is_null(), "{queue}");

    wait_thread_text(&crew, &bot, "новое").await;
    let messages = thread(&crew, &bot).await;
    let at = |text: &str| messages.iter().position(|m| m["text"] == text).expect(text);
    assert!(at("старое сообщение") < at("новое сообщение"), "новое обогнало старое: {messages:?}");
    assert!(at("старое") < at("новое"), "ответы не по порядку: {messages:?}");
    assert_eq!(fake.requests.lock().unwrap().len(), 3);
}

#[tokio::test(flavor = "multi_thread")]
async fn steer_puts_the_text_into_the_current_turn() {
    let (crew, fake, bot) = setup("queue-steer", &[]).await;
    *fake.script.lock().unwrap() = vec![
        Step::Tool("run_command", json!({ "command": "echo один > C:\\Windows\\deloist-probe.txt" })),
        Step::Say("готово после карточки".into()),
    ];

    crew.clear_events();
    crew.say(&bot, "старт").await;
    let approval = crew.wait_event("карточка", |e| e["type"] == "approval").await;
    let approval_id = approval["id"].as_str().unwrap().to_string();

    let (_, queued) = crew
        .send("POST", &format!("/api/bots/{bot}/messages"), Some(json!({ "text": "вставить сейчас" })))
        .await;
    let qid = queued["item"]["id"].as_str().unwrap().to_string();
    wait_queue(&crew, &bot, 1).await;

    crew.ok("POST", &format!("/api/bots/{bot}/queue/{qid}/steer"), None).await;
    wait_queue(&crew, &bot, 0).await;
    crew.ok("POST", &format!("/api/approvals/{approval_id}"), Some(json!({ "allow": true }))).await;
    let end = crew.wait_event("конец хода", |e| e["type"] == "turn_end").await;
    assert_eq!(end["failed"], false, "{end}");
    wait_thread_text(&crew, &bot, "готово после карточки").await;

    let starts = crew.events.lock().unwrap().iter().filter(|e| e["type"] == "turn_start").count();
    assert_eq!(starts, 1, "«сейчас» не должно быть отдельным ходом");

    {
        let requests = fake.requests.lock().unwrap();
        assert_eq!(requests.len(), 2);
        let sent = requests[1]["messages"].to_string();
        assert!(sent.contains("вставить сейчас"), "модель не получила текст: {sent}");
    }

    let messages = thread(&crew, &bot).await;
    let inserted = messages
        .iter()
        .find(|m| m["text"] == "вставить сейчас")
        .expect("нет сообщения в треде");
    assert_eq!(inserted["role"], "user");
    assert_eq!(inserted["source"], "steer");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_steer_that_missed_the_model_starts_as_its_own_turn() {
    let (crew, fake, bot) = setup("queue-steer-late", &[]).await;
    let gate = Arc::new(tokio::sync::Notify::new());
    *fake.script.lock().unwrap() = vec![
        Step::Hold(gate.clone()),
        Step::Say("ответ на сейчас".into()),
    ];

    crew.clear_events();
    crew.say(&bot, "долгий ход").await;
    wait_requests(&fake, 1).await;

    let (_, queued) = crew
        .send("POST", &format!("/api/bots/{bot}/messages"), Some(json!({ "text": "поздно" })))
        .await;
    let qid = queued["item"]["id"].as_str().unwrap().to_string();
    wait_queue(&crew, &bot, 1).await;

    crew.ok("POST", &format!("/api/bots/{bot}/queue/{qid}/steer"), None).await;
    wait_queue(&crew, &bot, 0).await;
    gate.notify_one();

    wait_thread_text(&crew, &bot, "ответ на сейчас").await;
    let starts = crew.events.lock().unwrap().iter().filter(|e| e["type"] == "turn_start").count();
    assert_eq!(starts, 2, "недоехавшее «сейчас» должно стать отдельным ходом");

    {
        let requests = fake.requests.lock().unwrap();
        assert_eq!(requests.len(), 2);
        let sent = requests[1]["messages"].to_string();
        assert!(sent.contains("поздно"), "текст потерялся: {sent}");
    }

    let messages = thread(&crew, &bot).await;
    let inserted = messages
        .iter()
        .find(|m| m["text"] == "поздно")
        .expect("нет сообщения в треде");
    assert_eq!(inserted["role"], "user");
    assert_eq!(inserted["source"], "chat", "опоздавшее «сейчас» стало обычным ходом");
}

#[tokio::test(flavor = "multi_thread")]
async fn after_a_restart_the_queue_is_read_and_keeps_going() {
    let (fake, port) = fake_server().await;
    let dir = common::temp_data_dir("queue-restart");
    let mut first = Crew::start_at(dir.clone(), &[]).await;
    let bot = configure(&first, port).await;
    *fake.script.lock().unwrap() = vec![
        Step::Tool("run_command", json!({ "command": "echo один > C:\\Windows\\deloist-probe.txt" })),
        Step::Say("после перезапуска".into()),
    ];

    first.say(&bot, "старт").await;
    first.wait_event("карточка", |e| e["type"] == "approval").await;
    first.say(&bot, "в очереди").await;
    wait_queue(&first, &bot, 1).await;
    let raw = std::fs::read_to_string(dir.join("queue.json")).unwrap();
    assert!(raw.contains("в очереди"), "очередь не записана: {raw}");

    first.keep_data();
    drop(first);

    // Файл старого формата без поля причины остаётся читаемым.
    let path = dir.join("queue.json");
    let mut legacy: Value = serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
    legacy[&bot].as_object_mut().unwrap().remove("pause_reason");
    std::fs::write(&path, serde_json::to_vec(&legacy).unwrap()).unwrap();

    // Прерванный ход продолжается сам (ответ «после перезапуска»), а за ним идёт очередь: пауза после перезапуска не ставится.
    let second = Crew::start_at(dir.clone(), &[]).await;
    wait_thread_text(&second, &bot, "после перезапуска").await;
    wait_thread_text(&second, &bot, "в очереди").await;
    let queue = second.ok("GET", &format!("/api/bots/{bot}/queue"), None).await;
    assert_eq!(queue["paused"], false, "после перезапуска очередь не стоит: {queue}");
    assert_ne!(queue["pauseReason"], "restart", "{queue}");
}

#[tokio::test(flavor = "multi_thread")]
async fn manual_pause_survives_restart() {
    let (fake, port) = fake_server().await;
    let dir = common::temp_data_dir("queue-manual-restart");
    let mut first = Crew::start_at(dir.clone(), &[]).await;
    let bot = configure(&first, port).await;
    *fake.script.lock().unwrap() = vec![Step::Tool("run_command", json!({ "command": "echo один > C:\\Windows\\deloist-probe.txt" }))];
    first.say(&bot, "старт").await;
    first.wait_event("карточка", |e| e["type"] == "approval").await;
    first.say(&bot, "в очереди").await;
    wait_queue(&first, &bot, 1).await;
    first.ok("POST", &format!("/api/bots/{bot}/stop"), None).await;
    let paused = wait_paused(&first, &bot).await;
    assert_eq!(paused["pauseReason"], "manual");
    first.keep_data();
    drop(first);

    let second = Crew::start_at(dir, &[]).await;
    let queue = second.ok("GET", &format!("/api/bots/{bot}/queue"), None).await;
    assert_eq!(queue["pauseReason"], "manual", "{queue}");
    assert_eq!(queue["paused"], true, "{queue}");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_queued_chat_after_a_night_delegation_is_not_treated_as_unattended() {
    let (crew, fake, bot) = setup("queue-delegate", &[("CREW_TEST_TICK_MS", "100")]).await;
    let state = crew.ok("GET", "/api/state", None).await;
    let target = state["bots"][1]["id"].as_str().unwrap().to_string();
    let target_name = state["bots"][1]["name"].as_str().unwrap().to_string();
    crew.ok("POST", "/api/rooms", Some(json!({ "name": "Пара", "botIds": [bot, target], "lead": bot }))).await;
    let gate = Arc::new(tokio::sync::Notify::new());
    *fake.script.lock().unwrap() = vec![
        Step::Tool("delegate", json!({ "bot": target_name, "task": "сделай часть" })),
        Step::Hold(gate.clone()),
    ];
    let created = crew
        .ok("POST", "/api/routines", Some(json!({ "botId": bot, "title": "Ночь", "prompt": "работай", "everyMinutes": 60 })))
        .await;
    let routine = created["routine"]["id"].as_str().unwrap().to_string();
    crew.ok("POST", &format!("/api/routines/{routine}/run"), Some(json!({ "attended": false }))).await;

    // Делегат думает, человек пишет ему: сообщение в очереди.
    common::wait_until("делегат занят", 20, || {
        let fake = fake.clone();
        async move { fake.requests.lock().unwrap().len() >= 2 }
    })
    .await;
    crew.say(&target, "из очереди").await;
    wait_queue(&crew, &target, 1).await;
    gate.notify_one();
    wait_thread_text(&crew, &target, "из очереди").await;
    wait_queue(&crew, &target, 0).await;

    common::wait_until("запрос из очереди", 20, || {
        let fake = fake.clone();
        async move {
            fake.requests.lock().unwrap().iter().any(|r| r["messages"].as_array().unwrap().last().unwrap()["content"] == "из очереди")
        }
    })
    .await;
    let requests = fake.requests.lock().unwrap();
    let request = requests.iter().find(|r| r["messages"].as_array().unwrap().last().unwrap()["content"] == "из очереди").unwrap();
    let system = request["messages"][0]["content"].as_str().unwrap();
    assert!(!system.contains("человека рядом может не быть"), "разговор с человеком не должен идти как ночная рутина");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_paused_queue_does_not_hold_back_a_routine() {
    let (crew, fake, bot) = setup("queue-routine", &[("CREW_TEST_TICK_MS", "100")]).await;
    *fake.script.lock().unwrap() = vec![
        Step::Tool("run_command", json!({ "command": "echo один > C:\\Windows\\deloist-probe.txt" })),
        Step::Say("не сейчас".into()),
    ];

    crew.clear_events();
    crew.say(&bot, "старт").await;
    crew.wait_event("карточка", |e| e["type"] == "approval").await;
    crew.say(&bot, "в очереди").await;
    wait_queue(&crew, &bot, 1).await;
    // Человек нажал «стоп» и ушёл: очередь ждёт его, а ночная работа идёт.
    crew.ok("POST", &format!("/api/bots/{bot}/stop"), None).await;
    crew.wait_event("конец хода", |e| e["type"] == "turn_end").await;
    wait_paused(&crew, &bot).await;

    let created = crew
        .ok(
            "POST",
            "/api/routines",
            Some(json!({ "botId": bot, "title": "Проверка", "prompt": "сделай работу", "everyMinutes": 60 })),
        )
        .await;
    let routine = created["routine"]["id"].as_str().unwrap().to_string();
    crew.ok("PATCH", &format!("/api/routines/{routine}"), Some(json!({ "nextRun": common::due_now() }))).await;

    let event = crew.wait_event("запуск рутины", |e| e["type"] == "journal").await;
    assert_eq!(event["entry"]["title"], "Проверка", "{event}");

    // Сообщение человека не потеряно и не уехало само.
    let queue = crew.ok("GET", &format!("/api/bots/{bot}/queue"), None).await;
    assert_eq!(queue["paused"], true, "{queue}");
    assert_eq!(queue["items"].as_array().unwrap().len(), 1, "{queue}");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_broken_queue_file_does_not_stop_the_app() {
    let dir = common::temp_data_dir("queue-broken");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("queue.json"), "{ это не json").unwrap();

    let (fake, port) = fake_server().await;
    let crew = Crew::start_at(dir, &[]).await;
    let bot = configure(&crew, port).await;
    *fake.script.lock().unwrap() = vec![Step::Say("работает".into())];

    crew.say(&bot, "привет").await;
    let end = crew.wait_event("конец хода", |e| e["type"] == "turn_end").await;
    assert_eq!(end["failed"], false, "{end}");
    assert_eq!(end["text"], "работает");

    let queue = crew.ok("GET", &format!("/api/bots/{bot}/queue"), None).await;
    assert_eq!(queue["items"].as_array().unwrap().len(), 0, "{queue}");
    assert_eq!(queue["paused"], false, "{queue}");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_moved_message_reaches_the_model_in_the_new_order() {
    let (crew, fake, bot) = setup("queue-move-order", &[]).await;
    *fake.script.lock().unwrap() = vec![
        Step::Tool("run_command", json!({ "command": "echo один > C:\\Windows\\deloist-probe.txt" })),
        Step::Say("ответ 1".into()),
        Step::Say("ответ 2".into()),
        Step::Say("ответ 3".into()),
        Step::Say("ответ 4".into()),
    ];

    crew.say(&bot, "старт").await;
    let approval = crew.wait_event("карточка", |e| e["type"] == "approval").await;
    let id = approval["id"].as_str().unwrap().to_string();

    let a = queued(&crew, &bot, "первое-a").await;
    let _b = queued(&crew, &bot, "второе-b").await;
    let c = queued(&crew, &bot, "третье-c").await;
    wait_queue(&crew, &bot, 3).await;

    let moved = crew
        .ok("POST", &format!("/api/bots/{bot}/queue/{c}/move"), Some(json!({ "before": a })))
        .await;
    assert_eq!(queue_texts(&moved), ["третье-c", "первое-a", "второе-b"], "{moved}");
    assert_eq!(moved["paused"], false, "{moved}");

    crew.ok("POST", &format!("/api/approvals/{id}"), Some(json!({ "allow": true }))).await;
    wait_thread_text(&crew, &bot, "ответ 4").await;

    let requests = fake.requests.lock().unwrap();
    assert_eq!(requests.len(), 5, "два шага первого хода и три сообщения очереди");
    let seen: Vec<String> = requests
        .iter()
        .filter_map(|request| {
            let last = request["messages"].as_array()?.last()?;
            last["content"].as_str().map(str::to_string)
        })
        .filter(|text| matches!(text.as_str(), "первое-a" | "второе-b" | "третье-c"))
        .collect();
    assert_eq!(seen, ["третье-c", "первое-a", "второе-b"], "модель получила не тот порядок: {seen:?}");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_message_can_go_to_the_end_and_back_without_loss() {
    let (crew, fake, bot) = setup("queue-move-end", &[]).await;
    *fake.script.lock().unwrap() = vec![Step::Tool("run_command", json!({ "command": "echo один > C:\\Windows\\deloist-probe.txt" }))];

    crew.say(&bot, "старт").await;
    crew.wait_event("карточка", |e| e["type"] == "approval").await;
    let a = queued(&crew, &bot, "a").await;
    let b = queued(&crew, &bot, "b").await;
    let c = queued(&crew, &bot, "c").await;
    wait_queue(&crew, &bot, 3).await;

    let moved = crew
        .ok("POST", &format!("/api/bots/{bot}/queue/{a}/move"), Some(json!({ "before": null })))
        .await;
    assert_eq!(queue_texts(&moved), ["b", "c", "a"], "{moved}");

    // Средний c идёт вперёд, затем назад.
    let moved = crew
        .ok("POST", &format!("/api/bots/{bot}/queue/{c}/move"), Some(json!({ "before": b })))
        .await;
    assert_eq!(queue_texts(&moved), ["c", "b", "a"], "{moved}");

    let moved = crew
        .ok("POST", &format!("/api/bots/{bot}/queue/{c}/move"), Some(json!({ "before": a })))
        .await;
    assert_eq!(queue_texts(&moved), ["b", "c", "a"], "{moved}");

    let ids = queue_ids(&moved);
    assert_eq!(ids.len(), 3, "{moved}");
    assert_eq!(ids.iter().collect::<std::collections::HashSet<_>>().len(), 3, "элементы потерялись или задвоились: {ids:?}");
}

#[tokio::test(flavor = "multi_thread")]
async fn moving_before_itself_keeps_the_order() {
    let (crew, fake, bot) = setup("queue-move-self", &[]).await;
    *fake.script.lock().unwrap() = vec![Step::Tool("run_command", json!({ "command": "echo один > C:\\Windows\\deloist-probe.txt" }))];

    crew.say(&bot, "старт").await;
    crew.wait_event("карточка", |e| e["type"] == "approval").await;
    let a = queued(&crew, &bot, "a").await;
    let b = queued(&crew, &bot, "b").await;
    wait_queue(&crew, &bot, 2).await;

    let moved = crew
        .ok("POST", &format!("/api/bots/{bot}/queue/{b}/move"), Some(json!({ "before": b })))
        .await;
    assert_eq!(queue_ids(&moved), [a, b], "{moved}");
    assert_eq!(moved["paused"], false, "{moved}");
}

#[tokio::test(flavor = "multi_thread")]
async fn moving_refuses_unknown_ids_and_bad_bodies() {
    let (crew, fake, bot) = setup("queue-move-bad", &[]).await;
    *fake.script.lock().unwrap() = vec![Step::Tool("run_command", json!({ "command": "echo один > C:\\Windows\\deloist-probe.txt" }))];

    crew.say(&bot, "старт").await;
    crew.wait_event("карточка", |e| e["type"] == "approval").await;
    let a = queued(&crew, &bot, "a").await;
    let b = queued(&crew, &bot, "b").await;
    wait_queue(&crew, &bot, 2).await;

    let path = format!("/api/bots/{bot}/queue/{a}/move");
    let (status, _) = crew.send("POST", &format!("/api/bots/{bot}/queue/q_none/move"), Some(json!({ "before": b }))).await;
    assert_eq!(status, 404, "чужой qid");
    let (status, _) = crew.send("POST", &path, Some(json!({ "before": "q_none" }))).await;
    assert_eq!(status, 404, "чужой before");
    let queue = crew.ok("GET", &format!("/api/bots/{bot}/queue"), None).await;
    assert_eq!(queue_ids(&queue), [a.clone(), b.clone()], "очередь изменилась при ошибке: {queue}");

    let (status, _) = crew.send("POST", &format!("/api/bots/bot_none/queue/{a}/move"), Some(json!({ "before": b }))).await;
    assert_eq!(status, 404, "нет бота");

    let (status, _) = crew.send("POST", &path, None).await;
    assert_eq!(status, 400, "пустое тело");
    let (status, _) = crew.send("POST", &path, Some(json!({}))).await;
    assert_eq!(status, 400, "нет поля before");
    let (status, body) = crew.send("POST", &path, Some(json!({ "before": 5 }))).await;
    assert_eq!(status, 400, "before не строка: {body}");
    assert_eq!(body["error"], "куда передвинуть, не указано", "{body}");

    let queue = crew.ok("GET", &format!("/api/bots/{bot}/queue"), None).await;
    assert_eq!(queue_ids(&queue), [a, b], "очередь изменилась при ошибке: {queue}");
}

#[tokio::test(flavor = "multi_thread")]
async fn moving_a_message_does_not_resume_a_paused_queue() {
    let (crew, fake, bot) = setup("queue-move-paused", &[]).await;
    *fake.script.lock().unwrap() = vec![
        Step::Tool("run_command", json!({ "command": "echo один > C:\\Windows\\deloist-probe.txt" })),
        Step::Say("после паузы".into()),
    ];

    crew.say(&bot, "старт").await;
    crew.wait_event("карточка", |e| e["type"] == "approval").await;
    let a = queued(&crew, &bot, "a").await;
    let b = queued(&crew, &bot, "b").await;
    wait_queue(&crew, &bot, 2).await;

    crew.ok("POST", &format!("/api/bots/{bot}/stop"), None).await;
    crew.wait_event("конец хода", |e| e["type"] == "turn_end").await;
    wait_paused(&crew, &bot).await;
    let requests = fake.requests.lock().unwrap().len();

    let moved = crew
        .ok("POST", &format!("/api/bots/{bot}/queue/{b}/move"), Some(json!({ "before": a })))
        .await;
    assert_eq!(queue_ids(&moved), [b, a], "{moved}");
    assert_eq!(moved["paused"], true, "перестановка сняла паузу: {moved}");

    tokio::time::sleep(Duration::from_millis(300)).await;
    assert_eq!(fake.requests.lock().unwrap().len(), requests, "перестановка запустила новый ход");
}

#[tokio::test(flavor = "multi_thread")]
async fn moving_emits_the_new_order() {
    let (crew, fake, bot) = setup("queue-move-event", &[]).await;
    *fake.script.lock().unwrap() = vec![Step::Tool("run_command", json!({ "command": "echo один > C:\\Windows\\deloist-probe.txt" }))];

    crew.say(&bot, "старт").await;
    crew.wait_event("карточка", |e| e["type"] == "approval").await;
    let a = queued(&crew, &bot, "a").await;
    let _b = queued(&crew, &bot, "b").await;
    let c = queued(&crew, &bot, "c").await;
    wait_queue(&crew, &bot, 3).await;

    crew.clear_events();
    crew.ok("POST", &format!("/api/bots/{bot}/queue/{c}/move"), Some(json!({ "before": a }))).await;
    let event = crew
        .wait_event("очередь обновилась", |e| e["type"] == "queue_updated" && e["botId"] == bot.as_str())
        .await;
    assert_eq!(queue_texts(&event), ["c", "a", "b"], "{event}");
    assert_eq!(event["paused"], false, "{event}");
}

#[tokio::test(flavor = "multi_thread")]
async fn moved_order_survives_a_restart() {
    let (fake, port) = fake_server().await;
    let dir = common::temp_data_dir("queue-move-restart");
    let mut first = Crew::start_at(dir.clone(), &[]).await;
    let bot = configure(&first, port).await;
    *fake.script.lock().unwrap() = vec![Step::Tool("run_command", json!({ "command": "echo один > C:\\Windows\\deloist-probe.txt" }))];

    first.say(&bot, "старт").await;
    first.wait_event("карточка", |e| e["type"] == "approval").await;
    let a = queued(&first, &bot, "a").await;
    let _b = queued(&first, &bot, "b").await;
    let c = queued(&first, &bot, "c").await;
    wait_queue(&first, &bot, 3).await;

    let moved = first
        .ok("POST", &format!("/api/bots/{bot}/queue/{c}/move"), Some(json!({ "before": a })))
        .await;
    assert_eq!(queue_texts(&moved), ["c", "a", "b"], "{moved}");

    first.keep_data();
    drop(first);

    // Продолжение прерванного хода занимает бота: очередь ждёт его конца и порядок виден целиком.
    let gate = Arc::new(tokio::sync::Notify::new());
    *fake.script.lock().unwrap() = vec![Step::Hold(gate.clone())];
    let second = Crew::start_at(dir, &[]).await;
    let queue = second.ok("GET", &format!("/api/bots/{bot}/queue"), None).await;
    assert_eq!(queue["paused"], false, "после перезапуска очередь не стоит: {queue}");
    assert_eq!(queue_texts(&queue), ["c", "a", "b"], "порядок после перезапуска потерян: {queue}");
}


#[tokio::test(flavor = "multi_thread")]
async fn a_held_message_says_why_and_the_board_lists_it() {
    let (crew, fake, bot) = setup("queue-reason", &[]).await;
    *fake.script.lock().unwrap() = vec![
        Step::Tool("run_command", json!({ "command": "echo один > C:\\Windows\\deloist-probe.txt" })),
        Step::Say("первый".into()),
    ];

    crew.clear_events();
    crew.say(&bot, "старт").await;
    let approval = crew.wait_event("карточка", |e| e["type"] == "approval").await;
    let id = approval["id"].as_str().unwrap().to_string();
    crew.say(&bot, "потом это").await;
    let queue = wait_queue(&crew, &bot, 1).await;
    assert_eq!(queue["holdReason"], "card", "карточка без ответа держит очередь: {queue}");

    let board = crew.ok("GET", "/api/needs", None).await;
    assert_eq!(board["count"], 1, "значок считает только «Нужен ты»: {board}");
    let working = board["working"].as_array().unwrap();
    assert_eq!(working.len(), 1, "{board}");
    assert_eq!(working[0]["botId"], bot.as_str());
    assert!(working[0]["since"].as_i64().unwrap() > 0);
    let waiting = board["waiting"].as_array().unwrap();
    assert_eq!(waiting.len(), 1, "{board}");
    assert_eq!(waiting[0]["kind"], "queue");
    assert_eq!(waiting[0]["count"], 1);
    assert_eq!(waiting[0]["reason"], "card");
    assert_eq!(waiting[0]["text"], "потом это");

    crew.ok("POST", &format!("/api/approvals/{id}"), Some(json!({ "allow": true }))).await;
    wait_thread_text(&crew, &bot, "первый").await;
    wait_queue(&crew, &bot, 0).await;
    let board = crew.ok("GET", "/api/needs", None).await;
    assert!(board["waiting"].as_array().unwrap().is_empty(), "{board}");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_stopped_queue_reports_the_manual_pause_on_the_board() {
    let (crew, fake, bot) = setup("queue-reason-pause", &[]).await;
    *fake.script.lock().unwrap() = vec![
        Step::Tool("run_command", json!({ "command": "echo один > C:\\Windows\\deloist-probe.txt" })),
        Step::Say("после паузы".into()),
    ];
    crew.clear_events();
    crew.say(&bot, "старт").await;
    crew.wait_event("карточка", |e| e["type"] == "approval").await;
    crew.say(&bot, "в очереди").await;
    wait_queue(&crew, &bot, 1).await;
    crew.ok("POST", &format!("/api/bots/{bot}/stop"), None).await;
    crew.wait_event("конец хода", |e| e["type"] == "turn_end").await;
    let paused = wait_paused(&crew, &bot).await;
    assert_eq!(paused["holdReason"], "paused_manual", "{paused}");
    let board = crew.ok("GET", "/api/needs", None).await;
    assert_eq!(board["waiting"][0]["reason"], "paused_manual", "{board}");
    assert!(board["working"].as_array().unwrap().is_empty(), "{board}");
}
