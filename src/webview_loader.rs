//! Свой загрузчик WebView2: deloist.exe не импортирует WebView2Loader.dll.
//!
//! Крейт `webview2-com-sys` на не-MSVC целях объявляет функции загрузчика как обычный
//! `extern "system"` блок с `link(name = "WebView2Loader.dll")`. Если эти же символы определены
//! в самом exe, компоновщик не берёт ни одной записи из импорт-библиотеки, и в таблице
//! импорта exe библиотеки нет. Функции ниже повторяют поведение загрузчика Microsoft:
//! находят Evergreen-среду (или фиксированную версию), грузят `EmbeddedBrowserWebView.dll`
//! и вызывают её недокументированный экспорт `CreateWebViewEnvironmentWithOptionsInternal`.
//! Порядок поиска и контракт вызова взяты из OpenWebView2Loader (jchv, лицензия ISC, см. THIRD_PARTY/jchv-OpenWebView2Loader.txt):
//! https://github.com/jchv/OpenWebView2Loader . Политики реестра и пакеты UWP не поддерживаем.

use std::path::{Path, PathBuf};

/// Самая старая версия среды, с которой работает загрузчик (как в загрузчике Microsoft).
const MINIMUM_VERSION: [u32; 4] = [86, 0, 616, 0];

/// Идентификаторы каналов в порядке: stable, beta, dev, canary, internal.
const CHANNEL_UUIDS: [&str; 5] = [
    "{F3017226-FE2A-4295-8BDF-00C3A9A7E4C5}",
    "{2CD8A007-E189-409D-A2C8-9AF4EF3C72AA}",
    "{0D50BFEC-CD6A-4F9A-964C-C7416E3ACB10}",
    "{65C35B14-6C1D-4122-AC46-7148CC9D6497}",
    "{BE59E8FD-089A-411B-A3B0-051D9E417818}",
];
const CHANNEL_NAMES: [&str; 5] = ["", "beta", "dev", "canary", "internal"];

/// Разбор «120.0.2210.91» в четыре числа. Хвост после четвёртого числа (пробел, канал) отбрасывается.
pub fn parse_version(text: &str) -> Option<[u32; 4]> {
    let mut out = [0u32; 4];
    let mut rest = text.trim();
    for (i, slot) in out.iter_mut().enumerate() {
        let end = rest.find(|c: char| !c.is_ascii_digit()).unwrap_or(rest.len());
        if end == 0 {
            return None;
        }
        *slot = rest[..end].parse().ok()?;
        rest = &rest[end..];
        if i < 3 {
            rest = rest.strip_prefix('.')?;
        }
    }
    Some(out)
}

/// Версия не младше минимальной.
pub fn version_supported(version: [u32; 4]) -> bool {
    version >= MINIMUM_VERSION
}

/// Путь к клиентской библиотеке внутри папки среды.
pub fn client_dll_path(runtime_folder: &Path) -> PathBuf {
    runtime_folder.join("EBWebView").join("x64").join("EmbeddedBrowserWebView.dll")
}

/// Версия фиксированной среды: из имени папки вида `Microsoft.WebView2.FixedVersionRuntime.120.0.2210.91.x64`.
pub fn version_from_folder_name(name: &str) -> Option<String> {
    let bytes = name.as_bytes();
    (0..bytes.len())
        .filter(|&i| bytes[i].is_ascii_digit() && (i == 0 || bytes[i - 1] == b'.'))
        .find_map(|i| parse_version(&name[i..]).map(|v| format!("{}.{}.{}.{}", v[0], v[1], v[2], v[3])))
}

/// Для `CompareBrowserVersions`: 1, если первая версия новее второй, -1, если старше, иначе 0.
/// Знак как в загрузчике Microsoft: результат сравнивает version1 с version2.
pub fn compare_versions(a: [u32; 4], b: [u32; 4]) -> i32 {
    match a.cmp(&b) {
        std::cmp::Ordering::Less => -1,
        std::cmp::Ordering::Equal => 0,
        std::cmp::Ordering::Greater => 1,
    }
}

/// Путь `Software\Microsoft\EdgeUpdate\ClientState\{guid}` канала.
pub fn client_state_key(channel: usize) -> String {
    format!("Software\\Microsoft\\EdgeUpdate\\ClientState\\{}", CHANNEL_UUIDS[channel])
}

