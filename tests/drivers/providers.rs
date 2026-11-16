//! Экран «Подключить свою ИИ»: реестр ACP подменяется файлом, агент — заглушка `fake_acp` в zip-архиве.

use crate::common;

use std::path::{Path, PathBuf};

use std::sync::{Arc, Mutex};

use axum::body::{Body, Bytes};
use axum::extract::State;
use axum::http::HeaderMap;
use axum::response::Response;
use axum::routing::{get, post};
use axum::Router;
use common::{Crew, BIN};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

/// Архив с заглушкой и реестр, где она стоит под именем `codex-acp` (на экране — ChatGPT).
fn registry_ver(dir: &Path, version: &str, sha_ok: bool) -> PathBuf {
    std::fs::create_dir_all(dir).unwrap();
    let fake = Path::new(BIN).parent().unwrap().join("examples").join("fake_acp.exe");
    std::fs::copy(&fake, dir.join("fake_acp.exe")).unwrap();
    let zip = dir.join("agent.zip");
    let status = std::process::Command::new("tar")
        .args(["-a", "-cf"])
        .arg(&zip)
        .arg("-C")
        .arg(dir)
        .arg("fake_acp.exe")
        .status()
        .unwrap();
    assert!(status.success(), "не собрал архив");
    let bytes = std::fs::read(&zip).unwrap();
    let mut sha: String = Sha256::digest(&bytes).iter().map(|b| format!("{b:02x}")).collect();
    if !sha_ok {
        sha = "0".repeat(64);
    }
    let url = format!("file:///{}", zip.display().to_string().replace('\\', "/"));
    let binary = json!({ "archive": url, "cmd": "./fake_acp.exe", "sha256": sha });
    let data = json!({
        "version": "1.0.0",
        "agents": [
            {
                "id": "codex-acp", "name": "Codex", "version": version, "description": "заглушка",
                "distribution": { "binary": { "windows-x86_64": binary.clone(), "windows-aarch64": binary } },
            },
            { "id": "py-agent", "name": "Py Agent", "version": "0.1.0", "distribution": { "uvx": { "package": "py-agent" } } },
            { "id": "fake-npm", "name": "Fake npm", "version": "1.0.0", "distribution": { "npx": { "package": "fake-agent@1.0.0", "args": ["acp"] } } },
        ],
    });
    let path = dir.join("registry.json");
    std::fs::write(&path, data.to_string()).unwrap();
    path
}

fn registry(dir: &Path, sha_ok: bool) -> PathBuf {
    registry_ver(dir, "2.6.0", sha_ok)
}

async fn start(name: &str, sha_ok: bool, auth: bool) -> (Crew, PathBuf) {
    let dir = common::temp_data_dir(&format!("{name}-registry"));
    let path = registry(&dir, sha_ok);
    let log = dir.join("fake.log");
    let mut env: Vec<(&str, String)> = vec![
        ("CREW_TEST_ACP_REGISTRY", path.display().to_string()),
        ("FAKE_ACP_LOG", log.display().to_string()),
    ];
    if auth {
        env.push(("FAKE_ACP_AUTH", "1".into()));
    }
    let pairs: Vec<(&str, &str)> = env.iter().map(|(k, v)| (*k, v.as_str())).collect();
    let crew = Crew::start_with_env(name, &pairs).await;
    crew.listen().await;
    (crew, log)
}

fn methods(log: &Path) -> Vec<Value> {
    std::fs::read_to_string(log).unwrap_or_default().lines().filter_map(|l| serde_json::from_str(l).ok()).collect()
}

async fn chatgpt(crew: &Crew) -> Value {
    crew.ok("GET", "/api/providers/codex-acp", None).await
}

async fn wait_status(crew: &Crew, wanted: &str) -> Value {
    let started = std::time::Instant::now();
    loop {
        let view = chatgpt(crew).await;
        if view["status"] == wanted {
            return view;
        }
        assert!(started.elapsed().as_secs() < 30, "не дождался «{wanted}»: {view}");
        tokio::time::sleep(std::time::Duration::from_millis(150)).await;
    }
}

