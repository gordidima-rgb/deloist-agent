//! Командная строка, MCP-сервер и картинки.
//!
//! Всё это работает поверх уже запущенной копии, поэтому тесты поднимают обычный сервер
//! через общую обвязку и говорят с ним так же, как это делал бы человек из консоли.

use crate::common;

use std::io::{BufRead, BufReader, Write};
use std::path::Path;
use std::process::{Command, Stdio};


use axum::extract::State;
use axum::response::{IntoResponse, Response};
use axum::routing::post;
use axum::{Json, Router};
use common::{Crew, BIN};
use serde_json::{json, Value};

/// Запуск команды так, как её запустит человек: с той же папкой данных.
fn cli(data: &Path, args: &[&str]) -> (i32, String, String) {
    let out = Command::new(BIN)
        .args(args)
        .env("CREW_DATA_DIR", data)
        .stdin(Stdio::null())
        .output()
        .expect("запуск deloist");
    (
        out.status.code().unwrap_or(-1),
        String::from_utf8_lossy(&out.stdout).to_string(),
        String::from_utf8_lossy(&out.stderr).to_string(),
    )
}

fn bot_id(state: &Value, name: &str) -> String {
    state["bots"]
        .as_array()
        .unwrap()
        .iter()
        .find(|b| b["name"] == name)
        .and_then(|b| b["id"].as_str())
        .expect("бота нет")
        .to_string()
}

#[tokio::test]
async fn the_command_line_sees_the_team_and_says_it_is_running() {
    let crew = Crew::start("cli-basic").await;
    let (code, out, err) = cli(&crew.data_dir, &["bots", "--json"]);
    assert_eq!(code, 0, "stderr: {err}");
    let data: Value = serde_json::from_str(out.trim()).expect("не JSON");
    assert_eq!(data["bots"].as_array().unwrap().len(), 3, "{data}");

    let (code, out, _) = cli(&crew.data_dir, &["rooms", "--json"]);
    assert_eq!(code, 0);
    let data: Value = serde_json::from_str(out.trim()).unwrap();
    assert!(data["rooms"].as_array().is_some());

    // Без приложения команда обязана сказать это и вернуть свой код.
    let empty = std::env::temp_dir().join(format!("deloist-nobody-{}", std::process::id()));
    let (code, _, err) = cli(&empty, &["bots"]);
    assert_eq!(code, 3, "код «не запущено»: {err}");
    assert!(err.to_lowercase().contains("не запущен"), "{err}");
    let _ = std::fs::remove_dir_all(&empty);
}

#[tokio::test]
async fn run_gives_a_task_to_a_bot_and_exports_what_it_made() {
    let crew = Crew::start("cli-run").await;
    let state = crew.ok("GET", "/api/state", None).await;
    let name = state["bots"][0]["name"].as_str().unwrap().to_string();
    let export = crew.data_dir.join("exported");

    let (code, out, err) = cli(
        &crew.data_dir,
        &[
            "run",
            "сохрани это в файл",
            "--bot",
            &name,
            "--export",
            export.to_str().unwrap(),
            "--json",
            "--timeout",
            "60",
        ],
    );
    assert_eq!(code, 0, "stdout: {out}; stderr: {err}");
    let data: Value = serde_json::from_str(out.trim()).expect("не JSON");
    assert!(data["answer"].as_str().is_some_and(|a| !a.is_empty()), "{data}");
    let exported = data["exported"].as_array().unwrap();
    assert!(exported.iter().any(|f| f == "draft.md"), "файл не выгружен: {data}");
    assert!(export.join("draft.md").is_file(), "файла нет на диске");

    // Имя бота, которого нет, — ошибка аргументов, а не молчаливый запуск.
    let (code, _, err) = cli(&crew.data_dir, &["run", "привет", "--bot", "Такого нет"]);
    assert_eq!(code, 2, "{err}");
}

#[tokio::test]
async fn a_team_file_goes_out_and_comes_back() {
    let crew = Crew::start("cli-team").await;
    let file = crew.data_dir.join("team.json");
    let (code, out, err) = cli(&crew.data_dir, &["team", "export", "--out", file.to_str().unwrap()]);
    assert_eq!(code, 0, "{out} {err}");
    let team: Value = serde_json::from_str(&std::fs::read_to_string(&file).unwrap()).unwrap();
    assert_eq!(team["format"], "deloist-team");
    assert_eq!(team["bots"].as_array().unwrap().len(), 3);

    let (code, out, err) = cli(&crew.data_dir, &["team", "apply", file.to_str().unwrap(), "--json"]);
    assert_eq!(code, 0, "{out} {err}");
    let data: Value = serde_json::from_str(out.trim()).unwrap();
    assert_eq!(data["imported"]["bots"], 3, "{data}");
}

