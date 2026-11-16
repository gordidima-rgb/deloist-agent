//! MCP по HTTP: настоящий exe ходит в поднятую рядом заглушку MCP-сервера
//! (Streamable HTTP: JSON и SSE), а инструменты видит бот.

use crate::common;

use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use axum::body::{Body, Bytes};
use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::post;
use axum::Router;
use common::Crew;
use serde_json::{json, Value};

// ---------------------------------------------------------------------------
// Заглушка MCP-сервера
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, PartialEq, Eq)]
enum Wire {
    /// Ответ обычным JSON.
    Json,
    /// Ответ потоком `text/event-stream`, как у настоящих HTTP MCP-серверов.
    /// Внутри есть комментарий, ping и чужое уведомление: клиент обязан их пропустить.
    Sse,
}

#[derive(Default)]
struct FakeState {
    session: String,
    init_count: usize,
    list_count: usize,
    call_count: usize,
    initialized: usize,
    calls: Vec<Value>,
    unauthorized: bool,
    /// Задержка перед ответом: проверяем таймаут, а не вечное ожидание.
    delay_ms: u64,
}

#[derive(Clone)]
struct FakeMcp {
    inner: Arc<Mutex<FakeState>>,
    wire: Wire,
}

impl FakeMcp {
    fn reply(&self, message: Value, session: Option<&str>) -> Response {
        let (body, kind) = match self.wire {
            Wire::Json => (message.to_string(), "application/json"),
            Wire::Sse => {
                let mut text = String::new();
                text.push_str(": keep-alive\n\n");
                text.push_str("event: ping\ndata: {\"jsonrpc\":\"2.0\",\"method\":\"ping\"}\n\n");
                text.push_str("event: message\ndata: {\"jsonrpc\":\"2.0\",\"method\":\"notifications/message\",\"params\":{}}\n\n");
                text.push_str(&format!("event: message\ndata: {message}\n\n"));
                (text, "text/event-stream")
            }
        };
        let mut builder = Response::builder().status(StatusCode::OK).header("content-type", kind);
        if let Some(session) = session {
            builder = builder.header("mcp-session-id", session);
        }
        builder.body(Body::from(body)).unwrap()
    }
}

fn tool_list() -> Value {
    json!({
        "tools": [{
            "name": "echo",
            "description": "Повторяет текст",
            "inputSchema": { "type": "object", "properties": { "text": { "type": "string" } } },
        }],
    })
}

async fn mcp(State(fake): State<FakeMcp>, headers: HeaderMap, body: Bytes) -> Response {
    {
        let state = fake.inner.lock().unwrap();
        if state.unauthorized {
            return (StatusCode::UNAUTHORIZED, "нужен ключ").into_response();
        }
    }
    let delay = fake.inner.lock().unwrap().delay_ms;
    if delay > 0 {
        tokio::time::sleep(Duration::from_millis(delay)).await;
    }
    let message: Value = match serde_json::from_slice(&body) {
        Ok(value) => value,
        Err(_) => return (StatusCode::BAD_REQUEST, "тело не JSON").into_response(),
    };
    let method = message["method"].as_str().unwrap_or_default().to_string();
    let id = message.get("id").cloned();

    if method == "initialize" {
        let (session, reply) = {
            let mut state = fake.inner.lock().unwrap();
            state.init_count += 1;
            state.session = format!("session-{}", state.init_count);
            let session = state.session.clone();
            let reply = json!({
                "jsonrpc": "2.0",
                "id": id,
                "result": {
                    "protocolVersion": "2025-03-26",
                    "capabilities": { "tools": {} },
                    "serverInfo": { "name": "fake-http", "version": "1" },
                },
            });
            (session, reply)
        };
        return fake.reply(reply, Some(&session));
    }

    // Дальше нужна живая сессия: как настоящий сервер, отвечаем 404 без неё.
    let provided = headers
        .get("mcp-session-id")
        .and_then(|value| value.to_str().ok())
        .unwrap_or_default()
        .to_string();
    let valid = {
        let state = fake.inner.lock().unwrap();
        !state.session.is_empty() && state.session == provided
    };
    if !valid {
        return (StatusCode::NOT_FOUND, "unknown session: pass Mcp-Session-Id").into_response();
    }

    match method.as_str() {
        "notifications/initialized" => {
            fake.inner.lock().unwrap().initialized += 1;
            StatusCode::ACCEPTED.into_response()
        }
        "tools/list" => {
            fake.inner.lock().unwrap().list_count += 1;
            fake.reply(json!({ "jsonrpc": "2.0", "id": id, "result": tool_list() }), None)
        }
        "tools/call" => {
            let arguments = message["params"]["arguments"].clone();
            let text = arguments["text"].as_str().unwrap_or_default();
            {
                let mut state = fake.inner.lock().unwrap();
                state.call_count += 1;
                state.calls.push(message.clone());
            }
            fake.reply(
                json!({
                    "jsonrpc": "2.0",
                    "id": id,
                    "result": { "content": [{ "type": "text", "text": format!("echo: {text}") }] },
                }),
                None,
            )
        }
        _ => fake.reply(
            json!({ "jsonrpc": "2.0", "id": id, "error": { "code": -32601, "message": "нет метода" } }),
            None,
        ),
    }
}

