//! Общие папки и навыки команды (в коде и в API это `room`): проверка при записи.

use crate::common::{temp_data_dir, wait_until, Crew};
use std::sync::{Arc, Mutex};
use serde_json::{json, Value};

async fn room_with_bot(crew: &Crew) -> (String, String) {
    let bot = crew.ok("POST", "/api/bots", Some(json!({ "name": "Рен" }))).await["bot"]["id"].as_str().unwrap().to_string();
    let room = crew
        .ok("POST", "/api/rooms", Some(json!({ "name": "Совет", "botIds": [bot], "lead": bot })))
        .await["room"]["id"]
        .as_str()
        .unwrap()
        .to_string();
    (bot, room)
}

fn work_dir(name: &str) -> std::path::PathBuf {
    let dir = temp_data_dir(name);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

#[tokio::test]
async fn a_team_gets_shared_folders_and_skills_after_validation() {
    let crew = Crew::start("team-rooms").await;
    let (_bot, room) = room_with_bot(&crew).await;
    let skill = crew.ok("POST", "/api/skills", Some(json!({ "name": "Общий", "body": "делай так" }))).await["skill"]["id"].as_str().unwrap().to_string();
    let work = work_dir("team-rooms-work");
    let path = work.to_str().unwrap();

    let patched = crew.ok("PATCH", &format!("/api/rooms/{room}"), Some(json!({ "folders": [path], "skills": [skill] }))).await;
    assert_eq!(patched["room"]["folders"].as_array().unwrap().len(), 1);
    assert_eq!(patched["room"]["skills"][0], skill);
    // Переименование не трогает общие папки и навыки.
    let renamed = crew.ok("PATCH", &format!("/api/rooms/{room}"), Some(json!({ "name": "Штаб" }))).await;
    assert_eq!(renamed["room"]["name"], "Штаб");
    assert_eq!(renamed["room"]["folders"].as_array().unwrap().len(), 1);

    let bad = |body: Value| {
        let crew = &crew;
        let room = room.clone();
        async move { crew.send("PATCH", &format!("/api/rooms/{room}"), Some(body)).await.0 }
    };
    assert_eq!(bad(json!({ "folders": ["relative"] })).await, 400);
    assert_eq!(bad(json!({ "folders": [work.join("нет").to_str().unwrap()] })).await, 400);
    assert_eq!(bad(json!({ "folders": [crew.data_dir.to_str().unwrap()] })).await, 400, "служебная папка программы");
    assert_eq!(bad(json!({ "folders": [format!("{path}{}..", std::path::MAIN_SEPARATOR)] })).await, 400, "..");
    assert_eq!(bad(json!({ "folders": "C:/x" })).await, 400);
    assert_eq!(bad(json!({ "skills": ["sk_нет"] })).await, 400);
    assert_eq!(bad(json!({ "skills": "x" })).await, 400);
    // Отказ ничего не меняет.
    let state = crew.ok("GET", "/api/state", None).await;
    let room_json = state["rooms"].as_array().unwrap().iter().find(|r| r["id"] == room.as_str()).unwrap().clone();
    assert_eq!(room_json["folders"].as_array().unwrap().len(), 1);

    // Пустой список снимает доступ.
    let cleared = crew.ok("PATCH", &format!("/api/rooms/{room}"), Some(json!({ "folders": [], "skills": [] }))).await;
    assert!(cleared["room"]["folders"].as_array().unwrap().is_empty());
    assert!(cleared["room"]["skills"].as_array().unwrap().is_empty());
    let _ = std::fs::remove_dir_all(&work);
}

#[tokio::test]
async fn a_team_can_be_created_renamed_and_deleted() {
    let crew = Crew::start("team-rooms-crud").await;
    let (_bot, room) = room_with_bot(&crew).await;
    let renamed = crew.ok("PATCH", &format!("/api/rooms/{room}"), Some(json!({ "name": "Другое имя", "instructions": "правило" }))).await;
    assert_eq!(renamed["room"]["name"], "Другое имя");
    assert_eq!(renamed["room"]["instructions"], "правило");
    crew.ok("DELETE", &format!("/api/rooms/{room}"), None).await;
    let (status, _) = crew.send("PATCH", &format!("/api/rooms/{room}"), Some(json!({ "name": "x" }))).await;
    assert_eq!(status, 404);
}

#[tokio::test]
async fn the_window_is_told_when_a_team_is_created_changed_or_deleted() {
    let crew = Crew::start("team-rooms-live").await;
    crew.listen().await;
    let bot = crew.ok("POST", "/api/bots", Some(json!({ "name": "Рен" }))).await["bot"]["id"].as_str().unwrap().to_string();
    crew.clear_events();
    let room = crew.ok("POST", "/api/rooms", Some(json!({ "name": "Совет", "botIds": [bot] }))).await["room"]["id"].as_str().unwrap().to_string();
    crew.wait_event("новая команда", |e| e["type"] == "state_changed").await;
    crew.clear_events();
    crew.ok("PATCH", &format!("/api/rooms/{room}"), Some(json!({ "name": "Штаб" }))).await;
    crew.wait_event("правка команды", |e| e["type"] == "state_changed").await;
    crew.clear_events();
    crew.ok("DELETE", &format!("/api/rooms/{room}"), None).await;
    crew.wait_event("команда удалена", |e| e["type"] == "state_changed").await;
}

/// Мельчайший настоящий png: восемь байт подписи и пустой хвост хватает для проверки по первым байтам.
fn tiny_png() -> Vec<u8> {
    let mut v = vec![0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a];
    v.extend_from_slice(&[0u8; 32]);
    v
}

fn b64(bytes: &[u8]) -> String {
    const T: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::new();
    for chunk in bytes.chunks(3) {
        let n = (chunk[0] as u32) << 16 | (*chunk.get(1).unwrap_or(&0) as u32) << 8 | *chunk.get(2).unwrap_or(&0) as u32;
        for i in 0..4 {
            if i <= chunk.len() {
                out.push(T[(n >> (18 - 6 * i) & 63) as usize] as char);
            } else {
                out.push('=');
            }
        }
    }
    out
}

async fn put_picture(crew: &Crew, room: &str, bytes: Vec<u8>) -> (u16, Value) {
    let data = b64(&bytes);
    let res = crew
        .client
        .post(format!("{}/api/rooms/{room}/picture", crew.base))
        .header("cookie", &crew.cookie)
        .json(&json!({ "data": data }))
        .send()
        .await
        .unwrap();
    let status = res.status().as_u16();
    (status, res.json().await.unwrap_or(Value::Null))
}

#[tokio::test]
async fn a_team_picture_is_checked_by_magic_bytes_served_and_removed_with_the_team() {
    let crew = Crew::start("team-picture").await;
    let (_bot, room) = room_with_bot(&crew).await;

    // Нет картинки: пусто и в данных, и в выдаче.
    let (status, _) = crew.send("GET", &format!("/api/rooms/{room}/picture"), None).await;
    assert_eq!(status, 404);

    // Не картинка (текст под видом png) отклоняется по первым байтам.
    let (status, _) = put_picture(&crew, &room, b"<html>not an image</html>".to_vec()).await;
    assert_eq!(status, 400);
    // Слишком большая — отказ, ничего не записано.
    let mut big = tiny_png();
    big.resize(4 * 1024 * 1024 + 1, 0);
    let (status, _) = put_picture(&crew, &room, big).await;
    assert!(status == 400 || status == 413, "большая картинка принята: {status}");

    let (status, body) = put_picture(&crew, &room, tiny_png()).await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(body["room"]["picture"], "png");
    assert!(body["room"]["pictureAt"].as_i64().unwrap() > 0);

    let res = crew.client.get(format!("{}/api/rooms/{room}/picture", crew.base)).header("cookie", &crew.cookie).send().await.unwrap();
    assert_eq!(res.status().as_u16(), 200);
    assert_eq!(res.headers()["content-type"], "image/png");
    assert_eq!(res.headers()["cache-control"], "no-cache");
    assert_eq!(res.bytes().await.unwrap().to_vec(), tiny_png());
    let file = crew.data_dir.join("rooms").join(&room).join("picture.png");
    assert!(file.exists());

    // Состояние отдаёт картинку вместе с командой.
    let state = crew.ok("GET", "/api/state", None).await;
    assert_eq!(state["rooms"][0]["picture"], "png");

    // Убрать: файл и поле уходят.
    crew.ok("DELETE", &format!("/api/rooms/{room}/picture"), None).await;
    assert!(!file.exists());
    let (status, _) = crew.send("GET", &format!("/api/rooms/{room}/picture"), None).await;
    assert_eq!(status, 404);

    // Удаление команды убирает и папку с картинкой.
    put_picture(&crew, &room, tiny_png()).await;
    assert!(file.exists());
    crew.ok("DELETE", &format!("/api/rooms/{room}"), None).await;
    assert!(!crew.data_dir.join("rooms").join(&room).exists());

    // Без настроенного сервиса «нарисовать заново» отказывает понятно и ничего не тратит.
    let (_bot2, room2) = room_with_bot(&crew).await;
    let (status, _) = crew.send("POST", &format!("/api/rooms/{room2}/picture/generate"), None).await;
    assert_eq!(status, 400);
}

#[derive(Clone, Default)]
struct FakeImages {
    calls: Arc<Mutex<Vec<Value>>>,
}

async fn images_reply(axum::extract::State(fake): axum::extract::State<FakeImages>, axum::Json(body): axum::Json<Value>) -> axum::response::Response {
    use axum::response::IntoResponse;
    fake.calls.lock().unwrap().push(body);
    axum::Json(json!({ "data": [{ "b64_json": "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mNkYPhfDwAChwGA60e6kgAAAABJRU5ErkJggg==" }] })).into_response()
}

#[tokio::test(flavor = "multi_thread")]
async fn a_new_team_gets_a_picture_in_the_background_when_images_are_ready() {
    let fake = FakeImages::default();
    let router = axum::Router::new().route("/v1/images/generations", axum::routing::post(images_reply)).with_state(fake.clone());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
    let crew = Crew::start("team-auto-picture").await;
    let bot = crew.ok("POST", "/api/bots", Some(json!({ "name": "Рен", "role": "Редактор" }))).await["bot"]["id"].as_str().unwrap().to_string();

    // Сервис не настроен: команда создаётся, картинки нет, сервис не вызывали.
    assert_eq!(crew.ok("GET", "/api/state", None).await["imageReady"], false);
    let quiet = crew.ok("POST", "/api/rooms", Some(json!({ "name": "Без картинки", "botIds": [bot] }))).await["room"]["id"].as_str().unwrap().to_string();
    tokio::time::sleep(std::time::Duration::from_millis(400)).await;
    assert_eq!(crew.ok("GET", "/api/state", None).await["rooms"][0]["picture"], "");
    assert!(fake.calls.lock().unwrap().is_empty());
    crew.ok("DELETE", &format!("/api/rooms/{quiet}"), None).await;

    crew.ok("PUT", "/api/settings", Some(json!({ "imageBaseUrl": format!("http://127.0.0.1:{port}/v1"), "imageModel": "fake-image", "imageApiKey": "k", "imageDaily": 5 }))).await;
    assert_eq!(crew.ok("GET", "/api/state", None).await["imageReady"], true);
    let room = crew
        .ok("POST", "/api/rooms", Some(json!({ "name": "Кофейня", "botIds": [bot], "instructions": "Печём булочки" })))
        .await["room"]["id"]
        .as_str()
        .unwrap()
        .to_string();
    wait_until("автокартинка", 10, || async { crew.ok("GET", "/api/state", None).await["rooms"][0]["picture"] == "png" }).await;
    let sent = fake.calls.lock().unwrap()[0].clone();
    let prompt = sent["prompt"].as_str().unwrap();
    assert!(prompt.contains("Кофейня") && prompt.contains("Печём булочки") && prompt.contains("Редактор") && prompt.contains("No text"), "{prompt}");
    assert_eq!(sent["size"], "1024x1024");

    // «Нарисовать заново» заменяет картинку и тратит ещё один рисунок.
    crew.ok("DELETE", &format!("/api/rooms/{room}/picture"), None).await;
    let again = crew.ok("POST", &format!("/api/rooms/{room}/picture/generate"), None).await;
    assert_eq!(again["room"]["picture"], "png");
    assert_eq!(fake.calls.lock().unwrap().len(), 2);
}

#[tokio::test]
async fn a_team_with_an_orchestrator_starts_in_interview_and_keeps_its_leader() {
    let crew = Crew::start("team-orchestrator-room").await;
    crew.listen().await;
    crew.clear_events();
    let (status, made) = crew.send("POST", "/api/rooms", Some(json!({ "name": "Главная", "withOrchestrator": true }))).await;
    assert_eq!(status, 201, "{made}");
    assert_eq!(made["room"]["stage"], "interview");
    assert_eq!(made["room"]["botIds"].as_array().unwrap().len(), 1);
    assert_eq!(made["bot"]["orchestrator"], true);
    assert_eq!(made["bot"]["name"], "Оркестратор");
    assert!(made["bot"]["persona"].as_str().unwrap().contains("propose_team"));
    crew.wait_event("окно узнало", |e| e["type"] == "state_changed").await;
    let room = made["room"]["id"].as_str().unwrap().to_string();
    let orch = made["bot"]["id"].as_str().unwrap().to_string();

    // Без флага поведение прежнее: stage ready, оркестратора нет.
    let plain = crew.ok("POST", "/api/rooms", Some(json!({ "name": "Обычная" }))).await;
    assert_eq!(plain["room"]["stage"], "ready");
    assert!(plain.get("bot").is_none());

    // Оркестратора нельзя убрать, пока команда жива.
    let (status, body) = crew.send("DELETE", &format!("/api/bots/{orch}"), None).await;
    assert_eq!(status, 409, "{body}");
    assert!(body["error"].as_str().unwrap().contains("Оркестратор"));
    let (status, _) = crew.send("PATCH", &format!("/api/rooms/{room}"), Some(json!({ "botIds": [] }))).await;
    assert_eq!(status, 409);
    // Удалили команду — бота снова можно удалить.
    crew.ok("DELETE", &format!("/api/rooms/{room}"), None).await;
    let (status, _) = crew.send("DELETE", &format!("/api/bots/{orch}"), None).await;
    assert_eq!(status, 200);
}

#[tokio::test]
async fn rebuild_brings_an_orchestrator_into_an_old_team() {
    let crew = Crew::start("team-rebuild-old").await;
    let (bot, room) = room_with_bot(&crew).await;
    let (status, res) = crew.send("POST", &format!("/api/rooms/{room}/rebuild"), None).await;
    assert_eq!(status, 200, "{res}");
    assert_eq!(res["room"]["stage"], "interview");
    assert_eq!(res["bot"]["orchestrator"], true);
    assert_eq!(res["room"]["lead"], res["bot"]["id"]);
    assert!(res["room"]["botIds"].as_array().unwrap().contains(&json!(bot)), "старый участник остался");
    let (status, _) = crew.send("POST", "/api/rooms/room_нет/rebuild", None).await;
    assert_eq!(status, 404);
}

#[tokio::test]
async fn a_fresh_install_starts_with_one_team_and_an_orchestrator() {
    let crew = Crew::start_with_env("team-fresh-install", &[("CREW_LEGACY_STARTERS", "0")]).await;
    let state = crew.ok("GET", "/api/state", None).await;
    let bots = state["bots"].as_array().unwrap();
    let rooms = state["rooms"].as_array().unwrap();
    assert_eq!(bots.len(), 1, "{bots:?}");
    assert_eq!(bots[0]["orchestrator"], true);
    assert_eq!(bots[0]["skin"], 13);
    assert_eq!(bots[0]["driver"], "", "драйвер и модель берутся из настроек, как у прежних стартовых ботов");
    assert_eq!(rooms.len(), 1);
    assert_eq!(rooms[0]["name"], "Главная команда");
    assert_eq!(rooms[0]["stage"], "interview");
    assert_eq!(rooms[0]["lead"], bots[0]["id"]);
    // Смена языка переводит и бота, и команду.
    crew.ok("PUT", "/api/settings", Some(json!({ "uiLang": "en" }))).await;
    let state = crew.ok("GET", "/api/state", None).await;
    assert_eq!(state["bots"][0]["name"], "Orchestrator");
    assert_eq!(state["rooms"][0]["name"], "Main team");
}

#[tokio::test(flavor = "multi_thread")]
async fn post_room_with_a_bot_already_in_a_team_returns_4xx_and_creates_no_room() {
    let crew = Crew::start("busy-post").await;
    let bot = crew.ok("POST", "/api/bots", Some(json!({ "name": "Аня" }))).await["bot"]["id"].as_str().unwrap().to_string();
    crew.ok("POST", "/api/rooms", Some(json!({ "name": "Первая", "botIds": [bot.clone()], "lead": bot.clone() }))).await;
    let (status, reply) = crew.send("POST", "/api/rooms", Some(json!({ "name": "Вторая", "botIds": [bot.clone()], "lead": bot.clone() }))).await;
    assert!(status >= 400 && status < 500, "{status} {reply}");
    assert!(reply.to_string().contains("уже в команде"));
    let state = crew.ok("GET", "/api/state", None).await;
    assert!(!state["rooms"].as_array().unwrap().iter().any(|r| r["name"] == "Вторая"));
}

#[tokio::test(flavor = "multi_thread")]
async fn patch_room_adding_a_bot_from_another_team_returns_4xx() {
    let crew = Crew::start("busy-patch").await;
    let b1 = crew.ok("POST", "/api/bots", Some(json!({ "name": "Аня" }))).await["bot"]["id"].as_str().unwrap().to_string();
    let b2 = crew.ok("POST", "/api/bots", Some(json!({ "name": "Боря" }))).await["bot"]["id"].as_str().unwrap().to_string();
    let r2 = crew.ok("POST", "/api/rooms", Some(json!({ "name": "Вторая", "botIds": [b2.clone()], "lead": b2.clone() }))).await["room"]["id"].as_str().unwrap().to_string();
    crew.ok("POST", "/api/rooms", Some(json!({ "name": "Первая", "botIds": [b1.clone()], "lead": b1.clone() }))).await;
    let (status, reply) = crew.send("PATCH", &format!("/api/rooms/{r2}"), Some(json!({ "botIds": [b2.clone(), b1.clone()] }))).await;
    assert!(status >= 400 && status < 500, "{status} {reply}");
    assert!(reply.to_string().contains("уже в команде"));
}

#[tokio::test(flavor = "multi_thread")]
async fn unarchiving_a_room_conflicting_with_another_returns_4xx() {
    let crew = Crew::start("unarch-conflict").await;
    let bot = crew.ok("POST", "/api/bots", Some(json!({ "name": "Аня" }))).await["bot"]["id"].as_str().unwrap().to_string();
    let r1 = crew.ok("POST", "/api/rooms", Some(json!({ "name": "Первая", "botIds": [bot.clone()], "lead": bot.clone() }))).await["room"]["id"].as_str().unwrap().to_string();
    crew.ok("PATCH", &format!("/api/rooms/{r1}"), Some(json!({ "archived": true }))).await;
    crew.ok("POST", "/api/rooms", Some(json!({ "name": "Вторая", "botIds": [bot.clone()], "lead": bot.clone() }))).await;
    let (status, reply) = crew.send("PATCH", &format!("/api/rooms/{r1}"), Some(json!({ "archived": false }))).await;
    assert!(status >= 400 && status < 500, "{status} {reply}");
    assert!(reply.to_string().contains("уже в команде"));
}
