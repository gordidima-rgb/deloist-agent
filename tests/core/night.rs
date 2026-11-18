//! Ночная смена: расписание, журнал, предел токенов и поведение без человека.

use crate::common;

use std::sync::{Arc, Mutex};

use axum::body::Bytes;
use axum::extract::State;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::post;
use axum::{Json, Router};
use common::Crew;
use serde_json::{json, Value};

/// Модель по сценарию: что вернуть на каждый следующий запрос и сколько токенов сказать.
#[derive(Clone, Default)]
struct Fake {
    script: Arc<Mutex<Vec<Value>>>,
    requests: Arc<Mutex<Vec<Value>>>,
    usage: Arc<Mutex<Option<(i64, i64)>>>,
    fail: Arc<Mutex<Option<(u16, String)>>>,
}

fn say(text: &str) -> Value {
    json!({ "role": "assistant", "content": text })
}

fn tool(name: &str, args: Value) -> Value {
    json!({ "role": "assistant", "content": "", "tool_calls": [{ "id": format!("c{}", rand_id()), "type": "function", "function": { "name": name, "arguments": args.to_string() } }] })
}

fn rand_id() -> u128 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()
}

async fn completions(State(fake): State<Fake>, body: Bytes) -> Response {
    let request: Value = serde_json::from_slice(&body).unwrap();
    fake.requests.lock().unwrap().push(request);
    if let Some((status, text)) = fake.fail.lock().unwrap().clone() {
        return (StatusCode::from_u16(status).unwrap(), text).into_response();
    }
    let message = {
        let mut script = fake.script.lock().unwrap();
        if script.is_empty() { say("готово") } else { script.remove(0) }
    };
    let mut reply = json!({ "choices": [{ "message": message }] });
    if let Some((prompt, completion)) = *fake.usage.lock().unwrap() {
        reply["usage"] = json!({ "prompt_tokens": prompt, "completion_tokens": completion });
    }
    Json(reply).into_response()
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

async fn add_routine(crew: &Crew, bot: &str, extra: Value) -> String {
    let mut body = json!({ "botId": bot, "title": "Проверка", "prompt": "проверь сайт", "everyMinutes": 60 });
    for (key, value) in extra.as_object().unwrap() {
        body[key] = value.clone();
    }
    let (status, data) = crew.send("POST", "/api/routines", Some(body)).await;
    assert_eq!(status, 201, "{data}");
    data["routine"]["id"].as_str().unwrap().to_string()
}

async fn journal(crew: &Crew) -> Value {
    crew.ok("GET", "/api/journal", None).await
}

/// Запустить рутину как по расписанию, без человека, и дождаться её записи в журнале.
async fn run_and_wait(crew: &Crew, id: &str, mut body: Value) -> Value {
    if body.get("attended").is_none() {
        body["attended"] = json!(false);
    }
    crew.clear_events();
    crew.ok("POST", &format!("/api/routines/{id}/run"), Some(body)).await;
    crew.wait_event("журнал", |e| e["type"] == "journal").await["entry"].clone()
}

async fn thread(crew: &Crew, bot: &str, limit: Option<i64>) -> Vec<Value> {
    let path = match limit {
        Some(limit) => format!("/api/bots/{bot}/thread?limit={limit}"),
        None => format!("/api/bots/{bot}/thread"),
    };
    crew.ok("GET", &path, None).await["messages"].as_array().cloned().unwrap_or_default()
}

fn has_text(messages: &[Value], text: &str) -> bool {
    messages.iter().any(|m| m["text"] == text)
}

#[tokio::test(flavor = "multi_thread")]
async fn schedule_fields_are_validated_and_saved() {
    let crew = Crew::start("night-fields").await;
    let state = crew.ok("GET", "/api/state", None).await;
    let bot = state["bots"][0]["id"].as_str().unwrap().to_string();

    let (status, _) = crew.send("POST", "/api/routines", Some(json!({ "botId": bot, "prompt": "x", "at": "25:00" }))).await;
    assert_eq!(status, 400, "неверное время");
    let (status, _) = crew.send("POST", "/api/routines", Some(json!({ "botId": bot, "prompt": "x", "windowFrom": "вечер" }))).await;
    assert_eq!(status, 400, "неверное окно");

    let id = add_routine(&crew, &bot, json!({ "at": "09:00", "days": [1, 2, 3, 4, 5, 9, 5] })).await;
    let state = crew.ok("GET", "/api/state", None).await;
    let routine = &state["routines"][0];
    assert_eq!(routine["at"], "09:00");
    assert_eq!(routine["days"], json!([1, 2, 3, 4, 5]), "мусорные дни выброшены");
    let in_a_while = routine["nextRun"].as_i64().unwrap() - routine["createdAt"].as_i64().unwrap();
    assert!((0..=4 * 86_400_000).contains(&in_a_while), "следующий запуск в ближайшие дни: {in_a_while}");

    let patched = crew.ok("PATCH", &format!("/api/routines/{id}"), Some(json!({ "at": "", "windowFrom": "22:00", "windowTo": "07:00" }))).await;
    assert_eq!(patched["routine"]["at"], "");
    assert_eq!(patched["routine"]["windowFrom"], "22:00");
    let (status, _) = crew.send("PATCH", &format!("/api/routines/{id}"), Some(json!({ "windowTo": "7" }))).await;
    assert_eq!(status, 400);
}

#[tokio::test(flavor = "multi_thread")]
async fn scheduled_run_lands_in_the_journal_with_tokens() {
    let (crew, fake, bot) = setup("night-run", &[("CREW_TEST_TICK_MS", "100")]).await;
    *fake.usage.lock().unwrap() = Some((1_000, 200));
    let id = add_routine(&crew, &bot, json!({})).await;
    crew.ok("PATCH", &format!("/api/routines/{id}"), Some(json!({ "nextRun": common::due_now() }))).await;

    let event = crew.wait_event("журнал", |e| e["type"] == "journal").await;
    let entry = &event["entry"];
    assert_eq!(entry["status"], "ok", "{entry}");
    assert_eq!(entry["tokens"], 1200);
    assert_eq!(entry["title"], "Проверка");
    assert_eq!(entry["text"], "готово");
    assert_eq!(entry["unattended"], true, "расписание работает без человека");

    let log = journal(&crew).await;
    assert_eq!(log["entries"].as_array().unwrap().len(), 1);
    assert_eq!(log["usage"]["tokens"], 1200);
    assert_eq!(log["usage"]["estimated"], false);

    let prompt = fake.requests.lock().unwrap()[0]["messages"][0]["content"].as_str().unwrap().to_string();
    assert!(prompt.contains("человека рядом может не быть"), "{prompt}");

    // Срок передвинулся вперёд, и второй раз сам не запустится.
    let state = crew.ok("GET", "/api/state", None).await;
    assert!(state["routines"][0]["nextRun"].as_i64().unwrap() > state["routines"][0]["lastRun"].as_i64().unwrap());
    assert_eq!(state["routines"][0]["lastStatus"], "ok");

    // Журнал переживает перезапуск: он в файле.
    let text = std::fs::read_to_string(crew.data_dir.join("journal.jsonl")).unwrap();
    assert_eq!(text.lines().count(), 1);
}

#[tokio::test(flavor = "multi_thread")]
async fn no_usage_from_the_provider_means_an_estimate() {
    let (crew, _fake, bot) = setup("night-estimate", &[]).await;
    let id = add_routine(&crew, &bot, json!({})).await;
    crew.ok("POST", &format!("/api/routines/{id}/run"), None).await;
    let event = crew.wait_event("журнал", |e| e["type"] == "journal").await;
    assert!(event["entry"]["tokens"].as_i64().unwrap() > 0, "{event}");
    assert_eq!(event["entry"]["unattended"], false, "кнопку нажал человек");
    let log = journal(&crew).await;
    assert_eq!(log["usage"]["estimated"], true);
}

#[tokio::test(flavor = "multi_thread")]
async fn nobody_answers_at_night_so_the_action_is_refused_and_left_for_morning() {
    let (crew, fake, bot) = setup("night-absent", &[("CREW_TEST_UNATTENDED_WAIT_MS", "300")]).await;
    *fake.script.lock().unwrap() = vec![
        tool("run_command", json!({ "command": "echo один > C:\\Windows\\deloist-probe.txt" })),
        tool("run_command", json!({ "command": "echo два > C:\\Windows\\deloist-probe.txt" })),
        tool("ask_user", json!({ "question": "какой отчёт нужен?" })),
        say("Проверил что мог. Ждёт решения: команда и выбор отчёта."),
    ];
    let id = add_routine(&crew, &bot, json!({})).await;
    crew.ok("POST", &format!("/api/routines/{id}/run"), Some(json!({ "attended": false }))).await;

    let event = crew.wait_event("журнал", |e| e["type"] == "journal").await;
    let entry = &event["entry"];
    assert_eq!(entry["status"], "needs", "{entry}");
    let missed = entry["missed"].as_array().unwrap();
    assert!(missed.iter().any(|m| m.as_str().unwrap().contains("какой отчёт")), "{entry}");
    assert_eq!(missed.len(), 2, "одна и та же команда не дублируется: {entry}");

    let cards = crew.events.lock().unwrap().iter().filter(|e| e["type"] == "approval").count();
    assert_eq!(cards, 1, "после первого молчания карточки больше не показываются");
    let ran = crew.events.lock().unwrap().iter().filter(|e| e["type"] == "tool_update" && e["status"] == "ok" && e["result"].as_str().unwrap_or("").contains("код выхода")).count();
    assert_eq!(ran, 0, "команда без разрешения не выполняется");
}

#[tokio::test(flavor = "multi_thread")]
async fn the_daily_limit_pauses_routines_until_midnight_but_not_a_person() {
    let (crew, fake, bot) = setup("night-limit", &[("CREW_TEST_TICK_MS", "100")]).await;
    *fake.usage.lock().unwrap() = Some((600, 100));
    crew.ok("PUT", "/api/settings", Some(json!({ "dailyTokenLimit": 1000 }))).await;
    *fake.script.lock().unwrap() = vec![
        tool("list_files", json!({})),
        tool("list_files", json!({})),
        tool("list_files", json!({})),
        say("не должно дойти"),
    ];
    let id = add_routine(&crew, &bot, json!({})).await;
    crew.ok("POST", &format!("/api/routines/{id}/run"), Some(json!({ "attended": false }))).await;

    let event = crew.wait_event("журнал", |e| e["type"] == "journal").await;
    assert_eq!(event["entry"]["status"], "paused", "{event}");
    assert_eq!(event["entry"]["tokens"], 1400, "два шага и стоп");
    assert_eq!(fake.requests.lock().unwrap().len(), 2);
    // Ход не провалился: запись журнала запусков ждёт ближайшей местной полуночи.
    let runs: Vec<Value> = serde_json::from_str(&std::fs::read_to_string(crew.data_dir.join("runs.json")).unwrap()).unwrap();
    let paused = &runs[0]["state"];
    assert_eq!((paused["kind"].as_str(), paused["reason"].as_str()), (Some("paused"), Some("budget")), "{paused}");
    let tomorrow = chrono::Local::now().date_naive().succ_opt().unwrap().and_time(chrono::NaiveTime::MIN);
    let midnight = chrono::TimeZone::from_local_datetime(&chrono::Local, &tomorrow).earliest().unwrap().timestamp_millis();
    assert_eq!(paused["until_ms"].as_i64(), Some(midnight), "{paused}");

    // Расписание больше не запускает: пропуск виден в журнале.
    crew.clear_events();
    fake.requests.lock().unwrap().clear();
    crew.ok("PATCH", &format!("/api/routines/{id}"), Some(json!({ "nextRun": common::due_now() }))).await;
    let event = crew.wait_event("пауза", |e| e["type"] == "journal").await;
    assert_eq!(event["entry"]["status"], "paused");
    assert_eq!(event["entry"]["tokens"], 0);
    assert!(fake.requests.lock().unwrap().is_empty(), "модель не вызывается");

    // Человек в чате продолжает работать: он видит, что происходит.
    crew.clear_events();
    fake.script.lock().unwrap().clear();
    crew.say(&bot, "привет").await;
    let end = crew.wait_event("ответ человеку", |e| e["type"] == "turn_end").await;
    assert_eq!(end["failed"], false, "{end}");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_night_routine_out_of_its_window_is_skipped_not_replayed() {
    let (crew, fake, bot) = setup("night-skip", &[("CREW_TEST_TICK_MS", "100")]).await;
    // Окно, в котором мы точно не находимся: через 3 часа от текущего момента.
    let hour = (chrono_hour() + 3) % 24;
    let from = format!("{hour:02}:00");
    let to = format!("{:02}:00", (hour + 1) % 24);
    let id = add_routine(&crew, &bot, json!({ "windowFrom": from, "windowTo": to })).await;
    crew.ok("PATCH", &format!("/api/routines/{id}"), Some(json!({ "nextRun": common::due_now() }))).await;

    let event = crew.wait_event("пропуск", |e| e["type"] == "journal").await;
    assert_eq!(event["entry"]["status"], "skipped", "{event}");
    assert!(fake.requests.lock().unwrap().is_empty(), "модель не вызывается");
    let state = crew.ok("GET", "/api/state", None).await;
    assert!(state["routines"][0]["nextRun"].as_i64().unwrap() > 1);
}

#[tokio::test(flavor = "multi_thread")]
async fn quiet_is_default_and_the_rule_is_only_in_quiet_routine_prompts() {
    let (crew, fake, bot) = setup("night-quiet-prompt", &[]).await;
    let quiet_id = add_routine(&crew, &bot, json!({})).await;
    let loud_id = add_routine(&crew, &bot, json!({ "quiet": false })).await;

    let state = crew.ok("GET", "/api/state", None).await;
    let routines = state["routines"].as_array().unwrap();
    let quiet = routines.iter().find(|r| r["id"] == quiet_id.as_str()).unwrap();
    let loud = routines.iter().find(|r| r["id"] == loud_id.as_str()).unwrap();
    assert_eq!(quiet["quiet"], true, "по умолчанию тихо");
    assert_eq!(loud["quiet"], false);

    run_and_wait(&crew, &quiet_id, json!({})).await;
    run_and_wait(&crew, &loud_id, json!({})).await;
    let requests = fake.requests.lock().unwrap().clone();
    let quiet_prompt = requests[0]["messages"][0]["content"].as_str().unwrap();
    let loud_prompt = requests[1]["messages"][0]["content"].as_str().unwrap();
    assert!(quiet_prompt.contains("[ТИХО]"), "нет правила в промпте рутины: {quiet_prompt}");
    assert!(!loud_prompt.contains("[ТИХО]"), "правило попало в обычную рутину: {loud_prompt}");

    crew.clear_events();
    crew.say(&bot, "привет").await;
    crew.wait_event("ответ в чате", |e| e["type"] == "turn_end").await;
    let requests = fake.requests.lock().unwrap();
    let chat_prompt = requests.last().unwrap()["messages"][0]["content"].as_str().unwrap();
    assert!(!chat_prompt.contains("[ТИХО]"), "правило попало в обычный чат: {chat_prompt}");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_run_started_by_a_person_is_never_hidden() {
    let (crew, fake, bot) = setup("night-quiet-person", &[]).await;
    *fake.script.lock().unwrap() = vec![say("проверил, всё спокойно")];
    let id = add_routine(&crew, &bot, json!({})).await;

    let entry = run_and_wait(&crew, &id, json!({ "attended": true })).await;
    assert_eq!(entry["status"], "ok", "{entry}");
    let prompt = fake.requests.lock().unwrap()[0]["messages"][0]["content"].as_str().unwrap().to_string();
    assert!(!prompt.contains("[ТИХО]"), "человек ждёт ответа, молчать незачем: {prompt}");
    let messages = thread(&crew, &bot, None).await;
    assert!(messages.iter().any(|m| m["text"] == "проверил, всё спокойно"), "{messages:?}");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_message_steered_into_a_quiet_run_stays_visible() {
    let (crew, fake, bot) = setup("night-quiet-steer", &[]).await;
    *fake.script.lock().unwrap() = vec![tool("run_command", json!({ "command": "echo ок > C:\\Windows\\deloist-probe.txt" })), say("[ТИХО]")];
    let id = add_routine(&crew, &bot, json!({})).await;

    crew.clear_events();
    crew.ok("POST", &format!("/api/routines/{id}/run"), Some(json!({ "attended": false }))).await;
    let card = crew.wait_event("карточка", |e| e["type"] == "approval").await;
    let (_, queued) = crew.send("POST", &format!("/api/bots/{bot}/messages"), Some(json!({ "text": "важная просьба" }))).await;
    let qid = queued["item"]["id"].as_str().unwrap().to_string();
    crew.ok("POST", &format!("/api/bots/{bot}/queue/{qid}/steer"), None).await;
    crew.ok("POST", &format!("/api/approvals/{}", card["id"].as_str().unwrap()), Some(json!({ "allow": true }))).await;

    let event = crew.wait_event("журнал", |e| e["type"] == "journal").await;
    assert_eq!(event["entry"]["status"], "ok", "{event}");
    assert_eq!(event["entry"]["text"], "Сообщать нечего.", "{event}");
    let messages = thread(&crew, &bot, None).await;
    assert!(messages.iter().any(|m| m["text"] == "важная просьба"), "сообщение человека пропало: {messages:?}");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_marker_alone_hides_the_routine_turn_and_its_journal_is_quiet() {
    let (crew, fake, bot) = setup("night-quiet-turn", &[]).await;
    *fake.usage.lock().unwrap() = Some((1_000, 200));
    *fake.script.lock().unwrap() = vec![say("[ТИХО]")];
    let id = add_routine(&crew, &bot, json!({})).await;

    let entry = run_and_wait(&crew, &id, json!({})).await;
    assert_eq!(entry["status"], "quiet", "{entry}");
    assert_eq!(entry["text"], "", "тихая запись без текста: {entry}");
    assert_eq!(entry["tokens"], 1200, "токены считаются как обычно: {entry}");

    let state = crew.ok("GET", "/api/state", None).await;
    assert_eq!(state["routines"][0]["lastStatus"], "quiet");
    assert_eq!(state["bots"][0]["preview"], Value::Null, "превью бота не показывает тихий ход: {state}");
    let end = crew.find_event(|e| e["type"] == "turn_end").expect("нет turn_end");
    assert_eq!(end["quiet"], true, "{end}");

    let messages = thread(&crew, &bot, None).await;
    assert!(messages.is_empty(), "тихий ход не должен быть виден: {messages:?}");

    crew.clear_events();
    *fake.script.lock().unwrap() = vec![say("живой ответ")];
    crew.say(&bot, "привет").await;
    crew.wait_event("ответ в чате", |e| e["type"] == "turn_end").await;
    let requests = fake.requests.lock().unwrap();
    let last = serde_json::to_string(&requests.last().unwrap()["messages"]).unwrap();
    assert!(!last.contains("[рутина]"), "рутина попала в следующий запрос: {last}");
    assert!(!last.contains("[ТИХО]"), "маркер попал в следующий запрос: {last}");
}

#[tokio::test(flavor = "multi_thread")]
async fn without_quiet_the_marker_is_an_ordinary_answer() {
    let (crew, fake, bot) = setup("night-quiet-off", &[]).await;
    *fake.script.lock().unwrap() = vec![say("[ТИХО]")];
    let id = add_routine(&crew, &bot, json!({ "quiet": false })).await;

    let entry = run_and_wait(&crew, &id, json!({})).await;
    assert_eq!(entry["status"], "ok", "{entry}");
    assert_eq!(entry["text"], "[ТИХО]", "{entry}");
    let messages = thread(&crew, &bot, None).await;
    assert!(has_text(&messages, "[ТИХО]"), "ответ виден в треде: {messages:?}");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_marker_followed_by_text_keeps_the_whole_answer() {
    let (crew, fake, bot) = setup("night-quiet-long", &[]).await;
    *fake.script.lock().unwrap() = vec![say("[ТИХО] но вот что нашёл")];
    let id = add_routine(&crew, &bot, json!({})).await;

    let entry = run_and_wait(&crew, &id, json!({})).await;
    assert_eq!(entry["status"], "ok", "{entry}");
    assert_eq!(entry["text"], "[ТИХО] но вот что нашёл");
    let messages = thread(&crew, &bot, None).await;
    assert!(has_text(&messages, "[ТИХО] но вот что нашёл"), "{messages:?}");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_quiet_answer_after_a_tool_still_hides_the_turn() {
    let (crew, fake, bot) = setup("night-quiet-tool", &[]).await;
    *fake.script.lock().unwrap() = vec![tool("list_files", json!({})), say("[ТИХО]")];
    let id = add_routine(&crew, &bot, json!({})).await;

    let entry = run_and_wait(&crew, &id, json!({})).await;
    assert_eq!(entry["status"], "quiet", "{entry}");
    assert!(
        crew.find_event(|e| e["type"] == "tool_update" && e["status"] == "ok").is_some(),
        "инструмент должен отработать"
    );
    let messages = thread(&crew, &bot, None).await;
    assert!(messages.is_empty(), "сообщения тихого хода скрыты: {messages:?}");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_missed_decision_makes_a_quiet_answer_visible() {
    let (crew, fake, bot) = setup("night-quiet-needs", &[("CREW_TEST_UNATTENDED_WAIT_MS", "300")]).await;
    *fake.script.lock().unwrap() = vec![tool("run_command", json!({ "command": "echo один > C:\\Windows\\deloist-probe.txt" })), say("[ТИХО]")];
    let id = add_routine(&crew, &bot, json!({})).await;

    let entry = run_and_wait(&crew, &id, json!({ "attended": false })).await;
    assert_eq!(entry["status"], "needs", "{entry}");
    assert_eq!(entry["text"], "Сообщать нечего.", "{entry}");
    assert!(!entry["missed"].as_array().unwrap().is_empty(), "{entry}");
    let messages = thread(&crew, &bot, None).await;
    assert!(!messages.is_empty(), "ход с решением виден в треде: {messages:?}");
    assert!(messages.iter().any(|m| m["text"] == "[рутина] Проверка\nпроверь сайт"), "{messages:?}");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_fallback_turn_is_not_quiet() {
    let main = Fake::default();
    let spare = Fake::default();
    let main_port = serve(main.clone()).await;
    let spare_port = serve(spare.clone()).await;

    let crew = Crew::start_with_env("night-quiet-fallback", &[]).await;
    crew.ok(
        "PUT",
        "/api/settings",
        Some(json!({
            "driver": "api", "model": "main-model",
            "baseUrl": format!("http://127.0.0.1:{main_port}/v1"), "apiKey": "main-key",
            "fallbackDriver": "api", "fallbackModel": "backup-model",
            "fallbackBaseUrl": format!("http://127.0.0.1:{spare_port}/v1"), "fallbackApiKey": "backup-key",
        })),
    )
    .await;
    crew.listen().await;
    let state = crew.ok("GET", "/api/state", None).await;
    let bot = state["bots"][0]["id"].as_str().unwrap().to_string();

    *main.fail.lock().unwrap() = Some((429, "usage limit reached".into()));
    *spare.script.lock().unwrap() = vec![say("[ТИХО]")];
    let id = add_routine(&crew, &bot, json!({})).await;

    let entry = run_and_wait(&crew, &id, json!({})).await;
    assert_eq!(entry["status"], "ok", "{entry}");
    assert_eq!(entry["fallback"], "backup-model", "{entry}");
    assert_eq!(entry["text"], "Сообщать нечего.", "{entry}");
    let messages = thread(&crew, &bot, None).await;
    assert!(has_text(&messages, "[ТИХО]"), "после запасной ход виден: {messages:?}");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_quiet_marker_in_a_live_chat_is_an_ordinary_message() {
    let (crew, fake, bot) = setup("night-quiet-chat", &[]).await;
    *fake.script.lock().unwrap() = vec![say("[ТИХО]")];
    crew.say(&bot, "привет").await;
    let end = crew.wait_event("ответ в чате", |e| e["type"] == "turn_end").await;
    assert_ne!(end["quiet"], true, "{end}");

    let messages = thread(&crew, &bot, None).await;
    assert!(has_text(&messages, "привет"), "{messages:?}");
    assert!(has_text(&messages, "[ТИХО]"), "ответ не скрыт: {messages:?}");
}

#[tokio::test(flavor = "multi_thread")]
async fn old_threads_read_as_before_and_quiet_records_do_not_eat_the_limit() {
    let crew = Crew::start("night-quiet-old-thread").await;
    let state = crew.ok("GET", "/api/state", None).await;
    let bot = state["bots"][0]["id"].as_str().unwrap().to_string();
    let path = crew.data_dir.join("threads").join(format!("{bot}.jsonl"));

    let old = vec![
        json!({ "id": "old1", "role": "user", "text": "старый вопрос" }),
        json!({ "id": "old2", "role": "assistant", "text": "старый ответ" }),
    ];
    std::fs::write(&path, old.iter().map(|l| l.to_string()).collect::<Vec<_>>().join("\n") + "\n").unwrap();
    let messages = thread(&crew, &bot, None).await;
    assert_eq!(messages.len(), 2, "старые записи без turnId читаются: {messages:?}");

    let mut lines = old.clone();
    lines.push(json!({ "id": "new1", "role": "assistant", "text": "важное сообщение" }));
    for i in 0..50 {
        let turn = format!("turn{i}");
        lines.push(json!({ "role": "user", "text": "[рутина] проверка", "turnId": turn }));
        lines.push(json!({ "role": "assistant", "text": "[ТИХО]", "turnId": turn }));
        lines.push(json!({ "role": "quiet", "turnId": turn }));
    }
    std::fs::write(&path, lines.iter().map(|l| l.to_string()).collect::<Vec<_>>().join("\n") + "\n").unwrap();

    let limited = thread(&crew, &bot, Some(10)).await;
    assert_eq!(limited.len(), 3, "лимит считается после отсева: {limited:?}");
    assert!(has_text(&limited, "важное сообщение"), "{limited:?}");
    assert!(!limited.iter().any(|m| m["text"] == "[ТИХО]"), "{limited:?}");
}

#[tokio::test(flavor = "multi_thread")]
async fn an_old_state_file_without_quiet_reads_as_quiet() {
    let dir = common::temp_data_dir("night-quiet-state");
    let mut first = Crew::start_at(dir.clone(), &[]).await;
    let state = first.ok("GET", "/api/state", None).await;
    let bot = state["bots"][0]["id"].as_str().unwrap().to_string();
    add_routine(&first, &bot, json!({})).await;
    first.keep_data();
    drop(first);

    let path = dir.join("state.json");
    let text = std::fs::read_to_string(&path).unwrap();
    let mut state: Value = serde_json::from_str(&text).unwrap();
    state["routines"][0].as_object_mut().unwrap().remove("quiet");
    std::fs::write(&path, serde_json::to_string_pretty(&state).unwrap()).unwrap();

    let second = Crew::start_at(dir, &[]).await;
    let state = second.ok("GET", "/api/state", None).await;
    assert_eq!(state["routines"][0]["quiet"], true, "старый файл считается тихим: {state}");
}

#[tokio::test(flavor = "multi_thread")]
async fn catch_up_last_runs_once_and_logs_missed() {
    let (crew, fake, bot) = setup("night-catchup-last", &[("CREW_TEST_TICK_MS", "100")]).await;
    let id = add_routine(&crew, &bot, json!({ "catchUp": "last" })).await;
    let now_ms = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_millis() as i64 - 3 * 3600 * 1000;
    crew.ok("PATCH", &format!("/api/routines/{id}"), Some(json!({ "nextRun": now_ms }))).await;

    // Ждём либо пропуск, либо запуск. Сначала придёт запись «Пропущено …».
    let mut seen_missed = false;
    for _ in 0..50 {
        crew.wait_event("журнал", |e| e["type"] == "journal").await;
        let log = journal(&crew).await;
        if let Some(entries) = log["entries"].as_array() {
            if entries.iter().any(|e| e["text"].as_str().unwrap_or("").contains("Пропущено")) {
                seen_missed = true;
                break;
            }
        }
    }
    assert!(seen_missed, "должна быть запись «Пропущено»");
    crew.wait_event("ход", |e| e["type"] == "turn_end").await;
    // Рутина запустилась: модель вызвана.
    assert!(!fake.requests.lock().unwrap().is_empty(), "рутина должна запуститься");
}

#[tokio::test(flavor = "multi_thread")]
async fn catch_up_skip_does_not_run() {
    let (crew, fake, bot) = setup("night-catchup-skip", &[("CREW_TEST_TICK_MS", "100")]).await;
    let id = add_routine(&crew, &bot, json!({ "catchUp": "skip" })).await;
    let now_ms = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_millis() as i64 - 3 * 3600 * 1000;
    crew.ok("PATCH", &format!("/api/routines/{id}"), Some(json!({ "nextRun": now_ms }))).await;

    let event = crew.wait_event("пропуск", |e| e["type"] == "journal").await;
    assert_eq!(event["entry"]["status"], "skipped", "{event}");
    assert!(event["entry"]["text"].as_str().unwrap_or("").contains("Пропущено"), "{event}");
    assert!(fake.requests.lock().unwrap().is_empty(), "рутина не должна запускаться");
}

fn chrono_hour() -> u32 {
    let script = "(Get-Date).Hour";
    let output = std::process::Command::new("pwsh").args(["-NoProfile", "-Command", script]).output().unwrap();
    String::from_utf8_lossy(&output.stdout).trim().parse().unwrap()
}

#[cfg(windows)]
#[tokio::test(flavor = "multi_thread")]
async fn autostart_uses_only_the_test_key_here() {
    let key = format!(r"HKCU\Software\CrewTest{}", rand_id());
    let crew = Crew::start_with_env("night-autostart", &[("CREW_TEST_RUN_KEY", &key)]).await;
    // Запись от прежнего названия: иначе приложение стартовало бы дважды.
    let old = std::process::Command::new("reg").args(["add", &key, "/v", "Crew", "/d", "old.exe --tray", "/f"]).output().unwrap();
    assert!(old.status.success());
    crew.ok("PUT", "/api/settings", Some(json!({ "autostart": true }))).await;
    let query = std::process::Command::new("reg").args(["query", &key, "/v", "Deloist"]).output().unwrap();
    let text = String::from_utf8_lossy(&query.stdout).to_string();
    assert!(text.contains("--tray"), "{text}");
    let old = std::process::Command::new("reg").args(["query", &key, "/v", "Crew"]).output().unwrap();
    assert!(!old.status.success(), "старая запись осталась");

    crew.ok("PUT", "/api/settings", Some(json!({ "autostart": false }))).await;
    let query = std::process::Command::new("reg").args(["query", &key, "/v", "Deloist"]).output().unwrap();
    assert!(!query.status.success(), "значение убрано");
    let _ = std::process::Command::new("reg").args(["delete", &key, "/f"]).output();
    let state = crew.ok("GET", "/api/state", None).await;
    assert_eq!(state["settings"]["autostart"], false);
}
