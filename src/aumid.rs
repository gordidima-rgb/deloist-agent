//! Идентификатор приложения (AppUserModelID) для уведомлений Windows.
//!
//! Этот файл подключают два исполняемых файла: `deloist.exe` и загрузчик `deloist-setup` (через `#[path]`),
//! поэтому константа у них одна. Ярлык «Пуск» с этим идентификатором даёт тостам имя «Deloist»
//! вместо «Windows PowerShell».

#[cfg(windows)]
use std::path::Path;

/// Идентификатор, которым подписаны ярлык «Пуск» установки и тосты программы.
pub const AUMID: &str = "Deloist.Bots";

/// Ярлык «Пуск» от установки, относительно `%APPDATA%`.
#[cfg(windows)]
const START_MENU_LINK: &str = r"Microsoft\Windows\Start Menu\Programs\Deloist.lnk";

/// Подписывать ли окно нашим идентификатором. Решение одно на процесс:
/// - ярлык «Пуск» ведёт на этот exe и уже несёт `AUMID` → да;
/// - ярлык ведёт на этот exe, но без идентификатора (установка до 0.3.5: автообновление ярлык не трогает)
///   → дописываем идентификатор в тот же ярлык и тогда да;
/// - ярлыка нет или он на другую копию (отладочная сборка, Deloist Dev, портативная) → нет.
///   Иначе закреплённый значок на панели задач и окно разъехались бы на два значка.
#[cfg(windows)]
pub fn in_use() -> bool {
    static CHOICE: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *CHOICE.get_or_init(|| {
        let Ok(link) = start_menu_link() else { return false };
        let Ok(exe) = std::env::current_exe() else { return false };
        adopt(&link, &exe)
    })
}

/// Суть `in_use` без кэша, для тестов: `link` — ярлык, `exe` — путь этой программы.
#[cfg(windows)]
pub fn adopt(link: &Path, exe: &Path) -> bool {
    if !link.exists() {
        return false;
    }
    let Some((target, workdir)) = shortcut_target(link) else { return false };
    if !same_file(&target, exe) {
        return false;
    }
    if shortcut_app_id(link).as_deref() == Some(AUMID) {
        return true;
    }
    match write_shortcut(link, &target, &workdir, Some(AUMID)) {
        Ok(()) => true,
        Err(e) => {
            eprintln!("ярлык {}: идентификатор уведомлений не записан: {e}", link.display());
            false
        }
    }
}

#[cfg(windows)]
fn start_menu_link() -> Result<std::path::PathBuf, String> {
    let dir = std::env::var_os("APPDATA").filter(|dir| !dir.is_empty()).ok_or("не найдена папка меню «Пуск»")?;
    Ok(Path::new(&dir).join(START_MENU_LINK))
}

/// Отправитель уведомлений может использовать ярлык Deloist независимо от группировки окна на панели задач.
#[cfg(windows)]
pub fn notification_app_id() -> Result<&'static str, String> {
    static REGISTRATION: std::sync::Mutex<()> = std::sync::Mutex::new(());
    let _registration = REGISTRATION.lock().map_err(|e| e.to_string())?;
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    ensure_notification_shortcut(&start_menu_link()?, &exe)?;
    Ok(AUMID)
}

#[cfg(windows)]
fn ensure_notification_shortcut(link: &Path, exe: &Path) -> Result<(), String> {
    let alternate = link.with_file_name("Deloist Боты.lnk");
    let mut available = None;
    for candidate in [link, alternate.as_path()] {
        if !candidate.try_exists().map_err(|e| e.to_string())? {
            available.get_or_insert(candidate);
            continue;
        }
        if shortcut_app_id(candidate).as_deref() == Some(AUMID) {
            if let Some((target, _)) = shortcut_target(candidate) {
                if target.try_exists().map_err(|e| e.to_string())? {
                    return Ok(());
                }
                // Наш ярлык остался после переноса или удаления старой копии.
                available.get_or_insert(candidate);
            }
        } else if adopt(candidate, exe) {
            return Ok(());
        }
    }
    let candidate = available.ok_or("ярлыки Deloist в меню «Пуск» заняты другими приложениями")?;
    let menu = candidate.parent().ok_or("не найдена папка меню «Пуск»")?;
    let workdir = exe.parent().ok_or("не найдена папка Deloist")?;
    std::fs::create_dir_all(menu).map_err(|e| e.to_string())?;
    write_shortcut(candidate, exe, workdir, Some(AUMID))
}

#[cfg(windows)]
fn same_file(a: &Path, b: &Path) -> bool {
    match (std::fs::canonicalize(a), std::fs::canonicalize(b)) {
        (Ok(a), Ok(b)) => a == b,
        _ => false,
    }
}

