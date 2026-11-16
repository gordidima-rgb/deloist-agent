//! MCP-клиент: свои инструменты из внешних серверов без кода.
//!
//! Транспорта два. Пустой `url` — сервер запускается по stdio и живёт, пока им пользуются.
//! Непустой `url` — MCP Streamable HTTP (спецификация 2025-03-26): POST с JSON-RPC,
//! ответ приходит телом JSON или потоком `text/event-stream`. Каждый инструмент
//! показывается модели как `mcp__<сервер>__<инструмент>`. По умолчанию такой вызов
//! спрашивает человека: чужие инструменты могут делать что угодно.

use std::collections::{BTreeMap, HashMap};
use std::process::Stdio;
use std::sync::atomic::{AtomicBool, AtomicI64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use futures_util::StreamExt;
use reqwest::header::{ACCEPT, CONTENT_TYPE};
use reqwest::{Response, StatusCode};
use serde_json::{json, Value};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, ChildStdin, Command};
use tokio::sync::{oneshot, Mutex as AsyncMutex};

use crate::store::McpServer;
use crate::App;

const TIMEOUT: Duration = Duration::from_secs(30);
const MAX_RESULT: usize = 12_000;
/// Больше этого ответа не читаем: поток «в бесконечность» — уже не ответ.
const MAX_HTTP_BODY: usize = 1024 * 1024;
/// Версия протокола для HTTP. stdio остаётся на старой: живые серверы к ней привыкли.
const HTTP_PROTOCOL: &str = "2025-03-26";
const SESSION_HEADER: &str = "mcp-session-id";
/// После неудачного запуска сервер не трогаем это время: иначе он тормозил бы каждый ход каждого бота.
const RETRY_AFTER: Duration = Duration::from_secs(300);
/// Предел длины имени инструмента у моделей.
const MAX_TOOL_NAME: usize = 64;
/// Описание инструмента уходит модели в каждом запросе: длиннее этого оно только съедает контекст.
const MAX_DESCRIPTION: usize = 1024;

/// Таймаут запроса. В отладочной сборке тесты могут его укоротить.
fn timeout() -> Duration {
    if cfg!(debug_assertions) {
        if let Some(ms) = std::env::var("CREW_TEST_MCP_TIMEOUT_MS").ok().and_then(|v| v.parse().ok()) {
            return Duration::from_millis(ms);
        }
    }
    TIMEOUT
}

fn timeout_text() -> String {
    let value = timeout();
    if value.subsec_millis() == 0 {
        format!("{} с", value.as_secs())
    } else {
        format!("{} мс", value.as_millis())
    }
}

#[derive(Default)]
pub struct Mcp {
    sessions: Mutex<HashMap<String, Arc<Session>>>,
    failed: Mutex<HashMap<String, Instant>>,
    /// Запуск по одному: два хода не должны поднять два процесса одного сервера.
    starting: AsyncMutex<()>,
}

/// Закрыть сервер: его настройки изменились или он больше не нужен.
pub fn close(app: &App, id: &str) {
    app.mcp.failed.lock().unwrap().remove(id);
    let session = app.mcp.sessions.lock().unwrap().remove(id);
    if let Some(session) = session {
        session.kill();
    }
}

/// Адрес для показа: без логина, пароля и параметров, где обычно и лежит ключ.
pub fn public_url(url: &str) -> String {
    match reqwest::Url::parse(url.trim()) {
        Ok(mut parsed) => {
            let _ = parsed.set_username("");
            let _ = parsed.set_password(None);
            parsed.set_query(None);
            parsed.set_fragment(None);
            parsed.to_string()
        }
        Err(_) => String::new(),
    }
}

/// Имя сервера по умолчанию: хост, а не весь адрес. Имя видят модель и журнал.
pub fn default_name(url: &str) -> String {
    reqwest::Url::parse(url.trim())
        .ok()
        .and_then(|parsed| parsed.host_str().map(str::to_string))
        .unwrap_or_else(|| "MCP".to_string())
}

/// Живая сессия MCP-сервера: процесс по stdio или HTTP-подключение.
enum Session {
    Stdio(Arc<StdioSession>),
    Http(Arc<HttpSession>),
}

impl Session {
    fn alive(&self) -> bool {
        match self {
            Session::Stdio(session) => session.alive(),
            // HTTP-сессия не держит соединение: смерть сервера видна на первом запросе.
            Session::Http(_) => true,
        }
    }

    /// Та же ли это настройка: после правки команды или адреса сессию берём заново.
    fn matches(&self, cfg: &McpServer) -> bool {
        match self {
            Session::Stdio(session) => session.command == cfg.command && session.args == cfg.args && session.env == cfg.env,
            Session::Http(session) => session.url == cfg.url.trim() && session.header_templates == cfg.headers,
        }
    }

