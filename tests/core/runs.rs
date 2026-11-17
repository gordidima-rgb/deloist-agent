//! Журнал запусков: обрыв хода перезапуском программы. Ход продолжается один раз сам или ждёт человека.

use crate::common;

use std::sync::{Arc, Mutex};
use std::time::Duration;

use axum::body::Bytes;
use axum::extract::State;
use axum::response::{IntoResponse, Response};
use axum::routing::post;
use axum::{Json, Router};
use common::Crew;
use serde_json::{json, Value};

#[derive(Clone)]
enum Step {
    Say(String),
    Tool(&'static str, Value),
    /// Ответ висит, пока процесс не убьют: так ход остаётся оборванным посреди шага.
    Hold,
}

#[derive(Clone, Default)]
struct Fake {
    script: Arc<Mutex<Vec<Step>>>,
    requests: Arc<Mutex<Vec<Value>>>,
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
        Step::Hold => {
            tokio::time::sleep(Duration::from_secs(600)).await;
            json!({ "role": "assistant", "content": "поздно" })
        }
        Step::Say(text) => json!({ "role": "assistant", "content": text }),
        Step::Tool(name, args) => json!({
            "role": "assistant",
            "content": "",
            "tool_calls": [{ "id": "call_1", "type": "function", "function": { "name": name, "arguments": args.to_string() } }],
        }),
    };
    Json(json!({ "choices": [{ "message": message }] })).into_response()
}

async fn fake_server() -> (Fake, u16) {
    let fake = Fake::default();
    let router = Router::new().route("/v1/chat/completions", post(completions)).with_state(fake.clone());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
    (fake, port)
}

async fn first_start(name: &str, port: u16) -> (Crew, std::path::PathBuf, String) {
    let dir = common::temp_data_dir(name);
    let crew = Crew::start_at(dir.clone(), &[]).await;
    crew.ok(
        "PUT",
        "/api/settings",
        Some(json!({ "driver": "api", "model": "test-model", "baseUrl": format!("http://127.0.0.1:{port}/v1"), "apiKey": "test-key" })),
    )
    .await;
    crew.listen().await;
    let state = crew.ok("GET", "/api/state", None).await;
    let bot = state["bots"][0]["id"].as_str().unwrap().to_string();
    (crew, dir, bot)
}

async fn restart(dir: &std::path::Path) -> Crew {
    let crew = Crew::start_at(dir.to_path_buf(), &[]).await;
    crew.listen().await;
    crew
}

fn runs_file(dir: &std::path::Path) -> Vec<Value> {
    serde_json::from_str(&std::fs::read_to_string(dir.join("runs.json")).unwrap_or_else(|_| "[]".into())).unwrap()
}

async fn thread(crew: &Crew, bot: &str) -> Vec<Value> {
    crew.ok("GET", &format!("/api/bots/{bot}/thread?limit=100"), None).await["messages"].as_array().cloned().unwrap_or_default()
}

async fn wait_thread(crew: &Crew, bot: &str, text: &str) -> Vec<Value> {
    common::wait_until(text, 30, || async { thread(crew, bot).await.iter().any(|m| m["text"].as_str().is_some_and(|t| t.contains(text))) }).await;
    thread(crew, bot).await
}

/// Запись, как её оставил бы оборванный ход: шаги и состояние задаются тестом.
fn write_run(dir: &std::path::Path, bot: &str, origin: Value, steps: Value, attempts: u32) {
    let run = json!({
        "id": "run_test", "bot_id": bot, "origin": origin, "source": "chat", "unattended": false, "quiet": false, "floor": null,
        "started_ms": 1, "updated_ms": 1, "state": { "kind": "running" }, "attempts": attempts, "steps": steps, "note": "",
    });
    std::fs::write(dir.join("runs.json"), json!([run]).to_string()).unwrap();
}

fn step(label: &str, effect: &str, state: &str) -> Value {
    json!({ "call_id": format!("c_{label}"), "tool": "t", "label": label, "effect": effect, "state": state })
}