#[tokio::test]
async fn one_click_downloads_finds_the_sign_in_and_the_models() {
    let (crew, log) = start("prov-install", true, false).await;
    let list = crew.ok("GET", "/api/providers", None).await;
    let first = &list["providers"][0];
    assert_eq!(first["name"], "ChatGPT", "{list}");
    assert_eq!(first["status"], "not_installed");
    assert!(list["providers"].as_array().unwrap().iter().any(|p| p["id"] == "deepseek" && p["kind"] == "key"));

    crew.ok("POST", "/api/providers/codex-acp/install", None).await;
    let view = wait_status(&crew, "connected").await;
    assert_eq!(view["installed"], "2.6.0");
    assert_eq!(view["account"]["account"]["email"], "fake@example.com");
    let models: Vec<&str> = view["models"].as_array().unwrap().iter().filter_map(|m| m["id"].as_str()).collect();
    assert_eq!(models, ["fake-1", "fake-2"]);
    assert!(crew.find_event(|e| e["type"] == "provider" && e["provider"]["status"] == "installing").is_some(), "не было хода скачивания");
    assert!(crew.data_dir.join("agents").join("codex-acp").join("fake_acp.exe").exists());

    // Пробный запуск не оставляет процесс: вход проверен, агент погашен.
    assert!(methods(&log).iter().any(|m| m["method"] == "session/new"));

    // Основной для новых ботов: ответы идут через агента, модель бота доходит до агента.
    crew.ok("PATCH", "/api/providers/codex-acp", Some(json!({ "default": true }))).await;
    let state = crew.ok("GET", "/api/state", None).await;
    assert_eq!(state["settings"]["driver"], "acp:codex-acp");
    let bot = state["bots"][0]["id"].as_str().unwrap().to_string();
    crew.ok("PATCH", &format!("/api/bots/{bot}"), Some(json!({ "model": "fake-2" }))).await;
    crew.clear_events();
    crew.say(&bot, "привет").await;
    let done = crew.wait_event("конец хода", |e| e["type"] == "turn_end").await;
    assert_eq!(done["failed"], false, "{:?}", crew.find_event(|e| e["type"] == "error"));
    let chosen = methods(&log).into_iter().find(|m| m["method"] == "session/set_config_option").expect("модель не выбрана");
    assert_eq!(chosen["params"]["value"], "fake-2");
    let own_driver = state["bots"].as_array().unwrap().iter().filter(|b| b["driver"].as_str().unwrap_or_default().is_empty()).count();
    assert!(own_driver > 0);
    assert_eq!(chatgpt(&crew).await["usedBy"], own_driver, "основной провайдер у всех ботов без своего");
}

#[tokio::test]
async fn slow_probe_keeps_connected_model_usable() {
    let dir = common::temp_data_dir("prov-slow-probe-registry");
    let path = registry(&dir, true);
    let marker = dir.join("slow-probe");
    let registry_path = path.display().to_string();
    let marker_path = marker.display().to_string();
    let crew = Crew::start_with_env("prov-slow-probe", &[("CREW_TEST_ACP_REGISTRY", &registry_path), ("FAKE_ACP_SLOW_PROBE_FILE", &marker_path)]).await;
    crew.listen().await;
    crew.ok("POST", "/api/providers/codex-acp/install", None).await;
    let before = wait_status(&crew, "connected").await;
    std::fs::write(&marker, "slow").unwrap();
    crew.ok("POST", "/api/providers/codex-acp/check", None).await;
    let started = std::time::Instant::now();
    let after = loop {
        let view = chatgpt(&crew).await;
        if view["status"] != "checking" { break view; }
        assert!(started.elapsed().as_secs() < 80, "probe did not finish: {view}");
        tokio::time::sleep(std::time::Duration::from_millis(200)).await;
    };
    assert_eq!(after["status"], "connected", "{after}");
    assert_eq!(after["models"], before["models"]);
    assert!(after["error"].as_str().unwrap_or_default().contains("последнее известное состояние"));
    crew.ok("PATCH", "/api/providers/codex-acp", Some(json!({ "default": true }))).await;
    let state = crew.ok("GET", "/api/state", None).await;
    let bot = state["bots"][0]["id"].as_str().unwrap().to_string();
    crew.ok("PATCH", &format!("/api/bots/{bot}"), Some(json!({ "model": "fake-2" }))).await;
    crew.clear_events();
    crew.say(&bot, "hello").await;
    let done = crew.wait_event("turn after slow probe", |e| e["type"] == "turn_end").await;
    assert_eq!(done["failed"], false, "{done}");
}

