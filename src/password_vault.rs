//! Менеджер паролей: Bitwarden (`bw`) и 1Password (`op`) за одним модулем.
//!
//! Наружу уходят только входы `VaultLogin`: название, логин, есть ли код и адреса. Пароль и код
//! отдаёт `secret_for`, а его не вызывает ни один адрес API. Ключ сессии Bitwarden живёт в памяти
//! и забывается после 30 минут без работы (фоновая проверка раз в минуту), по «Заблокировать» и
//! при выходе. Токен 1Password лежит в зашифрованном хранилище секретов (DPAPI), не в `secrets.json`.
//! Stderr CLI не читается: в нём может быть секрет, поэтому ошибки возвращаются фиксированными текстами.
//!
//! Затирание памяти best effort и только в ограниченных местах: обнуляется буфер вывода CLI и
//! сама строка ключа. Копии, которые делает serde при разборе, строки в `VaultLogin`, JSON-запрос к
//! браузеру со вставляемым значением и память Chrome этим не затираются и живут до освобождения.

use std::net::IpAddr;
use std::path::{Path, PathBuf};
use std::process::{ExitStatus, Stdio};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant};

use serde::Serialize;
use serde_json::Value;
use tokio::io::AsyncReadExt;
use tokio::process::{Child, Command};

use crate::App;

const CLI_TIMEOUT: Duration = Duration::from_secs(20);
const STDOUT_CAP: usize = 2 * 1024 * 1024;
const SESSION_TTL: Duration = Duration::from_secs(30 * 60);
const SWEEP_EVERY: Duration = Duration::from_secs(60);
const BW_PASS_ENV: &str = "DELOIST_BW_PASS";
const BW_SESSION_ENV: &str = "BW_SESSION";
const OP_TOKEN_ENV: &str = "OP_SERVICE_ACCOUNT_TOKEN";
/// Область и имя в зашифрованном хранилище секретов для токена 1Password.
const SECRET_SCOPE: &str = "vault";
const OP_SECRET_NAME: &str = "onepassword";
const BW_NAME: &str = if cfg!(windows) { "bw.exe" } else { "bw" };
const OP_NAME: &str = if cfg!(windows) { "op.exe" } else { "op" };
const HTTPS_PORT: u16 = 443;
/// Консольная программа без окна консоли (CREATE_NO_WINDOW).
#[cfg(windows)]
const NO_WINDOW: u32 = 0x0800_0000;

/// Вход для списка: без пароля и без кода. Только поля, которые безопасно показать.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct VaultLogin {
    /// С источником: `bw:<id элемента>` или `op:<id хранилища>/<id элемента>`.
    pub id: String,
    pub source: &'static str,
    pub title: String,
    pub username: Option<String>,
    pub has_totp: bool,
    pub urls: Vec<String>,
}

#[allow(dead_code)] // V2: инструмент входа читает секрет через secret_for
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SecretKind {
    Password,
    Totp,
}

pub struct BitwardenState {
    pub cli: bool,
    pub status: &'static str,
    pub email: String,
}

pub struct OnePasswordState {
    pub cli: bool,
    pub connected: bool,
}

/// Ключ сессии Bitwarden. При сбросе строка затирается нулями.
struct BwSession {
    key: String,
    used: Instant,
}

impl Drop for BwSession {
    fn drop(&mut self) {
        wipe_text(std::mem::take(&mut self.key));
    }
}

#[derive(Default)]
pub struct Vault {
    bitwarden: Mutex<Option<BwSession>>,
}

fn locked<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(|e| e.into_inner())
}

impl Vault {
    /// «Заблокировать»: ключ сессии забывается.
    pub fn forget_bitwarden(&self) {
        locked(&self.bitwarden).take();
    }

    fn set_bitwarden(&self, key: String) {
        *locked(&self.bitwarden) = Some(BwSession { key, used: Instant::now() });
    }

    fn bitwarden_key(&self, touch: bool) -> Option<String> {
        self.bitwarden_key_at(touch, Instant::now())
    }

    /// Живой ключ или `None`. Без работы дольше 30 минут ключ забывается; `touch` продлевает срок.
    fn bitwarden_key_at(&self, touch: bool, now: Instant) -> Option<String> {
        let mut slot = locked(&self.bitwarden);
        expire_locked(&mut slot, now);
        let session = slot.as_mut()?;
        if touch {
            session.used = now;
        }
        Some(session.key.clone())
    }

    /// Фоновая проверка: ключ, простаивавший дольше срока, забывается.
    pub fn expire_idle(&self, now: Instant) {
        expire_locked(&mut locked(&self.bitwarden), now);
    }
}

fn expire_locked(slot: &mut MutexGuard<'_, Option<BwSession>>, now: Instant) {
    let idle = slot.as_ref().is_some_and(|session| now.duration_since(session.used) >= SESSION_TTL);
    if idle {
        slot.take();
    }
}

