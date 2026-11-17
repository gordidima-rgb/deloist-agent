//! Ответ из уведомления Windows: вопрос бота даёт тост с кнопками, ответ в окне его снимает.
//! Настоящий тост в тестах не показывается: `CREW_TEST_TOAST_SINK` пишет по строке JSON на показ и на снятие.

use crate::common::{wait_until, Crew};

use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use axum::body::Bytes;
use axum::extract::State;
use axum::response::{IntoResponse, Response};
use axum::routing::post;
use axum::{Json, Router};
use serde_json::{json, Value};

#[derive(Clone, Default)]
struct Fake {
    script: Arc<Mutex<Vec<Value>>>,
}

fn tool(name: &str, args: Value) -> Value {
    json!({ "role": "assistant", "content": "", "tool_calls": [{ "id": "c1", "type": "function", "function": { "name": name, "arguments": args.to_string() } }] })
}

async fn completions(State(fake): State<Fake>, _body: Bytes) -> Response {
    let message = {
        let mut script = fake.script.lock().unwrap();
        if script.is_empty() { json!({ "role": "assistant", "content": "готово" }) } else { script.remove(0) }
    };
    Json(json!({ "choices": [{ "message": message }] })).into_response()
}

fn lines(sink: &PathBuf) -> Vec<Value> {
    std::fs::read_to_string(sink).unwrap_or_default().lines().filter_map(|l| serde_json::from_str(l).ok()).collect()
}

#[tokio::test(flavor = "multi_thread")]
async fn a_question_shows_a_toast_with_buttons_and_answering_in_the_app_removes_it() {
    let fake = Fake::default();
    let router = Router::new().route("/v1/chat/completions", post(completions)).with_state(fake.clone());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });

    let sink = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("toast-sink.jsonl");
    let _ = std::fs::remove_file(&sink);
    let sink_text = sink.display().to_string();
    let crew = Crew::start_with_env("toast-reply", &[("CREW_TEST_TOAST_SINK", &sink_text)]).await;
    crew.ok("PUT", "/api/settings", Some(json!({ "driver": "api", "baseUrl": format!("http://127.0.0.1:{port}/v1"), "apiKey": "k" }))).await;
    crew.listen().await;
    let bot = crew.ok("GET", "/api/state", None).await["bots"][0]["id"].as_str().unwrap().to_string();

    *fake.script.lock().unwrap() = vec![tool("ask_user", json!({ "question": "Какой склад?", "choices": ["Москва", "Казань"] }))];
    crew.say(&bot, "заведи отчёт").await;
    let question = crew.wait_event("вопрос", |e| e["type"] == "question").await;
    let id = question["id"].as_str().unwrap().to_string();

    wait_until("тост показан", 10, || async { lines(&sink).iter().any(|l| l["event"] == "show" && l["id"] == id.as_str()) }).await;
    let shown = lines(&sink).into_iter().find(|l| l["event"] == "show").unwrap();
    assert_eq!(shown["kind"], "question");
    assert_eq!(shown["botId"], bot.as_str());
    assert_eq!(shown["body"], "Какой склад?");
    assert_eq!(shown["buttons"], json!(["Москва", "Казань"]));
    assert_eq!(shown["input"], false);
    assert_eq!(shown["args"], json!([format!("q|{id}|c0"), format!("q|{id}|c1")]));
    assert!(!lines(&sink).iter().any(|l| l["event"] == "remove"), "пока ответа нет, тост на месте");

    crew.ok("POST", &format!("/api/questions/{id}"), Some(json!({ "answer": "Казань" }))).await;
    wait_until("тост снят", 10, || async { lines(&sink).iter().any(|l| l["event"] == "remove" && l["id"] == id.as_str()) }).await;
}
