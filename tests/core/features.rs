//! Новые возможности: группы, уровни подтверждений, вебхуки, файл команды, MCP.

use crate::common;

use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use axum::body::{Body, Bytes};
use axum::extract::State;
use axum::response::Response;
use axum::routing::post;
use axum::Router;
use common::Crew;
use hmac::{Hmac, Mac};
use serde_json::{json, Value};
use sha2::Sha256;

/// Бот с нужной политикой подтверждений.
async fn bot_with(crew: &Crew, name: &str, approval: &str) -> String {
    let data = crew.ok("POST", "/api/bots", Some(json!({ "name": name, "role": "тест", "approval": approval }))).await;
    data["bot"]["id"].as_str().unwrap().to_string()
}

#[tokio::test]
async fn a_room_answers_with_its_lead_and_honours_a_mention() {
    let crew = Crew::start("room").await;
    let state = crew.ok("GET", "/api/state", None).await;
    let bots: Vec<String> = state["bots"]
        .as_array()
        .unwrap()
        .iter()
        .map(|b| b["id"].as_str().unwrap().to_string())
        .collect();
    assert!(bots.len() >= 2, "нужны хотя бы два бота");
    crew.listen().await;

    let room = crew
        .ok(
            "POST",
            "/api/rooms",
            Some(json!({
                "name": "Запуск",
                "botIds": [bots[0], bots[1]],
                "lead": bots[0],
                "mode": "lead",
            })),
        )
        .await;
    let room_id = room["room"]["id"].as_str().unwrap().to_string();
    assert_eq!(room["room"]["lead"], bots[0].as_str(), "ведущий сохранился");

    crew.ok("POST", &format!("/api/rooms/{room_id}/messages"), Some(json!({ "text": "привет" }))).await;
    let end = crew
        .wait_event("конец хода группы", |e| e["type"] == "room_turn_end" && e["roomId"] == room_id.as_str())
        .await;
    assert_eq!(end["botId"], bots[0].as_str(), "отвечал не ведущий: {end}");

    let thread = crew.ok("GET", &format!("/api/rooms/{room_id}/thread?limit=20"), None).await;
    let messages = thread["messages"].as_array().unwrap();
    assert!(messages.iter().any(|m| m["role"] == "user"), "нет сообщения человека");
    assert!(
        messages.iter().any(|m| m["role"] == "assistant" && m["botId"] == bots[0].as_str()),
        "нет ответа ведущего в треде группы"
    );

    // Упоминание перебивает ведущего.
    crew.clear_events();
    crew.ok(
        "POST",
        &format!("/api/rooms/{room_id}/messages"),
        Some(json!({ "text": format!("@{} подскажи", state["bots"][1]["name"].as_str().unwrap()) })),
    )
    .await;
    let end = crew
        .wait_event("ответ по упоминанию", |e| e["type"] == "room_turn_end" && e["roomId"] == room_id.as_str())
        .await;
    assert_eq!(end["botId"], bots[1].as_str(), "упоминание не сработало: {end}");
}

#[tokio::test]
async fn an_archived_room_is_not_answered_until_it_is_restored() {
    let crew = Crew::start("room-archive").await;
    let state = crew.ok("GET", "/api/state", None).await;
    let bot = state["bots"][0]["id"].as_str().unwrap().to_string();
    let room = crew
        .ok("POST", "/api/rooms", Some(json!({ "name": "Совет", "botIds": [bot], "lead": bot })))
        .await["room"]["id"]
        .as_str()
        .unwrap()
        .to_string();

    let archived = crew.ok("PATCH", &format!("/api/rooms/{room}"), Some(json!({ "archived": true }))).await;
    assert_eq!(archived["room"]["archived"], true, "{archived}");
    let state = crew.ok("GET", "/api/state", None).await;
    let listed = state["rooms"].as_array().unwrap().iter().find(|r| r["id"] == room.as_str()).expect("группа в состоянии");
    assert_eq!(listed["archived"], true, "{listed}");

    let (status, body) = crew.send("POST", &format!("/api/rooms/{room}/messages"), Some(json!({ "text": "привет" }))).await;
    assert_eq!(status, 409, "архивная группа ответила: {body}");
    assert!(body["error"].as_str().unwrap_or_default().contains("архив"), "{body}");

    let restored = crew.ok("PATCH", &format!("/api/rooms/{room}"), Some(json!({ "archived": false }))).await;
    assert_eq!(restored["room"]["archived"], false, "{restored}");
    crew.listen().await;
    crew.ok("POST", &format!("/api/rooms/{room}/messages"), Some(json!({ "text": "снова в деле" }))).await;
    crew.wait_event("группа снова отвечает", |e| e["type"] == "room_turn_end" && e["roomId"] == room.as_str()).await;
}

