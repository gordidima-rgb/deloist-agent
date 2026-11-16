//! Лёгкий браузер Moli: на ПК ставится только по просьбе человека, из настроек; в облаке вшит в образ.
//! Версия закреплена: архив скачивается с GitHub проекта и сверяется с размером и sha256,
//! записанными здесь. Другой файл под тем же именем не установится.
//! Проект: https://github.com/lexmount/moli (Apache-2.0, файл LICENSE-APACHE). На ПК мы его не распространяем;
//! образ облака содержит его, текст лицензии лежит в THIRD_PARTY/lexmount-moli.txt.
//! Скачивание и сверку делает тот же код, что у Obscura (`obscura.rs`).

use std::path::{Path, PathBuf};
use std::sync::Mutex;

use serde_json::{json, Value};

use crate::obscura::{download, Pin};

pub const VERSION: &str = "1.1.24";
pub const PROJECT_URL: &str = "https://github.com/lexmount/moli";

/// Одна готовая сборка выпуска. Значения сверены через GitHub API 13.11.2026.
#[derive(Clone, Copy)]
struct Build {
    target: &'static str,
    ext: &'static str,
    asset_id: u64,
    size: u64,
    sha256: &'static str,
}

const WINDOWS: Build = Build {
    target: "x86_64-pc-windows-msvc",
    ext: "zip",
    asset_id: 740_613_708,
    size: 45_097_962,
    sha256: "e57f7605aa8ef9d4b268d9bd232d3f285e6f1f81ccbcd4f0e6cda6ea94b4f3b0",
};

const LINUX: Build = Build {
    target: "x86_64-unknown-linux-gnu",
    ext: "tar.gz",
    asset_id: 740_613_886,
    size: 45_363_347,
    sha256: "09e54e48be914df9596e8b9046cd2e1c67d5911761a09b0445488dd5d39c56d6",
};

/// Сборка под эту машину. Для остальных систем и процессоров готовой сборки нет.
fn build() -> Option<Build> {
    if cfg!(all(windows, target_arch = "x86_64")) {
        Some(WINDOWS)
    } else if cfg!(all(target_os = "linux", target_arch = "x86_64")) {
        Some(LINUX)
    } else {
        None
    }
}

impl Build {
    fn asset_name(&self) -> String {
        format!("moli-{}.{}", self.target, self.ext)
    }

    /// Папка верхнего уровня внутри архива.
    fn top_dir(&self) -> String {
        format!("moli-v{VERSION}-{}", self.target)
    }

    /// Сначала через API: с некоторых сетей прямые ссылки на выпуски GitHub отвечают 504.
    fn pin(&self) -> Pin {
        Pin {
            urls: vec![
                format!("https://api.github.com/repos/lexmount/moli/releases/assets/{}", self.asset_id),
                format!("https://github.com/lexmount/moli/releases/download/v{VERSION}/{}", self.asset_name()),
            ],
            size: self.size,
            sha256: self.sha256.to_string(),
        }
    }
}

fn exe_name() -> &'static str {
    if cfg!(windows) {
        "moli.exe"
    } else {
        "moli"
    }
}

/// Режим облака: `CREW_CLOUD=1`. Там движок по умолчанию Moli и ничего не скачивается.
pub fn cloud() -> bool {
    crate::cloud::active()
}

/// Движок бота по настройке: пусто в облаке значит Moli, пусто на ПК значит Chrome.
pub fn engine_choice(setting: &str, cloud: bool) -> &'static str {
    match setting {
        "moli" => "moli",
        "obscura" => "obscura",
        "" if cloud => "moli",
        _ => "chrome",
    }
}

/// Папка установленной версии. Новая версия ляжет рядом и старую не тронет.
pub fn install_dir(data_dir: &Path) -> PathBuf {
    data_dir.join("engines").join(format!("moli-{VERSION}"))
}

