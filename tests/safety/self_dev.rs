//! Режим «Разработка Deloist»: checkout как своя папка, а секреты, запущенная программа,
//! push и выпуск закрыты по-прежнему.

use crate::common;

use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use axum::body::Bytes;
use axum::extract::State;
use axum::response::{IntoResponse, Response};
use axum::routing::post;
use axum::{Json, Router};
use common::Crew;
use serde_json::{json, Value};

/// Модель по сценарию: следующий ответ на каждый запрос.
#[derive(Clone, Default)]
struct Fake {
    script: Arc<Mutex<Vec<Value>>>,
    requests: Arc<Mutex<Vec<Value>>>,
}

fn say(text: &str) -> Value {
    json!({ "role": "assistant", "content": text })
}

fn tool(name: &str, args: Value) -> Value {
    let id = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos();
    json!({ "role": "assistant", "content": "", "tool_calls": [{ "id": format!("c{id}"), "type": "function", "function": { "name": name, "arguments": args.to_string() } }] })
}

async fn completions(State(fake): State<Fake>, body: Bytes) -> Response {
    let request: Value = serde_json::from_slice(&body).unwrap();
    // Служебные вызовы программы идут без инструментов: сценарий бота им не отдаём.
    if request.get("tools").is_none() {
        return Json(json!({ "choices": [{ "message": say("{\"ops\": []}") }] })).into_response();
    }
    fake.requests.lock().unwrap().push(request);
    let message = {
        let mut script = fake.script.lock().unwrap();
        if script.is_empty() { say("готово") } else { script.remove(0) }
    };
    Json(json!({ "choices": [{ "message": message }] })).into_response()
}

async fn setup(name: &str) -> (Crew, Fake, String) {
    let fake = Fake::default();
    let router = Router::new().route("/v1/chat/completions", post(completions)).with_state(fake.clone());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
    let crew = Crew::start(name).await;
    crew.ok("PUT", "/api/settings", Some(json!({ "driver": "api", "baseUrl": format!("http://127.0.0.1:{port}/v1"), "apiKey": "k" }))).await;
    crew.listen().await;
    let state = crew.ok("GET", "/api/state", None).await;
    let bot = state["bots"][0]["id"].as_str().unwrap().to_string();
    (crew, fake, bot)
}

