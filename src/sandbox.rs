//! Изоляция выполнения команд: AppContainer → restricted token → job object → как раньше.
//!
//! Уровень подбирается один раз: первое обращение к [`status`] запускает канарейку (а это
//! отдельный процесс), поэтому в горячем пути `run_command` проверки уже нет.
//!
//! `CREW_SANDBOX=off|auto|strict`:
//! - `off` — как раньше, без изоляции;
//! - `auto` (по умолчанию) — restricted token, job object и текстовый фильтр [`looks_protected`];
//! - `strict` — только AppContainer: если клетка не поднялась, команда не выполняется вовсе.
//!
//! Причина, по которой уровень не поднялся, живёт в [`Status::detail`], а не в выводе команды:
//! обычная команда не должна получать лишний шум из-за особенностей машины.

use std::collections::HashSet;
use std::ffi::OsString;
use std::os::windows::ffi::OsStrExt;
use std::os::windows::io::AsRawHandle;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

use windows_sys::core::{HRESULT, PCWSTR};
use windows_sys::Win32::Foundation::{
    CloseHandle, GetLastError, HANDLE, INVALID_HANDLE_VALUE, LocalFree, SetHandleInformation, ERROR_FILE_NOT_FOUND,
    ERROR_PATH_NOT_FOUND, HANDLE_FLAG_INHERIT,
};
use windows_sys::Win32::Security::{
    CreateRestrictedToken, GetTokenInformation, PSID, SECURITY_ATTRIBUTES, SECURITY_CAPABILITIES,
    SID_AND_ATTRIBUTES, TokenIsRestricted, TokenPrivileges, DISABLE_MAX_PRIVILEGE, LUA_TOKEN,
    TOKEN_ADJUST_DEFAULT, TOKEN_ADJUST_SESSIONID, TOKEN_ALL_ACCESS, TOKEN_ASSIGN_PRIMARY, TOKEN_DUPLICATE, TOKEN_QUERY,
};
use windows_sys::Win32::System::JobObjects::{
    AssignProcessToJobObject, CreateJobObjectW, SetInformationJobObject, TerminateJobObject,
    JobObjectBasicUIRestrictions, JobObjectExtendedLimitInformation, JOBOBJECT_BASIC_UI_RESTRICTIONS,
    JOBOBJECT_EXTENDED_LIMIT_INFORMATION, JOB_OBJECT_LIMIT_ACTIVE_PROCESS, JOB_OBJECT_UILIMIT_DESKTOP, JOB_OBJECT_UILIMIT_DISPLAYSETTINGS,
    JOB_OBJECT_UILIMIT_EXITWINDOWS, JOB_OBJECT_UILIMIT_GLOBALATOMS, JOB_OBJECT_UILIMIT_READCLIPBOARD,
    JOB_OBJECT_UILIMIT_SYSTEMPARAMETERS, JOB_OBJECT_UILIMIT_WRITECLIPBOARD,
};
use windows_sys::Win32::System::LibraryLoader::{GetProcAddress, LoadLibraryW};
use windows_sys::Win32::System::Threading::{
    CreateProcessAsUserW, CreateProcessW, DeleteProcThreadAttributeList, GetCurrentProcess, GetExitCodeProcess,
    InitializeProcThreadAttributeList, OpenProcessToken, ResumeThread, UpdateProcThreadAttribute, WaitForSingleObject,
    PROCESS_INFORMATION, STARTUPINFOEXW, STARTF_USESTDHANDLES, CREATE_NO_WINDOW, CREATE_SUSPENDED,
    CREATE_UNICODE_ENVIRONMENT, EXTENDED_STARTUPINFO_PRESENT, LPPROC_THREAD_ATTRIBUTE_LIST,
    PROC_THREAD_ATTRIBUTE_SECURITY_CAPABILITIES,
};
use windows_sys::Win32::Foundation::HMODULE;

/// Сколько процессов разрешено в одном job: сама команда и её дети. Сборки (`cargo`, `npm`, `make -j`)
/// запускают десятки параллельно, поэтому предел только от разрастания без конца.
const MAX_PROCESSES: u32 = 512;
/// Профиль AppContainer один на пользователя, поэтому копии Deloist его делят.
const APP_CONTAINER_NAME: &str = "deloist.sandbox";
const APP_CONTAINER_TITLE: &str = "Deloist sandbox";
const APP_CONTAINER_DESCRIPTION: &str = "Песочница Deloist: команды бота";
/// internetClient (S-1-15-3-1): без него npm, pip и git остаются без сети. S-1-15-2-1 — это группа
/// ALL APPLICATION PACKAGES, а не возможность, и с ней CreateProcess отвечает «параметр задан неверно».
const CAPABILITY_INTERNET_CLIENT: (u64, &[u32]) = (15, &[3, 1]);
/// `WAIT_TIMEOUT` из winbase.h; в windows-sys константы нет.
const WAIT_TIMEOUT_CODE: u32 = 258;
/// `SE_GROUP_ENABLED` лежит в Win32_System_SystemServices, который нам не нужен.
const SE_GROUP_ENABLED_FLAG: u32 = 4;
/// HRESULT_FROM_WIN32(ERROR_ALREADY_EXISTS): профиль уже создан прошлым запуском.
const HRESULT_ALREADY_EXISTS: HRESULT = 0x8007_00B7u32 as i32;
/// Переменные, которые в AppContainer смотрят в `<воркспейс>\.home`: иначе npm/pip не пишут кэш.
const HOME_VARS: [&str; 6] = ["HOME", "USERPROFILE", "APPDATA", "LOCALAPPDATA", "TEMP", "TMP"];
/// Сколько ждём канарейку самопроверки. С запасом: на занятой машине или под антивирусом первый
/// cmd.exe стартует небыстро, а провал канарейки понизил бы защиту до конца сеанса.
const PROBE_TIMEOUT: Duration = Duration::from_secs(5);
/// Как часто ожидание команды смотрит, не закрывается ли программа.
const WAIT_SLICE: Duration = Duration::from_millis(250);

/// Программа закрывается: идущие команды надо остановить, иначе выход ждал бы их до таймаута.
static SHUTTING_DOWN: AtomicBool = AtomicBool::new(false);

/// Остановить команды и не начинать новых. Зовётся при выходе и перед обновлением.
#[allow(dead_code)]
pub fn shutdown() {
    SHUTTING_DOWN.store(true, Ordering::SeqCst);
}

// ---------------------------------------------------------------------------
// Уровни, режим и статус
// ---------------------------------------------------------------------------

/// Лестница изоляции сверху вниз: первая работающая ступень и становится уровнем.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Level {
    AppContainer,
    RestrictedToken,
    JobOnly,
    Off,
}

impl Level {
    pub fn name(self) -> &'static str {
        match self {
            Level::AppContainer => "AppContainer",
            Level::RestrictedToken => "RestrictedToken",
            Level::JobOnly => "JobOnly",
            Level::Off => "Off",
        }
    }

    /// Следующая ступень вниз: если уровень не поднялся, пробуем более слабый.
    fn weaker(self) -> Option<Level> {
        match self {
            Level::AppContainer => Some(Level::RestrictedToken),
            Level::RestrictedToken => Some(Level::JobOnly),
            Level::JobOnly => Some(Level::Off),
            Level::Off => None,
        }
    }
}

/// Что в итоге работает и почему именно это.
#[derive(Clone, Debug)]
pub struct Status {
    pub level: Level,
    /// Выбранный режим, фактический уровень и причина отката — одной строкой для человека.
    pub detail: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    Off,
    Auto,
    Strict,
}

