//! Облако на бесплатной машине Instabox: вход в аккаунт, создание машины, запуск контейнера
//! `deloist-server`, состояние, пропуск шлюза, обновление и удаление.
//! Замысел и договор API — `notes/instabox-hosting.md`; договор со шлюзом — `notes/web-version.md`.
//!
//! Формы запросов и ответов сверены с официальной документацией (16.11.2026):
//! - вход агента: <https://instabox.sh/docs/security/agent-sign-up.md> (`POST /v1/auth/agent`,
//!   `POST /v1/auth/agent/token` с `authorization_pending` / `slow_down` / `expired_token` / `access_denied`);
//! - создание, операции, удаление: <https://instabox.sh/docs/api-reference/introduction.md>
//!   (`Idempotency-Key`, `X-Confirm-Delete`, `GET /v1/operations/{id}?wait=25s`, ошибки `{error:{code,..}}`);
//! - команда: `POST /v1/machines/{id}/exec` (`argv`, `stdin`; <https://instabox.sh/docs/api-reference/openapi.yaml>);
//! - порт: <https://instabox.sh/docs/access/ports.md> (`POST /v1/machines/{id}/ports` → `url`);
//! - сон, пробуждение, перерыв: <https://instabox.sh/docs/machines/auto-pause.md> (`409 machine_cooldown`, `Retry-After`).
//!
//! Защита (чек-лист в заметке):
//! - ключ `ibk_…`, закрытый ключ шлюза и `CREW_SECRETS_KEY` живут только в хранилище секретов ПК
//!   (`KeyStore`, область `instabox:cloud`). В ответы API, журнал и ошибки они не попадают: всё, что
//!   приходит снаружи и идёт в текст, проходит `scrub`;
//! - адрес API зашит (`PROD_BASE`); подмена — только в отладочной сборке и только на loopback (`base_url`);
//! - команды на машине идут как `argv`, пользовательский текст в shell не склеивается; имя машины и
//!   тег образа проверяются по белому списку (`valid_name`, `valid_tag`).

use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use base64::Engine;
use ed25519_dalek::{Signer, SigningKey};
use rand::RngCore;
use serde_json::{json, Value};

pub const PROD_BASE: &str = "https://api.instabox.sh";
/// Образ сервера. Репозиторий зашит; тег выбирает `default_tag` или человек (`valid_tag`).
const IMAGE_REPO: &str = "ghcr.io/gordidima-rgb/deloist-server";
const TEMPLATE: &str = "agent-browser";
const DEFAULT_NAME: &str = "deloist";
const CONTAINER: &str = "deloist";
const PORT: u16 = 8080;
/// Каталоги на постоянном диске машины `/data` (проверено на живой машине 16.11.2026). Exec идёт от
/// пользователя `user` (uid 1000, группы sudo и docker, sudo без пароля), контейнер — от uid 10001.
const DATA_DIR: &str = "/data/deloist/data";
const ENV_PATH: &str = "/data/deloist/cloud.env";
/// Контейнер пишет в каталог данных как uid 10001, иначе «Permission denied» на блокировке.
const CONTAINER_UID: &str = "10001:10001";
/// Область в хранилище секретов ПК.
pub const SCOPE: &str = "instabox:cloud";
const GATEWAY_DOMAIN: &[u8] = b"deloist-gw-v1\n";
/// Что просим у человека: машины и больше ничего. Ключи и оплата агентскому ключу недоступны.
const SCOPES: [&str; 3] = ["machines:read", "machines:write", "machines:delete"];

// --- Хранилище ключей -----------------------------------------------------------------

/// Секреты этого модуля. На ПК — область `instabox:cloud` в `Secrets` (DPAPI), в тестах — память.
pub trait KeyStore: Send + Sync {
    fn get(&self, name: &str) -> Option<String>;
    fn set(&self, name: &str, value: &str) -> Result<(), String>;
    fn delete(&self, name: &str) -> Result<(), String>;
}

/// Хранилище поверх `Secrets` программы.
pub struct SecretsStore(pub Arc<crate::App>);

impl KeyStore for SecretsStore {
    fn get(&self, name: &str) -> Option<String> {
        self.0.connectors.secrets.get(SCOPE, name)
    }
    fn set(&self, name: &str, value: &str) -> Result<(), String> {
        self.0.connectors.secrets.set(SCOPE, name, value)
    }
    fn delete(&self, name: &str) -> Result<(), String> {
        self.0.connectors.secrets.delete(SCOPE, name)
    }
}

const K_API: &str = "api_key";
const K_LOGIN: &str = "login";
const K_MACHINE: &str = "machine_id";
const K_NAME: &str = "machine_name";
const K_ORIGIN: &str = "origin";
const K_GATEWAY: &str = "gateway_key";
const K_SECRETS: &str = "secrets_key";
const K_UID: &str = "user_id";
const K_IMAGE: &str = "image";

// --- Проверки и вспомогательное --------------------------------------------------------

/// Имя машины: строчные латиница и цифры, дефис внутри, до 32 символов.
pub fn valid_name(name: &str) -> bool {
    let b = name.as_bytes();
    (1..=32).contains(&b.len())
        && b[0].is_ascii_alphanumeric()
        && !name.ends_with('-')
        && b.iter().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || *c == b'-')
}

/// Тег образа Docker: буквы, цифры, `_ . -`, не с точки и дефиса, до 128 символов.
pub fn valid_tag(tag: &str) -> bool {
    let b = tag.as_bytes();
    (1..=128).contains(&b.len()) && (b[0].is_ascii_alphanumeric() || b[0] == b'_') && b.iter().all(|c| c.is_ascii_alphanumeric() || b"_.-".contains(c))
}

/// В GHCR сейчас лежит только тег `stand` (16.11.2026): ни `main`, ни `v<версия>` CI не выкладывает.
/// Пока так, по умолчанию `stand`; нужный тег можно передать в `provision` и `update`.
const DEFAULT_TAG: &str = "stand";
/// Тег образа по умолчанию.
pub fn default_tag() -> String {
    DEFAULT_TAG.to_string()
}

/// Адрес API Instabox. Зашит. Отладочная сборка принимает `DELOIST_INSTABOX_BASE_URL`, но только
/// `http://127.0.0.1:…` или `http://localhost:…` (поддельный сервер в тестах). В выпуске строки нет вовсе.
pub fn base_url() -> String {
    #[cfg(debug_assertions)]
    if let Ok(value) = std::env::var("DELOIST_INSTABOX_BASE_URL") {
        let value = value.trim().trim_end_matches('/').to_string();
        if value.starts_with("http://127.0.0.1:") || value.starts_with("http://localhost:") {
            return value;
        }
    }
    PROD_BASE.to_string()
}

fn now_secs() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0)
}

fn rand_hex(bytes: usize) -> String {
    let mut buf = vec![0u8; bytes];
    rand::thread_rng().fill_bytes(&mut buf);
    crate::store::hex(&buf)
}

/// Всё, что похоже на секрет, заменяется. Применяется к любому тексту из сети, который может уйти человеку.
pub fn scrub(text: &str) -> String {
    fn secretish(token: &str) -> bool {
        let long_prefixed = token.len() >= 12 && ["ibk_", "ibg_", "iba_", "ibp_"].iter().any(|p| token.starts_with(p));
        let hex = token.len() >= 32 && token.bytes().all(|b| b.is_ascii_hexdigit());
        long_prefixed || hex || token.len() >= 48
    }
    let mut out = String::with_capacity(text.len());
    let mut token = String::new();
    let flush = |token: &mut String, out: &mut String| {
        if !token.is_empty() {
            out.push_str(if secretish(token) { "[скрыто]" } else { token });
            token.clear();
        }
    };
    for ch in text.chars() {
        if ch.is_ascii_alphanumeric() || ch == '_' {
            token.push(ch);
        } else {
            flush(&mut token, &mut out);
            out.push(ch);
        }
    }
    flush(&mut token, &mut out);
    out
}

/// Ошибка вызова Instabox: ничего секретного в тексте нет (`scrub`).
#[derive(Debug, Clone)]
pub struct Fail {
    pub status: u16,
    pub code: String,
    pub message: String,
    pub retry_after: Option<u64>,
}

impl Fail {
    fn local(status: u16, message: impl Into<String>) -> Self {
        Self { status, code: "local".into(), message: message.into(), retry_after: None }
    }
    fn network(e: &reqwest::Error) -> Self {
        let what = if e.is_timeout() { "время ожидания вышло" } else if e.is_connect() { "нет соединения" } else { "сбой сети" };
        Self { status: 0, code: "unreachable".into(), message: format!("Instabox недоступен: {what}"), retry_after: None }
    }
    pub fn text(&self) -> String {
        match self.code.as_str() {
            "local" | "unreachable" => self.message.clone(),
            code => format!("Instabox: {} ({code})", self.message),
        }
    }
}

/// Тело ответа с ошибкой → `Fail`. Сообщение берётся из `error.message`, секреты вычищаются.
fn fail_from(status: u16, body: &Value, header_retry: Option<u64>) -> Fail {
    let err = body.get("error").filter(|e| e.is_object());
    let code = err.and_then(|e| e.get("code")).and_then(Value::as_str).unwrap_or("error");
    let code: String = code.chars().filter(|c| c.is_ascii_lowercase() || *c == '_').take(48).collect();
    let message = err.and_then(|e| e.get("message")).and_then(Value::as_str).unwrap_or("запрос отклонён");
    let retry_after = err.and_then(|e| e.get("retryAfterSeconds")).and_then(Value::as_u64).or(header_retry);
    Fail { status, code, message: scrub(&message.chars().take(300).collect::<String>()), retry_after }
}

