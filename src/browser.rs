//! Браузер бота: настоящий Chrome или Edge со своим постоянным профилем.
//! Окно живёт за пределами экрана, фокус не забирает, и человек может вывести его к себе.
//! У каждого бота свой профиль: вход, сделанный для одного бота, другому недоступен.

use std::collections::{HashMap, HashSet, VecDeque};
use std::net::IpAddr;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::atomic::{AtomicBool, AtomicI64, AtomicU8, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use futures_util::future::join_all;
use serde_json::{json, Value};
use tokio::process::{Child, Command};
use tokio::sync::mpsc;

use crate::cdp::Cdp;
use crate::fetch::is_public_ip;
use crate::store::{now, Store};

const WINDOW_WIDTH: i64 = 1280;
const WINDOW_HEIGHT: i64 = 900;
const MAX_LOG: usize = 300;
/// Сколько ждём файл отладки встроенного WebView2. Внешнему Chrome хватает 15 с.
const EMBEDDED_PORT_WAIT: Duration = Duration::from_secs(15);

// ---------------------------------------------------------------------------
// Путь браузера: встроенный WebView2 или внешний Chrome/Edge
// ---------------------------------------------------------------------------

/// Фактический путь, которым пользуется Session. Встроенный режим пока не включён:
/// проверка на машине показала, что WebView2 не создаёт окружение в процессе Low integrity.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
#[allow(dead_code)]
pub enum Backend {
    WebView2,
    Chrome,
    Edge,
    /// Лёгкий движок без Chromium и без окна. Ставится только из настроек.
    Obscura,
    /// Лёгкий движок Moli: тоже без окна. В облаке основной.
    Moli,
    /// Настоящий Chrome человека через расширение Deloist (`user_chrome.rs`).
    UserChrome,
}

impl Backend {
    pub fn as_str(self) -> &'static str {
        match self {
            Backend::WebView2 => "webview2",
            Backend::Chrome => "chrome",
            Backend::Edge => "edge",
            Backend::Obscura => "obscura",
            Backend::Moli => "moli",
            Backend::UserChrome => "user_chrome",
        }
    }

    fn code(self) -> u8 {
        match self {
            Backend::WebView2 => 1,
            Backend::Chrome => 2,
            Backend::Edge => 3,
            Backend::Obscura => 4,
            Backend::UserChrome => 5,
            Backend::Moli => 6,
        }
    }
}

/// Последний запущенный backend: UI и диагностика видят фактический путь, а не догадку.
static LAST_BACKEND: AtomicU8 = AtomicU8::new(0);

fn remember_backend(backend: Backend) {
    LAST_BACKEND.store(backend.code(), Ordering::Relaxed);
}

/// Фактический путь последнего запущенного браузера: "webview2", "chrome" или "edge".
/// Пока ни один бот не открывал браузер, отвечает предположением по настройкам машины.
#[allow(dead_code)]
pub fn backend() -> &'static str {
    match LAST_BACKEND.load(Ordering::Relaxed) {
        1 => "webview2",
        2 => "chrome",
        3 => "edge",
        4 => "obscura",
        5 => "user_chrome",
        6 => "moli",
        _ => predicted_backend(),
    }
}

