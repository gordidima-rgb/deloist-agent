//! Вложения в чате: файл → воркспейс бота → описание в сообщении → то, что видит модель.

use crate::common;

use std::sync::{Arc, Mutex};

use axum::body::{Body, Bytes};
use axum::extract::State;
use axum::response::Response;
use axum::routing::post;
use axum::Router;
use common::Crew;
use serde_json::{json, Value};

/// Настоящий PNG 1×1: по нему видно, что файл дошёл байт в байт.
const PNG: &[u8] = &[
    0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a, 0x00, 0x00, 0x00, 0x0d, 0x49, 0x48, 0x44, 0x52, 0x00, 0x00,
    0x00, 0x01, 0x00, 0x00, 0x00, 0x01, 0x08, 0x06, 0x00, 0x00, 0x00, 0x1f, 0x15, 0xc4, 0x89, 0x00, 0x00, 0x00,
    0x0a, 0x49, 0x44, 0x41, 0x54, 0x78, 0x9c, 0x63, 0x00, 0x01, 0x00, 0x00, 0x05, 0x00, 0x01, 0x0d, 0x0a, 0x2d,
    0xb4, 0x00, 0x00, 0x00, 0x00, 0x49, 0x45, 0x4e, 0x44, 0xae, 0x42, 0x60, 0x82,
];

fn base64(bytes: &[u8]) -> String {
    const TABLE: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::new();
    for chunk in bytes.chunks(3) {
        let b = [chunk[0], *chunk.get(1).unwrap_or(&0), *chunk.get(2).unwrap_or(&0)];
        let n = ((b[0] as u32) << 16) | ((b[1] as u32) << 8) | b[2] as u32;
        out.push(TABLE[(n >> 18) as usize & 63] as char);
        out.push(TABLE[(n >> 12) as usize & 63] as char);
        out.push(if chunk.len() > 1 { TABLE[(n >> 6) as usize & 63] as char } else { '=' });
        out.push(if chunk.len() > 2 { TABLE[n as usize & 63] as char } else { '=' });
    }
    out
}

async fn first_bot(crew: &Crew) -> String {
    let state = crew.ok("GET", "/api/state", None).await;
    state["bots"][0]["id"].as_str().unwrap().to_string()
}

async fn upload(crew: &Crew, bot: &str, name: &str, bytes: &[u8]) -> Value {
    crew.ok("POST", &format!("/api/bots/{bot}/attach"), Some(json!({ "name": name, "data": base64(bytes) }))).await
}

#[tokio::test]
async fn upload_lands_in_the_workspace_and_is_served_back() {
    let crew = Crew::start("attach").await;
    let bot = first_bot(&crew).await;

    let saved = upload(&crew, &bot, "скрин шот экрана.png", PNG).await;
    let path = saved["path"].as_str().unwrap().to_string();
    let date = chrono_date();
    assert_eq!(path, format!("attachments/{date}/скрин шот экрана.png"));
    assert_eq!(saved["kind"], "image");
    assert_eq!(saved["size"].as_u64().unwrap() as usize, PNG.len());

    // Файл лежит там, где его найдёт бот своими инструментами.
    let on_disk = crew.data_dir.join("workspaces").join(&bot).join(&path);
    assert_eq!(std::fs::read(&on_disk).unwrap(), PNG);

    // Картинка отдаётся как картинка, иначе миниатюра в переписке не покажется.
    let res = crew
        .client
        .get(format!("{}/api/bots/{bot}/file/raw", crew.base))
        .query(&[("path", path.as_str())])
        .header("cookie", &crew.cookie)
        .send()
        .await
        .unwrap();
    assert_eq!(res.status().as_u16(), 200);
    assert_eq!(res.headers()["content-type"], "image/png");
    assert_eq!(res.bytes().await.unwrap().as_ref(), PNG);

    // Тот же файл ещё раз: второй не затирает первый.
    let again = upload(&crew, &bot, "скрин шот экрана.png", PNG).await;
    assert_ne!(again["path"], saved["path"]);
    assert_eq!(again["path"], format!("attachments/{date}/скрин шот экрана-2.png"));
}

fn chrono_date() -> String {
    chrono::Local::now().format("%Y-%m-%d").to_string()
}

