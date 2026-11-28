//! Демо-драйвер целиком: инструменты, подтверждение, отказ, делегирование, память, файлы.

use crate::common;

use common::Crew;
use serde_json::{json, Value};

#[tokio::test]
async fn demo_turns_go_through_the_whole_loop() {
    let crew = Crew::start("smoke").await;
    let state = crew.ok("GET", "/api/state", None).await;
    let bots = state["bots"].as_array().unwrap();
    assert_eq!(bots.len(), 3, "ожидал трёх ботов");
    assert_eq!(state["settings"]["driver"], "demo");
    let bot = bots[0]["id"].as_str().unwrap().to_string();
    crew.listen().await;

    // Инструмент без подтверждения.
    crew.say(&bot, "покажи файлы в воркспейсе").await;
    crew.wait_event("конец хода с list_files", |e| e["type"] == "turn_end" && e["botId"] == bot.as_str()).await;
    let tool = crew.find_event(|e| e["type"] == "tool_update" && e["botId"] == bot.as_str()).expect("инструмент не вызывался");
    assert_eq!(tool["status"], "ok");

    // Читающая команда в своей папке идёт без карточки.
    crew.clear_events();
    crew.say(&bot, "запусти команду").await;
    crew.wait_event("конец хода с ver", |e| e["type"] == "turn_end" && e["botId"] == bot.as_str()).await;
    assert!(crew.find_event(|e| e["type"] == "approval").is_none(), "читающая команда спросила разрешение");
    let ran = crew.find_event(|e| e["type"] == "tool_update" && e["botId"] == bot.as_str()).expect("команда не выполнялась");
    assert_eq!(ran["status"], "ok", "{ran}");

    // Команда с подтверждением: строгий режим спрашивает на любую команду.
    crew.ok("PATCH", &format!("/api/bots/{bot}"), Some(json!({ "approval": "strict" }))).await;
    crew.clear_events();
    crew.say(&bot, "запусти команду").await;
    let approval = crew.wait_event("карточка подтверждения", |e| e["type"] == "approval").await;
    assert_eq!(approval["tool"], "run_command");
    let id = approval["id"].as_str().unwrap();

    let pending = crew.ok("GET", &format!("/api/bots/{bot}/pending"), None).await;
    assert!(pending["pending"].as_array().unwrap().iter().any(|p| p["id"] == id), "висящее подтверждение не видно в /pending");

    let (busy, queued) = crew.send("POST", &format!("/api/bots/{bot}/messages"), Some(json!({ "text": "ещё" }))).await;
    assert_eq!(busy, 202, "занятый бот принимает сообщение в очередь");
    assert_eq!(queued["queued"], true, "{queued}");
    let qid = queued["item"]["id"].as_str().unwrap();
    // Убираем, чтобы очередь не начала отдельный ход посреди сценария.
    crew.ok("DELETE", &format!("/api/bots/{bot}/queue/{qid}"), None).await;

    crew.ok("POST", &format!("/api/approvals/{id}"), Some(json!({ "allow": true }))).await;
    crew.wait_event("конец хода с командой", |e| e["type"] == "turn_end").await;
    let done = crew.find_event(|e| e["type"] == "tool_update").unwrap();
    assert_eq!(done["status"], "ok", "{done}");
    assert!(done["result"].as_str().unwrap().contains("код выхода: 0"), "{done}");

    // Тред сохранился.
    let thread = crew.ok("GET", &format!("/api/bots/{bot}/thread?limit=50"), None).await;
    let messages = thread["messages"].as_array().unwrap();
    assert!(messages.len() >= 4, "в треде мало сообщений: {}", messages.len());
    assert!(messages.iter().any(|m| m["toolCalls"].as_array().is_some_and(|c| !c.is_empty())), "в треде нет инструментов");

    // Отказ: команда не выполняется.
    crew.clear_events();
    crew.say(&bot, "запусти команду echo привет").await;
    let approval = crew.wait_event("второе подтверждение", |e| e["type"] == "approval").await;
    let id = approval["id"].as_str().unwrap();
    crew.ok("POST", &format!("/api/approvals/{id}"), Some(json!({ "allow": false }))).await;
    crew.wait_event("отклонённый инструмент", |e| e["type"] == "tool_update" && e["status"] == "denied").await;
    crew.wait_event("конец хода после отказа", |e| e["type"] == "turn_end").await;
    crew.ok("PATCH", &format!("/api/bots/{bot}"), Some(json!({ "approval": "ask" }))).await;

    // Делегирование.
    // Поручать можно только участнику своей команды: собираем команду из бота и Редактора.
    let editor = bots.iter().find(|b| b["name"] == "Редактор").expect("в демо нет Редактора")["id"].as_str().unwrap().to_string();
    crew.ok("POST", "/api/rooms", Some(json!({ "name": "Пара", "botIds": [bot, editor], "lead": bot, "mode": "lead" }))).await;
    crew.clear_events();
    crew.say(&bot, "делегируй это Редактору").await;
    let delegation = crew.wait_event("делегирование", |e| e["type"] == "delegate_end").await;
    crew.wait_event("конец хода делегирования", |e| e["type"] == "turn_end" && e["botId"] == bot.as_str()).await;
    let target = delegation["to"].as_str().unwrap();
    let thread = crew.ok("GET", &format!("/api/bots/{target}/thread?limit=10"), None).await;
    assert!(thread["messages"].as_array().unwrap().iter().any(|m| m["role"] == "user"), "у получателя нет задачи");
    // Закрытая задача будит выдавшего своим ходом: дождаться его конца, иначе его turn_end примут за конец следующего хода.
    let lead = bot.as_str();
    let turns = |kind: &'static str| crew.count_events(move |e| e["type"] == kind && e["botId"] == lead);
    common::wait_until("итог разбудил выдавшего", 30, || async {
        let woken = crew.ok("GET", &format!("/api/bots/{lead}/thread?limit=20"), None).await.to_string().contains("Закрылась задача");
        woken && turns("turn_start") == turns("turn_end")
    })
    .await;

    // Память.
    crew.ok("POST", "/api/memory/facts", Some(json!({ "scope": bot, "title": "тестовая заметка" }))).await;
    let memory = crew.ok("GET", &format!("/api/bots/{bot}/memory"), None).await;
    assert!(memory["facts"].as_array().unwrap().iter().any(|f| f["title"] == "тестовая заметка"), "{memory}");
    // Личная память бота — файл memory.md в его папке.
    assert!(memory["file"].as_str().unwrap().contains("- тестовая заметка"), "{memory}");
    let file = std::fs::read_to_string(crew.data_dir.join("workspaces").join(&bot).join("memory.md")).unwrap();
    assert!(file.contains("- тестовая заметка"), "{file}");

    // Файл бота виден через API, выход из воркспейса — нет.
    crew.clear_events();
    crew.say(&bot, "сохрани это в файл").await;
    crew.wait_event("запись файла", |e| e["type"] == "turn_end").await;
    let files = crew.ok("GET", &format!("/api/bots/{bot}/files?path="), None).await;
    assert!(files["items"].as_array().unwrap().iter().any(|i| i["name"] == "draft.md"), "{files}");
    let file = crew.ok("GET", &format!("/api/bots/{bot}/file?path=draft.md"), None).await;
    assert!(file["text"].as_str().unwrap().contains("Черновик"));
    let (status, _) = crew.send("GET", &format!("/api/bots/{bot}/file?path=../../state.json"), None).await;
    assert_eq!(status, 400, "выход из воркспейса должен быть запрещён");
}

