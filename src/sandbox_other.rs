//! Изоляции команд вне Windows нет. Те же имена, что у `sandbox.rs`, но закрытым по умолчанию:
//! команда бота не запускается вовсе, пока человек сам не скажет `CREW_SANDBOX=off`
//! (так, например, делает контейнер: изоляцию даёт он). Молча без клетки ничего не выполняется.

use std::ffi::OsString;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

static SHUTTING_DOWN: AtomicBool = AtomicBool::new(false);

#[allow(dead_code)]
pub fn shutdown() {
    SHUTTING_DOWN.store(true, Ordering::SeqCst);
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Level {
    AppContainer,
    Off,
}

impl Level {
    pub fn name(self) -> &'static str {
        match self {
            Level::AppContainer => "AppContainer",
            Level::Off => "Off",
        }
    }
}

#[derive(Clone, Debug)]
pub struct Status {
    pub level: Level,
    pub detail: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    Off,
    Auto,
    Strict,
}

impl Mode {
    #[allow(dead_code)]
    pub fn name(self) -> &'static str {
        match self {
            Mode::Off => "off",
            Mode::Auto => "auto",
            Mode::Strict => "strict",
        }
    }
}

/// Разбор `CREW_SANDBOX`. Незнакомое значение — `auto`: молча выключать защиту нельзя.
pub fn parse_mode(raw: &str) -> Mode {
    match raw.trim().to_ascii_lowercase().as_str() {
        "off" | "none" | "0" => Mode::Off,
        "strict" | "appcontainer" | "app-container" => Mode::Strict,
        _ => Mode::Auto,
    }
}

pub fn mode() -> Mode {
    parse_mode(&std::env::var("CREW_SANDBOX").unwrap_or_default())
}

pub fn status() -> Status {
    Status { level: Level::Off, detail: "на этой системе песочницы нет".into() }
}

/// Почему команду нельзя запускать. `None` — человек явно выключил песочницу (`CREW_SANDBOX=off`).
pub fn refusal() -> Option<String> {
    (mode() != Mode::Off)
        .then(|| "на этой системе нет песочницы для команд. Запусти в контейнере и поставь CREW_SANDBOX=off, если изоляцию даёт он".to_string())
}

#[derive(Clone, Copy, Debug)]
pub struct Outcome {
    pub code: i32,
    pub timed_out: bool,
    pub cancelled: bool,
}

#[allow(dead_code)]
pub fn spawn(_cmdline: &str, _cwd: &Path, _env: &[(OsString, OsString)], _timeout: Duration, _out: &Path, _err: &Path) -> Result<i32, String> {
    Err("песочница недоступна на этой системе".into())
}

pub fn spawn_ex(_cmdline: &str, _cwd: &Path, _env: &[(OsString, OsString)], _timeout: Duration, _out: &Path, _err: &Path) -> Result<Outcome, String> {
    Err("песочница недоступна на этой системе".into())
}

pub fn spawn_ex_cancellable(
    _cmdline: &str, _cwd: &Path, _env: &[(OsString, OsString)], _timeout: Duration,
    _out: &Path, _err: &Path, _cancel: Option<&tokio_util::sync::CancellationToken>,
) -> Result<Outcome, String> {
    Err("песочница недоступна на этой системе".into())
}

/// Текстовый слой для явного `CREW_SANDBOX=off`: личные папки и файлы секретов программы.
///
/// На Linux это не граница безопасности: команду можно собрать так, что подстроки не видно
/// (`cat $(printf ...)`, переменные, base64). Границей служит контейнер (облако) и права
/// процесса: ключ шифрования закрыт от других процессов через `PR_SET_DUMPABLE=0`.
pub fn looks_protected(command: &str) -> Option<&'static str> {
    let lower = command.to_lowercase();
    for needle in [".config/deloist", "deloist/secrets"] {
        if lower.contains(needle) {
            return Some("обращение к папке ключа шифрования секретов (deloist)");
        }
    }
    if let Some(key) = crate::secrets::key_file_path() {
        let key = key.to_string_lossy().to_lowercase();
        if !key.is_empty() && lower.contains(&key) {
            return Some("обращение к ключу шифрования секретов (secrets.key)");
        }
    }
    for (needle, reason) in [
        ("$home", "обращение к домашней папке ($HOME)"),
        ("${home}", "обращение к домашней папке ($HOME)"),
        ("~/", "обращение к домашней папке (~)"),
    ] {
        if lower.contains(needle) {
            return Some(reason);
        }
    }
    for (name, reason) in [
        (".ssh", "обращение к ключам SSH (.ssh)"),
        ("secrets.json", "обращение к файлу секретов (secrets.json)"),
        ("connectors.bin", "обращение к файлу секретов подключений (connectors.bin)"),
        ("secrets.key", "обращение к ключу шифрования секретов (secrets.key)"),
    ] {
        if lower.split(|c: char| matches!(c, '/' | '"' | '\'' | ' ' | '\t' | '(' | '=' | ';' | '&' | '|' | ',' | ':' | '\n')).any(|part| part == name) {
            return Some(reason);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn refuses_unless_explicitly_off() {
        assert_eq!(parse_mode("off"), Mode::Off);
        assert_eq!(parse_mode("garbage"), Mode::Auto);
        assert_eq!(status().level, Level::Off);
        assert!(spawn_ex("true", Path::new("."), &[], Duration::from_secs(1), Path::new("o"), Path::new("e")).is_err());
    }

    #[test]
    fn text_layer_catches_secrets() {
        assert!(looks_protected("cat ~/.ssh/id_rsa").is_some());
        assert!(looks_protected("cat $HOME/x").is_some());
        assert!(looks_protected("cat data/secrets.json").is_some());
        assert!(looks_protected("ls -la").is_none());
    }
}
