//! Песочница на живых командах: что работает именно на этой машине и как она откатывается.
//!
//! Модуль подключаем через `#[path]`: пакет — это bin-крейт, интеграционный тест не может
//! сослаться на `crate::sandbox`. Проверяем ровно тот же код, что уходит в сборку.

#[path = "../../src/sandbox.rs"]
mod sandbox;

use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant};

use sandbox::Level;

/// Канарейки и вывод команд живут в target/tmp: рабочая папка агента может быть read-only.
fn setup() {
    static ONCE: std::sync::Once = std::sync::Once::new();
    ONCE.call_once(|| {
        let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("sandbox-canary");
        std::fs::create_dir_all(&dir).expect("папка канареек");
        std::env::set_var("CREW_SANDBOX_TMP", &dir);
    });
}

fn temp_dir(name: &str) -> PathBuf {
    let stamp = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos();
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(format!("sandbox-{name}-{stamp}"));
    std::fs::create_dir_all(&dir).expect("временная папка");
    dir
}

/// Окружение, которого хватает cmd и ping. Ключи процесса сюда не попадают.
fn env() -> Vec<(OsString, OsString)> {
    const KEEP: &[&str] = &[
        "PATH",
        "PATHEXT",
        "SystemRoot",
        "SystemDrive",
        "windir",
        "ComSpec",
        "TEMP",
        "TMP",
        "USERPROFILE",
        "NUMBER_OF_PROCESSORS",
        "PROCESSOR_ARCHITECTURE",
        "OS",
    ];
    KEEP.iter().filter_map(|name| std::env::var_os(name).map(|value| (OsString::from(*name), value))).collect()
}

fn shell(inner: &str) -> String {
    let comspec = std::env::var("ComSpec").unwrap_or_else(|_| "cmd.exe".to_string());
    format!("\"{comspec}\" /d /s /c \"{inner}\"")
}

fn run(dir: &Path, inner: &str, timeout: Duration) -> sandbox::Outcome {
    let out = dir.join("out.txt");
    let err = dir.join("err.txt");
    sandbox::spawn_ex(&shell(inner), dir, &env(), timeout, &out, &err).expect("запуск команды")
}

fn read(path: &Path) -> String {
    std::fs::read(path).map(|bytes| String::from_utf8_lossy(&bytes).into_owned()).unwrap_or_default()
}

#[test]
fn echo_works_at_the_current_level() {
    setup();
    let dir = temp_dir("echo");
    // Уровень фиксируем до запуска: первый вызов status() и есть самопроверка.
    let level = sandbox::status().level;
    let outcome = run(&dir, "echo sandbox-ok", Duration::from_secs(20));
    let out = read(&dir.join("out.txt"));
    let err = read(&dir.join("err.txt"));
    assert!(!outcome.timed_out, "echo не уложился в таймаут (уровень {})", level.name());
    assert_eq!(outcome.code, 0, "echo вернул код {} (уровень {}), stderr: {err}", outcome.code, level.name());
    assert!(out.contains("sandbox-ok"), "stdout пустой или битый: {out:?}; stderr: {err:?}");
}

#[test]
fn profile_paths_are_refused_or_run_without_file_isolation() {
    setup();
    let level = sandbox::status().level;
    let command = "type %USERPROFILE%\\deloist-sandbox-test.txt";
    match sandbox::looks_protected(command) {
        Some(reason) => {
            assert!(!reason.is_empty(), "причина отказа пустая");
            assert_ne!(level, Level::AppContainer, "текстовый фильтр не должен работать при AppContainer");
        }
        None => {
            // Молчать фильтр может только тогда, когда запрет держит файловая система.
            assert_eq!(level, Level::AppContainer, "фильтр пропустил %USERPROFILE% при уровне {}", level.name());
            let dir = temp_dir("userprofile");
            let outcome = run(&dir, command, Duration::from_secs(20));
            assert!(!outcome.timed_out, "type не уложился в таймаут");
        }
    }
}

#[test]
fn timeout_kills_the_process() {
    setup();
    let dir = temp_dir("timeout");
    let started = Instant::now();
    let outcome = run(&dir, "ping -n 30 127.0.0.1", Duration::from_millis(800));
    assert!(outcome.timed_out, "таймаут не сработал: {outcome:?}");
    assert!(started.elapsed() < Duration::from_secs(15), "таймаут ждал слишком долго: {:?}", started.elapsed());
    assert_eq!(outcome.code, -1, "убитый процесс не должен отдавать код");
}