impl Mode {
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

/// Текущий режим из окружения.
pub fn mode() -> Mode {
    parse_mode(&std::env::var("CREW_SANDBOX").unwrap_or_default())
}

static STATUS: OnceLock<Mutex<Option<Status>>> = OnceLock::new();

fn status_cache() -> &'static Mutex<Option<Status>> {
    STATUS.get_or_init(|| Mutex::new(None))
}

/// Ленивая самопроверка: выполняется один раз и запоминается.
pub fn status() -> Status {
    let cache = status_cache();
    let mut slot = cache.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    if slot.is_none() {
        // probe никогда не зовёт status(), иначе получился бы самоблокирующийся вызов.
        *slot = Some(probe(mode()));
    }
    slot.as_ref().expect("слот только что заполнили").clone()
}

/// Понижает запомненный уровень, когда изоляция отказала в бою. Причина остаётся в detail.
fn demote(from: Level, reason: &str) {
    let cache = status_cache();
    let mut slot = cache.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    let Some(status) = slot.as_mut() else {
        return;
    };
    if status.level != from {
        return;
    }
    if let Some(next) = from.weaker() {
        status.detail = format!("{}; уровень понижен до {}: {reason}", status.detail, next.name());
        status.level = next;
    }
}

fn probe(selected: Mode) -> Status {
    if selected == Mode::Off {
        return Status {
            level: Level::Off,
            detail: "CREW_SANDBOX=off: команды выполняются как раньше, без изоляции".to_string(),
        };
    }
    let ladder: &[Level] = if selected == Mode::Strict {
        // strict обещает клетку: слабее он не спускается, иначе обещание было бы ложным.
        &[Level::AppContainer]
    } else {
        &[Level::RestrictedToken, Level::JobOnly, Level::Off]
    };
    let mut reasons: Vec<String> = Vec::new();
    for level in ladder {
        match probe_level(*level) {
            Ok(detail) => {
                let note = if reasons.is_empty() {
                    String::new()
                } else {
                    format!("; слабее не понадобилось, выше не поднялось: {}", reasons.join("; "))
                };
                return Status {
                    level: *level,
                    detail: format!("выбран {}, фактически {}: {detail}{note}", selected.name(), level.name()),
                };
            }
            Err(reason) => reasons.push(format!("{} — {reason}", level.name())),
        }
    }
    let refusal = if selected == Mode::Strict { "; команды не выполняются" } else { "" };
    Status {
        level: Level::Off,
        detail: format!("выбран {}, но изоляция не поднялась: {}{refusal}", selected.name(), reasons.join("; ")),
    }
}

/// Проверка одной ступени. `Ok` — ступень работает, `Err` — причина отказа.
fn probe_level(level: Level) -> Result<String, String> {
    match level {
        Level::Off => Ok("обычный запуск без изоляции".to_string()),
        Level::JobOnly => {
            canary(Level::JobOnly)?;
            Ok("job object с kill-on-close, канарейка читается".to_string())
        }
        Level::RestrictedToken => {
            // Токен и его привилегии проверяем отдельно от запуска: так видно, что именно сломалось.
            let original = open_own_token()?;
            match create_restricted_token() {
                Ok(restricted) => {
                    let before = privilege_count(&original)?;
                    let after = privilege_count(&restricted)?;
                    if after >= before {
                        return Err(format!("привилегии не снялись ({before} → {after})"));
                    }
                    canary(Level::RestrictedToken)?;
                    Ok(format!("привилегий {before} → {after}, канарейка читается"))
                }
                Err(error) => {
                    // Свой токен уже ограничен (песочница, служба): копию Windows не даёт,
                    // но дети наследуют ограничение как есть — уровень честно работает.
                    if is_token_restricted(&original).unwrap_or(false) {
                        canary(Level::RestrictedToken)?;
                        Ok(format!("свой токен уже ограничен средой ({error}), ограничение наследуется"))
                    } else {
                        Err(error)
                    }
                }
            }
        }
        Level::AppContainer => container_canary(),
    }
}

// ---------------------------------------------------------------------------
// Публичный запуск
// ---------------------------------------------------------------------------

/// Что получилось: код выхода и признак того, что процесс убит по таймауту.
#[derive(Clone, Copy, Debug)]
pub struct Outcome {
    pub code: i32,
    pub timed_out: bool,
    pub cancelled: bool,
}

/// Запуск команды на выбранном уровне. Возвращает код выхода.
///
/// `cmdline` — полная командная строка (см. `process::shell_command_line`), `env` — явный блок
/// окружения, `out`/`err` — файлы, куда процесс пишет свой вывод.
#[allow(dead_code)] // В приложении нужен признак таймаута, поэтому там вызывается spawn_ex.
pub fn spawn(
    cmdline: &str,
    cwd: &Path,
    env: &[(OsString, OsString)],
    timeout: Duration,
    out: &Path,
    err: &Path,
) -> Result<i32, String> {
    spawn_ex(cmdline, cwd, env, timeout, out, err).map(|outcome| outcome.code)
}

/// То же, что [`spawn`], но с признаком таймаута: вызывающему нужно отличать убитый процесс.
///
/// Если ступень отказала до создания процесса, спускаемся ниже и пробуем ещё раз; уровень
/// понижается и это видно в [`status`]. Ошибки самой команды (файл не найден) наверх отдаются
/// как есть: из-за опечатки в команде защиту ослаблять нельзя.
pub fn spawn_ex(
    cmdline: &str,
    cwd: &Path,
    env: &[(OsString, OsString)],
    timeout: Duration,
    out: &Path,
    err: &Path,
) -> Result<Outcome, String> {
    spawn_ex_cancellable(cmdline, cwd, env, timeout, out, err, None)
}

/// То же с остановкой конкретного хода; проверяется в блокирующем ожидании процесса.
pub fn spawn_ex_cancellable(
    cmdline: &str, cwd: &Path, env: &[(OsString, OsString)], timeout: Duration,
    out: &Path, err: &Path, cancel: Option<&tokio_util::sync::CancellationToken>,
) -> Result<Outcome, String> {
    let mut level = status().level;
    let strict = mode() == Mode::Strict;
    if strict && level != Level::AppContainer {
        return Err(format!("песочница strict недоступна: {}", status().detail));
    }
    loop {
        match spawn_at(level, cmdline, cwd, env, timeout, out, err, cancel) {
            Ok(outcome) => return Ok(outcome),
            Err(SpawnError::Command(message)) => return Err(message),
            // В strict отказ клетки для этой команды — отказ команды. Уровень не понижаем:
            // одна неудачная папка не должна снимать защиту со всех следующих команд.
            Err(SpawnError::Sandbox(message)) if strict => return Err(message),
            Err(SpawnError::Sandbox(message)) => match level.weaker() {
                Some(next) => {
                    demote(level, &message);
                    level = next;
                }
                None => return Err(message),
            },
        }
    }
}

// ---------------------------------------------------------------------------
// Текстовый слой
// ---------------------------------------------------------------------------

