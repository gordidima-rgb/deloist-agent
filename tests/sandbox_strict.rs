//! Песочница в режиме strict: клетка AppContainer или отказ, третьего не бывает.
//! Отдельный файл, потому что режим читается один раз на процесс.
#![cfg(windows)]

#[path = "../src/sandbox.rs"]
mod sandbox;

use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::time::Duration;

use sandbox::Level;

fn setup() {
    static ONCE: std::sync::Once = std::sync::Once::new();
    ONCE.call_once(|| {
        let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("sandbox-strict-canary");
        std::fs::create_dir_all(&dir).expect("папка канареек");
        std::env::set_var("CREW_SANDBOX_TMP", &dir);
        std::env::set_var("CREW_SANDBOX", "strict");
    });
}

fn temp_dir(name: &str) -> PathBuf {
    let stamp = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos();
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(format!("strict-{name}-{stamp}"));
    std::fs::create_dir_all(&dir).expect("временная папка");
    dir
}

fn env() -> Vec<(OsString, OsString)> {
    ["PATH", "PATHEXT", "SystemRoot", "SystemDrive", "windir", "ComSpec", "TEMP", "TMP", "USERPROFILE"]
        .iter()
        .filter_map(|name| std::env::var_os(name).map(|value| (OsString::from(*name), value)))
        .collect()
}

fn run(dir: &Path, inner: &str) -> Result<(sandbox::Outcome, String), String> {
    let comspec = std::env::var("ComSpec").unwrap_or_else(|_| "cmd.exe".to_string());
    let line = format!("\"{comspec}\" /d /s /c \"{inner}\"");
    let (out, err) = (dir.join("out.txt"), dir.join("err.txt"));
    let outcome = sandbox::spawn_ex(&line, dir, &env(), Duration::from_secs(20), &out, &err)?;
    Ok((outcome, String::from_utf8_lossy(&std::fs::read(&out).unwrap_or_default()).into_owned()))
}

#[test]
fn strict_is_a_cage_or_nothing() {
    setup();
    let status = sandbox::status();
    eprintln!("strict: {} — {}", status.level.name(), status.detail);
    let dir = temp_dir("cage");
    match status.level {
        Level::AppContainer => {
            let secret = temp_dir("outside").join("secret.txt");
            std::fs::write(&secret, "outside-marker").unwrap();
            let (_, out) = run(&dir, &format!("type \"{}\"", secret.display())).expect("запуск в клетке");
            assert!(!out.contains("outside-marker"), "клетка прочитала файл вне рабочей папки");
            let (outcome, _) = run(&dir, "echo x> inside.txt").expect("запуск в клетке");
            assert_eq!(outcome.code, 0, "в своей папке клетка обязана писать");
            assert!(dir.join("inside.txt").exists());
        }
        // Клетка не поднялась: команда не выполняется вовсе, а не идёт без защиты.
        _ => {
            assert!(run(&dir, "echo x> inside.txt").is_err(), "strict выполнил команду без клетки");
            assert!(!dir.join("inside.txt").exists());
        }
    }
}

#[test]
fn a_folder_the_cage_cannot_use_refuses_only_that_command() {
    setup();
    if sandbox::status().level != Level::AppContainer {
        return;
    }
    // В System32 клетке не создать свою домашнюю папку: эта команда не выполняется.
    let system = PathBuf::from(std::env::var("SystemRoot").unwrap()).join("System32");
    let out = temp_dir("refused").join("out.txt");
    let err = out.with_file_name("err.txt");
    let comspec = std::env::var("ComSpec").unwrap();
    let refused = sandbox::spawn_ex(&format!("\"{comspec}\" /d /s /c \"echo ran\""), &system, &env(), Duration::from_secs(10), &out, &err);
    assert!(refused.is_err(), "команда выполнилась без клетки: {refused:?}");
    // А защита для следующих команд не снялась.
    assert_eq!(sandbox::status().level, Level::AppContainer, "{}", sandbox::status().detail);
    let dir = temp_dir("after");
    assert!(run(&dir, "echo ok").is_ok());
}
