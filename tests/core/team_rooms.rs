//! Общие папки и навыки команды (в коде и в API это `room`): проверка при записи.

use crate::common::{temp_data_dir, Crew};
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
