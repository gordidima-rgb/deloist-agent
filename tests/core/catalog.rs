//! Каталог: настоящий exe с подменённой папкой каталога (`CREW_CATALOG_DIR`) и заглушкой
//! модуля коннекторов (`CREW_CONNECTORS_URL`). Установка, удаление, обновление с правками
//! человека, подключение приложения и запрет записи по ключу программы.

use crate::common;

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use axum::extract::{Path as UrlPath, State};
use axum::routing::{delete, post};
use axum::{Json, Router};
use common::{temp_data_dir, Crew};
use serde_json::{json, Value};

fn skill(version: &str, body: &str) -> Value {
    json!({
        "schema": 1, "id": "skill.brief", "kind": "skill", "version": version, "category": "research", "tier": "verified",
        "title": { "ru": "Короткий бриф", "en": "Short brief" },
        "summary": { "ru": "Вывод, доказательства, пробелы", "en": "Finding, evidence, gaps" },
        "source": { "name": "Test", "license": "MIT" },
        "skill": { "body": { "ru": body, "en": body } }
    })
}

fn agent(version: &str, persona: &str) -> Value {
    json!({
        "schema": 1, "id": "agent.desk", "kind": "agent", "version": version, "category": "research", "tier": "public",
        "title": { "ru": "Стол", "en": "Desk" }, "summary": { "ru": "Исследует" },
        "source": { "name": "Test", "license": "MIT" },
        "agent": {
            "role": { "ru": "Исследования" }, "persona": { "ru": persona },
            "skills": ["skill.brief"], "model_hint": "smart",
            "avatar": { "emoji": "🧭", "color": "#4fb4ff", "skin": 2 },
            "sites": ["ozon.ru"], "approval": "ask",
            "checks": [
                { "task": { "ru": "Ответить", "en": "Answer" }, "criteria": { "ru": ["по делу"], "en": ["to the point"] }, "behavior": { "require": ["ответ|answer"], "forbid": [] } },
                { "task": { "ru": "Проверить", "en": "Check" }, "criteria": { "ru": ["точно"], "en": ["accurate"] }, "behavior": { "require": ["провер|check"], "forbid": [] } }
            ]
        }
    })
}

fn app() -> Value {
    json!({
        "schema": 1, "id": "app.kassa", "kind": "app", "version": "1.0.0", "category": "business", "tier": "verified",
        "title": { "ru": "Касса" }, "summary": { "ru": "Платежи" },
        "source": { "name": "Test", "license": "MIT" },
        "app": {
            "transport": "http", "url": "https://mcp.kassa.test/mcp", "egress": ["mcp.kassa.test"],
            "auth": { "kind": "api_key", "fields": [{ "name": "KASSA_TOKEN", "label": { "ru": "Токен" }, "secret": true }] }
        }
    })
}

fn cabinet() -> Value {
    json!({
        "schema": 1, "id": "app.portal", "kind": "app", "version": "1.0.0", "category": "personal", "tier": "verified",
        "title": { "ru": "Портал" }, "summary": { "ru": "Кабинет" },
        "source": { "name": "Test", "license": "MIT" },
        "app": { "web_cabinet": { "url": "https://www.portal.test/", "sites": ["portal.test"] } }
    })
}

fn write_catalog(dir: &Path, items: &[Value]) {
    let _ = std::fs::remove_dir_all(dir);
    std::fs::create_dir_all(dir).unwrap();
    for item in items {
        let name = item["id"].as_str().unwrap().replace('.', "-");
        std::fs::write(dir.join(format!("{name}.json")), serde_json::to_string_pretty(item).unwrap()).unwrap();
    }
}

fn catalog_dir(name: &str) -> PathBuf {
    temp_data_dir(&format!("{name}-catalog"))
}

async fn start(data: PathBuf, catalog: &Path, extra: &[(&str, &str)]) -> Crew {
    let dir = catalog.display().to_string();
    let mut env: Vec<(&str, &str)> = vec![("CREW_CATALOG_DIR", dir.as_str())];
    env.extend_from_slice(extra);
    Crew::start_at(data, &env).await
}

async fn new_bot(crew: &Crew, name: &str) -> String {
    let data = crew.ok("POST", "/api/bots", Some(json!({ "name": name }))).await;
    data["bot"]["id"].as_str().unwrap().to_string()
}

