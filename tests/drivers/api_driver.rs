//! api-драйвер без внешних сервисов: фейковый OpenAI-совместимый эндпоинт стримит ответ с tool_call,
//! а мы смотрим, что цикл «текст → инструмент → финальный ответ» работает.

use crate::common;

use std::sync::{Arc, Mutex};

use axum::body::{Body, Bytes};
use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use axum::response::Response;
use axum::routing::post;
use axum::Router;
use common::Crew;
use serde_json::{json, Value};

#[derive(Clone, Default)]
struct RetryHeaders {
    calls: Arc<Mutex<Vec<(String, Value)>>>,
}

async fn retry_headers(State(fake): State<RetryHeaders>, headers: HeaderMap, body: Bytes) -> Response {
    let payload: Value = serde_json::from_slice(&body).unwrap();
    let user_agent = headers.get("user-agent").unwrap().to_str().unwrap().to_owned();
    let retry = payload.get("stream_options").is_some();
    fake.calls.lock().unwrap().push((user_agent, payload));
    if retry {
        return Response::builder().status(StatusCode::BAD_REQUEST).body(Body::from("unsupported stream_options")).unwrap();
    }
    Response::builder().header("content-type", "application/json")
        .body(Body::from(json!({ "choices": [{ "message": { "content": "ok" } }] }).to_string())).unwrap()
}

#[tokio::test]
async fn user_agent_contains_package_version_on_initial_request_and_retry() {
    let fake = RetryHeaders::default();
    let app = Router::new().route("/v1/chat/completions", post(retry_headers)).with_state(fake.clone());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });

    let crew = Crew::start("api-user-agent").await;
    crew.ok("PUT", "/api/settings", Some(json!({
        "driver": "api", "model": "fake-model",
        "baseUrl": format!("http://127.0.0.1:{port}/v1"), "apiKey": "test-key"
    }))).await;
    crew.listen().await;
    let state = crew.ok("GET", "/api/state", None).await;
    let bot = state["bots"][0]["id"].as_str().unwrap();
    crew.say(bot, "hello").await;
    crew.wait_event("turn end", |e| e["type"] == "turn_end").await;

    let calls = fake.calls.lock().unwrap();
    assert_eq!(calls.len(), 2);
    assert!(calls[0].1.get("stream_options").is_some());
    assert!(calls[1].1.get("stream_options").is_none());
    for (user_agent, _) in calls.iter() {
        assert_eq!(user_agent, concat!("deloist/", env!("CARGO_PKG_VERSION")));
    }
}

#[derive(Clone, Default)]
struct Fake {
    calls: Arc<Mutex<Vec<Value>>>,
}

fn sse(chunks: &[Value]) -> String {
    let mut out: String = chunks.iter().map(|c| format!("data: {c}\n\n")).collect();
    out.push_str("data: [DONE]\n\n");
    out
}

async fn completions(State(fake): State<Fake>, body: Bytes) -> Response {
    let payload: Value = serde_json::from_slice(&body).unwrap_or_else(|_| json!({}));
    let has_tool_result = payload["messages"].as_array().is_some_and(|m| m.iter().any(|x| x["role"] == "tool"));
    fake.calls.lock().unwrap().push(payload);

    let text = if has_tool_result {
        sse(&[
            json!({ "choices": [{ "index": 0, "delta": { "role": "assistant", "content": "Готово: " } }] }),
            json!({ "choices": [{ "index": 0, "delta": { "content": "воркспейс пуст, файлов нет." } }] }),
            json!({ "choices": [{ "index": 0, "delta": {}, "finish_reason": "stop" }] }),
        ])
    } else {
        sse(&[
            json!({ "choices": [{ "index": 0, "delta": { "role": "assistant", "content": "Сейчас посмотрю воркспейс." } }] }),
            json!({ "choices": [{ "index": 0, "delta": { "tool_calls": [{
                "index": 0, "id": "call_fake_1", "type": "function",
                "function": { "name": "list_files", "arguments": "{\"path\":\"\"}" },
            }] } }] }),
            json!({ "choices": [{ "index": 0, "delta": {}, "finish_reason": "tool_calls" }] }),
        ])
    };
    Response::builder().header("content-type", "text/event-stream").body(Body::from(text)).unwrap()
}

