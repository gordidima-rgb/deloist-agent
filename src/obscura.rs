//! Лёгкий браузер Obscura: ставится только по просьбе человека, из настроек.
//! Версия закреплена: архив скачивается с GitHub проекта и сверяется с размером и sha256,
//! записанными здесь. Другой файл под тем же именем не установится.
//! Проект: https://github.com/h4ckf0r0day/obscura (Apache-2.0). Мы не распространяем его сами.

use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::Duration;

use futures_util::StreamExt;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use tokio::io::AsyncWriteExt;

pub const VERSION: &str = "0.2.6";
/// Обычная сборка: с отрисовкой страниц и без маскировки отпечатка. Маскировку мы не ставим:
/// бот не обходит защиту сайтов от роботов.
const ASSET_NAME: &str = "obscura-x86_64-windows.zip";
const ASSET_SIZE: u64 = 69_076_668;
const ASSET_SHA256: &str = "0a110d0693753a252ed7ffd61e682363e8622caf79697f564c529099a31a6242";
/// Сначала через API: с некоторых сетей прямые ссылки на выпуски GitHub отвечают 504.
const ASSET_URLS: [&str; 2] = [
    "https://api.github.com/repos/h4ckf0r0day/obscura/releases/assets/696469078",
    "https://github.com/h4ckf0r0day/obscura/releases/download/v0.2.6/obscura-x86_64-windows.zip",
];
pub const PROJECT_URL: &str = "https://github.com/h4ckf0r0day/obscura";
const FILES: [&str; 2] = ["obscura.exe", "obscura-worker.exe"];

/// Что и откуда ставить. Отдельно от констант, чтобы тесты могли подставить свой архив.
pub struct Pin {
    pub urls: Vec<String>,
    pub size: u64,
    pub sha256: String,
}

impl Pin {
    fn release() -> Self {
        Pin { urls: ASSET_URLS.iter().map(|u| u.to_string()).collect(), size: ASSET_SIZE, sha256: ASSET_SHA256.to_string() }
    }
}

/// Папка установленной версии. Новая версия ляжет рядом и старую не тронет.
pub fn install_dir(data_dir: &Path) -> PathBuf {
    data_dir.join("engines").join(format!("obscura-{VERSION}"))
}

/// Путь к obscura.exe, если оба файла на месте.
pub fn installed_exe(data_dir: &Path) -> Option<PathBuf> {
    let dir = install_dir(data_dir);
    FILES.iter().all(|name| dir.join(name).is_file()).then(|| dir.join(FILES[0]))
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
    json!({
        "installed": installed_exe(data_dir).is_some(),
        "version": VERSION,
        "downloadMb": (ASSET_SIZE as f64 / 1_048_576.0).round(),
        "state": state,
        "percent": progress.percent,
        "error": progress.error,
        "project": PROJECT_URL,
    })
}

/// Установка в фоне. Повторный вызов, пока идёт первая, ничего не начинает.
pub fn start_install(data_dir: PathBuf) -> Result<(), String> {
    {
        let mut progress = PROGRESS.lock().unwrap();
        if matches!(progress.state, "downloading" | "installing") {
            return Err("Obscura уже скачивается".into());
        }
        *progress = Progress { state: "downloading", percent: 0, error: String::new() };
    }
    tokio::spawn(async move {
        let result = install(&Pin::release(), &data_dir, |percent| set("downloading", percent, ""), || set("installing", 100, "")).await;
        match result {
            Ok(()) => set("idle", 0, ""),
            Err(e) => set("error", 0, &format!("Не удалось установить Obscura: {e}")),
        }
    });
    Ok(())
}

/// Удаление файлов движка. Профили ботов в Obscura удаляются тоже: без движка они ни к чему.
pub async fn uninstall(data_dir: &Path) -> Result<(), String> {
    if matches!(PROGRESS.lock().unwrap().state, "downloading" | "installing") {
        return Err("дождись конца установки".into());
    }
    for dir in [install_dir(data_dir), data_dir.join("browser-obscura")] {
        if dir.exists() {
            tokio::fs::remove_dir_all(&dir).await.map_err(|e| format!("не смог удалить {}: {e}", dir.display()))?;
        }
    }
    set("idle", 0, "");
    Ok(())
}

// ---------------------------------------------------------------------------
// Скачивание и распаковка
// ---------------------------------------------------------------------------

fn client() -> Result<reqwest::Client, String> {
    reqwest::Client::builder()
        .user_agent(concat!("deloist/", env!("CARGO_PKG_VERSION")))
        .connect_timeout(Duration::from_secs(15))
        .build()
        .map_err(|e| e.to_string())
}