struct Reply {
    body: Value,
}

// --- Клиент -----------------------------------------------------------------------------

/// Всё, что нужно вызову: хранилище, адрес и сеть. Дёшево клонируется.
#[derive(Clone)]
pub struct Ctx {
    store: Arc<dyn KeyStore>,
    base: String,
    http: reqwest::Client,
    /// Секунда в опросах. 1 с в программе; тесты ставят миллисекунду.
    unit: Duration,
    /// Сколько ждать операцию, секунд (в единицах `unit`).
    op_budget: u64,
    health_budget: u64,
}

impl Ctx {
    pub fn new(store: Arc<dyn KeyStore>) -> Self {
        Self::with_base(store, base_url())
    }

    pub fn with_base(store: Arc<dyn KeyStore>, base: String) -> Self {
        let http = reqwest::Client::builder()
            .timeout(Duration::from_secs(40))
            .redirect(reqwest::redirect::Policy::none())
            .user_agent(concat!("deloist/", env!("CARGO_PKG_VERSION")))
            .build()
            .unwrap_or_default();
        Self { store, base, http, unit: Duration::from_secs(1), op_budget: 900, health_budget: 90 }
    }

    #[cfg(test)]
    fn fast(mut self) -> Self {
        self.unit = Duration::from_millis(2);
        self.op_budget = 3000;
        self.health_budget = 3000;
        self
    }

    fn production(&self) -> bool {
        self.base == PROD_BASE
    }

    fn api_key(&self) -> Result<String, Fail> {
        self.store.get(K_API).filter(|k| !k.is_empty()).ok_or_else(|| Fail::local(401, "сначала войдите в Instabox"))
    }

    async fn sleep(&self, units: u64) {
        tokio::time::sleep(self.unit * units.min(u32::MAX as u64) as u32).await;
    }

    async fn send(&self, req: reqwest::RequestBuilder) -> Result<(u16, Value, Option<u64>), Fail> {
        let res = req.send().await.map_err(|e| Fail::network(&e))?;
        let status = res.status().as_u16();
        let retry = res.headers().get("retry-after").and_then(|v| v.to_str().ok()).and_then(|v| v.trim().parse::<u64>().ok());
        let bytes = res.bytes().await.map_err(|e| Fail::network(&e))?;
        let body = if bytes.iter().all(|b| b.is_ascii_whitespace()) { Value::Null } else { serde_json::from_slice(&bytes).unwrap_or(Value::Null) };
        Ok((status, body, retry))
    }

    /// Вызов с ключом. POST и DELETE получают свой `Idempotency-Key`, если `idem`.
    async fn call(&self, method: reqwest::Method, path: &str, body: Option<Value>, idem: bool) -> Result<Reply, Fail> {
        let key = self.api_key()?;
        let mut req = self.http.request(method, format!("{}{path}", self.base)).header("x-api-key", key);
        if idem {
            req = req.header("idempotency-key", rand_hex(16));
        }
        if let Some(body) = body {
            req = req.json(&body);
        }
        self.finish(req).await
    }

    async fn finish(&self, req: reqwest::RequestBuilder) -> Result<Reply, Fail> {
        let (status, body, retry) = self.send(req).await?;
        if (200..300).contains(&status) {
            return Ok(Reply { body });
        }
        let mut fail = fail_from(status, &body, retry);
        if status == 401 {
            fail.message = "ключ Instabox не принят: войдите заново".into();
        }
        Err(fail)
    }

    /// Операция: ждём до `succeeded` или `failed` (`GET /v1/operations/{id}?wait=25s`).
    async fn wait_operation(&self, id: &str) -> Result<Value, Fail> {
        if id.is_empty() || !id.bytes().all(|b| b.is_ascii_alphanumeric() || b"_-".contains(&b)) {
            return Err(Fail::local(502, "Instabox вернул странный номер операции"));
        }
        let mut waited = 0u64;
        loop {
            let reply = self.call(reqwest::Method::GET, &format!("/v1/operations/{id}?wait=25s"), None, false).await?;
            match reply.body.get("state").and_then(Value::as_str) {
                Some("succeeded") => return Ok(reply.body),
                Some("failed") => {
                    let mut fail = fail_from(409, &json!({ "error": reply.body.get("error").cloned().unwrap_or(Value::Null) }), None);
                    if fail.code == "error" {
                        fail.message = "операция не удалась".into();
                    }
                    return Err(fail);
                }
                _ => {}
            }
            waited += 1;
            if waited > self.op_budget {
                return Err(Fail::local(504, "Instabox слишком долго выполняет операцию"));
            }
            // Длинный опрос держит запрос сам; пауза нужна только если сервер ответил сразу.
            self.sleep(1).await;
        }
    }

    async fn get_machine(&self, id: &str) -> Result<Value, Fail> {
        Ok(self.call(reqwest::Method::GET, &format!("/v1/machines/{id}"), None, false).await?.body)
    }

    /// Команда без shell: `argv`. Ответ — `ExecResult`; ненулевой код возврата — не ошибка вызова.
    async fn exec(&self, machine: &str, argv: &[&str], stdin: Option<&str>, timeout_s: u64) -> Result<ExecOut, Fail> {
        let key = self.api_key()?;
        let mut body = json!({ "argv": argv, "timeoutSeconds": timeout_s.clamp(1, 600) });
        if let Some(stdin) = stdin {
            body["stdin"] = json!(stdin);
        }
        let req = self
            .http
            .post(format!("{}/v1/machines/{machine}/exec", self.base))
            .header("x-api-key", key)
            .header("x-instabox-exec-keepalive", "1")
            .timeout(Duration::from_secs(timeout_s.clamp(1, 600) + 60))
            .json(&body);
        let (status, value, retry) = self.send(req).await?;
        if !(200..300).contains(&status) {
            return Err(fail_from(status, &value, retry));
        }
        // Ошибка после ответа 200 приходит телом `{"error": …, "status": N}`.
        if value.get("exitCode").is_none() {
            return Err(fail_from(value.get("status").and_then(Value::as_u64).unwrap_or(502) as u16, &value, None));
        }
        Ok(ExecOut {
            code: value.get("exitCode").and_then(Value::as_i64).unwrap_or(-1),
            stderr: scrub(value.get("stderr").and_then(Value::as_str).unwrap_or("")),
        })
    }
}

struct ExecOut {
    code: i64,
    stderr: String,
}

// --- Состояние в памяти ---------------------------------------------------------------

#[derive(Default)]
struct State {
    signin: Option<Signin>,
    signin_gen: u64,
    job: Option<Job>,
    job_gen: u64,
    cache: Option<(Instant, Value)>,
}

struct Signin {
    /// `pending` | `approved` | `denied` | `expired` | `error`.
    phase: &'static str,
    user_code: String,
    uri: String,
    uri_complete: String,
    expires_at: i64,
    error: Option<String>,
}

struct Job {
    kind: &'static str,
    steps: &'static [(&'static str, &'static str)],
    step: usize,
    /// `running` | `done` | `error`.
    state: &'static str,
    error: Option<String>,
    retry_after: Option<u64>,
    started: i64,
}

const PROVISION_STEPS: &[(&str, &str)] = &[
    ("machine", "Создаю машину"),
    ("port", "Открываю адрес"),
    ("keys", "Готовлю ключи"),
    ("files", "Записываю настройки"),
    ("docker", "Запускаю Deloist"),
    ("health", "Жду ответа"),
];
const UPDATE_STEPS: &[(&str, &str)] = &[("wake", "Бужу машину"), ("files", "Записываю настройки"), ("docker", "Обновляю образ"), ("health", "Жду ответа")];
const DELETE_STEPS: &[(&str, &str)] = &[("delete", "Удаляю машину")];

#[derive(Clone, Default)]
pub struct Hub {
    st: Arc<Mutex<State>>,
}

pub struct JobFail {
    pub status: u16,
    pub message: String,
}

impl Hub {
    // --- вход ---

