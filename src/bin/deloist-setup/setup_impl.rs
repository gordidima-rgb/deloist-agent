// Deloist-Setup.exe: маленький загрузчик-установщик (около 1–2 МБ).
//
// Скачивает подписанное описание версии, проверяет подпись ключа выпуска, потом сам `deloist.exe`
// (по контрольной сумме из описания) и `WebView2Loader.dll`, кладёт их в папку пользователя,
// делает ярлыки и запись в «Приложения и возможности». Права администратора не нужны, поэтому
// автообновление программы продолжает подменять тот же `deloist.exe` рядом с собой.
//
//   Deloist-Setup.exe                          установка с вопросами
//   Deloist-Setup.exe --silent [--desktop] [--launch] [--dir <папка>]
//   uninstall.exe --uninstall [--silent] [--delete-data]   (копия загрузчика в папке установки)

// Тот же файл, что у программы: идентификатор уведомлений и запись ярлыка с ним.
#[path = "../../aumid.rs"]
#[allow(dead_code)] // программе нужна часть функций, которые установщику не нужны
mod aumid;
mod net;
mod sys;
mod ui;

use std::path::{Path, PathBuf};

use ed25519_dalek::{Signature, Verifier, VerifyingKey};
use serde::Deserialize;
use sha2::{Digest, Sha256};

/// Открытая часть ключа выпуска; тест сверяет её с ключом в `src/update.rs`.
const PUBLIC_KEY_HEX: &str = "1305c9147a9320549fd77d4ad8cfb8b958eacadc9d758b4bcd3b09f914b2a834";
const MANIFEST_URL: &str = "https://github.com/gordidima-rgb/deloist-agent/releases/latest/download/latest.json";
const MANIFEST_LIMIT: u64 = 64 * 1024;
const SIGNATURE_LIMIT: u64 = 1024;
const EXE_LIMIT: u64 = 300 * 1024 * 1024;
const LOADER_LIMIT: u64 = 8 * 1024 * 1024;
const UNINSTALL_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Uninstall\DeloistBots";
/// Файлы, которые кладёт установка и обновление. Больше при удалении не трогаем ничего.
const OWN_FILES: &[&str] = &["deloist.exe", "deloist.old.exe", "deloist.update.tmp", "WebView2Loader.dll", "WebView2Loader.old.dll", "uninstall.exe", "deloist.exe.setup", "WebView2Loader.dll.setup"];

pub fn w(text: &str) -> Vec<u16> {
    text.encode_utf16().chain(std::iter::once(0)).collect()
}

/// Подмены для тестов работают только в отладочной сборке: в выпуске их нет.
fn test_env(name: &str) -> Option<String> {
    if cfg!(debug_assertions) {
        std::env::var(name).ok().filter(|v| !v.is_empty())
    } else {
        None
    }
}

fn russian() -> bool {
    unsafe { windows_sys::Win32::Globalization::GetUserDefaultUILanguage() & 0xFF == 0x19 }
}

fn tr(ru: &str, en: &str) -> String {
    if russian() { ru } else { en }.to_string()
}

fn title() -> String {
    tr("Deloist Боты", "Deloist Bots")
}

// ---------------------------------------------------------------------------
// Описание версии
// ---------------------------------------------------------------------------

#[derive(Deserialize, Clone)]
struct Part {
    url: String,
    sha256: String,
    size: u64,
}

#[derive(Deserialize)]
struct Manifest {
    version: String,
    url: String,
    sha256: String,
    size: u64,
    loader: Option<Part>,
}

fn from_hex(text: &str) -> Option<Vec<u8>> {
    let text = text.trim();
    if text.len() % 2 != 0 {
        return None;
    }
    (0..text.len()).step_by(2).map(|i| u8::from_str_radix(text.get(i..i + 2)?, 16).ok()).collect()
}