async fn state(crew: &Crew) -> Value {
    crew.ok("GET", "/api/state", None).await
}

fn item<'a>(list: &'a Value, id: &str) -> &'a Value {
    list["items"].as_array().unwrap().iter().find(|i| i["id"] == id).unwrap_or_else(|| panic!("нет {id} в {list}"))
}

#[tokio::test]
async fn the_builtin_catalog_lists_every_kind_with_filters() {
    let crew = Crew::start("catalog-builtin").await;
    let all = crew.ok("GET", "/api/catalog", None).await;
    assert_eq!(all["origin"], "builtin");
    assert_eq!(all["errors"], 0, "встроенный каталог без ошибок");
    for kind in ["skill", "agent", "app"] {
        let list = crew.ok("GET", &format!("/api/catalog?kind={kind}"), None).await;
        let items = list["items"].as_array().unwrap();
        assert!(!items.is_empty(), "нет элементов вида {kind}");
        assert!(items.iter().all(|i| i["kind"] == kind && i["state"]["installed"] == false));
    }
    let found = crew.ok("GET", "/api/catalog?q=%D1%83%D1%82%D0%B2%D0%B5%D1%80%D0%B6%D0%B4%D0%B5%D0%BD%D0%B8%D0%B5", None).await;
    assert!(found["items"].as_array().unwrap().iter().any(|i| i["id"] == "skill.check-a-claim"), "поиск по слову «утверждение» находит проверку утверждения");
    let public = crew.ok("GET", "/api/catalog?tier=public", None).await;
    assert!(public["items"].as_array().unwrap().iter().all(|i| i["tier"] == "public"));
    let detail = crew.ok("GET", "/api/catalog/agent.research-desk", None).await;
    assert_eq!(detail["item"]["skillItems"][0]["id"], "skill.check-a-claim", "панель агента показывает его навыки");
    let specialist = crew.ok("GET", "/api/catalog/agent.rust-developer", None).await;
    assert_eq!(specialist["item"]["agent"]["rank"], "specialist");
    assert_eq!(specialist["item"]["agent"]["model_hint"], "standard");
    assert_eq!(specialist["item"]["agent"]["effort"], "medium");
    assert_eq!(specialist["item"]["agent"]["triggers"]["files"][0], "*.rs");
    assert_eq!(specialist["item"]["agent"]["checks"].as_array().unwrap().len(), 2);
    let (status, _) = crew.send("GET", "/api/catalog/skill.nope", None).await;
    assert_eq!(status, 404);
}

#[tokio::test]
async fn a_specialist_agent_installs_with_its_catalog_rank() {
    let crew = Crew::start("catalog-rank").await;
    let made = crew.ok("POST", "/api/catalog/agent.rust-developer/install", Some(json!({}))).await;
    assert_eq!(made["bot"]["rank"], "specialist", "манифест agent.rank доходит до бота: {made}");
    let st = state(&crew).await;
    let bot = st["bots"].as_array().unwrap().iter().find(|b| b["id"] == made["bot"]["id"]).unwrap();
    assert_eq!(bot["rank"], "specialist");
    assert_eq!(bot["inReserve"], false, "только что установленный специалист ещё не в резерве");
    assert_eq!(bot["guests"], json!([]));
}

