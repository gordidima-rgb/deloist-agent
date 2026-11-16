//! Мост в Telegram: свой бот человека, привязка чата кодом и переписка с выбранным ботом команды.
//!
//! Токен лежит только в `secrets.json`; в `state.json`, журналах и ответах API его нет. Адрес
//! Telegram содержит токен, поэтому в сообщения об ошибках и логи попадают только метод и
//! описание от Telegram, но никогда сам адрес.

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use rand::Rng;
use serde_json::{json, Value};
use tokio::sync::broadcast::error::RecvError;
use tokio_util::sync::CancellationToken;

use crate::store::now;
use crate::turn::TurnOptions;
use crate::App;

const API: &str = "https://api.telegram.org";
/// Long poll: Telegram держит запрос до 25 секунд.
const POLL_TIMEOUT: u64 = 25;
const CODE_TTL_MS: i64 = 10 * 60 * 1000;
const MAX_MESSAGE_CHARS: usize = 4000;
const STRANGER_REPLY_MS: i64 = 60 * 60 * 1000;
const PREFIX: &str = "[Telegram] ";

/// Базовый адрес Telegram. Подмена для тестов работает только в отладочной сборке:
/// выпущенная программа не может увести токен человека на чужой адрес.
fn api_base() -> String {
    #[cfg(debug_assertions)]
    {
        if let Ok(url) = std::env::var("CREW_TEST_TELEGRAM_API") {
            let url = url.trim().trim_end_matches('/');
            if !url.is_empty() {
                return url.to_string();
            }
        }
    }
    API.to_string()
}

fn client() -> reqwest::Client {
    reqwest::Client::builder()
        .timeout(Duration::from_secs(POLL_TIMEOUT + 15))
        .build()
        .unwrap_or_else(|_| reqwest::Client::new())
}

/// Шестизначный код привязки чата и время, до которого он действует.
#[derive(Clone, Debug, PartialEq)]
pub struct Pairing {
    pub code: String,
    pub expires_at: i64,
}

impl Pairing {
    pub fn new(at: i64) -> Self {
        Self { code: make_code(), expires_at: at + CODE_TTL_MS }
    }

    pub fn accepts(&self, code: &str, at: i64) -> bool {
        at < self.expires_at && self.code == code.trim()
    }
}

pub fn make_code() -> String {
    rand::thread_rng().gen_range(100_000..=999_999).to_string()
}

/// Что делать с одним обновлением Telegram. Разбор отделён от сети: его проверяют тесты.
#[derive(Debug, PartialEq)]
pub enum Incoming {
    Pair { chat_id: i64 },
    Text { chat_id: i64, text: String },
    Stop { chat_id: i64 },
    New { chat_id: i64 },
    /// Чат не привязан: один ответ в час, чтобы не заваливать чужого человека.
    Stranger { chat_id: i64 },
    Ignore,
}

fn start_code(text: &str) -> Option<&str> {
    let rest = text.strip_prefix("/start")?;
    if !rest.chars().next().is_some_and(char::is_whitespace) {
        return None;
    }
    let code = rest.trim();
    (!code.is_empty() && !code.chars().any(char::is_whitespace)).then_some(code)
}

pub fn classify(update: &Value, paired_chat: &str, pairing: Option<&Pairing>, at: i64) -> Incoming {
    let Some(message) = update.get("message") else {
        return Incoming::Ignore;
    };
    let Some(chat_id) = message
        .get("chat")
        .and_then(|chat| chat.get("id"))
        .and_then(|id| id.as_i64().or_else(|| id.as_str().and_then(|text| text.parse().ok())))
    else {
        return Incoming::Ignore;
    };
    let Some(text) = message.get("text").and_then(Value::as_str).map(str::trim) else {
        return Incoming::Ignore;
    };
    if text.is_empty() {
        return Incoming::Ignore;
    }
    if let Some(code) = start_code(text) {
        if pairing.is_some_and(|pairing| pairing.accepts(code, at)) {
            return Incoming::Pair { chat_id };
        }
    }
    if !paired_chat.is_empty() && chat_id.to_string() == paired_chat {
        return match text {
            "/stop" => Incoming::Stop { chat_id },
            "/new" => Incoming::New { chat_id },
            _ if text.starts_with('/') => Incoming::Ignore,
            _ => Incoming::Text { chat_id, text: text.to_string() },
        };
    }
    Incoming::Stranger { chat_id }
}

