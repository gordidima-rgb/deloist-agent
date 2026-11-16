//! Подключения из каталога на настоящем exe: согласие, секреты вне state.json, env и заголовки,
//! проверка хоста, вход через OAuth с заглушкой провайдера (PKCE, state, повтор кода, обновление)
//! и MCP OAuth с регистрацией клиента.

use crate::common;

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use axum::body::{Body, Bytes};
use axum::extract::{RawQuery, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::Router;
use common::Crew;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

fn has_node() -> Option<PathBuf> {
    let name = if cfg!(windows) { "node.exe" } else { "node" };
    let path = std::env::var_os("PATH")?;
    std::env::split_paths(&path).map(|dir| dir.join(name)).find(|candidate| candidate.is_file())
}

/// Заглушка stdio-сервера: инструмент называется по тому, дошёл ли ключ через env и не попал ли в argv.
const ENV_MCP: &str = r#"
const want = 'sk-live-0123456789';
let buf = '';
const send = (m) => process.stdout.write(JSON.stringify(m) + '\n');
process.stdin.on('data', (d) => {
  buf += d;
  let i;
  while ((i = buf.indexOf('\n')) >= 0) {
    const line = buf.slice(0, i).trim();
    buf = buf.slice(i + 1);
    if (!line) continue;
    let msg; try { msg = JSON.parse(line); } catch { continue; }
    if (msg.id === undefined) continue;
    if (msg.method === 'initialize') {
      send({ jsonrpc: '2.0', id: msg.id, result: { protocolVersion: '2024-11-05', capabilities: { tools: {} }, serverInfo: { name: 'env', version: '1' } } });
    } else if (msg.method === 'tools/list') {
      const inEnv = process.env.DEMO_TOKEN === want;
      const inArgs = process.argv.join(' ').includes('sk-live');
      const name = inEnv && !inArgs ? 'token_ok' : (inArgs ? 'token_in_args' : 'token_missing');
      send({ jsonrpc: '2.0', id: msg.id, result: { tools: [{ name, inputSchema: { type: 'object' } }] } });
    } else {
      send({ jsonrpc: '2.0', id: msg.id, error: { code: -32601, message: 'нет метода' } });
    }
  }
});
"#;

const TOKEN: &str = "sk-live-0123456789";

fn all_files(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            all_files(&path, out);
        } else {
            out.push(path);
        }
    }
}

/// Нигде в папке данных секрет не лежит открытым текстом.
fn assert_not_on_disk(dir: &Path, secret: &str) {
    let mut files = Vec::new();
    all_files(dir, &mut files);
    assert!(!files.is_empty());
    for file in files {
        let bytes = std::fs::read(&file).unwrap_or_default();
        assert!(!bytes.windows(secret.len()).any(|w| w == secret.as_bytes()), "секрет лежит открыто в {}", file.display());
    }
}

async fn text(crew: &Crew, path: &str) -> String {
    crew.client.get(format!("{}{path}", crew.base)).header("cookie", &crew.cookie).send().await.unwrap().text().await.unwrap()
}

fn stdio_manifest(node: &Path, script: &Path) -> Value {
    json!({
        "schema": 1,
        "id": "app.demo",
        "kind": "app",
        "version": "1.0.0",
        "title": { "ru": "Демо" },
        "app": {
            "transport": "stdio",
            "install": { "kind": "none" },
            "command": node.to_string_lossy(),
            "args": [script.to_string_lossy()],
            "env": { "DEMO_TOKEN": "{{secret:token}}" },
            "egress": ["api.example.com"],
            "auth": { "kind": "fields", "fields": [{ "name": "token", "label": { "ru": "Токен" }, "help": "https://example.com/where" }] },
        },
    })
}

