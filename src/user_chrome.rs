//! «Мой Chrome»: бот работает в настоящем браузере человека через расширение Deloist.
//!
//! Расширение подключает `chrome.debugger` к вкладкам бота и изображает браузерный уровень CDP
//! (`Target.createTarget`, `Target.attachedToTarget`…). Здесь из этого получается обычный
//! [`Cdp`]: остальной код браузера (`browser.rs`, `browser_tools.rs`, экран бота) не знает,
//! что говорит с Chrome человека. Между `Cdp` и расширением стоит фильтр — вся политика здесь,
//! в программе, а не в расширении:
//!
//! - сайт: каждый origin — отдельное согласие человека для бота (`user_browser`); вкладка на
//!   неразрешённом сайте не читается, а переход верхнего фрейма туда обрывается до загрузки;
//! - методы CDP: только белый список того, чем пользуется наш код; cookies, хранилище,
//!   автозаполнение и код, который их читает, закрыты, пока человек не разрешит один вызов;
//! - без человека (рутина, ночь): только если включено боту и только на сайтах с правилом «всегда».

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};
use std::sync::{Arc, LazyLock, Mutex, OnceLock, Weak};
use std::time::Duration;

use axum::extract::{Path as UrlPath, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post, put};
use axum::{Json, Router};
use regex::Regex;
use serde_json::{json, Value};
use tokio::sync::mpsc;

use crate::cdp::Cdp;
use crate::grants::Scope;
use crate::native_host::{self, FROM_EXTENSION_MAX, HELLO_MAX, TO_EXTENSION_MAX};
use crate::pipe_auth::Action;
#[allow(unused_imports)]
pub use crate::native_host::hmac_sha256;
use crate::store::{now, Bot, Store};
use crate::App;

/// Инструмент разрешений «сайт в Chrome человека».
pub const TOOL: &str = "user_browser";
/// Ключи в `extra`: у бота — режим и работа без человека, в настройках — выбор по умолчанию.
const BOT_MODE: &str = "browserMode";
const BOT_UNATTENDED: &str = "userChromeUnattended";
const SETTINGS_KEY: &str = "userChrome";

pub const NOT_CONNECTED: &str = "Бот настроен работать в твоём Chrome, но расширение Deloist сейчас не подключено. Скажи человеку: открыть Chrome с расширением Deloist или Настройки → «Мой Chrome» → «Подключить».";

// ---------------------------------------------------------------------------
// Настройки
// ---------------------------------------------------------------------------

/// Режим бота: `own` — свой отдельный браузер, `mine` — Chrome человека, пусто — как по умолчанию.
pub fn bot_mode(bot: &Bot) -> &str {
    match bot.extra.get(BOT_MODE).and_then(Value::as_str) {
        Some("mine") => "mine",
        Some("own") => "own",
        _ => "",
    }
}

pub fn default_mode(_store: &Store) -> String {
    // Глобального «по умолчанию в моём Chrome» больше нет: только явно у бота.
    "own".into()
}

/// Работать ли этому боту в Chrome человека.
pub fn wanted(store: &Store, bot_id: &str) -> bool {
    let Some(bot) = store.bot(bot_id) else { return false };
    match bot_mode(&bot) {
        "mine" => true,
        "own" => false,
        _ => default_mode(store) == "mine",
    }
}

pub fn unattended_allowed(bot: &Bot) -> bool {
    bot.extra.get(BOT_UNATTENDED).and_then(Value::as_bool).unwrap_or(false)
}

// ---------------------------------------------------------------------------
// Политика: какие вызовы CDP проходят
// ---------------------------------------------------------------------------

#[derive(Debug, PartialEq)]
pub enum Verdict {
    Allow,
    /// Секрет: cookies, хранилище, автозаполнение. Проходит только по явному разрешению на один вызов.
    Secret,
    Refuse(String),
}

/// Закрыто всегда, что бы ни было в белом списке. Открывает только явное разрешение на один вызов.
static SECRET_METHOD: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"^(Storage|DOMStorage|IndexedDB|CacheStorage|Autofill)\.|^Network\.(getCookies|getAllCookies|setCookies?|deleteCookies|clearBrowserCookies|getRequestPostData)$|^Page\.(getCookies|addScriptToEvaluateOnNewDocument)$|^Runtime\.(addBinding|compileScript)$",
    )
    .unwrap()
});

/// Код, который читает секреты страницы. Поиск по словам, без учёта регистра.
static SECRET_CODE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)cookie|localstorage|sessionstorage|indexeddb|cookiestore|\bcaches\b|credentials").unwrap());

/// Браузерный уровень, который изображает расширение.
const BROWSER_METHODS: &[&str] = &[
    "Target.setDiscoverTargets",
    "Target.setAutoAttach",
    "Target.createTarget",
    "Target.closeTarget",
    "Target.detachFromTarget",
    "Browser.setDownloadBehavior",
    "Browser.close",
];

/// Вкладка: ровно то, чем пользуется наш код (`browser.rs`, `browser_tools.rs`, `screen.rs`, `teach.rs`).
const TAB_METHODS: &[&str] = &[
    "Accessibility.getFullAXTree",
    "Browser.getWindowForTarget",
    "Browser.setWindowBounds",
    "DOM.enable",
    "DOM.getContentQuads",
    "DOM.getDocument",
    "DOM.getOuterHTML",
    "DOM.resolveNode",
    "DOM.scrollIntoViewIfNeeded",
    "DOM.setFileInputFiles",
    "Debugger.enable",
    "Debugger.evaluateOnCallFrame",
    "Debugger.getScriptSource",
    "Debugger.pause",
    "Debugger.removeBreakpoint",
    "Debugger.resume",
    "Debugger.searchInContent",
    "Debugger.setBreakpointByUrl",
    "Debugger.stepInto",
    "Debugger.stepOut",
    "Debugger.stepOver",
    "Emulation.clearDeviceMetricsOverride",
    "Emulation.setDeviceMetricsOverride",
    "Fetch.continueRequest",
    "Fetch.enable",
    "Fetch.failRequest",
    "Input.dispatchKeyEvent",
    "Input.dispatchMouseEvent",
    "Input.insertText",
    "Log.enable",
    "Network.enable",
    "Network.getResponseBody",
    "Page.bringToFront",
    "Page.captureScreenshot",
    "Page.enable",
    "Page.getFrameTree",
    "Page.getLayoutMetrics",
    "Page.getNavigationHistory",
    "Page.handleJavaScriptDialog",
    "Page.navigate",
    "Page.navigateToHistoryEntry",
    "Page.reload",
    "Page.screencastFrameAck",
    "Page.startScreencast",
    "Page.stopLoading",
    "Page.stopScreencast",
    "Runtime.callFunctionOn",
    "Runtime.enable",
    "Runtime.evaluate",
    "Runtime.getProperties",
    "Runtime.runIfWaitingForDebugger",
];

/// Пока вкладка стоит на неразрешённом сайте, проходят только эти: уйти оттуда и не мешать загрузке.
const LEAVE_METHODS: &[&str] = &[
    "Page.navigate",
    "Page.navigateToHistoryEntry",
    "Page.getNavigationHistory",
    "Page.stopLoading",
    "Page.enable",
    "Runtime.enable",
    "Log.enable",
    "Network.enable",
    "DOM.enable",
    "Fetch.enable",
    "Fetch.continueRequest",
    "Fetch.failRequest",
    "Runtime.runIfWaitingForDebugger",
    "Page.screencastFrameAck",
    "Page.stopScreencast",
    "Browser.getWindowForTarget",
    "Browser.setWindowBounds",
    "Page.bringToFront",
    "Debugger.resume",
];

/// Текст кода в вызове, если вызов исполняет код.
fn code_of(method: &str, params: &Value) -> Option<String> {
    match method {
        "Runtime.evaluate" | "Debugger.evaluateOnCallFrame" => params["expression"].as_str().map(str::to_string),
        "Runtime.callFunctionOn" => {
            let mut code = params["functionDeclaration"].as_str().unwrap_or_default().to_string();
            // Аргументы — тоже данные функции: `f(x){return document[x]}` с x="cookie".
            if let Some(args) = params["arguments"].as_array() {
                code.push(' ');
                code.push_str(&Value::Array(args.clone()).to_string());
            }
            Some(code)
        }
        "Debugger.setBreakpointByUrl" => params["condition"].as_str().map(str::to_string),
        _ => None,
    }
}

/// Что делать с вызовом. `browser_level` — без `sessionId`, к самому «браузеру».
pub fn classify(method: &str, params: &Value, browser_level: bool) -> Verdict {
    if SECRET_METHOD.is_match(method) {
        return Verdict::Secret;
    }
    // Fetch.* боту недоступен: перехват навигации делает расширение, а не бот.
    if method.starts_with("Fetch.") {
        return Verdict::Refuse("Fetch.* в Chrome человека боту недоступен.".into());
    }
    if method == "Fetch.disable" {
        return Verdict::Refuse("Fetch.disable в Chrome человека запрещён.".into());
    }
    let listed = if browser_level { BROWSER_METHODS } else { TAB_METHODS };
    if !listed.contains(&method) {
        return Verdict::Refuse(format!("{method} недоступен в Chrome человека: бот пользуется там только своими вкладками и обычными действиями."));
    }
    if code_of(method, params).is_some_and(|code| SECRET_CODE.is_match(&code)) {
        return Verdict::Secret;
    }
    Verdict::Allow
}

