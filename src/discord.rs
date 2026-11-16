//! Мост в Discord: свой бот человека, привязка личных сообщений кодом и переписка с выбранным
//! ботом команды.
//!
//! Токен лежит только в `secrets.json`; в `state.json`, журналах и ответах API его нет. Адрес
//! Gateway не содержит токен: он уходит только в кадре Identify. Поэтому в сообщения об ошибках
//! попадают метод и код ответа, но никогда заголовок авторизации.

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use futures_util::{SinkExt, StreamExt};
use serde_json::{json, Value};
use tokio::sync::broadcast::error::RecvError;
use tokio::time::Instant;
use tokio_tungstenite::tungstenite::Message;
use tokio_util::sync::CancellationToken;

use crate::store::now;
use crate::telegram::Pairing;
use crate::App;

const API: &str = "https://discord.com/api/v10";
const GATEWAY: &str = "wss://gateway.discord.gg/?v=10&encoding=json";
/// DIRECT_MESSAGES: в личных сообщениях текст приходит без привилегированного intent.
const INTENTS: i64 = 4096;
const MAX_MESSAGE_CHARS: usize = 2000;
const STRANGER_REPLY_MS: i64 = 60 * 60 * 1000;
const PREFIX: &str = "[Discord] ";

/// Базовый адрес Discord. Подмена для тестов работает только в отладочной сборке:
/// выпущенная программа не может увести токен человека на чужой адрес.
fn api_base() -> String {
    #[cfg(debug_assertions)]
    {
        if let Ok(url) = std::env::var("CREW_TEST_DISCORD_API") {
            let url = url.trim().trim_end_matches('/');
            if !url.is_empty() {
                return url.to_string();
            }
        }
    }
    API.to_string()
}

/// Адрес Gateway. Подмена для тестов тоже только в отладочной сборке.
fn gateway_url() -> String {
    #[cfg(debug_assertions)]
    {
        if let Ok(url) = std::env::var("CREW_TEST_DISCORD_GATEWAY") {
            let url = url.trim();
            if !url.is_empty() {
                return url.to_string();
            }
        }
    }
    GATEWAY.to_string()
}

fn client() -> reqwest::Client {
    reqwest::Client::builder()
        .timeout(Duration::from_secs(30))
        .build()
        .unwrap_or_else(|_| reqwest::Client::new())
}

/// Ошибка Discord без адреса запроса и без заголовка с токеном.
fn network_error(error: reqwest::Error) -> String {
    if error.is_timeout() {
        "Discord не ответил вовремя".to_string()
    } else if error.is_connect() {
        "нет связи с Discord".to_string()
    } else {
        "ошибка сети Discord".to_string()
    }
}

/// Проверка токена: `GET /users/@me` отдаёт имя бота.
pub async fn fetch_username(token: &str) -> Result<String, String> {
    let response = client()
        .get(format!("{}/users/@me", api_base()))
        .header("Authorization", format!("Bot {token}"))
        .send()
        .await
        .map_err(network_error)?;
    let status = response.status();
    let value: Value = response.json().await.map_err(|_| "ответ Discord не разобрать".to_string())?;
    if !status.is_success() {
        return Err(format!("HTTP {}", status.as_u16()));
    }
    Ok(value["username"].as_str().unwrap_or_default().to_string())
}

enum PostError {
    /// Discord попросил подождать столько секунд и повторить.
    RateLimited(f64),
    Failed(String),
}

async fn post_message(client: &reqwest::Client, token: &str, channel_id: &str, content: &str) -> Result<(), PostError> {
    let response = client
        .post(format!("{}/channels/{channel_id}/messages", api_base()))
        .header("Authorization", format!("Bot {token}"))
        .json(&json!({ "content": content }))
        .send()
        .await
        .map_err(|error| PostError::Failed(network_error(error)))?;
    let status = response.status();
    let value: Value = response.json().await.unwrap_or(Value::Null);
    if status.as_u16() == 429 {
        return Err(PostError::RateLimited(value["retry_after"].as_f64().unwrap_or(1.0).clamp(0.0, 60.0)));
    }
    if !status.is_success() {
        return Err(PostError::Failed(format!("HTTP {}", status.as_u16())));
    }
    Ok(())
}

/// Отправка с делением на части по 2000 символов и одним повтором после 429.
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
                    eprintln!("[discord] сообщение: {error}");
                    return;
                }
            }
        }
    }
}

/// Что делать с одним `MESSAGE_CREATE`. Разбор отделён от сети: его проверяют тесты.
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

/// `start <код>` или `/start <код>`: в Discord команда без слэша тоже обычный текст.
fn start_code(text: &str) -> Option<&str> {
    let rest = text.strip_prefix("start").or_else(|| text.strip_prefix("/start"))?;
    if !rest.chars().next().is_some_and(char::is_whitespace) {
        return None;
    }
    let code = rest.trim();
    (!code.is_empty() && !code.chars().any(char::is_whitespace)).then_some(code)
}

