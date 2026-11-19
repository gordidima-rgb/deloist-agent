//! GigaChat (Сбер). Похож на OpenAI, но со своими отличиями:
//! - ключ авторизации меняется на токен доступа на 30 минут (`/api/v2/oauth`);
//! - инструменты называются `functions`, ответ — `function_call` с аргументами-объектом, по одному за шаг;
//! - результат инструмента — сообщение `role: function` с JSON в `content`;
//! - сертификаты серверов выпущены Минцифры: его корневой сертификат лежит в `assets/`;
//! - картинки не лежат в сообщении: файл загружается в `POST /files` (multipart, `purpose=general`),
//!   а сообщение несёт его номер в `attachments`.

use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};
use std::time::Duration;

use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use tokio_util::sync::CancellationToken;

use super::{DriverEvent, Sink, ToolCall, Usage};

const OAUTH_URL: &str = "https://ngw.devices.sberbank.ru:9443/api/v2/oauth";
const API_URL: &str = "https://gigachat.devices.sberbank.ru/api/v1";
const SCOPE: &str = "GIGACHAT_API_PERS";
/// Корневой сертификат Минцифры (Russian Trusted Root CA), SHA-256
/// D2:6D:2D:02:31:B7:C3:9F:92:CC:73:85:12:BA:54:10:35:19:E4:40:5D:68:B5:BD:70:3E:97:88:CA:8E:CF:31.
const RUSSIAN_ROOT_CA: &[u8] = include_bytes!("../../assets/russian_trusted_root_ca.pem");

/// Модели GigaChat для выбора на экране провайдеров.
pub const MODELS: &[(&str, &str)] = &[("GigaChat-2", "GigaChat 2 Lite"), ("GigaChat-2-Pro", "GigaChat 2 Pro"), ("GigaChat-2-Max", "GigaChat 2 Max")];

fn client() -> Result<reqwest::Client, String> {
    let ca = reqwest::Certificate::from_pem(RUSSIAN_ROOT_CA).map_err(|e| e.to_string())?;
    reqwest::Client::builder()
        .add_root_certificate(ca)
        .connect_timeout(Duration::from_secs(15))
        .timeout(Duration::from_secs(180))
        .build()
        .map_err(|e| e.to_string())
}

fn tokens() -> &'static Mutex<HashMap<String, (String, i64)>> {
    static TOKENS: OnceLock<Mutex<HashMap<String, (String, i64)>>> = OnceLock::new();
    TOKENS.get_or_init(|| Mutex::new(HashMap::new()))
}

fn uuid4() -> String {
    let bytes: [u8; 16] = rand::random();
    let hex: String = bytes.iter().map(|b| format!("{b:02x}")).collect();
    format!("{}-{}-4{}-a{}-{}", &hex[0..8], &hex[8..12], &hex[13..16], &hex[17..20], &hex[20..32])
}

/// Токен доступа по ключу авторизации. Живёт 30 минут: держим его в памяти и берём новый заранее.
async fn access_token(client: &reqwest::Client, auth_key: &str) -> Result<String, String> {
    let cache_key: String = Sha256::digest(auth_key.as_bytes()).iter().map(|b| format!("{b:02x}")).collect();
    let now = crate::store::now();
    if let Some((token, expires)) = tokens().lock().unwrap().get(&cache_key).cloned() {
        if expires - now > 60_000 {
            return Ok(token);
        }
    }
    let response = client
        .post(OAUTH_URL)
        .header("authorization", format!("Basic {}", auth_key.trim()))
        .header("rquid", uuid4())
        .header("accept", "application/json")
        .form(&[("scope", SCOPE)])
        .send()
        .await
        .map_err(|e| format!("GigaChat: не получил токен: {}", e.without_url()))?;
    let status = response.status().as_u16();
    let body: Value = response.json().await.unwrap_or(Value::Null);
    if status != 200 {
        let why = body["message"].as_str().unwrap_or("ключ не подошёл");
        return Err(format!("API {status}: GigaChat не принял ключ авторизации: {why}"));
    }
    let token = body["access_token"].as_str().ok_or("GigaChat: нет токена в ответе")?.to_string();
    let expires = body["expires_at"].as_i64().unwrap_or(now + 25 * 60_000);
    tokens().lock().unwrap().insert(cache_key, (token.clone(), expires));
    Ok(token)
}