    async fn request(&self, method: &str, params: Value) -> Result<Value, String> {
        match self {
            Session::Stdio(session) => session.request(method, params).await,
            Session::Http(session) => session.request(method, params).await,
        }
    }

    async fn tools(&self) -> Vec<Value> {
        match self {
            Session::Stdio(session) => session.tools().await,
            Session::Http(session) => session.tools().await,
        }
    }

    fn set_tools(&self, list: Vec<Value>) {
        match self {
            Session::Stdio(session) => *session.tools.lock().unwrap() = list,
            Session::Http(session) => *session.tools.lock().unwrap() = list,
        }
    }

    fn forget_tools(&self) {
        self.set_tools(Vec::new());
    }

    /// Погасить процесс или закрыть HTTP-сессию: спецификация просит сказать серверу DELETE.
    fn kill(&self) {
        match self {
            Session::Stdio(session) => {
                let _ = session.child.lock().unwrap().start_kill();
            }
            Session::Http(session) => {
                if let Ok(runtime) = tokio::runtime::Handle::try_current() {
                    let session = session.clone();
                    runtime.spawn(async move { session.close().await });
                }
            }
        }
    }
}

struct StdioSession {
    child: Mutex<Child>,
    stdin: AsyncMutex<ChildStdin>,
    pending: Mutex<HashMap<i64, oneshot::Sender<Value>>>,
    next_id: AtomicI64,
    tools: Mutex<Vec<Value>>,
    /// Имя и команда запуска: по ним видно, что настройки сервера поменяли.
    name: String,
    command: String,
    args: Vec<String>,
    /// Шаблоны окружения, не значения: по ним видно, что настройку поменяли.
    env: BTreeMap<String, String>,
}

impl StdioSession {
    fn alive(&self) -> bool {
        matches!(self.child.lock().unwrap().try_wait(), Ok(None))
    }

    async fn request(&self, method: &str, params: Value) -> Result<Value, String> {
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let (tx, rx) = oneshot::channel();
        self.pending.lock().unwrap().insert(id, tx);
        let line = json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params }).to_string();
        {
            let mut stdin = self.stdin.lock().await;
            stdin.write_all(line.as_bytes()).await.map_err(|e| format!("MCP: {e}"))?;
            stdin.write_all(b"\n").await.map_err(|e| format!("MCP: {e}"))?;
            stdin.flush().await.map_err(|e| format!("MCP: {e}"))?;
        }
        let answer = match tokio::time::timeout(timeout(), rx).await {
            Ok(Ok(value)) => value,
            Ok(Err(_)) => return Err("MCP-сервер закрыл соединение".into()),
            Err(_) => {
                self.pending.lock().unwrap().remove(&id);
                return Err(format!("MCP-сервер не ответил за {}", timeout_text()));
            }
        };
        if let Some(message) = error_text(&answer) {
            return Err(format!("MCP: {message}"));
        }
        Ok(answer.get("result").cloned().unwrap_or(Value::Null))
    }

    async fn notify(&self, method: &str, params: Value) -> Result<(), String> {
        let line = json!({ "jsonrpc": "2.0", "method": method, "params": params }).to_string();
        let mut stdin = self.stdin.lock().await;
        stdin.write_all(line.as_bytes()).await.map_err(|e| format!("MCP: {e}"))?;
        stdin.write_all(b"\n").await.map_err(|e| format!("MCP: {e}"))?;
        stdin.flush().await.map_err(|e| format!("MCP: {e}"))
    }

    async fn tools(&self) -> Vec<Value> {
        let cached = self.tools.lock().unwrap().clone();
        if !cached.is_empty() {
            return cached;
        }
        match self.request("tools/list", json!({})).await {
            Ok(result) => {
                let list = result["tools"].as_array().cloned().unwrap_or_default();
                *self.tools.lock().unwrap() = list.clone();
                list
            }
            Err(e) => {
                eprintln!("[mcp] «{}»: {e}", self.name);
                Vec::new()
            }
        }
    }
}

/// HTTP-сессия MCP Streamable HTTP: один POST на запрос, `Mcp-Session-Id` из ответа.
struct HttpSession {
    url: String,
    /// Только для сообщений: URL не печатаем целиком, в нём может быть ключ.
    label: String,
    client: reqwest::Client,
    session_id: AsyncMutex<Option<String>>,
    /// Инициализацию ведём по очереди: два запроса не должны открывать две сессии.
    handshake: AsyncMutex<()>,
    /// Сессия открыта. Отдельный флаг нужен серверам без `Mcp-Session-Id`.
    ready: AtomicBool,
    next_id: AtomicI64,
    tools: Mutex<Vec<Value>>,
    /// Заголовки с подставленными секретами. Наружу не показываются.
    headers: Vec<(String, String)>,
    header_templates: BTreeMap<String, String>,
}

struct Reply {
    message: Value,
    session_id: Option<String>,
}

enum Failure {
    /// Сессия протухла: можно один раз переинициализироваться и повторить.
    Session(String),
    Fatal(String),
}