/// Что нельзя делать команде, когда файловой изоляции нет.
///
/// Проверка смотрит на команды, а не на подстроки: `git commit -m "fix reg add"` проходит,
/// а `cmd /c "reg add …"` — нет. Лучше пропустить сомнительное, чем сломать нормальную команду.
pub fn looks_protected(command: &str) -> Option<&'static str> {
    let lower = command.to_lowercase();

    // Явные ссылки на профиль пользователя: кэши, ключи, личные папки.
    for (needle, reason) in [
        ("%appdata%", "обращение к %APPDATA% вне воркспейса"),
        ("%localappdata%", "обращение к %LOCALAPPDATA% вне воркспейса"),
        ("%userprofile%", "обращение к %USERPROFILE% вне воркспейса"),
        ("%homepath%", "обращение к домашней папке (%HOMEPATH%)"),
        ("$env:userprofile", "обращение к профилю ($env:USERPROFILE)"),
        ("$env:appdata", "обращение к $env:APPDATA вне воркспейса"),
        ("$env:localappdata", "обращение к $env:LOCALAPPDATA вне воркспейса"),
        ("$home", "обращение к домашней папке ($HOME)"),
    ] {
        if lower.contains(needle) {
            return Some(reason);
        }
    }
    if mentions_path_segment(&lower, ".ssh") {
        return Some("обращение к ключам SSH (.ssh) — к серверам подключайся именем из ~/.ssh/config, например `ssh имя`");
    }
    if mentions_path_segment(&lower, "secrets.json") {
        return Some("обращение к файлу секретов (secrets.json)");
    }
    if mentions_path_segment(&lower, "connectors.bin") {
        return Some("обращение к файлу секретов подключений (connectors.bin)");
    }
    if mentions_path_segment(&lower, "secrets.key") {
        return Some("обращение к ключу шифрования секретов (secrets.key)");
    }
    if lower.contains("\\\\?\\") {
        return Some("путь \\\\?\\ в обход обычных проверок");
    }

    for segment in segments(command) {
        if let Some(reason) = guarded_command(&segment) {
            return Some(reason);
        }
    }
    None
}

/// Настоящий путь: `.ssh` как отдельное имя, а не часть «.ssh2» или «my.sshfile».
fn mentions_path_segment(text: &str, needle: &str) -> bool {
    let bytes = text.as_bytes();
    let mut from = 0;
    while let Some(found) = text[from..].find(needle) {
        let start = from + found;
        let end = start + needle.len();
        let before_ok = start == 0
            || matches!(bytes[start - 1], b'\\' | b'/' | b'"' | b'\'' | b' ' | b'\t' | b'(' | b'=' | b';' | b'&' | b'|' | b',' | b':');
        let after_ok = end == bytes.len() || !matches!(bytes[end], b'a'..=b'z' | b'A'..=b'Z' | b'0'..=b'9' | b'_' | b'-');
        if before_ok && after_ok {
            return true;
        }
        from = end;
    }
    false
}

/// Команды делим по `&`, `|`, `;` и переводу строки, но не внутри кавычек: там это текст.
fn segments(command: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut current = String::new();
    let mut in_quotes = false;
    let mut chars = command.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '"' => {
                in_quotes = !in_quotes;
                current.push(c);
            }
            '&' | '|' | ';' | '\n' | '\r' if !in_quotes => {
                if (c == '&' || c == '|') && chars.peek() == Some(&c) {
                    chars.next();
                }
                out.push(std::mem::take(&mut current));
            }
            _ => current.push(c),
        }
    }
    out.push(current);
    out
}

fn words(segment: &str) -> Vec<String> {
    segment
        .split_whitespace()
        .map(|word| word.replace('"', ""))
        .filter(|word| !word.is_empty())
        .collect()
}

/// Разбирает сегмент и говорит, что в нём нельзя: `cmd /c reg add …` — тоже нельзя.
fn guarded_command(segment: &str) -> Option<&'static str> {
    let words = words(segment);
    let mut i = 0;
    while i < words.len() {
        let word = words[i].to_ascii_lowercase();
        match word.as_str() {
            "cmd" | "cmd.exe" => {
                i += 1;
                // Пропускаем ключи cmd до /c или /k: дальше начинается сама команда.
                while i < words.len() && words[i].starts_with('/') {
                    let flag = words[i].to_ascii_lowercase();
                    i += 1;
                    if flag == "/c" || flag == "/k" || flag == "/r" {
                        break;
                    }
                }
            }
            "call" => i += 1,
            _ => break,
        }
    }
    let rest = &words[i..];
    let name = rest.first()?.to_ascii_lowercase();
    let name = name.strip_suffix(".exe").unwrap_or(&name);
    let second = rest.get(1).map(|word| word.to_ascii_lowercase()).unwrap_or_default();
    match name {
        "reg" if matches!(second.as_str(), "add" | "delete" | "import" | "restore") => Some("правка реестра (reg add)"),
        // Смотреть список задач можно, ставить и менять — нет.
        "schtasks" if rest.iter().skip(1).any(|word| {
            matches!(word.to_ascii_lowercase().as_str(), "/create" | "/change" | "/delete" | "/run")
        }) => Some("планировщик задач (schtasks)"),
        "net" if second == "user" => Some("управление учётными записями (net user)"),
        _ => None,
    }
}

// ---------------------------------------------------------------------------
// Окружение
// ---------------------------------------------------------------------------

/// Блок окружения для CreateProcess: строки `ИМЯ=значение`, сортировка и двойной ноль в конце.
fn build_env_block(env: &[(OsString, OsString)]) -> Vec<u16> {
    let mut pairs: Vec<(&str, &std::ffi::OsStr)> =
        env.iter().map(|(name, value)| (name.to_str().unwrap_or_default(), value.as_os_str())).collect();
    // Windows ждёт имена в алфавитном порядке; регистр при сравнении не важен.
    pairs.sort_by(|a, b| a.0.to_ascii_lowercase().cmp(&b.0.to_ascii_lowercase()));
    let mut block: Vec<u16> = Vec::new();
    for (name, value) in pairs {
        let name: Vec<u16> = name.encode_utf16().filter(|c| *c != 0).collect();
        // Имя с '=' (или пустое) разваливает блок: такую переменную просто пропускаем.
        if name.is_empty() || name.contains(&(b'=' as u16)) {
            continue;
        }
        let value: Vec<u16> = value.encode_wide().filter(|c| *c != 0).collect();
        block.extend_from_slice(&name);
        block.push(b'=' as u16);
        block.extend_from_slice(&value);
        block.push(0);
    }
    block.push(0);
    block
}

/// В AppContainer домашние переменные смотрят в `<воркспейс>\.home`: иначе npm/pip не пишут кэш.
fn remap_home_env(env: &[(OsString, OsString)], home: &Path) -> Vec<(OsString, OsString)> {
    let home_value = home.as_os_str().to_os_string();
    let mut out: Vec<(OsString, OsString)> = env
        .iter()
        .filter(|(name, _)| {
            let name = name.to_string_lossy();
            !HOME_VARS.iter().any(|var| name.eq_ignore_ascii_case(var))
        })
        .cloned()
        .collect();
    for name in HOME_VARS {
        out.push((OsString::from(name), home_value.clone()));
    }
    out
}

// ---------------------------------------------------------------------------
// Запуск: Win32
// ---------------------------------------------------------------------------

/// Ошибка запуска: команду искать не надо (`Command`) или подвела сама изоляция (`Sandbox`).
enum SpawnError {
    Command(String),
    Sandbox(String),
}

impl SpawnError {
    fn command(message: impl Into<String>) -> Self {
        SpawnError::Command(message.into())
    }

    fn sandbox(message: impl Into<String>) -> Self {
        SpawnError::Sandbox(message.into())
    }

    fn text(&self) -> &str {
        match self {
            SpawnError::Command(message) | SpawnError::Sandbox(message) => message,
        }
    }
}

/// Самопроверка возвращает человеческий текст, поэтому ошибку запуска переводим в строку.
impl From<SpawnError> for String {
    fn from(error: SpawnError) -> String {
        error.text().to_string()
    }
}

struct Handle(HANDLE);

impl Handle {
    fn new(raw: HANDLE) -> Option<Handle> {
        if raw.is_null() || raw == INVALID_HANDLE_VALUE {
            None
        } else {
            Some(Handle(raw))
        }
    }

    fn raw(&self) -> HANDLE {
        self.0
    }
}

impl Drop for Handle {
    fn drop(&mut self) {
        unsafe { CloseHandle(self.0) };
    }
}

