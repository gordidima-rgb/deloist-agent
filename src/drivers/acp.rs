//! Драйвер ACP (Agent Client Protocol): бот работает через внешнего агента — Claude Code, Codex,
//! Gemini CLI, Copilot, Goose и других, — который говорит с нами JSON-RPC по stdio.
//!
//! Агент — отдельная программа со своими инструментами и своим входом в аккаунт. Мы запускаем её
//! в воркспейсе бота, отдаём ей сообщение человека и показываем, что она делает. Файлы агент читает
//! и пишет через нас (`fs/*`), и только внутри воркспейса. Терминал мы ему не объявляем.
//! Разрешения агента идут в наши карточки подтверждения.
//!
//! Процесс агента живёт дольше одного хода: на бота один процесс и одна сессия, пока не поменялась
//! команда запуска. Упал — на следующем ходе запускаем заново и продолжаем сессию, если агент умеет.

use std::collections::{HashMap, HashSet, VecDeque};
use std::path::{Component, Path, PathBuf};
use std::process::Stdio;
use std::sync::atomic::{AtomicBool, AtomicI64, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

use serde_json::{json, Value};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, ChildStdin};
use tokio::sync::{mpsc, oneshot, Mutex as AsyncMutex};
use tokio_util::sync::CancellationToken;

use super::{DriverEvent, Sink};
use crate::store::Bot;
use crate::App;

pub const PROTOCOL_VERSION: i64 = 1;
/// Ключ разговора для личного чата бота; команда — `room:<id>`.
pub const PERSONAL_THREAD: &str = "bot";

/// Режимы агентов, в которых правки и команды идут через `session/request_permission`.
/// Codex: `read-only`; Claude Code, Gemini и большинство остальных: `default`.
const ASKING_MODES: [&str; 2] = ["read-only", "default"];
/// Первый запуск через npx скачивает пакет: на это уходит до минуты и больше.
const INIT_TIMEOUT: Duration = Duration::from_secs(120);
const SESSION_TIMEOUT: Duration = Duration::from_secs(60);
/// Вход в аккаунт агента идёт в браузере: человеку нужно время.
const AUTH_TIMEOUT: Duration = Duration::from_secs(5 * 60);
/// После отмены агент обязан закончить ход сам. Не закончил — гасим процесс.
const CANCEL_GRACE: Duration = Duration::from_secs(10);
const STDERR_LINES: usize = 30;
/// Сколько прошлых сообщений пересказать новой сессии и сколько символов на каждое.
const RECAP_MESSAGES: usize = 12;
const RECAP_CHARS: usize = 2000;
/// Дальше последних `RECAP_MESSAGES` сообщения короче, всего не больше `RECAP_MAX_MESSAGES` и `RECAP_BUDGET` знаков.
const RECAP_OLD_CHARS: usize = 500;
const RECAP_MAX_MESSAGES: usize = 80;
const RECAP_BUDGET: usize = 40_000;
/// Заполненность окна агента, после которой прежнюю сессию не продолжаем: 85% — уже тесно.
const FRESH_SESSION_AT: f64 = 0.85;
const MAX_READ: u64 = 4 * 1024 * 1024;

/// Как запустить агента. Полная команда со всеми аргументами, как у CLI-драйвера.
pub struct Spec<'a> {
    pub command: &'a str,
    pub env: &'a [(String, String)],
    /// Наши секреты, которые агенту видеть незачем.
    pub hidden_env: &'a [String],
    /// Режим сессии агента (`session/set_mode`), если он такой объявил. Пусто — режим агента по умолчанию.
    pub mode: &'a str,
    /// Модель агента (его `configOptions` категории `model`). Пусто — модель агента по умолчанию.
    pub model: &'a str,
    /// Усилие рассуждения (`configOptions` категории `thought_level`). Пусто или не объявлено агентом — не трогаем.
    pub effort: &'a str,
    /// Прочие выбираемые настройки агента, выбранные у бота: `(configId, valueId)`.
    pub options: &'a [(String, String)],
    /// Папка, внутри которой у каждого процесса агента свой `TEMP`/`TMP` (убирается после выхода процесса).
    pub temp_root: Option<&'a Path>,
    /// Способ входа: `api-key` — по ключу из окружения, пусто — вход агента через браузер.
    pub auth: &'a str,
}

// ---------------------------------------------------------------------------
// Ссылка входа Google
// ---------------------------------------------------------------------------

const LINK_PREFIX: &str = "https://accounts.google.com/";

/// Ссылка входа, пока идёт вход в аккаунт. Лежит только в памяти: ни в журнал, ни в файлы, ни в ответы списка.
fn link_store() -> &'static Mutex<Option<String>> {
    static LINK: OnceLock<Mutex<Option<String>>> = OnceLock::new();
    LINK.get_or_init(Default::default)
}

/// Ссылка `https://accounts.google.com/...` из строки (вывод агента, уведомление). Только этот адрес.
pub fn extract_link(text: &str) -> Option<String> {
    let text = text.replace("\\u0026", "&");
    let start = text.find(LINK_PREFIX)?;
    let link: String = text[start..].chars().take_while(|c| c.is_ascii_graphic() && !"\"'<>\\`".contains(*c)).collect();
    (link.len() > LINK_PREFIX.len() && link.len() <= 4096).then_some(link)
}

pub fn set_signin_link(link: &str) {
    if let Some(link) = extract_link(link) {
        *link_store().lock().unwrap() = Some(link);
    }
}

pub fn signin_link() -> Option<String> {
    link_store().lock().unwrap().clone()
}

pub fn clear_signin_link() {
    link_store().lock().unwrap().take();
}

/// Живые временные папки процессов: всё остальное в корне — остатки прошлых запусков.
fn live_temps() -> &'static Mutex<HashSet<PathBuf>> {
    static LIVE: OnceLock<Mutex<HashSet<PathBuf>>> = OnceLock::new();
    LIVE.get_or_init(Default::default)
}

/// Своя временная папка процесса. Заодно убирает остатки упавших запусков.
fn make_temp(root: &Path) -> Option<PathBuf> {
    static COUNTER: AtomicI64 = AtomicI64::new(0);
    let dir = root.join(format!("{}-{}", std::process::id(), COUNTER.fetch_add(1, Ordering::SeqCst)));
    let mut live = live_temps().lock().unwrap();
    if let Ok(entries) = std::fs::read_dir(root) {
        for entry in entries.flatten() {
            // Только давно брошенное: папка живого процесса другой копии программы моложе суток.
            let old = entry.metadata().and_then(|m| m.modified()).ok().and_then(|t| t.elapsed().ok()).is_some_and(|age| age > Duration::from_secs(24 * 3600));
            if old && !live.contains(&entry.path()) {
                let _ = std::fs::remove_dir_all(entry.path());
            }
        }
    }
    std::fs::create_dir_all(&dir).ok()?;
    live.insert(dir.clone());
    Some(dir)
}

async fn drop_temp(dir: PathBuf) {
    for _ in 0..20 {
        if std::fs::remove_dir_all(&dir).is_ok() || !dir.exists() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
    live_temps().lock().unwrap().remove(&dir);
}

impl Spec<'_> {
    fn signature(&self) -> String {
        let mut env: Vec<String> = self.env.iter().map(|(k, v)| format!("{k}={v}")).collect();
        env.sort();
        format!("{}\u{1}{}", self.command.trim(), env.join("\u{1}"))
    }
}

// ---------------------------------------------------------------------------
// Процесс агента
// ---------------------------------------------------------------------------

struct Agent {
    signature: String,
    pid: u32,
    child: Mutex<Child>,
    stdin: AsyncMutex<ChildStdin>,
    pending: Mutex<HashMap<i64, oneshot::Sender<Value>>>,
    next_id: AtomicI64,
    /// Куда идут уведомления и запросы агента, пока идёт ход. Нет хода — запросы получают отказ.
    inbox: Mutex<Option<mpsc::UnboundedSender<Value>>>,
    alive: AtomicBool,
    stderr: Arc<Mutex<VecDeque<String>>>,
    /// Ответ на `initialize`: что агент умеет и как в него входить.
    init: Mutex<Value>,
    /// Сессии бота по разговорам (`bot` — личный чат, `room:<id>` — команда): у каждого своя. Замок заодно не даёт двум ходам одного бота идти в один процесс.
    sessions: AsyncMutex<HashMap<String, String>>,
    /// Своя временная папка процесса: убирается, когда процесс закрылся.
    temp: Option<PathBuf>,
}

enum CallError {
    Rpc(Value),
    Closed,
    Timeout,
}

impl Agent {
    fn is_alive(&self) -> bool {
        self.alive.load(Ordering::SeqCst) && matches!(self.child.lock().unwrap().try_wait(), Ok(None))
    }

    fn stderr_tail(&self) -> String {
        let lines = self.stderr.lock().unwrap();
        let text: Vec<&str> = lines.iter().map(String::as_str).collect();
        let text = text.join("\n");
        let start = text.char_indices().rev().nth(800).map_or(0, |(i, _)| i);
        text[start..].trim().to_string()
    }

    async fn write(&self, message: &Value) -> Result<(), String> {
        let mut line = message.to_string();
        trace(">>", &line);
        line.push('\n');
        let mut stdin = self.stdin.lock().await;
        stdin.write_all(line.as_bytes()).await.map_err(|e| e.to_string())?;
        stdin.flush().await.map_err(|e| e.to_string())
    }

    async fn send_request(&self, method: &str, params: Value) -> Result<oneshot::Receiver<Value>, String> {
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let (tx, rx) = oneshot::channel();
        self.pending.lock().unwrap().insert(id, tx);
        if let Err(e) = self.write(&json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params })).await {
            self.pending.lock().unwrap().remove(&id);
            return Err(e);
        }
        Ok(rx)
    }

    async fn call(&self, method: &str, params: Value, timeout: Duration) -> Result<Value, CallError> {
        let rx = self.send_request(method, params).await.map_err(|_| CallError::Closed)?;
        match tokio::time::timeout(timeout, rx).await {
            Ok(Ok(reply)) => unpack(reply),
            Ok(Err(_)) => Err(CallError::Closed),
            Err(_) => Err(CallError::Timeout),
        }
    }

    async fn notify(&self, method: &str, params: Value) {
        let _ = self.write(&json!({ "jsonrpc": "2.0", "method": method, "params": params })).await;
    }

    async fn respond(&self, id: Value, result: Value) {
        let _ = self.write(&json!({ "jsonrpc": "2.0", "id": id, "result": result })).await;
    }

    async fn respond_error(&self, id: Value, code: i64, message: &str) {
        let _ = self.write(&json!({ "jsonrpc": "2.0", "id": id, "error": { "code": code, "message": message } })).await;
    }

    fn kill(&self) {
        self.alive.store(false, Ordering::SeqCst);
        kill_tree(self.pid);
        let _ = self.child.lock().unwrap().start_kill();
        self.pending.lock().unwrap().clear();
    }
}

/// `DELOIST_ACP_TRACE=<файл>` — весь обмен с агентом построчно: для разбора несовместимостей.
fn trace(direction: &str, line: &str) {
    let Some(path) = std::env::var_os("DELOIST_ACP_TRACE") else { return };
    if let Ok(mut file) = std::fs::OpenOptions::new().create(true).append(true).open(path) {
        use std::io::Write;
        let _ = writeln!(file, "{direction} {line}");
    }
}

fn unpack(reply: Value) -> Result<Value, CallError> {
    if let Some(error) = reply.get("error").filter(|e| !e.is_null()) {
        return Err(CallError::Rpc(error.clone()));
    }
    Ok(reply.get("result").cloned().unwrap_or(Value::Null))
}

/// Через `cmd /c` агент запускает node и дальше: гасим всё дерево, а не одну оболочку.
#[cfg(windows)]
fn kill_tree(pid: u32) {
    use std::os::windows::process::CommandExt;
    let _ = std::process::Command::new("taskkill")
        .args(["/T", "/F", "/PID", &pid.to_string()])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .creation_flags(0x0800_0000)
        .status();
}

#[cfg(not(windows))]
fn kill_tree(_pid: u32) {}

fn pool() -> &'static Mutex<HashMap<String, Arc<Agent>>> {
    static POOL: OnceLock<Mutex<HashMap<String, Arc<Agent>>>> = OnceLock::new();
    POOL.get_or_init(|| Mutex::new(HashMap::new()))
}

/// Последняя сессия бота в каждом разговоре, ключ `(бот, разговор)`: после перезапуска агента её можно продолжить через `session/load`.
fn known_sessions() -> &'static Mutex<HashMap<(String, String), (String, String)>> {
    static SESSIONS: OnceLock<Mutex<HashMap<(String, String), (String, String)>>> = OnceLock::new();
    SESSIONS.get_or_init(|| Mutex::new(HashMap::new()))
}

/// Файл с последними сессиями на диске: после перезапуска программы (Dev обновляется после каждого слияния)
/// агент продолжает прежнюю сессию через `session/load`, а не начинает с коротким пересказом.
fn sessions_path(data_dir: &Path) -> PathBuf {
    data_dir.join("acp-sessions.json")
}

fn saved_session(data_dir: &Path, key: &(String, String)) -> Option<(String, String)> {
    let text = std::fs::read_to_string(sessions_path(data_dir)).ok()?;
    let map: HashMap<String, (String, String)> = serde_json::from_str(&text).ok()?;
    map.get(&format!("{}\u{1}{}", key.0, key.1)).cloned()
}

/// Записать (или убрать, если `value` пуст) сессию в файл. Ошибки записи не мешают ходу.
fn save_session(data_dir: &Path, key: &(String, String), value: Option<(String, String)>) {
    let path = sessions_path(data_dir);
    let mut map: HashMap<String, (String, String)> =
        std::fs::read_to_string(&path).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default();
    let name = format!("{}\u{1}{}", key.0, key.1);
    match value {
        Some(value) if map.get(&name) == Some(&value) => return,
        Some(value) => {
            map.insert(name, value);
        }
        None => {
            if map.remove(&name).is_none() {
                return;
            }
        }
    }
    let Ok(text) = serde_json::to_string(&map) else { return };
    let tmp = path.with_extension("json.tmp");
    if std::fs::write(&tmp, text).is_ok() {
        let _ = std::fs::rename(&tmp, &path);
    }
}

