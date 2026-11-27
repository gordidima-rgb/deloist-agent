//! Надёжные запуски процесса «триггер → бот»: те же правила, что у рутин (лимит, обрыв, причины, тишина, догон, очередь).

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
    Say(String),
    Tool(&'static str, Value),
    Fail(u16, &'static str),
    /// Ответ висит, пока процесс не убьют: ход остаётся оборванным посреди шага.
    Hold,
    /// Модель думает `ms` и отвечает.
    Slow(u64),
}

#[derive(Clone, Default)]
struct Fake {
    script: Arc<Mutex<Vec<Step>>>,
    requests: Arc<Mutex<Vec<Value>>>,
    /// Что провайдер сообщает о расходе: (вход, выход).
    usage: Arc<Mutex<Option<(i64, i64)>>>,
}

impl Fake {
    fn calls(&self) -> usize {
        self.requests.lock().unwrap().len()
    }
    fn script(&self, steps: Vec<Step>) {
        *self.script.lock().unwrap() = steps;
    }
}

async fn completions(State(fake): State<Fake>, body: Bytes) -> Response {
    fake.requests.lock().unwrap().push(serde_json::from_slice(&body).unwrap_or_else(|_| json!({})));
    let step = {
        let mut script = fake.script.lock().unwrap();
        if script.is_empty() {
            Step::Say("готово".into())
        } else {
            script.remove(0)
        }
    };
    let message = match step {
        Step::Fail(status, text) => return (StatusCode::from_u16(status).unwrap(), text).into_response(),
        Step::Hold => {
            tokio::time::sleep(Duration::from_secs(600)).await;
            json!({ "role": "assistant", "content": "поздно" })
        }
        Step::Slow(ms) => {
            tokio::time::sleep(Duration::from_millis(ms)).await;
            json!({ "role": "assistant", "content": "медленно" })
        }
        Step::Say(text) => json!({ "role": "assistant", "content": text }),
        Step::Tool(name, args) => json!({
            "role": "assistant", "content": "",
            "tool_calls": [{ "id": "call_1", "type": "function", "function": { "name": name, "arguments": args.to_string() } }],
        }),
    };
    let mut body = json!({ "choices": [{ "message": message }] });
    if let Some((input, output)) = *fake.usage.lock().unwrap() {
        body["usage"] = json!({ "prompt_tokens": input, "completion_tokens": output });
    }
    Json(body).into_response()
}

async fn fake_server() -> (Fake, u16) {
    let fake = Fake::default();
    let router = Router::new().route("/v1/chat/completions", post(completions)).with_state(fake.clone());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
    (fake, port)
}

async fn configure(crew: &Crew, port: u16) -> String {
    crew.ok(
        "PUT",
        "/api/settings",
        Some(json!({ "driver": "api", "model": "test-model", "baseUrl": format!("http://127.0.0.1:{port}/v1"), "apiKey": "test-key" })),
    )
    .await;
    crew.listen().await;
    let bot = crew.ok("GET", "/api/state", None).await["bots"][0]["id"].as_str().unwrap().to_string();
    crew.ok("PATCH", &format!("/api/bots/{bot}"), Some(json!({ "approval": "auto" }))).await;
    bot
}

async fn setup(name: &str, env: &[(&str, &str)]) -> (Crew, Fake, String) {
    let (fake, port) = fake_server().await;
    let crew = Crew::start_with_env(name, env).await;
    let bot = configure(&crew, port).await;
    (crew, fake, bot)
}

fn runs_file_at(dir: &std::path::Path) -> Vec<Value> {
    serde_json::from_str(&std::fs::read_to_string(dir.join("runs.json")).unwrap_or_else(|_| "[]".into())).unwrap()
}

fn runs_file(crew: &Crew) -> Vec<Value> {
    runs_file_at(&crew.data_dir)
}

/// Запись журнала запусков, которую ведёт шаг процесса (не разговор человека).
fn process_record(crew: &Crew, kind: &str) -> Option<Value> {
    runs_file(crew).into_iter().find(|r| r["origin"]["process"].is_object() && r["state"]["kind"] == kind)
}

async fn wait_journal_state(crew: &Crew, kind: &str) -> Value {
    common::wait_until(kind, 30, || async { process_record(crew, kind).is_some() }).await;
    process_record(crew, kind).unwrap()
}

async fn thread(crew: &Crew, bot: &str) -> Vec<Value> {
    crew.ok("GET", &format!("/api/bots/{bot}/thread?limit=100"), None).await["messages"].as_array().cloned().unwrap_or_default()
}

fn said(messages: &[Value], text: &str) -> usize {
    messages.iter().filter(|m| m["text"].as_str().is_some_and(|t| t.contains(text))).count()
}

fn node(id: &str, kind: &str, fields: Value) -> Value {
    json!({ "id": id, "kind": kind, "x": 0, "y": 0, "fields": fields })
}

/// «Триггер → бот»: так выглядит рутина как процесс. `trigger` — поля запускающего узла, `extra` — поля шага-бота.
fn chain(bot: &str, trigger: Value, schedule: Option<Value>, extra: Value) -> Value {
    let mut t = node("t", "trigger", trigger);
    if let Some(schedule) = schedule {
        t["schedule"] = schedule;
    }
    let mut fields = json!({ "task": "сделай работу", "bot_id": bot });
    fields.as_object_mut().unwrap().extend(extra.as_object().cloned().unwrap_or_default());
    json!({ "nodes": [t, node("b", "bot", fields)], "edges": [{ "id": "e1", "from": "t", "to": "b" }] })
}

fn hook_chain(bot: &str, extra: Value) -> Value {
    chain(bot, json!({ "trigger": "webhook" }), None, extra)
}

fn daily_chain(bot: &str) -> Value {
    chain(bot, json!({ "trigger": "schedule" }), Some(json!({ "kind": "daily", "time": "09:00" })), json!({}))
}

async fn create(crew: &Crew, title: &str, graph: Value) -> String {
    let made = crew.ok("POST", "/api/workflows", Some(json!({ "title": title }))).await;
    let id = made["workflow"]["id"].as_str().unwrap().to_string();
    crew.ok("PUT", &format!("/api/workflows/{id}/graph"), Some(json!({ "base_revision": 1, "graph": graph }))).await;
    id
}

/// Вебхук запускает процесс без человека: так идёт запуск «по расписанию».
async fn fire(crew: &Crew, id: &str) -> String {
    let url = crew.ok("POST", &format!("/api/workflows/{id}/nodes/t/webhook"), None).await["url"].as_str().unwrap().to_string();
    let bare = reqwest::Client::builder().no_proxy().build().unwrap();
    let res = bare.post(&url).body("данные").send().await.unwrap();
    assert_eq!(res.status().as_u16(), 202);
    res.json::<Value>().await.unwrap()["run_id"].as_str().unwrap().to_string()
}

async fn run_by_hand(crew: &Crew, id: &str) -> String {
    crew.ok("POST", &format!("/api/workflows/{id}/run"), Some(json!({}))).await["run"]["id"].as_str().unwrap().to_string()
}

async fn wf_runs(crew: &Crew, id: &str) -> Vec<Value> {
    crew.ok("GET", &format!("/api/workflows/{id}/runs"), None).await["runs"].as_array().cloned().unwrap_or_default()
}

async fn wait_wf_run(crew: &Crew, id: &str, run: &str, status: &str) -> Value {
    common::wait_until(&format!("запуск {status}"), 30, || async { wf_runs(crew, id).await.iter().any(|r| r["id"] == run && r["status"] == status) }).await;
    wf_runs(crew, id).await.into_iter().find(|r| r["id"] == run).unwrap()
}

fn step<'a>(run: &'a Value, node: &str) -> &'a Value {
    run["steps"].as_array().unwrap().iter().filter(|s| s["node_id"] == node).last().unwrap_or_else(|| panic!("нет шага {node}: {run}"))
}