/// Порядок перебора каналов: stable первым, для предпочтения canary — наоборот.
pub fn channel_order(canary_first: bool) -> [usize; 5] {
    if canary_first {
        [4, 3, 2, 1, 0]
    } else {
        [0, 1, 2, 3, 4]
    }
}

/// Значение реестра `EBWebView` — путь к папке версии; версия — последний компонент пути.
pub fn version_from_runtime_path(path: &str) -> Option<String> {
    let last = path.trim_end_matches('\\').rsplit('\\').next()?;
    parse_version(last).map(|v| format!("{}.{}.{}.{}", v[0], v[1], v[2], v[3]))
}

#[cfg(windows)]
mod imp {
    use super::*;
    use std::ffi::c_void;
    use std::os::windows::ffi::{OsStrExt, OsStringExt};
    use windows_sys::Win32::System::Com::CoTaskMemAlloc;
    use windows_sys::Win32::System::LibraryLoader::{GetProcAddress, LoadLibraryW};
    use windows_sys::Win32::System::Registry::{
        RegCloseKey, RegOpenKeyExW, RegQueryValueExW, HKEY, HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE, KEY_READ,
        KEY_WOW64_32KEY,
    };

    type Hresult = i32;
    const S_OK: Hresult = 0;
    const E_INVALIDARG: Hresult = 0x8007_0057_u32 as i32;
    const E_POINTER: Hresult = 0x8000_4003_u32 as i32;
    const E_FAIL: Hresult = 0x8000_4005_u32 as i32;
    const FILE_NOT_FOUND: Hresult = 0x8007_0002_u32 as i32;

    fn wide(text: &std::ffi::OsStr) -> Vec<u16> {
        text.encode_wide().chain(std::iter::once(0)).collect()
    }

    /// Строка из PCWSTR; null и пустая строка дают None.
    unsafe fn pwstr_to_string(ptr: *const u16) -> Option<String> {
        if ptr.is_null() {
            return None;
        }
        let mut len = 0;
        while unsafe { *ptr.add(len) } != 0 {
            len += 1;
        }
        if len == 0 {
            return None;
        }
        let slice = unsafe { std::slice::from_raw_parts(ptr, len) };
        Some(String::from_utf16_lossy(slice))
    }

    fn env_text(name: &str) -> Option<String> {
        std::env::var_os(name).map(|v| v.to_string_lossy().into_owned()).filter(|v| !v.is_empty())
    }

    /// Строковое значение реестра (REG_SZ) с ключа под корнем `root`, 32-битное представление.
    fn registry_string(root: HKEY, key: &str, value: &str) -> Option<String> {
        let key_w = wide(std::ffi::OsStr::new(key));
        let value_w = wide(std::ffi::OsStr::new(value));
        let mut handle: HKEY = std::ptr::null_mut();
        // SAFETY: обычный вызов реестра с валидными нуль-терминированными строками.
        unsafe {
            if RegOpenKeyExW(root, key_w.as_ptr(), 0, KEY_READ | KEY_WOW64_32KEY, &mut handle) != 0 {
                return None;
            }
            let mut size: u32 = 0;
            let mut ok = RegQueryValueExW(
                handle,
                value_w.as_ptr(),
                std::ptr::null(),
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                &mut size,
            ) == 0;
            let mut buf = vec![0u16; (size as usize / 2) + 2];
            if ok {
                size = (buf.len() * 2) as u32 - 2;
                ok = RegQueryValueExW(
                    handle,
                    value_w.as_ptr(),
                    std::ptr::null(),
                    std::ptr::null_mut(),
                    buf.as_mut_ptr().cast(),
                    &mut size,
                ) == 0;
            }
            RegCloseKey(handle);
            if !ok {
                return None;
            }
            let len = (size as usize / 2).min(buf.len());
            let end = buf[..len].iter().position(|&c| c == 0).unwrap_or(len);
            let text = std::ffi::OsString::from_wide(&buf[..end]).to_string_lossy().into_owned();
            if text.is_empty() { None } else { Some(text) }
        }
    }

    struct Params {
        browser_folder: Option<String>,
        user_data: Option<String>,
        canary_first: bool,
    }

