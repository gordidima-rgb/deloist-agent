//! `propose_team` и `search_catalog`: бот предлагает команду карточкой, создаёт её только человек из окна.

use crate::common;

use common::Crew;
use serde_json::{json, Value};
use std::sync::{Arc, Mutex};

use axum::body::{Body, Bytes};
use axum::extract::State;
use axum::response::Response;
use axum::routing::post;
use axum::Router;

#[derive(Clone, Default)]
struct Model {
    seen: Arc<Mutex<Vec<Value>>>,
}

fn sse(chunks: &[Value]) -> String {
    let mut out: String = chunks.iter().map(|c| format!("data: {c}\n\n")).collect();
    out.push_str("data: [DONE]\n\n");
    out
}

fn say(text: &str) -> String {
    sse(&[
        json!({ "choices": [{ "index": 0, "delta": { "role": "assistant", "content": text } }] }),
        json!({ "choices": [{ "index": 0, "delta": {}, "finish_reason": "stop" }], "usage": { "prompt_tokens": 10, "completion_tokens": 5 } }),
    ])
}

fn tool_step(name: &str, args: Value) -> String {
    sse(&[
        json!({ "choices": [{ "index": 0, "delta": { "tool_calls": [{ "index": 0, "id": format!("call_{name}"), "type": "function", "function": { "name": name, "arguments": args.to_string() } }] } }] }),
        json!({ "choices": [{ "index": 0, "delta": {}, "finish_reason": "tool_calls" }], "usage": { "prompt_tokens": 100, "completion_tokens": 10 } }),
    ])
}

/// «PROPOSE {json}» — вызвать propose_team с этими аргументами; «SEARCH слова» — search_catalog.
async fn completions(State(model): State<Model>, body: Bytes) -> Response {
    let payload: Value = serde_json::from_slice(&body).unwrap_or_else(|_| json!({}));
    model.seen.lock().unwrap().push(payload.clone());
    let messages = payload["messages"].as_array().cloned().unwrap_or_default();
    let last_role = messages.last().and_then(|m| m["role"].as_str()).unwrap_or_default();
    let last_user = messages.iter().rev().find(|m| m["role"] == "user").and_then(|m| m["content"].as_str()).unwrap_or_default();
    let text = if last_role == "tool" {
        say("готово")
    } else if let Some(at) = last_user.find("PROPOSE ") {
        tool_step("propose_team", serde_json::from_str(&last_user[at + 8..]).unwrap_or_else(|_| json!({})))
    } else if let Some(at) = last_user.find("BOTCHANGE ") {
        tool_step("propose_bot_change", serde_json::from_str(&last_user[at + 10..]).unwrap_or_else(|_| json!({})))
    } else if let Some(at) = last_user.find("ROUTINE ") {
        tool_step("propose_routine", serde_json::from_str(&last_user[at + 8..]).unwrap_or_else(|_| json!({})))
    } else if let Some(at) = last_user.find("SEARCH ") {
        tool_step("search_catalog", json!({ "query": last_user[at + 7..].trim() }))
    } else {
        say("готово")
    };
    Response::builder().header("content-type", "text/event-stream").body(Body::from(text)).unwrap()
}

async fn setup(name: &str) -> (Crew, String, usize) {
    setup_env(name, &[]).await
}

async fn setup_env(name: &str, env: &[(&str, &str)]) -> (Crew, String, usize) {
    let router = Router::new().route("/v1/chat/completions", post(completions)).with_state(Model::default());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
    let crew = Crew::start_with_env(name, env).await;
    crew.ok(
        "PUT",
        "/api/settings",
        Some(json!({ "driver": "api", "model": "m", "baseUrl": format!("http://127.0.0.1:{port}/v1"), "apiKey": "test-key" })),
    )
    .await;
    let host = crew
        .ok("POST", "/api/bots", Some(json!({ "name": "Редактор", "role": "главный", "driver": "api", "model": "host-model" })))
        .await["bot"]["id"]
        .as_str()
        .unwrap()
        .to_string();
    let base = crew.ok("GET", "/api/state", None).await["bots"].as_array().map_or(0, Vec::len);
    (crew, host, base)
}

async fn state_bots(crew: &Crew) -> Vec<Value> {
    crew.ok("GET", "/api/state", None).await["bots"].as_array().cloned().unwrap_or_default()
}

fn bot_named<'a>(bots: &'a [Value], name: &str) -> Option<&'a Value> {
    bots.iter().find(|b| b["name"] == name)
}

fn agent_token(crew: &Crew) -> String {
    let runtime: Value = serde_json::from_str(&std::fs::read_to_string(crew.data_dir.join("runtime.json")).unwrap()).unwrap();
    runtime["token"].as_str().unwrap().to_string()
}

/// Ход с вызовом инструмента кончился ошибкой (до карточки): текст ошибки и отсутствие карточки.
async fn expect_refused(crew: &Crew, host: &str, args: Value, must_contain: &str) {
    crew.clear_events();
    crew.say(host, &format!("PROPOSE {args}")).await;
    let tool = crew.wait_event("результат инструмента", |e| e["type"] == "tool_update" && e["botId"] == host).await;
    assert_eq!(tool["status"], "error", "{tool}");
    assert!(tool["result"].as_str().unwrap_or_default().contains(must_contain), "{tool}");
    crew.wait_event("конец", |e| e["type"] == "turn_end" && e["botId"] == host).await;
    assert!(crew.find_event(|e| e["type"] == "team_proposal").is_none(), "карточка показана при ошибке");
}

fn first_agent(items: &Value, installed: bool) -> Value {
    items["items"]
        .as_array()
        .unwrap()
        .iter()
        .find(|i| i["kind"] == "agent" && i["state"]["installed"].as_bool().unwrap_or(false) == installed)
        .cloned()
        .expect("в каталоге есть агент")
}

