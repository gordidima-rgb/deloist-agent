//! Сторож локального сервера. Его адрес доступен любому сайту, открытому в браузере на этой же
//! машине, поэтому loopback сам по себе ничего не защищает. Проверяем каждый запрос:
//! чей это адрес (DNS rebinding), откуда запрос (CSRF) и есть ли у него сессия окна приложения.

use std::sync::{Arc, Mutex};

use axum::body::Body;
use axum::extract::{ConnectInfo, Request, State};
use axum::http::{header, HeaderMap, HeaderName, HeaderValue, Method, StatusCode};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use axum::Json;
use rand::RngCore;
use serde_json::json;

const COOKIE_PREFIX: &str = "crew_session";
/// В облаке cookie с префиксом `__Host-`: только Secure, Path=/ и без Domain.
const CLOUD_COOKIE: &str = "__Host-crew_session";
const GATEWAY_PATH: &str = "/auth/gateway";
/// Выход из облака: удаляет предъявленную сессию и стирает cookie.
const LOGOUT_PATH: &str = "/auth/logout";
/// Префикс подписи шлюза: подпись берётся по `GATEWAY_DOMAIN || payload`, чтобы подпись того же
/// ключа под другим назначением не сошла за вход.
const GATEWAY_DOMAIN: &[u8] = b"deloist-gw-v1\n";
/// Жизнь облачной сессии и предел одновременных сессий.
const SESSION_TTL: i64 = 12 * 3600;
const MAX_SESSIONS: usize = 64;

const CSP: &str = "default-src 'self'; style-src 'self' 'unsafe-inline'; img-src 'self' data:; \
                   connect-src 'self'; frame-ancestors 'self'; base-uri 'none'; form-action 'none'";

fn now_secs() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0)
}

pub fn random_token() -> String {
    let mut bytes = [0u8; 32];
    rand::thread_rng().fill_bytes(&mut bytes);
    crate::store::hex(&bytes)
}

pub(crate) fn same(a: &str, b: &str) -> bool {
    // Без раннего выхода: время сравнения не подсказывает, сколько символов совпало.
    a.len() == b.len() && a.bytes().zip(b.bytes()).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

/// Облачный режим (`--server` + `CREW_CLOUD=1`): вход даёт шлюз, а не одноразовая ссылка.
/// Выставляется один раз при старте сервера; на ПК остаётся false.
static CLOUD: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

pub fn cloud_enabled() -> bool {
    CLOUD.load(std::sync::atomic::Ordering::Relaxed)
}

/// Публичный адрес облака (`CREW_PUBLIC_ORIGIN`, уже проверенный): по нему Telegram шлёт вебхуки.
static PUBLIC_ORIGIN: std::sync::OnceLock<String> = std::sync::OnceLock::new();

pub fn public_origin() -> Option<String> {
    PUBLIC_ORIGIN.get().cloned()
}

pub fn set_public_origin(origin: &str) {
    let _ = PUBLIC_ORIGIN.set(origin.trim_end_matches('/').to_string());
}

pub fn set_cloud(on: bool) {
    CLOUD.store(on, std::sync::atomic::Ordering::Relaxed);
}

/// Токен шлюза живёт не дольше этого срока, секунд.
const GATEWAY_MAX_TTL: i64 = 60;
/// Допуск часов шлюза и контейнера, секунд.
const IAT_SKEW: i64 = 5;
/// Токен, выпущенный не раньше чем за столько секунд до старта процесса, ещё годен.
const START_SLACK: i64 = 2;
const NONCE_MIN: usize = 16;
const NONCE_MAX: usize = 128;
/// Токены подписаны шлюзом и живут минуту, поэтому память под nonce ограничена сама; это запас.
const MAX_SEEN_NONCES: usize = 4096;
const GATEWAY_REFUSAL: &str = "вход не удался";

/// Настройки облачного режима. Закрытого ключа здесь нет и быть не может: только открытый ключ шлюза.
pub struct CloudConfig {
    key: ed25519_dalek::VerifyingKey,
    user_id: String,
    origin: String,
    host: String,
}

impl CloudConfig {
    pub fn origin(&self) -> &str {
        &self.origin
    }

    /// Облачный режим включён только строкой `CREW_CLOUD=1`. Без неё — `Ok(None)`.
    /// Включён, но хоть одна переменная пуста или неверна — ошибка: молча открываться нельзя.
    pub fn from_env() -> Result<Option<Self>, String> {
        Self::parse(|name| std::env::var(name).ok())
    }

    pub fn parse(get: impl Fn(&str) -> Option<String>) -> Result<Option<Self>, String> {
        if get("CREW_CLOUD").as_deref() != Some("1") {
            return Ok(None);
        }
        let need = |name: &str| get(name).filter(|v| !v.is_empty()).ok_or_else(|| format!("Облачный режим: не задана переменная {name}"));
        let key_hex = need("CREW_GATEWAY_PUBKEY")?;
        let bytes = unhex(&key_hex).filter(|b| b.len() == 32).ok_or("CREW_GATEWAY_PUBKEY должен быть открытым ключом Ed25519: 64 символа hex")?;
        let key = ed25519_dalek::VerifyingKey::from_bytes(bytes.as_slice().try_into().unwrap())
            .map_err(|_| "CREW_GATEWAY_PUBKEY не является ключом Ed25519".to_string())?;
        if key.is_weak() {
            return Err("CREW_GATEWAY_PUBKEY: слабый ключ (малого порядка) не принимается".into());
        }
        let user_id = need("CREW_USER_ID")?;
        if !(1..=64).contains(&user_id.len()) || !user_id.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-') {
            return Err("CREW_USER_ID: от 1 до 64 символов из A-Za-z0-9_-".into());
        }
        let origin = need("CREW_PUBLIC_ORIGIN")?;
        let host = origin_host(&origin).ok_or("CREW_PUBLIC_ORIGIN должен быть адресом вида https://хост без пути")?;
        Ok(Some(Self { key, user_id, origin: origin.to_ascii_lowercase(), host }))
    }
}

fn unhex(text: &str) -> Option<Vec<u8>> {
    if text.len() % 2 != 0 || !text.is_ascii() {
        return None;
    }
    (0..text.len()).step_by(2).map(|i| u8::from_str_radix(&text[i..i + 2], 16).ok()).collect()
}

/// `https://хост[:порт]` без пути, запроса, фрагмента и логина. Возвращает `хост[:порт]` в нижнем регистре.
fn origin_host(origin: &str) -> Option<String> {
    let rest = origin.strip_prefix("https://")?;
    let (name, port) = match rest.split_once(':') {
        Some((name, port)) => (name, Some(port)),
        None => (rest, None),
    };
    let name_ok = !name.is_empty()
        && !name.starts_with(['.', '-'])
        && !name.ends_with(['.', '-'])
        && name.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'.' || b == b'-');
    // Явный :443 браузер в Origin не пишет, такой адрес никогда бы не совпал.
    let port_ok = port.is_none_or(|p| !p.is_empty() && p.len() <= 5 && p.bytes().all(|b| b.is_ascii_digit()) && p.parse::<u16>().is_ok_and(|n| n > 0 && n != 443));
    (name_ok && port_ok).then(|| rest.to_ascii_lowercase())
}

struct Cloud {
    config: CloudConfig,
    /// Старт процесса, секунды: токены, выпущенные раньше, после перезапуска не годятся.
    started: i64,
    /// Уже использованные nonce и время (секунды), когда токен с ними всё равно истёк бы.
    seen: Mutex<std::collections::HashMap<String, i64>>,
}

#[derive(serde::Deserialize)]
struct GatewayClaims {
    uid: String,
    aud: String,
    iat: i64,
    exp: i64,
    nonce: String,
}