#[allow(dead_code)]
fn predicted_backend() -> &'static str {
    match find_browser("") {
        Ok(found) => found.backend.as_str(),
        Err(_) => "chrome",
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Mode {
    Auto,
    Embedded,
    External,
}

fn parse_mode(value: &str) -> Result<Mode, String> {
    match value.trim().to_ascii_lowercase().as_str() {
        "" | "auto" => Ok(Mode::Auto),
        "embedded" | "webview2" | "webview" => Ok(Mode::Embedded),
        "external" | "chrome" | "edge" => Ok(Mode::External),
        other => Err(format!("CREW_BROWSER={other}: ожидается auto, embedded или external")),
    }
}

fn mode_from_env() -> Result<Mode, String> {
    match std::env::var("CREW_BROWSER") {
        Ok(value) => parse_mode(&value),
        Err(_) => Ok(Mode::Auto),
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Plan {
    Embedded,
    External,
}

/// Встроенный режим включается только по явной просьбе и только после проверки.
/// По умолчанию (auto) внешний путь остаётся рабочим: встроенный пока не подтверждён.
fn choose_plan(mode: Mode, embedded_ok: bool) -> Result<Plan, String> {
    match mode {
        Mode::Embedded if !embedded_ok => Err(embedded_unavailable()),
        Mode::Embedded => Ok(Plan::Embedded),
        Mode::External => Ok(Plan::External),
        // В auto встроенный путь не выбираем: проверка на этой машине не прошла.
        Mode::Auto => Ok(Plan::External),
    }
}

fn embedded_unavailable() -> String {
    "встроенный WebView2 не включён: проверка показала, что WebView2 не создаёт окружение в процессе с пониженным уровнем целостности (Low integrity), а окна-хоста в этой сборке нет. Оставь CREW_BROWSER=auto или external: браузером бота будет Chrome/Edge. Подробности в отчёте.".to_string()
}

/// Аргументы WebView2. Если встроенный режим когда-нибудь включат, здесь уже есть
/// обязательные аргументы wry по умолчанию и запрос порта отладки.
#[allow(dead_code)]
pub(crate) fn embedded_browser_args() -> String {
    "--disable-features=msWebOOUI,msPdfOOUI,msSmartScreenProtection --remote-debugging-port=0 \
     --disable-background-timer-throttling --disable-renderer-backgrounding \
     --disable-backgrounding-occluded-windows"
        .to_string()
}

/// Первая строка файла — порт, вторая — путь WebSocket. Вторая строка бывает пустой.
#[derive(Clone, Debug, PartialEq, Eq)]
struct DevToolsEndpoint {
    port: u16,
    path: Option<String>,
}

fn parse_devtools_active_port(text: &str) -> Result<DevToolsEndpoint, String> {
    let mut lines = text.lines().map(str::trim).filter(|line| !line.is_empty());
    let port_line = lines.next().ok_or_else(|| "файл DevToolsActivePort пуст".to_string())?;
    let port: u16 = port_line.parse().map_err(|_| format!("первая строка DevToolsActivePort не порт: {port_line:?}"))?;
    if port == 0 {
        return Err("DevToolsActivePort содержит порт 0".into());
    }
    let path = lines.next().filter(|line| line.starts_with('/')).map(str::to_string);
    Ok(DevToolsEndpoint { port, path })
}

impl DevToolsEndpoint {
    fn ws_url(&self, from_version: Option<&str>) -> Option<String> {
        if let Some(url) = from_version {
            return Some(url.to_string());
        }
        self.path.as_ref().map(|path| format!("ws://127.0.0.1:{}{path}", self.port))
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct VersionInfo {
    browser: String,
    protocol: Option<String>,
    ws: Option<String>,
}

fn parse_json_version(text: &str) -> Result<VersionInfo, String> {
    let value: Value = serde_json::from_str(text).map_err(|e| format!("/json/version вернул не JSON: {e}"))?;
    let browser = value["Browser"].as_str().filter(|text| !text.is_empty()).ok_or_else(|| "/json/version без поля Browser".to_string())?;
    Ok(VersionInfo {
        browser: browser.to_string(),
        protocol: value["Protocol-Version"].as_str().map(str::to_string),
        ws: value["webSocketDebuggerUrl"].as_str().map(str::to_string),
    })
}

fn version_url(port: u16) -> String {
    format!("http://127.0.0.1:{port}/json/version")
}

/// Документ WebView2/Chromium может лежать и в профиле, и в подпапке EBWebView.
fn devtools_port_files(profile: &Path) -> [PathBuf; 2] {
    [profile.join("DevToolsActivePort"), profile.join("EBWebView").join("DevToolsActivePort")]
}

async fn wait_for_devtools_port(profile: &Path) -> Result<DevToolsEndpoint, String> {
    let deadline = Instant::now() + EMBEDDED_PORT_WAIT;
    loop {
        for file in devtools_port_files(profile) {
            if let Ok(text) = tokio::fs::read_to_string(&file).await {
                if let Ok(endpoint) = parse_devtools_active_port(&text) {
                    return Ok(endpoint);
                }
            }
        }
        if Instant::now() >= deadline {
            return Err(format!(
                "WebView2 не создал файл DevToolsActivePort за {} с: он мог проигнорировать --remote-debugging-port",
                EMBEDDED_PORT_WAIT.as_secs()
            ));
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

async fn json_version(port: u16) -> Result<VersionInfo, String> {
    let client = reqwest::Client::builder()
        .no_proxy()
        .timeout(Duration::from_secs(3))
        .build()
        .map_err(|e| format!("не смог собрать HTTP-клиент: {e}"))?;
    let response = client
        .get(version_url(port))
        .send()
        .await
        .map_err(|e| format!("/json/version на порту {port} не отвечает: {e}"))?;
    if !response.status().is_success() {
        return Err(format!("/json/version на порту {port} ответил {}", response.status()));
    }
    let text = response.text().await.map_err(|e| format!("/json/version на порту {port}: {e}"))?;
    parse_json_version(&text)
}

/// Окружения WebView2 не подтверждено: окна-хоста нет, поэтому честно говорим об этом.
#[allow(dead_code)]
async fn verify_embedded(profile: &Path) -> Result<String, String> {
    let endpoint = wait_for_devtools_port(profile).await?;
    let version = json_version(endpoint.port).await?;
    endpoint.ws_url(version.ws.as_deref()).ok_or_else(|| "/json/version не дал WebSocket и в DevToolsActivePort нет пути".to_string())
}

#[derive(Clone, Debug)]
struct FoundBrowser {
    path: PathBuf,
    backend: Backend,
}

fn external_backend(path: &Path) -> Backend {
    let name = path.file_name().and_then(|name| name.to_str()).unwrap_or_default().to_ascii_lowercase();
    if name == "msedge.exe" || name == "microsoft-edge" || name == "microsoft-edge-stable" {
        Backend::Edge
    } else {
        // Chromium и Brave — тот же движок, backend для них "chrome".
        Backend::Chrome
    }
}

/// Все привычные места установки. Порядок: сначала Chrome, потом Edge, Chromium, Brave.
#[cfg(not(windows))]
fn standard_browser_paths() -> Vec<FoundBrowser> {
    // Вне Windows браузер ищем по PATH: так он лежит и в пакете, и в контейнере.
    let names = [
        ("google-chrome", Backend::Chrome),
        ("google-chrome-stable", Backend::Chrome),
        ("microsoft-edge", Backend::Edge),
        ("microsoft-edge-stable", Backend::Edge),
        ("chromium", Backend::Chrome),
        ("chromium-browser", Backend::Chrome),
        ("brave-browser", Backend::Chrome),
    ];
    let dirs: Vec<PathBuf> = std::env::var_os("PATH").map(|path| std::env::split_paths(&path).collect()).unwrap_or_default();
    let mut found = Vec::new();
    for (name, backend) in names {
        for dir in &dirs {
            found.push(FoundBrowser { path: dir.join(name), backend });
        }
    }
    found
}

#[cfg(windows)]
fn standard_browser_paths() -> Vec<FoundBrowser> {
    let env_dir = |name: &str| std::env::var_os(name).map(PathBuf::from);
    let programs = [env_dir("ProgramFiles"), env_dir("ProgramFiles(x86)"), env_dir("LOCALAPPDATA")];
    let mut found = Vec::new();
    for base in programs.into_iter().flatten() {
        found.push(FoundBrowser { path: base.join(r"Google\Chrome\Application\chrome.exe"), backend: Backend::Chrome });
        found.push(FoundBrowser { path: base.join(r"Microsoft\Edge\Application\msedge.exe"), backend: Backend::Edge });
        found.push(FoundBrowser { path: base.join(r"Chromium\Application\chrome.exe"), backend: Backend::Chrome });
        found.push(FoundBrowser { path: base.join(r"BraveSoftware\Brave-Browser\Application\brave.exe"), backend: Backend::Chrome });
    }
    found
}

fn find_browser(configured: &str) -> Result<FoundBrowser, String> {
    if !configured.trim().is_empty() {
        let path = PathBuf::from(configured.trim());
        if !path.exists() {
            return Err(format!(
                "браузер не найден по пути из настроек (browser_path): {}. Проверь путь или очисти поле, чтобы искать Chrome, Edge, Chromium и Brave автоматически",
                path.display()
            ));
        }
        if !path.is_file() {
            return Err(format!("поле browser_path указывает на папку, а не на exe: {}", path.display()));
        }
        let is_exe = !cfg!(windows) || path.extension().and_then(|ext| ext.to_str()).is_some_and(|ext| ext.eq_ignore_ascii_case("exe"));
        if !is_exe {
            return Err(format!("поле browser_path должно указывать на .exe, а не на файл: {}", path.display()));
        }
        return Ok(FoundBrowser { backend: external_backend(&path), path });
    }
    let candidates = standard_browser_paths();
    if let Some(found) = candidates.iter().find(|found| found.path.is_file()) {
        return Ok(found.clone());
    }
    Err("не нашёл Chrome, Edge, Chromium или Brave: укажи путь к exe в настройках, в поле browser_path".into())
}

const OBSCURA_MISSING: &str = "В настройках выбран лёгкий браузер Obscura, но он не установлен. Скажи человеку: Настройки → Браузер, скачать Obscura или вернуть Chrome.";

/// Ответ модели, когда нужен человек: у Obscura нет окна, войти или пройти проверку в нём нельзя.
const OBSCURA_NO_WINDOW: &str = "У лёгкого браузера Obscura нет окна, человеку показать нечего. Вход, пароли, коды и проверки «я не робот» делаются только в Chrome: попроси человека переключить браузер в Настройки → Браузер на Chrome и повтори.";

const MOLI_MISSING: &str = "В настройках выбран лёгкий браузер Moli, но он не установлен. Скажи человеку: Настройки → Браузер, скачать Moli или вернуть Chrome.";

/// Ответ модели, когда нужен человек: у Moli нет окна, войти или пройти проверку в нём нельзя.
const MOLI_NO_WINDOW: &str = "У лёгкого браузера Moli нет окна, человеку показать нечего. Вход, пароли, коды и проверки «я не робот» делаются только в Chrome: попроси человека переключить браузер в Настройки → Браузер на Chrome и повтори.";

/// Подсказка, если внешний браузер отказался стартовать.
fn low_integrity_hint() -> &'static str {
    "Программа работает из папки с пониженным уровнем целостности (Low integrity), поэтому Chrome/Edge отказываются запускаться. Перенеси deloist.exe вместе с WebView2Loader.dll в обычную папку, например %LOCALAPPDATA%\\Programs\\Deloist, и запусти оттуда."
}

/// Бот не ходит на эту машину и в локальную сеть. Обход только для тестов и только в отладочной сборке.
pub fn address_allowed(ip: IpAddr) -> bool {
    if cfg!(debug_assertions) && ip.is_loopback() && std::env::var_os("CREW_TEST_ALLOW_LOOPBACK").is_some() {
        return true;
    }
    is_public_ip(ip)
}

// ---------------------------------------------------------------------------
// Состояние вкладок
// ---------------------------------------------------------------------------

#[derive(Clone, Default)]
pub struct Request {
    pub id: String,
    pub method: String,
    pub url: String,
    pub kind: String,
    pub status: i64,
    pub mime: String,
    pub request_headers: Value,
    pub response_headers: Value,
}

#[derive(Clone)]
pub struct Script {
    pub id: String,
    pub url: String,
    pub length: i64,
}

pub struct Page {
    pub session: String,
    pub url: String,
    pub title: String,
    pub ready: bool,
    pub refs: HashMap<u32, i64>,
    pub console: VecDeque<String>,
    pub requests: VecDeque<Request>,
    pub scripts: Vec<Script>,
    pub paused: Option<Value>,
    pub dialog: Option<Value>,
    pub mouse: (f64, f64),
    pub debugger_on: bool,
    pub last_net: i64,
    /// Код бота выполнялся в документе этой вкладки. Сбрасывается только новой загрузкой главного кадра
    /// (`Page.frameNavigated`), не сменой адреса внутри документа (`pushState`, `#`). Решает не он,
    /// а набор `Tabs::tainted` по origin: он переживает открытие страницы в другой вкладке и возврат по истории.
    pub evaled: bool,
    /// Узлы (backend id), куда программа вставила пароль или код: их значение в снимке всегда скрыто.
    /// Сбрасывается вместе с документом.
    pub secret_nodes: HashSet<i64>,
    /// Узлы из номеров последнего снимка, найденные в дочерних кадрах (не в главном документе).
    pub child_nodes: HashSet<i64>,
}

#[derive(Default)]
pub struct Tabs {
    pub current: Option<String>,
    pub order: Vec<String>,
    pub pages: HashMap<String, Page>,
    pub by_session: HashMap<String, String>,
    pub new_tab: bool,
    pub downloads: Vec<String>,
    names: HashMap<String, String>,
    /// Origin, на которых в этой сессии выполнялся код бота (eval, условие точки останова), и вкладки,
    /// где это случилось. Вход паролем на таком origin не делаем: страница в любой вкладке могла быть
    /// подготовлена кодом (`window.open`, возврат по истории из bfcache).
    tainted: HashMap<String, HashSet<String>>,
    /// Кадры трансляции вкладки (`Page.screencastFrame`) уходят экрану бота, пока он открыт.
    pub frame_tx: Option<mpsc::UnboundedSender<(String, Value)>>,
}

impl Tabs {
    /// Код бота выполнился во вкладке `target` на странице с этим адресом.
    pub fn mark_script_ran(&mut self, target: &str, urls: &[&str]) {
        if let Some(page) = self.pages.get_mut(target) {
            page.evaled = true;
        }
        for url in urls {
            if let Some(origin) = url_origin(url) {
                self.tainted.entry(origin).or_default().insert(target.to_string());
            }
        }
    }

    /// Выполнялся ли код бота на этом origin в этой сессии браузера. Отметка не снимается до его перезапуска.
    pub fn origin_tainted(&self, origin: &str) -> bool {
        self.tainted.contains_key(origin)
    }

    /// Новая вкладка, открытая из вкладки с кодом бота (`window.open`), наследует отметку.
    pub fn inherit_taint(&mut self, opener: &str, target: &str) {
        for owners in self.tainted.values_mut() {
            if owners.contains(opener) {
                owners.insert(target.to_string());
            }
        }
    }
}

/// Origin адреса страницы: только http и https с хостом.
pub fn url_origin(url: &str) -> Option<String> {
    let parsed = reqwest::Url::parse(url).ok()?;
    if !matches!(parsed.scheme(), "http" | "https") || parsed.host_str().is_none() {
        return None;
    }
    Some(parsed.origin().ascii_serialization())
}

fn push_limited<T>(queue: &mut VecDeque<T>, item: T) {
    if queue.len() >= MAX_LOG {
        queue.pop_front();
    }
    queue.push_back(item);
}

fn console_args(args: &Value) -> String {
    args.as_array()
        .map(|items| {
            items
                .iter()
                .map(|arg| match arg.get("value") {
                    Some(Value::String(text)) => text.clone(),
                    Some(other) => other.to_string(),
                    None => arg["description"].as_str().unwrap_or_default().to_string(),
                })
                .collect::<Vec<_>>()
                .join(" ")
        })
        .unwrap_or_default()
}

// ---------------------------------------------------------------------------
// Запуск
// ---------------------------------------------------------------------------

/// Браузер закрывается вместе с программой, даже если она упала.
#[cfg(windows)]
pub(crate) fn tie_to_this_process(child: &Child) {
    use std::sync::OnceLock;
    use windows_sys::Win32::System::JobObjects::{
        AssignProcessToJobObject, CreateJobObjectW, JobObjectExtendedLimitInformation, SetInformationJobObject,
        JOBOBJECT_EXTENDED_LIMIT_INFORMATION, JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
    };
    static JOB: OnceLock<usize> = OnceLock::new();
    // Вызовы WinAPI с локальной структурой нужного размера; дескриптор живёт до конца процесса.
    unsafe {
        let job = *JOB.get_or_init(|| {
            let job = CreateJobObjectW(core::ptr::null(), core::ptr::null());
            let mut info: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = core::mem::zeroed();
            info.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
            SetInformationJobObject(
                job,
                JobObjectExtendedLimitInformation,
                &info as *const _ as *const core::ffi::c_void,
                core::mem::size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
            );
            job as usize
        });
        if let Some(handle) = child.raw_handle() {
            AssignProcessToJobObject(job as _, handle as _);
        }
    }
}

#[cfg(not(windows))]
pub(crate) fn tie_to_this_process(_child: &Child) {}

/// Левый верхний угол за пределами всех мониторов.
#[cfg(windows)]
fn offscreen_position() -> (i64, i64) {
    #[link(name = "user32")]
    extern "system" {
        fn GetSystemMetrics(index: i32) -> i32;
    }
    // SM_XVIRTUALSCREEN = 76, SM_YVIRTUALSCREEN = 77. Вызов без указателей.
    let (x, y) = unsafe { (GetSystemMetrics(76), GetSystemMetrics(77)) };
    (x as i64 - WINDOW_WIDTH - 2000, y as i64 - WINDOW_HEIGHT - 2000)
}

#[cfg(not(windows))]
fn offscreen_position() -> (i64, i64) {
    (-4000, -4000)
}

// ---------------------------------------------------------------------------
// Сессия
// ---------------------------------------------------------------------------

pub struct Session {
    pub cdp: Arc<Cdp>,
    pub tabs: Arc<Mutex<Tabs>>,
    pub workspace: PathBuf,
    child: Option<tokio::sync::Mutex<Child>>,
    backend: Backend,
    last_used: AtomicI64,
    pub shown: AtomicBool,
    /// Только у «моего Chrome»: фильтр и согласия этой сессии.
    pub relay: Option<Arc<crate::user_chrome::BotRelay>>,
}

impl Session {
    /// Фактический путь этого бота: "webview2", "chrome" или "edge".
    pub fn backend(&self) -> &'static str {
        self.backend.as_str()
    }

    async fn launch(store: &Store, bot_id: &str) -> Result<Arc<Self>, String> {
        let mode = mode_from_env()?;
        // Встроенный режим пока не подтверждён (в Low integrity WebView2 не создаёт окружение),
        // поэтому автоматически он не выбирается: рабочий внешний путь остаётся основным.
        let plan = choose_plan(mode, false)?;
        if plan == Plan::Embedded {
            return Err(embedded_unavailable());
        }
        let settings = store.settings();
        let workspace = store.workspace(bot_id);
        let downloads = workspace.join("downloads");
        tokio::fs::create_dir_all(&downloads).await.map_err(|e| e.to_string())?;
        if !crate::cloud::active() && crate::user_chrome::wanted(store, bot_id) {
            let (cdp, events, relay) = crate::user_chrome::open(store, bot_id)?;
            let session = Self::start_with(cdp, events, Backend::UserChrome, workspace, downloads, None, Some(relay)).await?;
            // Своя вкладка бота в его группе; вкладки человека бот не трогает.
            session.cdp.call(None, "Target.createTarget", json!({ "url": "about:blank" })).await?;
            remember_backend(Backend::UserChrome);
            return Ok(session);
        }
        // Облако: только Moli из образа. Настройка движка и путь к браузеру из настроек до запуска не доходят.
        let engine = if crate::cloud::active() { "moli" } else { crate::moli::engine_choice(&settings.browser_engine, false) };
        if engine == "moli" {
            let exe = crate::moli::installed_exe(store.data_dir()).ok_or(MOLI_MISSING)?;
            let profile = store.moli_profile(bot_id);
            tokio::fs::create_dir_all(&profile).await.map_err(|e| e.to_string())?;
            return Self::launch_moli(exe, profile, workspace, downloads).await;
        }
        if engine == "obscura" {
            let exe = crate::obscura::installed_exe(store.data_dir()).ok_or(OBSCURA_MISSING)?;
            let profile = store.obscura_profile(bot_id);
            tokio::fs::create_dir_all(&profile).await.map_err(|e| e.to_string())?;
            return Self::launch_obscura(exe, profile, workspace, downloads).await;
        }
        crate::cloud::deny(crate::cloud::NO_FEATURE)?;
        let found = find_browser(&settings.browser_path)?;
        let profile = store.browser_profile(bot_id);
        tokio::fs::create_dir_all(&profile).await.map_err(|e| e.to_string())?;
        Self::launch_external(found, profile, workspace, downloads).await
    }

    /// Obscura: отдельный процесс `serve` на свободном порту, адрес он пишет в ready-файл.
    /// Окна нет, частную сеть он закрывает сам, и наш перехват адресов работает поверх этого.
    async fn launch_obscura(exe: PathBuf, profile: PathBuf, workspace: PathBuf, downloads: PathBuf) -> Result<Arc<Self>, String> {
        crate::cloud::deny(crate::cloud::NO_FEATURE)?;
        let ready = profile.join("ready.json");
        let _ = tokio::fs::remove_file(&ready).await;
        let mut command = Command::new(&exe);
        command
            // Ключи и токены из окружения программы движку не нужны.
            .env_clear()
            .envs(crate::process::safe_env_pairs())
            .arg("--storage-dir")
            .arg(profile.join("storage"));
        // Тестовый сайт живёт на этой машине: только в отладочной сборке и только по флагу теста.
        if cfg!(debug_assertions) && std::env::var_os("CREW_TEST_ALLOW_LOOPBACK").is_some() {
            command.arg("--allow-private-network");
        }
        command
            .args(["serve", "--port", "0", "--quiet", "--ready-file"])
            .arg(&ready)
            .current_dir(exe.parent().unwrap_or(&profile))
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .kill_on_drop(true);
        #[cfg(windows)]
        command.creation_flags(0x0800_0000); // консольная программа: без окна консоли
        let mut child = command.spawn().map_err(|e| format!("не смог запустить Obscura {}: {e}", exe.display()))?;
        tie_to_this_process(&child);

        let url = wait_for_ready(&ready, &mut child).await?;
        let session = Self::start(&url, Backend::Obscura, workspace, downloads, Some(child)).await?;
        // У Chrome первая вкладка открывается при запуске. Obscura стартует пустым.
        session.cdp.call(None, "Target.createTarget", json!({ "url": "about:blank" })).await?;
        remember_backend(Backend::Obscura);
        Ok(session)
    }

    /// Moli: отдельный процесс `serve` на свободном порту. Ready-файла у него нет, поэтому порт выбираем сами
    /// и ждём ответа `/json/version`. Окна нет; частную сеть закрываем его же ключом, наш перехват адресов работает поверх.
    async fn launch_moli(exe: PathBuf, profile: PathBuf, workspace: PathBuf, downloads: PathBuf) -> Result<Arc<Self>, String> {
        let port = free_port().map_err(|e| format!("нет свободного порта для Moli: {e}"))?;
        let mut command = Command::new(&exe);
        command
            // Ключи и токены из окружения программы движку не нужны.
            .env_clear()
            .envs(crate::process::safe_env_pairs())
            .args(["serve", "--layout", "--host", "127.0.0.1", "--port"])
            .arg(port.to_string())
            .args(["--timeout", "30", "--profile-dir"])
            .arg(profile.join("storage"));
        // Тестовый сайт живёт на этой машине: только в отладочной сборке и только по флагу теста.
        if !(cfg!(debug_assertions) && std::env::var_os("CREW_TEST_ALLOW_LOOPBACK").is_some()) {
            command.arg("--block-private-networks");
        }
        command
            .current_dir(exe.parent().unwrap_or(&profile))
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .kill_on_drop(true);
        #[cfg(windows)]
        command.creation_flags(0x0800_0000); // консольная программа: без окна консоли
        let mut child = command.spawn().map_err(|e| format!("не смог запустить Moli {}: {e}", exe.display()))?;
        tie_to_this_process(&child);

        let url = wait_for_moli(port, &mut child).await?;
        let session = Self::start(&url, Backend::Moli, workspace, downloads, Some(child)).await?;
        // У Chrome первая вкладка открывается при запуске. Moli стартует с пустой.
        session.cdp.call(None, "Target.createTarget", json!({ "url": "about:blank" })).await?;
        remember_backend(Backend::Moli);
        Ok(session)
    }

    async fn launch_external(found: FoundBrowser, profile: PathBuf, workspace: PathBuf, downloads: PathBuf) -> Result<Arc<Self>, String> {
        crate::cloud::deny(crate::cloud::NO_FEATURE)?;
        let port_file = profile.join("DevToolsActivePort");
        let _ = tokio::fs::remove_file(&port_file).await;

        mark_profile_clean(&profile);
        let mut command = Command::new(&found.path);
        // Ключи и токены из окружения программы браузеру не нужны.
        command.env_clear().envs(crate::process::safe_env_pairs());
        command.args(external_args(&profile, std::env::var_os("CREW_BROWSER_HEADLESS").is_some()));
        command.stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null()).kill_on_drop(true);
        let mut child = command.spawn().map_err(|e| format!("не смог запустить браузер {}: {e}", found.path.display()))?;
        tie_to_this_process(&child);

        let url = wait_for_port(&port_file, &mut child).await.map_err(|e| {
            if lowered_integrity() {
                format!("{e}. {}", low_integrity_hint())
            } else {
                e
            }
        })?;
        let session = Self::start(&url, found.backend, workspace, downloads, Some(child)).await?;
        remember_backend(found.backend);
        Ok(session)
    }

    /// Общая часть внешнего и будущего встроенного пути: подключение CDP и подготовка вкладок.
    async fn start(url: &str, backend: Backend, workspace: PathBuf, downloads: PathBuf, child: Option<Child>) -> Result<Arc<Self>, String> {
        let (cdp, events) = Cdp::connect(url).await?;
        Self::start_with(cdp, events, backend, workspace, downloads, child, None).await
    }

    async fn start_with(
        cdp: Arc<Cdp>,
        events: mpsc::UnboundedReceiver<Value>,
        backend: Backend,
        workspace: PathBuf,
        downloads: PathBuf,
        child: Option<Child>,
        relay: Option<Arc<crate::user_chrome::BotRelay>>,
    ) -> Result<Arc<Self>, String> {
        let tabs = Arc::new(Mutex::new(Tabs::default()));
        tokio::spawn(pump(cdp.clone(), tabs.clone(), events));

        cdp.call(
            None,
            "Browser.setDownloadBehavior",
            json!({ "behavior": "allow", "downloadPath": downloads.display().to_string(), "eventsEnabled": true }),
        )
        .await?;
        cdp.call(None, "Target.setDiscoverTargets", json!({ "discover": true })).await?;
        cdp.call(None, "Target.setAutoAttach", json!({ "autoAttach": true, "waitForDebuggerOnStart": true, "flatten": true })).await?;

        Ok(Arc::new(Self {
            cdp,
            tabs,
            workspace,
            child: child.map(tokio::sync::Mutex::new),
            backend,
            last_used: AtomicI64::new(now()),
            shown: AtomicBool::new(false),
            relay,
        }))
    }

    fn alive(&self) -> bool {
        !self.cdp.is_closed()
    }

    fn touch(&self) {
        self.last_used.store(now(), Ordering::Relaxed);
    }

    /// Текущая вкладка, готовая к работе.
    pub async fn session_id(&self) -> Result<String, String> {
        // Пятнадцать секунд: на занятом компьютере новая вкладка не успевала за пять, и шаг падал без причины.
        for _ in 0..300 {
            {
                let tabs = self.tabs.lock().unwrap();
                if let Some(page) = tabs.current.as_ref().and_then(|id| tabs.pages.get(id)) {
                    if page.ready {
                        return Ok(page.session.clone());
                    }
                }
            }
            if !self.alive() {
                return Err("браузер закрыт".into());
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        Err("вкладка не готова".into())
    }

    /// Текущая вкладка без ожидания: экран бота опрашивает её сам и не должен висеть на старте вкладки.
    pub fn current_session(&self) -> Option<String> {
        let tabs = self.tabs.lock().unwrap();
        tabs.current.as_ref().and_then(|id| tabs.pages.get(id)).filter(|page| page.ready).map(|page| page.session.clone())
    }

    pub fn is_obscura(&self) -> bool {
        self.backend == Backend::Obscura
    }

    /// Лёгкие движки без окна (Obscura, Moli): человеку показать нечего, правила окон и входа к ним не относятся.
    pub fn is_windowless(&self) -> bool {
        matches!(self.backend, Backend::Obscura | Backend::Moli)
    }

    pub fn is_moli(&self) -> bool {
        self.backend == Backend::Moli
    }

    pub async fn call(&self, method: &str, params: Value) -> Result<Value, String> {
        self.touch();
        let session = self.session_id().await?;
        self.cdp.call(Some(&session), method, params).await
    }

    /// Ввод мышью и клавиатурой. Браузер отвечает на него, только когда страница обработала событие,
    /// а диалог или точка останова её держат: тогда ответа не ждём, это обычный исход клика.
    pub async fn input(&self, method: &str, params: Value) -> Result<(), String> {
        let call = self.call(method, params);
        tokio::pin!(call);
        loop {
            tokio::select! {
                result = &mut call => return result.map(|_| ()),
                _ = tokio::time::sleep(Duration::from_millis(50)) => {
                    if self.with_page(|page| page.dialog.is_some() || page.paused.is_some()).unwrap_or(false) {
                        return Ok(());
                    }
                }
            }
        }
    }

    /// Код бота выполняется на текущей странице (и, если нужно, на других адресах из `extra`): запоминаем origin.
    pub fn mark_script_ran(&self, extra: &[&str]) -> Result<(), String> {
        let mut tabs = self.tabs.lock().unwrap();
        let id = tabs.current.clone().ok_or("нет открытой вкладки")?;
        let url = tabs.pages.get(&id).map(|page| page.url.clone()).ok_or("нет открытой вкладки")?;
        let mut urls = vec![url.as_str()];
        urls.extend_from_slice(extra);
        tabs.mark_script_ran(&id, &urls);
        Ok(())
    }

    /// Выполнялся ли код бота на этом origin в этой сессии, или в текущем документе вкладки.
    pub fn code_ran_on(&self, origin: &str) -> bool {
        let tabs = self.tabs.lock().unwrap();
        tabs.origin_tainted(origin) || tabs.current.as_ref().and_then(|id| tabs.pages.get(id)).is_some_and(|page| page.evaled)
    }

    pub fn with_page<R>(&self, read: impl FnOnce(&mut Page) -> R) -> Result<R, String> {
        let mut tabs = self.tabs.lock().unwrap();
        let id = tabs.current.clone().ok_or("нет открытой вкладки")?;
        let page = tabs.pages.get_mut(&id).ok_or("нет открытой вкладки")?;
        Ok(read(page))
    }

    async fn window(&self) -> Result<i64, String> {
        let result = self.call("Browser.getWindowForTarget", json!({})).await?;
        result["windowId"].as_i64().ok_or_else(|| "нет окна".to_string())
    }

    /// Окно выходит к человеку: на экран и на передний план.
    pub async fn show(&self) -> Result<(), String> {
        if self.backend == Backend::Obscura {
            return Err(OBSCURA_NO_WINDOW.into());
        }
        if self.backend == Backend::Moli {
            return Err(MOLI_NO_WINDOW.into());
        }
        self.shown.store(true, Ordering::Relaxed);
        // Окно человека не двигаем: вкладка бота просто выходит вперёд.
        if self.backend == Backend::UserChrome {
            return self.call("Page.bringToFront", json!({})).await.map(|_| ());
        }
        if let Ok(window) = self.window().await {
            let bounds = json!({ "left": 80, "top": 60, "width": WINDOW_WIDTH, "height": WINDOW_HEIGHT });
            let _ = self.cdp.call(None, "Browser.setWindowBounds", json!({ "windowId": window, "bounds": bounds })).await;
        }
        self.call("Page.bringToFront", json!({})).await.map(|_| ())
    }

    /// Окно снова уходит за пределы экрана и не мешает человеку.
    pub async fn hide(&self) {
        if self.is_windowless() {
            return;
        }
        self.shown.store(false, Ordering::Relaxed);
        if self.backend == Backend::UserChrome {
            return;
        }
        if let Ok(window) = self.window().await {
            let (left, top) = offscreen_position();
            let bounds = json!({ "left": left, "top": top });
            let _ = self.cdp.call(None, "Browser.setWindowBounds", json!({ "windowId": window, "bounds": bounds })).await;
        }
    }

    /// Закрытие штатное: браузер успевает записать cookies на диск.
    async fn close(&self) {
        // У «моего Chrome» Browser.close только отпускает вкладки бота: браузер человека не закрывается.
        let _ = tokio::time::timeout(Duration::from_secs(3), self.cdp.call(None, "Browser.close", json!({}))).await;
        if let Some(relay) = &self.relay {
            crate::user_chrome::release(relay);
        }
        let Some(child) = self.child.as_ref() else { return };
        let mut child = child.lock().await;
        if tokio::time::timeout(Duration::from_secs(3), child.wait()).await.is_err() {
            let _ = child.kill().await;
        }
    }
}

/// Процессу закрыта запись в собственный профиль: так выглядит метка Low integrity.
fn lowered_integrity() -> bool {
    let Some(profile) = std::env::var_os("LOCALAPPDATA").map(PathBuf::from) else { return false };
    let probe = profile.join(format!("crew-probe-{}", now()));
    match std::fs::write(&probe, b"") {
        Ok(()) => {
            let _ = std::fs::remove_file(&probe);
            false
        }
        Err(e) => e.kind() == std::io::ErrorKind::PermissionDenied,
    }
}

/// Obscura пишет адрес в ready-файл, когда порт уже слушается.
async fn wait_for_ready(ready: &Path, child: &mut Child) -> Result<String, String> {
    for _ in 0..150 {
        if let Ok(text) = tokio::fs::read_to_string(ready).await {
            if !text.trim().is_empty() {
                return crate::obscura::parse_ready_file(&text);
            }
        }
        if let Ok(Some(status)) = child.try_wait() {
            return Err(format!("Obscura завершился при запуске с кодом {:?}", status.code()));
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    Err("Obscura не поднял порт отладки за 15 с".into())
}

/// Свободный порт на loopback: занимаем и сразу отпускаем. Гонка возможна, но Moli при занятом порту завершится,
/// и `wait_for_moli` это увидит.
fn free_port() -> std::io::Result<u16> {
    let listener = std::net::TcpListener::bind("127.0.0.1:0")?;
    Ok(listener.local_addr()?.port())
}

/// Moli отвечает на `/json/version`, когда порт уже слушается. Чужой ответ на этом порту не принимаем.
async fn wait_for_moli(port: u16, child: &mut Child) -> Result<String, String> {
    let client = reqwest::Client::builder().timeout(Duration::from_secs(1)).no_proxy().build().map_err(|e| e.to_string())?;
    let url = format!("http://127.0.0.1:{port}/json/version");
    for _ in 0..150 {
        if let Ok(response) = client.get(&url).send().await {
            if let Ok(text) = response.text().await {
                if let Ok(ws) = crate::moli::parse_version(&text) {
                    return Ok(ws);
                }
            }
        }
        if let Ok(Some(status)) = child.try_wait() {
            return Err(format!("Moli завершился при запуске с кодом {:?}", status.code()));
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    Err("Moli не поднял порт отладки за 15 с".into())
}

async fn wait_for_port(port_file: &Path, child: &mut Child) -> Result<String, String> {
    let mut handed_off = false;
    for _ in 0..150 {
        if let Ok(text) = tokio::fs::read_to_string(port_file).await {
            let mut lines = text.lines();
            if let (Some(port), Some(path)) = (lines.next(), lines.next()) {
                return Ok(format!("ws://127.0.0.1:{port}{path}"));
            }
        }
        if !handed_off {
            if let Ok(Some(status)) = child.try_wait() {
                launch_exit(status.code())?;
                handed_off = true;
            }
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    Err("браузер не поднял порт отладки за 15 с: файл DevToolsActivePort не появился. Проверь путь browser_path и права на папку профиля".into())
}

/// Командная строка браузера бота: обычный Chrome/Edge со своим постоянным профилем.
///
/// `--disable-blink-features=AutomationControlled`: порт отладки сам по себе включает в Blink
/// признак автоматизации, и сайт видит `navigator.webdriver === true` (замер 11.11.2026 на
/// Chrome 157: без порта `false`, с `--remote-debugging-port=0` — `true`, остальной отпечаток
/// тот же). Ozon по этому признаку отвечал «отключите VPN». Флаг возвращает браузер к тому, каким
/// он запускается у человека; это не подмена из JS. Так же запускает браузер человека OpenBot.
/// `--enable-automation` не ставим: он включает тот же признак и полосу «управляется программой».
fn external_args(profile: &Path, headless: bool) -> Vec<String> {
    let (left, top) = offscreen_position();
    let mut args = vec![
        "--remote-debugging-port=0".to_string(),
        format!("--user-data-dir={}", profile.display()),
        "--no-first-run".into(),
        "--no-default-browser-check".into(),
        // Программу закрыли вместе с браузером: без этого Chrome при следующем запуске показывает
        // «Восстановить страницы? Работа Chrome была завершена некорректно» (владелец, 12.11.2026).
        "--hide-crash-restore-bubble".into(),
        "--disable-blink-features=AutomationControlled".into(),
        // Окно за пределами экрана и без фокуса: страницы при этом должны работать как обычно.
        "--disable-backgrounding-occluded-windows".into(),
        "--disable-renderer-backgrounding".into(),
        "--disable-background-timer-throttling".into(),
        "--start-minimized".into(),
        format!("--window-position={left},{top}"),
        format!("--window-size={WINDOW_WIDTH},{WINDOW_HEIGHT}"),
    ];
    // Вне Windows: без экрана (сервер, контейнер) окно открыть негде, поэтому браузер без окна.
    #[cfg(not(windows))]
    let headless = headless || (std::env::var_os("DISPLAY").is_none() && std::env::var_os("WAYLAND_DISPLAY").is_none());
    if headless {
        args.push("--headless=new".into());
    }
    #[cfg(not(windows))]
    {
        // В контейнере /dev/shm мал, и Chromium падает на тяжёлых страницах.
        args.push("--disable-dev-shm-usage".into());
        // Клетка Chromium в контейнере часто недоступна. Отключается только явно: образ ставит это сам,
        // потому что изоляцию даёт контейнер.
        if std::env::var_os("CREW_BROWSER_NO_SANDBOX").is_some_and(|v| v == "1") {
            args.push("--no-sandbox".into());
        }
    }
    args.push("about:blank".into());
    args
}

/// Профиль бота закрывается вместе с программой, часто без «нормального» выхода Chrome.
/// Помечаем прошлый выход чистым, чтобы Chrome не предлагал восстановить вкладки (на старых
/// версиях флаг `--hide-crash-restore-bubble` не помогает). Не получилось — не страшно.
fn mark_profile_clean(profile: &Path) {
    let file = profile.join("Default").join("Preferences");
    let Ok(text) = std::fs::read_to_string(&file) else { return };
    let Ok(mut prefs) = serde_json::from_str::<serde_json::Value>(&text) else { return };
    let Some(root) = prefs.as_object_mut() else { return };
    let profile_prefs = root.entry("profile").or_insert_with(|| serde_json::json!({}));
    if let Some(p) = profile_prefs.as_object_mut() {
        p.insert("exit_type".into(), serde_json::json!("Normal"));
        p.insert("exited_cleanly".into(), serde_json::json!(true));
    }
    if let Ok(out) = serde_json::to_string(&prefs) {
        let _ = std::fs::write(&file, out);
    }
}

/// Стартовый процесс браузера вышел, а порта ещё нет. Код 0 — не сбой: Edge (и Chrome с
/// некоторыми политиками) перезапускает себя новым процессом и выходит, а порт отладки
/// поднимает уже тот. До 11.11.2026 это считалось ошибкой, и Edge у бота не запускался.
fn launch_exit(code: Option<i32>) -> Result<(), String> {
    match code {
        Some(0) => Ok(()),
        code => Err(format!("браузер завершился при запуске с кодом {code:?}: профиль занят или запуск запрещён")),
    }
}

// ---------------------------------------------------------------------------
// События браузера
// ---------------------------------------------------------------------------

async fn prepare_page(cdp: Arc<Cdp>, tabs: Arc<Mutex<Tabs>>, target: String, session: String) {
    let sid = Some(session.as_str());
    // Сначала проверка адресов: она живёт в процессе браузера и отвечает, пока вкладка ждёт отладчик.
    // Первый переход вкладки уже должен идти через неё.
    let patterns = json!({ "patterns": [{ "resourceType": "Document" }, { "resourceType": "XHR" }, { "resourceType": "Fetch" }] });
    let _ = cdp.call(sid, "Fetch.enable", patterns).await;
    // Остальное вкладка, открытая со страницы, не ответит, пока её не отпустить. Поэтому всё разом.
    let steps = ["Page.enable", "Runtime.enable", "Log.enable", "Network.enable", "DOM.enable", "Runtime.runIfWaitingForDebugger"];
    join_all(steps.map(|method| cdp.call(sid, method, json!({})))).await;
    if let Some(page) = tabs.lock().unwrap().pages.get_mut(&target) {
        page.ready = true;
    }
}

/// Запрос из вкладки: на этой машине и в локальной сети страницам делать нечего.
async fn check_request(cdp: Arc<Cdp>, session: String, params: Value) {
    let id = params["requestId"].clone();
    let url = params["request"]["url"].as_str().unwrap_or_default();
    let (method, body) = if is_blocked_url(url).await {
        ("Fetch.failRequest", json!({ "requestId": id, "errorReason": "BlockedByClient" }))
    } else {
        ("Fetch.continueRequest", json!({ "requestId": id }))
    };
    let _ = cdp.call(Some(&session), method, body).await;
}

/// Сеть и схемы, куда бот не ходит. Если адрес не определился, решит сам браузер: он покажет ошибку.
/// В облаке открываются только http, https и пустая страница: `file://` и прочее закрыты, и по ссылке, и по редиректу.
fn allowed_scheme_in_cloud(url: &str, scheme: &str) -> bool {
    matches!(scheme, "http" | "https") || url.trim().eq_ignore_ascii_case("about:blank")
}

pub async fn is_blocked_url(url: &str) -> bool {
    let Ok(parsed) = reqwest::Url::parse(url) else { return crate::cloud::active() };
    if crate::cloud::active() && !allowed_scheme_in_cloud(url, parsed.scheme()) {
        return true;
    }
    if !matches!(parsed.scheme(), "http" | "https") {
        return false;
    }
    let Some(host) = parsed.host_str() else { return true };
    let port = parsed.port_or_known_default().unwrap_or(80);
    let bare = host.trim_start_matches('[').trim_end_matches(']');
    if let Ok(ip) = bare.parse::<IpAddr>() {
        return !address_allowed(ip);
    }
    let blocked = match tokio::net::lookup_host((bare, port)).await {
        Ok(addrs) => addrs.into_iter().any(|addr| !address_allowed(addr.ip())),
        Err(_) => false,
    };
    blocked
}

async fn pump(cdp: Arc<Cdp>, tabs: Arc<Mutex<Tabs>>, mut events: mpsc::UnboundedReceiver<Value>) {
    while let Some(event) = events.recv().await {
        let method = event["method"].as_str().unwrap_or_default();
        let params = &event["params"];
        match method {
            "Target.attachedToTarget" => {
                let info = &params["targetInfo"];
                let session = params["sessionId"].as_str().unwrap_or_default().to_string();
                let target = info["targetId"].as_str().unwrap_or_default().to_string();
                if info["type"] == "page" {
                    {
                        let mut state = tabs.lock().unwrap();
                        state.by_session.insert(session.clone(), target.clone());
                        state.order.push(target.clone());
                        if state.current.is_some() {
                            state.new_tab = true;
                        }
                        state.current = Some(target.clone());
                        if let Some(opener) = info["openerId"].as_str() {
                            state.inherit_taint(opener, &target);
                        }
                        state.pages.insert(
                            target.clone(),
                            Page {
                                session: session.clone(),
                                url: info["url"].as_str().unwrap_or_default().to_string(),
                                title: info["title"].as_str().unwrap_or_default().to_string(),
                                ready: false,
                                refs: HashMap::new(),
                                console: VecDeque::new(),
                                requests: VecDeque::new(),
                                scripts: Vec::new(),
                                paused: None,
                                dialog: None,
                                mouse: (0.0, 0.0),
                                debugger_on: false,
                                last_net: now(),
                                evaled: false,
                                secret_nodes: HashSet::new(),
                                child_nodes: HashSet::new(),
                            },
                        );
                    }
                    tokio::spawn(prepare_page(cdp.clone(), tabs.clone(), target, session));
                } else {
                    let cdp = cdp.clone();
                    tokio::spawn(async move {
                        let _ = cdp.call(Some(&session), "Runtime.runIfWaitingForDebugger", json!({})).await;
                        let _ = cdp.call(None, "Target.detachFromTarget", json!({ "sessionId": session })).await;
                    });
                }
            }
            "Target.targetInfoChanged" => {
                let info = &params["targetInfo"];
                let mut state = tabs.lock().unwrap();
                if let Some(page) = info["targetId"].as_str().and_then(|id| state.pages.get_mut(id)) {
                    set_page_url(page, info["url"].as_str().unwrap_or_default());
                    page.title = info["title"].as_str().unwrap_or_default().to_string();
                }
            }
            "Target.targetDestroyed" | "Target.detachedFromTarget" => {
                let mut state = tabs.lock().unwrap();
                let target = params["targetId"].as_str().map(str::to_string).or_else(|| {
                    params["sessionId"].as_str().and_then(|sid| state.by_session.get(sid).cloned())
                });
                if let Some(target) = target {
                    if let Some(page) = state.pages.remove(&target) {
                        state.by_session.remove(&page.session);
                        state.order.retain(|id| id != &target);
                        if state.current.as_deref() == Some(target.as_str()) {
                            state.current = state.order.last().cloned();
                        }
                    }
                }
            }
            "Browser.downloadWillBegin" => {
                if let (Some(guid), Some(name)) = (params["guid"].as_str(), params["suggestedFilename"].as_str()) {
                    tabs.lock().unwrap().names.insert(guid.to_string(), name.to_string());
                }
            }
            "Browser.downloadProgress" => {
                if params["state"] == "completed" {
                    let mut state = tabs.lock().unwrap();
                    if let Some(name) = params["guid"].as_str().and_then(|guid| state.names.remove(guid)) {
                        state.downloads.push(name);
                    }
                }
            }
            "Page.screencastFrame" => {
                let state = tabs.lock().unwrap();
                if let (Some(tx), Some(session)) = (state.frame_tx.as_ref(), event["sessionId"].as_str()) {
                    let _ = tx.send((session.to_string(), params.clone()));
                }
            }
            "Fetch.requestPaused" => {
                if let Some(session) = event["sessionId"].as_str() {
                    tokio::spawn(check_request(cdp.clone(), session.to_string(), params.clone()));
                }
            }
            _ => {
                let Some(session) = event["sessionId"].as_str() else { continue };
                let mut state = tabs.lock().unwrap();
                let Some(target) = state.by_session.get(session).cloned() else { continue };
                let Some(page) = state.pages.get_mut(&target) else { continue };
                page_event(page, method, params);
            }
        }
    }
}

/// Смена адреса вкладки. Отметку «выполнялся код» она не трогает: `history.pushState` и `#` меняют адрес
/// в том же документе, где код остался. Сбрасывает только новая загрузка главного кадра.
fn set_page_url(page: &mut Page, url: &str) {
    page.url = url.to_string();
}

fn page_event(page: &mut Page, method: &str, params: &Value) {
    match method {
        "Runtime.consoleAPICalled" => {
            let kind = params["type"].as_str().unwrap_or("log");
            push_limited(&mut page.console, format!("{kind}: {}", console_args(&params["args"])));
        }
        "Runtime.exceptionThrown" => {
            let details = &params["exceptionDetails"];
            let text = details["exception"]["description"].as_str().or(details["text"].as_str()).unwrap_or("исключение");
            push_limited(&mut page.console, format!("исключение: {text}"));
        }
        "Log.entryAdded" => {
            let entry = &params["entry"];
            push_limited(
                &mut page.console,
                format!("{}: {} {}", entry["level"].as_str().unwrap_or("log"), entry["text"].as_str().unwrap_or_default(), entry["url"].as_str().unwrap_or_default()),
            );
        }
        "Network.requestWillBeSent" => {
            page.last_net = now();
            let request = &params["request"];
            push_limited(
                &mut page.requests,
                Request {
                    id: params["requestId"].as_str().unwrap_or_default().to_string(),
                    method: request["method"].as_str().unwrap_or_default().to_string(),
                    url: request["url"].as_str().unwrap_or_default().to_string(),
                    kind: params["type"].as_str().unwrap_or_default().to_string(),
                    request_headers: request["headers"].clone(),
                    ..Request::default()
                },
            );
        }
        "Network.responseReceived" => {
            page.last_net = now();
            let response = &params["response"];
            if let Some(item) = page.requests.iter_mut().rev().find(|r| r.id == params["requestId"].as_str().unwrap_or_default()) {
                item.status = response["status"].as_i64().unwrap_or(0);
                item.mime = response["mimeType"].as_str().unwrap_or_default().to_string();
                item.response_headers = response["headers"].clone();
            }
        }
        "Network.loadingFinished" | "Network.loadingFailed" => page.last_net = now(),
        "Debugger.scriptParsed" => {
            if page.scripts.len() < 3000 {
                page.scripts.push(Script {
                    id: params["scriptId"].as_str().unwrap_or_default().to_string(),
                    url: params["url"].as_str().unwrap_or_default().to_string(),
                    length: params["length"].as_i64().unwrap_or(0),
                });
            }
        }
        "Debugger.paused" => page.paused = Some(params.clone()),
        "Debugger.resumed" => page.paused = None,
        // Перезагрузка и переход: новая загрузка страницы, код прежней к ней уже не относится.
        "Page.frameNavigated" if params["frame"]["parentId"].is_null() => {
            page.evaled = false;
            page.secret_nodes.clear();
        }
        "Page.javascriptDialogOpening" => page.dialog = Some(params.clone()),
        "Page.javascriptDialogClosed" => page.dialog = None,
        _ => {}
    }
}

// ---------------------------------------------------------------------------
// Все браузеры программы
// ---------------------------------------------------------------------------

#[derive(Default)]
pub struct Browsers {
    sessions: tokio::sync::Mutex<HashMap<String, Arc<Session>>>,
    images: Mutex<HashMap<String, Vec<u8>>>,
}

impl Browsers {
    /// Браузер бота: запускается при первой нужде и заново, если человек его закрыл.
    pub async fn session(&self, store: &Store, bot_id: &str) -> Result<Arc<Session>, String> {
        let mut sessions = self.sessions.lock().await;
        if let Some(session) = sessions.get(bot_id) {
            if session.alive() {
                session.touch();
                return Ok(session.clone());
            }
        }
        // Мёртвую сессию закрываем до новой: иначе её окно и профиль останутся занятыми.
        if let Some(dead) = sessions.remove(bot_id) {
            dead.close().await;
        }
        let session = Session::launch(store, bot_id).await?;
        sessions.insert(bot_id.to_string(), session.clone());
        Ok(session)
    }

    pub async fn close(&self, bot_id: &str) {
        let session = self.sessions.lock().await.remove(bot_id);
        if let Some(session) = session {
            session.close().await;
        }
    }

    pub async fn close_all(&self) {
        let all: Vec<Arc<Session>> = self.sessions.lock().await.drain().map(|(_, s)| s).collect();
        for session in all {
            session.close().await;
        }
    }

    /// Браузер без дела освобождает память. Профиль остаётся, и вход не теряется.
    pub async fn close_idle(&self, max_idle_ms: i64, is_busy: impl Fn(&str) -> bool) {
        let idle: Vec<String> = self
            .sessions
            .lock()
            .await
            .iter()
            .filter(|(bot, session)| {
                !session.alive()
                    || (!is_busy(bot)
                        && !session.shown.load(Ordering::Relaxed)
                        && now() - session.last_used.load(Ordering::Relaxed) > max_idle_ms)
            })
            .map(|(bot, _)| bot.clone())
            .collect();
        for bot in idle {
            self.close(&bot).await;
        }
    }

    /// Ход закончился: страницу на точке останова не оставляем висеть.
    pub async fn resume_paused(&self, bot_id: &str) {
        let session = self.sessions.lock().await.get(bot_id).cloned();
        let Some(session) = session else { return };
        let paused = session.with_page(|page| page.paused.is_some()).unwrap_or(false);
        if paused {
            let _ = session.call("Debugger.resume", json!({})).await;
        }
    }

    pub fn put_image(&self, bot_id: &str, image: Vec<u8>) {
        self.images.lock().unwrap().insert(bot_id.to_string(), image);
    }

    pub fn take_image(&self, bot_id: &str) -> Option<Vec<u8>> {
        self.images.lock().unwrap().remove(bot_id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bare_page(url: &str) -> Page {
        Page {
            session: "s1".into(),
            url: url.into(),
            title: String::new(),
            ready: true,
            refs: HashMap::new(),
            console: VecDeque::new(),
            requests: VecDeque::new(),
            scripts: Vec::new(),
            paused: None,
            dialog: None,
            mouse: (0.0, 0.0),
            debugger_on: false,
            last_net: 0,
            evaled: false,
            secret_nodes: HashSet::new(),
            child_nodes: HashSet::new(),
        }
    }

    #[test]
    fn eval_flag_is_reset_by_a_new_document_only() {
        let url = "https://login.example.com/signin";
        let mut page = bare_page(url);
        page.evaled = true;
        // Навигация во фрейме флаг страницы не сбрасывает, навигация главного кадра сбрасывает.
        page_event(&mut page, "Page.frameNavigated", &json!({ "frame": { "id": "f2", "parentId": "f1" } }));
        assert!(page.evaled);
        page_event(&mut page, "Page.frameNavigated", &json!({ "frame": { "id": "f1" } }));
        assert!(!page.evaled);
        // Смена адреса внутри документа (pushState, #) флаг не сбрасывает, даже на другой путь.
        page.evaled = true;
        set_page_url(&mut page, "https://login.example.com/signin?x");
        set_page_url(&mut page, "https://login.example.com/other#frag");
        assert!(page.evaled, "pushState не должен очищать отметку");
    }

    fn tabs_with(pages: &[(&str, &str)]) -> Tabs {
        let mut tabs = Tabs::default();
        for (id, url) in pages {
            tabs.pages.insert(id.to_string(), bare_page(url));
            tabs.order.push(id.to_string());
        }
        tabs
    }

    #[test]
    fn tainted_origins_stay_for_the_whole_browser_session() {
        let a = "https://login.example.com/signin";
        let mut tabs = tabs_with(&[("t1", a)]);
        tabs.mark_script_ran("t1", &[a]);
        assert!(tabs.origin_tainted("https://login.example.com"));
        assert!(!tabs.origin_tainted("https://other.example.com"));
        // window.open: вторая вкладка наследует отметку; закрытие всех вкладок её не снимает
        // (код мог зарегистрировать service worker, который переживёт вкладки).
        tabs.pages.insert("t2".into(), bare_page(a));
        tabs.inherit_taint("t1", "t2");
        tabs.pages.remove("t1");
        tabs.pages.remove("t2");
        assert!(tabs.origin_tainted("https://login.example.com"));
    }

    #[test]
    fn a_tab_that_ran_code_does_not_taint_other_origins() {
        let mut tabs = tabs_with(&[("t1", "https://a.example.com/"), ("t2", "https://b.example.com/")]);
        tabs.mark_script_ran("t1", &["https://a.example.com/"]);
        assert!(tabs.origin_tainted("https://a.example.com"));
        assert!(!tabs.origin_tainted("https://b.example.com"));
        tabs.pages.insert("t3".into(), bare_page("about:blank"));
        tabs.mark_script_ran("t3", &["about:blank"]);
        assert!(tabs.pages["t3"].evaled);
    }

    #[test]
    fn devtools_active_port_parses_port_and_path() {
        let endpoint = parse_devtools_active_port("9222\n/devtools/browser/6f7e-1\n").unwrap();
        assert_eq!(endpoint.port, 9222);
        assert_eq!(endpoint.path.as_deref(), Some("/devtools/browser/6f7e-1"));
        assert_eq!(endpoint.ws_url(None).as_deref(), Some("ws://127.0.0.1:9222/devtools/browser/6f7e-1"));
    }

    #[test]
    fn devtools_active_port_allows_missing_path() {
        let endpoint = parse_devtools_active_port("51000\n").unwrap();
        assert_eq!(endpoint.port, 51000);
        assert_eq!(endpoint.path, None);
        assert_eq!(endpoint.ws_url(None), None);
        assert_eq!(endpoint.ws_url(Some("ws://127.0.0.1:51000/devtools/browser/x")).as_deref(), Some("ws://127.0.0.1:51000/devtools/browser/x"));
    }

    #[test]
    fn devtools_active_port_rejects_garbage() {
        assert!(parse_devtools_active_port("").is_err());
        assert!(parse_devtools_active_port("not-a-port\n/devtools/browser/x").is_err());
        assert!(parse_devtools_active_port("0\n/devtools/browser/x").is_err());
    }

    #[test]
    fn json_version_parses_and_requires_browser() {
        let text = r#"{"Browser":"Edge/156.0.4314.58","Protocol-Version":"1.3","webSocketDebuggerUrl":"ws://127.0.0.1:9222/devtools/browser/abc"}"#;
        let version = parse_json_version(text).unwrap();
        assert_eq!(version.browser, "Edge/156.0.4314.58");
        assert_eq!(version.protocol.as_deref(), Some("1.3"));
        assert_eq!(version.ws.as_deref(), Some("ws://127.0.0.1:9222/devtools/browser/abc"));
        assert!(parse_json_version("{}").is_err());
        assert!(parse_json_version("не json").is_err());
        assert_eq!(version_url(9222), "http://127.0.0.1:9222/json/version");
    }

    #[test]
    fn mode_parsing_and_plan_selection() {
        assert_eq!(parse_mode("").unwrap(), Mode::Auto);
        assert_eq!(parse_mode("AUTO").unwrap(), Mode::Auto);
        assert_eq!(parse_mode("embedded").unwrap(), Mode::Embedded);
        assert_eq!(parse_mode("webview2").unwrap(), Mode::Embedded);
        assert_eq!(parse_mode("external").unwrap(), Mode::External);
        assert!(parse_mode("opera").is_err());

        // auto никогда не уходит в непроверенный встроенный путь.
        assert_eq!(choose_plan(Mode::Auto, true).unwrap(), Plan::External);
        assert_eq!(choose_plan(Mode::External, true).unwrap(), Plan::External);
        assert_eq!(choose_plan(Mode::Embedded, true).unwrap(), Plan::Embedded);
        assert!(choose_plan(Mode::Embedded, false).is_err());
    }

    #[test]
    fn embedded_args_ask_for_cdp_port() {
        let args = embedded_browser_args();
        assert!(args.contains("--remote-debugging-port=0"));
        assert!(args.contains("--disable-features=msWebOOUI,msPdfOOUI,msSmartScreenProtection"));
        assert!(args.contains("--disable-background-timer-throttling"));
    }

    #[cfg(windows)]
    #[test]
    fn external_backends_are_named_by_exe() {
        // Браузер бота запускается как у человека: без признака автоматизации и без headless.
        let args = external_args(Path::new(r"C:\p"), false);
        assert!(args.iter().any(|a| a == "--disable-blink-features=AutomationControlled"), "{args:?}");
        assert!(!args.iter().any(|a| a.contains("enable-automation") || a.contains("headless") || a.contains("proxy")), "{args:?}");
        assert!(args.iter().any(|a| a == r"--user-data-dir=C:\p"), "постоянный профиль бота");
        assert!(args.iter().any(|a| a == "--hide-crash-restore-bubble"), "без «Восстановить страницы?»");
        // Edge передаёт запуск новому процессу и выходит с кодом 0: это не сбой.
        assert!(launch_exit(Some(0)).is_ok());
        assert!(launch_exit(Some(-2147483645)).unwrap_err().contains("-2147483645"));
        assert!(launch_exit(None).is_err());
        assert_eq!(external_backend(Path::new(r"C:\Program Files\Google\Chrome\Application\chrome.exe")), Backend::Chrome);
        assert_eq!(external_backend(Path::new(r"C:\Program Files (x86)\Microsoft\Edge\Application\msedge.exe")), Backend::Edge);
        assert_eq!(external_backend(Path::new(r"C:\Program Files\BraveSoftware\Brave-Browser\Application\brave.exe")), Backend::Chrome);
        assert_eq!(Backend::WebView2.as_str(), "webview2");
        assert_eq!(Backend::Chrome.as_str(), "chrome");
        assert_eq!(Backend::Edge.as_str(), "edge");
        assert_eq!(Backend::Obscura.as_str(), "obscura");
        assert_eq!(Backend::Moli.as_str(), "moli");
    }

    #[test]
    fn configured_browser_path_is_checked() {
        let missing = std::env::temp_dir().join(format!("crew-no-such-browser-{}.exe", std::process::id()));
        let error = find_browser(missing.to_string_lossy().as_ref()).unwrap_err();
        assert!(error.contains("browser_path"), "{error}");
        assert!(error.contains("не найден"), "{error}");
    }

    #[test]
    fn the_cloud_opens_only_http_https_and_the_blank_page() {
        for ok in [("http://example.org/", "http"), ("https://example.org/", "https"), ("about:blank", "about")] {
            assert!(super::allowed_scheme_in_cloud(ok.0, ok.1), "{}", ok.0);
        }
        for bad in [("file:///etc/passwd", "file"), ("about:srcdoc", "about"), ("data:text/html,x", "data"), ("javascript:alert(1)", "javascript"), ("chrome://version", "chrome"), ("ftp://x/", "ftp")] {
            assert!(!super::allowed_scheme_in_cloud(bad.0, bad.1), "{}", bad.0);
        }
    }
}