#[tokio::test(flavor = "multi_thread")]
async fn accept_creates_exactly_the_stored_team_with_safe_defaults() {
    let (crew, host, _base) = setup("team-accept").await;
    crew.listen().await;
    // Права и команды в аргументах: ни одно поле не должно подействовать.
    let args = json!({
        "newBots": [
            { "name": "Автор", "role": "пишет", "instructions": "Пиши коротко.\n**жирный** <b>тег</b>", "approval": "auto", "browserSites": ["evil.example"], "driver": "cli", "model": "x", "acpCommand": "evil.exe", "cliCommand": "evil.exe", "folders": ["C:\\"], "monthlyTokenLimit": 5, "rank": "specialist" },
            { "name": "Корректор", "role": "проверяет", "instructions": "Ищи ошибки." },
        ],
        "room": { "new": "Лента" },
        "lead": "Автор",
        "approval": "auto", "browserSites": ["evil.example"], "folders": ["C:\\"], "programFolders": ["C:\\"], "skills": ["x"], "acpCommand": "evil.exe",
    });
    crew.say(&host, &format!("PROPOSE {args}")).await;
    let event = crew.wait_event("карточка команды", |e| e["type"] == "team_proposal").await;
    assert_eq!(event["botId"], host.as_str());
    let card = &event["proposal"];
    assert_eq!(card["newBots"][0]["name"], "Автор");
    assert_eq!(card["newBots"][1]["role"], "проверяет");
    assert_eq!(card["room"]["name"], "Лента");
    assert_eq!(card["room"]["isNew"], true);
    assert_eq!(card["lead"], "Автор");
    assert!(!card.to_string().contains("evil"), "в карточку просочилось лишнее: {card}");
    // До решения человека ничего не создано, а ход ждёт.
    tokio::time::sleep(std::time::Duration::from_millis(300)).await;
    assert!(bot_named(&state_bots(&crew).await, "Автор").is_none(), "создано без решения");
    assert!(crew.find_event(|e| e["type"] == "turn_end" && e["botId"] == host.as_str()).is_none(), "ход не ждёт человека");

    // Ключ программы принять карточку не может: решает только окно.
    let id = card["id"].as_str().unwrap().to_string();
    let token = agent_token(&crew);
    let by_key = crew.client.post(format!("{}/api/bots/{host}/offers/{id}/accept", crew.base)).bearer_auth(&token).send().await.unwrap();
    assert_eq!(by_key.status().as_u16(), 403, "ключ программы принял свою карточку");
    assert!(bot_named(&state_bots(&crew).await, "Автор").is_none(), "ключом создан бот");

    let (status, reply) = crew.send("POST", &format!("/api/bots/{host}/offers/{id}/accept"), None).await;
    assert_eq!(status, 200, "{reply}");
    let tool = crew.wait_event("итог инструмента", |e| e["type"] == "tool_update" && e["botId"] == host.as_str()).await;
    assert_eq!(tool["status"], "ok", "{tool}");
    let result = tool["result"].as_str().unwrap_or_default();
    assert!(result.contains("Автор") && result.contains("Корректор") && result.contains("Лента"), "{result}");
    crew.wait_event("конец", |e| e["type"] == "turn_end" && e["botId"] == host.as_str()).await;

    let state = crew.ok("GET", "/api/state", None).await;
    let bots = state["bots"].as_array().unwrap();
    for name in ["Автор", "Корректор"] {
        let bot = bot_named(bots, name).unwrap_or_else(|| panic!("нет {name}"));
        assert_eq!(bot["approval"], "ask", "{bot}");
        assert_eq!(bot["browserSites"], json!([]), "{bot}");
        assert_eq!(bot["monthlyTokenLimit"], 0, "{bot}");
        assert_eq!(bot["driver"], "api", "{bot}");
        assert_eq!(bot["model"], "host-model", "{bot}");
        assert_eq!(bot["rank"], "lead", "{bot}");
    }
    assert_eq!(bot_named(bots, "Автор").unwrap()["persona"], "Пиши коротко.\n**жирный** <b>тег</b>");
    let author = bot_named(bots, "Автор").unwrap()["id"].clone();
    let room = state["rooms"].as_array().unwrap().iter().find(|r| r["name"] == "Лента").expect("команда создана");
    assert_eq!(room["lead"], author);
    assert_eq!(room["botIds"].as_array().unwrap().len(), 2);
    assert_eq!(room["folders"], json!([]));
    assert_eq!(room["skills"], json!([]));
    // Карточка одноразовая.
    let (again, _) = crew.send("POST", &format!("/api/bots/{host}/offers/{id}/accept"), None).await;
    assert_eq!(again, 404);
}

#[tokio::test(flavor = "multi_thread")]
async fn decline_creates_nothing_and_the_agent_hears_the_refusal() {
    let (crew, host, _base) = setup("team-decline").await;
    crew.listen().await;
    crew.say(&host, &format!("PROPOSE {}", json!({ "newBots": [{ "name": "Лишний", "role": "r", "instructions": "i" }], "room": { "new": "Пустая" } }))).await;
    let event = crew.wait_event("карточка", |e| e["type"] == "team_proposal").await;
    let id = event["proposal"]["id"].as_str().unwrap().to_string();
    let (status, _) = crew.send("POST", &format!("/api/bots/{host}/offers/{id}/decline"), None).await;
    assert_eq!(status, 204);
    let tool = crew.wait_event("итог", |e| e["type"] == "tool_update" && e["botId"] == host.as_str()).await;
    assert!(tool["result"].as_str().unwrap_or_default().contains("отказался"), "{tool}");
    crew.wait_event("конец", |e| e["type"] == "turn_end" && e["botId"] == host.as_str()).await;
    assert!(bot_named(&state_bots(&crew).await, "Лишний").is_none());
    let state = crew.ok("GET", "/api/state", None).await;
    assert!(state["rooms"].as_array().unwrap().is_empty(), "{state}");
    // Решённую карточку принять уже нельзя.
    let (late, _) = crew.send("POST", &format!("/api/bots/{host}/offers/{id}/accept"), None).await;
    assert_eq!(late, 404);
}