#[tokio::test]
async fn streamed_tool_call_runs_and_the_final_answer_is_saved() {
    let fake = Fake::default();
    let app = Router::new().route("/v1/chat/completions", post(completions)).with_state(fake.clone());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let fake_port = listener.local_addr().unwrap().port();
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });

    let crew = Crew::start("api").await;
    crew.ok(
        "PUT",
        "/api/settings",
        Some(json!({
            "driver": "api",
            "model": "fake-model",
            "baseUrl": format!("http://127.0.0.1:{fake_port}/v1"),
            "apiKey": "test-key",
        })),
    )
    .await;
    assert_eq!(crew.ok("GET", "/api/state", None).await["settings"]["hasKey"], true);
    crew.listen().await;

    let state = crew.ok("GET", "/api/state", None).await;
    let bot = state["bots"][0]["id"].as_str().unwrap().to_string();
    crew.say(&bot, "что у тебя в файлах?").await;
    crew.wait_event("конец хода", |e| e["type"] == "turn_end").await;

    let calls = fake.calls.lock().unwrap().clone();
    assert!(calls.len() >= 2, "ожидал два обращения к модели, было {}", calls.len());
    let last = calls.last().unwrap();
    assert_eq!(last["model"], "fake-model");
    assert_eq!(last["stream"], true);
    assert!(last["tools"].as_array().is_some_and(|t| t.len() >= 5), "инструменты не переданы модели");
    let tool_message = last["messages"].as_array().unwrap().iter().find(|m| m["role"] == "tool").unwrap().clone();
    assert_eq!(tool_message["tool_call_id"], "call_fake_1");

    let tool = crew.find_event(|e| e["type"] == "tool_update").expect("инструмент не выполнялся");
    assert_eq!(tool["status"], "ok");

    let thread = crew.ok("GET", &format!("/api/bots/{bot}/thread?limit=20"), None).await;
    let assistant: Vec<&Value> = thread["messages"].as_array().unwrap().iter().filter(|m| m["role"] == "assistant").collect();
    assert!(assistant.iter().any(|m| m["toolCalls"].as_array().is_some_and(|c| !c.is_empty())), "нет записи с инструментом");
    assert!(assistant.iter().any(|m| m["text"].as_str().unwrap_or_default().contains("Готово")), "нет финального текста");
}

#[derive(Clone, Default)]
struct Plain {
    calls: Arc<Mutex<Vec<Value>>>,
}

async fn plain(State(fake): State<Plain>, body: Bytes) -> Response {
    let payload: Value = serde_json::from_slice(&body).unwrap_or_else(|_| json!({}));
    fake.calls.lock().unwrap().push(payload);
    let text = sse(&[
        json!({ "choices": [{ "index": 0, "delta": { "content": "ответ" } }] }),
        json!({ "choices": [{ "index": 0, "delta": {}, "finish_reason": "stop" }] }),
        // Счётчик с попаданием в кэш: так его присылает DeepSeek.
        json!({ "choices": [], "usage": { "prompt_tokens": 100, "completion_tokens": 10, "prompt_cache_hit_tokens": 80 } }),
    ]);
    Response::builder().header("content-type", "text/event-stream").body(Body::from(text)).unwrap()
}

async fn start_plain(name: &str, env: &[(&str, &str)]) -> (Crew, Plain) {
    let fake = Plain::default();
    let app = Router::new().route("/v1/chat/completions", post(plain)).with_state(fake.clone());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });

    let crew = Crew::start_with_env(name, env).await;
    crew.ok(
        "PUT",
        "/api/settings",
        Some(json!({
            "driver": "api",
            "model": "fake-model",
            "baseUrl": format!("http://127.0.0.1:{port}/v1"),
            "apiKey": "test-key",
        })),
    )
    .await;
    crew.listen().await;
    (crew, fake)
}