/// Погасить агентов всех ботов: при выходе программы.
pub fn shutdown() {
    let agents: Vec<Arc<Agent>> = pool().lock().unwrap().drain().map(|(_, agent)| agent).collect();
    for agent in agents {
        agent.kill();
    }
}

async fn agent_for(bot_id: &str, spec: &Spec<'_>, workspace: &Path) -> Result<Arc<Agent>, String> {
    let signature = spec.signature();
    let existing = pool().lock().unwrap().get(bot_id).cloned();
    if let Some(agent) = existing {
        if agent.signature == signature && agent.is_alive() {
            return Ok(agent);
        }
        agent.kill();
        pool().lock().unwrap().remove(bot_id);
    }
    let agent = spawn(spec, signature, workspace).await?;
    pool().lock().unwrap().insert(bot_id.to_string(), agent.clone());
    Ok(agent)
}

/// Встроенный поиск агента. codex-acp принимает `CODEX_CONFIG` (JSON, вливается в конфиг сессии Codex),
/// а `web_search = "live"` включает у Codex поиск по свежему интернету вместо кэша OpenAI. Поиск — родной
/// инструмент модели: без ключей, без нашего браузера. Свой `CODEX_CONFIG` человека не трогаем.
/// У claude-agent-acp WebSearch/WebFetch включены сами, отдельной настройки нет.
fn native_search_env(command: &str, env: &[(String, String)], inherited: bool) -> Option<(String, String)> {
    let is_codex = command.to_lowercase().contains("codex-acp");
    let own = inherited || env.iter().any(|(name, _)| name.eq_ignore_ascii_case("CODEX_CONFIG"));
    (is_codex && !own).then(|| ("CODEX_CONFIG".to_string(), json!({ "web_search": "live" }).to_string()))
}

async fn spawn(spec: &Spec<'_>, signature: String, workspace: &Path) -> Result<Arc<Agent>, String> {
    // Политика облака: агенты запускаются командой, а консоли в облаке нет. Единственная точка запуска ACP.
    crate::cloud::deny(crate::cloud::NO_DRIVER)?;
    let command = spec.command.trim();
    if command.is_empty() {
        return Err("не указана команда запуска агента ACP: впиши её в настройках".into());
    }
    let program = super::cli::first_word(command).unwrap_or_default();
    let mut cmd = crate::process::shell_command(command);
    // Агенту нужно своё окружение целиком (его вход, его настройки), а наши ключи — нет.
    for name in spec.hidden_env {
        if !name.is_empty() {
            cmd.env_remove(name);
        }
    }
    for (name, _) in std::env::vars_os() {
        let name = name.to_string_lossy().to_string();
        if name.to_uppercase().starts_with("CREW_") {
            cmd.env_remove(&name);
        }
    }
    for (name, value) in spec.env {
        cmd.env(name, value);
    }
    if let Some((name, value)) = native_search_env(command, spec.env, std::env::var_os("CODEX_CONFIG").is_some()) {
        cmd.env(name, value);
    }
    let temp = spec.temp_root.and_then(make_temp);
    if let Some(dir) = &temp {
        cmd.env("TEMP", dir).env("TMP", dir).env("TMPDIR", dir);
    }
    let _ = std::fs::create_dir_all(workspace);
    cmd.current_dir(workspace)
        .env("NO_COLOR", "1")
        // Агент запускает программы по голому имени: подложенный в его папке файл не должен подхватиться вместо настоящего.
        .env("NoDefaultCurrentDirectoryInExePath", "1")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    let mut child = match cmd.spawn() {
        Ok(child) => child,
        Err(e) => {
            if let Some(dir) = temp {
                tokio::spawn(drop_temp(dir));
            }
            return Err(format!("не смог запустить агента «{program}»: {e}"));
        }
    };
    crate::browser::tie_to_this_process(&child);
    let pid = child.id().unwrap_or(0);
    let stdin = child.stdin.take().ok_or("агент без stdin")?;
    let stdout = child.stdout.take().ok_or("агент без stdout")?;
    let stderr = child.stderr.take().ok_or("агент без stderr")?;

    let agent = Arc::new(Agent {
        signature,
        pid,
        child: Mutex::new(child),
        stdin: AsyncMutex::new(stdin),
        pending: Mutex::new(HashMap::new()),
        next_id: AtomicI64::new(1),
        inbox: Mutex::new(None),
        alive: AtomicBool::new(true),
        stderr: Arc::new(Mutex::new(VecDeque::new())),
        init: Mutex::new(Value::Null),
        sessions: AsyncMutex::new(HashMap::new()),
        temp,
    });

    let tail = agent.stderr.clone();
    tokio::spawn(async move {
        let mut lines = BufReader::new(stderr).lines();
        while let Ok(Some(line)) = lines.next_line().await {
            // Ссылку входа запоминаем для карточки, а в хвост вывода (он попадает в ошибки) не кладём.
            let line = match extract_link(&line) {
                Some(link) => {
                    set_signin_link(&link);
                    line.replace(&link, "[ссылка входа скрыта]")
                }
                None => line,
            };
            let mut tail = tail.lock().unwrap();
            if tail.len() >= STDERR_LINES {
                tail.pop_front();
            }
            tail.push_back(line);
        }
    });

    let reader = agent.clone();
    tokio::spawn(async move {
        let mut lines = BufReader::new(stdout).lines();
        while let Ok(Some(line)) = lines.next_line().await {
            trace("<<", &line);
            if line.contains(LINK_PREFIX) {
                set_signin_link(&line);
            }
            let Ok(message) = serde_json::from_str::<Value>(line.trim()) else { continue };
            if message.get("method").and_then(Value::as_str).is_some() {
                let inbox = reader.inbox.lock().unwrap().clone();
                let delivered = inbox.map(|tx| tx.send(message.clone()).is_ok()).unwrap_or(false);
                if !delivered {
                    answer_outside_turn(&reader, &message).await;
                }
                continue;
            }
            if let Some(id) = message.get("id").and_then(Value::as_i64) {
                let waiter = reader.pending.lock().unwrap().remove(&id);
                if let Some(tx) = waiter {
                    let _ = tx.send(message);
                }
            }
        }
        // Агент закрылся: ждущие узнают сразу, а не по таймауту.
        reader.alive.store(false, Ordering::SeqCst);
        reader.pending.lock().unwrap().clear();
        reader.inbox.lock().unwrap().take();
        if let Some(dir) = reader.temp.clone() {
            drop_temp(dir).await;
        }
    });

    let init = agent
        .call(
            "initialize",
            json!({
                "protocolVersion": PROTOCOL_VERSION,
                "clientCapabilities": { "fs": { "readTextFile": true, "writeTextFile": true }, "terminal": false },
                "clientInfo": { "name": "deloist", "title": "Deloist", "version": env!("CARGO_PKG_VERSION") },
            }),
            INIT_TIMEOUT,
        )
        .await;
    let init = match init {
        Ok(init) => init,
        Err(e) => {
            // Дать stderr дочитаться: там обычно и написано, почему агент не встал.
            tokio::time::sleep(Duration::from_millis(300)).await;
            let why = match e {
                CallError::Timeout => format!("не ответил за {} с", INIT_TIMEOUT.as_secs()),
                CallError::Closed => "закрылся при запуске".to_string(),
                CallError::Rpc(error) => format!("отказался начинать работу: {}", rpc_message(&error)),
            };
            let tail = agent.stderr_tail();
            agent.kill();
            let suffix = if tail.is_empty() { String::new() } else { format!(" Вывод агента: {tail}") };
            return Err(format!("агент «{program}» {why}.{suffix}"));
        }
    };
    let version = init["protocolVersion"].as_i64().unwrap_or(0);
    if version != PROTOCOL_VERSION {
        agent.kill();
        return Err(format!(
            "агент «{program}» говорит на версии протокола ACP {version}, а Deloist — на {PROTOCOL_VERSION}. Обнови агента"
        ));
    }
    *agent.init.lock().unwrap() = init;
    Ok(agent)
}

/// Запрос агента пришёл, когда хода нет: разрешения не даём, файлы не трогаем.
async fn answer_outside_turn(agent: &Agent, message: &Value) {
    let Some(id) = message.get("id").cloned() else { return };
    if message["method"] == "session/request_permission" {
        agent.respond(id, json!({ "outcome": { "outcome": "cancelled" } })).await;
    } else {
        agent.respond_error(id, -32601, "нет хода: запрос отклонён").await;
    }
}

fn rpc_message(error: &Value) -> String {
    let message = error["message"].as_str().unwrap_or("ошибка агента").to_string();
    match error.get("data").filter(|d| !d.is_null()) {
        Some(Value::String(detail)) => format!("{message}: {detail}"),
        Some(detail) if detail.get("message").is_some() => format!("{message}: {}", detail["message"].as_str().unwrap_or_default()),
        _ => message,
    }
}

fn needs_auth(error: &Value) -> bool {
    let message = error["message"].as_str().unwrap_or_default().to_lowercase();
    error["code"].as_i64() == Some(-32000) || message.contains("auth")
}

// ---------------------------------------------------------------------------
// Ход
// ---------------------------------------------------------------------------

pub struct Request<'a> {
    pub app: &'a Arc<App>,
    pub bot: &'a Bot,
    /// Сообщения в виде для модели: системное, история, последнее сообщение человека.
    pub messages: &'a [Value],
    pub workspace: &'a Path,
    /// Разговор, которому принадлежит ход: `bot` — личный чат, `room:<id>` — команда. У бота в каждом своя сессия агента.
    pub thread: &'a str,
    pub spec: Spec<'a>,
}

/// Переменная окружения MCP-сервера браузера: путь к файлу с ключом идущего хода.
pub const TURN_KEY_ENV: &str = "DELOIST_BROWSER_TURN_KEY_FILE";

fn turn_key_path(data_dir: &Path, bot_id: &str) -> PathBuf {
    data_dir.join("browser-turn-keys").join(format!("{bot_id}.key"))
}

/// Ключ моста к браузеру на время одного хода: выдаётся при старте хода, снимается при выходе.
struct TurnKey<'a> {
    app: &'a Arc<crate::App>,
    bot_id: String,
    path: PathBuf,
}

impl<'a> TurnKey<'a> {
    fn issue(app: &'a Arc<crate::App>, bot_id: &str) -> Option<Self> {
        let token = app.turns.issue_browser_token(bot_id)?;
        let path = turn_key_path(app.store.data_dir(), bot_id);
        std::fs::create_dir_all(path.parent()?).ok()?;
        let tmp = path.with_extension("key.tmp");
        if std::fs::write(&tmp, token).is_err() {
            let _ = std::fs::remove_file(&tmp);
            app.turns.revoke_browser_token(bot_id);
            return None;
        }
        if std::fs::rename(&tmp, &path).is_err() {
            let _ = std::fs::remove_file(&tmp);
            app.turns.revoke_browser_token(bot_id);
            return None;
        }
        Some(Self { app, bot_id: bot_id.to_string(), path })
    }
}

impl Drop for TurnKey<'_> {
    fn drop(&mut self) {
        self.app.turns.revoke_browser_token(&self.bot_id);
        let _ = std::fs::remove_file(&self.path);
    }
}

pub async fn run(req: Request<'_>, cancel: &CancellationToken, sink: Sink<'_>) -> Result<(), String> {
    crate::cloud::deny(crate::cloud::NO_DRIVER)?;
    let bot_id = req.bot.id.clone();
    // Живёт до конца функции; без хода (его нет у вызова вне очереди) ключа нет и мост закрыт.
    let _turn_key = TurnKey::issue(req.app, &bot_id);
    let agent = agent_for(&bot_id, &req.spec, req.workspace).await?;
    let mut sessions = agent.sessions.lock().await;
    let key = (bot_id.clone(), req.thread.to_string());

    let (tx, mut inbox) = mpsc::unbounded_channel::<Value>();
    let mut carry: Option<String> = None;
    // Сводку пишет модель команды по API, как у доктора памяти. Нет такой модели — новая сессия
    // началась бы с чистого листа: тогда оставляем агенту прежнюю, он сожмётся сам, как раньше.
    let writer = reported_usage_in(&bot_id, req.thread)
        .is_some_and(|(used, size)| needs_fresh_session(used, size))
        .then(|| crate::service::house_bot(req.app))
        .flatten();
    if let Some(writer) = writer {
        // Окно агента почти полно: прежнюю сессию не продолжаем, начинаем новую и уносим в неё сводку.
        sessions.remove(req.thread);
        known_sessions().lock().unwrap().remove(&key);
        save_session(req.app.store.data_dir(), &key, None);
        forget_usage(&bot_id, req.thread);
        carry = fresh_session_summary(req.app, req.bot, &writer, req.thread).await;
    }
    let fresh = if !sessions.contains_key(req.thread) {
        let (id, fresh) = open_session(&agent, &req, cancel).await?;
        sessions.insert(req.thread.to_string(), id);
        fresh
    } else {
        false
    };
    let session_id = sessions.get(req.thread).cloned().unwrap_or_default();
    known_sessions().lock().unwrap().insert(key.clone(), (agent.signature.clone(), session_id.clone()));
    save_session(req.app.store.data_dir(), &key, Some((agent.signature.clone(), session_id.clone())));

    let prompt = prompt_blocks(&agent, req.messages, fresh, carry.as_deref());
    *agent.inbox.lock().unwrap() = Some(tx);
    let reply = agent.send_request("session/prompt", json!({ "sessionId": session_id, "prompt": prompt })).await;
    let mut reply = match reply {
        Ok(rx) => rx,
        Err(e) => {
            agent.inbox.lock().unwrap().take();
            return Err(format!("агент не принял сообщение: {e}"));
        }
    };

    let mut turn = TurnState { allowed_paths: HashSet::new(), approval_policy: None, known_calls: HashMap::new(), cancelled: false, wrote: false, worked: false, bot_id: bot_id.clone(), thread: req.thread.to_string() };
    let mut deadline: Option<tokio::time::Instant> = None;
    let outcome = loop {
        if turn.cancelled && deadline.is_none() {
            deadline = Some(tokio::time::Instant::now() + CANCEL_GRACE);
            agent.notify("session/cancel", json!({ "sessionId": session_id })).await;
        }
        let grace = deadline.unwrap_or_else(|| tokio::time::Instant::now() + Duration::from_secs(3600));
        tokio::select! {
            biased;
            _ = cancel.cancelled(), if !turn.cancelled => {
                turn.cancelled = true;
            }
            done = &mut reply => {
                break match done {
                    Ok(done) => unpack(done),
                    Err(_) => Err(CallError::Closed),
                };
            }
            message = inbox.recv() => {
                let Some(message) = message else {
                    // Агент закрылся, ответа на prompt уже не будет.
                    break Err(CallError::Closed);
                };
                handle_message(&agent, &req, &session_id, &message, &mut turn, cancel, sink).await;
            }
            _ = tokio::time::sleep_until(grace), if deadline.is_some() => {
                agent.kill();
                break Err(CallError::Timeout);
            }
        }
    };
    agent.inbox.lock().unwrap().take();
    // Что агент ещё прислал до ответа на prompt, но мы не успели разобрать.
    while let Ok(message) = inbox.try_recv() {
        if message.get("id").is_some() {
            answer_outside_turn(&agent, &message).await;
        } else {
            handle_update(&message, &mut turn, sink);
        }
    }

    match outcome {
        Ok(result) => match result["stopReason"].as_str().unwrap_or("end_turn") {
            "cancelled" => Err("остановлено".into()),
            "refusal" => Err("агент отказался выполнять просьбу".into()),
            _ if turn.cancelled => Err("остановлено".into()),
            _ => Ok(()),
        },
        Err(_) if turn.cancelled => Err("остановлено".into()),
        Err(CallError::Rpc(error)) => {
            if needs_auth(&error) {
                sessions.remove(req.thread);
            }
            Err(format!("агент вернул ошибку: {}", rpc_message(&error)))
        }
        Err(CallError::Closed) | Err(CallError::Timeout) => {
            let tail = agent.stderr_tail();
            agent.kill();
            pool().lock().unwrap().remove(&bot_id);
            let suffix = if tail.is_empty() { String::new() } else { format!(" Вывод агента: {tail}") };
            Err(format!("агент неожиданно закрылся посреди ответа.{suffix}"))
        }
    }
}