/// Путь и рабочая папка ярлыка. Не читается — `None`.
#[cfg(windows)]
fn shortcut_target(link: &Path) -> Option<(std::path::PathBuf, std::path::PathBuf)> {
    use std::os::windows::ffi::OsStringExt;
    use windows::core::{Interface, HSTRING};
    use windows::Win32::System::Com::{CoCreateInstance, CoInitializeEx, IPersistFile, CLSCTX_INPROC_SERVER, COINIT_APARTMENTTHREADED, STGM_READ};
    use windows::Win32::UI::Shell::{IShellLinkW, ShellLink};

    fn text(buf: &[u16]) -> std::path::PathBuf {
        let end = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
        std::ffi::OsString::from_wide(&buf[..end]).into()
    }
    unsafe {
        let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
        let shell: IShellLinkW = CoCreateInstance(&ShellLink, None, CLSCTX_INPROC_SERVER).ok()?;
        let file: IPersistFile = shell.cast().ok()?;
        file.Load(&HSTRING::from(link.as_os_str()), STGM_READ).ok()?;
        let mut path = [0u16; 1024];
        shell.GetPath(&mut path, std::ptr::null_mut(), 0).ok()?;
        let mut dir = [0u16; 1024];
        shell.GetWorkingDirectory(&mut dir).ok()?;
        Some((text(&path), text(&dir)))
    }
}

/// Задаёт идентификатор процессу. Вызывать в начале `main`, до создания окон,
/// и только если `in_use()`.
#[cfg(windows)]
pub fn set_process_app_id() {
    use windows::core::HSTRING;
    use windows::Win32::UI::Shell::SetCurrentProcessExplicitAppUserModelID;
    // Ошибка здесь не мешает работе: у уведомлений свой явный идентификатор.
    let _ = unsafe { SetCurrentProcessExplicitAppUserModelID(&HSTRING::from(AUMID)) };
}

/// Создаёт ярлык через IShellLinkW и, если задан `app_id`, записывает PKEY_AppUserModel_ID через IPropertyStore.
#[cfg(windows)]
pub fn write_shortcut(link: &Path, target: &Path, workdir: &Path, app_id: Option<&str>) -> Result<(), String> {
    use windows::core::{Interface, HSTRING};
    use windows::Win32::Storage::EnhancedStorage::PKEY_AppUserModel_ID;
    use windows::Win32::System::Com::{CoCreateInstance, CoInitializeEx, IPersistFile, CLSCTX_INPROC_SERVER, COINIT_APARTMENTTHREADED};
    use windows::Win32::UI::Shell::PropertiesSystem::IPropertyStore;
    use windows::Win32::UI::Shell::{IShellLinkW, ShellLink};

    fn err(what: &str, e: windows::core::Error) -> String {
        format!("{what}: {e}")
    }
    unsafe {
        // Код возврата не важен: если COM уже поднят в этом потоке, это тоже годится.
        let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
        let shell: IShellLinkW = CoCreateInstance(&ShellLink, None, CLSCTX_INPROC_SERVER).map_err(|e| err("COM не создал ярлык", e))?;
        shell.SetPath(&HSTRING::from(target.as_os_str())).map_err(|e| err("путь ярлыка", e))?;
        shell.SetWorkingDirectory(&HSTRING::from(workdir.as_os_str())).map_err(|e| err("рабочая папка", e))?;
        shell.SetDescription(&HSTRING::from("Deloist")).map_err(|e| err("описание", e))?;
        if let Some(id) = app_id {
            let store: IPropertyStore = shell.cast().map_err(|e| err("свойства ярлыка", e))?;
            let value = prop_string(id)?;
            store.SetValue(&PKEY_AppUserModel_ID, &value).map_err(|e| err("AppUserModelID", e))?;
            store.Commit().map_err(|e| err("сохранение свойств", e))?;
        }
        let file: IPersistFile = shell.cast().map_err(|e| err("сохранение ярлыка", e))?;
        file.Save(&HSTRING::from(link.as_os_str()), true).map_err(|e| err("запись файла ярлыка", e))
    }
}