fn failure_text(failure: Failure) -> String {
    match failure {
        Failure::Session(message) | Failure::Fatal(message) => message,
    }
}

fn error_text(message: &Value) -> Option<String> {
    let error = message.get("error")?;
    Some(error["message"].as_str().unwrap_or("ошибка сервера").to_string())
}

/// Плоский короткий текст для сообщения об ошибке: без переводов строк и гигантских тел.
fn clean_hint(text: &str) -> String {
    let flat = text.split_whitespace().collect::<Vec<_>>().join(" ");
    let short: String = flat.chars().take(200).collect();
    if flat.chars().count() > 200 {
        format!("{short}…")
    } else {
        short
    }
}

fn with_hint(message: String, hint: &str) -> String {
    if hint.is_empty() {
        message
    } else {
        format!("{message}: {hint}")
    }
}

/// Адрес MCP-сервера: только http/https с хостом и без пробелов.
pub fn valid_http_url(url: &str) -> bool {
    let trimmed = url.trim();
    if trimmed.chars().any(|c| c.is_whitespace() || c.is_control()) {
        return false;
    }
    match reqwest::Url::parse(trimmed) {
        Ok(parsed) => matches!(parsed.scheme(), "http" | "https") && parsed.host_str().is_some_and(|host| !host.is_empty()),
        Err(_) => false,
    }
}

/// Проверить транспорт: либо команда (stdio), либо адрес (HTTP), но не оба сразу.
pub fn check_transport(url: &str, command: &str) -> Result<(), String> {
    let url = url.trim();
    let command = command.trim();
    match (url.is_empty(), command.is_empty()) {
        (true, true) => Err("нужна команда MCP-сервера или его URL".into()),
        (false, false) => Err("укажите что-то одно: команду запуска или URL".into()),
        (false, true) => {
            if valid_http_url(url) {
                Ok(())
            } else {
                Err("URL MCP-сервера должен начинаться с http:// или https://".into())
            }
        }
        (true, false) => crate::cloud::deny(crate::cloud::NO_STDIO),
    }
}

impl HttpSession {
    /// Инициализация: `initialize` с новым `Mcp-Session-Id`, затем `notifications/initialized`.
    /// `stale` — сессия, на которую сервер пожаловался. Если её уже заменил другой запрос,
    /// второй раз не открываем: у сервера с одной сессией это сломало бы первую.
    async fn handshake(&self, stale: Option<Option<String>>) -> Result<(), String> {
        let _guard = self.handshake.lock().await;
        if self.ready.load(Ordering::Relaxed) {
            match &stale {
                None => return Ok(()),
                Some(old) if *self.session_id.lock().await != *old => return Ok(()),
                Some(_) => {}
            }
        }
        self.ready.store(false, Ordering::Relaxed);
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let payload = json!({
            "jsonrpc": "2.0",
            "id": id,
            "method": "initialize",
            "params": {
                "protocolVersion": HTTP_PROTOCOL,
                "capabilities": {},
                "clientInfo": { "name": "deloist", "version": env!("CARGO_PKG_VERSION") },
            },
        });
        // Новую сессию открываем без старого идентификатора: так требует спецификация.
        let reply = self.post_once(&payload, Some(id), None).await.map_err(failure_text)?;
        if let Some(message) = error_text(&reply.message) {
            return Err(format!("MCP «{}»: не удалось инициализироваться: {message}", self.label));
        }
        *self.session_id.lock().await = reply.session_id.clone();
        self.tools.lock().unwrap().clear();

        // Уведомление без id: сервер отвечает 202 и тела не присылает. Готовы только после него,
        // иначе чужой запрос мог бы обогнать уведомление.
        let note = json!({ "jsonrpc": "2.0", "method": "notifications/initialized", "params": {} });
        self.post_once(&note, None, reply.session_id.as_deref()).await.map_err(failure_text)?;
        self.ready.store(true, Ordering::Relaxed);
        Ok(())
    }

    /// Закрыть сессию на сервере. Ответ не важен: сервер мог уже забыть её сам.
    async fn close(&self) {
        let Some(session) = self.session_id.lock().await.take() else { return };
        self.ready.store(false, Ordering::Relaxed);
        let mut request = self.client.delete(&self.url).header(SESSION_HEADER, session).timeout(Duration::from_secs(5));
        for (name, value) in &self.headers {
            request = request.header(name.as_str(), value.as_str());
        }
        let _ = request.send().await;
    }