struct TurnState {
    thread: String,
    bot_id: String,
    /// Файлы, на правку которых человек уже дал разрешение в этом ходе.
    allowed_paths: HashSet<PathBuf>,
    approval_policy: Option<String>,
    known_calls: HashMap<String, KnownCall>,
    cancelled: bool,
    /// Агент уже что-то написал, а потом работал: следующий текст — новое сообщение, с новой строки.
    wrote: bool,
    worked: bool,
}

/// Only identity and a conflict marker; never retain tool contents, output or arguments.
struct KnownCall {
    identity: Option<(String, String)>,
    safe: bool,
}

/// Команда ACP-агента массивом аргументов, которую строгий разбор признал правкой своих файлов, идёт без карточки (наша ветка).
/// `false`: на main действует более строгое правило «карточка всегда» (тесты `acp_shell_requests_…_requires_a_card`).
const ACP_OWN_SCRIPT_WITHOUT_CARD: bool = false;

/// Сводка разговора для новой сессии агента. Сжимаем тред той же кнопкой, что «Сжать историю»,
/// и берём сводку так же, как её берёт системный промпт. Сводки нет — сессия всё равно новая.
async fn fresh_session_summary(app: &Arc<App>, bot: &Bot, writer: &Bot, thread: &str) -> Option<String> {
    // Сжал — `compact()` сам сказал чату `compacted`, как и кнопка. Сжимать было нечего —
    // событие посылаем здесь: сессия всё равно новая.
    // `Box::pin` разрывает цикл асинхронных вызовов: сжатие может позвать модель, а её ход — снова агента.
    // Сводка — того разговора, которому принадлежит сессия: команды и личного чата не смешиваем.
    let room = thread.strip_prefix("room:").and_then(|id| app.store.room(id));
    let (compacted, summary_of) = match &room {
        Some(room) => (Box::pin(crate::compact::room_thread(app, room, true)).await, crate::store::Thread::Room(&room.id)),
        None => (Box::pin(crate::compact::bot_thread_by(app, bot, writer, true)).await, crate::store::Thread::Bot(&bot.id)),
    };
    if compacted.is_none() {
        let event = match &room {
            Some(room) => json!({ "type": "compacted", "roomId": &room.id, "dropped": 0, "summarized": false }),
            None => json!({ "type": "compacted", "botId": &bot.id, "dropped": 0, "summarized": false }),
        };
        app.turns.emit(event);
    }
    let summary = app.store.thread_summary(summary_of).await;
    match summary {
        Some(text) if !text.trim().is_empty() => Some(text),
        _ => {
            eprintln!("[acp] у бота {} заполнено окно, но сводки нет: начинаю сессию без неё", bot.id);
            None
        }
    }
}

/// Имя нашего MCP-моста в сессии агента: со случайной частью на время работы программы. Агент называет сервер в запросе
/// разрешения только строкой, и посторонний сервер с именем `deloist_browser` (из его собственных настроек) не должен
/// получить поблажку: случайного имени он не знает. Сессия агента живёт дольше одного хода, поэтому имя одно на запуск.
pub(crate) fn bridge_name() -> &'static str {
    static NAME: OnceLock<String> = OnceLock::new();
    NAME.get_or_init(|| format!("deloist_browser_{}", &crate::guard::random_token().chars().filter(|c| c.is_ascii_alphanumeric()).take(16).collect::<String>()))
}

/// Новая сессия или продолжение прежней. `true` — сессия новая, агент ничего не знает о разговоре.
async fn open_session(agent: &Agent, req: &Request<'_>, cancel: &CancellationToken) -> Result<(String, bool), String> {
    let cwd = std::fs::canonicalize(req.workspace).unwrap_or_else(|_| req.workspace.to_path_buf());
    let cwd = plain_path(&cwd);
    let init = agent.init.lock().unwrap().clone();
    let executable = std::env::current_exe().map_err(|e| format!("не найден исполняемый файл Deloist: {e}"))?;
    // Ключ хода в argv не кладём (его видно в списке процессов): в окружение идёт путь к файлу с ключом.
    let servers = json!([{ "name": bridge_name(), "command": executable.to_string_lossy(),
        "args": ["--mcp", "--browser-bot", &req.bot.id],
        "env": [
            { "name": "CREW_DATA_DIR", "value": req.app.store.data_dir().to_string_lossy() },
            { "name": TURN_KEY_ENV, "value": turn_key_path(req.app.store.data_dir(), &req.bot.id).to_string_lossy() },
        ] }]);

    let session_key = (req.bot.id.clone(), req.thread.to_string());
    let previous = known_sessions().lock().unwrap().get(&session_key).cloned().or_else(|| saved_session(req.app.store.data_dir(), &session_key));
    if let Some((signature, id)) = previous {
        if signature == agent.signature && init["agentCapabilities"]["loadSession"] == true {
            // Агент повторит историю уведомлениями: хода нет, поэтому они никуда не попадут.
            let loaded = agent
                .call("session/load", json!({ "sessionId": id, "cwd": cwd, "mcpServers": servers }), SESSION_TIMEOUT)
                .await;
            if let Ok(result) = &loaded {
                // Результат загрузки может нести настройки; если их нет, усилие остаётся прежним.
                set_effort(agent, &id, result, req.spec.effort).await;
                set_options(agent, &id, result, req.spec.options).await;
                return Ok((id, false));
            }
        }
    }

    let mut params = json!({ "cwd": cwd, "mcpServers": servers });
    if supports_system_prompt(&init) {
        params["systemPrompt"] = json!({ "mode": "append", "content": [{ "type": "text", "text": system_text(req.messages) }] });
    }
    let mut authenticated = false;
    loop {
        match agent.call("session/new", params.clone(), SESSION_TIMEOUT).await {
            Ok(result) => {
                let id = result["sessionId"].as_str().ok_or("агент не дал номер сессии")?;
                set_mode(agent, id, &result, req.spec.mode).await?;
                set_model(agent, id, &result, req.spec.model).await?;
                set_effort(agent, id, &result, req.spec.effort).await;
                set_options(agent, id, &result, req.spec.options).await;
                return Ok((id.to_string(), true));
            }
            Err(CallError::Rpc(error)) if needs_auth(&error) && !authenticated => {
                authenticate(agent, &init, &error, req.spec.auth, cancel).await?;
                authenticated = true;
            }
            Err(CallError::Rpc(error)) => return Err(format!("агент не открыл сессию: {}", rpc_message(&error))),
            Err(CallError::Timeout) => return Err(format!("агент не открыл сессию за {} с", SESSION_TIMEOUT.as_secs())),
            Err(CallError::Closed) => {
                let tail = agent.stderr_tail();
                let suffix = if tail.is_empty() { String::new() } else { format!(" Вывод агента: {tail}") };
                return Err(format!("агент закрылся при открытии сессии.{suffix}"));
            }
        }
    }
}

/// Режим из настроек, если агент его знает. Неизвестный режим — понятная ошибка, а не молчаливый режим по умолчанию.
async fn set_mode(agent: &Agent, session_id: &str, session: &Value, mode: &str) -> Result<(), String> {
    let modes = &session["modes"];
    let known: Vec<&str> = modes["availableModes"].as_array().map(|m| m.iter().filter_map(|m| m["id"].as_str()).collect()).unwrap_or_default();
    // Без выбора человека берём режим, в котором агент спрашивает разрешения: тогда решают наши карточки
    // и режим бота. В «авто»-режимах агент правит сам своей песочницей, а на Windows она бывает немой.
    let mode = match mode.trim() {
        "" => match ASKING_MODES.iter().find(|m| known.contains(m)) {
            Some(mode) => *mode,
            None => return Ok(()),
        },
        mode => mode,
    };
    if modes["currentModeId"] == mode {
        return Ok(());
    }
    if !known.contains(&mode) {
        return Err(format!("у агента нет режима «{mode}». Есть: {}", if known.is_empty() { "—".to_string() } else { known.join(", ") }));
    }
    agent
        .call("session/set_mode", json!({ "sessionId": session_id, "modeId": mode }), SESSION_TIMEOUT)
        .await
        .map(|_| ())
        .map_err(|e| match e {
            CallError::Rpc(error) => format!("агент не переключил режим: {}", rpc_message(&error)),
            _ => "агент не переключил режим".to_string(),
        })
}

/// Способ входа, который агент проводит сам (обычно через браузер). Вход по ключу API ждёт ключ,
/// которого у нас нет, поэтому он последний.
fn login_method<'a>(methods: &'a [Value], wanted: &str) -> Option<&'a Value> {
    let usable = |m: &&Value| m["type"].as_str().unwrap_or("agent") == "agent" && m["id"].as_str().is_some();
    let by_key = |m: &&Value| {
        let text = format!("{} {}", m["id"].as_str().unwrap_or_default(), m["name"].as_str().unwrap_or_default()).to_lowercase();
        text.contains("api") || text.contains("key")
    };
    // Вход по ключу: ключ уже в окружении агента, ждать браузер незачем.
    if wanted == "api-key" {
        if let Some(method) = methods.iter().filter(usable).find(|m| by_key(m)) {
            return Some(method);
        }
    }
    // Вход через Google (Antigravity): именно он, а не соседний вход через Vertex.
    if let Some(method) = methods.iter().filter(usable).find(|m| m["id"] == "oauth-personal") {
        return Some(method);
    }
    methods.iter().filter(usable).find(|m| !by_key(m)).or_else(|| methods.iter().find(usable))
}

/// Модели агента: настройка категории `model`, а если её нет — список `models`.
pub fn models_of(session: &Value) -> Vec<Value> {
    if let Some(option) = model_option(session) {
        let list: Vec<Value> = option["options"]
            .as_array()
            .map(|o| {
                o.iter()
                    .filter_map(|m| {
                        let id = m["value"].as_str()?;
                        Some(json!({ "id": id, "name": m["name"].as_str().unwrap_or(id), "description": m["description"] }))
                    })
                    .collect()
            })
            .unwrap_or_default();
        if !list.is_empty() {
            return list;
        }
    }
    session["models"]["availableModels"]
        .as_array()
        .map(|o| {
            o.iter()
                .filter_map(|m| {
                    let id = m["modelId"].as_str()?;
                    Some(json!({ "id": id, "name": m["name"].as_str().unwrap_or(id), "description": m["description"] }))
                })
                .collect()
        })
        .unwrap_or_default()
}

fn model_option(session: &Value) -> Option<&Value> {
    session["configOptions"].as_array()?.iter().find(|o| o["category"] == "model" || o["id"] == "model")
}

async fn set_model(agent: &Agent, session_id: &str, session: &Value, model: &str) -> Result<(), String> {
    let model = model.trim();
    if model.is_empty() {
        return Ok(());
    }
    let result = if let Some(option) = model_option(session) {
        if option["currentValue"] == model {
            return Ok(());
        }
        agent
            .call(
                "session/set_config_option",
                json!({ "sessionId": session_id, "configId": option["id"], "value": model }),
                SESSION_TIMEOUT,
            )
            .await
    } else if session["models"].is_object() {
        if session["models"]["currentModelId"] == model {
            return Ok(());
        }
        agent.call("session/set_model", json!({ "sessionId": session_id, "modelId": model }), SESSION_TIMEOUT).await
    } else {
        return Ok(());
    };
    result.map(|_| ()).map_err(|e| match e {
        CallError::Rpc(error) => format!("агент не переключил модель «{model}»: {}", rpc_message(&error)),
        _ => format!("агент не переключил модель «{model}»"),
    })
}

fn effort_option(session: &Value) -> Option<&Value> {
    session["configOptions"].as_array()?.iter().find(|o| o["category"] == "thought_level")
}

