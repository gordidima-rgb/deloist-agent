//! Вход через OAuth для подключений: системный браузер, обратный адрес на этой машине
//! (`http://127.0.0.1:<порт>/cb`), PKCE S256 и случайный `state`.
//!
//! Провайдеры с нашим client id (Яндекс ID, VK ID, Google, Notion) описаны в [`PROVIDERS`];
//! client id владелец впишет, когда зарегистрирует приложения. Для удалённых MCP-серверов
//! с OAuth есть стандартный путь MCP: поиск сервера авторизации по `.well-known` и
//! регистрация клиента на лету (Dynamic Client Registration), без нашего client id.
//! Токены лежат в хранилище секретов (`oauth.*`), в state.json — только адреса и client id.

use std::collections::{BTreeMap, HashMap};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

use crate::App;

pub const ACCESS: &str = "oauth.access_token";
pub const REFRESH: &str = "oauth.refresh_token";
pub const EXPIRES: &str = "oauth.expires_at";
/// Секрет клиента, который ввёл человек (если сервер не умеет регистрацию).
pub const CLIENT_SECRET: &str = "oauth.client_secret";
/// Сколько живёт незавершённый вход.
const FLOW_TTL: Duration = Duration::from_secs(600);
/// Токен, который истечёт раньше этого, обновляем заранее.
const REFRESH_AHEAD_MS: i64 = 60_000;

/// Настройка OAuth подключения. Пустые поля берутся у встроенного провайдера.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", default)]
pub struct Config {
    /// yandex, vk, google, notion или пусто (всё задано в манифесте / найдено по MCP).
    pub provider: String,
    pub authorize_url: String,
    pub token_url: String,
    /// Только для MCP OAuth: адрес регистрации клиента.
    #[serde(skip_serializing_if = "String::is_empty")]
    pub registration_url: String,
    pub scopes: Vec<String>,
    pub client_id: String,
    /// Дополнительные параметры адреса входа (`access_type=offline` и т.п.).
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    pub extra: BTreeMap<String, String>,
    /// RFC 8707: адрес MCP-сервера, для которого просим токен.
    #[serde(skip_serializing_if = "String::is_empty")]
    pub resource: String,
    /// Порт обратного адреса, под который зарегистрирован клиент. 0 — любой свободный.
    pub redirect_port: u16,
    /// client id ввёл человек (у сервера нет регистрации клиента): не регистрируем и не сбрасываем.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub manual_client: bool,
    /// RFC 9207: `issuer` из метаданных сервера авторизации и обещание присылать `iss`.
    #[serde(skip_serializing_if = "String::is_empty")]
    pub issuer: String,
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub iss_required: bool,
}

/// Параметры адреса входа, которые ставит само приложение: `extra` их не подменяет.
pub const RESERVED_PARAMS: &[&str] = &["response_type", "client_id", "client_secret", "redirect_uri", "code_challenge", "code_challenge_method", "state", "resource", "scope", "code_verifier"];

/// Встроенный провайдер. client id — заглушки: их заполнит владелец после регистрации.
pub struct Provider {
    pub id: &'static str,
    pub title: &'static str,
    pub authorize_url: &'static str,
    pub token_url: &'static str,
    pub scopes: &'static [&'static str],
    pub client_id: &'static str,
    /// Секрет «настольного» клиента. Для Google он не секретен по их же документации;
    /// для остальных пусто — у них PKCE без секрета.
    pub client_secret: &'static str,
    pub extra: &'static [(&'static str, &'static str)],
    /// Порт, под который зарегистрирован обратный адрес. 0 — любой (RFC 8252 §7.3).
    pub port: u16,
}

pub const PROVIDERS: &[Provider] = &[
    Provider {
        id: "yandex",
        title: "Яндекс ID",
        authorize_url: "https://oauth.yandex.ru/authorize",
        token_url: "https://oauth.yandex.ru/token",
        scopes: &[],
        client_id: "", // TODO(owner): client id приложения на oauth.yandex.ru
        client_secret: "",
        extra: &[("force_confirm", "yes")],
        port: 0,
    },
    Provider {
        id: "vk",
        title: "VK ID",
        authorize_url: "https://id.vk.com/authorize",
        token_url: "https://id.vk.com/oauth2/auth",
        scopes: &[],
        client_id: "", // TODO(owner): id приложения в кабинете VK ID
        client_secret: "",
        extra: &[],
        port: 0,
    },
    Provider {
        id: "google",
        title: "Google",
        authorize_url: "https://accounts.google.com/o/oauth2/v2/auth",
        token_url: "https://oauth2.googleapis.com/token",
        scopes: &[],
        client_id: "", // TODO(owner): OAuth client «Desktop app» в Google Cloud
        client_secret: "",
        extra: &[("access_type", "offline"), ("prompt", "consent")],
        port: 0,
    },
    Provider {
        id: "notion",
        title: "Notion",
        authorize_url: "https://api.notion.com/v1/oauth/authorize",
        token_url: "https://api.notion.com/v1/oauth/token",
        scopes: &[],
        client_id: "", // TODO(owner): public integration в Notion; лучше MCP OAuth к mcp.notion.com
        client_secret: "",
        extra: &[("owner", "user")],
        port: 0,
    },
];

/// Настройка встроенного провайдера; от подключения берутся только scopes.
pub fn from_provider(p: &Provider, scopes: &[String]) -> Config {
    Config {
        provider: p.id.into(),
        authorize_url: p.authorize_url.into(),
        token_url: p.token_url.into(),
        registration_url: String::new(),
        scopes: if scopes.is_empty() { p.scopes.iter().map(|s| s.to_string()).collect() } else { scopes.to_vec() },
        client_id: p.client_id.into(),
        extra: p.extra.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect(),
        resource: String::new(),
        redirect_port: p.port,
        manual_client: false,
        issuer: String::new(),
        iss_required: false,
    }
}

pub fn provider(id: &str) -> Option<&'static Provider> {
    PROVIDERS.iter().find(|p| p.id == id)
}

