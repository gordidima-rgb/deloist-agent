//! Перенос ботов из Hermes Agent: список профилей без содержимого, создание ботов и память.

use std::path::{Path, PathBuf};

use serde_json::{json, Value};

use crate::common::{self, Crew};

fn write(path: &Path, text: impl AsRef<[u8]>) {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).unwrap();
    }
    std::fs::write(path, text).unwrap();
}

fn persona_chars(text: &str) -> usize {
    text.chars().count()
}

fn profile<'a>(data: &'a Value, name: &str) -> &'a Value {
    data["profiles"]
        .as_array()
        .unwrap()
        .iter()
        .find(|item| item["name"] == name)
        .unwrap_or_else(|| panic!("нет профиля {name}: {data}"))
}

#[tokio::test]
async fn lists_without_contents_and_creates_a_bot_with_persona_and_memory() {
    let root: PathBuf = common::temp_data_dir("hermes-root");
    let persona = "Ты — помощник Hermes.\nОтвечай коротко.";
    let work_persona = "Рабочая персона Hermes.";
    // Секрет лежит в двух файлах, которые читать нельзя ни при каких условиях.
    let secret = "sk-hermes-fake-secret-4821";
    write(&root.join("SOUL.md"), persona);
    write(&root.join("memories").join("MEMORY.md"), " Любит кофе \n§\nЖивёт в Новосибирске ");
    write(&root.join("memories").join("USER.md"), "Работает над Deloist");
    write(&root.join(".env"), format!("HERMES_API_KEY={secret}\n"));
    write(&root.join("auth.json"), format!("{{\"token\":\"{secret}\"}}\n"));
    write(&root.join("profiles").join("work").join("SOUL.md"), work_persona);
    write(&root.join("profiles").join("work").join("memories").join("MEMORY.md"), "Ведёт проект");
    write(&root.join("profiles").join("work").join(".env"), format!("HERMES_API_KEY={secret}\n"));
    write(&root.join("profiles").join("work").join("auth.json"), format!("{{\"token\":\"{secret}\"}}\n"));
    write(&root.join("profiles").join("work").join("state.db"), "not a database");

    let hermes = root.to_string_lossy().to_string();
    let crew = Crew::start_with_env("hermes-import", &[("HERMES_HOME", &hermes)]).await;

    // GET: только счётчики. Ни персоны, ни памяти, ни секретов в ответе нет.
    let listed = crew
        .client
        .get(format!("{}/api/import/hermes", crew.base))
        .header("cookie", &crew.cookie)
        .send()
        .await
        .unwrap();
    assert_eq!(listed.status().as_u16(), 200);
    let listed = listed.text().await.unwrap();
    assert!(!listed.contains("Ты — помощник Hermes"), "содержимое SOUL.md попало в список: {listed}");
    assert!(!listed.contains("Рабочая персона"), "содержимое SOUL.md профиля попало в список: {listed}");
    assert!(!listed.contains("Любит кофе"), "запись памяти попала в список: {listed}");
    assert!(!listed.contains(secret), "секрет Hermes попал в список: {listed}");
    let data: Value = serde_json::from_str(&listed).unwrap();
    assert_eq!(data["found"], true);
    assert_eq!(data["root"], hermes);
    assert_eq!(data["profiles"].as_array().unwrap().len(), 2);
    let default = profile(&data, "default");
    assert_eq!(default["personaChars"], persona_chars(persona));
    assert_eq!(default["memoryEntries"], 3);
    let work = profile(&data, "work");
    assert_eq!(work["personaChars"], persona_chars(work_persona));
    assert_eq!(work["memoryEntries"], 1);

    // POST: по боту на профиль, персоны и факты памяти на месте.
    let created = crew
        .ok("POST", "/api/import/hermes", Some(json!({ "profiles": ["default", "work"] })))
        .await;
    assert!(!created.to_string().contains(secret), "секрет вернулся из POST: {created}");
    let ids = created["botIds"].as_array().unwrap();
    assert_eq!(ids.len(), 2, "{created}");

    let state = crew.ok("GET", "/api/state", None).await;
    assert!(!state.to_string().contains(secret), "секрет попал в состояние: {state}");
    let bots = state["bots"].as_array().unwrap();
    let default_bot = bots.iter().find(|bot| bot["name"] == "Hermes · default").expect("бот профиля default");
    let work_bot = bots.iter().find(|bot| bot["name"] == "work").expect("бот профиля work");
    for bot in [default_bot, work_bot] {
        assert_eq!(bot["role"], "перенесён из Hermes");
        assert!(!bot.to_string().contains(secret), "секрет попал в бота: {bot}");
    }
    assert_eq!(default_bot["persona"], persona);
    assert_eq!(work_bot["persona"], work_persona);

    let default_memory = crew
        .ok("GET", &format!("/api/bots/{}/memory", default_bot["id"].as_str().unwrap()), None)
        .await;
    assert!(!default_memory.to_string().contains(secret), "секрет попал в память: {default_memory}");
    let titles: Vec<&str> = default_memory["facts"].as_array().unwrap().iter().map(|fact| fact["title"].as_str().unwrap()).collect();
    assert!(titles.contains(&"Любит кофе"), "{default_memory}");
    assert!(titles.contains(&"Живёт в Новосибирске"), "{default_memory}");
    assert!(titles.contains(&"Работает над Deloist"), "{default_memory}");

    let work_memory = crew
        .ok("GET", &format!("/api/bots/{}/memory", work_bot["id"].as_str().unwrap()), None)
        .await;
    assert_eq!(work_memory["facts"].as_array().unwrap().len(), 1, "{work_memory}");
    assert_eq!(work_memory["facts"][0]["title"], "Ведёт проект");

    drop(crew);
    let _ = std::fs::remove_dir_all(&root);
}