/// Значения усилия, которые агент объявил: `[{id, name}]`. Пусто — у агента такой настройки нет.
pub fn efforts_of(session: &Value) -> Vec<Value> {
    let Some(option) = effort_option(session) else { return Vec::new() };
    option["options"]
        .as_array()
        .map(|o| {
            o.iter()
                .filter_map(|v| {
                    let id = v["value"].as_str()?;
                    Some(json!({ "id": id, "name": v["name"].as_str().unwrap_or(id) }))
                })
                .collect()
        })
        .unwrap_or_default()
}

/// Настройка и значение для `session/set_config_option`, если агент объявил это значение и оно ещё не стоит.
fn effort_call<'a>(session: &'a Value, effort: &str) -> Option<(&'a Value, String)> {
    let effort = effort.trim();
    if effort.is_empty() {
        return None;
    }
    let option = effort_option(session)?;
    if option["currentValue"] == effort || !efforts_of(session).iter().any(|e| e["id"] == effort) {
        return None;
    }
    Some((&option["id"], effort.to_string()))
}

/// Усилие бота. Не объявил агент — молча пропускаем; ошибка смены не должна ронять ход.
async fn set_effort(agent: &Agent, session_id: &str, session: &Value, effort: &str) {
    let Some((config_id, value)) = effort_call(session, effort) else { return };
    let _ = agent
        .call("session/set_config_option", json!({ "sessionId": session_id, "configId": config_id, "value": value }), SESSION_TIMEOUT)
        .await;
}

/// Значение усилия, которое стоит у агента сейчас.
pub fn effort_default_of(session: &Value) -> Option<String> {
    effort_option(session)?["currentValue"].as_str().map(str::to_string)
}

fn is_extra_select(o: &Value) -> bool {
    let kind = o["type"].as_str().unwrap_or("select");
    kind == "select"
        && o["id"].as_str().is_some()
        && !matches!(o["category"].as_str(), Some("model" | "mode" | "thought_level"))
        && o["id"] != "model"
        && o["id"] != "mode"
}

/// Остальные выбираемые настройки агента (скорость и т. п.): `[{id, name, values: [{id, name}], default}]`.
pub fn extra_options_of(session: &Value) -> Vec<Value> {
    let Some(list) = session["configOptions"].as_array() else { return Vec::new() };
    list.iter()
        .filter(|o| is_extra_select(o))
        .filter_map(|o| {
            let id = o["id"].as_str()?;
            let values: Vec<Value> = o["options"]
                .as_array()?
                .iter()
                .filter_map(|v| {
                    let vid = v["value"].as_str()?;
                    Some(json!({ "id": vid, "name": v["name"].as_str().unwrap_or(vid) }))
                })
                .collect();
            if values.is_empty() {
                return None;
            }
            Some(json!({ "id": id, "name": o["name"].as_str().unwrap_or(id), "values": values, "default": o["currentValue"] }))
        })
        .collect()
}

/// Выбор бота по прочим настройкам: ставим только объявленные агентом настройки и значения.
fn options_calls(session: &Value, wanted: &[(String, String)]) -> Vec<(String, String)> {
    let declared = extra_options_of(session);
    let all: Vec<&Value> = session["configOptions"].as_array().map(|l| l.iter().collect()).unwrap_or_default();
    wanted
        .iter()
        .filter(|(id, value)| {
            declared.iter().any(|o| o["id"] == id.as_str() && o["values"].as_array().is_some_and(|v| v.iter().any(|v| v["id"] == value.as_str())))
                && !all.iter().any(|o| o["id"] == id.as_str() && o["currentValue"] == value.as_str())
        })
        .cloned()
        .collect()
}

async fn set_options(agent: &Agent, session_id: &str, session: &Value, wanted: &[(String, String)]) {
    for (id, value) in options_calls(session, wanted) {
        let _ = agent
            .call("session/set_config_option", json!({ "sessionId": session_id, "configId": id, "value": value }), SESSION_TIMEOUT)
            .await;
    }
}

/// Что узнали об агенте пробным запуском: есть ли вход, чей он и какие модели.
pub struct Probe {
    pub connected: bool,
    /// session/new did not respond before the deadline; authentication remains unknown.
    pub timed_out: bool,
    pub account: Value,
    pub models: Vec<Value>,
    /// Усилие рассуждения, которое объявил агент: `[{id, name}]`.
    pub efforts: Vec<Value>,
    pub effort_default: Option<String>,
    /// Прочие выбираемые настройки агента.
    pub options: Vec<Value>,
    pub agent: Value,
    pub error: Option<String>,
    /// Лимиты подписки, если агент умеет `/status` (Codex): `[{label, left, resets}]`.
    pub limits: Vec<Value>,
}

/// Строки `/status` вида `**5h limit:** 86% left (resets 16:01)` (Codex) и `/usage` вида
/// `Current session: 30% used · resets Nov 13, 11:10am (Asia/Bangkok)` (Claude).
pub fn parse_limits(text: &str) -> Vec<Value> {
    let mut out = Vec::new();
    for line in text.lines() {
        let line = line.trim();
        if let Some(limit) = usage_line(line) {
            out.push(limit);
            continue;
        }
        let Some(rest) = line.strip_prefix("**") else { continue };
        let Some((label, value)) = rest.split_once(":**") else { continue };
        let value = value.trim();
        let Some((number, tail)) = value.split_once("% left") else { continue };
        let Ok(left) = number.trim().parse::<i64>() else { continue };
        let resets = tail.trim().trim_start_matches('(').trim_end_matches(')').trim();
        let resets = resets.strip_prefix("resets").map(str::trim).unwrap_or(resets);
        out.push(json!({ "label": label.trim(), "left": left.clamp(0, 100), "resets": resets }));
    }
    out
}

/// Строка `/usage` Claude: «Current session» — окно 5 часов, «Current week (…)» — неделя.
/// Метки приводятся к виду Codex (`5h limit`, `Weekly limit`), чтобы интерфейс читал их одинаково.
fn usage_line(line: &str) -> Option<Value> {
    let (label, value) = line.split_once(':')?;
    let (number, tail) = value.trim().split_once("% used")?;
    let used = number.trim().parse::<i64>().ok()?;
    let label = label.trim();
    let label = if label.eq_ignore_ascii_case("current session") {
        "5h limit".to_string()
    } else if let Some(scope) = label.strip_prefix("Current week") {
        let scope = scope.trim().trim_start_matches('(').trim_end_matches(')').trim();
        if scope.is_empty() || scope.eq_ignore_ascii_case("all models") {
            "Weekly limit".to_string()
        } else {
            format!("Weekly limit · {scope}")
        }
    } else {
        return None;
    };
    let resets = tail.trim().trim_start_matches('·').trim();
    let resets = resets.strip_prefix("resets").map(str::trim).unwrap_or(resets);
    // Часовой пояс в скобках длинный и человеку не нужен: время и так его.
    let resets = resets.split(" (").next().unwrap_or(resets).trim();
    Some(json!({ "label": label, "left": (100 - used).clamp(0, 100), "resets": resets }))
}

/// Лимиты через `/status`: агент отвечает сам, без модели и без расхода подписки.
async fn status_limits(agent: &Agent, session_id: &str, inbox: &mut mpsc::UnboundedReceiver<Value>, seen: &mut Vec<Value>) -> Vec<Value> {
    // Список команд приходит уведомлением сразу после открытия сессии.
    let deadline = tokio::time::Instant::now() + Duration::from_millis(2500);
    // Codex отвечает на `/status`, Claude — на `/usage`; обе команды локальные и подписку не тратят.
    let mut command = None;
    while command.is_none() {
        let message = tokio::select! {
            message = inbox.recv() => message,
            _ = tokio::time::sleep_until(deadline) => None,
        };
        let Some(message) = message else { break };
        let update = &message["params"]["update"];
        if update["sessionUpdate"] == "available_commands_update" {
            let has = |name: &str| update["availableCommands"].as_array().is_some_and(|c| c.iter().any(|c| c["name"] == name));
            command = if has("status") { Some("/status") } else if has("usage") { Some("/usage") } else { None };
        }
        seen.push(message);
    }
    let Some(command) = command else {
        // Grok Build не даёт команды лимитов, но отвечает на своё расширение `_x.ai/billing`
        // (то же, что его `/usage` в терминале; модель не вызывается). Остальные скажут «нет метода».
        return match agent.call("_x.ai/billing", json!({}), Duration::from_secs(10)).await {
            Ok(reply) => billing_limits(&reply),
            Err(_) => Vec::new(),
        };
    };
    let Ok(mut reply) = agent.send_request("session/prompt", json!({ "sessionId": session_id, "prompt": [{ "type": "text", "text": command }] })).await else {
        return Vec::new();
    };
    let mut text = String::new();
    // `/usage` у Claude считает ещё и «на что ушли лимиты» и отвечает за 25–30 с (замер 13.11.2026).
    let deadline = tokio::time::Instant::now() + Duration::from_secs(if command == "/usage" { 60 } else { 20 });
    loop {
        tokio::select! {
            _ = &mut reply => break,
            message = inbox.recv() => {
                let Some(message) = message else { break };
                let update = &message["params"]["update"];
                if update["sessionUpdate"] == "agent_message_chunk" {
                    text.push_str(update["content"]["text"].as_str().unwrap_or_default());
                } else {
                    seen.push(message);
                }
            }
            _ = tokio::time::sleep_until(deadline) => break,
        }
    }
    // Остаток ответа мог прийти раньше самого ответа на prompt.
    while let Ok(message) = inbox.try_recv() {
        let update = &message["params"]["update"];
        if update["sessionUpdate"] == "agent_message_chunk" {
            text.push_str(update["content"]["text"].as_str().unwrap_or_default());
        } else {
            seen.push(message);
        }
    }
    parse_limits(&text)
}

/// Ответ Grok на `_x.ai/billing` → окно лимита: `config.creditUsagePercent` — израсходованная доля
/// текущего периода (неделя или месяц), `config.currentPeriod.end` — когда он сбросится.
fn billing_limits(reply: &Value) -> Vec<Value> {
    let config = &reply["config"];
    let Some(used) = config["creditUsagePercent"].as_f64() else {
        return Vec::new();
    };
    let label = match config["currentPeriod"]["type"].as_str().unwrap_or_default() {
        "USAGE_PERIOD_TYPE_WEEKLY" => "Weekly limit",
        "USAGE_PERIOD_TYPE_MONTHLY" => "Monthly limit",
        "USAGE_PERIOD_TYPE_DAILY" => "Daily limit",
        _ => "Limit",
    };
    let end = config["currentPeriod"]["end"].as_str().or_else(|| config["billingPeriodEnd"].as_str()).unwrap_or_default();
    let resets = chrono::DateTime::parse_from_rfc3339(end)
        .map(|t| {
            let t = t.with_timezone(&chrono::Local);
            let format = if chrono::Timelike::minute(&t) == 0 { "%b %-d, %-I%P" } else { "%b %-d, %-I:%M%P" };
            t.format(format).to_string()
        })
        .unwrap_or_default();
    let left = (100.0 - used).round().clamp(0.0, 100.0) as i64;
    vec![json!({ "label": label, "left": left, "resets": resets })]
}

/// Пробный запуск агента в отдельной папке: initialize и session/new. `login` — если входа нет,
/// попросить агента войти (он откроет браузер) и проверить ещё раз. Процесс после проверки гасится.
pub async fn probe(spec: &Spec<'_>, dir: &Path, login: bool, cancel: &CancellationToken) -> Result<Probe, String> {
    if login {
        clear_signin_link();
    }
    let agent = spawn(spec, format!("probe\u{1}{}", spec.signature()), dir).await?;
    let (tx, mut inbox) = mpsc::unbounded_channel::<Value>();
    *agent.inbox.lock().unwrap() = Some(tx);
    let init = agent.init.lock().unwrap().clone();
    let cwd = plain_path(&std::fs::canonicalize(dir).unwrap_or_else(|_| dir.to_path_buf()));
    let params = json!({ "cwd": cwd, "mcpServers": [] });
    let mut tried_login = false;
    let mut timed_out = false;
    // Агент отдаёт список моделей и без входа (Antigravity), поэтому «сессия открылась» входом не считается:
    // если нужен вход (`auth` задан), входим сразу.
    let mut early: Option<String> = None;
    if login && !spec.auth.is_empty() {
        tried_login = true;
        if let Err(e) = authenticate(&agent, &init, &Value::Null, spec.auth, cancel).await {
            early = Some(e);
        }
    }
    let outcome: Result<Option<Value>, String> = if let Some(e) = early { Err(e) } else { loop {
        let attempt = tokio::select! {
            result = agent.call("session/new", params.clone(), SESSION_TIMEOUT) => result,
            _ = cancel.cancelled() => break Err("остановлено".to_string()),
        };
        match attempt {
            Ok(session) => break Ok(Some(session)),
            Err(CallError::Rpc(error)) if needs_auth(&error) => {
                if !login || tried_login {
                    break Ok(None);
                }
                tried_login = true;
                if let Err(e) = authenticate(&agent, &init, &error, spec.auth, cancel).await {
                    break Err(e);
                }
            }
            Err(CallError::Rpc(error)) => break Err(format!("агент не открыл сессию: {}", rpc_message(&error))),
            Err(CallError::Timeout) => {
                timed_out = true;
                break Err(format!("агент не ответил за {} с", SESSION_TIMEOUT.as_secs()));
            },
            Err(CallError::Closed) => {
                let tail = agent.stderr_tail();
                break Err(if tail.is_empty() { "агент закрылся".into() } else { format!("агент закрылся: {tail}") });
            }
        }
    } };
    let mut seen: Vec<Value> = Vec::new();
    let mut limits = Vec::new();
    if let Ok(Some(session)) = &outcome {
        if let Some(id) = session["sessionId"].as_str() {
            limits = status_limits(&agent, id, &mut inbox, &mut seen).await;
        }
    }
    agent.inbox.lock().unwrap().take();
    while let Ok(message) = inbox.try_recv() {
        seen.push(message);
    }
    let mut account = Value::Null;
    for message in seen {
        if message.get("id").is_some() {
            answer_outside_turn(&agent, &message).await;
        } else if message["method"] == "_auth/status_update" {
            account = message["params"]["authStatus"].clone();
        }
    }
    agent.kill();
    clear_signin_link();
    let info = init["agentInfo"].clone();
    Ok(match outcome {
        Ok(Some(session)) => Probe { connected: true, timed_out, account, models: models_of(&session), efforts: efforts_of(&session), effort_default: effort_default_of(&session), options: extra_options_of(&session), agent: info, error: None, limits },
        Ok(None) => Probe { connected: false, timed_out, account, models: Vec::new(), efforts: Vec::new(), effort_default: None, options: Vec::new(), agent: info, error: None, limits },
        Err(e) => Probe { connected: false, timed_out, account, models: Vec::new(), efforts: Vec::new(), effort_default: None, options: Vec::new(), agent: info, error: Some(e), limits },
    })
}