#[tokio::test]
async fn a_stdio_connector_needs_consent_and_gets_its_key_through_env_only() {
    let Some(node) = has_node() else {
        eprintln!("node не найден: проверка пропущена");
        return;
    };
    let crew = Crew::start("conn-stdio").await;
    crew.listen().await;
    // Скрипт вне папки данных: в нём самом записан ожидаемый ключ.
    let scripts = common::temp_data_dir("conn-stdio-script");
    std::fs::create_dir_all(&scripts).unwrap();
    let script = scripts.join("env_mcp.mjs");
    std::fs::write(&script, ENV_MCP).unwrap();

    let installed = crew.ok("POST", "/api/connectors/install", Some(json!({ "manifest": stdio_manifest(&node, &script) }))).await;
    let consent = &installed["consent"];
    assert_eq!(consent["id"], "demo");
    assert_eq!(consent["secrets"], json!([{ "name": "token", "set": false }]));
    assert_eq!(consent["env"], json!(["DEMO_TOKEN"]));
    assert_eq!(consent["envValues"]["DEMO_TOKEN"], "{{secret:token}}");
    assert_eq!(consent["warnings"][0]["code"], "not-pinned");
    assert_eq!(consent["egress"], json!(["api.example.com"]));
    assert_eq!(consent["autoApprove"], false);
    assert_eq!(consent["pinned"], false, "команда без установки не закреплена");
    assert!(consent["command"].as_str().unwrap().contains("env_mcp.mjs"), "{consent}");
    assert_eq!(installed["connector"]["state"], "pending-consent");

    // До согласия сервер не запускается ни для проверки, ни для модели.
    let (status, body) = crew.send("POST", "/api/connectors/demo/check", None).await;
    assert_eq!(status, 409, "{body}");
    let (status, body) = crew.send("POST", "/api/mcp/demo/check", None).await;
    assert_eq!(status, 502, "{body}");
    assert!(body["error"].as_str().unwrap().contains("не подтверждено"), "{body}");

    let confirmed = crew.ok("POST", "/api/connectors/demo/confirm", Some(json!({}))).await;
    assert_eq!(confirmed["connector"]["state"], "needs-auth", "{confirmed}");
    assert_eq!(confirmed["connector"]["autoApprove"], false);
    let (status, body) = crew.send("POST", "/api/connectors/demo/check", None).await;
    assert_eq!(status, 502);
    assert!(body["error"].as_str().unwrap().contains("нет секрета token"), "{body}");

    let (status, body) = crew.send("POST", "/api/connectors/demo/secret", Some(json!({ "name": "other", "value": "x" }))).await;
    assert_eq!(status, 400, "чужое имя: {body}");
    let (status, _) = crew.send("POST", "/api/connectors/demo/secret", Some(json!({ "name": "oauth.access_token", "value": "x" }))).await;
    assert_eq!(status, 400, "имена oauth.* задаёт только вход");
    let set = crew.ok("POST", "/api/connectors/demo/secret", Some(json!({ "name": "token", "value": TOKEN }))).await;
    assert_eq!(set["connector"]["secrets"][0]["set"], true);
    assert!(!set.to_string().contains(TOKEN));

    let checked = crew.ok("POST", "/api/connectors/demo/check", None).await;
    assert_eq!(checked["tools"], json!(["token_ok"]), "ключ дошёл через env и не попал в argv");
    assert_eq!(checked["connector"]["state"], "ok");
    assert_eq!(checked["connector"]["tools"], 1);

    let listed = crew.ok("GET", "/api/connectors", None).await;
    assert_eq!(listed["connectors"].as_array().unwrap().len(), 1);

    // Каталожный сервер не правится и не удаляется через /api/mcp.
    let (status, _) = crew.send("PATCH", "/api/mcp/demo", Some(json!({ "command": "cmd" }))).await;
    assert_eq!(status, 409);
    let (status, _) = crew.send("DELETE", "/api/mcp/demo", None).await;
    assert_eq!(status, 409);

    // Секрет нигде: ни в state.json, ни в других файлах, ни в API, ни в событиях.
    let state_json = std::fs::read_to_string(crew.data_dir.join("state.json")).unwrap();
    assert!(state_json.contains("{{secret:token}}"), "в state.json ссылка, а не значение");
    assert_not_on_disk(&crew.data_dir, TOKEN);
    for path in ["/api/state", "/api/connectors"] {
        assert!(!text(&crew, path).await.contains(TOKEN), "секрет в {path}");
    }
    let events = serde_json::to_string(&*crew.events.lock().unwrap()).unwrap();
    assert!(events.contains("\"connector\""), "события о подключении есть");
    assert!(!events.contains(TOKEN), "секрет в потоке событий");

    crew.ok("DELETE", "/api/connectors/demo", None).await;
    assert_eq!(crew.ok("GET", "/api/connectors", None).await["connectors"], json!([]));
    let (status, _) = crew.send("POST", "/api/connectors/demo/secret", Some(json!({ "name": "token", "value": "y" }))).await;
    assert_eq!(status, 404);
    let _ = std::fs::remove_dir_all(scripts);
}

#[cfg(windows)]
#[tokio::test]
async fn an_npx_connector_uses_the_managed_node_when_there_is_no_system_npx() {
    let dist = common::node_dist::start("v26.13.1").await;
    let path = common::node_dist::without_node();
    let pairs = [("CREW_TEST_NODE_DIST", dist.base.as_str()), ("PATH", path.as_str())];
    let crew = Crew::start_with_env("conn-managed-node", &pairs).await;
    let manifest = json!({
        "schema": 1,
        "id": "app.demo",
        "kind": "app",
        "version": "1.0.0",
        "title": "Демо",
        "app": {
            "transport": "stdio",
            "install": { "kind": "npx", "package": "fake-mcp", "pin": "1.0.0" },
            "auth": { "kind": "none" }
        }
    });
    let installed = crew.ok("POST", "/api/connectors/install", Some(json!({ "manifest": manifest }))).await;
    assert_eq!(installed["consent"]["runtime"]["found"], false);
    assert!(!installed["consent"]["runtime"]["help"].as_str().unwrap_or_default().contains("nodejs.org"));

    crew.ok("POST", "/api/connectors/demo/confirm", None).await;
    let node = crew.data_dir.join("runtimes").join("node").join("v26.13.1").join("node.exe");
    assert!(node.is_file(), "управляемый Node не скачался");
    let state = crew.ok("GET", "/api/state", None).await;
    let server = state["mcp"].as_array().unwrap().iter().find(|s| s["id"] == "demo").unwrap().clone();
    assert_eq!(server["command"].as_str(), Some(&*node.to_string_lossy()), "{server}");
    let args = server["args"].as_array().unwrap();
    assert!(args[0].as_str().unwrap_or_default().ends_with("npx-cli.js"), "{server}");
    assert_eq!(dist.hits("v26.13.1/node-v26.13.1-win-x64.zip"), 1);
}

#[tokio::test]
async fn bad_manifests_are_refused() {
    let crew = Crew::start("conn-bad").await;
    let base = |install: Value| json!({ "manifest": { "id": "app.x", "kind": "app", "version": "1.0.0", "app": { "install": install } } });
    for (install, needle) in [
        (json!({ "kind": "npx", "package": "srv", "pin": "latest" }), "@latest"),
        (json!({ "kind": "npx", "package": "srv@latest", "pin": "1.0.0" }), "pin"),
        (json!({ "kind": "uvx", "package": "srv", "pin": ">=1.0" }), "точная"),
        (json!({ "kind": "binary", "url": "https://example.com/x.exe", "size": 10, "sha256": "zz", "file": "x.exe" }), "sha256"),
    ] {
        let (status, body) = crew.send("POST", "/api/connectors/install", Some(base(install.clone()))).await;
        assert_eq!(status, 400, "{install}: {body}");
        assert!(body["error"].as_str().unwrap().contains(needle), "{install}: {body}");
    }
    // Секрет в аргументах обычного MCP-сервера тоже не принимаем.
    let (status, body) = crew.send("POST", "/api/mcp", Some(json!({ "command": "srv", "args": ["--key={{secret:k}}"] }))).await;
    assert_eq!(status, 400, "{body}");
    let (status, body) = crew.send("POST", "/api/mcp", Some(json!({ "command": "srv", "env": { "PATH": "C:\\evil" } }))).await;
    assert_eq!(status, 400, "{body}");
    let made = crew.ok("POST", "/api/mcp", Some(json!({ "command": "srv", "env": { "K": "{{secret:k}}", "PLAIN": "typed-by-hand" } }))).await;
    let id = made["server"]["id"].as_str().unwrap().to_string();
    let state = crew.ok("GET", "/api/state", None).await;
    let server = state["mcp"].as_array().unwrap().iter().find(|s| s["id"] == id.as_str()).unwrap().clone();
    assert_eq!(server["env"]["K"], "{{secret:k}}");
    assert_eq!(server["env"]["PLAIN"], "•••", "вписанное руками значение не показываем");
}