/// Проверка ключа и список моделей — для экрана провайдеров.
pub async fn models(auth_key: &str) -> Result<Vec<Value>, String> {
    let client = client()?;
    let token = access_token(&client, auth_key).await?;
    let response = client.get(format!("{API_URL}/models")).bearer_auth(token).send().await.map_err(|e| e.without_url().to_string())?;
    let status = response.status().as_u16();
    if status != 200 {
        return Err(format!("API {status}: GigaChat не отдал список моделей"));
    }
    let body: Value = response.json().await.map_err(|e| e.to_string())?;
    let list: Vec<Value> = body["data"]
        .as_array()
        .map(|d| d.iter().filter_map(|m| m["id"].as_str()).map(|id| json!({ "id": id, "name": id })).collect())
        .unwrap_or_default();
    if list.is_empty() {
        return Ok(MODELS.iter().map(|(id, name)| json!({ "id": id, "name": name })).collect());
    }
    Ok(list)
}

const FILES_MAX: usize = 512;

fn files() -> &'static Mutex<HashMap<String, String>> {
    static FILES: OnceLock<Mutex<HashMap<String, String>>> = OnceLock::new();
    FILES.get_or_init(|| Mutex::new(HashMap::new()))
}

fn hex_sha(bytes: &[u8]) -> String {
    Sha256::digest(bytes).iter().map(|b| format!("{b:02x}")).collect()
}

/// Картинки из сообщений человека: (отпечаток data-адреса, тип, байты).
fn images_of(messages: &[Value]) -> Vec<(String, String, Vec<u8>)> {
    let mut out = Vec::new();
    for message in messages.iter().filter(|m| m["role"] == "user") {
        for part in message["content"].as_array().into_iter().flatten() {
            let Some(url) = part["image_url"]["url"].as_str() else { continue };
            let Some((head, data)) = url.strip_prefix("data:").and_then(|r| r.split_once(',')) else { continue };
            let mime = head.split(';').next().unwrap_or("image/png").to_string();
            out.push((hex_sha(url.as_bytes()), mime, crate::browser_tools::base64_decode(data)));
        }
    }
    out
}

/// Загружает картинки в хранилище GigaChat и возвращает номера файлов по отпечатку. Номер помним по ключу и
/// содержимому: тот же снимок в следующих шагах хода не загружаем заново. Не загрузилось — картинки просто нет в ответе.
async fn upload_images(client: &reqwest::Client, base: &str, token: &str, cache_key: &str, messages: &[Value]) -> HashMap<String, String> {
    let mut ids = HashMap::new();
    for (hash, mime, bytes) in images_of(messages) {
        let key = format!("{cache_key}:{hash}");
        if let Some(id) = files().lock().unwrap().get(&key).cloned() {
            ids.insert(hash, id);
            continue;
        }
        let extension = mime.rsplit('/').next().unwrap_or("png");
        let boundary = format!("deloist{}", uuid4());
        let mut body: Vec<u8> = Vec::new();
        body.extend(format!("--{boundary}\r\nContent-Disposition: form-data; name=\"file\"; filename=\"image.{extension}\"\r\nContent-Type: {mime}\r\n\r\n").as_bytes());
        body.extend(&bytes);
        body.extend(format!("\r\n--{boundary}\r\nContent-Disposition: form-data; name=\"purpose\"\r\n\r\ngeneral\r\n--{boundary}--\r\n").as_bytes());
        let sent = client
            .post(format!("{base}/files"))
            .bearer_auth(token)
            .header("content-type", format!("multipart/form-data; boundary={boundary}"))
            .body(body)
            .send()
            .await;
        let Ok(response) = sent else { continue };
        if !response.status().is_success() {
            continue;
        }
        let data: Value = response.json().await.unwrap_or(Value::Null);
        if let Some(id) = data["id"].as_str() {
            let mut files = files().lock().unwrap();
            // Таблица живёт до конца процесса: без предела она росла с каждой новой картинкой (аудит 19.11.2026).
            // Сброшенный номер ничего не ломает: картинку просто загрузят ещё раз.
            if files.len() >= FILES_MAX {
                files.clear();
            }
            files.insert(key, id.to_string());
            drop(files);
            ids.insert(hash, id.to_string());
        }
    }
    ids
}

fn text_of(content: &Value) -> String {
    match content {
        Value::String(text) => text.clone(),
        Value::Array(parts) => parts.iter().filter_map(|p| p["text"].as_str()).collect::<Vec<_>>().join("\n"),
        _ => String::new(),
    }
}