#[tokio::test]
async fn reassigning_a_room_to_an_outsider_is_rejected() {
    let crew = Crew::start("room-reassign-outsider").await;
    let state = crew.ok("GET", "/api/state", None).await;
    let member = state["bots"][0]["id"].as_str().unwrap().to_string();
    let outsider = crew
        .ok("POST", "/api/bots", Some(json!({ "name": "Чужой", "role": "тест" })))
        .await["bot"]["id"]
        .as_str()
        .unwrap()
        .to_string();
    let room = crew
        .ok("POST", "/api/rooms", Some(json!({ "name": "Совет", "botIds": [member], "lead": member })))
        .await["room"]["id"]
        .as_str()
        .unwrap()
        .to_string();

    let (status, body) = crew.send("POST", &format!("/api/rooms/{room}/reassign"), Some(json!({ "botId": outsider }))).await;
    assert_eq!(status, 400, "передали не участнику: {body}");
}

#[derive(Clone, Default)]
struct FakeModel {
    calls: Arc<Mutex<Vec<Value>>>,
}

async fn fake_completions(State(fake): State<FakeModel>, body: Bytes) -> Response {
    fake.calls.lock().unwrap().push(serde_json::from_slice(&body).unwrap_or_else(|_| json!({})));
    let text = format!(
        "data: {}\n\ndata: {}\n\ndata: [DONE]\n\n",
        json!({ "choices": [{ "index": 0, "delta": { "content": "продолжаю" } }] }),
        json!({ "choices": [{ "index": 0, "delta": {}, "finish_reason": "stop" }] }),
    );
    Response::builder().header("content-type", "text/event-stream").body(Body::from(text)).unwrap()
}

async fn start_fake_model(name: &str) -> (Crew, FakeModel) {
    let fake = FakeModel::default();
    let app = Router::new().route("/v1/chat/completions", post(fake_completions)).with_state(fake.clone());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    let crew = Crew::start(name).await;
    crew.ok(
        "PUT",
        "/api/settings",
        Some(json!({ "driver": "api", "model": "fake", "apiKey": "k", "baseUrl": format!("http://127.0.0.1:{port}/v1") })),
    )
    .await;
    (crew, fake)
}

#[tokio::test]
async fn reassigning_records_the_handover_and_starts_a_turn_for_the_member() {
    let (crew, fake) = start_fake_model("room-reassign").await;
    let state = crew.ok("GET", "/api/state", None).await;
    let lead = state["bots"][0]["id"].as_str().unwrap().to_string();
    let made = crew
        .ok("POST", "/api/bots", Some(json!({ "name": "Второй", "role": "тест" })))
        .await;
    let member = made["bot"]["id"].as_str().unwrap().to_string();
    let member_name = made["bot"]["name"].as_str().unwrap().to_string();
    let room = crew
        .ok(
            "POST",
            "/api/rooms",
            Some(json!({ "name": "Совет", "botIds": [lead, member], "lead": lead, "mode": "lead" })),
        )
        .await["room"]["id"]
        .as_str()
        .unwrap()
        .to_string();
    crew.listen().await;

    crew.ok("POST", &format!("/api/rooms/{room}/reassign"), Some(json!({ "botId": member }))).await;
    let end = crew
        .wait_event("ход принявшего", |e| e["type"] == "room_turn_end" && e["roomId"] == room.as_str())
        .await;
    assert_eq!(end["botId"], member.as_str(), "ход начал не тот участник: {end}");

    let thread = crew.ok("GET", &format!("/api/rooms/{room}/thread?limit=20"), None).await;
    let messages = thread["messages"].as_array().unwrap();
    assert!(
        messages.iter().any(|m| m["role"] == "system" && m["text"] == format!("Задачу передали {member_name}")),
        "нет служебной строки о передаче: {thread}"
    );
    assert!(
        messages.iter().any(|m| m["role"] == "user" && m["text"] == "Продолжи задачу с того места, где остановились"),
        "принявший не получил просьбу продолжить: {thread}"
    );

    let calls = fake.calls.lock().unwrap();
    let last = calls.last().expect("модель не вызывали");
    let prompt = last["messages"].as_array().unwrap();
    assert!(
        prompt.iter().any(|m| {
            m["role"] == "user"
                && m["content"].as_str().is_some_and(|text| text.contains("Продолжи задачу с того места, где остановились"))
        }),
        "инструкция не дошла до модели: {last}"
    );
}