/// Заглушка MCP-клиента: пишет запросы построчно и читает ответы.
struct Mcp {
    child: std::process::Child,
    out: BufReader<std::process::ChildStdout>,
}

impl Mcp {
    fn start(data: &Path) -> Self {
        let mut child = Command::new(BIN)
            .arg("--mcp")
            .env("CREW_DATA_DIR", data)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .expect("запуск MCP-сервера");
        let out = BufReader::new(child.stdout.take().unwrap());
        Self { child, out }
    }

    fn send(&mut self, request: Value) -> Value {
        let stdin = self.child.stdin.as_mut().unwrap();
        writeln!(stdin, "{request}").unwrap();
        stdin.flush().unwrap();
        let mut line = String::new();
        self.out.read_line(&mut line).expect("ответ MCP");
        serde_json::from_str(&line).expect("ответ не JSON")
    }
}

impl Drop for Mcp {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

#[tokio::test]
async fn the_mcp_server_lists_the_team_and_sends_work() {
    let crew = Crew::start("mcp-server").await;
    let state = crew.ok("GET", "/api/state", None).await;
    let name = state["bots"][0]["name"].as_str().unwrap().to_string();

    let mut mcp = Mcp::start(&crew.data_dir);
    let hello = mcp.send(json!({
        "jsonrpc": "2.0", "id": 1, "method": "initialize",
        "params": { "protocolVersion": "2024-11-05", "capabilities": {}, "clientInfo": { "name": "test", "version": "1" } }
    }));
    assert_eq!(hello["result"]["serverInfo"]["name"], "deloist", "{hello}");

    let listed = mcp.send(json!({ "jsonrpc": "2.0", "id": 2, "method": "tools/list", "params": {} }));
    let tools: Vec<String> = listed["result"]["tools"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t["name"].as_str().unwrap().to_string())
        .collect();
    for wanted in ["list_bots", "send_message", "read_thread", "list_rooms"] {
        assert!(tools.contains(&wanted.to_string()), "нет инструмента {wanted}: {tools:?}");
    }

    let bots = mcp.send(json!({
        "jsonrpc": "2.0", "id": 3, "method": "tools/call",
        "params": { "name": "list_bots", "arguments": {} }
    }));
    let text = bots["result"]["content"][0]["text"].as_str().unwrap_or_default();
    assert!(text.contains(&name), "нет бота в списке: {text}");

    let sent = mcp.send(json!({
        "jsonrpc": "2.0", "id": 4, "method": "tools/call",
        "params": { "name": "send_message", "arguments": { "bot": name, "text": "привет", "timeout": 60 } }
    }));
    assert_eq!(sent["result"]["isError"], false, "{sent}");
    let answer = sent["result"]["content"][0]["text"].as_str().unwrap_or_default();
    assert!(!answer.trim().is_empty(), "пустой ответ: {sent}");

    // Неизвестный инструмент — ошибка в ответе, а не падение сервера.
    let bad = mcp.send(json!({
        "jsonrpc": "2.0", "id": 5, "method": "tools/call",
        "params": { "name": "delete_everything", "arguments": {} }
    }));
    assert_eq!(bad["result"]["isError"], true, "{bad}");
}

// Внешний агент через MCP вспоминает и запоминает в общей памяти той же копии.
#[tokio::test]
async fn an_mcp_agent_recalls_and_remembers() {
    let crew = Crew::start("mcp-memory").await;
    let mut mcp = Mcp::start(&crew.data_dir);
    let call = |id: i64, name: &str, arguments: Value| json!({ "jsonrpc": "2.0", "id": id, "method": "tools/call", "params": { "name": name, "arguments": arguments } });

    let listed = mcp.send(json!({ "jsonrpc": "2.0", "id": 1, "method": "tools/list", "params": {} }));
    let names: Vec<&str> = listed["result"]["tools"].as_array().unwrap().iter().filter_map(|t| t["name"].as_str()).collect();
    assert!(names.contains(&"recall") && names.contains(&"remember"), "{names:?}");

    let saved = mcp.send(call(2, "remember", json!({ "title": "Отчёты Ozon — по понедельникам", "body": "Шаблон в папке reports", "kind": "project" })));
    assert_eq!(saved["result"]["isError"], false, "{saved}");
    let memory = crew.ok("GET", "/api/memory", None).await;
    assert_eq!(memory["facts"][0]["title"], "Отчёты Ozon — по понедельникам");
    assert_eq!(memory["facts"][0]["author"], "mcp");
    assert_eq!(memory["journal"][0]["who"]["via"], "mcp", "правка в журнале и отменяется");

    let found = mcp.send(call(3, "recall", json!({ "query": "отчёт озон" })));
    let text = found["result"]["content"][0]["text"].as_str().unwrap_or_default();
    assert!(text.contains("[m1]") && text.contains("Шаблон в папке reports"), "{found}");

    let secret = mcp.send(call(4, "remember", json!({ "title": "Пароль Ozon: Qwerty123" })));
    assert_eq!(secret["result"]["isError"], true, "секреты не пишутся: {secret}");

    // Личное бота внешнему агенту не править и не видеть.
    let state = crew.ok("GET", "/api/state", None).await;
    let bot = state["bots"][0]["id"].as_str().unwrap().to_string();
    let own = crew.ok("POST", "/api/memory/facts", Some(json!({ "title": "Рабочая заметка бота", "scope": bot }))).await["fact"]["id"].as_str().unwrap().to_string();
    let foreign = mcp.send(call(5, "remember", json!({ "id": own, "title": "Переписано агентом" })));
    assert_eq!(foreign["result"]["isError"], true, "{foreign}");
    let memory = crew.ok("GET", &format!("/api/bots/{bot}/memory"), None).await;
    assert_eq!(memory["facts"][0]["title"], "Рабочая заметка бота");

    let month = crew.data_dir.join("memory").join("episodes").join("2026-11");
    std::fs::create_dir_all(&month).unwrap();
    std::fs::write(month.join("2026-11-03 Личный.md"), format!("---\nid: e1\ntitle: \"Личный отчёт\"\nthread: \"bot:{bot}\"\n---\nотчёт\n")).unwrap();
    std::fs::write(month.join("2026-11-04 Группа.md"), "---\nid: e2\ntitle: \"Отчёт в группе\"\nthread: \"room:r1\"\n---\nотчёт\n").unwrap();
    let episodes = mcp.send(call(6, "recall", json!({ "scope": "episodes", "query": "отчёт" })));
    let text = episodes["result"]["content"][0]["text"].as_str().unwrap_or_default();
    assert!(text.contains("[e2]") && !text.contains("[e1]"), "только общие эпизоды: {text}");
}

// ---------------------------------------------------------------------------
// Картинки
// ---------------------------------------------------------------------------

/// Байты картинки: содержимое нам не важно, важно, что файл сохраняется один в один.
fn png_1x1() -> Vec<u8> {
    b"\x89PNG\r\n\x1a\n picture bytes".to_vec()
}

#[derive(Clone)]
struct FakeImages {
    calls: std::sync::Arc<std::sync::Mutex<Vec<Value>>>,
    auth: std::sync::Arc<std::sync::Mutex<Vec<String>>>,
    fail: bool,
    /// Отвечать ссылкой, как делают некоторые шлюзы, а не байтами.
    link: Option<String>,
}

async fn images(State(fake): State<FakeImages>, headers: axum::http::HeaderMap, Json(body): Json<Value>) -> Response {
    fake.calls.lock().unwrap().push(body.clone());
    let auth = headers.get("authorization").and_then(|v| v.to_str().ok()).unwrap_or_default().to_string();
    fake.auth.lock().unwrap().push(auth);
    if fake.fail {
        return (axum::http::StatusCode::UNAUTHORIZED, "no").into_response();
    }
    if let Some(link) = &fake.link {
        return Json(json!({ "data": [{ "url": link }] })).into_response();
    }
    let encoded = b64(&png_1x1());
    Json(json!({ "data": [{ "b64_json": encoded }] })).into_response()
}

async fn picture_file(headers: axum::http::HeaderMap) -> Response {
    // Ключ сервиса не должен уходить по ссылке из ответа.
    if headers.contains_key("authorization") {
        return axum::http::StatusCode::IM_A_TEAPOT.into_response();
    }
    png_1x1().into_response()
}

/// Тот же base64, что у сервера: свой кодировщик держим в тесте, чтобы не звать внутренности.
fn b64(bytes: &[u8]) -> String {
    const TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::new();
    for chunk in bytes.chunks(3) {
        let b = [chunk[0], *chunk.get(1).unwrap_or(&0), *chunk.get(2).unwrap_or(&0)];
        let n = (b[0] as u32) << 16 | (b[1] as u32) << 8 | b[2] as u32;
        out.push(TABLE[(n >> 18) as usize & 63] as char);
        out.push(TABLE[(n >> 12) as usize & 63] as char);
        out.push(if chunk.len() > 1 { TABLE[(n >> 6) as usize & 63] as char } else { '=' });
        out.push(if chunk.len() > 2 { TABLE[n as usize & 63] as char } else { '=' });
    }
    out
}

async fn serve_fake(fail: bool) -> (String, FakeImages, tokio::task::JoinHandle<()>) {
    serve_images(fail, false).await
}

async fn serve_fake_links() -> (String, FakeImages, tokio::task::JoinHandle<()>) {
    serve_images(false, true).await
}

async fn serve_images(fail: bool, links: bool) -> (String, FakeImages, tokio::task::JoinHandle<()>) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let fake = FakeImages {
        calls: Default::default(),
        auth: Default::default(),
        fail,
        link: links.then(|| format!("http://{addr}/files/picture.png")),
    };
    let app = Router::new()
        .route("/v1/images/generations", post(images))
        .route("/files/picture.png", axum::routing::get(picture_file))
        .with_state(fake.clone());
    let handle = tokio::spawn(async move {
        let _ = axum::serve(listener, app).await;
    });
    (format!("http://{addr}/v1"), fake, handle)
}

