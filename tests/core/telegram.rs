//! Мост Telegram: токен только в секретах, привязка чата кодом, переписка с выбранным ботом
//! и ответ, который возвращается в чат после конца хода.
//!
//! Вместо api.telegram.org — маленький сервер: он помнит обновления и отправленные сообщения.


use std::sync::{Arc, Mutex};
use std::time::Duration;

use axum::body::Bytes;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::post;
use axum::Json;
use axum::Router;
use crate::common::{wait_until, Crew};
use serde_json::{json, Value};

const TOKEN: &str = "123456:TEST-token";
const USERNAME: &str = "deloist_test_bot";

#[derive(Clone, Default)]
struct FakeTg {
    updates: Arc<Mutex<Vec<Value>>>,
    sent: Arc<Mutex<Vec<Value>>>,
}

async fn telegram(State(fake): State<FakeTg>, Path((token, method)): Path<(String, String)>, body: Bytes) -> Response {
    if token != format!("bot{TOKEN}") {
        return (StatusCode::UNAUTHORIZED, Json(json!({ "ok": false, "description": "Unauthorized" }))).into_response();
    }
    match method.as_str() {
        "getMe" => (
            StatusCode::OK,
            Json(json!({
                "ok": true,
                "result": { "id": 1, "is_bot": true, "first_name": "Deloist", "username": USERNAME },
            })),
        )
            .into_response(),
        "getUpdates" => {
            let request: Value = serde_json::from_slice(&body).unwrap_or_else(|_| json!({}));
            let offset = request["offset"].as_i64().unwrap_or(0);
            let updates: Vec<Value> = fake
                .updates
                .lock()
                .unwrap()
                .iter()
                .filter(|update| update["update_id"].as_i64().unwrap_or(0) >= offset)
                .cloned()
                .collect();
            (StatusCode::OK, Json(json!({ "ok": true, "result": updates }))).into_response()
        }
        "sendMessage" => {
            let request: Value = serde_json::from_slice(&body).unwrap_or_else(|_| json!({}));
            fake.sent.lock().unwrap().push(request);
            let id = fake.sent.lock().unwrap().len();
            (StatusCode::OK, Json(json!({ "ok": true, "result": { "message_id": id } }))).into_response()
        }
        _ => (StatusCode::NOT_FOUND, Json(json!({ "ok": false, "description": "Not Found" }))).into_response(),
    }
}

async fn fake_telegram() -> (String, FakeTg) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let fake = FakeTg::default();
    let router = Router::new().route("/{token}/{method}", post(telegram)).with_state(fake.clone());
    tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
    (format!("http://127.0.0.1:{port}"), fake)
}

async fn setup(name: &str) -> (Crew, FakeTg, String, String) {
    let (base, fake) = fake_telegram().await;
    let crew = Crew::start_with_env(name, &[("CREW_TEST_TELEGRAM_API", &base)]).await;
    let state = crew.ok("GET", "/api/state", None).await;
    let bot = state["bots"][0]["id"].as_str().unwrap().to_string();
    let bot_name = state["bots"][0]["name"].as_str().unwrap().to_string();
    let connected = crew
        .ok("PUT", "/api/telegram", Some(json!({ "token": TOKEN, "botId": bot })))
        .await;
    assert_eq!(connected["username"], USERNAME);
    (crew, fake, bot, bot_name)
}

fn push(fake: &FakeTg, update_id: i64, chat_id: i64, text: &str) {
    fake.updates.lock().unwrap().push(json!({
        "update_id": update_id,
        "message": {
            "message_id": update_id,
            "date": 1,
            "chat": { "id": chat_id, "type": "private" },
            "from": { "id": chat_id, "is_bot": false },
            "text": text,
        },
    }));
}