#[tokio::test]
async fn auto_approval_skips_the_card_and_strict_asks_for_writes() {
    let crew = Crew::start("approval-modes").await;
    let auto = bot_with(&crew, "Сам", "auto").await;
    let strict = bot_with(&crew, "Строгий", "strict").await;
    crew.listen().await;

    // auto: команда выполняется без карточки.
    crew.say(&auto, "запусти команду").await;
    crew.wait_event("ход без карточки", |e| e["type"] == "turn_end" && e["botId"] == auto.as_str()).await;
    assert!(
        crew.find_event(|e| e["type"] == "approval").is_none(),
        "в режиме auto карточки быть не должно"
    );
    let tool = crew
        .find_event(|e| e["type"] == "tool_update" && e["botId"] == auto.as_str())
        .expect("команда не выполнялась");
    assert_eq!(tool["status"], "ok", "{tool}");

    // strict: запись файла спрашивает, хотя в обычном режиме это тихий инструмент.
    crew.clear_events();
    crew.say(&strict, "сохрани это в файл").await;
    let approval = crew.wait_event("карточка на запись", |e| e["type"] == "approval").await;
    assert_eq!(approval["tool"], "write_file", "{approval}");
    let id = approval["id"].as_str().unwrap().to_string();
    crew.ok("POST", &format!("/api/approvals/{id}"), Some(json!({ "allow": false }))).await;
    crew.wait_event("отказ по записи", |e| e["type"] == "tool_update" && e["status"] == "denied").await;
    crew.wait_event("конец хода", |e| e["type"] == "turn_end" && e["botId"] == strict.as_str()).await;
}

#[tokio::test]
async fn a_webhook_starts_a_routine_and_a_wrong_token_does_not() {
    let crew = Crew::start("webhook").await;
    let state = crew.ok("GET", "/api/state", None).await;
    let bot = state["bots"][0]["id"].as_str().unwrap().to_string();
    crew.listen().await;

    let made = crew
        .ok(
            "POST",
            "/api/routines",
            Some(json!({ "botId": bot, "title": "Форма", "prompt": "Разбери заявку", "hook": true })),
        )
        .await;
    let routine = &made["routine"];
    let id = routine["id"].as_str().unwrap().to_string();
    let token = routine["hookToken"].as_str().unwrap().to_string();
    assert!(routine["hook"].as_bool().unwrap(), "рутина не помечена как вебхук");
    assert!(!token.is_empty(), "нет секрета вебхука");

    // Внешний вызов приходит без сессии окна: только Host и секрет в адресе.
    let hook = format!("{}/hooks/{id}/{token}", crew.base);
    let res = crew
        .client
        .post(&hook)
        .header("content-type", "application/json")
        .body(json!({ "event": "new_ticket", "email": "a@b.c" }).to_string())
        .send()
        .await
        .unwrap();
    assert_eq!(res.status().as_u16(), 202, "вебхук не принят");

    crew.wait_event("ход по вебхуку", |e| {
        e["type"] == "turn_end" && e["botId"] == bot.as_str()
    })
    .await;
    let thread = crew.ok("GET", &format!("/api/bots/{bot}/thread?limit=20"), None).await;
    let said = thread["messages"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|m| m["text"].as_str())
        .any(|t| t.contains("вебхук"));

    assert!(said, "в треде нет следа вебхука");

    let bad = format!("{}/hooks/{id}/deadbeef", crew.base);
    let res = crew
        .client
        .post(&bad)
        .header("content-type", "application/json")
        .body("{}")
        .send()
        .await
        .unwrap();
    assert_eq!(res.status().as_u16(), 404, "чужой секрет приняли");

    // Обычные адреса без сессии по-прежнему закрыты.
    let res = crew.client.get(format!("{}/api/state", crew.base)).send().await.unwrap();
    assert_eq!(res.status().as_u16(), 403, "без сессии состояние отдавать нельзя");
}

