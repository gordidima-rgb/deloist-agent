//! Мост Discord: токен только в секретах, привязка личной переписки кодом, переписка с выбранным
//! ботом и ответ, который возвращается в личку после конца хода.
//!
//! Вместо discord.com — маленький REST-сервер и поддельный Gateway: он проверяет Identify,
//! присылает события и замечает закрытие сокета.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use axum::body::Bytes;
use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use crate::common::{wait_until, Crew};
use futures_util::{SinkExt, StreamExt};
use serde_json::{json, Value};
use tokio_tungstenite::tungstenite::Message;

const TOKEN: &str = "discord-test-token";
const USERNAME: &str = "deloist_test_bot";

#[derive(Clone, Default)]
struct FakeDiscord {
    sent: Arc<Mutex<Vec<Value>>>,
}

async fn me(State(_fake): State<FakeDiscord>, headers: HeaderMap) -> Response {
    let expected = format!("Bot {TOKEN}");
    if headers.get("authorization").and_then(|value| value.to_str().ok()) != Some(expected.as_str()) {
        return (StatusCode::UNAUTHORIZED, Json(json!({ "message": "401: Unauthorized" }))).into_response();
    }
    (StatusCode::OK, Json(json!({ "id": "900", "username": USERNAME }))).into_response()
}

async fn send_message(
    State(fake): State<FakeDiscord>,
    Path(channel_id): Path<String>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    let expected = format!("Bot {TOKEN}");
    if headers.get("authorization").and_then(|value| value.to_str().ok()) != Some(expected.as_str()) {
        return (StatusCode::UNAUTHORIZED, Json(json!({ "message": "401: Unauthorized" }))).into_response();
    }
    let request: Value = serde_json::from_slice(&body).unwrap_or_else(|_| json!({}));
    fake.sent.lock().unwrap().push(json!({ "channel_id": channel_id, "content": request["content"] }));
    (StatusCode::OK, Json(json!({ "id": "1" }))).into_response()
}

async fn fake_discord() -> (String, FakeDiscord) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let fake = FakeDiscord::default();
    let router = Router::new()
        .route("/users/@me", get(me))
        .route("/channels/{channel_id}/messages", post(send_message))
        .with_state(fake.clone());
    tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
    (format!("http://127.0.0.1:{port}"), fake)
}

#[derive(Clone, Default)]
struct FakeGateway {
    token: Arc<Mutex<Option<String>>>,
    out: Arc<Mutex<Option<tokio::sync::mpsc::UnboundedSender<Value>>>>,
    closed: Arc<Mutex<bool>>,
}

impl FakeGateway {
    fn push(&self, event: Value) {
        let sender = self.out.lock().unwrap().clone().expect("Gateway ещё не принял Identify");
        sender.send(event).expect("Gateway уже закрыт");
    }
}

async fn gateway_socket(stream: tokio::net::TcpStream, state: FakeGateway) {
    let Ok(socket) = tokio_tungstenite::accept_async(stream).await else {
        return;
    };
    let (mut sink, mut stream) = socket.split();
    let hello = json!({ "op": 10, "d": { "heartbeat_interval": 30_000 } });
    if sink.send(Message::text(hello.to_string())).await.is_err() {
        return;
    }
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<Value>();
    loop {
        tokio::select! {
            incoming = stream.next() => match incoming {
                Some(Ok(Message::Text(text))) => {
                    let Ok(value) = serde_json::from_str::<Value>(text.as_str()) else {
                        continue;
                    };
                    match value["op"].as_i64() {
                        Some(1) => {
                            let _ = sink.send(Message::text(json!({ "op": 11 }).to_string())).await;
                        }
                        Some(2) => {
                            *state.token.lock().unwrap() = Some(value["d"]["token"].as_str().unwrap_or_default().to_string());
                            *state.out.lock().unwrap() = Some(tx.clone());
                            break;
                        }
                        _ => {}
                    }
                }
                Some(Ok(Message::Close(_))) | None | Some(Err(_)) => {
                    *state.closed.lock().unwrap() = true;
                    return;
                }
                _ => {}
            },
            _ = tokio::time::sleep(Duration::from_secs(10)) => return,
        }
    }
    loop {
        tokio::select! {
            event = rx.recv() => {
                let Some(event) = event else {
                    return;
                };
                if sink.send(Message::text(event.to_string())).await.is_err() {
                    *state.closed.lock().unwrap() = true;
                    return;
                }
            }
            incoming = stream.next() => match incoming {
                Some(Ok(Message::Text(text))) => {
                    let Ok(value) = serde_json::from_str::<Value>(text.as_str()) else {
                        continue;
                    };
                    if value["op"].as_i64() == Some(1) {
                        let _ = sink.send(Message::text(json!({ "op": 11 }).to_string())).await;
                    }
                }
                Some(Ok(Message::Ping(payload))) => {
                    let _ = sink.send(Message::Pong(payload)).await;
                }
                Some(Ok(Message::Close(_))) | None | Some(Err(_)) => {
                    *state.closed.lock().unwrap() = true;
                    return;
                }
                _ => {}
            }
        }
    }
}

