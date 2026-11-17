//! Обновления: новая версия самой программы и подключённые программы вроде Claude и OpenCode.
//! Своя версия ставится только с подписью ключа выпуска: файл на чужом сервере подменить можно,
//! подпись нельзя. Подключённые программы обновляются своей фиксированной командой, строка из
//! настроек нужна лишь чтобы найти программу. Ничего не запускается без нажатия человека.

use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use ed25519_dalek::{Signature, Verifier, VerifyingKey};
use futures_util::StreamExt;
use regex::Regex;
use serde::Deserialize;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use tokio::io::AsyncWriteExt;

use crate::process::shell_command;
use crate::App;

pub const CURRENT: &str = env!("CARGO_PKG_VERSION");

/// Открытая часть ключа выпуска. Закрытая часть хранится только у владельца, в репозиторий не попадает.
const PUBLIC_KEY_HEX: &str = "1305c9147a9320549fd77d4ad8cfb8b958eacadc9d758b4bcd3b09f914b2a834";
const MANIFEST_URL: &str = "https://github.com/gordidima-rgb/deloist-agent/releases/latest/download/latest.json";
const NPM_REGISTRY: &str = "https://registry.npmjs.org";

const CHECK_EVERY: Duration = Duration::from_secs(6 * 3600);
const FIRST_CHECK_AFTER: Duration = Duration::from_secs(30);
const MANIFEST_LIMIT: usize = 64 * 1024;
const SIGNATURE_LIMIT: usize = 1024;
const REGISTRY_LIMIT: usize = 512 * 1024;
const EXE_LIMIT: u64 = 300 * 1024 * 1024;
const TOOL_UPDATE_TIMEOUT: Duration = Duration::from_secs(600);

#[derive(Clone, Deserialize)]
struct Manifest {
    version: String,
    #[serde(default)]
    notes: String,
    url: String,
    sha256: String,
    size: u64,
}

#[derive(Clone, Copy)]
enum How {
    /// Программа умеет обновляться сама: `claude update`.
    Own(&'static str),
    /// Через npm: `npm install -g <пакет>@latest`.
    Npm,
}

struct Known {
    id: &'static str,
    name: &'static str,
    package: &'static str,
    how: How,
}

const KNOWN: &[Known] = &[
    Known { id: "claude", name: "Claude", package: "@anthropic-ai/claude-code", how: How::Own("update") },
    Known { id: "codex", name: "Codex", package: "@openai/codex", how: How::Npm },
    Known { id: "opencode", name: "OpenCode", package: "opencode-ai", how: How::Own("upgrade") },
    Known { id: "gemini", name: "Gemini", package: "@google/gemini-cli", how: How::Npm },
];

#[derive(Clone)]
struct ToolStatus {
    id: &'static str,
    name: &'static str,
    binary: String,
    package: &'static str,
    how: How,
    current: String,
    latest: String,
    state: &'static str,
    error: String,
}

#[derive(Default)]
struct Inner {
    latest: Option<Manifest>,
    state: &'static str,
    percent: u64,
    error: String,
    install_when_idle: bool,
    /// Версия уже лежит на месте старой; перезапуск ждёт, пока боты закончат работу.
    restart_when_idle: bool,
    /// Куда встала новая версия (путь работающей копии после переименования может быть другим).
    ready_exe: Option<PathBuf>,
    tools: Vec<ToolStatus>,
}

#[derive(Default)]
pub struct Updates {
    inner: Mutex<Inner>,
}

impl Updates {
    pub fn snapshot(&self) -> Value {
        let inner = self.inner.lock().unwrap();
        let tools: Vec<Value> = inner
            .tools
            .iter()
            .map(|t| json!({ "id": t.id, "name": t.name, "current": t.current, "latest": t.latest, "state": t.state, "error": t.error }))
            .collect();
        json!({
            "current": CURRENT,
            "app": {
                "latest": inner.latest.as_ref().map(|m| m.version.clone()),
                "notes": inner.latest.as_ref().map(|m| m.notes.clone()).unwrap_or_default(),
                "state": if inner.state.is_empty() { "idle" } else { inner.state },
                "percent": inner.percent,
                "error": inner.error,
                "restartPending": inner.restart_when_idle,
            },
            "tools": tools,
        })
    }