#[tokio::test]
async fn language_and_font_are_remembered_and_checked() {
    let crew = Crew::start("look").await;
    let state = crew.ok("GET", "/api/state", None).await;
    assert_eq!(state["settings"]["uiLang"], "", "язык ещё не выбран");
    assert_eq!(state["settings"]["uiFont"], "system");
    crew.ok("PUT", "/api/settings", Some(json!({ "uiLang": "en", "uiFont": "golos" }))).await;
    crew.ok("PUT", "/api/settings", Some(json!({ "uiLang": "de", "uiFont": "comic" }))).await;
    let state = crew.ok("GET", "/api/state", None).await;
    assert_eq!(state["settings"]["uiLang"], "en", "чужой язык не принимается");
    assert_eq!(state["settings"]["uiFont"], "golos", "чужой шрифт не принимается");
}

#[tokio::test]
async fn changing_the_language_translates_only_untouched_starters() {
    let crew = Crew::start("starters").await;
    let state = crew.ok("GET", "/api/state", None).await;
    let names = |state: &Value| state["bots"].as_array().unwrap().iter().map(|b| b["name"].as_str().unwrap().to_string()).collect::<Vec<_>>();
    assert!(names(&state).contains(&"Аналитик".to_string()));
    let edited = state["bots"].as_array().unwrap().iter().find(|b| b["name"] == "Инженер").unwrap()["id"].as_str().unwrap().to_string();
    crew.ok("PATCH", &format!("/api/bots/{edited}"), Some(json!({ "persona": "мой характер" }))).await;

    crew.ok("PUT", "/api/settings", Some(json!({ "uiLang": "en" }))).await;
    let state = crew.ok("GET", "/api/state", None).await;
    let list = names(&state);
    assert!(list.contains(&"Analyst".to_string()), "{list:?}");
    assert!(list.contains(&"Editor".to_string()), "{list:?}");
    assert!(list.contains(&"Инженер".to_string()), "правку человека не трогаем: {list:?}");

    crew.ok("PUT", "/api/settings", Some(json!({ "uiLang": "ru" }))).await;
    let list = names(&crew.ok("GET", "/api/state", None).await);
    assert!(list.contains(&"Аналитик".to_string()), "{list:?}");
    assert!(list.contains(&"Редактор".to_string()), "{list:?}");
}