/// Сервер «забыл» сессию: как после перезапуска.
async fn expire(State(fake): State<FakeMcp>) -> Response {
    fake.inner.lock().unwrap().session = "expired".to_string();
    StatusCode::OK.into_response()
}

/// С этого момента сервер требует ключ.
async fn require_key(State(fake): State<FakeMcp>) -> Response {
    fake.inner.lock().unwrap().unauthorized = true;
    StatusCode::OK.into_response()
}

/// Поднять заглушку и вернуть её адрес `.../mcp`.
async fn start_fake(wire: Wire) -> (FakeMcp, String) {
    let fake = FakeMcp { inner: Arc::new(Mutex::new(FakeState::default())), wire };
    let app = Router::new()
        .route("/mcp", post(mcp))
        .route("/expire", post(expire))
        .route("/require-key", post(require_key))
        .with_state(fake.clone());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    (fake, format!("http://127.0.0.1:{port}/mcp"))
}

fn base_of(url: &str) -> String {
    url.strip_suffix("/mcp").unwrap_or(url).to_string()
}

async fn add_server(crew: &Crew, body: Value) -> String {
    let made = crew.ok("POST", "/api/mcp", Some(body)).await;
    made["server"]["id"].as_str().unwrap().to_string()
}

fn counts(fake: &FakeMcp) -> (usize, usize, usize, usize) {
    let state = fake.inner.lock().unwrap();
    (state.init_count, state.list_count, state.call_count, state.initialized)
}

// ---------------------------------------------------------------------------
// Основной сценарий
// ---------------------------------------------------------------------------

#[tokio::test]
async fn an_http_mcp_server_shows_its_tools_and_reuses_the_session() {
    let (fake, url) = start_fake(Wire::Json).await;
    let crew = Crew::start("mcp-http").await;

    let made = crew.ok("POST", "/api/mcp", Some(json!({ "name": "HTTP-заглушка", "url": url }))).await;
    let id = made["server"]["id"].as_str().unwrap().to_string();
    assert_eq!(made["server"]["url"].as_str(), Some(url.as_str()), "{made}");
    assert_eq!(made["server"]["command"], "", "у HTTP-сервера пустая команда: {made}");

    let state = crew.ok("GET", "/api/state", None).await;
    let listed: Vec<&Value> = state["mcp"].as_array().unwrap().iter().filter(|s| s["id"] == id.as_str()).collect();
    assert_eq!(listed.len(), 1, "сервер не попал в состояние");
    assert_eq!(listed[0]["url"].as_str(), Some(url.as_str()));

    let checked = crew.ok("POST", &format!("/api/mcp/{id}/check"), None).await;
    assert_eq!(checked["tools"].as_array().unwrap(), &vec![json!("echo")], "{checked}");

    // Второй раз сессия переиспользуется: initialize один, tools/list два.
    let checked = crew.ok("POST", &format!("/api/mcp/{id}/check"), None).await;
    assert_eq!(checked["tools"][0], "echo");
    let (init, list, _, initialized) = counts(&fake);
    assert_eq!(init, 1, "сессия не должна открываться заново");
    assert_eq!(list, 2, "каждая проверка спрашивает инструменты заново");
    assert_eq!(initialized, 1, "notifications/initialized уходит один раз на сессию");
}