impl Cloud {
    /// Проверка токена шлюза. Любая причина отказа снаружи выглядит одинаково.
    fn accept(&self, token: &str, now: i64) -> bool {
        use base64::Engine;
        let b64 = base64::engine::general_purpose::URL_SAFE_NO_PAD;
        let Some((payload_b64, sig_b64)) = token.split_once('.') else { return false };
        let (Ok(payload), Ok(sig)) = (b64.decode(payload_b64), b64.decode(sig_b64)) else { return false };
        let Ok(sig) = ed25519_dalek::Signature::from_slice(&sig) else { return false };
        // Подпись — по точным байтам payload, до любого разбора.
        let mut signed = GATEWAY_DOMAIN.to_vec();
        signed.extend_from_slice(&payload);
        if self.config.key.verify_strict(&signed, &sig).is_err() {
            return false;
        }
        let Ok(claims) = serde_json::from_slice::<GatewayClaims>(&payload) else { return false };
        if !same(&claims.uid, &self.config.user_id) || !same(&claims.aud, &self.config.origin) {
            return false;
        }
        if claims.iat > now + IAT_SKEW || claims.iat < self.started - START_SLACK {
            return false;
        }
        if claims.exp < now || claims.exp > now + GATEWAY_MAX_TTL {
            return false;
        }
        if !(NONCE_MIN..=NONCE_MAX).contains(&claims.nonce.len()) || !claims.nonce.bytes().all(|b| b.is_ascii_graphic()) {
            return false;
        }
        // Проверка и запись одним захватом: два одновременных запроса с одним токеном не пройдут оба.
        let mut seen = self.seen.lock().unwrap();
        seen.retain(|_, until| *until >= now);
        if seen.contains_key(&claims.nonce) || seen.len() >= MAX_SEEN_NONCES {
            return false;
        }
        seen.insert(claims.nonce, claims.exp);
        true
    }
}

pub struct Guard {
    hosts: Vec<String>,
    origins: Vec<String>,
    cloud: Option<Cloud>,
    /// Одноразовый: ссылка со входом срабатывает один раз и дальше ничего не значит.
    /// Действующие одноразовые ссылки: окно и питомцы запускаются независимо и не должны
    /// отбирать вход друг у друга.
    boot: Mutex<Vec<String>>,
    session: String,
    /// Облако: сессии, выданные шлюзом (id, конец жизни в секундах). Старейшая уходит первой.
    sessions: Mutex<std::collections::VecDeque<(String, i64)>>,
    cookie_name: String,
    /// Ключ для местных программ: командной строки и MCP-сервера. Лежит в `data/runtime.json`.
    agent: String,
}

impl Guard {
    pub fn new(port: u16, boot_token: String, agent_token: String) -> Self {
        let hosts = vec![format!("127.0.0.1:{port}"), format!("localhost:{port}")];
        let origins = vec![format!("http://{}", hosts[0]), format!("http://{}", hosts[1])];
        Self { hosts, origins, cloud: None, boot: Mutex::new(vec![boot_token]), session: random_token(), sessions: Mutex::default(), cookie_name: format!("{COOKIE_PREFIX}_{port}"), agent: agent_token }
    }

    /// Облачный режим: к loopback добавляется ровно один публичный Host и один Origin,
    /// одноразовые ссылки `?boot=` гаснут, вход идёт только через `/auth/gateway`.
    pub fn with_cloud(mut self, config: CloudConfig) -> Self {
        self.hosts.push(config.host.clone());
        self.origins.push(config.origin.clone());
        self.boot = Mutex::new(Vec::new());
        self.cookie_name = CLOUD_COOKIE.to_string();
        let started = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0);
        self.cloud = Some(Cloud { config, started, seen: Mutex::new(Default::default()) });
        self
    }

    /// Новая одноразовая ссылка: окно пересоздаётся при открытии из трея.
    pub fn issue_boot(&self) -> String {
        let token = random_token();
        let mut slots = self.boot.lock().unwrap();
        // Неиспользованные ссылки не копятся: живы только последние.
        if slots.len() >= MAX_BOOT_LINKS {
            slots.remove(0);
        }
        slots.push(token.clone());
        token
    }

    fn take_boot(&self, offered: &str) -> bool {
        let mut slots = self.boot.lock().unwrap();
        if let Some(found) = slots.iter().position(|token| same(token, offered)) {
            slots.remove(found);
            return true;
        }
        false
    }

    fn cookies<'a>(&'a self, headers: &'a HeaderMap) -> impl Iterator<Item = &'a str> {
        headers
            .get_all(header::COOKIE)
            .iter()
            .filter_map(|value| value.to_str().ok())
            .flat_map(|value| value.split(';'))
            .filter_map(|pair| pair.trim().split_once('='))
            .filter(|(name, _)| *name == self.cookie_name)
            .map(|(_, value)| value)
    }

    fn has_session(&self, headers: &HeaderMap) -> bool {
        self.has_session_at(headers, now_secs())
    }

    /// Окно предъявило действующую сессию. В облаке — одну из выданных шлюзом и ещё не истёкшую
    /// (сравнение со всеми без раннего выхода), на ПК — единственную сессию процесса.
    fn has_session_at(&self, headers: &HeaderMap, now: i64) -> bool {
        if self.cloud.is_none() {
            return self.cookies(headers).any(|value| same(value, &self.session));
        }
        let live = self.sessions.lock().unwrap();
        let mut found = false;
        for value in self.cookies(headers) {
            for (id, until) in live.iter() {
                found |= same(value, id) & (*until > now);
            }
        }
        found
    }

    /// Облако: новая сессия после успешного входа через шлюз. При 64 живых уходит старейшая.
    fn new_session(&self, now: i64) -> String {
        let id = random_token();
        let mut live = self.sessions.lock().unwrap();
        live.retain(|(_, until)| *until > now);
        while live.len() >= MAX_SESSIONS {
            live.pop_front();
        }
        live.push_back((id.clone(), now + SESSION_TTL));
        id
    }

    /// Облако: выход. Удаляется только предъявленная сессия, остальные живут.
    fn end_session(&self, headers: &HeaderMap) {
        let offered: Vec<&str> = self.cookies(headers).collect();
        self.sessions.lock().unwrap().retain(|(id, _)| !offered.iter().any(|value| same(value, id)));
    }

    /// Местная программа с ключом из паспорта работающей копии. Только к API, и только если
    /// ключ непустой: пустой ключ не должен открывать доступ никому.
    fn has_agent(&self, peer: std::net::IpAddr, headers: &HeaderMap) -> bool {
        // Ключ лежит в паспорте копии на этой машине: из сети и через публичный адрес он не действует.
        let loopback_host = headers
            .get(header::HOST)
            .and_then(|value| value.to_str().ok())
            .is_some_and(|host| self.hosts[..2].iter().any(|allowed| allowed == host));
        if self.agent.is_empty() || !peer.is_loopback() || !loopback_host {
            return false;
        }
        // Облако: прокси Instabox ходит на контейнер с 127.0.0.1 и Host 127.0.0.1:<порт> и для запросов
        // из интернета. Такой запрос несёт заголовки прокси, и ключ программы по нему не действует.
        if self.cloud.is_some() && ["x-forwarded-for", "x-forwarded-host", "x-forwarded-proto", "forwarded", "x-real-ip"].iter().any(|name| headers.contains_key(*name)) {
            return false;
        }
        headers
            .get(header::AUTHORIZATION)
            .and_then(|value| value.to_str().ok())
            .and_then(|value| value.strip_prefix("Bearer "))
            .map(|offered| same(offered.trim(), &self.agent))
            .unwrap_or(false)
    }
}

fn refuse(status: StatusCode, message: &str) -> Response {
    (status, Json(json!({ "error": message }))).into_response()
}

/// Главное окно и питомцы могут открыться разом, у каждого своя ссылка.
const MAX_BOOT_LINKS: usize = 16;

fn boot_param(query: Option<&str>) -> Option<&str> {
    query?.split('&').find_map(|pair| pair.strip_prefix("boot="))
}

/// Куда вести после входа. Питомцы живут на своей странице, но адрес входа у них тот же
/// одноразовый `/`; список целей закрыт, чтобы ссылка не стала открытым редиректом.
fn boot_target(query: Option<&str>) -> &'static str {
    let wanted = query.and_then(|q| q.split('&').find_map(|pair| pair.strip_prefix("to=")));
    match wanted {
        Some("/pet.html") => "/pet.html",
        Some("/dock.html") => "/dock.html",
        Some("/snap.html") => "/snap.html",
        _ => "/",
    }
}

fn has_body(headers: &HeaderMap) -> bool {
    headers.contains_key(header::TRANSFER_ENCODING)
        || headers
            .get(header::CONTENT_LENGTH)
            .and_then(|v| v.to_str().ok())
            .is_some_and(|v| v != "0")
}

