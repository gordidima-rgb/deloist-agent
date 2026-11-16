//! Проверка длительных команд и остановки дерева процессов через настоящий цикл модели.
use crate::common;
use axum::{body::{Body, Bytes}, extract::State, response::Response, routing::post, Router};
use common::{Crew, wait_until};
use serde_json::{json, Value};

async fn model(State(args): State<Value>, body: Bytes) -> Response {
    let req: Value = serde_json::from_slice(&body).unwrap();
    let done = req["messages"].as_array().unwrap().iter().any(|m| m["role"] == "tool");
    let informed = req["messages"].as_array().unwrap().iter().any(|m| m["role"] == "tool" && m["content"].as_str().unwrap_or("").contains("превысила лимит"));
    let chunk = if done {
        json!({"choices":[{"index":0,"delta":{"content":if informed { "Получил уведомление и продолжил" } else { "Готово" }},"finish_reason":"stop"}]})
    } else {
        json!({"choices":[{"index":0,"delta":{"tool_calls":[{"index":0,"id":"command1","type":"function","function":{"name":"run_command","arguments":args.to_string()}}]},"finish_reason":"tool_calls"}]})
    };
    Response::builder().header("content-type", "text/event-stream").body(Body::from(format!("data: {chunk}\n\ndata: [DONE]\n\n"))).unwrap()
}
async fn setup(name: &str, args: Value, env: &[(&str, &str)]) -> (Crew, String) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    tokio::spawn(async move { axum::serve(listener, Router::new().route("/v1/chat/completions", post(model)).with_state(args)).await.unwrap() });
    let crew = Crew::start_with_env(name, env).await;
    crew.ok("PUT", "/api/settings", Some(json!({"driver":"api","model":"fake","baseUrl":format!("http://127.0.0.1:{port}/v1"),"apiKey":"test"}))).await;
    let state = crew.ok("GET", "/api/state", None).await;
    let bot = state["bots"][0]["id"].as_str().unwrap().to_string();
    crew.ok("PATCH", &format!("/api/bots/{bot}"), Some(json!({"approval":"auto"}))).await;
    crew.listen().await;
    (crew, bot)
}
#[tokio::test]
async fn command_can_run_longer_than_sixty_seconds() {
    let (crew, bot) = setup("long-command", json!({"command":"ping -n 64 127.0.0.1 > nul && echo complete"}), &[]).await;
    crew.say(&bot,"Запусти длительный тест").await;
    wait_until("команда дольше минуты завершилась", 95, || async { crew.find_event(|e| e["type"] == "turn_end").is_some() }).await;
    let event = crew.find_event(|e| e["type"] == "tool_update").expect("нет результата");
    assert_eq!(event["status"], "ok");
    let result = event["result"].as_str().unwrap();
    assert!(result.contains("complete") && !result.contains("таймаут"), "{result}");
}
async fn stopping_kills_child_tree(mode: &str) {
    let (crew, bot) = setup("stop-command", json!({"command":"ping -n 6 127.0.0.1 > nul && echo escaped > marker.txt"}), &[("CREW_SANDBOX",mode)]).await;
    crew.say(&bot,"Запусти команду").await;
    crew.wait_event("запуск команды", |e| e["type"] == "tool_start" && e["name"] == "run_command").await;
    tokio::time::sleep(std::time::Duration::from_millis(800)).await;
    crew.ok("POST", &format!("/api/bots/{bot}/stop"), None).await;
    wait_until("ход остановился", 6, || async { crew.find_event(|e| e["type"] == "turn_end").is_some() }).await;
    tokio::time::sleep(std::time::Duration::from_secs(6)).await;
    assert!(!crew.data_dir.join("workspaces").join(&bot).join("marker.txt").exists(), "дочерний процесс продолжил работу: {mode}");
}
#[tokio::test]
async fn stop_terminates_the_command_in_sandbox_and_without_it() {
    stopping_kills_child_tree("auto").await;
    stopping_kills_child_tree("off").await;
}
#[tokio::test]
async fn explicit_short_timeout_still_works() {
    let (crew, bot) = setup("timeout-command", json!({"command":"ping -n 6 127.0.0.1 > nul && echo complete", "timeout_seconds":1}), &[]).await;
    crew.say(&bot,"Запусти команду").await;
    crew.wait_event("конец хода", |e| e["type"] == "turn_end").await;
    let event = crew.find_event(|e| e["type"] == "tool_update").unwrap();
    assert_eq!(event["status"], "error");
    assert!(event["result"].as_str().unwrap().contains("превысила лимит 1 с"), "{event}");
    let thread = crew.ok("GET", &format!("/api/bots/{bot}/thread"), None).await;
    assert!(thread["messages"].as_array().unwrap().iter().any(|m| m["text"] == "Получил уведомление и продолжил"), "модель не получила сообщение о тайм-ауте");
}
