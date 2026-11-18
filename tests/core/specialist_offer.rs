use crate::common;

use common::Crew;
use serde_json::{json, Value};
use std::sync::{Arc, Mutex};

use axum::body::{Body, Bytes};
use axum::extract::State;
use axum::response::Response;
use axum::routing::post;
use axum::Router;

const SPECIALISTS_RULE: &str =
    "Если задача явно по профилю специалиста — вызови offer_specialist, а не делай сама и не вызывай assign_task без согласия человека.";

#[derive(Clone, Default)]
struct Model {
    seen: Arc<Mutex<Vec<Value>>>,
}

fn sse(chunks: &[Value]) -> String {
    let mut out: String = chunks.iter().map(|c| format!("data: {c}\n\n")).collect();
    out.push_str("data: [DONE]\n\n");
    out
}

fn say(text: &str) -> String {
    sse(&[
        json!({ "choices": [{ "index": 0, "delta": { "role": "assistant", "content": text } }] }),
        json!({ "choices": [{ "index": 0, "delta": {}, "finish_reason": "stop" }], "usage": { "prompt_tokens": 10, "completion_tokens": 5 } }),
    ])
}

fn tool_step(name: &str, args: Value) -> String {
    sse(&[
        json!({ "choices": [{ "index": 0, "delta": { "tool_calls": [{ "index": 0, "id": format!("call_{name}"), "type": "function", "function": { "name": name, "arguments": args.to_string() } }] } }] }),
        json!({ "choices": [{ "index": 0, "delta": {}, "finish_reason": "tool_calls" }], "usage": { "prompt_tokens": 100, "completion_tokens": 10 } }),
    ])
}

async fn completions(State(model): State<Model>, body: Bytes) -> Response {
    let payload: Value = serde_json::from_slice(&body).unwrap_or_else(|_| json!({}));
    model.seen.lock().unwrap().push(payload.clone());
    let messages = payload["messages"].as_array().cloned().unwrap_or_default();
    let system = messages.first().and_then(|m| m["content"].as_str()).unwrap_or_default().to_string();
    let last_role = messages.last().and_then(|m| m["role"].as_str()).unwrap_or_default();
    let last_user = messages
        .iter()
        .rev()
        .find(|m| m["role"] == "user")
        .and_then(|m| m["content"].as_str())
        .unwrap_or_default();
    // Последний шаг хода — результат инструмента: отвечаем текстом, иначе ход зациклится.
    let text = if last_role == "tool" {
        say("готово")
    } else if system.contains("Ты — Лид") {
        if last_user.contains("не специалист") {
            tool_step("offer_specialist", json!({ "bot": "Гамма", "reason": "по профилю", "task": "сделай" }))
        } else if last_user.contains("нужен специалист") {
            tool_step("offer_specialist", json!({ "bot": "Бета", "reason": "Rust-разработка", "task": "почини сборку" }))
        } else if last_user.contains("@Бета") {
            tool_step("assign_task", json!({ "bot": "Бета", "text": "почини сборку" }))
        } else {
            say("готово")
        }
    } else {
        say("готово")
    };
    Response::builder().header("content-type", "text/event-stream").body(Body::from(text)).unwrap()
}

async fn setup(name: &str) -> (Crew, String, String, String, Arc<Mutex<Vec<Value>>>) {
    let seen = Arc::new(Mutex::new(Vec::new()));
    let router = Router::new()
        .route("/v1/chat/completions", post(completions))
        .with_state(Model { seen: seen.clone() });
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });

    let crew = Crew::start(name).await;
    crew.ok(
        "PUT",
        "/api/settings",
        Some(json!({ "driver": "api", "model": "m", "baseUrl": format!("http://127.0.0.1:{port}/v1"), "apiKey": "test-key" })),
    )
    .await;
    let lead = crew.ok("POST", "/api/bots", Some(json!({ "name": "Лид", "role": "старший", "rank": "lead" }))).await["bot"]["id"]
        .as_str()
        .unwrap()
        .to_string();
    let specialist = crew
        .ok("POST", "/api/bots", Some(json!({ "name": "Бета", "role": "Rust-разработка", "rank": "specialist" })))
        .await["bot"]["id"]
        .as_str()
        .unwrap()
        .to_string();
    let plain = crew.ok("POST", "/api/bots", Some(json!({ "name": "Гамма", "role": "своя роль", "rank": "lead" }))).await["bot"]["id"]
        .as_str()
        .unwrap()
        .to_string();
    (crew, lead, specialist, plain, seen)
}