/// Части сообщения Telegram: не длиннее лимита, по возможности по границе строки.
/// Склейка частей даёт исходный текст без потерь.
pub fn split_text(text: &str, limit: usize) -> Vec<String> {
    let limit = limit.max(1);
    let chars: Vec<char> = text.chars().collect();
    let mut parts = Vec::new();
    let mut start = 0;
    while start < chars.len() {
        let mut end = (start + limit).min(chars.len());
        if end < chars.len() {
            if let Some(position) = chars[start..end].iter().rposition(|c| *c == '\n') {
                if position > 0 {
                    end = start + position + 1;
                }
            }
        }
        parts.push(chars[start..end].iter().collect());
        start = end;
    }
    parts
}

#[derive(Default)]
pub struct Telegram {
    code: Mutex<Option<Pairing>>,
    username: Mutex<String>,
    /// Ходы, начатые из Telegram: по ним `turn_end` возвращает ответ в чат.
    turns: Mutex<HashSet<String>>,
    /// Кому уже отвечали «этот бот не подключён» и когда.
    warned: Mutex<HashMap<i64, i64>>,
    /// Текущий long poll: `DELETE /api/telegram` отменяет его сразу.
    poll: Mutex<Option<CancellationToken>>,
    /// Облако: обновления из вебхука идут в один обработчик по очереди, как из long poll.
    hook_tx: Mutex<Option<tokio::sync::mpsc::Sender<Value>>>,
    /// Неудачные попытки вебхука: одно общее окно (число и начало, мс). За мостом docker/Instabox
    /// адрес собеседника у всех один, поэтому по адресам не считаем; верный секрет не ограничивается.
    hook_fails: Mutex<(u32, i64)>,
    /// Почему мост стоит (например, бот уже работает в облаке); пусто — всё в порядке.
    problem: Mutex<String>,
}

impl Telegram {
    pub fn code(&self) -> Option<Pairing> {
        self.code.lock().unwrap().clone()
    }

    pub fn set_code(&self, pairing: Pairing) {
        *self.code.lock().unwrap() = Some(pairing);
    }

    pub fn clear_code(&self) {
        *self.code.lock().unwrap() = None;
    }

    pub fn problem(&self) -> String {
        self.problem.lock().unwrap().clone()
    }

    fn set_problem(&self, text: &str) {
        *self.problem.lock().unwrap() = text.to_string();
    }

    pub fn set_username(&self, username: String) {
        *self.username.lock().unwrap() = username;
    }

    /// Забыть всё, что относится к мосту, и остановить опрос.
    pub fn forget(&self) {
        self.clear_code();
        self.username.lock().unwrap().clear();
        self.turns.lock().unwrap().clear();
        self.warned.lock().unwrap().clear();
        if let Some(cancel) = self.poll.lock().unwrap().take() {
            cancel.cancel();
        }
    }

    /// Имя бота Telegram для ответа `GET /api/telegram`; ошибка сети не мешает остальным полям.
    pub async fn username(&self, token: &str) -> String {
        let cached = self.username.lock().unwrap().clone();
        if !cached.is_empty() {
            return cached;
        }
        match fetch_username(token).await {
            Ok(name) => {
                self.set_username(name.clone());
                name
            }
            Err(e) => {
                eprintln!("[telegram] getMe: {e}");
                String::new()
            }
        }
    }

    fn should_warn(&self, chat_id: i64, at: i64) -> bool {
        let mut warned = self.warned.lock().unwrap();
        match warned.get(&chat_id) {
            Some(last) if at - last < STRANGER_REPLY_MS => false,
            _ => {
                warned.insert(chat_id, at);
                true
            }
        }
    }
}

