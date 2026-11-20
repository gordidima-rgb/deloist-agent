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
    match run_key() {
        Some(key) => set_autostart_at(&key, enabled),
        None => Ok(()),
    }
}

/// Запись автозапуска в указанный ключ реестра (`HKCU\...`). Тесты дают свой ключ, настоящий не трогают.
#[cfg(windows)]
fn set_autostart_at(key: &str, enabled: bool) -> Result<(), String> {
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;

    let key = key.to_string();
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

/// Нужна ли запись автозапуска: её держит и обычный автозапуск, и режим сервера. Выключить один, пока включён другой, нельзя.
pub fn run_entry_wanted(autostart: bool, server_mode: bool) -> bool {
    autostart || server_mode
}

/// Режим сервера есть только в Windows и не в облаке: там нет ни сна, ни автозапуска пользователя.
pub fn server_mode_supported() -> bool {
    cfg!(windows) && !crate::cloud::active()
}

/// То, что просим у системы про сон. Вынесено, чтобы проверить порядок вызовов без настоящей Windows.
pub trait ExecutionState: Send + Sync + 'static {
    /// `true` — «система не засыпай» (дисплей гаснуть может), `false` — снять запрет.
    fn set(&self, keep_awake: bool);
}

struct WindowsExecutionState;

#[cfg(windows)]
impl ExecutionState for WindowsExecutionState {
    fn set(&self, keep_awake: bool) {
        use windows_sys::Win32::System::Power::{SetThreadExecutionState, ES_CONTINUOUS, ES_SYSTEM_REQUIRED};
        // ES_CONTINUOUS действует, пока жива эта нить: без ES_DISPLAY_REQUIRED экран гаснуть может, план питания не меняется.
        // SAFETY: функция без указателей, меняет только состояние вызвавшей нити.
        unsafe {
            SetThreadExecutionState(if keep_awake { ES_CONTINUOUS | ES_SYSTEM_REQUIRED } else { ES_CONTINUOUS });
        }
    }
}

#[cfg(not(windows))]
impl ExecutionState for WindowsExecutionState {
    fn set(&self, _keep_awake: bool) {}
}

type SleepCommand = (bool, std::sync::mpsc::Sender<()>);

/// Запрет сна на отдельной долгой нити: `ES_CONTINUOUS` привязан к нити, которая его поставила,
/// поэтому ставить и снимать его надо на одной и той же нити. Нить кончается вместе с запретом (или с программой).
pub struct SleepLock {
    backend: std::sync::Arc<dyn ExecutionState>,
    thread: std::sync::Mutex<Option<std::sync::mpsc::Sender<SleepCommand>>>,
    held: std::sync::atomic::AtomicBool,
}

impl SleepLock {
    pub fn new(backend: std::sync::Arc<dyn ExecutionState>) -> Self {
        Self { backend, thread: std::sync::Mutex::new(None), held: std::sync::atomic::AtomicBool::new(false) }
    }

    /// Включить или снять запрет. Возвращается, когда нить ответила.
    pub fn set(&self, on: bool) {
        let mut slot = self.thread.lock().unwrap();
        if on && slot.is_none() {
            let (tx, rx) = std::sync::mpsc::channel::<SleepCommand>();
            let backend = self.backend.clone();
            let spawned = std::thread::Builder::new().name("deloist-keep-awake".into()).spawn(move || {
                for (keep, ack) in rx {
                    backend.set(keep);
                    let _ = ack.send(());
                }
                // Канал закрыт: запрет снимаем в любом случае.
                backend.set(false);
            });
            if spawned.is_err() {
                return;
            }
            *slot = Some(tx);
        }
        let wait_for = |tx: &std::sync::mpsc::Sender<SleepCommand>, keep: bool| {
            let (ack, wait) = std::sync::mpsc::channel();
            if tx.send((keep, ack)).is_ok() {
                let _ = wait.recv_timeout(std::time::Duration::from_secs(2));
            }
        };
        if on {
            if let Some(tx) = slot.as_ref() {
                wait_for(tx, true);
            }
        } else if let Some(tx) = slot.take() {
            wait_for(&tx, false);
        }
        self.held.store(on && slot.is_some(), std::sync::atomic::Ordering::SeqCst);
    }

    pub fn held(&self) -> bool {
        self.held.load(std::sync::atomic::Ordering::SeqCst)
    }
}