pub fn classify(message: &Value, paired_user: &str, pairing: Option<&Pairing>, at: i64) -> Incoming {
    // В канале сервера сообщение адресовано не человеку: мост отвечает только в личку.
    if message.get("guild_id").is_some_and(|guild| !guild.is_null()) {
        return Incoming::Ignore;
    }
    let Some(author) = message.get("author") else {
        return Incoming::Ignore;
    };
    if author.get("bot").and_then(Value::as_bool) == Some(true) {
        return Incoming::Ignore;
    }
    let Some(user_id) = author.get("id").and_then(Value::as_str).filter(|id| !id.is_empty()) else {
        return Incoming::Ignore;
    };
    let Some(channel_id) = message.get("channel_id").and_then(Value::as_str).filter(|id| !id.is_empty()) else {
        return Incoming::Ignore;
    };
    let Some(text) = message.get("content").and_then(Value::as_str).map(str::trim) else {
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

#[derive(Default)]
pub struct Discord {
    code: Mutex<Option<Pairing>>,
    username: Mutex<String>,
    /// Ходы, начатые из Discord: по ним `turn_end` возвращает ответ в личку.
    turns: Mutex<HashSet<String>>,
    /// Кому уже отвечали «этот бот не подключён» и когда.
    warned: Mutex<HashMap<String, i64>>,
    /// Текущее соединение с Gateway: `DELETE /api/discord` закрывает его сразу.
    gateway: Mutex<Option<CancellationToken>>,
}

impl Discord {
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

    /// Забыть всё, что относится к мосту, и закрыть Gateway.
    pub fn forget(&self) {
        self.clear_code();
        self.username.lock().unwrap().clear();
        self.turns.lock().unwrap().clear();
        self.warned.lock().unwrap().clear();
        if let Some(cancel) = self.gateway.lock().unwrap().take() {
            cancel.cancel();
        }
    }

    /// Имя бота Discord для ответа `GET /api/discord`; ошибка сети не мешает остальным полям.
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
                eprintln!("[discord] users/@me: {e}");
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

/// Коды закрытия, после которых токен повторять бессмысленно.
fn rejected_close(code: u16) -> bool {
    matches!(code, 4004 | 4014)
}

enum SessionEnd {
    Stop,
    Reconnect { ready: bool },
    /// Discord закрыл соединение кодом 4004 или 4014: токен неверный.
    Rejected,
}

/// Запуск обоих циклов: Gateway и ответы на конец хода.
pub fn start(app: Arc<App>) {
    let replies = app.clone();
    tokio::spawn(async move { reply_loop(replies).await });
    tokio::spawn(async move { gateway_loop(app).await });
}

async fn gateway_loop(app: Arc<App>) {
    loop {
        let token = app.store.discord_token();
        let settings = app.store.discord();
        if token.is_empty() || !settings.enabled {
            tokio::time::sleep(Duration::from_secs(1)).await;
            continue;
        }
        let cancel = CancellationToken::new();
        *app.discord.gateway.lock().unwrap() = Some(cancel.clone());
        let mut delay = 1u64;
        loop {
            let settings = app.store.discord();
            if app.store.discord_token() != token || !settings.enabled {
                break;
            }
            let end = tokio::select! {
                _ = cancel.cancelled() => break,
                end = session(&app, &token, &cancel) => end,
            };
            match end {
                SessionEnd::Stop => break,
                SessionEnd::Rejected => {
                    eprintln!("[discord] Discord отклонил токен");
                    if let Err(e) = app.store.update_discord(|discord| discord.enabled = false).await {
                        eprintln!("[discord] не смог выключить мост: {e}");
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
        *app.discord.gateway.lock().unwrap() = None;
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
}

/// Одно соединение: hello, Identify, heartbeat и события. При разрыве вызывающий подключится снова.
async fn session(app: &Arc<App>, token: &str, cancel: &CancellationToken) -> SessionEnd {
    let (socket, _) = match tokio::select! {
        _ = cancel.cancelled() => return SessionEnd::Stop,
        result = tokio_tungstenite::connect_async(gateway_url()) => result,
    } {
        Ok(pair) => pair,
        Err(e) => {
            eprintln!("[discord] gateway: {e}");
            return SessionEnd::Reconnect { ready: false };
        }
    };
    let (mut sink, mut stream) = socket.split();
    let client = client();
    let mut seq: Option<i64> = None;
    let mut beat_every: Option<Duration> = None;
    let mut next_beat = Instant::now() + Duration::from_secs(86_400);
    let mut ready = false;
    loop {
        tokio::select! {
            _ = cancel.cancelled() => {
                let _ = sink.send(Message::Close(None)).await;
                return SessionEnd::Stop;
            }
            _ = tokio::time::sleep_until(next_beat), if beat_every.is_some() => {
                let beat = json!({ "op": 1, "d": seq });
                if sink.send(Message::text(beat.to_string())).await.is_err() {
                    return SessionEnd::Reconnect { ready };
                }
                if let Some(every) = beat_every {
                    next_beat = Instant::now() + every;
                }
            }
            incoming = stream.next() => {
                let Some(message) = incoming else {
                    return SessionEnd::Reconnect { ready };
                };
                let message = match message {
                    Ok(message) => message,
                    Err(e) => {
                        eprintln!("[discord] gateway: {e}");
                        return SessionEnd::Reconnect { ready };
                    }
                };
                let text = match message {
                    Message::Text(text) => text.as_str().to_string(),
                    Message::Binary(bytes) => match String::from_utf8(bytes.to_vec()) {
                        Ok(text) => text,
                        Err(_) => continue,
                    },
                    Message::Close(frame) => {
                        if frame.as_ref().is_some_and(|frame| rejected_close(u16::from(frame.code))) {
                            return SessionEnd::Rejected;
                        }
                        return SessionEnd::Reconnect { ready };
                    }
                    Message::Ping(payload) => {
                        let _ = sink.send(Message::Pong(payload)).await;
                        continue;
                    }
                    _ => continue,
                };
                let Ok(value) = serde_json::from_str::<Value>(&text) else {
                    continue;
                };
                match value["op"].as_i64() {
                    Some(10) => {
                        let interval = value["d"]["heartbeat_interval"].as_u64().unwrap_or(41_250).max(100);
                        beat_every = Some(Duration::from_millis(interval));
                        next_beat = Instant::now() + Duration::from_millis(interval);
                        let identify = json!({
                            "op": 2,
                            "d": {
                                "token": token,
                                "intents": INTENTS,
                                "properties": {
                                    "os": "windows",
                                    "browser": "deloist",
                                    "device": "deloist",
                                },
                            },
                        });
                        if sink.send(Message::text(identify.to_string())).await.is_err() {
                            return SessionEnd::Reconnect { ready };
                        }
                    }
                    Some(1) => {
                        let beat = json!({ "op": 1, "d": seq });
                        if sink.send(Message::text(beat.to_string())).await.is_err() {
                            return SessionEnd::Reconnect { ready };
                        }
                    }
                    Some(7) | Some(9) => return SessionEnd::Reconnect { ready },
                    Some(0) => {
                        if let Some(sequence) = value["s"].as_i64() {
                            seq = Some(sequence);
                        }
                        match value["t"].as_str() {
                            Some("READY") => ready = true,
                            Some("MESSAGE_CREATE") => {
                                if let Some(message) = value.get("d") {
                                    handle_message(app, &client, token, message).await;
                                }
                            }
                            _ => {}
                        }
                    }
                    _ => {}
                }
            }
        }
    }
}

async fn handle_message(app: &Arc<App>, client: &reqwest::Client, token: &str, message: &Value) {
    let settings = app.store.settings();
    let pairing = app.discord.code();
    let at = now();
    let bot_id = settings.discord.bot_id.clone();
    match classify(message, &settings.discord.user_id, pairing.as_ref(), at) {
        Incoming::Ignore => {}
        Incoming::Stranger { user_id, channel_id } => {
            if app.discord.should_warn(&user_id, at) {
                send_text(client, token, &channel_id, "Этот бот не подключён к вашему Deloist").await;
            }
        }
        Incoming::Pair { user_id, channel_id } => {
            if let Err(e) = app.store.update_discord(|discord| {
                discord.user_id = user_id;
                discord.channel_id = channel_id.clone();
                discord.enabled = true;
            }).await {
                eprintln!("[discord] не смог сохранить человека: {e}");
                return;
            }
            app.discord.clear_code();
            let name = app.store.bot(&bot_id).map(|bot| bot.name).unwrap_or_default();
            let text = if name.is_empty() {
                "Готово: пишите сюда".to_string()
            } else {
                format!("Готово: пишите сюда, отвечает {name}")
            };
            send_text(client, token, &channel_id, &text).await;
        }
        Incoming::Text { text, .. } => {
            crate::telegram::deliver(app, "discord", PREFIX, &bot_id, &text).await;
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
                send_text(client, token, &channel_id, "Бот сейчас отвечает: сожми историю, когда он закончит").await;
                return;
            }
            let text = match crate::compact::bot_thread(app, &bot, true).await {
                Some(_) => "История сжата.",
                None => "Сжимать пока нечего.",
            };
            send_text(client, token, &channel_id, text).await;
        }
    }
}

async fn reply_loop(app: Arc<App>) {
    let client = client();
    let mut events = app.turns.subscribe();
    loop {
        match events.recv().await {
            Ok(event) => match event["type"].as_str() {
                Some("turn_start") if event["source"] == "discord" => {
                    if let Some(turn_id) = event["turnId"].as_str() {
                        app.discord.turns.lock().unwrap().insert(turn_id.to_string());
                    }
                }
                Some("turn_end") => {
                    let Some(turn_id) = event["turnId"].as_str() else {
                        continue;
                    };
                    if !app.discord.turns.lock().unwrap().remove(turn_id) {
                        continue;
                    }
                    let bot_id = event["botId"].as_str().unwrap_or_default().to_string();
                    let settings = app.store.settings();
                    if settings.discord.bot_id != bot_id {
                        continue;
                    }
                    let channel_id = settings.discord.channel_id;
                    if channel_id.is_empty() {
                        continue;
                    }
                    let pending = app.turns.pending_for(&bot_id);
                    let Some(text) = crate::telegram::turn_reply(event["text"].as_str().unwrap_or_default(), &pending) else {
                        continue;
                    };
                    let token = app.store.discord_token();
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

    fn message(user_id: &str, channel_id: &str, text: &str) -> Value {
        json!({
            "id": "1",
            "channel_id": channel_id,
            "author": { "id": user_id, "username": "человек" },
            "content": text,
        })
    }

    fn guild_message(user_id: &str, channel_id: &str, text: &str) -> Value {
        let mut value = message(user_id, channel_id, text);
        value["guild_id"] = json!("999");
        value
    }

    #[test]
    fn retries_back_off_up_to_a_minute() {
        assert_eq!(next_delay(1), 2);
        assert_eq!(next_delay(16), 32);
        assert_eq!(next_delay(40), 60);
        assert_eq!(next_delay(60), 60);
    }

    #[test]
    fn bad_token_close_codes_stop_the_loop() {
        assert!(rejected_close(4004));
        assert!(rejected_close(4014));
        assert!(!rejected_close(1000));
        assert!(!rejected_close(4000));
    }

    #[test]
    fn only_the_right_start_code_pairs_the_user() {
        let pairing = Pairing::new(5_000);
        assert_eq!(
            classify(&message("42", "555", "start 123456"), "", Some(&Pairing { code: "123456".into(), expires_at: 9_000 }), 5_000),
            Incoming::Pair { user_id: "42".into(), channel_id: "555".into() }
        );
        assert_eq!(
            classify(&message("42", "555", "/start 123456"), "", Some(&Pairing { code: "123456".into(), expires_at: 9_000 }), 5_000),
            Incoming::Pair { user_id: "42".into(), channel_id: "555".into() }
        );
        assert_eq!(
            classify(&message("42", "555", "start 654321"), "", Some(&pairing), 5_000),
            Incoming::Stranger { user_id: "42".into(), channel_id: "555".into() }
        );
        assert_eq!(
            classify(&message("42", "555", "start 123456"), "", Some(&Pairing { code: "123456".into(), expires_at: 5_000 }), 5_000),
            Incoming::Stranger { user_id: "42".into(), channel_id: "555".into() }
        );
        assert_eq!(
            classify(&message("42", "555", "start"), "", Some(&pairing), 5_000),
            Incoming::Stranger { user_id: "42".into(), channel_id: "555".into() }
        );
    }

    #[test]
    fn guild_messages_and_bot_authors_are_ignored() {
        let pairing = Pairing::new(5_000);
        assert_eq!(classify(&guild_message("42", "555", "start 123456"), "", Some(&pairing), 5_000), Incoming::Ignore);
        let mut bot = message("42", "555", "привет");
        bot["author"]["bot"] = json!(true);
        assert_eq!(classify(&bot, "42", Some(&pairing), 5_000), Incoming::Ignore);
    }

    #[test]
    fn the_paired_user_gets_text_stop_and_new_and_others_are_ignored() {
        let pairing = Pairing::new(5_000);
        assert_eq!(
            classify(&message("42", "555", "привет"), "42", Some(&pairing), 5_000),
            Incoming::Text { user_id: "42".into(), channel_id: "555".into(), text: "привет".into() }
        );
        assert_eq!(
            classify(&message("42", "555", "stop"), "42", Some(&pairing), 5_000),
            Incoming::Stop { user_id: "42".into(), channel_id: "555".into() }
        );
        assert_eq!(
            classify(&message("42", "555", "/new"), "42", Some(&pairing), 5_000),
            Incoming::New { user_id: "42".into(), channel_id: "555".into() }
        );
        assert_eq!(classify(&message("42", "555", "/unknown"), "42", Some(&pairing), 5_000), Incoming::Ignore);
        assert_eq!(
            classify(&message("77", "888", "привет"), "42", Some(&pairing), 5_000),
            Incoming::Stranger { user_id: "77".into(), channel_id: "888".into() }
        );
    }
}