fn spawn_at(
    level: Level,
    cmdline: &str,
    cwd: &Path,
    env: &[(OsString, OsString)],
    timeout: Duration,
    out: &Path,
    err: &Path,
    cancel: Option<&tokio_util::sync::CancellationToken>,
) -> Result<Outcome, SpawnError> {
    if cancel.is_some_and(|token| token.is_cancelled()) {
        return Ok(Outcome { code: -1, timed_out: false, cancelled: true });
    }
    // Вывод пишем в файлы, как и раньше: команда не упирается в буфер пайпа. Handle должен
    // наследоваться — у std он по умолчанию не наследуемый.
    let out_file = std::fs::File::create(out)
        .map_err(|e| SpawnError::command(format!("не смог открыть {}: {e}", out.display())))?;
    let err_file = std::fs::File::create(err)
        .map_err(|e| SpawnError::command(format!("не смог открыть {}: {e}", err.display())))?;
    // stdin команде не нужен: NUL ведёт себя как пустой ввод и не ломает cmd.
    let nul = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open("NUL")
        .map_err(|e| SpawnError::command(format!("не смог открыть NUL: {e}")))?;
    let out_handle = out_file.as_raw_handle() as HANDLE;
    let err_handle = err_file.as_raw_handle() as HANDLE;
    let nul_handle = nul.as_raw_handle() as HANDLE;
    for handle in [out_handle, err_handle, nul_handle] {
        inherit_handle(handle).map_err(SpawnError::command)?;
    }

    let mut child_env = env.to_vec();
    if level == Level::AppContainer {
        let home = cwd.join(".home");
        std::fs::create_dir_all(&home)
            .map_err(|e| SpawnError::sandbox(format!("не смог создать {}: {e}", home.display())))?;
        child_env = remap_home_env(&child_env, &home);
    }
    let env_block = build_env_block(&child_env);
    let mut cmd_wide = wide(cmdline);
    let cwd_wide = wide_path(cwd);

    // Job создаём до процесса, чтобы присоединить его раньше ResumeThread.
    let job = match level {
        Level::Off => None,
        _ => Some(create_job()?),
    };

    let container = match level {
        Level::AppContainer => Some(app_container()?),
        _ => None,
    };
    // AppContainer своего токена не требует: CreateProcess строит его сам из SECURITY_CAPABILITIES.
    let token = match level {
        Level::RestrictedToken => match create_restricted_token() {
            Ok(token) => Some(token),
            // Копию сделать не вышло, но свой токен уже ограничен: ребёнок унаследует его
            // через обычный CreateProcessW, и запрет останется тем же.
            Err(error) => {
                if own_token_restricted() {
                    None
                } else {
                    return Err(SpawnError::sandbox(error));
                }
            }
        },
        _ => None,
    };

    // SECURITY_CAPABILITIES ссылается на SID контейнера и на его capability: оба живут
    // в Arc<Container> и в этих локальных переменных до самого CreateProcess.
    let mut capability = SID_AND_ATTRIBUTES::default();
    let mut capabilities = SECURITY_CAPABILITIES::default();
    let attributes = match (&container, level) {
        (Some(container), Level::AppContainer) => {
            grant_workspace(container, cwd)?;
            fill_capabilities(container, &mut capability, &mut capabilities);
            let mut list = AttributeList::new(1).map_err(SpawnError::sandbox)?;
            list.set_security_capabilities(&capabilities).map_err(SpawnError::sandbox)?;
            Some(list)
        }
        _ => None,
    };

    let mut si: STARTUPINFOEXW = unsafe { std::mem::zeroed() };
    // С расширенным STARTUPINFO Windows ждёт размер STARTUPINFOEXW, без него — обычный.
    si.StartupInfo.cb = if attributes.is_some() {
        std::mem::size_of::<STARTUPINFOEXW>() as u32
    } else {
        std::mem::size_of::<windows_sys::Win32::System::Threading::STARTUPINFOW>() as u32
    };
    si.StartupInfo.dwFlags = STARTF_USESTDHANDLES;
    si.StartupInfo.hStdInput = nul_handle;
    si.StartupInfo.hStdOutput = out_handle;
    si.StartupInfo.hStdError = err_handle;
    if let Some(list) = &attributes {
        si.lpAttributeList = list.raw();
    }

    let mut flags = CREATE_NO_WINDOW | CREATE_SUSPENDED | CREATE_UNICODE_ENVIRONMENT;
    if attributes.is_some() {
        flags |= EXTENDED_STARTUPINFO_PRESENT;
    }
    let mut pi: PROCESS_INFORMATION = unsafe { std::mem::zeroed() };
    let environment = env_block.as_ptr() as *const core::ffi::c_void;
    let created = match &token {
        Some(token) => unsafe {
            CreateProcessAsUserW(
                token.raw(),
                std::ptr::null(),
                cmd_wide.as_mut_ptr(),
                std::ptr::null::<SECURITY_ATTRIBUTES>(),
                std::ptr::null::<SECURITY_ATTRIBUTES>(),
                1,
                flags,
                environment,
                cwd_wide.as_ptr(),
                &si.StartupInfo,
                &mut pi,
            )
        },
        None => unsafe {
            CreateProcessW(
                std::ptr::null(),
                cmd_wide.as_mut_ptr(),
                std::ptr::null::<SECURITY_ATTRIBUTES>(),
                std::ptr::null::<SECURITY_ATTRIBUTES>(),
                1,
                flags,
                environment,
                cwd_wide.as_ptr(),
                &si.StartupInfo,
                &mut pi,
            )
        },
    };
    if created == 0 {
        let code = unsafe { GetLastError() };
        return Err(classify_process_error(code, level));
    }
    let process = Handle::new(pi.hProcess).ok_or_else(|| SpawnError::sandbox("CreateProcess без handle процесса"))?;
    let thread = Handle::new(pi.hThread).ok_or_else(|| SpawnError::sandbox("CreateProcess без handle потока"))?;

    if let Some(job) = &job {
        if unsafe { AssignProcessToJobObject(job.raw(), process.raw()) } == 0 {
            let code = unsafe { GetLastError() };
            unsafe { windows_sys::Win32::System::Threading::TerminateProcess(process.raw(), 1) };
            return Err(SpawnError::sandbox(format!("AssignProcessToJobObject: {}", win_error(code))));
        }
    }
    if unsafe { ResumeThread(thread.raw()) } == u32::MAX {
        let code = unsafe { GetLastError() };
        terminate(&job, &process);
        return Err(SpawnError::sandbox(format!("ResumeThread: {}", win_error(code))));
    }

    // Дальше процесс уже работает: ошибки не откатывают уровень, иначе команду запустили бы дважды.
    let deadline = Instant::now() + timeout;
    let mut outcome = Outcome { code: -1, timed_out: false, cancelled: false };
    loop {
        let left = deadline.saturating_duration_since(Instant::now());
        let slice = left.min(WAIT_SLICE).as_millis() as u32;
        match unsafe { WaitForSingleObject(process.raw(), slice) } {
            0 => {
                let mut code: u32 = 0;
                if unsafe { GetExitCodeProcess(process.raw(), &mut code) } != 0 {
                    outcome.code = code as i32;
                }
                break;
            }
            WAIT_TIMEOUT_CODE if left.is_zero() || SHUTTING_DOWN.load(Ordering::SeqCst) || cancel.is_some_and(|token| token.is_cancelled()) => {
                outcome.cancelled = SHUTTING_DOWN.load(Ordering::SeqCst) || cancel.is_some_and(|token| token.is_cancelled());
                outcome.timed_out = !outcome.cancelled;
                // По таймауту гасим всё дерево: и команду, и всё, что она запустила.
                terminate(&job, &process);
                unsafe { WaitForSingleObject(process.raw(), 5000) };
                break;
            }
            WAIT_TIMEOUT_CODE => {}
            _ => {
                terminate(&job, &process);
                unsafe { WaitForSingleObject(process.raw(), 5000) };
                break;
            }
        }
    }
    Ok(outcome)
}

