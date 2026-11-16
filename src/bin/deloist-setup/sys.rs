//! Системные мелочи установки: реестр текущего пользователя, ярлыки, проверка WebView2.
//! Всё пишется только в `HKCU` и в папки пользователя: права администратора не нужны.

use std::path::Path;
use std::ptr::{null, null_mut};

use windows_sys::Win32::Foundation::INVALID_HANDLE_VALUE;
use windows_sys::Win32::Security::WinTrust::*;
use windows_sys::Win32::System::Registry::*;

use crate::w;

const WEBVIEW_CLIENT: &str = r"Microsoft\EdgeUpdate\Clients\{F3017226-FE2A-4295-8BDF-00C3A9A7E4C5}";
/// Постоянный адрес установщика Evergreen Runtime на сайте Microsoft.
pub const WEBVIEW_BOOTSTRAPPER: &str = "https://go.microsoft.com/fwlink/p/?LinkId=2124703";

fn read_string(root: HKEY, subkey: &str, name: &str) -> Option<String> {
    let mut size: u32 = 0;
    let (key, value) = (w(subkey), w(name));
    unsafe {
        if RegGetValueW(root, key.as_ptr(), value.as_ptr(), RRF_RT_REG_SZ, null_mut(), null_mut(), &mut size) != 0 || size < 2 {
            return None;
        }
        let mut buffer = vec![0u16; (size as usize).div_ceil(2)];
        if RegGetValueW(root, key.as_ptr(), value.as_ptr(), RRF_RT_REG_SZ, null_mut(), buffer.as_mut_ptr() as *mut _, &mut size) != 0 {
            return None;
        }
        let len = buffer.iter().position(|&c| c == 0).unwrap_or(buffer.len());
        Some(String::from_utf16_lossy(&buffer[..len]))
    }
}

/// Стоит ли на машине Microsoft Edge WebView2 Runtime (без него окно программы не откроется).
pub fn webview2_installed() -> bool {
    [
        (HKEY_LOCAL_MACHINE, format!(r"SOFTWARE\WOW6432Node\{WEBVIEW_CLIENT}")),
        (HKEY_LOCAL_MACHINE, format!(r"SOFTWARE\{WEBVIEW_CLIENT}")),
        (HKEY_CURRENT_USER, format!(r"Software\{WEBVIEW_CLIENT}")),
    ]
    .iter()
    .any(|(root, subkey)| read_string(*root, subkey, "pv").is_some_and(|v| !v.is_empty() && v != "0.0.0.0"))
}

pub fn set_string(subkey: &str, name: &str, value: &str) -> Result<(), String> {
    unsafe {
        let mut key: HKEY = null_mut();
        let status = RegCreateKeyExW(HKEY_CURRENT_USER, w(subkey).as_ptr(), 0, null(), 0, KEY_SET_VALUE, null(), &mut key, null_mut());
        if status != 0 {
            return Err(format!("реестр: код {status}"));
        }
        let data = w(value);
        let status = RegSetValueExW(key, w(name).as_ptr(), 0, REG_SZ, data.as_ptr() as *const u8, (data.len() * 2) as u32);
        RegCloseKey(key);
        if status != 0 {
            return Err(format!("реестр: код {status}"));
        }
    }
    Ok(())
}

pub fn set_dword(subkey: &str, name: &str, value: u32) -> Result<(), String> {
    unsafe {
        let mut key: HKEY = null_mut();
        let status = RegCreateKeyExW(HKEY_CURRENT_USER, w(subkey).as_ptr(), 0, null(), 0, KEY_SET_VALUE, null(), &mut key, null_mut());
        if status != 0 {
            return Err(format!("реестр: код {status}"));
        }
        let status = RegSetValueExW(key, w(name).as_ptr(), 0, REG_DWORD, &value as *const u32 as *const u8, 4);
        RegCloseKey(key);
        if status != 0 {
            return Err(format!("реестр: код {status}"));
        }
    }
    Ok(())
}

pub fn delete_tree(subkey: &str) {
    unsafe { RegDeleteTreeW(HKEY_CURRENT_USER, w(subkey).as_ptr()) };
}

/// Ярлык с идентификатором программы (AppUserModelID) через COM: по нему уведомления Windows
/// показываются как «Deloist». Если COM не вышел, ярлык делает PowerShell без идентификатора, как раньше.
pub fn make_shortcut(link: &Path, target: &Path, workdir: &Path) -> Result<(), String> {
    if let Some(dir) = link.parent() {
        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    }
    match crate::aumid::write_shortcut(link, target, workdir, Some(crate::aumid::AUMID)) {
        Ok(()) => Ok(()),
        Err(com) => {
            // Установщик запущен без консоли: в обычной сборке строка до пользователя не дойдёт.
            eprintln!("ярлык {} через COM не создан: {com}; пробую PowerShell", link.display());
            powershell_shortcut(link, target, workdir)
        }
    }
}