#[tokio::test]
async fn an_expired_http_session_is_renewed_once() {
    let (fake, url) = start_fake(Wire::Json).await;
    let crew = Crew::start("mcp-http-expire").await;
    let id = add_server(&crew, json!({ "name": "Забывчивый", "url": url })).await;
    crew.ok("POST", &format!("/api/mcp/{id}/check"), None).await;

    let res = crew.client.post(format!("{}/expire", base_of(&url))).send().await.unwrap();
    assert_eq!(res.status().as_u16(), 200);

    let checked = crew.ok("POST", &format!("/api/mcp/{id}/check"), None).await;
    assert_eq!(checked["tools"][0], "echo", "после 404 клиент должен переинициализироваться: {checked}");
    let (init, _, _, _) = counts(&fake);
    assert_eq!(init, 2, "переинициализация ровно одна, а не цикл");
}

#[tokio::test]
async fn an_sse_http_mcp_server_is_understood() {
    let (fake, url) = start_fake(Wire::Sse).await;
    let crew = Crew::start("mcp-http-sse").await;
    let id = add_server(&crew, json!({ "name": "Потоковый", "url": url })).await;

    let checked = crew.ok("POST", &format!("/api/mcp/{id}/check"), None).await;
    assert_eq!(checked["tools"][0], "echo", "SSE-ответ не разобран: {checked}");
    let (init, list, _, initialized) = counts(&fake);
    assert_eq!(init, 1);
    assert_eq!(list, 1);
    assert_eq!(initialized, 1);
}

#[tokio::test]
async fn a_broken_http_mcp_server_gives_a_clear_error() {
    // Порт свободен: сервера на нём нет.
    let port = common::free_port();
    let url = format!("http://127.0.0.1:{port}/mcp");
    let crew = Crew::start("mcp-http-broken").await;
    let id = add_server(&crew, json!({ "name": "Мёртвый", "url": url })).await;

    let (status, data) = crew.send("POST", &format!("/api/mcp/{id}/check"), None).await;
    assert_eq!(status, 502, "{data}");
    let error = data["error"].as_str().unwrap_or_default();
    assert!(error.contains("MCP"), "ошибка должна называть MCP: {error}");
    assert!(error.contains("подключиться"), "ошибка должна говорить о подключении: {error}");
    assert!(!error.contains("127.0.0.1"), "адрес с ключом в ошибку не попадает: {error}");
}

#[tokio::test]
async fn a_401_http_mcp_server_asks_for_a_key() {
    let (fake, url) = start_fake(Wire::Json).await;
    fake.inner.lock().unwrap().unauthorized = true;
    let crew = Crew::start("mcp-http-key").await;
    let id = add_server(&crew, json!({ "name": "С ключом", "url": url })).await;

    let (status, data) = crew.send("POST", &format!("/api/mcp/{id}/check"), None).await;
    assert_eq!(status, 502, "{data}");
    let error = data["error"].as_str().unwrap_or_default();
    assert!(error.contains("требует ключ"), "непонятная ошибка 401: {error}");
}

#[tokio::test]
async fn a_silent_http_mcp_server_times_out_and_does_not_hang_the_turn() {
    let (fake, url) = start_fake(Wire::Json).await;
    fake.inner.lock().unwrap().delay_ms = 10_000;
    let crew = Crew::start_with_env("mcp-http-silent", &[("CREW_TEST_MCP_TIMEOUT_MS", "300")]).await;
    let id = add_server(&crew, json!({ "name": "Молчун", "url": url })).await;

    let started = Instant::now();
    let (status, data) = crew.send("POST", &format!("/api/mcp/{id}/check"), None).await;
    assert_eq!(status, 502, "{data}");
    assert!(data["error"].as_str().unwrap_or_default().contains("не ответил"), "{data}");
    assert!(started.elapsed() < Duration::from_secs(5), "таймаут не сработал");

    // Ход с молчащим сервером не виснет: инструментов нет, бот отвечает сам.
    crew.listen().await;
    let state = crew.ok("GET", "/api/state", None).await;
    let bot = state["bots"][0]["id"].as_str().unwrap().to_string();
    let turn_started = Instant::now();
    crew.say(&bot, "привет").await;
    crew.wait_event("конец хода", |e| e["type"] == "turn_end").await;
    assert!(turn_started.elapsed() < Duration::from_secs(10), "ход завис на молчащем MCP-сервере");
}