fn last_system(seen: &Arc<Mutex<Vec<Value>>>) -> String {
    seen.lock()
        .unwrap()
        .iter()
        .rev()
        .find_map(|payload| payload["messages"][0]["content"].as_str().map(str::to_string))
        .unwrap_or_default()
}

fn specialist_block(prompt: &str) -> Option<String> {
    let start = prompt.find("# Специалисты команды")?;
    let tail = &prompt[start..];
    let end = tail.find(SPECIALISTS_RULE)? + SPECIALISTS_RULE.len();
    Some(tail[..end].to_string())
}

#[tokio::test(flavor = "multi_thread")]
async fn the_lead_prompt_lists_specialists_and_the_specialist_prompt_does_not() {
    let (crew, lead, specialist, _plain, seen) = setup("offer-prompt").await;
    crew.listen().await;

    crew.say(&lead, "привет").await;
    crew.wait_event("конец", |e| e["type"] == "turn_end" && e["botId"] == lead.as_str()).await;
    let prompt = last_system(&seen);
    let block = specialist_block(&prompt).unwrap_or_else(|| panic!("у ведущего нет блока специалистов: {prompt}"));
    assert!(block.contains("@Бета"), "{block}");
    assert!(block.contains("Rust-разработка"), "роль специалиста: {block}");
    assert!(block.contains(SPECIALISTS_RULE), "{block}");
    assert!(block.chars().count() <= 1_500, "блок длиннее 1500 знаков: {}", block.chars().count());
    assert!(!block.contains("@Гамма"), "ведущий не специалист: {block}");

    crew.say(&specialist, "привет").await;
    crew.wait_event("конец", |e| e["type"] == "turn_end" && e["botId"] == specialist.as_str()).await;
    let prompt = last_system(&seen);
    assert!(!prompt.contains("# Специалисты команды"), "специалист получил блок: {prompt}");

    // Специалистов не осталось — блока нет и у ведущего.
    crew.ok("PATCH", &format!("/api/bots/{specialist}"), Some(json!({ "rank": "lead" }))).await;
    crew.clear_events();
    crew.say(&lead, "ещё раз").await;
    crew.wait_event("конец", |e| e["type"] == "turn_end" && e["botId"] == lead.as_str()).await;
    let prompt = last_system(&seen);
    assert!(!prompt.contains("# Специалисты команды"), "блок остался без специалистов: {prompt}");
}

