//! Свой Node.js: если на компьютере нет Node 20 или новее, Deloist скачивает LTS один раз
//! и дальше запускает npm-агентов и MCP-подключения своим `node.exe`.
//!
//! Одна копия на компьютер: `<данные>/runtimes/node/<версия>/`. Рядом с `node.exe` лежит
//! `deloist-install.json` с его sha256 — при каждом запуске сумма проверяется, подменённый
//! файл скачивается заново. Архив и `SHASUMS256.txt` берутся с одного адреса, только https,
//! размер архива ограничен 80 МБ.

use std::path::{Path, PathBuf};
use std::process::Stdio;

use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

use crate::App;

const DIST: &str = "https://nodejs.org/dist";
const SIZE_CAP: u64 = 80 * 1024 * 1024;
const MIN_MAJOR: u64 = 20;
const INSTALL_RECORD: &str = "deloist-install.json";

/// Один экземпляр загрузки: два агента не должны качать Node одновременно.
static DOWNLOAD_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

/// Где лежит Node и чем ставить npm-пакеты. `managed == false` — системный Node с PATH.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NodePaths {
    pub node: PathBuf,
    pub npm_cli: PathBuf,
    pub managed: bool,
}

/// Этапы, о которых можно сказать в интерфейсе, пока идёт управляемая установка.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Stage {
    Downloading,
    Verifying,
    Unpacking,
}

/// Node для запуска npm-агентов: системный 20+ или своя скачанная LTS.
pub async fn ensure_node(app: &App) -> Result<NodePaths, String> {
    ensure_node_with_progress(app, |_| {}).await
}

/// То же, но с отчётом об этапах: экран провайдера показывает, что именно происходит.
pub async fn ensure_node_with_progress<F>(app: &App, progress: F) -> Result<NodePaths, String>
where
    F: Fn(Stage) + Send + Sync,
{
    // Политика облака: свой Node.js и npm не скачиваем и не запускаем.
    crate::cloud::deny(crate::cloud::NO_INSTALL)?;
    let _guard = DOWNLOAD_LOCK.lock().await;
    if let Some(paths) = system_node().await {
        return Ok(paths);
    }
    let root = absolute(app.store.data_dir().join("runtimes").join("node"));
    if let Some(paths) = installed_node(&root).await {
        return Ok(paths);
    }
    download_node(&root, &progress).await
}

fn absolute(path: PathBuf) -> PathBuf {
    if path.is_absolute() {
        path
    } else {
        std::env::current_dir().map(|dir| dir.join(&path)).unwrap_or(path)
    }
}

/// Уже скачанный управляемый Node без сети: нужен карточкам до согласия.
pub fn installed_node_paths(data_dir: &Path) -> Option<NodePaths> {
    let root = data_dir.join("runtimes").join("node");
    let mut versions: Vec<(Version, PathBuf)> = std::fs::read_dir(&root)
        .ok()?
        .flatten()
        .filter(|e| e.file_type().ok().is_some_and(|t| t.is_dir()))
        .filter_map(|e| {
            let name = e.file_name().to_string_lossy().to_string();
            Some((parse_version(&name)?, e.path()))
        })
        .collect();
    versions.sort_by(|a, b| b.0.cmp(&a.0));
    for (_, dir) in versions {
        let record = std::fs::read_to_string(dir.join(INSTALL_RECORD)).ok().and_then(|t| serde_json::from_str::<Value>(&t).ok());
        let Some(expected) = record.as_ref().and_then(|r| r["sha256"].as_str()).map(str::to_ascii_lowercase) else { continue };
        let node = dir.join(node_file_name());
        if sha_file_sync(&node).ok().as_deref() != Some(expected.as_str()) {
            continue;
        }
        let npm_cli = dir.join(npm_cli_path());
        if !npm_cli.is_file() {
            continue;
        }
        return Some(NodePaths { node, npm_cli, managed: true });
    }
    None
}

type Version = (u64, u64, u64);

fn parse_version(text: &str) -> Option<Version> {
    let text = text.trim().trim_start_matches(['v', 'V']);
    let mut parts = text.split('.');
    let major = parts.next()?.parse().ok()?;
    let minor = parts.next()?.parse().ok()?;
    let patch = parts.next()?.split(|c: char| !c.is_ascii_digit()).next()?.parse().ok()?;
    Some((major, minor, patch))
}