async fn fake_gateway() -> (String, FakeGateway) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let fake = FakeGateway::default();
    let state = fake.clone();
    tokio::spawn(async move {
        loop {
            let Ok((stream, _)) = listener.accept().await else {
                return;
            };
            let state = state.clone();
            tokio::spawn(async move { gateway_socket(stream, state).await });
        }
    });
    (format!("ws://127.0.0.1:{port}"), fake)
}

async fn setup(name: &str) -> (Crew, FakeDiscord, FakeGateway, String, String) {
    let (base, rest) = fake_discord().await;
    let (gateway, gateway_fake) = fake_gateway().await;
    let crew = Crew::start_with_env(
        name,
        &[("CREW_TEST_DISCORD_API", &base), ("CREW_TEST_DISCORD_GATEWAY", &gateway)],
    )
    .await;
    let state = crew.ok("GET", "/api/state", None).await;
    let bot = state["bots"][0]["id"].as_str().unwrap().to_string();
    let bot_name = state["bots"][0]["name"].as_str().unwrap().to_string();
    let connected = crew
        .ok("PUT", "/api/discord", Some(json!({ "token": TOKEN, "botId": bot })))
        .await;
    assert_eq!(connected["username"], USERNAME);
    (crew, rest, gateway_fake, bot, bot_name)
}

async fn wait_identified(gateway: &FakeGateway) {
    wait_until("Gateway принял Identify", 25, || async {
        gateway.token.lock().unwrap().is_some()
    })
    .await;
}

fn dispatch(user_id: &str, channel_id: &str, content: &str) -> Value {
    json!({
        "op": 0,
        "s": 1,
        "t": "MESSAGE_CREATE",
        "d": {
            "id": "1",
            "channel_id": channel_id,
            "author": { "id": user_id, "username": "человек" },
            "content": content,
        },
    })
}

/// Привязка личной переписки кодом из `/api/discord/pair`.
async fn pair(crew: &Crew, gateway: &FakeGateway) {
    let pair = crew.ok("POST", "/api/discord/pair", None).await;
    let code = pair["code"].as_str().unwrap().to_string();
    assert_eq!(code.len(), 6);
    assert!(code.chars().all(|c| c.is_ascii_digit()), "{code}");
    gateway.push(dispatch("555", "777", &format!("start {code}")));
    wait_until("переписка привязана", 20, || async {
        crew.ok("GET", "/api/discord", None).await["paired"] == true
    })
    .await;
}