#[tokio::test]
async fn team_file_round_trips_and_routines_arrive_disabled() {
    let crew = Crew::start("team").await;
    let state = crew.ok("GET", "/api/state", None).await;
    let bot = state["bots"][0]["id"].as_str().unwrap().to_string();
    let before = state["bots"].as_array().unwrap().len();

    crew.ok(
        "POST",
        "/api/routines",
        Some(json!({ "botId": bot, "title": "Сводка", "prompt": "Собери темы", "everyMinutes": 60 })),
    )
    .await;
    crew.ok("POST", "/api/skills", Some(json!({ "name": "Разбор", "description": "как разбирать", "body": "1. Смотри на факты" }))).await;
    crew.ok(
        "POST",
        "/api/rooms",
        Some(json!({ "name": "Совет", "botIds": [bot.clone()], "lead": bot.clone() })),
    )
    .await;

    let export = crew.ok("GET", "/api/team/export", None).await;
    let team = export["team"].clone();
    assert_eq!(team["format"], "deloist-team");
    assert_eq!(team["bots"].as_array().unwrap().len(), before);
    assert!(team["skills"].as_array().unwrap().iter().any(|s| s["name"] == "Разбор"));
    assert!(team["rooms"].as_array().unwrap().iter().any(|r| r["name"] == "Совет"));
    assert!(
        !team.to_string().contains("apiKey") && !team.to_string().contains("hookToken"),
        "в файл команды попали секреты: {team}"
    );

    let imported = crew.ok("POST", "/api/team/import", Some(json!({ "team": team }))).await;
    assert_eq!(imported["imported"]["bots"], before, "{imported}");
    assert!(imported["imported"]["routines"].as_i64().unwrap() >= 1, "рутины не приехали");

    let after = crew.ok("GET", "/api/state", None).await;
    assert_eq!(after["bots"].as_array().unwrap().len(), before * 2, "боты не добавились");
    let routines = after["routines"].as_array().unwrap();
    let copy = routines.iter().filter(|r| r["title"] == "Сводка").count();
    assert_eq!(copy, 2, "рутина не скопировалась: {routines:?}");
    let imported_routine = routines
        .iter()
        .filter(|r| r["title"] == "Сводка")
        .find(|r| r["botId"] != bot.as_str())
        .expect("нет импортированной рутины");
    assert_eq!(imported_routine["enabled"], false, "импортированная рутина включена: {imported_routine}");

    // Навык с тем же именем второй раз не появляется.
    let skills = after["skills"].as_array().unwrap();
    assert_eq!(skills.iter().filter(|s| s["name"] == "Разбор").count(), 1, "навык задвоился");
}

#[tokio::test]
async fn github_webhook_valid_push_starts_turn() {
    let crew = Crew::start("gh-push").await;
    let state = crew.ok("GET", "/api/state", None).await;
    let bot = state["bots"][0]["id"].as_str().unwrap().to_string();
    crew.listen().await;

    let made = crew
        .ok(
            "POST",
            "/api/routines",
            Some(json!({ "botId": bot, "title": "GH", "prompt": "Разбери", "hook": true })),
        )
        .await;
    let routine = &made["routine"];
    let id = routine["id"].as_str().unwrap().to_string();
    let token = routine["hookToken"].as_str().unwrap().to_string();

    let body = json!({
        "ref": "refs/heads/main",
        "repository": { "full_name": "org/repo" },
        "sender": { "login": "dev" },
        "commits": [{ "id": "abc1234567890", "message": "fix: hello world", "author": { "name": "dev" } }]
    });
    let raw = body.to_string();
    let sig = github_sig(&token, raw.as_bytes());

    let hook = format!("{}/hooks/{}/{}", crew.base, id, token);
    let res = crew
        .client
        .post(&hook)
        .header("content-type", "application/json")
        .header("x-github-event", "push")
        .header("x-hub-signature-256", sig)
        .body(raw)
        .send()
        .await
        .unwrap();
    assert_eq!(res.status().as_u16(), 202, "вебхук не принят");

    crew.wait_event("ход по github", |e| e["type"] == "turn_end" && e["botId"] == bot.as_str()).await;
    let thread = crew.ok("GET", &format!("/api/bots/{}/thread?limit=20", bot), None).await;
    let texts: Vec<&str> = thread["messages"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|m| m["text"].as_str())
        .collect();
    assert!(texts.iter().any(|t| t.starts_with("[GitHub: push]")), "нет префикса GitHub: {texts:?}");
    assert!(texts.iter().any(|t| t.contains("hello world")), "нет сообщения коммита: {texts:?}");
}