    async fn request(&self, method: &str, params: Value) -> Result<Value, String> {
        if !self.ready.load(Ordering::Relaxed) {
            self.handshake(None).await?;
        }
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let payload = json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params });
        let mut retried = false;
        loop {
            let session = self.session_id.lock().await.clone();
            match self.post_once(&payload, Some(id), session.as_deref()).await {
                Ok(reply) => {
                    if let Some(message) = error_text(&reply.message) {
                        return Err(format!("MCP «{}»: {message}", self.label));
                    }
                    return Ok(reply.message.get("result").cloned().unwrap_or(Value::Null));
                }
                Err(Failure::Session(message)) if !retried => {
                    retried = true;
                    // 404 или жалоба на сессию: один раз открываем новую и повторяем запрос.
                    self.handshake(Some(session))
                        .await
                        .map_err(|e| format!("{message}; новая сессия не открылась: {e}"))?;
                }
                Err(failure) => return Err(failure_text(failure)),
            }
        }
    }

    async fn tools(&self) -> Vec<Value> {
        let cached = self.tools.lock().unwrap().clone();
        if !cached.is_empty() {
            return cached;
        }
        match self.request("tools/list", json!({})).await {
            Ok(result) => {
                let list = result["tools"].as_array().cloned().unwrap_or_default();
                *self.tools.lock().unwrap() = list.clone();
                list
            }
            Err(e) => {
                eprintln!("[mcp] «{}»: {e}", self.label);
                Vec::new()
            }
        }
    }

    /// Один POST. `want_id` = None — уведомление без ответа.
    async fn post_once(&self, payload: &Value, want_id: Option<i64>, session: Option<&str>) -> Result<Reply, Failure> {
        let mut request = self
            .client
            .post(&self.url)
            .header(ACCEPT, "application/json, text/event-stream")
            .header(CONTENT_TYPE, "application/json")
            .timeout(timeout())
            .body(payload.to_string());
        for (name, value) in &self.headers {
            request = request.header(name.as_str(), value.as_str());
        }
        if let Some(session) = session {
            request = request.header(SESSION_HEADER, session);
        }
        let response = request.send().await.map_err(|e| Failure::Fatal(self.transport_error(e)))?;
        let status = response.status();
        let session_id = response
            .headers()
            .get(SESSION_HEADER)
            .and_then(|value| value.to_str().ok())
            .map(str::to_string);

        if status == StatusCode::UNAUTHORIZED || status == StatusCode::FORBIDDEN {
            // Ключ в заголовках уже есть: значит, он не подошёл или вход протух.
            let hint = if self.headers.is_empty() { ". Добавьте его в адрес сервера, например ?key=…" } else { ": проверьте ключ или войдите заново" };
            return Err(Failure::Fatal(format!("MCP «{}»: сервер требует ключ (HTTP {}){hint}", self.label, status.as_u16())));
        }
        if status == StatusCode::NOT_FOUND || status == StatusCode::BAD_REQUEST {
            // Тело важно: по нему видно, жалуется сервер на сессию или на сам запрос.
            let text = read_body_text(response, 4096).await.unwrap_or_default();
            let hint = clean_hint(&text);
            let about_session = status == StatusCode::NOT_FOUND || hint.to_lowercase().contains("session");
            if about_session {
                return Err(Failure::Session(with_hint(
                    format!("MCP «{}»: сервер не принял сессию (HTTP {})", self.label, status.as_u16()),
                    &hint,
                )));
            }
            return Err(Failure::Fatal(with_hint(
                format!("MCP «{}»: HTTP {}", self.label, status.as_u16()),
                &hint,
            )));
        }
        if !status.is_success() {
            let text = read_body_text(response, 4096).await.unwrap_or_default();
            return Err(Failure::Fatal(with_hint(
                format!("MCP «{}»: HTTP {}", self.label, status.as_u16()),
                &clean_hint(&text),
            )));
        }
        let Some(want_id) = want_id else {
            return Ok(Reply { message: Value::Null, session_id });
        };
        let message = read_message(response, want_id)
            .await
            .map_err(|e| Failure::Fatal(format!("MCP «{}»: {e}", self.label)))?;
        Ok(Reply { message, session_id })
    }

    /// Ошибки транспорта без URL: в адресе может быть ключ.
    fn transport_error(&self, error: reqwest::Error) -> String {
        if error.is_timeout() {
            return format!("MCP «{}»: сервер не ответил за {}", self.label, timeout_text());
        }
        let error = error.without_url();
        if error.is_connect() {
            format!("MCP «{}»: не удалось подключиться: {error}", self.label)
        } else if error.is_request() {
            format!("MCP «{}»: запрос не отправился: {error}", self.label)
        } else {
            format!("MCP «{}»: {error}", self.label)
        }
    }
}

/// Прочитать тело целиком, но не больше `cap` байт.
async fn read_body_text(response: Response, cap: usize) -> Result<String, String> {
    let mut stream = response.bytes_stream();
    let mut body: Vec<u8> = Vec::new();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|e| e.without_url().to_string())?;
        let room = cap.saturating_sub(body.len());
        if room == 0 {
            break;
        }
        body.extend_from_slice(&chunk[..chunk.len().min(room)]);
    }
    Ok(String::from_utf8_lossy(&body).into_owned())
}