fn verify(message: &[u8], signature_hex: &[u8]) -> Result<(), String> {
    let key_hex = test_env("DELOIST_SETUP_PUBKEY").unwrap_or_else(|| PUBLIC_KEY_HEX.to_string());
    let key: [u8; 32] = from_hex(&key_hex).and_then(|k| k.try_into().ok()).ok_or("ключ выпуска повреждён")?;
    let key = VerifyingKey::from_bytes(&key).map_err(|_| "ключ выпуска повреждён")?;
    let text = std::str::from_utf8(signature_hex).map_err(|_| "подпись повреждена")?;
    let bytes: [u8; 64] = from_hex(text).and_then(|s| s.try_into().ok()).ok_or("подпись повреждена")?;
    key.verify(message, &Signature::from_bytes(&bytes)).map_err(|_| "подпись не сошлась".to_string())
}

fn read_manifest() -> Result<Manifest, String> {
    let url = test_env("DELOIST_SETUP_MANIFEST_URL").unwrap_or_else(|| MANIFEST_URL.to_string());
    let bytes = net::get_bytes(&url, MANIFEST_LIMIT)?;
    let signature = net::get_bytes(&format!("{url}.sig"), SIGNATURE_LIMIT)?;
    // Сначала подпись и только потом разбор: непроверенные данные не читаем.
    verify(&bytes, &signature)?;
    serde_json::from_slice(&bytes).map_err(|_| "описание версии повреждено".to_string())
}

/// Скачивает файл во временное имя, проверяет размер и контрольную сумму. Возвращает путь к временному файлу.
fn download(part: &Part, limit: u64, temp: &Path, mut progress: impl FnMut(u64, u64)) -> Result<(), String> {
    use std::io::Write;
    if !cfg!(debug_assertions) && !part.url.starts_with("https://") {
        return Err("адрес файла небезопасен".to_string());
    }
    if part.size == 0 || part.size > limit {
        return Err("размер файла не подходит".to_string());
    }
    let expected = from_hex(&part.sha256).ok_or("описание версии повреждено")?;
    let mut file = std::fs::File::create(temp).map_err(|e| format!("нет права записи: {e}"))?;
    let mut hasher = Sha256::new();
    let mut received = 0u64;
    let mut head = Vec::new();
    let result = net::get(&part.url, part.size, &mut |chunk, _| {
        if head.len() < 2 {
            head.extend_from_slice(&chunk[..chunk.len().min(2)]);
        }
        hasher.update(chunk);
        file.write_all(chunk).map_err(|e| e.to_string())?;
        received += chunk.len() as u64;
        progress(received, part.size);
        Ok(())
    });
    drop(file);
    let checked = result.and_then(|_| {
        if received != part.size {
            Err("файл скачался не целиком".to_string())
        } else if hasher.finalize().as_slice() != expected.as_slice() {
            Err("контрольная сумма не сошлась".to_string())
        } else if !head.starts_with(b"MZ") {
            Err("это не программа для Windows".to_string())
        } else {
            Ok(())
        }
    });
    if checked.is_err() {
        let _ = std::fs::remove_file(temp);
    }
    checked
}

// ---------------------------------------------------------------------------
// Пути и параметры
// ---------------------------------------------------------------------------

#[derive(Clone)]
struct Options {
    dir: PathBuf,
    desktop: bool,
    launch: bool,
}

fn default_dir() -> PathBuf {
    if let Some(dir) = test_env("DELOIST_SETUP_DIR") {
        return PathBuf::from(dir);
    }
    let base = std::env::var_os("LOCALAPPDATA").map(PathBuf::from).unwrap_or_else(|| PathBuf::from(r"C:\Users\Public\AppData\Local"));
    base.join("Programs").join("Deloist")
}

fn registry_key() -> String {
    test_env("DELOIST_SETUP_REG").unwrap_or_else(|| UNINSTALL_KEY.to_string())
}

fn start_menu_link() -> PathBuf {
    let dir = test_env("DELOIST_SETUP_START_DIR").map(PathBuf::from).unwrap_or_else(|| {
        PathBuf::from(std::env::var_os("APPDATA").unwrap_or_default()).join(r"Microsoft\Windows\Start Menu\Programs")
    });
    dir.join("Deloist.lnk")
}