#[tokio::test(flavor = "multi_thread")]
async fn the_token_is_saved_in_secrets_and_never_returned() {
    let (crew, _rest, gateway, bot, _bot_name) = setup("discord-token").await;
    wait_identified(&gateway).await;

    let status = crew.ok("GET", "/api/discord", None).await;
    assert_eq!(status["connected"], true);
    assert_eq!(status["enabled"], true);
    assert_eq!(status["botId"], bot.as_str());
    assert_eq!(status["paired"], false);
    assert_eq!(status["username"], USERNAME);
    assert!(!status.to_string().contains(TOKEN), "GET /api/discord отдал токен: {status}");

    let state = crew.ok("GET", "/api/state", None).await;
    assert!(!state.to_string().contains(TOKEN), "/api/state отдал токен");

    let state_file = std::fs::read_to_string(crew.data_dir.join("state.json")).unwrap();
    assert!(!state_file.contains(TOKEN), "state.json содержит токен");
    let secrets = crate::common::read_secrets(&crew.data_dir);
    assert!(secrets.contains("discordBotToken"), "в секретах нет ключа: {secrets}");
    assert!(secrets.contains(TOKEN), "токен не сохранён в secrets.json");

    // PUT без токена меняет только отвечающего бота, привязка и токен остаются.
    let switched = crew.ok("PUT", "/api/discord", Some(json!({ "botId": bot }))).await;
    assert_eq!(switched["username"], USERNAME);
    let after_switch = crew.ok("GET", "/api/discord", None).await;
    assert_eq!(after_switch["connected"], true);
    assert_eq!(after_switch["botId"], bot.as_str());

    // DELETE забывает токен, человека, канал и код, а Gateway закрывается.
    crew.ok("DELETE", "/api/discord", None).await;
    let secrets = crate::common::read_secrets(&crew.data_dir);
    assert!(!secrets.contains(TOKEN), "токен остался в secrets.json: {secrets}");
    let after = crew.ok("GET", "/api/discord", None).await;
    assert_eq!(after["connected"], false);
    assert_eq!(after["paired"], false);
    assert_eq!(after["username"], "");
    wait_until("Gateway закрыт после DELETE", 20, || async {
        *gateway.closed.lock().unwrap()
    })
    .await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_pairing_dm_with_the_right_code_stores_the_user() {
    let (crew, rest, gateway, _bot, bot_name) = setup("discord-pair").await;
    crew.listen().await;
    wait_identified(&gateway).await;
    pair(&crew, &gateway).await;

    wait_until("приветствие в Discord", 20, || async {
        rest.sent.lock().unwrap().iter().any(|message| {
            message["channel_id"] == "777"
                && message["content"].as_str().unwrap_or_default().contains("Готово: пишите сюда")
        })
    })
    .await;
    let greeting = rest
        .sent
        .lock()
        .unwrap()
        .iter()
        .find(|message| message["channel_id"] == "777")
        .cloned()
        .unwrap();
    assert!(
        greeting["content"].as_str().unwrap_or_default().contains(&bot_name),
        "в приветствии нет имени бота: {greeting}"
    );

    let state = crew.ok("GET", "/api/state", None).await;
    assert_eq!(state["settings"]["discord"]["userId"], "555");
    assert_eq!(state["settings"]["discord"]["channelId"], "777");
    assert_eq!(state["settings"]["discord"]["enabled"], true);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_busy_bot_shows_the_discord_message_in_its_queue_with_the_prefix() {
    let (crew, _rest, gateway, bot, _bot_name) = setup("discord-queue").await;
    crew.listen().await;
    wait_identified(&gateway).await;
    pair(&crew, &gateway).await;

    // Первый ход занят карточкой разрешения: сообщение из Discord должно встать в очередь.
    crew.say(&bot, "нарисуй кота").await;
    crew.wait_event("карточка", |event| event["type"] == "approval").await;
    gateway.push(dispatch("555", "777", "пока занят"));
    wait_until("сообщение в очереди", 20, || async {
        let queue = crew.ok("GET", &format!("/api/bots/{bot}/queue"), None).await;
        queue["items"].as_array().is_some_and(|items| {
            items.iter().any(|item| item["text"] == "[Discord] пока занят")
        })
    })
    .await;
    let card = crew.find_event(|event| event["type"] == "approval").unwrap();
    crew.ok("POST", &format!("/api/approvals/{}", card["id"].as_str().unwrap()), Some(json!({ "allow": false }))).await;
    crew.wait_event("конец хода из очереди", |event| {
        event["type"] == "turn_end" && event["botId"] == bot.as_str()
    })
    .await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_dm_from_the_paired_user_reaches_the_bot_and_the_answer_goes_back() {
    let (crew, rest, gateway, bot, _bot_name) = setup("discord-dm").await;
    crew.listen().await;
    wait_identified(&gateway).await;
    pair(&crew, &gateway).await;

    gateway.push(dispatch("555", "777", "привет из дискорда"));
    let end = crew
        .wait_event("конец хода из Discord", |event| {
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
        texts.iter().any(|text| text == "[Discord] привет из дискорда"),
        "в треде бота нет сообщения с префиксом: {texts:?}"
    );

    // Ответ хода уходит в привязанную личку.
    wait_until("ответ бота в Discord", 20, || async {
        rest.sent
            .lock()
            .unwrap()
            .iter()
            .any(|message| message["channel_id"] == "777" && message["content"] == answer.as_str())
    })
    .await;

    // Чужой человек не попадает в бота: получает только отказ, и то не в тред.
    gateway.push(dispatch("999", "888", "чужое сообщение"));
    wait_until("отказ чужому человеку", 20, || async {
        rest.sent.lock().unwrap().iter().any(|message| {
            message["channel_id"] == "888"
                && message["content"].as_str().unwrap_or_default().contains("не подключён")
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
async fn stop_and_new_from_the_paired_user_act_on_the_bot() {
    let (crew, rest, gateway, bot, _bot_name) = setup("discord-commands").await;
    crew.listen().await;
    wait_identified(&gateway).await;
    pair(&crew, &gateway).await;

    // stop снимает идущий ход, который ждёт карточку.
    crew.say(&bot, "нарисуй кота").await;
    crew.wait_event("карточка", |event| event["type"] == "approval").await;
    gateway.push(dispatch("555", "777", "stop"));
    crew.wait_event("ход остановлен", |event| {
        event["type"] == "turn_end" && event["botId"] == bot.as_str()
    })
    .await;
    wait_until("бот свободен", 20, || async {
        crew.ok("GET", "/api/state", None).await["bots"][0]["busy"] == false
    })
    .await;

    // new — то же, что «Сжать историю»: в личку приходит итог действия.
    gateway.push(dispatch("555", "777", "new"));
    wait_until("ответ на new", 20, || async {
        rest.sent.lock().unwrap().iter().any(|message| {
            message["channel_id"] == "777"
                && matches!(message["content"].as_str(), Some("История сжата.") | Some("Сжимать пока нечего."))
        })
    })
    .await;
}