/// Ответ может быть JSON или потоком SSE. Берём только сообщение с нашим `id`,
/// остальное (`event:`, комментарии, ping, чужие уведомления) пропускаем.
async fn read_message(response: Response, want_id: i64) -> Result<Value, String> {
    let is_sse = response
        .headers()
        .get(CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .is_some_and(|kind| kind.to_ascii_lowercase().contains("text/event-stream"));
    let mut stream = response.bytes_stream();
    let mut buffer: Vec<u8> = Vec::new();
    let mut data = String::new();
    let mut total = 0usize;
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|e| e.without_url().to_string())?;
        total += chunk.len();
        if total > MAX_HTTP_BODY {
            return Err("ответ сервера больше 1 МБ".into());
        }
        buffer.extend_from_slice(&chunk);
        if !is_sse {
            continue;
        }
        while let Some(position) = buffer.iter().position(|byte| *byte == b'\n') {
            let line: Vec<u8> = buffer.drain(..=position).collect();
            let line = String::from_utf8_lossy(&line);
            let line = line.trim_end_matches(['\r', '\n']);
            if line.is_empty() {
                if let Some(message) = sse_message(&data, want_id) {
                    return Ok(message);
                }
                data.clear();
                continue;
            }
            if line.starts_with(':') {
                continue; // комментарий-«пинг»: connection alive
            }
            if let Some(rest) = line.strip_prefix("data:") {
                if !data.is_empty() {
                    data.push('\n');
                }
                data.push_str(rest.strip_prefix(' ').unwrap_or(rest));
            }
        }
    }
    if is_sse {
        if let Some(message) = sse_message(&data, want_id) {
            return Ok(message);
        }
        return Err(format!("поток событий закончился без ответа (id {want_id})"));
    }
    let text = String::from_utf8_lossy(&buffer);
    find_message(&text, want_id)
        .ok_or_else(|| format!("ответ без нужного сообщения (id {want_id}): {}", clean_hint(&text)))
}

fn sse_message(data: &str, want_id: i64) -> Option<Value> {
    let text = data.trim();
    if text.is_empty() {
        return None;
    }
    find_message(text, want_id)
}

/// Найти в JSON-RPC-ответе (одном или пакете) сообщение с нужным id.
fn find_message(text: &str, want_id: i64) -> Option<Value> {
    let value: Value = serde_json::from_str(text).ok()?;
    let items = match value {
        Value::Array(items) => items,
        other => vec![other],
    };
    // Запрос сервера к нам (с `method`) может нести тот же номер, но это не наш ответ.
    items
        .into_iter()
        .find(|message| message.get("method").is_none() && message.get("id").and_then(Value::as_i64) == Some(want_id))
}

fn sanitize(name: &str) -> String {
    let clean: String = name
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() || c == '_' || c == '-' { c } else { '_' })
        .take(60)
        .collect();
    if clean.is_empty() { "tool".into() } else { clean }
}

/// Имя инструмента для модели: `mcp__<сервер>__<инструмент>`, не длиннее предела API.
pub fn tool_name(server: &McpServer, tool: &str) -> String {
    let prefix = format!("mcp__{}__", sanitize(&server.id));
    let room = MAX_TOOL_NAME.saturating_sub(prefix.len());
    let short: String = sanitize(tool).chars().take(room).collect();
    format!("{prefix}{short}")
}

/// Разобрать имя на сервер и инструмент.
pub fn split_name(name: &str) -> Option<(String, String)> {
    let rest = name.strip_prefix("mcp__")?;
    let (server, tool) = rest.split_once("__")?;
    Some((server.to_string(), tool.to_string()))
}

/// Какой сервер отвечает за это имя инструмента.
pub fn server_of<'a>(servers: &'a [McpServer], full: &str) -> Option<&'a McpServer> {
    let (id, _) = split_name(full)?;
    servers.iter().find(|s| sanitize(&s.id) == id)
}

fn quoted(part: &str) -> String {
    if part.is_empty() || part.chars().any(|c| c.is_whitespace() || "\"&|<>^()%!".contains(c)) {
        format!("\"{}\"", part.replace('"', "\\\""))
    } else {
        part.to_string()
    }
}

/// Полный путь к существующему .exe: его можно запустить без cmd.exe.
#[cfg(windows)]
fn is_exe(command: &str) -> bool {
    let path = std::path::Path::new(command.trim());
    path.is_absolute() && path.extension().is_some_and(|e| e.eq_ignore_ascii_case("exe")) && path.is_file()
}