fn desktop_link() -> PathBuf {
    let dir = test_env("DELOIST_SETUP_DESKTOP_DIR")
        .map(PathBuf::from)
        .or_else(sys::desktop_dir)
        .unwrap_or_else(|| PathBuf::from(std::env::var_os("USERPROFILE").unwrap_or_default()).join("Desktop"));
    dir.join("Deloist.lnk")
}

// ---------------------------------------------------------------------------
// Установка
// ---------------------------------------------------------------------------

/// Папка установки: пустая, новая или уже наша. В чужую папку с файлами не ставим:
/// удаление вместе с данными снесло бы там чужой `data`.
fn check_target_dir(dir: &Path) -> Result<(), String> {
    if dir.join("deloist.exe").is_file() {
        return Ok(());
    }
    let occupied = std::fs::read_dir(dir).map(|mut entries| entries.next().is_some()).unwrap_or(false);
    if occupied {
        return Err(format!("{} {}", tr("Папка уже занята другими файлами:", "The folder already holds other files:"), dir.display()));
    }
    Ok(())
}

/// Удаляет перечисленные файлы, когда значение уходит из области видимости: временные
/// файлы не остаются ни на одном пути ошибки.
struct Cleanup(Vec<PathBuf>);

impl Drop for Cleanup {
    fn drop(&mut self) {
        for path in &self.0 {
            let _ = std::fs::remove_file(path);
        }
    }
}

/// Файл, поставленный на место; прежний лежит рядом под именем `.old`, чтобы откатить.
struct Placed {
    target: PathBuf,
    backup: Option<PathBuf>,
}

impl Placed {
    fn undo(self) {
        let _ = std::fs::remove_file(&self.target);
        if let Some(backup) = self.backup {
            let _ = std::fs::rename(backup, &self.target);
        }
    }
}

fn backup_name(target: &Path) -> PathBuf {
    let ext = target.extension().and_then(|e| e.to_str()).unwrap_or("bin");
    target.with_extension(format!("old.{ext}"))
}

/// Работающий файл нельзя перезаписать, но можно переименовать: так же подменяет себя автообновление
/// (для exe) и так же обходим запущенную программу, которая держит свою dll.
fn replace_file(temp: &Path, target: &Path) -> Result<Placed, String> {
    let mut backup = None;
    if target.exists() {
        let old = backup_name(target);
        let _ = std::fs::remove_file(&old);
        std::fs::rename(target, &old).map_err(|e| format!("не смог отодвинуть старый {}: {e}", target.display()))?;
        backup = Some(old);
    }
    match std::fs::rename(temp, target) {
        Ok(()) => Ok(Placed { target: target.to_path_buf(), backup }),
        Err(e) => {
            if let Some(old) = backup {
                let _ = std::fs::rename(old, target);
            }
            Err(format!("не смог положить файл на место: {e}"))
        }
    }
}

fn ensure_webview(report: &dyn Fn(u32, &str)) -> Result<(), String> {
    if test_env("DELOIST_SETUP_SKIP_WEBVIEW").is_some() || sys::webview2_installed() {
        return Ok(());
    }
    report(90, &tr("Ставлю Microsoft Edge WebView2 (Windows спросит разрешение)", "Installing Microsoft Edge WebView2 (Windows will ask for permission)"));
    let temp = std::env::temp_dir().join(format!("MicrosoftEdgeWebview2Setup-{:016x}.exe", rand::random::<u64>()));
    let _cleanup = Cleanup(vec![temp.clone()]);
    let bytes = net::get_bytes(sys::WEBVIEW_BOOTSTRAPPER, 16 * 1024 * 1024)?;
    {
        use std::io::Write;
        // create_new: чужой файл с таким именем не перезаписываем и не подхватываем.
        let mut file = std::fs::OpenOptions::new().write(true).create_new(true).open(&temp).map_err(|e| e.to_string())?;
        file.write_all(&bytes).map_err(|e| e.to_string())?;
    }
    if !bytes.starts_with(b"MZ") || !sys::authenticode_valid(&temp) || !sys::signed_by_microsoft(&temp) {
        return Err("установщик WebView2 без действующей подписи Microsoft Corporation".to_string());
    }
    let status = std::process::Command::new(&temp).args(["/silent", "/install"]).status().map_err(|e| e.to_string())?;
    if !status.success() && !sys::webview2_installed() {
        return Err("не удалось поставить Microsoft Edge WebView2".to_string());
    }
    Ok(())
}