    /// Начало входа. Возвращает данные для человека и запускает опрос в фоне.
    /// Секрет ожидания (`claimToken`) остаётся в задаче опроса и наружу не уходит.
    pub async fn signin_start(&self, ctx: Ctx) -> Result<Value, Fail> {
        {
            let st = self.st.lock().unwrap();
            if let Some(s) = st.signin.as_ref().filter(|s| s.phase == "pending" && s.expires_at > now_secs()) {
                return Ok(signin_view(s));
            }
        }
        let (status, body, retry) = ctx
            .send(ctx.http.post(format!("{}/v1/auth/agent", ctx.base)).json(&json!({ "name": DEFAULT_NAME, "scopes": SCOPES })))
            .await?;
        if status != 200 {
            return Err(fail_from(status, &body, retry));
        }
        let text = |k: &str| body.get(k).and_then(Value::as_str).unwrap_or("").to_string();
        let (claim, user_code, uri, uri_complete) = (text("claimToken"), text("userCode"), text("verificationUri"), text("verificationUriComplete"));
        let code_ok = (1..=32).contains(&user_code.len()) && user_code.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-');
        if claim.is_empty() || !code_ok || !trusted_link(&ctx, &uri_complete) || !trusted_link(&ctx, &uri) {
            return Err(Fail::local(502, "Instabox вернул неожиданный ответ при входе"));
        }
        let interval = body.get("interval").and_then(Value::as_u64).unwrap_or(5).clamp(1, 60);
        let expires_in = body.get("expiresIn").and_then(Value::as_i64).unwrap_or(900).clamp(60, 86_400);
        let gen = {
            let mut st = self.st.lock().unwrap();
            st.signin_gen += 1;
            st.signin = Some(Signin { phase: "pending", user_code, uri, uri_complete, expires_at: now_secs() + expires_in, error: None });
            st.signin_gen
        };
        let view = signin_view(self.st.lock().unwrap().signin.as_ref().unwrap());
        let hub = self.clone();
        tokio::spawn(async move { hub.poll_signin(ctx, claim, interval, expires_in as u64, gen).await });
        Ok(view)
    }

    async fn poll_signin(&self, ctx: Ctx, claim: String, mut interval: u64, expires_in: u64, gen: u64) {
        let mut spent = 0u64;
        let set = |phase: &'static str, error: Option<String>| {
            let mut st = self.st.lock().unwrap();
            if st.signin_gen == gen {
                if let Some(s) = st.signin.as_mut() {
                    s.phase = phase;
                    s.error = error;
                }
            }
        };
        loop {
            ctx.sleep(interval).await;
            spent += interval;
            if self.st.lock().unwrap().signin_gen != gen {
                return; // начат новый вход
            }
            if spent > expires_in + 30 {
                return set("expired", None);
            }
            let sent = ctx.send(ctx.http.post(format!("{}/v1/auth/agent/token", ctx.base)).json(&json!({ "claimToken": claim }))).await;
            let (status, body, _) = match sent {
                Ok(r) => r,
                Err(_) => continue, // сеть моргнула: попробуем в следующий раз
            };
            match status {
                200 => {
                    let key = body.get("key").and_then(Value::as_str).unwrap_or("");
                    if !key.starts_with("ibk_") {
                        return set("error", Some("Instabox прислал странный ключ".into()));
                    }
                    let login = body.get("login").and_then(Value::as_str).unwrap_or("").chars().take(64).collect::<String>();
                    if let Err(e) = ctx.store.set(K_API, key).and_then(|_| ctx.store.set(K_LOGIN, &login)) {
                        return set("error", Some(scrub(&e)));
                    }
                    // Ключ у нас: сообщаем серверу, чтобы он стёр свою запечатанную копию.
                    let _ = ctx.send(ctx.http.post(format!("{}/v1/auth/agent/token", ctx.base)).json(&json!({ "claimToken": claim, "ack": true }))).await;
                    self.st.lock().unwrap().cache = None;
                    return set("approved", None);
                }
                204 => {}
                _ => match body.pointer("/error/code").and_then(Value::as_str).or_else(|| body.get("error").and_then(Value::as_str)).unwrap_or("") {
                    "authorization_pending" => {}
                    "slow_down" => interval += 5,
                    "expired_token" => return set("expired", None),
                    "access_denied" => return set("denied", None),
                    _ if status == 429 || status >= 500 => {}
                    _ => return set("error", Some("Instabox отклонил вход".into())),
                },
            }
        }
    }

    pub fn signin_status(&self, ctx: &Ctx) -> Value {
        let st = self.st.lock().unwrap();
        let signed_in = ctx.store.get(K_API).is_some();
        match st.signin.as_ref() {
            Some(s) => {
                let mut view = signin_view(s);
                view["signedIn"] = json!(signed_in);
                view
            }
            None => json!({ "phase": "none", "signedIn": signed_in }),
        }
    }

    // --- состояние ---

    /// Один снимок для окна: вход, машина, текущая работа. Машину не будит (`GET /v1/machines/{id}`).
    pub async fn status(&self, ctx: &Ctx) -> Value {
        let signed_in = ctx.store.get(K_API).is_some();
        let machine = match (signed_in, ctx.store.get(K_MACHINE)) {
            (true, Some(id)) => Some(self.machine_view(ctx, &id).await),
            _ => None,
        };
        let st = self.st.lock().unwrap();
        json!({
            "signedIn": signed_in,
            "login": ctx.store.get(K_LOGIN).unwrap_or_default(),
            "machine": machine,
            "job": st.job.as_ref().map(job_view),
            "signin": st.signin.as_ref().map(signin_view),
            "image": ctx.store.get(K_IMAGE),
            "defaultTag": default_tag(),
        })
    }

    async fn machine_view(&self, ctx: &Ctx, id: &str) -> Value {
        {
            let st = self.st.lock().unwrap();
            if let Some((at, v)) = st.cache.as_ref() {
                if at.elapsed() < Duration::from_secs(3) && st.job.as_ref().is_none_or(|j| j.state != "running") {
                    return v.clone();
                }
            }
        }
        let origin = ctx.store.get(K_ORIGIN);
        let view = match ctx.get_machine(id).await {
            Ok(m) => {
                let (state, retry) = classify(&m);
                json!({ "state": state, "ready": m.pointer("/runtime/ready").and_then(Value::as_bool).unwrap_or(false), "retryAfterSeconds": retry, "cooldownUntil": m.get("cooldownUntil").cloned().filter(|v| v.is_string()), "origin": origin, "name": ctx.store.get(K_NAME) })
            }
            Err(f) if f.status == 404 => json!({ "state": "missing", "origin": origin, "name": ctx.store.get(K_NAME) }),
            Err(f) if f.status == 401 => json!({ "state": "signin_needed", "origin": origin, "error": f.text() }),
            Err(f) => json!({ "state": "unreachable", "retryAfterSeconds": f.retry_after, "origin": origin, "name": ctx.store.get(K_NAME), "error": f.text() }),
        };
        self.st.lock().unwrap().cache = Some((Instant::now(), view.clone()));
        view
    }

    // --- «Открыть облако» ---

    /// Пропуск шлюза на 30 с, если машина работает. Спит — будим в фоне и просим повторить.
    pub async fn open(&self, ctx: Ctx) -> Result<Value, Fail> {
        let (id, origin, gw) = deployed(&ctx)?;
        let machine = ctx.get_machine(&id).await?;
        let (state, retry) = classify(&machine);
        if state != "running" {
            if state == "paused" {
                let (hub, ctx2) = (self.clone(), ctx.clone());
                if self.begin_job("wake", UPDATE_STEPS).is_some() {
                    let id2 = id.clone();
                    tokio::spawn(async move { hub.run_wake(ctx2, id2).await });
                }
                return Ok(json!({ "ok": false, "state": "waking", "retryAfterSeconds": 5 }));
            }
            return Ok(json!({ "ok": false, "state": state, "retryAfterSeconds": retry }));
        }
        match ctx.call(reqwest::Method::POST, &format!("/v1/machines/{id}/wake"), Some(json!({})), false).await {
            Ok(_) => {}
            Err(f) if f.code == "machine_cooldown" => return Ok(json!({ "ok": false, "state": "cooldown", "retryAfterSeconds": f.retry_after })),
            Err(f) => return Err(f),
        }
        let url = gateway_url(&gw, &ctx.store.get(K_UID).unwrap_or_default(), &origin)?;
        Ok(json!({ "ok": true, "state": "running", "url": url }))
    }

    async fn run_wake(&self, ctx: Ctx, id: String) {
        let result = ensure_running(&ctx, &id).await;
        self.finish_job("wake", result);
    }

    // --- работы в фоне ---

    fn begin_job(&self, kind: &'static str, steps: &'static [(&'static str, &'static str)]) -> Option<u64> {
        let mut st = self.st.lock().unwrap();
        if st.job.as_ref().is_some_and(|j| j.state == "running") {
            return None;
        }
        st.job_gen += 1;
        st.job = Some(Job { kind, steps, step: 0, state: "running", error: None, retry_after: None, started: now_secs() });
        st.cache = None;
        Some(st.job_gen)
    }

    fn set_step(&self, step: usize) {
        if let Some(job) = self.st.lock().unwrap().job.as_mut() {
            job.step = step;
        }
    }

    fn finish_job(&self, kind: &str, result: Result<(), Fail>) {
        let mut st = self.st.lock().unwrap();
        st.cache = None;
        if let Some(job) = st.job.as_mut().filter(|j| j.kind == kind && j.state == "running") {
            match result {
                Ok(()) => job.state = "done",
                Err(f) => {
                    job.state = "error";
                    job.error = Some(f.text());
                    job.retry_after = f.retry_after;
                }
            }
        }
    }

    pub fn provision(&self, ctx: Ctx, name: Option<&str>, tag: Option<&str>) -> Result<Value, JobFail> {
        let name = name.unwrap_or(DEFAULT_NAME).to_string();
        if !valid_name(&name) {
            return Err(JobFail { status: 400, message: "имя машины: строчные латинские буквы, цифры и дефис, до 32 символов".into() });
        }
        let tag = tag.map(str::to_string).unwrap_or_else(default_tag);
        if !valid_tag(&tag) {
            return Err(JobFail { status: 400, message: "тег образа: буквы, цифры и _ . -, до 128 символов".into() });
        }
        if ctx.api_key().is_err() {
            return Err(JobFail { status: 409, message: "сначала войдите в Instabox".into() });
        }
        if self.begin_job("provision", PROVISION_STEPS).is_none() {
            return Err(JobFail { status: 409, message: "другая работа с облаком ещё идёт".into() });
        }
        let hub = self.clone();
        tokio::spawn(async move {
            let result = hub.provision_run(&ctx, &name, &tag).await;
            hub.finish_job("provision", result);
        });
        Ok(json!({ "started": true, "kind": "provision" }))
    }

    async fn provision_run(&self, ctx: &Ctx, name: &str, tag: &str) -> Result<(), Fail> {
        // 1. Машина. Если id уже сохранён и машина жива — берём её, второй не создаём (на бесплатном тарифе одна).
        self.set_step(0);
        let mut machine_id = None;
        if let Some(id) = ctx.store.get(K_MACHINE) {
            match ctx.get_machine(&id).await {
                Ok(_) => machine_id = Some(id),
                Err(f) if f.status == 404 => {}
                Err(f) => return Err(f),
            }
        }
        let machine_id = match machine_id {
            Some(id) => id,
            None => {
                let reply = ctx
                    .call(reqwest::Method::POST, "/v1/machines", Some(json!({ "name": name, "templateId": TEMPLATE, "desiredState": "running" })), true)
                    .await?;
                let id = reply.body.get("machineId").and_then(Value::as_str).unwrap_or("").to_string();
                let op = reply.body.get("id").and_then(Value::as_str).unwrap_or("").to_string();
                if !valid_machine_id(&id) {
                    return Err(Fail::local(502, "Instabox вернул странный номер машины"));
                }
                // Номер — сразу в хранилище: если дальше что-то оборвётся, повтор возьмёт эту же машину.
                ctx.store.set(K_MACHINE, &id).and_then(|_| ctx.store.set(K_NAME, name)).map_err(|e| Fail::local(500, scrub(&e)))?;
                ctx.wait_operation(&op).await?;
                id
            }
        };
        ensure_running(ctx, &machine_id).await?;

        // 2. Публичный порт 8080. Адрес нужен до запуска: он идёт в CREW_PUBLIC_ORIGIN.
        self.set_step(1);
        let port = ctx.call(reqwest::Method::POST, &format!("/v1/machines/{machine_id}/ports"), Some(json!({ "port": PORT, "name": "Deloist", "access": "public" })), false).await?;
        let origin = normalize_origin(ctx, &machine_id, port.body.get("url").and_then(Value::as_str).unwrap_or(""))?;
        ctx.store.set(K_ORIGIN, &origin).map_err(|e| Fail::local(500, scrub(&e)))?;

        // 3. Ключи. Уже есть — не меняем: контейнер и пропуска должны совпадать.
        self.set_step(2);
        ensure_keys(ctx)?;
        ctx.store.set(K_IMAGE, &image_ref(tag)).map_err(|e| Fail::local(500, scrub(&e)))?;

        // 4–6. Файл настроек, контейнер, здоровье.
        self.deploy(ctx, &machine_id, 3).await
    }

    /// Шаги «файл → контейнер → здоровье». `first` — номер шага «файл» в списке.
    async fn deploy(&self, ctx: &Ctx, machine_id: &str, first: usize) -> Result<(), Fail> {
        self.set_step(first);
        let env = env_file(ctx, machine_id)?;
        prepare_files(ctx, machine_id, &env).await?;

        self.set_step(first + 1);
        let image = ctx.store.get(K_IMAGE).ok_or_else(|| Fail::local(409, "образ не выбран"))?;
        run_container(ctx, machine_id, &image).await?;

        self.set_step(first + 2);
        let origin = ctx.store.get(K_ORIGIN).and_then(|o| check_origin(ctx, machine_id, &o)).ok_or_else(|| Fail::local(409, "нет верного адреса облака"))?;
        wait_health(ctx, &origin).await
    }

    pub fn update(&self, ctx: Ctx, tag: Option<&str>) -> Result<Value, JobFail> {
        let tag = tag.map(str::to_string).unwrap_or_else(default_tag);
        if !valid_tag(&tag) {
            return Err(JobFail { status: 400, message: "тег образа: буквы, цифры и _ . -, до 128 символов".into() });
        }
        let Some(machine_id) = ctx.store.get(K_MACHINE).filter(|_| ctx.api_key().is_ok()) else {
            return Err(JobFail { status: 409, message: "облака ещё нет".into() });
        };
        if self.begin_job("update", UPDATE_STEPS).is_none() {
            return Err(JobFail { status: 409, message: "другая работа с облаком ещё идёт".into() });
        }
        let hub = self.clone();
        tokio::spawn(async move {
            let result = async {
                hub.set_step(0);
                ensure_running(&ctx, &machine_id).await?;
                ctx.store.set(K_IMAGE, &image_ref(&tag)).map_err(|e| Fail::local(500, scrub(&e)))?;
                hub.deploy(&ctx, &machine_id, 1).await
            }
            .await;
            hub.finish_job("update", result);
        });
        Ok(json!({ "started": true, "kind": "update" }))
    }

    /// Удаляет машину и забывает ключи. Без `forget_account` ключ Instabox остаётся (машину можно создать заново).
    pub fn delete(&self, ctx: Ctx, forget_account: bool) -> Result<Value, JobFail> {
        let Some(machine_id) = ctx.store.get(K_MACHINE) else {
            return Err(JobFail { status: 409, message: "облака нет".into() });
        };
        if ctx.api_key().is_err() {
            return Err(JobFail { status: 409, message: "сначала войдите в Instabox".into() });
        }
        if self.begin_job("delete", DELETE_STEPS).is_none() {
            return Err(JobFail { status: 409, message: "другая работа с облаком ещё идёт".into() });
        }
        let hub = self.clone();
        tokio::spawn(async move {
            let result = async {
                let key = ctx.api_key()?;
                let req = ctx
                    .http
                    .delete(format!("{}/v1/machines/{machine_id}", ctx.base))
                    .header("x-api-key", key)
                    .header("idempotency-key", rand_hex(16))
                    .header("x-confirm-delete", machine_id.as_str());
                let (status, body, retry) = ctx.send(req).await?;
                let missing = status == 404;
                if !missing {
                    if status != 202 && status != 200 {
                        return Err(fail_from(status, &body, retry));
                    }
                    if let Some(op) = body.get("id").and_then(Value::as_str) {
                        ctx.wait_operation(op).await?;
                    }
                }
                // Машины нет: ключи, которыми можно войти в неё, больше не нужны.
                let mut gone: Vec<&str> = vec![K_MACHINE, K_NAME, K_ORIGIN, K_GATEWAY, K_SECRETS, K_UID, K_IMAGE];
                if forget_account {
                    gone.extend([K_API, K_LOGIN]);
                }
                for name in gone {
                    ctx.store.delete(name).map_err(|e| Fail::local(500, scrub(&e)))?;
                }
                Ok(())
            }
            .await;
            hub.finish_job("delete", result);
        });
        Ok(json!({ "started": true, "kind": "delete" }))
    }
}

