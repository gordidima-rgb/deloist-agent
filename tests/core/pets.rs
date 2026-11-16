//! Стеклянные шары, настройки питомцев и переход из питомца в чат главного окна.

use crate::common;

use common::{Crew, BOOT};
use serde_json::json;

#[tokio::test]
async fn settings_remember_pets_mode_and_refuse_unknown_values() {
    let crew = Crew::start("pets-settings").await;
    let state = crew.ok("GET", "/api/state", None).await;
    assert_eq!(state["settings"]["petsMode"], "off", "по умолчанию выключены");

    let saved = crew.ok("PUT", "/api/settings", Some(json!({ "petsMode": "desktop" }))).await;
    assert_eq!(saved["settings"]["petsMode"], "desktop");

    // Чужие значения не принимаются: остаётся последнее верное.
    crew.ok("PUT", "/api/settings", Some(json!({ "petsMode": "always" }))).await;
    let state = crew.ok("GET", "/api/state", None).await;
    assert_eq!(state["settings"]["petsMode"], "desktop");

    crew.ok("PUT", "/api/settings", Some(json!({ "petsMode": "top" }))).await;
    let state = crew.ok("GET", "/api/state", None).await;
    assert_eq!(state["settings"]["petsMode"], "top");
}

#[tokio::test]
async fn pet_and_orb_files_are_served_with_the_right_types() {
    let crew = Crew::start("pets-files").await;
    let files = [
        ("/pet.html", "text/html; charset=utf-8"),
        ("/pet.js", "text/javascript; charset=utf-8"),
        ("/pet-events.js", "text/javascript; charset=utf-8"),
        #[cfg(deloist_private)]
        ("/orbs.webp", "image/webp"),
        ("/icons/apps/index.json", "application/json"),
        ("/icons/apps/telegram.svg", "image/svg+xml"),
    ];
    for (path, kind) in files {
        let res = crew.client.get(format!("{}{path}", crew.base)).header("cookie", &crew.cookie).send().await.unwrap();
        assert_eq!(res.status().as_u16(), 200, "{path}");
        assert_eq!(res.headers()["content-type"].to_str().unwrap(), kind, "{path}");
        assert!(!res.bytes().await.unwrap().is_empty(), "{path} пустой");
    }
    // Каждый значок из списка приложений на месте: 404 оставил бы на шаре обычную загрузку вместо логотипа.
    let index = crew.client.get(format!("{}/icons/apps/index.json", crew.base)).header("cookie", &crew.cookie).send().await.unwrap().text().await.unwrap();
    let index: serde_json::Value = serde_json::from_str(&index).unwrap();
    assert!(index.as_object().unwrap().len() > 100);
    for slug in index.as_object().unwrap().keys() {
        let path = format!("/icons/apps/{slug}.svg");
        let res = crew.client.get(format!("{}{path}", crew.base)).header("cookie", &crew.cookie).send().await.unwrap();
        assert_eq!(res.status().as_u16(), 200, "{path}");
    }
}

#[tokio::test]
async fn every_bot_has_its_own_orb_and_new_bots_do_not_repeat_neighbours() {
    let crew = Crew::start("pets-orbs").await;
    let state = crew.ok("GET", "/api/state", None).await;
    let skins = |state: &serde_json::Value| -> Vec<i64> { state["bots"].as_array().unwrap().iter().map(|b| b["skin"].as_i64().unwrap()).collect() };
    let first = skins(&state);
    assert!(first.iter().all(|s| (0..20).contains(s)), "{first:?}");
    let mut unique = first.clone();
    unique.dedup();
    assert_eq!(unique.len(), first.len(), "стартовая команда: у каждого свой шар {first:?}");

    // Каждый новый бот получает шар, которого в команде ещё нет, пока шаров хватает.
    for n in 0..17 {
        crew.ok("POST", "/api/bots", Some(json!({ "name": format!("Бот {n}") }))).await;
    }
    let all = skins(&crew.ok("GET", "/api/state", None).await);
    let mut seen = all.clone();
    seen.sort_unstable();
    seen.dedup();
    assert_eq!(seen.len(), 20, "двадцать ботов — двадцать разных шаров: {all:?}");
}