/// Возвращает предупреждения: ярлыки не критичны, программа без них работает.
fn install(options: &Options, report: &dyn Fn(u32, &str)) -> Result<Vec<String>, String> {
    report(2, &tr("Проверяю последнюю версию", "Checking the latest version"));
    check_target_dir(&options.dir)?;
    let manifest = read_manifest()?;
    // С 0.3.4 загрузчик WebView2 внутри exe, поля `loader` в новых описаниях нет; у старых версий оно есть.
    let loader = manifest.loader.clone();
    std::fs::create_dir_all(&options.dir).map_err(|e| format!("не создал папку {}: {e}", options.dir.display()))?;

    let exe_tmp = options.dir.join("deloist.exe.setup");
    let dll_tmp = options.dir.join("WebView2Loader.dll.setup");
    // Временные файлы убираются при любом выходе из функции, в том числе по ошибке.
    let _cleanup = Cleanup(vec![exe_tmp.clone(), dll_tmp.clone()]);
    let label = tr("Скачиваю Deloist", "Downloading Deloist");
    let exe_part = Part { url: manifest.url.clone(), sha256: manifest.sha256.clone(), size: manifest.size };
    let mut last = 0;
    download(&exe_part, EXE_LIMIT, &exe_tmp, |done, total| {
        let percent = 5 + (done * 80 / total) as u32;
        if percent != last {
            last = percent;
            report(percent, &format!("{label}: {} / {} MB", done / 1_048_576, total / 1_048_576));
        }
    })?;
    if let Some(loader) = &loader {
        download(loader, LOADER_LIMIT, &dll_tmp, |_, _| {})?;
    }
    ensure_webview(report)?;

    report(93, &tr("Устанавливаю", "Installing"));
    let dll = match &loader {
        Some(_) => Some(replace_file(&dll_tmp, &options.dir.join("WebView2Loader.dll"))?),
        None => None,
    };
    let exe_path = options.dir.join("deloist.exe");
    let exe = match replace_file(&exe_tmp, &exe_path) {
        Ok(placed) => placed,
        Err(e) => {
            undo_loader(dll);
            return Err(e);
        }
    };
    let me = std::env::current_exe().map_err(|e| e.to_string())?;
    let uninstaller = options.dir.join("uninstall.exe");
    if me != uninstaller {
        let _ = std::fs::remove_file(&uninstaller);
        if let Err(e) = std::fs::copy(&me, &uninstaller) {
            exe.undo();
            undo_loader(dll);
            return Err(format!("не положил uninstall.exe: {e}"));
        }
    }

    // Сначала запись в «Приложениях»: без неё программу нечем удалить штатно.
    if let Err(e) = register(&options.dir, &manifest.version) {
        let _ = std::fs::remove_file(&uninstaller);
        exe.undo();
        undo_loader(dll);
        return Err(e);
    }
    let mut warnings = Vec::new();
    if let Err(e) = sys::make_shortcut(&start_menu_link(), &exe_path, &options.dir) {
        warnings.push(e);
    }
    if options.desktop {
        if let Err(e) = sys::make_shortcut(&desktop_link(), &exe_path, &options.dir) {
            warnings.push(e);
        }
    }
    report(100, &tr("Готово", "Done"));
    Ok(warnings)
}

fn undo_loader(dll: Option<Placed>) {
    if let Some(dll) = dll {
        dll.undo();
    }
}