#[tokio::test]
async fn a_bot_draws_a_picture_and_the_file_lands_in_its_workspace() {
    let (base, fake, handle) = serve_fake(false).await;
    let crew = Crew::start("image").await;
    crew.ok(
        "PUT",
        "/api/settings",
        Some(json!({ "imageBaseUrl": base, "imageModel": "fake-image", "imageApiKey": "test-key", "imageDaily": 5 })),
    )
    .await;
    let state = crew.ok("GET", "/api/state", None).await;
    assert_eq!(state["settings"]["hasImageKey"], true, "ключ картинок не сохранился");
    let id = bot_id(&state, "Аналитик");
    // В демо-режиме картинка спрашивает разрешение: закрываем карточку сразу.
    crew.ok("PATCH", &format!("/api/bots/{id}"), Some(json!({ "approval": "auto" }))).await;
    crew.listen().await;

    crew.say(&id, "нарисуй кота в шляпе").await;
    crew.wait_event("ход с картинкой", |e| e["type"] == "turn_end" && e["botId"] == id.as_str()).await;

    let tool = crew
        .find_event(|e| e["type"] == "tool_update" && e["botId"] == id.as_str())
        .expect("инструмент не вызывался");
    assert_eq!(tool["status"], "ok", "{tool}");
    assert!(tool["result"].as_str().unwrap_or_default().contains("images/"), "{tool}");

    let calls = fake.calls.lock().unwrap();
    assert_eq!(calls.len(), 1, "сервис вызван не один раз");
    assert_eq!(calls[0]["n"], 1);
    assert!(calls[0]["prompt"].as_str().unwrap_or_default().contains("кота"), "{:?}", calls[0]);

    let files = crew.ok("GET", &format!("/api/bots/{id}/files?path=images"), None).await;
    let items = files["items"].as_array().unwrap();
    assert_eq!(items.len(), 1, "картинки нет в воркспейсе: {files}");
    assert!(items[0]["modified"].as_i64().unwrap_or(0) > 0, "у файла нет времени правки");
    handle.abort();
}