    fn set_app(&self, state: &'static str, error: &str) {
        let mut inner = self.inner.lock().unwrap();
        inner.state = state;
        inner.error = error.to_string();
    }

    fn app_state(&self) -> &'static str {
        self.inner.lock().unwrap().state
    }
}

fn emit(app: &App) {
    app.turns.emit(json!({ "type": "updates", "updates": app.updates.snapshot() }));
}

// ---------------------------------------------------------------------------
// Откуда брать и чему верить
// ---------------------------------------------------------------------------

pub(crate) fn test_env(name: &str) -> Option<String> {
    if cfg!(debug_assertions) {
        std::env::var(name).ok()
    } else {
        None
    }
}

fn manifest_url() -> Option<String> {
    if cfg!(debug_assertions) {
        // Отладочная сборка и тесты в настоящую сеть не ходят.
        return test_env("CREW_TEST_UPDATE_URL");
    }
    Some(MANIFEST_URL.to_string())
}

fn npm_registry() -> Option<String> {
    if cfg!(debug_assertions) {
        return test_env("CREW_TEST_NPM_URL");
    }
    Some(NPM_REGISTRY.to_string())
}

fn from_hex(text: &str) -> Option<Vec<u8>> {
    let text = text.trim();
    if text.len() % 2 != 0 {
        return None;
    }
    (0..text.len()).step_by(2).map(|i| u8::from_str_radix(text.get(i..i + 2)?, 16).ok()).collect()
}

fn to_hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

pub(crate) fn verify(message: &[u8], signature_hex: &[u8]) -> Result<(), String> {
    let key_hex = test_env("CREW_TEST_UPDATE_KEY").unwrap_or_else(|| PUBLIC_KEY_HEX.to_string());
    let key: [u8; 32] = from_hex(&key_hex).and_then(|k| k.try_into().ok()).ok_or("ключ выпуска повреждён")?;
    let key = VerifyingKey::from_bytes(&key).map_err(|_| "ключ выпуска повреждён")?;
    let text = std::str::from_utf8(signature_hex).map_err(|_| "подпись повреждена")?;
    let bytes: [u8; 64] = from_hex(text).and_then(|s| s.try_into().ok()).ok_or("подпись повреждена")?;
    key.verify(message, &Signature::from_bytes(&bytes)).map_err(|_| "подпись не сошлась".to_string())
}

fn version_parts(text: &str) -> Option<(u64, u64, u64)> {
    let mut parts = text.trim().trim_start_matches('v').split('.');
    let major = parts.next()?.parse().ok()?;
    let minor = parts.next()?.parse().ok()?;
    let patch = parts.next()?.split(|c: char| !c.is_ascii_digit()).next()?.parse().ok()?;
    Some((major, minor, patch))
}

/// Хвост предварительной версии: `0.3.3-beta.2` → `Some(2)`, `0.3.3` → `None`.
/// Выпуск выше любой своей беты, бета с большим номером выше меньшей.
fn prerelease_rank(text: &str) -> (u8, u64) {
    let Some((_, tail)) = text.trim().split_once('-') else { return (1, 0) };
    let number = tail.rsplit(|c: char| !c.is_ascii_digit()).next().and_then(|n| n.parse().ok()).unwrap_or(0);
    (0, number)
}

fn is_newer(candidate: &str, current: &str) -> bool {
    match (version_parts(candidate), version_parts(current)) {
        (Some(a), Some(b)) if a != b => a > b,
        (Some(_), Some(_)) => prerelease_rank(candidate) > prerelease_rank(current),
        _ => false,
    }
}

fn client() -> Result<reqwest::Client, String> {
    reqwest::Client::builder()
        .user_agent(format!("deloist/{CURRENT}"))
        .connect_timeout(Duration::from_secs(15))
        .build()
        .map_err(|e| e.to_string())
}

async fn fetch_limited(url: &str, limit: usize) -> Result<Vec<u8>, String> {
    let response = client()?
        .get(url)
        .timeout(Duration::from_secs(30))
        .send()
        .await
        .map_err(|e| e.to_string())?;
    if !response.status().is_success() {
        return Err(format!("HTTP {}", response.status().as_u16()));
    }
    let mut stream = response.bytes_stream();
    let mut out = Vec::new();
    while let Some(chunk) = stream.next().await {
        out.extend_from_slice(&chunk.map_err(|e| e.to_string())?);
        if out.len() > limit {
            return Err("ответ слишком большой".to_string());
        }
    }
    Ok(out)
}