/// Фоновая проверка простоя: раз в минуту забывает ключ Bitwarden, который не использовали 30 минут.
pub fn start(app: Arc<App>) {
    tokio::spawn(async move {
        loop {
            tokio::time::sleep(SWEEP_EVERY).await;
            app.vault.expire_idle(Instant::now());
        }
    });
}

/// Затирает буфер с секретом: байты обнуляются, `black_box` не даёт компилятору убрать запись.
fn wipe(mut bytes: Vec<u8>) {
    bytes.fill(0);
    std::hint::black_box(&bytes);
}

fn wipe_text(text: String) {
    wipe(text.into_bytes());
}

/// Ищет программу в абсолютных папках PATH. Относительные папки пропускаются; найденный файл
/// внутри папки данных (и её `workspaces`) отвергается: там пишет бот.
fn pick_program(dirs: impl IntoIterator<Item = PathBuf>, name: &str, data_dir: &Path) -> Option<PathBuf> {
    dirs.into_iter()
        .filter(|dir| dir.is_absolute())
        .map(|dir| dir.join(name))
        .find(|candidate| candidate.is_file() && !inside_data(candidate, data_dir))
}

fn inside_data(candidate: &Path, data_dir: &Path) -> bool {
    match (std::fs::canonicalize(candidate), std::fs::canonicalize(data_dir)) {
        // `workspaces` лежит внутри папки данных, поэтому отдельная проверка не нужна.
        (Ok(file), Ok(data)) => file.starts_with(&data),
        // Если проверить нельзя, программу не запускаем.
        _ => true,
    }
}

fn find_program(app: &App, override_var: &str, name: &str) -> Option<PathBuf> {
    let data_dir = app.store.data_dir();
    if cfg!(debug_assertions) {
        // Подмена для тестов работает только в отладочной сборке.
        if let Some(path) = std::env::var_os(override_var).map(PathBuf::from).filter(|p| p.is_absolute()) {
            return (path.is_file() && !inside_data(&path, data_dir)).then_some(path);
        }
    }
    let path = std::env::var_os("PATH")?;
    pick_program(std::env::split_paths(&path), name, data_dir)
}

/// Запускает CLI и возвращает код выхода и сырой stdout. Stderr не читается: в нём может быть секрет.
async fn run(who: &str, program: &Path, args: &[&str], envs: &[(&str, &str)]) -> Result<(bool, Vec<u8>), String> {
    // Менеджеры паролей работают через их командную строку (op, bw): в облаке консоли нет.
    crate::cloud::deny(crate::cloud::NO_CONSOLE)?;
    let mut command = Command::new(program);
    command
        .args(args)
        .envs(envs.iter().copied())
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .kill_on_drop(true);
    #[cfg(windows)]
    command.creation_flags(NO_WINDOW);
    let mut child = command.spawn().map_err(|_| format!("не смог запустить {who}"))?;
    match tokio::time::timeout(CLI_TIMEOUT, collect(who, &mut child)).await {
        Ok(result) => result.map(|(status, out)| (status.success(), out)),
        Err(_) => {
            let _ = child.kill().await;
            Err(format!("{who} не ответил"))
        }
    }
}

async fn collect(who: &str, child: &mut Child) -> Result<(ExitStatus, Vec<u8>), String> {
    let stdout = child.stdout.take().ok_or_else(|| format!("{who} не ответил"))?;
    let mut out = Vec::new();
    if stdout.take(STDOUT_CAP as u64 + 1).read_to_end(&mut out).await.is_err() {
        wipe(out);
        return Err(format!("{who} не ответил"));
    }
    if out.len() > STDOUT_CAP {
        wipe(out);
        let _ = child.kill().await;
        return Err(format!("{who} прислал слишком большой ответ"));
    }
    let status = child.wait().await.map_err(|_| format!("{who} не ответил"))?;
    Ok((status, out))
}

// ---------------------------------------------------------------------------
// Общие правила: адреса и идентификаторы
// ---------------------------------------------------------------------------

/// Разбор адреса для тестов. Принимает только `http://127.0.0.1:<порт>`, `http://localhost:<порт>`
/// или `http://[::1]:<порт>` без пути, и только если стоит второй маркер `CREW_TEST_MODE=1`.
fn parse_test_origin(raw: Option<&str>, marker: Option<&str>) -> Option<String> {
    if marker.map(str::trim) != Some("1") {
        return None;
    }
    let value = raw?.trim();
    let authority = value.strip_prefix("http://")?;
    let port = ["127.0.0.1:", "localhost:", "[::1]:"].iter().find_map(|host| authority.strip_prefix(host))?;
    let valid = !port.is_empty() && port.len() <= 5 && port.bytes().all(|b| b.is_ascii_digit()) && port.parse::<u32>().is_ok_and(|p| (1..=65535).contains(&p));
    valid.then(|| value.to_string())
}