/// Заполняет описание возможностей контейнера. Аргументы — ссылки, чтобы адреса не менялись.
fn fill_capabilities(
    container: &Container,
    capability: &mut SID_AND_ATTRIBUTES,
    capabilities: &mut SECURITY_CAPABILITIES,
) {
    *capability = SID_AND_ATTRIBUTES { Sid: container.capability.as_psid(), Attributes: SE_GROUP_ENABLED_FLAG };
    *capabilities = SECURITY_CAPABILITIES {
        AppContainerSid: container.sid.as_psid(),
        Capabilities: capability,
        CapabilityCount: 1,
        Reserved: 0,
    };
}

fn classify_process_error(code: u32, level: Level) -> SpawnError {    let text = format!("CreateProcess ({}): {}", level.name(), win_error(code));
    // Команду не нашли — это не поломка изоляции: уровень из-за опечатки не понижаем.
    let command_problem = level == Level::Off
        || matches!(code, ERROR_FILE_NOT_FOUND | ERROR_PATH_NOT_FOUND | 193 /* ERROR_BAD_EXE_FORMAT */ | 267 /* ERROR_DIRECTORY */);
    if command_problem {
        SpawnError::command(text)
    } else {
        SpawnError::sandbox(text)
    }
}

fn terminate(job: &Option<Handle>, process: &Handle) {
    match job {
        Some(job) => unsafe { TerminateJobObject(job.raw(), 1) },
        None => {
            // При откате до Level::Off job нет: завершаем только дерево этого процесса.
            use std::os::windows::process::CommandExt;
            let pid = unsafe { windows_sys::Win32::System::Threading::GetProcessId(process.raw()) };
            if pid != 0 {
                let _ = std::process::Command::new("taskkill")
                    .args(["/PID", &pid.to_string(), "/T", "/F"])
                    .creation_flags(CREATE_NO_WINDOW)
                    .stdin(std::process::Stdio::null()).stdout(std::process::Stdio::null()).stderr(std::process::Stdio::null())
                    .status();
            }
            unsafe { windows_sys::Win32::System::Threading::TerminateProcess(process.raw(), 1) }
        },
    };
}

fn create_job() -> Result<Handle, SpawnError> {
    let raw = unsafe { CreateJobObjectW(std::ptr::null(), std::ptr::null()) };
    let job = Handle::new(raw).ok_or_else(|| SpawnError::sandbox(format!("CreateJobObjectW: {}", win_error(unsafe { GetLastError() }))))?;

    // Без kill-on-close: запущенное в фоне (сервер разработки, `start`) живёт и после команды, как раньше.
    // Всё дерево гасится явно, по таймауту или при выходе из программы. Предела памяти нет:
    // компилятор и сборщики фронтенда законно берут больше пары гигабайт.
    let mut limits: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = unsafe { std::mem::zeroed() };
    limits.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_ACTIVE_PROCESS;
    limits.BasicLimitInformation.ActiveProcessLimit = MAX_PROCESSES;
    let ok = unsafe {
        SetInformationJobObject(
            job.raw(),
            JobObjectExtendedLimitInformation,
            &limits as *const _ as *const core::ffi::c_void,
            std::mem::size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
        )
    };
    if ok == 0 {
        return Err(SpawnError::sandbox(format!("SetInformationJobObject (limits): {}", win_error(unsafe { GetLastError() }))));
    }

    // Команде в песочнице нечего делать с чужим рабочим столом, буфером и настройками экрана.
    let mut ui: JOBOBJECT_BASIC_UI_RESTRICTIONS = unsafe { std::mem::zeroed() };
    ui.UIRestrictionsClass = JOB_OBJECT_UILIMIT_DESKTOP
        | JOB_OBJECT_UILIMIT_READCLIPBOARD
        | JOB_OBJECT_UILIMIT_WRITECLIPBOARD
        | JOB_OBJECT_UILIMIT_GLOBALATOMS
        | JOB_OBJECT_UILIMIT_SYSTEMPARAMETERS
        | JOB_OBJECT_UILIMIT_DISPLAYSETTINGS
        | JOB_OBJECT_UILIMIT_EXITWINDOWS;
    // Не считаем провал фатальным: на части сборок Windows эти ограничения не ставятся,
    // а kill-on-close и лимиты уже работают.
    unsafe {
        SetInformationJobObject(
            job.raw(),
            JobObjectBasicUIRestrictions,
            &ui as *const _ as *const core::ffi::c_void,
            std::mem::size_of::<JOBOBJECT_BASIC_UI_RESTRICTIONS>() as u32,
        );
    }
    Ok(job)
}

fn open_own_token() -> Result<Handle, String> {
    let mut token: HANDLE = std::ptr::null_mut();
    let process = unsafe { GetCurrentProcess() };
    if unsafe { OpenProcessToken(process, TOKEN_ALL_ACCESS, &mut token) } != 0 {
        if let Some(handle) = Handle::new(token) {
            return Ok(handle);
        }
    }
    // Полный доступ бывает закрыт и на своём токене: для ограниченной копии хватает этого.
    let mask = TOKEN_QUERY | TOKEN_DUPLICATE | TOKEN_ASSIGN_PRIMARY | TOKEN_ADJUST_DEFAULT | TOKEN_ADJUST_SESSIONID;
    token = std::ptr::null_mut();
    if unsafe { OpenProcessToken(process, mask, &mut token) } == 0 {
        return Err(format!("OpenProcessToken: {}", win_error(unsafe { GetLastError() })));
    }
    Handle::new(token).ok_or_else(|| "OpenProcessToken вернул пустой handle".to_string())
}

fn create_restricted_token() -> Result<Handle, String> {
    let original = open_own_token()?;
    let mut restricted: HANDLE = std::ptr::null_mut();
    let ok = unsafe {
        CreateRestrictedToken(
            original.raw(),
            DISABLE_MAX_PRIVILEGE | LUA_TOKEN,
            0,
            std::ptr::null(),
            0,
            std::ptr::null(),
            0,
            std::ptr::null(),
            &mut restricted,
        )
    };
    if ok == 0 {
        return Err(format!("CreateRestrictedToken: {}", win_error(unsafe { GetLastError() })));
    }
    Handle::new(restricted).ok_or_else(|| "CreateRestrictedToken вернул пустой handle".to_string())
}

/// Сколько привилегий в токене: у ограниченной копии их заведомо меньше.
fn privilege_count(token: &Handle) -> Result<u32, String> {
    let mut needed: u32 = 0;
    unsafe { GetTokenInformation(token.raw(), TokenPrivileges, std::ptr::null_mut(), 0, &mut needed) };
    if needed == 0 {
        return Err(format!("GetTokenInformation (размер): {}", win_error(unsafe { GetLastError() })));
    }
    let mut buffer = vec![0u8; needed as usize];
    let ok = unsafe {
        GetTokenInformation(token.raw(), TokenPrivileges, buffer.as_mut_ptr() as *mut core::ffi::c_void, needed, &mut needed)
    };
    if ok == 0 {
        return Err(format!("GetTokenInformation (привилегии): {}", win_error(unsafe { GetLastError() })));
    }
    // TOKEN_PRIVILEGES начинается с DWORD PrivilegeCount.
    Ok(unsafe { *(buffer.as_ptr() as *const u32) })
}

