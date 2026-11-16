//! Запуск команд оболочки так же на всех системах.

use std::ffi::OsString;

use tokio::process::Command;

/// Что нужно обычной команде для работы. Всё остальное из окружения, а там лежат ключи,
/// команда бота не видит.
pub(crate) const SAFE_ENV: &[&str] = &[
    "PATH", "PATHEXT", "SYSTEMROOT", "SYSTEMDRIVE", "WINDIR", "COMSPEC", "TEMP", "TMP", "USERPROFILE", "USERNAME",
    "HOMEDRIVE", "HOMEPATH", "APPDATA", "LOCALAPPDATA", "PROGRAMDATA", "PROGRAMFILES", "PROGRAMFILES(X86)",
    "PROGRAMW6432", "COMMONPROGRAMFILES", "COMMONPROGRAMFILES(X86)", "NUMBER_OF_PROCESSORS",
    "PROCESSOR_ARCHITECTURE", "OS", "HOME", "USER", "LANG", "LC_ALL", "TERM",
];

/// Команда целиком уходит оболочке. На Windows это `cmd.exe /d /s /c "…"` без
/// переэкранирования аргументов и без всплывающего окна консоли.
pub fn shell_command(command: &str) -> Command {
    #[cfg(windows)]
    let mut cmd = {
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        let comspec = std::env::var("ComSpec").unwrap_or_else(|_| "cmd.exe".to_string());
        let mut cmd = Command::new(comspec);
        cmd.raw_arg(format!("/d /s /c \"{command}\""));
        cmd.creation_flags(CREATE_NO_WINDOW);
        cmd
    };
    #[cfg(not(windows))]
    let mut cmd = {
        let mut cmd = Command::new("/bin/sh");
        cmd.arg("-c").arg(command);
        cmd
    };
    cmd.env_remove("CREW_BOOT_TOKEN");
    // В облаке ключ шифрования секретов main уже убрал из окружения; это второй замок.
    cmd.env_remove("CREW_SECRETS_KEY");
    cmd
}

/// Окружение команды в виде пар: его же получает `sandbox::spawn`, которому нужен явный блок.
pub fn safe_env_pairs() -> Vec<(OsString, OsString)> {
    let mut out = Vec::new();
    for (name, value) in std::env::vars_os() {
        let upper = name.to_string_lossy().to_uppercase();
        if SAFE_ENV.contains(&upper.as_str()) {
            out.push((name, value));
        }
    }
    out
}

/// Для `run_command`: окружение только из короткого списка.
pub fn keep_safe_env(cmd: &mut Command) {
    cmd.env_clear();
    for (name, value) in safe_env_pairs() {
        cmd.env(name, value);
    }
}

/// Командная строка для `CreateProcess`: он не собирает аргументы сам, поэтому `cmd.exe`
/// и его ключи пишем одной строкой — так же, как их получает [`shell_command`].
pub fn shell_command_line(command: &str) -> String {
    let comspec = std::env::var("ComSpec").unwrap_or_else(|_| "cmd.exe".to_string());
    format!("\"{comspec}\" /d /s /c \"{command}\"")
}