/// Адрес для тестов: только в отладочной сборке, одна точная строка из `CREW_TEST_VAULT_ORIGIN`
/// при `CREW_TEST_MODE=1`. В выпуске этой функции нет, и чтения переменных тоже.
#[cfg(debug_assertions)]
fn test_origin() -> Option<String> {
    parse_test_origin(std::env::var("CREW_TEST_VAULT_ORIGIN").ok().as_deref(), std::env::var("CREW_TEST_MODE").ok().as_deref())
}

#[cfg(not(debug_assertions))]
fn test_origin() -> Option<String> {
    None
}

/// Точное совпадение с адресом теста: и страница, и сохранённый адрес равны ему целиком.
fn is_test_origin_match(test: Option<&str>, page_origin: &str, saved_url: &str) -> bool {
    test.is_some_and(|test| page_origin == test && saved_url.trim() == test)
}

/// Адрес, на котором вход вообще ищут: https, либо (только в тестах) точно адрес из `CREW_TEST_VAULT_ORIGIN`.
pub fn vault_origin_ok(origin: &str) -> bool {
    is_https_origin(origin) || test_origin().as_deref() == Some(origin)
}

/// Адрес страницы и сохранённого входа сравниваются только так (см. схему защиты).
pub fn origin_matches(page_origin: &str, saved_url: &str) -> bool {
    if is_test_origin_match(test_origin().as_deref(), page_origin, saved_url) {
        return true;
    }
    origin_matches_by_rule(page_origin, saved_url)
}

fn origin_matches_by_rule(page_origin: &str, saved_url: &str) -> bool {
    let Some(page) = page_origin.strip_prefix("https://") else {
        return false;
    };
    let Some((page_host, page_port)) = split_authority(page.split('/').next().unwrap_or_default()) else {
        return false;
    };
    let saved = saved_url.trim();
    let saved = match saved.split_once("://") {
        Some((scheme, rest)) => {
            if !scheme.eq_ignore_ascii_case("https") {
                return false;
            }
            rest
        }
        None => saved,
    };
    let Some((saved_host, saved_port)) = split_authority(saved.split(['/', '?', '#']).next().unwrap_or_default()) else {
        return false;
    };
    // Порт сравниваем всегда: отсутствующий порт означает 443 с обеих сторон.
    if saved_port.unwrap_or(HTTPS_PORT) != page_port.unwrap_or(HTTPS_PORT) {
        return false;
    }
    // Сохранённый адрес без точки («com») не сайт: так можно выбрать весь домен верхнего уровня.
    if !saved_host.contains('.') && saved_host != "localhost" && !is_ip(&saved_host) {
        return false;
    }
    if saved_host == page_host {
        return true;
    }
    // Поддомен сохранённого сайта — да, родительский домен — нет. IP совпадает только точно.
    !is_ip(&saved_host) && !is_ip(&page_host) && page_host.ends_with(&format!(".{saved_host}"))
}

/// Хост и необязательный порт из `host`, `host:port` или `[ipv6]:port`.
fn split_authority(authority: &str) -> Option<(String, Option<u16>)> {
    if authority.is_empty() || authority.contains(['@', ' ', '\\', '\t']) {
        return None;
    }
    let (host, port) = if let Some(rest) = authority.strip_prefix('[') {
        let (host, after) = rest.split_once(']')?;
        let port = match after {
            "" => None,
            _ => Some(after.strip_prefix(':')?.parse::<u16>().ok()?),
        };
        (host, port)
    } else {
        match authority.split_once(':') {
            Some((host, port)) => (host, Some(port.parse::<u16>().ok()?)),
            None => (authority, None),
        }
    };
    let host = host.trim_end_matches('.').to_ascii_lowercase();
    (!host.is_empty()).then_some((host, port))
}

fn is_ip(host: &str) -> bool {
    host.parse::<IpAddr>().is_ok()
}

fn is_https_origin(origin: &str) -> bool {
    origin.strip_prefix("https://").is_some_and(|rest| !rest.is_empty() && !rest.starts_with('/'))
}

/// Идентификатор из нашего списка: только буквы, цифры, `-` и `_`, без лидирующего `-`.
/// Так значение не похоже на флаг и не попадает в аргументы CLI как ключ.
fn safe_id(id: &str) -> bool {
    !id.is_empty()
        && !id.starts_with('-')
        && id.len() <= 128
        && id.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
}

fn op_ids(id: &str) -> Option<(&str, &str)> {
    let (vault, item) = id.strip_prefix("op:")?.split_once('/')?;
    Some((vault, item))
}

/// Оба куска `op:<хранилище>/<элемент>` проходят проверку идентификатора.
fn safe_op_ids(vault: &str, item: &str) -> bool {
    safe_id(vault) && safe_id(item)
}

fn non_empty(text: Option<&str>) -> Option<String> {
    text.filter(|t| !t.is_empty()).map(str::to_string)
}

// ---------------------------------------------------------------------------
// Bitwarden (`bw`)
// ---------------------------------------------------------------------------