#[tokio::test(flavor = "multi_thread")]
async fn a_human_turn_cut_off_mid_step_continues_once_and_keeps_the_steps() {
    let (fake, port) = fake_server().await;
    let (mut first, dir, bot) = first_start("runs-human", port).await;
    *fake.script.lock().unwrap() = vec![Step::Tool("list_files", json!({})), Step::Hold];
    first.say(&bot, "осмотри папку").await;
    // Первый шаг готов и записан, второй запрос к модели завис: программу «убивает» перезапуск.
    common::wait_until("шаг записан", 20, || async {
        runs_file(&dir).iter().any(|r| r["steps"].as_array().is_some_and(|s| s.iter().any(|x| x["tool"] == "list_files" && x["state"] == "ok")))
    })
    .await;
    first.keep_data();
    drop(first);
    let saved = std::fs::read_to_string(dir.join("threads").join(format!("{bot}.jsonl"))).unwrap();
    assert!(saved.contains("list_files"), "шаг до обрыва не попал в тред: {saved}");

    *fake.script.lock().unwrap() = vec![Step::Say("довёл до конца".into())];
    let second = restart(&dir).await;
    let messages = wait_thread(&second, &bot, "довёл до конца").await;
    let note = messages.iter().find(|m| m["role"] == "user" && m["text"].as_str().is_some_and(|t| t.starts_with("Тебя прервал перезапуск"))).expect("заметки продолжения нет в треде");
    assert!(note["text"].as_str().unwrap().contains("список файлов"), "{note}");
    let runs = runs_file(&dir);
    assert_eq!(runs.len(), 1, "продолжение берёт прежнюю запись: {runs:?}");
    assert_eq!(runs[0]["attempts"], 1);
    common::wait_until("запись закрыта", 20, || async { runs_file(&dir)[0]["state"]["kind"] == "done" }).await;
    // Модель получила ход заново один раз: запрос с заметкой и ответом на неё.
    assert_eq!(fake.requests.lock().unwrap().len(), 3, "ждали три запроса: два до обрыва и один после");
}

