//! Док агентов: настройки, выбор ботов, кнопка «+» и страница дока.

use crate::common;

use common::{Crew, BOOT};
use serde_json::json;

#[tokio::test]
async fn dock_settings_have_defaults_are_saved_and_refuse_unknown_values() {
    let crew = Crew::start("dock-settings").await;
    let settings = crew.ok("GET", "/api/state", None).await["settings"].clone();
    assert_eq!(settings["dockEnabled"], true, "док включён сразу, как в образце");
    assert_eq!(settings["dockLayer"], "top");
    assert_eq!(settings["dockDisplay"], "");
    assert_eq!(settings["dockEdge"], "right");
    assert_eq!(settings["dockAutohide"], false);
    assert_eq!(settings["dockResults"], true);
    assert_eq!(settings["dockActivity"], true);
    assert_eq!(settings["uiTheme"], "");

    let saved = crew
        .ok(
            "PUT",
            "/api/settings",
            Some(json!({
                "dockEnabled": false,
                "dockLayer": "normal",
                "dockDisplay": r"\\.\DISPLAY2",
                "dockEdge": "left",
                "dockOffset": 0.75,
                "dockAutohide": true,
                "dockResults": false,
                "dockActivity": false,
                "uiTheme": "dark",
            })),
        )
        .await["settings"]
        .clone();
    assert_eq!(saved["dockEnabled"], false);
    assert_eq!(saved["dockLayer"], "normal");
    assert_eq!(saved["dockDisplay"], r"\\.\DISPLAY2");
    assert_eq!(saved["dockEdge"], "left");
    assert_eq!(saved["dockOffset"], 0.75);
    assert_eq!(saved["dockAutohide"], true);
    assert_eq!(saved["dockResults"], false);
    assert_eq!(saved["dockActivity"], false);
    assert_eq!(saved["uiTheme"], "dark");

    // Чужие значения не принимаются, место вдоль края не выходит за 0..1.
    crew.ok("PUT", "/api/settings", Some(json!({ "dockLayer": "under", "dockEdge": "top", "dockOffset": 7, "uiTheme": "pink" }))).await;
    let settings = crew.ok("GET", "/api/state", None).await["settings"].clone();
    assert_eq!(settings["dockLayer"], "normal");
    assert_eq!(settings["dockEdge"], "left");
    assert_eq!(settings["dockOffset"], 1.0);
    assert_eq!(settings["uiTheme"], "dark");
    crew.ok("PUT", "/api/settings", Some(json!({ "dockOffset": -3 }))).await;
    assert_eq!(crew.ok("GET", "/api/state", None).await["settings"]["dockOffset"], 0.0);

    // «Вместе с окном программы» — пустое имя дисплея.
    crew.ok("PUT", "/api/settings", Some(json!({ "dockDisplay": "" }))).await;
    assert_eq!(crew.ok("GET", "/api/state", None).await["settings"]["dockDisplay"], "");
}

#[tokio::test]
async fn every_bot_is_in_the_dock_until_taken_out_and_new_ones_arrive_there() {
    let crew = Crew::start("dock-bots").await;
    let state = crew.ok("GET", "/api/state", None).await;
    let bots = state["bots"].as_array().unwrap();
    assert!(bots.iter().all(|bot| bot["dock"] == true), "все боты в доке");
    let first = bots[0]["id"].as_str().unwrap().to_string();

    crew.listen().await;
    crew.ok("PATCH", &format!("/api/bots/{first}"), Some(json!({ "dock": false }))).await;
    crew.wait_event("бот ушёл из дока", |e| e["type"] == "state_changed").await;
    let state = crew.ok("GET", "/api/state", None).await;
    assert_eq!(state["bots"][0]["dock"], false);
    assert_eq!(state["bots"][1]["dock"], true, "другие остались");

    // Новый бот появляется в доке, и док узнаёт об этом сразу.
    crew.clear_events();
    let created = crew.ok("POST", "/api/bots", Some(json!({ "name": "Новичок" }))).await;
    assert_eq!(created["bot"]["dock"], true);
    crew.wait_event("новый бот", |e| e["type"] == "state_changed").await;
}

#[tokio::test]
async fn the_plus_opens_creation_in_the_main_window_even_from_the_tray() {
    let crew = Crew::start("dock-new").await;
    crew.listen().await;
    crew.ok("POST", "/api/dock/new-bot", None).await;
    crew.wait_event("создание бота", |e| e["type"] == "open_new_bot").await;
    // Окно было в трее: просьба ждёт его открытия и исполняется один раз.
    assert_eq!(crew.ok("POST", "/api/open-request", None).await["botId"], "+new");
    assert_eq!(crew.ok("POST", "/api/open-request", None).await["botId"], serde_json::Value::Null);
}

