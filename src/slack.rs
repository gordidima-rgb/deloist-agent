//! Мост в Slack: своё приложение человека, Socket Mode, привязка личных сообщений кодом и
//! переписка с выбранным ботом команды.
//!
//! Оба токена лежат только в `secrets.json`; в `state.json`, журналах и ответах API их нет.
//! Адрес Socket Mode содержит одноразовый ticket, поэтому он не попадает ни в логи, ни в ошибки:
//! в сообщениях остаются только метод и код ответа.

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use futures_util::{SinkExt, StreamExt};
use serde_json::{json, Value};
use tokio::sync::broadcast::error::RecvError;
use tokio_tungstenite::tungstenite::Message;
use tokio_util::sync::CancellationToken;

use crate::store::now;
use crate::telegram::Pairing;
use crate::App;

const API: &str = "https://slack.com/api";
/// Slack разрешает 40 000 символов, но такой текст неудобно читать в личке.
const MAX_MESSAGE_CHARS: usize = 3_500;
const STRANGER_REPLY_MS: i64 = 60 * 60 * 1000;
const PREFIX: &str = "[Slack] ";

/// Базовый адрес Slack. Подмена для тестов работает только в отладочной сборке:
/// выпущенная программа не может увести токен человека на чужой адрес.
fn api_base() -> String {
    #[cfg(debug_assertions)]
    {
        if let Ok(url) = std::env::var("CREW_TEST_SLACK_API") {
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
        .timeout(Duration::from_secs(30))
        .build()
        .unwrap_or_else(|_| reqwest::Client::new())
}

/// Ошибка Slack без адреса запроса и без заголовка с токеном.
#[derive(Debug, PartialEq)]
pub enum SlackError {
    /// `invalid_auth` или `not_authed`: токен больше не примут.
    Rejected,
    Failed(String),
}

impl SlackError {
    /// Текст для человека: в нём нет ни токена, ни адреса.
    pub fn message(&self) -> String {
        match self {
            SlackError::Rejected => "Slack отклонил токен".to_string(),
            SlackError::Failed(error) => format!("Slack не принял токен: {error}"),
        }
    }
}

/// Что Slack рассказал о своём боте и рабочем пространстве.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Identity {
    pub user_id: String,
    pub bot_id: String,
    pub team: String,
    pub username: String,
}

fn network_error(error: reqwest::Error) -> String {
    if error.is_timeout() {
        "Slack не ответил вовремя".to_string()
    } else if error.is_connect() {
        "нет связи со Slack".to_string()
    } else {
        "ошибка сети Slack".to_string()
    }
}

fn rejected(error: &str) -> bool {
    matches!(error, "invalid_auth" | "not_authed")
}

async fn call(client: &reqwest::Client, token: &str, method: &str, body: Option<Value>) -> Result<Value, SlackError> {
    let mut request = client
        .post(format!("{}/{method}", api_base()))
        .header("Authorization", format!("Bearer {token}"));
    if let Some(body) = body {
        request = request.json(&body);
    }
    let response = request.send().await.map_err(|error| SlackError::Failed(network_error(error)))?;
    let status = response.status();
    let value: Value = response
        .json()
        .await
        .map_err(|_| SlackError::Failed("ответ Slack не разобрать".to_string()))?;
    if !status.is_success() {
        return Err(SlackError::Failed(format!("HTTP {}", status.as_u16())));
    }
    if value["ok"] != true {
        let error = value["error"].as_str().unwrap_or("").trim();
        return Err(if rejected(error) { SlackError::Rejected } else { SlackError::Failed(error.to_string()) });
    }
    Ok(value)
}

/// `POST auth.test`: имя бота, его id и рабочее пространство.
pub async fn auth_test(token: &str) -> Result<Identity, SlackError> {
    let value = call(&client(), token, "auth.test", None).await?;
    Ok(Identity {
        user_id: value["user_id"].as_str().unwrap_or_default().to_string(),
        bot_id: value["bot_id"].as_str().unwrap_or_default().to_string(),
        team: value["team"].as_str().unwrap_or_default().to_string(),
        username: value["user"].as_str().unwrap_or_default().to_string(),
    })
}

/// `POST apps.connections.open`: адрес Socket Mode. Адрес одноразовый, поэтому наружу его не отдаём.
async fn connections_open(client: &reqwest::Client, app_token: &str) -> Result<String, SlackError> {
    let value = call(client, app_token, "apps.connections.open", None).await?;
    let url = value["url"].as_str().unwrap_or_default().trim();
    if url.is_empty() {
        return Err(SlackError::Failed("Slack не вернул адрес Socket Mode".to_string()));
    }
    Ok(url.to_string())
}

/// Проверка пары токенов перед сохранением: `auth.test` и `apps.connections.open`.
pub async fn check_tokens(bot_token: &str, app_token: &str) -> Result<Identity, SlackError> {
    let identity = auth_test(bot_token).await?;
    let client = client();
    connections_open(&client, app_token).await?;
    Ok(identity)
}

enum PostError {
    /// Slack попросил подождать столько секунд и повторить.
    RateLimited(f64),
    Failed(String),
}

async fn post_message(client: &reqwest::Client, token: &str, channel_id: &str, content: &str) -> Result<(), PostError> {
    let response = client
        .post(format!("{}/chat.postMessage", api_base()))
        .header("Authorization", format!("Bearer {token}"))
        .header("Content-Type", "application/json; charset=utf-8")
        .json(&json!({ "channel": channel_id, "text": content }))
        .send()
        .await
        .map_err(|error| PostError::Failed(network_error(error)))?;
    let status = response.status();
    let retry_after = response
        .headers()
        .get("retry-after")
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.trim().parse::<f64>().ok())
        .unwrap_or(1.0)
        .clamp(0.0, 60.0);
    let value: Value = response.json().await.unwrap_or(Value::Null);
    if status.as_u16() == 429 {
        return Err(PostError::RateLimited(retry_after));
    }
    if !status.is_success() || value["ok"] == false {
        let error = value["error"].as_str().unwrap_or("").trim();
        return Err(PostError::Failed(if error.is_empty() {
            format!("HTTP {}", status.as_u16())
        } else {
            error.to_string()
        }));
    }
    Ok(())
}