#[tokio::test(flavor = "multi_thread")]
async fn the_token_is_saved_in_secrets_and_never_returned() {
    let (crew, _fake, bot, _bot_name) = setup("tg-token").await;

    let status = crew.ok("GET", "/api/telegram", None).await;
    assert_eq!(status["connected"], true);
    assert_eq!(status["enabled"], true);
    assert_eq!(status["botId"], bot.as_str());
    assert_eq!(status["chatPaired"], false);
    assert_eq!(status["username"], USERNAME);
    assert!(!status.to_string().contains(TOKEN), "GET /api/telegram отдал токен: {status}");

    let state = crew.ok("GET", "/api/state", None).await;
    assert!(!state.to_string().contains(TOKEN), "/api/state отдал токен");

    let state_file = std::fs::read_to_string(crew.data_dir.join("state.json")).unwrap();
    assert!(!state_file.contains(TOKEN), "state.json содержит токен");
    let secrets = crate::common::read_secrets(&crew.data_dir);
    assert!(secrets.contains("telegramBotToken"), "в секретах нет ключа: {secrets}");
    assert!(secrets.contains(TOKEN), "токен не сохранён в secrets.json");

    // DELETE забывает токен, чат и код.
    crew.ok("DELETE", "/api/telegram", None).await;
    let secrets = crate::common::read_secrets(&crew.data_dir);
    assert!(!secrets.contains(TOKEN), "токен остался в secrets.json: {secrets}");
    let after = crew.ok("GET", "/api/telegram", None).await;
    assert_eq!(after["connected"], false);
    assert_eq!(after["chatPaired"], false);
    assert_eq!(after["username"], "");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_busy_bot_shows_the_telegram_message_in_its_queue_with_the_prefix() {
    let (crew, fake, bot, _bot_name) = setup("tg-queue").await;
    crew.listen().await;
    let pair = crew.ok("POST", "/api/telegram/pair", None).await;
    push(&fake, 1, 555, &format!("/start {}", pair["code"].as_str().unwrap()));
    wait_until("чат привязан", 20, || async {
        crew.ok("GET", "/api/telegram", None).await["chatPaired"] == true
    })
    .await;

    // Первый ход занят карточкой разрешения: сообщение из Telegram должно встать в очередь.
    crew.say(&bot, "нарисуй кота").await;
    let card = crew.wait_event("карточка", |e| e["type"] == "approval").await;
    push(&fake, 2, 555, "пока занят");
    wait_until("сообщение в очереди", 20, || async {
        let queue = crew.ok("GET", &format!("/api/bots/{bot}/queue"), None).await;
        queue["items"].as_array().is_some_and(|items| {
            items.iter().any(|item| item["text"] == "[Telegram] пока занят")
        })
    })
    .await;

    // Отказ закрывает карточку; очередь сама начинает ход из Telegram.
    crew.ok("POST", &format!("/api/approvals/{}", card["id"].as_str().unwrap()), Some(json!({ "allow": false }))).await;
    crew.wait_event("конец хода из очереди", |e| {
        e["type"] == "turn_end" && e["botId"] == bot.as_str()
    })
    .await;
}

#[tokio::test(flavor = "multi_thread")]
async fn stop_and_new_from_the_paired_chat_act_on_the_bot() {
    let (crew, fake, bot, _bot_name) = setup("tg-commands").await;
    crew.listen().await;
    let pair = crew.ok("POST", "/api/telegram/pair", None).await;
    push(&fake, 1, 555, &format!("/start {}", pair["code"].as_str().unwrap()));
    wait_until("чат привязан", 20, || async {
        crew.ok("GET", "/api/telegram", None).await["chatPaired"] == true
    })
    .await;

    // /stop снимает идущий ход, который ждёт карточку.
    crew.say(&bot, "нарисуй кота").await;
    crew.wait_event("карточка", |e| e["type"] == "approval").await;
    push(&fake, 2, 555, "/stop");
    crew.wait_event("ход остановлен", |e| e["type"] == "turn_end" && e["botId"] == bot.as_str()).await;
    wait_until("бот свободен", 20, || async {
        crew.ok("GET", "/api/state", None).await["bots"][0]["busy"] == false
    })
    .await;

    // /new — то же, что «Сжать историю»: в чат приходит итог действия.
    push(&fake, 3, 555, "/new");
    wait_until("ответ на /new", 20, || async {
        fake.sent.lock().unwrap().iter().any(|m| {
            m["chat_id"] == 555
                && matches!(m["text"].as_str(), Some("История сжата.") | Some("Сжимать пока нечего."))
        })
    })
    .await;
}

#[tokio::test(flavor = "multi_thread")]
async fn pairing_prefix_and_answer_round_trip_through_the_fake_telegram() {
    let (crew, fake, bot, bot_name) = setup("tg-pair").await;
    crew.listen().await;

    let pair = crew.ok("POST", "/api/telegram/pair", None).await;
    let code = pair["code"].as_str().unwrap().to_string();
    assert_eq!(code.len(), 6);
    assert!(code.chars().all(|c| c.is_ascii_digit()), "{code}");

    push(&fake, 1, 555, &format!("/start {code}"));
    wait_until("чат привязан", 20, || async {
        crew.ok("GET", "/api/telegram", None).await["chatPaired"] == true
    })
    .await;
    wait_until("приветствие в Telegram", 20, || async {
        fake.sent.lock().unwrap().iter().any(|m| {
            m["chat_id"] == 555 && m["text"].as_str().unwrap_or_default().contains("Готово: пишите сюда")
        })
    })
    .await;
    let greeting = fake
        .sent
        .lock()
        .unwrap()
        .iter()
        .find(|m| m["chat_id"] == 555)
        .cloned()
        .unwrap();
    assert!(
        greeting["text"].as_str().unwrap_or_default().contains(&bot_name),
        "в приветствии нет имени бота: {greeting}"
    );

    // Текст из привязанного чата доходит до выбранного бота с видимым префиксом.
    push(&fake, 2, 555, "привет из телеграма");
    let end = crew
        .wait_event("конец хода из Telegram", |e| e["type"] == "turn_end" && e["botId"] == bot.as_str())
        .await;
    let answer = end["text"].as_str().unwrap_or_default().to_string();
    assert!(!answer.is_empty(), "пустой ответ бота: {end}");

    let thread = crew.ok("GET", &format!("/api/bots/{bot}/thread?limit=20"), None).await;
    let texts: Vec<String> = thread["messages"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|m| m["text"].as_str().map(str::to_string))
        .collect();
    assert!(
        texts.iter().any(|text| text == "[Telegram] привет из телеграма"),
        "в треде бота нет сообщения с префиксом: {texts:?}"
    );

    // Ответ хода уходит в привязанный чат обычным текстом, без parse_mode.
    wait_until("ответ бота в Telegram", 20, || async {
        fake.sent.lock().unwrap().iter().any(|m| m["chat_id"] == 555 && m["text"] == answer.as_str())
    })
    .await;
    let reply = fake
        .sent
        .lock()
        .unwrap()
        .iter()
        .find(|m| m["chat_id"] == 555 && m["text"] == answer.as_str())
        .cloned()
        .unwrap();
    assert!(reply.get("parse_mode").is_none(), "у ответа появился parse_mode: {reply}");

    // Чужой чат не попадает в бота: получает только отказ, и то не в тред.
    push(&fake, 3, 777, "чужое сообщение");
    wait_until("отказ чужому чату", 20, || async {
        fake.sent.lock().unwrap().iter().any(|m| {
            m["chat_id"] == 777 && m["text"].as_str().unwrap_or_default().contains("не подключён")
        })
    })
    .await;
    tokio::time::sleep(Duration::from_millis(500)).await;
    let thread = crew.ok("GET", &format!("/api/bots/{bot}/thread?limit=50"), None).await;
    let texts = thread["messages"].as_array().unwrap();
    assert!(
        !texts.iter().any(|m| m["text"].as_str().unwrap_or_default().contains("чужое сообщение")),
        "сообщение чужого чата попало в бота"
    );
}
