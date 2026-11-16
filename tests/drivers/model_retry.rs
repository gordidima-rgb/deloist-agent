//! Временный сбой модели не повторяет уже выполненные инструменты.
use crate::common;
use std::sync::{Arc, atomic::{AtomicUsize, Ordering}};
use axum::{body::Bytes, extract::State, http::StatusCode, response::{IntoResponse, Response}, routing::post, Json, Router};
use common::Crew;
use serde_json::{json, Value};
#[derive(Clone)]
struct Fake { calls: Arc<AtomicUsize>, mode: &'static str }
async fn model(State(fake): State<Fake>, body: Bytes) -> Response {
    let n = fake.calls.fetch_add(1, Ordering::SeqCst);
    let req: Value = serde_json::from_slice(&body).unwrap();
    if fake.mode == "unauthorized" { return (StatusCode::UNAUTHORIZED, "bad key").into_response(); }
    if fake.mode == "offline" || (fake.mode == "recover" && n == 1) { return (StatusCode::INTERNAL_SERVER_ERROR, "temporary outage").into_response(); }
    let has_tool = req["messages"].as_array().unwrap().iter().any(|m| m["role"] == "tool");
    let message = if !has_tool {
        json!({"role":"assistant", "content":"", "tool_calls":[{"id":"write1","type":"function","function":{"name":"write_file","arguments":"{\"path\":\"created.txt\",\"content\":\"done\"}"}}]})
    } else { json!({"role":"assistant","content":"Завершено после восстановления"}) };
    Json(json!({"choices":[{"message":message}]})).into_response()
}
async fn setup(mode: &'static str) -> (Crew, String, Fake) {
    let fake = Fake { calls: Arc::new(AtomicUsize::new(0)), mode };
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    tokio::spawn({ let fake=fake.clone(); async move { axum::serve(listener, Router::new().route("/v1/chat/completions",post(model)).with_state(fake)).await.unwrap() }});
    let crew = Crew::start("model-retry").await;
    crew.ok("PUT", "/api/settings", Some(json!({"driver":"api","model":"fake","baseUrl":format!("http://127.0.0.1:{port}/v1"),"apiKey":"test"}))).await;
    let state = crew.ok("GET", "/api/state", None).await;
    let bot=state["bots"][0]["id"].as_str().unwrap().to_string();
    crew.ok("PATCH", &format!("/api/bots/{bot}"), Some(json!({"approval":"auto"}))).await;
    crew.listen().await;
    (crew,bot,fake)
}
#[tokio::test]
async fn resumes_after_server_error_without_repeating_the_tool() {
    let (crew,bot,fake)=setup("recover").await;
    crew.say(&bot,"Выполни задачу").await;
    crew.wait_event("повтор запроса", |e| e["type"] == "model_retry").await;
    let end=crew.wait_event("конец хода", |e| e["type"] == "turn_end").await;
    assert_eq!(end["failed"],false);
    assert_eq!(fake.calls.load(Ordering::SeqCst),3);
    let events=crew.events.lock().unwrap();
    assert_eq!(events.iter().filter(|e| e["type"] == "tool_start" && e["name"] == "write_file").count(),1);
    drop(events);
    let thread=crew.ok("GET", &format!("/api/bots/{bot}/thread"),None).await;
    assert!(thread["messages"].as_array().unwrap().iter().any(|m| m["text"] == "Завершено после восстановления"));
}
#[tokio::test]
async fn wrong_key_is_not_retried() {
    let (crew,bot,fake)=setup("unauthorized").await;
    crew.say(&bot,"Задача").await;
    crew.wait_event("ошибка", |e| e["type"] == "turn_end").await;
    assert_eq!(fake.calls.load(Ordering::SeqCst),1);
    assert!(crew.find_event(|e| e["type"] == "model_retry").is_none());
}
#[tokio::test]
async fn stop_interrupts_the_reconnect_wait() {
    let (crew,bot,_)=setup("offline").await;
    crew.say(&bot,"Задача").await;
    crew.wait_event("ожидание модели", |e| e["type"] == "model_retry").await;
    crew.ok("POST", &format!("/api/bots/{bot}/stop"),None).await;
    crew.wait_event("остановка", |e| e["type"] == "turn_end").await;
}
#[tokio::test]
async fn group_also_recovers_without_repeating_the_tool() {
    let (crew,bot,fake)=setup("recover").await;
    let room=crew.ok("POST", "/api/rooms", Some(json!({"name":"Совет","botIds":[bot],"lead":bot,"mode":"lead"}))).await;
    let id=room["room"]["id"].as_str().unwrap();
    crew.ok("POST", &format!("/api/rooms/{id}/messages"),Some(json!({"text":"Выполни задачу"}))).await;
    crew.wait_event("повтор в группе", |e| e["type"] == "room_retry" && e["roomId"] == id).await;
    let end=crew.wait_event("конец хода группы", |e| e["type"] == "room_turn_end" && e["roomId"] == id).await;
    assert_eq!(end["failed"],false);
    assert_eq!(fake.calls.load(Ordering::SeqCst),3);
    let events=crew.events.lock().unwrap();
    assert_eq!(events.iter().filter(|e| e["type"] == "tool_start" && e["name"] == "write_file").count(),1);
}