// ---------------------------------------------------------------------------
// Своя версия
// ---------------------------------------------------------------------------

async fn read_manifest() -> Result<Option<Manifest>, String> {
    let Some(url) = manifest_url() else {
        return Ok(None);
    };
    let bytes = fetch_limited(&url, MANIFEST_LIMIT).await?;
    let signature = fetch_limited(&format!("{url}.sig"), SIGNATURE_LIMIT).await?;
    // Сначала подпись и только потом разбор: непроверенные данные не читаем.
    verify(&bytes, &signature)?;
    serde_json::from_slice(&bytes).map(Some).map_err(|_| "описание версии повреждено".to_string())
}

async fn check_app(app: &App, manual: bool) {
    if matches!(app.updates.app_state(), "waiting" | "downloading" | "installing" | "ready") {
        return;
    }
    match read_manifest().await {
        Ok(Some(manifest)) if is_newer(&manifest.version, CURRENT) => {
            app.updates.inner.lock().unwrap().latest = Some(manifest);
            app.updates.set_app("available", "");
        }
        Ok(_) => {
            app.updates.inner.lock().unwrap().latest = None;
            app.updates.set_app("idle", "");
        }
        Err(message) => {
            // Плановая проверка без сети никого не беспокоит. Подмену подписи молча проглатывать нельзя.
            if manual || message.contains("подпись") {
                app.updates.inner.lock().unwrap().latest = None;
                app.updates.set_app("error", &format!("Не удалось проверить обновление: {message}"));
            }
        }
    }
}

fn target_exe() -> Result<PathBuf, String> {
    if let Some(path) = test_env("CREW_TEST_UPDATE_EXE") {
        return Ok(PathBuf::from(path));
    }
    std::env::current_exe().map_err(|e| e.to_string())
}

fn old_exe(exe: &Path) -> PathBuf {
    exe.with_extension("old.exe")
}

fn temp_exe(exe: &Path) -> PathBuf {
    exe.with_extension("update.tmp")
}

/// Хвосты прошлого обновления: старый файл можно удалить, только когда прежняя копия закрылась.
pub fn clean_leftovers() {
    let Ok(exe) = std::env::current_exe() else { return };
    let _ = std::fs::remove_file(temp_exe(&exe));
    let old = old_exe(&exe);
    if std::fs::remove_file(&old).is_ok() || !old.exists() {
        return;
    }
    // После перезапуска старая копия ещё закрывается и держит свой файл: Windows не даёт его удалить.
    // Пробуем в фоне, пока она не выйдет, но не дольше минуты.
    std::thread::spawn(move || {
        for _ in 0..120 {
            std::thread::sleep(std::time::Duration::from_millis(500));
            if std::fs::remove_file(&old).is_ok() || !old.exists() {
                return;
            }
        }
    });
}

async fn download(app: &App, manifest: &Manifest, target: &Path) -> Result<(), String> {
    if !cfg!(debug_assertions) && !manifest.url.starts_with("https://") {
        return Err("адрес сборки небезопасен".to_string());
    }
    if manifest.size == 0 || manifest.size > EXE_LIMIT {
        return Err("размер сборки не подходит".to_string());
    }
    let expected = from_hex(&manifest.sha256).ok_or("описание версии повреждено")?;
    let response = client()?.get(&manifest.url).send().await.map_err(|e| e.to_string())?;
    if !response.status().is_success() {
        return Err(format!("HTTP {}", response.status().as_u16()));
    }
    let mut file = tokio::fs::File::create(target).await.map_err(|e| format!("нет права записи рядом с программой: {e}"))?;
    let mut stream = response.bytes_stream();
    let mut hasher = Sha256::new();
    let (mut received, mut shown) = (0u64, 0u64);
    let mut head = Vec::new();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|e| e.to_string())?;
        received += chunk.len() as u64;
        if received > manifest.size {
            return Err("сборка больше заявленного размера".to_string());
        }
        if head.len() < 2 {
            head.extend_from_slice(&chunk[..chunk.len().min(2)]);
        }
        hasher.update(&chunk);
        file.write_all(&chunk).await.map_err(|e| e.to_string())?;
        let percent = received * 100 / manifest.size;
        if percent >= shown + 5 {
            shown = percent;
            app.updates.inner.lock().unwrap().percent = percent;
            emit(app);
        }
    }
    file.flush().await.map_err(|e| e.to_string())?;
    drop(file);
    if received != manifest.size {
        return Err("сборка скачалась не целиком".to_string());
    }
    if hasher.finalize().as_slice() != expected.as_slice() {
        return Err(format!("контрольная сумма не сошлась, ожидалась {}", to_hex(&expected)));
    }
    if !head.starts_with(b"MZ") {
        return Err("это не программа для Windows".to_string());
    }
    Ok(())
}