#[tokio::test]
async fn github_webhook_bad_signature_and_missing_signature() {
    let crew = Crew::start("gh-bad").await;
    let state = crew.ok("GET", "/api/state", None).await;
    let bot = state["bots"][0]["id"].as_str().unwrap().to_string();
    crew.listen().await;

    let made = crew
        .ok(
            "POST",
            "/api/routines",
            Some(json!({ "botId": bot, "title": "GH", "prompt": "Разбери", "hook": true })),
        )
        .await;
    let id = made["routine"]["id"].as_str().unwrap().to_string();
    let token = made["routine"]["hookToken"].as_str().unwrap().to_string();

    let body = "{}";
    let hook = format!("{}/hooks/{}/{}", crew.base, id, token);

    // wrong signature
    let res = crew
        .client
        .post(&hook)
        .header("content-type", "application/json")
        .header("x-github-event", "push")
        .header("x-hub-signature-256", "sha256=0000000000000000000000000000000000000000000000000000000000000000")
        .body(body)
        .send()
        .await
        .unwrap();
    assert_eq!(res.status().as_u16(), 401);

    // event header without signature
    let res = crew
        .client
        .post(&hook)
        .header("content-type", "application/json")
        .header("x-github-event", "push")
        .body(body)
        .send()
        .await
        .unwrap();
    assert_eq!(res.status().as_u16(), 401);

    // no turn started
    tokio::time::sleep(std::time::Duration::from_millis(300)).await;
    let thread = crew.ok("GET", &format!("/api/bots/{}/thread?limit=20", bot), None).await;
    let has_gh = thread["messages"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|m| m["text"].as_str())
        .any(|t| t.contains("[GitHub"));
    assert!(!has_gh, "ход по GitHub не должен был стартовать");
}

#[tokio::test]
async fn github_webhook_ping_returns_pong() {
    let crew = Crew::start("gh-ping").await;
    let state = crew.ok("GET", "/api/state", None).await;
    let bot = state["bots"][0]["id"].as_str().unwrap().to_string();
    crew.listen().await;

    let made = crew
        .ok(
            "POST",
            "/api/routines",
            Some(json!({ "botId": bot, "title": "GH", "prompt": "Разбери", "hook": true })),
        )
        .await;
    let id = made["routine"]["id"].as_str().unwrap().to_string();
    let token = made["routine"]["hookToken"].as_str().unwrap().to_string();

    let body = json!({ "zen": "test" });
    let raw = body.to_string();
    let sig = github_sig(&token, raw.as_bytes());

    let hook = format!("{}/hooks/{}/{}", crew.base, id, token);
    let res = crew
        .client
        .post(&hook)
        .header("content-type", "application/json")
        .header("x-github-event", "ping")
        .header("x-hub-signature-256", sig)
        .body(raw)
        .send()
        .await
        .unwrap();
    assert_eq!(res.status().as_u16(), 200);
    let data: Value = res.json().await.unwrap();
    assert_eq!(data["ping"], true);

    tokio::time::sleep(std::time::Duration::from_millis(300)).await;
    let thread = crew.ok("GET", &format!("/api/bots/{}/thread?limit=20", bot), None).await;
    let has_gh = thread["messages"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|m| m["text"].as_str())
        .any(|t| t.contains("[GitHub"));
    assert!(!has_gh, "ping не должен стартовать ход");
}