/// Выход из аккаунта агента (ACP `logout`): агент сам стирает свой вход. Процесс после этого гасится.
pub async fn logout(spec: &Spec<'_>, dir: &Path) -> Result<(), String> {
    let agent = spawn(spec, format!("logout\u{1}{}", spec.signature()), dir).await?;
    let result = agent.call("logout", json!({}), Duration::from_secs(30)).await;
    agent.kill();
    match result {
        Ok(_) => Ok(()),
        Err(CallError::Rpc(e)) => Err(format!("агент не вышел из аккаунта: {}", rpc_message(&e))),
        Err(CallError::Timeout) => Err("агент не ответил на выход из аккаунта".into()),
        Err(CallError::Closed) => Err("агент закрылся при выходе из аккаунта".into()),
    }
}

/// Вход в аккаунт агента. Окно входа открывает сам агент, мы только ждём.
async fn authenticate(agent: &Agent, init: &Value, error: &Value, wanted: &str, cancel: &CancellationToken) -> Result<(), String> {
    let methods = init["authMethods"].as_array().cloned().unwrap_or_default();
    let usable = login_method(&methods, wanted);
    let Some(method) = usable else {
        let names: Vec<&str> = methods.iter().filter_map(|m| m["name"].as_str()).collect();
        let how = if names.is_empty() { rpc_message(error) } else { names.join(", ") };
        return Err(format!("Агенту нужен вход: {how}. Войди в агента в его программе и повтори"));
    };
    let name = method["name"].as_str().unwrap_or_default().to_string();
    let call = agent.call("authenticate", json!({ "methodId": method["id"] }), AUTH_TIMEOUT);
    let result = tokio::select! {
        result = call => result,
        _ = cancel.cancelled() => return Err("остановлено".into()),
    };
    match result {
        Ok(_) => Ok(()),
        Err(CallError::Rpc(e)) => Err(format!("Агенту нужен вход ({name}): {}", rpc_message(&e))),
        Err(CallError::Timeout) => Err(format!("Агенту нужен вход ({name}): вход не завершён за 5 минут")),
        Err(CallError::Closed) => {
            let tail = agent.stderr_tail();
            Err(format!("Агенту нужен вход ({name}): программа закрылась до конца входа{}", if tail.is_empty() { String::new() } else { format!(". Её вывод: {tail}") }))
        }
    }
}

fn supports_system_prompt(init: &Value) -> bool {
    let cap = &init["agentCapabilities"]["sessionCapabilities"]["systemPrompt"];
    !(cap.is_null() || *cap == false)
}

/// Путь без префикса `\\?\`: так его понимают все агенты.
fn plain_path(path: &Path) -> String {
    let text = path.display().to_string();
    text.strip_prefix(r"\\?\").map(str::to_string).unwrap_or(text)
}

fn content_text(content: &Value) -> String {
    match content {
        Value::String(text) => text.clone(),
        Value::Array(parts) => parts
            .iter()
            .filter_map(|p| p["text"].as_str())
            .collect::<Vec<_>>()
            .join("\n"),
        _ => String::new(),
    }
}

fn system_text(messages: &[Value]) -> String {
    let mut text = messages
        .iter()
        .filter(|m| m["role"] == "system")
        .map(|m| content_text(&m["content"]))
        .collect::<Vec<_>>()
        .join("\n\n");
    text.push_str("\n\nДля поиска и чтения информации используй свой поиск в интернете или web_fetch; браузер — только когда нужен вход в аккаунт, клики, формы или страница не читается без JavaScript. Не открывай в браузере Google, Яндекс или DuckDuckGo ради поиска.");
    text.push_str("\n\nСвои файлы правь встроенной правкой файлов агента, а не скриптом PowerShell: так человек видит понятное действие, а не длинную команду.");
    text.push_str(&"\n\nДля сайтов используй инструменты browser_* MCP-сервера deloist_browser: это браузер твоего бота в Deloist. browser_nav, browser_mouse и browser_keyboard уже возвращают свежий снимок страницы — отдельный browser_page snapshot после них не нужен. Для входа и передачи управления вызови browser_human этого сервера; чтобы просто показать человеку экран браузера без ожидания, вызови browser_show_to_human. Если действие ещё выполняется и вернуло job, продолжай ждать через browser_wait с этим job, пока не придёт результат; повторно действие не запускай. Если сервер недоступен, сообщи об ошибке, не подменяй браузер другим.".replace("deloist_browser", bridge_name()));
    text.push_str(&"\n\nТот же MCP-сервер deloist_browser даёт командные инструменты твоего бота: delegate (bot, task) — передать задачу другому боту команды по имени и получить его ответ; list_bots — какие боты есть в команде; remember и recall — долгая память. Если в сообщении человека есть «@Имя», это значит «передай задачу этому боту через delegate» с полной постановкой; не говори, что такого инструмента нет, а вызови его.".replace("deloist_browser", bridge_name()));
    text.push_str(&"\n\nСпросить человека (вопрос, выбор из вариантов, короткая форма) — инструмент ask_user того же сервера deloist_browser: question, choices (до 8 вариантов кнопками) или form; ответ придёт результатом вызова. Не задавай такие вопросы обычным текстом и не ищи этот инструмент по списку: он здесь. Собрать команду из ботов (создать новых или добавить существующих) — инструмент propose_team: он только предлагает состав карточкой, а создаёт его человек кнопкой; карточка и есть вопрос, отдельно через ask_user «создать?» не спрашивай. Программу Deloist по её HTTP-адресам скриптами (curl, PowerShell, Python) не вызывай и ключ программы из файлов не читай: всё, что тебе позволено, есть в инструментах этого сервера.".replace("deloist_browser", bridge_name()));
    text.push_str("\n\nTelegram: открытый канал читай инструментом telegram_posts того же сервера (channel, query — поиск по каналу, before — к старым постам): без браузера, с датами и ссылками на посты. Каналы по теме ищи своим поиском в интернете (site:t.me слово). Чаты, группы и закрытые каналы — в браузере на web.telegram.org: вступать в них и писать там можно только с разрешения человека.");
    text.push_str("\n\nТот же сервер даёт планировщик Deloist: list_routines (расписания ботов), create_routine, update_routine, delete_routine. Просьба «раз в полчаса…», «каждый день в 9…», «через два часа выведи итог» — это рутина: создай её через create_routine (every_minutes или at; в prompt — полная постановка на каждый запуск, потому что рутина проснётся без этого разговора) и назови человеку расписание и время следующего запуска. Разовое «через N часов» тоже ставь рутиной: в её prompt напиши, что после последнего запуска её нужно выключить (update_routine с enabled=false). Не говори, что у тебя нет планировщика, и не предлагай cron или календарь. Создание и удаление рутин человек подтверждает карточкой.");
    text.push_str("\n\nК серверам подключайся именем из ~/.ssh/config (`ssh имя` из блока Host), а не путём к файлу ключа: папка .ssh и сами ключи закрыты песочницей. Если нужного имени нет — спроси человека, какой хост использовать.");
    // Агент ACP выполняет команды сам: {{secret:…}} у него нет, поэтому ключи — только внутри команды.
    text.push_str("\n\nКлючи проекта (.env на компьютере или на сервере): перед чтением Deloist спросит человека карточкой. Значения ключей не печатай ни в ответе, ни в выводе команд: загружай их и используй внутри одной команды (например, ssh <сервер> \"cd <проект> && python3 -c '…load_dotenv()…'\") и выводи только итог, вроде HTTP-кода. Сервер ищи по псевдониму из ~/.ssh/config (оттуда смотри только строки Host), без -i.");
    text
}

/// Сообщения человека после последнего ответа бота: это и есть вопрос хода.
fn latest_user_index(messages: &[Value]) -> usize {
    let mut start = messages.len();
    while start > 0 && messages[start - 1]["role"] == "user" {
        start -= 1;
    }
    start
}

/// `carry` — сводка прошлого разговора: в новой сессии агент не знает ничего, кроме пересказа.
fn prompt_blocks(agent: &Agent, messages: &[Value], fresh: bool, carry: Option<&str>) -> Vec<Value> {
    let init = agent.init.lock().unwrap().clone();
    let images = init["agentCapabilities"]["promptCapabilities"]["image"] == true;
    let start = latest_user_index(messages);
    let mut blocks: Vec<Value> = Vec::new();

    if fresh {
        if let Some(block) = crate::compact::prompt_block(carry) {
            blocks.push(json!({ "type": "text", "text": block }));
        }
        let mut intro: Vec<String> = Vec::new();
        if !supports_system_prompt(&init) {
            let system = system_text(messages);
            if !system.is_empty() {
                intro.push(format!("# Кто ты и как работаешь\n{system}"));
            }
        }
        let history: Vec<&Value> = messages[..start]
            .iter()
            .filter(|m| matches!(m["role"].as_str(), Some("user") | Some("assistant")))
            .filter(|m| !content_text(&m["content"]).trim().is_empty())
            .collect();
        if !history.is_empty() {
            let mut recap = String::from("# Начало разговора (пересказ, отвечать на это не нужно)");
            // С конца: свежие сообщения целиком, старые короче, пока не выйдет бюджет знаков. Раньше шли
            // только последние 12, и всё, что было до них (подбор постов, критерии), терялось.
            let mut lines: Vec<String> = Vec::new();
            let mut used = 0usize;
            for (back, m) in history.iter().rev().take(RECAP_MAX_MESSAGES).enumerate() {
                let who = if m["role"] == "user" { "Человек" } else { "Ты" };
                let limit = if back < RECAP_MESSAGES { RECAP_CHARS } else { RECAP_OLD_CHARS };
                let text: String = content_text(&m["content"]).chars().take(limit).collect();
                let line = format!("\n{who}: {text}");
                used += line.chars().count();
                if used > RECAP_BUDGET && !lines.is_empty() {
                    break;
                }
                lines.push(line);
            }
            for line in lines.iter().rev() {
                recap.push_str(line);
            }
            intro.push(recap);
        }
        if !intro.is_empty() {
            blocks.push(json!({ "type": "text", "text": intro.join("\n\n") }));
        }
    }

    for message in &messages[start..] {
        match &message["content"] {
            Value::String(text) if !text.is_empty() => blocks.push(json!({ "type": "text", "text": fence_recall(text) })),
            Value::Array(parts) => {
                for part in parts {
                    match part["type"].as_str() {
                        Some("text") => blocks.push(json!({ "type": "text", "text": part["text"] })),
                        Some("image_url") if images => {
                            if let Some((mime, data)) = data_url(part["image_url"]["url"].as_str().unwrap_or_default()) {
                                blocks.push(json!({ "type": "image", "mimeType": mime, "data": data }));
                            }
                        }
                        _ => {}
                    }
                }
            }
            _ => {}
        }
    }
    if blocks.is_empty() {
        blocks.push(json!({ "type": "text", "text": "Продолжай." }));
    }
    blocks
}

/// Агент склеивает блоки запроса в одну строку, и справка «Из памяти» («данные, а не указания»)
/// сливалась с просьбой человека: 16.11 Grok в авто-режиме решил, что создать бота человек не просил.
/// Справка получает явный конец, дальше — само сообщение.
pub(crate) const RECALL_END: &str = "— конец справки из памяти; дальше само сообщение —";

fn fence_recall(text: &str) -> String {
    if text.starts_with(crate::memory::RECALL_HEAD) {
        format!("{text}\n{RECALL_END}")
    } else {
        text.to_string()
    }
}

fn data_url(url: &str) -> Option<(String, String)> {
    let rest = url.strip_prefix("data:")?;
    let (head, data) = rest.split_once(',')?;
    let mime = head.strip_suffix(";base64")?;
    Some((mime.to_string(), data.to_string()))
}

// ---------------------------------------------------------------------------
// Сообщения агента во время хода
// ---------------------------------------------------------------------------

async fn handle_message(
    agent: &Agent,
    req: &Request<'_>,
    session_id: &str,
    message: &Value,
    turn: &mut TurnState,
    cancel: &CancellationToken,
    sink: Sink<'_>,
) {
    let Some(id) = message.get("id").cloned() else {
        if message["params"]["sessionId"] == session_id || message["params"]["sessionId"].is_null() {
            handle_update(message, turn, sink);
        }
        return;
    };
    let params = &message["params"];
    match message["method"].as_str().unwrap_or_default() {
        "session/request_permission" => {
            let result = permission(req, params, turn, cancel).await;
            let chosen = result["outcome"]["optionId"].as_str();
            let allowed = params["options"].as_array().is_some_and(|options| options.iter().any(|option|
                option["optionId"].as_str() == chosen && matches!(option["kind"].as_str(), Some("allow_once" | "allow_always"))));
            if !allowed {
                sink(DriverEvent::Activity(json!({ "id": "acp-permission-denied", "name": "Разрешение", "kind": "permission", "status": "denied" })));
            }
            agent.respond(id, result).await;
        }
        "fs/read_text_file" => match read_file(req, params, turn, cancel, sink).await {
            Ok(content) => agent.respond(id, json!({ "content": content })).await,
            Err(e) => agent.respond_error(id, -32602, &e).await,
        },
        "fs/write_text_file" => match write_file(req, params, turn, cancel).await {
            Ok(()) => agent.respond(id, json!({})).await,
            Err(e) => agent.respond_error(id, -32602, &e).await,
        },
        _ => agent.respond_error(id, -32601, "Deloist не умеет этот метод").await,
    }
}

/// Последний `usage_update` агента по сессиям, ключ `(бот, разговор)`: `(занято токенов, окно в токенах)`.
fn usage_store() -> &'static Mutex<HashMap<(String, String), (u64, u64)>> {
    static STORE: OnceLock<Mutex<HashMap<(String, String), (u64, u64)>>> = OnceLock::new();
    STORE.get_or_init(Default::default)
}