#[tokio::test]
async fn the_demo_answers_in_the_chosen_language() {
    let crew = Crew::start("demo-lang").await;
    crew.listen().await;
    crew.ok("PUT", "/api/settings", Some(json!({ "uiLang": "en" }))).await;
    let state = crew.ok("GET", "/api/state", None).await;
    let bot = state["bots"][0]["id"].as_str().unwrap().to_string();
    crew.say(&bot, "hello").await;
    let message = crew
        .wait_event("ответ демо", |e| e["type"] == "message" && e["message"]["role"] == "assistant")
        .await;
    assert!(message["message"]["text"].as_str().unwrap().starts_with("Got it"), "{message}");
}

#[tokio::test]
async fn interface_files_are_served_with_the_right_types() {
    let crew = Crew::start("assets").await;
    for (path, kind) in [
        ("/", "text/html"),
        ("/app.js", "text/javascript"),
        ("/theme.js", "text/javascript"),
        ("/avatar.js", "text/javascript"),
        ("/avatar-states.js", "text/javascript"),
        ("/updates.js", "text/javascript"),
        ("/feedback.js", "text/javascript"),
        ("/i18n.js", "text/javascript"),
        ("/fonts/onest-cyrillic.woff2", "font/woff2"),
        ("/fonts/onest-latin.woff2", "font/woff2"),
        ("/fonts/golos-text-cyrillic.woff2", "font/woff2"),
        ("/fonts/golos-text-latin.woff2", "font/woff2"),
        #[cfg(deloist_private)]
        ("/orbs.webp", "image/webp"),
        #[cfg(deloist_private)]
        ("/avatar-gears.mp4", "video/mp4"),
        ("/style.css", "text/css"),
        ("/fonts/nunito-800-cyrillic.woff2", "font/woff2"),
        ("/fonts/nunito-800-latin.woff2", "font/woff2"),
    ] {
        let res = crew.client.get(format!("{}{path}", crew.base)).header("cookie", &crew.cookie).send().await.unwrap();
        assert_eq!(res.status().as_u16(), 200, "{path}");
        assert!(res.headers()["content-type"].to_str().unwrap().starts_with(kind), "{path}");
        assert!(!res.bytes().await.unwrap().is_empty(), "{path}");
    }

    // В стартовой команде нет цветных эмодзи.
    let state = crew.ok("GET", "/api/state", None).await;
    assert!(state["bots"].as_array().unwrap().iter().all(|b| b["emoji"] == ""));
}

#[tokio::test]
async fn avatar_look_stays_in_bounds_and_the_orb_is_remembered() {
    let crew = Crew::start("look").await;
    let state = crew.ok("GET", "/api/state", None).await;
    assert!(state["settings"].get("avatarMaterial").is_none(), "вида ботов больше нет: все шары");

    let bot = state["bots"][0]["id"].as_str().unwrap().to_string();
    let updated = crew.ok("PATCH", &format!("/api/bots/{bot}"), Some(json!({ "skin": 99 }))).await;
    assert_eq!(updated["bot"]["skin"], 19);
    let updated = crew.ok("PATCH", &format!("/api/bots/{bot}"), Some(json!({ "skin": 7 }))).await;
    assert_eq!(updated["bot"]["skin"], 7);
    let state = crew.ok("GET", "/api/state", None).await;
    assert_eq!(state["bots"][0]["skin"], 7, "выбор шара помнится");
}