// ---------------------------------------------------------------------------
// HTTP: заглушка MCP и OAuth-провайдера в одном сервере
// ---------------------------------------------------------------------------

#[derive(Default)]
struct Fake {
    port: u16,
    /// Какой заголовок Authorization принимает MCP. Пусто — любой.
    accept_auth: String,
    /// На чужой ключ отвечать 400 и повторять его в теле: клиент не должен нести его дальше.
    echo_bad_key: bool,
    /// code → (challenge, redirect_uri, client_id)
    codes: HashMap<String, (String, String, String)>,
    used: Vec<String>,
    token_calls: usize,
    refresh_calls: usize,
    verifier_ok: bool,
    expires_in: i64,
    fail_refresh: bool,
    issued: usize,
    registered: Vec<Value>,
    resource_seen: String,
    /// Сервер авторизации без `registration_endpoint`.
    no_registration: bool,
    /// 401 с `WWW-Authenticate: Bearer resource_metadata=…`; стандартный адрес метаданных при этом молчит.
    challenge: bool,
    meta_hits: usize,
    secret_seen: String,
    /// Подмена в метаданных: `resource` защищаемого ресурса, `issuer`, хост адреса токенов.
    prm_resource: String,
    issuer: String,
    token_host: String,
    /// Метаданные без `code_challenge_methods_supported`.
    no_pkce: bool,
}

type Shared = Arc<Mutex<Fake>>;

fn b64url(bytes: &[u8]) -> String {
    const ABC: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";
    let mut out = String::new();
    for chunk in bytes.chunks(3) {
        let n = (chunk[0] as u32) << 16 | (*chunk.get(1).unwrap_or(&0) as u32) << 8 | *chunk.get(2).unwrap_or(&0) as u32;
        for i in 0..chunk.len() + 1 {
            out.push(ABC[(n >> (18 - 6 * i) & 63) as usize] as char);
        }
    }
    out
}

fn pairs(text: &str) -> HashMap<String, String> {
    reqwest::Url::parse(&format!("http://x/?{text}")).unwrap().query_pairs().map(|(k, v)| (k.into_owned(), v.into_owned())).collect()
}

async fn authorize(State(fake): State<Shared>, RawQuery(query): RawQuery) -> Response {
    let q = pairs(&query.unwrap_or_default());
    let mut f = fake.lock().unwrap();
    f.issued += 1;
    let code = format!("code-{}", f.issued);
    if let Some(resource) = q.get("resource") {
        f.resource_seen = resource.clone();
    }
    assert_eq!(q.get("code_challenge_method").map(String::as_str), Some("S256"));
    f.codes.insert(code.clone(), (q["code_challenge"].clone(), q["redirect_uri"].clone(), q["client_id"].clone()));
    let location = format!("{}?code={code}&state={}", q["redirect_uri"], q["state"]);
    Response::builder().status(302).header("location", location).body(Body::empty()).unwrap()
}

async fn token(State(fake): State<Shared>, body: Bytes) -> Response {
    let form = pairs(&String::from_utf8_lossy(&body));
    let mut f = fake.lock().unwrap();
    f.token_calls += 1;
    let grant = form.get("grant_type").cloned().unwrap_or_default();
    if grant == "refresh_token" {
        f.refresh_calls += 1;
        if f.fail_refresh {
            return (StatusCode::BAD_REQUEST, axum::Json(json!({ "error": "invalid_grant" }))).into_response();
        }
    } else {
        let code = form.get("code").cloned().unwrap_or_default();
        if f.used.contains(&code) {
            return (StatusCode::BAD_REQUEST, axum::Json(json!({ "error": "invalid_grant" }))).into_response();
        }
        let Some((challenge, redirect, client)) = f.codes.get(&code).cloned() else {
            return (StatusCode::BAD_REQUEST, axum::Json(json!({ "error": "invalid_grant" }))).into_response();
        };
        f.used.push(code);
        let verifier = form.get("code_verifier").cloned().unwrap_or_default();
        f.verifier_ok = b64url(&Sha256::digest(verifier.as_bytes())) == challenge && form.get("redirect_uri") == Some(&redirect) && form.get("client_id") == Some(&client);
        if !f.verifier_ok {
            return (StatusCode::BAD_REQUEST, axum::Json(json!({ "error": "invalid_grant" }))).into_response();
        }
    }
    f.secret_seen = form.get("client_secret").cloned().unwrap_or_default();
    let n = f.token_calls;
    f.accept_auth = format!("Bearer at-{n}");
    axum::Json(json!({ "access_token": format!("at-{n}"), "refresh_token": format!("rt-{n}"), "expires_in": f.expires_in, "token_type": "Bearer" })).into_response()
}

async fn register(State(fake): State<Shared>, body: Bytes) -> Response {
    let value: Value = serde_json::from_slice(&body).unwrap_or(Value::Null);
    fake.lock().unwrap().registered.push(value);
    (StatusCode::CREATED, axum::Json(json!({ "client_id": "dyn-client-1" }))).into_response()
}

async fn custom_meta(State(fake): State<Shared>) -> Response {
    fake.lock().unwrap().meta_hits += 1;
    resource_meta_body(fake)
}