/// Работающий exe в Windows нельзя перезаписать, но можно переименовать: на его место встаёт новый.
fn swap_files(exe: &Path, fresh: &Path) -> Result<(), String> {
    let old = old_exe(exe);
    let _ = std::fs::remove_file(&old);
    std::fs::rename(exe, &old).map_err(|e| format!("не смог отодвинуть текущую версию: {e}"))?;
    if let Err(e) = std::fs::rename(fresh, exe) {
        let _ = std::fs::rename(&old, exe);
        return Err(format!("не смог поставить новую версию: {e}"));
    }
    Ok(())
}

fn restart(exe: &Path) -> Result<(), String> {
    crate::cloud::deny(crate::cloud::NO_INSTALL)?;
    let args: Vec<String> = std::env::args().skip(1).filter(|a| a != "--tray" && a != "--after-update").collect();
    std::process::Command::new(exe)
        .args(args)
        .arg("--after-update")
        .spawn()
        .map(|_| ())
        .map_err(|e| format!("не смог запустить новую версию: {e}"))
}

async fn install(app: Arc<App>) {
    if crate::cloud::active() {
        return;
    }
    let Some(manifest) = app.updates.inner.lock().unwrap().latest.clone() else {
        return;
    };
    let fail = |message: String| {
        app.updates.set_app("error", &format!("Не удалось обновить: {message}"));
        emit(&app);
    };
    let exe = match target_exe() {
        Ok(exe) => exe,
        Err(e) => return fail(e),
    };
    let fresh = temp_exe(&exe);
    app.updates.inner.lock().unwrap().percent = 0;
    app.updates.set_app("downloading", "");
    emit(&app);
    if let Err(e) = download(&app, &manifest, &fresh).await {
        let _ = tokio::fs::remove_file(&fresh).await;
        return fail(e);
    }
    // Пока качали, бот мог начать работу: её не обрываем.
    if !app.turns.busy_bots().is_empty() {
        app.updates.set_app("waiting", "");
        emit(&app);
        while !app.turns.busy_bots().is_empty() {
            tokio::time::sleep(Duration::from_secs(1)).await;
        }
    }
    app.updates.set_app("installing", "");
    emit(&app);
    if let Err(e) = swap_files(&exe, &fresh) {
        let _ = tokio::fs::remove_file(&fresh).await;
        return fail(e);
    }
    // Тест подменяет отдельный файл: самого себя он не перезапускает.
    if test_env("CREW_TEST_UPDATE_EXE").is_some() {
        app.updates.set_app("installed", "");
        emit(&app);
        return;
    }
    if let Err(e) = restart(&exe) {
        let _ = std::fs::rename(&exe, &fresh);
        let _ = std::fs::rename(old_exe(&exe), &exe);
        return fail(e);
    }
    exit_gracefully(&app).await
}

/// Браузеры закрываем штатно: они успеют записать cookies, и вход не пропадёт.
async fn exit_gracefully(app: &Arc<App>) -> ! {
    app.browsers.close_all().await;
    crate::mcp::shutdown(app).await;
    crate::sandbox::shutdown();
    app.pets.stop();
    tokio::time::sleep(Duration::from_millis(300)).await;
    std::process::exit(0);
}

