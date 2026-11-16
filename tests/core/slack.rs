//! Мост Slack: оба токена только в секретах, привязка личной переписки кодом, Socket Mode с
//! подтверждением конвертов, переписка с выбранным ботом и ответ, который возвращается в личку
//! после конца хода.
//!
//! Вместо slack.com — маленький REST-сервер, а вместо Socket Mode — локальный сокет: он присылает
//! `hello`, события и `disconnect`, запоминает подтверждения и замечает закрытие сокета.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use axum::body::Bytes;
use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::post;
use axum::{Json, Router};
use crate::common::{wait_until, Crew};
use futures_util::{SinkExt, StreamExt};
use serde_json::{json, Value};
use tokio_tungstenite::tungstenite::Message;

const BOT_TOKEN: &str = "xoxb-test-token";
const APP_TOKEN: &str = "xapp-test-token";
const USERNAME: &str = "deloist_test_bot";
const TEAM: &str = "Deloist Test";
const BOT_USER_ID: &str = "U0";

fn authorized(headers: &HeaderMap, token: &str) -> bool {
    let expected = format!("Bearer {token}");
    headers.get("authorization").and_then(|value| value.to_str().ok()) == Some(expected.as_str())
}

/// REST Slack: `auth.test`, `apps.connections.open` и `chat.postMessage`.
#[derive(Clone, Default)]
struct FakeSlack {
    sent: Arc<Mutex<Vec<Value>>>,
    opens: Arc<AtomicUsize>,
    socket_url: Arc<Mutex<String>>,
}

async fn auth_test(State(_fake): State<FakeSlack>, headers: HeaderMap) -> Response {
    if !authorized(&headers, BOT_TOKEN) {
        return (StatusCode::OK, Json(json!({ "ok": false, "error": "invalid_auth" }))).into_response();
    }
    (
        StatusCode::OK,
        Json(json!({
            "ok": true,
            "user_id": BOT_USER_ID,
            "bot_id": "B0",
            "team": TEAM,
            "user": USERNAME,
        })),
    )
        .into_response()
}

async fn connections_open(State(fake): State<FakeSlack>, headers: HeaderMap) -> Response {
    if !authorized(&headers, APP_TOKEN) {
        return (StatusCode::OK, Json(json!({ "ok": false, "error": "invalid_auth" }))).into_response();
    }
    fake.opens.fetch_add(1, Ordering::SeqCst);
    let url = fake.socket_url.lock().unwrap().clone();
    (StatusCode::OK, Json(json!({ "ok": true, "url": url }))).into_response()
}

async fn post_message(State(fake): State<FakeSlack>, headers: HeaderMap, body: Bytes) -> Response {
    if !authorized(&headers, BOT_TOKEN) {
        return (StatusCode::OK, Json(json!({ "ok": false, "error": "invalid_auth" }))).into_response();
    }
    let request: Value = serde_json::from_slice(&body).unwrap_or_else(|_| json!({}));
    fake.sent.lock().unwrap().push(request);
    (StatusCode::OK, Json(json!({ "ok": true }))).into_response()
}

/// Socket Mode: одно соединение за раз, события приходят из теста.
#[derive(Clone, Default)]
struct FakeSocket {
    tx: Arc<Mutex<Option<tokio::sync::mpsc::UnboundedSender<Value>>>>,
    acks: Arc<Mutex<Vec<Value>>>,
    connections: Arc<AtomicUsize>,
    closed: Arc<AtomicUsize>,
    next_id: Arc<AtomicUsize>,
}

impl FakeSocket {
    /// Отправить конверт с событием `message` и вернуть его `envelope_id`.
    fn push_message(&self, user_id: &str, channel_id: &str, text: &str) -> String {
        let id = format!("env-{}", self.next_id.fetch_add(1, Ordering::SeqCst) + 1);
        self.push(json!({
            "envelope_id": id,
            "type": "events_api",
            "payload": {
                "type": "event_callback",
                "event": {
                    "type": "message",
                    "channel_type": "im",
                    "channel": channel_id,
                    "user": user_id,
                    "text": text,
                },
            },
        }));
        id
    }

    fn push(&self, event: Value) {
        let sender = self.tx.lock().unwrap().clone().expect("Socket Mode ещё не подключился");
        sender.send(event).expect("Socket Mode уже закрыт");
    }
}

