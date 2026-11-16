//! Экономная передача задач: бриф, свежий контекст делегата, короткий итог и его цена.

use crate::common;

use axum::body::{Body, Bytes};
use axum::extract::State;
use axum::response::Response;
use axum::routing::post;
use axum::Router;
use common::Crew;
use serde_json::{json, Value};
use std::sync::{Arc, Mutex};

const DELEGATION_TAIL: &str = "Верни итог не длиннее 20 строк: что сделано, какие файлы, как проверено, что осталось.";
const TOO_LONG: &str = "Бриф длиннее 6000 знаков: сократи до сути — цель, входные данные, что вернуть";
const OLD_MARK: &str = "старое-сообщение-МАРКЕР";
const BRIEF: &str = "Разбери файл report.txt: что изменить.";

#[derive(Clone, Default)]
struct Model {
    /// Все тела запросов этого теста: у каждого теста свой сервер и свой список.
    requests: Arc<Mutex<Vec<Value>>>,
}

fn sse(chunks: &[Value]) -> String {
    let mut out: String = chunks.iter().map(|c| format!("data: {c}\n\n")).collect();
    out.push_str("data: [DONE]\n\n");
    out
}

fn text_step(text: &str, prompt: i64, completion: i64) -> String {
    sse(&[
        json!({ "choices": [{ "index": 0, "delta": { "role": "assistant", "content": text } }] }),
        json!({ "choices": [{ "index": 0, "delta": {}, "finish_reason": "stop" }], "usage": { "prompt_tokens": prompt, "completion_tokens": completion } }),
    ])
}

fn tool_step(name: &str, args: Value) -> String {
    sse(&[
        json!({ "choices": [{ "index": 0, "delta": { "tool_calls": [{ "index": 0, "id": format!("call_{name}"), "type": "function", "function": { "name": name, "arguments": args.to_string() } }] } }] }),
        json!({ "choices": [{ "index": 0, "delta": {}, "finish_reason": "tool_calls" }], "usage": { "prompt_tokens": 100, "completion_tokens": 10 } }),
    ])
}

/// Текст сообщения: строка или массив частей с `text`.
fn text_of(message: &Value) -> String {
    match &message["content"] {
        Value::String(text) => text.clone(),
        Value::Array(parts) => parts.iter().filter_map(|part| part["text"].as_str()).collect::<Vec<_>>().join("\n"),
        _ => String::new(),
    }
}

async fn completions(State(model): State<Model>, body: Bytes) -> Response {
    let payload: Value = serde_json::from_slice(&body).unwrap_or_else(|_| json!({}));
    model.requests.lock().unwrap().push(payload.clone());
    let messages = payload["messages"].as_array().cloned().unwrap_or_default();
    let system = messages.first().and_then(|m| m["content"].as_str()).unwrap_or_default().to_string();
    let user_text = messages.iter().map(text_of).collect::<Vec<_>>().join(" ");
    let used_tool = messages.iter().any(|m| m["role"] == "tool");

    let text = if used_tool {
        text_step("готово", 10, 5)
    } else if system.contains("Ты — Альфа") {
        if user_text.contains("ДЛИННЫЙ_БРИФ") {
            tool_step("delegate", json!({ "bot": "Бета", "task": "д".repeat(6001) }))
        } else {
            tool_step("delegate", json!({ "bot": "Бета", "task": BRIEF }))
        }
    } else if user_text.contains(DELEGATION_TAIL) {
        // Делегат отвечает длинным отчётом: вызывающий должен получить сжатый итог.
        let report = (1..=40).map(|i| format!("строка {i}")).collect::<Vec<_>>().join("\n");
        text_step(&report, 123, 7)
    } else {
        text_step("ответ на старое сообщение", 11, 3)
    };
    Response::builder().header("content-type", "text/event-stream").body(Body::from(text)).unwrap()
}

async fn with_model(name: &str) -> (Crew, String, String, Arc<Mutex<Vec<Value>>>) {
    let model = Model::default();
    let requests = model.requests.clone();
    let router = Router::new().route("/v1/chat/completions", post(completions)).with_state(model);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
    let crew = Crew::start(name).await;
    crew.ok("PUT", "/api/settings", Some(json!({ "driver": "api", "model": "m", "baseUrl": format!("http://127.0.0.1:{port}/v1"), "apiKey": "test-key" }))).await;
    let alpha = crew.ok("POST", "/api/bots", Some(json!({ "name": "Альфа", "role": "старший", "approval": "auto" }))).await["bot"]["id"].as_str().unwrap().to_string();
    let beta = crew.ok("POST", "/api/bots", Some(json!({ "name": "Бета", "role": "младший", "approval": "auto" }))).await["bot"]["id"].as_str().unwrap().to_string();
    (crew, alpha, beta, requests)
}

fn system_of(request: &Value) -> String {
    request["messages"][0]["content"].as_str().unwrap_or_default().to_string()
}

fn is_beta(request: &Value) -> bool {
    system_of(request).contains("Ты — Бета")
}

fn says_delegation_tail(request: &Value) -> bool {
    request["messages"].as_array().is_some_and(|msgs| msgs.iter().any(|m| text_of(m).contains(DELEGATION_TAIL)))
}