fn signin_view(s: &Signin) -> Value {
    json!({
        "phase": s.phase,
        "userCode": s.user_code,
        "verificationUri": s.uri,
        "verificationUriComplete": s.uri_complete,
        "expiresAt": s.expires_at,
        "error": s.error,
    })
}

fn job_view(job: &Job) -> Value {
    json!({
        "kind": job.kind,
        "state": job.state,
        "step": job.step,
        "steps": job.steps.iter().map(|(id, label)| json!({ "id": id, "label": label })).collect::<Vec<_>>(),
        "error": job.error,
        "retryAfterSeconds": job.retry_after,
        "startedAt": job.started,
    })
}

/// Ссылка для человека ведёт на сайт Instabox (или на поддельный сервер в тестах), и больше никуда.
fn trusted_link(ctx: &Ctx, link: &str) -> bool {
    let Ok(url) = reqwest::Url::parse(link) else { return false };
    if !url.username().is_empty() || url.password().is_some() {
        return false;
    }
    let host = url.host_str().unwrap_or("");
    if ctx.production() {
        url.scheme() == "https" && (host == "instabox.sh" || host.ends_with(".instabox.sh"))
    } else {
        reqwest::Url::parse(&ctx.base).ok().is_some_and(|b| b.host_str() == url.host_str() && b.port() == url.port())
    }
}

/// Состояние машины для окна и срок перерыва. `paused` — «спит».
fn classify(machine: &Value) -> (&'static str, Option<u64>) {
    let until = machine.get("cooldownUntil").and_then(Value::as_str).and_then(|t| chrono::DateTime::parse_from_rfc3339(t).ok()).map(|t| t.timestamp());
    if let Some(until) = until.filter(|u| *u > now_secs()) {
        return ("cooldown", Some((until - now_secs()) as u64));
    }
    let state = match machine.get("state").and_then(Value::as_str).unwrap_or("") {
        "running" => "running",
        "suspended" | "suspending" => "paused",
        "stopped" | "stopping" => "stopped",
        "creating" | "starting" | "resuming" => "starting",
        "deleting" | "deleted" => "missing",
        _ => "error",
    };
    (state, None)
}