async fn socket(stream: tokio::net::TcpStream, state: FakeSocket) {
    let Ok(socket) = tokio_tungstenite::accept_async(stream).await else {
        return;
    };
    state.connections.fetch_add(1, Ordering::SeqCst);
    let (mut sink, mut stream) = socket.split();
    if sink.send(Message::text(json!({ "type": "hello" }).to_string())).await.is_err() {
        return;
    }
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<Value>();
    *state.tx.lock().unwrap() = Some(tx);
    loop {
        tokio::select! {
            event = rx.recv() => {
                let Some(event) = event else {
                    return;
                };
                if sink.send(Message::text(event.to_string())).await.is_err() {
                    state.closed.fetch_add(1, Ordering::SeqCst);
                    *state.tx.lock().unwrap() = None;
                    return;
                }
            }
            incoming = stream.next() => match incoming {
                Some(Ok(Message::Text(text))) => {
                    let Ok(value) = serde_json::from_str::<Value>(text.as_str()) else {
                        continue;
                    };
                    state.acks.lock().unwrap().push(value);
                }
                Some(Ok(Message::Ping(payload))) => {
                    let _ = sink.send(Message::Pong(payload)).await;
                }
                Some(Ok(Message::Close(_))) | None | Some(Err(_)) => {
                    state.closed.fetch_add(1, Ordering::SeqCst);
                    *state.tx.lock().unwrap() = None;
                    return;
                }
                _ => {}
            },
        }
    }
}

async fn fake_slack() -> (String, FakeSlack, FakeSocket) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let socket_url = format!("ws://127.0.0.1:{port}/socket");
    let socket_state = FakeSocket::default();
    let state = socket_state.clone();
    tokio::spawn(async move {
        loop {
            let Ok((stream, _)) = listener.accept().await else {
                return;
            };
            let state = state.clone();
            tokio::spawn(async move { socket(stream, state).await });
        }
    });

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let fake = FakeSlack { socket_url: Arc::new(Mutex::new(socket_url)), ..FakeSlack::default() };
    let router = Router::new()
        .route("/auth.test", post(auth_test))
        .route("/apps.connections.open", post(connections_open))
        .route("/chat.postMessage", post(post_message))
        .with_state(fake.clone());
    tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
    (format!("http://127.0.0.1:{port}"), fake, socket_state)
}

async fn setup(name: &str) -> (Crew, FakeSlack, FakeSocket, String, String) {
    let (base, rest, socket) = fake_slack().await;
    let crew = Crew::start_with_env(name, &[("CREW_TEST_SLACK_API", &base)]).await;
    let state = crew.ok("GET", "/api/state", None).await;
    let bot = state["bots"][0]["id"].as_str().unwrap().to_string();
    let bot_name = state["bots"][0]["name"].as_str().unwrap().to_string();
    let connected = crew
        .ok("PUT", "/api/slack", Some(json!({ "botToken": BOT_TOKEN, "appToken": APP_TOKEN, "botId": bot })))
        .await;
    assert_eq!(connected["username"], USERNAME);
    (crew, rest, socket, bot, bot_name)
}

async fn wait_connected(socket: &FakeSocket) {
    wait_until("Socket Mode подключился", 25, || async {
        socket.connections.load(Ordering::SeqCst) >= 1 && socket.tx.lock().unwrap().is_some()
    })
    .await;
}

async fn wait_ack(socket: &FakeSocket, envelope_id: &str) {
    wait_until("конверт подтверждён", 20, || async {
        socket.acks.lock().unwrap().iter().any(|ack| ack["envelope_id"] == envelope_id)
    })
    .await;
}

/// Привязка личной переписки кодом из `/api/slack/pair`.
async fn pair(crew: &Crew, socket: &FakeSocket) {
    let pair = crew.ok("POST", "/api/slack/pair", None).await;
    let code = pair["code"].as_str().unwrap().to_string();
    assert_eq!(code.len(), 6);
    assert!(code.chars().all(|c| c.is_ascii_digit()), "{code}");
    let envelope = socket.push_message("U1", "D1", &format!("start {code}"));
    wait_ack(socket, &envelope).await;
    wait_until("переписка привязана", 20, || async {
        crew.ok("GET", "/api/slack", None).await["paired"] == true
    })
    .await;
}