/// Бинарник из `CREW_MOLI_PATH`: так он попадает в образ облака. Файла нет — переменная не считается.
pub fn baked_exe() -> Option<PathBuf> {
    let path = PathBuf::from(std::env::var_os("CREW_MOLI_PATH").filter(|v| !v.is_empty())?);
    path.is_file().then_some(path)
}

/// Путь к moli: вшитый в образ или установленный из настроек.
pub fn installed_exe(data_dir: &Path) -> Option<PathBuf> {
    if let Some(path) = baked_exe() {
        return Some(path);
    }
    let exe = install_dir(data_dir).join(exe_name());
    exe.is_file().then_some(exe)
}

// ---------------------------------------------------------------------------
// Состояние установки
// ---------------------------------------------------------------------------

#[derive(Clone, Default)]
struct Progress {
    /// idle, downloading, installing или error.
    state: &'static str,
    percent: u64,
    error: String,
}

static PROGRESS: Mutex<Progress> = Mutex::new(Progress { state: "idle", percent: 0, error: String::new() });

fn set(state: &'static str, percent: u64, error: &str) {
    *PROGRESS.lock().unwrap() = Progress { state, percent, error: error.to_string() };
}

pub fn status(data_dir: &Path) -> Value {
    let progress = PROGRESS.lock().unwrap().clone();
    let state = if progress.state.is_empty() { "idle" } else { progress.state };
    let size = build().map(|b| b.size).unwrap_or(0);
    json!({
        "installed": installed_exe(data_dir).is_some(),
        "version": VERSION,
        "downloadMb": (size as f64 / 1_048_576.0).round(),
        "state": state,
        "percent": progress.percent,
        "error": progress.error,
        "project": PROJECT_URL,
        "cloud": cloud(),
        "baked": baked_exe().is_some(),
        "supported": build().is_some(),
    })
}

/// Установка в фоне. Повторный вызов, пока идёт первая, ничего не начинает.
pub fn start_install(data_dir: PathBuf) -> Result<(), String> {
    if cloud() {
        return Err("В облаке Moli уже вшит в образ и не скачивается".into());
    }
    let Some(build) = build() else {
        return Err("Для этой системы готовой сборки Moli нет".into());
    };
    {
        let mut progress = PROGRESS.lock().unwrap();
        if matches!(progress.state, "downloading" | "installing") {
            return Err("Moli уже скачивается".into());
        }
        *progress = Progress { state: "downloading", percent: 0, error: String::new() };
    }
    tokio::spawn(async move {
        let result = install(&build, &build.pin(), &data_dir, |percent| set("downloading", percent, ""), || set("installing", 100, "")).await;
        match result {
            Ok(()) => set("idle", 0, ""),
            Err(e) => set("error", 0, &format!("Не удалось установить Moli: {e}")),
        }
    });
    Ok(())
}

/// Удаление файлов движка. Профили ботов в Moli удаляются тоже: без движка они ни к чему.
pub async fn uninstall(data_dir: &Path) -> Result<(), String> {
    if matches!(PROGRESS.lock().unwrap().state, "downloading" | "installing") {
        return Err("дождись конца установки".into());
    }
    for dir in [install_dir(data_dir), data_dir.join("browser-moli")] {
        if dir.exists() {
            tokio::fs::remove_dir_all(&dir).await.map_err(|e| format!("не смог удалить {}: {e}", dir.display()))?;
        }
    }
    set("idle", 0, "");
    Ok(())
}

// ---------------------------------------------------------------------------
// Распаковка
// ---------------------------------------------------------------------------

/// Распаковать архив: zip встроенным в Windows tar, tar.gz системным tar. Содержимое проверяет вызывающий.
async fn extract_archive(archive: &Path, into: &Path) -> Result<(), String> {
    crate::cloud::deny(crate::cloud::NO_INSTALL)?;
    if cfg!(windows) {
        return crate::obscura::extract_zip(archive, into).await;
    }
    let mut command = tokio::process::Command::new("tar");
    command.arg("-x").arg("-z").arg("-f").arg(archive).arg("-C").arg(into).stdin(std::process::Stdio::null());
    let output = command.output().await.map_err(|e| format!("не запустился tar: {e}"))?;
    if !output.status.success() {
        return Err(format!("архив не распаковался: {}", String::from_utf8_lossy(&output.stderr).trim()));
    }
    Ok(())
}