/// Адрес провайдера: https, а http — только на этой машине (заглушки в тестах, свои прокси).
pub fn url_ok(url: &str) -> bool {
    let Ok(parsed) = reqwest::Url::parse(url.trim()) else { return false };
    let Some(host) = parsed.host_str() else { return false };
    match parsed.scheme() {
        "https" => true,
        "http" => matches!(host, "127.0.0.1" | "localhost" | "[::1]"),
        _ => false,
    }
}

// ---------------------------------------------------------------------------
// PKCE
// ---------------------------------------------------------------------------

pub fn base64url(bytes: &[u8]) -> String {
    const ABC: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";
    let mut out = String::with_capacity(bytes.len() * 4 / 3 + 2);
    for chunk in bytes.chunks(3) {
        let n = (chunk[0] as u32) << 16 | (*chunk.get(1).unwrap_or(&0) as u32) << 8 | *chunk.get(2).unwrap_or(&0) as u32;
        let take = chunk.len() + 1;
        for i in 0..take {
            out.push(ABC[(n >> (18 - 6 * i) & 63) as usize] as char);
        }
    }
    out
}

pub fn challenge(verifier: &str) -> String {
    base64url(&Sha256::digest(verifier.as_bytes()))
}

pub(crate) fn new_verifier() -> String {
    use rand::RngCore;
    let mut bytes = [0u8; 32];
    rand::thread_rng().fill_bytes(&mut bytes);
    base64url(&bytes)
}

// ---------------------------------------------------------------------------
// Входы в процессе
// ---------------------------------------------------------------------------

#[derive(Default)]
pub struct Flows {
    /// Один вход на подключение: новый «Подключить» гасит прежний.
    active: Mutex<HashMap<String, tokio::task::AbortHandle>>,
    /// Обновление токена по одному на подключение: два хода не тратят один refresh-токен дважды.
    refreshing: Mutex<HashMap<String, Arc<tokio::sync::Mutex<()>>>>,
    /// Что сервер ответил на 401: адрес `resource_metadata` и `scope` из `WWW-Authenticate`. В памяти, не на диске.
    pub challenges: Arc<Mutex<HashMap<String, Challenge>>>,
}

impl Flows {
    pub fn cancel(&self, id: &str) {
        if let Some(handle) = self.active.lock().unwrap().remove(id) {
            handle.abort();
        }
    }
}

pub(crate) fn http() -> reqwest::Client {
    reqwest::Client::builder()
        .no_proxy()
        .redirect(reqwest::redirect::Policy::none())
        .user_agent(concat!("deloist/", env!("CARGO_PKG_VERSION")))
        .timeout(Duration::from_secs(20))
        .build()
        .unwrap_or_default()
}

/// Собрать полную настройку: встроенный провайдер плюс то, что задано в манифесте.
pub fn resolve(cfg: &Config) -> Result<(Config, String), String> {
    let mut out = cfg.clone();
    let mut secret = String::new();
    if !cfg.provider.is_empty() {
        let p = provider(&cfg.provider).ok_or_else(|| format!("неизвестный OAuth-провайдер «{}»", cfg.provider))?;
        // Всё, кроме scopes, — только от встроенного провайдера: наш клиент не уходит на чужие адреса.
        out = from_provider(p, &cfg.scopes);
        secret = p.client_secret.to_string();
        if out.client_id.is_empty() {
            return Err(format!("client_id для «{}» ещё не задан владельцем приложения", p.title));
        }
    }
    if out.client_id.is_empty() {
        return Err("у подключения не задан client_id".into());
    }
    if !url_ok(&out.authorize_url) || !url_ok(&out.token_url) {
        return Err("адреса OAuth должны быть https://".into());
    }
    Ok((out, secret))
}

pub struct Started {
    pub url: String,
    /// Куда откроется вход: хост (и порт) сервера авторизации, чтобы человек видел это до браузера.
    pub host: String,
}

/// Client id (и секрет), которые ввёл человек: у сервера авторизации нет регистрации клиента.
#[derive(Default)]
pub struct Manual {
    pub client_id: String,
    pub client_secret: String,
    /// Порт обратного адреса, под который клиент зарегистрирован у сервера. 0 — любой свободный.
    pub redirect_port: u16,
}

pub enum StartError {
    /// Регистрации клиента нет: нужен client id от человека. Внутри — хост сервера авторизации.
    NeedsClientId(String),
    Failed(String),
}

impl From<String> for StartError {
    fn from(text: String) -> Self {
        Self::Failed(text)
    }
}

impl From<&str> for StartError {
    fn from(text: &str) -> Self {
        Self::Failed(text.to_string())
    }
}

/// Что сервер ответил на 401 в `WWW-Authenticate: Bearer …` (MCP, раздел Authorization Server Discovery).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Challenge {
    /// Заголовок говорит о схеме Bearer.
    pub bearer: bool,
    pub resource_metadata: String,
    pub scopes: Vec<String>,
}

/// Значение параметра `key` из заголовка `WWW-Authenticate`: `key="значение"` или `key=токен`.
fn header_param(header: &str, key: &str) -> Option<String> {
    let lower = header.to_ascii_lowercase();
    let mut from = 0;
    while let Some(found) = lower[from..].find(key) {
        let at = from + found;
        from = at + key.len();
        let before_ok = at == 0 || matches!(lower.as_bytes()[at - 1], b' ' | b',' | b'\t');
        let rest = header[from..].trim_start();
        if !before_ok || !rest.starts_with('=') {
            continue;
        }
        let value = rest[1..].trim_start();
        return Some(match value.strip_prefix('"') {
            Some(quoted) => quoted.split('"').next().unwrap_or_default().to_string(),
            None => value.split([',', ' ']).next().unwrap_or_default().to_string(),
        });
    }
    None
}

pub fn parse_challenge(header: &str) -> Challenge {
    Challenge {
        bearer: header.to_ascii_lowercase().contains("bearer"),
        resource_metadata: header_param(header, "resource_metadata").unwrap_or_default().chars().take(2048).collect(),
        scopes: header_param(header, "scope").unwrap_or_default().split_whitespace().take(32).map(str::to_string).collect(),
    }
}