#[tokio::test]
async fn oversize_empty_and_traversal_are_refused() {
    let crew = Crew::start("attach-bad").await;
    let bot = first_bot(&crew).await;
    let url = format!("/api/bots/{bot}/attach");

    // Больше лимита: ясное сообщение с размером, файл не создан.
    let big = vec![b'a'; 10 * 1024 * 1024 + 1];
    let (status, body) = crew.send("POST", &url, Some(json!({ "name": "big.txt", "data": base64(&big) }))).await;
    assert_eq!(status, 400, "{body}");
    assert!(body["error"].as_str().unwrap_or_default().contains("10 МБ"), "{body}");
    assert!(!crew.data_dir.join("workspaces").join(&bot).join("attachments").exists());

    // Ровно на лимите — проходит: потолок тела запроса выше, чем у обычного JSON.
    let limit = vec![b'a'; 10 * 1024 * 1024];
    let (status, _) = crew.send("POST", &url, Some(json!({ "name": "edge.txt", "data": base64(&limit) }))).await;
    assert_eq!(status, 201);

    // Пусто и не base64.
    let (status, _) = crew.send("POST", &url, Some(json!({ "name": "a.txt", "data": "" }))).await;
    assert_eq!(status, 400);
    let (status, _) = crew.send("POST", &url, Some(json!({ "name": "a.txt", "data": "не base64" }))).await;
    assert_eq!(status, 400);

    // Путь в имени: отказ, а не молчаливая обрезка. Снаружи воркспейса ничего не появилось.
    for name in ["../../побег.txt", "..\\..\\побег.txt", "a/b.txt", "C:\\x\\y.txt", ".."] {
        let (status, body) = crew.send("POST", &url, Some(json!({ "name": name, "data": base64(b"x") }))).await;
        assert_eq!(status, 400, "{name}: {body}");
    }
    assert!(!crew.data_dir.join("workspaces").join("побег.txt").exists());
    assert!(!crew.data_dir.join("побег.txt").exists());

    // Ссылка на вложение в сообщении: только внутри attachments/, и файл должен существовать.
    for path in ["../secret.txt", "attachments/../memory.md", "attachments/2000-01-01/нет.txt", "memory.md", "/etc/passwd"] {
        let (status, body) = crew
            .send("POST", &format!("/api/bots/{bot}/messages"), Some(json!({ "text": "вот", "attachments": [path] })))
            .await;
        assert_eq!(status, 400, "{path}: {body}");
    }
}

// ---- что видит модель ----------------------------------------------------------------------

#[derive(Clone, Default)]
struct Fake {
    calls: Arc<Mutex<Vec<Value>>>,
    /// Пока включено, модель отвечает медленно: бот остаётся занятым и сообщения встают в очередь.
    slow: Arc<Mutex<bool>>,
}

async fn completions(State(fake): State<Fake>, body: Bytes) -> Response {
    fake.calls.lock().unwrap().push(serde_json::from_slice(&body).unwrap_or_else(|_| json!({})));
    if *fake.slow.lock().unwrap() {
        tokio::time::sleep(std::time::Duration::from_secs(5)).await;
    }
    let text = format!(
        "data: {}\n\ndata: {}\n\ndata: [DONE]\n\n",
        json!({ "choices": [{ "index": 0, "delta": { "content": "принял" } }] }),
        json!({ "choices": [{ "index": 0, "delta": {}, "finish_reason": "stop" }] }),
    );
    Response::builder().header("content-type", "text/event-stream").body(Body::from(text)).unwrap()
}

async fn start_model(name: &str, vision: bool) -> (Crew, Fake, String) {
    let fake = Fake::default();
    let app = Router::new().route("/v1/chat/completions", post(completions)).with_state(fake.clone());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    let crew = Crew::start(name).await;
    crew.ok(
        "PUT",
        "/api/settings",
        Some(json!({
            "driver": "api", "model": "fake", "apiKey": "k", "vision": vision,
            "baseUrl": format!("http://127.0.0.1:{port}/v1"),
        })),
    )
    .await;
    crew.listen().await;
    let bot = first_bot(&crew).await;
    (crew, fake, bot)
}

/// Последнее сообщение человека в запросе к модели.
fn last_user(fake: &Fake) -> Value {
    let calls = fake.calls.lock().unwrap();
    let messages = calls.last().expect("модель не вызывали")["messages"].as_array().unwrap().clone();
    messages.into_iter().rev().find(|m| m["role"] == "user").unwrap()
}