async fn spawn_stdio(cfg: &McpServer, env: Vec<(String, String)>) -> Result<Arc<StdioSession>, String> {
    // Политика облака: сервер по stdio — это запуск команды, консоли в облаке нет.
    crate::cloud::deny(crate::cloud::NO_STDIO)?;
    if cfg.command.trim().is_empty() {
        return Err("не указана команда MCP-сервера".into());
    }
    #[cfg(windows)]
    let mut command = if is_exe(&cfg.command) {
        // Программу .exe запускаем напрямую: аргументы экранирует Rust, cmd.exe ничего не толкует.
        let mut command = Command::new(cfg.command.trim());
        command.args(&cfg.args);
        command
    } else {
        // `npx` и прочие .cmd напрямую не запустить. Кавычки, %, ^, & и прочее cmd.exe толкует
        // сам, поэтому такие команды и аргументы отклоняются ещё при проверке шаблонов.
        let line = std::iter::once(cfg.command.as_str()).chain(cfg.args.iter().map(String::as_str)).map(quoted).collect::<Vec<_>>().join(" ");
        let comspec = std::env::var("ComSpec").unwrap_or_else(|_| "cmd.exe".into());
        let mut command = Command::new(comspec);
        command.raw_arg(format!("/d /s /c \"{line}\""));
        command
    };
    #[cfg(not(windows))]
    let mut command = {
        let mut command = Command::new(&cfg.command);
        command.args(&cfg.args);
        command
    };
    command.stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::null()).kill_on_drop(true);
    crate::process::keep_safe_env(&mut command);
    // Свой Node запускает npx-cli, а тот ищет `node` в PATH: папку node.exe ставим первой.
    let name = std::path::Path::new(cfg.command.trim()).file_name().and_then(|n| n.to_str()).unwrap_or_default();
    if name.eq_ignore_ascii_case("node.exe") || name.eq_ignore_ascii_case("node") {
        command.env("PATH", crate::runtime_node::path_with_node(std::path::Path::new(cfg.command.trim())));
    }
    // Секреты — только в окружение процесса: в командной строке их видно всем программам.
    for (name, value) in env {
        command.env(name, value);
    }
    #[cfg(windows)]
    command.creation_flags(0x0800_0000); // CREATE_NO_WINDOW

    let mut child = command.spawn().map_err(|e| format!("не смог запустить «{}»: {e}", cfg.command))?;
    let stdin = child.stdin.take().ok_or("MCP-сервер без stdin")?;
    let stdout = child.stdout.take().ok_or("MCP-сервер без stdout")?;

    let session = Arc::new(StdioSession {
        child: Mutex::new(child),
        stdin: AsyncMutex::new(stdin),
        pending: Mutex::new(HashMap::new()),
        next_id: AtomicI64::new(1),
        tools: Mutex::new(Vec::new()),
        name: cfg.name.clone(),
        command: cfg.command.clone(),
        args: cfg.args.clone(),
        env: cfg.env.clone(),
    });

    // Читаем ответы построчно и раскладываем по ожидающим запросам.
    let reader = session.clone();
    tokio::spawn(async move {
        let mut lines = BufReader::new(stdout).lines();
        while let Ok(Some(line)) = lines.next_line().await {
            let Ok(value) = serde_json::from_str::<Value>(&line) else { continue };
            // Запрос от сервера к нам: у него есть method, а номер может совпасть с нашим запросом.
            // На ping отвечаем, остальное вежливо отклоняем.
            if let Some(method) = value.get("method").and_then(Value::as_str) {
                let id = value.get("id").cloned();
                let reply = match (method, id) {
                    ("ping", Some(id)) => Some(json!({ "jsonrpc": "2.0", "id": id, "result": {} })),
                    (_, Some(id)) => Some(json!({ "jsonrpc": "2.0", "id": id, "error": { "code": -32601, "message": "нет такого метода" } })),
                    _ => None,
                };
                if let Some(reply) = reply {
                    let mut stdin = reader.stdin.lock().await;
                    let _ = stdin.write_all(reply.to_string().as_bytes()).await;
                    let _ = stdin.write_all(b"\n").await;
                    let _ = stdin.flush().await;
                }
                continue;
            }
            if let Some(id) = value.get("id").and_then(Value::as_i64) {
                let waiter = reader.pending.lock().unwrap().remove(&id);
                if let Some(tx) = waiter {
                    let _ = tx.send(value);
                }
            }
        }
        // Сервер закрылся: те, кто ждёт ответа, узнают об этом сразу, а не через 30 секунд.
        reader.pending.lock().unwrap().clear();
    });

    session
        .request(
            "initialize",
            json!({
                "protocolVersion": "2024-11-05",
                "capabilities": {},
                "clientInfo": { "name": "deloist", "version": env!("CARGO_PKG_VERSION") },
            }),
        )
        .await
        .map_err(|e| format!("MCP «{}»: {e}", cfg.name))?;
    let _ = session.notify("notifications/initialized", json!({})).await;
    Ok(session)
}

