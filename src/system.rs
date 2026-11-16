//! Что приложение просит у Windows: автозапуск и «не засыпай, идёт работа». Только по выбору человека.

/// Команда автозапуска: приложение стартует в трее и не показывает окно.
#[cfg(windows)]
const RUN_KEY: &str = r"HKCU\Software\Microsoft\Windows\CurrentVersion\Run";
#[cfg(windows)]
const RUN_VALUE: &str = "Deloist";
/// Так значение называлось раньше: если оставить, приложение стартовало бы дважды.
#[cfg(windows)]
const OLD_RUN_VALUE: &str = "Crew";

#[cfg(windows)]
fn run_key() -> Option<String> {
    // В отладочной сборке настоящий автозапуск не трогаем никогда: тесты работают со своим ключом или ни с каким.
    if cfg!(debug_assertions) {
        return std::env::var("CREW_TEST_RUN_KEY").ok();
    }
    Some(RUN_KEY.to_string())
}

#[cfg(windows)]
pub fn set_autostart(enabled: bool) -> Result<(), String> {
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;

    let Some(key) = run_key() else { return Ok(()) };
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    let data = format!("\"{}\" --tray", exe.display());
    let mut command = std::process::Command::new("reg");
    if enabled {
        command.args(["add", &key, "/v", RUN_VALUE, "/t", "REG_SZ", "/d", &data, "/f"]);
    } else {
        command.args(["delete", &key, "/v", RUN_VALUE, "/f"]);
    }
    let output = command.creation_flags(CREATE_NO_WINDOW).output().map_err(|e| e.to_string())?;
    let _ = std::process::Command::new("reg")
        .args(["delete", &key, "/v", OLD_RUN_VALUE, "/f"])
        .creation_flags(CREATE_NO_WINDOW)
        .output();
    // Выключить уже выключенное не ошибка.
    if output.status.success() || !enabled {
        Ok(())
    } else {
        Err("Windows не дала включить автозапуск".to_string())
    }
}

#[cfg(not(windows))]
pub fn set_autostart(_enabled: bool) -> Result<(), String> {
    Err("автозапуск сделан только для Windows".to_string())
}

/// Ярлык «Deloist» на рабочем столе. Рабочий стол бывает перенесён в OneDrive,
/// поэтому путь к нему спрашиваем у Windows, а не собираем сами.
#[cfg(windows)]
pub fn create_desktop_shortcut() -> Result<(), String> {
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;

    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    let folder = exe.parent().map(|p| p.display().to_string()).unwrap_or_default();
    // Пути идут через переменные окружения, а не в текст команды: кавычки и апострофы в имени папки ничего не сломают.
    // Ярлык с тем же именем, но на другую программу не трогаем: «Deloist» на столе часто ярлык сайта из браузера.
    let script = "$d=[Environment]::GetFolderPath('Desktop'); \
        if (-not $env:DELOIST_LNK_DIR) { $env:DELOIST_LNK_DIR=$d }; \
        $w=New-Object -ComObject WScript.Shell; \
        $p=Join-Path $env:DELOIST_LNK_DIR ($env:DELOIST_LNK_NAME + '.lnk'); \
        if (Test-Path -LiteralPath $p) { if ($w.CreateShortcut($p).TargetPath -ne $env:DELOIST_EXE) { exit 3 } }; \
        $s=$w.CreateShortcut($p); \
        $s.TargetPath=$env:DELOIST_EXE; $s.WorkingDirectory=$env:DELOIST_DIR; \
        $s.IconLocation=$env:DELOIST_EXE + ',0'; $s.Description=$env:DELOIST_LNK_NAME; $s.Save()";
    let mut command = std::process::Command::new("powershell");
    command
        .args(["-NoProfile", "-NonInteractive", "-ExecutionPolicy", "Bypass", "-Command", script])
        .env("DELOIST_EXE", &exe)
        .env("DELOIST_DIR", folder)
        .env("DELOIST_LNK_NAME", SHORTCUT_NAME)
        .env_remove("DELOIST_LNK_DIR");
    // Тесты кладут ярлык в свою папку, а не на настоящий рабочий стол.
    if cfg!(debug_assertions) {
        let dir = std::env::var("CREW_TEST_SHORTCUT_DIR").map_err(|_| "ярлык в отладочной сборке не создаётся".to_string())?;
        command.env("DELOIST_LNK_DIR", dir);
    }
    let output = command.creation_flags(CREATE_NO_WINDOW).output().map_err(|e| e.to_string())?;
    match output.status.code() {
        Some(0) => Ok(()),
        Some(3) => Err(format!("На рабочем столе уже есть другой ярлык «{SHORTCUT_NAME}»: его не трогаю")),
        _ => Err("Windows не дала создать ярлык на рабочем столе".to_string()),
    }
}

