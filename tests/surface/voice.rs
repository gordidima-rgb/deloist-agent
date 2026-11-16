//! Голос, снимок экрана и правка выделенного без Windows: настоящий exe в режиме `--server`,
//! фейковый сайт распознавания и фейковая модель. Горячие клавиши «нажимаются» отладочным
//! крючком (`CREW_TEST_HOTKEYS=1`): настоящих сочетаний, SendInput и снимков стола здесь нет.

use crate::common;

use std::sync::{Arc, Mutex};

use axum::body::{Body, Bytes};
use axum::extract::{Query, State};
use axum::http::HeaderMap;
use axum::response::{IntoResponse, Response};
use axum::routing::post;
use axum::Router;
use base64::Engine;
use common::{temp_data_dir, Crew};
use serde_json::{json, Value};

#[cfg(deloist_private)]
const TOKEN: &str = "device-token-123";

/// Что увидел сайт распознавания.
#[cfg(deloist_private)]
#[derive(Clone, Debug)]
struct Heard {
    query: std::collections::HashMap<String, String>,
    auth: String,
    kind: String,
    bytes: usize,
}

#[cfg(deloist_private)]
#[derive(Clone, Default)]
struct Site {
    heard: Arc<Mutex<Vec<Heard>>>,
    /// Код ответа сайта: 200 — распознал.
    answer: Arc<Mutex<u16>>,
}

#[cfg(deloist_private)]
async fn transcribe(State(site): State<Site>, Query(query): Query<std::collections::HashMap<String, String>>, headers: HeaderMap, body: Bytes) -> Response {
    let text = |name: &str| headers.get(name).and_then(|v| v.to_str().ok()).unwrap_or_default().to_string();
    site.heard.lock().unwrap().push(Heard { query, auth: text("authorization"), kind: text("content-type"), bytes: body.len() });
    let code = *site.answer.lock().unwrap();
    if code == 200 {
        axum::Json(json!({ "text": "  привет, бот  " })).into_response()
    } else {
        (axum::http::StatusCode::from_u16(code).unwrap(), axum::Json(json!({ "detail": "rate_limited" }))).into_response()
    }
}

#[cfg(deloist_private)]
async fn site() -> (String, Site) {
    let site = Site::default();
    *site.answer.lock().unwrap() = 200;
    let app = Router::new().route("/api/agent/voice/transcribe", post(transcribe)).with_state(site.clone());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    (format!("http://127.0.0.1:{port}/api/agent/voice/transcribe"), site)
}

#[cfg(deloist_private)]
/// Папка данных с уже выполненным входом в аккаунт.
fn signed_in_dir(name: &str) -> std::path::PathBuf {
    let dir = temp_data_dir(name);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("secrets.json"), json!({ "accountToken": TOKEN, "accountEmail": "me@deloist.ru" }).to_string()).unwrap();
    dir
}

#[cfg(deloist_private)]
fn pcm(bytes: usize) -> String {
    base64::engine::general_purpose::STANDARD.encode(vec![0u8; bytes])
}