fn register(dir: &Path, version: &str) -> Result<(), String> {
    let key = registry_key();
    let uninstall = dir.join("uninstall.exe");
    sys::set_string(&key, "DisplayName", &title())?;
    sys::set_string(&key, "DisplayVersion", version)?;
    sys::set_string(&key, "Publisher", "Deloist")?;
    sys::set_string(&key, "InstallLocation", &dir.to_string_lossy())?;
    sys::set_string(&key, "DisplayIcon", &dir.join("deloist.exe").to_string_lossy())?;
    sys::set_string(&key, "UninstallString", &format!("\"{}\" --uninstall", uninstall.display()))?;
    sys::set_dword(&key, "NoModify", 1)?;
    sys::set_dword(&key, "NoRepair", 1)
}

fn launch(dir: &Path) {
    use std::os::windows::process::CommandExt;
    // DETACHED_PROCESS | CREATE_NEW_PROCESS_GROUP: программа живёт сама, после закрытия загрузчика.
    let _ = std::process::Command::new(dir.join("deloist.exe")).current_dir(dir).creation_flags(0x0000_0208).spawn();
}

// ---------------------------------------------------------------------------
// Удаление
// ---------------------------------------------------------------------------

/// Убирает ярлыки и запись в реестре. Файлы удаляет вторая копия загрузчика, когда эта закроется.
fn unregister() {
    let _ = std::fs::remove_file(start_menu_link());
    let _ = std::fs::remove_file(desktop_link());
    sys::delete_tree(&registry_key());
}

/// Удаляет только то, что кладёт установка, и `data`, если просили. Чужие файлы в папке остаются.
fn remove_files(dir: &Path, delete_data: bool) -> Result<(), String> {
    let mut failed = Vec::new();
    for name in OWN_FILES {
        let path = dir.join(name);
        if path.exists() && std::fs::remove_file(&path).is_err() {
            failed.push(name.to_string());
        }
    }
    if delete_data {
        let data = dir.join("data");
        if data.exists() && std::fs::remove_dir_all(&data).is_err() {
            failed.push("data".to_string());
        }
        let _ = std::fs::remove_file(dir.join(".env"));
    }
    let _ = std::fs::remove_dir(dir);
    if failed.is_empty() { Ok(()) } else { Err(failed.join(", ")) }
}

fn clean_temp_copies() {
    if let Ok(entries) = std::fs::read_dir(std::env::temp_dir()) {
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().to_string();
            if name.starts_with("deloist-uninstall-") && name.ends_with(".exe") {
                let _ = std::fs::remove_file(entry.path());
            }
        }
    }
}

fn is_named(path: &Path, name: &str) -> bool {
    path.file_name().is_some_and(|n| n.to_string_lossy().eq_ignore_ascii_case(name))
}

/// Удаление запускается только из `uninstall.exe`, у которого рядом лежит `deloist.exe`:
/// скачанный в «Загрузки» Deloist-Setup.exe с ключом `--uninstall` ничего не сотрёт.
fn uninstall_dir() -> Option<PathBuf> {
    let me = std::env::current_exe().ok()?;
    let dir = me.parent()?.to_path_buf();
    (is_named(&me, "uninstall.exe") && dir.join("deloist.exe").is_file()).then_some(dir)
}