fn is_token_restricted(token: &Handle) -> Result<bool, String> {
    let mut value: u32 = 0;
    let mut needed: u32 = 0;
    let ok = unsafe {
        GetTokenInformation(
            token.raw(),
            TokenIsRestricted,
            &mut value as *mut u32 as *mut core::ffi::c_void,
            std::mem::size_of::<u32>() as u32,
            &mut needed,
        )
    };
    if ok == 0 {
        return Err(format!("GetTokenInformation (TokenIsRestricted): {}", win_error(unsafe { GetLastError() })));
    }
    Ok(value != 0)
}

/// Токен самого приложения уже ограничен средой. Так бывает в песочницах и у служб:
/// создать из него ещё одну ограниченную копию Windows отказывается (ERROR_INVALID_PARAMETER),
/// но дети наследуют ограничение и без нас.
fn own_token_restricted() -> bool {
    static RESTRICTED: OnceLock<bool> = OnceLock::new();
    *RESTRICTED.get_or_init(|| match open_own_token() {
        Ok(token) => is_token_restricted(&token).unwrap_or(false),
        Err(_) => false,
    })
}

fn inherit_handle(handle: HANDLE) -> Result<(), String> {
    if unsafe { SetHandleInformation(handle, HANDLE_FLAG_INHERIT, HANDLE_FLAG_INHERIT) } == 0 {
        return Err(format!("SetHandleInformation: {}", win_error(unsafe { GetLastError() })));
    }
    Ok(())
}

fn win_error(code: u32) -> String {
    let text = std::io::Error::from_raw_os_error(code as i32).to_string();
    format!("{text} (код {code})")
}

fn hresult_text(hr: HRESULT) -> String {
    let raw = hr as u32;
    if raw & 0xFFFF_0000 == 0x8007_0000 {
        std::io::Error::from_raw_os_error((raw & 0xFFFF) as i32).to_string()
    } else {
        format!("0x{raw:08x}")
    }
}

// ---------------------------------------------------------------------------
// AppContainer
// ---------------------------------------------------------------------------

/// Атрибуты процесса для AppContainer: список атрибутов живёт до CreateProcess.
struct AttributeList {
    list: LPPROC_THREAD_ATTRIBUTE_LIST,
    buffer: Vec<u8>,
}

impl AttributeList {
    fn new(count: u32) -> Result<AttributeList, String> {
        let mut size: usize = 0;
        // Первый вызов заведомо неуспешен: он только сообщает нужный размер.
        unsafe { InitializeProcThreadAttributeList(std::ptr::null_mut(), count, 0, &mut size) };
        if size == 0 {
            return Err(format!("InitializeProcThreadAttributeList: {}", win_error(unsafe { GetLastError() })));
        }
        let mut buffer = vec![0u8; size];
        let list = buffer.as_mut_ptr() as LPPROC_THREAD_ATTRIBUTE_LIST;
        if unsafe { InitializeProcThreadAttributeList(list, count, 0, &mut size) } == 0 {
            return Err(format!("InitializeProcThreadAttributeList: {}", win_error(unsafe { GetLastError() })));
        }
        Ok(AttributeList { list, buffer })
    }

    fn raw(&self) -> LPPROC_THREAD_ATTRIBUTE_LIST {
        self.list
    }

    fn set_security_capabilities(&mut self, caps: &SECURITY_CAPABILITIES) -> Result<(), String> {
        let ok = unsafe {
            UpdateProcThreadAttribute(
                self.list,
                0,
                PROC_THREAD_ATTRIBUTE_SECURITY_CAPABILITIES as usize,
                caps as *const _ as *const core::ffi::c_void,
                std::mem::size_of::<SECURITY_CAPABILITIES>(),
                std::ptr::null_mut(),
                std::ptr::null(),
            )
        };
        if ok == 0 {
            return Err(format!("UpdateProcThreadAttribute: {}", win_error(unsafe { GetLastError() })));
        }
        Ok(())
    }
}

impl Drop for AttributeList {
    fn drop(&mut self) {
        unsafe { DeleteProcThreadAttributeList(self.list) };
        let _ = &self.buffer;
    }
}

/// SID в собственном буфере: userenv отдаёт SID через LocalAlloc, а он должен пережить CreateProcess.
struct SidBuf {
    words: Vec<u32>,
}

impl SidBuf {
    unsafe fn copy_from(sid: PSID) -> Option<SidBuf> {
        if sid.is_null() {
            return None;
        }
        let bytes = sid as *const u8;
        // SID: revision, subauthority_count, authority[6], subauthority[count].
        let count = *bytes.add(1) as usize;
        let len = 8 + 4 * count;
        let mut words = vec![0u32; (len + 3) / 4];
        std::ptr::copy_nonoverlapping(bytes, words.as_mut_ptr() as *mut u8, len);
        Some(SidBuf { words })
    }

    fn from_parts(authority: u64, sub_authorities: &[u32]) -> SidBuf {
        let mut bytes = vec![1u8, sub_authorities.len() as u8];
        bytes.extend_from_slice(&authority.to_be_bytes()[2..8]);
        for sub in sub_authorities {
            bytes.extend_from_slice(&sub.to_le_bytes());
        }
        let mut words = vec![0u32; (bytes.len() + 3) / 4];
        unsafe { std::ptr::copy_nonoverlapping(bytes.as_ptr(), words.as_mut_ptr() as *mut u8, bytes.len()) };
        SidBuf { words }
    }

    fn as_psid(&self) -> PSID {
        self.words.as_ptr() as PSID
    }

    fn to_text(&self) -> String {
        let bytes = self.words.as_ptr() as *const u8;
        unsafe {
            let revision = *bytes;
            let count = *bytes.add(1) as usize;
            let mut authority: u64 = 0;
            for i in 0..6 {
                authority = (authority << 8) | *bytes.add(2 + i) as u64;
            }
            let mut text = format!("S-{revision}-{authority}");
            for i in 0..count {
                let sub = std::ptr::read_unaligned(bytes.add(8 + 4 * i) as *const u32);
                text.push_str(&format!("-{sub}"));
            }
            text
        }
    }
}

type CreateProfileFn =
    unsafe extern "system" fn(PCWSTR, PCWSTR, PCWSTR, *mut SID_AND_ATTRIBUTES, u32, *mut PSID) -> HRESULT;
type DeriveSidFn = unsafe extern "system" fn(PCWSTR, *mut PSID) -> HRESULT;

struct UserEnv {
    create_profile: CreateProfileFn,
    derive_sid: DeriveSidFn,
}

static USERENV: OnceLock<Option<UserEnv>> = OnceLock::new();

fn userenv() -> Result<&'static UserEnv, SpawnError> {
    USERENV
        .get_or_init(|| unsafe { load_userenv() })
        .as_ref()
        .ok_or_else(|| SpawnError::sandbox("userenv.dll: функции AppContainer недоступны (Windows старше 8 или запрет)"))
}

unsafe fn load_userenv() -> Option<UserEnv> {
    let module = LoadLibraryW(wide("userenv.dll").as_ptr());
    if module.is_null() {
        return None;
    }
    Some(UserEnv {
        create_profile: symbol(module, b"CreateAppContainerProfile\0")?,
        derive_sid: symbol(module, b"DeriveAppContainerSidFromAppContainerName\0")?,
    })
}

unsafe fn symbol<T: Copy>(module: HMODULE, name: &[u8]) -> Option<T> {
    let procedure = GetProcAddress(module, name.as_ptr())?;
    Some(std::mem::transmute_copy(&procedure))
}

/// Профиль контейнера: SID и capability.
struct Container {
    sid: SidBuf,
    sid_text: String,
    capability: SidBuf,
}

static CONTAINER: OnceLock<Mutex<Option<std::sync::Arc<Container>>>> = OnceLock::new();

