//! Причина остановки и ответ на неё: связь (повтор сам или человек), вход, проверка, застрял, серия ошибок рутины.

use crate::common;

use std::sync::{Arc, Mutex};
use std::time::Duration;

use axum::body::Bytes;
use axum::extract::State;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::post;
use axum::{Json, Router};
use common::Crew;
use serde_json::{json, Value};

fn now_ms() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_millis() as i64
}

#[derive(Clone)]
enum Step {
    Fail(u16, &'static str),
    Tool(&'static str, Value),
    /// Модель думает `ms` и только потом зовёт инструмент.
    SlowTool(u64, &'static str, Value),
    /// Модель думает `ms` и отвечает.
    Slow(u64),
}

#[derive(Clone, Default)]
struct Fake {
    script: Arc<Mutex<Vec<Step>>>,
    calls: Arc<Mutex<usize>>,
}

impl Fake {
    fn calls(&self) -> usize {
        *self.calls.lock().unwrap()
    }
    fn script(&self, steps: Vec<Step>) {
        *self.script.lock().unwrap() = steps;
    }
}

fn tool_message(name: &str, args: &Value) -> Value {
    json!({
        "role": "assistant", "content": "",
        "tool_calls": [{ "id": "call_1", "type": "function", "function": { "name": name, "arguments": args.to_string() } }],
    })
}

async fn completions(State(fake): State<Fake>, _body: Bytes) -> Response {
    *fake.calls.lock().unwrap() += 1;
    let step = {
        let mut script = fake.script.lock().unwrap();
        if script.is_empty() {
            None
        } else {
            Some(script.remove(0))
        }
    };
    let message = match step {
        None => json!({ "role": "assistant", "content": "готово" }),
        Some(Step::Fail(status, text)) => return (StatusCode::from_u16(status).unwrap(), text).into_response(),
        Some(Step::Tool(name, args)) => tool_message(name, &args),
        Some(Step::SlowTool(ms, name, args)) => {
            tokio::time::sleep(Duration::from_millis(ms)).await;
            tool_message(name, &args)
        }
        Some(Step::Slow(ms)) => {
            tokio::time::sleep(Duration::from_millis(ms)).await;
            json!({ "role": "assistant", "content": "медленно" })
        }
    };
    Json(json!({ "choices": [{ "message": message }] })).into_response()
}

async fn setup(name: &str, env: &[(&str, &str)]) -> (Crew, Fake, String) {
    let fake = Fake::default();
    let router = Router::new().route("/v1/chat/completions", post(completions)).with_state(fake.clone());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
    let crew = Crew::start_with_env(name, env).await;
    crew.ok(
        "PUT",
        "/api/settings",
        Some(json!({ "driver": "api", "model": "test-model", "baseUrl": format!("http://127.0.0.1:{port}/v1"), "apiKey": "test-key" })),
    )
    .await;
    crew.listen().await;
    let bot = crew.ok("GET", "/api/state", None).await["bots"][0]["id"].as_str().unwrap().to_string();
    crew.ok("PATCH", &format!("/api/bots/{bot}"), Some(json!({ "approval": "auto" }))).await;
    (crew, fake, bot)
}

fn runs_file(crew: &Crew) -> Vec<Value> {
    serde_json::from_str(&std::fs::read_to_string(crew.data_dir.join("runs.json")).unwrap_or_else(|_| "[]".into())).unwrap()
}

async fn thread(crew: &Crew, bot: &str) -> Vec<Value> {
    crew.ok("GET", &format!("/api/bots/{bot}/thread?limit=100"), None).await["messages"].as_array().cloned().unwrap_or_default()
}

fn count_said(messages: &[Value], text: &str) -> usize {
    messages.iter().filter(|m| m["text"].as_str().is_some_and(|t| t.contains(text))).count()
}

async fn wait_run(crew: &Crew, label: &str, pred: impl Fn(&Value) -> bool) -> Value {
    let found: Arc<Mutex<Option<Value>>> = Arc::default();
    common::wait_until(label, 30, || async {
        let hit = runs_file(crew).into_iter().find(|r| pred(r));
        *found.lock().unwrap() = hit.clone();
        hit.is_some()
    })
    .await;
    let run = found.lock().unwrap().clone().unwrap();
    run
}

async fn add_routine(crew: &Crew, bot: &str, title: &str) -> String {
    let body = json!({ "botId": bot, "title": title, "prompt": "сделай работу", "everyMinutes": 600 });
    crew.ok("POST", "/api/workflows", Some(body)).await["process"]["id"].as_str().unwrap().to_string()
}

async fn run_unattended(crew: &Crew, id: &str) -> Value {
    crew.clear_events();
    crew.ok("POST", &format!("/api/workflows/{id}/run"), Some(json!({ "attended": false }))).await;
    crew.wait_event("журнал", |e| e["type"] == "journal" && e["entry"]["status"] != "paused").await["entry"].clone()
}

async fn routine(crew: &Crew, id: &str) -> Value {
    crew.ok("GET", "/api/state", None).await["processes"].as_array().unwrap().iter().find(|r| r["id"] == id).cloned().unwrap()
}

const FAST: [(&str, &str); 2] = [("CREW_TEST_TICK_MS", "200"), ("CREW_TEST_HOUR_MS", "60000")];

#[tokio::test(flavor = "multi_thread")]
async fn a_network_error_before_any_step_retries_by_itself_and_succeeds() {
    let (crew, fake, bot) = setup("reasons-net-retry", &FAST).await;
    // Первая попытка и пять переподключений внутри хода: дальше ход встаёт на паузу «сеть».
    fake.script(vec![Step::Fail(503, "temporary outage"); 6]);
    crew.say(&bot, "сделай работу").await;
    let end = crew.wait_event("конец хода", |e| e["type"] == "turn_end").await;
    assert_eq!(end["failed"], false, "обрыв связи не провал: {end}");
    let run = wait_run(&crew, "пауза «сеть»", |r| r["state"]["kind"] == "paused").await;
    assert_eq!(run["state"]["reason"], "network", "{run}");
    assert!(run["state"]["until_ms"].as_i64().unwrap() <= now_ms() + 2_000, "первый повтор через «минуту» теста: {run}");
    assert_eq!(count_said(&thread(&crew, &bot).await, "повторю в"), 1);

    // Срок вышел: тик продолжает ход, модель отвечает.
    let done = wait_run(&crew, "ход закончен", |r| r["state"]["kind"] == "done").await;
    assert_eq!((done["retries"].as_u64(), done["attempts"].as_u64()), (Some(1), Some(0)), "{done}");
    assert_eq!(fake.calls(), 7, "шесть отказов и одно продолжение");
    assert_eq!(count_said(&thread(&crew, &bot).await, "готово"), 1);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_network_error_after_a_local_step_asks_the_human_and_never_retries() {
    let (crew, fake, bot) = setup("reasons-net-human", &FAST).await;
    let mut script = vec![Step::Tool("write_file", json!({ "path": "created.txt", "content": "готово" }))];
    script.extend(vec![Step::Fail(503, "temporary outage"); 6]);
    fake.script(script);
    crew.say(&bot, "сделай работу").await;
    let run = wait_run(&crew, "ждёт человека", |r| r["state"]["kind"] == "needs_human").await;
    assert_eq!(run["state"]["reason"], "network", "{run}");
    assert!(run["state"]["text"].as_str().unwrap().contains("Повторить"), "{run}");
    assert_eq!(run["acted"], true);

    // Самому повторять нельзя: прошлый шаг уже менял файлы.
    tokio::time::sleep(Duration::from_millis(1_800)).await;
    assert_eq!(fake.calls(), 7);
    assert_eq!(runs_file(&crew)[0]["state"]["kind"], "needs_human");

    // «Повторить» — та же кнопка «Продолжить»: ход идёт дальше с заметкой о связи.
    crew.ok("POST", &format!("/api/runs/{}/continue", run["id"].as_str().unwrap()), None).await;
    wait_run(&crew, "ход закончен", |r| r["state"]["kind"] == "done").await;
    assert_eq!(count_said(&thread(&crew, &bot).await, "Связь с провайдером оборвалась"), 1);
}

#[tokio::test(flavor = "multi_thread")]
async fn two_bots_waiting_for_one_login_leave_one_card() {
    let (crew, fake, first) = setup("reasons-login", &[]).await;
    let second = crew.ok("POST", "/api/bots", Some(json!({ "name": "Второй", "role": "тест" }))).await["bot"]["id"].as_str().unwrap().to_string();
    fake.script(vec![Step::Fail(401, "bad key"), Step::Fail(401, "bad key")]);
    for bot in [&first, &second] {
        crew.clear_events();
        crew.say(bot, "сделай работу").await;
        crew.wait_event("конец хода", |e| e["type"] == "turn_end" && e["botId"] == bot.as_str()).await;
    }
    let runs = runs_file(&crew);
    let waiting: Vec<&Value> = runs.iter().filter(|r| r["state"]["kind"] == "needs_human").collect();
    assert_eq!(waiting.len(), 1, "{runs:?}");
    assert_eq!(waiting[0]["state"]["reason"], "login");
    assert!(waiting[0]["state"]["text"].as_str().unwrap().starts_with("Нужен вход в"), "{waiting:?}");
    let other = runs.iter().find(|r| r["bot_id"] == second.as_str()).unwrap();
    assert_eq!((other["state"]["kind"].as_str(), other["state"]["reason"].as_str()), (Some("failed"), Some("login")), "{other}");
    let needs = crew.ok("GET", "/api/needs", None).await;
    assert_eq!(needs["items"].as_array().unwrap().iter().filter(|i| i["kind"] == "run").count(), 1, "{needs}");
    assert_eq!(fake.calls(), 2);
}

#[tokio::test(flavor = "multi_thread")]
async fn three_login_failures_pause_the_routine_with_one_chat_message() {
    let (crew, fake, bot) = setup("reasons-login-routine", &[("CREW_TEST_TICK_MS", "100")]).await;
    let id = add_routine(&crew, &bot, "Ночная").await;
    fake.script(vec![Step::Fail(401, "bad key"); 3]);
    for n in 1..=3 {
        let entry = run_unattended(&crew, &id).await;
        assert_eq!((entry["status"].as_str(), entry["reason"].as_str()), (Some(if n == 1 { "needs" } else { "failed" }), Some("login")), "{entry}");
    }
    let saved = routine(&crew, &id).await;
    assert_eq!((saved["errorPaused"].as_bool(), saved["failures"].as_u64(), saved["enabled"].as_bool()), (Some(true), Some(3), Some(true)), "{saved}");
    assert_eq!(count_said(&thread(&crew, &bot).await, "3 ошибки подряд"), 1, "одно сообщение на серию");

    // Срок пришёл, а рутина на паузе: запуска нет, в журнале сказано почему.
    let calls = fake.calls();
    crew.clear_events();
    crew.ok("PATCH", &format!("/api/workflows/{id}"), Some(json!({ "nextRun": now_ms() - 1_000 }))).await;
    let skipped = crew.wait_event("пропуск", |e| e["type"] == "journal" && e["entry"]["text"].as_str().is_some_and(|t| t.contains("на паузе из-за ошибок"))).await;
    assert_eq!(skipped["entry"]["status"], "paused");
    assert_eq!(fake.calls(), calls);

    // «Возобновить»: тот же PATCH рутины, который ключ программы не может подать.
    let resumed = crew.ok("PATCH", &format!("/api/workflows/{id}"), Some(json!({ "errorPaused": false }))).await;
    assert!(resumed["process"]["errorPaused"].is_null() && resumed["process"]["failures"].is_null(), "{resumed}");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_failed_check_is_kept_as_the_reason() {
    let (crew, fake, bot) = setup("reasons-check", &[]).await;
    let id = add_routine(&crew, &bot, "Проверка").await;
    fake.script(vec![Step::Tool("finish_turn", json!({ "status": "failed", "note": "тесты красные" }))]);
    let entry = run_unattended(&crew, &id).await;
    assert_eq!((entry["status"].as_str(), entry["reason"].as_str()), (Some("failed"), Some("check")), "{entry}");
    let run = wait_run(&crew, "запись закрыта", |r| r["state"]["kind"] == "failed").await;
    assert_eq!(run["state"]["reason"], "check", "{run}");
    assert_eq!(routine(&crew, &id).await["stopReason"], "check");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_busy_bot_is_awaited_for_an_hour_then_the_run_is_skipped_with_a_journal_line() {
    let (crew, fake, bot) = setup("reasons-busy", &[("CREW_TEST_TICK_MS", "100"), ("CREW_TEST_HOUR_MS", "2400")]).await;
    fake.script(vec![Step::Slow(15_000)]);
    crew.say(&bot, "долгое дело").await;
    let id = add_routine(&crew, &bot, "Ночная").await;
    crew.clear_events();
    crew.ok("PATCH", &format!("/api/workflows/{id}"), Some(json!({ "nextRun": now_ms() - 1_000 }))).await;
    let entry = crew.wait_event("запись «бот был занят»", |e| e["type"] == "journal" && e["entry"]["status"] == "busy").await["entry"].clone();
    assert!(entry["text"].as_str().unwrap().contains("Бот был занят"), "{entry}");
    assert_eq!(fake.calls(), 1, "рутина не стартовала");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_run_longer_than_its_limit_stops_as_stuck() {
    // «Час» теста — секунда: предел рутины 3 часа = 3 секунды.
    let (crew, fake, bot) = setup("reasons-time-limit", &[("CREW_TEST_HOUR_MS", "1000")]).await;
    let id = add_routine(&crew, &bot, "Долгая").await;
    fake.script(vec![Step::SlowTool(3_600, "read_file", json!({ "path": "нет-такого.txt" }))]);
    let entry = run_unattended(&crew, &id).await;
    assert_eq!((entry["status"].as_str(), entry["reason"].as_str()), (Some("failed"), Some("stuck")), "{entry}");
    assert!(entry["text"].as_str().unwrap().contains("Превышено время запуска"), "{entry}");
    let run = wait_run(&crew, "запись закрыта", |r| r["state"]["kind"] == "failed").await;
    assert_eq!(run["state"]["reason"], "stuck", "{run}");
}