fn uninstall(silent: bool, delete_data_flag: bool) {
    use std::os::windows::process::CommandExt;
    use windows_sys::Win32::UI::WindowsAndMessaging::{MB_ICONQUESTION, MB_YESNO};
    let (Some(dir), Ok(me)) = (uninstall_dir(), std::env::current_exe()) else {
        if !silent {
            ui::error(&tr("Удаление запускается только из папки установленной программы (uninstall.exe).", "Uninstall only runs from the installed program folder (uninstall.exe)."), &title());
        }
        std::process::exit(2);
    };
    let mut delete_data = delete_data_flag;
    if !silent {
        let question = tr("Удалить Deloist Боты с этого компьютера?", "Remove Deloist Bots from this computer?");
        if ui::ask(&question, &title(), MB_YESNO | MB_ICONQUESTION) != ui::YES {
            return;
        }
        let keep = tr(
            "Оставить твои данные (боты, память, разговоры, ключи)?\n\nДа — оставить, их можно будет вернуть повторной установкой.\nНет — удалить всё.",
            "Keep your data (bots, memory, chats, keys)?\n\nYes - keep it, a new install will pick it up.\nNo - delete everything.",
        );
        delete_data = ui::ask(&keep, &title(), MB_YESNO | MB_ICONQUESTION) != ui::YES;
    }
    unregister();
    // Эту копию Windows держит открытой, поэтому файлы чистит копия во временной папке.
    let copy = std::env::temp_dir().join(format!("deloist-uninstall-{}.exe", std::process::id()));
    let spawned = std::fs::copy(&me, &copy).is_ok()
        && std::process::Command::new(&copy)
            .arg("--finish-uninstall")
            .arg(&dir)
            .arg(if delete_data { "1" } else { "0" })
            .arg(if silent { "silent" } else { "ask" })
            .creation_flags(0x0800_0000)
            .spawn()
            .is_ok();
    if !spawned && !silent {
        ui::error(&tr("Не удалось закончить удаление. Удали папку вручную.", "Could not finish the removal. Delete the folder by hand."), &title());
    }
}

fn finish_uninstall(dir: &Path, delete_data: bool, silent: bool) {
    // Только временная копия, запущенная самим удалением, и только для папки с нашим uninstall.exe.
    let from_temp = std::env::current_exe().ok().is_some_and(|me| me.file_name().is_some_and(|n| n.to_string_lossy().starts_with("deloist-uninstall-")));
    if !from_temp || !dir.is_absolute() || !dir.join("uninstall.exe").is_file() {
        std::process::exit(2);
    }
    // Главная копия закрывается не мгновенно: пробуем несколько секунд.
    let mut result = remove_files(dir, delete_data);
    for _ in 0..20 {
        if result.is_ok() {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(500));
        result = remove_files(dir, delete_data);
    }
    clean_temp_copies();
    if silent {
        return;
    }
    match result {
        Ok(()) => ui::info(&tr("Deloist Боты удалён.", "Deloist Bots was removed."), &title()),
        Err(left) => ui::error(&format!("{} {left}", tr("Не удалось удалить: ", "Could not remove: ")), &title()),
    }
}