/// Фоновая установка: скачать, проверить, положить на место старой. Ничего не перезапускается:
/// боты работают дальше, а новая версия запустится при следующем запуске или по кнопке «Перезапустить».
async fn auto_install(app: Arc<App>) {
    if crate::cloud::active() {
        return;
    }
    // Отладочная сборка подменяет только тестовый файл, а не самого себя.
    if cfg!(debug_assertions) && test_env("CREW_TEST_UPDATE_EXE").is_none() {
        return;
    }
    let Some(manifest) = app.updates.inner.lock().unwrap().latest.clone() else {
        return;
    };
    let fail = |message: String| {
        app.updates.set_app("error", &format!("Не удалось обновить: {message}"));
        emit(&app);
    };
    let exe = match target_exe() {
        Ok(exe) => exe,
        Err(e) => return fail(e),
    };
    let fresh = temp_exe(&exe);
    app.updates.inner.lock().unwrap().percent = 0;
    app.updates.set_app("downloading", "");
    emit(&app);
    if let Err(e) = download(&app, &manifest, &fresh).await {
        let _ = tokio::fs::remove_file(&fresh).await;
        return fail(e);
    }
    if let Err(e) = swap_files(&exe, &fresh) {
        let _ = tokio::fs::remove_file(&fresh).await;
        return fail(e);
    }
    app.updates.inner.lock().unwrap().ready_exe = Some(exe);
    app.updates.set_app("ready", "");
    emit(&app);
}

/// Перезапуск на уже поставленную версию.
async fn restart_ready(app: Arc<App>) {
    let Some(exe) = app.updates.inner.lock().unwrap().ready_exe.clone() else {
        return;
    };
    app.updates.set_app("installing", "");
    emit(&app);
    // Тест подменяет отдельный файл: самого себя он не перезапускает.
    if test_env("CREW_TEST_UPDATE_EXE").is_some() {
        app.updates.set_app("installed", "");
        emit(&app);
        return;
    }
    if let Err(e) = restart(&exe) {
        app.updates.set_app("ready", &format!("Не удалось перезапустить: {e}"));
        emit(&app);
        return;
    }
    exit_gracefully(&app).await
}

/// Нажатие «Перезапустить». Идущую работу не обрываем: подождём, пока боты закончат.
pub fn request_restart(app: &Arc<App>) -> Result<(), String> {
    crate::cloud::deny(crate::cloud::NO_INSTALL)?;
    {
        let mut inner = app.updates.inner.lock().unwrap();
        if inner.state != "ready" || inner.ready_exe.is_none() {
            return Err("готовой версии нет".to_string());
        }
        if !app.turns.busy_bots().is_empty() {
            inner.restart_when_idle = true;
            inner.error.clear();
            drop(inner);
            emit(app);
            return Ok(());
        }
        inner.restart_when_idle = false;
    }
    tokio::spawn(restart_ready(app.clone()));
    Ok(())
}

/// Нажатие «Обновить». Идущую работу не обрываем: подождём, пока боты закончат.
pub fn request_install(app: &Arc<App>) -> Result<(), String> {
    crate::cloud::deny(crate::cloud::NO_INSTALL)?;
    if app.updates.app_state() == "ready" {
        return request_restart(app);
    }
    {
        let mut inner = app.updates.inner.lock().unwrap();
        if inner.latest.is_none() {
            return Err("новой версии нет".to_string());
        }
        if matches!(inner.state, "waiting" | "downloading" | "installing" | "installed") {
            return Ok(());
        }
        if !app.turns.busy_bots().is_empty() {
            inner.install_when_idle = true;
            inner.state = "waiting";
            inner.error.clear();
            drop(inner);
            emit(app);
            return Ok(());
        }
    }
    tokio::spawn(install(app.clone()));
    Ok(())
}

/// Зовётся планировщиком: не пора ли поставить версию, которую отложили до конца работы.
pub fn tick(app: &Arc<App>) {
    let (ready, restart_now) = {
        let mut inner = app.updates.inner.lock().unwrap();
        let idle = app.turns.busy_bots().is_empty();
        let ready = inner.install_when_idle && idle;
        let restart_now = inner.restart_when_idle && idle && inner.state == "ready";
        if ready {
            inner.install_when_idle = false;
        }
        if restart_now {
            inner.restart_when_idle = false;
        }
        (ready, restart_now)
    };
    if ready {
        tokio::spawn(install(app.clone()));
    }
    if restart_now {
        tokio::spawn(restart_ready(app.clone()));
    }
}

// ---------------------------------------------------------------------------
// Подключённые программы
// ---------------------------------------------------------------------------