fn with_security_headers(mut response: Response) -> Response {
    let headers = response.headers_mut();
    let fixed: [(HeaderName, &'static str); 3] = [
        (header::CONTENT_SECURITY_POLICY, CSP),
        (header::X_CONTENT_TYPE_OPTIONS, "nosniff"),
        (header::REFERRER_POLICY, "no-referrer"),
    ];
    for (name, value) in fixed {
        headers.insert(name, HeaderValue::from_static(value));
    }
    response
}

fn normalize_path(path: &str) -> String {
    // Схлопываем повторные слэши и приводим к нижнему регистру, чтобы обход через
    // `/API/User-Chrome` или `//api/user-chrome` не проходил мимо проверок.
    let mut out = String::new();
    let mut prev_slash = false;
    for ch in path.chars() {
        if ch == '/' {
            if prev_slash {
                continue;
            }
            prev_slash = true;
        } else {
            prev_slash = false;
        }
        out.push(ch);
    }
    out.to_lowercase()
}

fn verdict(guard: &Guard, peer: std::net::IpAddr, method: &Method, path: &str, query: Option<&str>, headers: &HeaderMap) -> Option<Response> {
    let text = |name: HeaderName| headers.get(name).and_then(|v| v.to_str().ok());
    let norm = normalize_path(path);

    // Rebinding: чужой домен, указывающий на 127.0.0.1, приходит со своим Host.
    let host_ok = text(header::HOST).is_some_and(|host| guard.hosts.iter().any(|allowed| allowed == host));
    if !host_ok {
        return Some(refuse(StatusCode::FORBIDDEN, "запрос не к этому приложению"));
    }

    // Вебхуки приходят снаружи, без сессии окна: их пускает секрет в адресе.
    // Host уже проверен, поэтому чужой домен, указывающий на 127.0.0.1, сюда не пройдёт.
    if norm.starts_with("/hooks/") {
        return if method == Method::POST { None } else { Some(refuse(StatusCode::METHOD_NOT_ALLOWED, "только POST")) };
    }

    // CSRF: браузер сам подписывает, откуда запрос.
    if headers.contains_key(header::ORIGIN) {
        let origin_ok = text(header::ORIGIN).is_some_and(|origin| guard.origins.iter().any(|allowed| allowed == origin));
        if !origin_ok {
            return Some(refuse(StatusCode::FORBIDDEN, "запрос с чужого сайта"));
        }
    }
    // Переход со шлюза на вход — обычная навигация GET с другого адреса (same-site / cross-site).
    // Только для этого пути; защита от подделки входа — подпись, uid и одноразовый nonce.
    let gateway_nav = guard.cloud.is_some() && method == Method::GET && norm == GATEWAY_PATH;
    if let Some(site) = headers.get("sec-fetch-site").and_then(|v| v.to_str().ok()) {
        if site != "same-origin" && site != "none" && !gateway_nav {
            return Some(refuse(StatusCode::FORBIDDEN, "запрос с чужого сайта"));
        }
    }

    // text/plain браузер отправляет с другого сайта без предварительной проверки: такие не берём.
    if method != Method::GET && method != Method::HEAD && has_body(headers) {
        let is_json = text(header::CONTENT_TYPE)
            .and_then(|kind| kind.split(';').next())
            .is_some_and(|kind| kind.trim().eq_ignore_ascii_case("application/json"));
        if !is_json {
            return Some(refuse(StatusCode::UNSUPPORTED_MEDIA_TYPE, "нужен application/json"));
        }
    }

    // Вход через шлюз (облако). Все причины отказа — один и тот же ответ.
    if gateway_nav {
        let token = query.and_then(|q| q.split('&').find_map(|pair| pair.strip_prefix("t="))).unwrap_or("");
        let now = now_secs();
        if guard.cloud.as_ref().is_some_and(|cloud| cloud.accept(token, now)) {
            let cookie = format!("{}={}; HttpOnly; Secure; SameSite=Strict; Path=/; Max-Age={SESSION_TTL}", guard.cookie_name, guard.new_session(now));
            return Some(redirect_home(Some(cookie), "/"));
        }
        return Some(refuse(StatusCode::FORBIDDEN, GATEWAY_REFUSAL));
    }
    // Выход (облако): только POST; Origin и Sec-Fetch уже проверены выше, как у любого POST.
    if guard.cloud.is_some() && norm == LOGOUT_PATH {
        if method != Method::POST {
            return Some(refuse(StatusCode::METHOD_NOT_ALLOWED, "только POST"));
        }
        guard.end_session(headers);
        let cookie = format!("{}=; HttpOnly; Secure; SameSite=Strict; Path=/; Max-Age=0", guard.cookie_name);
        return Some(Response::builder().status(StatusCode::NO_CONTENT).header(header::SET_COOKIE, cookie).header(header::CACHE_CONTROL, "no-store").body(Body::empty()).unwrap());
    }
    if norm == GATEWAY_PATH {
        // Вне облака пути нет: отвечаем как на любой запрос без сессии.
        return Some(refuse(StatusCode::FORBIDDEN, "нет сессии: открой приложение Deloist"));
    }

    // Вход: окно приложения приходит один раз с токеном и получает сессию.
    let session = guard.has_session(headers);
    // В облаке одноразовых ссылок нет совсем: даже с действующей сессией `?boot=` не обрабатывается.
    if guard.cloud.is_some() && norm == "/" && boot_param(query).is_some() {
        return Some(refuse(StatusCode::FORBIDDEN, "ссылка для входа не действует"));
    }
    if norm == "/" {
        if let Some(offered) = boot_param(query) {
            if session {
                return Some(redirect_home(None, boot_target(query)));
            }
            if guard.take_boot(offered) {
                let cookie = format!("{}={}; HttpOnly; SameSite=Strict; Path=/", guard.cookie_name, guard.session);
                return Some(redirect_home(Some(cookie), boot_target(query)));
            }
            return Some(refuse(StatusCode::FORBIDDEN, "ссылка для входа уже не действует"));
        }
    }
    // Политика облака: у ботов нет консоли. Маршруты, которые запускают процессы, ставят программы или
    // трогают рабочий стол и файлы вне папки бота, закрыты для всех, и для человека тоже.
    if guard.cloud.is_some() && cloud_closed(&norm) {
        return Some(refuse(StatusCode::FORBIDDEN, crate::cloud::NO_FEATURE));
    }
    // Местная программа (командная строка, MCP-сервер) приходит с ключом из паспорта копии.
    // Только к API: страницы и вебхуки этого ключа не принимают.
    if !session && norm.starts_with("/api/") && guard.has_agent(peer, headers) {
        // Ответы человека и правила — только из окна. Ключ лежит в паспорте копии, его может
        // прочитать скрипт бота в своей папке и выдать себе вечные разрешения.
        if human_only(&norm) || catalog_write(method, &norm) || config_write(method, &norm) || habit_write(method, &norm) || todo_write(method, &norm) || room_write(method, &norm) || bot_room_create(method, &norm) || bot_room_delete(method, &norm) || routine_write(method, &norm) {
            return Some(refuse(StatusCode::FORBIDDEN, "это решает человек в окне приложения"));
        }
        return None;
    }
    if !session {
        return Some(refuse(StatusCode::FORBIDDEN, "нет сессии: открой приложение Deloist"));
    }
    None
}

/// Пути, закрытые в облаке (`norm` уже в нижнем регистре, без повторных слэшей).
pub(crate) fn cloud_closed(norm: &str) -> bool {
    const PREFIXES: [&str; 19] = [
        "/api/vault",              // менеджеры паролей через op и bw
        "/api/selfdev",            // самодоработка: git и cargo
        "/api/browser/obscura",    // чужой движок, скачивание и запуск
        "/api/browser/moli/install",
        "/api/user-chrome",        // «Мой Chrome» человека
        "/api/selection",
        "/api/hotkeys",
        "/api/dock",
        "/api/shortcut",
        "/api/updates/app",        // обновление идёт новым образом, а не заменой файла
        "/api/updates/restart",    // перезапуск на поставленную версию
        "/api/updates/tools",      // npm и CLI-агенты
        "/api/providers/scan",
        "/api/voice/prepare",      // локальное распознавание речи
        "/api/voice/settings",
        "/api/voice/local",
        "/api/import/hermes",      // читает домашнюю папку
        "/api/know-me",            // читает домашнюю папку
        "/api/cloud/instabox",     // управление облаком идёт с ПК, не изнутри него
    ];
    if PREFIXES.iter().any(|p| norm == *p || norm.starts_with(&format!("{p}/"))) {
        return true;
    }
    if let Some(rest) = norm.strip_prefix("/api/providers/") {
        // Установка агентов, проверка и вход через их программу, ссылка входа, выход и ключ Gemini.
        let tail = rest.split_once('/').map(|(_, t)| t).unwrap_or("");
        if matches!(tail, "install" | "check" | "connect" | "cancel" | "signin-link" | "logout") || tail.starts_with("accounts") {
            return true;
        }
    }
    if let Some(rest) = norm.strip_prefix("/api/bots/") {
        let tail = rest.split_once('/').map(|(_, t)| t).unwrap_or("");
        // Экран и питомцы: окна рабочего стола.
        if tail == "screen" || tail.starts_with("screen/") || tail == "pet-place" || tail == "file/open-in-vscode" {
            return true;
        }
    }
    false
}

fn redirect_home(cookie: Option<String>, target: &'static str) -> Response {
    let mut builder = Response::builder()
        .status(StatusCode::SEE_OTHER)
        .header(header::LOCATION, target)
        .header(header::CACHE_CONTROL, "no-store");
    if let Some(cookie) = cookie {
        builder = builder.header(header::SET_COOKIE, cookie);
    }
    builder.body(Body::empty()).unwrap()
}

/// Пути, где отвечает только человек: карточки, вопросы, правила, разрешения и подключения.
fn human_only(path: &str) -> bool {
    path.starts_with("/api/approvals/")
        || path.starts_with("/api/questions/")
        || path.starts_with("/api/permissions")
        // Доверенные устройства: пропуск телефона и второго ПК, список и отзыв решает только человек.
        || path.starts_with("/api/devices")
        // Режим сервера включает автозапуск и запрет сна: решает только человек.
        || path.starts_with("/api/server-mode")
        // Боты других ПК аккаунта: знакомство, чтение, сообщения и одобрения на чужом ПК — только окно.
        || path.starts_with("/api/remote/")
        // Правка выделенного вставляет текст в окно человека, сочетания клавиш — его настройка,
        // распознавание речи тратит его баланс: скрипту бота сюда нельзя.
        || path.starts_with("/api/selection/")
        // Установка и перезапуск на новую версию: меняют то, что запускается на компьютере.
        || path == "/api/updates/app"
        || path == "/api/updates/restart"
        || path.starts_with("/api/hotkeys")
        || path.starts_with("/api/voice/")
        // Режим разработки открывает боту исходники программы: включает его только человек.
        || path.starts_with("/api/selfdev")
        // Подключения: согласие на запуск, ключи и входы — решение человека.
        || path.starts_with("/api/connectors")
        // Вход в свой MCP-сервер по OAuth: ссылка входа, client id и секрет клиента — решение человека.
        || (path.starts_with("/api/mcp/") && path.contains("/oauth/"))
        // «Начать бесплатно»: вход в OpenRouter открывает браузер и кладёт ключ в хранилище.
        || (path.starts_with("/api/providers/") && (path.ends_with("/connect") || path.ends_with("/cancel")))
        || path.starts_with("/api/know-me")
        // Токен своего бота Telegram, привязка чата и переписка от лица человека — только окно.
        || path.starts_with("/api/telegram")
        // Токен своего бота Discord, привязка личной переписки и ответы от лица человека — тоже.
        || path.starts_with("/api/discord")
        // Токены приложения Slack, привязка личной переписки и ответы от лица человека.
        || path.starts_with("/api/slack")
        // Вход человека в Google в браузере бота и отметка состояния — только окно.
        // Ветка треда создаёт бота с персоной и перепиской: это тоже решает человек.
        || (path.starts_with("/api/bots/")
            && (path.ends_with("/browser/sign-in")
                || path.ends_with("/sign-in-state")
                || path.ends_with("/branch")
                // Запуск редактора кода на компьютере человека: только по его кнопке в просмотрщике файлов.
                || path.ends_with("/file/open-in-vscode")))
        // Правила бота в мессенджерах (что можно писать, вступать, удалять): задаёт только человек в окне.
        || (path.starts_with("/api/bots/") && (path.ends_with("/messengers") || path.contains("/messengers/")))
        // Перенос ботов из Hermes создаёт ботов и пишет их персоны и память: решает человек.
        || path.starts_with("/api/import/hermes")
        // Передача задачи в группе запускает ход бота так же, как сообщение человека.
        || (path.starts_with("/api/rooms/") && path.ends_with("/reassign"))
        // «Перестроить команду» возвращает расспрос оркестратора и может добавить его в команду: решает человек.
        || (path.starts_with("/api/rooms/") && path.trim_end_matches('/').ends_with("/rebuild"))
        // Согласие на специалиста запускает ход и передаёт работу: решает человек.
        || (path.starts_with("/api/bots/") && path.contains("/offers/"))
        // Заявка командной строки на добавление команды из файла: ключ программы её подаёт, читает статус
        // по номеру и отзывает (`cancel`), а принять, отклонить и увидеть список ждущих может только окно.
        || path.starts_with("/api/team/import-requests")
        || (path.starts_with("/api/team/import-request/") && (path.trim_end_matches('/').ends_with("/approve") || path.trim_end_matches('/').ends_with("/deny")))
        // Бот-оптимизатор читает журналы и спрашивает человека: ответ, выключатель и список
        // его решений видит и меняет только окно. Ключ программы из `runtime.json` сюда не ходит.
        || path.starts_with("/api/optimizer")
        // Рабочие процессы: создают, запускают и решают согласия только из окна. Шаг-бот ходит внутри программы.
        || path.starts_with("/api/workflows")
        || path.starts_with("/api/workflow-")
        || (path.starts_with("/api/bots/") && path.ends_with("/optimizer"))
        // Пустить бота в свой Chrome решает только человек.
        || path.starts_with("/api/user-chrome")
        // Скачать, удалить или сменить движок браузера бота (Obscura, Moli) решает человек:
        // это установка программы на его компьютер.
        || path.starts_with("/api/browser/obscura")
        || path.starts_with("/api/browser/moli")
        // Ссылка входа Google ведёт в аккаунт человека: её видит только окно, ключу программы она не отдаётся.
        || (path.starts_with("/api/providers/") && path.ends_with("/signin-link"))
        // Менеджер паролей: подключение и разблокировка хранилища — решение человека. Пароли и коды
        // по этим адресам не отдаются вообще; ключ программы сюда не должен достать ни входа, ни токена.
        || path.starts_with("/api/vault")
        // Облако на Instabox: вход в аккаунт, создание и удаление машины, пропуск шлюза. Ключ Instabox и
        // ключи облака лежат в секретах; ключу программы сюда нельзя ни читать состояние, ни открывать облако.
        || path.starts_with("/api/cloud/instabox")
        // Начать с пустыми ключами откладывает файл секретов в сторону: решает только человек.
        || path.starts_with("/api/secrets")
        // Проверка напоминания шлёт сообщение человеку прямо сейчас: это решает человек в окне.
        || (path.starts_with("/api/routines/") && path.ends_with("/test-reminder"))
        // Аккаунт Deloist: согласие с документами даёт только сам человек; выгрузка отдаёт его
        // личные данные; возврат денег и удаление аккаунта — его решение.
        || path.starts_with("/api/account/consent")
        || path.starts_with("/api/account/details")
        || path.starts_with("/api/account/refund")
        || path.starts_with("/api/account/export")
        || path.starts_with("/api/account/delete")
}

/// Привычки меняет и отмечает только человек в окне. Бот отмечает свои привычки инструментом
/// `check_habit`, который идёт внутри программы, а не через этот адрес. Читать ключу можно.
fn habit_write(method: &Method, path: &str) -> bool {
    (path == "/api/habits" || path.starts_with("/api/habits/")) && method != Method::GET && method != Method::HEAD
}

/// Задачи меняет и отмечает только человек в окне. Бот добавляет и отмечает свои задачи инструментами
/// `add_todo` и `complete_todo`, которые идут внутри программы, а не через этот адрес. Читать ключу можно.
fn todo_write(method: &Method, path: &str) -> bool {
    (path == "/api/todos" || path.starts_with("/api/todos/")) && method != Method::GET && method != Method::HEAD
}

/// Установка из каталога создаёт ботов, даёт сайты и подключает приложения: это решает человек.
/// Смотреть каталог ключу можно.
fn catalog_write(method: &Method, path: &str) -> bool {
    (path == "/api/catalog" || path.starts_with("/api/catalog/")) && method != Method::GET && method != Method::HEAD
}

/// Настройки и провайдеры задают команды агентов (`cliCommand`, `acpCommand`), адреса API и ключи.
/// Скрипт бота с ключом программы направил бы так ключ человека на свой адрес или запустил свою
/// программу без карточки. Смотреть их ключу можно, менять — только человеку в окне.
/// Настройки, провайдеры и аккаунт: читать ключу программы можно, менять нельзя. Выход из аккаунта,
/// смена тарифа, пополнение и вход — деньги и учётная запись человека (13.11.2026).
/// Правка команды (PATCH /api/rooms/{id}) меняет состав, общие папки и навыки, то есть доступ ботов:
/// скрипт бота с ключом программы не должен ни открыть папку, ни вписать бота в команду с папками.
fn room_write(method: &Method, path: &str) -> bool {
    method == Method::PATCH && path.strip_prefix("/api/rooms/").is_some_and(|rest| !rest.is_empty() && !rest.contains('/'))
}

/// Создание бота и команды (`POST /api/bots`, `POST /api/rooms`) принимает `approval`, сайты, папки и навыки:
/// скрипт бота с ключом программы создал бы бота со «Свободой действий» или команду с общей папкой.
/// Боты собирают команду инструментом `propose_team`: решение принимает человек по карточке.
fn bot_room_create(method: &Method, path: &str) -> bool {
    method == Method::POST && matches!(path.trim_end_matches('/'), "/api/bots" | "/api/rooms" | "/api/team/import")
}

/// Удаление бота и команды (`DELETE /api/bots/{id}`, `DELETE /api/rooms/{id}`) решает человек в окне.
fn bot_room_delete(method: &Method, path: &str) -> bool {
    method == Method::DELETE
        && ["/api/bots/", "/api/rooms/"].iter().any(|prefix| path.strip_prefix(prefix).is_some_and(|rest| !rest.trim_end_matches('/').is_empty() && !rest.trim_end_matches('/').contains('/')))
}

/// Рутина для другого бота со «Свободой действий» — это команды без человека. Ключу программы создавать,
/// менять и удалять рутины нельзя (читать и запускать можно). Агенты ведут свои рутины инструментами
/// `create_routine` / `update_routine` / `delete_routine` через мост, там действуют карточки одобрения.
fn routine_write(method: &Method, path: &str) -> bool {
    let rest = path.trim_end_matches('/');
    match rest.strip_prefix("/api/routines") {
        Some("") => method == Method::POST,
        Some(tail) => tail.strip_prefix('/').is_some_and(|id| !id.is_empty() && !id.contains('/') && !id.contains('.')) && (method == Method::PATCH || method == Method::DELETE),
        None => false,
    }
}

fn config_write(method: &Method, path: &str) -> bool {
    (path == "/api/settings" || path == "/api/providers" || path.starts_with("/api/providers/") || path.starts_with("/api/account/"))
        && method != Method::GET
        && method != Method::HEAD
}

/// Запрос пришёл из окна приложения (с сессией), а не по ключу программы.
#[derive(Clone, Copy)]
pub struct WindowSession;

pub async fn check(
    State(guard): State<Arc<Guard>>,
    ConnectInfo(peer): ConnectInfo<std::net::SocketAddr>,
    mut request: Request,
    next: Next,
) -> Response {
    let refused = verdict(&guard, peer.ip(), request.method(), request.uri().path(), request.uri().query(), request.headers());
    if refused.is_none() && guard.has_session(request.headers()) {
        request.extensions_mut().insert(WindowSession);
    }
    let response = match refused {
        Some(response) => response,
        None => next.run(request).await,
    };
    with_security_headers(response)
}

#[cfg(test)]
mod tests {
    use super::*;
    use base64::Engine;
    use ed25519_dalek::{Signer, SigningKey};

    const B64: base64::engine::GeneralPurpose = base64::engine::general_purpose::URL_SAFE_NO_PAD;
    const ORIGIN: &str = "https://u-test.app.example.org";

    fn check_as(guard: &Guard, method: &Method, path: &str, query: Option<&str>, headers: &HeaderMap) -> Option<Response> {
        verdict(guard, std::net::IpAddr::from([127, 0, 0, 1]), method, path, query, headers)
    }

    fn signing_key() -> SigningKey {
        let mut seed = [0u8; 32];
        rand::thread_rng().fill_bytes(&mut seed);
        SigningKey::from_bytes(&seed)
    }

    fn env(key: &SigningKey, extra: &[(&'static str, &'static str)]) -> impl Fn(&str) -> Option<String> {
        let mut vars: Vec<(String, String)> = vec![
            ("CREW_CLOUD".into(), "1".into()),
            ("CREW_GATEWAY_PUBKEY".into(), crate::store::hex(key.verifying_key().as_bytes())),
            ("CREW_USER_ID".into(), "user_1-a".into()),
            ("CREW_PUBLIC_ORIGIN".into(), ORIGIN.into()),
        ];
        for (name, value) in extra {
            vars.retain(|(n, _)| n != name);
            if !value.is_empty() {
                vars.push((name.to_string(), value.to_string()));
            }
        }
        move |name: &str| vars.iter().find(|(n, _)| n == name).map(|(_, v)| v.clone())
    }

    fn cloud_guard(key: &SigningKey) -> Guard {
        let config = CloudConfig::parse(env(key, &[])).unwrap().unwrap();
        Guard::new(8899, "boot".into(), "agent".into()).with_cloud(config)
    }

    fn token(key: &SigningKey, payload: &str) -> String {
        format!("{}.{}", B64.encode(payload), B64.encode(key.sign(&[GATEWAY_DOMAIN, payload.as_bytes()].concat()).to_bytes()))
    }

    /// Подпись прежнего формата: без префикса назначения.
    fn legacy_token(key: &SigningKey, payload: &str) -> String {
        format!("{}.{}", B64.encode(payload), B64.encode(key.sign(payload.as_bytes()).to_bytes()))
    }

    fn cookie_of(response: &Response) -> String {
        response.headers()[header::SET_COOKIE].to_str().unwrap().split(';').next().unwrap().to_string()
    }

    fn with_cookie(cookie: &str) -> HeaderMap {
        let mut h = headers("u-test.app.example.org");
        h.insert(header::COOKIE, cookie.parse().unwrap());
        h
    }

    fn now() -> i64 {
        std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs() as i64
    }

    fn claims(uid: &str, exp: i64, nonce: &str) -> String {
        full_claims(uid, exp, nonce, now(), ORIGIN)
    }

    fn headers(host: &str) -> HeaderMap {
        let mut h = HeaderMap::new();
        h.insert(header::HOST, host.parse().unwrap());
        h
    }

    fn gateway(guard: &Guard, t: &str) -> Option<Response> {
        let mut h = headers("u-test.app.example.org");
        h.insert("sec-fetch-site", "cross-site".parse().unwrap());
        check_as(guard, &Method::GET, "/auth/gateway", Some(&format!("t={t}")), &h)
    }

    fn granted(response: Option<Response>) -> bool {
        response.is_some_and(|r| r.status() == StatusCode::SEE_OTHER && r.headers().contains_key(header::SET_COOKIE))
    }

    #[test]
    fn startup_requires_every_variable() {
        let key = signing_key();
        assert!(CloudConfig::parse(env(&key, &[])).unwrap().is_some());
        // Без CREW_CLOUD=1 режим не включается, остальное не смотрим.
        assert!(CloudConfig::parse(env(&key, &[("CREW_CLOUD", "")])).unwrap().is_none());
        assert!(CloudConfig::parse(env(&key, &[("CREW_CLOUD", "true")])).unwrap().is_none());
        for name in ["CREW_GATEWAY_PUBKEY", "CREW_USER_ID", "CREW_PUBLIC_ORIGIN"] {
            assert!(CloudConfig::parse(env(&key, &[(name, "")])).is_err(), "{name} пуст");
        }
        let bad = [
            ("CREW_GATEWAY_PUBKEY", "abcd"),
            ("CREW_GATEWAY_PUBKEY", "zz00000000000000000000000000000000000000000000000000000000000000"),
            ("CREW_GATEWAY_PUBKEY", "00000000000000000000000000000000000000000000000000000000000000000000"),
            ("CREW_USER_ID", "a b"),
            ("CREW_USER_ID", "a/b"),
            ("CREW_USER_ID", "жук"),
            ("CREW_USER_ID", "0123456789012345678901234567890123456789012345678901234567890123456789"),
            ("CREW_PUBLIC_ORIGIN", "http://u-test.app.example.org"),
            ("CREW_PUBLIC_ORIGIN", "u-test.app.example.org"),
            ("CREW_PUBLIC_ORIGIN", "https://u-test.app.example.org/"),
            ("CREW_PUBLIC_ORIGIN", "https://u-test.app.example.org/path"),
            ("CREW_PUBLIC_ORIGIN", "https://user@u-test.app.example.org"),
            ("CREW_PUBLIC_ORIGIN", "https://u-test.app.example.org?x=1"),
            ("CREW_PUBLIC_ORIGIN", "https://"),
            ("CREW_PUBLIC_ORIGIN", "https://u-test.app.example.org:443"),
        ];
        for (name, value) in bad {
            assert!(CloudConfig::parse(env(&key, &[(name, value)])).is_err(), "{name}={value} принят");
        }
        // Порт допустим, он входит в Host и Origin.
        let config = CloudConfig::parse(env(&key, &[("CREW_PUBLIC_ORIGIN", "https://Host.Example:8443")])).unwrap().unwrap();
        assert_eq!(config.host, "host.example:8443");
        assert_eq!(config.origin, "https://host.example:8443");
    }

    #[test]
    fn valid_token_gives_a_secure_session_once() {
        let key = signing_key();
        let guard = cloud_guard(&key);
        let t = token(&key, &claims("user_1-a", now() + 30, "0123456789abcdef"));
        let response = gateway(&guard, &t).expect("ответ");
        assert_eq!(response.status(), StatusCode::SEE_OTHER);
        assert_eq!(response.headers()[header::LOCATION], "/");
        let cookie = response.headers()[header::SET_COOKIE].to_str().unwrap().to_string();
        assert!(cookie.contains("HttpOnly") && cookie.contains("Secure") && cookie.contains("SameSite=Strict") && cookie.contains("Path=/"), "{cookie}");
        // С этой cookie проходит сторож.
        let mut h = headers("u-test.app.example.org");
        h.insert(header::COOKIE, cookie.split(';').next().unwrap().parse().unwrap());
        assert!(check_as(&guard, &Method::GET, "/api/state", None, &h).is_none());
        // Тот же токен второй раз — отказ.
        assert!(!granted(gateway(&guard, &t)));
    }

    #[test]
    fn cloud_sessions_expire_and_the_clock_is_injectable() {
        let key = signing_key();
        let guard = cloud_guard(&key);
        let id = guard.new_session(1_000);
        let h = with_cookie(&format!("{CLOUD_COOKIE}={id}"));
        assert!(guard.has_session_at(&h, 1_000));
        assert!(guard.has_session_at(&h, 1_000 + SESSION_TTL - 1));
        assert!(!guard.has_session_at(&h, 1_000 + SESSION_TTL), "истекла ровно через 12 часов");
        assert!(!guard.has_session_at(&with_cookie(&format!("{CLOUD_COOKIE}=not-a-session")), 1_000));
        // Процессная сессия ПК в облаке не действует.
        assert!(!guard.has_session_at(&with_cookie(&format!("{CLOUD_COOKIE}={}", guard.session)), 1_000));
    }

    #[test]
    fn the_oldest_cloud_session_is_evicted_at_the_cap() {
        let key = signing_key();
        let guard = cloud_guard(&key);
        let first = guard.new_session(10);
        for i in 1..MAX_SESSIONS as i64 {
            guard.new_session(10 + i);
        }
        assert!(guard.has_session_at(&with_cookie(&format!("{CLOUD_COOKIE}={first}")), 11), "ровно 64 живы");
        guard.new_session(100);
        assert_eq!(guard.sessions.lock().unwrap().len(), MAX_SESSIONS);
        assert!(!guard.has_session_at(&with_cookie(&format!("{CLOUD_COOKIE}={first}")), 101), "старейшая вытеснена");
    }

    #[test]
    fn two_logins_give_two_sessions_and_logout_ends_only_its_own() {
        let key = signing_key();
        let guard = cloud_guard(&key);
        let a = gateway(&guard, &token(&key, &claims("user_1-a", now() + 30, "0123456789abcdef"))).expect("вход A");
        let b = gateway(&guard, &token(&key, &claims("user_1-a", now() + 30, "fedcba9876543210"))).expect("вход B");
        let (ca, cb) = (cookie_of(&a), cookie_of(&b));
        assert_ne!(ca, cb, "у каждого входа своя сессия");
        assert!(check_as(&guard, &Method::GET, "/api/state", None, &with_cookie(&ca)).is_none());
        assert!(check_as(&guard, &Method::GET, "/api/state", None, &with_cookie(&cb)).is_none());

        // GET на выход не принимается.
        assert_eq!(check_as(&guard, &Method::GET, "/auth/logout", None, &with_cookie(&ca)).unwrap().status(), StatusCode::METHOD_NOT_ALLOWED);
        // Чужой Origin и чужой Sec-Fetch-Site отклоняются, сессия цела.
        let mut evil = with_cookie(&ca);
        evil.insert(header::ORIGIN, "https://evil.example".parse().unwrap());
        assert_eq!(check_as(&guard, &Method::POST, "/auth/logout", None, &evil).unwrap().status(), StatusCode::FORBIDDEN);
        let mut cross = with_cookie(&ca);
        cross.insert("sec-fetch-site", "cross-site".parse().unwrap());
        assert_eq!(check_as(&guard, &Method::POST, "/auth/logout", None, &cross).unwrap().status(), StatusCode::FORBIDDEN);
        assert!(check_as(&guard, &Method::GET, "/api/state", None, &with_cookie(&ca)).is_none(), "сессия A жива");

        let out = check_as(&guard, &Method::POST, "/auth/logout", None, &with_cookie(&ca)).expect("ответ");
        assert_eq!(out.status(), StatusCode::NO_CONTENT);
        let cleared = out.headers()[header::SET_COOKIE].to_str().unwrap();
        assert!(cleared.contains("Max-Age=0") && cleared.starts_with(&format!("{CLOUD_COOKIE}=;")), "{cleared}");
        assert!(check_as(&guard, &Method::GET, "/api/state", None, &with_cookie(&ca)).is_some(), "A закрыта");
        assert!(check_as(&guard, &Method::GET, "/api/state", None, &with_cookie(&cb)).is_none(), "B жива");
    }

    #[test]
    fn a_signature_without_the_domain_prefix_is_refused() {
        let key = signing_key();
        let guard = cloud_guard(&key);
        let payload = claims("user_1-a", now() + 30, "0123456789abcdef");
        assert!(!granted(gateway(&guard, &legacy_token(&key, &payload))), "старый формат подписи принят");
        assert!(granted(gateway(&guard, &token(&key, &payload))));
    }

    #[tokio::test]
    async fn bad_tokens_get_the_same_refusal() {
        let key = signing_key();
        let other = signing_key();
        let guard = cloud_guard(&key);
        let n = now();
        let good = claims("user_1-a", n + 30, "aaaaaaaaaaaaaaaa");
        let cases = [
            ("чужая подпись", token(&other, &good)),
            ("чужой uid", token(&key, &claims("user_2", n + 30, "bbbbbbbbbbbbbbbb"))),
            ("просрочен", token(&key, &claims("user_1-a", n - 1, "cccccccccccccccc"))),
            ("слишком далеко", token(&key, &claims("user_1-a", n + 3600, "dddddddddddddddd"))),
            ("короткий nonce", token(&key, &claims("user_1-a", n + 30, "short"))),
            ("без nonce", token(&key, &json!({ "uid": "user_1-a", "exp": n + 30 }).to_string())),
            ("не json", token(&key, "garbage")),
            ("без точки", "abc".to_string()),
            ("пусто", String::new()),
            ("не base64", "!!!.???".to_string()),
            ("нулевая подпись", format!("{}.{}", B64.encode(&good), B64.encode([0u8; 64]))),
        ];
        let mut bodies = std::collections::HashSet::new();
        for (name, t) in cases {
            let response = gateway(&guard, &t).unwrap_or_else(|| panic!("{name}: пропущен"));
            assert_eq!(response.status(), StatusCode::FORBIDDEN, "{name}");
            assert!(!response.headers().contains_key(header::SET_COOKIE), "{name}");
            bodies.insert(axum::body::to_bytes(response.into_body(), 4096).await.unwrap());
        }
        assert_eq!(bodies.len(), 1, "тексты отказов различаются");
        // Хороший токен после всех неудач всё ещё работает: отказы nonce не сжигают.
        assert!(granted(gateway(&guard, &token(&key, &good))));
    }

    #[test]
    fn host_and_origin_are_exact() {
        let key = signing_key();
        let guard = cloud_guard(&key);
        let t = token(&key, &claims("user_1-a", now() + 30, "eeeeeeeeeeeeeeee"));
        let session = gateway(&guard, &t).unwrap().headers()[header::SET_COOKIE].to_str().unwrap().split(';').next().unwrap().to_string();
        let with = |host: &str, origin: Option<&str>| {
            let mut h = headers(host);
            h.insert(header::COOKIE, session.parse().unwrap());
            if let Some(origin) = origin {
                h.insert(header::ORIGIN, origin.parse().unwrap());
            }
            check_as(&guard, &Method::GET, "/api/state", None, &h)
        };
        assert!(with("u-test.app.example.org", Some(ORIGIN)).is_none());
        assert!(with("127.0.0.1:8899", Some("http://127.0.0.1:8899")).is_none());
        for host in ["evil.example", "u-test.app.example.org:8443", "x.u-test.app.example.org", "u-other.app.example.org"] {
            assert!(with(host, None).is_some(), "{host}");
        }
        for origin in ["https://evil.example", "http://u-test.app.example.org", "https://u-other.app.example.org", "null"] {
            assert!(with("u-test.app.example.org", Some(origin)).is_some(), "{origin}");
        }
        // Токен на чужом Host не принимается.
        let t2 = token(&key, &claims("user_1-a", now() + 30, "ffffffffffffffff"));
        let mut h = headers("evil.example");
        h.insert("sec-fetch-site", "cross-site".parse().unwrap());
        assert!(!granted(check_as(&guard, &Method::GET, "/auth/gateway", Some(&format!("t={t2}")), &h)));
    }

    #[test]
    fn boot_link_is_dead_in_the_cloud_and_gateway_path_is_dead_outside() {
        let key = signing_key();
        let guard = cloud_guard(&key);
        let h = headers("u-test.app.example.org");
        let r = check_as(&guard, &Method::GET, "/", Some("boot=boot"), &h).unwrap();
        assert_eq!(r.status(), StatusCode::FORBIDDEN);
        assert!(!r.headers().contains_key(header::SET_COOKIE));
        // На ПК пути нет, даже с подписанным токеном.
        let plain = Guard::new(8899, "boot".into(), "agent".into());
        let t = token(&key, &claims("user_1-a", now() + 30, "gggggggggggggggg"));
        let r = check_as(&plain, &Method::GET, "/auth/gateway", Some(&format!("t={t}")), &headers("127.0.0.1:8899")).unwrap();
        assert_eq!(r.status(), StatusCode::FORBIDDEN);
        assert!(!r.headers().contains_key(header::SET_COOKIE));
        // И публичный Host на ПК не принят.
        assert!(check_as(&plain, &Method::GET, "/", None, &headers("u-test.app.example.org")).is_some());
    }

    fn full_claims(uid: &str, exp: i64, nonce: &str, iat: i64, aud: &str) -> String {
        json!({ "uid": uid, "aud": aud, "iat": iat, "exp": exp, "nonce": nonce }).to_string()
    }

    fn cloud_of(guard: &Guard) -> &Cloud {
        guard.cloud.as_ref().unwrap()
    }

    #[test]
    fn exp_boundary_is_inclusive_and_iat_aud_are_checked() {
        let key = signing_key();
        let guard = cloud_guard(&key);
        let cloud = cloud_of(&guard);
        let n = now();
        let t = |exp: i64, nonce: &str, iat: i64, aud: &str| token(&key, &full_claims("user_1-a", exp, nonce, iat, aud));
        assert!(cloud.accept(&t(n, "0000000000000001", n, ORIGIN), n), "exp == now");
        assert!(!cloud.accept(&t(n - 1, "0000000000000002", n, ORIGIN), n), "exp == now-1");
        assert!(cloud.accept(&t(n + 60, "0000000000000003", n, ORIGIN), n), "exp == now+60");
        assert!(!cloud.accept(&t(n + 61, "0000000000000004", n, ORIGIN), n), "exp == now+61");
        // iat раньше старта процесса (минус допуск) и в будущем.
        assert!(!cloud.accept(&t(n + 30, "0000000000000005", cloud.started - 3, ORIGIN), n), "iat до старта");
        assert!(cloud.accept(&t(n + 30, "0000000000000006", cloud.started - 2, ORIGIN), n), "iat на границе");
        assert!(!cloud.accept(&t(n + 30, "0000000000000007", n + 6, ORIGIN), n), "iat из будущего");
        // Не тому адресу.
        assert!(!cloud.accept(&t(n + 30, "0000000000000008", n, "https://u-other.app.example.org"), n), "чужой aud");
        assert!(!cloud.accept(&t(n + 30, "0000000000000009", n, ""), n), "пустой aud");
        // Нет iat или aud вовсе.
        let no_iat = json!({ "uid": "user_1-a", "aud": ORIGIN, "exp": n + 30, "nonce": "000000000000000a" }).to_string();
        assert!(!cloud.accept(&token(&key, &no_iat), n));
        let no_aud = json!({ "uid": "user_1-a", "iat": n, "exp": n + 30, "nonce": "000000000000000b" }).to_string();
        assert!(!cloud.accept(&token(&key, &no_aud), n));
    }

    #[test]
    fn nonce_set_is_bounded_and_pruned() {
        let key = signing_key();
        let guard = cloud_guard(&key);
        let cloud = cloud_of(&guard);
        let n = now();
        for i in 0..MAX_SEEN_NONCES {
            let t = token(&key, &full_claims("user_1-a", n + 30, &format!("{i:016}"), n, ORIGIN));
            assert!(cloud.accept(&t, n), "токен {i}");
        }
        let extra = token(&key, &full_claims("user_1-a", n + 30, "extra-nonce-00000", n, ORIGIN));
        assert!(!cloud.accept(&extra, n), "полный набор принял ещё один");
        // Когда прежние истекли, они вычищаются и место появляется.
        let later = n + 31;
        let fresh = token(&key, &full_claims("user_1-a", later + 30, "fresh-nonce-000000", later, ORIGIN));
        assert!(cloud.accept(&fresh, later));
        assert_eq!(cloud.seen.lock().unwrap().len(), 1, "истёкшие не вычищены");
    }

    #[test]
    fn malformed_encodings_are_refused() {
        let key = signing_key();
        let guard = cloud_guard(&key);
        let cloud = cloud_of(&guard);
        let n = now();
        let json = full_claims("user_1-a", n + 30, "1111111111111111", n, ORIGIN);
        // Байты после JSON, даже подписанные.
        assert!(!cloud.accept(&token(&key, &format!("{json}x")), n));
        assert!(!cloud.accept(&token(&key, &format!("{json}{json}")), n));
        // Подпись по одному, а предъявлен другой payload.
        let good = token(&key, &json);
        let (payload, sig) = good.split_once('.').unwrap();
        let altered = full_claims("user_1-a", n + 30, "2222222222222222", n, ORIGIN);
        assert!(!cloud.accept(&format!("{}.{sig}", B64.encode(altered)), n));
        // Неканоническая base64: с паддингом и с «лишними» битами в хвосте.
        assert!(!cloud.accept(&format!("{payload}=.{sig}"), n));
        assert!(!cloud.accept(&format!("{payload}.{sig}="), n));
        let mut chars: Vec<char> = sig.chars().collect();
        // Последний символ 86-символьной подписи несёт 4 значащих бита: меняем незначащие.
        let last = chars.pop().unwrap();
        let alphabet = "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";
        let idx = alphabet.find(last).unwrap();
        chars.push(alphabet.chars().nth((idx ^ 1) % 64).unwrap());
        let tweaked: String = chars.into_iter().collect();
        assert!(!cloud.accept(&format!("{payload}.{tweaked}"), n));
        // Стандартный алфавит вместо url-safe.
        assert!(!cloud.accept(&format!("{}+/.{sig}", payload), n));
        // Сам хороший токен проходит.
        assert!(cloud.accept(&good, n));
    }

    fn peer(ip: [u8; 4]) -> std::net::IpAddr {
        std::net::IpAddr::from(ip)
    }

    fn agent_headers(host: &str) -> HeaderMap {
        let mut h = headers(host);
        h.insert(header::AUTHORIZATION, "Bearer agent".parse().unwrap());
        h
    }

    #[test]
    fn cloud_refuses_the_agent_key_on_requests_that_carry_proxy_headers() {
        let key = signing_key();
        let cloud = cloud_guard(&key);
        let plain = Guard::new(8899, "boot".into(), "agent".into());
        // Instabox ходит на контейнер с loopback и Host 127.0.0.1:<порт>, добавляя заголовки прокси.
        for name in ["x-forwarded-for", "x-forwarded-host", "x-forwarded-proto", "forwarded", "x-real-ip"] {
            let mut h = agent_headers("127.0.0.1:8899");
            h.insert(name, "203.0.113.9".parse().unwrap());
            let r = verdict(&cloud, peer([127, 0, 0, 1]), &Method::GET, "/api/state", None, &h).expect(name);
            assert_eq!(r.status(), StatusCode::FORBIDDEN, "{name}");
            // На ПК (не облако) локальные инструменты за своим прокси не ломаются.
            assert!(verdict(&plain, peer([127, 0, 0, 1]), &Method::GET, "/api/state", None, &h).is_none(), "{name} на ПК");
        }
        // Без заголовков прокси из того же loopback ключ работает.
        assert!(verdict(&cloud, peer([127, 0, 0, 1]), &Method::GET, "/api/state", None, &agent_headers("127.0.0.1:8899")).is_none());
    }

    #[test]
    fn agent_key_works_only_from_loopback_peer_and_loopback_host() {
        let key = signing_key();
        let cloud = cloud_guard(&key);
        let plain = Guard::new(8899, "boot".into(), "agent".into());
        for guard in [&cloud, &plain] {
            // Свой компьютер и свой адрес: можно.
            assert!(verdict(guard, peer([127, 0, 0, 1]), &Method::GET, "/api/state", None, &agent_headers("127.0.0.1:8899")).is_none());
            assert!(verdict(guard, peer([127, 0, 0, 1]), &Method::GET, "/api/state", None, &agent_headers("localhost:8899")).is_none());
            // Не loopback-сосед по сети: нельзя.
            for ip in [[10, 0, 0, 5], [172, 17, 0, 2], [8, 8, 8, 8]] {
                let r = verdict(guard, peer(ip), &Method::GET, "/api/state", None, &agent_headers("127.0.0.1:8899")).expect("пропущен");
                assert_eq!(r.status(), StatusCode::FORBIDDEN, "{ip:?}");
            }
            // Решения человека агентскому ключу закрыты и в облаке.
            for path in ["/api/approvals/x", "/api/vault", "/api/user-chrome", "/api/connectors"] {
                let r = verdict(guard, peer([127, 0, 0, 1]), &Method::POST, path, None, &agent_headers("127.0.0.1:8899")).expect("пропущен");
                assert_eq!(r.status(), StatusCode::FORBIDDEN, "{path}");
            }
        }
        // Облако: ключ через публичный Host не действует, даже с loopback-пира (контейнер за прокси).
        let r = verdict(&cloud, peer([127, 0, 0, 1]), &Method::GET, "/api/state", None, &agent_headers("u-test.app.example.org")).expect("пропущен");
        assert_eq!(r.status(), StatusCode::FORBIDDEN);
        let r = verdict(&cloud, peer([10, 0, 0, 5]), &Method::GET, "/api/state", None, &agent_headers("u-test.app.example.org")).expect("пропущен");
        assert_eq!(r.status(), StatusCode::FORBIDDEN);
    }

    #[test]
    fn websocket_upgrade_with_foreign_origin_is_refused() {
        let key = signing_key();
        let guard = cloud_guard(&key);
        let t = token(&key, &claims("user_1-a", now() + 30, "wwwwwwwwwwwwwwww"));
        let session = gateway(&guard, &t).unwrap().headers()[header::SET_COOKIE].to_str().unwrap().split(';').next().unwrap().to_string();
        let upgrade = |origin: &str| {
            let mut h = headers("u-test.app.example.org");
            h.insert(header::COOKIE, session.parse().unwrap());
            h.insert(header::CONNECTION, "Upgrade".parse().unwrap());
            h.insert(header::UPGRADE, "websocket".parse().unwrap());
            h.insert(header::ORIGIN, origin.parse().unwrap());
            check_as(&guard, &Method::GET, "/api/events", None, &h)
        };
        assert!(upgrade(ORIGIN).is_none());
        for origin in ["https://evil.example", "http://u-test.app.example.org", "null"] {
            assert!(upgrade(origin).is_some_and(|r| r.status() == StatusCode::FORBIDDEN), "{origin}");
        }
    }

    #[test]
    fn cloud_cookie_is_host_prefixed() {
        let key = signing_key();
        let guard = cloud_guard(&key);
        let t = token(&key, &claims("user_1-a", now() + 30, "cookiecookiecookie"));
        let response = gateway(&guard, &t).unwrap();
        let cookie = response.headers()[header::SET_COOKIE].to_str().unwrap().to_string();
        assert!(cookie.starts_with("__Host-crew_session="), "{cookie}");
        assert!(cookie.contains("Secure") && cookie.contains("Path=/") && !cookie.contains("Domain"), "{cookie}");
        let plain = Guard::new(8899, "boot".into(), "agent".into());
        assert_eq!(plain.cookie_name, "crew_session_8899");
    }

    #[test]
    fn weak_key_and_explicit_443_are_refused_at_startup() {
        let key = signing_key();
        // Единичная точка: ключ малого порядка.
        let identity = format!("01{}", "00".repeat(31));
        let weak = CloudConfig::parse(|name| match name {
            "CREW_CLOUD" => Some("1".into()),
            "CREW_GATEWAY_PUBKEY" => Some(identity.clone()),
            "CREW_USER_ID" => Some("u1".into()),
            "CREW_PUBLIC_ORIGIN" => Some(ORIGIN.into()),
            _ => None,
        });
        assert!(weak.is_err());
        let e = |origin: &'static str| CloudConfig::parse(env(&key, &[("CREW_PUBLIC_ORIGIN", origin)]));
        assert!(e("https://u-test.app.example.org:443").is_err());
        assert!(e("https://u-test.app.example.org:8443").is_ok());
    }

    #[test]
    fn cloud_policy_closes_console_install_and_desktop_routes() {
        for path in [
            "/api/vault", "/api/vault/logins", "/api/selfdev/clone", "/api/browser/obscura/install", "/api/browser/moli/install",
            "/api/providers/x/install", "/api/providers/scan", "/api/providers/x/check", "/api/providers/x/connect", "/api/updates/app", "/api/updates/restart",
            "/api/updates/tools/x", "/api/voice/prepare", "/api/voice/local/x/download", "/api/user-chrome/connect", "/api/selection/current",
            "/api/hotkeys", "/api/dock/new-bot", "/api/shortcut", "/api/bots/x/screen", "/api/bots/x/screen/input", "/api/bots/x/pet-place", "/api/bots/x/file/open-in-vscode",
            "/api/import/hermes", "/api/know-me/read", "/api/voice/settings",
        ] {
            assert!(cloud_closed(&normalize_path(path)), "{path} открыт в облаке");
        }
        for path in ["/api/state", "/api/bots/x/messages", "/api/browser/moli", "/api/mcp", "/api/providers", "/api/providers/x", "/api/bots/x/file", "/api/voice/transcribe"] {
            assert!(!cloud_closed(&normalize_path(path)), "{path} закрыт зря");
        }
        // Регистр и повторные слэши не обходят проверку.
        assert!(cloud_closed(&normalize_path("//API/Vault")));
    }
}