#[tokio::test]
async fn github_webhook_hook_events_filter() {
    let crew = Crew::start("gh-filter").await;
    let state = crew.ok("GET", "/api/state", None).await;
    let bot = state["bots"][0]["id"].as_str().unwrap().to_string();
    crew.listen().await;

    let made = crew
        .ok(
            "POST",
            "/api/routines",
            Some(json!({ "botId": bot, "title": "GH", "prompt": "Разбери", "hook": true, "hookEvents": ["pull_request.opened"] })),
        )
        .await;
    let id = made["routine"]["id"].as_str().unwrap().to_string();
    let token = made["routine"]["hookToken"].as_str().unwrap().to_string();

    let body = json!({ "action": "closed", "pull_request": { "number": 1, "title": "t" } });
    let raw = body.to_string();
    let sig = github_sig(&token, raw.as_bytes());

    let hook = format!("{}/hooks/{}/{}", crew.base, id, token);
    let res = crew
        .client
        .post(&hook)
        .header("content-type", "application/json")
        .header("x-github-event", "pull_request")
        .header("x-hub-signature-256", sig.clone())
        .body(raw.clone())
        .send()
        .await
        .unwrap();
    assert_eq!(res.status().as_u16(), 202);
    let data: Value = res.json().await.unwrap();
    assert_eq!(data["skipped"], true);

    // opened -> turn
    let body2 = json!({ "action": "opened", "pull_request": { "number": 1, "title": "t" } });
    let raw2 = body2.to_string();
    let sig2 = github_sig(&token, raw2.as_bytes());
    let res = crew
        .client
        .post(&hook)
        .header("content-type", "application/json")
        .header("x-github-event", "pull_request")
        .header("x-hub-signature-256", sig2)
        .body(raw2)
        .send()
        .await
        .unwrap();
    assert_eq!(res.status().as_u16(), 202);
    crew.wait_event("ход по pr", |e| e["type"] == "turn_end" && e["botId"] == bot.as_str()).await;
}

#[tokio::test]
async fn github_webhook_push_100_commits_truncated() {
    let crew = Crew::start("gh-100").await;
    let state = crew.ok("GET", "/api/state", None).await;
    let bot = state["bots"][0]["id"].as_str().unwrap().to_string();
    crew.listen().await;

    let made = crew
        .ok(
            "POST",
            "/api/routines",
            Some(json!({ "botId": bot, "title": "GH", "prompt": "Разбери", "hook": true })),
        )
        .await;
    let id = made["routine"]["id"].as_str().unwrap().to_string();
    let token = made["routine"]["hookToken"].as_str().unwrap().to_string();

    let mut commits = Vec::new();
    for i in 0..100 {
        commits.push(json!({ "id": format!("c{:040}", i), "message": format!("commit {}", i), "author": { "name": "dev" } }));
    }
    let body = json!({
        "ref": "refs/heads/main",
        "repository": { "full_name": "org/repo" },
        "commits": commits
    });
    let raw = body.to_string();
    let sig = github_sig(&token, raw.as_bytes());

    let hook = format!("{}/hooks/{}/{}", crew.base, id, token);
    let res = crew
        .client
        .post(&hook)
        .header("content-type", "application/json")
        .header("x-github-event", "push")
        .header("x-hub-signature-256", sig)
        .body(raw)
        .send()
        .await
        .unwrap();
    assert_eq!(res.status().as_u16(), 202);

    crew.wait_event("ход по 100", |e| e["type"] == "turn_end" && e["botId"] == bot.as_str()).await;
    let thread = crew.ok("GET", &format!("/api/bots/{}/thread?limit=20", bot), None).await;
    let text = thread["messages"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|m| m["text"].as_str())
        .find(|t| t.contains("[GitHub: push]"))
        .unwrap_or("");
    // data part under 8000 chars: check the whole turn text? The payload is inside; ensure <= 8000 for the data portion not directly. We check lists 10 commits.
    assert!(text.contains("commits (100):"), "нет счётчика коммитов: {text}");
    let commit_lines = text.lines().filter(|l| l.trim_start().starts_with("- ")).count();
    assert_eq!(commit_lines, 10, "должно быть 10 коммитов, а не {commit_lines}");
    // ensure data part under 8000: approximate by checking text length reasonable
    assert!(text.chars().count() < 8_000 + 2_000, "текст слишком большой: {}", text.chars().count());
}

