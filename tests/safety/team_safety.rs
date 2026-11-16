//! Группы, файл команды, передача задач, MCP и вебхуки: сценарии, которые ломались при проверке.

use crate::common;

use std::io::{Read, Write};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use axum::body::{Body, Bytes};
use axum::extract::State;
use axum::response::Response;
use axum::routing::post;
use axum::Router;
use common::Crew;
use serde_json::{json, Value};

fn node() -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    std::env::split_paths(&path).map(|d| d.join("node.exe")).find(|c| c.is_file())
}

async fn room_of(crew: &Crew, bots: &[&str]) -> String {
    let made = crew.ok("POST", "/api/rooms", Some(json!({ "name": "Проба", "botIds": bots, "lead": bots[0] }))).await;
    made["room"]["id"].as_str().unwrap().to_string()
}

fn ids(state: &Value) -> Vec<String> {
    state["bots"].as_array().unwrap().iter().map(|b| b["id"].as_str().unwrap().to_string()).collect()
}

fn bot_of(state: &Value, id: &str) -> Value {
    state["bots"].as_array().unwrap().iter().find(|b| b["id"] == id).unwrap().clone()
}

// ---------------------------------------------------------------- группы

#[tokio::test(flavor = "multi_thread")]
async fn a_bot_answering_in_a_room_is_busy_and_the_room_names_it() {
    let crew = Crew::start("room-busy").await;
    let bots = ids(&crew.ok("GET", "/api/state", None).await);
    crew.listen().await;
    let room = room_of(&crew, &[&bots[0]]).await;
    strict(&crew, &bots[0]).await;
    crew.ok("POST", &format!("/api/rooms/{room}/messages"), Some(json!({ "text": "запусти команду" }))).await;
    crew.wait_event("карточка в группе", |e| e["type"] == "approval").await;
    let state = crew.ok("GET", "/api/state", None).await;
    assert_eq!(bot_of(&state, &bots[0])["busy"], true, "бот ждёт решения, но считается свободным");
    assert_eq!(state["rooms"][0]["activeBot"], bots[0].as_str(), "интерфейс должен знать, чью карточку показать");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_personal_message_waits_for_the_room_and_keeps_its_card() {
    let crew = Crew::start("room-order").await;
    let bots = ids(&crew.ok("GET", "/api/state", None).await);
    crew.listen().await;
    let room = room_of(&crew, &[&bots[0]]).await;
    strict(&crew, &bots[0]).await;
    crew.ok("POST", &format!("/api/rooms/{room}/messages"), Some(json!({ "text": "запусти команду" }))).await;
    let room_card = crew.wait_event("карточка группы", |e| e["type"] == "approval").await;
    let room_card = room_card["id"].as_str().unwrap().to_string();

    // Личное сообщение занятому боту встаёт в очередь, второго хода рядом с группой нет.
    crew.clear_events();
    crew.say(&bots[0], "запусти команду").await;
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert!(crew.find_event(|e| e["type"] == "approval").is_none(), "личный ход пошёл параллельно с группой");

    crew.ok("POST", &format!("/api/approvals/{room_card}"), Some(json!({ "allow": false }))).await;
    crew.wait_event("конец хода группы", |e| e["type"] == "room_turn_end").await;

    // Группа закончила, очередь разобралась, и карточка личного хода живёт.
    let personal = crew.wait_event("карточка личного хода", |e| e["type"] == "approval").await;
    let id = personal["id"].as_str().unwrap().to_string();
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert!(crew.find_event(|e| e["type"] == "approval_closed" && e["id"] == id.as_str()).is_none());
    crew.ok("POST", &format!("/api/approvals/{id}"), Some(json!({ "allow": false }))).await;
    crew.wait_event("конец личного хода", |e| e["type"] == "turn_end" && e["roomId"].is_null()).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_second_quick_message_is_refused_not_lost() {
    let crew = Crew::start("room-quick").await;
    let bots = ids(&crew.ok("GET", "/api/state", None).await);
    crew.listen().await;
    let room = room_of(&crew, &[&bots[0]]).await;
    let path = format!("/api/rooms/{room}/messages");
    let (a, b) = tokio::join!(
        crew.send("POST", &path, Some(json!({ "text": "первое" }))),
        crew.send("POST", &path, Some(json!({ "text": "второе" })))
    );
    assert!(a.0 == 409 || b.0 == 409, "оба приняты: {} и {}", a.0, b.0);
    crew.wait_event("конец хода", |e| e["type"] == "room_turn_end").await;
    let thread = crew.ok("GET", &format!("/api/rooms/{room}/thread?limit=50"), None).await;
    let messages = thread["messages"].as_array().unwrap();
    assert_eq!(messages.iter().filter(|m| m["role"] == "user").count(), 1, "отказанное сообщение не должно висеть без ответа");
    assert_eq!(crew.ok("POST", &path, Some(json!({ "text": "третье" }))).await["ok"], true, "после хода группа свободна");
}

#[tokio::test(flavor = "multi_thread")]
async fn stopping_a_room_ends_the_turn_once_and_frees_the_bot() {
    let crew = Crew::start("room-stop").await;
    let bots = ids(&crew.ok("GET", "/api/state", None).await);
    crew.listen().await;
    let room = room_of(&crew, &[&bots[0]]).await;
    strict(&crew, &bots[0]).await;
    crew.ok("POST", &format!("/api/rooms/{room}/messages"), Some(json!({ "text": "запусти команду" }))).await;
    crew.wait_event("карточка", |e| e["type"] == "approval").await;
    assert_eq!(crew.ok("POST", &format!("/api/rooms/{room}/stop"), None).await["stopped"], true);
    crew.wait_event("конец", |e| e["type"] == "room_turn_end").await;
    tokio::time::sleep(Duration::from_millis(400)).await;
    let ends = crew.count_events(|e| e["type"] == "room_turn_end");
    assert_eq!(ends, 1, "конец хода пришёл {ends} раз");
    let state = crew.ok("GET", "/api/state", None).await;
    assert_eq!(bot_of(&state, &bots[0])["busy"], false);
    assert_eq!(state["rooms"][0]["busy"], false);
    let command_ran = crew.find_event(|e| e["type"] == "tool_update" && e["status"] == "ok");
    assert!(command_ran.is_none(), "команда выполнилась после «Стоп»: {command_ran:?}");
}

// ---------------------------------------------------------------- файл команды

#[tokio::test(flavor = "multi_thread")]
async fn a_team_file_cannot_choose_the_driver_or_open_a_webhook() {
    let crew = Crew::start("team-safe").await;
    let team = json!({
        "format": "deloist-team",
        "bots": [{ "name": "Чужой", "role": "р", "persona": "я".repeat(30_000), "driver": "cli", "model": "дорогая", "approval": "auto", "color": "url(x)" }],
        "routines": [{ "bot": "Чужой", "title": "Т", "prompt": "П", "hook": true, "enabled": true, "hookToken": "известный" }],
    });
    crew.ok("POST", "/api/team/import", Some(json!({ "team": team }))).await;
    let state = crew.ok("GET", "/api/state", None).await;
    let bot = state["bots"].as_array().unwrap().iter().find(|b| b["name"] == "Чужой").unwrap().clone();
    assert_eq!(bot["approval"], "ask");
    assert_eq!(bot["driver"], "", "чужой файл выбрал, чем отвечать");
    assert_eq!(bot["model"], "", "чужой файл выбрал модель");
    assert_ne!(bot["color"], "url(x)");
    assert_eq!(bot["persona"].as_str().unwrap().chars().count(), 20_000, "длинный текст обрезается");
    let routine = &state["routines"][0];
    assert_eq!(routine["enabled"], false);
    assert_eq!(routine["hook"], false, "вебхук из файла не включается");
    assert_eq!(routine["hookToken"], "");

    let export = crew.ok("GET", "/api/team/export", None).await;
    let raw = export["team"]["bots"][0].as_object().unwrap().clone();
    assert!(!raw.contains_key("driver") && !raw.contains_key("model"), "экспорт не несёт способ запуска: {raw:?}");

    let (status, _) = crew.send("POST", "/api/team/import", Some(json!({ "team": { "bots": [{ "name": "Без формата" }] } }))).await;
    assert_eq!(status, 400, "файл без отметки формата не принимается");
}

// ---------------------------------------------------------------- передача задачи

#[derive(Clone, Default)]
struct Model {
    seen: Arc<Mutex<Vec<String>>>,
}

fn sse(chunks: &[Value]) -> String {
    let mut out: String = chunks.iter().map(|c| format!("data: {c}\n\n")).collect();
    out.push_str("data: [DONE]\n\n");
    out
}

fn tool_step(name: &str, args: Value) -> String {
    sse(&[
        json!({ "choices": [{ "index": 0, "delta": { "tool_calls": [{ "index": 0, "id": format!("call_{name}"), "type": "function", "function": { "name": name, "arguments": args.to_string() } }] } }] }),
        json!({ "choices": [{ "index": 0, "delta": {}, "finish_reason": "tool_calls" }], "usage": { "prompt_tokens": 100, "completion_tokens": 10 } }),
    ])
}

/// Старший передаёт задачу Младшему, Младший хочет запустить команду.
async fn completions(State(model): State<Model>, body: Bytes) -> Response {
    let payload: Value = serde_json::from_slice(&body).unwrap_or_else(|_| json!({}));
    let messages = payload["messages"].as_array().cloned().unwrap_or_default();
    let system = messages[0]["content"].as_str().unwrap_or_default().to_string();
    let used_tool = messages.iter().any(|m| m["role"] == "tool");
    model.seen.lock().unwrap().push(system.chars().take(40).collect());
    let text = if used_tool {
        sse(&[
            json!({ "choices": [{ "index": 0, "delta": { "role": "assistant", "content": "Готово." } }] }),
            json!({ "choices": [{ "index": 0, "delta": {}, "finish_reason": "stop" }], "usage": { "prompt_tokens": 100, "completion_tokens": 10 } }),
        ])
    } else if system.contains("Ты — Старший") {
        tool_step("delegate", json!({ "bot": "Младший", "task": "запусти" }))
    } else {
        tool_step("run_command", json!({ "command": "echo проба > C:\\Windows\\deloist-probe.txt" }))
    };
    Response::builder().header("content-type", "text/event-stream").body(Body::from(text)).unwrap()
}

async fn with_model(name: &str) -> Crew {
    let router = Router::new().route("/v1/chat/completions", post(completions)).with_state(Model::default());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
    let crew = Crew::start(name).await;
    crew.ok("PUT", "/api/settings", Some(json!({ "driver": "api", "model": "m", "baseUrl": format!("http://127.0.0.1:{port}/v1"), "apiKey": "test-key" }))).await;
    crew
}

/// Демо-команда только читает и в обычном режиме идёт без вопроса, а тесту нужна висящая карточка.
async fn strict(crew: &Crew, bot: &str) {
    crew.ok("PATCH", &format!("/api/bots/{bot}"), Some(json!({ "approval": "strict" }))).await;
}

async fn bot_named(crew: &Crew, name: &str, approval: &str) -> String {
    crew.ok("POST", "/api/bots", Some(json!({ "name": name, "role": "р", "approval": approval }))).await["bot"]["id"].as_str().unwrap().to_string()
}

#[tokio::test(flavor = "multi_thread")]
async fn handing_a_task_to_an_auto_bot_does_not_skip_the_question() {
    let crew = with_model("delegate-safe").await;
    let senior = bot_named(&crew, "Старший", "ask").await;
    let junior = bot_named(&crew, "Младший", "auto").await;
    crew.ok("POST", "/api/rooms", Some(json!({ "name": "Пара", "botIds": [senior, junior], "lead": senior }))).await;
    crew.listen().await;
    crew.say(&senior, "сделай").await;
    let card = crew.wait_event("вопрос про команду", |e| e["type"] == "approval").await;
    assert_eq!(card["tool"], "run_command");
    assert_eq!(card["botId"], junior.as_str());
    crew.ok("POST", &format!("/api/approvals/{}", card["id"].as_str().unwrap()), Some(json!({ "allow": false }))).await;
    crew.wait_event("конец", |e| e["type"] == "turn_end" && e["botId"] == senior.as_str()).await;
    let store = crew.ok("GET", "/api/state", None).await;
    assert_eq!(bot_of(&store, &junior)["approval"], "auto", "настройка получателя не меняется");
}

#[tokio::test(flavor = "multi_thread")]
async fn between_auto_bots_nothing_is_asked() {
    let crew = with_model("delegate-auto").await;
    let senior = bot_named(&crew, "Старший", "auto").await;
    let junior = bot_named(&crew, "Младший", "auto").await;
    crew.ok("POST", "/api/rooms", Some(json!({ "name": "Пара", "botIds": [senior, junior], "lead": senior }))).await;
    crew.listen().await;
    crew.say(&senior, "сделай").await;
    crew.wait_event("конец", |e| e["type"] == "turn_end" && e["botId"] == senior.as_str()).await;
    assert!(crew.find_event(|e| e["type"] == "approval").is_none(), "оба разрешили себе работать без вопросов");
}

#[tokio::test(flavor = "multi_thread")]
async fn room_turns_are_counted_in_the_daily_tokens() {
    let crew = with_model("room-tokens").await;
    let bot = bot_named(&crew, "Младший", "ask").await;
    crew.listen().await;
    let room = room_of(&crew, &[&bot]).await;
    crew.ok("POST", &format!("/api/rooms/{room}/messages"), Some(json!({ "text": "сделай" }))).await;
    let card = crew.wait_event("карточка", |e| e["type"] == "approval").await;
    crew.ok("POST", &format!("/api/approvals/{}", card["id"].as_str().unwrap()), Some(json!({ "allow": false }))).await;
    crew.wait_event("конец", |e| e["type"] == "room_turn_end").await;
    let journal = crew.ok("GET", "/api/journal?limit=5", None).await;
    assert!(journal["usage"]["tokens"].as_i64().unwrap_or(0) >= 110, "токены группы не посчитаны: {journal}");
}

// ---------------------------------------------------------------- MCP

const MCP_HEAD: &str = r#"
let buf = '';
process.stdin.setEncoding('utf8');
const send = (o) => process.stdout.write(JSON.stringify(o) + '\n');
process.stdin.on('data', (c) => {
  buf += c; let i;
  while ((i = buf.indexOf('\n')) >= 0) {
    const line = buf.slice(0, i).trim(); buf = buf.slice(i + 1);
    if (!line) continue;
    const m = JSON.parse(line);
    if (m.id === undefined) continue;
    if (m.method === 'initialize') send({ jsonrpc: '2.0', id: m.id, result: { protocolVersion: '2024-11-05', capabilities: { tools: {} }, serverInfo: { name: 'f', version: '1' } } });
"#;

fn mcp_script(crew: &Crew, name: &str, body: &str) -> String {
    let script = crew.data_dir.join(name);
    std::fs::write(&script, format!("{MCP_HEAD}    {body}\n  }}\n}});\n")).unwrap();
    script.to_string_lossy().to_string()
}

fn lists(tool: &str) -> String {
    format!("else if (m.method === 'tools/list') send({{ jsonrpc: '2.0', id: m.id, result: {{ tools: [{{ name: '{tool}', inputSchema: {{ type: 'object' }} }}] }} }});")
}

async fn add_mcp(crew: &Crew, node: &std::path::Path, script: &str) -> String {
    crew.ok("POST", "/api/mcp", Some(json!({ "name": "A", "command": node.to_string_lossy(), "args": [script] }))).await["server"]["id"].as_str().unwrap().to_string()
}

#[tokio::test(flavor = "multi_thread")]
async fn a_request_from_the_server_is_not_taken_for_an_answer() {
    let Some(node) = node() else { return };
    let crew = Crew::start("mcp-id").await;
    let script = mcp_script(&crew, "m.mjs", "else if (m.method === 'tools/list') { send({ jsonrpc: '2.0', id: m.id, method: 'roots/list', params: {} }); send({ jsonrpc: '2.0', id: m.id, result: { tools: [{ name: 'echo', inputSchema: { type: 'object' } }] } }); }");
    let id = add_mcp(&crew, &node, &script).await;
    let checked = crew.ok("POST", &format!("/api/mcp/{id}/check"), None).await;
    assert_eq!(checked["tools"], json!(["echo"]), "{checked}");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_dead_server_fails_fast_and_does_not_slow_every_turn() {
    let Some(node) = node() else { return };
    let crew = Crew::start("mcp-dead").await;
    let script = crew.data_dir.join("dead.mjs");
    std::fs::write(&script, "process.exit(0);\n").unwrap();
    let id = add_mcp(&crew, &node, &script.to_string_lossy()).await;
    let started = Instant::now();
    let (status, _) = crew.send("POST", &format!("/api/mcp/{id}/check"), None).await;
    assert_eq!(status, 502);
    assert!(started.elapsed() < Duration::from_secs(5), "умерший сервер определяли {:?}", started.elapsed());

    // Первый ход узнаёт, что сервер мёртв, следующий даже не пробует его запускать.
    let bots = ids(&crew.ok("GET", "/api/state", None).await);
    crew.listen().await;
    crew.say(&bots[0], "привет").await;
    crew.wait_event("первый ход", |e| e["type"] == "turn_end").await;
    crew.clear_events();
    let started = Instant::now();
    crew.say(&bots[0], "привет ещё раз").await;
    crew.wait_event("второй ход", |e| e["type"] == "turn_end").await;
    assert!(started.elapsed() < Duration::from_secs(3), "ход ждал мёртвый сервер {:?}", started.elapsed());
}

#[tokio::test(flavor = "multi_thread")]
async fn editing_a_server_takes_effect_at_once() {
    let Some(node) = node() else { return };
    let crew = Crew::start("mcp-edit").await;
    let old = mcp_script(&crew, "old.mjs", &lists("old_tool"));
    let new = mcp_script(&crew, "new.mjs", &lists("new_tool"));
    let id = add_mcp(&crew, &node, &old).await;
    assert_eq!(crew.ok("POST", &format!("/api/mcp/{id}/check"), None).await["tools"], json!(["old_tool"]));
    crew.ok("PATCH", &format!("/api/mcp/{id}"), Some(json!({ "args": [new] }))).await;
    assert_eq!(crew.ok("POST", &format!("/api/mcp/{id}/check"), None).await["tools"], json!(["new_tool"]));
}

// ---------------------------------------------------------------- вебхук

fn raw_post(port: u16, path: &str) -> String {
    let mut stream = std::net::TcpStream::connect(("127.0.0.1", port)).unwrap();
    stream.set_read_timeout(Some(Duration::from_secs(3))).unwrap();
    let request = format!("POST {path} HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nContent-Type: application/json\r\nContent-Length: 2\r\nConnection: close\r\n\r\n{{}}");
    stream.write_all(request.as_bytes()).unwrap();
    let mut answer = String::new();
    let _ = stream.read_to_string(&mut answer);
    answer.lines().next().unwrap_or_default().to_string()
}

#[tokio::test(flavor = "multi_thread")]
async fn the_hook_path_does_not_open_the_api() {
    let crew = Crew::start("hook-path").await;
    for path in ["/hooks/../api/settings", "/hooks/..%2fapi%2fsettings", "/hooks/%2e%2e/api/team/import", "/hooks/x/../../api/team/import", "//hooks/../api/team/import"] {
        let line = tokio::task::block_in_place(|| raw_post(crew.port, path));
        assert!(!line.contains(" 200") && !line.contains(" 201") && !line.contains(" 202"), "{path} → {line}");
    }
    assert_eq!(crew.ok("GET", "/api/state", None).await["bots"].as_array().unwrap().len(), 3);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_hook_for_a_busy_bot_waits_and_its_secret_is_long() {
    let crew = Crew::start("hook-busy").await;
    let bots = ids(&crew.ok("GET", "/api/state", None).await);
    crew.listen().await;
    let made = crew.ok("POST", "/api/routines", Some(json!({ "botId": bots[0], "title": "Форма", "prompt": "Разбери", "hook": true }))).await;
    let (id, token) = (made["routine"]["id"].as_str().unwrap().to_string(), made["routine"]["hookToken"].as_str().unwrap().to_string());
    assert_eq!(token.len(), 64, "секрет короче остальных секретов программы");

    strict(&crew, &bots[0]).await;
    crew.say(&bots[0], "запусти команду").await;
    let card = crew.wait_event("бот занят", |e| e["type"] == "approval").await;
    let res = crew.client.post(format!("{}/hooks/{id}/{token}", crew.base)).body(r#"{"заявка":1}"#).send().await.unwrap();
    assert_eq!(res.status().as_u16(), 202, "занятый бот не повод терять событие");

    crew.ok("POST", &format!("/api/approvals/{}", card["id"].as_str().unwrap()), Some(json!({ "allow": false }))).await;
    let started = Instant::now();
    loop {
        let thread = crew.ok("GET", &format!("/api/bots/{}/thread?limit=50", bots[0]), None).await;
        let done = thread["messages"].as_array().unwrap().iter().filter_map(|m| m["text"].as_str()).any(|t| t.contains("[вебхук]") && t.contains("заявка"));
        if done {
            break;
        }
        assert!(started.elapsed() < Duration::from_secs(15), "вызов так и не дождался бота");
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
}