#[tokio::test]
async fn displays_are_listed_with_their_windows_names() {
    let crew = Crew::start("dock-displays").await;
    let list = crew.ok("GET", "/api/dock/displays", None).await;
    let displays = list["displays"].as_array().unwrap();
    assert!(!displays.is_empty(), "хотя бы один монитор есть всегда");
    assert_eq!(displays.iter().filter(|d| d["primary"] == true).count(), 1, "{list}");
    for display in displays {
        assert!(!display["id"].as_str().unwrap().is_empty());
        assert!(display["width"].as_i64().unwrap() > 0);
    }
}

#[tokio::test]
async fn the_dock_page_is_served_and_closed_without_a_session() {
    let crew = Crew::start("dock-files").await;
    for (path, kind) in [("/dock.html", "text/html; charset=utf-8"), ("/dock.js", "text/javascript; charset=utf-8")] {
        let res = crew.client.get(format!("{}{path}", crew.base)).header("cookie", &crew.cookie).send().await.unwrap();
        assert_eq!(res.status().as_u16(), 200, "{path}");
        assert_eq!(res.headers()["content-type"].to_str().unwrap(), kind, "{path}");
    }

    let stranger = Crew::spawn("dock-security", &[]).await;
    for path in ["/dock.html", "/dock.js", "/kit.css", "/ui/kit.js", "/api/dock/displays"] {
        let status = stranger.client.get(format!("{}{path}", stranger.base)).send().await.unwrap().status();
        assert_eq!(status.as_u16(), 403, "{path} открыт без сессии");
    }
    let status = stranger.client.post(format!("{}/api/dock/new-bot", stranger.base)).send().await.unwrap().status();
    assert_eq!(status.as_u16(), 403);

    // Ссылка входа ведёт на страницу дока и никуда больше.
    let res = stranger.client.get(format!("{}/?boot={BOOT}&to=/dock.html", stranger.base)).send().await.unwrap();
    assert_eq!(res.status().as_u16(), 303);
    assert_eq!(res.headers()["location"], "/dock.html");
}

#[tokio::test]
async fn server_mode_keeps_the_dock_closed() {
    let crew = Crew::start("dock-server").await;
    crew.ok("PUT", "/api/settings", Some(json!({ "dockEnabled": false }))).await;
    crew.ok("PUT", "/api/settings", Some(json!({ "dockEnabled": true }))).await;
    tokio::time::sleep(std::time::Duration::from_millis(600)).await;
    // Окно дока завело бы профиль WebView2; в режиме --server его быть не должно.
    assert!(!crew.data_dir.join("webview-pets").exists(), "в --server дока нет");
    assert!(!crew.data_dir.join("dock.log").exists());
}
#[tokio::test]
async fn changing_appearance_notifies_the_dock_with_updated_state() {
    let crew = Crew::start("dock-look").await;
    let state = crew.ok("GET", "/api/state", None).await;
    let id = state["bots"][0]["id"].as_str().unwrap();
    crew.listen().await;
    crew.ok("PATCH", &format!("/api/bots/{id}"), Some(json!({"skin": 2, "color": "#123456"}))).await;
    crew.wait_event("обновление профиля", |e| e["type"] == "state_changed").await;
    let state = crew.ok("GET", "/api/state", None).await;
    let bot = state["bots"].as_array().unwrap().iter().find(|b| b["id"] == id).unwrap();
    assert_eq!(bot["skin"], 2);
    assert_eq!(bot["color"], "#123456");
}

#[tokio::test]
async fn every_bot_carries_its_binding_limit_for_the_ring() {
    let crew = Crew::start("dock-limit").await;
    // По умолчанию предел токенов команды на день есть: кольцо показывает его.
    let state = crew.ok("GET", "/api/state", None).await;
    for bot in state["bots"].as_array().unwrap() {
        let limit = &bot["limit"];
        assert_eq!(limit["source"], "tokens", "{bot}");
        assert_eq!(limit["unit"], "tokens");
        assert_eq!(limit["cap"], 1_000_000);
        let left = limit["left"].as_f64().unwrap();
        assert!((0.0..=1.0).contains(&left), "доля от 0 до 1: {left}");
        assert!(limit["moneyLeftKop"].is_null(), "у бота не на кошельке денег нет");
    }
    // Без пределов кольца нет: только тусклый обод цвета бота.
    crew.ok("PUT", "/api/settings", Some(json!({ "dailyTokenLimit": 0 }))).await;
    let state = crew.ok("GET", "/api/state", None).await;
    assert!(state["bots"].as_array().unwrap().iter().all(|bot| bot["limit"].is_null()), "{state}");
}