#[tokio::test]
async fn a_skill_installs_once_uninstalls_and_updates_without_losing_edits() {
    let catalog = catalog_dir("catalog-skill");
    write_catalog(&catalog, &[skill("1.0.0", "Шаг один.\nШаг два.")]);
    let data = temp_data_dir("catalog-skill");
    let mut crew = start(data.clone(), &catalog, &[]).await;
    let anna = new_bot(&crew, "Аня").await;
    let boris = new_bot(&crew, "Борис").await;

    // Дважды «Добавить» — один навык; второй раз только подключает ещё одному боту.
    let first = crew.ok("POST", "/api/catalog/skill.brief/install", Some(json!({ "bots": [anna] }))).await;
    let second = crew.ok("POST", "/api/catalog/skill.brief/install", Some(json!({ "bots": [anna, boris] }))).await;
    let skill_id = first["skill"]["id"].as_str().unwrap().to_string();
    assert_eq!(second["skill"]["id"], skill_id, "установка идемпотентна");
    let st = state(&crew).await;
    let skills: Vec<&Value> = st["skills"].as_array().unwrap().iter().filter(|s| s["catalogId"] == "skill.brief").collect();
    assert_eq!(skills.len(), 1);
    assert_eq!(skills[0]["origin"], "catalog");
    assert_eq!(skills[0]["catalogVersion"], "1.0.0");
    for bot in st["bots"].as_array().unwrap().iter().filter(|b| b["id"] == anna || b["id"] == boris) {
        assert_eq!(bot["skills"].as_array().unwrap().iter().filter(|s| **s == skill_id).count(), 1, "навык подключён один раз");
    }
    let list = crew.ok("GET", "/api/catalog?kind=skill", None).await;
    assert_eq!(item(&list, "skill.brief")["state"]["installed"], true);
    assert_eq!(item(&list, "skill.brief")["state"]["bots"].as_array().unwrap().len(), 2);

    // Человек правит навык, затем выходит новая версия каталога.
    crew.ok("PATCH", &format!("/api/skills/{skill_id}"), Some(json!({ "body": "Мой шаг.\nШаг два." }))).await;
    crew.keep_data();
    drop(crew);
    write_catalog(&catalog, &[skill("1.1.0", "Шаг один, точнее.\nШаг два.")]);
    let mut crew = start(data.clone(), &catalog, &[]).await;
    let list = crew.ok("GET", "/api/catalog?kind=skill", None).await;
    let state_now = &item(&list, "skill.brief")["state"];
    assert_eq!(state_now["update"], true, "видно «Обновить»");
    assert_eq!(state_now["edited"], true, "видно, что навык правили");

    let (status, conflict) = crew.send("POST", "/api/catalog/skill.brief/update", Some(json!({}))).await;
    assert_eq!(status, 409, "правку человека молча не затираем: {conflict}");
    assert_eq!(conflict["conflicts"][0]["mine"]["body"], "Мой шаг.\nШаг два.");
    assert_eq!(conflict["conflicts"][0]["theirs"]["body"], "Шаг один, точнее.\nШаг два.");
    let body_of = |st: &Value| st["skills"].as_array().unwrap().iter().find(|s| s["id"] == skill_id.as_str()).unwrap().clone();
    assert_eq!(body_of(&state(&crew).await)["body"], "Мой шаг.\nШаг два.", "после 409 ничего не поменялось");

    // «Оставить мою»: текст человека на месте, версия отмечена.
    crew.ok("POST", "/api/catalog/skill.brief/update", Some(json!({ "resolve": "mine" }))).await;
    let kept = body_of(&state(&crew).await);
    assert_eq!(kept["body"], "Мой шаг.\nШаг два.");
    assert_eq!(kept["catalogVersion"], "1.1.0");
    let list = crew.ok("GET", "/api/catalog?kind=skill", None).await;
    assert_eq!(item(&list, "skill.brief")["state"]["update"], false);

    // Следующая версия, теперь «Взять новую»: правка человека уходит в историю, её можно вернуть.
    crew.keep_data();
    drop(crew);
    write_catalog(&catalog, &[skill("1.2.0", "Новый текст.")]);
    let crew = start(data, &catalog, &[]).await;
    crew.ok("POST", "/api/catalog/skill.brief/update", Some(json!({ "resolve": "theirs" }))).await;
    let taken = body_of(&state(&crew).await);
    assert_eq!(taken["body"], "Новый текст.");
    assert_eq!(taken["previous"].as_array().unwrap().last().unwrap()["body"], "Мой шаг.\nШаг два.");
    crew.ok("POST", &format!("/api/skills/{skill_id}/undo"), None).await;
    assert_eq!(body_of(&state(&crew).await)["body"], "Мой шаг.\nШаг два.", "отмена возвращает правку человека");

    // Удалить: навыка нет нигде, ни в профилях ботов.
    crew.ok("DELETE", "/api/catalog/skill.brief", None).await;
    let st = state(&crew).await;
    assert!(!st["skills"].as_array().unwrap().iter().any(|s| s["catalogId"] == "skill.brief"));
    assert!(st["bots"].as_array().unwrap().iter().all(|b| !b["skills"].as_array().unwrap().iter().any(|s| *s == skill_id)));
    let _ = std::fs::remove_dir_all(&catalog);
}