fn to_hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// Архив целиком во временный файл, со сверкой размера и суммы. Неподходящий файл удаляется.
pub(crate) async fn download(pin: &Pin, url: &str, target: &Path, progress: &impl Fn(u64)) -> Result<(), String> {
    let response = client()?
        .get(url)
        .header("Accept", "application/octet-stream")
        .timeout(Duration::from_secs(600))
        .send()
        .await
        .map_err(|e| e.to_string())?;
    if !response.status().is_success() {
        return Err(format!("HTTP {}", response.status().as_u16()));
    }
    let mut file = tokio::fs::File::create(target).await.map_err(|e| format!("нет права записи в папку данных: {e}"))?;
    let mut stream = response.bytes_stream();
    let mut hasher = Sha256::new();
    let (mut received, mut shown) = (0u64, 0u64);
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|e| e.to_string())?;
        received += chunk.len() as u64;
        if received > pin.size {
            return Err("архив больше ожидаемого".into());
        }
        hasher.update(&chunk);
        file.write_all(&chunk).await.map_err(|e| e.to_string())?;
        let percent = received * 100 / pin.size.max(1);
        if percent >= shown + 2 {
            shown = percent;
            progress(percent);
        }
    }
    file.flush().await.map_err(|e| e.to_string())?;
    drop(file);
    if received != pin.size {
        return Err("архив скачался не целиком".into());
    }
    let got = to_hex(&hasher.finalize());
    if !got.eq_ignore_ascii_case(&pin.sha256) {
        return Err(format!("контрольная сумма не сошлась: ждали {}, пришла {got}", pin.sha256));
    }
    Ok(())
}

/// Встроенный в Windows tar (с 2018 года) понимает zip. Полный путь: чужой tar из PATH не нужен.
fn tar_exe() -> PathBuf {
    let root = std::env::var_os("SystemRoot").map(PathBuf::from).unwrap_or_else(|| PathBuf::from(r"C:\Windows"));
    root.join("System32").join("tar.exe")
}

/// Распаковать zip встроенным tar. Содержимое проверяет вызывающий.
pub(crate) async fn extract_zip(zip: &Path, into: &Path) -> Result<(), String> {
    crate::cloud::deny(crate::cloud::NO_INSTALL)?;
    let mut command = tokio::process::Command::new(tar_exe());
    command.arg("-x").arg("-f").arg(zip).arg("-C").arg(into).stdin(std::process::Stdio::null());
    #[cfg(windows)]
    command.creation_flags(0x0800_0000);
    let output = command.output().await.map_err(|e| format!("не запустился tar: {e}"))?;
    if !output.status.success() {
        return Err(format!("архив не распаковался: {}", String::from_utf8_lossy(&output.stderr).trim()));
    }
    Ok(())
}

async fn extract(zip: &Path, into: &Path) -> Result<(), String> {
    extract_zip(zip, into).await?;
    for name in FILES {
        let path = into.join(name);
        let head = tokio::fs::read(&path).await.map_err(|_| format!("в архиве нет {name}"))?;
        if !head.starts_with(b"MZ") {
            return Err(format!("{name} не программа для Windows"));
        }
    }
    Ok(())
}