pub async fn bitwarden_state(app: &App) -> BitwardenState {
    let Some(program) = find_program(app, "CREW_TEST_BW", BW_NAME) else {
        return BitwardenState { cli: false, status: "absent", email: String::new() };
    };
    let key = app.vault.bitwarden_key(false);
    let has_key = key.is_some();
    let answer = match key.as_deref() {
        Some(key) => run("Bitwarden", &program, &["status", "--nointeraction"], &[(BW_SESSION_ENV, key)]).await,
        None => run("Bitwarden", &program, &["status", "--nointeraction"], &[]).await,
    };
    if let Some(key) = key {
        wipe_text(key);
    }
    let parsed = answer.ok().filter(|(ok, _)| *ok).and_then(|(_, out)| serde_json::from_slice::<Value>(&out).ok());
    let Some(json) = parsed else {
        return BitwardenState { cli: true, status: "unauthenticated", email: String::new() };
    };
    let status = match json["status"].as_str() {
        // «Разблокирован» имеет смысл только с ключом, который держит программа.
        Some("unlocked") if has_key => "unlocked",
        Some("unlocked") | Some("locked") => {
            // CLI говорит «заблокирован»: ключ, который мы держим, больше не годится.
            if has_key {
                app.vault.forget_bitwarden();
            }
            "locked"
        }
        _ => "unauthenticated",
    };
    BitwardenState { cli: true, status, email: json["userEmail"].as_str().unwrap_or_default().to_string() }
}

/// Мастер-пароль уходит в `bw` через переменную окружения. Строку затираем после вызова.
pub async fn bitwarden_unlock(app: &App, password: String) -> Result<&'static str, String> {
    let answer = match find_program(app, "CREW_TEST_BW", BW_NAME) {
        Some(program) => {
            let args = ["unlock", "--passwordenv", BW_PASS_ENV, "--raw", "--nointeraction"];
            run("Bitwarden", &program, &args, &[(BW_PASS_ENV, password.as_str())]).await
        }
        None => Err("Bitwarden CLI не найден: установите bw и выполните bw login".to_string()),
    };
    wipe_text(password);
    match answer {
        Ok((true, out)) => {
            let key = String::from_utf8_lossy(&out).trim().to_string();
            wipe(out);
            if key.is_empty() {
                return Err("Bitwarden не принял пароль".to_string());
            }
            app.vault.set_bitwarden(key);
            Ok("unlocked")
        }
        Ok((false, out)) => {
            wipe(out);
            Err("Bitwarden не принял пароль".to_string())
        }
        Err(e) => Err(e),
    }
}

pub fn bitwarden_lock(app: &App) {
    app.vault.forget_bitwarden();
}

async fn bitwarden_logins(app: &App, origin: &str) -> Result<Vec<VaultLogin>, String> {
    let Some(key) = app.vault.bitwarden_key(true) else {
        return Ok(Vec::new());
    };
    let Some(program) = find_program(app, "CREW_TEST_BW", BW_NAME) else {
        wipe_text(key);
        return Ok(Vec::new());
    };
    let answer = run(
        "Bitwarden",
        &program,
        &["list", "items", "--url", origin, "--nointeraction"],
        &[(BW_SESSION_ENV, key.as_str())],
    )
    .await;
    wipe_text(key);
    let (ok, out) = answer?;
    if !ok {
        wipe(out);
        return Err("Bitwarden не отдал список входов".to_string());
    }
    // Список содержит пароли: разбираем из буфера и затираем его сразу.
    let parsed = parse_bitwarden_items(&out);
    wipe(out);
    let mut logins = parsed?;
    logins.retain(|login| login.urls.iter().any(|url| origin_matches(origin, url)));
    Ok(logins)
}

/// Разбор списка `bw list items`. Пароль в структуру не попадает.
fn parse_bitwarden_items(bytes: &[u8]) -> Result<Vec<VaultLogin>, String> {
    let items: Value = serde_json::from_slice(bytes).map_err(|_| "Bitwarden прислал не то".to_string())?;
    Ok(items.as_array().into_iter().flatten().filter_map(bitwarden_login).collect())
}

fn bitwarden_login(item: &Value) -> Option<VaultLogin> {
    if item["type"].as_i64() != Some(1) {
        return None;
    }
    let id = item["id"].as_str().filter(|id| safe_id(id))?;
    let login = &item["login"];
    Some(VaultLogin {
        id: format!("bw:{id}"),
        source: "bitwarden",
        title: item["name"].as_str().unwrap_or_default().to_string(),
        username: non_empty(login["username"].as_str()),
        has_totp: login["totp"].as_str().is_some_and(|totp| !totp.is_empty()),
        urls: login["uris"].as_array().into_iter().flatten().filter_map(|uri| uri["uri"].as_str()).map(str::to_string).collect(),
    })
}