pub const SECRET_REFUSED: &str = "Cookies, хранилище сайта и пароли браузера в Chrome человека боту закрыты. Если они правда нужны, человек разрешит один раз: browser_devtools cookies или storage покажут карточку.";

/// Место для разрешений: схема, хост и порт; только http(s).
pub fn origin_of(url: &str) -> Option<String> {
    let parsed = reqwest::Url::parse(url).ok()?;
    if !matches!(parsed.scheme(), "http" | "https") || parsed.host_str().is_none() {
        return None;
    }
    Some(parsed.origin().ascii_serialization())
}

static DENY_PATTERNS: LazyLock<Vec<String>> = LazyLock::new(|| {
    serde_json::from_str::<Value>(include_str!("../extension/deny.json"))
        .ok()
        .and_then(|v| v.get("deny").and_then(Value::as_array).cloned())
        .unwrap_or_default()
        .into_iter()
        .filter_map(|v| v.as_str().map(str::to_string))
        .collect()
});

fn host_matches_pattern(host: &str, pattern: &str) -> bool {
    let host = host.to_lowercase().trim_end_matches('.').to_string();
    let pattern = pattern.to_lowercase().trim_end_matches('.').to_string();
    if let Some(suffix) = pattern.strip_prefix("*.") {
        let suffix = suffix.trim_start_matches('.').trim_end_matches('.').to_string();
        host == suffix || host.ends_with(&format!(".{suffix}"))
    } else {
        host == pattern
    }
}

/// Путь как его видит `normPath` в extension/deny.js: проценты раскрыты, подряд идущие `/` слиты.
fn normalise_path(path: &str) -> String {
    let bytes = path.as_bytes();
    let mut decoded: Vec<u8> = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            if let (Some(h), Some(l)) = (hex_val(bytes[i + 1]), hex_val(bytes[i + 2])) {
                decoded.push((h << 4) | l);
                i += 3;
                continue;
            }
        }
        decoded.push(bytes[i]);
        i += 1;
    }
    let decoded = String::from_utf8_lossy(&decoded);
    let mut out = String::new();
    for c in decoded.chars() {
        if c == '/' && out.ends_with('/') {
            continue;
        }
        out.push(c);
    }
    if !out.starts_with('/') {
        out.insert(0, '/');
    }
    out
}

/// Запрещён ли адрес (жёсткий список, без карточек и «всегда»).
pub fn denied_url(url: &str) -> bool {
    // Адрес, который не разобрался, считается запретным (как в deny.js): лучше отказать, чем пропустить.
    let Ok(parsed) = reqwest::Url::parse(url) else { return true; };
    let host = parsed.host_str().unwrap_or_default().to_lowercase().trim_end_matches('.').to_string();
    // Регистр пути не важен: `/WebStore` — тот же магазин (шаблон ниже уже в нижнем регистре).
    let path = normalise_path(parsed.path()).to_lowercase();
    for pat in DENY_PATTERNS.iter() {
        let pat = pat.trim().to_lowercase();
        if let Some((phost, ppath)) = pat.split_once('/') {
            let ppath = normalise_path(&format!("/{}", ppath.trim_start_matches('/')));
            if host_matches_pattern(&host, phost) && path.starts_with(&ppath) {
                return true;
            }
        } else if host_matches_pattern(&host, &pat) {
            return true;
        }
    }
    false
}

/// Страницы без сайта, на которых читать нечего: пустая вкладка и страница ошибки самого Chrome.
fn harmless(url: &str) -> bool {
    url.is_empty() || url == "about:blank" || url.starts_with("chrome-error://")
}

// ---------------------------------------------------------------------------
// Связь с расширением
// ---------------------------------------------------------------------------

/// Одно подключённое расширение (Chrome или Edge человека).
pub struct Link {
    id: u64,
    out: mpsc::UnboundedSender<Vec<u8>>,
    browser: Mutex<String>,
    version: Mutex<String>,
    since: i64,
    /// Состояние протокола. Один замок на назначение `n` и отправку в канал: кадры уходят
    /// в том порядке, в каком получили номера.
    conn: Mutex<crate::pipe_auth::Conn>,
    /// Закрыть подключение снаружи (отвязка, новый код): цикл чтения в `serve_link` это ждёт.
    kill: tokio::sync::Notify,
}

impl Link {
    fn new(id: u64, out: mpsc::UnboundedSender<Vec<u8>>, conn: crate::pipe_auth::Conn) -> Self {
        Link { id, out, browser: Mutex::new(String::new()), version: Mutex::new(String::new()), since: now(), conn: Mutex::new(conn), kill: tokio::sync::Notify::new() }
    }

    fn send(&self, message: &Value) -> bool {
        let mut conn = self.conn.lock().unwrap();
        let Some(text) = conn.seal(message) else {
            return false;
        };
        match native_host::encode(text.as_bytes(), TO_EXTENSION_MAX) {
            Ok(frame) => self.out.send(frame).is_ok(),
            Err(e) => {
                conn.undo_seal();
                eprintln!("[user-chrome] {e}");
                false
            }
        }
    }

    /// Расширение само сообщило, что не связано с приложением (справка для окна, ничего больше).
    fn peer_unpaired(&self) -> bool {
        self.conn.lock().unwrap().peer_status.as_ref().is_some_and(|s| s["paired"] == false)
    }

    /// Расширение прислало верный `ready`: через это подключение можно работать.
    fn ready(&self) -> bool {
        self.conn.lock().unwrap().is_ready()
    }

    fn browser(&self) -> String {
        self.browser.lock().unwrap().clone()
    }
}

#[derive(Clone)]
struct Tab {
    target: String,
    url: String,
}

/// Сессия одного бота в Chrome человека: фильтр между его [`Cdp`] и расширением.
pub struct BotRelay {
    pub bot_id: String,
    name: String,
    color: String,
    link: Arc<Link>,
    /// В `Cdp`: ответы и события. Закрыть канал — закрыть сессию.
    inbound: Mutex<Option<mpsc::UnboundedSender<String>>>,
    tabs: Mutex<HashMap<String, Tab>>,
    /// Сайты, разрешённые «один раз»: до конца этой сессии.
    once: Mutex<HashSet<String>>,
    /// Сколько секретных вызовов человек разрешил прямо сейчас.
    unlock: AtomicU32,
    /// Переходы, остановленные без согласия: бот увидит их в следующем снимке.
    blocked: Mutex<Vec<String>>,
}

impl BotRelay {
    fn deliver(&self, text: String) {
        if let Some(tx) = self.inbound.lock().unwrap().as_ref() {
            let _ = tx.send(text);
        }
    }

    fn answer_error(&self, id: &Value, message: &str) {
        trace("✗", &format!("{id} {message}"));
        self.deliver(json!({ "id": id, "error": { "message": message } }).to_string());
    }

    pub fn is_closed(&self) -> bool {
        self.inbound.lock().unwrap().is_none()
    }

    /// Закрыть сессию: `Cdp` бота видит «браузер закрыт», разрешения «один раз» пропадают.
    fn close(&self) {
        self.inbound.lock().unwrap().take();
        self.once.lock().unwrap().clear();
        self.tabs.lock().unwrap().clear();
    }

    /// Можно ли боту сейчас работать на этом адресе.
    pub fn allows(&self, url: &str) -> bool {
        if harmless(url) {
            return true;
        }
        if denied_url(url) {
            return false;
        }
        let Some(origin) = origin_of(url) else { return false };
        // «Один раз» человек давал в своём ходе; ход без человека им не пользуется — только правилами.
        let unattended = app().is_some_and(|app| app.turns.is_unattended(&self.bot_id));
        (!unattended && self.once.lock().unwrap().contains(&origin)) || standing_allowed(&self.bot_id, &origin)
    }

    pub fn allow_once(&self, origin: &str) {
        self.once.lock().unwrap().insert(origin.to_string());
    }

    pub fn unlock_secret_once(&self) {
        self.unlock.fetch_add(1, Ordering::SeqCst);
    }

    fn take_unlock(&self) -> bool {
        self.unlock.try_update(Ordering::SeqCst, Ordering::SeqCst, |n| n.checked_sub(1)).is_ok()
    }

    /// Остановленные переходы с прошлого раза.
    pub fn take_blocked(&self) -> Vec<String> {
        std::mem::take(&mut *self.blocked.lock().unwrap())
    }

    fn tab_url(&self, session: &str) -> Option<String> {
        self.tabs.lock().unwrap().get(session).map(|tab| tab.url.clone())
    }