#[cfg(deloist_private)]
#[tokio::test(flavor = "multi_thread")]
async fn speech_goes_to_the_site_with_the_device_token_and_the_page_gets_only_text() {
    let (url, site) = site().await;
    let crew = Crew::start_at(signed_in_dir("voice-proxy"), &[("CREW_TEST_VOICE_URL", &url), ("CREW_TEST_CLOUD_VOICE", "1")]).await;
    // По умолчанию речь распознаёт компьютер; сайт — по выбору в настройках, и только с флагом облачного голоса.
    crew.ok("PUT", "/api/voice/settings", Some(json!({ "engine": "deloist" }))).await;

    let answer = crew.ok("POST", "/api/voice/transcribe", Some(json!({ "pcm": pcm(32000), "rate": 16000, "lang": "ru" }))).await;
    assert_eq!(answer, json!({ "text": "привет, бот" }));
    let heard = site.heard.lock().unwrap().clone();
    assert_eq!(heard.len(), 1);
    assert_eq!(heard[0].auth, format!("Bearer {TOKEN}"), "токен устройства, как у других вызовов агента");
    assert_eq!(heard[0].kind, "application/octet-stream");
    assert_eq!(heard[0].bytes, 32000, "сырой PCM, без base64 и заголовка WAV");
    assert_eq!(heard[0].query.get("rate").map(String::as_str), Some("16000"));
    assert_eq!(heard[0].query.get("lang").map(String::as_str), Some("ru"));
    assert!(!answer.to_string().contains(TOKEN));

    // Обрывок короче 0,3 с и чужая частота до сайта не доходят: это платный запрос.
    let (status, body) = crew.send("POST", "/api/voice/transcribe", Some(json!({ "pcm": pcm(2000) }))).await;
    assert_eq!(status, 422, "{body}");
    let (status, _) = crew.send("POST", "/api/voice/transcribe", Some(json!({ "pcm": pcm(32000), "rate": 44100 }))).await;
    assert_eq!(status, 422);
    assert_eq!(site.heard.lock().unwrap().len(), 1);

    // Сайт ответил «слишком часто» — человек видит понятный текст с тем же кодом.
    *site.answer.lock().unwrap() = 429;
    let (status, body) = crew.send("POST", "/api/voice/transcribe", Some(json!({ "pcm": pcm(32000) }))).await;
    assert_eq!(status, 429);
    assert!(body["error"].as_str().unwrap().contains("Подожди"), "{body}");

    // Горячие клавиши без отладочного крючка не нажать.
    let (status, _) = crew.send("POST", "/api/hotkeys/test/fire", Some(json!({ "action": "voice" }))).await;
    assert_eq!(status, 404);
}

#[cfg(deloist_private)]
#[tokio::test(flavor = "multi_thread")]
async fn without_an_account_nothing_is_sent() {
    let (url, site) = site().await;
    let crew = Crew::start_with_env("voice-anon", &[("CREW_TEST_VOICE_URL", &url), ("CREW_TEST_CLOUD_VOICE", "1")]).await;
    // По умолчанию речь распознаёт компьютер; сайт — по выбору в настройках, и только с флагом облачного голоса.
    crew.ok("PUT", "/api/voice/settings", Some(json!({ "engine": "deloist" }))).await;
    let (status, body) = crew.send("POST", "/api/voice/transcribe", Some(json!({ "pcm": pcm(32000) }))).await;
    assert_eq!(status, 401);
    assert!(body["error"].as_str().unwrap().contains("Войди"), "{body}");
    assert!(site.heard.lock().unwrap().is_empty());
}

#[tokio::test(flavor = "multi_thread")]
async fn the_pages_and_the_hotkey_settings_are_served() {
    let crew = Crew::start("voice-settings").await;
    for (path, needle) in [("/voice.js", "PcmRecorder"), ("/snap.js", "/api/selection/"), ("/snap.html", "/snap.js")] {
        let res = crew.client.get(format!("{}{path}", crew.base)).header("cookie", &crew.cookie).send().await.unwrap();
        assert_eq!(res.status().as_u16(), 200, "{path}");
        assert!(res.text().await.unwrap().contains(needle), "{path}");
    }

    let config = crew.ok("GET", "/api/hotkeys", None).await;
    assert_eq!(config["voice"], json!({ "enabled": true, "keys": "Ctrl + Space" }));
    assert_eq!(config["snap"]["keys"], "Alt + S");
    assert_eq!(config["edit"]["keys"], "Alt + E");
    assert_eq!(config["sendNow"], false);
    assert_eq!(config["defaults"]["snap"]["keys"], "Alt + S", "для кнопки «Сбросить»");
    // Без окна программы сочетаний нет, и настройки честно это показывают.
    assert_eq!(config["status"]["voice"], "unavailable");

    let config = crew.ok("PUT", "/api/hotkeys", Some(json!({ "snap": { "keys": "alt+ctrl+x" }, "edit": { "enabled": false }, "voice": { "keys": "f19" }, "sendNow": true }))).await;
    assert_eq!(config["snap"]["keys"], "Ctrl + Alt + X");
    assert_eq!(config["voice"]["keys"], "F19", "одна F-клавиша годится");
    assert_eq!(config["edit"]["enabled"], false);
    assert_eq!(config["sendNow"], true);
    assert_eq!(config["rejected"], json!({}));

    // Хранится в обычных настройках и видно окну и доку вместе с остальными.
    let saved: Value = serde_json::from_str(&std::fs::read_to_string(crew.data_dir.join("state.json")).unwrap()).unwrap();
    assert_eq!(saved["settings"]["hotkeys"]["snap"]["keys"], "Ctrl + Alt + X");
    let state = crew.ok("GET", "/api/state", None).await;
    assert_eq!(state["settings"]["hotkeys"]["voice"]["keys"], "F19");
    // Обычная правка настроек сочетания не трогает: их проверяет только /api/hotkeys.
    crew.ok("PUT", "/api/settings", Some(json!({ "hotkeys": { "voice": { "keys": "Ctrl+C" } } }))).await;
    assert_eq!(crew.ok("GET", "/api/hotkeys", None).await["voice"]["keys"], "F19");

    for bad in ["Q", "Shift+Q", "F5", "F12", "Alt+Space", "Ctrl+C", "Alt+F4", "Alt+Tab", "Win+L", "Ctrl+Alt+Del"] {
        let (status, body) = crew.send("PUT", "/api/hotkeys", Some(json!({ "voice": { "keys": bad } }))).await;
        assert_eq!(status, 400, "{bad}: {body}");
    }
    assert_eq!(crew.ok("GET", "/api/hotkeys", None).await["voice"]["keys"], "F19");
}