#[tokio::test]
async fn sign_in_waits_for_the_connect_button() {
    let (crew, log) = start("prov-auth", true, true).await;
    crew.ok("POST", "/api/providers/codex-acp/install", None).await;
    let view = wait_status(&crew, "not_connected").await;
    assert!(view["account"].is_null(), "{view}");
    assert!(!methods(&log).iter().any(|m| m["method"] == "authenticate"), "без кнопки входа не просим");
    let (status, _) = crew.send("PATCH", "/api/providers/codex-acp", Some(json!({ "default": true }))).await;
    assert_eq!(status, 200, "сделать основным можно, выбор входа — дело человека");

    crew.ok("POST", "/api/providers/codex-acp/connect", None).await;
    wait_status(&crew, "connected").await;
    assert!(methods(&log).iter().any(|m| m["method"] == "authenticate"));
}

#[tokio::test]
async fn a_tampered_archive_is_not_installed() {
    let (crew, _) = start("prov-sha", false, false).await;
    crew.ok("POST", "/api/providers/codex-acp/install", None).await;
    let view = wait_status(&crew, "error").await;
    assert!(view["error"].as_str().unwrap().contains("контрольной суммой"), "{view}");
    assert!(view["installed"].is_null());
    assert!(!crew.data_dir.join("agents").join("codex-acp").join("fake_acp.exe").exists());
}

#[tokio::test]
async fn search_covers_the_whole_registry_and_unsupported_agents_say_so() {
    let (crew, _) = start("prov-search", true, false).await;
    let found = crew.ok("GET", "/api/providers?q=py", None).await;
    let agent = found["providers"].as_array().unwrap().iter().find(|p| p["id"] == "py-agent").cloned().expect("поиск не нашёл");
    assert_eq!(agent["supported"], false);
    let (status, body) = crew.send("POST", "/api/providers/py-agent/install", None).await;
    assert_eq!(status, 200, "{body}");
    let started = std::time::Instant::now();
    loop {
        let view = crew.ok("GET", "/api/providers/py-agent", None).await;
        if view["status"] == "error" {
            assert!(view["error"].as_str().unwrap().contains("Python"), "{view}");
            break;
        }
        assert!(started.elapsed().as_secs() < 10, "{view}");
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
}

#[tokio::test]
async fn removing_the_agent_clears_it() {
    let (crew, _) = start("prov-remove", true, false).await;
    crew.ok("POST", "/api/providers/codex-acp/install", None).await;
    wait_status(&crew, "connected").await;
    let view = crew.ok("DELETE", "/api/providers/codex-acp", None).await;
    assert_eq!(view["status"], "not_installed");
    assert!(!crew.data_dir.join("agents").join("codex-acp").exists());
    let (status, _) = crew.send("POST", "/api/providers/..%5Cevil/install", None).await;
    assert_ne!(status, 200);
}

// ---------------------------------------------------------------------------
// Свой адрес, поиск серверов, свой агент, лимиты
// ---------------------------------------------------------------------------

#[derive(Clone, Default)]
struct Mock {
    auth: Arc<Mutex<Vec<String>>>,
}

async fn mock_models(State(mock): State<Mock>, headers: HeaderMap) -> Response {
    let auth = headers.get("authorization").and_then(|v| v.to_str().ok()).unwrap_or_default().to_string();
    mock.auth.lock().unwrap().push(auth.clone());
    if auth == "Bearer bad-key" {
        return Response::builder().status(401).body(Body::from("{}")).unwrap();
    }
    Response::builder()
        .header("content-type", "application/json")
        .body(Body::from(json!({ "data": [{ "id": "mock-1" }, { "id": "mock-2" }] }).to_string()))
        .unwrap()
}

async fn mock_chat(State(mock): State<Mock>, headers: HeaderMap, body: Bytes) -> Response {
    let auth = headers.get("authorization").and_then(|v| v.to_str().ok()).unwrap_or_default().to_string();
    mock.auth.lock().unwrap().push(auth);
    let payload: Value = serde_json::from_slice(&body).unwrap_or_default();
    let text = format!("модель {}", payload["model"].as_str().unwrap_or_default());
    let chunk = json!({ "choices": [{ "index": 0, "delta": { "role": "assistant", "content": text } }] });
    let done = json!({ "choices": [{ "index": 0, "delta": {}, "finish_reason": "stop" }] });
    Response::builder()
        .header("content-type", "text/event-stream")
        .body(Body::from(format!("data: {chunk}\n\ndata: {done}\n\ndata: [DONE]\n\n")))
        .unwrap()
}

async fn mock_server() -> (String, Mock) {
    let mock = Mock::default();
    let app = Router::new()
        .route("/v1/models", get(mock_models))
        .route("/v1/chat/completions", post(mock_chat))
        .with_state(mock.clone());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}/v1", listener.local_addr().unwrap());
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    (url, mock)
}