pub async fn install(pin: &Pin, data_dir: &Path, progress: impl Fn(u64), installing: impl Fn()) -> Result<(), String> {
    let engines = data_dir.join("engines");
    tokio::fs::create_dir_all(&engines).await.map_err(|e| e.to_string())?;
    let zip = engines.join(format!("{ASSET_NAME}.part"));
    let staging = engines.join(format!("obscura-{VERSION}.staging"));
    let _ = tokio::fs::remove_dir_all(&staging).await;

    let mut last_error = String::from("нет адреса для скачивания");
    let mut downloaded = false;
    for url in &pin.urls {
        match download(pin, url, &zip, &progress).await {
            Ok(()) => {
                downloaded = true;
                break;
            }
            Err(e) => {
                let _ = tokio::fs::remove_file(&zip).await;
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
    let extracted = extract(&zip, &staging).await;
    let _ = tokio::fs::remove_file(&zip).await;
    if let Err(e) = extracted {
        let _ = tokio::fs::remove_dir_all(&staging).await;
        return Err(e);
    }
    let target = install_dir(data_dir);
    let _ = tokio::fs::remove_dir_all(&target).await;
    tokio::fs::rename(&staging, &target).await.map_err(|e| format!("не смог переложить файлы: {e}"))
}

// ---------------------------------------------------------------------------
// Запуск
// ---------------------------------------------------------------------------

/// Адрес отладчика из ready-файла `obscura serve --ready-file`.
pub fn parse_ready_file(text: &str) -> Result<String, String> {
    let value: Value = serde_json::from_str(text.trim()).map_err(|_| "ready-файл Obscura не JSON".to_string())?;
    let host = value["host"].as_str().ok_or("в ready-файле нет host")?;
    let port = value["port"].as_u64().filter(|p| (1..=65535).contains(p)).ok_or("в ready-файле нет порта")?;
    let path = value["websocket_path"].as_str().filter(|p| p.starts_with('/')).unwrap_or("/devtools/browser");
    // Отладчик Obscura слушает только эту машину: другой адрес значит, что файл не его.
    if host != "127.0.0.1" && host != "localhost" && host != "::1" {
        return Err(format!("Obscura слушает не локальный адрес: {host}"));
    }
    Ok(format!("ws://{host}:{port}{path}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::AsyncReadExt;

    fn temp(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("crew-obscura-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
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
        format!("http://127.0.0.1:{port}/a.zip")
    }

    /// Zip тем же встроенным tar: -a выбирает формат по расширению.
    fn make_zip(dir: &Path, files: &[(&str, &[u8])]) -> Vec<u8> {
        let src = dir.join("src");
        std::fs::create_dir_all(&src).unwrap();
        for (name, body) in files {
            std::fs::write(src.join(name), body).unwrap();
        }
        let zip = dir.join("made.zip");
        let names: Vec<&str> = files.iter().map(|(n, _)| *n).collect();
        let status = std::process::Command::new(tar_exe()).arg("-a").arg("-c").arg("-f").arg(&zip).arg("-C").arg(&src).args(&names).status().unwrap();
        assert!(status.success());
        std::fs::read(zip).unwrap()
    }

    fn pin_for(url: String, bytes: &[u8]) -> Pin {
        Pin { urls: vec![url], size: bytes.len() as u64, sha256: to_hex(&Sha256::digest(bytes)) }
    }

    #[cfg(windows)]
    #[tokio::test]
    async fn installs_a_verified_archive() {
        let dir = temp("ok");
        let zip = make_zip(&dir, &[("obscura.exe", b"MZ main"), ("obscura-worker.exe", b"MZ worker")]);
        let url = serve(zip.clone()).await;
        let data = dir.join("data");
        assert!(installed_exe(&data).is_none());
        install(&pin_for(url, &zip), &data, |_| {}, || {}).await.unwrap();
        let exe = installed_exe(&data).expect("не установилось");
        assert_eq!(std::fs::read(exe).unwrap(), b"MZ main");
        assert!(!data.join("engines").join(format!("{ASSET_NAME}.part")).exists(), "архив должен удаляться");
        uninstall(&data).await.unwrap();
        assert!(installed_exe(&data).is_none());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[cfg(windows)]
    #[tokio::test]
    async fn wrong_checksum_installs_nothing() {
        let dir = temp("sum");
        let zip = make_zip(&dir, &[("obscura.exe", b"MZ main"), ("obscura-worker.exe", b"MZ worker")]);
        let url = serve(zip.clone()).await;
        let mut pin = pin_for(url, &zip);
        pin.sha256 = "00".repeat(32);
        let data = dir.join("data");
        let error = install(&pin, &data, |_| {}, || {}).await.unwrap_err();
        assert!(error.contains("контрольная сумма"), "{error}");
        assert!(installed_exe(&data).is_none());
        assert!(!install_dir(&data).exists());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[cfg(windows)]
    #[tokio::test]
    async fn wrong_size_and_missing_files_are_refused() {
        let dir = temp("size");
        let zip = make_zip(&dir, &[("obscura.exe", b"MZ main")]);
        let url = serve(zip.clone()).await;
        let data = dir.join("data");

        let mut pin = pin_for(url.clone(), &zip);
        pin.size -= 1;
        let error = install(&pin, &data, |_| {}, || {}).await.unwrap_err();
        assert!(error.contains("больше"), "{error}");

        let error = install(&pin_for(url, &zip), &data, |_| {}, || {}).await.unwrap_err();
        assert!(error.contains("obscura-worker.exe"), "{error}");
        assert!(installed_exe(&data).is_none());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[cfg(windows)]
    #[tokio::test]
    async fn not_a_windows_program_is_refused() {
        let dir = temp("mz");
        let zip = make_zip(&dir, &[("obscura.exe", b"#!/bin/sh"), ("obscura-worker.exe", b"MZ worker")]);
        let url = serve(zip.clone()).await;
        let data = dir.join("data");
        let error = install(&pin_for(url, &zip), &data, |_| {}, || {}).await.unwrap_err();
        assert!(error.contains("не программа"), "{error}");
        assert!(installed_exe(&data).is_none());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn release_pin_is_the_plain_windows_build() {
        let pin = Pin::release();
        assert_eq!(pin.sha256.len(), 64);
        assert!(pin.urls.iter().all(|u| u.starts_with("https://")));
        assert!(!ASSET_NAME.contains("stealth"), "маскировку отпечатка не ставим");
    }

    #[test]
    fn ready_file_gives_a_local_websocket() {
        let url = parse_ready_file(r#"{"pid":1,"host":"127.0.0.1","port":51069,"websocket_path":"/devtools/browser"}"#).unwrap();
        assert_eq!(url, "ws://127.0.0.1:51069/devtools/browser");
        assert!(parse_ready_file("").is_err());
        assert!(parse_ready_file(r#"{"host":"127.0.0.1","port":0}"#).is_err());
        assert!(parse_ready_file(r#"{"host":"10.0.0.5","port":9222,"websocket_path":"/devtools/browser"}"#).is_err());
    }
}