/// Какие известные программы реально используются: строка настроек только указывает на файл.
fn connected(app: &App) -> Vec<(&'static Known, String)> {
    let settings = app.store.settings();
    let mut commands = Vec::new();
    if settings.driver == "cli" || app.store.bots().iter().any(|b| b.driver == "cli") {
        commands.push(settings.cli_command.clone());
    }
    if settings.fallback_driver == "cli" {
        let spare = settings.fallback_cli_command.trim();
        commands.push(if spare.is_empty() { settings.cli_command.clone() } else { spare.to_string() });
    }
    let mut found: Vec<(&'static Known, String)> = Vec::new();
    for command in commands {
        let Some(binary) = crate::drivers::cli::first_word(&command) else { continue };
        // Строка уйдёт в оболочку: символы её управления здесь не нужны никому честному.
        if binary.chars().any(|c| "\"&|<>^%!\r\n".contains(c)) {
            continue;
        }
        let stem = Path::new(&binary).file_stem().map(|s| s.to_string_lossy().to_lowercase()).unwrap_or_default();
        if let Some(known) = KNOWN.iter().find(|k| k.id == stem) {
            if !found.iter().any(|(k, _)| k.id == known.id) {
                found.push((known, binary));
            }
        }
    }
    found
}

fn hide_keys(app: &App, command: &mut tokio::process::Command) {
    command.env_remove("OPENAI_API_KEY");
    let name = app.store.settings().api_key_env;
    if !name.is_empty() {
        command.env_remove(name);
    }
}

async fn version_of(app: &App, binary: &str) -> Option<String> {
    if crate::cloud::active() {
        return None;
    }
    let mut command = shell_command(&format!("\"{binary}\" --version"));
    hide_keys(app, &mut command);
    command.stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::null()).kill_on_drop(true);
    let output = tokio::time::timeout(Duration::from_secs(20), command.output()).await.ok()?.ok()?;
    let text = String::from_utf8_lossy(&output.stdout);
    Regex::new(r"\d+\.\d+\.\d+").ok()?.find(&text).map(|m| m.as_str().to_string())
}

async fn latest_of(package: &str) -> Option<String> {
    let base = npm_registry()?;
    let bytes = fetch_limited(&format!("{base}/{package}/latest"), REGISTRY_LIMIT).await.ok()?;
    let data: Value = serde_json::from_slice(&bytes).ok()?;
    data.get("version")?.as_str().map(str::to_string)
}

async fn check_tools(app: &App) {
    let mut tools = Vec::new();
    for (known, binary) in connected(app) {
        // Программу, которая сейчас обновляется, не трогаем.
        let busy = app.updates.inner.lock().unwrap().tools.iter().any(|t| t.id == known.id && t.state == "updating");
        if busy {
            continue;
        }
        let Some(current) = version_of(app, &binary).await else { continue };
        let latest = latest_of(known.package).await.unwrap_or_default();
        let state = if is_newer(&latest, &current) { "available" } else { "current" };
        tools.push(ToolStatus {
            id: known.id,
            name: known.name,
            binary,
            package: known.package,
            how: known.how,
            current,
            latest,
            state,
            error: String::new(),
        });
    }
    let mut inner = app.updates.inner.lock().unwrap();
    let updating: Vec<ToolStatus> = inner.tools.iter().filter(|t| t.state == "updating").cloned().collect();
    inner.tools = updating.into_iter().chain(tools).collect();
}

pub async fn check_all(app: &App, manual: bool) {
    check_app(app, manual).await;
    check_tools(app).await;
    emit(app);
}

fn set_tool(app: &App, id: &str, state: &'static str, error: &str, current: Option<String>) {
    let mut inner = app.updates.inner.lock().unwrap();
    if let Some(tool) = inner.tools.iter_mut().find(|t| t.id == id) {
        tool.state = state;
        tool.error = error.to_string();
        if let Some(current) = current {
            tool.current = current;
        }
    }
}