#[tokio::test(flavor = "multi_thread")]
async fn a_too_long_brief_is_refused_and_the_target_is_not_started() {
    let (crew, alpha, _beta, requests) = with_model("handoff-long-brief").await;
    crew.listen().await;
    crew.say(&alpha, "ДЛИННЫЙ_БРИФ").await;
    crew.wait_event("конец Альфы", |e| e["type"] == "turn_end" && e["botId"] == alpha.as_str()).await;

    assert!(crew.find_event(|e| e["type"] == "delegate_end").is_none(), "делегат не должен запускаться");
    let requests = requests.lock().unwrap();
    assert!(!requests.iter().any(is_beta), "запрос к боту-получателю не должен уйти");

    let tool_error = requests
        .iter()
        .flat_map(|r| r["messages"].as_array().cloned().unwrap_or_default())
        .filter(|m| m["role"] == "tool")
        .map(|m| text_of(&m))
        .find(|text| text.contains(TOO_LONG));
    assert!(tool_error.is_some(), "модель должна увидеть отказ с текстом про 6000 знаков");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_delegated_turn_sees_the_brief_but_not_the_target_history() {
    let (crew, alpha, beta, requests) = with_model("handoff-fresh-context").await;
    crew.listen().await;

    // У получателя уже была своя переписка — делегат её видеть не должен.
    crew.say(&beta, OLD_MARK).await;
    crew.wait_event("конец Беты", |e| e["type"] == "turn_end" && e["botId"] == beta.as_str()).await;
    assert!(
        requests.lock().unwrap().iter().filter(|r| is_beta(r)).any(|r| r["messages"].as_array().is_some_and(|msgs| msgs.iter().any(|m| text_of(m).contains(OLD_MARK)))),
        "обычный ход Беты видит свою историю"
    );

    crew.say(&alpha, "передай задачу").await;
    crew.wait_event("конец Альфы", |e| e["type"] == "turn_end" && e["botId"] == alpha.as_str()).await;

    {
        let requests = requests.lock().unwrap();
        let delegated = requests.iter().find(|r| is_beta(r) && says_delegation_tail(r)).expect("делегированный запрос Беты");
        let messages = delegated["messages"].as_array().unwrap();
        assert_eq!(messages.len(), 2, "только системный промпт и бриф: {messages:?}");
        assert_eq!(messages[1]["role"], "user");
        assert!(text_of(&messages[1]).contains(BRIEF), "бриф дошёл целиком");
        assert!(!text_of(&messages[1]).contains(OLD_MARK), "прошлая переписка в бриф не попала");
    }

    // Делегация остаётся в треде получателя: человек видит, что ему поручали.
    let thread = crew.ok("GET", &format!("/api/bots/{beta}/thread?limit=20"), None).await;
    assert!(
        thread["messages"].as_array().is_some_and(|msgs| msgs
            .iter()
            .any(|m| m["role"] == "user" && m["text"].as_str().is_some_and(|text| text.contains(BRIEF)))),
        "бриф не записан в тред получателя: {thread}"
    );

    // Обычный ход той же Беты по-прежнему видит историю.
    crew.clear_events();
    crew.say(&beta, "продолжим").await;
    crew.wait_event("конец Беты после делегации", |e| e["type"] == "turn_end" && e["botId"] == beta.as_str()).await;
    let requests = requests.lock().unwrap();
    let last = requests.iter().rev().find(|r| is_beta(r)).expect("запрос Беты");
    assert!(
        last["messages"].as_array().is_some_and(|msgs| msgs.iter().any(|m| text_of(m).contains(OLD_MARK))),
        "обычный ход Беты должен видеть её прошлую переписку"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn the_result_contract_is_short_and_its_tokens_are_reported() {
    let (crew, alpha, _beta, requests) = with_model("handoff-result-contract").await;
    crew.listen().await;
    crew.say(&alpha, "передай задачу").await;
    let end = crew.wait_event("конец делегации", |e| e["type"] == "delegate_end").await;
    crew.wait_event("конец Альфы", |e| e["type"] == "turn_end" && e["botId"] == alpha.as_str()).await;

    // Бриф несёт требование к итогу, а вызывающий получает сжатые 20 строк.
    let requests = requests.lock().unwrap();
    let delegated = requests.iter().find(|r| is_beta(r)).expect("запрос Беты");
    assert!(says_delegation_tail(delegated), "хвост про 20 строк должен быть в брифе");
    let result = requests
        .iter()
        .flat_map(|r| r["messages"].as_array().cloned().unwrap_or_default())
        .filter(|m| m["role"] == "tool")
        .map(|m| text_of(&m))
        .find(|text| text.contains("(итог сокращён)"))
        .expect("итог с отметкой об обрезке");
    let lines: Vec<&str> = result.lines().collect();
    assert_eq!(lines.len(), 20, "20 строк вместе с отметкой: {result}");
    assert_eq!(lines[19], "(итог сокращён)");
    assert_eq!(lines[0], "строка 1");
    assert_eq!(lines[18], "строка 19");

    // 123 входа + 7 выхода одного шага делегата.
    assert_eq!(end["tokens"], 130, "делегация показывает потраченные токены: {end}");
}