#[tokio::test]
async fn an_unedited_skill_updates_in_place() {
    let catalog = catalog_dir("catalog-clean");
    write_catalog(&catalog, &[skill("1.0.0", "Старое.")]);
    let data = temp_data_dir("catalog-clean");
    let mut crew = start(data.clone(), &catalog, &[]).await;
    crew.ok("POST", "/api/catalog/skill.brief/install", Some(json!({}))).await;
    crew.keep_data();
    drop(crew);
    write_catalog(&catalog, &[skill("2.0.0", "Новое.")]);
    let crew = start(data, &catalog, &[]).await;
    let done = crew.ok("POST", "/api/catalog/skill.brief/update", Some(json!({}))).await;
    assert_eq!(done["skill"]["body"], "Новое.", "нетронутый навык обновляется без вопросов");
    assert_eq!(done["skill"]["catalogVersion"], "2.0.0");
    let _ = std::fs::remove_dir_all(&catalog);
}

#[tokio::test]
async fn an_agent_becomes_a_bot_and_leaves_cleanly() {
    let catalog = catalog_dir("catalog-agent");
    write_catalog(&catalog, &[skill("1.0.0", "Шаги."), agent("1.0.0", "Ты исследователь.")]);
    let data = temp_data_dir("catalog-agent");
    let mut crew = start(data.clone(), &catalog, &[]).await;

    // Сайты агента без согласия не выдаются.
    let made = crew.ok("POST", "/api/catalog/agent.desk/install", Some(json!({}))).await;
    let bot = made["bot"].clone();
    let bot_id = bot["id"].as_str().unwrap().to_string();
    assert_eq!(bot["name"], "Стол");
    assert_eq!(bot["role"], "Исследования");
    assert_eq!(bot["persona"], "Ты исследователь.");
    assert_eq!(bot["approval"], "ask");
    assert_eq!(bot["color"], "#4fb4ff");
    assert_eq!(bot["skin"], 2);
    assert_eq!(bot["driver"], "", "модель не навязываем: остаётся выбор человека");
    assert_eq!(bot["modelLevel"], "strong", "старое smart нормализуется на загрузке");
    assert_eq!(bot["effort"], "medium");
    assert!(bot["browserSites"].as_array().unwrap().is_empty(), "сайты только с согласия");
    let st = state(&crew).await;
    let brief = st["skills"].as_array().unwrap().iter().find(|s| s["catalogId"] == "skill.brief").expect("навык агента установлен").clone();
    assert_eq!(bot["skills"][0], brief["id"]);

    let again = crew.ok("POST", "/api/catalog/agent.desk/install", Some(json!({}))).await;
    assert_eq!(again["bot"]["id"], bot_id.as_str(), "второй раз — тот же бот");
    assert_eq!(state(&crew).await["bots"].as_array().unwrap().iter().filter(|b| b["catalogId"] == "agent.desk").count(), 1);

    // Человек правит характер; новая версия агента не затирает его без спроса.
    crew.ok("PATCH", &format!("/api/bots/{bot_id}"), Some(json!({ "persona": "Ты мой исследователь." }))).await;
    crew.keep_data();
    drop(crew);
    write_catalog(&catalog, &[skill("1.0.0", "Шаги."), agent("1.1.0", "Ты исследователь, версия два.")]);
    let mut crew = start(data.clone(), &catalog, &[]).await;
    let (status, conflict) = crew.send("POST", "/api/catalog/agent.desk/update", Some(json!({}))).await;
    assert_eq!(status, 409);
    assert_eq!(conflict["conflicts"][0]["what"], "persona");
    crew.ok("POST", "/api/catalog/agent.desk/update", Some(json!({ "resolve": "mine" }))).await;
    let kept = state(&crew).await["bots"].as_array().unwrap().iter().find(|b| b["id"] == bot_id.as_str()).unwrap().clone();
    assert_eq!(kept["persona"], "Ты мой исследователь.");
    assert_eq!(kept["catalogVersion"], "1.1.0");

    // Удалить: бот и навык, который принёс агент, уходят.
    crew.ok("DELETE", "/api/catalog/agent.desk", None).await;
    let st = state(&crew).await;
    assert!(!st["bots"].as_array().unwrap().iter().any(|b| b["id"] == bot_id.as_str()));
    assert!(!st["skills"].as_array().unwrap().iter().any(|s| s["catalogId"] == "skill.brief"), "навык агента никому больше не нужен");

    // С согласием на сайты — сайты у бота.
    let made = crew.ok("POST", "/api/catalog/agent.desk/install", Some(json!({ "sites": true }))).await;
    assert_eq!(made["bot"]["browserSites"], json!(["ozon.ru"]));
    crew.keep_data();
    drop(crew);
    let _ = std::fs::remove_dir_all(&catalog);
    let _ = std::fs::remove_dir_all(&data);
}