/// Текст в привязанное место, без хода бота. Не подключено — понятная причина; других получателей нет.
pub(crate) async fn push_text(app: &Arc<App>, text: &str) -> Result<(), String> {
    let token = app.store.slack_bot_token();
    let place = app.store.settings().slack.channel_id;
    if token.trim().is_empty() || place.trim().is_empty() {
        return Err("Slack не подключён".into());
    }
    for part in crate::telegram::split_text(text, MAX_MESSAGE_CHARS) {
        post_message(&client(), &token, &place, &part).await.map_err(|_| "Slack не принял сообщение".to_string())?;
    }
    Ok(())
}

/// Отправка с делением на части по 3500 символов и одним повтором после 429.
async fn send_text(client: &reqwest::Client, token: &str, channel_id: &str, text: &str) {
    for part in crate::telegram::split_text(text, MAX_MESSAGE_CHARS) {
        let mut retried = false;
        loop {
            match post_message(client, token, channel_id, &part).await {
                Ok(()) => break,
                Err(PostError::RateLimited(seconds)) => {
                    if retried {
                        return;
                    }
                    retried = true;
                    tokio::time::sleep(Duration::from_secs_f64(seconds)).await;
                }
                Err(PostError::Failed(error)) => {
                    eprintln!("[slack] сообщение: {error}");
                    return;
                }
            }
        }
    }
}

/// Что делать с одним событием `message`. Разбор отделён от сети: его проверяют тесты.
#[derive(Debug, PartialEq)]
pub enum Incoming {
    Pair { user_id: String, channel_id: String },
    Text { user_id: String, channel_id: String, text: String },
    Stop { user_id: String, channel_id: String },
    New { user_id: String, channel_id: String },
    /// Человек не привязан: один ответ в час, чтобы не заваливать чужого человека.
    Stranger { user_id: String, channel_id: String },
    Ignore,
}

/// `start <код>` или `/start <код>`: в личных сообщениях Slack слэш-команда тоже приходит текстом.
fn start_code(text: &str) -> Option<&str> {
    let rest = text.strip_prefix("start").or_else(|| text.strip_prefix("/start"))?;
    if !rest.chars().next().is_some_and(char::is_whitespace) {
        return None;
    }
    let code = rest.trim();
    (!code.is_empty() && !code.chars().any(char::is_whitespace)).then_some(code)
}