/// Заполненность контекста сессии бота в этом разговоре, как её назвал сам агент (`usage_update`): `(used, size)`.
fn reported_usage_in(bot_id: &str, thread: &str) -> Option<(u64, u64)> {
    usage_store().lock().unwrap().get(&(bot_id.to_string(), thread.to_string())).copied()
}

/// Заполненность окна в личном чате бота: её показывает его карточка. Сессии команд считаются отдельно.
pub fn reported_usage(bot_id: &str) -> Option<(u64, u64)> {
    reported_usage_in(bot_id, PERSONAL_THREAD)
}

/// `used` и `size` из уведомления `usage_update`. Окно без размера или с нулём не годится.
fn usage_of(update: &Value) -> Option<(u64, u64)> {
    let used = update["used"].as_u64()?;
    let size = update["size"].as_u64().filter(|s| *s > 0)?;
    Some((used, size))
}

/// Пора ли начинать сессию заново, а не продолжать прежнюю. Размер окна неизвестен (0) — не пора.
fn needs_fresh_session(used: u64, size: u64) -> bool {
    size > 0 && used as f64 / size as f64 >= FRESH_SESSION_AT
}

/// Забыть заполненность окна: после новой сессии агент сообщит её заново, и порог не сработает дважды.
fn forget_usage(bot_id: &str, thread: &str) {
    usage_store().lock().unwrap().remove(&(bot_id.to_string(), thread.to_string()));
}

fn handle_update(message: &Value, turn: &mut TurnState, sink: Sink<'_>) {
    if message["method"] != "session/update" {
        return;
    }
    let update = &message["params"]["update"];
    match update["sessionUpdate"].as_str().unwrap_or_default() {
        "usage_update" => {
            if let Some(usage) = usage_of(update) {
                usage_store().lock().unwrap().insert((turn.bot_id.clone(), turn.thread.clone()), usage);
            }
        }
        "agent_message_chunk" => {
            let content = &update["content"];
            if content["type"] == "text" {
                if let Some(text) = content["text"].as_str().filter(|t| !t.is_empty()) {
                    if turn.wrote && turn.worked {
                        sink(DriverEvent::Text("

".to_string()));
                    }
                    turn.wrote = true;
                    turn.worked = false;
                    sink(DriverEvent::Text(text.to_string()));
                }
            }
        }
        "tool_call" | "tool_call_update" => {
            if let Some(id) = update["toolCallId"].as_str().filter(|id| id.len() <= 256) {
                if update["sessionUpdate"] == "tool_call" {
                    if turn.known_calls.contains_key(id) {
                        turn.known_calls.insert(id.to_string(), KnownCall { identity: None, safe: false });
                    } else if turn.known_calls.len() < 256 {
                        let title_ok = update["title"].as_str().is_none_or(|s| s.len() <= 256);
                        let identity = crate::describe::acp_mcp_tool(update).filter(|(server, tool)| server.len() <= 256 && tool.len() <= 256);
                        let safe = title_ok && update["rawInput"]["command"].is_null() && update["rawInput"]["shell_command"].is_null()
                            && update["locations"].as_array().is_none_or(|v| v.is_empty())
                            && matches!(update["kind"].as_str().unwrap_or("other"), "other" | "execute")
                            && identity.is_some();
                        turn.known_calls.insert(id.to_string(), KnownCall { identity, safe });
                    }
                } else if !update["rawInput"].is_null() || !update["locations"].is_null() || !update["title"].is_null() || !update["kind"].is_null() {
                    // Do not combine potentially contradictory fragments.
                    turn.known_calls.remove(id);
                }
            }
            if let Some(activity) = activity_of(update) {
                turn.worked = true;
                sink(DriverEvent::Activity(activity));
            }
        }
        "plan" => {
            let entries = update["entries"].as_array().cloned().unwrap_or_default();
            if entries.is_empty() {
                return;
            }
            let lines: Vec<String> = entries
                .iter()
                .map(|e| {
                    let mark = match e["status"].as_str() {
                        Some("completed") => "[x]",
                        Some("in_progress") => "[~]",
                        _ => "[ ]",
                    };
                    format!("{mark} {}", e["content"].as_str().unwrap_or_default())
                })
                .collect();
            let done = entries.iter().all(|e| e["status"] == "completed");
            sink(DriverEvent::Activity(json!({
                "id": "acp-plan", "name": "План", "kind": "think",
                "status": if done { "ok" } else { "running" }, "result": lines.join("\n"),
            })));
        }
        _ => {}
    }
}

/// Строка хода работы в чате: то же, что у наших инструментов, но выполняет их агент.
fn activity_of(update: &Value) -> Option<Value> {
    let id = update["toolCallId"].as_str()?;
    let mut out = json!({ "id": format!("acp-{id}") });
    if let Some(title) = update["title"].as_str() {
        out["name"] = json!(title);
    }
    if let Some(kind) = update["kind"].as_str() {
        out["kind"] = json!(kind);
    }
    if let Some(status) = update["status"].as_str() {
        out["status"] = json!(match status {
            "completed" => "ok",
            "failed" => "error",
            _ => "running",
        });
    } else if update["sessionUpdate"] == "tool_call" {
        out["status"] = json!("running");
    }
    let mut args = serde_json::Map::new();
    if let Some(path) = update["locations"].as_array().and_then(|l| l.first()).and_then(|l| l["path"].as_str()) {
        args.insert("path".into(), json!(path));
    }
    if let Some(command) = update["rawInput"]["command"].as_str() {
        args.insert("command".into(), json!(command));
    }
    if let Some(url) = update["rawInput"]["url"].as_str() {
        args.insert("url".into(), json!(url));
    }
    // Короткие поля запроса: по ним чат показывает, что именно искали, нажимали, печатали (без вложенных данных).
    for key in ["action", "query", "q", "text", "keys", "ref", "selector", "label"] {
        match &update["rawInput"][key] {
            Value::String(s) if !s.is_empty() => {
                args.insert(key.into(), json!(s.chars().take(200).collect::<String>()));
            }
            Value::Number(n) => {
                args.insert(key.into(), json!(n.to_string()));
            }
            _ => {}
        }
    }
    if !args.is_empty() {
        out["args"] = Value::Object(args);
    }
    let result = tool_content_text(&update["content"]);
    if !result.is_empty() {
        out["result"] = json!(result);
    }
    Some(out)
}

fn tool_content_text(content: &Value) -> String {
    let Some(items) = content.as_array() else { return String::new() };
    let mut parts: Vec<String> = Vec::new();
    for item in items {
        match item["type"].as_str() {
            Some("content") => {
                if let Some(text) = item["content"]["text"].as_str() {
                    parts.push(text.to_string());
                }
            }
            Some("diff") => {
                let path = item["path"].as_str().unwrap_or_default();
                // Что записано: начало нового текста, чтобы у шага «Записывает файл» был предпросмотр.
                let lower = path.to_lowercase();
                let secretive = [".env", ".pem", ".key", "secret", "token", "credential", "password"].iter().any(|w| lower.contains(w));
                match item["newText"].as_str().filter(|t| !secretive && !t.trim().is_empty()) {
                    Some(text) => parts.push(format!("Правка файла {path}\n{}", text.chars().take(1500).collect::<String>())),
                    None => parts.push(format!("Правка файла {path}")),
                }
            }
            _ => {}
        }
    }
    let text = parts.join("\n");
    text.chars().take(4000).collect()
}

/// Какой из наших инструментов ближе всего к действию агента: по нему решаем, спрашивать ли человека.
fn our_tool_for(kind: &str) -> &'static str {
    match kind {
        "read" | "search" | "think" => "read_file",
        "edit" | "delete" | "move" => "write_file",
        "fetch" => "web_fetch",
        // Команды и всё неизвестное — самое опасное.
        _ => "run_command",
    }
}

fn kind_label(kind: &str) -> &'static str {
    match kind {
        "read" => "чтение файла",
        "search" => "поиск",
        "edit" => "правка файла",
        "delete" => "удаление",
        "move" => "перемещение файла",
        "execute" => "команда",
        "fetch" => "загрузка из интернета",
        "think" => "размышление",
        "switch_mode" => "смена режима",
        _ => "действие",
    }
}

async fn permission(req: &Request<'_>, params: &Value, turn: &mut TurnState, cancel: &CancellationToken) -> Value {
    let cancelled = json!({ "outcome": { "outcome": "cancelled" } });
    if turn.cancelled || cancel.is_cancelled() {
        return cancelled;
    }
    let options = params["options"].as_array().cloned().unwrap_or_default();
    let pick = |kinds: &[&str]| {
        kinds
            .iter()
            .find_map(|kind| options.iter().find(|o| o["kind"] == *kind))
            .and_then(|o| o["optionId"].as_str())
            .map(str::to_string)
    };
    let allow = pick(&["allow_once", "allow_always"]);
    let reject = pick(&["reject_once", "reject_always"]);
    let selected = |id: String| json!({ "outcome": { "outcome": "selected", "optionId": id } });

    let call = &params["toolCall"];
    let kind = call["kind"].as_str().unwrap_or("other");
    let paths: Vec<PathBuf> = call["locations"]
        .as_array()
        .map(|l| l.iter().filter_map(|l| l["path"].as_str()).map(PathBuf::from).collect())
        .unwrap_or_default();

    // Файл с ключами или ключ SSH — карточка `secret` при любом режиме бота, без «всегда».
    let command = match &call["rawInput"]["command"] {
        Value::Array(parts) => parts.iter().filter_map(Value::as_str).collect::<Vec<_>>().join(" "),
        other => other.as_str().unwrap_or_default().to_string(),
    };
    let shell_command = call["rawInput"]["shell_command"].as_str().unwrap_or_default();
    // Antigravity (Gemini) присылает команду только названием: `rawInput` пустой. Проверяем название как команду.
    let title_command = (kind == "execute"
        && call["rawInput"]["command"].is_null()
        && call["rawInput"]["shell_command"].is_null()
        && paths.is_empty()
        && crate::describe::acp_mcp_tool(call).is_none())
    .then(|| call["title"].as_str().unwrap_or_default())
    .unwrap_or_default();
    let touch = crate::keys::command_touch(&command).or_else(|| crate::keys::command_touch(shell_command))
        .or_else(|| crate::keys::command_touch(title_command))
        .or_else(|| paths.iter().find_map(|p| crate::keys::path_touch(&p.to_string_lossy())));
    // Push, выпуск и зеркало агент тоже не делает без человека, в каком бы режиме ни был бот.
    // Metacharacters in an argv element are data, not shell separators. Keep checking
    // past them so an option value cannot hide a following `push` or `release`.
    let release_text = if call["rawInput"]["command"].is_array() {
        command.replace(['&', '|', ';', '\n', '\r'], " ")
    } else { command.clone() };
    let releases = crate::reach::release_command(&release_text).is_some() || crate::reach::release_command(shell_command).is_some()
        || crate::reach::release_command(title_command).is_some();
    // Режим бота берём свежим: человек мог включить «Свободу действий» посреди хода.
    let fresh = req.app.store.bot(&req.bot.id);
    let mut floored = fresh.as_ref().unwrap_or(req.bot).clone();
    // Пол осторожности передачи: человек мог включить «Свободу действий» получателю, но задача пришла от бота, который спрашивает.
    floored.approval = req.app.turns.effective_approval(&req.app.store, &req.bot.id, &floored.approval);
    let bot = &floored;
    if turn.approval_policy.as_ref().is_some_and(|old| crate::tools::stricter(old, &bot.approval) != *old) {
        turn.allowed_paths.clear();
    }
    turn.approval_policy = Some(bot.approval.clone());
    // Наши MCP-инструменты (`deloist_browser`: браузер бота, команда, рутины) приходят в приложение отдельным вызовом,
    // и там действуют обычные правила: сайты, разрешения, карточки. Спрашивать второй раз здесь незачем.
    // Сервер определяется по имени, которое прислал агент; чужой сервер с другим именем (`evil.browser_nav`) идёт по общим правилам.
    let sparse = call["title"].as_str().is_none_or(|s| s.is_empty())
        && (call["rawInput"].is_null() || call["rawInput"].as_object().is_some_and(|o| o.iter().all(|(k, v)|
            matches!(k.as_str(), "arguments" | "args") && v.as_object().is_some_and(|args| args.is_empty()))))
        && (call["locations"].is_null() || call["locations"].as_array().is_some_and(|v| v.is_empty()));
    let known = if sparse { call["toolCallId"].as_str().and_then(|id| turn.known_calls.get(id)) } else { None };
    let safe_known = known.filter(|known| known.safe);
    let mcp = crate::describe::acp_mcp_tool(call).or_else(|| safe_known.and_then(|known| known.identity.clone()));
    let ours = matches!(kind, "other" | "execute") && paths.is_empty()
        && call["locations"].as_array().is_none_or(|v| v.is_empty())
        && call["rawInput"]["command"].is_null() && call["rawInput"]["shell_command"].is_null()
        && mcp.as_ref().is_some_and(|(server, tool)| server == bridge_name() && (crate::tools::is_bridge_tool(tool) || tool == "browser_wait"));
    // Правка файла автозапуска и всего внутри `.git` (хуки, настройки агентов, профили) спрашивается и в своей папке (кроме «решай сам»);
    // защищённые файлы (общая проверка `needs_approval_for`, пол для режима «решай сам») спрашиваются всегда.
    let guarded_edit = matches!(kind, "edit" | "delete" | "move") && paths.iter().any(|p|
        (crate::tools::autorun_path(p) && bot.approval != "auto")
            || crate::tools::needs_approval_for(req.app, bot, "write_file", &json!({"path":p.to_string_lossy()})));
    let conflicting_known = known.is_some() && safe_known.is_none();
    let mut ask = releases || touch.is_some() || guarded_edit || conflicting_known || (!ours && crate::tools::needs_approval(req.app, bot, our_tool_for(kind)));
    // Команда только названием (Antigravity), которая по строгому разбору лишь читает: карточка не нужна, как у своего `run_command`.
    if ask && !releases && touch.is_none() && !conflicting_known && !title_command.is_empty()
        && crate::tools::acp_title_command_is_read_only(req.app, bot, title_command)
    {
        ask = false;
    }
    // Дописать строки данных в свой файл (`cat >> x.csv <<'EOF'`): карточка не нужна, пока файл не защищённый и лежит в своей папке.
    if ask && !releases && touch.is_none() && !conflicting_known && !title_command.is_empty()
        && crate::tools::acp_title_command_is_own_data_write(req.app, bot, title_command)
    {
        ask = false;
    }
    // Правка своего файла командой PowerShell: если строгий разбор доказал, что она только пишет в своих папках, карточка не нужна.
    // Выключено слиянием с main (91e808d): его тесты требуют карточку и для массива аргументов, пока владелец не решит иначе.
    if ACP_OWN_SCRIPT_WITHOUT_CARD && ask && !releases && touch.is_none() && !conflicting_known && kind == "execute" && bot.approval == "ask" && call["rawInput"]["shell_command"].is_null() {
        // Только массив аргументов: строка уходит в оболочку агента (bash, cmd), где `$(…)`, обратные кавычки и `$VAR` раскрываются
        // до PowerShell, и разбор по тексту ей не верит. Любая строковая форма спрашивает.
        let argv: Vec<String> = match &call["rawInput"]["command"] {
            Value::Array(parts) => parts.iter().filter_map(Value::as_str).map(str::to_string).collect(),
            _ => Vec::new(),
        };
        let script = crate::shellplan::script_from_argv(&argv);
        if script.is_some_and(|script| crate::describe::own_space_script(req.app, &bot.id, &script)) {
            ask = false;
        }
    }
    let allowed = if !ask {
        true
    } else {
        let title = match &touch {
            Some(touch) => touch.title(),
            None if mcp.is_some() => "вызов инструмента".to_string(),
            None => kind_label(kind).to_string(),
        };
        let human = crate::describe::acp(req.app, bot, call, &paths);
        let detail = human.details.clone();
        let purpose = crate::describe::purpose_of(&call["rawInput"]);
        let human = crate::describe::Described { purpose, ..human };
        let question = crate::turn::request_confirmation(
            req.app,
            &req.bot.id,
            crate::turn::Confirmation {
                tool: "acp",
                title: &title,
                detail: &detail,
                allow_label: None,
                deny_label: None,
                timeout: crate::turn::APPROVAL_TIMEOUT,
                scope: touch.as_ref().map(|t| t.scope("acp")),
                room: None,
                danger: false,
                sensitive: touch.is_some(),
                remote: crate::grants::Remote::No,
                human: Some(human),
            },
        );
        tokio::select! {
            answer = question => answer,
            _ = cancel.cancelled() => {
                turn.cancelled = true;
                return cancelled;
            }
        }
    };
    if allowed {
        if ask && matches!(kind, "edit" | "delete" | "move") {
            turn.allowed_paths.extend(paths);
        }
        match allow {
            Some(id) => selected(id),
            None => cancelled,
        }
    } else {
        match reject {
            Some(id) => selected(id),
            None => cancelled,
        }
    }
}