/// Самый новый LTS из `index.json` Node.js. Записи без `lts` или с `lts: false` не LTS.
fn lts_version(index: &Value) -> Option<String> {
    index
        .as_array()?
        .iter()
        .filter(|entry| entry["lts"].as_bool() != Some(false))
        .filter_map(|entry| entry["version"].as_str())
        .filter_map(|version| Some((parse_version(version)?, version)))
        .max_by_key(|(parsed, _)| *parsed)
        .map(|(_, version)| version.to_string())
}

/// Строка `SHASUMS256.txt` для файла: `<sha256>  <имя>` (бывает и `*имя`).
fn checksum_for(text: &str, file: &str) -> Option<String> {
    for line in text.lines() {
        let mut parts = line.split_whitespace();
        let (Some(sha), Some(name)) = (parts.next(), parts.next()) else { continue };
        let name = name.strip_prefix('*').unwrap_or(name);
        if name == file && sha.len() == 64 && sha.bytes().all(|b| b.is_ascii_hexdigit()) {
            return Some(sha.to_ascii_lowercase());
        }
    }
    None
}

fn archive_name(version: &str) -> String {
    let arch = if cfg!(target_arch = "aarch64") { "arm64" } else { "x64" };
    if cfg!(windows) {
        format!("node-{version}-win-{arch}.zip")
    } else if cfg!(target_os = "macos") {
        format!("node-{version}-darwin-{arch}.tar.gz")
    } else {
        format!("node-{version}-linux-{arch}.tar.gz")
    }
}

fn archive_dir(version: &str) -> String {
    archive_name(version).trim_end_matches(".zip").trim_end_matches(".tar.gz").trim_end_matches(".tgz").to_string()
}

fn node_file_name() -> &'static str {
    if cfg!(windows) { "node.exe" } else { "node" }
}

fn npm_cli_path() -> PathBuf {
    if cfg!(windows) {
        PathBuf::from("node_modules").join("npm").join("bin").join("npm-cli.js")
    } else {
        PathBuf::from("lib").join("node_modules").join("npm").join("bin").join("npm-cli.js")
    }
}

/// Базовый адрес дистрибутива. В выпуске — только официальный https; в отладочной сборке
/// тесты подменяют его сервером-заглушкой, как `api_base()` в Telegram.
fn dist_base() -> String {
    #[cfg(debug_assertions)]
    {
        if let Ok(url) = std::env::var("CREW_TEST_NODE_DIST") {
            let url = url.trim().trim_end_matches('/');
            if !url.is_empty() {
                return url.to_string();
            }
        }
    }
    DIST.to_string()
}

fn client() -> Result<reqwest::Client, String> {
    reqwest::Client::builder()
        .connect_timeout(std::time::Duration::from_secs(15))
        .timeout(std::time::Duration::from_secs(10 * 60))
        .user_agent(concat!("Deloist/", env!("CARGO_PKG_VERSION")))
        .build()
        .map_err(|e| format!("нет интернета: {e}"))
}

async fn fetch_text(base: &str, path: &str, cap: u64) -> Result<String, String> {
    let response = client()?.get(format!("{base}/{path}")).send().await.map_err(|e| format!("нет интернета: {e}"))?;
    if !response.status().is_success() {
        return Err(format!("сервер Node.js ответил HTTP {}", response.status().as_u16()));
    }
    if response.content_length().is_some_and(|n| n > cap) {
        return Err("ответ сервера Node.js слишком большой".into());
    }
    let bytes = response.bytes().await.map_err(|e| format!("нет интернета: {e}"))?;
    if bytes.len() as u64 > cap {
        return Err("ответ сервера Node.js слишком большой".into());
    }
    String::from_utf8(bytes.to_vec()).map_err(|_| "сервер Node.js ответил не текстом".into())
}

/// PATH с папкой Node впереди: npm и запущенные им пакеты находят свой `node`.
pub fn path_with_node(node: &Path) -> std::ffi::OsString {
    let mut dirs: Vec<PathBuf> = node.parent().map(Path::to_path_buf).into_iter().collect();
    if let Some(path) = std::env::var_os("PATH") {
        dirs.extend(std::env::split_paths(&path));
    }
    std::env::join_paths(dirs).unwrap_or_default()
}