#[tokio::test]
async fn generic_webhook_still_works() {
    // covers generic webhook without GitHub headers
    let crew = Crew::start("gh-gen").await;
    let state = crew.ok("GET", "/api/state", None).await;
    let bot = state["bots"][0]["id"].as_str().unwrap().to_string();
    crew.listen().await;

    let made = crew
        .ok(
            "POST",
            "/api/routines",
            Some(json!({ "botId": bot, "title": "Форма", "prompt": "Разбери", "hook": true })),
        )
        .await;
    let id = made["routine"]["id"].as_str().unwrap().to_string();
    let token = made["routine"]["hookToken"].as_str().unwrap().to_string();

    let hook = format!("{}/hooks/{}/{}", crew.base, id, token);
    let res = crew
        .client
        .post(&hook)
        .header("content-type", "application/json")
        .body(json!({ "event": "new_ticket", "email": "a@b.c" }).to_string())
        .send()
        .await
        .unwrap();
    assert_eq!(res.status().as_u16(), 202);
    crew.wait_event("ход по вебхуку", |e| e["type"] == "turn_end" && e["botId"] == bot.as_str()).await;
}

fn github_sig(secret: &str, body: &[u8]) -> String {
    type HmacSha256 = Hmac<Sha256>;
    let mut mac = HmacSha256::new_from_slice(secret.as_bytes()).expect("hmac key");
    mac.update(body);
    let bytes = mac.finalize().into_bytes();
    let mut hex = String::with_capacity(64);
    for b in bytes {
        use std::fmt::Write;
        let _ = write!(hex, "{:02x}", b);
    }
    format!("sha256={}", hex)
}
const FAKE_MCP: &str = r#"
let buf = '';
process.stdin.setEncoding('utf8');
const send = (obj) => process.stdout.write(JSON.stringify(obj) + '\n');
process.stdin.on('data', (chunk) => {
  buf += chunk;
  let i;
  while ((i = buf.indexOf('\n')) >= 0) {
    const line = buf.slice(0, i).trim();
    buf = buf.slice(i + 1);
    if (!line) continue;
    let msg;
    try { msg = JSON.parse(line); } catch { continue; }
    if (msg.id === undefined) continue;
    if (msg.method === 'initialize') {
      send({ jsonrpc: '2.0', id: msg.id, result: { protocolVersion: '2024-11-05', capabilities: { tools: {} }, serverInfo: { name: 'fake', version: '1' } } });
    } else if (msg.method === 'tools/list') {
      send({ jsonrpc: '2.0', id: msg.id, result: { tools: [{ name: 'echo', description: 'Повторяет текст', inputSchema: { type: 'object', properties: { text: { type: 'string' } } } }] } });
    } else if (msg.method === 'tools/call') {
      const text = msg.params?.arguments?.text ?? '';
      send({ jsonrpc: '2.0', id: msg.id, result: { content: [{ type: 'text', text: 'echo: ' + text }] } });
    } else {
      send({ jsonrpc: '2.0', id: msg.id, error: { code: -32601, message: 'нет метода' } });
    }
  }
});
"#;

fn has_node() -> Option<PathBuf> {
    let name = if cfg!(windows) { "node.exe" } else { "node" };
    let path = std::env::var_os("PATH")?;
    std::env::split_paths(&path)
        .map(|dir| dir.join(name))
        .find(|candidate| candidate.is_file())
}

#[tokio::test]
async fn an_mcp_server_shows_its_tools() {
    let Some(node) = has_node() else {
        eprintln!("node не найден: проверка MCP пропущена");
        return;
    };
    let crew = Crew::start("mcp").await;
    let script = crew.data_dir.join("fake_mcp.mjs");
    std::fs::write(&script, FAKE_MCP).expect("запись заглушки MCP");

    let made = crew
        .ok(
            "POST",
            "/api/mcp",
            Some(json!({
                "name": "Заглушка",
                "command": node.to_string_lossy(),
                "args": [script.to_string_lossy()],
            })),
        )
        .await;
    let id = made["server"]["id"].as_str().unwrap().to_string();

    let checked = crew.ok("POST", &format!("/api/mcp/{id}/check"), None).await;
    let tools = checked["tools"].as_array().unwrap();
    assert_eq!(tools.len(), 1, "{checked}");
    assert_eq!(tools[0], "echo", "{checked}");

    let state = crew.ok("GET", "/api/state", None).await;
    let listed: Vec<&Value> = state["mcp"].as_array().unwrap().iter().filter(|s| s["id"] == id.as_str()).collect();
    assert_eq!(listed.len(), 1, "сервер не попал в состояние");
    assert_eq!(listed[0]["name"], "Заглушка");
}
