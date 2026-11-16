//! Загрузчик `Deloist-Setup.exe`: ставит только подписанную версию, пишет только в свою папку,
//! в `HKCU` и в ярлыки; удаление не трогает чужие файлы и по желанию оставляет данные.
//!
//! Настоящая сеть и настоящий реестр установки не задействованы: описание версии отдаёт локальный
//! сервер, ключ подписи тестовый, а ключ реестра и папки ярлыков подменены (только в отладочной сборке).
#![cfg(windows)]

use std::collections::HashMap;
use std::io::{Read, Write};
use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;
use std::time::{Duration, Instant};

use ed25519_dalek::{Signer, SigningKey};
use sha2::{Digest, Sha256};

const SETUP: &str = env!("CARGO_BIN_EXE_deloist-setup");

fn setup_bin() -> PathBuf {
    std::env::var_os("CREW_TEST_SETUP_BIN").map(PathBuf::from).unwrap_or_else(|| SETUP.into())
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn scratch(name: &str) -> PathBuf {
    let stamp = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos();
    let root = std::env::var_os("CREW_TEST_TEMP_DIR").map(PathBuf::from).unwrap_or_else(|| env!("CARGO_TARGET_TMPDIR").into());
    let dir = root.join(format!("{name}-{stamp}"));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

struct Release {
    base: String,
    key_hex: String,
    exe: Vec<u8>,
}

fn publish(tamper_exe: bool, sign_with_other_key: bool) -> Release {
    publish_with(tamper_exe, sign_with_other_key, true)
}

/// `with_loader = false` — описание без поля `loader`, как у выпусков, где загрузчик WebView2 внутри exe.
fn publish_with(tamper_exe: bool, sign_with_other_key: bool, with_loader: bool) -> Release {
    let key = SigningKey::from_bytes(&[7u8; 32]);
    let other = SigningKey::from_bytes(&[9u8; 32]);
    // Настоящий PE-файл: Windows не сохраняет ярлык на «exe», у которого нет разбираемого заголовка.
    let exe = std::fs::read(setup_bin()).expect("exe загрузчика");
    let dll = {
        let mut d = b"MZ fake loader".to_vec();
        d.extend(std::iter::repeat_n(2u8, 300));
        d
    };
    // Сервер отдаёт exe, который не совпадает с суммой в подписанном описании.
    let served_exe = if tamper_exe { let mut e = exe.clone(); e[100] ^= 0xff; e } else { exe.clone() };
    let build = |base: &str| {
        let mut value = serde_json::json!({
            "version": "9.9.9",
            "notes": "test",
            "url": format!("{base}/deloist.exe"),
            "sha256": hex(&Sha256::digest(&exe)),
            "size": exe.len(),
        });
        if with_loader {
            value["loader"] = serde_json::json!({ "url": format!("{base}/WebView2Loader.dll"), "sha256": hex(&Sha256::digest(&dll)), "size": dll.len() });
        }
        value
    };
    // Порт нужен до подписи, поэтому сначала занимаем его, потом строим описание.
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let base = format!("http://127.0.0.1:{port}");
    let manifest = serde_json::to_string_pretty(&build(&base)).unwrap();
    let signer = if sign_with_other_key { &other } else { &key };
    let signature = hex(&signer.sign(manifest.as_bytes()).to_bytes());
    let mut files = HashMap::new();
    files.insert("/latest.json".to_string(), manifest.into_bytes());
    files.insert("/latest.json.sig".to_string(), signature.into_bytes());
    files.insert("/deloist.exe".to_string(), served_exe);
    files.insert("/WebView2Loader.dll".to_string(), dll);
    serve_on(listener, files);
    Release { base, key_hex: hex(key.verifying_key().as_bytes()), exe }
}

/// Локальный «GitHub»: отдаёт готовые файлы по путям.
fn serve_on(listener: TcpListener, files: HashMap<String, Vec<u8>>) {
    let files = Arc::new(files);
    std::thread::spawn(move || {
        for stream in listener.incoming().flatten() {
            let files = files.clone();
            std::thread::spawn(move || {
                let mut stream = stream;
                let mut buffer = [0u8; 4096];
                let n = stream.read(&mut buffer).unwrap_or(0);
                let head = String::from_utf8_lossy(&buffer[..n]).to_string();
                let path = head.split_whitespace().nth(1).unwrap_or("/").to_string();
                let (status, body) = match files.get(&path) {
                    Some(body) => ("200 OK", body.clone()),
                    None => ("404 Not Found", Vec::new()),
                };
                let _ = write!(stream, "HTTP/1.1 {status}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", body.len());
                let _ = stream.write_all(&body);
            });
        }
    });
}

struct Sandbox {
    dir: PathBuf,
    start: PathBuf,
    desktop: PathBuf,
    reg: String,
}

impl Sandbox {
    fn new(name: &str) -> Self {
        // В выпускной сборке подмены путей и реестра выключены: тест задел бы настоящую установку.
        assert!(cfg!(debug_assertions), "тесты установщика запускаются только в отладочной сборке");
        let root = scratch(name);
        let stamp = root.file_name().unwrap().to_string_lossy().to_string();
        Self { dir: root.join("Deloist"), start: root.join("start"), desktop: root.join("desktop"), reg: format!(r"Software\DeloistSetupTest\{stamp}") }
    }

    fn command(&self, exe: &Path, release: Option<&Release>) -> Command {
        let mut cmd = Command::new(exe);
        cmd.env("DELOIST_SETUP_DIR", &self.dir)
            .env("DELOIST_SETUP_START_DIR", &self.start)
            .env("DELOIST_SETUP_DESKTOP_DIR", &self.desktop)
            .env("DELOIST_SETUP_REG", &self.reg)
            .env("DELOIST_SETUP_SKIP_WEBVIEW", "1");
        if let Some(release) = release {
            cmd.env("DELOIST_SETUP_MANIFEST_URL", format!("{}/latest.json", release.base)).env("DELOIST_SETUP_PUBKEY", &release.key_hex);
        }
        cmd
    }

    fn registered(&self) -> bool {
        Command::new("reg").args(["query", &format!(r"HKCU\{}", self.reg), "/v", "UninstallString"]).output().map(|o| o.status.success()).unwrap_or(false)
    }
}

impl Drop for Sandbox {
    fn drop(&mut self) {
        // Только свой ключ: общий родитель трогать нельзя, рядом идут параллельные тесты.
        let _ = Command::new("reg").args(["delete", &format!(r"HKCU\{}", self.reg), "/f"]).output();
    }
}

fn wait_for(label: &str, check: impl Fn() -> bool) {
    let until = Instant::now() + Duration::from_secs(30);
    while Instant::now() < until {
        if check() {
            return;
        }
        std::thread::sleep(Duration::from_millis(200));
    }
    panic!("не дождались: {label}");
}

#[test]
fn installs_signed_build_with_shortcuts_and_uninstall_entry() {
    let release = publish(false, false);
    let sandbox = Sandbox::new("setup-ok");
    let status = sandbox.command(&setup_bin(), Some(&release)).args(["--silent", "--desktop"]).status().unwrap();
    assert!(status.success(), "установка не прошла");
    assert_eq!(std::fs::read(sandbox.dir.join("deloist.exe")).unwrap(), release.exe);
    assert!(sandbox.dir.join("WebView2Loader.dll").exists(), "dll рядом с exe");
    assert!(sandbox.dir.join("uninstall.exe").exists());
    assert!(!sandbox.dir.join("deloist.exe.setup").exists(), "временные файлы убраны");
    assert!(sandbox.start.join("Deloist.lnk").exists(), "ярлык в меню «Пуск»");
    // Идентификатор программы для уведомлений хранится в ярлыке строкой UTF-16; PowerShell его не пишет.
    let start_link = std::fs::read(sandbox.start.join("Deloist.lnk")).unwrap();
    let app_id: Vec<u8> = "Deloist.Bots".encode_utf16().flat_map(|c| c.to_le_bytes()).collect();
    assert!(start_link.windows(app_id.len()).any(|part| part == app_id.as_slice()), "ярлык «Пуск» несёт AUMID Deloist.Bots");
    assert!(sandbox.desktop.join("Deloist.lnk").exists(), "ярлык на рабочем столе по согласию");
    assert!(sandbox.registered(), "запись в «Приложениях»");

    // Вторая установка поверх: данные на месте, старый exe отодвинут.
    std::fs::create_dir_all(sandbox.dir.join("data")).unwrap();
    std::fs::write(sandbox.dir.join("data").join("state.json"), "{}").unwrap();
    assert!(sandbox.command(&setup_bin(), Some(&release)).arg("--silent").status().unwrap().success());
    assert!(sandbox.dir.join("data").join("state.json").exists());
}

#[test]
fn installs_a_manifest_without_the_loader_field() {
    let release = publish_with(false, false, false);
    let sandbox = Sandbox::new("setup-no-loader");
    let status = sandbox.command(&setup_bin(), Some(&release)).args(["--silent"]).status().unwrap();
    assert!(status.success(), "установка без loader не прошла");
    assert_eq!(std::fs::read(sandbox.dir.join("deloist.exe")).unwrap(), release.exe);
    assert!(!sandbox.dir.join("WebView2Loader.dll").exists(), "dll не нужна и не создаётся");
    assert!(!sandbox.dir.join("WebView2Loader.dll.setup").exists());
    assert!(sandbox.registered());
}

#[test]
fn no_desktop_shortcut_without_consent() {
    let release = publish(false, false);
    let sandbox = Sandbox::new("setup-nodesktop");
    assert!(sandbox.command(&setup_bin(), Some(&release)).arg("--silent").status().unwrap().success());
    assert!(sandbox.start.join("Deloist.lnk").exists());
    assert!(!sandbox.desktop.join("Deloist.lnk").exists());
}

#[test]
fn refuses_manifest_signed_with_a_foreign_key() {
    let release = publish(false, true);
    let sandbox = Sandbox::new("setup-badsig");
    assert!(!sandbox.command(&setup_bin(), Some(&release)).arg("--silent").status().unwrap().success());
    assert!(!sandbox.dir.join("deloist.exe").exists());
    assert!(!sandbox.registered());
}

#[test]
fn refuses_build_that_does_not_match_the_signed_checksum() {
    let release = publish(true, false);
    let sandbox = Sandbox::new("setup-badhash");
    assert!(!sandbox.command(&setup_bin(), Some(&release)).arg("--silent").status().unwrap().success());
    assert!(!sandbox.dir.join("deloist.exe").exists());
    assert!(!sandbox.dir.join("deloist.exe.setup").exists(), "битый файл не остаётся");
    assert!(!sandbox.registered());
    assert!(!sandbox.start.join("Deloist.lnk").exists());
}

#[test]
fn uninstall_keeps_data_and_foreign_files_or_wipes_data_on_request() {
    let release = publish(false, false);
    for wipe in [false, true] {
        let sandbox = Sandbox::new(if wipe { "setup-wipe" } else { "setup-keep" });
        assert!(sandbox.command(&setup_bin(), Some(&release)).args(["--silent", "--desktop"]).status().unwrap().success());
        std::fs::create_dir_all(sandbox.dir.join("data")).unwrap();
        std::fs::write(sandbox.dir.join("data").join("state.json"), "{}").unwrap();
        std::fs::write(sandbox.dir.join("mine.txt"), "not ours").unwrap();

        let mut cmd = sandbox.command(&sandbox.dir.join("uninstall.exe"), None);
        cmd.args(["--uninstall", "--silent"]);
        if wipe {
            cmd.arg("--delete-data");
        }
        assert!(cmd.status().unwrap().success());
        wait_for("exe удалён", || !sandbox.dir.join("deloist.exe").exists() && !sandbox.dir.join("uninstall.exe").exists());
        assert!(!sandbox.registered(), "запись в реестре снята");
        assert!(!sandbox.start.join("Deloist.lnk").exists());
        assert!(!sandbox.desktop.join("Deloist.lnk").exists());
        assert!(sandbox.dir.join("mine.txt").exists(), "чужой файл в папке не трогаем");
        if wipe {
            wait_for("data удалена", || !sandbox.dir.join("data").exists());
        } else {
            assert!(sandbox.dir.join("data").join("state.json").exists(), "данные остаются");
        }
    }
}

#[test]
fn uninstall_is_refused_outside_the_installed_folder() {
    let release = publish(false, false);
    let sandbox = Sandbox::new("setup-guard");
    assert!(sandbox.command(&setup_bin(), Some(&release)).arg("--silent").status().unwrap().success());
    std::fs::create_dir_all(sandbox.dir.join("data")).unwrap();
    std::fs::write(sandbox.dir.join("data").join("state.json"), "{}").unwrap();
    // Загрузчик под своим именем (не uninstall.exe) удалять ничего не должен.
    let status = sandbox.command(&setup_bin(), None).args(["--uninstall", "--silent", "--delete-data"]).status().unwrap();
    assert!(!status.success());
    // Копия под именем uninstall.exe, но без deloist.exe рядом, тоже отказывает.
    let lone = scratch("setup-lone");
    std::fs::copy(setup_bin(), lone.join("uninstall.exe")).unwrap();
    std::fs::write(lone.join("keep.txt"), "x").unwrap();
    assert!(!Command::new(lone.join("uninstall.exe")).args(["--uninstall", "--silent", "--delete-data"]).status().unwrap().success());
    // Завершение удаления без своей временной копии и без uninstall.exe в папке отказывает.
    assert!(!Command::new(setup_bin()).args(["--finish-uninstall"]).arg(&lone).args(["1", "silent"]).status().unwrap().success());
    assert!(!Command::new(setup_bin()).args(["--finish-uninstall"]).arg(&sandbox.dir).args(["1", "silent"]).status().unwrap().success());
    assert!(sandbox.dir.join("deloist.exe").exists());
    assert!(sandbox.dir.join("data").join("state.json").exists());
    assert!(lone.join("keep.txt").exists());
}

#[test]
fn refuses_a_foreign_non_empty_folder() {
    let release = publish(false, false);
    let sandbox = Sandbox::new("setup-foreign");
    std::fs::create_dir_all(sandbox.dir.join("data")).unwrap();
    std::fs::write(sandbox.dir.join("data").join("theirs.txt"), "x").unwrap();
    assert!(!sandbox.command(&setup_bin(), Some(&release)).arg("--silent").status().unwrap().success());
    assert!(!sandbox.dir.join("deloist.exe").exists());
    assert!(sandbox.dir.join("data").join("theirs.txt").exists());
    assert!(!sandbox.registered());
}

#[test]
fn relative_dir_is_made_absolute_and_reinstall_keeps_a_backup_of_the_loader() {
    let release = publish(false, false);
    let sandbox = Sandbox::new("setup-reinstall");
    let work = sandbox.dir.parent().unwrap().to_path_buf();
    let mut first = sandbox.command(&setup_bin(), Some(&release));
    first.env_remove("DELOIST_SETUP_DIR").current_dir(&work).args(["--silent", "--dir", "rel-install"]);
    assert!(first.status().unwrap().success());
    let dir = work.join("rel-install");
    assert!(dir.join("deloist.exe").exists(), "относительный --dir стал абсолютным");
    // Запущенная программа держит dll открытой: поверх неё ставим через переименование.
    let held = std::fs::File::open(dir.join("WebView2Loader.dll")).unwrap();
    let mut second = sandbox.command(&setup_bin(), Some(&release));
    second.env_remove("DELOIST_SETUP_DIR").current_dir(&work).args(["--silent", "--dir", "rel-install"]);
    assert!(second.status().unwrap().success(), "переустановка поверх открытой dll");
    drop(held);
    assert!(dir.join("WebView2Loader.dll").exists());
    assert!(dir.join("WebView2Loader.old.dll").exists());
    assert!(!dir.join("WebView2Loader.dll.setup").exists());
}