pub fn classify(event: &Value, paired_user: &str, pairing: Option<&Pairing>, at: i64) -> Incoming {
    if event["type"].as_str() != Some("message") {
        return Incoming::Ignore;
    }
    // Мост отвечает только в личке: события каналов и групп не наши.
    if event["channel_type"].as_str() != Some("im") {
        return Incoming::Ignore;
    }
    // Правки, ответы ботов и прочие подтипы не считаются сообщением человека.
    if event.get("subtype").is_some_and(|subtype| !subtype.is_null()) {
        return Incoming::Ignore;
    }
    if event.get("bot_id").is_some_and(|bot_id| !bot_id.is_null() && bot_id.as_str() != Some("")) {
        return Incoming::Ignore;
    }
    let Some(user_id) = event.get("user").and_then(Value::as_str).filter(|id| !id.is_empty()) else {
        return Incoming::Ignore;
    };
    let Some(channel_id) = event.get("channel").and_then(Value::as_str).filter(|id| !id.is_empty()) else {
        return Incoming::Ignore;
    };
    let Some(text) = event.get("text").and_then(Value::as_str).map(str::trim) else {
        return Incoming::Ignore;
    };
    if text.is_empty() {
        return Incoming::Ignore;
    }
    if let Some(code) = start_code(text) {
        if pairing.is_some_and(|pairing| pairing.accepts(code, at)) {
            return Incoming::Pair { user_id: user_id.to_string(), channel_id: channel_id.to_string() };
        }
    }
    if !paired_user.is_empty() && user_id == paired_user {
        return match text {
            "stop" | "/stop" => Incoming::Stop { user_id: user_id.to_string(), channel_id: channel_id.to_string() },
            "new" | "/new" => Incoming::New { user_id: user_id.to_string(), channel_id: channel_id.to_string() },
            _ if text.starts_with('/') => Incoming::Ignore,
            _ => Incoming::Text {
                user_id: user_id.to_string(),
                channel_id: channel_id.to_string(),
                text: text.to_string(),
            },
        };
    }
    Incoming::Stranger { user_id: user_id.to_string(), channel_id: channel_id.to_string() }
}

/// Подтверждение конверта: Slack ждёт его сразу после получения события.
pub fn ack(envelope_id: &str) -> String {
    json!({ "envelope_id": envelope_id }).to_string()
}

#[derive(Default)]
pub struct Slack {
    code: Mutex<Option<Pairing>>,
    username: Mutex<String>,
    /// Ходы, начатые из Slack: по ним `turn_end` возвращает ответ в личку.
    turns: Mutex<HashSet<String>>,
    /// Кому уже отвечали «этот бот не подключён» и когда.
    warned: Mutex<HashMap<String, i64>>,
    /// Текущее соединение Socket Mode: `DELETE /api/slack` закрывает его сразу.
    socket: Mutex<Option<CancellationToken>>,
}

impl Slack {
    pub fn code(&self) -> Option<Pairing> {
        self.code.lock().unwrap().clone()
    }

    pub fn set_code(&self, pairing: Pairing) {
        *self.code.lock().unwrap() = Some(pairing);
    }

    pub fn clear_code(&self) {
        *self.code.lock().unwrap() = None;
    }

    pub fn set_username(&self, username: String) {
        *self.username.lock().unwrap() = username;
    }

    /// Забыть всё, что относится к мосту, и закрыть сокет.
    pub fn forget(&self) {
        self.clear_code();
        self.username.lock().unwrap().clear();
        self.turns.lock().unwrap().clear();
        self.warned.lock().unwrap().clear();
        if let Some(cancel) = self.socket.lock().unwrap().take() {
            cancel.cancel();
        }
    }

    /// Имя бота Slack для ответа `GET /api/slack`; ошибка сети не мешает остальным полям.
    pub async fn username(&self, token: &str) -> String {
        let cached = self.username.lock().unwrap().clone();
        if !cached.is_empty() {
            return cached;
        }
        match auth_test(token).await {
            Ok(identity) => {
                let name = if identity.username.is_empty() { identity.bot_id } else { identity.username };
                self.set_username(name.clone());
                name
            }
            Err(e) => {
                eprintln!("[slack] auth.test: {}", e.message());
                String::new()
            }
        }
    }

    fn should_warn(&self, user_id: &str, at: i64) -> bool {
        let mut warned = self.warned.lock().unwrap();
        match warned.get(user_id) {
            Some(last) if at - last < STRANGER_REPLY_MS => false,
            _ => {
                warned.insert(user_id.to_string(), at);
                true
            }
        }
    }
}