async fn bitwarden_secret(app: &App, item_id: &str, origin: &str, kind: SecretKind) -> Result<Option<String>, String> {
    let Some(key) = app.vault.bitwarden_key(true) else {
        return Err("Bitwarden заблокирован".to_string());
    };
    let result = match find_program(app, "CREW_TEST_BW", BW_NAME) {
        Some(program) => bitwarden_item_secret(&program, &key, item_id, origin, kind).await,
        None => Err("Bitwarden CLI не найден".to_string()),
    };
    wipe_text(key);
    result
}

/// Пароль или код одного элемента Bitwarden. Идентификатор и адрес проверяются здесь.
pub(crate) async fn bitwarden_item_secret(program: &Path, key: &str, item_id: &str, origin: &str, kind: SecretKind) -> Result<Option<String>, String> {
    if !safe_id(item_id) {
        return Ok(None);
    }
    let env = [(BW_SESSION_ENV, key)];
    let (ok, out) = run("Bitwarden", program, &["get", "item", item_id, "--nointeraction"], &env).await?;
    if !ok {
        wipe(out);
        return Ok(None);
    }
    let item: Value = match serde_json::from_slice(&out) {
        Ok(item) => item,
        Err(_) => {
            wipe(out);
            return Err("Bitwarden прислал не то".to_string());
        }
    };
    wipe(out);
    // Адрес проверяем заново: ответ CLI не считаем надёжным фильтром.
    let allowed = item["login"]["uris"].as_array().into_iter().flatten().filter_map(|uri| uri["uri"].as_str()).any(|uri| origin_matches(origin, uri));
    if !allowed {
        return Ok(None);
    }
    match kind {
        SecretKind::Password => Ok(item["login"]["password"].as_str().filter(|p| !p.is_empty()).map(str::to_string)),
        SecretKind::Totp => {
            let (ok, out) = run("Bitwarden", program, &["get", "totp", item_id, "--nointeraction"], &env).await?;
            let code = String::from_utf8_lossy(&out).trim().to_string();
            wipe(out);
            Ok(ok.then_some(code).filter(|code| !code.is_empty()))
        }
    }
}

// ---------------------------------------------------------------------------
// 1Password (`op`)
// ---------------------------------------------------------------------------

/// Токен лежит в зашифрованном хранилище секретов, не в `secrets.json`.
fn onepassword_token(app: &App) -> Option<String> {
    app.connectors.secrets.get(SECRET_SCOPE, OP_SECRET_NAME).filter(|token| !token.is_empty())
}

pub fn onepassword_state(app: &App) -> OnePasswordState {
    OnePasswordState {
        cli: find_program(app, "CREW_TEST_OP", OP_NAME).is_some(),
        connected: onepassword_token(app).is_some(),
    }
}

/// Проверяет токен через `op whoami` и только потом сохраняет его.
pub async fn onepassword_connect(app: &App, token: &str) -> Result<(), String> {
    let program = find_program(app, "CREW_TEST_OP", OP_NAME).ok_or_else(|| "1Password CLI (op) не найден: установите op".to_string())?;
    let (ok, out) = run("1Password", &program, &["whoami", "--format", "json"], &[(OP_TOKEN_ENV, token)]).await?;
    wipe(out);
    if !ok {
        return Err("1Password отклонил токен".to_string());
    }
    app.connectors
        .secrets
        .set(SECRET_SCOPE, OP_SECRET_NAME, token)
        .map_err(|_| "не смог сохранить токен 1Password".to_string())
}

pub fn onepassword_forget(app: &App) -> Result<(), String> {
    app.connectors.secrets.delete(SECRET_SCOPE, OP_SECRET_NAME)
}

async fn onepassword_logins(app: &App, origin: &str) -> Result<Vec<VaultLogin>, String> {
    let Some(token) = onepassword_token(app) else {
        return Ok(Vec::new());
    };
    let Some(program) = find_program(app, "CREW_TEST_OP", OP_NAME) else {
        return Ok(Vec::new());
    };
    let env = [(OP_TOKEN_ENV, token.as_str())];
    let (ok, out) = run("1Password", &program, &["item", "list", "--categories", "Login", "--format", "json"], &env).await?;
    if !ok {
        wipe(out);
        return Err("1Password не отдал список входов".to_string());
    }
    let parsed = parse_onepassword_list(&out);
    wipe(out);
    let mut logins = parsed?;
    logins.retain(|login| login.urls.iter().any(|url| origin_matches(origin, url)));
    // Код в списке не виден: подробности берём только для входов на этом адресе.
    for login in logins.iter_mut() {
        let Some((vault, item)) = op_ids(&login.id).map(|(v, i)| (v.to_string(), i.to_string())) else {
            continue;
        };
        let (ok, detail) = run("1Password", &program, &["item", "get", item.as_str(), "--vault", vault.as_str(), "--format", "json"], &env).await?;
        login.has_totp = ok && has_otp_field(&detail);
        wipe(detail);
    }
    Ok(logins)
}