#[tokio::test]
async fn a_skill_the_person_uses_elsewhere_survives_the_agent() {
    let catalog = catalog_dir("catalog-shared");
    write_catalog(&catalog, &[skill("1.0.0", "Шаги."), agent("1.0.0", "Ты исследователь.")]);
    let crew = start(temp_data_dir("catalog-shared"), &catalog, &[]).await;
    let other = new_bot(&crew, "Вера").await;
    crew.ok("POST", "/api/catalog/agent.desk/install", Some(json!({}))).await;
    let skill_id = state(&crew).await["skills"].as_array().unwrap().iter().find(|s| s["catalogId"] == "skill.brief").unwrap()["id"].clone();
    crew.ok("PATCH", &format!("/api/bots/{other}"), Some(json!({ "skills": [skill_id] }))).await;
    crew.ok("DELETE", "/api/catalog/agent.desk", None).await;
    assert!(state(&crew).await["skills"].as_array().unwrap().iter().any(|s| s["id"] == skill_id), "навык остался: им пользуется другой бот");
    let _ = std::fs::remove_dir_all(&catalog);
}

// ---------------------------------------------------------------------------
// Заглушка модуля коннекторов
// ---------------------------------------------------------------------------

#[derive(Default)]
struct Fake {
    calls: Vec<String>,
    secrets: Vec<(String, String)>,
    installed: Option<Value>,
}

type FakeState = Arc<Mutex<Fake>>;

async fn fake_connectors() -> (String, FakeState) {
    let state: FakeState = Arc::default();
    async fn install(State(s): State<FakeState>, Json(body): Json<Value>) -> Json<Value> {
        let mut s = s.lock().unwrap();
        s.calls.push("install".into());
        s.installed = Some(body.clone());
        Json(json!({ "id": "cn_1", "card": { "host": body["manifest"]["app"]["url"] } }))
    }
    async fn step(State(s): State<FakeState>, UrlPath((id, step)): UrlPath<(String, String)>, body: Option<Json<Value>>) -> Json<Value> {
        let mut s = s.lock().unwrap();
        s.calls.push(format!("{step}:{id}"));
        if step == "secret" {
            let body = body.unwrap().0;
            s.secrets.push((body["name"].as_str().unwrap().into(), body["value"].as_str().unwrap().into()));
        }
        match step.as_str() {
            "check" => Json(json!({ "tools": [{ "name": "payments_list" }, { "name": "payment_get" }] })),
            _ => Json(json!({ "status": "ok" })),
        }
    }
    async fn oauth(State(s): State<FakeState>, UrlPath(id): UrlPath<String>) -> Json<Value> {
        s.lock().unwrap().calls.push(format!("oauth:{id}"));
        Json(json!({ "url": "https://oauth.kassa.test/authorize?client=deloist" }))
    }
    async fn remove(State(s): State<FakeState>, UrlPath(id): UrlPath<String>) -> Json<Value> {
        s.lock().unwrap().calls.push(format!("delete:{id}"));
        Json(json!({ "ok": true }))
    }
    let router = Router::new()
        .route("/api/connectors/install", post(install))
        .route("/api/connectors/{id}/oauth/start", post(oauth))
        .route("/api/connectors/{id}/{step}", post(step))
        .route("/api/connectors/{id}", delete(remove))
        .with_state(state.clone());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
    (base, state)
}