    /// Переменные среды переопределяют аргументы, как в загрузчике Microsoft.
    fn params_with_overrides(browser_folder: *const u16, user_data: *const u16) -> Params {
        // SAFETY: указатели приходят от вызывающего, это PCWSTR или null.
        let (arg_folder, arg_data) = unsafe { (pwstr_to_string(browser_folder), pwstr_to_string(user_data)) };
        Params {
            browser_folder: env_text("WEBVIEW2_BROWSER_EXECUTABLE_FOLDER").or(arg_folder),
            user_data: env_text("WEBVIEW2_USER_DATA_FOLDER").or(arg_data),
            canary_first: env_text("WEBVIEW2_RELEASE_CHANNEL_PREFERENCE").as_deref() == Some("1"),
        }
    }

    struct Found {
        dll: PathBuf,
        version: String,
        channel: &'static str,
        fixed: bool,
    }

    fn find_fixed(folder: &str) -> Result<Found, Hresult> {
        let mut root = PathBuf::from(folder);
        if root.is_relative() {
            let exe = std::env::current_exe().map_err(|_| E_FAIL)?;
            root = exe.parent().ok_or(E_FAIL)?.join(root);
        }
        let dll = client_dll_path(&root);
        if !dll.exists() {
            return Err(FILE_NOT_FOUND);
        }
        let name = root.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        Ok(Found { dll, version: version_from_folder_name(&name).unwrap_or_default(), channel: "", fixed: true })
    }

    fn find_installed(canary_first: bool) -> Result<Found, Hresult> {
        for channel in channel_order(canary_first) {
            let key = client_state_key(channel);
            for root in [HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE] {
                let Some(path) = registry_string(root, &key, "EBWebView") else { continue };
                let Some(version) = version_from_runtime_path(&path) else { continue };
                let parsed = parse_version(&version).unwrap();
                if !version_supported(parsed) {
                    continue;
                }
                let dll = client_dll_path(Path::new(&path));
                if dll.exists() {
                    return Ok(Found { dll, version, channel: CHANNEL_NAMES[channel], fixed: false });
                }
            }
        }
        Err(FILE_NOT_FOUND)
    }

    fn find(params: &Params) -> Result<Found, Hresult> {
        match params.browser_folder.as_deref() {
            Some(folder) => find_fixed(folder),
            None => find_installed(params.canary_first),
        }
    }

    type CreateInternal = unsafe extern "system" fn(bool, i32, *const u16, *mut c_void, *mut c_void) -> Hresult;

    fn create_with_client_dll(found: &Found, user_data: Option<&str>, options: *mut c_void, handler: *mut c_void) -> Hresult {
        let dll_w = wide(found.dll.as_os_str());
        // SAFETY: загружаем библиотеку среды WebView2 и вызываем её известный экспорт.
        unsafe {
            let module = LoadLibraryW(dll_w.as_ptr());
            if module.is_null() {
                return E_FAIL;
            }
            let Some(proc_addr) = GetProcAddress(module, b"CreateWebViewEnvironmentWithOptionsInternal\0".as_ptr()) else {
                return E_FAIL;
            };
            let create: CreateInternal = std::mem::transmute(proc_addr);
            let data_w = user_data.map(|d| wide(std::ffi::OsStr::new(d)));
            let data_ptr = data_w.as_ref().map_or(std::ptr::null(), |d| d.as_ptr());
            // Второй аргумент: 0 — установленная среда, 1 — фиксированная версия.
            create(true, i32::from(found.fixed), data_ptr, options, handler)
        }
    }

    #[no_mangle]
    pub unsafe extern "system" fn CreateCoreWebView2EnvironmentWithOptions(
        browser_executable_folder: *const u16,
        user_data_folder: *const u16,
        environment_options: *mut c_void,
        handler: *mut c_void,
    ) -> Hresult {
        if handler.is_null() {
            return E_POINTER;
        }
        let params = params_with_overrides(browser_executable_folder, user_data_folder);
        match find(&params) {
            Ok(found) => create_with_client_dll(&found, params.user_data.as_deref(), environment_options, handler),
            Err(code) => code,
        }
    }

    #[no_mangle]
    pub unsafe extern "system" fn CreateCoreWebView2Environment(handler: *mut c_void) -> Hresult {
        unsafe { CreateCoreWebView2EnvironmentWithOptions(std::ptr::null(), std::ptr::null(), std::ptr::null_mut(), handler) }
    }