/// Ошибка Telegram без адреса запроса: в адресе лежит токен.
fn network_error(error: reqwest::Error) -> String {
    if error.is_timeout() {
        "Telegram не ответил вовремя".to_string()
    } else if error.is_connect() {
        "нет связи с Telegram".to_string()
    } else {
        "ошибка сети Telegram".to_string()
    }
}

async fn call(client: &reqwest::Client, token: &str, method: &str, body: Value) -> Result<Value, String> {
    let url = format!("{}/bot{token}/{method}", api_base());
    let response = client.post(url).json(&body).send().await.map_err(network_error)?;
    let status = response.status();
    let value: Value = response.json().await.map_err(|_| "ответ Telegram не разобрать".to_string())?;
    if !status.is_success() || value["ok"] != true {
        let description = value["description"].as_str().unwrap_or("").trim();
        return Err(if description.is_empty() {
            format!("HTTP {}", status.as_u16())
        } else {
            description.to_string()
        });
    }
    Ok(value.get("result").cloned().unwrap_or(Value::Null))
}

/// Проверка токена: `getMe` отдаёт имя бота.
pub async fn fetch_username(token: &str) -> Result<String, String> {
    let result = call(&client(), token, "getMe", json!({})).await?;
    Ok(result["username"].as_str().unwrap_or_default().to_string())
}

async fn get_updates(client: &reqwest::Client, token: &str, offset: i64) -> Result<Vec<Value>, String> {
    let result = call(
        client,
        token,
        "getUpdates",
        json!({ "offset": offset, "timeout": POLL_TIMEOUT, "allowed_updates": ["message"] }),
    )
    .await?;
    Ok(result.as_array().cloned().unwrap_or_default())
}

/// Следующая пауза перед повтором: не больше минуты.
fn next_delay(delay: u64) -> u64 {
    (delay * 2).min(60)
}

/// Что отправить в чат по концу хода. Карточка разрешения важнее финального текста:
/// человек должен знать, что ответа не будет, пока он не откроет Deloist.
pub(crate) fn turn_reply(text: &str, pending: &[Value]) -> Option<String> {
    if pending.iter().any(|item| item["kind"] == "approval") {
        return Some("Нужно разрешение — открой Deloist".to_string());
    }
    (!text.trim().is_empty()).then(|| text.to_string())
}

/// Отправка с повтором и задёржкой до минуты. Без `parse_mode`: текст уходит как есть.
/// Возвращает `false`, если какая-то часть так и не ушла.
async fn send_text(client: &reqwest::Client, token: &str, chat_id: i64, text: &str) -> bool {
    for part in split_text(text, MAX_MESSAGE_CHARS) {
        let body = json!({ "chat_id": chat_id, "text": part });
        let mut delay = 1u64;
        let mut sent = false;
        for attempt in 0..6u32 {
            match call(client, token, "sendMessage", body.clone()).await {
                Ok(_) => {
                    sent = true;
                    break;
                }
                Err(e) => {
                    eprintln!("[telegram] sendMessage: {e}");
                    if attempt < 5 {
                        tokio::time::sleep(Duration::from_secs(delay)).await;
                        delay = next_delay(delay);
                    }
                }
            }
        }
        if !sent {
            return false;
        }
    }
    true
}

/// Текст в привязанный чат, без хода бота (напоминания). Не подключено или не привязано — понятная причина.
pub(crate) async fn push_text(app: &Arc<App>, text: &str) -> Result<(), String> {
    let token = app.store.telegram_token();
    let chat = app.store.telegram().chat_id;
    if token.trim().is_empty() || chat.trim().is_empty() {
        return Err("Telegram не подключён".into());
    }
    let chat_id: i64 = chat.trim().parse().map_err(|_| "Telegram не подключён".to_string())?;
    if send_text(&client(), &token, chat_id, text).await {
        Ok(())
    } else {
        Err("Telegram не принял сообщение".into())
    }
}

/// Запуск обоих циклов: опрос обновлений и ответы на конец хода.
pub fn start(app: Arc<App>) {
    let replies = app.clone();
    tokio::spawn(async move { reply_loop(replies).await });
    if crate::cloud::active() {
        let (tx, rx) = tokio::sync::mpsc::channel(HOOK_QUEUE);
        *app.telegram.hook_tx.lock().unwrap() = Some(tx);
        let worker = app.clone();
        tokio::spawn(async move { hook_worker(worker, rx).await });
    }
    tokio::spawn(async move { poll_loop(app).await });
}

