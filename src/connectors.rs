//! Подключения из каталога: MCP-сервер из манифеста с закреплённой установкой, согласием
//! человека, секретами вне state.json, входом через OAuth и проверкой, куда сервер ходит.
//!
//! Подключение — это обычный [`McpServer`] с заполненным `connector`. До согласия он выключен
//! и не запускается ни для модели, ни для «Проверить». Ключи живут в [`crate::secrets`], в
//! настройках сервера — только ссылки `{{secret:NAME}}`.

use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use regex::Regex;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::oauth;
use crate::secrets::{self, Secrets};
use crate::store::McpServer;
use crate::App;

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", default)]
pub struct Meta {
    pub manifest_id: String,
    pub version: String,
    pub title: String,
    pub summary: String,
    pub icon: String,
    pub install: Install,
    pub egress: Vec<String>,
    pub auth: Auth,
    /// Когда человек согласился на запуск. 0 — ещё нет: сервер не запускается.
    pub consented_at: i64,
    /// Вход протух или токен отозван: нужна кнопка «Переподключить».
    pub needs_reauth: bool,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", default)]
pub struct Install {
    /// npx | uvx | binary | none
    pub kind: String,
    pub package: String,
    pub pin: String,
    pub url: String,
    pub size: u64,
    pub sha256: String,
    pub file: String,
    /// uvx: зависимости не новее этой даты (`--exclude-newer`).
    #[serde(skip_serializing_if = "String::is_empty")]
    pub date: String,
    /// sha256 распакованного exe для архива: сверяем при каждом запуске.
    #[serde(skip_serializing_if = "String::is_empty")]
    pub exe_sha256: String,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", default)]
pub struct Auth {
    /// none | fields | oauth | mcp-oauth
    pub kind: String,
    pub fields: Vec<Field>,
    pub oauth: oauth::Config,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", default)]
pub struct Field {
    pub name: String,
    pub label: String,
    pub help: String,
}

/// Состояние подключений, которое не надо хранить на диске.
pub struct Connectors {
    pub secrets: Secrets,
    pub oauth: oauth::Flows,
    errors: Mutex<HashMap<String, String>>,
    tools: Mutex<HashMap<String, usize>>,
}

impl Connectors {
    pub fn open(data_dir: &Path) -> Self {
        Self { secrets: Secrets::open(data_dir), oauth: oauth::Flows::default(), errors: Mutex::default(), tools: Mutex::default() }
    }

    pub fn set_error(&self, id: &str, error: &str) {
        self.errors.lock().unwrap().insert(id.to_string(), error.to_string());
    }

    pub fn clear_error(&self, id: &str) {
        self.errors.lock().unwrap().remove(id);
    }

    pub fn set_tools(&self, id: &str, count: usize) {
        self.tools.lock().unwrap().insert(id.to_string(), count);
    }