// Главное свойство кэша: начало запроса не меняется от хода к ходу.
// Часы раньше стояли в системном промпте — и каждый ход был гарантированным промахом.
#[tokio::test]
async fn the_prompt_prefix_and_cache_are_stable_between_turns() {
    let (crew, fake) = start_plain("api-prefix", &[]).await;
    let state = crew.ok("GET", "/api/state", None).await;
    let bot = state["bots"][0]["id"].as_str().unwrap().to_string();

    crew.say(&bot, "первый вопрос").await;
    let end = crew.wait_event("конец первого хода", |e| e["type"] == "turn_end").await;
    assert_eq!(end["usage"]["cached"], 80, "кэш из счётчика провайдера: {end}");
    assert_eq!(end["usage"]["prompt"], 100);
    assert_eq!(end["steps"], 1);

    crew.clear_events();
    crew.say(&bot, "второй вопрос").await;
    crew.wait_event("конец второго хода", |e| e["type"] == "turn_end").await;

    let calls = fake.calls.lock().unwrap().clone();
    assert_eq!(calls.len(), 2, "по одному запросу на ход");
    let first = calls[0]["messages"][0]["content"].as_str().unwrap().to_string();
    let second = calls[1]["messages"][0]["content"].as_str().unwrap().to_string();
    assert_eq!(first, second, "системный промпт обязан совпадать байт в байт");
    assert!(!first.contains("Сейчас"), "часы не должны попадать в системный промпт");

    // Второй запрос — это первый плюс новый ход, а не пересобранная история.
    let before = calls[0]["messages"].as_array().unwrap().len();
    let after = calls[1]["messages"].as_array().unwrap().len();
    assert_eq!(after, before + 2, "вырос ровно на ответ бота и новый вопрос");
    assert_eq!(calls[0]["messages"].as_array().unwrap()[..], calls[1]["messages"].as_array().unwrap()[..before], "начало запроса совпадает целиком");
    assert_eq!(calls[0]["tools"], calls[1]["tools"], "схемы инструментов не меняются");
    let opening = calls[0]["messages"][1]["content"].as_str().unwrap();
    assert!(opening.starts_with("Время подготовки хода") && opening.ends_with("первый вопрос"), "часы склеены с вопросом: {opening}");

    let thread = crew.ok("GET", &format!("/api/bots/{bot}/thread?limit=20"), None).await;
    let replied = thread["messages"].as_array().unwrap().iter().any(|m| m["usage"]["cached"] == 80);
    assert!(replied, "в треде нет ответа с расходом: {thread}");
}

// Усилие рассуждения уходит только тем моделям, которые его принимают.
#[tokio::test]
async fn reasoning_effort_reaches_an_openai_compatible_server() {
    let (crew, fake) = start_plain("api-effort", &[]).await;
    let state = crew.ok("GET", "/api/state", None).await;
    let bot = state["bots"][0]["id"].as_str().unwrap().to_string();
    crew.ok(
        "PATCH",
        &format!("/api/bots/{bot}"),
        Some(json!({ "model": "gpt-6.1-terra", "effort": "high" })),
    )
    .await;

    crew.say(&bot, "подумай как следует").await;
    crew.wait_event("конец хода", |e| e["type"] == "turn_end").await;

    let calls = turn_calls(&fake);
    assert_eq!(calls.len(), 1, "один запрос на ход");
    assert_eq!(calls[0]["reasoning_effort"], "high", "усилие не дошло до сервера: {:?}", calls[0]);
}