/// Значение VT_LPWSTR. Память берём у COM, поэтому `PropVariantClear` при уничтожении освободит её сам.
#[cfg(windows)]
unsafe fn prop_string(text: &str) -> Result<windows::Win32::System::Com::StructuredStorage::PROPVARIANT, String> {
    use std::mem::ManuallyDrop;
    use windows::core::PWSTR;
    use windows::Win32::System::Com::CoTaskMemAlloc;
    use windows::Win32::System::Com::StructuredStorage::{PROPVARIANT, PROPVARIANT_0, PROPVARIANT_0_0, PROPVARIANT_0_0_0};
    use windows::Win32::System::Variant::VT_LPWSTR;

    let units: Vec<u16> = text.encode_utf16().chain(std::iter::once(0)).collect();
    let buffer = CoTaskMemAlloc(units.len() * 2) as *mut u16;
    if buffer.is_null() {
        return Err("нет памяти под AppUserModelID".to_string());
    }
    std::ptr::copy_nonoverlapping(units.as_ptr(), buffer, units.len());
    Ok(PROPVARIANT {
        Anonymous: PROPVARIANT_0 {
            Anonymous: ManuallyDrop::new(PROPVARIANT_0_0 {
                vt: VT_LPWSTR,
                wReserved1: 0,
                wReserved2: 0,
                wReserved3: 0,
                Anonymous: PROPVARIANT_0_0_0 { pwszVal: PWSTR(buffer) },
            }),
        },
    })
}

/// Идентификатор, записанный в ярлыке. Нет файла, нет свойства или ярлык не читается — `None`.
#[cfg(windows)]
pub fn shortcut_app_id(link: &Path) -> Option<String> {
    use windows::core::{Interface, HSTRING};
    use windows::Win32::Storage::EnhancedStorage::PKEY_AppUserModel_ID;
    use windows::Win32::System::Com::StructuredStorage::PropVariantToStringAlloc;
    use windows::Win32::System::Com::{CoCreateInstance, CoInitializeEx, CoTaskMemFree, IPersistFile, CLSCTX_INPROC_SERVER, COINIT_APARTMENTTHREADED, STGM_READ};
    use windows::Win32::UI::Shell::PropertiesSystem::IPropertyStore;
    use windows::Win32::UI::Shell::{IShellLinkW, ShellLink};

    unsafe {
        let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
        let shell: IShellLinkW = CoCreateInstance(&ShellLink, None, CLSCTX_INPROC_SERVER).ok()?;
        let file: IPersistFile = shell.cast().ok()?;
        file.Load(&HSTRING::from(link.as_os_str()), STGM_READ).ok()?;
        let store: IPropertyStore = shell.cast().ok()?;
        let value = store.GetValue(&PKEY_AppUserModel_ID).ok()?;
        let text = PropVariantToStringAlloc(&value).ok()?;
        let id = text.to_string().ok();
        CoTaskMemFree(Some(text.0 as *const _));
        id
    }
}

#[cfg(all(test, windows))]
mod tests {
    use super::*;

    fn temp_dir() -> std::path::PathBuf {
        let stamp = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos();
        let dir = std::env::temp_dir().join(format!("deloist-aumid-{}-{stamp}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn registers_a_branded_shortcut_when_start_menu_entry_is_missing() {
        let dir = temp_dir();
        let link = dir.join("Deloist.lnk");
        let exe = std::env::current_exe().unwrap();
        ensure_notification_shortcut(&link, &exe).unwrap();
        assert_eq!(shortcut_app_id(&link).as_deref(), Some(AUMID));
        assert!(same_file(&shortcut_target(&link).unwrap().0, &exe));
        assert!(!link.with_file_name("Deloist Боты.lnk").exists());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn registers_beside_a_shortcut_to_the_old_deloist() {
        let dir = temp_dir();
        let link = dir.join("Deloist.lnk");
        write_shortcut(&link, &dir.join("old-deloist.exe"), &dir, Some("ru.deloist.desktop")).unwrap();
        let before = std::fs::read(&link).unwrap();
        let exe = std::env::current_exe().unwrap();

        ensure_notification_shortcut(&link, &exe).unwrap();
        let current = link.with_file_name("Deloist Боты.lnk");
        assert_eq!(shortcut_app_id(&current).as_deref(), Some(AUMID));
        assert!(same_file(&shortcut_target(&current).unwrap().0, &exe));
        let registered = std::fs::read(&current).unwrap();
        ensure_notification_shortcut(&link, &exe).unwrap();
        assert_eq!(std::fs::read(&current).unwrap(), registered);
        assert_eq!(std::fs::read(&link).unwrap(), before);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn only_the_shortcuts_target_adopts_its_window_identity() {
        let dir = temp_dir();
        let link = dir.join("Deloist.lnk");
        let exe = std::env::current_exe().unwrap();
        write_shortcut(&link, &exe, &dir, None).unwrap();
        assert!(adopt(&link, &exe));
        assert_eq!(shortcut_app_id(&link).as_deref(), Some(AUMID));

        let other_exe = dir.join("other.exe");
        std::fs::write(&other_exe, b"other").unwrap();
        assert!(!adopt(&link, &other_exe));
        ensure_notification_shortcut(&link, &other_exe).unwrap();
        assert!(same_file(&shortcut_target(&link).unwrap().0, &exe));
        assert!(!link.with_file_name("Deloist Боты.lnk").exists());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
