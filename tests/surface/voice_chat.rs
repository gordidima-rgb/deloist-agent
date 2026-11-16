use crate::common;

use common::Crew;
use serde_json::json;
use std::sync::{Arc, Mutex};

use axum::body::{Body, Bytes};
use axum::extract::State;
use axum::response::Response;
use axum::routing::post;
use axum::Router;

#[derive(Clone, Default)]
struct Model {
    requests: Arc<Mutex<Vec<serde_json::Value>>>,
}

async fn completions(State(model): State<Model>, body: Bytes) -> Response {
    let payload: serde_json::Value = serde_json::from_slice(&body).unwrap_or_else(|_| json!({}));
    model.requests.lock().unwrap().push(payload.clone());
    let text = "data: {\"choices\":[{\"index\":0,\"delta\":{\"role\":\"assistant\",\"content\":\"готово\"}}]}\n\ndata: {\"choices\":[{\"index\":0,\"delta\":{},\"finish_reason\":\"stop\"}],\"usage\":{\"prompt_tokens\":10,\"completion_tokens\":5}}\n\ndata: [DONE]\n\n";
    Response::builder().header("content-type", "text/event-stream").body(Body::from(text)).unwrap()
}

async fn with_model(name: &str) -> (Crew, String, Arc<Mutex<Vec<serde_json::Value>>>) {
    let requests = Arc::new(Mutex::new(Vec::new()));
    let router = Router::new().route("/v1/chat/completions", post(completions)).with_state(Model { requests: requests.clone() });
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
    let crew = Crew::start(name).await;
    crew.ok("PUT", "/api/settings", Some(json!({ "driver": "api", "model": "m", "baseUrl": format!("http://127.0.0.1:{port}/v1"), "apiKey": "test-key" }))).await;
    let bot = crew.ok("POST", "/api/bots", Some(json!({ "name": "Бот", "role": "помощник", "approval": "strict" }))).await["bot"]["id"].as_str().unwrap().to_string();
    (crew, bot, requests)
}

#[tokio::test(flavor = "multi_thread")]
async fn voice_true_from_window_adds_note() {
    let (crew, bot, requests) = with_model("voice-window").await;
    crew.listen().await;
    crew.client
        .post(format!("{}/api/bots/{bot}/messages", crew.base))
        .header("cookie", &crew.cookie)
        .json(&json!({ "text": "привет", "voice": true }))
        .send()
        .await
        .unwrap();
    crew.wait_event("конец", |e| e["type"] == "turn_end" && e["botId"] == bot.as_str()).await;
    let reqs = requests.lock().unwrap();
    let found = reqs.iter().any(|r| {
        let msgs = r["messages"].as_array().cloned().unwrap_or_default();
        msgs.iter().any(|m| m["content"].as_str().map(|c| c.contains("Это голосовой разговор")).unwrap_or(false))
    });
    assert!(found, "заметка голосового разговора не дошла");
}

#[tokio::test(flavor = "multi_thread")]
async fn voice_true_with_agent_key_is_ignored() {
    let (crew, bot, requests) = with_model("voice-agent").await;
    crew.listen().await;
    let runtime: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(crew.data_dir.join("runtime.json")).unwrap()).unwrap();
    let token = runtime["token"].as_str().unwrap();
    crew.client
        .post(format!("{}/api/bots/{bot}/messages", crew.base))
        .bearer_auth(token)
        .json(&json!({ "text": "привет", "voice": true }))
        .send()
        .await
        .unwrap();
    crew.wait_event("конец", |e| e["type"] == "turn_end" && e["botId"] == bot.as_str()).await;
    let reqs = requests.lock().unwrap();
    let found = reqs.iter().any(|r| {
        let msgs = r["messages"].as_array().cloned().unwrap_or_default();
        msgs.iter().any(|m| m["content"].as_str().map(|c| c.contains("Это голосовой разговор")).unwrap_or(false))
    });
    assert!(!found, "ключ программы не должен включать голосовую заметку");
}