fn find_on_path(name: &str) -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    let names: Vec<String> = if cfg!(windows) { vec![format!("{name}.exe"), name.to_string()] } else { vec![name.to_string()] };
    std::env::split_paths(&path)
        .flat_map(|dir| names.iter().map(move |name| dir.join(name)))
        .find(|candidate| candidate.is_file())
}

fn find_npm_cli(node: &Path) -> Option<PathBuf> {
    let dir = node.parent()?;
    let mut candidates = vec![dir.join(npm_cli_path()), dir.join("..").join("lib").join("node_modules").join("npm").join("bin").join("npm-cli.js")];
    if let Some(appdata) = std::env::var_os("APPDATA") {
        candidates.push(PathBuf::from(appdata).join("npm").join("node_modules").join("npm").join("bin").join("npm-cli.js"));
    }
    candidates.into_iter().find(|candidate| candidate.is_file())
}

async fn run_version(node: &Path) -> Option<Version> {
    if crate::cloud::active() {
        return None;
    }
    let mut cmd = tokio::process::Command::new(node);
    cmd.arg("--version").stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::null()).kill_on_drop(true);
    #[cfg(windows)]
    cmd.creation_flags(0x0800_0000);
    let output = cmd.output().await.ok()?;
    if !output.status.success() {
        return None;
    }
    parse_version(&String::from_utf8_lossy(&output.stdout))
}

async fn system_node() -> Option<NodePaths> {
    let node = absolute(find_on_path("node")?);
    let (major, _, _) = run_version(&node).await?;
    if major < MIN_MAJOR {
        return None;
    }
    let npm_cli = find_npm_cli(&node)?;
    Some(NodePaths { node, npm_cli, managed: false })
}

async fn installed_node(root: &Path) -> Option<NodePaths> {
    let mut entries = tokio::fs::read_dir(root).await.ok()?;
    let mut versions: Vec<(Version, PathBuf)> = Vec::new();
    while let Ok(Some(entry)) = entries.next_entry().await {
        if !entry.file_type().await.ok().is_some_and(|t| t.is_dir()) {
            continue;
        }
        let name = entry.file_name().to_string_lossy().to_string();
        if let Some(version) = parse_version(&name) {
            versions.push((version, entry.path()));
        }
    }
    versions.sort_by(|a, b| b.0.cmp(&a.0));
    for (_, dir) in versions {
        let record = tokio::fs::read_to_string(dir.join(INSTALL_RECORD)).await.ok().and_then(|t| serde_json::from_str::<Value>(&t).ok());
        let expected = record.as_ref().and_then(|r| r["sha256"].as_str()).map(str::to_ascii_lowercase);
        let node = dir.join(node_file_name());
        let Some(expected) = expected else {
            let _ = tokio::fs::remove_dir_all(&dir).await;
            continue;
        };
        let sha = sha_file(&node).await.ok();
        if sha.as_deref() != Some(expected.as_str()) {
            let _ = tokio::fs::remove_dir_all(&dir).await;
            continue;
        }
        let Some((major, _, _)) = run_version(&node).await else {
            let _ = tokio::fs::remove_dir_all(&dir).await;
            continue;
        };
        if major < MIN_MAJOR {
            let _ = tokio::fs::remove_dir_all(&dir).await;
            continue;
        }
        let npm_cli = dir.join(npm_cli_path());
        if !npm_cli.is_file() {
            let _ = tokio::fs::remove_dir_all(&dir).await;
            continue;
        }
        remove_other_versions(root, &dir).await;
        return Some(NodePaths { node, npm_cli, managed: true });
    }
    None
}

async fn remove_other_versions(root: &Path, keep: &Path) {
    let Ok(mut entries) = tokio::fs::read_dir(root).await else { return };
    while let Ok(Some(entry)) = entries.next_entry().await {
        if entry.path() != keep {
            let _ = tokio::fs::remove_dir_all(entry.path()).await;
        }
    }
}