fn host_of(url: &str) -> String {
    reqwest::Url::parse(url.trim()).ok().and_then(|u| u.host_str().map(str::to_ascii_lowercase)).unwrap_or_default()
}

/// Хост с портом, если порт не стандартный: то, что человек видит в карточке.
pub fn host_port_of(url: &str) -> String {
    match reqwest::Url::parse(url.trim()) {
        Ok(u) => match (u.host_str(), u.port()) {
            (Some(host), Some(port)) => format!("{}:{port}", host.to_ascii_lowercase()),
            (Some(host), None) => host.to_ascii_lowercase(),
            _ => String::new(),
        },
        Err(_) => String::new(),
    }
}

/// Канонический вид ресурса (RFC 8707 / 9728): схема, хост, порт и путь без хвостового «/»;
/// без логина, запроса и фрагмента. Пусто — адрес не разобрался.
pub fn canonical_resource(url: &str) -> String {
    match reqwest::Url::parse(url.trim()) {
        Ok(u) if u.host_str().is_some() => format!("{}{}", u.origin().ascii_serialization(), u.path().trim_end_matches('/')),
        _ => String::new(),
    }
}

/// Схема, хост и порт совпадают.
fn same_origin(a: &reqwest::Url, b: &reqwest::Url) -> bool {
    a.scheme() == b.scheme()
        && a.host_str().map(str::to_ascii_lowercase) == b.host_str().map(str::to_ascii_lowercase)
        && a.port_or_known_default() == b.port_or_known_default()
}

/// Клиент для запроса к адресу, который назвал удалённый сервер. Свой адрес MCP-сервера (`own`: схема,
/// хост и порт те же) человек выбрал сам. Любой другой — только внешние адреса, закреплённые перед
/// запросом: подмена DNS между проверкой и запросом не уведёт на внутреннюю сеть.
async fn pinned_client(url: &reqwest::Url, own: Option<&reqwest::Url>) -> Result<reqwest::Client, String> {
    let mut builder = reqwest::Client::builder()
        .no_proxy()
        .redirect(reqwest::redirect::Policy::none())
        .user_agent(concat!("deloist/", env!("CARGO_PKG_VERSION")))
        .timeout(Duration::from_secs(20));
    if !own.is_some_and(|o| same_origin(o, url)) {
        let host = url.host_str().unwrap_or_default().to_ascii_lowercase();
        let port = url.port_or_known_default().unwrap_or(443);
        let addrs = public_addrs(&host, port).await;
        let Some(first) = addrs.first() else {
            return Err(format!("адрес {host} не найден или во внутренней сети"));
        };
        builder = builder.resolve(&host, *first);
    }
    builder.build().map_err(|e| e.to_string())
}

/// Все адреса хоста, если ни один не во внутренней сети. Пусто — нельзя ходить.
async fn public_addrs(host: &str, port: u16) -> Vec<std::net::SocketAddr> {
    let bare = host.trim_start_matches('[').trim_end_matches(']');
    let addrs: Vec<std::net::SocketAddr> = match bare.parse::<std::net::IpAddr>() {
        Ok(ip) => vec![std::net::SocketAddr::new(ip, port)],
        Err(_) => tokio::net::lookup_host((bare, port)).await.map(|found| found.collect()).unwrap_or_default(),
    };
    if addrs.is_empty() || addrs.iter().any(|a| !crate::fetch::is_public_ip(a.ip())) {
        return Vec::new();
    }
    addrs
}

pub fn save_cfg_into(server: &mut crate::store::McpServer, cfg: Config) {
    match server.connector.as_mut() {
        Some(meta) => meta.auth.oauth = cfg,
        None => server.login.get_or_insert_with(Default::default).oauth = cfg,
    }
}

/// Свой сервер: секрет клиента уходит на сервер токенов, поэтому адреса авторизации, токенов и регистрации
/// (если есть) — на одном и том же хосте, который человек увидит в карточке.
fn same_host(authorize: &str, token: &str, registration: &str) -> Result<(), String> {
    let first = host_of(authorize);
    for (name, url) in [("токенов", token), ("регистрации", registration)] {
        let host = host_of(url);
        if !url.is_empty() && host != first {
            return Err(format!("адреса входа на разных хостах: {first} (авторизация) и {host} (адрес {name}). Секрет клиента не отправляем на хост, который вы не видели"));
        }
    }
    Ok(())
}