#[tokio::test]
async fn bot_rank_and_reserve_preferences_are_patched_and_validated() {
    let crew = Crew::start("bot-preferences").await;
    let state = crew.ok("GET", "/api/state", None).await;
    let bots = state["bots"].as_array().unwrap();
    let bot = bots[1]["id"].as_str().unwrap().to_string();
    assert_eq!(bots[1]["rank"], "lead", "старые боты остаются лидами");
    assert_eq!(bots[1]["pinned"], false);
    assert_eq!(bots[1]["team"], "");
    assert_eq!(bots[1]["reserveAfterDays"], 7);
    assert_eq!(bots[1]["lastActiveAt"], 0);
    assert_eq!(bots[1]["inReserve"], false);
    assert_eq!(bots[1]["guests"], json!([]));

    let updated = crew
        .ok(
            "PATCH",
            &format!("/api/bots/{bot}"),
            Some(json!({ "rank": "specialist", "pinned": true, "team": "Разработка", "reserveAfterDays": 0 })),
        )
        .await;
    assert_eq!(updated["bot"]["rank"], "specialist");
    assert_eq!(updated["bot"]["pinned"], true);
    assert_eq!(updated["bot"]["team"], "Разработка");
    assert_eq!(updated["bot"]["reserveAfterDays"], 0);

    let state = crew.ok("GET", "/api/state", None).await;
    let shown = state["bots"].as_array().unwrap().iter().find(|b| b["id"] == bot.as_str()).unwrap();
    assert_eq!(shown["rank"], "specialist");
    assert_eq!(shown["team"], "Разработка");
    assert_eq!(shown["reserveAfterDays"], 0);
    assert_eq!(shown["inReserve"], false, "закреплённый не в резерве");

    // Плохие значения не сохраняются и объясняются по-русски.
    let bad = [
        (json!({ "rank": "boss" }), "rank"),
        (json!({ "pinned": "yes" }), "pinned"),
        (json!({ "team": "я".repeat(41) }), "team"),
        (json!({ "reserveAfterDays": -1 }), "reserveAfterDays"),
    ];
    for (patch, field) in bad {
        let (status, body) = crew.send("PATCH", &format!("/api/bots/{bot}"), Some(patch)).await;
        assert_eq!(status, 400, "{field}: {body}");
        let error = body["error"].as_str().unwrap();
        assert!(error.contains(field), "сообщение {error:?} не про {field}");
        assert!(error.chars().any(|c| ('а'..='я').contains(&c)), "сообщение не по-русски: {error}");
    }
    assert_eq!(crew.ok("GET", "/api/state", None).await["bots"][1]["reserveAfterDays"], 0, "плохое значение не сохранилось");
}

#[tokio::test]
async fn crud_keeps_the_file_format() {
    let crew = Crew::start("crud").await;

    let (status, _) = crew.send("POST", "/api/bots", Some(json!({ "name": "  " }))).await;
    assert_eq!(status, 400);
    let (status, created) = crew.send("POST", "/api/bots", Some(json!({ "name": "Тест", "emoji": "🧪" }))).await;
    assert_eq!(status, 201);
    let bot = created["bot"]["id"].as_str().unwrap().to_string();
    assert!(bot.starts_with("bot_"));
    assert_eq!(created["bot"]["role"], "без роли");

    let updated = crew.ok("PATCH", &format!("/api/bots/{bot}"), Some(json!({ "role": "проверка", "pinned": true }))).await;
    assert_eq!(updated["bot"]["role"], "проверка");
    assert_eq!(updated["bot"]["pinned"], true);
    let (status, _) = crew.send("PATCH", "/api/bots/bot_none", Some(json!({}))).await;
    assert_eq!(status, 404);

    let (status, _) = crew.send("POST", "/api/workflows", Some(json!({ "botId": "нет", "prompt": "x" }))).await;
    assert_eq!(status, 400);
    let (status, routine) = crew
        .send("POST", "/api/workflows", Some(json!({ "botId": bot, "prompt": "проверь", "everyMinutes": 1 })))
        .await;
    assert_eq!(status, 201);
    assert_eq!(routine["process"]["everyMinutes"], 5, "минимум пять минут");
    let routine = routine["process"]["id"].as_str().unwrap().to_string();
    let toggled = crew.ok("PATCH", &format!("/api/workflows/{routine}"), Some(json!({ "enabled": false }))).await;
    assert_eq!(toggled["process"]["enabled"], false);

    crew.ok("PUT", "/api/settings", Some(json!({ "model": "m", "historyLimit": 7 }))).await;
    let settings = crew.ok("GET", "/api/state", None).await["settings"].clone();
    assert_eq!(settings["model"], "m");
    assert_eq!(settings["historyLimit"], 7);

    // Файл состояния читается тем же форматом, что и у прежней версии.
    let raw = std::fs::read_to_string(crew.data_dir.join("state.json")).unwrap();
    let state: Value = serde_json::from_str(&raw).unwrap();
    assert_eq!(state["version"], 1);
    for key in ["createdAt", "settings", "bots"] {
        assert!(state.get(key).is_some(), "в state.json нет {key}");
    }
    assert!(state["settings"].get("apiKeyEnv").is_some());
    // Рутины в state.json больше нет: простой процесс лежит своим файлом с ботом в узле шага.
    let file = std::fs::read_to_string(crew.data_dir.join("workflows").join(format!("{routine}.json"))).unwrap();
    assert!(file.contains(&bot), "бот процесса записан в его файле");
    assert!(crew.ok("GET", "/api/state", None).await["processes"][0].get("botId").is_some());

    crew.ok("DELETE", &format!("/api/workflows/{routine}"), None).await;
    crew.ok("DELETE", &format!("/api/bots/{bot}"), None).await;
    let (status, _) = crew.send("GET", &format!("/api/bots/{bot}/thread"), None).await;
    assert_eq!(status, 404);
}
