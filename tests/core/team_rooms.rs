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