async fn run_tool_update(app: Arc<App>, tool: ToolStatus) {
    if crate::cloud::active() {
        set_tool(&app, &tool.id, "error", crate::cloud::NO_INSTALL, None);
        return;
    }
    let line = match tool.how {
        How::Own(argument) => format!("\"{}\" {argument}", tool.binary),
        How::Npm => format!("npm install -g {}@latest", tool.package),
    };
    let mut command = shell_command(&line);
    hide_keys(&app, &mut command);
    command.stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped()).kill_on_drop(true);
    let result = tokio::time::timeout(TOOL_UPDATE_TIMEOUT, command.output()).await;
    let problem = match result {
        Err(_) => Some("обновление идёт слишком долго, остановил".to_string()),
        Ok(Err(e)) => Some(e.to_string()),
        Ok(Ok(output)) if !output.status.success() => {
            let text = String::from_utf8_lossy(&output.stderr).trim().to_string();
            let tail: String = text.chars().rev().take(300).collect::<Vec<_>>().into_iter().rev().collect();
            Some(if tail.is_empty() { "команда обновления завершилась с ошибкой".to_string() } else { tail })
        }
        Ok(Ok(_)) => None,
    };
    if let Some(message) = problem {
        set_tool(&app, tool.id, "error", &message, None);
        emit(&app);
        return;
    }
    let current = version_of(&app, &tool.binary).await;
    let updated = current.as_deref().is_some_and(|v| !is_newer(&tool.latest, v));
    if updated {
        set_tool(&app, tool.id, "current", "", current);
    } else {
        set_tool(&app, tool.id, "error", "команда прошла, но версия не изменилась", current);
    }
    emit(&app);
}

/// Нажатие «Обновить» у подключённой программы. Посреди работы бота менять её нельзя.
pub fn request_tool_update(app: &Arc<App>, id: &str) -> Result<(), ToolUpdateError> {
    if !app.turns.busy_bots().is_empty() {
        return Err(ToolUpdateError::Busy);
    }
    let tool = {
        let mut inner = app.updates.inner.lock().unwrap();
        let Some(tool) = inner.tools.iter_mut().find(|t| t.id == id) else {
            return Err(ToolUpdateError::NotFound);
        };
        if tool.state == "updating" {
            return Ok(());
        }
        tool.state = "updating";
        tool.error.clear();
        tool.clone()
    };
    emit(app);
    tokio::spawn(run_tool_update(app.clone(), tool));
    Ok(())
}

pub enum ToolUpdateError {
    Busy,
    NotFound,
}

// ---------------------------------------------------------------------------
// Плановая проверка
// ---------------------------------------------------------------------------

pub fn start(app: Arc<App>) {
    // В отладочной сборке проверку запускает только сам тест (или стенд с CREW_TEST_UPDATE_SCHEDULE).
    let scheduled = test_env("CREW_TEST_UPDATE_SCHEDULE").map(|v| v.parse::<u64>().unwrap_or(1));
    if cfg!(debug_assertions) && scheduled.is_none() {
        return;
    }
    tokio::spawn(async move {
        tokio::time::sleep(scheduled.map_or(FIRST_CHECK_AFTER, Duration::from_secs)).await;
        loop {
            let settings = app.store.settings();
            if settings.check_updates {
                check_all(&app, false).await;
                if settings.auto_update && app.updates.app_state() == "available" {
                    auto_install(app.clone()).await;
                }
            }
            tokio::time::sleep(if scheduled.is_some() { Duration::from_secs(3600) } else { CHECK_EVERY }).await;
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn versions_compare_as_numbers() {
        assert!(is_newer("0.10.0", "0.9.9"));
        assert!(is_newer("v2.1.319", "2.1.318"));
        assert!(!is_newer("0.2.0", "0.2.0"));
        assert!(!is_newer("0.1.9", "0.2.0"));
        assert!(!is_newer("мусор", "0.2.0"));
        assert!(is_newer("1.0.0-beta", "0.9.0"));
        assert!(is_newer("0.3.3-beta.1", "0.3.2"));
        assert!(is_newer("0.3.3", "0.3.3-beta.1"), "выпуск выше своей беты");
        assert!(is_newer("0.3.3-beta.2", "0.3.3-beta.1"));
        assert!(!is_newer("0.3.3-beta.1", "0.3.3"));
        assert!(!is_newer("0.3.3-beta.1", "0.3.3-beta.1"));
    }

    #[test]
    fn hex_round_trip_refuses_garbage() {
        assert_eq!(from_hex("00ff"), Some(vec![0, 255]));
        assert_eq!(from_hex("0"), None);
        assert_eq!(from_hex("zz"), None);
        assert_eq!(to_hex(&[0, 255]), "00ff");
    }
}