/// Путь агента (обычно абсолютный) — в путь внутри воркспейса бота. Всё, что снаружи, — ошибка.
fn inside_workspace(req: &Request<'_>, raw: &str) -> Result<PathBuf, String> {
    let outside = || format!("путь вне папки бота: {raw}");
    if raw.trim().is_empty() {
        return Err("не указан путь".into());
    }
    let workspace = req.workspace;
    let _ = std::fs::create_dir_all(workspace);
    let root = std::fs::canonicalize(workspace).map_err(|e| e.to_string())?;
    let given = Path::new(raw);
    let candidate = if given.is_absolute() { given.to_path_buf() } else { workspace.join(given) };
    // Путь можно задать и через `\\?\`, и через короткие имена: сравниваем настоящие пути.
    let mut existing = candidate.clone();
    let mut rest: Vec<std::ffi::OsString> = Vec::new();
    while std::fs::symlink_metadata(&existing).is_err() {
        let name = existing.file_name().ok_or_else(outside)?.to_os_string();
        rest.push(name);
        existing = existing.parent().ok_or_else(outside)?.to_path_buf();
    }
    let real = std::fs::canonicalize(&existing).map_err(|_| outside())?;
    if real.strip_prefix(&root).is_err() {
        // Checkout бота-разработчика: те же права, что своя папка, но без секретов и запущенной программы.
        let store = &req.app.store;
        let projects = crate::tools::homes(&req.app, &req.bot.id);
        if projects.iter().any(|p| real.starts_with(p)) {
            let mut full = real.clone();
            for part in rest.iter().rev() {
                if !crate::store::plain_file_name(&part.to_string_lossy()) {
                    return Err(outside());
                }
                full.push(part);
            }
            if let Some(reason) = crate::reach::secret_reason(&full, store.data_dir(), workspace) {
                return Err(format!("{raw}: {reason}, сюда боту нельзя"));
            }
            let install = crate::selfdev::install_dir();
            let places = crate::reach::Places { workspace, projects: &projects, data_dir: store.data_dir(), install: install.as_deref() };
            if let Some(reason) = crate::reach::install_reason(&full, &places) {
                return Err(format!("{raw}: {reason}"));
            }
            return Ok(full);
        }
    }
    let relative = real.strip_prefix(&root).map_err(|_| outside())?;
    let mut rel = relative.to_path_buf();
    for part in rest.into_iter().rev() {
        rel.push(part);
    }
    if rel.components().any(|c| matches!(c, Component::Prefix(_) | Component::RootDir)) {
        return Err(outside());
    }
    // Окончательная проверка — та же, что у наших файловых инструментов: `..`, имена устройств, ссылки.
    req.app.store.safe_path(&req.bot.id, &rel.to_string_lossy()).map_err(|_| outside())
}

/// Путь для чтения. Внутри папки бота — как для записи; снаружи — по правилу родного `read_file`
/// (читать можно везде, кроме секретов): движки хранят свою память и заметки вне папки бота, а отказ
/// стоит бота хода и токенов, ничего не защищая. Секреты отсекает `read_target` ниже, запись сюда не распространяется.
fn readable_path(req: &Request<'_>, raw: &str) -> Result<PathBuf, String> {
    match inside_workspace(req, raw) {
        Err(e) if e.starts_with("путь вне папки бота") => {
            let given = Path::new(raw);
            let candidate = if given.is_absolute() { given.to_path_buf() } else { req.workspace.join(given) };
            std::fs::canonicalize(&candidate).map_err(|_| e)
        }
        other => other,
    }
}

async fn read_file(req: &Request<'_>, params: &Value, turn: &mut TurnState, cancel: &CancellationToken, sink: Sink<'_>) -> Result<String, String> {
    let raw = params["path"].as_str().unwrap_or_default();
    let path = readable_path(req, raw)?;
    // Resolve aliases before choosing the shared secret policy, and reject protected paths
    // before offering any approval. Its contents are not read until the decision arrives.
    let real = crate::reach::real_path(&path).ok_or("не удалось проверить путь")?;
    if let Some(touch) = crate::keys::path_touch(&real.to_string_lossy()) {
        let checked = crate::tools::key_file_target(&req.app.store, &req.bot.id, &real.to_string_lossy())?;
        let title = touch.title();
        let detail = format!("Агент хочет прочитать файл с ключами: {}", touch.label);
        let question = crate::turn::request_confirmation(req.app, &req.bot.id, crate::turn::Confirmation {
            tool: "read_file", title: &title, detail: &detail, allow_label: None, deny_label: None,
            timeout: crate::turn::APPROVAL_TIMEOUT, scope: Some(touch.scope("read_file")), room: None, danger: false, sensitive: true, remote: crate::grants::Remote::No, human: None,
        });
        let allowed = tokio::select! {
            answer = question => answer,
            _ = cancel.cancelled() => { turn.cancelled = true; false }
        };
        if !allowed {
            sink(DriverEvent::Activity(json!({ "id": "acp-secret-read-denied", "name": "Разрешение", "kind": "permission", "status": "denied" })));
            return Err("человек не разрешил чтение файла с ключами".into());
        }
        let current = readable_path(req, raw)?;
        let current = crate::reach::real_path(&current).ok_or("не удалось повторно проверить путь")?;
        let current = crate::tools::key_file_target(&req.app.store, &req.bot.id, &current.to_string_lossy())?;
        if current != checked { return Err("путь к файлу изменился во время подтверждения".into()); }
        let meta = tokio::fs::metadata(&checked).await.map_err(|e| e.to_string())?;
        if meta.len() > MAX_READ { return Err(format!("файл больше {MAX_READ} байт")); }
        let bytes = tokio::fs::read(&checked).await.map_err(|e| e.to_string())?;
        let (shown, count) = crate::keys::take_values(req.app, &req.bot.id, &String::from_utf8_lossy(&bytes))?;
        return Ok(crate::sensitive::mask(&format!("{shown}\n\n{}", crate::keys::saved_note(count)), false));
    }
    let path = crate::tools::read_target(&req.app.store, &req.bot.id, &real.to_string_lossy())?;
    let meta = tokio::fs::metadata(&path).await.map_err(|e| format!("не смог прочитать файл: {e}"))?;
    if meta.len() > MAX_READ {
        return Err(format!("файл больше {MAX_READ} байт"));
    }
    let bytes = tokio::fs::read(&path).await.map_err(|e| format!("не смог прочитать файл: {e}"))?;
    let text = String::from_utf8_lossy(&bytes).into_owned();
    let line = params["line"].as_u64().unwrap_or(1).max(1) as usize;
    let limit = params["limit"].as_u64().map(|l| l as usize);
    if line == 1 && limit.is_none() {
        return Ok(text);
    }
    let lines = text.split_inclusive('\n').skip(line - 1);
    Ok(match limit {
        Some(limit) => lines.take(limit).collect(),
        None => lines.collect(),
    })
}