#[tokio::test(flavor = "multi_thread")]
async fn bad_proposals_are_refused_before_any_card() {
    let (crew, host, base) = setup("team-bad").await;
    crew.listen().await;
    let bot = |name: &str| json!({ "name": name, "role": "r", "instructions": "i" });
    // Имя совпадает с существующим ботом (без учёта регистра).
    expect_refused(&crew, &host, json!({ "newBots": [bot("редактор")] }), "уже есть").await;
    // Повтор имени внутри предложения.
    expect_refused(&crew, &host, json!({ "newBots": [bot("Икс"), bot("икс")] }), "повторяется").await;
    // Больше пяти.
    let six: Vec<Value> = (0..6).map(|i| bot(&format!("Бот {i}"))).collect();
    expect_refused(&crew, &host, json!({ "newBots": six }), "до 5").await;
    // Слишком длинные инструкции.
    expect_refused(&crew, &host, json!({ "newBots": [{ "name": "Длинный", "role": "r", "instructions": "я".repeat(4_001) }] }), "длиннее").await;
    // Невидимые знаки в имени (переворот строки U+202E, нулевая ширина) отбиваются до карточки.
    expect_refused(&crew, &host, json!({ "newBots": [bot("Ав\u{202e}тор")] }), "невидимыми").await;
    expect_refused(&crew, &host, json!({ "newBots": [bot("Ав\u{200b}тор")] }), "невидимыми").await;
    expect_refused(&crew, &host, json!({ "newBots": [{ "name": "Бот", "role": "r", "instructions": "ок \u{202e}evil" }] }), "направления").await;
    expect_refused(&crew, &host, json!({ "newBots": [bot("Новый")], "room": { "new": "К\u{2066}" } }), "невидимых").await;
    // Неизвестные участники, команда и каталог.
    expect_refused(&crew, &host, json!({ "addBots": ["Призрак"] }), "нет бота").await;
    expect_refused(&crew, &host, json!({ "newBots": [bot("Новый")], "room": "Нет такой" }), "нет команды").await;
    expect_refused(&crew, &host, json!({ "catalogBots": ["agent.no-such-agent"] }), "в каталоге нет").await;
    expect_refused(&crew, &host, json!({ "newBots": [bot("Новый")], "room": { "new": "К" }, "lead": "Чужой" }), "нет среди участников").await;
    expect_refused(&crew, &host, json!({}), "нечего предлагать").await;
    assert_eq!(state_bots(&crew).await.len(), base, "что-то создалось");
}

#[tokio::test(flavor = "multi_thread")]
async fn the_card_names_shared_access_and_accept_adds_to_the_existing_team() {
    let (crew, host, _base) = setup("team-room").await;
    crew.listen().await;
    let helper = crew.ok("POST", "/api/bots", Some(json!({ "name": "Помощник", "role": "р" }))).await["bot"]["id"].as_str().unwrap().to_string();
    let shared = common::temp_data_dir("team-room-shared");
    std::fs::create_dir_all(&shared).unwrap();
    let room = crew
        .ok("POST", "/api/rooms", Some(json!({ "name": "Штаб", "botIds": [host], "lead": host })))
        .await["room"]["id"]
        .as_str()
        .unwrap()
        .to_string();
    let folder = shared.to_string_lossy().to_string();
    let (status, patched) = crew.send("PATCH", &format!("/api/rooms/{room}"), Some(json!({ "folders": [folder] }))).await;
    assert_eq!(status, 200, "{patched}");
    let folders = patched["room"]["folders"].clone();
    assert!(!folders.as_array().unwrap().is_empty(), "папка не принята: {patched}");

    crew.say(&host, &format!("PROPOSE {}", json!({ "newBots": [{ "name": "Новичок", "role": "р", "instructions": "i" }], "addBots": ["Помощник"], "room": "штаб", "lead": "Новичок" }))).await;
    let event = crew.wait_event("карточка", |e| e["type"] == "team_proposal").await;
    let card = &event["proposal"];
    assert_eq!(card["room"]["isNew"], false);
    let access = card["access"].to_string();
    assert!(access.contains("Новичок") && access.contains("Помощник"), "в строке доступа нет тех, кто получит доступ: {access}");
    assert!(access.contains("сменит ведущего"), "нет смены ведущего: {access}");
    assert_eq!(card["defaults"]["approval"], "ask", "{card}");
    assert_eq!(card["defaults"]["hostName"], "Редактор", "{card}");
    assert_eq!(card["defaults"]["model"], "host-model", "{card}");
    assert!(access.contains("доступ") && access.contains(folders[0].as_str().unwrap().replace('\\', "\\\\").as_str()), "нет строки про доступ: {access}");
    let id = card["id"].as_str().unwrap().to_string();
    let (status, reply) = crew.send("POST", &format!("/api/bots/{host}/offers/{id}/accept"), None).await;
    assert_eq!(status, 200, "{reply}");
    crew.wait_event("конец", |e| e["type"] == "turn_end" && e["botId"] == host.as_str()).await;

    let state = crew.ok("GET", "/api/state", None).await;
    let room_now = state["rooms"].as_array().unwrap().iter().find(|r| r["id"] == room.as_str()).unwrap();
    let members = room_now["botIds"].as_array().unwrap();
    assert_eq!(members.len(), 3, "{room_now}");
    assert!(members.iter().any(|m| m == helper.as_str()));
    let newcomer = bot_named(state["bots"].as_array().unwrap(), "Новичок").unwrap()["id"].clone();
    assert_eq!(room_now["lead"], newcomer);
    assert_eq!(room_now["folders"], folders, "папки команды не должны меняться");
    assert_eq!(state["rooms"].as_array().unwrap().len(), 1, "лишняя команда");
}

#[tokio::test(flavor = "multi_thread")]
async fn catalog_bots_install_through_the_card_only_with_safe_defaults() {
    let (crew, host, base) = setup("team-catalog").await;
    crew.listen().await;
    let items = crew.ok("GET", "/api/catalog", None).await;
    let agent = first_agent(&items, false);
    let id = agent["id"].as_str().unwrap().to_string();
    let title = agent["title"]["ru"].as_str().unwrap_or_else(|| agent["title"].as_str().unwrap()).to_string();

    // Ключ программы ставить из каталога сам не может.
    let token = agent_token(&crew);
    let direct = crew.client.post(format!("{}/api/catalog/{id}/install", crew.base)).bearer_auth(&token).json(&json!({})).send().await.unwrap();
    assert_eq!(direct.status().as_u16(), 403, "ключ программы установил из каталога");
    assert_eq!(state_bots(&crew).await.len(), base);

    crew.say(&host, &format!("PROPOSE {}", json!({ "catalogBots": [id], "room": { "new": "Каталожная" }, "approval": "auto" }))).await;
    let event = crew.wait_event("карточка", |e| e["type"] == "team_proposal").await;
    let card = &event["proposal"];
    assert_eq!(card["catalogBots"][0]["id"], id.as_str());
    assert_eq!(card["catalogBots"][0]["name"], title.as_str());
    assert!(card["catalogBots"][0]["needs"].is_array(), "{card}");
    assert_eq!(state_bots(&crew).await.len(), base, "поставлено до решения");
    let offer = card["id"].as_str().unwrap().to_string();
    let (status, reply) = crew.send("POST", &format!("/api/bots/{host}/offers/{offer}/accept"), None).await;
    assert_eq!(status, 200, "{reply}");
    crew.wait_event("конец", |e| e["type"] == "turn_end" && e["botId"] == host.as_str()).await;

    let state = crew.ok("GET", "/api/state", None).await;
    let bots = state["bots"].as_array().unwrap();
    assert_eq!(bots.len(), base + 1, "поставлено ровно одно: {bots:?}");
    let installed = bot_named(bots, &title).unwrap_or_else(|| panic!("нет «{title}»: {bots:?}"));
    assert_eq!(installed["catalogId"], id.as_str(), "{installed}");
    assert_eq!(installed["approval"], "ask");
    assert_eq!(installed["browserSites"], json!([]));
    let room = state["rooms"].as_array().unwrap().iter().find(|r| r["name"] == "Каталожная").expect("команда создана");
    assert_eq!(room["botIds"], json!([installed["id"]]));
}