    #[no_mangle]
    pub unsafe extern "system" fn GetAvailableCoreWebView2BrowserVersionString(
        browser_executable_folder: *const u16,
        version_info: *mut *mut u16,
    ) -> Hresult {
        if version_info.is_null() {
            return E_POINTER;
        }
        unsafe { *version_info = std::ptr::null_mut() };
        let params = params_with_overrides(browser_executable_folder, std::ptr::null());
        let found = match find(&params) {
            Ok(found) => found,
            Err(code) => return code,
        };
        if found.version.is_empty() {
            return E_FAIL;
        }
        let mut text = found.version.clone();
        if !found.channel.is_empty() {
            text.push(' ');
            text.push_str(found.channel);
        }
        let units: Vec<u16> = text.encode_utf16().chain(std::iter::once(0)).collect();
        // SAFETY: память выдаёт CoTaskMemAlloc, как требует контракт (вызывающий освобождает CoTaskMemFree).
        unsafe {
            let mem = CoTaskMemAlloc(units.len() * 2) as *mut u16;
            if mem.is_null() {
                return E_FAIL;
            }
            std::ptr::copy_nonoverlapping(units.as_ptr(), mem, units.len());
            *version_info = mem;
        }
        S_OK
    }

    #[no_mangle]
    pub unsafe extern "system" fn CompareBrowserVersions(version1: *const u16, version2: *const u16, result: *mut i32) -> Hresult {
        if result.is_null() {
            return E_POINTER;
        }
        // SAFETY: PCWSTR от вызывающего.
        let (a, b) = unsafe { (pwstr_to_string(version1), pwstr_to_string(version2)) };
        let (Some(a), Some(b)) = (a, b) else { return E_INVALIDARG };
        let (Some(a), Some(b)) = (parse_version(&a), parse_version(&b)) else { return E_INVALIDARG };
        unsafe { *result = compare_versions(a, b) };
        S_OK
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_versions() {
        assert_eq!(parse_version("120.0.2210.91"), Some([120, 0, 2210, 91]));
        assert_eq!(parse_version("120.0.2210.91 beta"), Some([120, 0, 2210, 91]));
        assert_eq!(parse_version("120.0.2210"), None);
        assert_eq!(parse_version("abc"), None);
        assert_eq!(parse_version("1..2.3"), None);
    }

    #[test]
    fn minimum_version_gate() {
        assert!(version_supported([86, 0, 616, 0]));
        assert!(version_supported([142, 0, 1, 1]));
        assert!(!version_supported([86, 0, 615, 99]));
        assert!(!version_supported([85, 9, 9999, 0]));
    }

    #[test]
    fn compares_versions() {
        assert_eq!(compare_versions([1, 2, 3, 4], [1, 2, 3, 4]), 0);
        assert_eq!(compare_versions([1, 2, 3, 5], [1, 2, 3, 4]), 1);
        assert_eq!(compare_versions([1, 2, 2, 9], [1, 2, 3, 4]), -1);
        assert_eq!(compare_versions([10, 0, 0, 0], [9, 99, 99, 99]), 1);
    }

    #[test]
    fn builds_paths_and_keys() {
        let dll = client_dll_path(Path::new("C:\\Program Files (x86)\\Microsoft\\EdgeWebView\\Application\\120.0.1.2"));
        assert!(dll.ends_with("EBWebView\\x64\\EmbeddedBrowserWebView.dll") || dll.ends_with("EBWebView/x64/EmbeddedBrowserWebView.dll"));
        assert_eq!(client_state_key(0), "Software\\Microsoft\\EdgeUpdate\\ClientState\\{F3017226-FE2A-4295-8BDF-00C3A9A7E4C5}");
        assert_eq!(channel_order(false), [0, 1, 2, 3, 4]);
        assert_eq!(channel_order(true), [4, 3, 2, 1, 0]);
    }

    #[test]
    fn extracts_version_from_paths() {
        assert_eq!(
            version_from_runtime_path("C:\\Program Files (x86)\\Microsoft\\EdgeWebView\\Application\\130.0.2849.80"),
            Some("130.0.2849.80".to_string())
        );
        assert_eq!(version_from_runtime_path("C:\\x\\Application\\130.0.2849.80\\"), Some("130.0.2849.80".to_string()));
        assert_eq!(version_from_runtime_path("C:\\x\\Application"), None);
        assert_eq!(
            version_from_folder_name("Microsoft.WebView2.FixedVersionRuntime.120.0.2210.91.x64"),
            Some("120.0.2210.91".to_string())
        );
        assert_eq!(version_from_folder_name("runtime"), None);
    }
}