async fn spawn_http(cfg: &McpServer, headers: Vec<(String, String)>) -> Result<Arc<HttpSession>, String> {
    let url = cfg.url.trim().to_string();
    if !valid_http_url(&url) {
        return Err(format!("MCP «{}»: адрес сервера должен начинаться с http:// или https://", cfg.name));
    }
    let client = reqwest::Client::builder()
        .no_proxy()
        // Редиректы не идём: сессионный заголовок не должен уехать на чужой адрес.
        .redirect(reqwest::redirect::Policy::none())
        .user_agent(concat!("deloist/", env!("CARGO_PKG_VERSION")))
        .build()
        .map_err(|e| format!("MCP «{}»: {e}", cfg.name))?;
    let session = Arc::new(HttpSession {
        url,
        label: cfg.name.clone(),
        client,
        session_id: AsyncMutex::new(None),
        handshake: AsyncMutex::new(()),
        ready: AtomicBool::new(false),
        next_id: AtomicI64::new(1),
        tools: Mutex::new(Vec::new()),
        headers,
        header_templates: cfg.headers.clone(),
    });
    session.handshake(None).await?;
    Ok(session)
}

async fn session_for(app: &Arc<App>, cfg: &McpServer) -> Result<Arc<Session>, String> {
    // Замок берём отдельной строкой: взятый в условии `if let`, он дожил бы до конца блока,
    // и повторная попытка взять его ниже повесила бы поток.
    let live = |app: &Arc<App>| {
        let found = app.mcp.sessions.lock().unwrap().get(&cfg.id).cloned();
        match found {
            Some(session) if session.alive() && session.matches(cfg) && !crate::connectors::token_expiring(app, cfg) => Some(session),
            Some(session) => {
                // Настройки поменяли или процесс умер: старую сессию закрываем, новую откроем ниже.
                session.kill();
                app.mcp.sessions.lock().unwrap().remove(&cfg.id);
                None
            }
            None => None,
        }
    };
    if let Some(session) = live(app) {
        return Ok(session);
    }
    let _starting = app.mcp.starting.lock().await;
    if let Some(session) = live(app) {
        return Ok(session);
    }
    let started = async {
        // Согласие, хост, свежий токен и секреты — до запуска процесса или первого запроса.
        let prepared = crate::connectors::prepare(app, cfg).await?;
        Ok::<_, String>(Arc::new(if cfg.is_http() {
            Session::Http(spawn_http(cfg, prepared.headers).await?)
        } else {
            Session::Stdio(spawn_stdio(cfg, prepared.env).await?)
        }))
    }
    .await;
    let session = match started {
        Ok(session) => session,
        Err(e) => return Err(crate::connectors::note_failure(app, cfg, &e).await),
    };
    app.connectors.clear_error(&cfg.id);
    app.mcp.sessions.lock().unwrap().insert(cfg.id.clone(), session.clone());
    Ok(session)
}

/// Схемы всех включённых серверов в формате инструментов модели.
pub async fn tool_defs(app: &Arc<App>) -> Vec<Value> {
    let mut out = Vec::new();
    for cfg in app.store.mcp_servers().into_iter().filter(|s| s.enabled) {
        let waiting = app.mcp.failed.lock().unwrap().get(&cfg.id).is_some_and(|at| at.elapsed() < RETRY_AFTER);
        if waiting {
            continue;
        }
        let session = match session_for(app, &cfg).await {
            Ok(session) => {
                app.mcp.failed.lock().unwrap().remove(&cfg.id);
                session
            }
            Err(_) => {
                app.mcp.failed.lock().unwrap().insert(cfg.id.clone(), Instant::now());
                continue;
            }
        };
        let tools = session.tools().await;
        if cfg.connector.is_some() {
            app.connectors.set_tools(&cfg.id, tools.len());
        }
        for tool in tools {
            let Some(name) = tool["name"].as_str() else { continue };
            let description = crate::tools::clamp(tool["description"].as_str().unwrap_or_default(), MAX_DESCRIPTION);
            let schema = tool.get("inputSchema").cloned().unwrap_or_else(|| json!({ "type": "object", "properties": {} }));
            out.push(json!({
                "type": "function",
                "function": {
                    "name": tool_name(&cfg, name),
                    "description": format!("[{}] {}", cfg.name, description),
                    "parameters": schema,
                },
            }));
        }
    }
    out
}

