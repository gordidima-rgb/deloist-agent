use crate::common;

use common::Crew;
use serde_json::json;
use std::sync::{Arc, Mutex};

static REQUESTS: Mutex<Vec<serde_json::Value>> = Mutex::new(Vec::new());
/// `REQUESTS` общий на файл: тесты идут по одному, иначе чужой `clear()` стирает записи соседа.
static SERIAL: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());
use axum::body::{Body, Bytes};
use axum::extract::State;
use axum::response::Response;
use axum::routing::post;
use axum::Router;

#[derive(Clone, Default)]
struct Model {
    seen: Arc<Mutex<Vec<String>>>,
    delegated_write: bool,
}

fn sse(chunks: &[serde_json::Value]) -> String {
    let mut out: String = chunks.iter().map(|c| format!("data: {c}\n\n")).collect();
    out.push_str("data: [DONE]\n\n");
    out
}

fn tool_step(name: &str, args: serde_json::Value) -> String {
    sse(&[
        json!({ "choices": [{ "index": 0, "delta": { "tool_calls": [{ "index": 0, "id": format!("call_{name}"), "type": "function", "function": { "name": name, "arguments": args.to_string() } }] } }] }),
        json!({ "choices": [{ "index": 0, "delta": {}, "finish_reason": "tool_calls" }], "usage": { "prompt_tokens": 100, "completion_tokens": 10 } }),
    ])
}

async fn completions(State(model): State<Model>, body: Bytes) -> Response {
    let payload: serde_json::Value = serde_json::from_slice(&body).unwrap_or_else(|_| json!({}));
    REQUESTS.lock().unwrap().push(payload.clone());
    let messages = payload["messages"].as_array().cloned().unwrap_or_default();
    let system = messages.first().and_then(|m| m["content"].as_str()).unwrap_or_default().to_string();
    let user_text = messages.iter().filter_map(|m| m["content"].as_str()).collect::<Vec<_>>().join(" ");
    model.seen.lock().unwrap().push(system.clone());
    let used_tool = messages.iter().any(|m| m["role"] == "tool");
    let text = if used_tool {
        sse(&[
            json!({ "choices": [{ "index": 0, "delta": { "role": "assistant", "content": "готово" } }] }),
            json!({ "choices": [{ "index": 0, "delta": {}, "finish_reason": "stop" }], "usage": { "prompt_tokens": 10, "completion_tokens": 5 } }),
        ])
    } else if system.contains("Ты — Альфа") {
        if user_text.contains("@Бета") {
            tool_step("delegate", json!({ "bot": "Бета", "task": "сделай" }))
        } else {
            tool_step("delegate", json!({ "bot": "Гамма", "task": "сделай" }))
        }
    } else if model.delegated_write && system.contains("Ты — Гамма") {
        tool_step("write_file", json!({ "path": "receiver.txt", "content": "needs the sender's approval policy" }))
    } else {
        sse(&[
            json!({ "choices": [{ "index": 0, "delta": { "role": "assistant", "content": "готово" } }] }),
            json!({ "choices": [{ "index": 0, "delta": {}, "finish_reason": "stop" }], "usage": { "prompt_tokens": 10, "completion_tokens": 5 } }),
        ])
    };
    Response::builder().header("content-type", "text/event-stream").body(Body::from(text)).unwrap()
}

async fn with_model(name: &str) -> (Crew, String, String, String) {
    with_model_case(name, false).await
}

async fn with_model_case(name: &str, delegated_write: bool) -> (Crew, String, String, String) {
    REQUESTS.lock().unwrap().clear();
    let router = Router::new().route("/v1/chat/completions", post(completions)).with_state(Model { delegated_write, ..Model::default() });
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
    let crew = Crew::start(name).await;
    crew.ok("PUT", "/api/settings", Some(json!({ "driver": "api", "model": "m", "baseUrl": format!("http://127.0.0.1:{port}/v1"), "apiKey": "test-key" }))).await;
    let alpha = crew.ok("POST", "/api/bots", Some(json!({ "name": "Альфа", "role": "старший", "approval": "strict" }))).await["bot"]["id"].as_str().unwrap().to_string();
    let beta = crew.ok("POST", "/api/bots", Some(json!({ "name": "Бета", "role": "младший", "approval": "auto" }))).await["bot"]["id"].as_str().unwrap().to_string();
    let gamma = crew.ok("POST", "/api/bots", Some(json!({ "name": "Гамма", "role": "младший", "approval": "auto" }))).await["bot"]["id"].as_str().unwrap().to_string();
    (crew, alpha, beta, gamma)
}