#[tokio::test]
async fn an_app_connects_through_the_connector_api() {
    let (base, fake) = fake_connectors().await;
    let catalog = catalog_dir("catalog-app");
    write_catalog(&catalog, &[app()]);
    let crew = start(temp_data_dir("catalog-app"), &catalog, &[("CREW_CONNECTORS_URL", base.as_str())]).await;

    // Шаг до «Подключить» отклоняется.
    let (status, _) = crew.send("POST", "/api/catalog/app.kassa/connect", Some(json!({ "step": "check" }))).await;
    assert_eq!(status, 409);

    let started = crew.ok("POST", "/api/catalog/app.kassa/install", Some(json!({}))).await;
    assert_eq!(started["connector"], "cn_1");
    assert_eq!(started["card"]["url"], "https://mcp.kassa.test/mcp", "карточка согласия из манифеста");
    assert_eq!(started["card"]["egress"], json!(["mcp.kassa.test"]));
    assert_eq!(started["card"]["auth"]["fields"][0]["name"], "KASSA_TOKEN");
    assert_eq!(started["remote"]["host"], "https://mcp.kassa.test/mcp", "карточка модуля коннекторов");
    assert_eq!(fake.lock().unwrap().installed.as_ref().unwrap()["catalogId"], "app.kassa");
    let list = crew.ok("GET", "/api/catalog?kind=app", None).await;
    assert_eq!(item(&list, "app.kassa")["state"]["status"], "pending");
    assert_eq!(item(&list, "app.kassa")["state"]["installed"], false);

    crew.ok("POST", "/api/catalog/app.kassa/connect", Some(json!({ "step": "confirm" }))).await;
    let (status, _) = crew.send("POST", "/api/catalog/app.kassa/connect", Some(json!({ "step": "secret", "name": "OTHER", "value": "x" }))).await;
    assert_eq!(status, 400, "поле не из манифеста не принимаем");
    crew.ok("POST", "/api/catalog/app.kassa/connect", Some(json!({ "step": "secret", "name": "KASSA_TOKEN", "value": "test-token-1" }))).await;
    let oauth = crew.ok("POST", "/api/catalog/app.kassa/connect", Some(json!({ "step": "oauth" }))).await;
    assert!(oauth["url"].as_str().unwrap().starts_with("https://oauth.kassa.test/"));
    let checked = crew.ok("POST", "/api/catalog/app.kassa/connect", Some(json!({ "step": "check" }))).await;
    assert_eq!(checked["tools"].as_array().unwrap().len(), 2);

    let list = crew.ok("GET", "/api/catalog?kind=app", None).await;
    assert_eq!(item(&list, "app.kassa")["state"]["installed"], true);
    // Секрет ушёл в модуль коннекторов и нигде у каталога не лёг.
    assert_eq!(fake.lock().unwrap().secrets, vec![("KASSA_TOKEN".to_string(), "test-token-1".to_string())]);
    for file in ["state.json", "catalog-installs.json"] {
        let text = std::fs::read_to_string(crew.data_dir.join(file)).unwrap_or_default();
        assert!(!text.contains("test-token-1"), "секрет попал в {file}");
    }
    let again = crew.ok("POST", "/api/catalog/app.kassa/install", Some(json!({}))).await;
    assert_eq!(again["already"], true, "повторное «Подключить» не создаёт второго подключения");

    crew.ok("DELETE", "/api/catalog/app.kassa", None).await;
    let calls = fake.lock().unwrap().calls.clone();
    assert_eq!(calls, ["install", "confirm:cn_1", "secret:cn_1", "oauth:cn_1", "check:cn_1", "delete:cn_1"]);
    let list = crew.ok("GET", "/api/catalog?kind=app", None).await;
    assert_eq!(item(&list, "app.kassa")["state"]["installed"], false);
    let _ = std::fs::remove_dir_all(&catalog);
}

#[tokio::test]
async fn without_the_connector_module_connect_says_so_and_shows_the_card() {
    let catalog = catalog_dir("catalog-noconn");
    write_catalog(&catalog, &[app()]);
    let crew = start(temp_data_dir("catalog-noconn"), &catalog, &[]).await;
    let (status, body) = crew.send("POST", "/api/catalog/app.kassa/install", Some(json!({}))).await;
    assert_eq!(status, 503);
    assert_eq!(body["card"]["auth"]["fields"][0]["name"], "KASSA_TOKEN", "карточка согласия есть и без модуля");
    let _ = std::fs::remove_dir_all(&catalog);
}