async fn resource_meta(State(fake): State<Shared>) -> Response {
    if fake.lock().unwrap().challenge {
        return StatusCode::NOT_FOUND.into_response();
    }
    resource_meta_body(fake)
}

fn resource_meta_body(fake: Shared) -> Response {
    let port = fake.lock().unwrap().port;
    let resource = Some(fake.lock().unwrap().prm_resource.clone()).filter(|r| !r.is_empty()).unwrap_or(format!("http://127.0.0.1:{port}/mcp"));
    axum::Json(json!({ "resource": resource, "authorization_servers": [format!("http://127.0.0.1:{port}")] })).into_response()
}

async fn as_meta(State(fake): State<Shared>) -> Response {
    let port = fake.lock().unwrap().port;
    let base = format!("http://127.0.0.1:{port}");
    let mut meta = json!({
        "issuer": base,
        "authorization_endpoint": format!("{base}/authorize"),
        "token_endpoint": format!("{base}/token"),
        "registration_endpoint": format!("{base}/register"),
        "code_challenge_methods_supported": ["S256"],
    });
    if fake.lock().unwrap().no_registration {
        meta.as_object_mut().unwrap().remove("registration_endpoint");
    }
    let (issuer, token_host, no_pkce) = {
        let f = fake.lock().unwrap();
        (f.issuer.clone(), f.token_host.clone(), f.no_pkce)
    };
    if !issuer.is_empty() {
        meta["issuer"] = json!(issuer);
    }
    if !token_host.is_empty() {
        meta["token_endpoint"] = json!(format!("http://{token_host}:{port}/token"));
    }
    if no_pkce {
        meta.as_object_mut().unwrap().remove("code_challenge_methods_supported");
    }
    axum::Json(meta).into_response()
}

async fn mcp(State(fake): State<Shared>, headers: HeaderMap, body: Bytes) -> Response {
    let want = fake.lock().unwrap().accept_auth.clone();
    let got = headers.get("authorization").and_then(|v| v.to_str().ok()).unwrap_or("");
    if want.is_empty() || got != want {
        if fake.lock().unwrap().echo_bad_key {
            return (StatusCode::BAD_REQUEST, format!("bad key: {got}")).into_response();
        }
        if fake.lock().unwrap().challenge {
            let port = fake.lock().unwrap().port;
            let header = format!("Bearer realm=\"mcp\", resource_metadata=\"http://127.0.0.1:{port}/custom-meta\"");
            return (StatusCode::UNAUTHORIZED, [("www-authenticate", header)], "нужен ключ").into_response();
        }
        return (StatusCode::UNAUTHORIZED, "нужен ключ").into_response();
    }
    let message: Value = serde_json::from_slice(&body).unwrap_or(Value::Null);
    let Some(id) = message.get("id").cloned() else {
        return StatusCode::ACCEPTED.into_response();
    };
    let result = match message["method"].as_str().unwrap_or_default() {
        "initialize" => json!({ "protocolVersion": "2025-03-26", "capabilities": { "tools": {} }, "serverInfo": { "name": "fake", "version": "1" } }),
        "tools/list" => json!({ "tools": [{ "name": "echo", "inputSchema": { "type": "object" } }] }),
        _ => json!({}),
    };
    Response::builder()
        .header("content-type", "application/json")
        .header("mcp-session-id", "s-1")
        .body(Body::from(json!({ "jsonrpc": "2.0", "id": id, "result": result }).to_string()))
        .unwrap()
}

async fn start_fake() -> Shared {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let fake: Shared = Arc::new(Mutex::new(Fake { port, expires_in: 3600, ..Default::default() }));
    let router = Router::new()
        .route("/authorize", get(authorize))
        .route("/token", post(token))
        .route("/register", post(register))
        .route("/.well-known/oauth-protected-resource/mcp", get(resource_meta))
        .route("/.well-known/oauth-authorization-server", get(as_meta))
        .route("/custom-meta", get(custom_meta))
        .route("/mcp", post(mcp))
        .with_state(fake.clone());
    tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
    fake
}

fn http_manifest(port: u16, auth: Value, headers: Value, host: &str) -> Value {
    json!({
        "schema": 1,
        "id": "app.web",
        "kind": "app",
        "version": "2.0.0",
        "title": "Веб",
        "app": { "transport": "http", "url": format!("http://{host}:{port}/mcp"), "egress": ["127.0.0.1"], "headers": headers, "auth": auth },
    })
}

#[tokio::test]
async fn an_http_connector_sends_its_key_in_a_header_and_only_to_egress_hosts() {
    let fake = start_fake().await;
    let port = fake.lock().unwrap().port;
    fake.lock().unwrap().accept_auth = "Token sk-hdr-42".into();
    let crew = Crew::start("conn-http").await;

    let auth = json!({ "kind": "fields", "fields": [{ "name": "key" }] });
    let (status, body) = crew.send("POST", "/api/connectors/install", Some(json!({ "manifest": http_manifest(port, auth.clone(), json!({ "Authorization": "Token {{secret:key}}" }), "localhost") }))).await;
    assert_eq!(status, 400, "localhost нет в egress: {body}");
    assert!(body["error"].as_str().unwrap().contains("egress"), "{body}");

    let installed = crew.ok("POST", "/api/connectors/install", Some(json!({ "manifest": http_manifest(port, auth, json!({ "Authorization": "Token {{secret:key}}" }), "127.0.0.1") }))).await;
    assert_eq!(installed["consent"]["headers"], json!(["Authorization"]));
    assert_eq!(installed["consent"]["runtime"], Value::Null);
    crew.ok("POST", "/api/connectors/web/confirm", None).await;
    crew.ok("POST", "/api/connectors/web/secret", Some(json!({ "name": "key", "value": "sk-hdr-42" }))).await;
    let checked = crew.ok("POST", "/api/connectors/web/check", None).await;
    assert_eq!(checked["tools"], json!(["echo"]));

    // Неверный ключ: сервер отвечает 401, подключение в ошибке, ключ в тексте ошибки не светится.
    crew.ok("POST", "/api/connectors/web/secret", Some(json!({ "name": "key", "value": "sk-wrong-77" }))).await;
    let (status, body) = crew.send("POST", "/api/connectors/web/check", None).await;
    assert_eq!(status, 502);
    assert!(!body.to_string().contains("sk-wrong-77"), "{body}");
    let listed = crew.ok("GET", "/api/connectors", None).await;
    assert_eq!(listed["connectors"][0]["state"], "error", "{listed}");
    assert_not_on_disk(&crew.data_dir, "sk-wrong-77");

    // Сервер повторил ключ в ответе: в ошибке, в карточке и в событиях его нет.
    crew.listen().await;
    fake.lock().unwrap().echo_bad_key = true;
    let (status, body) = crew.send("POST", "/api/connectors/web/check", None).await;
    assert_eq!(status, 502);
    assert!(body["error"].as_str().unwrap().contains("bad key"), "{body}");
    assert!(!body.to_string().contains("sk-wrong-77"), "{body}");
    assert!(!text(&crew, "/api/connectors").await.contains("sk-wrong-77"));
    let events = serde_json::to_string(&*crew.events.lock().unwrap()).unwrap();
    assert!(!events.contains("sk-wrong-77"), "ключ в событиях");
}