// Часы приезжают сообщением в конце истории: модель видит время, префикс не меняется.
#[tokio::test]
async fn the_clock_arrives_as_a_message_not_as_a_prompt_rewrite() {
    let (crew, fake) = start_plain("api-clock", &[("CREW_TEST_TIME_CONTEXT_MS", "0")]).await;
    let state = crew.ok("GET", "/api/state", None).await;
    let bot = state["bots"][0]["id"].as_str().unwrap().to_string();

    crew.say(&bot, "первый вопрос").await;
    crew.wait_event("конец первого хода", |e| e["type"] == "turn_end").await;
    crew.clear_events();
    crew.say(&bot, "второй вопрос").await;
    crew.wait_event("конец второго хода", |e| e["type"] == "turn_end").await;

    let calls = fake.calls.lock().unwrap().clone();
    let second = calls[1]["messages"].as_array().unwrap().clone();
    // Системный промпт объясняет эту строку, поэтому считаем только сообщения человека.
    let clocks = second.iter().filter(|m| m["role"] == "user" && m["content"].as_str().is_some_and(|t| t.contains("Время подготовки хода"))).count();
    assert_eq!(clocks, 2, "часы обоих ходов остаются в истории: {second:?}");
    assert_eq!(calls[0]["messages"][0]["content"], calls[1]["messages"][0]["content"], "промпт не переписан");
}

/// Запросы ходов без служебных: сводка истории — отдельный запрос со своим промптом.
fn turn_calls(fake: &Plain) -> Vec<Value> {
    fake.calls
        .lock()
        .unwrap()
        .iter()
        .filter(|c| {
            let system = c["messages"][0]["content"].as_str().unwrap_or_default();
            // Сводку и извлечение памяти программа заказывает сама: это не ходы.
            !system.contains("сворачиваешь старую часть") && !system.contains("извлекатель памяти")
        })
        .cloned()
        .collect()
}

/// Сколько раз начало запроса менялось: следующий запрос обязан продолжать предыдущий.
fn prefix_breaks(calls: &[Value]) -> usize {
    calls
        .windows(2)
        .filter(|pair| {
            let (previous, next) = (pair[0]["messages"].as_array().unwrap(), pair[1]["messages"].as_array().unwrap());
            next.len() < previous.len() || next[..previous.len()] != previous[..]
        })
        .count()
}

// Сдвиг на запись за ход ломал бы начало запроса на каждом ходе после переполнения.
#[tokio::test]
async fn a_full_history_moves_in_rare_jumps_in_a_personal_thread() {
    let (crew, fake) = start_plain("api-window", &[]).await;
    crew.ok("PUT", "/api/settings", Some(json!({ "historyLimit": 10 }))).await;
    let state = crew.ok("GET", "/api/state", None).await;
    let bot = state["bots"][0]["id"].as_str().unwrap().to_string();

    for n in 0..25 {
        crew.clear_events();
        crew.say(&bot, &format!("вопрос {n}")).await;
        crew.wait_event("конец хода", |e| e["type"] == "turn_end").await;
    }
    let calls = turn_calls(&fake);
    assert_eq!(calls.len(), 25);
    assert!(fake.calls.lock().unwrap().len() > calls.len(), "при сдвиге окна выпавшее свернули в сводку");
    let breaks = prefix_breaks(&calls);
    assert!((1..=3).contains(&breaks), "окно должно двигаться редкими рывками, а сдвигов {breaks}");
    let clock_seen = calls.iter().all(|c| c["messages"].as_array().unwrap().iter().any(|m| m["role"] == "user" && m["content"].as_str().is_some_and(|t| t.contains("Время подготовки хода"))));
    assert!(clock_seen, "после сдвига окна модель всё равно знает время");
}

async fn room_with_first_bot(crew: &Crew) -> (String, String) {
    let state = crew.ok("GET", "/api/state", None).await;
    let bot = state["bots"][0]["id"].as_str().unwrap().to_string();
    let room = crew
        .ok("POST", "/api/rooms", Some(json!({ "name": "Совет", "botIds": [bot], "lead": bot, "mode": "lead" })))
        .await;
    (room["room"]["id"].as_str().unwrap().to_string(), bot)
}

async fn say_in_room(crew: &Crew, room: &str, text: &str) {
    crew.clear_events();
    crew.ok("POST", &format!("/api/rooms/{room}/messages"), Some(json!({ "text": text }))).await;
    crew.wait_event("конец хода группы", |e| e["type"] == "room_turn_end" && e["roomId"] == room).await;
}