/// Вызвать инструмент MCP-сервера по полному имени.
pub async fn call(app: &Arc<App>, full: &str, args: &Value) -> Result<String, String> {
    let (server_id, tool) = split_name(full).ok_or_else(|| "неизвестный инструмент".to_string())?;
    let cfg = app
        .store
        .mcp_servers()
        .into_iter()
        .find(|s| sanitize(&s.id) == server_id)
        .ok_or_else(|| format!("MCP-сервер «{server_id}» не найден"))?;
    if !cfg.enabled {
        return Err(format!("MCP-сервер «{}» выключен", cfg.name));
    }
    let session = session_for(app, &cfg).await?;
    let original = session
        .tools()
        .await
        .iter()
        .find(|t| t["name"].as_str().is_some_and(|name| tool_name(&cfg, name) == full))
        .and_then(|t| t["name"].as_str().map(str::to_string));
    let Some(original) = original else {
        return Err(format!("у сервера «{}» нет инструмента «{tool}»", cfg.name));
    };

    let result = match session.request("tools/call", json!({ "name": original, "arguments": args })).await {
        Ok(result) => result,
        Err(e) => return Err(crate::connectors::note_failure(app, &cfg, &e).await),
    };
    let mut parts = Vec::new();
    for item in result["content"].as_array().cloned().unwrap_or_default() {
        match item["type"].as_str() {
            Some("text") => parts.push(item["text"].as_str().unwrap_or_default().to_string()),
            Some(other) => parts.push(format!("[{other}]")),
            None => {}
        }
    }
    let mut text = parts.join("\n");
    if text.is_empty() {
        text = result.to_string();
    }
    // Сервер мог вернуть свой же ключ: модели он не нужен.
    let text = crate::connectors::scrub(app, &cfg.id, &text);
    if result["isError"].as_bool().unwrap_or(false) {
        return Err(crate::tools::clamp(&text, MAX_RESULT));
    }
    Ok(crate::tools::clamp(&text, MAX_RESULT))
}

/// Проверка из интерфейса: поднять сервер и перечислить инструменты.
pub async fn check(app: &Arc<App>, id: &str) -> Result<Vec<String>, String> {
    let cfg = app.store.mcp_server(id).ok_or_else(|| "сервер не найден".to_string())?;
    // Человек нажал «Проверить»: прошлая неудача больше не повод не пробовать.
    app.mcp.failed.lock().unwrap().remove(id);
    let session = session_for(app, &cfg).await?;
    // Проверка должна показать ошибку, а не пустой список: спрашиваем заново, без кэша.
    session.forget_tools();
    let result = match session.request("tools/list", json!({})).await {
        Ok(result) => result,
        Err(e) => return Err(crate::connectors::note_failure(app, &cfg, &e).await),
    };
    let tools = result["tools"].as_array().cloned().unwrap_or_default();
    session.set_tools(tools.clone());
    Ok(tools.iter().filter_map(|t| t["name"].as_str().map(str::to_string)).collect())
}

/// Сколько инструментов у живой сессии сервера, если она уже их перечислила.
pub fn known_tools(app: &App, id: &str) -> Option<usize> {
    let session = app.mcp.sessions.lock().unwrap().get(id).cloned()?;
    let count = match &*session {
        Session::Stdio(s) => s.tools.lock().unwrap().len(),
        Session::Http(s) => s.tools.lock().unwrap().len(),
    };
    (count > 0).then_some(count)
}

/// Закрыть все живые серверы: при выходе из приложения.
pub async fn shutdown(app: &App) {
    let sessions: Vec<Arc<Session>> = app.mcp.sessions.lock().unwrap().drain().map(|(_, s)| s).collect();
    for session in sessions {
        session.kill();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_url_is_http_or_https_with_a_host() {
        assert!(valid_http_url("http://127.0.0.1:3001/mcp"));
        assert!(valid_http_url("https://example.com/mcp?key=secret"));
        assert!(valid_http_url("http://localhost/mcp"));
        assert!(!valid_http_url(""));
        assert!(!valid_http_url("example.com/mcp"));
        assert!(!valid_http_url("ftp://example.com/mcp"));
        assert!(!valid_http_url("file:///c:/mcp"));
        assert!(!valid_http_url("http://"));
        assert!(!valid_http_url("http://example.com/mcp с пробелом"));
    }

    #[test]
    fn a_shown_address_and_a_default_name_carry_no_key() {
        assert_eq!(public_url("https://user:pass@m.io/mcp?key=sk-1#x"), "https://m.io/mcp");
        assert_eq!(default_name("https://m.io/mcp?key=sk-1"), "m.io");
        assert_eq!(default_name("не адрес"), "MCP");
    }

    #[test]
    fn a_server_request_with_our_number_is_not_our_answer() {
        let text = r#"[{"jsonrpc":"2.0","id":7,"method":"ping"},{"jsonrpc":"2.0","id":7,"result":{"ok":true}}]"#;
        assert_eq!(find_message(text, 7).unwrap()["result"]["ok"], true);
        assert!(find_message(r#"{"jsonrpc":"2.0","id":7,"method":"ping"}"#, 7).is_none());
    }

    #[test]
    fn a_transport_is_either_a_command_or_a_url() {
        assert!(check_transport("http://127.0.0.1:3001/mcp", "").is_ok(), "пустая команда при HTTP допустима");
        assert!(check_transport("", "npx").is_ok());
        assert!(check_transport("http://127.0.0.1:3001/mcp", "npx").is_err(), "два транспорта сразу");
        assert!(check_transport("", "").is_err(), "без транспорта");
        assert!(check_transport("ftp://example.com/mcp", "").is_err());
    }
}