#[tokio::test]
async fn a_full_day_or_a_broken_service_stops_the_picture() {
    let (base, _fake, handle) = serve_fake(true).await;
    let crew = Crew::start("image-fail").await;
    crew.ok(
        "PUT",
        "/api/settings",
        Some(json!({ "imageBaseUrl": base, "imageModel": "fake-image", "imageApiKey": "bad", "imageDaily": 1 })),
    )
    .await;
    let state = crew.ok("GET", "/api/state", None).await;
    let id = bot_id(&state, "Редактор");
    crew.ok("PATCH", &format!("/api/bots/{id}"), Some(json!({ "approval": "auto" }))).await;
    crew.listen().await;

    crew.say(&id, "нарисуй закат").await;
    crew.wait_event("ход с ошибкой сервиса", |e| e["type"] == "turn_end" && e["botId"] == id.as_str()).await;
    let tool = crew
        .find_event(|e| e["type"] == "tool_update" && e["botId"] == id.as_str())
        .expect("инструмент не вызывался");
    assert_eq!(tool["status"], "error", "{tool}");
    let message = tool["result"].as_str().unwrap_or_default();
    assert!(message.contains("HTTP 401"), "ошибка без кода: {message}");
    // Тело ответа провайдера наружу не уходит.
    assert!(!message.contains("\"no\""), "в ошибку попало тело провайдера: {message}");

    // Папки images может не быть вовсе: значит, файл точно не появился.
    let (status, files) = crew.send("GET", &format!("/api/bots/{id}/files?path=images"), None).await;
    if status == 200 {
        assert!(files["items"].as_array().unwrap().is_empty(), "файл появился при ошибке");
    } else {
        assert_eq!(status, 400, "неожиданный код: {files}");
    }
    handle.abort();
}