#[tokio::test]
async fn pets_and_orbs_are_closed_without_a_session() {
    let crew = Crew::spawn("pets-security", &[]).await;
    for path in ["/pet.html", "/pet.js", "/pet-events.js", "/orbs.webp", "/icons/apps/index.json", "/icons/apps/telegram.svg"] {
        let status = crew.client.get(format!("{}{path}", crew.base)).send().await.unwrap().status();
        assert_eq!(status.as_u16(), 403, "{path} открыт без сессии");
    }

    let open = crew
        .client
        .post(format!("{}/api/bots/bot_1/open", crew.base))
        .header("content-type", "application/json")
        .body("{}")
        .send()
        .await
        .unwrap();
    assert_eq!(open.status().as_u16(), 403, "маршрут питомцев открыт без сессии");
}

#[tokio::test]
async fn the_pet_entry_link_lands_on_the_pet_page() {
    let crew = Crew::spawn("pets-entry", &[]).await;
    let res = crew
        .client
        .get(format!("{}/?boot={BOOT}&to=/pet.html", crew.base))
        .send()
        .await
        .unwrap();
    assert_eq!(res.status().as_u16(), 303);
    assert_eq!(res.headers()["location"], "/pet.html");
}

#[tokio::test]
async fn opening_a_bot_sends_the_event_and_an_unknown_bot_is_a_404() {
    let crew = Crew::start("pets-open").await;
    let state = crew.ok("GET", "/api/state", None).await;
    let bot = state["bots"][0]["id"].as_str().unwrap().to_string();
    crew.listen().await;

    crew.ok("POST", &format!("/api/bots/{bot}/open"), Some(json!({}))).await;
    let event = crew.wait_event("open_bot", |e| e["type"] == "open_bot" && e["botId"] == bot.as_str()).await;
    assert_eq!(event["botId"], bot);

    let (status, _) = crew.send("POST", "/api/bots/bot_nope/open", Some(json!({}))).await;
    assert_eq!(status, 404);
}

#[cfg(windows)]
#[tokio::test]
async fn a_click_waits_for_the_window_hidden_in_the_tray_and_is_taken_once() {
    let crew = Crew::start("pets-tray").await;
    let state = crew.ok("GET", "/api/state", None).await;
    let bot = state["bots"][0]["id"].as_str().unwrap().to_string();

    assert_eq!(crew.ok("POST", "/api/open-request", None).await["botId"], serde_json::Value::Null);
    crew.ok("POST", &format!("/api/bots/{bot}/open"), Some(json!({}))).await;
    assert_eq!(crew.ok("POST", "/api/open-request", None).await["botId"], bot.as_str());
    assert_eq!(crew.ok("POST", "/api/open-request", None).await["botId"], serde_json::Value::Null, "просьбу исполняют один раз");
}

#[tokio::test]
async fn changes_of_bots_and_settings_reach_the_pets() {
    let crew = Crew::start("pets-changes").await;
    let state = crew.ok("GET", "/api/state", None).await;
    let bot = state["bots"][0]["id"].as_str().unwrap().to_string();
    crew.listen().await;

    crew.ok("PATCH", &format!("/api/bots/{bot}"), Some(json!({ "skin": 3 }))).await;
    crew.wait_event("смена шара", |e| e["type"] == "state_changed").await;
}

#[tokio::test]
async fn server_mode_applies_pets_settings_without_spawning_them() {
    let crew = Crew::start("pets-server").await;
    crew.ok("PUT", "/api/settings", Some(json!({ "petsMode": "desktop" }))).await;
    crew.ok("PUT", "/api/settings", Some(json!({ "petsMode": "top" }))).await;
    tokio::time::sleep(std::time::Duration::from_millis(600)).await;

    let (status, health) = crew.send("GET", "/api/health", None).await;
    assert_eq!(status, 200, "{health}");
    assert_eq!(health["ok"], true);
    // Окно питомцев завело бы свой профиль WebView2; в режиме --server его быть не должно.
    assert!(!crew.data_dir.join("webview-pets").exists(), "в --server питомцы не запускаются");
}

fn chosen(state: &serde_json::Value) -> Vec<String> {
    state["bots"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|bot| bot["pet"] == true)
        .map(|bot| bot["id"].as_str().unwrap().to_string())
        .collect()
}