async fn download_node<F>(root: &Path, progress: &F) -> Result<NodePaths, String>
where
    F: Fn(Stage) + Send + Sync,
{
    let base = dist_base();
    if !cfg!(debug_assertions) && !base.starts_with("https://nodejs.org/dist") {
        return Err("Node.js качается только с https://nodejs.org/dist".into());
    }
    let index = fetch_text(&base, "index.json", 8 * 1024 * 1024).await?;
    let index: Value = serde_json::from_str(&index).map_err(|e| format!("index.json Node.js не разобрался: {e}"))?;
    let version = lts_version(&index).ok_or("в index.json Node.js нет версии LTS")?;
    let file = archive_name(&version);
    let sums = fetch_text(&base, &format!("{version}/SHASUMS256.txt"), 1024 * 1024).await?;
    let expected = checksum_for(&sums, &file).ok_or_else(|| format!("в SHASUMS256.txt нет строки для {file}: не ставлю"))?;

    tokio::fs::create_dir_all(root).await.map_err(|e| format!("не смог создать папку Node.js (диск): {e}"))?;
    let part = root.join(format!("_download-{file}"));
    progress(Stage::Downloading);
    if let Err(e) = fetch_archive(&base, &version, &file, &part).await {
        let _ = tokio::fs::remove_file(&part).await;
        return Err(e);
    }
    progress(Stage::Verifying);
    let actual = sha_file(&part).await.map_err(|e| format!("не смог прочитать архив Node.js: {e}"))?;
    if actual != expected {
        let _ = tokio::fs::remove_file(&part).await;
        return Err("контрольная сумма Node.js не сошлась: не ставлю".into());
    }
    if let Err(e) = archive_entries_ok(&part).await {
        let _ = tokio::fs::remove_file(&part).await;
        return Err(e);
    }
    progress(Stage::Unpacking);
    let staging = root.join(format!(".staging-{version}"));
    let target = root.join(&version);
    let _ = tokio::fs::remove_dir_all(&staging).await;
    let _ = tokio::fs::remove_dir_all(&target).await;
    tokio::fs::create_dir_all(&staging).await.map_err(|e| format!("не смог распаковать Node.js (диск): {e}"))?;
    let unpacked = async {
        crate::obscura::extract_zip(&part, &staging).await?;
        let inner = staging.join(archive_dir(&version));
        if !inner.is_dir() {
            return Err(format!("в архиве Node.js нет папки {}", archive_dir(&version)));
        }
        tokio::fs::rename(&inner, &target).await.map_err(|e| format!("не смог переложить Node.js: {e}"))
    }
    .await;
    let _ = tokio::fs::remove_file(&part).await;
    let _ = tokio::fs::remove_dir_all(&staging).await;
    if let Err(e) = unpacked {
        let _ = tokio::fs::remove_dir_all(&target).await;
        return Err(e);
    }

    let node = target.join(node_file_name());
    if !node.is_file() {
        let _ = tokio::fs::remove_dir_all(&target).await;
        return Err(format!("в архиве Node.js нет {}", node_file_name()));
    }
    let sha = sha_file(&node).await.map_err(|e| format!("не смог прочитать node.exe: {e}"))?;
    let Some((major, _, _)) = run_version(&node).await else {
        let _ = tokio::fs::remove_dir_all(&target).await;
        return Err("скачанный Node.js не запустился: не ставлю".into());
    };
    if major < MIN_MAJOR {
        let _ = tokio::fs::remove_dir_all(&target).await;
        return Err(format!("скачанный Node.js старее {MIN_MAJOR}: не ставлю"));
    }
    let record = json!({ "version": version, "sha256": sha });
    tokio::fs::write(target.join(INSTALL_RECORD), record.to_string()).await.map_err(|e| format!("не смог записать отметку Node.js (диск): {e}"))?;
    remove_other_versions(root, &target).await;
    let npm_cli = target.join(npm_cli_path());
    if !npm_cli.is_file() {
        return Err("в Node.js нет npm: не ставлю".into());
    }
    Ok(NodePaths { node, npm_cli, managed: true })
}

async fn fetch_archive(base: &str, version: &str, file: &str, part: &Path) -> Result<(), String> {
    let url = format!("{base}/{version}/{file}");
    let response = client()?.get(&url).send().await.map_err(|e| format!("нет интернета: {e}"))?;
    if !response.status().is_success() {
        return Err(format!("сервер Node.js ответил HTTP {}", response.status().as_u16()));
    }
    if response.content_length().is_some_and(|n| n > SIZE_CAP) {
        return Err("архив Node.js больше 80 МБ: не скачиваю".into());
    }
    let mut out = tokio::fs::File::create(part).await.map_err(|e| format!("не смог записать Node.js (диск): {e}"))?;
    let mut response = response;
    let mut got: u64 = 0;
    while let Some(chunk) = response.chunk().await.map_err(|e| format!("архив Node.js оборвался: {e}"))? {
        got += chunk.len() as u64;
        if got > SIZE_CAP {
            return Err("архив Node.js больше 80 МБ: не скачиваю".into());
        }
        out.write_all(&chunk).await.map_err(|e| format!("не смог записать Node.js (диск): {e}"))?;
    }
    out.flush().await.map_err(|e| format!("не смог записать Node.js (диск): {e}"))?;
    Ok(())
}