async fn poll_loop(app: Arc<App>) {
    let client = client();
    loop {
        let token = app.store.telegram_token();
        let settings = app.store.telegram();
        if token.is_empty() || !settings.enabled {
            tokio::time::sleep(Duration::from_secs(1)).await;
            continue;
        }
        let cancel = CancellationToken::new();
        *app.telegram.poll.lock().unwrap() = Some(cancel.clone());
        if crate::cloud::active() {
            // Облако: машина просыпается от входящего запроса, а не от long poll, поэтому вебхук.
            webhook_session(&app, &client, &token, &cancel).await;
            *app.telegram.poll.lock().unwrap() = None;
            tokio::time::sleep(Duration::from_millis(500)).await;
            continue;
        }
        // ПК: если бот раньше жил в облаке, у Telegram остался вебхук, и getUpdates с ним не работает.
        if !drop_webhook(&client, &token).await {
            // Бот работает в облаке: опрос отнял бы у облака сообщения. Стоим, пока токен не сменят.
            app.telegram.set_problem("бот работает в облаке");
            loop {
                tokio::select! {
                    _ = cancel.cancelled() => break,
                    _ = tokio::time::sleep(Duration::from_secs(1)) => {}
                }
                if app.store.telegram_token() != token || !app.store.telegram().enabled {
                    break;
                }
            }
            app.telegram.set_problem("");
            *app.telegram.poll.lock().unwrap() = None;
            tokio::time::sleep(Duration::from_millis(500)).await;
            continue;
        }
        let mut delay = 1u64;
        loop {
            let settings = app.store.telegram();
            if app.store.telegram_token() != token || !settings.enabled {
                break;
            }
            let result = tokio::select! {
                _ = cancel.cancelled() => break,
                result = get_updates(&client, &token, settings.offset) => result,
            };
            match result {
                Ok(updates) => {
                    delay = 1;
                    if updates.is_empty() {
                        // Настоящий long poll держит запрос сам; тестовый сервер отвечает сразу.
                        tokio::time::sleep(Duration::from_millis(300)).await;
                        continue;
                    }
                    process_updates(&app, &client, &token, &updates).await;
                }
                Err(e) => {
                    eprintln!("[telegram] getUpdates: {e}");
                    tokio::time::sleep(Duration::from_secs(delay)).await;
                    delay = next_delay(delay);
                }
            }
        }
        *app.telegram.poll.lock().unwrap() = None;
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
}

/// Общий путь для long poll и вебхука: разобрать обновления и запомнить `offset`.
/// Уже разобранное (`update_id` меньше offset) пропускается: Telegram может повторить доставку.
async fn process_updates(app: &Arc<App>, client: &reqwest::Client, token: &str, updates: &[Value]) {
    let start = app.store.telegram().offset;
    let mut next = start;
    for update in updates {
        let id = update["update_id"].as_i64().unwrap_or(0);
        if id != 0 && id < start {
            continue;
        }
        handle_update(app, client, token, update).await;
        next = next.max(id.saturating_add(1));
    }
    if next != start {
        if let Err(e) = app.store.update_telegram(|t| t.offset = next).await {
            eprintln!("[telegram] не смог сохранить offset: {e}");
        }
    }
}

async fn handle_update(app: &Arc<App>, client: &reqwest::Client, token: &str, update: &Value) {
    let settings = app.store.settings();
    let pairing = app.telegram.code();
    let at = now();
    let bot_id = settings.telegram.bot_id.clone();
    match classify(update, &settings.telegram.chat_id, pairing.as_ref(), at) {
        Incoming::Ignore => {}
        Incoming::Stranger { chat_id } => {
            if app.telegram.should_warn(chat_id, at) {
                send_text(client, token, chat_id, "Этот бот не подключён к вашему Deloist").await;
            }
        }
        Incoming::Pair { chat_id } => {
            if let Err(e) = app.store.update_telegram(|t| {
                t.chat_id = chat_id.to_string();
                t.enabled = true;
            }).await {
                eprintln!("[telegram] не смог сохранить чат: {e}");
                return;
            }
            app.telegram.clear_code();
            let name = app.store.bot(&bot_id).map(|bot| bot.name).unwrap_or_default();
            let text = if name.is_empty() {
                "Готово: пишите сюда".to_string()
            } else {
                format!("Готово: пишите сюда, отвечает {name}")
            };
            send_text(client, token, chat_id, &text).await;
        }
        Incoming::Text { chat_id, text } => {
            crate::workflow_run::on_message(app, "telegram", &chat_id.to_string(), &text);
            deliver(app, "telegram", PREFIX, &bot_id, &text).await;
        }
        Incoming::Stop { .. } => {
            if bot_id.is_empty() {
                return;
            }
            // Сначала пауза, потом отмена: как у кнопки остановки в окне.
            let _ = app.queues.pause(&bot_id).await;
            app.turns.stop_turn(&bot_id);
        }
        Incoming::New { chat_id } => {
            let Some(bot) = app.store.bot(&bot_id) else {
                return;
            };
            if app.turns.is_busy(&bot_id) {
                send_text(client, token, chat_id, "Бот сейчас отвечает: сожми историю, когда он закончит").await;
                return;
            }
            let text = match crate::compact::bot_thread(app, &bot, true).await {
                Some(_) => "История сжата.",
                None => "Сжимать пока нечего.",
            };
            send_text(client, token, chat_id, text).await;
        }
    }
}

// --- вебхук (облако) -------------------------------------------------------
// Параметры Bot API проверены 16.11.2026 на https://core.telegram.org/bots/api :
// setWebhook { url, allowed_updates, drop_pending_updates, secret_token (1-256 символов A-Za-z0-9_-) },
// deleteWebhook { drop_pending_updates }, getWebhookInfo -> { url, ... };
// секрет Telegram присылает в заголовке `X-Telegram-Bot-Api-Secret-Token`.

const HOOK_QUEUE: usize = 256;
/// 128 КиБ × очередь 256 = не больше 32 МиБ в памяти; настоящее обновление Telegram куда меньше.
pub(crate) const HOOK_BODY_MAX: usize = 128 * 1024;
/// Окно `update_id` вокруг offset: дальше — подделка или сбой, а огромный номер сдвинул бы offset навсегда.
const HOOK_ID_AHEAD: i64 = 100_000;
/// Как часто облако сверяет, что вебхук на месте.
const HOOK_RECHECK_SECS: u64 = 300;
/// Адреса облака Instabox: вебхук там снимать с ПК нельзя.
const CLOUD_HOST_SUFFIX: &str = ".instabox.tech";
const HOOK_FAILS_MAX: u32 = 30;
const HOOK_FAILS_WINDOW_MS: i64 = 60 * 1000;
pub const SECRET_HEADER: &str = "x-telegram-bot-api-secret-token";

/// Адрес, который Telegram вызывает в облаке.
fn webhook_url(bot_id: &str) -> Option<String> {
    let origin = crate::guard::public_origin()?;
    Some(format!("{origin}/hooks/telegram/{bot_id}"))
}

/// Хост адреса вебхука в нижнем регистре.
fn url_host(url: &str) -> String {
    let rest = url.split_once("://").map(|(_, r)| r).unwrap_or(url);
    let host = rest.split(['/', '?', '#']).next().unwrap_or("");
    let host = host.rsplit('@').next().unwrap_or("");
    host.split(':').next().unwrap_or("").to_ascii_lowercase()
}

/// Если у бота стоит вебхук, снять его (без потери накопленных обновлений).
/// `false` — вебхук стоит в облаке Instabox: его не трогаем, на ПК опрос не начинаем.
async fn drop_webhook(client: &reqwest::Client, token: &str) -> bool {
    let set = match call(client, token, "getWebhookInfo", json!({})).await {
        Ok(info) => {
            let url = info["url"].as_str().unwrap_or("").to_string();
            if url_host(&url).ends_with(CLOUD_HOST_SUFFIX) {
                return false;
            }
            !url.is_empty()
        }
        Err(_) => true,
    };
    if set {
        if let Err(e) = call(client, token, "deleteWebhook", json!({ "drop_pending_updates": false })).await {
            eprintln!("[telegram] deleteWebhook: {e}");
        }
    }
    true
}

/// Интервал сверки вебхука; в отладочной сборке тест может его сократить.
fn recheck_interval() -> Duration {
    #[cfg(debug_assertions)]
    {
        if let Some(secs) = std::env::var("CREW_TEST_TELEGRAM_RECHECK_SECS").ok().and_then(|v| v.parse::<u64>().ok()) {
            return Duration::from_secs(secs.max(1));
        }
    }
    Duration::from_secs(HOOK_RECHECK_SECS)
}

/// Облако: ставит вебхук и ждёт, пока токен, выключатель или отмена не сменятся; потом снимает вебхук.
async fn webhook_session(app: &Arc<App>, client: &reqwest::Client, token: &str, cancel: &CancellationToken) {
    let mut delay = 1u64;
    let mut registered = false;
    let mut last_check = std::time::Instant::now();
    loop {
        let settings = app.store.telegram();
        if cancel.is_cancelled() || app.store.telegram_token() != token || !settings.enabled {
            break;
        }
        if !registered {
            let result = match (webhook_url(&settings.bot_id), app.store.ensure_telegram_webhook_secret().await) {
                (Some(url), Ok(secret)) => call(
                    client,
                    token,
                    "setWebhook",
                    json!({ "url": url, "secret_token": secret, "allowed_updates": ["message"], "drop_pending_updates": false }),
                )
                .await
                .map(|_| ()),
                (None, _) => Err("нет публичного адреса облака".to_string()),
                (_, Err(e)) => Err(e.to_string()),
            };
            match result {
                Ok(()) => registered = true,
                Err(e) => {
                    eprintln!("[telegram] setWebhook: {e}");
                    tokio::select! {
                        _ = cancel.cancelled() => break,
                        _ = tokio::time::sleep(Duration::from_secs(delay)) => {}
                    }
                    delay = next_delay(delay);
                    continue;
                }
            }
        }
        tokio::select! {
            _ = cancel.cancelled() => break,
            _ = tokio::time::sleep(Duration::from_secs(1)) => {}
        }
        // Вебхук могли снять снаружи (ПК с тем же токеном): раз в несколько минут сверяем и ставим заново.
        if registered && last_check.elapsed() >= recheck_interval() {
            last_check = std::time::Instant::now();
            let want = webhook_url(&settings.bot_id).unwrap_or_default();
            if let Ok(info) = call(client, token, "getWebhookInfo", json!({})).await {
                if info["url"].as_str().unwrap_or("") != want {
                    registered = false;
                }
            }
        }
    }
    if registered {
        if let Err(e) = call(client, token, "deleteWebhook", json!({ "drop_pending_updates": false })).await {
            eprintln!("[telegram] deleteWebhook: {e}");
        }
    }
}

async fn hook_worker(app: Arc<App>, mut rx: tokio::sync::mpsc::Receiver<Value>) {
    let client = client();
    while let Some(update) = rx.recv().await {
        let token = app.store.telegram_token();
        if token.is_empty() || !app.store.telegram().enabled {
            continue;
        }
        process_updates(&app, &client, &token, std::slice::from_ref(&update)).await;
    }
}

/// Приём `POST /hooks/telegram/<бот>`. Отвечает сразу, обновление разбирается отдельно.
pub(crate) fn receive(app: &Arc<App>, bot: &str, peer: &str, secret: Option<&str>, body: &[u8]) -> axum::http::StatusCode {
    use axum::http::StatusCode;
    if !crate::cloud::active() {
        return StatusCode::NOT_FOUND;
    }
    let at = now();
    let _ = peer;
    let expected = app.store.telegram_webhook_secret();
    let settings = app.store.telegram();
    // Секрет сравнивается за постоянное время, остальные проверки — после него.
    let secret_ok = !expected.is_empty() && crate::guard::same(secret.unwrap_or(""), &expected);
    let known = settings.enabled && !settings.bot_id.is_empty() && settings.bot_id == bot && !app.store.telegram_token().is_empty();
    if !(secret_ok && known) {
        // Ограничиваем только неудачи, одним общим окном: верный секрет не троттлится никогда.
        let mut fails = app.telegram.hook_fails.lock().unwrap();
        if at - fails.1 >= HOOK_FAILS_WINDOW_MS {
            *fails = (0, at);
        }
        if fails.0 >= HOOK_FAILS_MAX {
            return StatusCode::TOO_MANY_REQUESTS;
        }
        fails.0 += 1;
        return StatusCode::FORBIDDEN;
    }
    if body.len() > HOOK_BODY_MAX {
        return StatusCode::PAYLOAD_TOO_LARGE;
    }
    let Ok(update) = serde_json::from_slice::<Value>(body) else {
        return StatusCode::BAD_REQUEST;
    };
    let Some(id) = update["update_id"].as_i64() else {
        return StatusCode::BAD_REQUEST;
    };
    let start = settings.offset;
    if start > 0 && (id < start.saturating_sub(1) || id > start.saturating_add(HOOK_ID_AHEAD)) {
        return StatusCode::BAD_REQUEST;
    }
    let tx = app.telegram.hook_tx.lock().unwrap().clone();
    match tx.map(|tx| tx.try_send(update)) {
        Some(Ok(())) => StatusCode::OK,
        // Очередь полна или обработчика нет: Telegram повторит доставку.
        _ => StatusCode::SERVICE_UNAVAILABLE,
    }
}

/// Общая доставка из мессенджера: префикс в треде и отметка источника у хода и в очереди.
pub(crate) async fn deliver(app: &Arc<App>, source: &str, prefix: &str, bot_id: &str, text: &str) {
    if bot_id.trim().is_empty() || app.store.bot(bot_id).is_none() {
        return;
    }
    let message = format!("{prefix}{text}");
    let busy = app.turns.is_busy(bot_id);
    if busy || app.queues.count(bot_id) > 0 {
        match app.queues.add_with(bot_id, &message, Vec::new(), true, false, Some(source)).await {
            Ok(_) => {
                if !busy {
                    crate::turn::drain_queue(app, bot_id).await;
                }
            }
            Err(e) => eprintln!("[{source}] очередь не приняла сообщение: {e:?}"),
        }
        return;
    }
    let opts = TurnOptions { source: Some(source.into()), window: true, ..TurnOptions::default() };
    if let Err(e) = crate::turn::start_turn_with(app, bot_id, &message, Vec::new(), opts) {
        eprintln!("[{source}] не смог начать ход: {}", e.message);
    }
}

async fn reply_loop(app: Arc<App>) {
    let client = client();
    let mut events = app.turns.subscribe();
    loop {
        match events.recv().await {
            Ok(event) => match event["type"].as_str() {
                Some("turn_start") if event["source"] == "telegram" => {
                    if let Some(turn_id) = event["turnId"].as_str() {
                        app.telegram.turns.lock().unwrap().insert(turn_id.to_string());
                    }
                }
                Some("turn_end") => {
                    let Some(turn_id) = event["turnId"].as_str() else {
                        continue;
                    };
                    if !app.telegram.turns.lock().unwrap().remove(turn_id) {
                        continue;
                    }
                    let bot_id = event["botId"].as_str().unwrap_or_default().to_string();
                    let settings = app.store.settings();
                    if settings.telegram.bot_id != bot_id {
                        continue;
                    }
                    let chat_id: i64 = match settings.telegram.chat_id.parse() {
                        Ok(chat_id) if chat_id != 0 && !settings.telegram.chat_id.is_empty() => chat_id,
                        _ => continue,
                    };
                    let pending = app.turns.pending_for(&bot_id);
                    let Some(text) = turn_reply(event["text"].as_str().unwrap_or_default(), &pending) else {
                        continue;
                    };
                    let token = app.store.telegram_token();
                    if token.is_empty() {
                        continue;
                    }
                    send_text(&client, &token, chat_id, &text).await;
                }
                _ => {}
            },
            Err(RecvError::Lagged(_)) => continue,
            Err(RecvError::Closed) => return,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn codes_are_six_digits() {
        for _ in 0..200 {
            let code = make_code();
            assert_eq!(code.len(), 6, "{code}");
            assert!(code.chars().all(|c| c.is_ascii_digit()), "{code}");
        }
    }

    #[test]
    fn a_code_lives_ten_minutes() {
        let pairing = Pairing::new(1_000_000);
        assert!(pairing.accepts(&pairing.code, 1_000_000 + 599_999));
        assert!(!pairing.accepts(&pairing.code, pairing.expires_at));
        assert!(!pairing.accepts("000000", 1_000_000));
    }

    #[test]
    fn retries_back_off_up_to_a_minute() {
        assert_eq!(next_delay(1), 2);
        assert_eq!(next_delay(16), 32);
        assert_eq!(next_delay(40), 60);
        assert_eq!(next_delay(60), 60);
    }

    #[test]
    fn an_approval_card_replaces_the_final_text() {
        assert_eq!(turn_reply("готово", &[]), Some("готово".to_string()));
        assert_eq!(
            turn_reply("готово", &[json!({ "kind": "approval", "id": "ap_1" })]),
            Some("Нужно разрешение — открой Deloist".to_string())
        );
        assert_eq!(turn_reply("  \n ", &[]), None);
    }

    #[test]
    fn long_messages_are_split_and_glued_back() {
        let text: String = (0..900).map(|i| format!("строка {i}\n")).collect();
        let parts = split_text(&text, 4000);
        assert!(parts.iter().all(|part| part.chars().count() <= 4000));
        assert_eq!(parts.concat(), text);
        let per_char = split_text("абвгдеёжзий", 3);
        assert_eq!(per_char.concat(), "абвгдеёжзий");
        assert!(per_char.iter().all(|part| part.chars().count() <= 3));
    }

    fn update(chat_id: i64, text: &str) -> Value {
        json!({
            "update_id": 10,
            "message": {
                "message_id": 1,
                "chat": { "id": chat_id, "type": "private" },
                "from": { "id": chat_id, "is_bot": false },
                "text": text,
            },
        })
    }

    #[test]
    fn only_the_right_start_code_pairs_the_chat() {
        let pairing = Pairing::new(5_000);
        assert_eq!(
            classify(&update(42, "/start 123456"), "", Some(&Pairing { code: "123456".into(), expires_at: 9_000 }), 5_000),
            Incoming::Pair { chat_id: 42 }
        );
        assert_eq!(
            classify(&update(42, "/start 654321"), "", Some(&pairing), 5_000),
            Incoming::Stranger { chat_id: 42 }
        );
        assert_eq!(
            classify(&update(42, "/start 123456"), "", Some(&Pairing { code: "123456".into(), expires_at: 5_000 }), 5_000),
            Incoming::Stranger { chat_id: 42 }
        );
        assert_eq!(classify(&update(42, "/start"), "", Some(&pairing), 5_000), Incoming::Stranger { chat_id: 42 });
    }

    #[test]
    fn the_paired_chat_gets_text_stop_and_new_and_others_are_ignored() {
        let pairing = Pairing::new(5_000);
        assert_eq!(
            classify(&update(42, "привет"), "42", Some(&pairing), 5_000),
            Incoming::Text { chat_id: 42, text: "привет".into() }
        );
        assert_eq!(classify(&update(42, "/stop"), "42", Some(&pairing), 5_000), Incoming::Stop { chat_id: 42 });
        assert_eq!(classify(&update(42, "/new"), "42", Some(&pairing), 5_000), Incoming::New { chat_id: 42 });
        assert_eq!(classify(&update(42, "/unknown"), "42", Some(&pairing), 5_000), Incoming::Ignore);
        assert_eq!(
            classify(&update(77, "привет"), "42", Some(&pairing), 5_000),
            Incoming::Stranger { chat_id: 77 }
        );
    }
}