// ---------------------------------------------------------------------------
// Точка входа
// ---------------------------------------------------------------------------

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let has = |flag: &str| args.iter().any(|a| a == flag);
    let value = |flag: &str| args.iter().position(|a| a == flag).and_then(|i| args.get(i + 1)).cloned();

    if let Some(i) = args.iter().position(|a| a == "--finish-uninstall") {
        if let (Some(dir), Some(data), Some(mode)) = (args.get(i + 1), args.get(i + 2), args.get(i + 3)) {
            finish_uninstall(Path::new(dir), data == "1", mode == "silent");
        }
        return;
    }
    if has("--uninstall") {
        uninstall(has("--silent"), has("--delete-data"));
        return;
    }

    unsafe { windows_sys::Win32::UI::WindowsAndMessaging::SetProcessDPIAware() };
    let dir = value("--dir").map(PathBuf::from).unwrap_or_else(default_dir);
    let dir = match std::path::absolute(&dir) {
        Ok(dir) => dir,
        Err(e) => {
            ui::error(&format!("{}: {e}", dir.display()), &title());
            std::process::exit(1);
        }
    };
    if has("--silent") {
        let options = Options { dir, desktop: has("--desktop"), launch: has("--launch") };
        match install(&options, &|_, _| {}) {
            Ok(_) => {
                if options.launch {
                    launch(&options.dir);
                }
            }
            Err(e) => {
                eprintln!("{e}");
                std::process::exit(1);
            }
        }
        return;
    }

    use windows_sys::Win32::UI::WindowsAndMessaging::{MB_ICONQUESTION, MB_YESNO, MB_YESNOCANCEL};
    if let Err(e) = check_target_dir(&dir) {
        ui::error(&e, &title());
        return;
    }
    let question = format!(
        "{}\n\n{}\n\n{}",
        tr("Установить Deloist Боты?", "Install Deloist Bots?"),
        format_args!("{} {}", tr("Папка:", "Folder:"), dir.display()),
        tr(
            "Да — установить и добавить ярлык на рабочий стол.\nНет — установить без ярлыка на рабочем столе (ярлык в меню «Пуск» будет).\nОтмена — выйти.\n\nПрава администратора не нужны.",
            "Yes - install and add a desktop shortcut.\nNo - install without a desktop shortcut (a Start menu shortcut is always added).\nCancel - quit.\n\nAdministrator rights are not required.",
        ),
    );
    let answer = ui::ask(&question, &title(), MB_YESNOCANCEL | MB_ICONQUESTION);
    if answer != ui::YES && answer != ui::NO {
        return;
    }
    let options = Options { dir: dir.clone(), desktop: answer == ui::YES, launch: false };
    let worker_options = options.clone();
    let result = ui::with_progress(&title(), move |reporter| install(&worker_options, &|p, t| reporter.report(p, t)));
    match result {
        Ok(warnings) => {
            let mut text = tr("Готово. Запустить Deloist Боты сейчас?", "Done. Start Deloist Bots now?");
            if !warnings.is_empty() {
                text = format!("{text}\n\n{}\n{}", tr("Не всё получилось:", "Not everything worked:"), warnings.join("\n"));
            }
            if ui::ask(&text, &title(), MB_YESNO | MB_ICONQUESTION) == ui::YES {
                launch(&dir);
            }
        }
        Err(e) => ui::error(&format!("{}\n\n{e}", tr("Установка не получилась.", "Installation failed.")), &title()),
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn public_key_matches_the_updater() {
        let updater = include_str!("../../update.rs");
        assert!(updater.contains(&format!("PUBLIC_KEY_HEX: &str = \"{}\"", super::PUBLIC_KEY_HEX)), "ключ загрузчика разошёлся с ключом обновлений");
        assert!(updater.contains(super::MANIFEST_URL), "адрес описания версии разошёлся");
    }

    #[test]
    fn hex_refuses_garbage() {
        assert!(super::from_hex("zz").is_none());
        assert!(super::from_hex("abc").is_none());
        assert_eq!(super::from_hex("0aff").unwrap(), vec![10, 255]);
    }

    #[test]
    fn foreign_non_empty_folder_is_refused_but_ours_and_empty_are_fine() {
        let root = std::env::temp_dir().join(format!("deloist-setup-dir-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        assert!(super::check_target_dir(&root).is_ok(), "новая папка");
        std::fs::create_dir_all(&root).unwrap();
        assert!(super::check_target_dir(&root).is_ok(), "пустая папка");
        std::fs::write(root.join("other.txt"), "x").unwrap();
        assert!(super::check_target_dir(&root).is_err(), "чужие файлы");
        std::fs::write(root.join("deloist.exe"), "x").unwrap();
        assert!(super::check_target_dir(&root).is_ok(), "наша установка");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn backup_keeps_the_extension() {
        assert_eq!(super::backup_name(std::path::Path::new(r"C:\a\WebView2Loader.dll")), std::path::PathBuf::from(r"C:\a\WebView2Loader.old.dll"));
        assert_eq!(super::backup_name(std::path::Path::new(r"C:\a\deloist.exe")), std::path::PathBuf::from(r"C:\a\deloist.old.exe"));
    }

    #[test]
    fn desktop_folder_survives_a_cyrillic_profile() {
        let desktop = super::sys::desktop_dir().expect("рабочий стол");
        assert!(desktop.is_dir(), "{}", desktop.display());
        assert!(!desktop.to_string_lossy().contains('?'));
    }
}