fn valid_machine_id(id: &str) -> bool {
    (1..=64).contains(&id.len()) && id.bytes().all(|b| b.is_ascii_alphanumeric() || b"_-".contains(&b))
}

fn image_ref(tag: &str) -> String {
    format!("{IMAGE_REPO}:{tag}")
}

/// Адрес из ответа Instabox → origin для `CREW_PUBLIC_ORIGIN`: https, хост, без пути и порта 443, строчными.
fn normalize_origin(ctx: &Ctx, machine_id: &str, url: &str) -> Result<String, Fail> {
    check_origin(ctx, machine_id, url).ok_or_else(|| Fail::local(502, "Instabox вернул странный адрес порта"))
}

/// Адрес облака обязан быть ровно `https://8080-<id без m_>.instabox.tech` (в тестах — поддельный сервер).
/// Проверяется и при сохранении, и перед каждым пропуском, открытием и опросом здоровья.
fn check_origin(ctx: &Ctx, machine_id: &str, url: &str) -> Option<String> {
    if ctx.production() {
        let id = machine_id.strip_prefix("m_").filter(|id| valid_machine_id(id))?;
        let want = format!("https://{PORT}-{id}.instabox.tech");
        let trimmed = url.strip_suffix('/').unwrap_or(url);
        return (trimmed == want).then_some(want);
    }
    // Проверка на поддельном сервере: порт публикуется на нём же.
    let parsed = reqwest::Url::parse(url).ok()?;
    let base = reqwest::Url::parse(&ctx.base).ok()?;
    let plain = parsed.username().is_empty() && parsed.password().is_none() && parsed.query().is_none() && parsed.fragment().is_none() && parsed.path() == "/";
    (plain && base.host_str() == parsed.host_str() && base.port() == parsed.port()).then(|| url.trim_end_matches('/').to_ascii_lowercase())
}

fn deployed(ctx: &Ctx) -> Result<(String, String, String), Fail> {
    ctx.api_key()?;
    let get = |k: &str| ctx.store.get(k).filter(|v| !v.is_empty());
    match (get(K_MACHINE), get(K_ORIGIN), get(K_GATEWAY)) {
        (Some(m), Some(o), Some(g)) => {
            let o = check_origin(ctx, &m, &o).ok_or_else(|| Fail::local(409, "сохранённый адрес облака не прошёл проверку: создайте облако заново"))?;
            Ok((m, o, g))
        }
        _ => Err(Fail::local(409, "облако ещё не создано")),
    }
}

/// Ключи шлюза и шифрования секретов, id пользователя. Созданное не пересоздаётся.
fn ensure_keys(ctx: &Ctx) -> Result<(), Fail> {
    let put = |name: &str, make: &dyn Fn() -> String| -> Result<(), Fail> {
        if ctx.store.get(name).is_none() {
            ctx.store.set(name, &make()).map_err(|e| Fail::local(500, scrub(&e)))?;
        }
        Ok(())
    };
    put(K_GATEWAY, &|| rand_hex(32))?;
    put(K_SECRETS, &|| rand_hex(32))?;
    put(K_UID, &|| format!("u-{}", rand_hex(8)))
}

fn signing_key(gw_hex: &str) -> Result<SigningKey, Fail> {
    let bytes: Vec<u8> = (0..gw_hex.len().min(64)).step_by(2).filter_map(|i| u8::from_str_radix(gw_hex.get(i..i + 2)?, 16).ok()).collect();
    let seed: [u8; 32] = bytes.try_into().map_err(|_| Fail::local(500, "ключ шлюза повреждён"))?;
    Ok(SigningKey::from_bytes(&seed))
}

/// Содержимое env-файла контейнера. Только известные переменные, значения проверены на перевод строки.
fn env_file(ctx: &Ctx, machine_id: &str) -> Result<String, Fail> {
    let need = |k: &str| ctx.store.get(k).filter(|v| !v.is_empty() && !v.contains(['\n', '\r'])).ok_or_else(|| Fail::local(409, "не хватает данных облака: создайте его заново"));
    let pubkey = crate::store::hex(signing_key(&need(K_GATEWAY)?)?.verifying_key().as_bytes());
    Ok(format!(
        "CREW_CLOUD=1\nHOST=0.0.0.0\nPORT={PORT}\nCREW_GATEWAY_PUBKEY={pubkey}\nCREW_USER_ID={}\nCREW_PUBLIC_ORIGIN={}\nCREW_SECRETS_KEY={}\n",
        need(K_UID)?,
        check_origin(ctx, machine_id, &need(K_ORIGIN)?).ok_or_else(|| Fail::local(409, "адрес облака не прошёл проверку"))?,
        need(K_SECRETS)?
    ))
}

/// Пропуск шлюза на 30 с: `{uid, aud, iat, exp, nonce}`, подпись Ed25519 над `"deloist-gw-v1\n" + payload`
/// (как проверяет `guard.rs`, как выпускает `examples/gw_token`).
pub fn gateway_url(gw_hex: &str, uid: &str, origin: &str) -> Result<String, Fail> {
    let key = signing_key(gw_hex)?;
    let origin = origin.trim_end_matches('/').to_ascii_lowercase();
    let now = now_secs();
    let payload = json!({ "uid": uid, "aud": origin, "iat": now, "exp": now + 30, "nonce": rand_hex(16) }).to_string();
    let mut signed = GATEWAY_DOMAIN.to_vec();
    signed.extend_from_slice(payload.as_bytes());
    let b64 = base64::engine::general_purpose::URL_SAFE_NO_PAD;
    Ok(format!("{origin}/auth/gateway?t={}.{}", b64.encode(&payload), b64.encode(key.sign(&signed).to_bytes())))
}

/// Машина должна работать: спящую будим (`POST /wake`, ждёт до 45 с), остановленную запускаем.
async fn ensure_running(ctx: &Ctx, id: &str) -> Result<(), Fail> {
    for _ in 0..3 {
        let machine = ctx.get_machine(id).await?;
        let (state, retry) = classify(&machine);
        match state {
            "running" if machine.pointer("/runtime/ready").and_then(Value::as_bool).unwrap_or(true) => return Ok(()),
            "cooldown" => return Err(Fail { status: 409, code: "machine_cooldown".into(), message: "машина отдыхает после долгой работы".into(), retry_after: retry }),
            "paused" => {
                // Только resume: машина, приостановленная через API, на запросы к порту не просыпается, и suspend мы не зовём.
                match ctx.call(reqwest::Method::POST, &format!("/v1/machines/{id}/resume"), Some(json!({})), true).await {
                    Ok(reply) => {
                        if let Some(op) = reply.body.get("id").and_then(Value::as_str) {
                            ctx.wait_operation(op).await?;
                        }
                    }
                    Err(f) if f.code == "operation_conflict" => ctx.sleep(2).await,
                    Err(f) => return Err(f),
                }
            }
            "running" | "starting" => {
                match ctx.call(reqwest::Method::POST, &format!("/v1/machines/{id}/wake"), Some(json!({})), false).await {
                    Ok(_) => {}
                    Err(f) if f.code == "operation_conflict" => ctx.sleep(2).await,
                    Err(f) => return Err(f),
                }
                ctx.sleep(3).await;
            }
            "stopped" => {
                let reply = ctx.call(reqwest::Method::POST, &format!("/v1/machines/{id}/start"), Some(json!({})), true).await?;
                if let Some(op) = reply.body.get("id").and_then(Value::as_str) {
                    ctx.wait_operation(op).await?;
                }
            }
            "missing" => return Err(Fail::local(404, "машины в Instabox больше нет")),
            _ => return Err(Fail::local(502, "машина в ошибке: посмотрите её в кабинете Instabox")),
        }
    }
    Err(Fail::local(504, "машина не поднялась"))
}

/// Каталог данных (владелец uid контейнера, 0700) и env-файл (0600). Текст env идёт в `stdin`, а не в
/// командную строку; команды с фиксированным текстом, пользовательского ввода в них нет.
async fn prepare_files(ctx: &Ctx, id: &str, env: &str) -> Result<(), Fail> {
    let need = |what: &str, out: ExecOut| -> Result<(), Fail> {
        if out.code == 0 { Ok(()) } else { Err(Fail::local(502, format!("{what} не удалось (код {}): {}", out.code, out.stderr.trim().chars().take(200).collect::<String>()))) }
    };
    need("Каталог данных", ctx.exec(id, &["mkdir", "-p", DATA_DIR], None, 60).await?)?;
    need("Владелец каталога", ctx.exec(id, &["sudo", "-n", "chown", CONTAINER_UID, DATA_DIR], None, 60).await?)?;
    need("Права каталога", ctx.exec(id, &["sudo", "-n", "chmod", "700", DATA_DIR], None, 60).await?)?;
    let write = format!("umask 077; cat > {ENV_PATH}");
    need("Файл настроек", ctx.exec(id, &["sh", "-c", &write], Some(env), 60).await?)
}