#[tokio::test]
async fn model_gets_image_part_inlined_text_and_a_path() {
    let (crew, fake, bot) = start_model("attach-model", true).await;
    let png = upload(&crew, &bot, "кадр.png", PNG).await;
    let note = upload(&crew, &bot, "заметка.md", "# Список\nкупить молоко".as_bytes()).await;
    let pdf = upload(&crew, &bot, "отчёт.pdf", b"%PDF-1.4 not really").await;
    let refs = json!([png["path"], note["path"], pdf["path"]]);

    crew.ok("POST", &format!("/api/bots/{bot}/messages"), Some(json!({ "text": "посмотри", "attachments": refs }))).await;
    crew.wait_event("конец хода", |e| e["type"] == "turn_end").await;

    let content = last_user(&fake)["content"].clone();
    let parts = content.as_array().expect("с картинкой содержимое — список частей");
    let text = parts[0]["text"].as_str().unwrap();
    assert!(text.starts_with("посмотри"), "{text}");
    // Текстовый файл вставлен в сообщение, а не только назван.
    assert!(text.contains("купить молоко"), "{text}");
    // Остальное — путём.
    assert!(text.contains(pdf["path"].as_str().unwrap()), "{text}");
    assert!(!text.contains("not really"), "бинарник попал в промпт");
    // Картинка — настоящей частью запроса, байт в байт.
    let image = parts.iter().find(|p| p["type"] == "image_url").expect("нет части с картинкой");
    assert_eq!(image["image_url"]["url"], format!("data:image/png;base64,{}", base64(PNG)));

    // Сообщение хранит описание файлов, и окно рисует их по нему.
    let thread = crew.ok("GET", &format!("/api/bots/{bot}/thread?limit=10"), None).await;
    let user = thread["messages"].as_array().unwrap().iter().find(|m| m["role"] == "user").unwrap().clone();
    assert_eq!(user["text"], "посмотри");
    let kinds: Vec<&str> = user["attachments"].as_array().unwrap().iter().map(|a| a["kind"].as_str().unwrap()).collect();
    assert_eq!(kinds, ["image", "text", "file"]);
}

#[tokio::test]
async fn without_vision_the_image_goes_by_path_only() {
    let (crew, fake, bot) = start_model("attach-novision", false).await;
    let png = upload(&crew, &bot, "кадр.png", PNG).await;
    // Вложение без текста — тоже сообщение.
    crew.ok("POST", &format!("/api/bots/{bot}/messages"), Some(json!({ "text": "", "attachments": [png["path"]] }))).await;
    crew.wait_event("конец хода", |e| e["type"] == "turn_end").await;

    let content = last_user(&fake)["content"].clone();
    let text = content.as_str().expect("без картинок содержимое — строка");
    assert!(text.contains(png["path"].as_str().unwrap()), "{text}");
    assert!(!content.to_string().contains("image_url"));
}

#[tokio::test]
async fn long_text_file_is_cut_in_the_prompt() {
    let (crew, fake, bot) = start_model("attach-long", true).await;
    let long = "строка данных\n".repeat(5000);
    let file = upload(&crew, &bot, "журнал.log", long.as_bytes()).await;
    crew.ok("POST", &format!("/api/bots/{bot}/messages"), Some(json!({ "text": "что тут", "attachments": [file["path"]] }))).await;
    crew.wait_event("конец хода", |e| e["type"] == "turn_end").await;
    let text = last_user(&fake)["content"].as_str().unwrap().to_string();
    assert!(text.contains("файл обрезан"), "обрезка не отмечена");
    assert!(text.len() < 40 * 1024, "в промпт ушёл весь файл: {} байт", text.len());
}

#[tokio::test]
async fn group_attachment_reaches_the_answering_bot() {
    let (crew, fake, bot) = start_model("attach-room", true).await;
    let room = crew
        .ok("POST", "/api/rooms", Some(json!({ "name": "Совет", "botIds": [bot], "lead": bot, "mode": "lead" })))
        .await["room"]["id"]
        .as_str()
        .unwrap()
        .to_string();
    let saved = crew
        .ok("POST", &format!("/api/rooms/{room}/attach"), Some(json!({ "name": "кадр.png", "data": base64(PNG) })))
        .await;
    crew.ok(
        "POST",
        &format!("/api/rooms/{room}/messages"),
        Some(json!({ "text": "что на картинке", "attachments": [saved["path"]] })),
    )
    .await;
    crew.wait_event("конец хода группы", |e| e["type"] == "room_turn_end").await;

    let content = last_user(&fake)["content"].clone();
    let image = content.as_array().and_then(|p| p.iter().find(|x| x["type"] == "image_url")).expect("нет картинки в запросе");
    assert!(image["image_url"]["url"].as_str().unwrap().starts_with("data:image/png;base64,"));
    let thread = crew.ok("GET", &format!("/api/rooms/{room}/thread?limit=10"), None).await;
    let user = thread["messages"].as_array().unwrap().iter().find(|m| m["role"] == "user").unwrap().clone();
    assert_eq!(user["attachments"][0]["path"], saved["path"]);
}