/// Первые байты файла: на Windows это `MZ`, на Linux `\x7fELF`.
fn is_native_program(path: &Path, windows: bool) -> bool {
    use std::io::Read;
    let mut head = [0u8; 4];
    let Ok(mut file) = std::fs::File::open(path) else { return false };
    if file.read_exact(&mut head).is_err() {
        return false;
    }
    if windows {
        head.starts_with(b"MZ")
    } else {
        head == *b"\x7fELF"
    }
}

/// Распаковать в staging и проверить, что внутри папка выпуска с настоящей программой moli.
async fn extract(build: &Build, archive: &Path, staging: &Path) -> Result<PathBuf, String> {
    extract_archive(archive, staging).await?;
    let top = staging.join(build.top_dir());
    let exe = top.join(exe_name());
    if !exe.is_file() {
        return Err(format!("в архиве нет {}", exe_name()));
    }
    if !is_native_program(&exe, cfg!(windows)) {
        return Err(format!("{} не программа для этой системы", exe_name()));
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&exe, std::fs::Permissions::from_mode(0o755)).map_err(|e| e.to_string())?;
    }
    Ok(top)
}

async fn install(build: &Build, pin: &Pin, data_dir: &Path, progress: impl Fn(u64), installing: impl Fn()) -> Result<(), String> {
    let engines = data_dir.join("engines");
    tokio::fs::create_dir_all(&engines).await.map_err(|e| e.to_string())?;
    let archive = engines.join(format!("{}.part", build.asset_name()));
    let staging = engines.join(format!("moli-{VERSION}.staging"));
    let _ = tokio::fs::remove_dir_all(&staging).await;

    let mut last_error = String::from("нет адреса для скачивания");
    let mut downloaded = false;
    for url in &pin.urls {
        match download(pin, url, &archive, &progress).await {
            Ok(()) => {
                downloaded = true;
                break;
            }
            Err(e) => {
                let _ = tokio::fs::remove_file(&archive).await;
                // Неверная сумма это не сбой сети: второй адрес отдаст то же самое.
                if e.contains("контрольная сумма") {
                    return Err(e);
                }
                last_error = e;
            }
        }
    }
    if !downloaded {
        return Err(last_error);
    }
    installing();
    tokio::fs::create_dir_all(&staging).await.map_err(|e| e.to_string())?;
    let extracted = extract(build, &archive, &staging).await;
    let _ = tokio::fs::remove_file(&archive).await;
    let top = match extracted {
        Ok(top) => top,
        Err(e) => {
            let _ = tokio::fs::remove_dir_all(&staging).await;
            return Err(e);
        }
    };
    let target = install_dir(data_dir);
    let _ = tokio::fs::remove_dir_all(&target).await;
    let moved = tokio::fs::rename(&top, &target).await.map_err(|e| format!("не смог переложить файлы: {e}"));
    let _ = tokio::fs::remove_dir_all(&staging).await;
    moved
}

// ---------------------------------------------------------------------------
// Запуск
// ---------------------------------------------------------------------------