#[tokio::test(flavor = "multi_thread")]
async fn the_tokens_are_saved_in_secrets_and_never_returned() {
    let (crew, _rest, socket, bot, _bot_name) = setup("slack-token").await;
    wait_connected(&socket).await;

    let status = crew.ok("GET", "/api/slack", None).await;
    assert_eq!(status["connected"], true);
    assert_eq!(status["enabled"], true);
    assert_eq!(status["botId"], bot.as_str());
    assert_eq!(status["paired"], false);
    assert_eq!(status["team"], TEAM);
    assert_eq!(status["username"], USERNAME);
    assert!(!status.to_string().contains(BOT_TOKEN), "GET /api/slack отдал токен бота: {status}");
    assert!(!status.to_string().contains(APP_TOKEN), "GET /api/slack отдал токен приложения: {status}");

    let state = crew.ok("GET", "/api/state", None).await;
    assert!(!state.to_string().contains(BOT_TOKEN), "/api/state отдал токен бота");
    assert!(!state.to_string().contains(APP_TOKEN), "/api/state отдал токен приложения");

    let state_file = std::fs::read_to_string(crew.data_dir.join("state.json")).unwrap();
    assert!(!state_file.contains(BOT_TOKEN), "state.json содержит токен бота");
    assert!(!state_file.contains(APP_TOKEN), "state.json содержит токен приложения");
    let secrets = crate::common::read_secrets(&crew.data_dir);
    assert!(secrets.contains("slackBotToken"), "в секретах нет ключа бота: {secrets}");
    assert!(secrets.contains("slackAppToken"), "в секретах нет ключа приложения: {secrets}");
    assert!(secrets.contains(BOT_TOKEN), "токен бота не сохранён в secrets.json");
    assert!(secrets.contains(APP_TOKEN), "токен приложения не сохранён в secrets.json");

    // PUT без токенов меняет только отвечающего бота, привязка и токены остаются.
    let switched = crew.ok("PUT", "/api/slack", Some(json!({ "botId": bot }))).await;
    assert_eq!(switched["username"], USERNAME);
    assert_eq!(switched["team"], TEAM);
    let after_switch = crew.ok("GET", "/api/slack", None).await;
    assert_eq!(after_switch["connected"], true);
    assert_eq!(after_switch["botId"], bot.as_str());

    // DELETE забывает оба токена, человека, канал и код, а сокет закрывается.
    crew.ok("DELETE", "/api/slack", None).await;
    let secrets = crate::common::read_secrets(&crew.data_dir);
    assert!(!secrets.contains(BOT_TOKEN), "токен бота остался в secrets.json: {secrets}");
    assert!(!secrets.contains(APP_TOKEN), "токен приложения остался в secrets.json: {secrets}");
    let after = crew.ok("GET", "/api/slack", None).await;
    assert_eq!(after["connected"], false);
    assert_eq!(after["paired"], false);
    assert_eq!(after["team"], "");
    assert_eq!(after["username"], "");
    wait_until("Socket Mode закрыт после DELETE", 20, || async {
        socket.closed.load(Ordering::SeqCst) >= 1
    })
    .await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_pairing_dm_with_the_right_code_stores_the_user() {
    let (crew, rest, socket, _bot, bot_name) = setup("slack-pair").await;
    wait_connected(&socket).await;
    pair(&crew, &socket).await;

    wait_until("приветствие в Slack", 20, || async {
        rest.sent.lock().unwrap().iter().any(|message| {
            message["channel"] == "D1"
                && message["text"].as_str().unwrap_or_default().contains("Готово: пишите сюда")
        })
    })
    .await;
    let greeting = rest
        .sent
        .lock()
        .unwrap()
        .iter()
        .find(|message| message["channel"] == "D1")
        .cloned()
        .unwrap();
    assert!(
        greeting["text"].as_str().unwrap_or_default().contains(&bot_name),
        "в приветствии нет имени бота: {greeting}"
    );

    let state = crew.ok("GET", "/api/state", None).await;
    assert_eq!(state["settings"]["slack"]["userId"], "U1");
    assert_eq!(state["settings"]["slack"]["channelId"], "D1");
    assert_eq!(state["settings"]["slack"]["enabled"], true);
    assert_eq!(state["settings"]["slack"]["team"], TEAM);
}

#[tokio::test(flavor = "multi_thread")]
async fn every_envelope_is_acked_even_when_the_event_is_ignored() {
    let (crew, rest, socket, _bot, _bot_name) = setup("slack-ack").await;
    wait_connected(&socket).await;

    // Сообщение бота и сообщение канала не наши, но подтвердить конверт всё равно надо.
    let bot_envelope = format!("env-bot-{}", socket.next_id.fetch_add(1, Ordering::SeqCst));
    socket.push(json!({
        "envelope_id": bot_envelope,
        "type": "events_api",
        "payload": { "event": {
            "type": "message", "channel_type": "im", "channel": "D1",
            "user": BOT_USER_ID, "text": "я бот", "bot_id": "B0",
        }},
    }));
    wait_ack(&socket, &bot_envelope).await;

    let channel_envelope = format!("env-channel-{}", socket.next_id.fetch_add(1, Ordering::SeqCst));
    socket.push(json!({
        "envelope_id": channel_envelope,
        "type": "events_api",
        "payload": { "event": {
            "type": "message", "channel_type": "channel", "channel": "C1",
            "user": "U2", "text": "в канале",
        }},
    }));
    wait_ack(&socket, &channel_envelope).await;

    // Подтверждения ушли в том же виде, в каком их ждёт Slack.
    let acks = socket.acks.lock().unwrap().clone();
    assert!(acks.iter().any(|ack| ack["envelope_id"].as_str() == Some(bot_envelope.as_str())));
    assert!(acks.iter().any(|ack| ack["envelope_id"].as_str() == Some(channel_envelope.as_str())));
    // Ни бот, ни канал не получили ответа, и в бота ничего не попало.
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert!(rest.sent.lock().unwrap().is_empty(), "на проигнорированное событие ушёл ответ");
    assert_eq!(crew.ok("GET", "/api/slack", None).await["paired"], false);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_dm_from_the_paired_user_reaches_the_bot_and_the_answer_goes_back() {
    let (crew, rest, socket, bot, _bot_name) = setup("slack-dm").await;
    crew.listen().await;
    wait_connected(&socket).await;
    pair(&crew, &socket).await;

    let envelope = socket.push_message("U1", "D1", "привет из слака");
    wait_ack(&socket, &envelope).await;
    let end = crew
        .wait_event("конец хода из Slack", |event| {
            event["type"] == "turn_end" && event["botId"] == bot.as_str()
        })
        .await;
    let answer = end["text"].as_str().unwrap_or_default().to_string();
    assert!(!answer.is_empty(), "пустой ответ бота: {end}");

    let thread = crew.ok("GET", &format!("/api/bots/{bot}/thread?limit=20"), None).await;
    let texts: Vec<String> = thread["messages"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|message| message["text"].as_str().map(str::to_string))
        .collect();
    assert!(
        texts.iter().any(|text| text == "[Slack] привет из слака"),
        "в треде бота нет сообщения с префиксом: {texts:?}"
    );

    // Ответ хода уходит в привязанную личку.
    wait_until("ответ бота в Slack", 20, || async {
        rest.sent
            .lock()
            .unwrap()
            .iter()
            .any(|message| message["channel"] == "D1" && message["text"] == answer.as_str())
    })
    .await;

    // Чужой человек не попадает в бота: получает только отказ, и то не в тред.
    let stranger = socket.push_message("U2", "D2", "чужое сообщение");
    wait_ack(&socket, &stranger).await;
    wait_until("отказ чужому человеку", 20, || async {
        rest.sent.lock().unwrap().iter().any(|message| {
            message["channel"] == "D2"
                && message["text"].as_str().unwrap_or_default().contains("не подключён")
        })
    })
    .await;
    tokio::time::sleep(Duration::from_millis(500)).await;
    let thread = crew.ok("GET", &format!("/api/bots/{bot}/thread?limit=50"), None).await;
    assert!(
        !thread["messages"]
            .as_array()
            .unwrap()
            .iter()
            .any(|message| message["text"].as_str().unwrap_or_default().contains("чужое сообщение")),
        "сообщение чужого человека попало в бота"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_disconnect_reconnects_through_apps_connections_open() {
    let (crew, rest, socket, _bot, _bot_name) = setup("slack-reconnect").await;
    wait_connected(&socket).await;
    assert!(rest.opens.load(Ordering::SeqCst) >= 1);

    socket.push(json!({ "type": "disconnect" }));
    wait_until("Socket Mode переподключился", 25, || async {
        socket.connections.load(Ordering::SeqCst) >= 2 && socket.tx.lock().unwrap().is_some()
    })
    .await;
    assert!(rest.opens.load(Ordering::SeqCst) >= 2, "адрес Socket Mode не запрашивали заново");

    // После переподключения личка по-прежнему работает.
    pair(&crew, &socket).await;
    assert_eq!(crew.ok("GET", "/api/slack", None).await["paired"], true);
}