/// Контейнер: `docker pull`, удаление старого, `docker run`. Всё через `argv`, без shell.
/// Если у пользователя `dev` нет прав на Docker, повторяем через `sudo -n` (проверяется живой машиной).
async fn run_container(ctx: &Ctx, id: &str, image: &str) -> Result<(), Fail> {
    if !image.strip_prefix(&format!("{IMAGE_REPO}:")).is_some_and(valid_tag) {
        return Err(Fail::local(400, "образ не из нашего списка"));
    }
    let probe = ctx.exec(id, &["docker", "info", "--format", "{{.ServerVersion}}"], None, 60).await?;
    let sudo = probe.code != 0 && probe.stderr.to_ascii_lowercase().contains("permission denied");
    let run = |args: Vec<&str>| {
        let mut argv: Vec<String> = if sudo { vec!["sudo".into(), "-n".into(), "docker".into()] } else { vec!["docker".into()] };
        argv.extend(args.into_iter().map(String::from));
        argv
    };
    let step = |what: &'static str, out: &ExecOut| -> Result<(), Fail> {
        if out.code == 0 {
            return Ok(());
        }
        let tail: String = out.stderr.trim().chars().rev().take(300).collect::<Vec<_>>().into_iter().rev().collect();
        Err(Fail::local(502, format!("{what} не удалось (код {}): {tail}", out.code)))
    };
    let exec = |argv: Vec<String>, timeout| async move {
        let refs: Vec<&str> = argv.iter().map(String::as_str).collect();
        ctx.exec(id, &refs, None, timeout).await
    };
    if sudo {
        step("Проверка Docker", &exec(run(vec!["info", "--format", "{{.ServerVersion}}"]), 60).await?)?;
    } else {
        step("Проверка Docker", &probe)?;
    }
    step("Загрузка образа", &exec(run(vec!["pull", image]), 600).await?)?;
    // Старого контейнера может не быть: код возврата не важен.
    let _ = exec(run(vec!["rm", "-f", CONTAINER]), 60).await?;
    let published = format!("{PORT}:{PORT}");
    let data_mount = format!("{DATA_DIR}:/data");
    step(
        "Запуск контейнера",
        &exec(
            run(vec!["run", "-d", "--name", CONTAINER, "--restart=always", "--cap-drop=ALL", "--security-opt", "no-new-privileges", "--pids-limit", "512", "--memory", "6g", "-p", &published, "-v", &data_mount, "--env-file", ENV_PATH, image]),
            120,
        )
        .await?,
    )
}

/// Сервер в облаке без сессии отвечает 403 на всё, кроме входа: 200 и 403 значат «наш сервер поднялся».
async fn wait_health(ctx: &Ctx, origin: &str) -> Result<(), Fail> {
    for _ in 0..ctx.health_budget {
        if let Ok(res) = ctx.http.get(format!("{origin}/api/health")).timeout(Duration::from_secs(50)).send().await {
            if matches!(res.status().as_u16(), 200 | 403) {
                return Ok(());
            }
        }
        ctx.sleep(2).await;
    }
    Err(Fail::local(504, "облако не ответило: контейнер не запустился"))
}