#[tokio::test(flavor = "multi_thread")]
async fn search_catalog_lists_up_to_ten_bots_read_only() {
    let (crew, host, base) = setup("team-search").await;
    crew.listen().await;
    let items = crew.ok("GET", "/api/catalog", None).await;
    let agent = first_agent(&items, false);
    let title = agent["title"]["ru"].as_str().unwrap_or_else(|| agent["title"].as_str().unwrap()).to_string();
    let word = title.split_whitespace().max_by_key(|w| w.chars().count()).unwrap().to_string();
    crew.say(&host, &format!("SEARCH {word}")).await;
    let tool = crew.wait_event("результат поиска", |e| e["type"] == "tool_update" && e["botId"] == host.as_str()).await;
    assert_eq!(tool["status"], "ok", "{tool}");
    let text = tool["result"].as_str().unwrap_or_default();
    assert!(text.contains(agent["id"].as_str().unwrap()), "{text}");
    assert!(text.contains("не установлен"), "{text}");
    let lines = text.lines().take_while(|l| !l.is_empty()).count();
    assert!((1..=10).contains(&lines), "{lines} строк");
    // Поиск ничего не создаёт и карточек не показывает.
    assert!(crew.find_event(|e| e["type"] == "team_proposal" || e["type"] == "approval").is_none());
    assert_eq!(state_bots(&crew).await.len(), base);
}

/// Каталог из одного агента, чей манифест просит свободный режим и сайты.
fn greedy_catalog(name: &str) -> String {
    let dir = common::temp_data_dir(&format!("{name}-catalog"));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let item = json!({
        "schema": 1, "id": "agent.greedy", "kind": "agent", "version": "1.0.0", "category": "research", "tier": "public",
        "title": { "ru": "Жадный" }, "summary": { "ru": "Просит лишнего" },
        "source": { "name": "Test", "license": "MIT" },
        "agent": {
            "role": { "ru": "Исследования" }, "persona": { "ru": "Работай." }, "model_hint": "standard",
            "avatar": { "emoji": "🧭", "color": "#4fb4ff" }, "sites": ["ozon.ru"], "approval": "auto",
            "checks": [
                { "task": { "ru": "Ответить", "en": "Answer" }, "criteria": { "ru": ["по делу"], "en": ["to the point"] }, "behavior": { "require": ["ответ|answer"], "forbid": [] } },
                { "task": { "ru": "Проверить", "en": "Check" }, "criteria": { "ru": ["точно"], "en": ["accurate"] }, "behavior": { "require": ["провер|check"], "forbid": [] } }
            ]
        }
    });
    std::fs::write(dir.join("agent-greedy.json"), serde_json::to_string_pretty(&item).unwrap()).unwrap();
    dir.display().to_string()
}