/// Встроенный tar понимает zip. Сначала смотрим список: путь за пределы папки не распаковываем.
async fn archive_entries_ok(archive: &Path) -> Result<(), String> {
    crate::cloud::deny(crate::cloud::NO_INSTALL)?;
    let root = std::env::var_os("SystemRoot").map(PathBuf::from).unwrap_or_else(|| PathBuf::from(r"C:\Windows"));
    let tar = if cfg!(windows) { root.join("System32").join("tar.exe") } else { PathBuf::from("tar") };
    let mut cmd = tokio::process::Command::new(tar);
    cmd.arg("-tf").arg(archive).stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped());
    #[cfg(windows)]
    cmd.creation_flags(0x0800_0000);
    let output = cmd.output().await.map_err(|e| format!("не смог прочитать архив Node.js: {e}"))?;
    if !output.status.success() {
        return Err(format!("архив Node.js не читается: {}", String::from_utf8_lossy(&output.stderr).trim()));
    }
    for line in String::from_utf8_lossy(&output.stdout).lines() {
        let path = line.trim().replace('\\', "/");
        let escape = path.starts_with('/')
            || path.split('/').any(|part| part == "..")
            || path.as_bytes().get(1) == Some(&b':')
            || path.starts_with("//");
        if escape {
            return Err("архив Node.js содержит путь за пределы папки: не распаковываю".into());
        }
    }
    Ok(())
}

fn sha_file_sync(path: &Path) -> Result<String, std::io::Error> {
    use std::io::Read;
    let mut file = std::fs::File::open(path)?;
    let mut hasher = Sha256::new();
    let mut buf = vec![0u8; 1 << 16];
    loop {
        let n = file.read(&mut buf)?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    Ok(hasher.finalize().iter().map(|b| format!("{b:02x}")).collect())
}

async fn sha_file(path: &Path) -> Result<String, String> {
    let mut file = tokio::fs::File::open(path).await.map_err(|e| e.to_string())?;
    let mut hasher = Sha256::new();
    let mut buf = vec![0u8; 1 << 16];
    loop {
        let n = file.read(&mut buf).await.map_err(|e| e.to_string())?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    Ok(hasher.finalize().iter().map(|b| format!("{b:02x}")).collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lts_picks_the_newest_lts_entry() {
        let index = json!([
            { "version": "v27.0.0", "lts": false },
            { "version": "v26.13.1", "lts": "Lithium" },
            { "version": "v24.11.0", "lts": "Krypton" },
            { "version": "v23.1.0" }
        ]);
        assert_eq!(lts_version(&index), Some("v26.13.1".into()));
        assert_eq!(lts_version(&json!([])), None);
    }

    #[test]
    fn shasums_find_the_exact_file() {
        let sums = "aa\tnot-a-hash  node-v1-win-x64.zip\n\
                    bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb  node-v26.13.1-win-x64.zip\n\
                    cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc *node-v26.13.1-win-arm64.zip\n";
        assert_eq!(
            checksum_for(sums, "node-v26.13.1-win-x64.zip"),
            Some("b".repeat(64))
        );
        assert_eq!(
            checksum_for(sums, "node-v26.13.1-win-arm64.zip"),
            Some("c".repeat(64))
        );
        assert_eq!(checksum_for(sums, "node-v26.13.1-linux-x64.tar.gz"), None);
    }

    #[test]
    fn versions_are_strict() {
        assert_eq!(parse_version("v26.13.1"), Some((26, 13, 1)));
        assert_eq!(parse_version("26.0.0"), Some((26, 0, 0)));
        assert_eq!(parse_version("v26.13.1-rc.1"), Some((26, 13, 1)));
        assert_eq!(parse_version("latest"), None);
    }
}