    /// Вызов от `Cdp` бота: фильтр, потом расширению.
    fn outbound(&self, text: &str) {
        trace("→", text);
        let Ok(mut message) = serde_json::from_str::<Value>(text) else { return };
        let id = message["id"].clone();
        let method = message["method"].as_str().unwrap_or_default().to_string();
        let session = message["sessionId"].as_str().map(str::to_string);
        let params = message.get("params").cloned().unwrap_or_else(|| json!({}));
        let mut unlock = false;
        match classify(&method, &params, session.is_none()) {
            Verdict::Allow => {}
            Verdict::Refuse(reason) => return self.answer_error(&id, &reason),
            Verdict::Secret => {
                if !self.take_unlock() {
                    return self.answer_error(&id, SECRET_REFUSED);
                }
                unlock = true;
            }
        }
        // Куда бот идёт: только на разрешённый сайт.
        let target_url = match method.as_str() {
            "Page.navigate" | "Target.createTarget" => params["url"].as_str().map(str::to_string),
            _ => None,
        };
        if let Some(url) = target_url {
            if !self.allows(&url) {
                return self.answer_error(&id, &consent_needed(&url));
            }
        }
        // Где бот стоит: на неразрешённом сайте читать и нажимать нельзя, только уйти.
        if let Some(session) = &session {
            if let Some(url) = self.tab_url(session) {
                if !LEAVE_METHODS.contains(&method.as_str()) && !self.allows(&url) {
                    return self.answer_error(&id, &consent_needed(&url));
                }
            }
        }
        message["params"] = params;
        let frame = json!({ "type": "cdp", "bot": self.bot_id, "name": self.name, "color": self.color, "unlock": unlock, "msg": message });
        if !self.link.send(&frame) {
            self.answer_error(&id, "расширение Deloist отключилось");
        }
    }

    /// Ответ или событие от расширения.
    fn inbound_message(&self, message: Value) {
        trace("←", &message.to_string());
        let method = message["method"].as_str().unwrap_or_default();
        let params = &message["params"];
        match method {
            "Target.attachedToTarget" => {
                let session = params["sessionId"].as_str().unwrap_or_default().to_string();
                let info = &params["targetInfo"];
                let tab = Tab { target: info["targetId"].as_str().unwrap_or_default().to_string(), url: info["url"].as_str().unwrap_or_default().to_string() };
                self.tabs.lock().unwrap().insert(session, tab);
            }
            "Target.targetInfoChanged" => {
                let info = &params["targetInfo"];
                let target = info["targetId"].as_str().unwrap_or_default();
                if let Some(tab) = self.tabs.lock().unwrap().values_mut().find(|tab| tab.target == target) {
                    tab.url = info["url"].as_str().unwrap_or_default().to_string();
                }
            }
            "Page.frameNavigated" => {
                if let Some(frame) = params.get("frame") {
                    let url = frame["url"].as_str().unwrap_or_default();
                    // Только верхний фрейм: подфреймы обрываются в Fetch.requestPaused.
                    let top = frame.get("parentId").map(|v| v.is_null()).unwrap_or(true);
                    if top && !self.allows(url) {
                        let session_id = message["sessionId"].as_str().unwrap_or_default();
                        let target = self.tabs.lock().unwrap().get(session_id).map(|t| t.target.clone());
                        if let Some(target) = target {
                            let id = HUB.next_internal_id();
                            let close = json!({ "id": id, "method": "Target.closeTarget", "params": { "targetId": target } });
                            self.link.send(&json!({ "type": "cdp", "bot": self.bot_id, "name": self.name, "color": self.color, "msg": close }));
                        }
                    }
                }
            }
            "Target.detachedFromTarget" => {
                if let Some(session) = params["sessionId"].as_str() {
                    self.tabs.lock().unwrap().remove(session);
                }
            }
            "Target.targetDestroyed" => {
                let target = params["targetId"].as_str().unwrap_or_default();
                self.tabs.lock().unwrap().retain(|_, tab| tab.target != target);
            }
            "Fetch.requestPaused" => {
                if let Some(session) = message["sessionId"].as_str() {
                    if self.stop_navigation(session, params) {
                        return;
                    }
                }
            }
            _ => {}
        }
        self.deliver(message.to_string());
    }

    /// Переход верхнего фрейма на неразрешённый сайт обрываем до загрузки: страница с входом
    /// человека не успевает открыться для бота. `true` — запрос остановлен здесь.
    fn stop_navigation(&self, session: &str, params: &Value) -> bool {
        if params["resourceType"] != "Document" {
            return false;
        }
        let url = params["request"]["url"].as_str().unwrap_or_default();
        // Любой фрейм, а не только верхний: переход подфрейма на запрещённый сайт тоже обрываем.
        if self.allows(url) {
            return false;
        }
        if let Some(origin) = origin_of(url) {
            let mut blocked = self.blocked.lock().unwrap();
            if !blocked.contains(&origin) {
                blocked.push(origin);
            }
        }
        let id = HUB.next_internal_id();
        let fail = json!({ "id": id, "method": "Fetch.failRequest", "sessionId": session, "params": { "requestId": params["requestId"], "errorReason": "BlockedByClient" } });
        self.link.send(&json!({ "type": "cdp", "bot": self.bot_id, "name": self.name, "color": self.color, "msg": fail }));
        true
    }
}

/// Журнал кадров для отладки: только отладочная сборка и только по `CREW_USER_CHROME_TRACE`.
fn trace(direction: &str, text: &str) {
    if !cfg!(debug_assertions) || std::env::var_os("CREW_USER_CHROME_TRACE").is_none() {
        return;
    }
    let Some(app) = app() else { return };
    let short: String = text.chars().take(400).collect();
    let line = format!("{} {direction} {short}\n", now());
    use std::io::Write;
    if let Ok(mut file) = std::fs::OpenOptions::new().create(true).append(true).open(app.store.data_dir().join("user-chrome-trace.log")) {
        let _ = file.write_all(line.as_bytes());
    }
}

fn consent_needed(url: &str) -> String {
    let place = origin_of(url).unwrap_or_else(|| url.chars().take(80).collect());
    format!("Сайт {place} в Chrome человека боту не разрешён. Открой его через browser_nav open: человек увидит карточку и решит.")
}

/// Разрешён ли сайт без карточки: правилом «всегда» или в этом разговоре. Без человека — только правилом.
fn standing_allowed(bot_id: &str, origin: &str) -> bool {
    let Some(app) = app() else { return false };
    let scope = site_scope(origin);
    let unattended = app.turns.is_unattended(bot_id);
    let conv = if unattended { None } else { app.turns.conv_of(bot_id) };
    if unattended && !app.store.bot(bot_id).is_some_and(|bot| unattended_allowed(&bot)) {
        return false;
    }
    app.turns.perms.find(bot_id, conv.as_deref(), &scope).is_some()
}

fn site_scope(origin: &str) -> Scope {
    Scope::new(TOOL, origin, "", "site", "")
}

// ---------------------------------------------------------------------------
// Хаб: все подключения и сессии
// ---------------------------------------------------------------------------

struct Hub {
    links: Mutex<Vec<Arc<Link>>>,
    relays: Mutex<HashMap<String, Arc<BotRelay>>>,
    next_link: AtomicU64,
    /// Номера служебных вызовов: далеко от номеров `Cdp`, ответы на них никто не ждёт.
    next_internal: AtomicU64,
}

impl Hub {
    fn next_internal_id(&self) -> u64 {
        self.next_internal.fetch_add(1, Ordering::Relaxed)
    }

    /// Подключение для новой сессии: сначала браузер, выбранный в настройках, потом самое свежее.
    fn pick(&self, preferred: &str) -> Option<Arc<Link>> {
        let links: Vec<Arc<Link>> = self.links.lock().unwrap().iter().filter(|link| link.ready()).cloned().collect();
        links.iter().rev().find(|link| !preferred.is_empty() && link.browser() == preferred).or_else(|| links.last()).cloned()
    }

    fn drop_link(&self, id: u64) {
        self.links.lock().unwrap().retain(|link| link.id != id);
        let mut relays = self.relays.lock().unwrap();
        relays.retain(|_, relay| {
            if relay.link.id == id {
                relay.close();
                false
            } else {
                true
            }
        });
    }

    /// Оборвать все подключения: после отвязки или нового кода ничто не работает на старом ключе.
    fn close_all_links(&self) {
        let links: Vec<Arc<Link>> = self.links.lock().unwrap().drain(..).collect();
        for link in &links {
            link.kill.notify_one();
        }
        let mut relays = self.relays.lock().unwrap();
        relays.retain(|_, relay| {
            if links.iter().any(|link| link.id == relay.link.id) {
                relay.close();
                false
            } else {
                true
            }
        });
    }

    fn relay_for(&self, link: u64, bot: &str) -> Option<Arc<BotRelay>> {
        self.relays.lock().unwrap().get(bot).filter(|relay| relay.link.id == link).cloned()
    }
}

static HUB: LazyLock<Hub> = LazyLock::new(|| Hub {
    links: Mutex::new(Vec::new()),
    relays: Mutex::new(HashMap::new()),
    next_link: AtomicU64::new(1),
    next_internal: AtomicU64::new(1 << 52),
});

static APP: OnceLock<Weak<App>> = OnceLock::new();

pub fn pairing_key_bytes() -> Option<Vec<u8>> {
    let app = app()?;
    let hexstr = app.connectors.secrets.get("user-chrome", "pairing_key")?;
    hex_decode(&hexstr)
}