/// Адрес отладчика из ответа `/json/version`. У Moli нет ready-файла: порт выбираем сами и спрашиваем его.
/// Ответ обязан быть от Moli (поле `Moli-Version`), иначе на порту мог отозваться чужой процесс.
pub fn parse_version(text: &str) -> Result<String, String> {
    let value: Value = serde_json::from_str(text.trim()).map_err(|_| "ответ /json/version не JSON".to_string())?;
    if value["Moli-Version"].as_str().is_none() {
        return Err("на порту отвечает не Moli".into());
    }
    let url = value["webSocketDebuggerUrl"].as_str().ok_or("в ответе нет webSocketDebuggerUrl")?;
    let rest = url.strip_prefix("ws://").ok_or("адрес отладчика не ws://")?;
    let authority = rest.split('/').next().unwrap_or("");
    let host = authority.rsplit_once(':').map(|(h, _)| h).unwrap_or(authority).trim_matches(|c| c == '[' || c == ']');
    // Отладчик Moli слушает только эту машину: другой адрес значит, что ответил не он.
    if !matches!(host, "127.0.0.1" | "localhost" | "::1") {
        return Err(format!("Moli слушает не локальный адрес: {host}"));
    }
    Ok(url.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use sha2::{Digest, Sha256};
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    fn temp(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("crew-moli-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn to_hex(bytes: &[u8]) -> String {
        bytes.iter().map(|b| format!("{b:02x}")).collect()
    }

    /// Простейший HTTP-сервер: на любой запрос отдаёт одни и те же байты.
    async fn serve(bytes: Vec<u8>) -> String {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        tokio::spawn(async move {
            loop {
                let Ok((mut socket, _)) = listener.accept().await else { return };
                let bytes = bytes.clone();
                tokio::spawn(async move {
                    let mut buf = [0u8; 4096];
                    let _ = socket.read(&mut buf).await;
                    let head = format!("HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", bytes.len());
                    let _ = socket.write_all(head.as_bytes()).await;
                    let _ = socket.write_all(&bytes).await;
                });
            }
        });
        format!("http://127.0.0.1:{port}/a")
    }

    /// Архив для этой системы: внутри папка выпуска с указанными файлами.
    fn make_archive(dir: &Path, files: &[(&str, &[u8])]) -> Vec<u8> {
        let build = build().expect("нет сборки для этой системы");
        let src = dir.join("src");
        let top = src.join(build.top_dir());
        std::fs::create_dir_all(&top).unwrap();
        for (name, body) in files {
            std::fs::write(top.join(name), body).unwrap();
        }
        let archive = dir.join(format!("made.{}", build.ext));
        let status = if cfg!(windows) {
            let root = std::env::var_os("SystemRoot").map(PathBuf::from).unwrap_or_else(|| PathBuf::from(r"C:\Windows"));
            std::process::Command::new(root.join("System32").join("tar.exe")).args(["-a", "-c", "-f"]).arg(&archive).arg("-C").arg(&src).arg(build.top_dir()).status().unwrap()
        } else {
            std::process::Command::new("tar").args(["-c", "-z", "-f"]).arg(&archive).arg("-C").arg(&src).arg(build.top_dir()).status().unwrap()
        };
        assert!(status.success());
        std::fs::read(archive).unwrap()
    }

    fn program() -> &'static [u8] {
        if cfg!(windows) {
            b"MZ moli"
        } else {
            b"\x7fELF moli"
        }
    }

    fn pin_for(url: String, bytes: &[u8]) -> Pin {
        Pin { urls: vec![url], size: bytes.len() as u64, sha256: to_hex(&Sha256::digest(bytes)) }
    }

    #[tokio::test]
    async fn installs_a_verified_archive() {
        let build = build().unwrap();
        let dir = temp("ok");
        let archive = make_archive(&dir, &[(exe_name(), program()), ("LICENSE-APACHE", b"license")]);
        let url = serve(archive.clone()).await;
        let data = dir.join("data");
        assert!(install_dir(&data).join(exe_name()).is_file() == false);
        install(&build, &pin_for(url, &archive), &data, |_| {}, || {}).await.unwrap();
        let exe = install_dir(&data).join(exe_name());
        assert_eq!(std::fs::read(&exe).unwrap(), program());
        assert!(install_dir(&data).join("LICENSE-APACHE").is_file(), "лицензия должна лежать рядом");
        assert!(!data.join("engines").join(format!("{}.part", build.asset_name())).exists(), "архив должен удаляться");
        assert!(!data.join("engines").join(format!("moli-{VERSION}.staging")).exists(), "черновая папка должна удаляться");
        uninstall(&data).await.unwrap();
        assert!(!install_dir(&data).exists());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[tokio::test]
    async fn wrong_checksum_installs_nothing() {
        let build = build().unwrap();
        let dir = temp("sum");
        let archive = make_archive(&dir, &[(exe_name(), program())]);
        let url = serve(archive.clone()).await;
        let mut pin = pin_for(url, &archive);
        pin.sha256 = "00".repeat(32);
        let data = dir.join("data");
        let error = install(&build, &pin, &data, |_| {}, || {}).await.unwrap_err();
        assert!(error.contains("контрольная сумма"), "{error}");
        assert!(!install_dir(&data).exists());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[tokio::test]
    async fn wrong_size_and_missing_files_are_refused() {
        let build = build().unwrap();
        let dir = temp("size");
        let archive = make_archive(&dir, &[("README.md", b"no program here")]);
        let url = serve(archive.clone()).await;
        let data = dir.join("data");

        let mut pin = pin_for(url.clone(), &archive);
        pin.size -= 1;
        let error = install(&build, &pin, &data, |_| {}, || {}).await.unwrap_err();
        assert!(error.contains("больше"), "{error}");

        let error = install(&build, &pin_for(url, &archive), &data, |_| {}, || {}).await.unwrap_err();
        assert!(error.contains(exe_name()), "{error}");
        assert!(!install_dir(&data).exists());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[tokio::test]
    async fn not_a_native_program_is_refused() {
        let build = build().unwrap();
        let dir = temp("magic");
        let archive = make_archive(&dir, &[(exe_name(), b"#!/bin/sh")]);
        let url = serve(archive.clone()).await;
        let data = dir.join("data");
        let error = install(&build, &pin_for(url, &archive), &data, |_| {}, || {}).await.unwrap_err();
        assert!(error.contains("не программа"), "{error}");
        assert!(!install_dir(&data).exists());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn release_pins_match_the_checked_values() {
        assert_eq!(WINDOWS.size, 45_097_962);
        assert_eq!(LINUX.size, 45_363_347);
        for build in [WINDOWS, LINUX] {
            let pin = build.pin();
            assert_eq!(pin.sha256.len(), 64);
            assert_eq!(pin.urls.len(), 2);
            assert!(pin.urls.iter().all(|u| u.starts_with("https://")));
            assert!(pin.urls[0].contains("/releases/assets/"));
            assert_eq!(pin.urls[1], format!("https://github.com/lexmount/moli/releases/download/v1.1.24/moli-{}.{}", build.target, build.ext));
            assert_eq!(build.top_dir(), format!("moli-v1.1.24-{}", build.target));
        }
    }

    #[test]
    fn engine_is_moli_by_default_only_in_the_cloud() {
        assert_eq!(engine_choice("", true), "moli");
        assert_eq!(engine_choice("", false), "chrome");
        assert_eq!(engine_choice("chrome", true), "chrome");
        assert_eq!(engine_choice("obscura", true), "obscura");
        assert_eq!(engine_choice("moli", false), "moli");
    }

    #[test]
    fn version_answer_gives_a_local_websocket() {
        let ok = r#"{"Browser":"Chrome/156","Moli-Version":"1.1.24","webSocketDebuggerUrl":"ws://127.0.0.1:39777/devtools/browser/abc"}"#;
        assert_eq!(parse_version(ok).unwrap(), "ws://127.0.0.1:39777/devtools/browser/abc");
        assert!(parse_version("").is_err());
        // Чужой процесс на том же порту: нет поля Moli-Version.
        assert!(parse_version(r#"{"Browser":"Chrome/156","webSocketDebuggerUrl":"ws://127.0.0.1:1/devtools/browser/abc"}"#).is_err());
        assert!(parse_version(r#"{"Moli-Version":"1","webSocketDebuggerUrl":"ws://10.0.0.5:9222/devtools/browser/abc"}"#).is_err());
        assert!(parse_version(r#"{"Moli-Version":"1","webSocketDebuggerUrl":"wss://127.0.0.1:9222/x"}"#).is_err());
    }
}