/// Ярлык через Windows Script Host в PowerShell: без идентификатора программы, зато без COM-вызовов.
/// Пути идут через переменные среды, а не в тексте команды, поэтому кавычки в них безвредны.
fn powershell_shortcut(link: &Path, target: &Path, workdir: &Path) -> Result<(), String> {
    use std::os::windows::process::CommandExt;
    let script = "$s=(New-Object -ComObject WScript.Shell).CreateShortcut($env:DL_LINK);$s.TargetPath=$env:DL_TARGET;$s.WorkingDirectory=$env:DL_WORKDIR;$s.Description='Deloist';$s.Save()";
    let status = std::process::Command::new(powershell())
        .args(["-NoProfile", "-NonInteractive", "-ExecutionPolicy", "Bypass", "-Command", script])
        .env("DL_LINK", link)
        .env("DL_TARGET", target)
        .env("DL_WORKDIR", workdir)
        .stdin(std::process::Stdio::null())
        .creation_flags(0x0800_0000)
        .output()
        .map_err(|e| format!("не запустился PowerShell: {e}"))?;
    if status.status.success() && link.exists() {
        Ok(())
    } else {
        Err(format!("не удалось создать ярлык {}: {}", link.display(), String::from_utf8_lossy(&status.stderr).trim()))
    }
}

/// Папка «Рабочий стол» (бывает перенесена в OneDrive): спрашиваем у Windows напрямую,
/// строка приходит в UTF-16, кириллический профиль не портится.
pub fn desktop_dir() -> Option<std::path::PathBuf> {
    use std::os::windows::ffi::OsStringExt;
    use windows_sys::Win32::System::Com::CoTaskMemFree;
    use windows_sys::Win32::UI::Shell::{FOLDERID_Desktop, SHGetKnownFolderPath};
    unsafe {
        let mut raw: *mut u16 = null_mut();
        if SHGetKnownFolderPath(&FOLDERID_Desktop, 0, null_mut(), &mut raw) != 0 || raw.is_null() {
            return None;
        }
        let mut len = 0;
        while *raw.add(len) != 0 {
            len += 1;
        }
        let path = std::ffi::OsString::from_wide(std::slice::from_raw_parts(raw, len));
        CoTaskMemFree(raw as *const _);
        Some(std::path::PathBuf::from(path))
    }
}

/// Системная программа по полному пути из `%SystemRoot%\System32`: не ищем её по PATH и
/// в папке установщика, где её мог подложить кто-то другой.
pub fn system_exe(relative: &str) -> std::path::PathBuf {
    let root = std::env::var_os("SystemRoot").map(std::path::PathBuf::from).filter(|p| p.is_absolute()).unwrap_or_else(|| std::path::PathBuf::from(r"C:\Windows"));
    root.join("System32").join(relative)
}

fn powershell() -> std::path::PathBuf {
    system_exe(r"WindowsPowerShell\v1.0\powershell.exe")
}

/// Подписан ли файл именем «Microsoft Corporation». Действительность подписи проверяет
/// `authenticode_valid`, здесь читаем только, кто подписал (имя в сертификате латиницей).
pub fn signed_by_microsoft(file: &Path) -> bool {
    use std::os::windows::process::CommandExt;
    let out = std::process::Command::new(powershell())
        .args(["-NoProfile", "-NonInteractive", "-ExecutionPolicy", "Bypass", "-Command", "$s=Get-AuthenticodeSignature -LiteralPath $env:DL_FILE; if ($s.Status -eq 'Valid') { $s.SignerCertificate.Subject }"])
        .env("DL_FILE", file)
        .stdin(std::process::Stdio::null())
        .creation_flags(0x0800_0000)
        .output();
    let Ok(out) = out else { return false };
    let subject = String::from_utf8_lossy(&out.stdout).trim().to_string();
    subject.split(", ").any(|part| part == "CN=Microsoft Corporation")
}

/// Проверка подписи Authenticode у файла Microsoft, который мы собираемся запустить.
pub fn authenticode_valid(file: &Path) -> bool {
    let path = w(&file.to_string_lossy());
    unsafe {
        let mut info: WINTRUST_FILE_INFO = std::mem::zeroed();
        info.cbStruct = std::mem::size_of::<WINTRUST_FILE_INFO>() as u32;
        info.pcwszFilePath = path.as_ptr();
        let mut data: WINTRUST_DATA = std::mem::zeroed();
        data.cbStruct = std::mem::size_of::<WINTRUST_DATA>() as u32;
        data.dwUIChoice = WTD_UI_NONE;
        data.fdwRevocationChecks = WTD_REVOKE_NONE;
        data.dwUnionChoice = WTD_CHOICE_FILE;
        data.Anonymous.pFile = &mut info;
        data.dwStateAction = WTD_STATEACTION_VERIFY;
        let mut action = WINTRUST_ACTION_GENERIC_VERIFY_V2;
        let verdict = WinVerifyTrust(INVALID_HANDLE_VALUE, &mut action, &mut data as *mut _ as *mut _);
        data.dwStateAction = WTD_STATEACTION_CLOSE;
        WinVerifyTrust(INVALID_HANDLE_VALUE, &mut action, &mut data as *mut _ as *mut _);
        verdict == 0
    }
}