fn hex_decode(s: &str) -> Option<Vec<u8>> {
    if s.len() % 2 != 0 { return None; }
    let mut out = Vec::with_capacity(s.len() / 2);
    for chunk in s.as_bytes().chunks(2) {
        let hi = hex_val(chunk[0])?;
        let lo = hex_val(chunk[1])?;
        out.push((hi << 4) | lo);
    }
    Some(out)
}

fn hex_val(b: u8) -> Option<u8> {
    match b {
        b'0'..=b'9' => Some(b - b'0'),
        b'a'..=b'f' => Some(b - b'a' + 10),
        b'A'..=b'F' => Some(b - b'A' + 10),
        _ => None,
    }
}

fn app() -> Option<Arc<App>> {
    APP.get().and_then(Weak::upgrade)
}

/// Новая сессия бота в Chrome человека. Старая сессия этого бота закрывается.
pub fn open(store: &Store, bot_id: &str) -> Result<(Arc<Cdp>, mpsc::UnboundedReceiver<Value>, Arc<BotRelay>), String> {
    let preferred = store.settings().extra.get(SETTINGS_KEY).and_then(|v| v["browser"].as_str().map(str::to_string)).unwrap_or_default();
    let link = HUB.pick(&preferred).ok_or(NOT_CONNECTED)?;
    let bot = store.bot(bot_id).ok_or("нет такого бота")?;
    let (out_tx, mut out_rx) = mpsc::unbounded_channel::<String>();
    let (in_tx, in_rx) = mpsc::unbounded_channel::<String>();
    let (cdp, events) = Cdp::from_channels(out_tx, in_rx);
    let relay = Arc::new(BotRelay {
        bot_id: bot_id.to_string(),
        name: bot.name.clone(),
        color: bot.color.clone(),
        link,
        inbound: Mutex::new(Some(in_tx)),
        tabs: Mutex::new(HashMap::new()),
        once: Mutex::new(HashSet::new()),
        unlock: AtomicU32::new(0),
        blocked: Mutex::new(Vec::new()),
    });
    if let Some(old) = HUB.relays.lock().unwrap().insert(bot_id.to_string(), relay.clone()) {
        old.close();
    }
    let weak = Arc::downgrade(&relay);
    tokio::spawn(async move {
        while let Some(text) = out_rx.recv().await {
            let Some(relay) = weak.upgrade() else { break };
            if relay.is_closed() {
                break;
            }
            relay.outbound(&text);
        }
    });
    push_state();
    Ok((cdp, events, relay))
}

/// «Остановить всё»: расширение отпускает все вкладки, сессии закрываются, «один раз» забыт.
pub fn stop_all() {
    for link in HUB.links.lock().unwrap().iter() {
        link.send(&json!({ "type": "stop_all" }));
    }
    stop_local();
}

fn stop_local() {
    let relays: Vec<Arc<BotRelay>> = HUB.relays.lock().unwrap().drain().map(|(_, relay)| relay).collect();
    for relay in relays {
        relay.close();
    }
    push_state();
}

/// Закрыть сессию одного бота (смена режима, удаление бота).
pub fn stop_bot(bot_id: &str) {
    if let Some(relay) = HUB.relays.lock().unwrap().remove(bot_id) {
        relay.link.send(&json!({ "type": "detach", "bot": bot_id }));
        relay.close();
    }
}

/// Сессия закрывается штатно (простой, закрытие программы): убрать её из хаба, если она ещё там.
pub fn release(relay: &Arc<BotRelay>) {
    let mut relays = HUB.relays.lock().unwrap();
    if relays.get(&relay.bot_id).is_some_and(|current| Arc::ptr_eq(current, relay)) {
        relays.remove(&relay.bot_id);
    }
    drop(relays);
    relay.close();
    push_state();
}

/// Расширению — что показать в его окне: боты, кто работает, разрешённые сайты.
pub fn push_state() {
    let Some(app) = app() else { return };
    let state = json!({ "type": "state", "bots": bots_state(&app) });
    for link in HUB.links.lock().unwrap().iter() {
        link.send(&state);
    }
}

fn bots_state(app: &App) -> Vec<Value> {
    let rules = app.turns.perms.rules();
    let active: HashSet<String> = HUB.relays.lock().unwrap().iter().filter(|(_, relay)| !relay.is_closed()).map(|(id, _)| id.clone()).collect();
    let default = default_mode(&app.store);
    app.store
        .bots()
        .into_iter()
        .map(|bot| {
            let sites: Vec<String> = rules.iter().filter(|rule| rule.bot_id == bot.id && rule.tool == TOOL).map(|rule| rule.place.clone()).collect();
            let mode = bot_mode(&bot).to_string();
            let mine = if mode.is_empty() { default == "mine" } else { mode == "mine" };
            json!({
                "id": bot.id, "name": bot.name, "color": bot.color, "mode": mode, "mine": mine,
                "unattended": unattended_allowed(&bot), "active": active.contains(&bot.id), "sites": sites,
            })
        })
        .collect()
}

// ---------------------------------------------------------------------------
// Канал для хоста
// ---------------------------------------------------------------------------

/// Поднять именованный канал для хоста. Вызывается после того, как записан `runtime.json`.
pub fn start(app: Arc<App>, token: String) {
    let _ = APP.set(Arc::downgrade(&app));
    write_extension_if_installed(app.store.data_dir());
    #[cfg(windows)]
    tokio::spawn(async move {
        if let Err(e) = serve_pipe(token).await {
            eprintln!("[user-chrome] канал для расширения не открылся: {e}");
        }
    });
    #[cfg(not(windows))]
    let _ = token;
}

#[cfg(windows)]
async fn serve_pipe(token: String) -> std::io::Result<()> {
    use tokio::net::windows::named_pipe::ServerOptions;
    let name = native_host::pipe_name(std::process::id());
    // Первый экземпляр: если имя уже занято кем-то, не работаем вовсе, а не делим канал с ним.
    let mut server = ServerOptions::new().first_pipe_instance(true).reject_remote_clients(true).create(&name)?;
    let token = Arc::new(token);
    loop {
        server.connect().await?;
        let connected = server;
        server = ServerOptions::new().reject_remote_clients(true).create(&name)?;
        tokio::spawn(serve_link(connected, token.clone()));
    }
}

/// Одно подключение хоста: ключ, потом кадры в обе стороны.
async fn serve_link<P>(pipe: P, token: Arc<String>)
where
    P: tokio::io::AsyncRead + tokio::io::AsyncWrite + Send + Unpin + 'static,
{
    let (mut reader, mut writer) = tokio::io::split(pipe);
    let hello = match tokio::time::timeout(Duration::from_secs(5), native_host::read_json(&mut reader, HELLO_MAX)).await {
        Ok(Ok(Some(hello))) => hello,
        _ => return,
    };
    let origin = hello["origin"].as_str().unwrap_or_default();
    let good_token = hello["token"].as_str().is_some_and(|given| same_secret(given, &token));
    if hello["type"] != "host" || !good_token || !native_host::origin_allowed(origin) {
        let refusal = json!({ "type": "error", "code": "refused", "text": "Deloist не принял это подключение." });
        let _ = native_host::write_frame(&mut writer, refusal.to_string().as_bytes(), TO_EXTENSION_MAX).await;
        return;
    }

    // 16 случайных байт соли приложения.
    let mut local_salt = [0u8; 16];
    for b in local_salt.iter_mut() {
        *b = rand::random();
    }
    let key: Option<[u8; 32]> = pairing_key_bytes().and_then(|k| k.try_into().ok());

    let (out, mut out_rx) = mpsc::unbounded_channel::<Vec<u8>>();
    let conn = crate::pipe_auth::Conn::app(key, local_salt);
    let hello_text = conn.hello_text();
    let link = Arc::new(Link::new(HUB.next_link.fetch_add(1, Ordering::Relaxed), out.clone(), conn));
    tokio::spawn(async move {
        use tokio::io::AsyncWriteExt;
        while let Some(frame) = out_rx.recv().await {
            if writer.write_all(&frame).await.is_err() || writer.flush().await.is_err() {
                break;
            }
        }
    });

    HUB.links.lock().unwrap().push(link.clone());

    // Hello уходит без подписи; без пары следом только status, больше ничего.
    if let Ok(frame) = native_host::encode(hello_text.as_bytes(), TO_EXTENSION_MAX) {
        let _ = out.send(frame);
    }
    if key.is_none() {
        if let Ok(frame) = native_host::encode_json(&json!({ "type": "status", "paired": false }), TO_EXTENSION_MAX) {
            let _ = out.send(frame);
        }
    }

    // Кадры одного подключения обрабатываются по одному, в порядке прихода.
    loop {
        let raw = tokio::select! {
            _ = link.kill.notified() => break,
            read = native_host::read_frame(&mut reader, FROM_EXTENSION_MAX) => match read {
                Ok(Some(raw)) => raw,
                Ok(None) | Err(_) => break,
            },
        };
        let Ok(text) = String::from_utf8(raw) else { break };
        let action = link.conn.lock().unwrap().on_message(&text);
        match action {
            Action::Close => break,
            Action::Ignore => {
                // Hello без пары: из него берём только названия браузера и версии для окна.
                let hello = link.conn.lock().unwrap().peer_hello.clone();
                if let Some(hello) = hello {
                    if link.browser().is_empty() {
                        *link.browser.lock().unwrap() = clean_word(hello["browser"].as_str().unwrap_or_default());
                        *link.version.lock().unwrap() = clean_word(hello["version"].as_str().unwrap_or_default());
                    }
                }
            }
            Action::SessionReady => {
                let hello = link.conn.lock().unwrap().peer_hello.clone().unwrap_or(Value::Null);
                *link.browser.lock().unwrap() = clean_word(hello["browser"].as_str().unwrap_or_default());
                *link.version.lock().unwrap() = clean_word(hello["version"].as_str().unwrap_or_default());
                link.send(&json!({ "type": "ready" }));
            }
            Action::Deliver(parsed) => match parsed.get("type").and_then(Value::as_str).unwrap_or_default() {
                "ready" => push_state(),
                "cdp" => {
                    let bot = parsed["bot"].as_str().unwrap_or_default();
                    if let Some(relay) = HUB.relay_for(link.id, bot) {
                        relay.inbound_message(parsed["msg"].clone());
                    }
                }
                "stopped" => stop_local(),
                _ => {}
            },
        }
    }

    HUB.drop_link(link.id);
    push_state();
}