#[tokio::test]
async fn the_agent_key_opens_the_api_and_nothing_else() {
    let crew = Crew::start("agent-key").await;
    let runtime: Value =
        serde_json::from_str(&std::fs::read_to_string(crew.data_dir.join("runtime.json")).expect("нет паспорта копии")).unwrap();
    let token = runtime["token"].as_str().unwrap().to_string();
    assert!(token.len() >= 32, "ключ короткий: {token}");

    let state = crew
        .client
        .get(format!("{}/api/state", crew.base))
        .bearer_auth(&token)
        .send()
        .await
        .unwrap();
    assert_eq!(state.status().as_u16(), 200, "ключ не пускает к API");

    let wrong = crew
        .client
        .get(format!("{}/api/state", crew.base))
        .bearer_auth(format!("{token}0"))
        .send()
        .await
        .unwrap();
    assert_eq!(wrong.status().as_u16(), 403, "чужой ключ пустили");

    // Файл как есть — только из воркспейса: ключу нечего делать в секретах рядом.
    let state: Value = state.json().await.unwrap();
    let id = bot_id(&state, "Аналитик");
    for path in ["../../secrets.json", "..%5C..%5Csecrets.json"] {
        let escaped = crew
            .client
            .get(format!("{}/api/bots/{id}/file/raw?path={path}", crew.base))
            .bearer_auth(&token)
            .send()
            .await
            .unwrap();
        assert_eq!(escaped.status().as_u16(), 400, "вышли из воркспейса через {path}");
    }

    // Ключ программы не должен открывать страницу: она ходит только по сессии окна.
    let page = crew.client.get(format!("{}/", crew.base)).bearer_auth(&token).send().await.unwrap();
    assert_eq!(page.status().as_u16(), 403, "ключ открыл страницу");

    // И не должен заменять секрет вебхука.
    let hook = crew
        .client
        .post(format!("{}/hooks/rt_none/deadbeef", crew.base))
        .bearer_auth(&token)
        .header("content-type", "application/json")
        .body("{}")
        .send()
        .await
        .unwrap();
    assert_eq!(hook.status().as_u16(), 404, "ключ обошёл секрет вебхука");
}

#[test]
fn the_help_lists_the_commands() {
    let out = Command::new(BIN).arg("help").output().expect("запуск");
    let text = String::from_utf8_lossy(&out.stdout).to_string();
    assert!(text.contains("deloist run"), "{text}");
    assert_eq!(out.status.code(), Some(2), "справка — это не запуск приложения");
}

// ---------------------------------------------------------------------------
// Ошибки, найденные при проверке ветки
// ---------------------------------------------------------------------------

/// Группа из одного бота: ход группы начинается не сразу после ответа на POST.
async fn one_bot_room(crew: &Crew, name: &str) -> String {
    let state = crew.ok("GET", "/api/state", None).await;
    let bot = bot_id(&state, "Аналитик");
    let room = crew
        .ok("POST", "/api/rooms", Some(json!({ "name": name, "botIds": [bot], "lead": bot, "mode": "lead" })))
        .await;
    room["room"]["id"].as_str().unwrap().to_string()
}

#[tokio::test]
async fn a_room_task_returns_the_answer_of_this_turn() {
    let crew = Crew::start("cli-room").await;
    one_bot_room(&crew, "Штаб").await;

    let (code, out, err) = cli(&crew.data_dir, &["run", "первый вопрос", "--room", "Штаб", "--json", "--timeout", "30"]);
    assert_eq!(code, 0, "{out} {err}");
    let first: Value = serde_json::from_str(out.trim()).unwrap();
    assert!(first["answer"].as_str().unwrap_or_default().contains("первый вопрос"), "не дождались ответа: {first}");

    let (code, out, err) = cli(&crew.data_dir, &["run", "второй вопрос", "--room", "Штаб", "--json", "--timeout", "30"]);
    assert_eq!(code, 0, "{out} {err}");
    let second: Value = serde_json::from_str(out.trim()).unwrap();
    assert!(second["answer"].as_str().unwrap_or_default().contains("второй вопрос"), "вернулся старый ответ: {second}");
}

/// Модель думает секунду с лишним и отказывает: группа в режиме auto сначала спрашивает её,
/// кому отвечать, и всё это время ещё не считается занятой.
async fn slow_refusal() -> Response {
    tokio::time::sleep(std::time::Duration::from_millis(1500)).await;
    axum::http::StatusCode::UNAUTHORIZED.into_response()
}