/// Разбор `op item list`. Логин в списке — `additional_information`, пароля в ответе нет.
fn parse_onepassword_list(bytes: &[u8]) -> Result<Vec<VaultLogin>, String> {
    let items: Value = serde_json::from_slice(bytes).map_err(|_| "1Password прислал не то".to_string())?;
    Ok(items.as_array().into_iter().flatten().filter_map(onepassword_login).collect())
}

fn onepassword_login(item: &Value) -> Option<VaultLogin> {
    let id = item["id"].as_str().filter(|id| safe_id(id))?;
    let vault = item["vault"]["id"].as_str().filter(|vault| safe_id(vault))?;
    Some(VaultLogin {
        id: format!("op:{vault}/{id}"),
        source: "1password",
        title: item["title"].as_str().unwrap_or_default().to_string(),
        username: non_empty(item["additional_information"].as_str()),
        has_totp: false,
        urls: item["urls"].as_array().into_iter().flatten().filter_map(|url| url["href"].as_str()).map(str::to_string).collect(),
    })
}

fn has_otp_field(bytes: &[u8]) -> bool {
    serde_json::from_slice::<Value>(bytes)
        .ok()
        .is_some_and(|item| item["fields"].as_array().into_iter().flatten().any(|field| field["type"] == "OTP"))
}

async fn onepassword_secret(app: &App, vault: &str, item: &str, origin: &str, kind: SecretKind) -> Result<Option<String>, String> {
    if !safe_op_ids(vault, item) {
        return Ok(None);
    }
    let Some(token) = onepassword_token(app) else {
        return Err("1Password не подключён".to_string());
    };
    let Some(program) = find_program(app, "CREW_TEST_OP", OP_NAME) else {
        return Err("1Password CLI (op) не найден".to_string());
    };
    let env = [(OP_TOKEN_ENV, token.as_str())];
    let (ok, out) = run("1Password", &program, &["item", "get", item, "--vault", vault, "--format", "json"], &env).await?;
    if !ok {
        wipe(out);
        return Ok(None);
    }
    let detail: Value = match serde_json::from_slice(&out) {
        Ok(detail) => detail,
        Err(_) => {
            wipe(out);
            return Err("1Password прислал не то".to_string());
        }
    };
    wipe(out);
    let allowed = detail["urls"].as_array().into_iter().flatten().filter_map(|url| url["href"].as_str()).any(|url| origin_matches(origin, url));
    if !allowed {
        return Ok(None);
    }
    let args: &[&str] = match kind {
        SecretKind::Password => &["item", "get", item, "--vault", vault, "--fields", "label=password", "--reveal"],
        SecretKind::Totp => &["item", "get", item, "--vault", vault, "--otp"],
    };
    let (ok, out) = run("1Password", &program, args, &env).await?;
    let value = String::from_utf8_lossy(&out).trim_end_matches(['\r', '\n']).to_string();
    wipe(out);
    Ok(ok.then_some(value).filter(|value| !value.is_empty()))
}

// ---------------------------------------------------------------------------
// Общие входы
// ---------------------------------------------------------------------------

/// Входы для страницы с адресом `origin` (https). Пароля и кода в ответе нет.
pub async fn logins_for(app: &App, origin: &str) -> Result<Vec<VaultLogin>, String> {
    if !vault_origin_ok(origin) {
        return Err("вход ищется только для сайта https".to_string());
    }
    let mut logins = bitwarden_logins(app, origin).await?;
    logins.extend(onepassword_logins(app, origin).await?);
    Ok(logins)
}

/// Есть ли подключённый и разблокированный источник. Пустой список входов бывает и из-за состояния
/// (заблокирован, не подключён), и из-за сайта: инструмент входа объясняет разницу.
pub async fn any_source_ready(app: &App) -> bool {
    let op = onepassword_state(app);
    bitwarden_state(app).await.status == "unlocked" || (op.connected && op.cli)
}

/// Пароль или код одного входа. Адрес проверяется здесь ещё раз. В адресах API этой функции нет.
#[allow(dead_code)] // V2: инструмент входа
pub(crate) async fn secret_for(app: &App, id: &str, origin: &str, kind: SecretKind) -> Result<Option<String>, String> {
    if !vault_origin_ok(origin) {
        return Err("вход можно взять только на сайте https".to_string());
    }
    if let Some(item) = id.strip_prefix("bw:") {
        return bitwarden_secret(app, item, origin, kind).await;
    }
    if let Some((vault, item)) = op_ids(id) {
        return onepassword_secret(app, vault, item, origin, kind).await;
    }
    Err("такого входа нет".to_string())
}

#[cfg(test)]
mod origin_override_tests {
    use super::*;

