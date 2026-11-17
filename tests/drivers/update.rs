//! Обновления: ставится только подписанное и более новое, работа ботов не обрывается.

use crate::common;

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use axum::extract::State;
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use axum::Router;
use common::{Crew, BIN};
use ed25519_dalek::{Signer, SigningKey};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

const SEED: [u8; 32] = [7; 32];
const NEW_EXE: &[u8] = b"MZ new version";

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

#[derive(Clone)]
struct Host {
    manifest: Vec<u8>,
    signature: String,
    exe: Vec<u8>,
}

async fn manifest(State(host): State<Host>) -> Response {
    host.manifest.into_response()
}
async fn signature(State(host): State<Host>) -> Response {
    host.signature.into_response()
}
async fn exe(State(host): State<Host>) -> Response {
    host.exe.into_response()
}

/// Подставной сервер выпусков. `sign_with` выбирает ключ подписи, `exe` то, что он реально отдаёт.
async fn release_host(version: &str, sign_with: [u8; 32], declared: &[u8], served: &[u8]) -> String {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let text = serde_json::to_string(&json!({
        "version": version,
        "notes": "заметки",
        "url": format!("http://127.0.0.1:{port}/app.bin"),
        "sha256": hex(&Sha256::digest(declared)),
        "size": served.len(),
    }))
    .unwrap();
    let signature = hex(&SigningKey::from_bytes(&sign_with).sign(text.as_bytes()).to_bytes());
    let host = Host { manifest: text.into_bytes(), signature, exe: served.to_vec() };
    let router = Router::new()
        .route("/latest.json", get(manifest))
        .route("/latest.json.sig", get(signature_route()))
        .route("/app.bin", get(exe))
        .with_state(host);
    tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
    format!("http://127.0.0.1:{port}/latest.json")
}

fn signature_route() -> axum::routing::MethodRouter<Host> {
    get(signature)
}

struct Fixture {
    crew: Crew,
    target: PathBuf,
}

async fn start(name: &str, url: &str, extra: &[(&str, &str)]) -> Fixture {
    let dir = common::temp_data_dir(name);
    std::fs::create_dir_all(&dir).unwrap();
    let target = dir.join("target.exe");
    std::fs::write(&target, b"MZ old version").unwrap();
    let key = hex(SigningKey::from_bytes(&SEED).verifying_key().as_bytes());
    let target_text = target.display().to_string();
    let mut env: Vec<(&str, &str)> = vec![
        ("CREW_TEST_UPDATE_URL", url),
        ("CREW_TEST_UPDATE_KEY", &key),
        ("CREW_TEST_UPDATE_EXE", &target_text),
    ];
    env.extend_from_slice(extra);
    let crew = Crew::start_with_env(name, &env).await;
    Fixture { crew, target }
}