#[tokio::test(flavor = "multi_thread")]
async fn a_room_is_awaited_while_it_is_still_choosing_who_answers() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let model = Router::new().route("/v1/chat/completions", post(slow_refusal));
    let handle = tokio::spawn(async move {
        let _ = axum::serve(listener, model).await;
    });

    let crew = Crew::start("cli-room-auto").await;
    crew.ok("PUT", "/api/settings", Some(json!({ "driver": "api", "baseUrl": format!("http://{addr}/v1"), "apiKey": "x", "model": "m" }))).await;
    let state = crew.ok("GET", "/api/state", None).await;
    let bot = bot_id(&state, "Аналитик");
    crew.ok("POST", "/api/rooms", Some(json!({ "name": "Штаб", "botIds": [bot], "lead": bot, "mode": "auto" }))).await;

    let (code, out, _) = cli(&crew.data_dir, &["run", "вопрос", "--room", "Штаб", "--json", "--timeout", "60"]);
    assert_eq!(code, 1, "вышли, не дождавшись группы: {out}");
    let data: Value = serde_json::from_str(out.trim()).unwrap();
    assert!(data["answer"].as_str().unwrap_or_default().contains("Не смог"), "{data}");
    handle.abort();
}

#[tokio::test]
async fn a_room_refuses_export_before_it_starts_working() {
    let crew = Crew::start("cli-room-export").await;
    let room = one_bot_room(&crew, "Штаб").await;
    let export = crew.data_dir.join("out");
    let (code, _, err) = cli(&crew.data_dir, &["run", "задача", "--room", "Штаб", "--export", export.to_str().unwrap()]);
    assert_eq!(code, 2, "{err}");
    tokio::time::sleep(std::time::Duration::from_millis(300)).await;
    let thread = crew.ok("GET", &format!("/api/rooms/{room}/thread"), None).await;
    assert!(thread["messages"].as_array().unwrap().is_empty(), "задача ушла в работу: {thread}");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_failed_turn_is_a_failure_and_never_the_previous_answer() {
    let crew = Crew::start("cli-failure").await;
    let (code, _, err) = cli(&crew.data_dir, &["run", "привет", "--bot", "Аналитик", "--timeout", "30"]);
    assert_eq!(code, 0, "{err}");

    // Постоянный отказ по ключу не повторяется; временный сетевой сбой теперь повторяется.
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let handle = tokio::spawn(async move { let _ = axum::serve(listener, Router::new().route("/v1/chat/completions", post(slow_refusal))).await; });
    crew.ok("PUT", "/api/settings", Some(json!({ "driver": "api", "baseUrl": format!("http://{addr}/v1"), "apiKey": "x", "model": "m" }))).await;
    let (code, out, _) = cli(&crew.data_dir, &["run", "ещё раз", "--bot", "Аналитик", "--json", "--timeout", "60"]);
    assert_eq!(code, 1, "сбой модели выдан за успех: {out}");
    let data: Value = serde_json::from_str(out.trim()).unwrap();
    assert_eq!(data["ok"], false, "{data}");
    assert!(data["answer"].as_str().unwrap_or_default().contains("Не смог"), "старый ответ вместо ошибки: {data}");
    handle.abort();
}

#[tokio::test]
async fn bad_arguments_never_reach_a_bot() {
    let crew = Crew::start("cli-args").await;
    let state = crew.ok("GET", "/api/state", None).await;
    let id = bot_id(&state, "Аналитик");

    // Опечатка во флаге и флаг без значения не должны отдать задачу случайному боту.
    for args in [
        vec!["run", "задача", "--bto", "Аналитик"],
        vec!["run", "задача", "--bot"],
        vec!["run", "задача", "--bot", ""],
        vec!["run", "задача", "--bot", "Аналитик", "--timeout", "скоро"],
    ] {
        let (code, _, err) = cli(&crew.data_dir, &args);
        assert_eq!(code, 2, "{args:?}: {err}");
    }
    tokio::time::sleep(std::time::Duration::from_millis(300)).await;
    for bot in crew.ok("GET", "/api/state", None).await["bots"].as_array().unwrap() {
        let bot = bot["id"].as_str().unwrap();
        let thread = crew.ok("GET", &format!("/api/bots/{bot}/thread"), None).await;
        assert!(thread["messages"].as_array().unwrap().is_empty(), "задача дошла до бота: {thread}");
    }

    // Текст без кавычек — это вся фраза, а не первое слово.
    let (code, out, err) = cli(&crew.data_dir, &["run", "собери", "три", "темы", "--bot", "Аналитик", "--json", "--timeout", "30"]);
    assert_eq!(code, 0, "{err}");
    assert!(out.contains("собери три темы"), "фраза обрезана: {out}");
    let thread = crew.ok("GET", &format!("/api/bots/{id}/thread"), None).await;
    assert!(thread["messages"].as_array().unwrap().iter().any(|m| m["text"] == "собери три темы"), "{thread}");
}

#[tokio::test]
async fn the_mcp_server_stays_responsive_while_a_task_runs() {
    let crew = Crew::start("mcp-busy").await;
    let mut mcp = Mcp::start(&crew.data_dir);

    // Мусор в строке — ошибка разбора в ответ, а не тишина, на которой клиент повиснет.
    let stdin = mcp.child.stdin.as_mut().unwrap();
    writeln!(stdin, "это не json").unwrap();
    stdin.flush().unwrap();
    let mut line = String::new();
    mcp.out.read_line(&mut line).unwrap();
    let reply: Value = serde_json::from_str(&line).expect("ответ не JSON");
    assert_eq!(reply["error"]["code"], -32700, "{reply}");

    // «Запусти» в строгом режиме ждёт подтверждения человека, поэтому задача висит все пять секунд.
    let state = crew.ok("GET", "/api/state", None).await;
    let analyst = state["bots"].as_array().unwrap().iter().find(|b| b["name"] == "Аналитик").unwrap()["id"].as_str().unwrap().to_string();
    crew.ok("PATCH", &format!("/api/bots/{analyst}"), Some(json!({ "approval": "strict" }))).await;
    let stdin = mcp.child.stdin.as_mut().unwrap();
    let call = json!({
        "jsonrpc": "2.0", "id": 10, "method": "tools/call",
        "params": { "name": "send_message", "arguments": { "bot": "Аналитик", "text": "запусти команду", "timeout": 5 } }
    });
    writeln!(stdin, "{call}").unwrap();
    writeln!(stdin, "{}", json!({ "jsonrpc": "2.0", "id": 11, "method": "ping" })).unwrap();
    stdin.flush().unwrap();

    let mut line = String::new();
    mcp.out.read_line(&mut line).unwrap();
    let first: Value = serde_json::from_str(&line).unwrap();
    assert_eq!(first["id"], 11, "пинг ждал конца задачи: {first}");

    let mut line = String::new();
    mcp.out.read_line(&mut line).unwrap();
    let second: Value = serde_json::from_str(&line).unwrap();
    assert_eq!(second["id"], 10, "{second}");
    assert_eq!(second["result"]["isError"], true, "{second}");
}

#[tokio::test]
async fn a_short_answer_survives_the_client_closing_its_input() {
    let crew = Crew::start("mcp-close").await;
    let mut mcp = Mcp::start(&crew.data_dir);
    {
        let mut stdin = mcp.child.stdin.take().unwrap();
        let call = json!({ "jsonrpc": "2.0", "id": 1, "method": "tools/call", "params": { "name": "list_bots", "arguments": {} } });
        writeln!(stdin, "{call}").unwrap();
    }
    let mut line = String::new();
    mcp.out.read_line(&mut line).unwrap();
    let reply: Value = serde_json::from_str(&line).expect("ответ потерян при закрытии ввода");
    assert!(reply["result"]["content"][0]["text"].as_str().unwrap_or_default().contains("Аналитик"), "{reply}");
    assert!(mcp.child.wait().unwrap().success(), "сервер должен выйти сам");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_failed_turn_is_an_mcp_error() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let handle = tokio::spawn(async move { let _ = axum::serve(listener, Router::new().route("/v1/chat/completions", post(slow_refusal))).await; });
    let crew = Crew::start("mcp-failure").await;
    crew.ok("PUT", "/api/settings", Some(json!({ "driver": "api", "baseUrl": format!("http://{addr}/v1"), "apiKey": "x", "model": "m" }))).await;
    let mut mcp = Mcp::start(&crew.data_dir);
    let sent = mcp.send(json!({
        "jsonrpc": "2.0", "id": 1, "method": "tools/call",
        "params": { "name": "send_message", "arguments": { "bot": "Аналитик", "text": "привет", "timeout": 60 } }
    }));
    assert_eq!(sent["result"]["isError"], true, "{sent}");
    handle.abort();
}

// --- картинки ---------------------------------------------------------------

async fn image_bot(crew: &Crew, settings: Value) -> String {
    crew.ok("PUT", "/api/settings", Some(settings)).await;
    let state = crew.ok("GET", "/api/state", None).await;
    let id = bot_id(&state, "Аналитик");
    crew.ok("PATCH", &format!("/api/bots/{id}"), Some(json!({ "approval": "auto" }))).await;
    id
}

/// Закончить ход и вернуть результат инструмента.
async fn draw(crew: &Crew, id: &str, text: &str) -> Value {
    crew.clear_events();
    crew.say(id, text).await;
    crew.wait_event("ход с картинкой", |e| e["type"] == "turn_end" && e["botId"] == id).await;
    crew.find_event(|e| e["type"] == "tool_update" && e["botId"] == id).expect("инструмент не вызывался")
}

// Несколько потоков: `cli` блокирует свой поток, а подставной сервис должен отвечать.
#[tokio::test(flavor = "multi_thread")]
async fn a_picture_is_exported_byte_for_byte() {
    let (base, _fake, handle) = serve_fake(false).await;
    let crew = Crew::start("image-export").await;
    image_bot(&crew, json!({ "imageBaseUrl": base, "imageModel": "fake-image", "imageApiKey": "k", "imageDaily": 5 })).await;
    let export = crew.data_dir.join("out");

    let (code, out, err) = cli(
        &crew.data_dir,
        &["run", "нарисуй кота", "--bot", "Аналитик", "--export", export.to_str().unwrap(), "--json", "--timeout", "30"],
    );
    assert_eq!(code, 0, "{out} {err}");
    let data: Value = serde_json::from_str(out.trim()).unwrap();
    let name = data["exported"][0].as_str().expect("файл не выгружен").to_string();
    assert_eq!(std::fs::read(export.join(&name)).unwrap(), png_1x1(), "картинка испорчена при выгрузке");
    handle.abort();
}

#[tokio::test]
async fn the_main_key_never_goes_to_another_image_service() {
    let (base, fake, handle) = serve_fake(false).await;
    let crew = Crew::start("image-key").await;
    let id = image_bot(
        &crew,
        json!({ "baseUrl": "https://api.example.com/v1", "apiKey": "main-secret", "imageBaseUrl": base, "imageModel": "fake-image" }),
    )
    .await;
    crew.listen().await;

    let tool = draw(&crew, &id, "нарисуй кота").await;
    assert_eq!(tool["status"], "error", "{tool}");
    assert!(fake.calls.lock().unwrap().is_empty(), "основной ключ ушёл чужому сервису: {:?}", fake.auth.lock().unwrap());

    // Тот же адрес — тот же провайдер: основному ключу можно туда.
    crew.ok("PUT", "/api/settings", Some(json!({ "baseUrl": base }))).await;
    let tool = draw(&crew, &id, "нарисуй кота").await;
    assert_eq!(tool["status"], "ok", "{tool}");
    assert_eq!(fake.auth.lock().unwrap()[0], "Bearer main-secret");
    handle.abort();
}

#[tokio::test]
async fn the_daily_limit_stops_the_paid_request_and_a_refused_one_is_not_counted() {
    let (base, fake, handle) = serve_fake(false).await;
    let crew = Crew::start("image-limit").await;
    let id = image_bot(&crew, json!({ "imageBaseUrl": base, "imageModel": "fake-image", "imageApiKey": "k", "imageDaily": 1 })).await;
    crew.listen().await;

    assert_eq!(draw(&crew, &id, "нарисуй кота").await["status"], "ok");
    let tool = draw(&crew, &id, "нарисуй пса").await;
    assert_eq!(tool["status"], "error", "{tool}");
    assert!(tool["result"].as_str().unwrap_or_default().contains("предел"), "{tool}");
    assert_eq!(fake.calls.lock().unwrap().len(), 1, "платный запрос ушёл сверх предела");
    handle.abort();

    let (base, _fake, handle) = serve_fake(true).await;
    let crew = Crew::start("image-refused").await;
    let id = image_bot(&crew, json!({ "imageBaseUrl": base, "imageModel": "fake-image", "imageApiKey": "k", "imageDaily": 1 })).await;
    crew.listen().await;
    for _ in 0..2 {
        let tool = draw(&crew, &id, "нарисуй кота").await;
        assert!(tool["result"].as_str().unwrap_or_default().contains("HTTP 401"), "отказ съел дневной предел: {tool}");
    }
    handle.abort();
}

#[tokio::test]
async fn the_model_from_the_form_hint_does_not_get_a_parameter_it_rejects() {
    let (base, fake, handle) = serve_fake(false).await;
    let crew = Crew::start("image-gpt").await;
    let id = image_bot(&crew, json!({ "imageBaseUrl": base, "imageModel": "gpt-image-1", "imageApiKey": "k" })).await;
    crew.listen().await;
    assert_eq!(draw(&crew, &id, "нарисуй кота").await["status"], "ok");
    assert!(fake.calls.lock().unwrap()[0].get("response_format").is_none(), "gpt-image-1 отвечает на это 400");
    handle.abort();
}

#[tokio::test]
async fn a_service_may_answer_with_a_link_instead_of_bytes() {
    let (base, _fake, handle) = serve_fake_links().await;
    let crew = Crew::start("image-link").await;
    let id = image_bot(&crew, json!({ "imageBaseUrl": base, "imageModel": "fake-image", "imageApiKey": "k" })).await;
    crew.listen().await;
    let tool = draw(&crew, &id, "нарисуй кота").await;
    assert_eq!(tool["status"], "ok", "{tool}");
    let files = crew.ok("GET", &format!("/api/bots/{id}/files?path=images"), None).await;
    assert_eq!(files["items"].as_array().unwrap().len(), 1, "{files}");
    handle.abort();
}