/// Пройти «браузером» по ссылке входа: провайдер редиректит на обратный адрес приложения.
async fn follow_login(url: &str) -> String {
    let client = reqwest::Client::builder().no_proxy().redirect(reqwest::redirect::Policy::none()).build().unwrap();
    let response = client.get(url).send().await.unwrap();
    assert_eq!(response.status().as_u16(), 302);
    response.headers()["location"].to_str().unwrap().to_string()
}

#[tokio::test]
async fn oauth_login_checks_pkce_and_state_and_refuses_a_reused_code() {
    let fake = start_fake().await;
    let port = fake.lock().unwrap().port;
    let crew = Crew::start("conn-oauth").await;
    crew.listen().await;
    let auth = json!({ "kind": "oauth", "oauth": {
        "authorizeUrl": format!("http://127.0.0.1:{port}/authorize"),
        "tokenUrl": format!("http://127.0.0.1:{port}/token"),
        "clientId": "test-client",
        "scopes": ["read", "write"],
    } });
    let installed = crew.ok("POST", "/api/connectors/install", Some(json!({ "manifest": http_manifest(port, auth, json!({}), "127.0.0.1") }))).await;
    assert_eq!(installed["consent"]["headers"], json!(["Authorization"]), "OAuth по HTTP сам добавляет Bearer");
    assert_eq!(installed["consent"]["oauth"]["hosts"], json!(["127.0.0.1"]), "хосты входа в карточке согласия");
    assert_eq!(installed["consent"]["oauth"]["scopes"], json!(["read", "write"]));

    let (status, _) = crew.send("POST", "/api/connectors/web/oauth/start", Some(json!({ "open": false }))).await;
    assert_eq!(status, 409, "вход только после согласия");
    crew.ok("POST", "/api/connectors/web/confirm", None).await;
    assert_eq!(crew.ok("GET", "/api/connectors", None).await["connectors"][0]["state"], "needs-auth");

    let started = crew.ok("POST", "/api/connectors/web/oauth/start", Some(json!({ "open": false }))).await;
    let url = started["url"].as_str().unwrap();
    let query = pairs(reqwest::Url::parse(url).unwrap().query().unwrap());
    assert_eq!(query["client_id"], "test-client");
    assert_eq!(query["scope"], "read write");
    assert!(query["redirect_uri"].starts_with("http://127.0.0.1:") && query["redirect_uri"].ends_with("/cb"));

    let callback = follow_login(url).await;
    let loopback = reqwest::Client::builder().no_proxy().build().unwrap();
    // Чужой state отклоняется, а вход продолжает ждать настоящий ответ.
    let forged = callback.replace(&format!("state={}", query["state"]), "state=forged");
    assert_eq!(loopback.get(&forged).send().await.unwrap().status().as_u16(), 400);
    let done = loopback.get(&callback).send().await.unwrap();
    assert_eq!(done.status().as_u16(), 200);
    crew.wait_event("вход завершён", |e| e["type"] == "connector" && e["state"] == "ok").await;
    {
        let f = fake.lock().unwrap();
        assert!(f.verifier_ok, "PKCE: verifier сошёлся с challenge");
        assert_eq!(f.token_calls, 1);
    }
    // Тот же код второй раз: обратный адрес уже закрыт.
    let again = loopback.get(&callback).send().await;
    assert!(again.is_err() || again.unwrap().status().as_u16() != 200, "повторный код принят");
    assert_eq!(fake.lock().unwrap().token_calls, 1, "повторный код не дошёл до обмена");

    let checked = crew.ok("POST", "/api/connectors/web/check", None).await;
    assert_eq!(checked["tools"], json!(["echo"]));
    let view = &checked["connector"];
    assert_eq!(view["auth"]["connected"], true);
    assert_eq!(view["state"], "ok");
    assert_not_on_disk(&crew.data_dir, "at-1");
    assert_not_on_disk(&crew.data_dir, "rt-1");
    assert!(!text(&crew, "/api/connectors").await.contains("at-1"));
    assert!(!text(&crew, "/api/state").await.contains("at-1"));
    let events = serde_json::to_string(&*crew.events.lock().unwrap()).unwrap();
    assert!(!events.contains("at-1") && !events.contains("rt-1"));

    // Токен истекает: перед запуском обновляется сам. Обновление не прошло — «Переподключить».
    fake.lock().unwrap().expires_in = 30;
    let started = crew.ok("POST", "/api/connectors/web/oauth/start", Some(json!({ "open": false }))).await;
    let callback = follow_login(started["url"].as_str().unwrap()).await;
    assert_eq!(loopback.get(&callback).send().await.unwrap().status().as_u16(), 200);
    crew.clear_events();
    crew.wait_event("второй вход", |e| e["type"] == "connector" && e["state"] == "ok").await;
    crew.ok("POST", "/api/connectors/web/check", None).await;
    assert_eq!(fake.lock().unwrap().refresh_calls, 1, "истекающий токен обновлён перед запуском");

    fake.lock().unwrap().fail_refresh = true;
    let (status, body) = crew.send("POST", "/api/connectors/web/check", None).await;
    assert_eq!(status, 502, "{body}");
    assert!(body["error"].as_str().unwrap().contains("Переподключить"), "{body}");
    assert_eq!(crew.ok("GET", "/api/connectors", None).await["connectors"][0]["state"], "needs-auth");
}