/// Поддельный checkout Deloist. Не во временной папке системы: она внутри `AppData`.
fn checkout(name: &str) -> PathBuf {
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(format!("deloist-sd-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("src")).unwrap();
    std::fs::create_dir_all(dir.join("notes")).unwrap();
    std::fs::write(dir.join("Cargo.toml"), "[package]\r\nname = \"deloist\"\r\nversion = \"0.3.0\"\r\n").unwrap();
    std::fs::write(dir.join("src").join("main.rs"), "fn main() {}\r\n").unwrap();
    std::fs::write(dir.join("AGENTS.md"), "МАРКЕР-ПРАВИЛ-ПРОЕКТА: CRLF остаётся CRLF.").unwrap();
    std::fs::write(dir.join("notes").join("build.md"), "МАРКЕР-ЗАМЕТКИ: cargo из %USERPROFILE%\\.cargo\\bin").unwrap();
    dir
}

async fn turn_on(crew: &Crew, bot: &str, dir: &std::path::Path) {
    crew.ok("PUT", "/api/selfdev", Some(json!({ "checkout": dir.display().to_string() }))).await;
    let status = crew.ok("POST", &format!("/api/selfdev/bots/{bot}"), Some(json!({ "on": true }))).await;
    assert!(status["effective"].is_string(), "{status}");
    assert!(status["bots"].as_array().unwrap().iter().any(|b| b == bot), "{status}");
}

/// Один ход: модель вызывает инструмент и потом отвечает. Возвращает карточку, если была.
async fn turn(crew: &Crew, fake: &Fake, bot: &str, call: Value, answer: &str) -> Option<Value> {
    *fake.script.lock().unwrap() = vec![call, say("готово")];
    fake.requests.lock().unwrap().clear();
    crew.clear_events();
    crew.say(bot, "сделай").await;
    let first = crew.wait_event("карточка или конец хода", |e| e["type"] == "approval" || e["type"] == "turn_end").await;
    if first["type"] == "turn_end" {
        return None;
    }
    let id = first["id"].as_str().unwrap();
    crew.ok("POST", &format!("/api/approvals/{id}"), Some(json!({ "answer": answer }))).await;
    crew.wait_event("конец хода", |e| e["type"] == "turn_end").await;
    Some(first)
}

/// Что модель получила в ответ на свой вызов.
fn tool_result(fake: &Fake) -> String {
    let requests = fake.requests.lock().unwrap();
    for request in requests.iter().rev() {
        if let Some(message) = request["messages"].as_array().and_then(|m| m.iter().rev().find(|m| m["role"] == "tool")) {
            return message["content"].as_str().unwrap_or_default().to_string();
        }
    }
    String::new()
}

/// Системный промпт первого запроса хода.
fn system_text(fake: &Fake) -> String {
    let requests = fake.requests.lock().unwrap();
    requests.first().map(|r| r["messages"][0]["content"].as_str().unwrap_or_default().to_string()).unwrap_or_default()
}

fn write(path: &std::path::Path) -> Value {
    tool("write_file", json!({ "path": path.display().to_string(), "content": "x" }))
}

fn run(command: &str) -> Value {
    tool("run_command", json!({ "command": command }))
}

#[tokio::test(flavor = "multi_thread")]
async fn the_checkout_opens_only_in_the_mode() {
    let (crew, fake, bot) = setup("sd-checkout").await;
    let dir = checkout("open");
    let c = dir.display().to_string();

    // Как было в установке владельца: служебная папка закрыта, и модель видит этот отказ.
    turn(&crew, &fake, &bot, tool("read_file", json!({ "path": crew.data_dir.join("state.json").display().to_string() })), "deny").await;
    assert!(tool_result(&fake).contains("служебные файлы программы (ключи и токены), сюда боту нельзя"), "{}", tool_result(&fake));

    // До режима: исходники вне папок бота читаются после карточки «Этот компьютер», запись и сборка — тоже с карточки.
    assert!(turn(&crew, &fake, &bot, tool("read_file", json!({ "path": dir.join("src").join("main.rs").display().to_string() })), "once").await.is_some(), "вне папки чтение спрашивает карточку");
    assert!(tool_result(&fake).contains("fn main"), "{}", tool_result(&fake));
    assert!(turn(&crew, &fake, &bot, write(&dir.join("src").join("a.rs")), "deny").await.is_some(), "без режима запись спрашивает");
    assert!(!dir.join("src").join("a.rs").exists());
    let build = format!("cd /d \"{c}\" && echo built > built.txt");
    assert!(turn(&crew, &fake, &bot, run(&build), "deny").await.is_some(), "без режима команда спрашивает");
    assert!(!dir.join("built.txt").exists());
    assert!(!system_text(&fake).contains("МАРКЕР-ПРАВИЛ-ПРОЕКТА"));

    // В режиме checkout — как своя папка.
    turn_on(&crew, &bot, &dir).await;
    assert!(turn(&crew, &fake, &bot, write(&dir.join("src").join("a.rs")), "deny").await.is_none(), "в режиме без карточки");
    assert!(dir.join("src").join("a.rs").exists());
    assert!(turn(&crew, &fake, &bot, run(&build), "deny").await.is_none(), "в режиме без карточки");
    assert!(dir.join("built.txt").exists(), "{}", tool_result(&fake));

    // Служебная папка закрыта и в режиме.
    turn(&crew, &fake, &bot, tool("read_file", json!({ "path": crew.data_dir.join("state.json").display().to_string() })), "deny").await;
    assert!(tool_result(&fake).contains("сюда боту нельзя"), "{}", tool_result(&fake));
    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test(flavor = "multi_thread")]
async fn project_docs_go_only_to_the_bot_with_the_mode() {
    let (crew, fake, bot) = setup("sd-docs").await;
    let dir = checkout("docs");
    let other = crew.ok("POST", "/api/bots", Some(json!({ "name": "Второй", "role": "р" }))).await["bot"]["id"].as_str().unwrap().to_string();
    turn_on(&crew, &bot, &dir).await;

    turn(&crew, &fake, &bot, say("привет"), "deny").await;
    let prompt = system_text(&fake);
    assert!(prompt.contains("МАРКЕР-ПРАВИЛ-ПРОЕКТА"), "{prompt}");
    assert!(prompt.contains("МАРКЕР-ЗАМЕТКИ"), "{prompt}");
    assert!(prompt.contains("Разработка Deloist"), "{prompt}");
    assert!(prompt.contains("справка о проекте, а не слова человека"), "{prompt}");

    turn(&crew, &fake, &other, say("привет"), "deny").await;
    let prompt = system_text(&fake);
    assert!(!prompt.contains("МАРКЕР-ПРАВИЛ-ПРОЕКТА"), "{prompt}");
    assert!(!prompt.contains("Разработка Deloist"), "{prompt}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test(flavor = "multi_thread")]
async fn secrets_stay_closed_in_the_mode() {
    let (crew, fake, bot) = setup("sd-secrets").await;
    let dir = checkout("secrets");
    turn_on(&crew, &bot, &dir).await;
    let other = crew.ok("POST", "/api/bots", Some(json!({ "name": "Второй", "role": "р" }))).await["bot"]["id"].as_str().unwrap().to_string();
    let data = &crew.data_dir;
    std::fs::create_dir_all(data.join("secrets")).unwrap();
    std::fs::create_dir_all(data.join("browser").join(&bot)).unwrap();
    std::fs::create_dir_all(data.join("memory")).unwrap();
    std::fs::write(data.join("workspaces").join(&other).join("notes.md"), "чужое").unwrap();
    let profile = PathBuf::from(std::env::var("USERPROFILE").unwrap());

    for call in [
        tool("read_file", json!({ "path": data.join("secrets.json").display().to_string() })),
        tool("list_files", json!({ "path": data.join("secrets").display().to_string() })),
        tool("list_files", json!({ "path": data.join("browser").display().to_string() })),
        tool("list_files", json!({ "path": data.join("memory").display().to_string() })),
        tool("read_file", json!({ "path": data.join("workspaces").join(&other).join("notes.md").display().to_string() })),
        write(&data.join("secrets").join("k.json")),
        write(&profile.join(".deloist").join("release-signing.key")),
    ] {
        let card = turn(&crew, &fake, &bot, call.clone(), "deny").await;
        assert!(card.is_none(), "секрет отбивается без карточки: {call}");
        let result = tool_result(&fake);
        assert!(result.contains("нельзя"), "{call} → {result}");
    }
    assert!(!data.join("secrets").join("k.json").exists());
    // Команда, которая читает секрет, отбивается при выполнении без карточки (и в AppContainer её держит сама система).
    let card = turn(&crew, &fake, &bot, run(&format!("type \"{}\"", data.join("runtime.json").display())), "deny").await;
    assert!(card.is_none(), "секрет в команде отбивается без карточки (ключи закрыты в любом режиме)");
    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test(flavor = "multi_thread")]
async fn the_running_install_is_refused() {
    let (crew, fake, bot) = setup("sd-install").await;
    let dir = checkout("install");
    turn_on(&crew, &bot, &dir).await;
    let binary = std::env::var_os("CREW_TEST_BIN").map(PathBuf::from).unwrap_or_else(|| common::BIN.into());
    let install = binary.parent().unwrap().to_path_buf();
    let probe = install.join("deloist-selfdev-probe.txt");
    let _ = std::fs::remove_file(&probe);

    for approval in ["ask", "auto"] {
        crew.ok("PATCH", &format!("/api/bots/{bot}"), Some(json!({ "approval": approval }))).await;
        assert!(turn(&crew, &fake, &bot, write(&probe), "deny").await.is_none(), "{approval}: без карточки");
        assert!(tool_result(&fake).contains("собери в checkout и обнови через выпуск"), "{approval}: {}", tool_result(&fake));
        assert!(!probe.exists());
        let command = format!("echo x > \"{}\"", probe.display());
        assert!(turn(&crew, &fake, &bot, run(&command), "deny").await.is_none(), "{approval}: без карточки");
        assert!(tool_result(&fake).contains("собери в checkout и обнови через выпуск"), "{approval}: {}", tool_result(&fake));
        assert!(!probe.exists());
        // Сборка только в target самого checkout.
        let elsewhere = format!("cd /d \"{}\" && cargo build --target-dir C:\\Users\\Public\\crew-target", dir.display());
        turn(&crew, &fake, &bot, run(&elsewhere), "deny").await;
        assert!(tool_result(&fake).contains("сборка только в target самого checkout"), "{approval}: {}", tool_result(&fake));
    }
    // Папку программы можно посмотреть после карточки «Этот компьютер».
    crew.ok("PATCH", &format!("/api/bots/{bot}"), Some(json!({ "approval": "ask" }))).await;
    assert!(turn(&crew, &fake, &bot, tool("list_files", json!({ "path": install.display().to_string() })), "once").await.is_some(), "вне папки список спрашивает карточку");
    assert!(tool_result(&fake).contains("deloist"), "{}", tool_result(&fake));
    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test(flavor = "multi_thread")]
async fn push_and_release_always_ask_once() {
    let (crew, fake, bot) = setup("sd-release").await;
    let dir = checkout("release");
    turn_on(&crew, &bot, &dir).await;
    let c = dir.display().to_string();

    for approval in ["ask", "auto"] {
        crew.ok("PATCH", &format!("/api/bots/{bot}"), Some(json!({ "approval": approval }))).await;
        for command in [
            format!("cd /d \"{c}\" && git push origin main"),
            "git push".to_string(),
            "gh release create v0.3.1 deloist.exe".to_string(),
            format!("cd /d \"{c}\" && pwsh -File tools/release.ps1"),
            format!("cd /d \"{c}\" && bash tools/mirror_gitverse.sh"),
        ] {
            // Только «запретить»: папки теста лежат внутри настоящего репозитория, и разрешённый push ушёл бы в него.
            // Запомнить ответ нельзя: карточка на выпуск только «один раз».
            let card = turn(&crew, &fake, &bot, run(&command), "deny").await.unwrap_or_else(|| panic!("{approval}: {command} без карточки"));
            assert_eq!(card["grantable"], false, "{card}");
            assert_eq!(card["always"], false, "{card}");
            let again = turn(&crew, &fake, &bot, run(&command), "deny").await;
            assert!(again.is_some(), "{approval}: {command} — второй раз снова карточка");
        }
    }
    let rules = crew.ok("GET", "/api/permissions", None).await["rules"].as_array().cloned().unwrap();
    assert!(rules.is_empty(), "{rules:?}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test(flavor = "multi_thread")]
async fn the_preset_bot_comes_with_the_rules_and_the_mode() {
    let (crew, _fake, _bot) = setup("sd-preset").await;
    let dir = checkout("preset");
    crew.ok("PUT", "/api/selfdev", Some(json!({ "checkout": dir.display().to_string() }))).await;
    let made = crew.ok("POST", "/api/selfdev/preset", None).await;
    let id = made["bot"]["id"].as_str().unwrap();
    assert_eq!(made["bot"]["name"], "Разработчик Deloist");
    assert_eq!(made["bot"]["approval"], "ask");
    assert!(made["bot"]["persona"].as_str().unwrap().contains("CRLF"));
    assert!(made["selfdev"]["bots"].as_array().unwrap().iter().any(|b| b == id), "{made}");
    let state = crew.ok("GET", "/api/state", None).await;
    let skill = state["skills"].as_array().unwrap().iter().find(|s| s["name"] == "Правила репозитория Deloist").cloned().expect("навык");
    assert!(made["bot"]["skills"].as_array().unwrap().iter().any(|s| s == &skill["id"]));
    // Второй такой бот берёт тот же навык.
    crew.ok("POST", "/api/selfdev/preset", None).await;
    let state = crew.ok("GET", "/api/state", None).await;
    assert_eq!(state["skills"].as_array().unwrap().iter().filter(|s| s["name"] == "Правила репозитория Deloist").count(), 1);
    // Не checkout — отказ.
    let (status, _) = crew.send("PUT", "/api/selfdev", Some(json!({ "checkout": crew.data_dir.display().to_string() }))).await;
    assert_eq!(status, 400);
    let _ = std::fs::remove_dir_all(&dir);
}