#[tokio::test]
async fn each_bot_is_chosen_on_its_own_and_the_mode_follows() {
    let crew = Crew::start("pets-choice").await;
    let state = crew.ok("GET", "/api/state", None).await;
    assert!(chosen(&state).is_empty(), "по умолчанию на столе никого");
    let first = state["bots"][0]["id"].as_str().unwrap().to_string();
    let second = state["bots"][1]["id"].as_str().unwrap().to_string();

    // Включили режим, никого не выбрав: показывается первый, иначе включение выглядело бы поломкой.
    crew.ok("PUT", "/api/settings", Some(json!({ "petsMode": "desktop" }))).await;
    assert_eq!(chosen(&crew.ok("GET", "/api/state", None).await), [first.clone()]);

    // Выбор независимый: второй добавился, первый остался.
    crew.ok("PATCH", &format!("/api/bots/{second}"), Some(json!({ "pet": true }))).await;
    assert_eq!(chosen(&crew.ok("GET", "/api/state", None).await), [first.clone(), second.clone()]);
    crew.ok("PATCH", &format!("/api/bots/{first}"), Some(json!({ "pet": false }))).await;
    let state = crew.ok("GET", "/api/state", None).await;
    assert_eq!(chosen(&state), [second.clone()]);
    assert_eq!(state["settings"]["petsMode"], "desktop");

    // Убрали последнего: режим выключился сам.
    crew.ok("PATCH", &format!("/api/bots/{second}"), Some(json!({ "pet": false }))).await;
    let state = crew.ok("GET", "/api/state", None).await;
    assert!(chosen(&state).is_empty());
    assert_eq!(state["settings"]["petsMode"], "off");

    // Выбрали бота при выключенном режиме: человек хочет его видеть.
    crew.ok("PATCH", &format!("/api/bots/{second}"), Some(json!({ "pet": true }))).await;
    assert_eq!(crew.ok("GET", "/api/state", None).await["settings"]["petsMode"], "desktop");

    // Выключили режим: выбор сохраняется, питомцы вернутся те же.
    crew.ok("PUT", "/api/settings", Some(json!({ "petsMode": "off" }))).await;
    crew.ok("PUT", "/api/settings", Some(json!({ "petsMode": "top" }))).await;
    assert_eq!(chosen(&crew.ok("GET", "/api/state", None).await), [second.clone()]);

    // Удалили единственного выбранного: режим не остаётся включённым впустую.
    crew.ok("DELETE", &format!("/api/bots/{second}"), None).await;
    assert_eq!(crew.ok("GET", "/api/state", None).await["settings"]["petsMode"], "off");
}

#[tokio::test]
async fn no_more_than_eight_pets_at_once() {
    let crew = Crew::start("pets-limit").await;
    for i in 0..9 {
        crew.ok("POST", "/api/bots", Some(json!({ "name": format!("Бот {i}") }))).await;
    }
    let state = crew.ok("GET", "/api/state", None).await;
    let ids: Vec<String> = state["bots"].as_array().unwrap().iter().map(|bot| bot["id"].as_str().unwrap().to_string()).collect();
    for id in &ids[..8] {
        crew.ok("PATCH", &format!("/api/bots/{id}"), Some(json!({ "pet": true }))).await;
    }
    let (status, body) = crew.send("PATCH", &format!("/api/bots/{}", ids[8]), Some(json!({ "pet": true }))).await;
    assert_eq!(status, 400, "{body}");
    assert_eq!(chosen(&crew.ok("GET", "/api/state", None).await).len(), 8);
    // Того, кто уже на столе, повторная галочка не задевает, а прочие правки девятого идут как обычно.
    crew.ok("PATCH", &format!("/api/bots/{}", ids[0]), Some(json!({ "pet": true }))).await;
    crew.ok("PATCH", &format!("/api/bots/{}", ids[8]), Some(json!({ "role": "ждёт" }))).await;
}

#[tokio::test]
async fn a_pet_remembers_its_place_quietly() {
    let crew = Crew::start("pets-place").await;
    let state = crew.ok("GET", "/api/state", None).await;
    let bot = state["bots"][0]["id"].as_str().unwrap().to_string();
    assert_eq!(state["bots"][0]["petX"], serde_json::Value::Null, "пока не двигали, места нет");
    crew.listen().await;

    // Экран слева от основного даёт отрицательные координаты.
    let place = format!("/api/bots/{bot}/pet-place");
    crew.ok("POST", &place, Some(json!({ "x": -1200, "y": 340 }))).await;
    let state = crew.ok("GET", "/api/state", None).await;
    assert_eq!(state["bots"][0]["petX"], -1200);
    assert_eq!(state["bots"][0]["petY"], 340);
    // Сотня таких записей за перетаскивание не должна перерисовывать все окна.
    tokio::time::sleep(std::time::Duration::from_millis(200)).await;
    assert_eq!(crew.count_events(|e| e["type"] == "state_changed"), 0);

    for bad in [json!({ "x": 1 }), json!({ "x": "1", "y": 2 }), json!({ "x": 99999, "y": 2 }), json!({ "x": 1.5, "y": 2 })] {
        let (status, body) = crew.send("POST", &place, Some(bad.clone())).await;
        assert_eq!(status, 400, "{bad}: {body}");
    }
    let state = crew.ok("GET", "/api/state", None).await;
    assert_eq!(state["bots"][0]["petX"], -1200, "неверные значения не портят место");

    let (status, _) = crew.send("POST", "/api/bots/bot_nope/pet-place", Some(json!({ "x": 1, "y": 2 }))).await;
    assert_eq!(status, 404);
}