#[tokio::test]
async fn mcp_oauth_finds_the_server_and_registers_a_client() {
    let fake = start_fake().await;
    let port = fake.lock().unwrap().port;
    let crew = Crew::start("conn-dcr").await;
    crew.listen().await;
    let installed = crew.ok("POST", "/api/connectors/install", Some(json!({ "manifest": http_manifest(port, json!({ "kind": "mcp-oauth" }), json!({}), "127.0.0.1") }))).await;
    assert_eq!(installed["consent"]["oauth"]["discovery"], "127.0.0.1", "где искать сервер входа — в карточке");
    crew.ok("POST", "/api/connectors/web/confirm", None).await;
    let started = crew.ok("POST", "/api/connectors/web/oauth/start", Some(json!({ "open": false }))).await;
    let url = started["url"].as_str().unwrap();
    let query = pairs(reqwest::Url::parse(url).unwrap().query().unwrap());
    assert_eq!(query["client_id"], "dyn-client-1", "клиент зарегистрирован на лету");
    assert_eq!(query["resource"], format!("http://127.0.0.1:{port}/mcp"));
    {
        let f = fake.lock().unwrap();
        assert_eq!(f.registered.len(), 1);
        assert_eq!(f.registered[0]["redirect_uris"][0], query["redirect_uri"]);
        assert_eq!(f.registered[0]["token_endpoint_auth_method"], "none");
    }
    let callback = follow_login(url).await;
    let loopback = reqwest::Client::builder().no_proxy().build().unwrap();
    assert_eq!(loopback.get(&callback).send().await.unwrap().status().as_u16(), 200);
    crew.wait_event("вход MCP OAuth", |e| e["type"] == "connector" && e["state"] == "ok").await;
    assert!(fake.lock().unwrap().verifier_ok);
    let checked = crew.ok("POST", "/api/connectors/web/check", None).await;
    assert_eq!(checked["tools"], json!(["echo"]));

    // Второй вход берёт того же клиента, если его порт свободен.
    crew.ok("POST", "/api/connectors/web/oauth/start", Some(json!({ "open": false }))).await;
    assert_eq!(fake.lock().unwrap().registered.len(), 1, "клиент не регистрируется заново");
}

/// Пройти вход «браузером» и дождаться, пока приложение сохранит токен.
async fn finish_login(crew: &Crew, url: &str) {
    let callback = follow_login(url).await;
    let loopback = reqwest::Client::builder().no_proxy().build().unwrap();
    assert_eq!(loopback.get(&callback).send().await.unwrap().status().as_u16(), 200);
    crew.wait_event("вход", |e| e["type"] == "connector" && e["state"] == "ok").await;
}

#[tokio::test]
async fn a_custom_server_asking_for_login_uses_the_resource_metadata_of_its_401() {
    let fake = start_fake().await;
    let port = fake.lock().unwrap().port;
    fake.lock().unwrap().challenge = true;
    let crew = Crew::start("mcp-401").await;
    crew.listen().await;
    let made = crew.ok("POST", "/api/mcp", Some(json!({ "name": "Свой", "url": format!("http://127.0.0.1:{port}/mcp") }))).await;
    let id = made["server"]["id"].as_str().unwrap().to_string();

    // Первый запрос без токена: 401 — сервер помечен «нужно войти», а не просто ошибкой.
    let (status, body) = crew.send("POST", &format!("/api/mcp/{id}/check"), None).await;
    assert_eq!(status, 502, "{body}");
    assert!(body["error"].as_str().unwrap().contains("нужно войти"), "{body}");
    let state = crew.ok("GET", "/api/state", None).await;
    assert_eq!(state["mcp"][0]["login"]["state"], "needs-login", "{state}");
    assert_eq!(fake.lock().unwrap().meta_hits, 0, "без клика в сеть за входом не ходим");

    // «Войти»: адрес метаданных берётся из заголовка, стандартный .well-known молчит.
    let started = crew.ok("POST", &format!("/api/mcp/{id}/oauth/start"), Some(json!({ "open": false }))).await;
    assert!(fake.lock().unwrap().meta_hits >= 1, "resource_metadata из WWW-Authenticate не использован");
    let url = started["url"].as_str().unwrap();
    let query = pairs(reqwest::Url::parse(url).unwrap().query().unwrap());
    assert_eq!(query["client_id"], "dyn-client-1");
    assert_eq!(query["resource"], format!("http://127.0.0.1:{port}/mcp"));
    finish_login(&crew, url).await;

    let checked = crew.ok("POST", &format!("/api/mcp/{id}/check"), None).await;
    assert_eq!(checked["tools"], json!(["echo"]));
    let state = crew.ok("GET", "/api/state", None).await;
    assert_eq!(state["mcp"][0]["login"]["state"], "connected", "{state}");
    // Токен только в хранилище: ни в ответах, ни в событиях, ни на диске открыто.
    let all = format!("{}{}{}", state, started, text(&crew, "/api/state").await);
    let events = serde_json::to_string(&*crew.events.lock().unwrap()).unwrap();
    for secret in ["at-1", "rt-1"] {
        assert!(!all.contains(secret) && !events.contains(secret), "{secret} утёк");
        assert_not_on_disk(&crew.data_dir, secret);
    }
}