    #[test]
    fn test_origin_is_one_exact_string_and_nothing_else() {
        let test = Some("http://127.0.0.1:54321");
        assert!(is_test_origin_match(test, "http://127.0.0.1:54321", "http://127.0.0.1:54321"));
        assert!(!is_test_origin_match(None, "http://127.0.0.1:54321", "http://127.0.0.1:54321"), "без переменной нет исключения");
        assert!(!is_test_origin_match(test, "http://127.0.0.1:54321", "http://127.0.0.1:54322"), "другой порт");
        assert!(!is_test_origin_match(test, "http://127.0.0.1:54321", "http://127.0.0.1:54321/login"), "путь в адресе");
        assert!(!is_test_origin_match(test, "http://127.0.0.1:54321", "example.com"), "чужой сайт");
    }

    #[test]
    fn test_origin_accepts_only_loopback_with_the_marker() {
        let ok = |raw: &str| parse_test_origin(Some(raw), Some("1"));
        assert_eq!(ok("http://127.0.0.1:54321").as_deref(), Some("http://127.0.0.1:54321"));
        assert_eq!(ok("http://localhost:8080").as_deref(), Some("http://localhost:8080"));
        assert_eq!(ok("http://[::1]:9").as_deref(), Some("http://[::1]:9"));
        for bad in [
            "http://example.com:80", "http://192.168.0.1:80", "http://127.0.0.1", "http://127.0.0.1:", "http://127.0.0.1:80/login",
            "http://127.0.0.1:80/", "https://127.0.0.1:80", "http://127.0.0.1:99999", "http://127.0.0.1:0", "http://127.0.0.1.evil.com:80",
            "http://localhost.evil.com:80", "http://user@127.0.0.1:80", "http://127.0.0.1:80@evil.com", "http://127.0.0.1:8a", "",
        ] {
            assert!(ok(bad).is_none(), "{bad} не должен проходить");
        }
        assert!(parse_test_origin(Some("http://127.0.0.1:54321"), None).is_none(), "без маркера отказ");
        assert!(parse_test_origin(Some("http://127.0.0.1:54321"), Some("0")).is_none(), "маркер должен быть 1");
        assert!(parse_test_origin(None, Some("1")).is_none());
    }