#[tokio::test(flavor = "multi_thread")]
async fn an_external_step_without_a_result_waits_for_the_human_and_continues_on_the_button() {
    let (fake, port) = fake_server().await;
    let (mut first, dir, bot) = first_start("runs-external", port).await;
    first.keep_data();
    drop(first);
    write_run(&dir, &bot, json!("human"), json!([step("чтение файла", "read", "ok"), step("письмо клиенту", "external", "started")]), 0);

    *fake.script.lock().unwrap() = vec![Step::Say("проверил и дошёл до конца".into())];
    let second = restart(&dir).await;
    let needs = second.ok("GET", "/api/needs", None).await;
    let item = needs["items"].as_array().unwrap().iter().find(|i| i["kind"] == "run").unwrap_or_else(|| panic!("в «Нужен ты» нет прерванного хода: {needs}")).clone();
    assert!(item["text"].as_str().unwrap().contains("Прервали во время: письмо клиенту"), "{item}");
    assert_eq!(item["botId"], bot.as_str());
    wait_thread(&second, &bot, "Прервали во время").await;
    assert_eq!(fake.requests.lock().unwrap().len(), 0, "внешний шаг не повторяется сам: модель не звали");

    second.ok("POST", &format!("/api/runs/{}/continue", item["id"].as_str().unwrap()), None).await;
    let messages = wait_thread(&second, &bot, "проверил и дошёл до конца").await;
    let note = messages.iter().find(|m| m["role"] == "user").expect("заметка продолжения");
    assert!(note["text"].as_str().unwrap().contains("прошёл ли он — неизвестно"), "{note}");
    common::wait_until("запись закрыта", 20, || async { runs_file(&dir)[0]["state"]["kind"] == "done" }).await;
    let needs = second.ok("GET", "/api/needs", None).await;
    assert!(!needs["items"].as_array().unwrap().iter().any(|i| i["kind"] == "run"), "{needs}");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_turn_cut_off_twice_asks_the_human() {
    let (fake, port) = fake_server().await;
    let (mut first, dir, bot) = first_start("runs-twice", port).await;
    first.keep_data();
    drop(first);
    write_run(&dir, &bot, json!("human"), json!([step("чтение файла", "read", "ok")]), 1);
    let second = restart(&dir).await;
    let needs = second.ok("GET", "/api/needs", None).await;
    let item = needs["items"].as_array().unwrap().iter().find(|i| i["kind"] == "run").unwrap_or_else(|| panic!("{needs}")).clone();
    assert!(item["text"].as_str().unwrap().contains("дважды"), "{item}");
    assert_eq!(fake.requests.lock().unwrap().len(), 0);
    // «Отменить» закрывает запись, из «Нужен ты» она уходит.
    second.ok("POST", &format!("/api/runs/{}/dismiss", item["id"].as_str().unwrap()), None).await;
    let needs = second.ok("GET", "/api/needs", None).await;
    assert!(!needs["items"].as_array().unwrap().iter().any(|i| i["kind"] == "run"), "{needs}");
}

#[tokio::test(flavor = "multi_thread")]
async fn an_interrupted_routine_continues_instead_of_starting_over() {
    let (fake, port) = fake_server().await;
    let (mut first, dir, bot) = first_start("runs-routine", port).await;
    let body = json!({ "botId": bot, "title": "Проверка", "prompt": "проверь сайт", "everyMinutes": 600 });
    let routine = first.ok("POST", "/api/routines", Some(body)).await["routine"]["id"].as_str().unwrap().to_string();
    first.keep_data();
    drop(first);
    // Рутина числилась «идёт», когда программа закрылась.
    let path = dir.join("state.json");
    let mut state: Value = serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
    state["routines"][0]["lastStatus"] = json!("running");
    std::fs::write(&path, serde_json::to_string(&state).unwrap()).unwrap();
    write_run(&dir, &bot, json!({ "routine": { "id": routine, "title": "Проверка" } }), json!([step("чтение файла", "read", "ok")]), 0);

    *fake.script.lock().unwrap() = vec![Step::Say("сайт проверен".into())];
    let second = restart(&dir).await;
    wait_thread(&second, &bot, "сайт проверен").await;
    let journal = second.ok("GET", "/api/journal", None).await;
    let entries = journal.as_array().cloned().or_else(|| journal["entries"].as_array().cloned()).unwrap_or_default();
    let entry = entries.iter().find(|e| e["routineId"] == routine.as_str()).unwrap_or_else(|| panic!("{journal}"));
    assert_eq!(entry["status"], "ok", "{entry}");
    assert!(!entries.iter().any(|e| e["status"] == "interrupted"), "перезапуск больше не «повтор с нуля»: {entries:?}");
    let state = second.ok("GET", "/api/state", None).await;
    let saved = state["routines"].as_array().unwrap().iter().find(|r| r["id"] == routine.as_str()).unwrap().clone();
    assert_eq!(saved["retryAttempt"].as_i64().unwrap_or(0), 0, "{saved}");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_queue_is_not_paused_by_a_restart() {
    let (fake, port) = fake_server().await;
    let (mut first, dir, bot) = first_start("runs-queue", port).await;
    first.keep_data();
    drop(first);
    std::fs::write(dir.join("queue.json"), json!({ bot.clone(): { "paused": true, "pause_reason": "restart", "items": [{ "id": "q1", "text": "из очереди" }] } }).to_string()).unwrap();
    *fake.script.lock().unwrap() = vec![Step::Say("очередь идёт".into())];
    let second = restart(&dir).await;
    wait_thread(&second, &bot, "очередь идёт").await;
    let queue = second.ok("GET", &format!("/api/bots/{bot}/queue"), None).await;
    assert_eq!(queue["paused"], false, "{queue}");
    assert_eq!(queue["items"].as_array().unwrap().len(), 0, "{queue}");
}