#[tokio::test]
async fn an_own_openai_address_checks_the_key_and_answers() {
    let (url, mock) = mock_server().await;
    let (crew, _) = start("prov-own", true, false).await;
    let (status, body) = crew.send("POST", "/api/providers", Some(json!({ "kind": "openai", "name": "Мой", "baseUrl": url, "apiKey": "bad-key" }))).await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(body["status"], "error", "плохой ключ: {body}");
    let id = body["id"].as_str().unwrap().to_string();

    let view = crew.ok("PATCH", &format!("/api/providers/{id}"), Some(json!({ "apiKey": "k1", "model": "mock-2" }))).await;
    assert_eq!(view["status"], "connected", "{view}");
    let models: Vec<&str> = view["models"].as_array().unwrap().iter().filter_map(|m| m["id"].as_str()).collect();
    assert_eq!(models, ["mock-1", "mock-2"]);
    assert_eq!(view["hasKey"], true);
    let secrets = common::read_secrets(&crew.data_dir);
    assert!(secrets.contains("k1"), "ключ не в secrets.json");
    let saved = std::fs::read_to_string(crew.data_dir.join("providers.json")).unwrap();
    assert!(!saved.contains("k1"), "ключ попал в providers.json");

    crew.ok("PATCH", &format!("/api/providers/{id}"), Some(json!({ "default": true }))).await;
    let state = crew.ok("GET", "/api/state", None).await;
    assert_eq!(state["settings"]["driver"], format!("openai:{id}"));
    let bot = state["bots"][0]["id"].as_str().unwrap().to_string();
    crew.clear_events();
    crew.say(&bot, "привет").await;
    let done = crew.wait_event("конец хода", |e| e["type"] == "turn_end").await;
    assert_eq!(done["failed"], false, "{:?}", crew.find_event(|e| e["type"] == "error"));
    let thread = crew.ok("GET", &format!("/api/bots/{bot}/thread?limit=4"), None).await;
    let answer = thread["messages"].as_array().unwrap().iter().rev().find(|m| m["role"] == "assistant").unwrap()["text"].clone();
    assert_eq!(answer, "модель mock-2");
    assert_eq!(mock.auth.lock().unwrap().last().unwrap(), "Bearer k1", "ключ ушёл не тот");

    let removed = crew.ok("DELETE", &format!("/api/providers/{id}"), None).await;
    assert_eq!(removed["removed"], true);
    let secrets = common::read_secrets(&crew.data_dir);
    assert!(!secrets.contains("k1"), "ключ остался после удаления");
}

#[tokio::test]
async fn scanning_finds_listed_model_servers() {
    let (url, _) = mock_server().await;
    let (crew, _) = start("prov-scan", true, false).await;
    crew.ok("PUT", "/api/settings", Some(json!({ "modelServers": [url] }))).await;
    let found = crew.ok("POST", "/api/providers/scan", None).await;
    let id = found["found"].as_array().unwrap().iter().filter_map(Value::as_str).find(|id| id.starts_with("server-")).expect("сервер не найден").to_string();
    let list = crew.ok("GET", "/api/providers", None).await;
    let server = list["providers"].as_array().unwrap().iter().find(|p| p["id"] == id.as_str()).cloned().expect("нет в списке");
    assert_eq!(server["status"], "connected");
    assert_eq!(server["local"], true);
    assert_eq!(server["needsKey"], false);
    assert_eq!(server["models"].as_array().unwrap().len(), 2);
}