#[tokio::test]
async fn the_voice_target_is_the_last_one_or_a_chosen_bot() {
    let crew = Crew::start("dock-voice").await;
    let state = crew.ok("GET", "/api/state", None).await;
    assert_eq!(state["settings"]["voiceTarget"], "", "по умолчанию — последнему, с кем говорил");
    let id = state["bots"][1]["id"].as_str().unwrap().to_string();
    let saved = crew.ok("PUT", "/api/settings", Some(json!({ "voiceTarget": id }))).await;
    assert_eq!(saved["settings"]["voiceTarget"], id.as_str());
    let saved = crew.ok("PUT", "/api/settings", Some(json!({ "voiceTarget": "last" }))).await;
    assert_eq!(saved["settings"]["voiceTarget"], "");
}

#[tokio::test]
async fn the_dock_loads_the_kit_and_its_svelte_components() {
    let crew = Crew::start("dock-kit").await;
    let get = |path: &str| {
        let url = format!("{}{path}", crew.base);
        let req = crew.client.get(url).header("cookie", &crew.cookie);
        async move {
            let res = req.send().await.unwrap();
            (res.status().as_u16(), res.text().await.unwrap())
        }
    };
    let (status, page) = get("/dock.html").await;
    assert_eq!(status, 200);
    for needle in ["/kit.css", "/ui/kit.css", "/ui/kit.js", "/dock.js"] {
        assert!(page.contains(needle), "док подключает {needle}");
    }
    assert!(!page.contains("class=\"seats\""), "старая разметка островка ушла в компонент");
    let (status, css) = get("/kit.css").await;
    assert_eq!(status, 200);
    for class in [".k-ring", ".k-mic", ".k-gear-tray", ".k-pix", ".k-mic-look-b", ".k-mic-look-c", ".k-seat.is-speaking"] {
        assert!(css.contains(class), "в ките есть {class}");
    }
    let (status, ui) = get("/ui/kit.js").await;
    assert_eq!(status, 200);
    for mount in ["mountDock", "mountStepPill", "mountMicButton", "mountMenu"] {
        assert!(ui.contains(mount), "DeloistUI.{mount}");
    }
    // Поддельный голос только в отладочной сборке, и state говорит доку, что можно его взять.
    let (status, _) = get("/dock-fake-voice.js").await;
    assert_eq!(status, if cfg!(debug_assertions) { 200 } else { 404 });
    assert_eq!(crew.ok("GET", "/api/state", None).await["debug"], cfg!(debug_assertions));
    // Нет бесконечной анимации на странице дока: ни в стилях дока, ни в ките, кроме девяти пикселей.
    let (_, script) = get("/dock.js").await;
    assert!(!script.contains("animation: pix"), "четырёхпиксельный значок заменён");
    assert!(!page.contains("infinite"), "в стилях дока нет бесконечных анимаций");
    let infinite: Vec<_> = css.lines().filter(|l| l.contains("infinite")).collect();
    assert_eq!(infinite.len(), 1, "в ките бесконечно дышат только девять пикселей: {infinite:?}");
    assert!(infinite[0].contains("k-pix"));
}

#[tokio::test]
async fn reading_answers_aloud_is_a_bot_switch_from_the_chat_menu() {
    let crew = Crew::start("dock-read-aloud").await;
    let state = crew.ok("GET", "/api/state", None).await;
    assert_eq!(state["bots"][0]["readAloud"], false, "по умолчанию ответы не озвучиваются");
    let id = state["bots"][0]["id"].as_str().unwrap().to_string();
    crew.ok("PATCH", &format!("/api/bots/{id}"), Some(json!({ "readAloud": true }))).await;
    let state = crew.ok("GET", "/api/state", None).await;
    assert_eq!(state["bots"][0]["readAloud"], true);
    assert_eq!(state["bots"][1]["readAloud"], false, "другие боты не тронуты");
    crew.ok("PATCH", &format!("/api/bots/{id}"), Some(json!({ "readAloud": false }))).await;
    let state = crew.ok("GET", "/api/state", None).await;
    assert_eq!(state["bots"][0]["readAloud"], false);
}