async fn write_file(req: &Request<'_>, params: &Value, turn: &mut TurnState, cancel: &CancellationToken) -> Result<(), String> {
    let raw = params["path"].as_str().unwrap_or_default();
    let path = inside_workspace(req, raw)?;
    let content = params["content"].as_str().ok_or("нет содержимого файла")?;
    let mut bot = req.app.store.bot(&req.bot.id).unwrap_or_else(|| req.bot.clone());
    bot.approval = req.app.turns.effective_approval(&req.app.store, &bot.id, &bot.approval);
    if turn.approval_policy.as_ref().is_some_and(|old| crate::tools::stricter(old, &bot.approval) != *old) {
        turn.allowed_paths.clear();
    }
    turn.approval_policy = Some(bot.approval.clone());
    let special = crate::tools::special_write_guard(req.app, &bot.id, &path.to_string_lossy());
    // Файл автозапуска и всё внутри `.git` спрашиваются и в своей папке (кроме «решай сам»), даже если путь уже разрешён в этом ходе.
    let guarded = crate::tools::autorun_path(&path) && bot.approval != "auto";
    let granted = !special && !guarded && turn.allowed_paths.iter().any(|p| inside_workspace(req, &p.to_string_lossy()).ok().as_deref() == Some(path.as_path()));
    if !granted && (guarded || crate::tools::needs_approval_for(req.app, &bot, "write_file", &json!({"path": path.to_string_lossy()}))) {
        let human = crate::describe::call(req.app, &bot, "write_file", &json!({ "path": raw, "content": content }));
        let detail = format!("Агент хочет записать файл: {raw}\n{} символов", content.chars().count());
        let question = crate::turn::request_confirmation(
            req.app,
            &req.bot.id,
            crate::turn::Confirmation {
                tool: "write_file",
                title: "запись файла",
                detail: &detail,
                allow_label: None,
                deny_label: None,
                timeout: crate::turn::APPROVAL_TIMEOUT,
                scope: None,
                room: None,
                danger: false,
                sensitive: false,
                remote: crate::grants::Remote::No,
                human: Some(human),
            },
        );
        let allowed = tokio::select! {
            answer = question => answer,
            _ = cancel.cancelled() => {
                turn.cancelled = true;
                false
            }
        };
        if !allowed {
            return Err("человек не разрешил запись файла".into());
        }
    }
    if let Some(parent) = path.parent() {
        tokio::fs::create_dir_all(parent).await.map_err(|e| e.to_string())?;
    }
    tokio::fs::write(&path, content).await.map_err(|e| format!("не смог записать файл: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn duplicate_tool_call_invalidates_identity_even_when_cache_is_full() {
        let mut turn = TurnState { thread: "bot".into(), bot_id: "test".into(), allowed_paths: HashSet::new(), approval_policy: None,
            known_calls: HashMap::new(), cancelled: false, wrote: false, worked: false };
        for i in 0..256 {
            turn.known_calls.insert(i.to_string(), KnownCall { identity: Some((bridge_name().to_string(), "browser_page".into())), safe: true });
        }
        let message = json!({"method":"session/update", "params":{"update":{"sessionUpdate":"tool_call", "toolCallId":"0", "kind":"execute", "rawInput":{"command":"danger"}}}});
        handle_update(&message, &mut turn, &mut |_| {});
        assert_eq!(turn.known_calls.len(), 256);
        assert!(!turn.known_calls["0"].safe);
        assert!(turn.known_calls["0"].identity.is_none());
    }

    #[test]
    fn grok_billing_becomes_a_weekly_window() {
        // Форма ответа Grok Build 1.0.84 на `_x.ai/billing` (снято 13.11.2026, без ид и почты).
        let reply = json!({ "config": { "creditUsagePercent": 11,
            "currentPeriod": { "type": "USAGE_PERIOD_TYPE_WEEKLY", "start": "2026-11-07T04:34:19.263+00:00", "end": "2026-11-14T04:34:19.263+00:00" },
            "prepaidBalance": { "val": 0 }, "isUnifiedBillingUser": true }, "subscription_tier": "SuperGrok" });
        let limits = billing_limits(&reply);
        assert_eq!(limits.len(), 1);
        assert_eq!(limits[0]["label"], "Weekly limit");
        assert_eq!(limits[0]["left"], 89);
        assert!(limits[0]["resets"].as_str().unwrap().starts_with("Nov 1"), "{}", limits[0]);
        assert!(billing_limits(&json!({ "config": {} })).is_empty());
        assert_eq!(billing_limits(&json!({ "config": { "creditUsagePercent": 140, "currentPeriod": { "type": "USAGE_PERIOD_TYPE_MONTHLY" } } }))[0]["left"], 0);
    }

    fn sample_session() -> Value {
        json!({
            "sessionId": "s1",
            "configOptions": [
                { "id": "model", "name": "Model", "category": "model", "type": "select", "currentValue": "gpt-6",
                  "options": [{ "value": "gpt-6", "name": "GPT-6" }] },
                { "id": "reasoning_effort", "name": "Усилие", "category": "thought_level", "type": "select", "currentValue": "medium",
                  "options": [{ "value": "low", "name": "Low" }, { "value": "medium", "name": "Medium" }, { "value": "high", "name": "High" }] },
                { "id": "speed", "name": "Скорость", "type": "select", "currentValue": "standard",
                  "options": [{ "value": "standard", "name": "Standard" }, { "value": "fast", "name": "Fast" }] },
            ],
        })
    }

    #[test]
    fn browser_actions_are_described_as_returning_a_snapshot() {
        let text = system_text(&[json!({ "role": "system", "content": "база" })]);
        assert!(text.contains("browser_nav, browser_mouse и browser_keyboard уже возвращают свежий снимок страницы"), "{text}");
        assert!(text.contains("отдельный browser_page snapshot после них не нужен"), "{text}");
    }

    #[test]
    fn telegram_channels_are_read_without_the_browser() {
        let text = system_text(&[json!({ "role": "system", "content": "база" })]);
        assert!(text.contains("telegram_posts"), "{text}");
        assert!(crate::tools::TEAM_TOOLS.contains(&"telegram_posts"), "агент ACP получает инструмент через MCP");
        assert!(crate::tools::is_bridge_tool("telegram_posts"));
    }

    #[test]
    fn the_recall_block_is_fenced_off_from_the_message() {
        let block = format!("{}\n[m7] дела: срез", crate::memory::RECALL_HEAD);
        assert_eq!(fence_recall(&block), format!("{block}\n{RECALL_END}"));
        assert_eq!(fence_recall("Сделай нового бота"), "Сделай нового бота");
        let text = system_text(&[json!({ "role": "system", "content": "база" })]);
        assert!(text.contains("отдельно через ask_user «создать?» не спрашивай"), "{text}");
    }

    #[test]
    fn the_agent_is_told_about_ask_user_and_not_to_script_the_api() {
        let text = system_text(&[json!({ "role": "system", "content": "база" })]);
        assert!(text.contains("ask_user") && text.contains("propose_team"), "{text}");
        assert!(text.contains("HTTP-адресам скриптами") && text.contains("какие боты есть"), "{text}");
        assert!(crate::tools::is_bridge_tool("ask_user"));
    }

    #[test]
    fn codex_gets_live_web_search_unless_the_human_configured_it() {
        let live = native_search_env("node C:\\data\\agents\\codex-acp\\node_modules\\.bin\\codex-acp", &[], false).expect("codex-acp");
        assert_eq!(live.0, "CODEX_CONFIG");
        assert_eq!(serde_json::from_str::<Value>(&live.1).unwrap(), json!({ "web_search": "live" }));
        assert!(native_search_env("npx -y @agentclientprotocol/codex-acp", &[], false).is_some());
        assert!(native_search_env("codex-acp", &[("CODEX_CONFIG".into(), "{}".into())], false).is_none());
        assert!(native_search_env("codex-acp", &[], true).is_none());
        assert!(native_search_env("claude-agent-acp", &[], false).is_none());
    }

    #[test]
    fn agent_web_fetch_needs_no_card_in_ask_and_auto_but_strict_asks() {
        assert_eq!(our_tool_for("fetch"), "web_fetch");
        for tool in ["web_fetch", "web_search"] {
            assert!(!crate::tools::policy_needs_approval("ask", tool, false, false), "{tool} ask");
            assert!(!crate::tools::policy_needs_approval("auto", tool, false, false), "{tool} auto");
            assert!(crate::tools::policy_needs_approval("strict", tool, false, false), "{tool} strict");
        }
    }

    #[test]
    fn usage_update_gives_used_and_size() {
        assert_eq!(usage_of(&json!({ "sessionUpdate": "usage_update", "used": 12000, "size": 200000 })), Some((12000, 200000)));
        assert_eq!(usage_of(&json!({ "used": 5, "size": 0 })), None);
        assert_eq!(usage_of(&json!({ "size": 100 })), None);
    }

    #[test]
    fn window_usage_is_tracked_per_conversation() {
        usage_store().lock().unwrap().insert(("ub".into(), "bot".into()), (10, 100));
        usage_store().lock().unwrap().insert(("ub".into(), "room:r1".into()), (90, 100));
        assert_eq!(reported_usage("ub"), Some((10, 100)), "личное окно — по личной сессии");
        assert!(!needs_fresh_session(10, 100));
        assert_eq!(reported_usage_in("ub", "room:r1"), Some((90, 100)));
        forget_usage("ub", "room:r1");
        assert_eq!(reported_usage_in("ub", "room:r1"), None);
        assert_eq!(reported_usage("ub"), Some((10, 100)), "забыть команду — не трогать личную сессию");
        forget_usage("ub", "bot");
    }
    #[test]
    fn sessions_survive_a_restart_through_the_file() {
        let dir = std::env::temp_dir().join(format!("acp-sessions-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let key = ("bot_1".to_string(), "main".to_string());
        assert_eq!(saved_session(&dir, &key), None);
        save_session(&dir, &key, Some(("sig".into(), "sess-7".into())));
        save_session(&dir, &("bot_2".into(), "main".into()), Some(("sig".into(), "sess-8".into())));
        assert_eq!(saved_session(&dir, &key), Some(("sig".into(), "sess-7".into())));
        save_session(&dir, &key, None);
        assert_eq!(saved_session(&dir, &key), None);
        assert_eq!(saved_session(&dir, &("bot_2".into(), "main".into())), Some(("sig".into(), "sess-8".into())));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_full_window_needs_a_fresh_session() {
        assert!(!needs_fresh_session(84, 100), "84% — ещё можно продолжать");
        assert!(needs_fresh_session(85, 100), "85% — пора начинать заново");
        assert!(needs_fresh_session(170_000, 200_000));
        assert!(!needs_fresh_session(900, 0), "размер окна неизвестен");
        assert!(!needs_fresh_session(0, 0));
    }

    #[test]
    fn effort_and_extra_options_are_parsed_from_session_new() {
        let s = sample_session();
        assert_eq!(efforts_of(&s), vec![json!({ "id": "low", "name": "Low" }), json!({ "id": "medium", "name": "Medium" }), json!({ "id": "high", "name": "High" })]);
        assert_eq!(effort_default_of(&s).as_deref(), Some("medium"));
        let extra = extra_options_of(&s);
        assert_eq!(extra.len(), 1, "модель и усилие не должны попасть в прочие настройки");
        assert_eq!(extra[0]["id"], "speed");
        assert_eq!(extra[0]["name"], "Скорость");
        assert_eq!(extra[0]["default"], "standard");
        assert_eq!(extra[0]["values"][1], json!({ "id": "fast", "name": "Fast" }));
        assert!(efforts_of(&json!({})).is_empty());
        assert!(extra_options_of(&json!({})).is_empty());
    }

    #[test]
    fn only_declared_effort_values_are_sent() {
        let s = sample_session();
        let (config, value) = effort_call(&s, "high").unwrap();
        assert_eq!((config.as_str(), value.as_str()), (Some("reasoning_effort"), "high"));
        assert!(effort_call(&s, "ultra").is_none(), "значения, которого агент не объявил, нет");
        assert!(effort_call(&s, "medium").is_none(), "уже стоит");
        assert!(effort_call(&s, "").is_none());
        assert!(effort_call(&json!({ "configOptions": [] }), "high").is_none(), "агент не объявил усилие");
    }

    #[test]
    fn only_declared_extra_options_are_applied() {
        let s = sample_session();
        let wanted = vec![
            ("speed".to_string(), "fast".to_string()),
            ("speed".to_string(), "warp".to_string()),
            ("nope".to_string(), "x".to_string()),
            ("model".to_string(), "gpt-6".to_string()),
        ];
        assert_eq!(options_calls(&s, &wanted), vec![("speed".to_string(), "fast".to_string())]);
    }

    #[test]
    fn activity_maps_status_and_target() {
        let update = json!({
            "sessionUpdate": "tool_call", "toolCallId": "t1", "title": "Читаю заметки", "kind": "read",
            "status": "pending", "locations": [{ "path": "C:\\w\\notes.txt" }],
        });
        let a = activity_of(&update).unwrap();
        assert_eq!(a["id"], "acp-t1");
        assert_eq!(a["status"], "running");
        assert_eq!(a["args"]["path"], "C:\\w\\notes.txt");
        let search = activity_of(&json!({ "sessionUpdate": "tool_call", "toolCallId": "t2", "rawInput": { "query": "цены", "ref": 7, "nested": { "a": 1 } } })).unwrap();
        assert_eq!(search["args"]["query"], "цены");
        assert_eq!(search["args"]["ref"], "7");
        assert!(search["args"].get("nested").is_none());
        let done = activity_of(&json!({ "sessionUpdate": "tool_call_update", "toolCallId": "t1", "status": "completed" })).unwrap();
        assert_eq!(done["status"], "ok");
        assert!(done.get("name").is_none(), "обновление не стирает заголовок");
    }

    #[test]
    fn an_edit_diff_carries_a_bounded_preview_of_the_new_text() {
        let update = json!({
            "sessionUpdate": "tool_call_update", "toolCallId": "t3", "kind": "edit", "status": "completed",
            "content": [{ "type": "diff", "path": "C:\\w\\rules.md", "oldText": null, "newText": "# Rules\nOne post, one idea." }],
        });
        let a = activity_of(&update).unwrap();
        assert_eq!(a["result"], "Правка файла C:\\w\\rules.md\n# Rules\nOne post, one idea.");
        let long = "x".repeat(5000);
        let big = activity_of(&json!({ "sessionUpdate": "tool_call_update", "toolCallId": "t4", "content": [{ "type": "diff", "path": "a.json", "newText": long }] })).unwrap();
        assert!(big["result"].as_str().unwrap().chars().count() < 1600);
        let none = activity_of(&json!({ "sessionUpdate": "tool_call_update", "toolCallId": "t5", "content": [{ "type": "diff", "path": "a.json" }] })).unwrap();
        assert_eq!(none["result"], "Правка файла a.json");
        let secret = activity_of(&json!({ "sessionUpdate": "tool_call_update", "toolCallId": "t6", "content": [{ "type": "diff", "path": ".env", "newText": "KEY=abc" }] })).unwrap();
        assert_eq!(secret["result"], "Правка файла .env");
    }

    #[test]
    fn the_question_is_everything_after_the_last_answer() {
        let messages = vec![
            json!({ "role": "system", "content": "s" }),
            json!({ "role": "user", "content": "раз" }),
            json!({ "role": "assistant", "content": "ответ" }),
            json!({ "role": "user", "content": "время" }),
            json!({ "role": "user", "content": "два" }),
        ];
        assert_eq!(latest_user_index(&messages), 3);
    }

    #[test]
    fn unknown_actions_are_treated_as_commands() {
        assert_eq!(our_tool_for("execute"), "run_command");
        assert_eq!(our_tool_for("other"), "run_command");
        assert_eq!(our_tool_for("edit"), "write_file");
        assert_eq!(our_tool_for("read"), "read_file");
    }

    #[test]
    fn status_lines_become_limits() {
        let text = "**Account:** ChatGPT plus (a@b.c)  \n**5h limit:** 86% left (resets 16:01)  \n**Weekly limit:** 57% left (resets Nov 12)\n**Context window:** 3% used";
        let limits = parse_limits(text);
        assert_eq!(limits.len(), 2);
        assert_eq!(limits[0]["label"], "5h limit");
        assert_eq!(limits[0]["left"], 86);
        assert_eq!(limits[0]["resets"], "16:01");
        assert_eq!(limits[1]["left"], 57);
    }

    #[test]
    fn claude_usage_lines_become_limits() {
        let text = "You are currently using your subscription to power your Claude Code usage\n\nCurrent session: 30% used · resets Nov 13, 11:10am (Asia/Bangkok)\nCurrent week (all models): 55% used · resets Nov 15, 4pm (Asia/Bangkok)\nCurrent week (Fable): 14% used · resets Nov 15, 4pm (Asia/Bangkok)\n\nLast 24h · 4304 requests · 20 sessions\n  85%  Give subagents a cheaper model";
        let limits = parse_limits(text);
        assert_eq!(limits.len(), 3);
        assert_eq!(limits[0], json!({ "label": "5h limit", "left": 70, "resets": "Nov 13, 11:10am" }));
        assert_eq!(limits[1], json!({ "label": "Weekly limit", "left": 45, "resets": "Nov 15, 4pm" }));
        assert_eq!(limits[2]["label"], "Weekly limit · Fable");
    }

    #[test]
    fn data_urls_become_image_blocks() {
        assert_eq!(data_url("data:image/png;base64,AAAA"), Some(("image/png".into(), "AAAA".into())));
        assert_eq!(data_url("https://x/y.png"), None);
    }
}