#[tokio::test(flavor = "multi_thread")]
async fn hotkey_settings_survive_a_restart() {
    let mut crew = Crew::start("voice-settings-restart").await;
    crew.ok("PUT", "/api/hotkeys", Some(json!({ "edit": { "keys": "Ctrl+Shift+E" }, "snap": { "enabled": false } }))).await;
    crew.keep_data();
    let dir = crew.data_dir.clone();
    drop(crew);
    let crew = Crew::start_at(dir, &[]).await;
    let config = crew.ok("GET", "/api/hotkeys", None).await;
    assert_eq!(config["edit"]["keys"], "Ctrl + Shift + E");
    assert_eq!(config["snap"]["enabled"], false);
    assert_eq!(config["voice"]["keys"], "Ctrl + Space");
}

#[tokio::test(flavor = "multi_thread")]
async fn hotkeys_open_the_last_bot_in_the_dock_or_wait_for_the_main_window() {
    let crew = Crew::start_with_env("voice-hotkeys", &[("CREW_TEST_HOTKEYS", "1")]).await;
    crew.listen().await;
    let state = crew.ok("GET", "/api/state", None).await;
    let second = state["bots"][1]["id"].as_str().unwrap().to_string();
    // Последним говорили со вторым ботом.
    crew.say(&second, "привет").await;
    crew.wait_event("ответ", |e| e["type"] == "turn_end" && e["botId"] == second.as_str()).await;

    crew.ok("POST", "/api/hotkeys/test/fire", Some(json!({ "action": "voice" }))).await;
    let event = crew.wait_event("голос", |e| e["type"] == "voice_hotkey").await;
    assert_eq!(event["botId"], second.as_str());
    assert_eq!(event["target"], "dock");

    // Снимок ложится во вложения того же бота и приходит доку событием.
    let fired = crew.ok("POST", "/api/hotkeys/test/fire", Some(json!({ "action": "snap" }))).await;
    let event = crew.wait_event("снимок", |e| e["type"] == "snap").await;
    // Снимок — обычное вложение: путь, имя, размер и вид, как у файла из скрепки.
    let attachment = &event["attachment"];
    assert_eq!(attachment["kind"], "image");
    assert!(attachment["name"].as_str().unwrap().starts_with("screen-"));
    let path = attachment["path"].as_str().unwrap();
    assert_eq!(fired["path"], path);
    assert!(path.starts_with("attachments/") && path.ends_with(".png"), "{path}");
    let file = crew.data_dir.join("workspaces").join(&second).join(path);
    let png = std::fs::read(&file).unwrap();
    assert_eq!(&png[..8], b"\x89PNG\r\n\x1a\n");
    assert_eq!(u32::from_be_bytes(png[16..20].try_into().unwrap()), 64, "ширина из IHDR");
    assert_eq!(u32::from_be_bytes(png[20..24].try_into().unwrap()), 48, "высота из IHDR");
    // Окно читает снимок обычным путём к файлам бота.
    let raw = crew.client.get(format!("{}/api/bots/{second}/file/raw?path={path}", crew.base)).header("cookie", &crew.cookie).send().await.unwrap();
    assert_eq!(raw.bytes().await.unwrap().as_ref(), png.as_slice());
    // Док отправляет снимок вложением, как скрепка: в истории он — вложение сообщения, а не путь в тексте.
    crew.clear_events();
    crew.ok("POST", &format!("/api/bots/{second}/messages"), Some(json!({ "text": "что тут за ошибка", "attachments": [path] }))).await;
    // Бот мог ещё заканчивать прошлый ход: сообщение тогда ждёт в очереди. Ждём его в истории.
    let thread_url = format!("/api/bots/{second}/thread");
    let mut mine = Value::Null;
    for _ in 0..100 {
        let thread = crew.ok("GET", &thread_url, None).await;
        let messages = thread.get("messages").unwrap_or(&thread).as_array().cloned().unwrap_or_default();
        if let Some(found) = messages.iter().rev().find(|m| m["role"] == "user" && m["text"] == "что тут за ошибка") {
            mine = found.clone();
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
    assert_eq!(mine["attachments"][0]["path"], path, "снимок — вложение сообщения: {mine}");

    // Дока нет: действие ждёт главное окно и отдаётся один раз.
    crew.ok("PUT", "/api/settings", Some(json!({ "dockEnabled": false }))).await;
    crew.clear_events();
    crew.ok("POST", "/api/hotkeys/test/fire", Some(json!({ "action": "voice" }))).await;
    let event = crew.wait_event("голос в окне", |e| e["type"] == "voice_hotkey").await;
    assert_eq!(event["target"], "main");
    let pending = crew.ok("POST", "/api/voice/pending", None).await;
    assert_eq!(pending["event"]["type"], "voice_hotkey");
    assert_eq!(pending["event"]["botId"], second.as_str());
    assert!(crew.ok("POST", "/api/voice/pending", None).await["event"].is_null());
}

#[tokio::test(flavor = "multi_thread")]
async fn voice_goes_to_the_chosen_bot_or_to_the_last_one() {
    let crew = Crew::start_with_env("voice-target", &[("CREW_TEST_HOTKEYS", "1")]).await;
    crew.listen().await;
    let state = crew.ok("GET", "/api/state", None).await;
    let first = state["bots"][0]["id"].as_str().unwrap().to_string();
    let second = state["bots"][1]["id"].as_str().unwrap().to_string();
    crew.say(&second, "привет").await;
    crew.wait_event("ответ", |e| e["type"] == "turn_end" && e["botId"] == second.as_str()).await;
    assert_eq!(crew.ok("GET", "/api/hotkeys", None).await["voiceTarget"], "last", "по умолчанию — последнему");
    assert_eq!(crew.ok("POST", "/api/voice/target", None).await["botId"], second.as_str());

    // Человек выбрал первого: голос с сочетания, микрофон на доке и снимок уходят ему.
    let config = crew.ok("PUT", "/api/hotkeys", Some(json!({ "voiceTarget": first }))).await;
    assert_eq!(config["voiceTarget"], first.as_str());
    assert_eq!(crew.ok("POST", "/api/voice/target", None).await["botId"], first.as_str());
    crew.clear_events();
    crew.ok("POST", "/api/hotkeys/test/fire", Some(json!({ "action": "voice" }))).await;
    assert_eq!(crew.wait_event("голос", |e| e["type"] == "voice_hotkey").await["botId"], first.as_str());
    crew.ok("POST", "/api/hotkeys/test/fire", Some(json!({ "action": "snap" }))).await;
    assert_eq!(crew.wait_event("снимок", |e| e["type"] == "snap").await["botId"], first.as_str());

    let (status, _) = crew.send("PUT", "/api/hotkeys", Some(json!({ "voiceTarget": "bot_nope" }))).await;
    assert_eq!(status, 400, "несуществующий бот");
    let (status, _) = crew.send("PUT", "/api/hotkeys", Some(json!({ "voiceTarget": "" }))).await;
    assert_eq!(status, 400);
    crew.ok("PUT", "/api/hotkeys", Some(json!({ "voiceTarget": "last" }))).await;
    assert_eq!(crew.ok("POST", "/api/voice/target", None).await["botId"], second.as_str());
}

#[tokio::test(flavor = "multi_thread")]
async fn voice_state_reaches_the_windows_but_not_programs_with_the_agent_key() {
    use futures_util::StreamExt;
    let crew = Crew::start_with_env("voice-state", &[("CREW_TEST_HOTKEYS", "1")]).await;
    crew.listen().await;
    let bot = crew.ok("GET", "/api/state", None).await["bots"][0]["id"].as_str().unwrap().to_string();

    // Программа с ключом подписана на тот же поток событий.
    let runtime: Value = serde_json::from_str(&std::fs::read_to_string(crew.data_dir.join("runtime.json")).unwrap()).unwrap();
    let token = runtime["token"].as_str().unwrap().to_string();
    let res = crew.client.get(format!("{}/api/events", crew.base)).bearer_auth(&token).send().await.unwrap();
    assert_eq!(res.status().as_u16(), 200);
    let agent_seen = Arc::new(Mutex::new(String::new()));
    {
        let agent_seen = agent_seen.clone();
        tokio::spawn(async move {
            let mut stream = res.bytes_stream();
            while let Some(Ok(chunk)) = stream.next().await {
                agent_seen.lock().unwrap().push_str(&String::from_utf8_lossy(&chunk));
            }
        });
    }

    let first = crew.ok("POST", "/api/voice/state", Some(json!({ "state": "recording", "botId": bot, "level": 0.42 }))).await;
    assert_eq!(first["sent"], true);
    // Смена шага проходит всегда (частоту уровня проверяет модульный тест: здесь она зависела бы от скорости машины).
    assert_eq!(crew.ok("POST", "/api/voice/state", Some(json!({ "state": "transcribing", "botId": bot }))).await["sent"], true);
    let (status, _) = crew.send("POST", "/api/voice/state", Some(json!({ "state": "dancing", "botId": bot }))).await;
    assert_eq!(status, 400);
    let (status, _) = crew.send("POST", "/api/voice/state", Some(json!({ "state": "done", "botId": "bot_nope" }))).await;
    assert_eq!(status, 404);

    let recording = crew.wait_event("запись", |e| e["type"] == "voice_state" && e["state"] == "recording").await;
    assert_eq!(recording["botId"], bot.as_str());
    assert_eq!(recording["level"], 0.42);
    crew.wait_event("распознаю", |e| e["type"] == "voice_state" && e["state"] == "transcribing").await;
    assert_eq!(crew.count_events(|e| e["type"] == "voice_state"), 2);
    assert!(crew.find_event(|e| e["type"] == "voice_state" && e.get("text").is_some()).is_none(), "текста записи в событиях нет");

    // Поток программы жив (обычные события доходят), но шагов голоса в нём нет.
    crew.ok("POST", "/api/hotkeys/test/fire", Some(json!({ "action": "voice" }))).await;
    common::wait_until("событие у программы", 10, || {
        let agent_seen = agent_seen.clone();
        async move { agent_seen.lock().unwrap().contains("voice_hotkey") }
    })
    .await;
    assert!(!agent_seen.lock().unwrap().contains("voice_state"), "уровень микрофона ушёл программе с ключом");
}

// --- правка выделенного через фейковую модель -----------------------------

#[derive(Clone, Default)]
struct Model {
    calls: Arc<Mutex<Vec<Value>>>,
}

async fn completions(State(model): State<Model>, body: Bytes) -> Response {
    let payload: Value = serde_json::from_slice(&body).unwrap_or_else(|_| json!({}));
    model.calls.lock().unwrap().push(payload);
    // Модель обернула ответ в Markdown и кавычки — программа это снимет.
    let chunks = [
        json!({ "choices": [{ "index": 0, "delta": { "role": "assistant", "content": "```\n«Добрый день, коллеги!\n" } }] }),
        json!({ "choices": [{ "index": 0, "delta": { "content": "Как ваши дела?»\n```" } }] }),
        json!({ "choices": [{ "index": 0, "delta": {}, "finish_reason": "stop" }] }),
    ];
    let mut text: String = chunks.iter().map(|c| format!("data: {c}\n\n")).collect();
    text.push_str("data: [DONE]\n\n");
    Response::builder().header("content-type", "text/event-stream").body(Body::from(text)).unwrap()
}

#[tokio::test(flavor = "multi_thread")]
async fn the_selected_text_is_rewritten_by_the_service_model_and_handed_back_for_paste() {
    let model = Model::default();
    let app = Router::new().route("/v1/chat/completions", post(completions)).with_state(model.clone());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });

    let crew = Crew::start_with_env("voice-edit", &[("CREW_TEST_HOTKEYS", "1")]).await;
    crew.ok("PUT", "/api/settings", Some(json!({ "driver": "api", "model": "fake-model", "baseUrl": format!("http://127.0.0.1:{port}/v1"), "apiKey": "test-key" }))).await;

    let selected = "здарова всем\r\nкак дела";
    let fired = crew.ok("POST", "/api/hotkeys/test/fire", Some(json!({ "action": "edit", "text": selected }))).await;
    let id = fired["id"].as_str().unwrap().to_string();

    // Подсказка видит длину, но не сам текст.
    let current = crew.ok("GET", "/api/selection/current", None).await;
    assert_eq!(current["id"], id.as_str());
    assert_eq!(current["chars"], selected.chars().count());
    assert!(current["notice"].is_null());
    assert!(!current.to_string().contains("здарова"));

    let (status, _) = crew.send("POST", &format!("/api/selection/{id}/rewrite"), Some(json!({ "instruction": "  " }))).await;
    assert_eq!(status, 400, "пустая просьба");
    let (status, _) = crew.send("POST", "/api/selection/nope/rewrite", Some(json!({ "instruction": "x" }))).await;
    assert_eq!(status, 404, "чужая правка");

    let rewritten = crew.ok("POST", &format!("/api/selection/{id}/rewrite"), Some(json!({ "instruction": "сделай вежливее" }))).await;
    assert_eq!(rewritten["ok"], true);
    let calls = model.calls.lock().unwrap().clone();
    assert_eq!(calls.len(), 1, "один служебный вызов");
    let messages = calls[0]["messages"].as_array().unwrap();
    assert!(messages[0]["content"].as_str().unwrap().contains("только новый текст"));
    let task = messages[1]["content"].as_str().unwrap();
    assert!(task.contains("Просьба: сделай вежливее"), "{task}");
    assert!(task.contains(selected), "{task}");
    assert!(calls[0].get("tools").is_none_or(|t| t.as_array().is_none_or(Vec::is_empty)), "без инструментов");

    // Вставка в проверке не жмёт Ctrl+V, а возвращает текст. Переводы строк как в выделенном.
    let applied = crew.ok("POST", &format!("/api/selection/{id}/apply"), None).await;
    assert_eq!(applied["ok"], true);
    assert_eq!(applied["pasted"], "Добрый день, коллеги!\r\nКак ваши дела?");
    let (status, _) = crew.send("POST", &format!("/api/selection/{id}/apply"), None).await;
    assert_eq!(status, 404, "вставка один раз");
    assert!(crew.ok("GET", "/api/selection/current", None).await["id"].is_null());

    // Вставка не удалась (UI Automation не ответила): правка жива, из подсказки можно повторить.
    let fired = crew.ok("POST", "/api/hotkeys/test/fire", Some(json!({ "action": "edit", "text": "привет", "pasteFails": "unsure" }))).await;
    let id = fired["id"].as_str().unwrap().to_string();
    crew.ok("POST", &format!("/api/selection/{id}/rewrite"), Some(json!({ "instruction": "вежливее" }))).await;
    let failed = crew.ok("POST", &format!("/api/selection/{id}/apply"), None).await;
    assert_eq!(failed["ok"], false);
    assert_eq!(failed["reason"], "unsure", "не «поле пароля», а «не удалось проверить поле»");
    assert_eq!(failed["text"], "Добрый день, коллеги!\nКак ваши дела?", "текст показан, чтобы его скопировать");
    assert_eq!(crew.ok("GET", "/api/selection/current", None).await["id"], id.as_str(), "правка не закрыта");
    let again = crew.ok("POST", &format!("/api/selection/{id}/apply"), None).await;
    assert_eq!(again["reason"], "unsure", "повтор доходит до вставки, а не до «нечего вставлять»");
    crew.ok("POST", &format!("/api/selection/{id}/cancel"), None).await;
    assert!(crew.ok("GET", "/api/selection/current", None).await["id"].is_null());
}