fn app_container() -> Result<std::sync::Arc<Container>, SpawnError> {
    let lock = CONTAINER.get_or_init(|| Mutex::new(None));
    let mut slot = lock.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    if let Some(container) = slot.as_ref() {
        return Ok(container.clone());
    }
    let container = std::sync::Arc::new(Container::create()?);
    *slot = Some(container.clone());
    Ok(container)
}

impl Container {
    fn create() -> Result<Container, SpawnError> {
        let userenv = userenv()?;
        let capability = SidBuf::from_parts(CAPABILITY_INTERNET_CLIENT.0, CAPABILITY_INTERNET_CLIENT.1);
        let name = wide(APP_CONTAINER_NAME);
        let title = wide(APP_CONTAINER_TITLE);
        let description = wide(APP_CONTAINER_DESCRIPTION);
        let mut capability_attribute =
            SID_AND_ATTRIBUTES { Sid: capability.as_psid(), Attributes: SE_GROUP_ENABLED_FLAG };
        let mut sid_ptr: PSID = std::ptr::null_mut();
        let hr = unsafe {
            (userenv.create_profile)(name.as_ptr(), title.as_ptr(), description.as_ptr(), &mut capability_attribute, 1, &mut sid_ptr)
        };
        if !sid_ptr.is_null() {
            unsafe { LocalFree(sid_ptr) };
            sid_ptr = std::ptr::null_mut();
        }
        if hr < 0 && hr != HRESULT_ALREADY_EXISTS {
            return Err(SpawnError::sandbox(format!(
                "CreateAppContainerProfile: 0x{:08x} ({})",
                hr as u32,
                hresult_text(hr)
            )));
        }
        let hr = unsafe { (userenv.derive_sid)(name.as_ptr(), &mut sid_ptr) };
        if hr < 0 || sid_ptr.is_null() {
            return Err(SpawnError::sandbox(format!(
                "DeriveAppContainerSidFromAppContainerName: 0x{:08x} ({})",
                hr as u32,
                hresult_text(hr)
            )));
        }
        let sid = unsafe { SidBuf::copy_from(sid_ptr) }.ok_or_else(|| SpawnError::sandbox("пустой SID контейнера"))?;
        unsafe { LocalFree(sid_ptr) };
        let sid_text = sid.to_text();
        Ok(Container { sid, sid_text, capability })
    }
}

static GRANTED: OnceLock<Mutex<HashSet<PathBuf>>> = OnceLock::new();

/// Выдаёт SID контейнера ACE на папку воркспейса. Без этого контейнер не прочитает даже cwd.
fn grant_workspace(container: &Container, dir: &Path) -> Result<(), SpawnError> {
    let key = dir.canonicalize().unwrap_or_else(|_| dir.to_path_buf());
    let granted = GRANTED.get_or_init(|| Mutex::new(HashSet::new()));
    if granted.lock().unwrap_or_else(|poisoned| poisoned.into_inner()).contains(&key) {
        return Ok(());
    }
    // /T — чтобы ACE получили и файлы, созданные до первого запуска: у них своей записи нет.
    let output = std::process::Command::new("icacls")
        .arg(dir)
        .arg("/grant")
        .arg(format!("*{}:(OI)(CI)M", container.sid_text))
        .arg("/T")
        .arg("/Q")
        .output()
        .map_err(|e| SpawnError::sandbox(format!("icacls: {e}")))?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(SpawnError::sandbox(format!(
            "icacls не выдал ACE SID контейнера на {}: код {:?} {}",
            dir.display(),
            output.status.code(),
            first_line(&stderr)
        )));
    }
    granted.lock().unwrap_or_else(|poisoned| poisoned.into_inner()).insert(key);
    Ok(())
}

// ---------------------------------------------------------------------------
// Самопроверка
// ---------------------------------------------------------------------------

/// Временная папка проверки: убираем за собой даже при ошибке.
struct TempDir(PathBuf);

impl TempDir {
    fn new() -> Result<TempDir, String> {
        let dir = canary_root()?.join(format!("probe-{}", unique_stamp()));
        std::fs::create_dir_all(&dir).map_err(|e| format!("не смог создать {}: {e}", dir.display()))?;
        Ok(TempDir(dir))
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn canary_root() -> Result<PathBuf, String> {
    let mut candidates: Vec<PathBuf> = Vec::new();
    if let Some(dir) = std::env::var_os("CREW_SANDBOX_TMP") {
        candidates.push(PathBuf::from(dir));
    }
    candidates.push(data_dir().join("tmp"));
    candidates.push(std::env::temp_dir());
    for dir in candidates {
        if std::fs::create_dir_all(&dir).is_ok() {
            return Ok(dir);
        }
    }
    Err("нет папки, куда можно положить канарейку".to_string())
}

/// Папка данных приложения. Повторяет правило `main.rs`: sandbox не должен зависеть от него.
fn data_dir() -> PathBuf {
    if let Some(dir) = std::env::var_os("CREW_DATA_DIR") {
        return PathBuf::from(dir);
    }
    app_root().join("data")
}

fn app_root() -> PathBuf {
    let exe = std::env::current_exe().unwrap_or_else(|_| PathBuf::from("deloist"));
    let dir = exe.parent().map(Path::to_path_buf).unwrap_or_else(|| PathBuf::from("."));
    dir.ancestors().find(|path| path.join("Cargo.toml").exists()).map(Path::to_path_buf).unwrap_or(dir)
}

fn unique_stamp() -> String {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|elapsed| elapsed.as_nanos())
        .unwrap_or(0);
    format!("{}-{nanos}", std::process::id())
}

fn canary(level: Level) -> Result<(), String> {
    let temp = TempDir::new()?;
    let canary = temp.0.join("canary.txt");
    let marker = format!("canary-{}", unique_stamp());
    std::fs::write(&canary, &marker).map_err(|e| format!("канарейка: {e}"))?;
    let out = temp.0.join("out.txt");
    let err = temp.0.join("err.txt");
    let outcome = spawn_at(level, &type_command(&canary), &temp.0, &probe_env(), PROBE_TIMEOUT, &out, &err, None)?;
    if outcome.timed_out {
        return Err("команда не уложилась в таймаут".to_string());
    }
    if outcome.code != 0 {
        return Err(format!("type вернул {}: {}", outcome.code, first_line(&read_text(&err))));
    }
    if !read_text(&out).contains(&marker) {
        return Err("канарейка прочиталась, но содержимое не совпало".to_string());
    }
    Ok(())
}

fn container_canary() -> Result<String, String> {
    let temp = TempDir::new()?;
    let work = temp.0.join("work");
    std::fs::create_dir_all(&work).map_err(|e| format!("рабочая папка контейнера: {e}"))?;
    let marker = format!("canary-{}", unique_stamp());
    let outside = temp.0.join("outside.txt");
    std::fs::write(&outside, &marker).map_err(|e| format!("канарейка: {e}"))?;
    let env = probe_env();

    // 1. Файл вне выданной папки контейнер читать не должен. Этот же запуск выдаёт ACE на work.
    let out = temp.0.join("out1.txt");
    let err = temp.0.join("err1.txt");
    let first = spawn_at(Level::AppContainer, &type_command(&outside), &work, &env, PROBE_TIMEOUT, &out, &err, None)?;
    if first.timed_out {
        return Err("type вне воркспейса не уложился в таймаут".to_string());
    }
    if first.code == 0 && read_text(&out).contains(&marker) {
        return Err("контейнер прочитал файл вне воркспейса: изоляции нет".to_string());
    }

    // 2. Внутри выданной папки контейнер обязан работать, иначе команды просто не запустятся.
    // Файл создаём после выдачи ACE: он унаследует запись от папки.
    let inside = work.join("inside.txt");
    std::fs::write(&inside, &marker).map_err(|e| format!("канарейка внутри: {e}"))?;
    let out = temp.0.join("out2.txt");
    let err = temp.0.join("err2.txt");
    let second = spawn_at(Level::AppContainer, &type_command(&inside), &work, &env, PROBE_TIMEOUT, &out, &err, None)?;
    if second.code != 0 || !read_text(&out).contains(&marker) {
        return Err(format!(
            "контейнер не читает выданную папку: код {}, {}",
            second.code,
            first_line(&read_text(&err))
        ));
    }
    Ok(format!("канарейка снаружи недоступна (код {}), внутри воркспейса читается", first.code))
}

fn probe_env() -> Vec<(OsString, OsString)> {
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
    let mut out = Vec::new();
    for name in KEEP {
        if let Some(value) = std::env::var_os(name) {
            out.push((OsString::from(*name), value));
        }
    }
    out
}

fn type_command(path: &Path) -> String {
    let comspec = std::env::var("ComSpec").unwrap_or_else(|_| "cmd.exe".to_string());
    format!("\"{comspec}\" /d /s /c \"type \"{}\"\"", path.display())
}

fn read_text(path: &Path) -> String {
    std::fs::read(path).map(|bytes| String::from_utf8_lossy(&bytes).into_owned()).unwrap_or_default()
}

fn first_line(text: &str) -> String {
    let line = text.lines().find(|line| !line.trim().is_empty()).unwrap_or("").trim();
    line.chars().take(200).collect()
}

fn wide(text: &str) -> Vec<u16> {
    text.encode_utf16().chain(std::iter::once(0)).collect()
}

fn wide_path(path: &Path) -> Vec<u16> {
    path.as_os_str().encode_wide().chain(std::iter::once(0)).collect()
}

// ---------------------------------------------------------------------------
// Юнит-тесты
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn protected_commands_are_caught() {
        let cases = [
            "type %APPDATA%\\Delos\\secrets.json",
            "echo %LOCALAPPDATA%",
            "dir %USERPROFILE%\\.ssh",
            "type C:\\Users\\me\\.ssh\\id_rsa",
            "copy data\\secrets.json backup\\",
            "copy data\\secrets\\connectors.bin x",
            "type %HOMEDRIVE%%HOMEPATH%\\.aws\\credentials",
            "powershell -c \"gc $env:USERPROFILE\\.aws\\credentials\"",
            "dir \\\\?\\C:\\Windows",
            "reg add HKCU\\Software\\Foo /v Bar /d 1 /f",
            "cmd /d /s /c \"reg add HKCU\\Software\\Foo /v Bar /f\"",
            "schtasks /create /tn bot /tr calc.exe /sc onlogon",
            "reg delete HKCU\\Software\\Foo /f",
            "net user bot pass /add",
            "net user",
        ];
        for command in cases {
            assert!(looks_protected(command).is_some(), "не поймали: {command}");
        }
    }