/// Начать вход: поднять обратный адрес и вернуть ссылку для браузера.
/// `mcp_discovery` — адрес MCP-сервера: найти сервер авторизации и зарегистрировать клиента на лету.
/// `egress` — хосты подключения из каталога; у своего сервера (`None`) сервер авторизации должен быть
/// на том же хосте или по https во внешней сети.
pub async fn start(app: &Arc<App>, id: &str, cfg: Config, mcp_discovery: Option<String>, egress: Option<&[String]>, manual: Option<Manual>) -> Result<Started, StartError> {
    app.connectors.oauth.cancel(id);
    let mut cfg = cfg;
    let mut typed_secret: Option<String> = None;
    if let Some(server_url) = mcp_discovery.as_deref() {
        let challenge = app.connectors.oauth.challenges.lock().unwrap().get(id).cloned().unwrap_or_default();
        if cfg.authorize_url.is_empty() || cfg.token_url.is_empty() {
            let hint = Some(challenge.resource_metadata.as_str()).filter(|u| !u.is_empty());
            let found = discover(server_url, hint, egress).await?;
            if egress.is_none() {
                same_host(&found.authorize_url, &found.token_url, &found.registration_url)?;
            }
            let server_host = host_of(server_url);
            for url in [&found.authorize_url, &found.token_url, &found.registration_url] {
                let host = host_of(url);
                if url.is_empty() || host == server_host {
                    continue;
                }
                match egress {
                    Some(list) if !crate::connectors::host_allowed(&host, list) => {
                        return Err(format!("сервер авторизации {host} не входит в разрешённые хосты подключения (egress)").into());
                    }
                    None if public_addrs(&host, reqwest::Url::parse(url).ok().and_then(|u| u.port_or_known_default()).unwrap_or(443)).await.is_empty() => {
                        return Err(format!("сервер авторизации {host} не найден или во внутренней сети").into());
                    }
                    _ => {}
                }
            }
            cfg.authorize_url = found.authorize_url;
            cfg.token_url = found.token_url;
            cfg.registration_url = found.registration_url;
            cfg.issuer = found.issuer;
            cfg.iss_required = found.iss_required;
            cfg.client_id.clear();
            cfg.manual_client = false;
            cfg.redirect_port = 0;
        }
        if cfg.scopes.is_empty() {
            cfg.scopes = challenge.scopes;
        }
        cfg.resource = canonical_resource(server_url);
        // Свой сервер: секрет клиента уходит на сервер токенов, поэтому все адреса входа — на одном хосте,
        // который человек видит в карточке. Подключения из каталога ограничены списком egress.
        if egress.is_none() {
            same_host(&cfg.authorize_url, &cfg.token_url, &cfg.registration_url)?;
        }
    }
    if let Some(manual) = &manual {
        let client_id = manual.client_id.trim();
        if client_id.is_empty() || client_id.len() > 512 || client_id.chars().any(|c| c.is_whitespace() || c.is_control()) {
            return Err("неверный client ID".into());
        }
        if manual.client_secret.len() > 2048 || manual.client_secret.contains(['\r', '\n']) {
            return Err("секрет клиента слишком длинный или с переводом строки".into());
        }
        cfg.client_id = client_id.to_string();
        cfg.manual_client = true;
        cfg.redirect_port = manual.redirect_port;
        // В хранилище секрет попадёт только после успешного входа (finish).
        typed_secret = Some(manual.client_secret.trim().to_string());
    }
    let needs_client = |cfg: &Config| mcp_discovery.is_some() && !cfg.manual_client && cfg.client_id.is_empty() && cfg.registration_url.is_empty();
    if needs_client(&cfg) {
        return Err(StartError::NeedsClientId(host_port_of(&cfg.authorize_url)));
    }
    // Порт, под который зарегистрирован клиент; занят — для MCP регистрируемся заново.
    let mut listener = None;
    if cfg.redirect_port != 0 {
        listener = TcpListener::bind(("127.0.0.1", cfg.redirect_port)).await.ok();
        if listener.is_none() && (mcp_discovery.is_none() || cfg.manual_client) {
            return Err(format!("порт {} для входа занят другой программой", cfg.redirect_port).into());
        }
    }
    let listener = match listener {
        Some(listener) => listener,
        None => {
            let listener = TcpListener::bind(("127.0.0.1", 0)).await.map_err(|e| format!("не открылся порт для входа: {e}"))?;
            if mcp_discovery.is_some() && !cfg.manual_client {
                cfg.client_id.clear();
            }
            listener
        }
    };
    let port = listener.local_addr().map_err(|e| e.to_string())?.port();
    let redirect_uri = format!("http://127.0.0.1:{port}/cb");
    if mcp_discovery.is_some() && cfg.client_id.is_empty() {
        if needs_client(&cfg) {
            return Err(StartError::NeedsClientId(host_port_of(&cfg.authorize_url)));
        }
        let own = reqwest::Url::parse(mcp_discovery.as_deref().unwrap_or_default()).ok();
        cfg.client_id = register(&cfg.registration_url, &redirect_uri, own.as_ref()).await?;
        cfg.redirect_port = port;
    }
    // Найденное и зарегистрированное запоминаем: в следующий раз не регистрируемся заново.
    if mcp_discovery.is_some() {
        let saved = cfg.clone();
        let _ = app.store.edit_mcp(id, |server| save_cfg_into(server, saved)).await;
    }
    let (full, mut client_secret) = resolve(&cfg)?;
    if let Some(typed) = &typed_secret {
        client_secret = typed.clone();
    } else if full.manual_client {
        client_secret = app.connectors.secrets.get(id, CLIENT_SECRET).unwrap_or_default();
    }
    let verifier = new_verifier();
    let state = crate::guard::random_token();
    let mut url = reqwest::Url::parse(&full.authorize_url).map_err(|e| e.to_string())?;
    {
        let mut query = url.query_pairs_mut();
        query
            .append_pair("response_type", "code")
            .append_pair("client_id", &full.client_id)
            .append_pair("redirect_uri", &redirect_uri)
            .append_pair("code_challenge", &challenge(&verifier))
            .append_pair("code_challenge_method", "S256")
            .append_pair("state", &state);
        if !full.scopes.is_empty() {
            query.append_pair("scope", &full.scopes.join(" "));
        }
        if !full.resource.is_empty() {
            query.append_pair("resource", &full.resource);
        }
        for (key, value) in full.extra.iter().filter(|(k, _)| !RESERVED_PARAMS.contains(&k.to_ascii_lowercase().as_str())) {
            query.append_pair(key, value);
        }
    }
    let host = host_port_of(&full.authorize_url);
    let flow = Flow { id: id.to_string(), state, verifier, redirect_uri, cfg: full, client_secret, typed_secret };
    let task = tokio::spawn(listen(app.clone(), listener, flow));
    app.connectors.oauth.active.lock().unwrap().insert(id.to_string(), task.abort_handle());
    Ok(Started { url: url.to_string(), host })
}

struct Flow {
    id: String,
    state: String,
    verifier: String,
    redirect_uri: String,
    cfg: Config,
    client_secret: String,
    /// Секрет, который ввёл человек: сохраняется, только если вход удался.
    typed_secret: Option<String>,
}