/// Своя копия ping.exe с уникальным именем: чужие процессы в проверку не попадут.
fn own_ping(dir: &Path, tag: &str) -> Option<(PathBuf, String)> {
    let system32 = PathBuf::from(std::env::var("SystemRoot").unwrap_or_else(|_| "C:\\Windows".into())).join("System32");
    let source = system32.join("PING.EXE");
    if !source.exists() {
        eprintln!("нет {}: тест пропущен", source.display());
        return None;
    }
    let child = dir.join(format!("crew-{tag}-{}.exe", std::process::id()));
    std::fs::copy(&source, &child).expect("копия ping.exe");
    let name = child.file_stem().expect("имя").to_string_lossy().into_owned();
    assert!(!process_running(&name), "процесс с таким именем уже был до теста");
    Some((child, name))
}

fn wait_until(seconds: u64, mut done: impl FnMut() -> bool) -> bool {
    let deadline = Instant::now() + Duration::from_secs(seconds);
    while Instant::now() < deadline {
        if done() {
            return true;
        }
        std::thread::sleep(Duration::from_millis(200));
    }
    false
}

#[test]
fn a_background_child_lives_on_like_a_dev_server() {
    setup();
    let dir = temp_dir("background");
    let Some((child, name)) = own_ping(&dir, "bg") else { return };
    // Бот запускает сервер разработки и идёт дальше: сервер не должен умереть вместе с командой.
    let outcome = run(&dir, &format!("start /b \"\" \"{}\" -n 8 127.0.0.1 > nul", child.display()), Duration::from_secs(20));
    assert!(!outcome.timed_out, "start не уложился в таймаут");
    std::thread::sleep(Duration::from_secs(2));
    assert!(process_running(&name), "фоновый процесс умер вместе с командой");
    assert!(wait_until(15, || !process_running(&name)), "фоновый процесс не завершился сам");
}

#[test]
fn a_timeout_kills_the_whole_tree() {
    setup();
    let dir = temp_dir("tree");
    let Some((child, name)) = own_ping(&dir, "tree") else { return };
    let command = format!("start /b \"\" \"{}\" -n 60 127.0.0.1 > nul & ping -n 60 127.0.0.1 > nul", child.display());
    let outcome = run(&dir, &command, Duration::from_millis(1500));
    assert!(outcome.timed_out, "таймаут не сработал: {outcome:?}");
    assert!(wait_until(10, || !process_running(&name)), "после таймаута ребёнок остался жить: {name}");
}

#[test]
fn a_build_may_run_many_processes_at_once() {
    setup();
    let dir = temp_dir("many");
    // Как cargo или npm: двенадцать процессов одновременно.
    let outcome = run(&dir, "for /l %i in (1,1,12) do @start /b \"\" ping -n 2 127.0.0.1 > nul", Duration::from_secs(30));
    let err = read(&dir.join("err.txt"));
    assert_eq!(outcome.code, 0, "двенадцать процессов не запустились: {err}");
    assert!(err.trim().is_empty(), "ошибки запуска: {err}");
}

#[test]
fn spawn_returns_the_exit_code() {
    setup();
    let dir = temp_dir("code");
    let out = dir.join("out.txt");
    let err = dir.join("err.txt");
    let code = sandbox::spawn(&shell("exit 3"), &dir, &env(), Duration::from_secs(20), &out, &err).expect("запуск");
    assert_eq!(code, 3, "код выхода не дошёл");
    let _ = sandbox::spawn(&shell("echo sandbox-ok"), &dir, &env(), Duration::from_secs(20), &out, &err);
}

#[test]
fn status_explains_the_choice() {
    setup();
    let status = sandbox::status();
    // Видно при --nocapture: в этой среде AppContainer обычно не поднимается, и важно чем именно.
    eprintln!("песочница: {} — {}", status.level.name(), status.detail);
    assert!(!status.detail.is_empty(), "статус без объяснения");
    assert!(status.detail.contains("выбран"), "в статусе нет выбранного режима: {}", status.detail);
    if sandbox::mode() == sandbox::Mode::Auto {
        assert_ne!(status.level, Level::AppContainer, "auto не должен поднимать AppContainer");
    }
}

/// Есть ли живой процесс с таким именем. tasklist говорит на языке системы, но CSV-строки
/// с кавычкой одинаковы везде, поэтому смотрим только на них.
fn process_running(name: &str) -> bool {
    let output = Command::new("tasklist")
        .args(["/fi", &format!("imagename eq {name}.exe"), "/fo", "csv", "/nh"])
        .output();
    match output {
        Ok(output) => String::from_utf8_lossy(&output.stdout).lines().any(|line| line.trim_start().starts_with('"')),
        // Нет tasklist — не выдумываем результат.
        Err(_) => false,
    }
}
