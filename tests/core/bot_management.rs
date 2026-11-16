//! Сквозная проверка инструментов управления командой через вызовы модели.
use crate::common;
use axum::{body::{Body, Bytes}, response::Response, routing::post, Router};
use common::Crew;
use serde_json::{json, Value};

async fn model(body: Bytes) -> Response {
    let req: Value = serde_json::from_slice(&body).unwrap();
    let results: Vec<&Value> = req["messages"].as_array().unwrap().iter().filter(|m| m["role"] == "tool").collect();
    let call = match results.len() {
        0 => Some(("create_bot", json!({"name":"Художник", "role":"рисует", "persona":"Создавай иллюстрации"}))),
        1 => {
            let created: Value = serde_json::from_str(results[0]["content"].as_str().unwrap()).unwrap_or(Value::Null);
            if created["id"].is_string() {
                Some(("update_bot", json!({"bot_id":created["id"], "persona":"Создавай иллюстрации и текстуры", "model":"artist-model"})))
            } else { None }
        },
        2 => Some(("list_bots", json!({}))),
        _ => None,
    };
    let chunk = if let Some((name, args)) = call {
        json!({"choices":[{"index":0,"delta":{"tool_calls":[{"index":0,"id":format!("call_{}", results.len()),"type":"function","function":{"name":name,"arguments":args.to_string()}}]},"finish_reason":"tool_calls"}]})
    } else {
        json!({"choices":[{"index":0,"delta":{"content":"Готово"},"finish_reason":"stop"}]})
    };
    Response::builder().header("content-type", "text/event-stream").body(Body::from(format!("data: {chunk}\n\ndata: [DONE]\n\n"))).unwrap()
}

#[tokio::test]
async fn bot_creates_updates_and_lists_a_bot() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    tokio::spawn(async move { axum::serve(listener, Router::new().route("/v1/chat/completions", post(model))).await.unwrap() });
    let crew = Crew::start("bot-management").await;
    crew.ok("PUT", "/api/settings", Some(json!({"driver":"api","model":"fake","baseUrl":format!("http://127.0.0.1:{port}/v1"),"apiKey":"test"}))).await;
    let state = crew.ok("GET", "/api/state", None).await;
    let actor = state["bots"][0]["id"].as_str().unwrap();
    crew.ok("PATCH", &format!("/api/bots/{actor}"), Some(json!({"approval":"auto"}))).await;
    crew.listen().await;
    crew.say(actor, "Создай художника и настрой его").await;
    crew.wait_event("конец хода", |e| e["type"] == "turn_end").await;
    let state = crew.ok("GET", "/api/state", None).await;
    let bot = state["bots"].as_array().unwrap().iter().find(|b| b["name"] == "Художник").expect("бот не создан");
    assert_eq!(bot["persona"], "Создавай иллюстрации и текстуры");
    assert_eq!(bot["model"], "artist-model");
    assert_eq!(bot["approval"], "ask");
    assert!(crew.data_dir.join("workspaces").join(bot["id"].as_str().unwrap()).exists());
    for name in ["create_bot", "update_bot", "list_bots"] {
        let start = crew.find_event(|e| e["type"] == "tool_start" && e["name"] == name).expect("нет вызова");
        assert!(crew.find_event(|e| e["type"] == "tool_update" && e["callId"] == start["callId"] && e["status"] == "ok").is_some(), "не выполнен {name}");
    }
    assert!(crew.find_event(|e| e["type"] == "state_changed").is_some());
}

#[tokio::test]
async fn creation_asks_permission_and_denial_changes_nothing() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    tokio::spawn(async move { axum::serve(listener, Router::new().route("/v1/chat/completions", post(model))).await.unwrap() });
    let crew = Crew::start("bot-management-denied").await;
    crew.ok("PUT", "/api/settings", Some(json!({"driver":"api","model":"fake","baseUrl":format!("http://127.0.0.1:{port}/v1"),"apiKey":"test"}))).await;
    let state = crew.ok("GET", "/api/state", None).await;
    let actor = state["bots"][0]["id"].as_str().unwrap();
    crew.listen().await;
    crew.say(actor, "Создай художника").await;
    let approval = crew.wait_event("запрос разрешения", |e| e["type"] == "approval").await;
    assert_eq!(approval["tool"], "create_bot");
    crew.ok("POST", &format!("/api/approvals/{}", approval["id"].as_str().unwrap()), Some(json!({"allow":false}))).await;
    crew.wait_event("конец хода", |e| e["type"] == "turn_end").await;
    let state = crew.ok("GET", "/api/state", None).await;
    assert!(!state["bots"].as_array().unwrap().iter().any(|b| b["name"] == "Художник"));
}