#[tokio::test]
async fn an_own_acp_agent_runs_from_its_command() {
    let (crew, _) = start("prov-own-acp", true, false).await;
    let fake = Path::new(BIN).parent().unwrap().join("examples").join("fake_acp.exe");
    let view = crew.ok("POST", "/api/providers", Some(json!({ "kind": "acp", "name": "Мой агент", "command": format!("\"{}\"", fake.display()) }))).await;
    let id = view["id"].as_str().unwrap().to_string();
    assert_eq!(view["custom"], true);
    crew.ok("POST", &format!("/api/providers/{id}/check"), None).await;
    let started = std::time::Instant::now();
    loop {
        let view = crew.ok("GET", &format!("/api/providers/{id}"), None).await;
        if view["status"] == "connected" {
            break;
        }
        assert!(started.elapsed().as_secs() < 20, "{view}");
        tokio::time::sleep(std::time::Duration::from_millis(150)).await;
    }
    let removed = crew.ok("DELETE", &format!("/api/providers/{id}"), None).await;
    assert_eq!(removed["removed"], true);
}

#[tokio::test]
async fn subscription_limits_come_from_the_agent_status() {
    let (crew, log) = start("prov-limits", true, false).await;
    crew.ok("POST", "/api/providers/codex-acp/install", None).await;
    let view = wait_status(&crew, "connected").await;
    let limits = view["limits"].as_array().cloned().unwrap_or_default();
    assert_eq!(limits.len(), 2, "{view}");
    assert_eq!(limits[0]["left"], 86);
    // Пока ни один бот не пользуется агентом, кружка нет смысла показывать… но лимиты уже известны.
    let all = crew.ok("GET", "/api/providers/limits", None).await;
    let chatgpt = all["providers"].as_array().unwrap().iter().find(|p| p["id"] == "codex-acp").cloned().expect("нет лимитов");
    assert_eq!(chatgpt["name"], "ChatGPT");
    // `/status` не тратит модель: обычных сообщений агент не получал.
    let prompts: Vec<Value> = methods(&log).into_iter().filter(|m| m["method"] == "session/prompt").collect();
    assert!(prompts.iter().all(|p| p["params"]["prompt"][0]["text"] == "/status"), "{prompts:?}");
}

// ---------------------------------------------------------------------------
// Управляемый Node.js для npm-агентов
// ---------------------------------------------------------------------------

// Управляемый Node.js (заглушка дистрибутива `node.exe`) собирается только на Windows.
#[cfg(windows)]
use common::node_dist;

fn saved_provider(crew: &Crew, id: &str) -> Value {
    let text = std::fs::read_to_string(crew.data_dir.join("providers.json")).unwrap();
    serde_json::from_str::<Value>(&text).unwrap()[id].clone()
}

fn fake_log(path: &Path) -> Vec<Value> {
    std::fs::read_to_string(path).unwrap_or_default().lines().filter_map(|l| serde_json::from_str(l).ok()).collect()
}

async fn wait_provider(crew: &Crew, id: &str, done: impl Fn(&Value) -> bool, secs: u64) -> Value {
    let started = std::time::Instant::now();
    loop {
        let view = crew.ok("GET", &format!("/api/providers/{id}"), None).await;
        if done(&view) {
            return view;
        }
        assert!(started.elapsed().as_secs() < secs, "не дождался {id}: {view}");
        tokio::time::sleep(std::time::Duration::from_millis(150)).await;
    }
}

#[cfg(windows)]
fn crew_with_node_dist(registry: &Path, dist: &node_dist::Dist, path: String, fake_log: &Path) -> Vec<(String, String)> {
    if let Some(parent) = fake_log.parent() {
        std::fs::create_dir_all(parent).unwrap();
    }
    vec![
        ("CREW_TEST_ACP_REGISTRY".to_string(), registry.display().to_string()),
        ("CREW_TEST_NODE_DIST".to_string(), dist.base.clone()),
        ("PATH".to_string(), path),
        ("FAKE_NODE_LOG".to_string(), fake_log.display().to_string()),
    ]
}