/// Сообщения OpenAI в сообщения GigaChat.
pub(crate) fn convert(messages: &[Value], files: &HashMap<String, String>) -> Vec<Value> {
    let mut out: Vec<Value> = Vec::new();
    let mut system: Vec<String> = Vec::new();
    // Имя инструмента по номеру вызова: в ответе GigaChat ждёт имя функции, а не номер.
    let mut names: HashMap<String, String> = HashMap::new();
    for message in messages {
        match message["role"].as_str().unwrap_or_default() {
            "system" => system.push(text_of(&message["content"])),
            "assistant" => {
                let calls = message["tool_calls"].as_array().cloned().unwrap_or_default();
                let text = text_of(&message["content"]);
                if calls.is_empty() {
                    out.push(json!({ "role": "assistant", "content": text }));
                }
                for call in calls {
                    let name = call["function"]["name"].as_str().unwrap_or_default().to_string();
                    let raw = &call["function"]["arguments"];
                    let args = match raw {
                        Value::String(s) => serde_json::from_str(s).unwrap_or_else(|_| json!({})),
                        Value::Object(_) => raw.clone(),
                        _ => json!({}),
                    };
                    if let Some(id) = call["id"].as_str() {
                        names.insert(id.to_string(), name.clone());
                    }
                    out.push(json!({ "role": "assistant", "content": text, "function_call": { "name": name, "arguments": args } }));
                }
            }
            "tool" => {
                let id = message["tool_call_id"].as_str().unwrap_or_default();
                let name = names.get(id).cloned().unwrap_or_default();
                let result = json!({ "result": text_of(&message["content"]) }).to_string();
                out.push(json!({ "role": "function", "name": name, "content": result }));
            }
            _ => {
                let mut text = text_of(&message["content"]);
                let mut attachments: Vec<String> = Vec::new();
                for part in message["content"].as_array().into_iter().flatten() {
                    let Some(url) = part["image_url"]["url"].as_str() else { continue };
                    match files.get(&hex_sha(url.as_bytes())) {
                        Some(id) => attachments.push(id.clone()),
                        None => text.push_str("
[картинка не передана: GigaChat не принял файл]"),
                    }
                }
                let mut entry = json!({ "role": "user", "content": text });
                if !attachments.is_empty() {
                    entry["attachments"] = json!(attachments);
                }
                out.push(entry);
            }
        }
    }
    if !system.is_empty() {
        out.insert(0, json!({ "role": "system", "content": system.join("\n\n") }));
    }
    out
}

fn functions(tools: &Value) -> Vec<Value> {
    tools
        .as_array()
        .map(|t| t.iter().filter_map(|t| t.get("function").cloned()).collect())
        .unwrap_or_default()
}

/// Тело запроса к модели: картинки сначала уезжают файлами, потом их номера идут в `attachments`.
async fn build_body(client: &reqwest::Client, base: &str, token: &str, cache_key: &str, model: &str, messages: &[Value], tools: &Value) -> Value {
    let ids = upload_images(client, base, token, cache_key, messages).await;
    let mut body = json!({ "model": model, "messages": convert(messages, &ids), "stream": false });
    let functions = functions(tools);
    if !functions.is_empty() {
        body["functions"] = json!(functions);
        body["function_call"] = json!("auto");
    }
    body
}

pub async fn run(
    auth_key: &str,
    model: &str,
    messages: &[Value],
    tools: &Value,
    cancel: &CancellationToken,
    sink: Sink<'_>,
) -> Result<(), String> {
    if auth_key.trim().is_empty() {
        return Err("нет ключа авторизации GigaChat: впиши его в «Подключить свою ИИ»".into());
    }
    let client = client()?;
    let token = access_token(&client, auth_key).await?;
    let cache_key = hex_sha(auth_key.as_bytes());
    let body = build_body(&client, API_URL, &token, &cache_key, model, messages, tools).await;
    let send = client.post(format!("{API_URL}/chat/completions")).bearer_auth(token).json(&body).send();
    let response = tokio::select! {
        _ = cancel.cancelled() => return Err("остановлено".into()),
        r = send => r.map_err(|e| format!("GigaChat не ответил: {}", e.without_url()))?,
    };
    let status = response.status().as_u16();
    let text = response.text().await.unwrap_or_default();
    if status != 200 {
        let short: String = text.chars().take(600).collect();
        return Err(format!("API {status}: {short}"));
    }
    let data: Value = serde_json::from_str(&text).map_err(|e| format!("GigaChat: непонятный ответ: {e}"))?;
    let message = &data["choices"][0]["message"];
    if let Some(content) = message["content"].as_str().filter(|c| !c.is_empty()) {
        sink(DriverEvent::Text(content.to_string()));
    }
    if let Some(call) = message.get("function_call").filter(|c| c.is_object()) {
        let args = match &call["arguments"] {
            Value::String(s) => serde_json::from_str(s).unwrap_or_else(|_| json!({})),
            other if other.is_object() => other.clone(),
            _ => json!({}),
        };
        sink(DriverEvent::Tool(ToolCall {
            id: crate::store::uid("call"),
            name: call["name"].as_str().unwrap_or_default().to_string(),
            args,
        }));
    }
    let usage = &data["usage"];
    if usage.is_object() {
        sink(DriverEvent::Usage(Usage {
            prompt: usage["prompt_tokens"].as_i64().unwrap_or(0),
            completion: usage["completion_tokens"].as_i64().unwrap_or(0),
            cached: usage["precached_prompt_tokens"].as_i64().unwrap_or(0),
        }));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tool_rounds_become_function_messages() {
        let messages = vec![
            json!({ "role": "system", "content": "ты бот" }),
            json!({ "role": "user", "content": "который час" }),
            json!({ "role": "assistant", "content": null, "tool_calls": [{ "id": "c1", "type": "function", "function": { "name": "clock", "arguments": "{\"tz\":\"msk\"}" } }] }),
            json!({ "role": "tool", "tool_call_id": "c1", "content": "12:00" }),
        ];
        let out = convert(&messages, &HashMap::new());
        assert_eq!(out[0]["role"], "system");
        assert_eq!(out[2]["function_call"]["arguments"]["tz"], "msk");
        assert_eq!(out[3]["role"], "function");
        assert_eq!(out[3]["name"], "clock");
        assert_eq!(serde_json::from_str::<Value>(out[3]["content"].as_str().unwrap()).unwrap()["result"], "12:00");
    }

    #[derive(Clone, Default)]
    struct Files {
        uploads: std::sync::Arc<Mutex<Vec<(String, usize)>>>,
    }

    async fn files_endpoint(axum::extract::State(files): axum::extract::State<Files>, headers: axum::http::HeaderMap, body: axum::body::Bytes) -> axum::Json<Value> {
        let kind = headers["content-type"].to_str().unwrap_or_default().to_string();
        files.uploads.lock().unwrap().push((kind, body.len()));
        axum::Json(json!({ "id": "file-123", "purpose": "general" }))
    }

    #[tokio::test]
    async fn an_image_is_uploaded_once_and_sent_by_file_id() {
        let files = Files::default();
        let app = axum::Router::new().route("/files", axum::routing::post(files_endpoint)).with_state(files.clone());
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://127.0.0.1:{}", listener.local_addr().unwrap().port());
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        let picture = format!("data:image/png;base64,{}", crate::browser_tools::base64_encode(b"not really a png"));
        let messages = vec![
            json!({ "role": "system", "content": "s" }),
            json!({ "role": "user", "content": [
                { "type": "text", "text": "что на картинке" },
                { "type": "image_url", "image_url": { "url": picture } },
            ] }),
        ];
        let client = reqwest::Client::new();
        let body = build_body(&client, &base, "tok", "key-a", "GigaChat-2", &messages, &json!([])).await;
        let user = &body["messages"][1];
        assert_eq!(user["content"], "что на картинке");
        assert_eq!(user["attachments"], json!(["file-123"]));
        assert!(!body.to_string().contains("data:image"), "data-адрес ушёл в сообщение");
        // Тот же снимок на следующем шаге хода: файл уже загружен.
        build_body(&client, &base, "tok", "key-a", "GigaChat-2", &messages, &json!([])).await;
        let uploads = files.uploads.lock().unwrap().clone();
        assert_eq!(uploads.len(), 1, "снимок загружен повторно: {uploads:?}");
        assert!(uploads[0].0.starts_with("multipart/form-data; boundary="));
    }

    #[test]
    fn a_failed_upload_leaves_a_note_instead_of_the_picture() {
        let messages = vec![json!({ "role": "user", "content": [
            { "type": "text", "text": "смотри" },
            { "type": "image_url", "image_url": { "url": "data:image/png;base64,AAAA" } },
        ] })];
        let out = convert(&messages, &HashMap::new());
        assert!(out[0]["content"].as_str().unwrap().contains("картинка не передана"));
        assert!(out[0].get("attachments").is_none());
    }

    #[test]
    fn the_root_certificate_parses() {
        assert!(reqwest::Certificate::from_pem(RUSSIAN_ROOT_CA).is_ok());
    }

    #[test]
    fn rquid_is_a_uuid() {
        let id = uuid4();
        assert_eq!(id.len(), 36);
        assert_eq!(id.as_bytes()[14], b'4');
    }
}