#[tokio::test]
async fn pets_chosen_before_the_choice_existed_stay_on_the_desktop() {
    let mut crew = Crew::start("pets-upgrade").await;
    let data_dir = crew.data_dir.clone();
    crew.keep_data();
    // В файле выпуска 0.2.0 режим есть, а выбора по ботам ещё нет.
    crew.ok("PUT", "/api/settings", Some(json!({ "petsMode": "desktop" }))).await;
    drop(crew);
    tokio::time::sleep(std::time::Duration::from_millis(500)).await;
    let file = data_dir.join("state.json");
    let mut old: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(&file).unwrap()).unwrap();
    for bot in old["bots"].as_array_mut().unwrap() {
        bot.as_object_mut().unwrap().remove("pet");
    }
    std::fs::write(&file, serde_json::to_string(&old).unwrap()).unwrap();

    let crew = Crew::start_at(data_dir, &[]).await;
    let state = crew.ok("GET", "/api/state", None).await;
    assert_eq!(state["settings"]["petsMode"], "desktop");
    assert_eq!(chosen(&state).len(), state["bots"].as_array().unwrap().len().min(6), "включённые раньше не пропали");
}

#[tokio::test]
async fn orb_numbers_are_kept_in_range_and_old_bots_get_one() {
    let crew = Crew::start("pets-skin").await;
    let state = crew.ok("GET", "/api/state", None).await;
    let bot = state["bots"][0]["id"].as_str().unwrap().to_string();
    let patch = |value: i64| json!({ "skin": value });

    let saved = crew.ok("PATCH", &format!("/api/bots/{bot}"), Some(patch(999))).await;
    assert_eq!(saved["bot"]["skin"], 19);

    let saved = crew.ok("PATCH", &format!("/api/bots/{bot}"), Some(patch(-4))).await;
    assert_eq!(saved["bot"]["skin"], 0);

    let saved = crew.ok("PATCH", &format!("/api/bots/{bot}"), Some(patch(2))).await;
    assert_eq!(saved["bot"]["skin"], 2);
}

#[tokio::test]
async fn bots_saved_with_the_old_look_get_an_orb_and_keep_their_data() {
    let mut crew = Crew::start("pets-old-look").await;
    let data_dir = crew.data_dir.clone();
    crew.keep_data();
    drop(crew);
    tokio::time::sleep(std::time::Duration::from_millis(500)).await;
    // Файл выпуска 0.3.4: у бота форма, уши и персонаж, шара нет.
    let file = data_dir.join("state.json");
    let mut old: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(&file).unwrap()).unwrap();
    for bot in old["bots"].as_array_mut().unwrap() {
        let bot = bot.as_object_mut().unwrap();
        bot.remove("skin");
        bot.insert("shape".into(), json!(3));
        bot.insert("ears".into(), json!(2));
        bot.insert("character".into(), json!(5));
    }
    old["settings"]["avatarMaterial"] = json!("anime");
    std::fs::write(&file, serde_json::to_string(&old).unwrap()).unwrap();

    let crew = Crew::start_at(data_dir.clone(), &[]).await;
    let state = crew.ok("GET", "/api/state", None).await;
    let bots = state["bots"].as_array().unwrap();
    let skins: Vec<i64> = bots.iter().map(|b| b["skin"].as_i64().unwrap()).collect();
    assert!(skins.iter().all(|s| (0..20).contains(s)), "{skins:?}");
    let mut unique = skins.clone();
    unique.sort_unstable();
    unique.dedup();
    assert_eq!(unique.len(), skins.len(), "соседи не повторяются: {skins:?}");
    assert_eq!(bots[0]["name"], old["bots"][0]["name"], "остальное не потеряно");
    // Прежние поля остаются в файле: откат к старой версии ничего не потеряет.
    let saved: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(data_dir.join("state.json")).unwrap()).unwrap();
    assert_eq!(saved["bots"][0]["shape"], 3);
    assert_eq!(saved["bots"][0]["character"], 5);
    assert_eq!(saved["bots"][0]["skin"], skins[0]);
}