#[tokio::test(flavor = "multi_thread")]
async fn a_catalog_manifest_asking_for_free_mode_and_sites_ends_as_ask_without_sites() {
    let dir = greedy_catalog("team-greedy");
    let (crew, host, base) = setup_env("team-greedy", &[("CREW_CATALOG_DIR", dir.as_str())]).await;
    crew.listen().await;
    crew.say(&host, &format!("PROPOSE {}", json!({ "catalogBots": ["agent.greedy"] }))).await;
    let event = crew.wait_event("карточка", |e| e["type"] == "team_proposal").await;
    let id = event["proposal"]["id"].as_str().unwrap().to_string();
    let (status, reply) = crew.send("POST", &format!("/api/bots/{host}/offers/{id}/accept"), None).await;
    assert_eq!(status, 200, "{reply}");
    crew.wait_event("конец", |e| e["type"] == "turn_end" && e["botId"] == host.as_str()).await;
    let bots = state_bots(&crew).await;
    assert_eq!(bots.len(), base + 1);
    let bot = bot_named(&bots, "Жадный").expect("бот поставлен");
    assert_eq!(bot["approval"], "ask", "{bot}");
    assert_eq!(bot["browserSites"], json!([]), "{bot}");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_catalog_name_taken_while_the_card_hung_fails_the_accept() {
    let dir = greedy_catalog("team-clash");
    let (crew, host, base) = setup_env("team-clash", &[("CREW_CATALOG_DIR", dir.as_str())]).await;
    crew.listen().await;
    crew.say(&host, &format!("PROPOSE {}", json!({ "catalogBots": ["agent.greedy"] }))).await;
    let event = crew.wait_event("карточка", |e| e["type"] == "team_proposal").await;
    let id = event["proposal"]["id"].as_str().unwrap().to_string();
    crew.ok("POST", "/api/bots", Some(json!({ "name": "Жадный", "role": "р" }))).await;
    crew.send("POST", &format!("/api/bots/{host}/offers/{id}/accept"), None).await;
    let tool = crew.wait_event("итог", |e| e["type"] == "tool_update" && e["botId"] == host.as_str()).await;
    assert!(tool["result"].as_str().unwrap_or_default().contains("уже появился"), "{tool}");
    crew.wait_event("конец", |e| e["type"] == "turn_end" && e["botId"] == host.as_str()).await;
    assert_eq!(state_bots(&crew).await.len(), base + 1, "поставился второй бот с тем же именем");
}

// ---------------------------------------------------------------------------
// propose_bot_change и propose_routine
// ---------------------------------------------------------------------------

/// Участник «Пост» вместе с хозяином в команде «Лента», плюс чужой бот вне команды. У участника права, которые менять нельзя.
async fn team_with_mate(name: &str, mate_approval: &str) -> (Crew, String, String, String) {
    let (crew, host, _base) = setup(name).await;
    crew.listen().await;
    // В свежем хранилище уже есть свой «Редактор»: хозяину даём имя, которое не повторяется.
    crew.ok("PATCH", &format!("/api/bots/{host}"), Some(json!({ "name": "Ведущий" }))).await;
    let mate = crew
        .ok("POST", "/api/bots", Some(json!({ "name": "Пост", "role": "пишет посты", "persona": "Пиши коротко.", "approval": mate_approval, "driver": "api", "model": "mate-model", "browserSites": ["x.com"], "monthlyTokenLimit": 777 })))
        .await["bot"]["id"]
        .as_str()
        .unwrap()
        .to_string();
    crew.ok("POST", "/api/bots", Some(json!({ "name": "Чужой", "role": "не в команде" }))).await;
    let room = crew.ok("POST", "/api/rooms", Some(json!({ "name": "Лента", "botIds": [host.clone(), mate.clone()] }))).await["room"]["id"].as_str().unwrap().to_string();
    (crew, host, mate, room)
}

async fn expect_refused_tool(crew: &Crew, host: &str, prefix: &str, args: Value, must_contain: &str) {
    crew.clear_events();
    crew.say(host, &format!("{prefix} {args}")).await;
    let tool = crew.wait_event("результат инструмента", |e| e["type"] == "tool_update" && e["botId"] == host).await;
    assert_eq!(tool["status"], "error", "{tool}");
    assert!(tool["result"].as_str().unwrap_or_default().contains(must_contain), "{tool}");
    crew.wait_event("конец", |e| e["type"] == "turn_end" && e["botId"] == host).await;
    assert!(crew.find_event(|e| e["type"] == "team_proposal").is_none(), "карточка показана при ошибке");
}

async fn bot_json(crew: &Crew, id: &str) -> Value {
    state_bots(crew).await.into_iter().find(|b| b["id"] == id).expect("бот на месте")
}

async fn routines_of(crew: &Crew) -> Vec<Value> {
    crew.ok("GET", "/api/state", None).await["processes"].as_array().cloned().unwrap_or_default()
}

#[tokio::test(flavor = "multi_thread")]
async fn bot_change_only_for_teammates_and_privileged_fields_are_ignored() {
    let (crew, host, mate, _room) = team_with_mate("change-basic", "strict").await;
    expect_refused_tool(&crew, &host, "BOTCHANGE", json!({ "bot": "Чужой", "role": "новая" }), "не в твоей команде").await;
    expect_refused_tool(&crew, &host, "BOTCHANGE", json!({ "bot": "Ведущий", "role": "новая" }), "себя").await;
    expect_refused_tool(&crew, &host, "BOTCHANGE", json!({ "bot": "Призрак", "role": "новая" }), "нет бота").await;
    expect_refused_tool(&crew, &host, "BOTCHANGE", json!({ "bot": "Пост" }), "нечего менять").await;
    expect_refused_tool(&crew, &host, "BOTCHANGE", json!({ "bot": "Пост", "role": "я".repeat(81) }), "role").await;
    expect_refused_tool(&crew, &host, "BOTCHANGE", json!({ "bot": "Пост", "instructions": "ок \u{202e}evil" }), "направления").await;
    expect_refused_tool(&crew, &host, "BOTCHANGE", json!({ "bot": "Пост", "removeFromRoom": "Нет такой" }), "removeFromRoom").await;
    let before = bot_json(&crew, &mate).await;

    crew.clear_events();
    let args = json!({
        "bot": "Пост", "role": "редактор постов", "instructions": "Пиши длинно.\n**жирный** <b>тег</b>",
        "approval": "auto", "browserSites": ["evil.example"], "driver": "cli", "model": "evil-model", "acpCommand": "evil.exe", "cliCommand": "evil.exe",
        "folders": ["C:\\"], "monthlyTokenLimit": 1, "rank": "specialist", "name": "Взломан",
    });
    crew.say(&host, &format!("BOTCHANGE {args}")).await;
    let event = crew.wait_event("карточка изменения", |e| e["type"] == "team_proposal").await;
    let card = &event["proposal"];
    assert_eq!(card["kind"], "bot_change");
    assert_eq!(card["target"]["name"], "Пост");
    assert_eq!(card["role"], json!({ "before": "пишет посты", "after": "редактор постов" }));
    assert_eq!(card["instructions"]["before"], "Пиши коротко.");
    assert_eq!(card["instructions"]["after"], "Пиши длинно.\n**жирный** <b>тег</b>");
    assert!(card["removeFrom"].is_null());
    assert!(!card.to_string().contains("evil"), "в карточку просочилось лишнее: {card}");
    tokio::time::sleep(std::time::Duration::from_millis(300)).await;
    assert_eq!(bot_json(&crew, &mate).await["role"], "пишет посты", "изменено без решения");
    assert!(crew.find_event(|e| e["type"] == "turn_end" && e["botId"] == host.as_str()).is_none(), "ход не ждёт человека");

    let id = card["id"].as_str().unwrap().to_string();
    let token = agent_token(&crew);
    let by_key = crew.client.post(format!("{}/api/bots/{host}/offers/{id}/accept", crew.base)).bearer_auth(&token).send().await.unwrap();
    assert_eq!(by_key.status().as_u16(), 403, "ключ программы принял свою карточку");
    assert_eq!(bot_json(&crew, &mate).await["role"], "пишет посты");

    let (status, reply) = crew.send("POST", &format!("/api/bots/{host}/offers/{id}/accept"), None).await;
    assert_eq!(status, 200, "{reply}");
    let tool = crew.wait_event("итог", |e| e["type"] == "tool_update" && e["botId"] == host.as_str()).await;
    assert_eq!(tool["status"], "ok", "{tool}");
    crew.wait_event("конец", |e| e["type"] == "turn_end" && e["botId"] == host.as_str()).await;
    assert!(crew.find_event(|e| e["type"] == "state_changed").is_some(), "нет state_changed");

    let after = bot_json(&crew, &mate).await;
    assert_eq!(after["role"], "редактор постов");
    assert_eq!(after["persona"], "Пиши длинно.\n**жирный** <b>тег</b>");
    // Всё остальное, что агент пытался подменить, осталось как было.
    for key in ["name", "approval", "browserSites", "driver", "model", "monthlyTokenLimit", "rank", "acpCommand", "cliCommand"] {
        assert_eq!(after[key], before[key], "поле {key} изменилось");
    }
    assert_eq!(after["approval"], "strict");
    let (again, _) = crew.send("POST", &format!("/api/bots/{host}/offers/{id}/accept"), None).await;
    assert_eq!(again, 404, "карточка одноразовая");
}

#[tokio::test(flavor = "multi_thread")]
async fn pending_cards_are_in_the_state_the_window_loads_until_decided() {
    let (crew, host, mate, _room) = team_with_mate("pending-cards", "ask").await;
    let none = crew.ok("GET", &format!("/api/bots/{host}/pending"), None).await;
    assert_eq!(none["offers"], json!([]));
    crew.say(&host, &format!("BOTCHANGE {}", json!({ "bot": "Пост", "role": "новая роль" }))).await;
    let event = crew.wait_event("карточка", |e| e["type"] == "team_proposal").await;
    let id = event["proposal"]["id"].as_str().unwrap().to_string();
    // Окно перезагрузили: карточка приходит из сервера в том же виде, что и событие.
    let loaded = crew.ok("GET", &format!("/api/bots/{host}/pending"), None).await;
    let offers = loaded["offers"].as_array().unwrap();
    assert_eq!(offers.len(), 1, "{loaded}");
    assert_eq!(offers[0]["type"], "team_proposal");
    assert_eq!(offers[0]["botId"], host.as_str());
    assert_eq!(offers[0]["proposal"], event["proposal"], "карточка из состояния отличается от живой");
    // Чужому боту она не показывается.
    let other = crew.ok("GET", &format!("/api/bots/{mate}/pending"), None).await;
    assert_eq!(other["offers"], json!([]));
    // После решения карточка не возвращается.
    let (status, _) = crew.send("POST", &format!("/api/bots/{host}/offers/{id}/decline"), None).await;
    assert_eq!(status, 204);
    let after = crew.ok("GET", &format!("/api/bots/{host}/pending"), None).await;
    assert_eq!(after["offers"], json!([]), "решённая карточка вернулась");
    crew.wait_event("конец", |e| e["type"] == "turn_end" && e["botId"] == host.as_str()).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn instructions_refuse_zero_width_text() {
    let (crew, host, _mate, _room) = team_with_mate("change-zero-width", "ask").await;
    expect_refused_tool(&crew, &host, "BOTCHANGE", json!({ "bot": "Пост", "instructions": "Пиши\u{200B} коротко" }), "невидимые").await;
    expect_refused_tool(&crew, &host, "BOTCHANGE", json!({ "bot": "Пост", "instructions": "Пиши\u{2060}коротко" }), "невидимые").await;
    expect_refused(&crew, &host, json!({ "newBots": [{ "name": "Новый", "role": "р", "instructions": "ок\u{FEFF}ок" }] }), "невидимыми").await;
    // Соединитель эмодзи остаётся допустимым: карточка показывается.
    crew.clear_events();
    crew.say(&host, &format!("BOTCHANGE {}", json!({ "bot": "Пост", "instructions": "Пиши с \u{1F468}\u{200D}\u{1F4BB} коротко" }))).await;
    crew.wait_event("карточка", |e| e["type"] == "team_proposal").await;
}

#[tokio::test(flavor = "multi_thread")]
async fn bot_change_is_not_applied_after_the_host_left_the_team() {
    let (crew, host, mate, room) = team_with_mate("change-host-left", "ask").await;
    crew.say(&host, &format!("BOTCHANGE {}", json!({ "bot": "Пост", "role": "новая роль" }))).await;
    let event = crew.wait_event("карточка", |e| e["type"] == "team_proposal").await;
    let id = event["proposal"]["id"].as_str().unwrap().to_string();
    // Человек убрал хозяина карточки из команды, пока она висела.
    crew.ok("PATCH", &format!("/api/rooms/{room}"), Some(json!({ "botIds": [mate.clone()] }))).await;
    let (status, reply) = crew.send("POST", &format!("/api/bots/{host}/offers/{id}/accept"), None).await;
    assert_eq!(status, 409, "{reply}");
    assert_eq!(bot_json(&crew, &mate).await["role"], "пишет посты", "правка применена бывшим участником");
}

#[tokio::test(flavor = "multi_thread")]
async fn accepted_routine_for_a_non_free_bot_keeps_ask_floor() {
    let (crew, host, _mate, _room) = team_with_mate("routine-floor", "ask").await;
    let args = json!({ "bot": "Пост", "title": "Пост дня", "prompt": "Напиши пост.", "schedule": { "at": "09:00" } });
    crew.say(&host, &format!("ROUTINE {args}")).await;
    let event = crew.wait_event("карточка рутины", |e| e["type"] == "team_proposal").await;
    assert_eq!(event["proposal"]["freeMode"], false);
    assert_eq!(event["proposal"]["quiet"], true, "карточка не показывает тихий режим: {event}");
    let id = event["proposal"]["id"].as_str().unwrap().to_string();
    let (status, reply) = crew.send("POST", &format!("/api/bots/{host}/offers/{id}/accept"), None).await;
    assert_eq!(status, 200, "{reply}");
    let made = routines_of(&crew).await;
    let routine = made.iter().find(|r| r["title"] == "Пост дня").expect("рутина создана");
    assert_eq!(routine["approvalFloor"], "ask", "пол осторожности не выставлен: {routine}");
    crew.wait_event("конец", |e| e["type"] == "turn_end" && e["botId"] == host.as_str()).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn bot_change_decline_and_stale_accept_change_nothing() {
    let (crew, host, mate, _room) = team_with_mate("change-decline", "ask").await;
    crew.say(&host, &format!("BOTCHANGE {}", json!({ "bot": "Пост", "role": "новая роль" }))).await;
    let event = crew.wait_event("карточка", |e| e["type"] == "team_proposal").await;
    let id = event["proposal"]["id"].as_str().unwrap().to_string();
    let (status, _) = crew.send("POST", &format!("/api/bots/{host}/offers/{id}/decline"), None).await;
    assert_eq!(status, 204);
    let tool = crew.wait_event("итог", |e| e["type"] == "tool_update" && e["botId"] == host.as_str()).await;
    assert!(tool["result"].as_str().unwrap_or_default().contains("отказался"), "{tool}");
    crew.wait_event("конец", |e| e["type"] == "turn_end" && e["botId"] == host.as_str()).await;
    assert_eq!(bot_json(&crew, &mate).await["role"], "пишет посты");

    // Пока карточка висела, человек сам поправил роль: карточка «было → стало» устарела и не применяется.
    crew.clear_events();
    crew.say(&host, &format!("BOTCHANGE {}", json!({ "bot": "Пост", "role": "роль агента" }))).await;
    let event = crew.wait_event("вторая карточка", |e| e["type"] == "team_proposal").await;
    let id = event["proposal"]["id"].as_str().unwrap().to_string();
    crew.ok("PATCH", &format!("/api/bots/{mate}"), Some(json!({ "role": "роль человека" }))).await;
    let (status, reply) = crew.send("POST", &format!("/api/bots/{host}/offers/{id}/accept"), None).await;
    assert_eq!(status, 409, "{reply}");
    assert_eq!(bot_json(&crew, &mate).await["role"], "роль человека", "карточка затёрла правку человека");
    crew.wait_event("итог", |e| e["type"] == "tool_update" && e["botId"] == host.as_str()).await;
    crew.wait_event("конец", |e| e["type"] == "turn_end" && e["botId"] == host.as_str()).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn bot_change_removes_a_teammate_from_the_named_room_only() {
    let (crew, host, mate, room) = team_with_mate("change-remove", "ask").await;
    // Второй комнаты не создаём: бот состоит только в одной команде.
    crew.say(&host, &format!("BOTCHANGE {}", json!({ "bot": "Пост", "removeFromRoom": "Лента" }))).await;
    let event = crew.wait_event("карточка", |e| e["type"] == "team_proposal").await;
    assert_eq!(event["proposal"]["removeFrom"]["name"], "Лента", "{event}");
    assert!(event["proposal"]["role"].is_null() && event["proposal"]["instructions"].is_null());
    let id = event["proposal"]["id"].as_str().unwrap().to_string();
    let (status, reply) = crew.send("POST", &format!("/api/bots/{host}/offers/{id}/accept"), None).await;
    assert_eq!(status, 200, "{reply}");
    crew.wait_event("конец", |e| e["type"] == "turn_end" && e["botId"] == host.as_str()).await;
    let state = crew.ok("GET", "/api/state", None).await;
    let rooms = state["rooms"].as_array().unwrap();
    let ids = |room_id: &str| rooms.iter().find(|r| r["id"] == room_id).unwrap()["botIds"].clone();
    assert_eq!(ids(&room), json!([host.clone()]));
    assert_eq!(bot_json(&crew, &mate).await["role"], "пишет посты", "роль не должна меняться");
}

#[tokio::test(flavor = "multi_thread")]
async fn routine_card_shows_who_when_and_the_free_mode_warning_and_creates_only_on_accept() {
    let (crew, host, mate, _room) = team_with_mate("routine-free", "auto").await;
    let before = routines_of(&crew).await.len();
    let args = json!({
        "bot": "Пост", "title": "Пост дня", "prompt": "Напиши пост про погоду.\n**жирный**",
        "schedule": { "at": "09:00", "days": [1, 2, 3, 4, 5] },
        "approval": "ask", "enabled": false, "approvalFloor": "strict", "botId": "evil",
    });
    crew.say(&host, &format!("ROUTINE {args}")).await;
    let event = crew.wait_event("карточка рутины", |e| e["type"] == "team_proposal").await;
    let card = &event["proposal"];
    assert_eq!(card["kind"], "routine");
    assert_eq!(card["target"]["name"], "Пост");
    assert_eq!(card["title"], "Пост дня");
    assert_eq!(card["prompt"], "Напиши пост про погоду.\n**жирный**");
    assert_eq!(card["schedule"], "по будням в 09:00");
    assert!(card["next"].as_str().unwrap_or_default().starts_with("Следующий запуск: "), "{card}");
    assert_eq!(card["freeMode"], true, "у бота «Свобода действий»: карточка должна предупредить");
    // До «Создать» рутины нет.
    tokio::time::sleep(std::time::Duration::from_millis(300)).await;
    assert_eq!(routines_of(&crew).await.len(), before, "рутина создана без решения");

    let id = card["id"].as_str().unwrap().to_string();
    let token = agent_token(&crew);
    let by_key = crew.client.post(format!("{}/api/bots/{host}/offers/{id}/accept", crew.base)).bearer_auth(&token).send().await.unwrap();
    assert_eq!(by_key.status().as_u16(), 403);
    let (status, reply) = crew.send("POST", &format!("/api/bots/{host}/offers/{id}/accept"), None).await;
    assert_eq!(status, 200, "{reply}");
    let tool = crew.wait_event("итог", |e| e["type"] == "tool_update" && e["botId"] == host.as_str()).await;
    assert_eq!(tool["status"], "ok", "{tool}");
    crew.wait_event("конец", |e| e["type"] == "turn_end" && e["botId"] == host.as_str()).await;
    let all = routines_of(&crew).await;
    let made: Vec<&Value> = all.iter().filter(|r| r["title"] == "Пост дня").collect();
    assert_eq!(made.len(), 1, "{all:?}");
    let r = made[0];
    assert_eq!(r["botId"], mate.as_str());
    assert_eq!(r["enabled"], true);
    assert_eq!(r["at"], "09:00");
    assert_eq!(r["days"], json!([1, 2, 3, 4, 5]));
    assert_eq!(r["prompt"], "Напиши пост про погоду.\n**жирный**");
    assert_eq!(r["approvalFloor"], "", "карточку подтвердил человек: пола осторожности нет");
}

#[tokio::test(flavor = "multi_thread")]
async fn routine_card_without_free_mode_has_no_warning_and_bad_requests_get_no_card() {
    let (crew, host, _mate, _room) = team_with_mate("routine-bad", "ask").await;
    let good = json!({ "bot": "Пост", "title": "Раз в час", "prompt": "Проверь ленту.", "schedule": { "every_minutes": 60, "window_from": "09:00", "window_to": "18:00" } });
    expect_refused_tool(&crew, &host, "ROUTINE", json!({ "bot": "Чужой", "title": "т", "prompt": "п", "schedule": { "at": "09:00" } }), "не в твоей команде").await;
    expect_refused_tool(&crew, &host, "ROUTINE", json!({ "bot": "Пост", "title": "т", "prompt": "п", "schedule": { "at": "9:00" } }), "ЧЧ:ММ").await;
    expect_refused_tool(&crew, &host, "ROUTINE", json!({ "bot": "Пост", "title": "т", "prompt": "п", "schedule": { "every_minutes": 1 } }), "чаще").await;
    expect_refused_tool(&crew, &host, "ROUTINE", json!({ "bot": "Пост", "title": "т", "prompt": "п", "schedule": { "at": "09:00", "every_minutes": 60 } }), "либо").await;
    expect_refused_tool(&crew, &host, "ROUTINE", json!({ "bot": "Пост", "title": "т", "prompt": "п", "schedule": {} }), "не сказано когда").await;
    expect_refused_tool(&crew, &host, "ROUTINE", json!({ "bot": "Пост", "title": "т", "prompt": "п", "schedule": { "every_minutes": 60, "window_from": "09:00" } }), "двумя полями").await;
    expect_refused_tool(&crew, &host, "ROUTINE", json!({ "bot": "Пост", "title": "т", "prompt": "", "schedule": { "at": "09:00" } }), "prompt").await;
    expect_refused_tool(&crew, &host, "ROUTINE", json!({ "bot": "Пост", "title": "", "prompt": "п", "schedule": { "at": "09:00" } }), "title").await;
    expect_refused_tool(&crew, &host, "ROUTINE", json!({ "bot": "Пост", "title": "т", "prompt": "п" }), "schedule").await;

    crew.clear_events();
    crew.say(&host, &format!("ROUTINE {good}")).await;
    let event = crew.wait_event("карточка рутины", |e| e["type"] == "team_proposal").await;
    assert_eq!(event["proposal"]["freeMode"], false, "{event}");
    assert!(event["proposal"]["schedule"].as_str().unwrap().contains("каждый час"), "{event}");
    let id = event["proposal"]["id"].as_str().unwrap().to_string();
    let (status, _) = crew.send("POST", &format!("/api/bots/{host}/offers/{id}/decline"), None).await;
    assert_eq!(status, 204);
    let tool = crew.wait_event("итог", |e| e["type"] == "tool_update" && e["botId"] == host.as_str()).await;
    assert!(tool["result"].as_str().unwrap_or_default().contains("отказался"), "{tool}");
    crew.wait_event("конец", |e| e["type"] == "turn_end" && e["botId"] == host.as_str()).await;
    assert!(routines_of(&crew).await.iter().all(|r| r["title"] != "Раз в час"), "отказ создал рутину");
}

#[tokio::test(flavor = "multi_thread")]
async fn an_orchestrator_card_fills_its_own_team_and_ends_the_interview() {
    let (crew, _host, _base) = setup("team-orchestrator").await;
    crew.listen().await;
    let made = crew.ok("POST", "/api/rooms", Some(json!({ "name": "Главная", "withOrchestrator": true, "botIds": [] }))).await;
    let room = made["room"]["id"].as_str().unwrap().to_string();
    let orch = made["bot"]["id"].as_str().unwrap().to_string();
    assert_eq!(made["room"]["stage"], "interview");
    assert_eq!(made["room"]["lead"], orch.as_str());
    assert_eq!(made["bot"]["orchestrator"], true);
    assert_eq!(made["bot"]["skin"], 13);

    // Агент просит «новую команду», но состав идёт в команду оркестратора.
    crew.say(&orch, &format!("PROPOSE {}", json!({ "newBots": [{ "name": "Автор", "role": "пишет", "instructions": "Пиши." }], "room": { "new": "Чужая" }, "lead": "Автор" }))).await;
    let event = crew.wait_event("карточка", |e| e["type"] == "team_proposal" && e["botId"] == orch.as_str()).await;
    assert_eq!(event["proposal"]["room"]["isNew"], false, "{event}");
    assert_eq!(event["proposal"]["room"]["id"], room.as_str());
    crew.clear_events();
    let id = event["proposal"]["id"].as_str().unwrap().to_string();
    let (status, reply) = crew.send("POST", &format!("/api/bots/{orch}/offers/{id}/accept"), None).await;
    assert_eq!(status, 200, "{reply}");
    crew.wait_event("команда обновилась", |e| e["type"] == "state_changed").await;
    crew.wait_event("конец", |e| e["type"] == "turn_end" && e["botId"] == orch.as_str()).await;

    let state = crew.ok("GET", "/api/state", None).await;
    let rooms = state["rooms"].as_array().unwrap();
    assert_eq!(rooms.len(), 1, "лишняя команда: {rooms:?}");
    assert_eq!(rooms[0]["stage"], "ready");
    let author = bot_named(state["bots"].as_array().unwrap(), "Автор").unwrap()["id"].clone();
    assert!(rooms[0]["botIds"].as_array().unwrap().contains(&author), "{rooms:?}");
    assert!(rooms[0]["botIds"].as_array().unwrap().contains(&json!(orch)), "оркестратор остался в команде");
    assert_eq!(rooms[0]["lead"], orch.as_str(), "ведущий не менялся");
    // Итог оркестратора после «Создать» виден в чате команды, а не только в его личном.
    let thread = crew.ok("GET", &format!("/api/rooms/{room}/thread"), None).await;
    let summary = thread["messages"].as_array().unwrap().iter().find(|m| m["role"] == "assistant" && m["botId"] == orch.as_str()).cloned();
    assert_eq!(summary.as_ref().map(|m| m["text"].clone()), Some(json!("готово")), "{thread}");

    // «Перестроить»: расспрос заново, следующая карточка снова идёт в ту же команду.
    let (status, again) = crew.send("POST", &format!("/api/rooms/{room}/rebuild"), None).await;
    assert_eq!(status, 200, "{again}");
    assert_eq!(again["room"]["stage"], "interview");
    assert_eq!(again["bot"]["id"], orch.as_str());
}

#[tokio::test(flavor = "multi_thread")]
async fn a_plain_bots_card_still_builds_a_new_team() {
    let (crew, host, _base) = setup("team-plain-still").await;
    crew.listen().await;
    crew.say(&host, &format!("PROPOSE {}", json!({ "newBots": [{ "name": "Один", "role": "р", "instructions": "i" }], "room": { "new": "Новая" } }))).await;
    let event = crew.wait_event("карточка", |e| e["type"] == "team_proposal").await;
    assert_eq!(event["proposal"]["room"]["isNew"], true);
    let id = event["proposal"]["id"].as_str().unwrap().to_string();
    let (status, _) = crew.send("POST", &format!("/api/bots/{host}/offers/{id}/accept"), None).await;
    assert_eq!(status, 200);
    let state = crew.ok("GET", "/api/state", None).await;
    assert_eq!(state["rooms"][0]["stage"], "ready");
}