#[cfg(windows)]
fn node_test_registry(name: &str) -> PathBuf {
    let dir = common::temp_data_dir(name);
    registry(&dir, true)
}

#[cfg(windows)]
async fn start_node_crew(name: &str, env: Vec<(String, String)>) -> Crew {
    let pairs: Vec<(&str, &str)> = env.iter().map(|(k, v)| (k.as_str(), v.as_str())).collect();
    let crew = Crew::start_with_env(name, &pairs).await;
    crew.listen().await;
    crew
}

#[cfg(windows)]
#[tokio::test]
async fn an_npm_agent_is_installed_and_launched_with_the_managed_node() {
    let dist = node_dist::start("v26.13.1").await;
    let log = common::temp_data_dir("prov-node-managed-log").join("fake-node.log");
    let registry = node_test_registry("prov-node-managed-registry");
    let env = crew_with_node_dist(&registry, &dist, node_dist::without_node(), &log);
    let crew = start_node_crew("prov-node-managed", env).await;

    crew.ok("POST", "/api/providers/fake-npm/install", None).await;
    let view = wait_provider(&crew, "fake-npm", |v| v["installed"] == "1.0.0", 30).await;
    assert_eq!(view["installed"], "1.0.0", "{view}");

    let node = crew.data_dir.join("runtimes").join("node").join("v26.13.1").join("node.exe");
    assert!(node.is_file(), "нет управляемого node.exe");
    assert!(crew.data_dir.join("runtimes").join("node").join("v26.13.1").join("deloist-install.json").is_file());
    let versions: Vec<_> = std::fs::read_dir(crew.data_dir.join("runtimes").join("node")).unwrap().flatten().filter(|e| e.path().is_dir()).collect();
    assert_eq!(versions.len(), 1, "должна остаться одна версия Node");
    let command = saved_provider(&crew, "fake-npm")["command"].as_str().unwrap_or_default().to_string();
    let node_text = node.display().to_string();
    assert!(command.starts_with(&node_text) || command.starts_with(&format!("\"{node_text}\"")), "агент стартует не управляемым node: {command} | node={node_text:?} len {} vs {}", command.len(), node_text.len());
    assert!(command.contains("fake-agent") && command.contains("agent.js"), "{command}");
    assert_eq!(dist.hits("v26.13.1/node-v26.13.1-win-x64.zip"), 1, "Node скачался не один раз");

    let started = std::time::Instant::now();
    loop {
        if fake_log(&log).iter().any(|e| e["kind"] == "agent" && e["args"][0].as_str().is_some_and(|a| a.ends_with("agent.js"))) {
            break;
        }
        assert!(started.elapsed().as_secs() < 20, "агент не запускался: {:?}", fake_log(&log));
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
}

#[cfg(windows)]
#[tokio::test]
async fn a_wrong_node_checksum_is_refused_and_nothing_is_installed() {
    let dist = node_dist::start("v26.13.1").await;
    dist.set_shasums(Some(format!("{}  node-v26.13.1-win-x64.zip\n", "0".repeat(64))));
    let log = common::temp_data_dir("prov-node-sha-log").join("fake-node.log");
    let registry = node_test_registry("prov-node-sha-registry");
    let env = crew_with_node_dist(&registry, &dist, node_dist::without_node(), &log);
    let crew = start_node_crew("prov-node-sha", env).await;

    crew.ok("POST", "/api/providers/fake-npm/install", None).await;
    let view = wait_provider(&crew, "fake-npm", |v| v["status"] == "error", 30).await;
    let error = view["error"].as_str().unwrap_or_default();
    assert!(error.contains("контрольная сумма"), "{view}");
    assert!(error.contains("скачать его не удалось"), "нет причины отказа: {view}");
    assert!(!error.contains("nodejs.org"), "остался тупик «поставь Node с nodejs.org»: {view}");
    assert!(!crew.data_dir.join("runtimes").join("node").join("v26.13.1").join("node.exe").exists());
    assert!(!crew.data_dir.join("agents/fake-npm").exists());
}

#[cfg(windows)]
#[tokio::test]
async fn a_missing_shasums_line_is_refused() {
    let dist = node_dist::start("v26.13.1").await;
    dist.set_shasums(Some(format!("{}  node-v26.13.1-linux-x64.tar.gz\n", "a".repeat(64))));
    let log = common::temp_data_dir("prov-node-nosha-log").join("fake-node.log");
    let registry = node_test_registry("prov-node-nosha-registry");
    let env = crew_with_node_dist(&registry, &dist, node_dist::without_node(), &log);
    let crew = start_node_crew("prov-node-nosha", env).await;

    crew.ok("POST", "/api/providers/fake-npm/install", None).await;
    let view = wait_provider(&crew, "fake-npm", |v| v["status"] == "error", 30).await;
    let error = view["error"].as_str().unwrap_or_default();
    assert!(error.contains("SHASUMS256.txt"), "{view}");
    assert!(!crew.data_dir.join("runtimes").join("node").join("v26.13.1").join("node.exe").exists());
    assert!(!crew.data_dir.join("agents/fake-npm").exists());
}

#[cfg(windows)]
#[tokio::test]
async fn a_tampered_managed_node_is_downloaded_again() {
    let dist = node_dist::start("v26.13.1").await;
    let log = common::temp_data_dir("prov-node-tamper-log").join("fake-node.log");
    let registry = node_test_registry("prov-node-tamper-registry");
    let env = crew_with_node_dist(&registry, &dist, node_dist::without_node(), &log);
    let crew = start_node_crew("prov-node-tamper", env).await;

    crew.ok("POST", "/api/providers/fake-npm/install", None).await;
    wait_provider(&crew, "fake-npm", |v| v["installed"] == "1.0.0", 30).await;
    // Ждём конца пробного запуска: повторная установка иначе получит «уже идёт».
    wait_provider(&crew, "fake-npm", |v| v["status"] == "error", 30).await;
    let node = crew.data_dir.join("runtimes").join("node").join("v26.13.1").join("node.exe");
    let original = std::fs::read(&node).unwrap();
    std::fs::write(&node, b"tampered").unwrap();
    let before = dist.hits("v26.13.1/node-v26.13.1-win-x64.zip");

    crew.ok("POST", "/api/providers/fake-npm/install", None).await;
    let started = std::time::Instant::now();
    loop {
        let restored = std::fs::read(&node).ok();
        if dist.hits("v26.13.1/node-v26.13.1-win-x64.zip") > before && restored.as_deref().is_some_and(|b| b != b"tampered") {
            break;
        }
        assert!(started.elapsed().as_secs() < 30, "подменённый node не скачался заново");
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
    let restored = std::fs::read(&node).unwrap();
    assert_ne!(restored, b"tampered");
    assert_eq!(restored, original, "node.exe не восстановился из архива");
}

#[tokio::test]
async fn a_broken_update_keeps_the_working_agent() {
    let dir = common::temp_data_dir("prov-update-broken-registry");
    let path = registry_ver(&dir, "1.0.0", true);
    let registry_path = path.display().to_string();
    let crew = Crew::start_with_env("prov-update-broken", &[("CREW_TEST_ACP_REGISTRY", &registry_path)]).await;
    crew.listen().await;
    crew.ok("POST", "/api/providers/codex-acp/install", None).await;
    let view = wait_status(&crew, "connected").await;
    assert_eq!(view["installed"], "1.0.0", "{view}");

    // Реестр теперь предлагает версию 2 с неверной суммой.
    registry_ver(&dir, "2.0.0", false);
    crew.ok("POST", "/api/providers/codex-acp/install", None).await;

    let started = std::time::Instant::now();
    loop {
        let view = chatgpt(&crew).await;
        if view["error"].as_str().map(|s| s.contains("контрольной")).unwrap_or(false) {
            break;
        }
        assert!(started.elapsed().as_secs() < 30, "{view}");
        tokio::time::sleep(std::time::Duration::from_millis(150)).await;
    }
    let view = chatgpt(&crew).await;
    assert_eq!(view["installed"], "1.0.0", "{view}");

    let agents_dir = crew.data_dir.join("agents");
    assert!(agents_dir.join("codex-acp").join("fake_acp.exe").exists());
    assert!(!agents_dir.join("codex-acp.staging").exists(), "остался staging");
    assert!(!agents_dir.join("codex-acp.old").exists(), "остался old");
}

#[tokio::test]
async fn a_good_update_replaces_the_files_and_leaves_no_leftovers() {
    let dir = common::temp_data_dir("prov-update-good-registry");
    let path = registry_ver(&dir, "1.0.0", true);
    let registry_path = path.display().to_string();
    let crew = Crew::start_with_env("prov-update-good", &[("CREW_TEST_ACP_REGISTRY", &registry_path)]).await;
    crew.listen().await;
    crew.ok("POST", "/api/providers/codex-acp/install", None).await;
    wait_status(&crew, "connected").await;

    registry_ver(&dir, "2.0.0", true);
    crew.ok("POST", "/api/providers/codex-acp/install", None).await;

    let started = std::time::Instant::now();
    loop {
        let view = chatgpt(&crew).await;
        if view["installed"] == "2.0.0" {
            break;
        }
        assert!(started.elapsed().as_secs() < 30, "{view}");
        tokio::time::sleep(std::time::Duration::from_millis(150)).await;
    }
    let view = chatgpt(&crew).await;
    assert_eq!(view["installed"], "2.0.0", "{view}");

    let agents_dir = crew.data_dir.join("agents");
    assert!(agents_dir.join("codex-acp").join("fake_acp.exe").exists());
    assert!(!agents_dir.join("codex-acp.staging").exists(), "остался staging");
    // Версия записывается сразу после распаковки, а `.old` убирается после проверки агента: ждём её.
    common::wait_until("старая папка убрана", 30, || async { !agents_dir.join("codex-acp.old").exists() }).await;
}

#[tokio::test]
async fn the_hourly_check_shows_a_new_version_without_a_click() {
    let dir = common::temp_data_dir("prov-update-hourly-registry");
    let path = registry_ver(&dir, "1.0.0", true);
    let registry_path = path.display().to_string();
    let crew = Crew::start_with_env("prov-update-hourly", &[
        ("CREW_TEST_ACP_REGISTRY", &registry_path),
        ("CREW_TEST_PROVIDER_REFRESH_MS", "200"),
    ]).await;
    crew.listen().await;
    crew.ok("POST", "/api/providers/codex-acp/install", None).await;
    wait_status(&crew, "connected").await;

    // Реестр теперь предлагает версию 2 — без клика «обновить».
    registry_ver(&dir, "2.0.0", true);

    let started = std::time::Instant::now();
    loop {
        let list = crew.ok("GET", "/api/providers", None).await;
        let provider = list["providers"].as_array().unwrap().iter().find(|p| p["id"] == "codex-acp").cloned();
        if provider.as_ref().map(|p| p["update"] == true).unwrap_or(false) {
            break;
        }
        assert!(started.elapsed().as_secs() < 10, "{list}");
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
}

#[cfg(windows)]
#[tokio::test]
async fn a_system_node_means_no_download() {
    let dist = node_dist::start("v26.13.1").await;
    let system = node_dist::fake_system_node("prov-node-system-bin");
    let log = common::temp_data_dir("prov-node-system-log").join("fake-node.log");
    let registry = node_test_registry("prov-node-system-registry");
    let env = crew_with_node_dist(&registry, &dist, node_dist::with_node(&system), &log);
    let crew = start_node_crew("prov-node-system", env).await;

    crew.ok("POST", "/api/providers/fake-npm/install", None).await;
    let view = wait_provider(&crew, "fake-npm", |v| v["installed"] == "1.0.0", 30).await;
    assert_eq!(view["installed"], "1.0.0", "{view}");
    assert_eq!(dist.total_hits(), 0, "при системном Node ничего не скачиваем");
    assert!(!crew.data_dir.join("runtimes/node").exists());
    let command = saved_provider(&crew, "fake-npm")["command"].as_str().unwrap_or_default().to_string();
    let system_node = system.join("node.exe").display().to_string();
    assert!(command.starts_with(&system_node) || command.starts_with(&format!("\"{system_node}\"")), "{command}");
}