async fn wait_turns(crew: &Crew, count: usize) {
    for _ in 0..500 {
        if crew.events.lock().unwrap().iter().filter(|e| e["type"] == "turn_end").count() >= count {
            return;
        }
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }
    panic!("не дождался {count}-го конца хода");
}

fn text_of(content: &Value) -> String {
    match content {
        Value::String(s) => s.clone(),
        Value::Array(parts) => parts.iter().filter_map(|p| p["text"].as_str()).collect::<Vec<_>>().join("
"),
        _ => String::new(),
    }
}

/// Все сообщения человека из последнего запроса к модели, по порядку.
fn user_texts(fake: &Fake) -> Vec<String> {
    let calls = fake.calls.lock().unwrap();
    calls.last().unwrap()["messages"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|m| m["role"] == "user")
        .map(|m| text_of(&m["content"]))
        .collect()
}

#[tokio::test]
async fn send_now_from_the_queue_keeps_the_file() {
    let (crew, fake, bot) = start_model("attach-queue", false).await;
    *fake.slow.lock().unwrap() = true;
    crew.say(&bot, "первое").await;
    // Дожидаемся, пока модель взяла запрос: бот точно занят.
    for _ in 0..100 {
        if !fake.calls.lock().unwrap().is_empty() {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }
    let file = upload(&crew, &bot, "список.txt", "купить хлеб".as_bytes()).await;
    // Вложение без текста в очереди: элемент не пустой.
    let queued = crew
        .ok("POST", &format!("/api/bots/{bot}/messages"), Some(json!({ "text": "", "attachments": [file["path"]] })))
        .await;
    assert_eq!(queued["queued"], true, "{queued}");
    let qid = queued["item"]["id"].as_str().unwrap().to_string();
    assert_eq!(queued["item"]["attachments"][0]["path"], file["path"]);

    // Стоп ставит очередь на паузу; бот свободен, элемент остаётся в очереди.
    *fake.slow.lock().unwrap() = false;
    crew.ok("POST", &format!("/api/bots/{bot}/stop"), None).await;
    wait_turns(&crew, 1).await;
    crew.ok("POST", &format!("/api/bots/{bot}/queue/{qid}/steer"), None).await;
    wait_turns(&crew, 2).await;

    let last = user_texts(&fake).pop().unwrap();
    assert!(last.contains("купить хлеб"), "файл потерялся при «сейчас»: {last}");
}

#[tokio::test]
async fn text_files_are_inlined_only_in_recent_messages_and_within_a_budget() {
    let (crew, fake, bot) = start_model("attach-budget", false).await;
    for i in 0..6 {
        let file = upload(&crew, &bot, &format!("файл{i}.txt"), format!("СОДЕРЖИМОЕ-{i}").as_bytes()).await;
        crew.ok("POST", &format!("/api/bots/{bot}/messages"), Some(json!({ "text": format!("m{i}"), "attachments": [file["path"]] }))).await;
        wait_turns(&crew, i + 1).await;
    }
    let texts = user_texts(&fake).join("
=====
");
    // Свежие сообщения несут содержимое, старые — только путь.
    assert!(texts.contains("СОДЕРЖИМОЕ-5") && texts.contains("СОДЕРЖИМОЕ-2"), "{texts}");
    assert!(!texts.contains("СОДЕРЖИМОЕ-0") && !texts.contains("СОДЕРЖИМОЕ-1"), "старые файлы всё ещё вставлены: {texts}");
    assert!(texts.contains("файл0.txt"), "у старого вложения пропал путь");

    // Три больших файла в одном сообщении: общий бюджет 48 КБ, а не 3 × 32.
    let big = "я".repeat(30_000);
    let mut refs = Vec::new();
    for i in 0..3 {
        refs.push(upload(&crew, &bot, &format!("большой{i}.txt"), big.as_bytes()).await["path"].clone());
    }
    crew.ok("POST", &format!("/api/bots/{bot}/messages"), Some(json!({ "text": "три", "attachments": refs }))).await;
    wait_turns(&crew, 7).await;
    let last = user_texts(&fake).pop().unwrap();
    assert!(last.len() < 52 * 1024, "бюджет текста превышен: {} байт", last.len());
    assert!(last.contains("файл обрезан") || last.contains("содержимое не вставлено"), "{last}");
}

#[tokio::test]
async fn same_name_uploads_in_parallel_never_overwrite_and_odd_names_are_fixed() {
    let crew = Crew::start("attach-race").await;
    let bot = first_bot(&crew).await;
    let bodies: Vec<String> = (0..6).map(|i| format!("n{i}")).collect();
    let sends = bodies.iter().map(|body| upload(&crew, &bot, "одинаковый.txt", body.as_bytes()));
    let saved = futures_util::future::join_all(sends).await;
    let mut paths: Vec<String> = saved.iter().map(|s| s["path"].as_str().unwrap().to_string()).collect();
    paths.sort();
    paths.dedup();
    assert_eq!(paths.len(), 6, "параллельные загрузки затёрли друг друга: {paths:?}");
    let mut bodies: Vec<String> = paths
        .iter()
        .map(|p| std::fs::read_to_string(crew.data_dir.join("workspaces").join(&bot).join(p)).unwrap())
        .collect();
    bodies.sort();
    assert_eq!(bodies, ["n0", "n1", "n2", "n3", "n4", "n5"]);

    // Имя устройства Windows и знаки направления письма.
    let con = upload(&crew, &bot, "CON.txt", b"x").await;
    assert!(con["path"].as_str().unwrap().ends_with("/_CON.txt"), "{con}");
    let rlo = upload(&crew, &bot, "фото\u{202E}.exe", b"x").await;
    assert!(rlo["path"].as_str().unwrap().ends_with("/фото.exe"), "{rlo}");
}

/// OpenAI-совместимый провайдер без картинок: на части `image_url` отвечает 400.
async fn picky(State(fake): State<Fake>, body: Bytes) -> Response {
    let payload: Value = serde_json::from_slice(&body).unwrap_or_else(|_| json!({}));
    let has_image = payload.to_string().contains("image_url");
    fake.calls.lock().unwrap().push(payload);
    if has_image {
        return Response::builder().status(400).body(Body::from("{\"error\":\"image input is not supported by this model\"}")).unwrap();
    }
    let text = format!(
        "data: {}

data: {}

data: [DONE]

",
        json!({ "choices": [{ "index": 0, "delta": { "content": "принял" } }] }),
        json!({ "choices": [{ "index": 0, "delta": {}, "finish_reason": "stop" }] }),
    );
    Response::builder().header("content-type", "text/event-stream").body(Body::from(text)).unwrap()
}

#[tokio::test]
async fn a_provider_that_rejects_images_gets_one_retry_with_a_note_and_is_remembered() {
    let fake = Fake::default();
    let app = Router::new().route("/v1/chat/completions", post(picky)).with_state(fake.clone());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    let crew = Crew::start("attach-picky").await;
    crew.ok(
        "PUT",
        "/api/settings",
        Some(json!({ "driver": "api", "model": "text-only", "apiKey": "k", "vision": true, "baseUrl": format!("http://127.0.0.1:{port}/v1") })),
    )
    .await;
    crew.listen().await;
    let bot = first_bot(&crew).await;

    let first = upload(&crew, &bot, "кадр.png", PNG).await;
    crew.ok("POST", &format!("/api/bots/{bot}/messages"), Some(json!({ "text": "что тут", "attachments": [first["path"]] }))).await;
    wait_turns(&crew, 1).await;
    {
        let calls = fake.calls.lock().unwrap();
        assert_eq!(calls.len(), 2, "ждал отказ и один повтор");
        assert!(calls[0].to_string().contains("image_url"));
        let retry = calls[1].to_string();
        assert!(!retry.contains("image_url"), "картинка осталась в повторе");
        assert!(retry.contains("картинка приложена файлом: attachments/"), "{retry}");
        assert!(retry.contains("эта модель её не видит"));
    }
    // Ход не провалился: ответ есть, ошибки нет.
    let thread = crew.ok("GET", &format!("/api/bots/{bot}/thread?limit=10"), None).await;
    let answer = thread["messages"].as_array().unwrap().iter().rev().find(|m| m["role"] == "assistant").unwrap().clone();
    assert_eq!(answer["text"], "принял");
    assert!(answer["error"].is_null() || answer["error"] == false);

    // Пара «провайдер и модель» запомнена: вторая картинка уходит сразу заметкой, без нового отказа.
    let second = upload(&crew, &bot, "ещё.png", PNG).await;
    crew.ok("POST", &format!("/api/bots/{bot}/messages"), Some(json!({ "text": "и это", "attachments": [second["path"]] }))).await;
    wait_turns(&crew, 2).await;
    let calls = fake.calls.lock().unwrap();
    assert_eq!(calls.len(), 3, "вторая картинка снова пробовалась с отказом");
    assert!(!calls[2].to_string().contains("image_url"));
}