/// Следующая пауза перед повтором: не больше минуты.
fn next_delay(delay: u64) -> u64 {
    (delay * 2).min(60)
}

enum SessionEnd {
    Stop,
    Reconnect { ready: bool },
    /// Slack ответил `invalid_auth` или `not_authed`: токен повторять бессмысленно.
    Rejected,
}

/// Запуск обоих циклов: Socket Mode и ответы на конец хода.
pub fn start(app: Arc<App>) {
    let replies = app.clone();
    tokio::spawn(async move { reply_loop(replies).await });
    tokio::spawn(async move { socket_loop(app).await });
}

async fn socket_loop(app: Arc<App>) {
    loop {
        let bot_token = app.store.slack_bot_token();
        let app_token = app.store.slack_app_token();
        let settings = app.store.slack();
        if bot_token.is_empty() || app_token.is_empty() || !settings.enabled {
            tokio::time::sleep(Duration::from_secs(1)).await;
            continue;
        }
        let cancel = CancellationToken::new();
        *app.slack.socket.lock().unwrap() = Some(cancel.clone());
        let mut delay = 1u64;
        loop {
            let settings = app.store.slack();
            if app.store.slack_bot_token() != bot_token || app.store.slack_app_token() != app_token || !settings.enabled {
                break;
            }
            let end = tokio::select! {
                _ = cancel.cancelled() => break,
                end = session(&app, &bot_token, &app_token, &cancel) => end,
            };
            match end {
                SessionEnd::Stop => break,
                SessionEnd::Rejected => {
                    eprintln!("[slack] Slack отклонил токен");
                    if let Err(e) = app.store.update_slack(|slack| slack.enabled = false).await {
                        eprintln!("[slack] не смог выключить мост: {e}");
                    }
                    break;
                }
                SessionEnd::Reconnect { ready } => {
                    if ready {
                        delay = 1;
                    }
                    tokio::select! {
                        _ = cancel.cancelled() => break,
                        _ = tokio::time::sleep(Duration::from_secs(delay)) => {}
                    }
                    delay = next_delay(delay);
                }
            }
        }
        *app.slack.socket.lock().unwrap() = None;
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
}

/// Одно соединение: открыть адрес через `apps.connections.open`, дождаться `hello`, подтверждать
/// конверты и разбирать личные сообщения. При разрыве вызывающий подключится снова.
async fn session(app: &Arc<App>, bot_token: &str, app_token: &str, cancel: &CancellationToken) -> SessionEnd {
    let client = client();
    let url = match tokio::select! {
        _ = cancel.cancelled() => return SessionEnd::Stop,
        result = connections_open(&client, app_token) => result,
    } {
        Ok(url) => url,
        Err(SlackError::Rejected) => return SessionEnd::Rejected,
        Err(SlackError::Failed(e)) => {
            eprintln!("[slack] apps.connections.open: {e}");
            return SessionEnd::Reconnect { ready: false };
        }
    };
    let (socket, _) = match tokio::select! {
        _ = cancel.cancelled() => return SessionEnd::Stop,
        result = tokio_tungstenite::connect_async(url.as_str()) => result,
    } {
        Ok(pair) => pair,
        Err(_) => {
            // Ошибка соединения может содержать адрес с одноразовым ticket: только факт.
            eprintln!("[slack] не удалось открыть соединение Socket Mode");
            return SessionEnd::Reconnect { ready: false };
        }
    };
    let (mut sink, mut stream) = socket.split();
    let mut ready = false;
    loop {
        tokio::select! {
            _ = cancel.cancelled() => {
                let _ = sink.send(Message::Close(None)).await;
                return SessionEnd::Stop;
            }
            incoming = stream.next() => {
                let Some(message) = incoming else {
                    return SessionEnd::Reconnect { ready };
                };
                let message = match message {
                    Ok(message) => message,
                    Err(_) => return SessionEnd::Reconnect { ready },
                };
                let text = match message {
                    Message::Text(text) => text.as_str().to_string(),
                    Message::Binary(bytes) => match String::from_utf8(bytes.to_vec()) {
                        Ok(text) => text,
                        Err(_) => continue,
                    },
                    Message::Close(_) => return SessionEnd::Reconnect { ready },
                    Message::Ping(payload) => {
                        let _ = sink.send(Message::Pong(payload)).await;
                        continue;
                    }
                    _ => continue,
                };
                let Ok(value) = serde_json::from_str::<Value>(&text) else {
                    continue;
                };
                // Подтверждаем любой конверт сразу, ещё до разбора события.
                if let Some(envelope_id) = value["envelope_id"].as_str() {
                    if sink.send(Message::text(ack(envelope_id))).await.is_err() {
                        return SessionEnd::Reconnect { ready };
                    }
                }
                match value["type"].as_str() {
                    Some("hello") => ready = true,
                    Some("disconnect") => return SessionEnd::Reconnect { ready: true },
                    Some("events_api") => {
                        if let Some(event) = value.get("payload").and_then(|payload| payload.get("event")) {
                            handle_event(app, &client, bot_token, event).await;
                        }
                    }
                    _ => {}
                }
            }
        }
    }
}

async fn handle_event(app: &Arc<App>, client: &reqwest::Client, bot_token: &str, event: &Value) {
    let settings = app.store.settings();
    let pairing = app.slack.code();
    let at = now();
    let bot_id = settings.slack.bot_id.clone();
    match classify(event, &settings.slack.user_id, pairing.as_ref(), at) {
        Incoming::Ignore => {}
        Incoming::Stranger { user_id, channel_id } => {
            if app.slack.should_warn(&user_id, at) {
                send_text(client, bot_token, &channel_id, "Этот бот не подключён к вашему Deloist").await;
            }
        }
        Incoming::Pair { user_id, channel_id } => {
            if let Err(e) = app.store.update_slack(|slack| {
                slack.user_id = user_id;
                slack.channel_id = channel_id.clone();
                slack.enabled = true;
            }).await {
                eprintln!("[slack] не смог сохранить человека: {e}");
                return;
            }
            app.slack.clear_code();
            let name = app.store.bot(&bot_id).map(|bot| bot.name).unwrap_or_default();
            let text = if name.is_empty() {
                "Готово: пишите сюда".to_string()
            } else {
                format!("Готово: пишите сюда, отвечает {name}")
            };
            send_text(client, bot_token, &channel_id, &text).await;
        }
        Incoming::Text { text, channel_id, .. } => {
            crate::workflow_run::on_message(app, "slack", &channel_id, &text);
            crate::telegram::deliver(app, "slack", PREFIX, &bot_id, &text).await;
        }
        Incoming::Stop { .. } => {
            if bot_id.is_empty() {
                return;
            }
            // Сначала пауза, потом отмена: как у кнопки остановки в окне.
            let _ = app.queues.pause(&bot_id).await;
            app.turns.stop_turn(&bot_id);
        }
        Incoming::New { channel_id, .. } => {
            let Some(bot) = app.store.bot(&bot_id) else {
                return;
            };
            if app.turns.is_busy(&bot_id) {
                send_text(client, bot_token, &channel_id, "Бот сейчас отвечает: сожми историю, когда он закончит").await;
                return;
            }
            let text = match crate::compact::bot_thread(app, &bot, true).await {
                Some(_) => "История сжата.",
                None => "Сжимать пока нечего.",
            };
            send_text(client, bot_token, &channel_id, text).await;
        }
    }
}

async fn reply_loop(app: Arc<App>) {
    let client = client();
    let mut events = app.turns.subscribe();
    loop {
        match events.recv().await {
            Ok(event) => match event["type"].as_str() {
                Some("turn_start") if event["source"] == "slack" => {
                    if let Some(turn_id) = event["turnId"].as_str() {
                        app.slack.turns.lock().unwrap().insert(turn_id.to_string());
                    }
                }
                Some("turn_end") => {
                    let Some(turn_id) = event["turnId"].as_str() else {
                        continue;
                    };
                    if !app.slack.turns.lock().unwrap().remove(turn_id) {
                        continue;
                    }
                    let bot_id = event["botId"].as_str().unwrap_or_default().to_string();
                    let settings = app.store.settings();
                    if settings.slack.bot_id != bot_id {
                        continue;
                    }
                    let channel_id = settings.slack.channel_id;
                    if channel_id.is_empty() {
                        continue;
                    }
                    let pending = app.turns.pending_for(&bot_id);
                    let Some(text) = crate::telegram::turn_reply(event["text"].as_str().unwrap_or_default(), &pending) else {
                        continue;
                    };
                    let token = app.store.slack_bot_token();
                    if token.is_empty() {
                        continue;
                    }
                    send_text(&client, &token, &channel_id, &text).await;
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

    fn event(user_id: &str, channel_id: &str, text: &str) -> Value {
        json!({
            "type": "message",
            "channel_type": "im",
            "channel": channel_id,
            "user": user_id,
            "text": text,
        })
    }

    #[test]
    fn retries_back_off_up_to_a_minute() {
        assert_eq!(next_delay(1), 2);
        assert_eq!(next_delay(16), 32);
        assert_eq!(next_delay(40), 60);
        assert_eq!(next_delay(60), 60);
    }

    #[test]
    fn an_envelope_ack_echoes_its_id() {
        let value: Value = serde_json::from_str(&ack("env-1")).unwrap();
        assert_eq!(value, json!({ "envelope_id": "env-1" }));
    }

    #[test]
    fn only_the_right_start_code_pairs_the_user() {
        let pairing = Pairing::new(5_000);
        assert_eq!(
            classify(&event("42", "D1", "start 123456"), "", Some(&Pairing { code: "123456".into(), expires_at: 9_000 }), 5_000),
            Incoming::Pair { user_id: "42".into(), channel_id: "D1".into() }
        );
        assert_eq!(
            classify(&event("42", "D1", "/start 123456"), "", Some(&Pairing { code: "123456".into(), expires_at: 9_000 }), 5_000),
            Incoming::Pair { user_id: "42".into(), channel_id: "D1".into() }
        );
        assert_eq!(
            classify(&event("42", "D1", "start 654321"), "", Some(&pairing), 5_000),
            Incoming::Stranger { user_id: "42".into(), channel_id: "D1".into() }
        );
        assert_eq!(
            classify(&event("42", "D1", "start 123456"), "", Some(&Pairing { code: "123456".into(), expires_at: 5_000 }), 5_000),
            Incoming::Stranger { user_id: "42".into(), channel_id: "D1".into() }
        );
        assert_eq!(
            classify(&event("42", "D1", "start"), "", Some(&pairing), 5_000),
            Incoming::Stranger { user_id: "42".into(), channel_id: "D1".into() }
        );
    }

    #[test]
    fn bot_messages_edits_and_channel_messages_are_ignored() {
        let pairing = Pairing::new(5_000);
        let mut bot = event("42", "D1", "привет");
        bot["bot_id"] = json!("B1");
        assert_eq!(classify(&bot, "", Some(&pairing), 5_000), Incoming::Ignore);

        let mut edited = event("42", "D1", "привет");
        edited["subtype"] = json!("message_changed");
        assert_eq!(classify(&edited, "", Some(&pairing), 5_000), Incoming::Ignore);

        let mut channel = event("42", "C1", "привет");
        channel["channel_type"] = json!("channel");
        assert_eq!(classify(&channel, "", Some(&pairing), 5_000), Incoming::Ignore);

        let mut other = event("42", "D1", "привет");
        other["type"] = json!("reaction_added");
        assert_eq!(classify(&other, "", Some(&pairing), 5_000), Incoming::Ignore);
    }

    #[test]
    fn the_paired_user_gets_text_stop_and_new_and_others_are_strangers() {
        let pairing = Pairing::new(5_000);
        assert_eq!(
            classify(&event("42", "D1", "привет"), "42", Some(&pairing), 5_000),
            Incoming::Text { user_id: "42".into(), channel_id: "D1".into(), text: "привет".into() }
        );
        assert_eq!(
            classify(&event("42", "D1", "stop"), "42", Some(&pairing), 5_000),
            Incoming::Stop { user_id: "42".into(), channel_id: "D1".into() }
        );
        assert_eq!(
            classify(&event("42", "D1", "/new"), "42", Some(&pairing), 5_000),
            Incoming::New { user_id: "42".into(), channel_id: "D1".into() }
        );
        assert_eq!(classify(&event("42", "D1", "/unknown"), "42", Some(&pairing), 5_000), Incoming::Ignore);
        assert_eq!(
            classify(&event("77", "D2", "привет"), "42", Some(&pairing), 5_000),
            Incoming::Stranger { user_id: "77".into(), channel_id: "D2".into() }
        );
    }
}