#[tokio::test]
async fn unknown_and_unsafe_profile_names_are_refused_and_persona_is_capped() {
    let root: PathBuf = common::temp_data_dir("hermes-root-caps");
    let long_persona = "я".repeat(12_500);
    write(&root.join("SOUL.md"), &long_persona);

    let hermes = root.to_string_lossy().to_string();
    let crew = Crew::start_with_env("hermes-caps", &[("HERMES_HOME", &hermes)]).await;

    let listed = crew.ok("GET", "/api/import/hermes", None).await;
    assert_eq!(profile(&listed, "default")["personaChars"], 12_500);

    let (status, data) = crew.send("POST", "/api/import/hermes", Some(json!({ "profiles": ["nope"] }))).await;
    assert_eq!(status, 400, "{data}");
    assert!(data["error"].as_str().unwrap().contains("не найден"), "сообщение не по-русски: {data}");

    for name in ["../default", "work/../default", "work/default", "/absolute", "C:\\hermes", ".."] {
        let (status, data) = crew.send("POST", "/api/import/hermes", Some(json!({ "profiles": [name] }))).await;
        assert_eq!(status, 400, "имя {name} прошло: {data}");
        assert!(data["error"].as_str().unwrap().contains("Недопустимое имя"), "{name}: {data}");
    }
    let state = crew.ok("GET", "/api/state", None).await;
    let hermes_bot = state["bots"].as_array().unwrap().iter().find(|bot| bot["name"] == "Hermes · default");
    assert!(hermes_bot.is_none(), "отказ создал бота: {state}");

    let created = crew.ok("POST", "/api/import/hermes", Some(json!({ "profiles": ["default"] }))).await;
    let id = created["botIds"][0].as_str().unwrap().to_string();
    let state = crew.ok("GET", "/api/state", None).await;
    let bot = state["bots"].as_array().unwrap().iter().find(|bot| bot["id"] == id).unwrap();
    assert_eq!(bot["persona"].as_str().unwrap().chars().count(), 12_000, "персона не обрезана");

    drop(crew);
    let _ = std::fs::remove_dir_all(&root);
}
