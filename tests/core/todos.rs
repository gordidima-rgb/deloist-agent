//! Задачи человека: фраза, повтор, перенос, архив, поиск, предел, инструменты бота.

use crate::common;

use std::sync::{Arc, Mutex};

use axum::body::Bytes;
use axum::extract::State;
use axum::response::{IntoResponse, Response};
use axum::routing::post;
use axum::{Json, Router};
use chrono::{Datelike, Duration, Local, NaiveDate};
use common::Crew;
use serde_json::{json, Value};

fn today() -> NaiveDate {
    Local::now().date_naive()
}

fn day_offset(n: i64) -> String {
    (today() + Duration::days(n)).format("%Y-%m-%d").to_string()
}

#[tokio::test(flavor = "multi_thread")]
async fn a_phrase_becomes_a_task_with_tomorrow_and_ten() {
    let crew = Crew::start("todos-phrase").await;
    crew.listen().await;
    // «в 10 часов»: голое «в 10» dates_ru временем не считает (см. правило в разборе фраз).
    let made = crew.ok("POST", "/api/todos", Some(json!({ "text": "позвонить маме завтра в 10 часов" }))).await;
    assert_eq!(made["title"].as_str().unwrap().to_lowercase(), "позвонить маме", "{made}");
    assert_eq!(made["due_date"], day_offset(1), "{made}");
    assert_eq!(made["due_time"], "10:00", "{made}");
    assert_eq!(made["bucket"], "active");
    crew.wait_event("todos_changed", |e| e["type"] == "todos_changed").await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_repeating_task_makes_one_next_instance_and_not_two() {
    let crew = Crew::start("todos-repeat").await;
    let made = crew.ok("POST", "/api/todos", Some(json!({ "text": "каждую среду вынести мусор" }))).await;
    assert_eq!(made["recurrence"]["freq"], "weekly", "{made}");
    let id = made["id"].as_str().unwrap().to_string();

    let first = crew.ok("POST", &format!("/api/todos/{id}/done"), None).await;
    assert_eq!(first["todo"]["done"], true);
    let next = &first["next"];
    assert!(next.is_object(), "следующий экземпляр: {first}");
    let next_day = NaiveDate::parse_from_str(next["due_date"].as_str().unwrap(), "%Y-%m-%d").unwrap();
    assert_eq!(next_day.weekday().num_days_from_monday(), 2, "среда");
    assert!(next_day > today());

    let again = crew.ok("POST", &format!("/api/todos/{id}/done"), None).await;
    assert!(again["next"].is_null(), "вторая отметка не создаёт второй экземпляр: {again}");
    let open = crew.ok("GET", "/api/todos", None).await;
    assert_eq!(open.as_array().unwrap().len(), 2, "исходная и один следующий");
}

#[tokio::test(flavor = "multi_thread")]
async fn carry_over_moves_overdue_tasks_and_says_how_many() {
    let crew = Crew::start("todos-carry").await;
    crew.ok("POST", "/api/todos", Some(json!({ "title": "старая один", "due_date": day_offset(-3) }))).await;
    crew.ok("POST", "/api/todos", Some(json!({ "title": "старая два", "due_date": day_offset(-1) }))).await;
    crew.ok("POST", "/api/todos", Some(json!({ "title": "сегодняшняя", "due_date": day_offset(0) }))).await;
    let carried = crew.ok("POST", "/api/todos/carry-over", None).await;
    assert_eq!(carried["count"], 2, "{carried}");
    assert_eq!(carried["text"], "Перенёс 2 задачи на сегодня.");
    let today_list = crew.ok("GET", "/api/todos?filter=today", None).await;
    assert_eq!(today_list.as_array().unwrap().len(), 3, "обе перенесённые и сегодняшняя");
    let nothing = crew.ok("POST", "/api/todos/carry-over", None).await;
    assert_eq!(nothing["count"], 0);
}

#[tokio::test(flavor = "multi_thread")]
async fn archive_then_restore_brings_the_task_back() {
    let crew = Crew::start("todos-archive").await;
    let made = crew.ok("POST", "/api/todos", Some(json!({ "title": "удалю и верну" }))).await;
    let id = made["id"].as_str().unwrap().to_string();
    let (status, _) = crew.send("DELETE", &format!("/api/todos/{id}"), None).await;
    assert_eq!(status, 204);
    assert!(crew.ok("GET", "/api/todos", None).await.as_array().unwrap().is_empty());
    let archive = crew.ok("GET", "/api/todos/archive", None).await;
    assert_eq!(archive.as_array().unwrap().len(), 1, "{archive}");
    let back = crew.ok("POST", &format!("/api/todos/{id}/restore"), None).await;
    assert_eq!(back["title"], "удалю и верну");
    assert_eq!(crew.ok("GET", "/api/todos", None).await.as_array().unwrap().len(), 1);
}

#[tokio::test(flavor = "multi_thread")]
async fn search_finds_a_phone_by_its_last_digits() {
    let crew = Crew::start("todos-search").await;
    crew.ok("POST", "/api/todos", Some(json!({ "title": "Позвонить +79001234567" }))).await;
    crew.ok("POST", "/api/todos", Some(json!({ "title": "Купить хлеб" }))).await;
    let by_digits = crew.ok("GET", "/api/todos?q=8%20900%20123-45-67", None).await;
    assert_eq!(by_digits.as_array().unwrap().len(), 1, "{by_digits}");
    assert_eq!(by_digits[0]["title"], "Позвонить +79001234567");
    let by_word = crew.ok("GET", "/api/todos?q=хлеб", None).await;
    assert_eq!(by_word.as_array().unwrap().len(), 1);
    let short = crew.ok("GET", "/api/todos?q=9%2000", None).await;
    assert!(short.as_array().unwrap().is_empty(), "короткое число не поиск номера");
}

#[tokio::test(flavor = "multi_thread")]
async fn the_two_thousand_and_first_task_is_refused() {
    // Файл с двумя тысячами задач кладём заранее: создавать их по одной долго.
    let dir = common::temp_data_dir("todos-limit");
    std::fs::create_dir_all(&dir).unwrap();
    let todos: Vec<Value> = (0..2000).map(|n| json!({ "id": format!("todo_{n}"), "title": format!("задача {n}"), "created_at": n })).collect();
    std::fs::write(dir.join("todos.json"), json!({ "todos": todos }).to_string()).unwrap();
    let crew = Crew::start_at(dir, &[]).await;
    let (status, body) = crew.send("POST", "/api/todos", Some(json!({ "title": "лишняя" }))).await;
    assert_eq!(status, 409, "{body}");
    assert_eq!(body["error"], "todo_limit");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_bot_adds_and_completes_a_task_without_asking() {
    let (crew, fake, bot) = setup("todos-bot").await;

    *fake.script.lock().unwrap() = vec![tool("add_todo", json!({ "text": "позвонить маме завтра в 10 часов" })), say("Добавил")];
    let (status, result) = ask_and_result(&crew, &bot, "добавь позвонить маме завтра в 10").await;
    assert_eq!(status, "ok", "{result}");
    assert!(result.to_lowercase().contains("позвонить маме"), "{result}");
    let list = crew.ok("GET", "/api/todos", None).await;
    assert_eq!(list.as_array().unwrap().len(), 1);
    assert_eq!(list[0]["done"], false);

    *fake.script.lock().unwrap() = vec![tool("complete_todo", json!({ "id": "позвонить маме" })), say("Готово")];
    let (status, result) = ask_and_result(&crew, &bot, "отметь позвонить маме").await;
    assert_eq!(status, "ok", "{result}");
    let list = crew.ok("GET", "/api/todos", None).await;
    assert_eq!(list[0]["done"], true, "бот отметил задачу");
}

// --- Разбор фразы моделью (todo_parse): сколько раз звали модель, что сохранилось ---

/// Ответ модели с двумя делами на одну дату: «купить хлеб завтра и позвонить маме».
fn two_tasks_reply() -> String {
    json!({ "tasks": [
        { "title": "Купить хлеб", "importance": "active", "due_date": day_offset(1) },
        { "title": "Позвонить маме", "importance": "key", "due_date": day_offset(1) },
    ] })
    .to_string()
}

/// Сколько ответов ещё ждут в сценарии: каждый вызов модели съедает один.
fn left(fake: &Fake) -> usize {
    fake.script.lock().unwrap().len()
}

#[tokio::test(flavor = "multi_thread")]
async fn a_phrase_the_code_finishes_makes_no_model_call() {
    let (crew, fake, _bot) = setup("todos-parse-code").await;
    *fake.script.lock().unwrap() = vec![say("не должен прозвучать")];
    let (status, made) = crew.send("POST", "/api/todos/from-text", Some(json!({ "text": "позвонить маме завтра в 10 часов" }))).await;
    assert_eq!(status, 201, "{made}");
    assert_eq!(made.as_array().unwrap().len(), 1, "{made}");
    assert_eq!(made[0]["due_time"], "10:00");
    assert_eq!(left(&fake), 1, "модель не звали");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_two_task_phrase_makes_one_model_call_and_two_todos() {
    let (crew, fake, _bot) = setup("todos-parse-two").await;
    *fake.script.lock().unwrap() = vec![say(&two_tasks_reply()), say("не должен прозвучать")];
    let (status, made) = crew.send("POST", "/api/todos/from-text", Some(json!({ "text": "купить хлеб завтра и позвонить маме" }))).await;
    assert_eq!(status, 201, "{made}");
    assert_eq!(made.as_array().unwrap().len(), 2, "{made}");
    assert_eq!(made[1]["bucket"], "key");
    assert_eq!(made[0]["due_date"], day_offset(1));
    assert_eq!(left(&fake), 1, "одна попытка модели");
    assert_eq!(crew.ok("GET", "/api/todos", None).await.as_array().unwrap().len(), 2);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_section_chosen_by_the_person_beats_the_model_guess() {
    let (crew, fake, _bot) = setup("todos-parse-bucket").await;
    *fake.script.lock().unwrap() = vec![say(&two_tasks_reply()), say(&two_tasks_reply())];
    let (status, made) = crew.send("POST", "/api/todos/from-text", Some(json!({ "text": "купить хлеб завтра и позвонить маме", "bucket": "someday" }))).await;
    assert_eq!(status, 201, "{made}");
    assert!(made.as_array().unwrap().iter().all(|todo| todo["bucket"] == "someday"), "выбор человека: {made}");
    // «Актуальное» — значение кнопки по умолчанию: догадку модели («key» у второй задачи) не перебивает.
    let (_, made) = crew.send("POST", "/api/todos/from-text", Some(json!({ "text": "купить хлеб завтра и позвонить маме", "bucket": "active" }))).await;
    assert_eq!(made[1]["bucket"], "key", "{made}");
    let (status, _) = crew.send("POST", "/api/todos/from-text", Some(json!({ "text": "купить хлеб", "bucket": "куда-то" }))).await;
    assert_eq!(status, 422, "неизвестный раздел");
}

#[tokio::test(flavor = "multi_thread")]
async fn preview_shows_the_model_split_and_saves_nothing() {
    let (crew, fake, _bot) = setup("todos-parse-preview").await;
    *fake.script.lock().unwrap() = vec![say(&two_tasks_reply())];
    let preview = crew.ok("POST", "/api/todos/parse", Some(json!({ "text": "купить хлеб завтра и позвонить маме" }))).await;
    assert_eq!(preview["source"], "model", "{preview}");
    assert_eq!(preview["tasks"].as_array().unwrap().len(), 2, "{preview}");
    assert!(crew.ok("GET", "/api/todos", None).await.as_array().unwrap().is_empty(), "предпросмотр ничего не сохраняет");
}

#[tokio::test(flavor = "multi_thread")]
async fn broken_json_twice_falls_back_to_one_task_and_loses_nothing() {
    let (crew, fake, _bot) = setup("todos-parse-broken").await;
    *fake.script.lock().unwrap() = vec![say("не json"), say("снова не json"), say("не должен прозвучать")];
    let (status, made) = crew.send("POST", "/api/todos/from-text", Some(json!({ "text": "купить хлеб завтра и позвонить маме" }))).await;
    assert_eq!(status, 201, "{made}");
    assert_eq!(made.as_array().unwrap().len(), 1, "без модели одна задача: {made}");
    let title = made[0]["title"].as_str().unwrap().to_lowercase();
    assert!(title.contains("хлеб") && title.contains("маме"), "ни одно слово не потеряно: {title}");
    assert_eq!(made[0]["due_date"], day_offset(1), "дату нашёл код");
    assert_eq!(left(&fake), 1, "две попытки модели, третьей нет");
}

#[tokio::test(flavor = "multi_thread")]
async fn without_an_api_model_the_phrase_still_becomes_a_task() {
    // Демо-режим: модели нет, фраза уходит в фолбэк кодом.
    let crew = Crew::start("todos-parse-demo").await;
    crew.listen().await;
    let (status, made) = crew.send("POST", "/api/todos/from-text", Some(json!({ "text": "купить хлеб завтра и позвонить маме" }))).await;
    assert_eq!(status, 201, "{made}");
    assert_eq!(made.as_array().unwrap().len(), 1, "{made}");
    let preview = crew.ok("POST", "/api/todos/parse", Some(json!({ "text": "полить цветы завтра и помыть окна" }))).await;
    assert_eq!(preview["source"], "fallback", "{preview}");
}

// --- Инструменты бота: сценарий модели по шагам, как в tests/core/habits.rs ---

#[derive(Clone, Default)]
struct Fake {
    script: Arc<Mutex<Vec<Value>>>,
}

fn say(text: &str) -> Value {
    json!({ "role": "assistant", "content": text })
}

fn tool(name: &str, args: Value) -> Value {
    let id = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos();
    json!({ "role": "assistant", "content": "", "tool_calls": [{ "id": format!("c{id}"), "type": "function", "function": { "name": name, "arguments": args.to_string() } }] })
}

async fn completions(State(fake): State<Fake>, _body: Bytes) -> Response {
    let message = {
        let mut script = fake.script.lock().unwrap();
        if script.is_empty() {
            say("готово")
        } else {
            script.remove(0)
        }
    };
    Json(json!({ "choices": [{ "message": message }] })).into_response()
}

async fn setup(name: &str) -> (Crew, Fake, String) {
    let fake = Fake::default();
    let router = Router::new().route("/v1/chat/completions", post(completions)).with_state(fake.clone());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });

    let crew = Crew::start_with_env(name, &[]).await;
    crew.ok("PUT", "/api/settings", Some(json!({ "driver": "api", "baseUrl": format!("http://127.0.0.1:{port}/v1"), "apiKey": "k" }))).await;
    crew.listen().await;
    let state = crew.ok("GET", "/api/state", None).await;
    let bot = state["bots"][0]["id"].as_str().unwrap().to_string();
    (crew, fake, bot)
}

/// Сказать боту и дождаться конца хода; вернуть результат инструмента (статус и текст).
async fn ask_and_result(crew: &Crew, bot: &str, text: &str) -> (String, String) {
    crew.clear_events();
    crew.say(bot, text).await;
    crew.wait_event("конец хода", |e| e["type"] == "turn_end").await;
    let update = crew.find_event(|e| e["type"] == "tool_update" && e["status"] != "running").expect("инструмент не отвечал");
    (update["status"].as_str().unwrap().to_string(), update["result"].as_str().unwrap_or_default().to_string())
}

#[tokio::test(flavor = "multi_thread")]
async fn a_file_is_stored_under_the_task_and_bad_files_are_refused() {
    let crew = Crew::start("todos-files").await;
    let made = crew.ok("POST", "/api/todos", Some(json!({ "text": "собрать отчёт" }))).await;
    let id = made["id"].as_str().unwrap().to_string();
    // «привет» в base64.
    let (status, with_file) = crew.send("POST", &format!("/api/todos/{id}/files"), Some(json!({ "name": "report.txt", "data": "0L/RgNC40LLQtdGC" }))).await;
    assert_eq!(status, 201, "{with_file}");
    let file = &with_file["files"][0];
    assert_eq!(file["name"], "report.txt");
    assert_eq!(file["size"], 12);
    let stored = std::fs::read(crew.data_dir.join("todo-files").join(&id).join(file["path"].as_str().unwrap())).unwrap();
    assert_eq!(String::from_utf8(stored).unwrap(), "привет");
    // Имя с путём, пустой файл и чужая задача не принимаются.
    let (status, _) = crew.send("POST", &format!("/api/todos/{id}/files"), Some(json!({ "name": "../x.txt", "data": "0L/RgNC40LLQtdGC" }))).await;
    assert_eq!(status, 422);
    let (status, _) = crew.send("POST", &format!("/api/todos/{id}/files"), Some(json!({ "name": "a.txt", "data": "" }))).await;
    assert_eq!(status, 422);
    let (status, _) = crew.send("POST", "/api/todos/nope/files", Some(json!({ "name": "a.txt", "data": "0L/RgNC40LLQtdGC" }))).await;
    assert_eq!(status, 404);
}