/// Без раннего выхода: время сравнения не подсказывает, сколько символов совпало.
fn same_secret(a: &str, b: &str) -> bool {
    a.len() == b.len() && a.bytes().zip(b.bytes()).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

fn clean_word(text: &str) -> String {
    text.chars().filter(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_')).take(32).collect()
}

// ---------------------------------------------------------------------------
// Согласия для инструментов браузера
// ---------------------------------------------------------------------------

pub const UNATTENDED_OFF: &str = "Это ход без человека (рутина или ночная работа), а работать в Chrome человека без него этому боту не разрешено. Скажи об этом в отчёте: включить можно в Настройки → «Мой Chrome».";

/// Ход без человека может пользоваться Chrome человека, только если это включено боту.
pub fn check_unattended(app: &App, bot_id: &str) -> Result<(), String> {
    if app.turns.is_unattended(bot_id) && !app.store.bot(bot_id).is_some_and(|bot| unattended_allowed(&bot)) {
        return Err(UNATTENDED_OFF.into());
    }
    Ok(())
}

/// Перед переходом на сайт: разрешён ли он; если нет — карточка человеку.
/// Без человека карточки нет: только сайты с правилом «всегда».
pub async fn gate_site(app: &Arc<App>, bot: &Bot, relay: &BotRelay, url: &str) -> Result<(), String> {
    if denied_url(url) {
        return Err("Этот сайт в Chrome человека запрещён (банк, пароли или госуслуги).".into());
    }
    let origin = origin_of(url).ok_or("в Chrome человека бот открывает только адреса http и https")?;
    if relay.allows(url) {
        return Ok(());
    }
    if app.turns.is_unattended(&bot.id) {
        return Err(format!(
            "Это ход без человека: в Chrome человека бот открывает только сайты, которые человек разрешил «всегда». {origin} не из них — скажи об этом в отчёте."
        ));
    }
    let host = reqwest::Url::parse(url).ok().and_then(|u| u.host_str().map(str::to_string)).unwrap_or_else(|| origin.clone());
    let title = format!("{} хочет открыть {host} в твоём Chrome", bot.name);
    let detail = format!(
        "Адрес: {url}\nБот откроет сайт во вкладке своей группы «Deloist · {}» с твоим входом и сможет его читать. Нажимать и вводить текст он спросит отдельно, cookies, пароли и оплату без отдельного вопроса не трогает. Остановить можно кнопкой «Остановить всё» в расширении.",
        bot.name
    );
    let question = crate::turn::Confirmation {
        tool: TOOL,
        title: &title,
        detail: &detail,
        allow_label: None,
        deny_label: None,
        timeout: Duration::from_secs(5 * 60),
        scope: Some(site_scope(&origin)),
        room: None,
        danger: false,
        sensitive: false,
        remote: crate::grants::Remote::No,
        human: None,
        // Подробности карточки и есть весь вызов (адрес сайта): отдельного отпечатка не нужно.
        exact: None,
    };
    if crate::turn::request_confirmation(app, &bot.id, question).await {
        relay.allow_once(&origin);
        push_state();
        Ok(())
    } else {
        Err(format!("Человек не разрешил открыть {host} в своём Chrome. Не пытайся открыть его другим способом."))
    }
}

/// Подсказка в снимок: какие переходы остановлены без согласия.
pub fn blocked_note(relay: &BotRelay) -> String {
    let blocked = relay.take_blocked();
    if blocked.is_empty() {
        return String::new();
    }
    format!(
        "\nПереход остановлен: {} — этот сайт человек в своём Chrome ещё не разрешил. Чтобы туда попасть, вызови browser_nav open с этим адресом: человек увидит карточку.",
        blocked.join(", ")
    )
}

// ---------------------------------------------------------------------------
// Установка: файлы расширения, манифест хоста, реестр
// ---------------------------------------------------------------------------

const EXTENSION_FILES: &[(&str, &str)] = &[
    ("manifest.json", include_str!("../extension/manifest.json")),
    ("background.js", include_str!("../extension/background.js")),
    ("popup.html", include_str!("../extension/popup.html")),
    ("popup.js", include_str!("../extension/popup.js")),
    ("popup.css", include_str!("../extension/popup.css")),
    ("_locales/ru/messages.json", include_str!("../extension/_locales/ru/messages.json")),
    ("_locales/en/messages.json", include_str!("../extension/_locales/en/messages.json")),
    ("deny.json", include_str!("../extension/deny.json")),
    ("deny.js", include_str!("../extension/deny.js")),
    ("protocol.js", include_str!("../extension/protocol.js")),
    ("protocol-vectors.json", include_str!("../extension/protocol-vectors.json")),
];

/// Папка распакованного расширения: в папке данных, версия всегда как у программы.
pub fn extension_dir(data_dir: &Path) -> PathBuf {
    data_dir.join("chrome-extension")
}

fn manifest_path(data_dir: &Path) -> PathBuf {
    data_dir.join("native-host").join(format!("{}.json", native_host::HOST_NAME))
}

pub fn write_extension(data_dir: &Path) -> std::io::Result<PathBuf> {
    let dir = extension_dir(data_dir);
    for (name, text) in EXTENSION_FILES {
        let path = dir.join(name);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        if std::fs::read_to_string(&path).ok().as_deref() != Some(*text) {
            std::fs::write(&path, text)?;
        }
    }
    Ok(dir)
}

/// После обновления программы расширение в папке данных обновляется само (Chrome подхватит при перезапуске).
fn write_extension_if_installed(data_dir: &Path) {
    if extension_dir(data_dir).exists() {
        if let Err(e) = write_extension(data_dir) {
            eprintln!("[user-chrome] не обновил файлы расширения: {e}");
        }
    }
}

pub fn host_manifest(exe: &Path) -> Value {
    json!({
        "name": native_host::HOST_NAME,
        "description": "Deloist: связь расширения с программой",
        "path": exe.display().to_string(),
        "type": "stdio",
        "allowed_origins": native_host::allowed_origins(),
    })
}

/// Ключи реестра хоста в HKCU: Chrome и Edge. Других ключей программа не пишет.
pub fn registry_keys() -> [String; 2] {
    [
        format!(r"Software\Google\Chrome\NativeMessagingHosts\{}", native_host::HOST_NAME),
        format!(r"Software\Microsoft\Edge\NativeMessagingHosts\{}", native_host::HOST_NAME),
    ]
}

#[cfg(windows)]
fn wide(text: &str) -> Vec<u16> {
    text.encode_utf16().chain(std::iter::once(0)).collect()
}

/// Значение ключа по умолчанию (REG_SZ). Ключ создаётся, если его нет.
#[cfg(windows)]
fn reg_set(subkey: &str, value: &str) -> Result<(), String> {
    use windows_sys::Win32::System::Registry::{RegSetKeyValueW, HKEY_CURRENT_USER, REG_SZ};
    let (key, data) = (wide(subkey), wide(value));
    // Строки живут до конца вызова; размер — в байтах вместе с нулём в конце.
    let code = unsafe { RegSetKeyValueW(HKEY_CURRENT_USER, key.as_ptr(), std::ptr::null(), REG_SZ, data.as_ptr().cast(), (data.len() * 2) as u32) };
    if code == 0 { Ok(()) } else { Err(std::io::Error::from_raw_os_error(code as i32).to_string()) }
}

#[cfg(windows)]
fn reg_get(subkey: &str) -> Option<String> {
    use windows_sys::Win32::System::Registry::{RegGetValueW, HKEY_CURRENT_USER, RRF_RT_REG_SZ};
    let key = wide(subkey);
    let mut buffer = vec![0u16; 2048];
    let mut size = (buffer.len() * 2) as u32;
    // Буфер и размер наши; функция пишет не больше `size` байт.
    let code = unsafe { RegGetValueW(HKEY_CURRENT_USER, key.as_ptr(), std::ptr::null(), RRF_RT_REG_SZ, std::ptr::null_mut(), buffer.as_mut_ptr().cast(), &mut size) };
    if code != 0 {
        return None;
    }
    let len = (size as usize / 2).saturating_sub(1).min(buffer.len());
    Some(String::from_utf16_lossy(&buffer[..len])).filter(|value| !value.is_empty())
}

#[cfg(windows)]
fn reg_delete(subkey: &str) -> Result<(), String> {
    use windows_sys::Win32::System::Registry::{RegDeleteKeyW, HKEY_CURRENT_USER};
    let key = wide(subkey);
    let code = unsafe { RegDeleteKeyW(HKEY_CURRENT_USER, key.as_ptr()) };
    // 2 — ключа уже нет: это и нужно.
    if code == 0 || code == 2 { Ok(()) } else { Err(std::io::Error::from_raw_os_error(code as i32).to_string()) }
}

#[cfg(not(windows))]
fn reg_set(_subkey: &str, _value: &str) -> Result<(), String> {
    Err("«Мой Chrome» пока работает только в Windows".into())
}

#[cfg(not(windows))]
fn reg_get(_subkey: &str) -> Option<String> {
    None
}

#[cfg(not(windows))]
fn reg_delete(_subkey: &str) -> Result<(), String> {
    Ok(())
}

/// Значение ключа по умолчанию — путь к манифесту, если ключ есть.
pub fn registered_manifest(key: &str) -> Option<String> {
    reg_get(key)
}

pub fn register(data_dir: &Path) -> Result<(), String> {
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    let manifest = manifest_path(data_dir);
    std::fs::create_dir_all(manifest.parent().unwrap()).map_err(|e| e.to_string())?;
    let text = serde_json::to_string_pretty(&host_manifest(&exe)).unwrap_or_default();
    std::fs::write(&manifest, text).map_err(|e| format!("не записал манифест хоста: {e}"))?;
    write_extension(data_dir).map_err(|e| format!("не записал файлы расширения: {e}"))?;
    let path = manifest.display().to_string();
    for key in registry_keys() {
        reg_set(&key, &path).map_err(|e| format!(r"не записал HKCU\{key}: {e}"))?;
    }
    Ok(())
}

pub fn unregister(data_dir: &Path) -> Result<(), String> {
    let ours = manifest_path(data_dir).display().to_string();
    for key in registry_keys() {
        // Чужую регистрацию (другая копия программы) не трогаем.
        if registered_manifest(&key).is_some_and(|path| path.eq_ignore_ascii_case(&ours)) {
            reg_delete(&key).map_err(|e| format!(r"не удалил HKCU\{key}: {e}"))?;
        }
    }
    let _ = std::fs::remove_file(manifest_path(data_dir));
    Ok(())
}

/// Chrome и Edge на этой машине.
fn browsers() -> Vec<Value> {
    let env_dir = |name: &str| std::env::var_os(name).map(PathBuf::from);
    let bases = [env_dir("ProgramFiles"), env_dir("ProgramFiles(x86)"), env_dir("LOCALAPPDATA")];
    let mut found = Vec::new();
    for (name, label, rel) in [
        ("chrome", "Google Chrome", r"Google\Chrome\Application\chrome.exe"),
        ("edge", "Microsoft Edge", r"Microsoft\Edge\Application\msedge.exe"),
    ] {
        if let Some(path) = bases.iter().flatten().map(|base| base.join(rel)).find(|path| path.is_file()) {
            found.push(json!({ "id": name, "name": label, "path": path.display().to_string() }));
        }
    }
    found
}

// ---------------------------------------------------------------------------
// API для настроек
// ---------------------------------------------------------------------------

type Shared = State<Arc<App>>;

fn fail(status: StatusCode, message: impl Into<String>) -> Response {
    (status, Json(json!({ "error": message.into() }))).into_response()
}

fn status(app: &App) -> Value {
    let data_dir = app.store.data_dir();
    let ours = manifest_path(data_dir).display().to_string();
    let registered = registry_keys().iter().any(|key| registered_manifest(key).is_some_and(|path| path.eq_ignore_ascii_case(&ours)));
    let links: Vec<Value> = HUB
        .links
        .lock()
        .unwrap()
        .iter()
        .map(|link| json!({ "browser": link.browser(), "version": link.version.lock().unwrap().clone(), "since": link.since, "ready": link.ready(), "peerUnpaired": link.peer_unpaired() }))
        .collect();
    let ready = links.iter().any(|link| link["ready"] == true);
    let settings = app.store.settings();
    let saved = settings.extra.get(SETTINGS_KEY).cloned().unwrap_or_else(|| json!({}));
    json!({
        "registered": registered,
        // «Подключено» — есть подключение, прошедшее проверку; просто пришедшее расширение — это ожидание пары.
        "connected": ready,
        "waitingForPairing": !ready && !links.is_empty(),
        "links": links,
        "browsers": browsers(),
        "extensionDir": extension_dir(data_dir).display().to_string(),
        "extensionId": native_host::EXTENSION_IDS[0],
        "default": default_mode(&app.store),
        "browser": saved["browser"].as_str().unwrap_or_default(),
        "bots": bots_state(app),
        "paired": app.connectors.secrets.get("user-chrome", "pairing_key").is_some(),
    })
}

async fn get_status(State(app): Shared) -> Response {
    Json(status(&app)).into_response()
}

async fn connect(State(app): Shared) -> Response {
    let data_dir = app.store.data_dir().to_path_buf();
    match tokio::task::spawn_blocking(move || register(&data_dir)).await {
        Ok(Ok(())) => Json(status(&app)).into_response(),
        Ok(Err(e)) => fail(StatusCode::INTERNAL_SERVER_ERROR, e),
        Err(e) => fail(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()),
    }
}

async fn disconnect(State(app): Shared) -> Response {
    stop_all();
    let data_dir = app.store.data_dir().to_path_buf();
    match tokio::task::spawn_blocking(move || unregister(&data_dir)).await {
        Ok(Ok(())) => Json(status(&app)).into_response(),
        Ok(Err(e)) => fail(StatusCode::INTERNAL_SERVER_ERROR, e),
        Err(e) => fail(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()),
    }
}

async fn stop(State(app): Shared) -> Response {
    stop_all();
    Json(status(&app)).into_response()
}

async fn put_settings(State(app): Shared, Json(body): Json<Value>) -> Response {
    let mut saved = app.store.settings().extra.get(SETTINGS_KEY).cloned().unwrap_or_else(|| json!({}));
    if let Some(mode) = body.get("default").and_then(Value::as_str) {
        if mode != "own" {
            return fail(StatusCode::BAD_REQUEST, "default: только own");
        }
        saved["default"] = json!(mode);
    }
    if let Some(browser) = body.get("browser").and_then(Value::as_str) {
        if !matches!(browser, "" | "chrome" | "edge") {
            return fail(StatusCode::BAD_REQUEST, "browser: chrome, edge или пусто");
        }
        saved["browser"] = json!(browser);
    }
    if let Err(e) = app.store.set_setting_extra(SETTINGS_KEY, saved).await {
        return fail(StatusCode::INTERNAL_SERVER_ERROR, e.to_string());
    }
    // Боты «как по умолчанию» начнут следующий шаг уже в новом браузере.
    for bot in app.store.bots() {
        if bot_mode(&bot).is_empty() {
            app.browsers.close(&bot.id).await;
            stop_bot(&bot.id);
        }
    }
    push_state();
    Json(status(&app)).into_response()
}

async fn put_bot(State(app): Shared, UrlPath(id): UrlPath<String>, Json(body): Json<Value>) -> Response {
    if app.store.bot(&id).is_none() {
        return fail(StatusCode::NOT_FOUND, "нет такого бота");
    }
    if let Some(mode) = body.get("mode").and_then(Value::as_str) {
        if !matches!(mode, "" | "own" | "mine") {
            return fail(StatusCode::BAD_REQUEST, "mode: own, mine или пусто");
        }
        if let Err(e) = app.store.set_bot_extra(&id, BOT_MODE, json!(mode)).await {
            return fail(StatusCode::INTERNAL_SERVER_ERROR, e.to_string());
        }
        app.browsers.close(&id).await;
        stop_bot(&id);
    }
    if let Some(on) = body.get("unattended").and_then(Value::as_bool) {
        if let Err(e) = app.store.set_bot_extra(&id, BOT_UNATTENDED, json!(on)).await {
            return fail(StatusCode::INTERNAL_SERVER_ERROR, e.to_string());
        }
    }
    push_state();
    Json(status(&app)).into_response()
}

async fn pairing_code_get(State(app): Shared) -> Response {
    Json(json!({ "paired": app.connectors.secrets.get("user-chrome", "pairing_key").is_some() })).into_response()
}

async fn pairing_code_post(State(app): Shared) -> Response {
    // Новый код каждый раз; хранится только производный ключ. PBKDF2 считает секунду-другую: вне потока сервера.
    let worker = app.clone();
    let code = tokio::task::spawn_blocking(move || generate_pairing_code(&worker)).await.unwrap_or_else(|e| Err(e.to_string()));
    match code {
        Ok(c) => {
            // Старые подключения знают прежний ключ: пусть переподключаются уже с новым.
            HUB.close_all_links();
            push_state();
            Json(json!({ "code": c })).into_response()
        }
        Err(e) => fail(StatusCode::INTERNAL_SERVER_ERROR, e),
    }
}

fn generate_pairing_code(app: &Arc<App>) -> Result<String, String> {
    const WORDS: &[&str] = &[
        "amber", "anchor", "apple", "arrow", "atlas", "beacon", "birch", "bloom", "brave", "brook",
        "cedar", "chalk", "charm", "chess", "civic", "cliff", "cloud", "coral", "crane", "crest",
        "daisy", "delta", "drift", "eagle", "ember", "fable", "falcon", "fern", "flame", "frost",
        "glade", "globe", "grace", "grove", "harbor", "haven", "hazel", "honey", "ivory", "jade",
        "karma", "latch", "laurel", "lemon", "lilac", "linen", "lotus", "lunar", "maple", "marsh",
        "meadow", "mirth", "mossy", "noble", "north", "ocean", "olive", "orbit", "panel", "pearl",
        "pine", "plume", "prism", "quartz", "quiet", "raven", "reef", "river", "robin", "sage",
        "satin", "scout", "shade", "shell", "sierra", "silk", "slate", "smoke", "solar", "spark",
        "spring", "stone", "storm", "sugar", "summit", "swift", "thorn", "tide", "trail", "ultra",
        "umbra", "unity", "velvet", "vivid", "waltz", "whale", "wheat", "willow", "winter", "wren",
    ];
    use rand::Rng;
    let mut rng = rand::thread_rng();
    let mut parts = Vec::with_capacity(8);
    for _ in 0..8 {
        parts.push(WORDS[rng.gen_range(0..WORDS.len())]);
    }
    let code = parts.join(" ");
    let key = crate::pipe_auth::derive_k(&code);
    app.connectors.secrets.set("user-chrome", "pairing_key", &crate::store::hex(&key))?;
    Ok(code)
}

async fn unpair(State(app): Shared) -> Response {
    let _ = app.connectors.secrets.delete_scope("user-chrome");
    stop_all();
    // Живые подключения держат старый сессионный ключ: рвём их, до новой пары работать нечему.
    HUB.close_all_links();
    push_state();
    Json(status(&app)).into_response()
}

async fn open_folder(State(app): Shared) -> Response {
    let dir = match write_extension(app.store.data_dir()) {
        Ok(dir) => dir,
        Err(e) => return fail(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()),
    };
    if cfg!(debug_assertions) && std::env::var_os("CREW_TEST_NO_EXPLORER").is_some() {
        return Json(json!({ "dir": dir.display().to_string() })).into_response();
    }
    let _ = std::process::Command::new("explorer.exe").arg(&dir).spawn();
    Json(json!({ "dir": dir.display().to_string() })).into_response()
}

pub fn routes() -> Router<Arc<App>> {
    Router::new()
        .route("/api/user-chrome", get(get_status))
        .route("/api/user-chrome/pairing", get(pairing_code_get).post(pairing_code_post))
        .route("/api/user-chrome/unpair", post(unpair))
        .route("/api/user-chrome/connect", post(connect))
        .route("/api/user-chrome/disconnect", post(disconnect))
        .route("/api/user-chrome/stop", post(stop))
        .route("/api/user-chrome/settings", put(put_settings))
        .route("/api/user-chrome/bots/{id}", put(put_bot))
        .route("/api/user-chrome/folder", post(open_folder))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn our_own_calls_pass_the_filter() {
        for method in TAB_METHODS {
            if method.starts_with("Fetch.") {
                continue;
            }
            assert_eq!(classify(method, &json!({}), false), Verdict::Allow, "{method}");
        }
        for method in BROWSER_METHODS {
            assert_eq!(classify(method, &json!({}), true), Verdict::Allow, "{method}");
        }
        assert!(matches!(classify("Fetch.enable", &json!({}), false), Verdict::Refuse(_)));
        // Внутренние выражения наших инструментов.
        for expression in ["document.title", "document.readyState", "document.activeElement", "document.elementFromPoint(10, 20)"] {
            assert_eq!(classify("Runtime.evaluate", &json!({ "expression": expression }), false), Verdict::Allow, "{expression}");
        }
    }

    #[test]
    fn the_exact_blocklist() {
        for method in [
            "Network.getCookies",
            "Network.getAllCookies",
            "Network.setCookie",
            "Network.setCookies",
            "Network.deleteCookies",
            "Network.clearBrowserCookies",
            "Network.getRequestPostData",
            "Storage.getCookies",
            "Storage.clearDataForOrigin",
            "Storage.getStorageKeyForFrame",
            "DOMStorage.getDOMStorageItems",
            "IndexedDB.requestData",
            "CacheStorage.requestEntries",
            "Autofill.trigger",
            "Page.getCookies",
            "Page.addScriptToEvaluateOnNewDocument",
            "Runtime.addBinding",
            "Runtime.compileScript",
        ] {
            assert_eq!(classify(method, &json!({}), false), Verdict::Secret, "{method}");
            assert_eq!(classify(method, &json!({}), true), Verdict::Secret, "{method} и на уровне браузера");
        }
        for code in [
            "document.cookie",
            "JSON.stringify({local:{...localStorage},session:{...sessionStorage}})",
            "window.LocalStorage",
            "indexedDB.databases()",
            "cookieStore.getAll()",
            "caches.keys()",
            "navigator.credentials.get({password:true})",
        ] {
            assert_eq!(classify("Runtime.evaluate", &json!({ "expression": code }), false), Verdict::Secret, "{code}");
            assert_eq!(classify("Debugger.evaluateOnCallFrame", &json!({ "expression": code }), false), Verdict::Secret, "{code}");
        }
        let via_args = json!({ "functionDeclaration": "function(k){return document[k]}", "arguments": [{ "value": "cookie" }] });
        assert_eq!(classify("Runtime.callFunctionOn", &via_args, false), Verdict::Secret, "секрет в аргументах");
        let condition = json!({ "url": "https://x.ru/a.js", "lineNumber": 1, "condition": "console.log(document.cookie)" });
        assert_eq!(classify("Debugger.setBreakpointByUrl", &condition, false), Verdict::Secret, "секрет в условии");
    }

    #[test]
    fn anything_unlisted_is_refused() {
        for (method, browser) in [
            ("Target.getTargets", true),
            ("Target.attachToTarget", true),
            ("Browser.getVersion", true),
            ("SystemInfo.getInfo", true),
            ("Page.navigate", true),
            ("Target.createTarget", false),
            ("Page.printToPDF", false),
            ("Page.setDownloadBehavior", false),
            ("Network.setExtraHTTPHeaders", false),
            ("Security.setIgnoreCertificateErrors", false),
            ("Emulation.setUserAgentOverride", false),
            ("Overlay.highlightNode", false),
        ] {
            assert!(matches!(classify(method, &json!({}), browser), Verdict::Refuse(_)), "{method}");
        }
    }

    #[test]
    fn origins_and_harmless_pages() {
        assert_eq!(origin_of("https://seller.ozon.ru/app/x?y=1").as_deref(), Some("https://seller.ozon.ru"));
        assert_eq!(origin_of("javascript:alert(1)"), None);
        assert_eq!(origin_of("file:///C:/x"), None);
        assert!(harmless("about:blank") && harmless("chrome-error://chromewebdata/") && harmless(""));
        assert!(!harmless("https://x.ru/"));
    }

    fn relay(bot: &str) -> (Arc<BotRelay>, mpsc::UnboundedReceiver<Vec<u8>>, mpsc::UnboundedReceiver<String>) {
        let (out, out_rx) = mpsc::unbounded_channel();
        let (in_tx, in_rx) = mpsc::unbounded_channel();
        let mut conn = crate::pipe_auth::Conn::app(Some([0u8; 32]), [1u8; 16]);
        let ext = crate::pipe_auth::Conn::new(Some([0u8; 32]), [2u8; 16], crate::pipe_auth::DIR_APP_TO_EXT, crate::pipe_auth::DIR_EXT_TO_APP);
        conn.on_message(&ext.hello_text());
        conn.seal(&json!({ "type": "ready" })).unwrap();
        let link = Arc::new(Link::new(999, out, conn));
        *link.browser.lock().unwrap() = "chrome".into();
        let relay = Arc::new(BotRelay {
            bot_id: bot.into(),
            name: "Рен".into(),
            color: "#4a90e2".into(),
            link,
            inbound: Mutex::new(Some(in_tx)),
            tabs: Mutex::new(HashMap::new()),
            once: Mutex::new(HashSet::new()),
            unlock: AtomicU32::new(0),
            blocked: Mutex::new(Vec::new()),
        });
        (relay, out_rx, in_rx)
    }

    fn sent(rx: &mut mpsc::UnboundedReceiver<Vec<u8>>) -> Option<Value> {
        rx.try_recv().ok().map(|frame| {
            let outer: Value = serde_json::from_slice(&frame[4..]).unwrap();
            if let Some(body) = outer.get("body").and_then(Value::as_str) {
                serde_json::from_str(body).unwrap()
            } else {
                outer
            }
        })
    }

    fn answered(rx: &mut mpsc::UnboundedReceiver<String>) -> Option<Value> {
        rx.try_recv().ok().map(|text| serde_json::from_str(&text).unwrap())
    }

    fn attach(relay: &BotRelay, session: &str, target: &str, url: &str) {
        relay.inbound_message(json!({ "method": "Target.attachedToTarget", "params": { "sessionId": session, "targetInfo": { "targetId": target, "url": url, "type": "page" } } }));
    }

    #[test]
    fn a_site_needs_consent_and_once_lasts_for_the_session() {
        let (relay, mut out, mut back) = relay("b-site");
        attach(&relay, "s1", "T1", "about:blank");
        assert!(answered(&mut back).is_some(), "событие ушло в Cdp");
        // Без согласия: переход отклонён здесь, расширение его не видит.
        relay.outbound(&json!({ "id": 1, "sessionId": "s1", "method": "Page.navigate", "params": { "url": "https://seller.ozon.ru/app" } }).to_string());
        assert!(sent(&mut out).is_none());
        let reply = answered(&mut back).unwrap();
        assert!(reply["error"]["message"].as_str().unwrap().contains("не разрешён"), "{reply}");
        // «Один раз» — и переход, и чтение проходят.
        relay.allow_once("https://seller.ozon.ru");
        relay.outbound(&json!({ "id": 2, "sessionId": "s1", "method": "Page.navigate", "params": { "url": "https://seller.ozon.ru/app" } }).to_string());
        let frame = sent(&mut out).unwrap();
        assert_eq!(frame["msg"]["method"], "Page.navigate");
        assert_eq!(frame["bot"], "b-site");
        relay.inbound_message(json!({ "method": "Target.targetInfoChanged", "params": { "targetInfo": { "targetId": "T1", "url": "https://seller.ozon.ru/app" } } }));
        relay.outbound(&json!({ "id": 3, "sessionId": "s1", "method": "Accessibility.getFullAXTree", "params": {} }).to_string());
        assert!(sent(&mut out).is_some());
        // Другой сайт того же бота — снова нужно согласие; правило на другой origin не переносится.
        assert!(!relay.allows("https://ozon.ru/"));
        assert!(!relay.allows("http://seller.ozon.ru/"), "схема входит в место");
        // Сессия закрыта — «один раз» забыт.
        relay.close();
        assert!(!relay.allows("https://seller.ozon.ru/app"));
    }

    #[test]
    fn a_tab_on_an_unallowed_site_can_only_leave() {
        let (relay, mut out, mut back) = relay("b-leave");
        attach(&relay, "s1", "T1", "https://evil.example/");
        answered(&mut back);
        relay.outbound(&json!({ "id": 1, "sessionId": "s1", "method": "DOM.getOuterHTML", "params": {} }).to_string());
        assert!(sent(&mut out).is_none());
        assert!(answered(&mut back).unwrap()["error"]["message"].as_str().unwrap().contains("evil.example"));
        relay.outbound(&json!({ "id": 2, "sessionId": "s1", "method": "Page.getNavigationHistory", "params": {} }).to_string());
        assert!(sent(&mut out).is_some(), "уйти можно");
    }

    #[test]
    fn top_navigation_to_an_unallowed_site_is_stopped_before_it_loads() {
        let (relay, mut out, mut back) = relay("b-nav");
        attach(&relay, "s1", "T1", "about:blank");
        answered(&mut back);
        let paused = |frame: &str, url: &str| {
            json!({ "method": "Fetch.requestPaused", "sessionId": "s1", "params": { "requestId": "r1", "resourceType": "Document", "frameId": frame, "request": { "url": url } } })
        };
        relay.inbound_message(paused("T1", "https://bank.example/login"));
        assert!(answered(&mut back).is_none(), "событие в Cdp не ушло");
        let fail = sent(&mut out).unwrap();
        assert_eq!(fail["msg"]["method"], "Fetch.failRequest");
        assert_eq!(fail["msg"]["params"]["errorReason"], "BlockedByClient");
        assert!(blocked_note(&relay).contains("https://bank.example"));
        assert!(blocked_note(&relay).is_empty(), "подсказка один раз");
        // Подфрейм на запрещённый сайт тоже обрывается.
        relay.inbound_message(paused("F2", "https://widget.example/"));
        assert!(answered(&mut back).is_none(), "событие в Cdp не ушло");
        let fail = sent(&mut out).unwrap();
        assert_eq!(fail["msg"]["method"], "Fetch.failRequest");
    }

    #[test]
    fn deny_list_covers_entries() {
        let data: Value = serde_json::from_str(include_str!("../extension/deny.json")).unwrap();
        for pat in data["deny"].as_array().unwrap() {
            let p = pat.as_str().unwrap();
            let hostpart = p.split('/').next().unwrap();
            let apex = hostpart.trim_start_matches("*.");
            let url = if p.contains('/') {
                format!("https://{}/{}", apex, p.split_once('/').unwrap().1.trim_start_matches('/'))
            } else {
                format!("https://{}/", apex)
            };
            assert!(denied_url(&url), "entry not denied: {} -> {}", p, url);
            if hostpart.starts_with("*.") {
                let suburl = if p.contains('/') {
                    format!("https://sub.{}/{}", apex, p.split_once('/').unwrap().1.trim_start_matches('/'))
                } else {
                    format!("https://sub.{}/", apex)
                };
                assert!(denied_url(&suburl), "subdomain not denied: {}", suburl);
            }
        }
        assert!(!denied_url("https://evilsberbank.ru/"));
        assert!(!denied_url("https://sberbank.ru.evil.com/"));
        assert!(denied_url("https://sberbank.ru./"));
    }

    #[test]
    fn denied_url_fails_closed_and_normalises_paths() {
        assert!(denied_url("not a url"));
        assert!(denied_url(""));
        assert!(denied_url("https://"));
        // chrome.google.com/webstore под любым написанием пути
        assert!(denied_url("https://chrome.google.com/webstore/detail/x"));
        assert!(denied_url("https://chrome.google.com//webstore"));
        assert!(denied_url("https://chrome.google.com/%77ebstore/category"));
        assert!(denied_url("https://chrome.google.com/web%73tore"));
        assert!(denied_url("https://chrome.google.com///webstore///x"));
        assert!(denied_url("https://chrome.google.com/WebStore/detail/x"));
        assert!(denied_url("https://Chrome.Google.com/%57EBSTORE"));
        assert!(!denied_url("https://chrome.google.com/other"));
        assert_eq!(normalise_path("//a///b%2Fc"), "/a/b/c");
        assert_eq!(normalise_path("/%zz%4"), "/%zz%4");
    }

    #[test]
    fn fetch_enable_from_bot_is_refused() {
        assert!(matches!(classify("Fetch.enable", &json!({}), false), Verdict::Refuse(_)));
    }

    #[test]
    fn unpairing_closes_every_live_link_so_nothing_is_picked() {
        use crate::pipe_auth::{Action, Conn, DIR_APP_TO_EXT, DIR_EXT_TO_APP};
        let (out, _out_rx) = mpsc::unbounded_channel();
        let mut app_side = Conn::app(Some([5u8; 32]), [1u8; 16]);
        let mut ext_side = Conn::new(Some([5u8; 32]), [2u8; 16], DIR_APP_TO_EXT, DIR_EXT_TO_APP);
        app_side.on_message(&ext_side.hello_text());
        ext_side.on_message(&app_side.hello_text());
        let ready = ext_side.seal(&json!({ "type": "ready" })).unwrap();
        assert!(matches!(app_side.on_message(&ready), Action::Deliver(_)));
        let link = Arc::new(Link::new(4242, out, app_side));
        HUB.links.lock().unwrap().push(link.clone());
        assert!(HUB.pick("").is_some_and(|l| l.id == 4242), "готовое подключение выбирается");
        HUB.close_all_links();
        assert!(HUB.pick("").is_none(), "после отвязки выбирать нечего");
        assert!(HUB.links.lock().unwrap().is_empty());
    }

    #[test]
    fn a_secret_passes_only_once_after_consent() {
        let (relay, mut out, mut back) = relay("b-secret");
        attach(&relay, "s1", "T1", "about:blank");
        answered(&mut back);
        let cookies = json!({ "id": 1, "sessionId": "s1", "method": "Network.getCookies", "params": {} }).to_string();
        relay.outbound(&cookies);
        assert!(sent(&mut out).is_none());
        assert_eq!(answered(&mut back).unwrap()["error"]["message"], SECRET_REFUSED);
        relay.unlock_secret_once();
        relay.outbound(&cookies);
        let frame = sent(&mut out).unwrap();
        assert_eq!(frame["unlock"], true, "расширение тоже видит, что это разрешено");
        relay.outbound(&cookies);
        assert!(sent(&mut out).is_none(), "разрешение было на один вызов");
    }
}