// ---------------------------------------------------------------------------
// Валидация и обратная совместимость
// ---------------------------------------------------------------------------

#[tokio::test]
async fn exactly_one_transport_is_required() {
    let crew = Crew::start("mcp-http-validate").await;

    let both = crew
        .send("POST", "/api/mcp", Some(json!({ "name": "оба", "command": "npx", "url": "http://127.0.0.1:9/mcp" })))
        .await;
    assert_eq!(both.0, 400, "{both:?}");
    let neither = crew.send("POST", "/api/mcp", Some(json!({ "name": "пусто" }))).await;
    assert_eq!(neither.0, 400, "{neither:?}");
    let bad = crew.send("POST", "/api/mcp", Some(json!({ "name": "ftp", "url": "ftp://example.com/mcp" }))).await;
    assert_eq!(bad.0, 400, "{bad:?}");
    assert!(bad.1["error"].as_str().unwrap_or_default().contains("http://"), "{bad:?}");

    // Старый PATCH с одним полем не ломается: команда и аргументы на месте.
    let made = crew.ok("POST", "/api/mcp", Some(json!({ "name": "stdio", "command": "node", "args": ["server.mjs"] }))).await;
    let id = made["server"]["id"].as_str().unwrap().to_string();
    let patched = crew.ok("PATCH", &format!("/api/mcp/{id}"), Some(json!({ "name": "Новое имя" }))).await;
    assert_eq!(patched["server"]["name"], "Новое имя");
    assert_eq!(patched["server"]["command"], "node");
    assert_eq!(patched["server"]["args"][0], "server.mjs");
    assert_eq!(patched["server"]["url"], "");

    // Смена транспорта одним полем: адрес убирает команду…
    let patched = crew.ok("PATCH", &format!("/api/mcp/{id}"), Some(json!({ "url": "http://127.0.0.1:9/mcp" }))).await;
    assert_eq!(patched["server"]["url"], "http://127.0.0.1:9/mcp");
    assert_eq!(patched["server"]["command"], "");
    // …а команда убирает адрес.
    let patched = crew.ok("PATCH", &format!("/api/mcp/{id}"), Some(json!({ "command": "node" }))).await;
    assert_eq!(patched["server"]["command"], "node");
    assert_eq!(patched["server"]["url"], "");

    // Оба сразу в PATCH — тоже ошибка, состояние не меняется.
    let both = crew
        .send("PATCH", &format!("/api/mcp/{id}"), Some(json!({ "url": "http://127.0.0.1:9/mcp", "command": "node" })))
        .await;
    assert_eq!(both.0, 400, "{both:?}");
}

#[tokio::test]
async fn an_old_state_file_without_url_still_loads() {
    let dir = common::temp_data_dir("mcp-old-state");
    std::fs::create_dir_all(&dir).unwrap();
    // Так выглядел state.json до появления HTTP: поля `url` ещё нет.
    let saved = json!({
        "version": 1,
        "mcp": [{
            "id": "mcp_old",
            "name": "Старый stdio",
            "command": "node",
            "args": ["server.mjs"],
            "enabled": true,
            "autoApprove": false,
            "createdAt": 1
        }]
    });
    std::fs::write(dir.join("state.json"), saved.to_string()).unwrap();

    let crew = Crew::start_at(dir, &[]).await;
    let state = crew.ok("GET", "/api/state", None).await;
    let servers = state["mcp"].as_array().unwrap();
    assert_eq!(servers.len(), 1, "старый сервер потерялся: {state}");
    assert_eq!(servers[0]["name"], "Старый stdio");
    assert_eq!(servers[0]["command"], "node");
    assert_eq!(servers[0]["url"], "", "у старой записи адрес пустой: {state}");
}

