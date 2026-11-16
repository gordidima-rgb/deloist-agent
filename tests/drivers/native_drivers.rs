//! Anthropic и Gemini против фейковых серверов: какой ключ куда уходит и что получает провайдер.

use crate::common;

use std::sync::{Arc, Mutex};

use axum::body::{Body, Bytes};
use axum::extract::State;
use axum::http::HeaderMap;
use axum::response::Response;
use axum::routing::post;
use axum::Router;
use common::Crew;
use serde_json::{json, Value};

#[derive(Clone, Default)]
struct Seen {
    requests: Arc<Mutex<Vec<(HeaderMap, Value)>>>,
}

async fn anthropic(State(seen): State<Seen>, headers: HeaderMap, body: Bytes) -> Response {
    let payload: Value = serde_json::from_slice(&body).unwrap_or_else(|_| json!({}));
    seen.requests.lock().unwrap().push((headers, payload));
    let events = [
        json!({ "type": "message_start", "message": { "usage": { "input_tokens": 12, "cache_read_input_tokens": 0 } } }),
        json!({ "type": "content_block_start", "index": 0, "content_block": { "type": "text", "text": "" } }),
        json!({ "type": "content_block_delta", "index": 0, "delta": { "type": "text_delta", "text": "Привет от Claude." } }),
        json!({ "type": "content_block_stop", "index": 0 }),
        json!({ "type": "message_delta", "delta": { "stop_reason": "end_turn" }, "usage": { "output_tokens": 5 } }),
        json!({ "type": "message_stop" }),
    ];
    let text: String = events.iter().map(|event| format!("event: {}\ndata: {event}\n\n", event["type"].as_str().unwrap())).collect();
    Response::builder().header("content-type", "text/event-stream").body(Body::from(text)).unwrap()
}

async fn openai(State(seen): State<Seen>, headers: HeaderMap, body: Bytes) -> Response {
    let payload: Value = serde_json::from_slice(&body).unwrap_or_else(|_| json!({}));
    seen.requests.lock().unwrap().push((headers, payload));
    let text = format!(
        "data: {}\n\ndata: {}\n\ndata: [DONE]\n\n",
        json!({ "choices": [{ "index": 0, "delta": { "role": "assistant", "content": "ок" } }] }),
        json!({ "choices": [{ "index": 0, "delta": {}, "finish_reason": "stop" }] }),
    );
    Response::builder().header("content-type", "text/event-stream").body(Body::from(text)).unwrap()
}

async fn serve(router: Router) -> u16 {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
    port
}

fn last_reply(thread: &Value) -> String {
    thread["messages"]
        .as_array()
        .and_then(|list| list.iter().rev().find(|m| m["role"] == "assistant"))
        .and_then(|m| m["text"].as_str())
        .unwrap_or_default()
        .to_string()
}

#[tokio::test]
async fn anthropic_gets_its_own_key_and_a_request_it_accepts() {
    let seen = Seen::default();
    let port = serve(Router::new().route("/v1/messages", post(anthropic)).with_state(seen.clone())).await;
    let crew = Crew::start("native-anthropic").await;
    crew.ok(
        "PUT",
        "/api/settings",
        Some(json!({
            "driver": "anthropic",
            "model": "claude-sonnet-5-5",
            "baseUrl": format!("http://127.0.0.1:{port}"),
            "apiKey": "sk-openai-only",
            "anthropicApiKey": "sk-ant-only",
            "temperature": 0.4,
        })),
    )
    .await;
    let state = crew.ok("GET", "/api/state", None).await;
    assert_eq!(state["settings"]["hasAnthropicKey"], true);
    let bot = state["bots"][0]["id"].as_str().unwrap().to_string();
    crew.listen().await;
    crew.say(&bot, "привет").await;
    crew.wait_event("конец хода", |e| e["type"] == "turn_end").await;

    let requests = seen.requests.lock().unwrap().clone();
    assert_eq!(requests.len(), 1);
    let (headers, body) = &requests[0];
    assert_eq!(headers["x-api-key"], "sk-ant-only", "Anthropic получает свой ключ");
    assert!(!format!("{headers:?}").contains("sk-openai-only"), "ключ OpenAI не уходит Anthropic");
    assert!(body.get("temperature").is_none(), "нынешние модели Claude отвечают на temperature ошибкой");
    assert_eq!(body["cache_control"]["type"], "ephemeral");
    assert_eq!(body["max_tokens"], 64_000);
    let thread = crew.ok("GET", &format!("/api/bots/{bot}/thread"), None).await;
    assert_eq!(last_reply(&thread), "Привет от Claude.");
}

#[tokio::test]
async fn a_bot_on_another_provider_never_gets_the_main_key() {
    let seen = Seen::default();
    let port = serve(Router::new().route("/v1/chat/completions", post(openai)).with_state(seen.clone())).await;
    let crew = Crew::start("native-foreign").await;
    crew.ok(
        "PUT",
        "/api/settings",
        Some(json!({
            "driver": "api",
            "model": "fake-model",
            "baseUrl": format!("http://127.0.0.1:{port}/v1"),
            "apiKey": "sk-openai-only",
        })),
    )
    .await;
    let state = crew.ok("GET", "/api/state", None).await;
    let bot = state["bots"][0]["id"].as_str().unwrap().to_string();
    // Бот на Anthropic при общих настройках OpenAI: ключа Anthropic нет, значит и запроса нет.
    crew.ok("PATCH", &format!("/api/bots/{bot}"), Some(json!({ "driver": "anthropic", "model": "claude-sonnet-5-5" }))).await;
    crew.listen().await;
    crew.say(&bot, "привет").await;
    crew.wait_event("конец хода", |e| e["type"] == "turn_end").await;
    let thread = crew.ok("GET", &format!("/api/bots/{bot}/thread"), None).await;
    assert!(last_reply(&thread).contains("нет ключа Anthropic"), "{}", last_reply(&thread));
    assert!(seen.requests.lock().unwrap().is_empty(), "запрос ушёл на адрес OpenAI");
}

#[tokio::test]
async fn an_empty_address_is_fine_for_native_providers_only() {
    let crew = Crew::start("native-address").await;
    crew.ok("PUT", "/api/settings", Some(json!({ "driver": "gemini", "baseUrl": "" }))).await;
    assert_eq!(crew.ok("GET", "/api/state", None).await["settings"]["baseUrl"], "");
    let (status, _) = crew.send("PUT", "/api/settings", Some(json!({ "driver": "api", "baseUrl": "" }))).await;
    assert_eq!(status, 400, "OpenAI-совместимому API нужен адрес");
}