/// Единственный запрет сна режима сервера в этой программе.
pub fn sleep_lock() -> &'static SleepLock {
    static LOCK: std::sync::OnceLock<SleepLock> = std::sync::OnceLock::new();
    LOCK.get_or_init(|| SleepLock::new(std::sync::Arc::new(WindowsExecutionState)))
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
#[cfg(all(windows, feature = "desktop"))]
pub fn release_idle_memory() {
    use windows_sys::Win32::System::Memory::SetProcessWorkingSetSizeEx;
    use windows_sys::Win32::System::Threading::GetCurrentProcess;
    // SAFETY: псевдоописатель своего процесса; значения MAX означают «обрезать рабочий набор».
    unsafe {
        SetProcessWorkingSetSizeEx(GetCurrentProcess(), usize::MAX, usize::MAX, 0);
    }
}

#[cfg(all(not(windows), feature = "desktop"))]
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
            "serverMode": server_mode_supported(),
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
            // Связь с телефоном и другими ПК живёт в закрытой части; в открытой сборке её нет, кнопок тоже нет.
            "relay": cfg!(deloist_private),
        },
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};

    #[test]
    fn device_link_is_offered_only_with_the_private_part() {
        assert_eq!(platform_json()["features"]["relay"], cfg!(deloist_private));
    }

    #[derive(Default)]
    struct Recorder {
        calls: Mutex<Vec<(bool, std::thread::ThreadId)>>,
    }

    struct Shared(Arc<Recorder>);

    impl ExecutionState for Shared {
        fn set(&self, keep_awake: bool) {
            self.0.calls.lock().unwrap().push((keep_awake, std::thread::current().id()));
        }
    }

    #[test]
    fn the_run_entry_stays_while_either_switch_is_on() {
        assert!(!run_entry_wanted(false, false));
        assert!(run_entry_wanted(true, false));
        assert!(run_entry_wanted(false, true));
        assert!(run_entry_wanted(true, true));
    }

    #[test]
    fn sleep_is_held_and_released_on_one_dedicated_thread() {
        let recorder = Arc::new(Recorder::default());
        let lock = SleepLock::new(Arc::new(Shared(recorder.clone())));
        assert!(!lock.held());
        lock.set(true);
        lock.set(true);
        assert!(lock.held());
        lock.set(false);
        assert!(!lock.held());
        lock.set(false);
        // Нить снимает запрет ещё раз, когда видит закрытый канал: даём ей закончить.
        for _ in 0..100 {
            if recorder.calls.lock().unwrap().len() >= 4 {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        let calls = recorder.calls.lock().unwrap().clone();
        let flags: Vec<bool> = calls.iter().map(|(on, _)| *on).collect();
        // Включение, повтор, снятие; в конце нить закрывает канал и снимает запрет ещё раз.
        assert_eq!(flags, vec![true, true, false, false], "{flags:?}");
        let thread = calls[0].1;
        assert!(calls.iter().all(|(_, t)| *t == thread), "включали и снимали на разных нитях");
        assert_ne!(thread, std::thread::current().id(), "запрет ставится не на нити вызвавшего");
    }

    #[test]
    fn turning_it_on_again_after_off_starts_a_fresh_thread() {
        let recorder = Arc::new(Recorder::default());
        let lock = SleepLock::new(Arc::new(Shared(recorder.clone())));
        lock.set(true);
        lock.set(false);
        lock.set(true);
        assert!(lock.held());
        lock.set(false);
        std::thread::sleep(std::time::Duration::from_millis(50));
        let flags: Vec<bool> = recorder.calls.lock().unwrap().iter().map(|(on, _)| *on).collect();
        assert_eq!(flags.first(), Some(&true));
        assert_eq!(flags.iter().filter(|on| **on).count(), 2);
        assert_eq!(flags.last(), Some(&false));
    }

    /// Запись автозапуска пишется и удаляется в ключе самого теста, настоящий ключ Run не трогается.
    #[cfg(windows)]
    #[test]
    fn the_run_entry_is_written_and_removed_in_a_test_key() {
        let root = r"HKCU\Software\DeloistTest";
        let key = format!(r"{root}\Run-{}", std::process::id());
        let output = || std::process::Command::new("reg").args(["query", &key, "/v", RUN_VALUE]).output().unwrap();
        assert!(!output().status.success());
        // Среда с урезанными правами (Low integrity) не даёт писать в HKCU: проверять нечего.
        if set_autostart_at(&key, true).is_err() {
            eprintln!("HKCU недоступен для записи, проверка пропущена");
            return;
        }
        let found = output();
        assert!(found.status.success(), "запись не появилась");
        let text = String::from_utf8_lossy(&found.stdout).to_string();
        assert!(text.contains("--tray"), "{text}");
        set_autostart_at(&key, false).unwrap();
        assert!(!output().status.success(), "запись не удалилась");
        set_autostart_at(&key, false).unwrap();
        let _ = std::process::Command::new("reg").args(["delete", root, "/f"]).output();
    }
}