    fn forget(&self, id: &str) {
        self.errors.lock().unwrap().remove(id);
        self.tools.lock().unwrap().remove(id);
        self.oauth.cancel(id);
    }
}

/// Отказ API: статус, текст и необязательные подробности.
pub struct Fail(pub u16, pub String, pub Value);

fn fail(status: u16, message: impl Into<String>) -> Fail {
    Fail(status, message.into(), Value::Null)
}

// ---------------------------------------------------------------------------
// Проверки шаблонов, имён и хостов
// ---------------------------------------------------------------------------

/// Переменные, которые для запуска задаёт само приложение: их список в `process::SAFE_ENV`
/// (PATH, COMSPEC, APPDATA, USERPROFILE…). Манифест их не подменяет.
fn locked_env(name: &str) -> bool {
    crate::process::SAFE_ENV.iter().any(|locked| locked.eq_ignore_ascii_case(name))
}

/// npm и uv читают настройки из окружения: свой реестр, свой индекс или `NODE_OPTIONS`
/// обошли бы закреплённую версию.
const INSTALLER_ENV_PREFIXES: &[&str] = &["NPM_", "NPM_CONFIG_", "UV_", "PIP_", "NODE_", "PYTHON"];

/// Знаки, которые cmd.exe толкует сам: кавычка, переменные, экранирование, конвейеры.
const CMD_META: &[char] = &['"', '%', '^', '&', '|', '<', '>', '!'];

fn command_part_ok(part: &str) -> bool {
    !part.contains(CMD_META) && !part.chars().any(char::is_control)
}
/// Эти заголовки ставит сам клиент MCP.
const LOCKED_HEADERS: &[&str] = &["host", "content-type", "content-length", "accept", "mcp-session-id", "transfer-encoding", "connection"];

fn env_name_ok(name: &str) -> bool {
    let mut chars = name.chars();
    chars.next().is_some_and(|c| c.is_ascii_alphabetic() || c == '_') && chars.all(|c| c.is_ascii_alphanumeric() || c == '_') && name.len() <= 64
}

fn header_name_ok(name: &str) -> bool {
    !name.is_empty() && name.len() <= 64 && name.bytes().all(|b| b.is_ascii_alphanumeric() || b"!#$%&'*+-.^_`|~".contains(&b))
}

fn check_refs(template: &str) -> Result<(), String> {
    for name in secrets::names_in(template) {
        if !secrets::valid_name(&name) {
            return Err(format!("недопустимое имя секрета «{name}»"));
        }
    }
    // Каждая ссылка закрыта: иначе при запуске подстановка всё равно откажет, лучше сразу.
    let mut rest = template;
    while let Some(start) = rest.find("{{secret:") {
        let after = &rest[start + "{{secret:".len()..];
        let Some(end) = after.find("}}") else {
            return Err("незакрытая ссылка {{secret:…}}".into());
        };
        rest = &after[end + 2..];
    }
    if template.contains('\r') || template.contains('\n') {
        return Err("перевод строки в значении".into());
    }
    Ok(())
}

/// Шаблоны сервера: секреты только в env и заголовках, имена — допустимые.
pub fn check_templates(server: &McpServer) -> Result<(), String> {
    if secrets::has_reference(&server.command) || secrets::has_reference(&server.url) || server.args.iter().any(|a| secrets::has_reference(a)) {
        return Err("секреты {{secret:…}} нельзя класть в команду, адрес или аргументы: их видно в списке процессов. Используйте env или заголовки".into());
    }
    if !command_part_ok(&server.command) || server.args.iter().any(|a| !command_part_ok(a)) {
        return Err("в команде и аргументах нельзя использовать \" % ^ & | < > ! и управляющие символы".into());
    }
    let installer = server.connector.as_ref().is_some_and(|m| matches!(m.install.kind.as_str(), "npx" | "uvx"));
    for (name, value) in &server.env {
        if !env_name_ok(name) {
            return Err(format!("недопустимое имя переменной окружения «{name}»"));
        }
        if locked_env(name) {
            return Err(format!("переменную {name} подменять нельзя"));
        }
        let upper = name.to_ascii_uppercase();
        if installer && INSTALLER_ENV_PREFIXES.iter().any(|prefix| upper.starts_with(prefix)) {
            return Err(format!("переменная {name} меняет установку пакета: её задавать нельзя"));
        }
        check_refs(value)?;
    }
    for (name, value) in &server.headers {
        if !header_name_ok(name) {
            return Err(format!("недопустимое имя заголовка «{name}»"));
        }
        if LOCKED_HEADERS.contains(&name.to_ascii_lowercase().as_str()) {
            return Err(format!("заголовок {name} ставит сам клиент"));
        }
        check_refs(value)?;
    }
    Ok(())
}

/// Имена секретов, на которые ссылается сервер.
pub fn referenced(server: &McpServer) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for value in server.env.values().chain(server.headers.values()) {
        for name in secrets::names_in(value) {
            if !out.contains(&name) {
                out.push(name);
            }
        }
    }
    out
}

fn egress_entry_ok(entry: &str) -> bool {
    let host = entry.strip_prefix("*.").unwrap_or(entry);
    !host.is_empty() && host.len() <= 253 && host.contains(|c: char| c.is_ascii_alphanumeric()) && host.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'.' || b == b'-')
}

/// Хост в списке: точно или как поддомен записи `*.example.com`.
pub fn host_allowed(host: &str, egress: &[String]) -> bool {
    let host = host.trim_end_matches('.').to_ascii_lowercase();
    egress.iter().any(|entry| match entry.strip_prefix("*.") {
        Some(base) => host.len() > base.len() + 1 && host.ends_with(&format!(".{base}")),
        None => host == *entry,
    })
}

fn url_host(url: &str) -> Option<String> {
    let parsed = reqwest::Url::parse(url.trim()).ok()?;
    Some(parsed.host_str()?.trim_start_matches('[').trim_end_matches(']').to_string())
}

/// Адрес HTTP-подключения обязан вести на хост из `egress[]` манифеста, и по https
/// (http — только на эту машину).
pub fn check_egress(server: &McpServer) -> Result<(), String> {
    let Some(meta) = &server.connector else { return Ok(()) };
    if !server.is_http() {
        return Ok(());
    }
    if !oauth::url_ok(&server.url) {
        return Err("адрес подключения должен быть https://".into());
    }
    let host = url_host(&server.url).ok_or("неверный адрес сервера")?;
    if host_allowed(&host, &meta.egress) {
        Ok(())
    } else {
        Err(format!("адрес сервера ведёт на {host}, а его нет в списке разрешённых хостов (egress) подключения"))
    }
}

// ---------------------------------------------------------------------------
// Закреплённые версии
// ---------------------------------------------------------------------------

fn re(pattern: &str) -> Regex {
    Regex::new(pattern).unwrap()
}

fn pin_refused(pin: &str) -> Option<String> {
    let lower = pin.trim().to_ascii_lowercase();
    if lower.is_empty() {
        return Some("нужна точная версия пакета (pin)".into());
    }
    if ["latest", "next", "*", "x"].contains(&lower.as_str()) {
        return Some(format!("версия «{pin}» не закреплена: нужна точная, никаких @latest"));
    }
    None
}

pub fn exact_semver(pin: &str) -> bool {
    re(r"^\d+\.\d+\.\d+(-[0-9A-Za-z.-]+)?(\+[0-9A-Za-z.-]+)?$").is_match(pin)
}

pub fn exact_pep440(pin: &str) -> bool {
    re(r"^\d+(\.\d+)*((a|b|rc)\d+)?(\.post\d+)?(\.dev\d+)?$").is_match(pin)
}

/// Команда и аргументы для установки из манифеста. Для `binary` команда — путь в engines,
/// файл появится только после согласия.
pub fn install_command(data_dir: &Path, id: &str, version: &str, install: &Install, command: &str, args: &[String]) -> Result<(String, Vec<String>), String> {
    match install.kind.as_str() {
        "npx" => {
            if install.package.contains('@') && !install.package.starts_with('@') || install.package.matches('@').count() > 1 {
                return Err("версию пишите в pin, а не в имени пакета".into());
            }
            if !re(r"^(@[a-z0-9][a-z0-9._~-]*/)?[a-z0-9][a-z0-9._~-]*$").is_match(&install.package) || install.package.len() > 214 {
                return Err(format!("неверное имя npm-пакета «{}»", install.package));
            }
            if let Some(why) = pin_refused(&install.pin) {
                return Err(why);
            }
            if !exact_semver(&install.pin) {
                return Err(format!("версия «{}» не точная: нужна вида 1.2.3, без ^ ~ > < и диапазонов", install.pin));
            }
            let mut out = vec!["-y".to_string(), format!("{}@{}", install.package, install.pin)];
            out.extend(args.iter().cloned());
            Ok(("npx".into(), out))
        }
        "uvx" => {
            if !re(r"^[A-Za-z0-9]([A-Za-z0-9._-]*[A-Za-z0-9])?$").is_match(&install.package) {
                return Err(format!("неверное имя пакета «{}»: версию пишите в pin", install.package));
            }
            if let Some(why) = pin_refused(&install.pin) {
                return Err(why);
            }
            if !exact_pep440(&install.pin) {
                return Err(format!("версия «{}» не точная: нужна вида 1.2.3, без == >= ~= и диапазонов", install.pin));
            }
            // Зависимости пакета uv берёт не новее даты манифеста: свежий взломанный выпуск не придёт.
            if !re(r"^\d{4}-\d{2}-\d{2}$").is_match(&install.date) {
                return Err("у uvx нужна дата закрепления зависимостей install.date (ГГГГ-ММ-ДД)".into());
            }
            let mut out = vec!["--exclude-newer".to_string(), install.date.clone(), format!("{}=={}", install.package, install.pin)];
            out.extend(args.iter().cloned());
            Ok(("uvx".into(), out))
        }
        "binary" => {
            let parsed = reqwest::Url::parse(&install.url).map_err(|_| "неверный адрес программы".to_string())?;
            if parsed.scheme() != "https" {
                return Err("программу скачиваем только по https://".into());
            }
            if install.size == 0 || install.size > 512 * 1024 * 1024 {
                return Err("у программы нужен точный размер (size), не больше 512 МБ".into());
            }
            if !re(r"^[0-9a-fA-F]{64}$").is_match(&install.sha256) {
                return Err("у программы нужна сумма sha256 (64 шестнадцатеричных знака)".into());
            }
            if !re(r"^[A-Za-z0-9._-]{1,64}\.exe$").is_match(&install.file) {
                return Err("у программы нужно имя файла .exe (file)".into());
            }
            Ok((engine_dir(data_dir, id, version).join(&install.file).to_string_lossy().into_owned(), args.to_vec()))
        }
        "none" | "" => Ok((command.trim().to_string(), args.to_vec())),
        other => Err(format!("неизвестный способ установки «{other}»")),
    }
}

pub fn engine_dir(data_dir: &Path, id: &str, version: &str) -> PathBuf {
    data_dir.join("engines").join(id).join(version)
}

async fn sha_file(path: &Path) -> Result<String, String> {
    use sha2::Digest;
    use tokio::io::AsyncReadExt;
    let mut file = tokio::fs::File::open(path).await.map_err(|e| e.to_string())?;
    let mut hasher = sha2::Sha256::new();
    let mut buf = vec![0u8; 1 << 16];
    loop {
        let n = file.read(&mut buf).await.map_err(|e| e.to_string())?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    Ok(hasher.finalize().iter().map(|b| format!("{b:02x}")).collect())
}

fn is_zip(install: &Install) -> bool {
    reqwest::Url::parse(&install.url).is_ok_and(|u| u.path().to_ascii_lowercase().ends_with(".zip"))
}

/// Поставить или проверить программу подключения. Сумма сверяется каждый раз: подменённый
/// файл не запустится, а скачается заново. Возвращает путь и sha256 самого exe.
pub async fn install_binary(data_dir: &Path, id: &str, version: &str, install: &Install) -> Result<(PathBuf, String), String> {
    let dir = engine_dir(data_dir, id, version);
    let exe = dir.join(&install.file);
    let expected = if is_zip(install) { install.exe_sha256.as_str() } else { install.sha256.as_str() };
    if !expected.is_empty() && is_windows_program(&exe).await {
        if let Ok(sha) = sha_file(&exe).await {
            if sha.eq_ignore_ascii_case(expected) {
                return Ok((exe, sha));
            }
        }
    }
    let parent = dir.parent().ok_or("нет папки engines")?.to_path_buf();
    tokio::fs::create_dir_all(&parent).await.map_err(|e| e.to_string())?;
    let part = parent.join(format!("{version}.part"));
    let pin = crate::obscura::Pin { urls: vec![install.url.clone()], size: install.size, sha256: install.sha256.clone() };
    let downloaded = crate::obscura::download(&pin, &install.url, &part, &|_| {}).await;
    if let Err(e) = downloaded {
        let _ = tokio::fs::remove_file(&part).await;
        return Err(e);
    }
    let staging = parent.join(format!("{version}.staging"));
    let _ = tokio::fs::remove_dir_all(&staging).await;
    tokio::fs::create_dir_all(&staging).await.map_err(|e| e.to_string())?;
    let placed = if is_zip(install) {
        crate::obscura::extract_zip(&part, &staging).await
    } else {
        tokio::fs::rename(&part, staging.join(&install.file)).await.map_err(|e| e.to_string())
    };
    let _ = tokio::fs::remove_file(&part).await;
    if let Err(e) = placed {
        let _ = tokio::fs::remove_dir_all(&staging).await;
        return Err(e);
    }
    let fresh = staging.join(&install.file);
    if !is_windows_program(&fresh).await {
        let _ = tokio::fs::remove_dir_all(&staging).await;
        return Err(format!("{} не программа для Windows", install.file));
    }
    let sha = sha_file(&fresh).await?;
    if !expected.is_empty() && !sha.eq_ignore_ascii_case(expected) {
        let _ = tokio::fs::remove_dir_all(&staging).await;
        return Err("программа в архиве не та, на которую давали согласие".into());
    }
    let _ = tokio::fs::remove_dir_all(&dir).await;
    tokio::fs::rename(&staging, &dir).await.map_err(|e| format!("не смог переложить файлы: {e}"))?;
    Ok((exe, sha))
}

async fn is_windows_program(path: &Path) -> bool {
    let Ok(mut file) = tokio::fs::File::open(path).await else { return false };
    let mut head = [0u8; 2];
    use tokio::io::AsyncReadExt;
    file.read_exact(&mut head).await.is_ok() && &head == b"MZ"
}

// ---------------------------------------------------------------------------
// Среды запуска
// ---------------------------------------------------------------------------

pub struct Runtime {
    pub name: &'static str,
    pub path: Option<PathBuf>,
    pub help: &'static str,
}

fn find_program(names: &[&str], extra_dirs: &[PathBuf]) -> Option<PathBuf> {
    let path = std::env::var_os("PATH").unwrap_or_default();
    std::env::split_paths(&path).chain(extra_dirs.iter().cloned()).flat_map(|dir| names.iter().map(move |name| dir.join(name))).find(|candidate| candidate.is_file())
}

/// Рядом с npx.cmd лежат node.exe и npm\bin\npx-cli.js: их можно запустить без cmd.exe.
fn node_and_npx_cli(npx: &Path) -> Option<(PathBuf, PathBuf)> {
    let dir = npx.parent()?;
    let node = dir.join(if cfg!(windows) { "node.exe" } else { "node" });
    let cli = dir.join("node_modules").join("npm").join("bin").join("npx-cli.js");
    (node.is_file() && cli.is_file()).then_some((node, cli))
}

pub fn runtime_for(kind: &str) -> Option<Runtime> {
    let home = std::env::var_os("USERPROFILE").map(PathBuf::from);
    match kind {
        "npx" => Some(Runtime {
            name: "node",
            path: find_program(if cfg!(windows) { &["npx.cmd", "npx.exe"] } else { &["npx"] }, &[]),
            help: "Deloist скачает Node.js сам при подключении.",
        }),
        "uvx" => Some(Runtime {
            name: "uv",
            path: find_program(
                if cfg!(windows) { &["uvx.exe"] } else { &["uvx"] },
                &home.iter().flat_map(|h| [h.join(".local").join("bin"), h.join(".cargo").join("bin")]).collect::<Vec<_>>(),
            ),
            help: "Для этого подключения нужен uv (запуск Python-программ). Установите его по инструкции https://docs.astral.sh/uv/ и нажмите «Подключить» ещё раз.",
        }),
        _ => None,
    }
}

// ---------------------------------------------------------------------------
// Манифест
// ---------------------------------------------------------------------------

fn localized(value: &Value) -> String {
    match value {
        Value::String(text) => text.clone(),
        Value::Object(map) => ["ru", "en"]
            .iter()
            .find_map(|lang| map.get(*lang).and_then(Value::as_str))
            .or_else(|| map.values().find_map(Value::as_str))
            .unwrap_or_default()
            .to_string(),
        _ => String::new(),
    }
}

fn text(value: &Value, key: &str) -> String {
    value.get(key).and_then(Value::as_str).unwrap_or_default().trim().to_string()
}

fn take(text: &str, n: usize) -> String {
    text.chars().take(n).collect()
}

/// Разобрать манифест приложения в выключенный сервер без согласия.
pub fn from_manifest(data_dir: &Path, manifest: &Value) -> Result<McpServer, String> {
    if !manifest.is_object() {
        return Err("нужен манифест: объект JSON".into());
    }
    let schema = manifest.get("schema").and_then(Value::as_i64).unwrap_or(1);
    if schema != 1 {
        return Err(format!("неизвестная версия манифеста {schema}"));
    }
    if text(manifest, "kind") != "app" {
        return Err("это не приложение (kind должен быть \"app\")".into());
    }
    let manifest_id = text(manifest, "id");
    if !re(r"^[a-z0-9][a-z0-9._-]{0,63}$").is_match(&manifest_id) {
        return Err("неверный id манифеста".into());
    }
    let version = text(manifest, "version");
    if !re(r"^[0-9A-Za-z]([0-9A-Za-z.+-]{0,30}[0-9A-Za-z])?$").is_match(&version) || version.contains("..") {
        return Err("неверная версия манифеста".into());
    }
    let app = manifest.get("app").filter(|v| v.is_object()).ok_or("в манифесте нет раздела app")?;
    let key = match text(app, "key") {
        k if k.is_empty() => manifest_id.strip_prefix("app.").unwrap_or(&manifest_id).replace('.', "-"),
        k => k,
    };
    // Короткий ключ: имя инструмента для модели `mcp__<ключ>__<инструмент>` ограничено 64 знаками.
    if !re(r"^[a-z0-9][a-z0-9-]{0,23}$").is_match(&key) {
        return Err("ключ приложения (app.key): латиница, цифры и дефис, до 24 знаков".into());
    }
    let url = text(app, "url");
    let transport = match text(app, "transport") {
        t if t.is_empty() => (if url.is_empty() { "stdio" } else { "http" }).to_string(),
        t => t,
    };
    let args: Vec<String> = app.get("args").and_then(Value::as_array).map(|a| a.iter().filter_map(Value::as_str).map(str::to_string).collect()).unwrap_or_default();
    let install: Install = serde_json::from_value(app.get("install").cloned().unwrap_or(json!({}))).map_err(|e| format!("раздел install: {e}"))?;
    let egress: Vec<String> = app.get("egress").or_else(|| manifest.get("egress")).and_then(Value::as_array).map(|a| a.iter().filter_map(Value::as_str).map(|s| s.trim().to_ascii_lowercase()).collect()).unwrap_or_default();
    if let Some(bad) = egress.iter().find(|e| !egress_entry_ok(e)) {
        return Err(format!("неверный хост в egress: «{bad}»"));
    }

    let (command, args, url) = match transport.as_str() {
        "stdio" => {
            // Политика облака: подключение, которое запускает команду, не принимается.
            crate::cloud::deny(crate::cloud::NO_STDIO)?;
            if !url.is_empty() {
                return Err("у stdio-приложения не бывает url".into());
            }
            let (command, args) = install_command(data_dir, &key, &version, &install, &text(app, "command"), &args)?;
            if command.is_empty() {
                return Err("нужна команда запуска или раздел install".into());
            }
            (command, args, String::new())
        }
        "http" => {
            if !crate::mcp::valid_http_url(&url) {
                return Err("адрес HTTP-приложения должен начинаться с http:// или https://".into());
            }
            if !matches!(install.kind.as_str(), "" | "none") {
                return Err("HTTP-приложение ничего не устанавливает (install.kind = none)".into());
            }
            if egress.is_empty() {
                return Err("у HTTP-приложения нужен список хостов egress".into());
            }
            (String::new(), Vec::new(), url)
        }
        other => return Err(format!("неизвестный транспорт «{other}»")),
    };

    let auth_value = app.get("auth").cloned().unwrap_or(json!({}));
    let auth_kind = match text(&auth_value, "kind") {
        k if k.is_empty() => "none".to_string(),
        k => k,
    };
    if !["none", "fields", "oauth", "mcp-oauth"].contains(&auth_kind.as_str()) {
        return Err(format!("неизвестный способ входа «{auth_kind}»"));
    }
    let mut fields = Vec::new();
    for item in auth_value.get("fields").and_then(Value::as_array).cloned().unwrap_or_default() {
        let name = text(&item, "name");
        if !secrets::valid_name(&name) || name.starts_with(secrets::RESERVED_PREFIX) {
            return Err(format!("неверное имя поля «{name}»"));
        }
        let help = text(&item, "help");
        if !help.is_empty() && !help.starts_with("https://") {
            return Err("ссылка «где взять» должна быть https://".into());
        }
        fields.push(Field { name, label: take(&localized(item.get("label").unwrap_or(&Value::Null)), 80), help });
    }
    let mut oauth_cfg: oauth::Config = serde_json::from_value(auth_value.get("oauth").cloned().unwrap_or(json!({}))).map_err(|e| format!("раздел auth.oauth: {e}"))?;
    oauth_cfg.registration_url.clear();
    oauth_cfg.resource.clear();
    match auth_kind.as_str() {
        "oauth" => {
            if oauth_cfg.provider.is_empty() {
                if !oauth::url_ok(&oauth_cfg.authorize_url) || !oauth::url_ok(&oauth_cfg.token_url) {
                    return Err("у OAuth нужен provider или адреса authorize_url и token_url по https".into());
                }
                if oauth_cfg.client_id.is_empty() {
                    return Err("у своего OAuth нужен client_id".into());
                }
            } else {
                if oauth::provider(&oauth_cfg.provider).is_none() {
                    return Err(format!("неизвестный OAuth-провайдер «{}»", oauth_cfg.provider));
                }
                // Встроенный провайдер — наш client id и наши адреса: манифест меняет только scopes,
                // иначе код и verifier ушли бы на чужой сервер токенов вместе с нашим клиентом.
                let only_scopes = oauth::Config { provider: oauth_cfg.provider.clone(), scopes: oauth_cfg.scopes.clone(), ..Default::default() };
                if oauth_cfg != only_scopes {
                    return Err("у встроенного OAuth-провайдера манифест задаёт только scopes".into());
                }
            }
            if let Some(key) = oauth_cfg.extra.keys().find(|k| oauth::RESERVED_PARAMS.contains(&k.to_ascii_lowercase().as_str())) {
                return Err(format!("параметр «{key}» в auth.oauth.extra задаёт само приложение"));
            }
        }
        "mcp-oauth" => {
            if transport != "http" {
                return Err("MCP OAuth бывает только у HTTP-приложений".into());
            }
            // Всё найдём у самого сервера: заданное в манифесте не берём.
            oauth_cfg = oauth::Config { scopes: oauth_cfg.scopes, ..Default::default() };
        }
        _ => oauth_cfg = oauth::Config::default(),
    }

    let env = crate::store::string_map(app.get("env"));
    let mut headers = crate::store::string_map(app.get("headers"));
    let oauth_login = auth_kind == "oauth" || auth_kind == "mcp-oauth";
    if oauth_login && transport == "http" && !headers.keys().any(|k| k.eq_ignore_ascii_case("authorization")) {
        headers.insert("Authorization".into(), format!("Bearer {{{{secret:{}}}}}", oauth::ACCESS));
    }

    let meta = Meta {
        manifest_id,
        version,
        title: take(&localized(manifest.get("title").unwrap_or(&Value::Null)), 60),
        summary: take(&localized(manifest.get("summary").unwrap_or(&Value::Null)), 300),
        icon: take(&text(manifest, "icon"), 60),
        install,
        egress,
        auth: Auth { kind: auth_kind, fields, oauth: oauth_cfg },
        consented_at: 0,
        needs_reauth: false,
    };
    let title = if meta.title.is_empty() { key.clone() } else { meta.title.clone() };
    let server = McpServer {
        id: key,
        name: take(&title, 40),
        command,
        url,
        args,
        enabled: false,
        auto_approve: false,
        created_at: crate::store::now(),
        env,
        headers,
        connector: Some(meta),
        login: None,
    };
    check_templates(&server)?;
    check_egress(&server)?;
    // Каждая ссылка на секрет должна вести к полю ввода или к входу через OAuth.
    let known: Vec<String> = server.connector.as_ref().unwrap().auth.fields.iter().map(|f| f.name.clone()).collect();
    for name in referenced(&server) {
        let from_oauth = oauth_login && name.starts_with(secrets::RESERVED_PREFIX);
        if !from_oauth && !known.contains(&name) {
            return Err(format!("ссылка на секрет «{name}», которого нет среди полей auth.fields"));
        }
    }
    Ok(server)
}

// ---------------------------------------------------------------------------
// Подготовка запуска
// ---------------------------------------------------------------------------

pub struct Prepared {
    pub env: Vec<(String, String)>,
    pub headers: Vec<(String, String)>,
}

/// Перед запуском: согласие, хост, свежий токен, подставленные секреты.
pub async fn prepare(app: &Arc<App>, server: &McpServer) -> Result<Prepared, String> {
    check_templates(server)?;
    if let Some(meta) = &server.connector {
        if meta.consented_at == 0 {
            return Err(format!("подключение «{}» ещё не подтверждено: откройте его карточку и нажмите «Подключить»", server.name));
        }
        check_egress(server)?;
        if meta.install.kind == "binary" {
            // Сумма сверяется при каждом запуске; подменённый файл скачается заново.
            install_binary(app.store.data_dir(), &server.id, &meta.version, &meta.install).await.map_err(|e| format!("программа подключения: {e}"))?;
        }
        if meta.auth.kind == "oauth" || meta.auth.kind == "mcp-oauth" {
            if meta.needs_reauth {
                return Err("вход истёк: нажмите «Переподключить»".into());
            }
            oauth::ensure_fresh(app, &server.id, &meta.auth.oauth).await?;
        }
    } else if let Some(login) = server.login.as_ref().filter(|_| server.is_http()) {
        // Свой сервер: пока входа не было, идём без токена (ответ 401 предложит войти).
        if app.connectors.secrets.get(&server.id, oauth::ACCESS).is_some() {
            if login.needs_reauth {
                return Err("вход истёк: нажмите «Войти»".into());
            }
            oauth::ensure_fresh(app, &server.id, &login.oauth).await?;
        }
    }
    let lookup = |name: &str| app.connectors.secrets.get(&server.id, name);
    let mut env = Vec::new();
    for (name, value) in &server.env {
        env.push((name.clone(), secrets::substitute(value, lookup)?));
    }
    let mut headers = Vec::new();
    for (name, value) in &server.headers {
        let value = secrets::substitute(value, lookup)?;
        if value.contains(['\r', '\n']) {
            return Err(format!("в заголовке {name} перевод строки"));
        }
        headers.push((name.clone(), value));
    }
    Ok(Prepared { env, headers })
}

/// Токен живой сессии скоро истечёт: сессию надо открыть заново, обновив токен.
pub fn token_expiring(app: &App, server: &McpServer) -> bool {
    if !server.connector.as_ref().is_some_and(is_oauth) && server.login.is_none() {
        return false;
    }
    let expires: i64 = app.connectors.secrets.get(&server.id, oauth::EXPIRES).and_then(|v| v.parse().ok()).unwrap_or(0);
    expires != 0 && crate::store::now() + 60_000 >= expires
}

/// Убрать из текста значения секретов этого сервера: сервер мог вернуть ключ в ошибке
/// или в ответе инструмента, а текст уходит странице, в журнал и модели.
pub fn scrub(app: &App, id: &str, text: &str) -> String {
    let mut out = text.to_string();
    let mut values: Vec<String> = app.connectors.secrets.names(id).iter().filter_map(|name| app.connectors.secrets.get(id, name)).filter(|v| v.len() >= 4).collect();
    // Длинные первыми: короткий секрет внутри длинного не оставит хвост.
    values.sort_by_key(|v| std::cmp::Reverse(v.len()));
    for value in values {
        if out.contains(&value) {
            out = out.replace(&value, "•••");
        }
    }
    out
}

/// Свой HTTP-сервер ответил 401: если ключа от человека в заголовках нет, предлагаем войти.
async fn custom_failure(app: &Arc<App>, server: &McpServer, error: String) -> String {
    if !server.is_http() || !error.contains("HTTP 401") {
        return error;
    }
    let own_key = server.headers.iter().any(|(k, v)| k.eq_ignore_ascii_case("authorization") && !v.contains("{{secret:oauth."));
    let bearer = app.connectors.oauth.challenges.lock().unwrap().get(&server.id).is_some_and(|c| c.bearer);
    // Без заголовка Bearer и с ключом в адресе (`?key=…`) это скорее неверный ключ, а не вход.
    let key_in_url = reqwest::Url::parse(server.url.trim()).is_ok_and(|u| u.query().is_some());
    if own_key || (!bearer && key_in_url) {
        return error;
    }
    // Сервер сам просит Bearer — говорим прямо; иначе оставляем его текст («требует ключ»), но «Войти» предлагаем.
    let text = if bearer { "нужно войти: нажмите «Войти»".to_string() } else { error };
    app.connectors.set_error(&server.id, &text);
    if !server.login.as_ref().is_some_and(|l| l.needs_reauth) {
        let _ = app
            .store
            .edit_mcp(&server.id, |s| {
                s.login.get_or_insert_with(Default::default);
                oauth::set_reauth(s, true);
            })
            .await;
    }
    announce(app, &server.id);
    text
}

/// Ошибка запуска или запроса: текст без секретов; 401 у подключения с OAuth значит, что вход протух.
pub async fn note_failure(app: &Arc<App>, server: &McpServer, error: &str) -> String {
    let error = scrub(app, &server.id, error);
    let Some(meta) = &server.connector else { return custom_failure(app, server, error).await };
    app.connectors.set_error(&server.id, &error);
    if is_oauth(meta) && error.contains("HTTP 401") && !meta.needs_reauth {
        oauth::mark_reauth(app, &server.id).await;
    }
    error
}

// ---------------------------------------------------------------------------
// Как подключение видит страница
// ---------------------------------------------------------------------------

fn is_oauth(meta: &Meta) -> bool {
    meta.auth.kind == "oauth" || meta.auth.kind == "mcp-oauth"
}

fn secret_list(app: &App, server: &McpServer) -> Vec<Value> {
    let meta = server.connector.as_ref();
    let mut out: Vec<Value> = Vec::new();
    let mut seen: Vec<String> = Vec::new();
    for field in meta.map(|m| m.auth.fields.clone()).unwrap_or_default() {
        let set = app.connectors.secrets.get(&server.id, &field.name).is_some();
        out.push(json!({ "name": field.name, "set": set, "label": field.label, "help": field.help }));
        seen.push(field.name);
    }
    for name in referenced(server) {
        if seen.contains(&name) || name.starts_with(secrets::RESERVED_PREFIX) {
            continue;
        }
        let set = app.connectors.secrets.get(&server.id, &name).is_some();
        out.push(json!({ "name": name, "set": set, "label": name, "help": "" }));
    }
    out
}

/// Состояние для карточки и почему.
pub fn state_of(app: &App, server: &McpServer) -> (&'static str, String) {
    let error = app.connectors.errors.lock().unwrap().get(&server.id).cloned().unwrap_or_default();
    if let Some(meta) = &server.connector {
        if meta.consented_at == 0 {
            return ("pending-consent", String::new());
        }
        if is_oauth(meta) && (meta.needs_reauth || app.connectors.secrets.get(&server.id, oauth::ACCESS).is_none()) {
            return ("needs-auth", error);
        }
    }
    if let Some(login) = &server.login {
        if login.needs_reauth || app.connectors.secrets.get(&server.id, oauth::ACCESS).is_none() {
            return ("needs-auth", error);
        }
    }
    let missing = referenced(server).into_iter().filter(|n| !n.starts_with(secrets::RESERVED_PREFIX)).any(|n| app.connectors.secrets.get(&server.id, &n).is_none());
    let missing_field = server.connector.as_ref().is_some_and(|m| m.auth.fields.iter().any(|f| app.connectors.secrets.get(&server.id, &f.name).is_none()));
    if missing || missing_field {
        return ("needs-auth", error);
    }
    if !error.is_empty() {
        return ("error", error);
    }
    if !server.enabled {
        return ("off", String::new());
    }
    ("ok", String::new())
}

/// Забыть вход в свой сервер: токены, секрет клиента, заголовок с токеном и найденные адреса.
pub async fn forget_login(app: &Arc<App>, id: &str) {
    for name in [oauth::ACCESS, oauth::REFRESH, oauth::EXPIRES, oauth::CLIENT_SECRET] {
        let _ = app.connectors.secrets.delete(id, name);
    }
    app.connectors.oauth.challenges.lock().unwrap().remove(id);
    let _ = app
        .store
        .edit_mcp(id, |s| {
            s.login = None;
            s.headers.retain(|k, v| !(k.eq_ignore_ascii_case("authorization") && v.contains("{{secret:oauth.")));
        })
        .await;
    app.connectors.clear_error(id);
    crate::mcp::close(app, id);
}

/// Вход в свой сервер для списка серверов: состояние без токенов.
pub fn login_view(app: &App, server: &McpServer) -> Option<Value> {
    let login = server.login.as_ref()?;
    let connected = !login.needs_reauth && app.connectors.secrets.get(&server.id, oauth::ACCESS).is_some();
    let error = app.connectors.errors.lock().unwrap().get(&server.id).cloned().unwrap_or_default();
    let host = reqwest::Url::parse(&login.oauth.authorize_url).ok().and_then(|u| u.host_str().map(str::to_string)).unwrap_or_default();
    Some(json!({ "state": if connected { "connected" } else { "needs-login" }, "error": error, "authHost": host }))
}

pub fn view(app: &App, server: &McpServer) -> Value {
    let meta = server.connector.clone().unwrap_or_default();
    let (state, error) = state_of(app, server);
    let tools = crate::mcp::known_tools(app, &server.id).or_else(|| app.connectors.tools.lock().unwrap().get(&server.id).copied());
    let expires: i64 = app.connectors.secrets.get(&server.id, oauth::EXPIRES).and_then(|v| v.parse().ok()).unwrap_or(0);
    let connected = is_oauth(&meta) && !meta.needs_reauth && app.connectors.secrets.get(&server.id, oauth::ACCESS).is_some();
    json!({
        "id": server.id,
        "manifestId": meta.manifest_id,
        "title": if meta.title.is_empty() { server.name.clone() } else { meta.title.clone() },
        "summary": meta.summary,
        "icon": meta.icon,
        "version": meta.version,
        "transport": if server.is_http() { "http" } else { "stdio" },
        "state": state,
        "error": error,
        "tools": tools,
        "enabled": server.enabled,
        "autoApprove": server.auto_approve,
        "consentedAt": meta.consented_at,
        "install": {
            "kind": if meta.install.kind.is_empty() { "none" } else { meta.install.kind.as_str() },
            "package": meta.install.package,
            "pin": meta.install.pin,
            "pinned": meta.install.kind != "none" && !meta.install.kind.is_empty() || server.is_http(),
        },
        "egress": meta.egress,
        "env": server.env.keys().collect::<Vec<_>>(),
        "headers": server.headers.keys().collect::<Vec<_>>(),
        "secrets": secret_list(app, server),
        "auth": { "kind": meta.auth.kind, "provider": meta.auth.oauth.provider, "connected": connected, "expiresAt": expires },
    })
}

fn shown_command(server: &McpServer) -> String {
    std::iter::once(server.command.as_str())
        .chain(server.args.iter().map(String::as_str))
        .map(|part| if part.contains(' ') { format!("\"{part}\"") } else { part.to_string() })
        .collect::<Vec<_>>()
        .join(" ")
}

fn host_of(url: &str) -> Option<String> {
    reqwest::Url::parse(url).ok().and_then(|u| u.host_str().map(str::to_ascii_lowercase))
}

/// Куда пойдёт вход через OAuth: хосты провайдера, своих адресов или найденного сервера авторизации.
fn oauth_info(server: &McpServer, meta: &Meta) -> Value {
    if !is_oauth(meta) {
        return Value::Null;
    }
    let cfg = &meta.auth.oauth;
    let (authorize, token) = match oauth::provider(&cfg.provider) {
        Some(p) => (p.authorize_url.to_string(), p.token_url.to_string()),
        None => (cfg.authorize_url.clone(), cfg.token_url.clone()),
    };
    let mut hosts: Vec<String> = Vec::new();
    for url in [&authorize, &token, &cfg.registration_url] {
        if let Some(host) = host_of(url) {
            if !hosts.contains(&host) {
                hosts.push(host);
            }
        }
    }
    let scopes = if cfg.scopes.is_empty() { oauth::provider(&cfg.provider).map(|p| p.scopes.iter().map(|s| s.to_string()).collect()).unwrap_or_default() } else { cfg.scopes.clone() };
    json!({
        "provider": cfg.provider,
        "hosts": hosts,
        // MCP OAuth: где искать сервер авторизации (сам он станет известен при входе).
        "discovery": if meta.auth.kind == "mcp-oauth" { host_of(&server.url).unwrap_or_default() } else { String::new() },
        "scopes": scopes,
    })
}

/// Карточка согласия: что запустится, откуда, куда пойдёт и какие секреты получит.
pub fn consent(app: &App, server: &McpServer) -> Value {
    let meta = server.connector.clone().unwrap_or_default();
    let install = &meta.install;
    let package = match install.kind.as_str() {
        "npx" => json!(format!("{}@{}", install.package, install.pin)),
        "uvx" => json!(format!("{}=={}", install.package, install.pin)),
        _ => Value::Null,
    };
    let binary = (install.kind == "binary").then(|| json!({ "url": install.url, "sha256": install.sha256.to_ascii_lowercase(), "size": install.size, "file": install.file }));
    let runtime = match install.kind.as_str() {
        "npx" => {
            let system = runtime_for("npx").is_some_and(|r| r.path.is_some());
            let found = system || crate::runtime_node::installed_node_paths(app.store.data_dir()).is_some();
            Some(json!({ "name": "node", "found": found, "help": if found { "" } else { "Deloist скачает Node.js сам при подключении." } }))
        }
        _ => runtime_for(&install.kind).map(|r| json!({ "name": r.name, "found": r.path.is_some(), "help": if r.path.is_some() { "" } else { r.help } })),
    };
    let mut names: Vec<String> = meta.auth.fields.iter().map(|f| f.name.clone()).collect();
    for name in referenced(server) {
        if !names.contains(&name) {
            names.push(name);
        }
    }
    let secrets_list: Vec<Value> = names.iter().map(|name| json!({ "name": name, "set": app.connectors.secrets.get(&server.id, name).is_some() })).collect();
    let mut warnings: Vec<Value> = Vec::new();
    if install.kind == "npx" {
        warnings.push(json!({ "code": "deps-unpinned", "text": "Версия пакета закреплена, но его зависимости не закреплены: npm возьмёт их свежими." }));
    }
    if !server.is_http() && matches!(install.kind.as_str(), "" | "none") {
        warnings.push(json!({ "code": "not-pinned", "text": "Команда не закреплена: запустится то, что сейчас лежит по этому пути." }));
    }
    json!({
        "id": server.id,
        "title": if meta.title.is_empty() { server.name.clone() } else { meta.title.clone() },
        "version": meta.version,
        "transport": if server.is_http() { "http" } else { "stdio" },
        "command": if server.is_http() { String::new() } else { shown_command(server) },
        "package": package,
        "binary": binary,
        "url": if server.is_http() { crate::mcp::public_url(&server.url) } else { String::new() },
        "pinned": server.is_http() || matches!(install.kind.as_str(), "npx" | "uvx" | "binary"),
        "depsPinned": match install.kind.as_str() { "npx" => json!(false), "uvx" => json!(install.date), _ => Value::Null },
        "egress": meta.egress,
        "secrets": secrets_list,
        "env": server.env.keys().collect::<Vec<_>>(),
        // Значения env из манифеста: ссылки на секреты и обычные строки как есть — человек видит,
        // что именно получит программа.
        "envValues": server.env,
        "headers": server.headers.keys().collect::<Vec<_>>(),
        "auth": { "kind": meta.auth.kind, "provider": meta.auth.oauth.provider },
        "oauth": oauth_info(server, &meta),
        "warnings": warnings,
        "runtime": runtime,
        "autoApprove": false,
    })
}

/// Сообщить странице новое состояние подключения. Только состояние, без значений.
pub fn announce(app: &App, id: &str) {
    let event = match app.store.mcp_server(id) {
        Some(server) => {
            let (state, error) = state_of(app, &server);
            json!({ "type": "connector", "id": id, "state": state, "error": error })
        }
        None => json!({ "type": "connector", "id": id, "state": "removed", "error": "" }),
    };
    app.turns.emit(event);
}

// ---------------------------------------------------------------------------
// Действия API
// ---------------------------------------------------------------------------

fn connector(app: &App, id: &str) -> Result<McpServer, Fail> {
    app.store.mcp_server(id).filter(|s| s.connector.is_some()).ok_or_else(|| fail(404, "подключение не найдено"))
}

pub fn list(app: &App) -> Value {
    let items: Vec<Value> = app.store.mcp_servers().iter().filter(|s| s.connector.is_some()).map(|s| view(app, s)).collect();
    json!({ "connectors": items })
}

pub async fn install(app: &Arc<App>, body: &Value) -> Result<Value, Fail> {
    let manifest = body.get("manifest").ok_or_else(|| fail(400, "нужен manifest"))?;
    let mut server = from_manifest(app.store.data_dir(), manifest).map_err(|e| fail(400, e))?;
    let mut keep_secrets = false;
    if let Some(current) = app.store.mcp_server(&server.id) {
        let Some(old) = &current.connector else {
            return Err(fail(409, format!("ключ «{}» уже занят MCP-сервером, добавленным вручную", server.id)));
        };
        server.created_at = current.created_at;
        // Секреты переходят только к тому же приложению на тот же хост: чужой манифест с тем же
        // ключом не получит чужой токен.
        let same_app = old.manifest_id == server.connector.as_ref().map(|m| m.manifest_id.clone()).unwrap_or_default();
        let same_host = current.is_http() == server.is_http() && host_of(&current.url) == host_of(&server.url);
        keep_secrets = same_app && same_host;
        if keep_secrets && current.is_http() {
            if let (Some(m), true) = (server.connector.as_mut(), old.auth.kind == "mcp-oauth") {
                // Зарегистрированного у того же сервера клиента сохраняем.
                if m.auth.kind == "mcp-oauth" {
                    m.auth.oauth = old.auth.oauth.clone();
                }
            }
        }
        if old.install.kind == "binary" {
            let _ = tokio::fs::remove_dir_all(app.store.data_dir().join("engines").join(&server.id)).await;
        }
    }
    crate::mcp::close(app, &server.id);
    app.connectors.forget(&server.id);
    if !keep_secrets {
        app.connectors.secrets.delete_scope(&server.id).map_err(|e| fail(500, e))?;
    }
    app.store.put_mcp(server.clone()).await.map_err(|e| fail(500, e.to_string()))?;
    announce(app, &server.id);
    Ok(json!({ "connector": view(app, &server), "consent": consent(app, &server) }))
}

pub async fn confirm(app: &Arc<App>, id: &str, body: &Value) -> Result<Value, Fail> {
    let server = connector(app, id)?;
    let meta = server.connector.clone().unwrap();
    let mut command = server.command.clone();
    // Повторное согласие не должно дважды дописать путь к npx-cli.js.
    let mut args: Vec<String> = server.args.iter().skip_while(|a| a.to_ascii_lowercase().ends_with("npx-cli.js")).cloned().collect();
    if meta.install.kind == "npx" {
        match runtime_for("npx").and_then(|r| r.path) {
            Some(path) => {
                // Полный путь: потом чужой npx из PATH не подменит наш. npx — это .cmd, а cmd.exe толкует
                // кавычки и %…% по-своему, поэтому, где можно, запускаем node.exe с npx-cli.js напрямую.
                command = path.to_string_lossy().into_owned();
                if let Some((node, cli)) = node_and_npx_cli(&path) {
                    command = node.to_string_lossy().into_owned();
                    args.insert(0, cli.to_string_lossy().into_owned());
                }
            }
            None => {
                // Системного npx нет: Deloist ставит свой Node и запускает npx-cli.js тем же способом.
                let paths = crate::runtime_node::ensure_node(app).await.map_err(|e| {
                    Fail(
                        412,
                        format!("для этого подключения нужен Node.js: скачать его не удалось — {e}"),
                        json!({ "runtime": { "name": "node", "found": false, "help": "Deloist скачает Node.js сам при подключении." } }),
                    )
                })?;
                let npx_cli = paths.npm_cli.with_file_name("npx-cli.js");
                if !npx_cli.is_file() {
                    return Err(Fail(412, "у Node.js нет npx: не могу запустить подключение".into(), json!({ "runtime": { "name": "node", "found": false, "help": "Deloist скачает Node.js сам при подключении." } })));
                }
                command = paths.node.to_string_lossy().into_owned();
                args.insert(0, npx_cli.to_string_lossy().into_owned());
            }
        }
    } else if let Some(runtime) = runtime_for(&meta.install.kind) {
        let Some(path) = runtime.path else {
            return Err(Fail(412, runtime.help.to_string(), json!({ "runtime": { "name": runtime.name, "found": false, "help": runtime.help } })));
        };
        command = path.to_string_lossy().into_owned();
    }
    let mut exe_sha = meta.install.exe_sha256.clone();
    if meta.install.kind == "binary" {
        let (exe, sha) = install_binary(app.store.data_dir(), id, &meta.version, &meta.install).await.map_err(|e| fail(502, format!("программа не установилась: {e}")))?;
        command = exe.to_string_lossy().into_owned();
        if is_zip(&meta.install) {
            exe_sha = sha;
        }
    }
    let auto = body.get("autoApprove").and_then(Value::as_bool).unwrap_or(false);
    let updated = app
        .store
        .edit_mcp(id, |s| {
            s.command = command;
            s.args = args;
            s.enabled = true;
            s.auto_approve = auto;
            if let Some(m) = s.connector.as_mut() {
                m.consented_at = crate::store::now();
                m.install.exe_sha256 = exe_sha;
            }
        })
        .await
        .map_err(|e| fail(500, e.to_string()))?
        .ok_or_else(|| fail(404, "подключение не найдено"))?;
    crate::mcp::close(app, id);
    app.connectors.clear_error(id);
    announce(app, id);
    Ok(json!({ "connector": view(app, &updated) }))
}

/// Имена, которые можно задать подключению руками: поля входа и ссылки из env/заголовков.
pub fn settable(server: &McpServer) -> Vec<String> {
    let mut out: Vec<String> = server.connector.as_ref().map(|m| m.auth.fields.iter().map(|f| f.name.clone()).collect()).unwrap_or_default();
    for name in referenced(server) {
        if !out.contains(&name) && !name.starts_with(secrets::RESERVED_PREFIX) {
            out.push(name);
        }
    }
    out
}

pub async fn set_secret(app: &Arc<App>, id: &str, body: &Value) -> Result<Value, Fail> {
    let server = app.store.mcp_server(id).ok_or_else(|| fail(404, "подключение не найдено"))?;
    let name = body.get("name").and_then(Value::as_str).unwrap_or_default().trim().to_string();
    let value = body.get("value").and_then(Value::as_str).unwrap_or_default();
    if !secrets::valid_name(&name) {
        return Err(fail(400, "неверное имя секрета"));
    }
    if name.starts_with(secrets::RESERVED_PREFIX) {
        return Err(fail(400, "токены входа задаёт только вход через OAuth"));
    }
    if !settable(&server).contains(&name) {
        return Err(fail(400, format!("подключение не использует секрет «{name}»")));
    }
    if value.len() > 16 * 1024 || value.contains(['\r', '\n']) {
        return Err(fail(400, "значение слишком длинное или с переводом строки"));
    }
    let result = if value.is_empty() { app.connectors.secrets.delete(id, &name) } else { app.connectors.secrets.set(id, &name, value.trim()) };
    result.map_err(|e| fail(500, e))?;
    crate::mcp::close(app, id);
    app.connectors.clear_error(id);
    announce(app, id);
    Ok(json!({ "connector": view(app, &server) }))
}

pub async fn oauth_start(app: &Arc<App>, id: &str, body: &Value) -> Result<Value, Fail> {
    let server = app.store.mcp_server(id).ok_or_else(|| fail(404, "подключение не найдено"))?;
    // client id, который ввёл человек, когда у сервера авторизации нет регистрации клиента.
    let manual = body.get("clientId").and_then(Value::as_str).map(str::trim).filter(|v| !v.is_empty()).map(|client_id| oauth::Manual {
        client_id: client_id.to_string(),
        client_secret: body.get("clientSecret").and_then(Value::as_str).unwrap_or_default().to_string(),
        redirect_port: body.get("redirectPort").and_then(Value::as_u64).and_then(|p| u16::try_from(p).ok()).unwrap_or(0),
    });
    let (cfg, discovery, egress, manual) = match &server.connector {
        Some(meta) => {
            if meta.consented_at == 0 {
                return Err(fail(409, "сначала подтвердите подключение"));
            }
            let discovery = match meta.auth.kind.as_str() {
                "oauth" => None,
                "mcp-oauth" => Some(server.url.trim().to_string()),
                _ => return Err(fail(400, "у этого подключения нет входа через OAuth")),
            };
            if discovery.is_none() {
                // Пустой client id встроенного провайдера — не ошибка сети, а незаполненная настройка.
                if let Err(e) = oauth::resolve(&meta.auth.oauth) {
                    return Err(fail(412, e));
                }
            }
            let manual = if discovery.is_some() { manual } else { None };
            (meta.auth.oauth.clone(), discovery, Some(meta.egress.clone()), manual)
        }
        // Свой сервер по адресу: тот же вход, найти сервер авторизации по ответу 401 или по адресу.
        None => {
            if !server.is_http() {
                return Err(fail(400, "вход через OAuth есть только у HTTP-серверов"));
            }
            let cfg = server.login.as_ref().map(|l| l.oauth.clone()).unwrap_or_default();
            (cfg, Some(server.url.trim().to_string()), None, manual)
        }
    };
    let started = match oauth::start(app, id, cfg, discovery, egress.as_deref(), manual).await {
        Ok(started) => started,
        Err(oauth::StartError::NeedsClientId(host)) => {
            return Err(Fail(
                409,
                "сервер авторизации не умеет регистрировать клиентов: введите client ID".into(),
                json!({ "needsClientId": true, "authHost": host }),
            ))
        }
        Err(oauth::StartError::Failed(e)) => return Err(fail(502, e)),
    };
    if body.get("open").and_then(Value::as_bool).unwrap_or(true) {
        crate::desktop::open_in_browser(&started.url);
    }
    Ok(json!({ "url": started.url }))
}

pub async fn check(app: &Arc<App>, id: &str) -> Result<Value, Fail> {
    let server = connector(app, id)?;
    if server.connector.as_ref().is_some_and(|m| m.consented_at == 0) {
        return Err(fail(409, "сначала подтвердите подключение"));
    }
    match crate::mcp::check(app, id).await {
        Ok(tools) => {
            app.connectors.set_tools(id, tools.len());
            app.connectors.clear_error(id);
            announce(app, id);
            let server = app.store.mcp_server(id).unwrap_or(server);
            Ok(json!({ "tools": tools, "connector": view(app, &server) }))
        }
        Err(e) => {
            announce(app, id);
            Err(fail(502, scrub(app, id, &e)))
        }
    }
}

pub async fn remove(app: &Arc<App>, id: &str) -> Result<Value, Fail> {
    let server = connector(app, id)?;
    crate::mcp::close(app, id);
    app.connectors.forget(id);
    app.store.remove_mcp(id).await.map_err(|e| fail(500, e.to_string()))?;
    app.connectors.secrets.delete_scope(id).map_err(|e| fail(500, e))?;
    if let Some(meta) = &server.connector {
        if meta.install.kind == "binary" {
            let _ = tokio::fs::remove_dir_all(app.store.data_dir().join("engines").join(id)).await;
        }
    }
    announce(app, id);
    Ok(json!({ "ok": true }))
}

/// Для `/api/state`: env и заголовки без значений, вписанных руками.
pub fn masked_map(map: &BTreeMap<String, String>) -> BTreeMap<String, String> {
    map.iter().map(|(k, v)| (k.clone(), secrets::masked(v))).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn manifest(app: Value) -> Value {
        json!({ "schema": 1, "id": "app.demo", "kind": "app", "version": "1.0.0", "title": { "ru": "Демо" }, "app": app })
    }

    fn data() -> PathBuf {
        std::env::temp_dir().join("crew-connectors-unit")
    }

    #[test]
    fn npx_needs_an_exact_version() {
        let ok = from_manifest(&data(), &manifest(json!({ "install": { "kind": "npx", "package": "@scope/srv", "pin": "1.2.3" }, "args": ["--ro"] }))).unwrap();
        assert_eq!(ok.command, "npx");
        assert_eq!(ok.args, vec!["-y", "@scope/srv@1.2.3", "--ro"]);
        assert!(!ok.enabled && !ok.auto_approve);
        for pin in ["latest", "^1.2.3", "~1.2", "1.x", ">=1.0.0", "1.2", ""] {
            let error = from_manifest(&data(), &manifest(json!({ "install": { "kind": "npx", "package": "srv", "pin": pin } })));
            assert!(error.is_err(), "{pin} принят");
        }
        let error = from_manifest(&data(), &manifest(json!({ "install": { "kind": "npx", "package": "srv@latest", "pin": "1.0.0" } }))).unwrap_err();
        assert!(error.contains("pin"), "{error}");
        let error = from_manifest(&data(), &manifest(json!({ "install": { "kind": "npx", "package": "srv", "pin": "latest" } }))).unwrap_err();
        assert!(error.contains("@latest"), "{error}");
    }

    #[test]
    fn uvx_needs_an_exact_version() {
        let ok = from_manifest(&data(), &manifest(json!({ "install": { "kind": "uvx", "package": "mcp-server-x", "pin": "0.4.0.post1", "date": "2026-11-01" } }))).unwrap();
        assert_eq!(ok.args, vec!["--exclude-newer", "2026-11-01", "mcp-server-x==0.4.0.post1"]);
        for pin in ["latest", ">=1", "~=1.0", "1.*"] {
            assert!(from_manifest(&data(), &manifest(json!({ "install": { "kind": "uvx", "package": "x", "pin": pin, "date": "2026-11-01" } }))).is_err(), "{pin}");
        }
        let error = from_manifest(&data(), &manifest(json!({ "install": { "kind": "uvx", "package": "x", "pin": "1.0.0" } }))).unwrap_err();
        assert!(error.contains("install.date"), "без даты зависимости не закреплены: {error}");
    }

    #[test]
    fn a_binary_needs_https_size_and_sha() {
        let good = json!({ "kind": "binary", "url": "https://example.com/x.zip", "size": 10, "sha256": "ab".repeat(32), "file": "x.exe" });
        let server = from_manifest(&data(), &manifest(json!({ "install": good }))).unwrap();
        assert!(server.command.ends_with("demo\\1.0.0\\x.exe") || server.command.ends_with("demo/1.0.0/x.exe"), "{}", server.command);
        for (key, bad) in [("url", json!("http://example.com/x.zip")), ("size", json!(0)), ("sha256", json!("abc")), ("file", json!("x.bat"))] {
            let mut install = good.clone();
            install[key] = bad;
            assert!(from_manifest(&data(), &manifest(json!({ "install": install }))).is_err(), "{key}");
        }
    }

    #[test]
    fn secrets_stay_out_of_args_and_must_be_declared() {
        let error = from_manifest(&data(), &manifest(json!({ "command": "srv", "args": ["--key={{secret:token}}"], "auth": { "kind": "fields", "fields": [{ "name": "token" }] } }))).unwrap_err();
        assert!(error.contains("аргумент"), "{error}");
        let error = from_manifest(&data(), &manifest(json!({ "command": "srv", "env": { "TOKEN": "{{secret:token}}" } }))).unwrap_err();
        assert!(error.contains("token"), "{error}");
        let ok = from_manifest(&data(), &manifest(json!({ "command": "srv", "env": { "TOKEN": "{{secret:token}}" }, "auth": { "kind": "fields", "fields": [{ "name": "token", "label": { "ru": "Токен" } }] } }))).unwrap();
        assert_eq!(referenced(&ok), vec!["token"]);
        assert!(from_manifest(&data(), &manifest(json!({ "command": "srv", "env": { "PATH": "x" } }))).is_err());
        assert!(from_manifest(&data(), &manifest(json!({ "command": "srv", "env": { "1BAD": "x" } }))).is_err());
    }

    #[test]
    fn http_host_must_be_in_egress() {
        let error = from_manifest(&data(), &manifest(json!({ "transport": "http", "url": "https://evil.example.net/mcp", "egress": ["api.example.com"] }))).unwrap_err();
        assert!(error.contains("egress"), "{error}");
        assert!(from_manifest(&data(), &manifest(json!({ "transport": "http", "url": "https://mcp.example.com/mcp", "egress": ["*.example.com"] }))).is_ok());
        assert!(from_manifest(&data(), &manifest(json!({ "transport": "http", "url": "https://example.com/mcp", "egress": ["*.example.com"] }))).is_err(), "*.x не покрывает сам x");
        assert!(from_manifest(&data(), &manifest(json!({ "transport": "http", "url": "https://mcp.example.com/mcp" }))).is_err(), "без egress");
        assert!(host_allowed("API.Example.com.", &["api.example.com".into()]));
        assert!(!host_allowed("api.example.com.evil.net", &["api.example.com".into()]));
        assert!(!host_allowed("badexample.com", &["*.example.com".into()]));
    }

    #[test]
    fn oauth_over_http_gets_a_bearer_header() {
        let server = from_manifest(&data(), &manifest(json!({ "transport": "http", "url": "https://mcp.example.com/mcp", "egress": ["mcp.example.com"], "auth": { "kind": "mcp-oauth" } }))).unwrap();
        assert_eq!(server.headers.get("Authorization").map(String::as_str), Some("Bearer {{secret:oauth.access_token}}"));
        assert!(from_manifest(&data(), &manifest(json!({ "command": "srv", "auth": { "kind": "mcp-oauth" } }))).is_err());
        assert!(from_manifest(&data(), &manifest(json!({ "command": "srv", "auth": { "kind": "oauth", "oauth": { "provider": "nope" } } }))).is_err());
        assert!(from_manifest(&data(), &manifest(json!({ "command": "srv", "auth": { "kind": "oauth", "oauth": { "provider": "yandex" } } }))).is_ok());
    }

    #[tokio::test]
    async fn a_binary_with_a_wrong_sha_installs_nothing() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let body = b"MZ fake program".to_vec();
        let served = body.clone();
        tokio::spawn(async move {
            while let Ok((mut socket, _)) = listener.accept().await {
                let served = served.clone();
                tokio::spawn(async move {
                    let mut buf = [0u8; 2048];
                    let _ = socket.read(&mut buf).await;
                    let head = format!("HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", served.len());
                    let _ = socket.write_all(head.as_bytes()).await;
                    let _ = socket.write_all(&served).await;
                });
            }
        });
        let dir = std::env::temp_dir().join(format!("crew-connectors-sha-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let mut install = Install { kind: "binary".into(), url: format!("http://127.0.0.1:{port}/x.exe"), size: body.len() as u64, sha256: "00".repeat(32), file: "x.exe".into(), ..Default::default() };
        let error = install_binary(&dir, "demo", "1.0.0", &install).await.unwrap_err();
        assert!(error.contains("контрольная сумма"), "{error}");
        assert!(!engine_dir(&dir, "demo", "1.0.0").exists(), "неверный файл не должен лечь в engines");
        assert!(!dir.join("engines").join("demo").join("1.0.0.part").exists());

        use sha2::Digest;
        install.sha256 = sha2::Sha256::digest(&body).iter().map(|b| format!("{b:02x}")).collect();
        let (exe, _) = install_binary(&dir, "demo", "1.0.0", &install).await.unwrap();
        assert_eq!(std::fs::read(&exe).unwrap(), body);

        // Подменённая программа не запускается: сумма сверяется снова, файл скачивается заново.
        std::fs::write(&exe, b"MZ evil replacement").unwrap();
        let (again, _) = install_binary(&dir, "demo", "1.0.0", &install).await.unwrap();
        assert_eq!(std::fs::read(again).unwrap(), body, "подмена осталась");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn a_builtin_provider_keeps_its_own_endpoints() {
        let base = |oauth: Value| manifest(json!({ "command": "srv", "auth": { "kind": "oauth", "oauth": oauth } }));
        for bad in [
            json!({ "provider": "google", "tokenUrl": "https://evil.example/token" }),
            json!({ "provider": "google", "authorizeUrl": "https://evil.example/auth" }),
            json!({ "provider": "google", "clientId": "other" }),
            json!({ "provider": "google", "redirectPort": 5000 }),
            json!({ "provider": "google", "extra": { "prompt": "none" } }),
        ] {
            let error = from_manifest(&data(), &base(bad.clone())).unwrap_err();
            assert!(error.contains("scopes"), "{bad}: {error}");
        }
        assert!(from_manifest(&data(), &base(json!({ "provider": "google", "scopes": ["drive.readonly"] }))).is_ok());
        let custom = |extra: Value| base(json!({ "authorizeUrl": "https://a.example/auth", "tokenUrl": "https://a.example/token", "clientId": "c", "extra": extra }));
        for key in ["redirect_uri", "client_id", "code_challenge", "state", "resource", "scope", "Response_Type"] {
            assert!(from_manifest(&data(), &custom(json!({ key: "x" }))).is_err(), "{key}");
        }
        assert!(from_manifest(&data(), &custom(json!({ "prompt": "consent" }))).is_ok());
    }

    #[test]
    fn cmd_metacharacters_are_refused() {
        for arg in ["x\"&calc&\"y", "%DEMO_TOKEN%", "a^b", "a|b", "a>b", "a<b", "!x!", "a\u{0}b"] {
            assert!(from_manifest(&data(), &manifest(json!({ "command": "srv", "args": [arg] }))).is_err(), "{arg}");
        }
        assert!(from_manifest(&data(), &manifest(json!({ "command": "srv&calc" }))).is_err());
        assert!(from_manifest(&data(), &manifest(json!({ "command": "C:\\Program Files\\srv.exe", "args": ["--dir=C:\\a b"] }))).is_ok());
    }

    #[test]
    fn installer_settings_cannot_come_from_env() {
        let npx = |env: Value| manifest(json!({ "install": { "kind": "npx", "package": "srv", "pin": "1.0.0" }, "env": env }));
        for name in ["NPM_CONFIG_REGISTRY", "npm_config_registry", "NODE_OPTIONS", "UV_INDEX_URL", "PIP_INDEX_URL", "PYTHONPATH", "APPDATA", "USERPROFILE", "TEMP", "PATH"] {
            assert!(from_manifest(&data(), &npx(json!({ name: "x" }))).is_err(), "{name}");
        }
        assert!(from_manifest(&data(), &npx(json!({ "MY_SERVICE_URL": "https://x" }))).is_ok());
        // У самостоятельной программы NODE_ENV можно, а APPDATA — нет.
        assert!(from_manifest(&data(), &manifest(json!({ "command": "srv", "env": { "NODE_ENV": "production" } }))).is_ok());
        assert!(from_manifest(&data(), &manifest(json!({ "command": "srv", "env": { "APPDATA": "C:\\x" } }))).is_err());
    }

    #[test]
    fn connector_urls_are_https_and_versions_are_clean() {
        assert!(from_manifest(&data(), &manifest(json!({ "transport": "http", "url": "http://mcp.example.com/mcp", "egress": ["mcp.example.com"] }))).is_err());
        assert!(from_manifest(&data(), &manifest(json!({ "transport": "http", "url": "http://127.0.0.1:5000/mcp", "egress": ["127.0.0.1"] }))).is_ok());
        for version in ["1.0.", "1..0", ".1", "1.0-"] {
            let mut m = manifest(json!({ "command": "srv" }));
            m["version"] = json!(version);
            assert!(from_manifest(&data(), &m).is_err(), "{version}");
        }
    }
}