// --- Тесты на поддельном Instabox -------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use axum::extract::{Path, Query, State};
    use axum::http::{HeaderMap, StatusCode};
    use axum::routing::{get, post};
    use axum::{Json, Router};
    use std::collections::HashMap;

    #[derive(Default)]
    struct Mem(Mutex<HashMap<String, String>>);
    impl KeyStore for Mem {
        fn get(&self, name: &str) -> Option<String> {
            self.0.lock().unwrap().get(name).cloned()
        }
        fn set(&self, name: &str, value: &str) -> Result<(), String> {
            self.0.lock().unwrap().insert(name.into(), value.into());
            Ok(())
        }
        fn delete(&self, name: &str) -> Result<(), String> {
            self.0.lock().unwrap().remove(name);
            Ok(())
        }
    }

    const KEY: &str = "ibk_abcdefghijklmnopqrstuvwxyz234567abcdefghijklmnopqrs";

    #[derive(Default)]
    struct Fake {
        /// Ответы опроса входа по порядку; после — одобрено.
        polls: Mutex<Vec<(u16, &'static str)>>,
        poll_times: Mutex<Vec<Instant>>,
        machine_state: Mutex<&'static str>,
        cooldown: Mutex<Option<String>>,
        create_status: Mutex<u16>,
        log: Mutex<Vec<String>>,
        docker_fail: Mutex<bool>,
        env_file: Mutex<String>,
        env_mode: Mutex<String>,
        deleted_with: Mutex<Option<String>>,
        keyless: Mutex<u32>,
        exec_calls: Mutex<Vec<Vec<String>>>,
        user_code: Mutex<String>,
    }

    type S = Arc<(Fake, String)>;

    fn err(code: &str, status: u16) -> (StatusCode, Json<Value>) {
        (StatusCode::from_u16(status).unwrap(), Json(json!({ "error": { "code": code, "message": format!("плохо {KEY}"), "retryAfterSeconds": 17 } })))
    }

    async fn serve(fake: Fake) -> (S, String) {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let state: S = Arc::new((fake, base.clone()));
        let app = Router::new()
            .route("/v1/auth/agent", post(|State(s): State<S>| async move {
                let code = { let c = s.0.user_code.lock().unwrap().clone(); if c.is_empty() { "ABCD-2345".to_string() } else { c } };
                Json(json!({ "claimToken": "ibg_secretsecretsecretsecret", "userCode": code, "verificationUri": format!("{}/dashboard/claim", s.1), "verificationUriComplete": format!("{}/dashboard/claim?code=ABCD-2345", s.1), "interval": 0, "expiresIn": 600, "expiresAt": "2030-01-01T00:00:00Z", "name": "deloist", "scopes": SCOPES }))
            }))
            .route("/v1/auth/agent/token", post(|State(s): State<S>, Json(b): Json<Value>| async move {
                assert!(b["claimToken"].as_str().unwrap().starts_with("ibg_"));
                if b.get("ack").is_some() {
                    s.0.log.lock().unwrap().push("ack".into());
                    return (StatusCode::OK, Json(json!({}))).into_response();
                }
                s.0.poll_times.lock().unwrap().push(Instant::now());
                let next = { let mut p = s.0.polls.lock().unwrap(); if p.is_empty() { None } else { Some(p.remove(0)) } };
                match next {
                    Some((status, code)) => (StatusCode::from_u16(status).unwrap(), Json(json!({ "error": { "code": code, "message": "m" } }))).into_response(),
                    None => Json(json!({ "key": KEY, "apiKey": { "id": "k1" }, "userId": 7, "login": "tester" })).into_response(),
                }
            }))
            .route("/v1/machines", post(|State(s): State<S>, headers: HeaderMap, Json(b): Json<Value>| async move {
                if headers.get("idempotency-key").is_none() || headers.get("x-api-key").is_none() {
                    *s.0.keyless.lock().unwrap() += 1;
                }
                s.0.log.lock().unwrap().push(format!("create {} {}", b["name"], b["templateId"]));
                let status = *s.0.create_status.lock().unwrap();
                if status == 409 {
                    return err("machine_cooldown", 409).into_response();
                }
                if status == 503 {
                    let mut r = (StatusCode::SERVICE_UNAVAILABLE, Json(json!({ "error": { "code": "no_capacity", "message": "нет места" } }))).into_response();
                    r.headers_mut().insert("retry-after", "60".parse().unwrap());
                    return r;
                }
                (StatusCode::ACCEPTED, Json(json!({ "id": "op_1", "machineId": "m_abc", "type": "create", "state": "queued", "error": null }))).into_response()
            }))
            .route("/v1/operations/{id}", get(|Path(_id): Path<String>, Query(q): Query<HashMap<String, String>>| async move {
                assert_eq!(q.get("wait").map(String::as_str), Some("25s"));
                Json(json!({ "id": "op_1", "machineId": "m_abc", "state": "succeeded", "error": null }))
            }))
            .route("/v1/machines/{id}", get(|State(s): State<S>| async move {
                let cd = s.0.cooldown.lock().unwrap().clone();
                Json(json!({ "id": "m_abc", "name": "deloist", "state": *s.0.machine_state.lock().unwrap(), "runtime": { "ready": true }, "cooldownUntil": cd }))
            }).delete(|State(s): State<S>, headers: HeaderMap| async move {
                *s.0.deleted_with.lock().unwrap() = headers.get("x-confirm-delete").map(|v| v.to_str().unwrap().to_string());
                (StatusCode::ACCEPTED, Json(json!({ "id": "op_1", "machineId": "m_abc", "type": "delete", "state": "queued", "error": null })))
            }))
            .route("/v1/machines/{id}/wake", post(|| async { Json(json!({ "state": "running" })) }))
            .route("/v1/machines/{id}/ports", post(|State(s): State<S>, Json(b): Json<Value>| async move {
                assert_eq!(b["port"], 8080);
                assert_eq!(b["access"], "public");
                (StatusCode::CREATED, Json(json!({ "id": "p1", "machineId": "m_abc", "port": 8080, "name": "Deloist", "url": format!("{}/", s.1), "access": "public" })))
            }))
            .route("/v1/machines/{id}/resume", post(|State(s): State<S>| async move {
                s.0.log.lock().unwrap().push("resume".into());
                *s.0.machine_state.lock().unwrap() = "running";
                (StatusCode::ACCEPTED, Json(json!({ "id": "op_1", "machineId": "m_abc", "type": "resume", "state": "queued", "error": null })))
            }))
            .route("/v1/machines/{id}/exec", post(|State(s): State<S>, Json(b): Json<Value>| async move {
                let argv: Vec<String> = b["argv"].as_array().expect("только argv").iter().map(|v| v.as_str().unwrap().to_string()).collect();
                assert!(b.get("cmd").is_none());
                s.0.exec_calls.lock().unwrap().push(argv.clone());
                if argv.first().map(String::as_str) == Some("sh") {
                    *s.0.env_mode.lock().unwrap() = argv[2].clone();
                    *s.0.env_file.lock().unwrap() = b["stdin"].as_str().unwrap_or("").to_string();
                }
                let fail = *s.0.docker_fail.lock().unwrap() && argv.contains(&"pull".to_string());
                Json(json!({ "stdout": "", "stderr": if fail { format!("denied token {KEY}") } else { String::new() }, "exitCode": if fail { 1 } else { 0 }, "timedOut": false, "truncated": false }))
            }))
            .route("/api/health", get(|| async { StatusCode::FORBIDDEN }))
            .with_state(state.clone());
        tokio::spawn(async move { axum::serve(listener, app).await.ok() });
        (state, base)
    }

    use axum::response::IntoResponse;

    fn mk(base: &str) -> (Ctx, Arc<Mem>) {
        let mem = Arc::new(Mem::default());
        (Ctx::with_base(mem.clone(), base.to_string()).fast(), mem)
    }

    async fn until(hub: &Hub, ctx: &Ctx, kind: &str) -> Value {
        for _ in 0..2000 {
            let st = hub.status(ctx).await;
            if st["job"]["kind"] == kind && st["job"]["state"] != "running" {
                return st;
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
        panic!("работа не закончилась");
    }

    #[tokio::test]
    async fn signin_honors_pending_slow_down_and_stores_the_key_silently() {
        let fake = Fake { polls: Mutex::new(vec![(400, "authorization_pending"), (400, "slow_down"), (400, "authorization_pending")]), ..Default::default() };
        let (state, base) = serve(fake).await;
        let (ctx, mem) = mk(&base);
        let hub = Hub::default();
        let started = hub.signin_start(ctx.clone()).await.unwrap();
        assert_eq!(started["userCode"], "ABCD-2345");
        assert!(started["verificationUriComplete"].as_str().unwrap().contains("code=ABCD-2345"));
        assert!(!started.to_string().contains("ibg_"), "claimToken не отдаётся");
        for _ in 0..500 {
            if hub.signin_status(&ctx)["phase"] == "approved" {
                break;
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
        let status = hub.signin_status(&ctx);
        assert_eq!(status["phase"], "approved");
        assert_eq!(status["signedIn"], true);
        assert_eq!(mem.get(K_API).as_deref(), Some(KEY));
        assert!(!status.to_string().contains("ibk_") && !hub.status(&ctx).await.to_string().contains("ibk_"));
        assert!(state.0.log.lock().unwrap().contains(&"ack".to_string()), "ключ подтверждён серверу");
        // После slow_down интервал вырос на 5 единиц (интервал сервера 0 → 5 после замечания).
        let times = state.0.poll_times.lock().unwrap().clone();
        assert_eq!(times.len(), 4);
        assert!(times[2] - times[1] >= Duration::from_millis(5 * 2), "slow_down замедлил опрос");
    }

    #[tokio::test]
    async fn signin_ends_on_denied_and_expired() {
        for (code, phase) in [("access_denied", "denied"), ("expired_token", "expired")] {
            let (_s, base) = serve(Fake { polls: Mutex::new(vec![(400, code)]), ..Default::default() }).await;
            let (ctx, mem) = mk(&base);
            let hub = Hub::default();
            hub.signin_start(ctx.clone()).await.unwrap();
            for _ in 0..500 {
                if hub.signin_status(&ctx)["phase"] == phase {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
            assert_eq!(hub.signin_status(&ctx)["phase"], phase);
            assert!(mem.get(K_API).is_none(), "ключа нет");
        }
    }

    #[tokio::test]
    async fn signin_refuses_a_user_code_with_odd_characters() {
        let (_s, base) = serve(Fake { user_code: Mutex::new("<b>x</b>".into()), ..Default::default() }).await;
        let (ctx, _) = mk(&base);
        assert!(Hub::default().signin_start(ctx).await.is_err());
    }

    #[tokio::test]
    async fn signin_refuses_a_verification_link_to_another_host() {
        let (_s, base) = serve(Fake::default()).await;
        let mem = Arc::new(Mem::default());
        let production = Ctx::with_base(mem, PROD_BASE.into());
        assert!(!trusted_link(&production, "https://evil.example/claim"));
        assert!(!trusted_link(&production, "http://instabox.sh/claim"));
        assert!(!trusted_link(&production, "https://instabox.sh.evil.example/claim"));
        assert!(trusted_link(&production, "https://dashboard.instabox.sh/dashboard/claim?code=A"));
        let (ctx, _) = mk(&base);
        assert!(trusted_link(&ctx, &format!("{base}/x")));
    }

    #[tokio::test]
    async fn provisioning_creates_a_machine_starts_the_container_and_keeps_secrets_out_of_view() {
        let (state, base) = serve(Fake { machine_state: Mutex::new("running"), ..Default::default() }).await;
        let (ctx, mem) = mk(&base);
        mem.set(K_API, KEY).unwrap();
        let hub = Hub::default();
        hub.provision(ctx.clone(), None, Some("stand")).ok().expect("старт");
        let st = until(&hub, &ctx, "provision").await;
        assert_eq!(st["job"]["state"], "done", "{st}");
        assert_eq!(st["machine"]["state"], "running");
        assert_eq!(*state.0.keyless.lock().unwrap(), 0, "Idempotency-Key и ключ на создании");
        assert!(state.0.log.lock().unwrap().contains(&"create \"deloist\" \"agent-browser\"".to_string()));

        // Файл настроек: 0600, все переменные, а ключ шлюза закрытый — нет.
        assert_eq!(*state.0.env_mode.lock().unwrap(), format!("umask 077; cat > {ENV_PATH}"));
        let calls0 = state.0.exec_calls.lock().unwrap().clone();
        assert!(calls0.iter().any(|c| c == &["sudo", "-n", "chown", "10001:10001", DATA_DIR]), "каталог отдан uid контейнера");
        assert!(calls0.iter().any(|c| c == &["sudo", "-n", "chmod", "700", DATA_DIR]));
        let env = state.0.env_file.lock().unwrap().clone();
        for want in ["CREW_CLOUD=1", "HOST=0.0.0.0", "PORT=8080", "CREW_GATEWAY_PUBKEY=", "CREW_USER_ID=u-", &format!("CREW_PUBLIC_ORIGIN={base}"), "CREW_SECRETS_KEY="] {
            assert!(env.contains(want), "нет {want}: {env}");
        }
        let seed = mem.get(K_GATEWAY).unwrap();
        assert!(!env.contains(&seed), "закрытый ключ шлюза не уходит на машину");
        assert_eq!(env.lines().find(|l| l.starts_with("CREW_SECRETS_KEY=")).unwrap().len(), "CREW_SECRETS_KEY=".len() + 64);

        // Команды — argv, образ нашего репозитория и тег.
        let calls = state.0.exec_calls.lock().unwrap().clone();
        let run = calls.iter().find(|c| c.contains(&"run".to_string())).expect("docker run");
        assert_eq!(&run[..2], &["docker".to_string(), "run".to_string()]);
        assert!(run.contains(&"ghcr.io/gordidima-rgb/deloist-server:stand".to_string()));
        assert!(run.windows(2).any(|w| w == ["--env-file", ENV_PATH]) && run.windows(2).any(|w| w == ["-v", "/data/deloist/data:/data"]));
        assert!(run.contains(&"--restart=always".to_string()) && run.contains(&"8080:8080".to_string()) );
        assert!(calls.iter().any(|c| c.contains(&"pull".to_string())));

        // Ни ответы, ни снимок состояния не содержат секретов.
        let all = format!("{} {}", st, hub.signin_status(&ctx));
        for secret in [KEY.to_string(), seed, mem.get(K_SECRETS).unwrap()] {
            assert!(!all.contains(&secret), "секрет в ответе");
        }
        assert!(!all.contains("ibk_"));
    }

    #[tokio::test]
    async fn the_gateway_pass_is_what_the_server_verifies() {
        let (_s, base) = serve(Fake { machine_state: Mutex::new("running"), ..Default::default() }).await;
        let (ctx, mem) = mk(&base);
        mem.set(K_API, KEY).unwrap();
        let hub = Hub::default();
        hub.provision(ctx.clone(), None, None).ok().unwrap();
        until(&hub, &ctx, "provision").await;
        let opened = hub.open(ctx.clone()).await.unwrap();
        assert_eq!(opened["ok"], true);
        let url = opened["url"].as_str().unwrap();
        let (origin, token) = url.split_once("/auth/gateway?t=").unwrap();
        assert_eq!(origin, base);
        // Разбираем так же, как guard.rs.
        let b64 = base64::engine::general_purpose::URL_SAFE_NO_PAD;
        let (payload_b64, sig_b64) = token.split_once('.').unwrap();
        let payload = b64.decode(payload_b64).unwrap();
        let sig = ed25519_dalek::Signature::from_slice(&b64.decode(sig_b64).unwrap()).unwrap();
        let mut signed = b"deloist-gw-v1\n".to_vec();
        signed.extend_from_slice(&payload);
        let public = signing_key(&mem.get(K_GATEWAY).unwrap()).unwrap().verifying_key();
        public.verify_strict(&signed, &sig).expect("подпись сходится");
        let claims: Value = serde_json::from_slice(&payload).unwrap();
        assert_eq!(claims["uid"].as_str(), mem.get(K_UID).as_deref());
        assert_eq!(claims["aud"], base.as_str());
        assert_eq!(claims["exp"].as_i64().unwrap() - claims["iat"].as_i64().unwrap(), 30);
        assert!(claims["nonce"].as_str().unwrap().len() >= 16);
        // Публичная часть в env совпадает с ключом, которым подписано.
        let env = _s.0.env_file.lock().unwrap().clone();
        assert!(env.contains(&format!("CREW_GATEWAY_PUBKEY={}", crate::store::hex(public.as_bytes()))));
    }

    #[tokio::test]
    async fn provisioning_reports_a_cooldown_and_capacity_with_retry_after() {
        for (status, code, retry) in [(409u16, "machine_cooldown", 17u64), (503, "no_capacity", 60)] {
            let (_s, base) = serve(Fake { create_status: Mutex::new(status), machine_state: Mutex::new("running"), ..Default::default() }).await;
            let (ctx, mem) = mk(&base);
            mem.set(K_API, KEY).unwrap();
            let hub = Hub::default();
            hub.provision(ctx.clone(), None, None).ok().unwrap();
            let st = until(&hub, &ctx, "provision").await;
            assert_eq!(st["job"]["state"], "error");
            assert_eq!(st["job"]["retryAfterSeconds"], retry, "{code}");
            assert!(st["job"]["error"].as_str().unwrap().contains(code));
            assert!(!st.to_string().contains("ibk_"), "ключ из текста ошибки вычищен");
            assert!(mem.get(K_MACHINE).is_none());
        }
    }

    #[tokio::test]
    async fn docker_errors_are_scrubbed_and_reported() {
        let (_s, base) = serve(Fake { docker_fail: Mutex::new(true), machine_state: Mutex::new("running"), ..Default::default() }).await;
        let (ctx, mem) = mk(&base);
        mem.set(K_API, KEY).unwrap();
        let hub = Hub::default();
        hub.provision(ctx.clone(), None, None).ok().unwrap();
        let st = until(&hub, &ctx, "provision").await;
        assert_eq!(st["job"]["state"], "error");
        let text = st["job"]["error"].as_str().unwrap();
        assert!(text.contains("Загрузка образа") && !text.contains("ibk_"), "{text}");
    }

    #[tokio::test]
    async fn a_paused_machine_is_resumed_never_suspended() {
        let (state, base) = serve(Fake { machine_state: Mutex::new("suspended"), ..Default::default() }).await;
        let (ctx, mem) = mk(&base);
        for (k, v) in [(K_API, KEY), (K_MACHINE, "m_abc"), (K_ORIGIN, base.as_str()), (K_GATEWAY, &rand_hex(32)), (K_UID, "u-1")] {
            mem.set(k, v).unwrap();
        }
        let hub = Hub::default();
        let first = hub.open(ctx.clone()).await.unwrap();
        assert_eq!(first["state"], "waking");
        assert_eq!(until(&hub, &ctx, "wake").await["job"]["state"], "done");
        assert!(state.0.log.lock().unwrap().contains(&"resume".to_string()));
        assert_eq!(hub.open(ctx.clone()).await.unwrap()["ok"], true);
    }

    #[tokio::test]
    async fn status_shows_paused_and_cooldown_without_waking() {
        let (state, base) = serve(Fake { machine_state: Mutex::new("suspended"), ..Default::default() }).await;
        let (ctx, mem) = mk(&base);
        mem.set(K_API, KEY).unwrap();
        mem.set(K_MACHINE, "m_abc").unwrap();
        let hub = Hub::default();
        assert_eq!(hub.status(&ctx).await["machine"]["state"], "paused");
        *state.0.cooldown.lock().unwrap() = Some((chrono::Utc::now() + chrono::Duration::seconds(600)).to_rfc3339());
        tokio::time::sleep(Duration::from_millis(3100)).await; // кэш снимка живёт 3 с
        let machine = hub.status(&ctx).await["machine"].clone();
        assert_eq!(machine["state"], "cooldown");
        assert!(machine["retryAfterSeconds"].as_u64().unwrap() > 500);
        // Сервер недоступен.
        let (dead, mem2) = mk("http://127.0.0.1:1");
        mem2.set(K_API, KEY).unwrap();
        mem2.set(K_MACHINE, "m_abc").unwrap();
        assert_eq!(Hub::default().status(&dead).await["machine"]["state"], "unreachable");
    }

    #[tokio::test]
    async fn delete_confirms_with_the_machine_id_and_forgets_the_keys() {
        let (state, base) = serve(Fake { machine_state: Mutex::new("running"), ..Default::default() }).await;
        let (ctx, mem) = mk(&base);
        mem.set(K_API, KEY).unwrap();
        let hub = Hub::default();
        hub.provision(ctx.clone(), None, None).ok().unwrap();
        until(&hub, &ctx, "provision").await;
        hub.delete(ctx.clone(), false).ok().expect("старт");
        let st = until(&hub, &ctx, "delete").await;
        assert_eq!(st["job"]["state"], "done", "{st}");
        assert_eq!(state.0.deleted_with.lock().unwrap().as_deref(), Some("m_abc"));
        for k in [K_MACHINE, K_GATEWAY, K_SECRETS, K_UID, K_ORIGIN] {
            assert!(mem.get(k).is_none(), "{k} забыт");
        }
        assert_eq!(mem.get(K_API).as_deref(), Some(KEY), "вход в аккаунт остаётся");
    }

    #[test]
    fn names_tags_and_scrub_follow_the_whitelist() {
        assert!(valid_name("deloist") && valid_name("a-1"));
        for bad in ["", "-a", "a-", "A", "a b", "a;b", "a/../b", &"x".repeat(33)] {
            assert!(!valid_name(bad), "{bad}");
        }
        assert!(valid_tag("v0.3.6") && valid_tag("main") && valid_tag("sha-abc_1"));
        for bad in ["", ".x", "-x", "a b", "a:b", "a@sha256:1", "a/b", "$(id)", "a;b", &"x".repeat(129)] {
            assert!(!valid_tag(bad), "{bad}");
        }
        let dirty = format!("key {KEY} and {} fine", "ab".repeat(32));
        let clean = scrub(&dirty);
        assert!(!clean.contains("ibk_") && !clean.contains(&"ab".repeat(32)) && clean.contains("fine"));
    }

    #[test]
    fn the_cloud_origin_must_be_exactly_the_machines_own_address() {
        let mem = Arc::new(Mem::default());
        let prod = Ctx::with_base(mem.clone(), PROD_BASE.into());
        let id = "m_agq6flsjjrovewk7wro6qmgnee";
        let good = "https://8080-agq6flsjjrovewk7wro6qmgnee.instabox.tech";
        assert_eq!(normalize_origin(&prod, id, &format!("{good}/")).unwrap(), good);
        assert_eq!(normalize_origin(&prod, id, good).unwrap(), good);
        for bad in ["https://evil.example/", "https://8080-other.instabox.tech/", "https://127.0.0.1/", "http://8080-agq6flsjjrovewk7wro6qmgnee.instabox.tech/", "https://8080-agq6flsjjrovewk7wro6qmgnee.instabox.tech:444/", "https://8080-agq6flsjjrovewk7wro6qmgnee.instabox.tech/x", "https://8080-agq6flsjjrovewk7wro6qmgnee.instabox.tech.evil.example/", "https://user@8080-agq6flsjjrovewk7wro6qmgnee.instabox.tech/", "https://8080-AGQ6flsjjrovewk7wro6qmgnee.instabox.tech/", ""] {
            assert!(normalize_origin(&prod, id, bad).is_err(), "{bad}");
        }
        assert!(normalize_origin(&prod, "agq", good).is_err(), "id без m_");
        // Подменённый сохранённый адрес не доходит ни до пропуска, ни до открытия.
        for (k, v) in [(K_API, KEY), (K_MACHINE, id), (K_ORIGIN, "https://evil.example"), (K_GATEWAY, "11".repeat(32).as_str())] {
            mem.set(k, v).unwrap();
        }
        assert!(deployed(&prod).is_err());
        mem.set(K_ORIGIN, good).unwrap();
        assert!(deployed(&prod).is_ok());
    }

    #[tokio::test]
    async fn a_bad_port_address_stores_nothing() {
        // Поддельный сервер отдаёт адрес чужого хоста.
        let (_s, base) = serve(Fake { machine_state: Mutex::new("running"), ..Default::default() }).await;
        let (ctx, mem) = mk(&base);
        assert!(normalize_origin(&ctx, "m_abc", "https://evil.example/").is_err());
        assert!(mem.get(K_ORIGIN).is_none());
    }

    #[test]
    fn the_api_address_is_pinned_and_overridable_only_to_loopback() {
        let base = base_url();
        assert!(base == PROD_BASE || base.starts_with("http://127.0.0.1:") || base.starts_with("http://localhost:"));
    }
}