    #[test]
    fn plain_http_is_refused_without_the_test_variable() {
        // Переменная в этом процессе не выставлена: обычные правила, http не проходит.
        assert!(!origin_matches("http://127.0.0.1:54321", "http://127.0.0.1:54321"));
        assert!(!vault_origin_ok("http://127.0.0.1:54321"));
        assert!(vault_origin_ok("https://login.example.com"));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn temp_root(name: &str) -> PathBuf {
        let stamp = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos();
        let dir = std::env::temp_dir().join(format!("deloist-vault-{name}-{stamp}"));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// Путь к заглушке из examples: тестовый бинарник лежит в target/debug/deps.
    fn fake_cli(name: &str) -> PathBuf {
        let exe = std::env::current_exe().unwrap();
        let debug = exe.parent().unwrap().parent().unwrap();
        debug.join("examples").join(format!("{name}{}", std::env::consts::EXE_SUFFIX))
    }

    #[test]
    fn origin_rule_table() {
        let cases = [
            ("https://example.com", "https://example.com", true),
            ("https://login.example.com", "example.com", true),
            ("https://example.com", "login.example.com", false),
            ("https://evil-example.com", "example.com", false),
            ("http://example.com", "example.com", false),
            ("https://example.com", "http://example.com", false),
            ("https://example.com", "https://example.com:8443", false),
            ("https://example.com:8443", "https://example.com:8443", true),
            ("https://example.com:8443", "example.com", false),
            ("https://example.com", "https://example.com:443", true),
            ("https://example.com", "example.com", true),
            ("https://192.168.1.10", "1.10", false),
            ("https://example.com", "user@example.com", false),
            ("https://a.com", "com", false),
            ("https://localhost", "localhost", true),
        ];
        for (page, saved, expected) in cases {
            assert_eq!(origin_matches(page, saved), expected, "{page} / {saved}");
        }
    }

    #[test]
    fn ids_never_look_like_flags() {
        assert!(!safe_id("--help"));
        assert!(!safe_id("-x"));
        assert!(!safe_id("a b"));
        assert!(safe_id("bw-good_1"));
        let (vault, item) = op_ids("op:v/--x").unwrap();
        assert!(!safe_op_ids(vault, item));
        assert!(!safe_op_ids("-v", "item"));
        assert!(safe_op_ids("vlt1", "op1"));
    }

    #[test]
    fn program_is_taken_only_from_absolute_dirs_outside_data() {
        let root = temp_root("pick");
        let data = root.join("data");
        let inside = data.join("workspaces").join("bot");
        let bin = root.join("bin");
        std::fs::create_dir_all(&inside).unwrap();
        std::fs::create_dir_all(&bin).unwrap();
        let name = "bwfake-pick.exe";
        std::fs::write(inside.join(name), b"x").unwrap();
        std::fs::write(bin.join(name), b"x").unwrap();
        // Относительная папка и папка внутри данных пропускаются, нужная остаётся.
        let picked = pick_program(vec![PathBuf::from("rel-dir"), inside.clone(), bin.clone()], name, &data);
        assert_eq!(picked, Some(bin.join(name)));
        // Если единственная копия внутри данных, ничего не находим.
        assert_eq!(pick_program(vec![inside.clone()], name, &data), None);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn bitwarden_list_keeps_logins_and_drops_password() {
        let text = r#"[{"id":"a1","type":1,"name":"Example","login":{"username":"anna","password":"pw-secret-9","totp":"JBSW","uris":[{"uri":"https://example.com/login"}]}},{"id":"n1","type":2,"name":"Note"}]"#;
        let logins = parse_bitwarden_items(text.as_bytes()).unwrap();
        assert_eq!(logins.len(), 1);
        assert_eq!(logins[0].id, "bw:a1");
        assert!(logins[0].has_totp);
        assert_eq!(logins[0].username.as_deref(), Some("anna"));
        assert_eq!(logins[0].urls, vec!["https://example.com/login".to_string()]);
        assert!(!serde_json::to_string(&logins).unwrap().contains("pw-secret-9"));
    }

    #[test]
    fn onepassword_list_parses_login_without_totp_flag() {
        let text = r#"[{"id":"op1","title":"Shop","vault":{"id":"vlt1"},"urls":[{"href":"https://shop.example.com"}],"additional_information":"anna@example.com"}]"#;
        let logins = parse_onepassword_list(text.as_bytes()).unwrap();
        assert_eq!(logins.len(), 1);
        assert_eq!(logins[0].id, "op:vlt1/op1");
        assert_eq!(logins[0].source, "1password");
        assert!(!logins[0].has_totp);
        assert_eq!(logins[0].username.as_deref(), Some("anna@example.com"));
    }

    #[test]
    fn bitwarden_session_is_forgotten_after_thirty_idle_minutes() {
        let vault = Vault::default();
        let start = Instant::now();
        vault.set_bitwarden("k-test".to_string());
        assert!(vault.bitwarden_key_at(true, start + Duration::from_secs(20 * 60)).is_some());
        assert!(vault.bitwarden_key_at(false, start + Duration::from_secs(45 * 60)).is_some());
        assert!(vault.bitwarden_key_at(false, start + Duration::from_secs(51 * 60)).is_none());
        // Фоновая проверка тоже забывает простаивающий ключ.
        vault.set_bitwarden("k-test-2".to_string());
        vault.expire_idle(Instant::now() + Duration::from_secs(31 * 60));
        assert!(vault.bitwarden_key(false).is_none());
    }

    /// Секрет через заглушку `bw`: правильный адрес отдаёт пароль, чужой и «флаговый» идентификатор нет.
    #[cfg(windows)]
    #[tokio::test]
    async fn secret_lookup_goes_through_the_fake_cli() {
        let root = temp_root("secret");
        let fixture = root.join("fixture.json");
        let log = root.join("argv.log");
        let fixture_json = json!({
            "bitwarden": { "email": "u@example.com", "items": [
                { "id": "bw-good", "type": 1, "name": "Example", "login": {
                    "username": "anna", "password": "pw-unit-4410", "totp": "246810",
                    "uris": [{ "uri": "example.com" }] } }
            ] }
        });
        std::fs::write(&fixture, fixture_json.to_string()).unwrap();
        std::env::set_var("FAKE_VAULT_FIXTURE", &fixture);
        std::env::set_var("FAKE_VAULT_LOG", &log);
        let bw = fake_cli("fake_bw");
        let session = "fake-session-123";

        let password = bitwarden_item_secret(&bw, session, "bw-good", "https://login.example.com", SecretKind::Password).await;
        assert_eq!(password, Ok(Some("pw-unit-4410".to_string())));
        let code = bitwarden_item_secret(&bw, session, "bw-good", "https://login.example.com", SecretKind::Totp).await;
        assert_eq!(code, Ok(Some("246810".to_string())));
        let evil = bitwarden_item_secret(&bw, session, "bw-good", "https://evil-example.com", SecretKind::Password).await;
        assert_eq!(evil, Ok(None));
        let spaced = bitwarden_item_secret(&bw, session, "bw-good x", "https://example.com", SecretKind::Password).await;
        assert_eq!(spaced, Ok(None));
        let flag = bitwarden_item_secret(&bw, session, "--help", "https://example.com", SecretKind::Password).await;
        assert_eq!(flag, Ok(None));

        let argv = std::fs::read_to_string(&log).unwrap_or_default();
        assert!(argv.contains("get item bw-good"), "заглушка не вызвана: {argv}");
        assert!(!argv.contains("--help") && !argv.contains("bw-good x"), "флаг или пробел ушли в CLI: {argv}");
        assert!(!argv.contains(session) && !argv.contains("pw-unit-4410"), "секрет в аргументах: {argv}");
        assert!(argv.lines().all(|line| line.contains("--nointeraction")), "нет --nointeraction: {argv}");
        let _ = std::fs::remove_dir_all(&root);
    }
}