pub(crate) fn same(a: &str, b: &str) -> bool {
    a.len() == b.len() && a.bytes().zip(b.bytes()).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

/// Ждать возврата из браузера. Принимаем ровно один ответ с верным `state`; после обмена кода
/// порт закрывается, и повторный код приходить уже некуда.
async fn listen(app: Arc<App>, listener: TcpListener, flow: Flow) {
    let deadline = tokio::time::Instant::now() + FLOW_TTL;
    loop {
        let mut socket = match tokio::time::timeout_at(deadline, listener.accept()).await {
            Ok(Ok((socket, _))) => socket,
            Ok(Err(_)) => continue,
            Err(_) => {
                finish(&app, &flow, Err("вход не завершён за 10 минут".into())).await;
                break;
            }
        };
        let mut buf = vec![0u8; 8192];
        let mut len = 0;
        // Только строка запроса: тело и заголовки нам не нужны.
        while len < buf.len() {
            match tokio::time::timeout(Duration::from_secs(5), socket.read(&mut buf[len..])).await {
                Ok(Ok(0)) | Err(_) | Ok(Err(_)) => break,
                Ok(Ok(n)) => {
                    len += n;
                    if buf[..len].windows(4).any(|w| w == b"\r\n\r\n") {
                        break;
                    }
                }
            }
        }
        let head = String::from_utf8_lossy(&buf[..len]).to_string();
        let target = head.lines().next().and_then(|line| line.strip_prefix("GET ")).and_then(|rest| rest.split(' ').next()).unwrap_or("");
        let en = app.store.settings().ui_lang == "en";
        let say = |ru: &'static str, english: &'static str| if en { english } else { ru };
        let parsed = reqwest::Url::parse(&format!("http://127.0.0.1{target}")).ok();
        let Some(parsed) = parsed.filter(|u| u.path() == "/cb") else {
            respond(&mut socket, 404, say("Не тот адрес.", "Wrong address.")).await;
            continue;
        };
        let params: HashMap<String, String> = parsed.query_pairs().map(|(k, v)| (k.into_owned(), v.into_owned())).collect();
        let offered = params.get("state").map(String::as_str).unwrap_or("");
        if !same(offered, &flow.state) {
            // Чужой или старый ответ: отклоняем, но вход не обрываем.
            respond(&mut socket, 400, say("Ответ не от этого входа (state не совпал).", "This answer is not from this sign-in (state mismatch).")).await;
            continue;
        }
        // RFC 9207 (MCP: Authorization Response Validation): `iss` сверяем с запомненным издателем до всего прочего.
        let iss = params.get("iss");
        let iss_bad = match iss {
            Some(value) => !flow.cfg.issuer.is_empty() && value != &flow.cfg.issuer,
            None => flow.cfg.iss_required,
        };
        if iss_bad {
            respond(&mut socket, 400, say("Ответ не прошёл проверку издателя (iss).", "The answer failed the issuer (iss) check.")).await;
            finish(&app, &flow, Err("ответ сервера авторизации не прошёл проверку iss".into())).await;
            break;
        }
        if let Some(error) = params.get("error") {
            let text: String = error.chars().take(80).collect();
            respond(&mut socket, 400, say("Вход отменён. Вернитесь в приложение.", "Sign-in cancelled. Return to the app.")).await;
            finish(&app, &flow, Err(format!("провайдер отказал: {text}"))).await;
            break;
        }
        let Some(code) = params.get("code").filter(|c| !c.is_empty()) else {
            respond(&mut socket, 400, say("В ответе нет кода.", "The answer has no code.")).await;
            continue;
        };
        let result = exchange(&flow, code, params.get("device_id").map(String::as_str)).await;
        match &result {
            Ok(_) => respond(&mut socket, 200, say("Готово, подключение работает. Можно вернуться в приложение.", "Done, the connection works. You can return to the app.")).await,
            Err(_) => respond(&mut socket, 400, say("Не удалось получить токен. Вернитесь в приложение.", "Could not get a token. Return to the app.")).await,
        }
        let saved = match result {
            Ok(tokens) => save_tokens(&app, &flow.id, &tokens, None),
            Err(e) => Err(e),
        };
        finish(&app, &flow, saved).await;
        break;
    }
    app.connectors.oauth.active.lock().unwrap().remove(&flow.id);
}

pub(crate) async fn respond(socket: &mut tokio::net::TcpStream, status: u16, text: &str) {
    let reason = match status {
        200 => "OK",
        404 => "Not Found",
        _ => "Bad Request",
    };
    let body = format!("<!doctype html><meta charset=\"utf-8\"><meta name=\"referrer\" content=\"no-referrer\"><title>Deloist</title><p style=\"font:16px system-ui;margin:3em\">{text}</p>");
    let head = format!(
        "HTTP/1.1 {status} {reason}\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nCache-Control: no-store\r\nReferrer-Policy: no-referrer\r\nConnection: close\r\n\r\n",
        body.len()
    );
    let _ = socket.write_all(head.as_bytes()).await;
    let _ = socket.write_all(body.as_bytes()).await;
    let _ = socket.shutdown().await;
}

#[derive(Debug)]
pub struct Tokens {
    access: String,
    refresh: Option<String>,
    expires_in: Option<i64>,
}

fn tokens_from(value: &Value) -> Result<Tokens, String> {
    let access = value["access_token"].as_str().filter(|t| !t.is_empty()).ok_or("в ответе нет access_token")?;
    Ok(Tokens {
        access: access.to_string(),
        refresh: value["refresh_token"].as_str().filter(|t| !t.is_empty()).map(str::to_string),
        expires_in: value["expires_in"].as_i64().or_else(|| value["expires_in"].as_str().and_then(|s| s.parse().ok())),
    })
}

/// Отказ сервера токенов. `revoked` — сервер прямо сказал, что вход больше не годится
/// (400 invalid_grant или 401); сбой сети или 5xx — нет.
pub struct TokenError {
    pub text: String,
    pub revoked: bool,
}

impl From<String> for TokenError {
    fn from(text: String) -> Self {
        Self { text, revoked: false }
    }
}

/// Клиент для POST на адрес входа. Найденные по адресу сервера (есть `resource`) закрепляются, как в `get_json`;
/// фиксированные адреса провайдера из манифеста человек уже одобрил в карточке согласия.
async fn client_for(url: &str, resource: &str) -> Result<reqwest::Client, String> {
    if resource.is_empty() {
        return Ok(http());
    }
    let parsed = reqwest::Url::parse(url.trim()).map_err(|_| "неверный адрес сервера авторизации".to_string())?;
    pinned_client(&parsed, reqwest::Url::parse(resource).ok().as_ref()).await
}

async fn token_request(cfg: &Config, client_secret: &str, mut form: Vec<(&str, String)>) -> Result<Tokens, TokenError> {
    form.push(("client_id", cfg.client_id.clone()));
    if !client_secret.is_empty() {
        form.push(("client_secret", client_secret.to_string()));
    }
    if !cfg.resource.is_empty() {
        form.push(("resource", cfg.resource.clone()));
    }
    let client = client_for(&cfg.token_url, &cfg.resource).await.map_err(TokenError::from)?;
    let response = client
        .post(&cfg.token_url)
        .header("Accept", "application/json")
        .form(&form)
        .send()
        .await
        .map_err(|e| TokenError::from(format!("сервер токенов недоступен: {}", e.without_url())))?;
    let status = response.status();
    let text = response.text().await.unwrap_or_default();
    let value: Value = serde_json::from_str(&text).unwrap_or(Value::Null);
    if !status.is_success() {
        // Код ошибки OAuth короткий и без секретов; тело целиком не показываем.
        let code = value["error"].as_str().unwrap_or("").chars().take(60).collect::<String>();
        let revoked = status.as_u16() == 401 || (status.as_u16() == 400 && code == "invalid_grant");
        return Err(TokenError { text: format!("сервер токенов ответил HTTP {} {code}", status.as_u16()).trim().to_string(), revoked });
    }
    tokens_from(&value).map_err(TokenError::from)
}

async fn exchange(flow: &Flow, code: &str, device_id: Option<&str>) -> Result<Tokens, String> {
    exchange_inner(flow, code, device_id).await.map_err(|e| e.text)
}

async fn exchange_inner(flow: &Flow, code: &str, device_id: Option<&str>) -> Result<Tokens, TokenError> {
    let mut form = vec![
        ("grant_type", "authorization_code".to_string()),
        ("code", code.to_string()),
        ("redirect_uri", flow.redirect_uri.clone()),
        ("code_verifier", flow.verifier.clone()),
    ];
    if let Some(device) = device_id.filter(|d| !d.is_empty()) {
        // VK ID возвращает device_id вместе с кодом и ждёт его (и state) при обмене.
        form.push(("device_id", device.to_string()));
        form.push(("state", flow.state.clone()));
    }
    token_request(&flow.cfg, &flow.client_secret, form).await
}

fn save_tokens(app: &App, id: &str, tokens: &Tokens, keep_refresh: Option<&str>) -> Result<(), String> {
    let secrets = &app.connectors.secrets;
    secrets.set(id, ACCESS, &tokens.access)?;
    match (&tokens.refresh, keep_refresh) {
        (Some(refresh), _) => secrets.set(id, REFRESH, refresh)?,
        (None, Some(_)) => {} // сервер не прислал новый refresh: старый остаётся
        (None, None) => secrets.delete(id, REFRESH)?,
    }
    match tokens.expires_in.filter(|s| *s > 0) {
        Some(seconds) => secrets.set(id, EXPIRES, &(crate::store::now() + seconds * 1000).to_string())?,
        None => secrets.delete(id, EXPIRES)?,
    }
    Ok(())
}

/// Итог входа: пометка «нужно переподключить» снимается или ставится, страница узнаёт событием.
async fn finish(app: &Arc<App>, flow: &Flow, result: Result<(), String>) {
    let id = flow.id.as_str();
    let result = result.and_then(|()| match &flow.typed_secret {
        Some(secret) if secret.is_empty() => app.connectors.secrets.delete(id, CLIENT_SECRET),
        Some(secret) => app.connectors.secrets.set(id, CLIENT_SECRET, secret),
        None => Ok(()),
    });
    let ok = result.is_ok();
    let _ = app
        .store
        .edit_mcp(id, |server| {
            if ok {
                set_reauth(server, false);
                // Свой сервер: после входа токен идёт в заголовок Authorization.
                if server.connector.is_none() && server.login.is_some() && !server.headers.keys().any(|k| k.eq_ignore_ascii_case("authorization")) {
                    server.headers.insert("Authorization".into(), format!("Bearer {{{{secret:{ACCESS}}}}}"));
                }
            }
        })
        .await;
    crate::mcp::close(app, id);
    match result {
        Ok(()) => {
            app.connectors.clear_error(id);
            crate::connectors::announce(app, id);
        }
        Err(e) => {
            app.connectors.set_error(id, &e);
            crate::connectors::announce(app, id);
        }
    }
}

/// Перед запуском сервера: есть ли токен и не пора ли его обновить.
pub async fn ensure_fresh(app: &Arc<App>, id: &str, cfg: &Config) -> Result<(), String> {
    let secrets = &app.connectors.secrets;
    if secrets.get(id, ACCESS).is_none() {
        return Err("нужно войти: нажмите «Подключить»".into());
    }
    let fresh = || {
        let expires: i64 = secrets.get(id, EXPIRES).and_then(|v| v.parse().ok()).unwrap_or(0);
        expires == 0 || crate::store::now() + REFRESH_AHEAD_MS < expires
    };
    if fresh() {
        return Ok(());
    }
    let gate = app.connectors.oauth.refreshing.lock().unwrap().entry(id.to_string()).or_default().clone();
    let _one_at_a_time = gate.lock().await;
    // Пока ждали, токен мог обновить соседний ход.
    if fresh() {
        return Ok(());
    }
    let Some(refresh) = secrets.get(id, REFRESH) else {
        mark_reauth(app, id).await;
        return Err("вход истёк: нажмите «Переподключить»".into());
    };
    let (full, mut client_secret) = resolve(cfg)?;
    if full.manual_client {
        client_secret = secrets.get(id, CLIENT_SECRET).unwrap_or_default();
    }
    let form = vec![("grant_type", "refresh_token".to_string()), ("refresh_token", refresh.clone())];
    match token_request(&full, &client_secret, form).await {
        Ok(tokens) => save_tokens(app, id, &tokens, Some(&refresh)),
        Err(e) if e.revoked => {
            mark_reauth(app, id).await;
            Err(format!("вход истёк ({}): нажмите «Переподключить»", e.text))
        }
        // Сеть или сбой сервера: вход ещё может быть жив, попробуем в следующий раз.
        Err(e) => Err(format!("токен не обновился: {}", e.text)),
    }
}

pub fn set_reauth(server: &mut crate::store::McpServer, value: bool) {
    if let Some(meta) = server.connector.as_mut() {
        meta.needs_reauth = value;
    }
    if let Some(login) = server.login.as_mut() {
        login.needs_reauth = value;
    }
}

pub async fn mark_reauth(app: &Arc<App>, id: &str) {
    let _ = app
        .store
        .edit_mcp(id, |server| {
            set_reauth(server, true);
        })
        .await;
    crate::connectors::announce(app, id);
}

// ---------------------------------------------------------------------------
// MCP OAuth: поиск сервера авторизации и регистрация клиента
// ---------------------------------------------------------------------------

struct Found {
    authorize_url: String,
    token_url: String,
    registration_url: String,
    issuer: String,
    iss_required: bool,
}

/// RFC 8414 / 9728: адрес `.well-known` вставляется между хостом и путём.
fn well_known(base: &reqwest::Url, name: &str) -> Vec<String> {
    let origin = base.origin().ascii_serialization();
    let path = base.path().trim_end_matches('/');
    let mut out = Vec::new();
    if !path.is_empty() {
        out.push(format!("{origin}/.well-known/{name}{path}"));
    }
    out.push(format!("{origin}/.well-known/{name}"));
    out
}

/// Метаданные по адресу. Адрес самого MCP-сервера (схема, хост и порт) человек выбрал сам; любой другой
/// (адрес из `resource_metadata`, сервер авторизации, даже тот же хост на другом порту) назвал удалённый сервер,
/// поэтому: только https (http — на этой машине), только внешние адреса (проверка и закрепление адреса против подмены DNS), без редиректов,
/// не больше 256 КБ.
async fn get_json(url: &str, own: &reqwest::Url) -> Option<Value> {
    if !url_ok(url) {
        return None;
    }
    let parsed = reqwest::Url::parse(url.trim()).ok()?;
    let client = pinned_client(&parsed, Some(own)).await.ok()?;
    let mut response = client.get(parsed).header("Accept", "application/json").send().await.ok()?;
    if !response.status().is_success() {
        return None;
    }
    let mut body = Vec::new();
    while let Some(chunk) = response.chunk().await.ok()? {
        body.extend_from_slice(&chunk);
        if body.len() > 256 * 1024 {
            return None;
        }
    }
    serde_json::from_slice(&body).ok()
}

/// RFC 9728 → RFC 8414 / OpenID: сначала адрес из `WWW-Authenticate` (`hint`), если он есть, иначе
/// догадка по адресу сервера. Спецификация MCP: modelcontextprotocol.io/specification/draft/basic/authorization.
async fn discover(server_url: &str, hint: Option<&str>, egress: Option<&[String]>) -> Result<Found, String> {
    let server = reqwest::Url::parse(server_url).map_err(|_| "неверный адрес MCP-сервера".to_string())?;
    let mut auth_base = server.clone();
    auth_base.set_path("");
    auth_base.set_query(None);
    if let Some(hint) = hint {
        // Адрес из ответа 401 назвал сервер: у подключения из каталога он тоже должен быть среди его хостов.
        let foreign = reqwest::Url::parse(hint).ok().filter(|u| !same_origin(u, &server));
        if let (Some(list), Some(u)) = (egress, foreign) {
            let host = u.host_str().unwrap_or_default().to_ascii_lowercase();
            if !crate::connectors::host_allowed(&host, list) {
                return Err(format!("сервер указал адрес метаданных на {host}, которого нет среди разрешённых хостов подключения (egress)"));
            }
        }
    }
    let mut candidates: Vec<String> = hint.map(str::to_string).into_iter().collect();
    candidates.extend(well_known(&server, "oauth-protected-resource"));
    for url in candidates {
        if let Some(meta) = get_json(&url, &server).await {
            // RFC 9728: `resource` в метаданных должен быть этим сервером, иначе чужие метаданные подменят вход.
            if let Some(resource) = meta["resource"].as_str() {
                if canonical_resource(resource) != canonical_resource(server_url) {
                    return Err(format!("метаданные входа описывают другой ресурс ({resource}), а не этот сервер: вход отменён"));
                }
            }
            if let Some(first) = meta["authorization_servers"].as_array().and_then(|a| a.first()).and_then(Value::as_str) {
                auth_base = reqwest::Url::parse(first).map_err(|_| "неверный адрес сервера авторизации".to_string())?;
            }
            break;
        }
    }
    let mut metadata = None;
    for name in ["oauth-authorization-server", "openid-configuration"] {
        for url in well_known(&auth_base, name) {
            if let Some(found) = get_json(&url, &server).await {
                metadata = Some(found);
                break;
            }
        }
        if metadata.is_some() {
            break;
        }
    }
    let meta = metadata.ok_or("сервер не сообщил, как входить (нет .well-known/oauth-authorization-server)")?;
    let text = |key: &str| meta[key].as_str().unwrap_or_default().to_string();
    let found = Found {
        authorize_url: text("authorization_endpoint"),
        token_url: text("token_endpoint"),
        registration_url: text("registration_endpoint"),
        issuer: text("issuer"),
        iss_required: meta["authorization_response_iss_parameter_supported"].as_bool().unwrap_or(false),
    };
    // RFC 8414 §3.3: издатель в метаданных совпадает с адресом, по которому их нашли.
    let normal = |url: &str| reqwest::Url::parse(url.trim()).map(|u| u.to_string()).unwrap_or_else(|_| url.trim().to_string()).trim_end_matches('/').to_string();
    if found.issuer.is_empty() || normal(&found.issuer) != normal(auth_base.as_str()) {
        return Err(format!("издатель в метаданных ({}) не совпадает с адресом сервера авторизации ({}): вход отменён", found.issuer, auth_base.as_str().trim_end_matches('/')));
    }
    // Спецификация MCP: сервер обязан объявить PKCE; без объявления не входим.
    let pkce = meta["code_challenge_methods_supported"].as_array().is_some_and(|methods| methods.iter().any(|m| m == "S256"));
    if !pkce {
        return Err("сервер авторизации не объявил поддержку PKCE S256: вход отменён".into());
    }
    if !url_ok(&found.authorize_url) || !url_ok(&found.token_url) {
        return Err("сервер авторизации дал адреса не по https".into());
    }
    // Нет регистрации клиента — не отказ: client id спросим у человека.
    if !found.registration_url.is_empty() && !url_ok(&found.registration_url) {
        return Err("адрес регистрации клиента не по https".into());
    }
    Ok(found)
}

async fn register(registration_url: &str, redirect_uri: &str, own: Option<&reqwest::Url>) -> Result<String, String> {
    let body = json!({
        "client_name": "Deloist Боты",
        "redirect_uris": [redirect_uri],
        "grant_types": ["authorization_code", "refresh_token"],
        "response_types": ["code"],
        "token_endpoint_auth_method": "none",
    });
    let target = reqwest::Url::parse(registration_url.trim()).map_err(|_| "неверный адрес регистрации клиента".to_string())?;
    let client = pinned_client(&target, own).await.map_err(|e| format!("регистрация клиента: {e}"))?;
    let response = client.post(target).json(&body).send().await.map_err(|e| format!("регистрация клиента: {}", e.without_url()))?;
    let status = response.status();
    let value: Value = response.json().await.unwrap_or(Value::Null);
    if !status.is_success() {
        return Err(format!("регистрация клиента: HTTP {}", status.as_u16()));
    }
    value["client_id"].as_str().filter(|c| !c.is_empty()).map(str::to_string).ok_or_else(|| "регистрация клиента: нет client_id".into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pkce_matches_rfc7636_appendix_b() {
        assert_eq!(challenge("dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk"), "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM");
        assert_eq!(new_verifier().len(), 43);
        assert_eq!(base64url(b"f"), "Zg");
        assert_eq!(base64url(b"fo"), "Zm8");
        assert_eq!(base64url(b"foo"), "Zm9v");
    }

    #[test]
    fn provider_urls_are_https_and_placeholders_refuse() {
        for p in PROVIDERS {
            assert!(url_ok(p.authorize_url) && url_ok(p.token_url), "{}", p.id);
        }
        let error = resolve(&Config { provider: "yandex".into(), ..Default::default() }).unwrap_err();
        assert!(error.contains("client_id"), "{error}");
        // client id и адреса из настройки подключения у встроенного провайдера не берутся.
        let error = resolve(&Config { provider: "google".into(), client_id: "x".into(), token_url: "https://evil.example/token".into(), ..Default::default() }).unwrap_err();
        assert!(error.contains("client_id"), "{error}");
        let full = from_provider(provider("google").unwrap(), &["drive".to_string()]);
        assert_eq!(full.extra.get("access_type").map(String::as_str), Some("offline"));
        assert_eq!(full.token_url, "https://oauth2.googleapis.com/token");
        assert_eq!(full.scopes, vec!["drive"]);
        assert!(!url_ok("http://example.com/token"));
        assert!(url_ok("http://127.0.0.1:5000/token"));
    }

    #[test]
    fn a_401_challenge_gives_the_metadata_address_and_scopes() {
        let c = parse_challenge(r#"Bearer realm="mcp", resource_metadata="https://mcp.example.com/.well-known/oauth-protected-resource", scope="files:read files:write""#);
        assert_eq!(c.resource_metadata, "https://mcp.example.com/.well-known/oauth-protected-resource");
        assert_eq!(c.scopes, vec!["files:read", "files:write"]);
        assert!(c.bearer);
        let bare = parse_challenge("Bearer resource_metadata=https://a.example/m, error=invalid_token");
        assert_eq!(bare.resource_metadata, "https://a.example/m");
        // Имя параметра внутри другого значения не считается.
        assert_eq!(parse_challenge(r#"Bearer realm="my_resource_metadata=x""#).resource_metadata, "");
        assert!(!parse_challenge("Basic realm=x").bearer);
    }

    #[test]
    fn resource_is_canonical_and_origins_include_the_port() {
        assert_eq!(canonical_resource("https://Mcp.Example.com/"), "https://mcp.example.com");
        assert_eq!(canonical_resource("https://mcp.example.com:443/v1/?x=1#f"), "https://mcp.example.com/v1");
        assert_eq!(canonical_resource("http://127.0.0.1:8080/mcp/"), "http://127.0.0.1:8080/mcp");
        assert_eq!(host_port_of("http://127.0.0.1:8080/x"), "127.0.0.1:8080");
        assert_eq!(host_port_of("https://A.example.com:443/x"), "a.example.com");
        let own = reqwest::Url::parse("http://127.0.0.1:5000/mcp").unwrap();
        assert!(same_origin(&own, &reqwest::Url::parse("http://127.0.0.1:5000/meta").unwrap()));
        assert!(!same_origin(&own, &reqwest::Url::parse("http://127.0.0.1:5001/meta").unwrap()), "другой порт — не свой");
        assert!(!same_origin(&own, &reqwest::Url::parse("https://127.0.0.1:5000/meta").unwrap()), "другая схема — не свой");
    }

    #[tokio::test]
    async fn a_foreign_origin_on_the_own_host_is_not_exempt_from_the_public_check() {
        let own = reqwest::Url::parse("http://127.0.0.1:5000/mcp").unwrap();
        assert!(pinned_client(&reqwest::Url::parse("http://127.0.0.1:5000/m").unwrap(), Some(&own)).await.is_ok());
        let other_port = reqwest::Url::parse("http://127.0.0.1:5001/m").unwrap();
        assert!(pinned_client(&other_port, Some(&own)).await.is_err(), "тот же хост, другой порт: внутренняя сеть");
        assert!(get_json("http://127.0.0.1:5001/m", &own).await.is_none());
    }

    #[test]
    fn well_known_goes_between_host_and_path() {
        let url = reqwest::Url::parse("https://mcp.example.com/v1/mcp").unwrap();
        assert_eq!(
            well_known(&url, "oauth-protected-resource"),
            vec!["https://mcp.example.com/.well-known/oauth-protected-resource/v1/mcp", "https://mcp.example.com/.well-known/oauth-protected-resource"]
        );
    }
}