    #[test]
    fn ssh_refusal_points_to_config_alias() {
        let reason = looks_protected("type C:\\Users\\me\\.ssh\\id_rsa").expect("должны поймать обращение к ключу");
        assert!(reason.contains("~/.ssh/config"), "в отказе нет подсказки про имя из ~/.ssh/config: {reason}");
    }

    #[test]
    fn normal_commands_are_not_touched() {
        let cases = [
            "npm install",
            "npm ci && npm run build",
            "git status",
            "git log --oneline -5",
            "git commit -m \"fix reg add parsing\"",
            "python script.py",
            "python -m venv .venv",
            "cargo build --offline",
            "echo привет",
            "dir ..",
            "mkdir build && cd build",
            "copy file.txt ..\\backup\\",
            "type README.md",
            "echo \"reg add\"",
            "npm config get cache",
            "git remote -v",
            "cd ..\\.. && dir",
            "npm install --prefix ..\\..\\app",
            "schtasks /query",
            "reg query HKCU\\Software\\Foo",
            "ssh myserver \"cd ~/projects/site && git status\"",
            "scp -q file.txt myserver:/tmp/",
        ];
        for command in cases {
            assert!(looks_protected(command).is_none(), "ложное срабатывание: {command} → {:?}", looks_protected(command));
        }
    }

    #[test]
    fn refusal_names_the_reason() {
        assert!(looks_protected("reg add HKCU\\X /v Y").unwrap().contains("реестр"));
        assert!(looks_protected("type %USERPROFILE%\\x").unwrap().contains("%USERPROFILE%"));
        assert!(looks_protected("type C:\\a\\.ssh\\id_rsa").unwrap().contains("SSH"));
    }

    #[test]
    fn segment_split_respects_quotes() {
        let parts = segments("echo \"a && b\" && reg add HKCU\\X");
        assert_eq!(parts.len(), 2, "{parts:?}");
        assert!(looks_protected("echo \"a && b\" && reg add HKCU\\X").is_some());
    }

    fn decode(block: &[u16]) -> Vec<String> {
        let mut out = Vec::new();
        let mut current: Vec<u16> = Vec::new();
        for &unit in block {
            if unit == 0 {
                if current.is_empty() {
                    break;
                }
                out.push(String::from_utf16(&current).expect("utf-16"));
                current.clear();
            } else {
                current.push(unit);
            }
        }
        out
    }

    #[test]
    fn env_block_is_sorted_and_double_null_terminated() {
        let env = vec![
            (OsString::from("B"), OsString::from("2")),
            (OsString::from("A"), OsString::from("1")),
            (OsString::from("PATH"), OsString::from("C:\\bin")),
        ];
        let block = build_env_block(&env);
        assert_eq!(decode(&block), vec!["A=1", "B=2", "PATH=C:\\bin"]);
        assert_eq!(&block[block.len() - 2..], &[0, 0], "блок обязан кончаться двумя нулями");
    }

    #[test]
    fn env_block_skips_broken_names() {
        let env = vec![
            (OsString::from(""), OsString::from("x")),
            (OsString::from("BAD=NAME"), OsString::from("x")),
            (OsString::from("OK"), OsString::from("да")),
        ];
        assert_eq!(decode(&build_env_block(&env)), vec!["OK=да"]);
    }

    #[test]
    fn home_variables_move_into_the_workspace() {
        let env = vec![
            (OsString::from("HOME"), OsString::from("C:\\Users\\me")),
            (OsString::from("PATH"), OsString::from("C:\\bin")),
            (OsString::from("temp"), OsString::from("C:\\Temp")),
        ];
        let remapped = remap_home_env(&env, Path::new("C:\\ws\\.home"));
        let get = |name: &str| {
            remapped
                .iter()
                .find(|(key, _)| key.to_string_lossy().eq_ignore_ascii_case(name))
                .map(|(_, value)| value.to_string_lossy().into_owned())
                .unwrap()
        };
        for name in HOME_VARS {
            assert_eq!(get(name), "C:\\ws\\.home", "{name}");
        }
        assert_eq!(get("PATH"), "C:\\bin", "остальные переменные не трогаем");
    }

    #[test]
    fn mode_parsing_is_forgiving_but_safe() {
        assert_eq!(parse_mode("off"), Mode::Off);
        assert_eq!(parse_mode(" OFF "), Mode::Off);
        assert_eq!(parse_mode("strict"), Mode::Strict);
        assert_eq!(parse_mode("AppContainer"), Mode::Strict);
        assert_eq!(parse_mode("auto"), Mode::Auto);
        assert_eq!(parse_mode(""), Mode::Auto);
        assert_eq!(parse_mode("что-то"), Mode::Auto, "незнакомое значение не выключает защиту");
    }
}