async fn summary(crew: &Crew, id: &str) -> Value {
    crew.ok("GET", "/api/workflows", None).await["workflows"].as_array().unwrap().iter().find(|w| w["id"] == id).cloned().unwrap()
}

/// Срок расписания в прошлом: так выглядит процесс, который проспал компьютер.
fn make_due(dir: &std::path::Path, id: &str, ago_ms: i64) {
    let path = dir.join("workflows").join(format!("{id}.json"));
    let mut stored: Value = serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
    for slot in stored["slots"].as_object_mut().unwrap().values_mut() {
        slot["next"] = json!(now_ms() - ago_ms);
    }
    std::fs::write(path, stored.to_string()).unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn a_limit_pauses_the_bot_step_and_one_resume_finishes_the_process() {
    let (crew, fake, bot) = setup("proc-limit", &[("CREW_TEST_TICK_MS", "200"), ("CREW_TEST_PAUSE_BASE_MS", "3500")]).await;
    let id = create(&crew, "Лимит", hook_chain(&bot, json!({}))).await;
    fake.script(vec![Step::Fail(429, "usage limit reached")]);
    let run = run_by_hand(&crew, &id).await;

    // Лимит — не провал: запись журнала ждёт срока, а шаг и процесс остаются «идёт».
    let paused = wait_journal_state(&crew, "paused").await;
    assert_eq!(paused["state"]["reason"], "limit", "{paused}");
    assert_eq!(paused["origin"]["process"]["id"], id.as_str(), "{paused}");
    let now = wf_runs(&crew, &id).await[0].clone();
    assert_eq!((now["status"].as_str(), step(&now, "b")["status"].as_str()), (Some("running"), Some("running")), "{now}");

    // Срок вышел: тик продолжает ход один раз, и шаг получает его итог.
    let done = wait_wf_run(&crew, &id, &run, "done").await;
    assert_eq!(step(&done, "b")["output"], "готово", "{done}");
    let journal = wait_journal_state(&crew, "done").await;
    assert_eq!((journal["pauses"].as_u64(), journal["attempts"].as_u64()), (Some(1), Some(0)), "{journal}");
    tokio::time::sleep(Duration::from_millis(600)).await;
    assert_eq!(fake.calls(), 2, "один отказ и одно продолжение");
    assert_eq!(runs_file(&crew).len(), 1);
    assert_eq!(said(&thread(&crew, &bot).await, "Лимит сбросился — продолжай задание"), 1);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_bot_step_cut_off_by_a_restart_continues_once_instead_of_failing() {
    let (fake, port) = fake_server().await;
    let mut first = Crew::start_with_env("proc-restart", &[]).await;
    let bot = configure(&first, port).await;
    let id = create(&first, "Обрыв", hook_chain(&bot, json!({}))).await;
    fake.script(vec![Step::Hold]);
    let run = run_by_hand(&first, &id).await;
    common::wait_until("запрос завис", 20, || async { fake.calls() == 1 }).await;
    let dir = first.data_dir.clone();
    first.keep_data();
    drop(first);

    fake.script(vec![Step::Say("доведено".into())]);
    let second = Crew::start_at(dir.clone(), &[]).await;
    second.listen().await;
    let done = wait_wf_run(&second, &id, &run, "done").await;
    let bot_step = step(&done, "b");
    assert_eq!((bot_step["status"].as_str(), bot_step["output"].as_str()), (Some("done"), Some("доведено")), "{done}");
    assert_eq!(bot_step["attempt"], 1, "шаг не начинали заново: {bot_step}");
    assert_eq!(fake.calls(), 2, "один обрыв и одно продолжение");
    assert_eq!(runs_file(&second).len(), 1, "{:?}", runs_file(&second));
    assert_eq!(said(&thread(&second, &bot).await, "Тебя прервал перезапуск программы"), 1);
}

#[tokio::test(flavor = "multi_thread")]
async fn three_failures_in_a_row_pause_the_process_with_one_chat_message() {
    let (crew, fake, bot) = setup("proc-series", &[("CREW_TEST_TICK_MS", "200")]).await;
    let id = create(&crew, "Серия", hook_chain(&bot, json!({}))).await;
    fake.script(vec![Step::Fail(400, "boom"); 3]);
    for n in 1..=3 {
        let run = run_by_hand(&crew, &id).await;
        let failed = wait_wf_run(&crew, &id, &run, "failed").await;
        assert_eq!(step(&failed, "b")["status"], "failed", "{n}: {failed}");
        common::wait_until("серия записана", 20, || async { summary(&crew, &id).await["failures"] == n }).await;
    }
    let saved = summary(&crew, &id).await;
    assert_eq!((saved["error_paused"].as_bool(), saved["failures"].as_u64(), saved["stop_reason"].as_str()), (Some(true), Some(3), Some("unknown")), "{saved}");
    assert_eq!(said(&thread(&crew, &bot).await, "на паузе: 3 ошибки подряд"), 1, "одно сообщение на серию");

    // Расписание на паузе: срок пришёл, запуска нет.
    let calls = fake.calls();
    let stored = crew.data_dir.join("workflows").join(format!("{id}.json"));
    assert!(std::fs::read_to_string(stored).unwrap().contains("error_paused"));
    // Ручной запуск возобновляет, а успех сбрасывает серию.
    let run = run_by_hand(&crew, &id).await;
    wait_wf_run(&crew, &id, &run, "done").await;
    assert_eq!(fake.calls(), calls + 1);
    common::wait_until("серия сброшена", 20, || async { summary(&crew, &id).await["failures"].as_u64().unwrap_or(0) == 0 }).await;
    let resumed = summary(&crew, &id).await;
    assert!(resumed["error_paused"] == false && resumed["stop_reason"].is_null(), "{resumed}");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_failed_check_fails_the_step_and_keeps_its_reason() {
    let (crew, fake, bot) = setup("proc-check", &[]).await;
    let id = create(&crew, "Проверка", hook_chain(&bot, json!({}))).await;
    fake.script(vec![Step::Tool("finish_turn", json!({ "status": "failed", "note": "тесты красные" }))]);
    let run = fire(&crew, &id).await;
    let failed = wait_wf_run(&crew, &id, &run, "failed").await;
    assert_eq!(step(&failed, "b")["status"], "failed", "{failed}");
    let journal = wait_journal_state(&crew, "failed").await;
    assert_eq!(journal["state"]["reason"], "check", "{journal}");
    assert_eq!(summary(&crew, &id).await["stop_reason"], "check");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_bot_that_needs_the_human_fails_the_step_instead_of_passing_an_empty_answer() {
    let (crew, fake, bot) = setup("proc-need-you", &[]).await;
    let id = create(&crew, "Нужен ты", hook_chain(&bot, json!({}))).await;
    fake.script(vec![Step::Tool("finish_turn", json!({ "status": "need_you", "note": "не хватает доступа" }))]);
    let run = fire(&crew, &id).await;
    let failed = wait_wf_run(&crew, &id, &run, "failed").await;
    assert!(step(&failed, "b")["error"].as_str().unwrap().contains("нужен человек"), "{failed}");
    assert_eq!(summary(&crew, &id).await["failures"].as_u64().unwrap_or(0), 0, "ожидание человека серию не растит");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_quiet_step_with_nothing_to_say_hides_the_turn_and_a_loud_one_does_not() {
    let (crew, fake, bot) = setup("proc-quiet", &[]).await;
    let quiet = create(&crew, "Тихий", hook_chain(&bot, json!({ "quiet": "true" }))).await;
    fake.script(vec![Step::Tool("finish_turn", json!({ "status": "nothing" }))]);
    let run = fire(&crew, &quiet).await;
    let done = wait_wf_run(&crew, &quiet, &run, "done").await;
    assert_eq!((step(&done, "b")["status"].as_str(), step(&done, "b")["output"].as_str()), (Some("done"), Some("")), "{done}");
    assert_eq!(said(&thread(&crew, &bot).await, "поручает тебе шаг"), 0, "тихий ход исчез из истории");

    // Шаг вывода после тихого бота без новостей ничего не шлёт: пустое сообщение в чат не уходит.
    let mut graph = hook_chain(&bot, json!({ "quiet": "true" }));
    graph["nodes"].as_array_mut().unwrap().push(node("o", "output", json!({ "target": "chat", "bot_id": bot })));
    graph["edges"].as_array_mut().unwrap().push(json!({ "id": "e2", "from": "b", "to": "o" }));
    let relayed = create(&crew, "Тихий с выводом", graph).await;
    fake.script(vec![Step::Tool("finish_turn", json!({ "status": "nothing" }))]);
    let before = thread(&crew, &bot).await.len();
    let run = fire(&crew, &relayed).await;
    let done = wait_wf_run(&crew, &relayed, &run, "done").await;
    assert_eq!(step(&done, "o")["status"].as_str(), Some("done"), "{done}");
    assert_eq!(thread(&crew, &bot).await.len(), before, "в чат ушло пустое сообщение");

    // Тот же ответ без «тихо»: ход виден.
    let loud = create(&crew, "Громкий", hook_chain(&bot, json!({}))).await;
    fake.script(vec![Step::Tool("finish_turn", json!({ "status": "nothing" }))]);
    let run = fire(&crew, &loud).await;
    wait_wf_run(&crew, &loud, &run, "done").await;
    assert_eq!(said(&thread(&crew, &bot).await, "поручает тебе шаг"), 1);

    // Запуск, который нажал человек, не прячут и в тихом процессе.
    let by_hand = run_by_hand(&crew, &quiet).await;
    wait_wf_run(&crew, &quiet, &by_hand, "done").await;
    assert_eq!(said(&thread(&crew, &bot).await, "поручает тебе шаг"), 2);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_busy_bot_is_awaited_for_an_hour_then_the_step_fails_without_a_model_call() {
    let (crew, fake, bot) = setup("proc-busy", &[("CREW_TEST_TICK_MS", "100"), ("CREW_TEST_HOUR_MS", "2400")]).await;
    let id = create(&crew, "Занят", hook_chain(&bot, json!({}))).await;
    fake.script(vec![Step::Slow(15_000)]);
    crew.say(&bot, "долгое дело").await;
    common::wait_until("бот занят", 20, || async { fake.calls() == 1 }).await;
    let run = fire(&crew, &id).await;
    let failed = wait_wf_run(&crew, &id, &run, "failed").await;
    assert!(step(&failed, "b")["error"].as_str().unwrap().contains("больше часа"), "{failed}");
    assert_eq!(fake.calls(), 1, "шаг не стартовал");
}

#[tokio::test(flavor = "multi_thread")]
async fn the_daily_limit_pauses_the_bot_step_until_midnight_but_not_a_person() {
    let (crew, fake, bot) = setup("proc-budget", &[("CREW_TEST_TICK_MS", "200")]).await;
    *fake.usage.lock().unwrap() = Some((600, 100));
    crew.say(&bot, "привет").await;
    crew.wait_event("ответ человеку", |e| e["type"] == "turn_end").await;
    crew.ok("PUT", "/api/settings", Some(json!({ "dailyTokenLimit": 500 }))).await;
    let id = create(&crew, "Бюджет", hook_chain(&bot, json!({}))).await;
    let calls = fake.calls();
    let run = fire(&crew, &id).await;

    let paused = wait_journal_state(&crew, "paused").await;
    assert_eq!(paused["state"]["reason"], "budget", "{paused}");
    let tomorrow = chrono::Local::now().date_naive().succ_opt().unwrap().and_time(chrono::NaiveTime::MIN);
    let midnight = chrono::TimeZone::from_local_datetime(&chrono::Local, &tomorrow).earliest().unwrap().timestamp_millis();
    assert_eq!(paused["state"]["until_ms"].as_i64(), Some(midnight), "{paused}");
    assert_eq!(fake.calls(), calls, "модель не вызвана");
    let now = wf_runs(&crew, &id).await.into_iter().find(|r| r["id"] == run.as_str()).unwrap();
    assert_eq!(step(&now, "b")["status"], "running", "шаг ждёт, а не падает: {now}");

    // Человек в чате продолжает работать.
    crew.clear_events();
    crew.say(&bot, "ещё раз").await;
    let end = crew.wait_event("ответ человеку", |e| e["type"] == "turn_end").await;
    assert_eq!(end["failed"], false, "{end}");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_new_term_does_not_start_while_the_limit_pause_is_ahead() {
    let (fake, port) = fake_server().await;
    let mut first = Crew::start_with_env("proc-ahead", &[]).await;
    let bot = configure(&first, port).await;
    let id = create(&first, "Лимит впереди", daily_chain(&bot)).await;
    fake.script(vec![Step::Fail(429, "usage limit reached")]);
    let run = run_by_hand(&first, &id).await;
    wait_journal_state(&first, "paused").await;
    let dir = first.data_dir.clone();
    first.keep_data();
    drop(first);

    // Пауза пережила перезапуск, а срок расписания пропущен ночью: новый запуск не нужен, разбудит пауза.
    make_due(&dir, &id, 30 * 60_000);
    let second = Crew::start_at(dir.clone(), &[("CREW_TEST_TICK_MS", "200")]).await;
    second.listen().await;
    tokio::time::sleep(Duration::from_millis(1500)).await;
    assert_eq!(fake.calls(), 1, "модель не звали");
    let runs = wf_runs(&second, &id).await;
    assert_eq!((runs.len(), runs[0]["id"].as_str(), runs[0]["status"].as_str()), (1, Some(run.as_str()), Some("running")), "{runs:?}");
    let journal = runs_file(&second);
    assert_eq!((journal.len(), journal[0]["state"]["kind"].as_str()), (1, Some("paused")), "{journal:?}");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_new_term_replaces_a_run_stuck_waiting_for_a_human_without_a_live_card() {
    let (fake, port) = fake_server().await;
    let mut first = Crew::start_with_env("proc-replace", &[]).await;
    let bot = configure(&first, port).await;
    let id = create(&first, "Заменить", daily_chain(&bot)).await;
    // Вход в провайдера: ход ждёт человека, карточки у него нет.
    fake.script(vec![Step::Fail(401, "bad key")]);
    let old = run_by_hand(&first, &id).await;
    let waiting = wait_journal_state(&first, "needs_human").await;
    assert_eq!(waiting["state"]["reason"], "login", "{waiting}");
    let dir = first.data_dir.clone();
    first.keep_data();
    drop(first);

    make_due(&dir, &id, 30 * 60_000);
    let second = Crew::start_at(dir.clone(), &[("CREW_TEST_TICK_MS", "200")]).await;
    second.listen().await;
    common::wait_until("новый запуск", 30, || async { wf_runs(&second, &id).await.len() == 2 }).await;
    let runs = wf_runs(&second, &id).await;
    let new = runs.iter().find(|r| r["id"] != old.as_str()).unwrap().clone();
    let new_id = new["id"].as_str().unwrap().to_string();
    let done = wait_wf_run(&second, &id, &new_id, "done").await;
    assert_eq!(step(&done, "b")["output"], "готово", "{done}");
    let old_now = wf_runs(&second, &id).await.into_iter().find(|r| r["id"] == old.as_str()).unwrap();
    assert_eq!(old_now["status"], "stopped", "прежний запуск закрыт: {old_now}");
    let journal = runs_file(&second);
    assert!(journal.iter().all(|r| !matches!(r["state"]["kind"].as_str(), Some("paused" | "needs_human"))), "ничего не ждёт: {journal:?}");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_late_term_runs_once_with_a_note_and_a_very_late_one_is_skipped() {
    let (fake, port) = fake_server().await;
    let mut first = Crew::start_with_env("proc-catchup", &[]).await;
    let bot = configure(&first, port).await;
    let late = create(&first, "Опоздал", daily_chain(&bot)).await;
    let skipped = create(&first, "Проспал", daily_chain(&bot)).await;
    let dir = first.data_dir.clone();
    first.keep_data();
    drop(first);

    fake.script(vec![Step::Tool("finish_turn", json!({ "status": "done" }))]);
    make_due(&dir, &late, 3 * 3_600_000);
    make_due(&dir, &skipped, 20 * 3_600_000);
    let second = Crew::start_at(dir.clone(), &[("CREW_TEST_TICK_MS", "200")]).await;
    second.listen().await;
    common::wait_until("догон", 30, || async { wf_runs(&second, &late).await.iter().any(|r| r["status"] == "done") }).await;
    let run = wf_runs(&second, &late).await[0].clone();
    let note = step(&run, "t")["output"].as_str().unwrap().to_string();
    assert!(note.contains("Запуск опоздал на 3 ч"), "{note}");
    assert!(fake.requests.lock().unwrap()[0].to_string().contains("Запуск опоздал на 3 ч"), "бот узнал, что запуск опоздал");
    common::wait_until("запись о пропуске", 20, || async { second.ok("GET", "/api/journal", None).await.to_string().contains("Срок прошёл") }).await;
    assert!(wf_runs(&second, &skipped).await.is_empty(), "слишком поздний срок не запускают");
    assert_eq!(fake.calls(), 1);
}