/// Имя ярлыка. «Deloist» на столе обычно уже занят ярлыком сайта, поэтому программа с ботами называется отдельно.
pub const SHORTCUT_NAME: &str = "Deloist Боты";

#[cfg(not(windows))]
pub fn create_desktop_shortcut() -> Result<(), String> {
    Err("ярлык сделан только для Windows".to_string())
}

/// Сбрасывает таймер сна. Звать чаще, чем раз в минуту. Крышку ноутбука это не отменяет.
#[cfg(windows)]
pub fn hold_off_sleep() {
    use windows_sys::Win32::System::Power::{SetThreadExecutionState, ES_SYSTEM_REQUIRED};
    // SAFETY: функция без указателей, меняет только таймер простоя.
    unsafe {
        SetThreadExecutionState(ES_SYSTEM_REQUIRED);
    }
}

#[cfg(not(windows))]
pub fn hold_off_sleep() {}

/// Окно закрыто и большая часть памяти больше не нужна: возвращаем её системе, а не ждём, пока та попросит.
#[cfg(windows)]
pub fn release_idle_memory() {
    use windows_sys::Win32::System::Memory::SetProcessWorkingSetSizeEx;
    use windows_sys::Win32::System::Threading::GetCurrentProcess;
    // SAFETY: псевдоописатель своего процесса; значения MAX означают «обрезать рабочий набор».
    unsafe {
        SetProcessWorkingSetSizeEx(GetCurrentProcess(), usize::MAX, usize::MAX, 0);
    }
}

#[cfg(not(windows))]
pub fn release_idle_memory() {}

/// Что на этой системе и в этой сборке работает, а что нет. Интерфейс прячет кнопки того, чего нет.
pub fn platform_json() -> serde_json::Value {
    let cloud = crate::cloud::active();
    let window = cfg!(feature = "desktop");
    let windows_window = cfg!(all(windows, feature = "desktop"));
    #[cfg(windows)]
    let shell = !cloud;
    // Вне Windows песочницы нет: команды бота идут только если человек явно выключил её (контейнер).
    #[cfg(not(windows))]
    let shell = !cloud && crate::sandbox::refusal().is_none();
    serde_json::json!({
        "os": std::env::consts::OS,
        "window": window,
        "cloud": cloud,
        "features": {
            "tray": window,
            "dock": windows_window && !cloud,
            "pets": windows_window && !cloud,
            "screenCapture": cfg!(windows) && !cloud,
            "hotkeys": cfg!(windows) && !cloud,
            "selectionEdit": windows_window && !cloud,
            "shell": shell,
            "myChrome": cfg!(windows) && !cloud,
            // Политика облака: без консоли. Агенты и драйверы через командную строку, менеджеры паролей
            // (op, bw), самодоработка, локальная речь и Obscura выключены.
            "cliAgents": !cloud,
            "passwordManagers": !cloud,
            "selfdev": !cloud,
            "localStt": !cloud,
            "obscura": !cloud,
            "stdioMcp": !cloud,
        },
    })
}
