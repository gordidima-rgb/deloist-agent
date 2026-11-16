//! Паспорт работающей копии: порт и одноразовый ключ доступа.
//!
//! Приложение пишет `data/runtime.json` при запуске и удаляет при выходе. По этому файлу
//! программа командной строки и MCP-сервер находят живую копию и получают право говорить с ней.
//! Право это ровно настолько сильное, насколько защищена папка данных: кто может прочитать файл,
//! тот и так может прочитать `secrets.json` и все воркспейсы.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

pub const FILE: &str = "runtime.json";

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Runtime {
    pub port: u16,
    pub token: String,
    pub pid: u32,
    pub started_at: i64,
    pub version: String,
}

pub fn path(data_dir: &Path) -> PathBuf {
    data_dir.join(FILE)
}

/// Записать паспорт: файл только для владельца, насколько это возможно на системе.
pub fn write(data_dir: &Path, runtime: &Runtime) -> std::io::Result<()> {
    let text = serde_json::to_string_pretty(runtime).map_err(std::io::Error::other)?;
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    use std::io::Write;
    let mut file = options.open(path(data_dir))?;
    file.write_all(text.as_bytes())?;
    file.flush()
}

pub fn remove(data_dir: &Path) {
    let _ = std::fs::remove_file(path(data_dir));
}

/// Прочитать паспорт живой копии. Устаревший файл (процесс уже завершился) считается отсутствующим.
pub fn read(data_dir: &Path) -> Option<Runtime> {
    let text = std::fs::read_to_string(path(data_dir)).ok()?;
    let runtime: Runtime = serde_json::from_str(&text).ok()?;
    if runtime.port == 0 || runtime.token.is_empty() {
        return None;
    }
    if !alive(runtime.pid) {
        return None;
    }
    Some(runtime)
}

fn alive(pid: u32) -> bool {
    if pid == 0 {
        return false;
    }
    #[cfg(windows)]
    {
        // OpenProcess вернёт null, если процесса нет или он чужой и недоступен для опроса.
        use windows_sys::Win32::Foundation::{CloseHandle, STILL_ACTIVE};
        use windows_sys::Win32::System::Threading::{GetExitCodeProcess, OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION};
        unsafe {
            let handle = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
            if handle.is_null() {
                return false;
            }
            let mut code = 0u32;
            let ok = GetExitCodeProcess(handle, &mut code) != 0;
            CloseHandle(handle);
            ok && code == STILL_ACTIVE as u32
        }
    }
    #[cfg(not(windows))]
    {
        Path::new(&format!("/proc/{pid}")).exists()
    }
}