// Живой ход пишет квитанцию и расход по модели: и в личном чате, и в группе.
#[tokio::test]
async fn replies_leave_receipts_and_model_usage() {
    let (crew, _fake) = start_plain("api-receipts", &[]).await;
    let (room, bot) = room_with_first_bot(&crew).await;
    crew.clear_events();
    crew.say(&bot, "привет").await;
    crew.wait_event("конец хода", |e| e["type"] == "turn_end" && e["botId"] == bot.as_str()).await;
    say_in_room(&crew, &room, "а в группе?").await;

    let month = chrono::Local::now().format("%Y-%m").to_string();
    let usage = crew.ok("GET", &format!("/api/usage?month={month}"), None).await;
    assert_eq!(usage["replies"], 2, "{usage}");
    let places: Vec<&str> = usage["receipts"].as_array().unwrap().iter().filter_map(|r| r["where"].as_str()).collect();
    assert_eq!(places, ["Совет", "dm"], "{usage}");
    assert!(usage["receipts"].as_array().unwrap().iter().all(|r| r["outcome"] == "ok" && r["model"] == "fake-model" && r["tokens"] == 110));
    assert_eq!(usage["models"][0]["id"], "fake-model");
    assert_eq!(usage["models"][0]["tokens"], 220);
    assert_eq!(usage["totalTokens"], 220);
    let report = crew.ok("GET", "/api/report", None).await;
    assert_eq!(report["okPercent"], 100);
}

// В группе последним модель должна видеть вопрос человека, а не часы.
#[tokio::test]
async fn in_a_group_the_question_comes_after_the_clock_and_usage_is_counted() {
    let (crew, fake) = start_plain("api-room-clock", &[]).await;
    let (room, _) = room_with_first_bot(&crew).await;
    say_in_room(&crew, &room, "какой план?").await;

    let calls = fake.calls.lock().unwrap().clone();
    let messages = calls[0]["messages"].as_array().unwrap();
    let last = messages.last().unwrap()["content"].as_str().unwrap();
    assert!(last.contains("Время подготовки хода"), "модель не знает времени: {last}");
    assert!(last.ends_with("какой план?"), "последним должен идти вопрос: {last}");
    assert!(!messages[0]["content"].as_str().unwrap().contains("Сейчас"), "часы не должны попадать в системный промпт");

    let journal = crew.ok("GET", "/api/journal", None).await;
    assert_eq!(journal["usage"]["tokens"], 110, "расход группы идёт в дневной счётчик: {journal}");
    assert_eq!(journal["usage"]["cached"], 80);

    let thread = crew.ok("GET", &format!("/api/rooms/{room}/thread?limit=50"), None).await;
    assert!(thread["messages"].as_array().unwrap().iter().all(|m| m["role"] != "time"), "часы не показываем человеку");
}

#[tokio::test]
async fn a_full_history_moves_in_rare_jumps_in_a_group() {
    let (crew, fake) = start_plain("api-room-window", &[]).await;
    crew.ok("PUT", "/api/settings", Some(json!({ "historyLimit": 10 }))).await;
    let (room, _) = room_with_first_bot(&crew).await;
    for n in 0..25 {
        say_in_room(&crew, &room, &format!("вопрос {n}")).await;
    }
    let calls = turn_calls(&fake);
    assert_eq!(calls.len(), 25);
    assert!(fake.calls.lock().unwrap().len() > calls.len(), "при сдвиге окна выпавшее свернули в сводку");
    let breaks = prefix_breaks(&calls);
    assert!((1..=3).contains(&breaks), "окно должно двигаться редкими рывками, а сдвигов {breaks}");

    let thread = crew.ok("GET", &format!("/api/rooms/{room}/thread?limit=100"), None).await;
    let messages = thread["messages"].as_array().unwrap();
    assert!(messages.iter().all(|m| m["role"] != "anchor"), "якорь не показываем человеку");
    assert_eq!(messages.iter().filter(|m| m["role"] == "user").count(), 25, "человек видит весь тред");
}