#[tokio::test]
async fn without_a_registration_endpoint_the_human_gives_a_client_id() {
    let fake = start_fake().await;
    let port = fake.lock().unwrap().port;
    fake.lock().unwrap().no_registration = true;
    let crew = Crew::start("mcp-manual-client").await;
    crew.listen().await;
    let made = crew.ok("POST", "/api/mcp", Some(json!({ "name": "Свой", "url": format!("http://127.0.0.1:{port}/mcp") }))).await;
    let id = made["server"]["id"].as_str().unwrap().to_string();

    let (status, body) = crew.send("POST", &format!("/api/mcp/{id}/oauth/start"), Some(json!({ "open": false }))).await;
    assert_eq!(status, 409, "{body}");
    assert_eq!(body["needsClientId"], true, "{body}");
    assert_eq!(body["authHost"], format!("127.0.0.1:{port}"), "хост входа с портом");
    assert!(fake.lock().unwrap().registered.is_empty());

    let started = crew.ok("POST", &format!("/api/mcp/{id}/oauth/start"), Some(json!({ "open": false, "clientId": "my-client", "clientSecret": "shh-secret-71" }))).await;
    let url = started["url"].as_str().unwrap();
    assert_eq!(pairs(reqwest::Url::parse(url).unwrap().query().unwrap())["client_id"], "my-client");
    assert!(!started.to_string().contains("shh-secret-71"));
    finish_login(&crew, url).await;
    assert_eq!(fake.lock().unwrap().secret_seen, "shh-secret-71", "секрет клиента ушёл серверу токенов");
    assert!(fake.lock().unwrap().registered.is_empty(), "регистрироваться не пытались");
    assert_eq!(crew.ok("POST", &format!("/api/mcp/{id}/check"), None).await["tools"], json!(["echo"]));
    let shown = text(&crew, "/api/state").await;
    assert!(!shown.contains("shh-secret-71") && !shown.contains("at-1"), "секрет в ответе");
    assert_not_on_disk(&crew.data_dir, "shh-secret-71");
    assert_not_on_disk(&crew.data_dir, "at-1");

    // Второй вход берёт сохранённый client id без вопросов.
    let again = crew.ok("POST", &format!("/api/mcp/{id}/oauth/start"), Some(json!({ "open": false }))).await;
    assert_eq!(pairs(reqwest::Url::parse(again["url"].as_str().unwrap()).unwrap().query().unwrap())["client_id"], "my-client");
}

/// Вход со свеженастроенной заглушкой: ответ на «Войти» и признак «клиент не регистрировался».
async fn custom_login_error(fake: &Shared, name: &str, body: Value) -> (u16, Value, Crew, String) {
    let port = fake.lock().unwrap().port;
    let crew = Crew::start(name).await;
    crew.listen().await;
    let made = crew.ok("POST", "/api/mcp", Some(json!({ "name": "Свой", "url": format!("http://127.0.0.1:{port}/mcp") }))).await;
    let id = made["server"]["id"].as_str().unwrap().to_string();
    let (status, answer) = crew.send("POST", &format!("/api/mcp/{id}/oauth/start"), Some(body)).await;
    (status, answer, crew, id)
}

#[tokio::test]
async fn login_metadata_for_another_resource_or_issuer_is_refused() {
    // Метаданные защищаемого ресурса описывают другой сервер.
    let fake = start_fake().await;
    fake.lock().unwrap().prm_resource = "https://other.example/mcp".into();
    let (status, body, _crew, _) = custom_login_error(&fake, "mcp-prm-mismatch", json!({ "open": false })).await;
    assert_eq!(status, 502, "{body}");
    assert!(body["error"].as_str().unwrap().contains("другой ресурс"), "{body}");
    assert!(fake.lock().unwrap().registered.is_empty(), "клиент не регистрировали");

    // Издатель не совпадает с адресом, по которому нашли метаданные.
    let fake = start_fake().await;
    fake.lock().unwrap().issuer = "https://evil.example".into();
    let (status, body, _crew, _) = custom_login_error(&fake, "mcp-issuer-mismatch", json!({ "open": false })).await;
    assert_eq!(status, 502, "{body}");
    assert!(body["error"].as_str().unwrap().contains("издатель"), "{body}");
    assert!(fake.lock().unwrap().registered.is_empty());

    // Нет объявления PKCE.
    let fake = start_fake().await;
    fake.lock().unwrap().no_pkce = true;
    let (status, body, _crew, _) = custom_login_error(&fake, "mcp-no-pkce", json!({ "open": false })).await;
    assert_eq!(status, 502, "{body}");
    assert!(body["error"].as_str().unwrap().contains("PKCE"), "{body}");
}

#[tokio::test]
async fn a_custom_server_whose_token_endpoint_is_on_another_host_is_refused() {
    let fake = start_fake().await;
    fake.lock().unwrap().token_host = "localhost".into();
    let (status, body, _crew, _) = custom_login_error(&fake, "mcp-token-host", json!({ "open": false })).await;
    assert_eq!(status, 502, "{body}");
    let error = body["error"].as_str().unwrap();
    assert!(error.contains("разных хостах") && error.contains("localhost") && error.contains("127.0.0.1"), "{body}");
    assert!(fake.lock().unwrap().registered.is_empty(), "до регистрации и секрета дело не дошло");
}

#[tokio::test]
async fn a_hint_on_the_own_host_with_another_port_is_not_exempt_and_a_bad_return_port_is_400() {
    let fake = start_fake().await;
    let port = fake.lock().unwrap().port;
    fake.lock().unwrap().challenge = true;
    let crew = Crew::start("mcp-hint-port").await;
    let made = crew.ok("POST", "/api/mcp", Some(json!({ "name": "Свой", "url": format!("http://127.0.0.1:{port}/mcp") }))).await;
    let id = made["server"]["id"].as_str().unwrap().to_string();
    let (status, _) = crew.send("POST", &format!("/api/mcp/{id}/check"), None).await;
    assert_eq!(status, 502);
    assert_eq!(fake.lock().unwrap().meta_hits, 0);

    // Порт возврата: не число, мал, велик — 400, а не «любой порт».
    for bad in [json!("abc"), json!(80), json!(70000), json!(-1), json!(0)] {
        let (status, body) = crew.send("POST", &format!("/api/mcp/{id}/oauth/start"), Some(json!({ "open": false, "clientId": "c", "redirectPort": bad.clone() }))).await;
        assert_eq!(status, 400, "{bad}: {body}");
    }
    assert!(fake.lock().unwrap().registered.is_empty());
}