#[tokio::test]
async fn a_web_cabinet_adds_sites_only_with_consent_and_takes_them_back() {
    let catalog = catalog_dir("catalog-cabinet");
    write_catalog(&catalog, &[cabinet()]);
    let crew = start(temp_data_dir("catalog-cabinet"), &catalog, &[]).await;
    let bot = new_bot(&crew, "Гоша").await;
    crew.ok("PATCH", &format!("/api/bots/{bot}"), Some(json!({ "browserSites": ["mail.test"] }))).await;
    let (status, _) = crew.send("POST", "/api/catalog/app.portal/install", Some(json!({ "bot": bot }))).await;
    assert_eq!(status, 400, "без согласия сайт не выдаётся");
    let done = crew.ok("POST", "/api/catalog/app.portal/install", Some(json!({ "bot": bot, "consent": true }))).await;
    assert_eq!(done["bot"]["browserSites"], json!(["mail.test", "portal.test"]));
    assert_eq!(done["url"], "https://www.portal.test/");
    let list = crew.ok("GET", "/api/catalog?kind=app", None).await;
    assert_eq!(item(&list, "app.portal")["mode"], "cabinet");
    assert_eq!(item(&list, "app.portal")["state"]["installed"], true);
    crew.ok("DELETE", "/api/catalog/app.portal", None).await;
    let st = state(&crew).await;
    let sites = st["bots"].as_array().unwrap().iter().find(|b| b["id"] == bot.as_str()).unwrap()["browserSites"].clone();
    assert_eq!(sites, json!(["mail.test"]), "уходит только то, что добавил кабинет");
    let _ = std::fs::remove_dir_all(&catalog);
}

#[tokio::test]
async fn a_broken_manifest_is_left_out() {
    let catalog = catalog_dir("catalog-broken");
    let mut no_license = skill("1.0.0", "Шаги.");
    no_license["source"] = json!({ "name": "Test" });
    no_license["id"] = json!("skill.nolicense");
    write_catalog(&catalog, &[skill("1.0.0", "Шаги."), no_license]);
    let crew = start(temp_data_dir("catalog-broken"), &catalog, &[]).await;
    let list = crew.ok("GET", "/api/catalog", None).await;
    assert_eq!(list["errors"], 1);
    assert!(list["items"].as_array().unwrap().iter().all(|i| i["id"] != "skill.nolicense"), "без лицензии в каталог не попадает");
    let (status, _) = crew.send("POST", "/api/catalog/skill.nolicense/install", Some(json!({}))).await;
    assert_eq!(status, 404);
    let _ = std::fs::remove_dir_all(&catalog);
}

#[tokio::test]
async fn the_agent_key_reads_the_catalog_but_cannot_install() {
    let crew = Crew::start("catalog-agent-key").await;
    let runtime: Value = serde_json::from_str(&std::fs::read_to_string(crew.data_dir.join("runtime.json")).unwrap()).unwrap();
    let token = runtime["token"].as_str().unwrap().to_string();
    let call = |method: reqwest::Method, path: &str| {
        crew.client.request(method, format!("{}{path}", crew.base)).bearer_auth(&token).json(&json!({ "bots": [] })).send()
    };
    let refused = [
        call(reqwest::Method::POST, "/api/catalog/skill.check-a-claim/install").await.unwrap(),
        call(reqwest::Method::POST, "/api/catalog/agent.research-desk/install").await.unwrap(),
        call(reqwest::Method::POST, "/api/catalog/app.yookassa/connect").await.unwrap(),
        call(reqwest::Method::POST, "/api/catalog/skill.check-a-claim/update").await.unwrap(),
        call(reqwest::Method::DELETE, "/api/catalog/skill.check-a-claim").await.unwrap(),
    ];
    for response in refused {
        assert_eq!(response.status().as_u16(), 403, "ключ программы поставил из каталога: {}", response.url());
    }
    let read = call(reqwest::Method::GET, "/api/catalog").await.unwrap();
    assert_eq!(read.status().as_u16(), 200, "смотреть каталог ключу можно");
    assert!(!crew.ok("GET", "/api/state", None).await["skills"].as_array().unwrap().iter().any(|s| s["catalogId"].as_str().is_some_and(|c| !c.is_empty())));
}