#[tokio::test(flavor = "multi_thread")]
async fn mention_gives_note_with_bot_id() {
    let _serial = SERIAL.lock().await;
    let (crew, alpha, beta, _gamma) = with_model("mention-note").await;
    crew.listen().await;
    crew.say(&alpha, &format!("@Бета привет")).await;
    crew.wait_event("конец", |e| e["type"] == "turn_end" && e["botId"] == alpha.as_str()).await;
    let reqs = REQUESTS.lock().unwrap();
    let found = reqs.iter().any(|r| {
        let msgs = r["messages"].as_array().cloned().unwrap_or_default();
        msgs.iter().any(|m| {
            m["content"].as_str().map(|c| c.contains("Человек упомянул ботов") && c.contains(&beta)).unwrap_or(false)
        })
    });
    assert!(found, "заметка с id Беты не дошла до модели");
    let _ = (crew, beta);
}

#[tokio::test(flavor = "multi_thread")]
async fn mentioned_delegate_runs_without_card() {
    let _serial = SERIAL.lock().await;
    let (crew, alpha, beta, _gamma) = with_model("mention-delegate-ok").await;
    // Гостем становится специалист, а не обычный бот.
    crew.ok("PATCH", &format!("/api/bots/{beta}"), Some(json!({ "rank": "specialist" }))).await;
    crew.listen().await;
    crew.say(&alpha, "@Бета сделай").await;
    // Делегирование упомянутому боту не должно поднять карточку.
    tokio::time::sleep(std::time::Duration::from_millis(300)).await;
    assert!(crew.find_event(|e| e["type"] == "approval").is_none(), "карточка не должна появляться для упомянутого бота");
    crew.wait_event("конец", |e| e["type"] == "turn_end" && e["botId"] == alpha.as_str()).await;

    // Делегирование записало специалиста гостем в тред вызывающего бота.
    let state = crew.ok("GET", "/api/state", None).await;
    let host = state["bots"].as_array().unwrap().iter().find(|b| b["id"] == alpha.as_str()).unwrap();
    assert!(host["lastActiveAt"].as_i64().is_some_and(|at| at > 0), "конец хода отметил активность хозяина");
    let guest = state["bots"].as_array().unwrap().iter().find(|b| b["id"] == beta.as_str()).unwrap();
    assert!(guest["lastActiveAt"].as_i64().is_some_and(|at| at > 0), "конец хода отметил активность делегата");
    let guests = host["guests"].as_array().unwrap_or_else(|| panic!("у хозяина нет гостей: {host}"));
    assert_eq!(guests.len(), 1, "{guests:?}");
    assert_eq!(guests[0]["botId"], beta.as_str());
    assert!(guests[0]["since"].as_i64().is_some_and(|at| at > 0));

    // removeGuest убирает гостя сразу.
    crew.ok("PATCH", &format!("/api/bots/{alpha}"), Some(json!({ "removeGuest": beta }))).await;
    let state = crew.ok("GET", "/api/state", None).await;
    let host = state["bots"].as_array().unwrap().iter().find(|b| b["id"] == alpha.as_str()).unwrap();
    assert!(host["guests"].as_array().unwrap().is_empty(), "гость остался: {host}");
}

#[tokio::test(flavor = "multi_thread")]
async fn unmentioned_handoff_runs_but_the_receiver_keeps_the_senders_strict_policy() {
    let _serial = SERIAL.lock().await;
    let (crew, alpha, _beta, gamma) = with_model_case("mention-delegate-policy", true).await;
    crew.listen().await;
    crew.say(&alpha, "сделай").await;
    let card = crew.wait_event("карточка", |e| e["type"] == "approval").await;
    assert_eq!(card["tool"], "write_file", "само поручение идёт без карточки; спрашивается действие получателя");
    assert_eq!(card["botId"], gamma);
    crew.ok("POST", &format!("/api/approvals/{}", card["id"].as_str().unwrap()), Some(json!({ "allow": false }))).await;
    crew.wait_event("конец", |e| e["type"] == "turn_end" && e["botId"] == alpha.as_str()).await;
    assert!(!crew.data_dir.join("workspaces").join(&gamma).join("receiver.txt").exists());
}