#[tokio::test]
async fn a_typed_client_secret_is_not_stored_when_the_start_fails() {
    let fake = start_fake().await;
    fake.lock().unwrap().no_registration = true;
    fake.lock().unwrap().no_pkce = true;
    let (status, body, crew, id) =
        custom_login_error(&fake, "mcp-secret-not-saved", json!({ "open": false, "clientId": "my-client", "clientSecret": "shh-typed-91" })).await;
    assert_eq!(status, 502, "{body}");
    assert_not_on_disk(&crew.data_dir, "shh-typed-91");
    let path = format!("/api/mcp/{id}/oauth/start");
    // Вход без секрета: введённый при неудачной попытке до сервера токенов не доходит (его нигде нет).
    fake.lock().unwrap().no_pkce = false;
    let first = crew.ok("POST", &path, Some(json!({ "open": false, "clientId": "my-client" }))).await;
    assert_eq!(first["authHost"], format!("127.0.0.1:{}", fake.lock().unwrap().port), "куда откроется вход — в ответе");
    finish_login(&crew, first["url"].as_str().unwrap()).await;
    assert_eq!(fake.lock().unwrap().secret_seen, "", "секрет из неудачной попытки не сохранён");
    // Успешный вход с секретом сохраняет его: следующий вход берёт его сам.
    let typed = crew.ok("POST", &path, Some(json!({ "open": false, "clientId": "my-client", "clientSecret": "shh-typed-91" }))).await;
    finish_login(&crew, typed["url"].as_str().unwrap()).await;
    assert_eq!(fake.lock().unwrap().secret_seen, "shh-typed-91");
    // Ответ браузеру уходит раньше, чем вход дописывает секрет: даём ему закончиться.
    tokio::time::sleep(std::time::Duration::from_millis(500)).await;
    let again = crew.ok("POST", &path, Some(json!({ "open": false }))).await;
    finish_login(&crew, again["url"].as_str().unwrap()).await;
    assert_eq!(fake.lock().unwrap().secret_seen, "shh-typed-91", "после входа секрет в хранилище");
}

#[tokio::test]
async fn a_manifest_may_reference_only_the_access_token_among_the_oauth_secrets() {
    let fake = start_fake().await;
    let port = fake.lock().unwrap().port;
    let crew = Crew::start("conn-oauth-refs").await;
    let (status, body) = crew
        .send(
            "POST",
            "/api/connectors/install",
            Some(json!({ "manifest": http_manifest(port, json!({ "kind": "mcp-oauth" }), json!({ "X-Leak": "{{secret:oauth.refresh_token}}" }), "127.0.0.1") })),
        )
        .await;
    assert_eq!(status, 400, "{body}");
}

#[tokio::test]
async fn a_manual_client_is_refused_for_a_stdio_server() {
    let crew = Crew::start("mcp-oauth-stdio").await;
    let made = crew.ok("POST", "/api/mcp", Some(json!({ "command": "srv" }))).await;
    let id = made["server"]["id"].as_str().unwrap().to_string();
    let (status, _) = crew.send("POST", &format!("/api/mcp/{id}/oauth/start"), Some(json!({ "open": false, "clientId": "x" }))).await;
    assert_eq!(status, 400);
}

#[tokio::test]
async fn secrets_json_holds_no_plaintext_value_on_disk() {
    const KEY: &str = "sk-disk-marker-8f3a91";
    let crew = Crew::start("secrets-no-plain").await;
    crew.ok("PUT", "/api/settings", Some(json!({ "apiKey": KEY }))).await;
    let raw = std::fs::read(crew.data_dir.join("secrets.json")).expect("secrets.json не записан");
    assert!(raw.starts_with(common::SEALED_MAGIC), "нет заголовка зашифрованного файла");
    assert!(!raw.windows(KEY.len()).any(|w| w == KEY.as_bytes()), "значение лежит в secrets.json открыто");
    assert!(common::read_secrets(&crew.data_dir).contains(KEY), "ключ не расшифровался");
    // Мастер-ключ не лежит в папке данных.
    assert!(!crew.data_dir.join("secrets.key").exists());
    assert!(!common::key_file_for(&crew.data_dir).starts_with(&crew.data_dir));
}

#[cfg(not(windows))]
#[tokio::test]
async fn migrating_a_plaintext_secrets_json_leaves_no_plaintext_under_data_dir() {
    const KEY: &str = "sk-legacy-plain-marker-5521";
    let data = common::temp_data_dir("migrate-plain");
    std::fs::create_dir_all(data.join("secrets")).unwrap();
    std::fs::write(data.join("secrets.json"), format!("{{\"providerKeys\":{{\"openrouter\":\"{KEY}\"}}}}")).unwrap();
    // Остаток прежней версии вне Windows: открытый connectors.bin.
    std::fs::write(data.join("secrets").join("connectors.bin"), format!("{{\"srv\":{{\"token\":\"{KEY}\"}}}}")).unwrap();
    let crew = Crew::start_at(data.clone(), &[]).await;
    crew.ok("PUT", "/api/settings", Some(json!({ "apiKey": "sk-other-1" }))).await;
    assert_not_on_disk(&crew.data_dir, KEY);
    assert!(!crew.data_dir.join("secrets.json.v1").exists(), "открытая копия .v1 не пишется вне Windows");
    assert!(!crew.data_dir.join("secrets.json.migrating").exists());
    assert!(common::read_secrets(&crew.data_dir).contains(KEY), "перенесённый ключ потерян");
}