async fn wait_state(crew: &Crew, state: &str) -> Value {
    let started = Instant::now();
    loop {
        let updates = crew.ok("GET", "/api/updates", None).await;
        if updates["app"]["state"] == state {
            return updates;
        }
        assert!(started.elapsed() < Duration::from_secs(15), "нет состояния {state}: {updates}");
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn a_signed_newer_version_is_offered_and_installed() {
    let url = release_host("99.0.0", SEED, NEW_EXE, NEW_EXE).await;
    let fx = start("update-ok", &url, &[]).await;

    let before = fx.crew.ok("GET", "/api/updates", None).await;
    assert_eq!(before["app"]["state"], "idle", "без проверки ничего не предлагается");
    assert_eq!(before["current"], env!("CARGO_PKG_VERSION"));

    let checked = fx.crew.ok("POST", "/api/updates/check", None).await;
    assert_eq!(checked["app"]["state"], "available", "{checked}");
    assert_eq!(checked["app"]["latest"], "99.0.0");
    assert_eq!(std::fs::read(&fx.target).unwrap(), b"MZ old version", "сама по себе версия не ставится");

    fx.crew.ok("POST", "/api/updates/app", None).await;
    wait_state(&fx.crew, "installed").await;
    assert_eq!(std::fs::read(&fx.target).unwrap(), NEW_EXE);
    assert_eq!(std::fs::read(fx.target.with_extension("old.exe")).unwrap(), b"MZ old version", "старая версия сохранена");
    assert!(!fx.target.with_extension("update.tmp").exists());
}

#[tokio::test(flavor = "multi_thread")]
async fn the_scheduled_check_installs_in_the_background_and_waits_for_a_restart() {
    let url = release_host("99.0.0", SEED, NEW_EXE, NEW_EXE).await;
    let fx = start("update-auto", &url, &[("CREW_TEST_UPDATE_SCHEDULE", "1")]).await;
    let updates = wait_state(&fx.crew, "ready").await;
    assert_eq!(updates["app"]["latest"], "99.0.0");
    assert_eq!(std::fs::read(&fx.target).unwrap(), NEW_EXE, "новая версия уже на месте");
    assert_eq!(std::fs::read(fx.target.with_extension("old.exe")).unwrap(), b"MZ old version");
    // Сама программа не перезапускается, повторная проверка готовое не сбрасывает.
    let checked = fx.crew.ok("POST", "/api/updates/check", None).await;
    assert_eq!(checked["app"]["state"], "ready", "{checked}");
    fx.crew.ok("POST", "/api/updates/restart", None).await;
    wait_state(&fx.crew, "installed").await;
}

#[tokio::test(flavor = "multi_thread")]
async fn the_whole_changelog_is_served_and_images_stay_inside_their_folder() {
    let crew = Crew::start("update-changelog").await;
    let all = crew.ok("GET", "/api/whats-new?all=1", None).await;
    assert_eq!(all["all"], true);
    assert!(all["sections"].as_array().unwrap().len() > 3, "{all}");
    let some = crew.ok("GET", "/api/whats-new?from=0.3.6", None).await;
    assert!(some["sections"].as_array().unwrap().len() < all["sections"].as_array().unwrap().len());
    for name in ["..%2f..%2fCargo.toml", "%2e%2e%5cCargo.toml", "nope.png"] {
        let (status, _) = crew.send("GET", &format!("/api/whats-new/image/{name}"), None).await;
        assert_eq!(status, 404, "{name}");
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn restart_without_a_ready_version_is_refused() {
    let url = release_host("99.0.0", SEED, NEW_EXE, NEW_EXE).await;
    let fx = start("update-norestart", &url, &[]).await;
    let (status, _) = fx.crew.send("POST", "/api/updates/restart", None).await;
    assert_eq!(status, 409);
}

#[tokio::test(flavor = "multi_thread")]
async fn auto_update_off_keeps_the_manual_flow() {
    let url = release_host("99.0.0", SEED, NEW_EXE, NEW_EXE).await;
    let fx = start("update-auto-off", &url, &[("CREW_TEST_UPDATE_SCHEDULE", "4")]).await;
    fx.crew.ok("PUT", "/api/settings", Some(json!({ "autoUpdate": false }))).await;
    // Первая плановая проверка идёт через 4 секунды; ждём, пока она найдёт версию.
    wait_state(&fx.crew, "available").await;
    tokio::time::sleep(Duration::from_millis(500)).await;
    assert_eq!(fx.crew.ok("GET", "/api/updates", None).await["app"]["state"], "available");
    assert_eq!(std::fs::read(&fx.target).unwrap(), b"MZ old version");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_wrong_signature_is_refused_and_shown() {
    let url = release_host("99.0.0", [9; 32], NEW_EXE, NEW_EXE).await;
    let fx = start("update-badsig", &url, &[]).await;
    let checked = fx.crew.ok("POST", "/api/updates/check", None).await;
    assert_eq!(checked["app"]["state"], "error", "{checked}");
    assert!(checked["app"]["error"].as_str().unwrap().contains("подпись"), "{checked}");
    assert_eq!(checked["app"]["latest"], Value::Null);
    let (status, _) = fx.crew.send("POST", "/api/updates/app", None).await;
    assert_eq!(status, 409, "ставить нечего");
    assert_eq!(std::fs::read(&fx.target).unwrap(), b"MZ old version");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_replaced_file_is_refused_even_with_a_good_signature() {
    let served = b"MZ evil version";
    let url = release_host("99.0.0", SEED, NEW_EXE, served).await;
    let fx = start("update-badhash", &url, &[]).await;
    fx.crew.ok("POST", "/api/updates/check", None).await;
    fx.crew.ok("POST", "/api/updates/app", None).await;
    let failed = wait_state(&fx.crew, "error").await;
    assert!(failed["app"]["error"].as_str().unwrap().contains("контрольная сумма"), "{failed}");
    assert_eq!(std::fs::read(&fx.target).unwrap(), b"MZ old version", "подменённый файл не поставлен");
    assert!(!fx.target.with_extension("update.tmp").exists(), "скачанное убрано");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_non_program_is_refused() {
    let junk = b"not a program";
    let url = release_host("99.0.0", SEED, junk, junk).await;
    let fx = start("update-notexe", &url, &[]).await;
    fx.crew.ok("POST", "/api/updates/check", None).await;
    fx.crew.ok("POST", "/api/updates/app", None).await;
    wait_state(&fx.crew, "error").await;
    assert_eq!(std::fs::read(&fx.target).unwrap(), b"MZ old version");
}

#[tokio::test(flavor = "multi_thread")]
async fn an_old_or_equal_version_is_not_offered() {
    for version in [env!("CARGO_PKG_VERSION"), "0.0.1"] {
        let url = release_host(version, SEED, NEW_EXE, NEW_EXE).await;
        let fx = start("update-old", &url, &[]).await;
        let checked = fx.crew.ok("POST", "/api/updates/check", None).await;
        assert_eq!(checked["app"]["state"], "idle", "{version}: {checked}");
        assert_eq!(checked["app"]["latest"], Value::Null);
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn installing_waits_for_the_bots_to_finish() {
    let url = release_host("99.0.0", SEED, NEW_EXE, NEW_EXE).await;
    let fx = start("update-busy", &url, &[("CREW_TEST_TICK_MS", "100")]).await;
    let crew = &fx.crew;
    crew.listen().await;
    let state = crew.ok("GET", "/api/state", None).await;
    let bot = state["bots"][0]["id"].as_str().unwrap().to_string();
    crew.ok("POST", "/api/updates/check", None).await;

    // Строгий режим: демо-команда только читает, а нам нужна висящая карточка.
    crew.ok("PATCH", &format!("/api/bots/{bot}"), Some(json!({ "approval": "strict" }))).await;
    crew.say(&bot, "запусти команду").await;
    let card = crew.wait_event("карточка", |e| e["type"] == "approval").await;

    crew.ok("POST", "/api/updates/app", None).await;
    wait_state(crew, "waiting").await;
    tokio::time::sleep(Duration::from_millis(400)).await;
    assert_eq!(std::fs::read(&fx.target).unwrap(), b"MZ old version", "работа бота не обрывается");

    crew.ok("POST", &format!("/api/approvals/{}", card["id"].as_str().unwrap()), Some(json!({ "allow": false }))).await;
    wait_state(crew, "installed").await;
    assert_eq!(std::fs::read(&fx.target).unwrap(), NEW_EXE);
}

#[tokio::test(flavor = "multi_thread")]
async fn updates_are_closed_without_a_session() {
    let crew = Crew::spawn("update-closed", &[]).await;
    for (method, path) in [("GET", "/api/updates"), ("POST", "/api/updates/check"), ("POST", "/api/updates/app"), ("POST", "/api/updates/tools/claude")] {
        let (status, _) = crew.send(method, path, None).await;
        assert_eq!(status, 403, "{path}");
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn without_an_address_a_check_offers_nothing() {
    let crew = Crew::start("update-noaddr").await;
    let checked = crew.ok("POST", "/api/updates/check", None).await;
    assert_eq!(checked["app"]["state"], "idle", "{checked}");
    assert_eq!(checked["tools"].as_array().unwrap().len(), 0, "в демо-режиме подключённых программ нет");
}

/// Снять копию программы можно только по её имени: родителя она уже сменила.
struct KillByName(&'static str);

impl Drop for KillByName {
    fn drop(&mut self) {
        let _ = std::process::Command::new("taskkill").args(["/F", "/IM", self.0]).output();
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn the_new_file_really_starts_after_the_old_one_quits() {
    const NAME: &str = "deloist-restart-test.exe";
    let _cleanup = KillByName(NAME);
    let bytes = std::fs::read(BIN).unwrap();
    let url = release_host("99.0.0", SEED, &bytes, &bytes).await;
    let dir = common::temp_data_dir("update-restart");
    std::fs::create_dir_all(&dir).unwrap();
    let exe = dir.join(NAME);
    std::fs::copy(BIN, &exe).unwrap();
    let port = common::free_port();
    let key = hex(SigningKey::from_bytes(&SEED).verifying_key().as_bytes());
    let mut first = std::process::Command::new(&exe)
        .arg("--server")
        .env("PORT", port.to_string())
        .env("HOST", "127.0.0.1")
        .env("CREW_DATA_DIR", dir.join("data"))
        .env("CREW_BOOT_TOKEN", common::BOOT)
        .env("CREW_TEST_UPDATE_URL", &url)
        .env("CREW_TEST_UPDATE_KEY", &key)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .unwrap();

    let client = reqwest::Client::builder().no_proxy().redirect(reqwest::redirect::Policy::none()).build().unwrap();
    let base = format!("http://127.0.0.1:{port}");
    // Одноразовая ссылка снова работает только у новой копии: так видно, что сменился процесс.
    let login = || async {
        let res = client.get(format!("{base}/?boot={}", common::BOOT)).send().await.ok()?;
        if res.status().as_u16() != 303 {
            return None;
        }
        Some(res.headers().get("set-cookie")?.to_str().ok()?.split(';').next()?.to_string())
    };
    let started = Instant::now();
    let cookie = loop {
        if let Some(cookie) = login().await {
            break cookie;
        }
        assert!(started.elapsed() < Duration::from_secs(20), "первая копия не запустилась");
        tokio::time::sleep(Duration::from_millis(100)).await;
    };
    for path in ["/api/updates/check", "/api/updates/app"] {
        let res = client.post(format!("{base}{path}")).header("cookie", &cookie).send().await.unwrap();
        assert!(res.status().is_success(), "{path}");
    }

    let started = Instant::now();
    while first.try_wait().unwrap().is_none() {
        assert!(started.elapsed() < Duration::from_secs(30), "старая копия не закрылась");
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    let started = Instant::now();
    while login().await.is_none() {
        assert!(started.elapsed() < Duration::from_secs(30), "новая копия не запустилась");
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    let started = Instant::now();
    while exe.with_extension("old.exe").exists() {
        assert!(started.elapsed() < Duration::from_secs(10), "старый файл остался");
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    assert_eq!(std::fs::read(&exe).unwrap(), bytes);
}

#[tokio::test(flavor = "multi_thread")]
async fn the_check_setting_is_remembered() {
    let crew = Crew::start("update-setting").await;
    let state = crew.ok("GET", "/api/state", None).await;
    assert_eq!(state["settings"]["checkUpdates"], true, "по умолчанию проверка включена");
    crew.ok("PUT", "/api/settings", Some(json!({ "checkUpdates": false }))).await;
    let state = crew.ok("GET", "/api/state", None).await;
    assert_eq!(state["settings"]["checkUpdates"], false);
}

// ---------------------------------------------------------------- подключённые программы

async fn registry_route() -> Response {
    axum::Json(json!({ "version": "2.1.319" })).into_response()
}

async fn fake_registry() -> String {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let router = Router::new().fallback(get(registry_route));
    tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
    format!("http://127.0.0.1:{port}")
}

fn fake_tool(name: &str, file: &str) -> PathBuf {
    let source = Path::new(BIN).parent().unwrap().join("examples").join("fake_cli.exe");
    let dir = common::temp_data_dir(name);
    std::fs::create_dir_all(&dir).unwrap();
    let copy = dir.join(file);
    std::fs::copy(source, &copy).unwrap();
    copy
}

async fn wait_tool(crew: &Crew, state: &str) -> Value {
    let started = Instant::now();
    loop {
        let updates = crew.ok("GET", "/api/updates", None).await;
        if updates["tools"][0]["state"] == state {
            return updates["tools"][0].clone();
        }
        assert!(started.elapsed() < Duration::from_secs(20), "нет состояния {state}: {updates}");
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn a_connected_program_is_offered_and_updated_on_a_click() {
    let registry = fake_registry().await;
    let tool = fake_tool("tool-ok", "claude.exe");
    let crew = Crew::start_with_env("tool-ok", &[("CREW_TEST_NPM_URL", &registry)]).await;
    crew.ok("PUT", "/api/settings", Some(json!({ "driver": "cli", "cliCommand": format!("\"{}\" -p", tool.display()) }))).await;

    let checked = crew.ok("POST", "/api/updates/check", None).await;
    let found = &checked["tools"][0];
    assert_eq!(found["id"], "claude", "{checked}");
    assert_eq!(found["name"], "Claude");
    assert_eq!(found["current"], "2.1.318");
    assert_eq!(found["latest"], "2.1.319");
    assert_eq!(found["state"], "available");
    assert!(!tool.with_file_name("version.txt").exists(), "без нажатия ничего не запускается");

    crew.ok("POST", "/api/updates/tools/claude", None).await;
    let done = wait_tool(&crew, "current").await;
    assert_eq!(done["current"], "2.1.319", "{done}");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_failed_program_update_shows_its_error() {
    let registry = fake_registry().await;
    let tool = fake_tool("tool-broken", "claude.exe");
    std::fs::write(tool.with_file_name("broken.txt"), "1").unwrap();
    let crew = Crew::start_with_env("tool-broken", &[("CREW_TEST_NPM_URL", &registry)]).await;
    crew.ok("PUT", "/api/settings", Some(json!({ "driver": "cli", "cliCommand": format!("\"{}\" -p", tool.display()) }))).await;
    crew.ok("POST", "/api/updates/check", None).await;
    crew.ok("POST", "/api/updates/tools/claude", None).await;
    let failed = wait_tool(&crew, "error").await;
    assert!(failed["error"].as_str().unwrap().contains("сломалось"), "{failed}");
    assert_eq!(failed["current"], "2.1.318");
}

#[tokio::test(flavor = "multi_thread")]
async fn only_known_connected_programs_are_listed() {
    let registry = fake_registry().await;
    let unknown = fake_tool("tool-unknown", "myagent.exe");
    let crew = Crew::start_with_env("tool-unknown", &[("CREW_TEST_NPM_URL", &registry)]).await;
    crew.ok("PUT", "/api/settings", Some(json!({ "driver": "cli", "cliCommand": format!("\"{}\"", unknown.display()) }))).await;
    let checked = crew.ok("POST", "/api/updates/check", None).await;
    assert_eq!(checked["tools"].as_array().unwrap().len(), 0, "{checked}");
    let (status, _) = crew.send("POST", "/api/updates/tools/myagent", None).await;
    assert_eq!(status, 404);

    // Команда с символами оболочки в имени программы не запускается вовсе.
    crew.ok("PUT", "/api/settings", Some(json!({ "driver": "cli", "cliCommand": "claude&calc -p" }))).await;
    let checked = crew.ok("POST", "/api/updates/check", None).await;
    assert_eq!(checked["tools"].as_array().unwrap().len(), 0, "{checked}");
}