#[tokio::test(flavor = "multi_thread")]
async fn an_offer_waits_for_the_person_and_accept_hands_the_work_over() {
    let (crew, lead, specialist, _plain, _seen) = setup("offer-accept").await;
    crew.listen().await;

    crew.say(&lead, "нужен специалист").await;
    let event = crew.wait_event("карточка специалиста", |e| e["type"] == "specialist_offer").await;
    assert_eq!(event["botId"], lead.as_str());
    let offer = &event["offer"];
    assert_eq!(offer["hostBotId"], lead.as_str());
    assert_eq!(offer["specialistId"], specialist.as_str());
    assert_eq!(offer["specialistName"], "Бета");
    assert_eq!(offer["reason"], "Rust-разработка");
    assert_eq!(offer["task"], "почини сборку");
    assert!(offer["at"].as_i64().is_some_and(|at| at > 0), "{offer}");
    let tool = crew.wait_event("результат инструмента", |e| e["type"] == "tool_update").await;
    assert_eq!(tool["status"], "ok", "{tool}");
    assert!(tool["result"].as_str().unwrap_or_default().contains("Карточка показана человеку"), "{tool}");
    crew.wait_event("конец", |e| e["type"] == "turn_end" && e["botId"] == lead.as_str()).await;

    // До согласия человека специалист не начинал работу и не стал гостем.
    tokio::time::sleep(std::time::Duration::from_millis(300)).await;
    assert!(crew.find_event(|e| e["type"] == "delegate_start").is_none(), "специалист запустился без согласия");
    let state = crew.ok("GET", "/api/state", None).await;
    let specialist_state = state["bots"].as_array().unwrap().iter().find(|b| b["id"] == specialist.as_str()).unwrap();
    assert_eq!(specialist_state["lastActiveAt"], 0, "специалист уже работал: {specialist_state}");

    let id = offer["id"].as_str().unwrap().to_string();
    crew.clear_events();
    let (status, _) = crew.send("POST", &format!("/api/bots/{lead}/offers/{id}/accept"), None).await;
    assert_eq!(status, 202, "согласие человека принимается");
    let started = crew.wait_event("делегирование", |e| e["type"] == "delegate_start" && e["to"] == specialist.as_str()).await;
    assert_eq!(started["from"], "Лид");
    crew.wait_event("конец делегирования", |e| e["type"] == "delegate_end" && e["to"] == specialist.as_str()).await;
    crew.wait_event("конец", |e| e["type"] == "turn_end" && e["botId"] == lead.as_str()).await;

    // Делегат стал гостем треда хозяина.
    let state = crew.ok("GET", "/api/state", None).await;
    let host = state["bots"].as_array().unwrap().iter().find(|b| b["id"] == lead.as_str()).unwrap();
    let guests = host["guests"].as_array().unwrap_or_else(|| panic!("у хозяина нет гостей: {host}"));
    assert!(guests.iter().any(|guest| guest["botId"] == specialist.as_str()), "{guests:?}");
    let specialist_state = state["bots"].as_array().unwrap().iter().find(|b| b["id"] == specialist.as_str()).unwrap();
    assert!(specialist_state["lastActiveAt"].as_i64().is_some_and(|at| at > 0), "{specialist_state}");
}

#[tokio::test(flavor = "multi_thread")]
async fn declining_or_an_unknown_offer_starts_nothing() {
    let (crew, lead, _specialist, _plain, _seen) = setup("offer-decline").await;
    crew.listen().await;

    crew.say(&lead, "нужен специалист").await;
    let event = crew.wait_event("карточка специалиста", |e| e["type"] == "specialist_offer").await;
    crew.wait_event("конец", |e| e["type"] == "turn_end" && e["botId"] == lead.as_str()).await;
    let id = event["offer"]["id"].as_str().unwrap().to_string();

    crew.clear_events();
    let (status, _) = crew.send("POST", &format!("/api/bots/{lead}/offers/{id}/decline"), None).await;
    assert_eq!(status, 204, "отказ без тела ответа");
    tokio::time::sleep(std::time::Duration::from_millis(400)).await;
    assert!(crew.find_event(|e| e["type"] == "turn_start").is_none(), "отказ запустил ход");
    assert!(crew.find_event(|e| e["type"] == "delegate_start").is_none(), "отказ передал работу");

    let (status, _) = crew.send("POST", &format!("/api/bots/{lead}/offers/offer_unknown/accept"), None).await;
    assert_eq!(status, 404, "неизвестное предложение не принимается");
    let (status, _) = crew.send("POST", &format!("/api/bots/{lead}/offers/offer_unknown/decline"), None).await;
    assert_eq!(status, 404, "неизвестное предложение не отклоняется");
}

#[tokio::test(flavor = "multi_thread")]
async fn offering_a_non_specialist_is_a_tool_error() {
    let (crew, lead, _specialist, _plain, _seen) = setup("offer-wrong-target").await;
    crew.listen().await;

    crew.say(&lead, "предложи не специалиста").await;
    let tool = crew.wait_event("ошибка инструмента", |e| e["type"] == "tool_update" && e["status"] == "error").await;
    assert!(tool["result"].as_str().unwrap_or_default().contains("не специалист"), "{tool}");
    assert!(crew.find_event(|e| e["type"] == "specialist_offer").is_none(), "карточка для не-специалиста");
    crew.wait_event("конец", |e| e["type"] == "turn_end" && e["botId"] == lead.as_str()).await;
}