// ---------------------------------------------------------------------------
// Инструмент у бота: вызов возвращает эхо
// ---------------------------------------------------------------------------

#[derive(Clone, Default)]
struct FakeModel {
    calls: Arc<Mutex<Vec<Value>>>,
    tool_name: Arc<Mutex<String>>,
}

fn sse(chunks: &[Value]) -> String {
    let mut out: String = chunks.iter().map(|chunk| format!("data: {chunk}\n\n")).collect();
    out.push_str("data: [DONE]\n\n");
    out
}

async fn completions(State(model): State<FakeModel>, body: Bytes) -> Response {
    let payload: Value = serde_json::from_slice(&body).unwrap_or_else(|_| json!({}));
    let has_tool_result = payload["messages"].as_array().is_some_and(|list| list.iter().any(|m| m["role"] == "tool"));
    model.calls.lock().unwrap().push(payload);
    let text = if has_tool_result {
        sse(&[
            json!({ "choices": [{ "index": 0, "delta": { "role": "assistant", "content": "Инструмент ответил." } }] }),
            json!({ "choices": [{ "index": 0, "delta": {}, "finish_reason": "stop" }] }),
        ])
    } else {
        let name = model.tool_name.lock().unwrap().clone();
        sse(&[
            json!({ "choices": [{ "index": 0, "delta": { "role": "assistant", "content": "Проверю эхо." } }] }),
            json!({ "choices": [{ "index": 0, "delta": { "tool_calls": [{
                "index": 0, "id": "call_echo_1", "type": "function",
                "function": { "name": name, "arguments": "{\"text\":\"привет\"}" },
            }] } }] }),
            json!({ "choices": [{ "index": 0, "delta": {}, "finish_reason": "tool_calls" }] }),
        ])
    };
    Response::builder().header("content-type", "text/event-stream").body(Body::from(text)).unwrap()
}

#[tokio::test]
async fn an_http_tool_appears_for_the_bot_and_returns_echo() {
    let (fake, url) = start_fake(Wire::Json).await;
    let model = FakeModel::default();
    let app = Router::new().route("/v1/chat/completions", post(completions)).with_state(model.clone());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let model_port = listener.local_addr().unwrap().port();
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });

    let crew = Crew::start("mcp-http-call").await;
    crew.ok(
        "PUT",
        "/api/settings",
        Some(json!({
            "driver": "api",
            "model": "fake-model",
            "baseUrl": format!("http://127.0.0.1:{model_port}/v1"),
            "apiKey": "test-key",
        })),
    )
    .await;
    let id = add_server(&crew, json!({ "name": "Эхо", "url": url, "autoApprove": true })).await;
    let full = format!("mcp__{id}__echo");
    *model.tool_name.lock().unwrap() = full.clone();

    crew.listen().await;
    let state = crew.ok("GET", "/api/state", None).await;
    let bot = state["bots"][0]["id"].as_str().unwrap().to_string();
    crew.say(&bot, "позови эхо").await;
    crew.wait_event("конец хода", |e| e["type"] == "turn_end").await;

    let calls = model.calls.lock().unwrap().clone();
    assert!(calls.len() >= 2, "ожидал два обращения к модели, было {}", calls.len());
    let tools = calls[0]["tools"].as_array().unwrap();
    let def = tools
        .iter()
        .find(|tool| tool["function"]["name"] == full.as_str())
        .unwrap_or_else(|| panic!("инструмент {full} не показан модели: {tools:?}"));
    assert!(def["function"]["description"].as_str().unwrap_or_default().contains("Эхо"));

    let tool = crew
        .find_event(|event| event["type"] == "tool_update" && event["status"] == "ok")
        .expect("MCP-инструмент не выполнился");
    assert_eq!(tool["result"], "echo: привет", "вызов не вернул эхо: {tool}");
    let before = calls[1]["messages"].as_array().unwrap();
    assert!(
        before.iter().any(|m| m["role"] == "tool" && m["content"].as_str().unwrap_or_default().contains("echo: привет")),
        "модель не увидела результат инструмента: {before:?}"
    );
    let (_, _, call_count, _) = counts(&fake);
    assert_eq!(call_count, 1, "tools/call должен уйти один раз");
}
