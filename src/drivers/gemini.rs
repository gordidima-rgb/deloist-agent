//! Драйвер Google Gemini: streamGenerateContent с alt=sse.
//! Системный промпт — в systemInstruction, инструменты — functionCall и functionResponse.

use std::collections::HashMap;
use std::sync::Mutex;

use futures_util::StreamExt;
use serde_json::{json, Value};
use tokio_util::sync::CancellationToken;

use super::{DriverEvent, Sink, ToolCall, Usage};
use crate::store::uid;

pub struct Request<'a> {
    pub model: &'a str,
    pub base_url: &'a str,
    pub api_key: &'a str,
    pub messages: &'a [Value],
    pub tools: Value,
    /// Только если человек задал её сам: «думающим» моделям Gemini лучше их собственная.
    pub temperature: Option<f64>,
}

const DEFAULT_BASE_URL: &str = "https://generativelanguage.googleapis.com";
/// Сколько подписей вызовов помнить. Нужны они в пределах хода, так что сотен хватает.
const MAX_SIGNATURES: usize = 512;

/// Подпись рассуждения при вызове инструмента. «Думающие» модели Gemini ждут её обратно вместе с
/// вызовом на следующем шаге хода и без неё отвечают ошибкой. В историю её не кладём: там формат
/// OpenAI, и чужое поле ушло бы другим провайдерам. Хватает памяти процесса: шаг идёт сразу за шагом.
static SIGNATURES: Mutex<Vec<(String, String)>> = Mutex::new(Vec::new());

fn remember_signature(call_id: &str, signature: &str) {
    let mut list = SIGNATURES.lock().unwrap();
    if list.len() >= MAX_SIGNATURES {
        list.remove(0);
    }
    list.push((call_id.to_string(), signature.to_string()));
}

fn signature_of(call_id: &str) -> Option<String> {
    SIGNATURES.lock().unwrap().iter().rev().find(|(id, _)| id == call_id).map(|(_, signature)| signature.clone())
}

/// Модель стоит в пути, версия — тоже: уже вписанные /v1 или /v1beta не дублируем.
fn endpoint(base_url: &str, model: &str) -> String {
    let base = base_url.trim().trim_end_matches('/');
    let root = if base.is_empty() { DEFAULT_BASE_URL } else { base };
    let root = if root.ends_with("/v1beta") || root.ends_with("/v1") {
        root.to_string()
    } else {
        format!("{root}/v1beta")
    };
    format!("{root}/models/{model}:streamGenerateContent?alt=sse")
}

/// Gemini ждёт args объектом: неразобранную строку показываем полем _raw, чтобы
/// модель увидела, что именно пришло, а провайдер не отверг запрос.
fn tool_input(raw: &Value) -> Value {
    match raw {
        Value::Object(_) => raw.clone(),
        Value::String(s) if s.trim().is_empty() => json!({}),
        Value::String(s) => match serde_json::from_str::<Value>(s) {
            Ok(Value::Object(map)) => Value::Object(map),
            Ok(other) => json!({ "_raw": other }),
            Err(_) => json!({ "_raw": s }),
        },
        _ => json!({}),
    }
}

fn text_part(text: &str) -> Option<Value> {
    if text.trim().is_empty() {
        None
    } else {
        Some(json!({ "text": text }))
    }
}

/// Часть истории OpenAI → часть Gemini. Картинка в истории лежит data-URL-ом:
/// Gemini принимает её только как inlineData, поэтому другие ссылки просто пропускаем.
fn part(part: &Value) -> Option<Value> {
    if let Some(text) = part["text"].as_str() {
        return text_part(text);
    }
    let url = part["image_url"]["url"].as_str()?;
    let (meta, data) = url.strip_prefix("data:")?.split_once(',')?;
    let mime_type = meta.strip_suffix(";base64")?;
    Some(json!({ "inlineData": { "mimeType": mime_type, "data": data } }))
}

fn content_parts(content: &Value) -> Vec<Value> {
    match content {
        Value::String(text) => text_part(text).into_iter().collect(),
        Value::Array(parts) => parts.iter().filter_map(part).collect(),
        _ => Vec::new(),
    }
}

/// Gemini ждёт чередования user/model: соседние сообщения одной роли склеиваем в один ход.
/// Заодно так собираются все functionResponse шага в одно сообщение user.
fn push_parts(contents: &mut Vec<Value>, role: &str, parts: Vec<Value>) {
    if parts.is_empty() {
        return;
    }
    if let Some(last) = contents.last_mut() {
        if last["role"] == role {
            if let Some(existing) = last["parts"].as_array_mut() {
                existing.extend(parts);
                return;
            }
        }
    }
    contents.push(json!({ "role": role, "parts": parts }));
}

/// OpenAI-история → системная строка и contents Gemini.
fn convert_contents(messages: &[Value]) -> (String, Vec<Value>) {
    let mut system = String::new();
    let mut contents: Vec<Value> = Vec::new();
    // У Gemini у вызовов нет id: ответ инструмента сопоставляем с объявлением по имени,
    // поэтому помним, какой id какую функцию вызывал в предыдущем ходе модели.
    let mut names: HashMap<String, String> = HashMap::new();
    for message in messages {
        match message["role"].as_str().unwrap_or_default() {
            "system" => {
                if let Some(text) = message["content"].as_str().filter(|text| !text.trim().is_empty()) {
                    if !system.is_empty() {
                        system.push_str("\n\n");
                    }
                    system.push_str(text);
                }
            }
            "user" => push_parts(&mut contents, "user", content_parts(&message["content"])),
            "assistant" => {
                let mut parts = content_parts(&message["content"]);
                for call in message["tool_calls"].as_array().into_iter().flatten() {
                    let name = call["function"]["name"].as_str().unwrap_or_default();
                    if name.is_empty() {
                        continue;
                    }
                    let id = call["id"].as_str().unwrap_or_default();
                    if !id.is_empty() {
                        names.insert(id.to_string(), name.to_string());
                    }
                    let mut part = json!({
                        "functionCall": { "name": name, "args": tool_input(&call["function"]["arguments"]) },
                    });
                    if let Some(signature) = signature_of(id) {
                        part["thoughtSignature"] = json!(signature);
                    }
                    parts.push(part);
                }
                push_parts(&mut contents, "model", parts);
            }
            "tool" => {
                let id = message["tool_call_id"].as_str().unwrap_or_default();
                // Без имени Gemini не поймёт, на что это ответ: такую запись лучше пропустить,
                // чем сорвать весь ход. В обычной истории имя всегда находится.
                let Some(name) = names.get(id) else { continue };
                let part = json!({
                    "functionResponse": {
                        "name": name,
                        "response": { "result": message["content"].as_str().unwrap_or_default() },
                    },
                });
                push_parts(&mut contents, "user", vec![part]);
            }
            _ => {}
        }
    }
    (system, contents)
}

/// Инструменты OpenAI `{type:function, function:{…}}` → объявления функций Gemini.
fn convert_tools(tools: &Value) -> Vec<Value> {
    let mut declarations = Vec::new();
    for tool in tools.as_array().into_iter().flatten() {
        let function = &tool["function"];
        let name = function["name"].as_str().unwrap_or_default();
        if name.is_empty() {
            continue;
        }
        let mut declaration = json!({ "name": name });
        if let Some(description) = function["description"].as_str().filter(|text| !text.is_empty()) {
            declaration["description"] = json!(description);
        }
        // Схема — обычный JSON Schema, в том числе от MCP-серверов с $schema, $ref и списками типов.
        // Поле `parameters` у Gemini — урезанный OpenAPI и на такие схемы отвечает 400,
        // а parametersJsonSchema принимает их как есть.
        if function["parameters"].is_object() {
            declaration["parametersJsonSchema"] = function["parameters"].clone();
        }
        declarations.push(declaration);
    }
    declarations
}

fn request_body(req: &Request<'_>) -> Value {
    let (system, contents) = convert_contents(req.messages);
    let mut body = json!({ "contents": contents });
    if let Some(temperature) = req.temperature {
        body["generationConfig"] = json!({ "temperature": temperature });
    }
    if !system.is_empty() {
        body["systemInstruction"] = json!({ "parts": [{ "text": system }] });
    }
    let declarations = convert_tools(&req.tools);
    if !declarations.is_empty() {
        body["tools"] = json!([{ "functionDeclarations": declarations }]);
    }
    body
}

/// Начало тела ошибки: провайдер иногда присылает страницу целиком, а в окно помещается строка.
fn short_text(text: &str, limit: usize) -> String {
    text.trim().chars().take(limit).collect()
}

/// Ответ не-200 и ошибка внутри потока в одном виде: `API <status>: <короткий текст>`.
/// Ключ сюда не попадает: он уходит только в заголовок запроса.
fn api_error(status: u16, body: &str) -> String {
    let message = serde_json::from_str::<Value>(body)
        .ok()
        .and_then(|parsed| parsed["error"]["message"].as_str().map(str::to_string))
        .unwrap_or_else(|| short_text(body, 600));
    format!("API {status}: {message}")
}

/// Gemini присылает ошибку прямо в потоке: HTTP-статуса у неё нет, но код нужен —
/// по нему решается, идти ли на запасную модель.
fn stream_error(error: &Value) -> String {
    let status = error["code"].as_u64().map_or(500, |code| code.min(u16::MAX as u64) as u16);
    format!("API {status}: {}", short_text(error["message"].as_str().unwrap_or("ошибка потока"), 300))
}

#[derive(Default)]
struct StreamState {
    prompt: i64,
    cached: i64,
    completion: i64,
    text: String,
    calls: Vec<ToolCall>,
    /// Причина, по которой ответ испорчен: фильтр безопасности, сломанный вызов и подобное.
    blocked: Option<String>,
    /// Ответ упёрся в предел длины.
    cut: bool,
    /// Пришла причина остановки: поток закончился, а не оборвался.
    finished: bool,
}

impl StreamState {
    fn report(&self, sink: Sink<'_>) {
        if self.prompt == 0 && self.completion == 0 {
            return;
        }
        // Из кэша не могло прийти больше, чем весь вход: иначе в окне будут проценты выше ста.
        let usage = Usage { prompt: self.prompt, completion: self.completion, cached: self.cached.min(self.prompt) };
        sink(DriverEvent::Usage(usage));
    }

    /// Обрабатывает один кусок ответа: и строку SSE, и обычный JSON без stream.
    fn handle(&mut self, event: &Value, sink: Sink<'_>) -> Result<(), String> {
        if let Some(error) = event.get("error").filter(|error| !error.is_null()) {
            return Err(stream_error(error));
        }
        if let Some(usage) = event.get("usageMetadata") {
            if let Some(prompt) = usage["promptTokenCount"].as_i64() {
                self.prompt = prompt.max(0);
            }
            // Рассуждение оплачивается отдельно от ответа: без него журнал и дневной предел занижены.
            let answer = usage["candidatesTokenCount"].as_i64().unwrap_or(0).max(0);
            let thoughts = usage["thoughtsTokenCount"].as_i64().unwrap_or(0).max(0);
            if answer + thoughts > 0 {
                self.completion = answer + thoughts;
            }
            if let Some(cached) = usage["cachedContentTokenCount"].as_i64() {
                self.cached = cached.max(0);
            }
            self.report(&mut *sink);
        }
        if let Some(reason) = event["promptFeedback"]["blockReason"].as_str() {
            self.blocked = Some(reason.to_string());
        }
        for candidate in event["candidates"].as_array().into_iter().flatten() {
            // STOP — обычное завершение, MAX_TOKENS — обрыв по длине; SAFETY, RECITATION,
            // MALFORMED_FUNCTION_CALL и прочее — ответ испорчен, даже если часть текста уже пришла.
            if let Some(reason) = candidate["finishReason"].as_str() {
                self.finished = true;
                match reason {
                    "STOP" | "FINISH_REASON_UNSPECIFIED" => {}
                    "MAX_TOKENS" => self.cut = true,
                    other => self.blocked = Some(other.to_string()),
                }
            }
            for part in candidate["content"]["parts"].as_array().into_iter().flatten() {
                // У «думающих» моделей часть текста — рассуждение: в ответ его не отдаём.
                if part["thought"].as_bool().unwrap_or(false) {
                    continue;
                }
                if let Some(text) = part["text"].as_str().filter(|text| !text.is_empty()) {
                    self.text.push_str(text);
                    sink(DriverEvent::Text(text.to_string()));
                }
                if let Some(call) = part.get("functionCall").filter(|call| call.is_object()) {
                    let name = call["name"].as_str().unwrap_or_default();
                    if name.is_empty() {
                        continue;
                    }
                    // Gemini не даёт id вызова: выдаём свой, уникальный на всю историю, а ответ
                    // инструмента сопоставляется по имени — см. convert_contents.
                    let id = uid("call");
                    if let Some(signature) = part["thoughtSignature"].as_str().filter(|text| !text.is_empty()) {
                        remember_signature(&id, signature);
                    }
                    self.calls.push(ToolCall { id, name: name.to_string(), args: tool_input(&call["args"]) });
                }
            }
        }
        Ok(())
    }

    /// Отдаёт накопленные вызовы. Испорченный, обрезанный или оборванный ответ — ошибка:
    /// тогда сработает и запасная модель, а полвызова инструмента не выполнится.
    fn finish(&mut self, sink: Sink<'_>) -> Result<(), String> {
        if let Some(reason) = &self.blocked {
            return Err(format!("Gemini: ответ остановлен, finishReason={reason}"));
        }
        let empty = self.calls.is_empty() && self.text.trim().is_empty();
        if self.cut && (empty || !self.calls.is_empty()) {
            return Err("Gemini: ответ упёрся в предел длины и оборвался".into());
        }
        if !self.finished && empty {
            return Err("Gemini: поток ответа оборвался".into());
        }
        for call in std::mem::take(&mut self.calls) {
            sink(DriverEvent::Tool(call));
        }
        Ok(())
    }
}

/// Ответ без stream:true: форма та же, только все части целиком.
fn emit_response(response: &Value, sink: Sink<'_>) -> Result<(), String> {
    let mut state = StreamState::default();
    state.handle(response, &mut *sink)?;
    state.finish(&mut *sink)
}

pub async fn run(req: Request<'_>, cancel: &CancellationToken, sink: Sink<'_>) -> Result<(), String> {
    if req.api_key.trim().is_empty() {
        return Err("нет ключа Gemini: впиши его в настройках или положи в .env как GEMINI_API_KEY".into());
    }
    if req.model.trim().is_empty() {
        return Err("не задана модель Gemini: впиши её в настройках или в профиле бота, например gemini-2.5-flash".into());
    }

    let url = endpoint(req.base_url, req.model.trim());
    let body = request_body(&req);
    let client = reqwest::Client::new();
    // Ключ — заголовком, а не в адресе: так он не попадает в журналы прокси и в тексты ошибок.
    let send = client
        .post(&url)
        .header("x-goog-api-key", req.api_key.trim())
        .header("content-type", "application/json")
        .header("user-agent", "deloist/0.2 (+local)")
        .body(body.to_string())
        .send();
    let res = tokio::select! {
        _ = cancel.cancelled() => return Err("остановлено".into()),
        res = send => res.map_err(|error| error.to_string())?,
    };

    let status = res.status().as_u16();
    if !res.status().is_success() {
        let detail = res.text().await.unwrap_or_default();
        return Err(api_error(status, &detail));
    }

    // Прокси иногда отвечает обычным JSON, не глядя на alt=sse: тогда читаем ответ целиком.
    let kind = res.headers().get("content-type").and_then(|value| value.to_str().ok()).unwrap_or_default();
    if !kind.contains("event-stream") {
        let raw = res.text().await.map_err(|error| error.to_string())?;
        let parsed: Value = serde_json::from_str(&raw).map_err(|error| error.to_string())?;
        return emit_response(&parsed, sink);
    }

    let mut stream = res.bytes_stream();
    let mut buf: Vec<u8> = Vec::new();
    let mut state = StreamState::default();
    'read: loop {
        let chunk = tokio::select! {
            _ = cancel.cancelled() => return Err("остановлено".into()),
            chunk = stream.next() => chunk,
        };
        let Some(chunk) = chunk else { break };
        buf.extend_from_slice(&chunk.map_err(|error| error.to_string())?);

        while let Some(nl) = buf.iter().position(|byte| *byte == b'\n') {
            let raw: Vec<u8> = buf.drain(..=nl).collect();
            let line = String::from_utf8_lossy(&raw);
            let Some(payload) = line.trim().strip_prefix("data:") else { continue };
            let payload = payload.trim();
            if payload == "[DONE]" {
                break 'read;
            }
            let Ok(event) = serde_json::from_str::<Value>(payload) else { continue };
            state.handle(&event, &mut *sink)?;
        }
    }
    state.finish(&mut *sink)
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    /// Разбор потока, разложенный по видам событий: так проверки читаются.
    #[derive(Default)]
    struct Parsed {
        text: String,
        calls: Vec<(String, String, Value)>,
        usage: Option<Usage>,
        error: Option<String>,
    }

    fn split(events: Vec<DriverEvent>) -> Parsed {
        let mut parsed = Parsed::default();
        for event in events {
            match event {
                DriverEvent::Text(chunk) => parsed.text.push_str(&chunk),
                DriverEvent::Tool(call) => parsed.calls.push((call.id, call.name, call.args)),
                DriverEvent::Usage(usage) => parsed.usage = Some(usage),
                DriverEvent::Activity(_) => {}
            }
        }
        parsed
    }

    /// Прогоняет SSE-текст через тот же разборщик, что и живой поток.
    fn drain(sse: &str) -> Parsed {
        let mut events = Vec::new();
        let mut state = StreamState::default();
        let mut error = None;
        {
            let mut sink = |event: DriverEvent| events.push(event);
            for line in sse.lines() {
                let Some(payload) = line.trim().strip_prefix("data:") else { continue };
                let payload = payload.trim();
                if payload == "[DONE]" {
                    break;
                }
                let Ok(event) = serde_json::from_str::<Value>(payload) else { continue };
                if let Err(message) = state.handle(&event, &mut sink) {
                    error = Some(message);
                    break;
                }
            }
            if error.is_none() {
                if let Err(message) = state.finish(&mut sink) {
                    error = Some(message);
                }
            }
        }
        let mut parsed = split(events);
        parsed.error = error;
        parsed
    }

    fn collect_response(response: &Value) -> Parsed {
        let mut events = Vec::new();
        let mut error = None;
        {
            let mut sink = |event: DriverEvent| events.push(event);
            let mut state = StreamState::default();
            if let Err(message) = state.handle(response, &mut sink) {
                error = Some(message);
            } else if let Err(message) = state.finish(&mut sink) {
                error = Some(message);
            }
        }
        let mut parsed = split(events);
        parsed.error = error;
        parsed
    }

    #[test]
    fn request_body_has_system_instruction_and_declarations() {
        let messages = vec![json!({ "role": "system", "content": "Ты — бот" }), json!({ "role": "user", "content": "привет" })];
        let tools = json!([{
            "type": "function",
            "function": { "name": "read_file", "description": "Читает файл", "parameters": { "type": "object", "properties": {} } },
        }]);
        let request = Request {
            model: " gemini-2.0-flash ",
            base_url: "",
            api_key: "AIza-secret",
            messages: &messages,
            tools,
            temperature: Some(0.3),
        };
        let body = request_body(&request);
        assert_eq!(body["systemInstruction"]["parts"][0]["text"], "Ты — бот");
        assert_eq!(body["contents"].as_array().map(Vec::len), Some(1));
        assert_eq!(body["contents"][0]["role"], "user");
        assert_eq!(body["tools"][0]["functionDeclarations"][0]["name"], "read_file");
        assert_eq!(body["generationConfig"]["temperature"], 0.3);
        let request = Request { temperature: None, ..request };
        assert!(request_body(&request).get("generationConfig").is_none(), "без своей температуры поле не шлём");
    }

    #[test]
    fn tools_become_function_declarations() {
        let tools = json!([{
            "type": "function",
            "function": {
                "name": "read_file",
                "description": "Читает файл",
                "parameters": { "type": "object", "properties": { "path": { "type": "string" } }, "required": ["path"] },
            },
        }]);
        let converted = json!([{ "functionDeclarations": convert_tools(&tools) }]);
        assert_eq!(converted[0]["functionDeclarations"][0]["name"], "read_file");
        assert_eq!(converted[0]["functionDeclarations"][0]["description"], "Читает файл");
        assert_eq!(converted[0]["functionDeclarations"][0]["parametersJsonSchema"]["properties"]["path"]["type"], "string");
        assert!(converted[0]["functionDeclarations"][0].get("parameters").is_none(), "поля взаимоисключающие");
        assert!(converted[0]["functionDeclarations"][0].get("function").is_none(), "формат OpenAI не должен протекать в запрос");
    }

    #[test]
    fn history_is_split_into_system_and_contents() {
        let messages = vec![
            json!({ "role": "system", "content": "Ты — бот" }),
            json!({ "role": "user", "content": "прочитай файл" }),
            json!({
                "role": "assistant",
                "content": "сейчас",
                "tool_calls": [{ "id": "call_1", "type": "function", "function": { "name": "read_file", "arguments": "{\"path\":\"a.txt\"}" } }],
            }),
            json!({ "role": "tool", "tool_call_id": "call_1", "content": "текст файла" }),
            json!({ "role": "user", "content": "спасибо" }),
        ];
        let (system, contents) = convert_contents(&messages);
        assert_eq!(system, "Ты — бот");
        assert_eq!(contents.len(), 3, "{contents:?}");
        assert_eq!(contents[0]["role"], "user");
        assert_eq!(contents[0]["parts"][0]["text"], "прочитай файл");
        assert_eq!(contents[1]["role"], "model");
        assert_eq!(contents[1]["parts"][0]["text"], "сейчас");
        assert_eq!(contents[1]["parts"][1]["functionCall"]["name"], "read_file");
        assert_eq!(contents[1]["parts"][1]["functionCall"]["args"]["path"], "a.txt");
        // Ответ инструмента нашёлся по имени: id у Gemini нет, а следующий текст человека
        // склеился в тот же ход user.
        assert_eq!(contents[2]["role"], "user");
        assert_eq!(contents[2]["parts"][0]["functionResponse"]["name"], "read_file");
        assert_eq!(contents[2]["parts"][0]["functionResponse"]["response"]["result"], "текст файла");
        assert_eq!(contents[2]["parts"][1]["text"], "спасибо");
    }

    #[test]
    fn base_url_gets_the_version_segment_once() {
        let path = ":streamGenerateContent?alt=sse";
        assert_eq!(
            endpoint("", "gemini-2.0-flash"),
            format!("https://generativelanguage.googleapis.com/v1beta/models/gemini-2.0-flash{path}")
        );
        assert_eq!(
            endpoint("https://generativelanguage.googleapis.com/", "m"),
            format!("https://generativelanguage.googleapis.com/v1beta/models/m{path}")
        );
        assert_eq!(
            endpoint("https://generativelanguage.googleapis.com/v1beta", "m"),
            format!("https://generativelanguage.googleapis.com/v1beta/models/m{path}")
        );
        assert_eq!(endpoint(" http://localhost:8080/v1/ ", "m"), format!("http://localhost:8080/v1/models/m{path}"));
    }

    #[test]
    fn sse_stream_yields_text_function_call_and_cached_usage() {
        let sse = r#"data: {"candidates":[{"content":{"parts":[{"text":"Думаю…","thought":true}],"role":"model"}}],"usageMetadata":{"promptTokenCount":8,"cachedContentTokenCount":3}}

data: {"candidates":[{"content":{"parts":[{"text":"Сейчас "}],"role":"model"}}]}

data: {"candidates":[{"content":{"parts":[{"text":"посмотрю."}],"role":"model"}}]}

data: {"candidates":[{"content":{"parts":[{"functionCall":{"name":"read_file","args":{"path":"a.txt"}}}],"role":"model"},"finishReason":"STOP"}],"usageMetadata":{"promptTokenCount":8,"candidatesTokenCount":12,"cachedContentTokenCount":3}}
"#;
        let parsed = drain(sse);
        assert!(parsed.error.is_none(), "{:?}", parsed.error);
        assert_eq!(parsed.text, "Сейчас посмотрю.", "рассуждение модели в ответ не попадает");
        assert_eq!(parsed.calls.len(), 1);
        assert!(parsed.calls[0].0.starts_with("call"), "Gemini не даёт id — выдаём свой");
        assert_eq!(parsed.calls[0].1, "read_file");
        assert_eq!(parsed.calls[0].2, json!({ "path": "a.txt" }));
        assert_eq!(parsed.usage, Some(Usage { prompt: 8, completion: 12, cached: 3 }));
    }

    #[test]
    fn function_call_without_arguments_becomes_empty_object() {
        let sse = r#"data: {"candidates":[{"content":{"parts":[{"functionCall":{"name":"clock"}}]},"finishReason":"STOP"}]}
"#;
        let parsed = drain(sse);
        assert!(parsed.error.is_none());
        assert_eq!(parsed.calls[0].2, json!({}), "без args — пустой объект, а не null");
    }

    #[test]
    fn response_without_stream_is_read_whole() {
        let response = json!({
            "candidates": [{
                "content": { "parts": [{ "text": "Готово" }, { "functionCall": { "name": "write_file", "args": { "path": "a.txt" } } }] },
                "finishReason": "STOP",
            }],
            "usageMetadata": { "promptTokenCount": 5, "candidatesTokenCount": 6, "cachedContentTokenCount": 2 },
        });
        let parsed = collect_response(&response);
        assert_eq!(parsed.text, "Готово");
        assert_eq!(parsed.calls[0].1, "write_file");
        assert_eq!(parsed.usage, Some(Usage { prompt: 5, completion: 6, cached: 2 }));
    }

    #[test]
    fn unauthorized_error_is_short_and_has_no_key() {
        let request = Request {
            model: "gemini-2.0-flash",
            base_url: "",
            api_key: "AIza-secret",
            messages: &[],
            tools: json!([]),
            temperature: None,
        };
        let message = api_error(401, r#"{"error":{"code":401,"message":"API key not valid. Please pass a valid API key.","status":"UNAUTHENTICATED"}}"#);
        assert_eq!(message, "API 401: API key not valid. Please pass a valid API key.");
        assert!(!message.contains(request.api_key), "ключ не должен попадать в текст ошибки");
    }

    #[test]
    fn stream_error_keeps_status() {
        let mut events = Vec::new();
        let mut state = StreamState::default();
        let error = {
            let mut sink = |event: DriverEvent| events.push(event);
            state.handle(&json!({ "error": { "code": 429, "message": "quota exceeded" } }), &mut sink)
        };
        assert_eq!(error.err().as_deref(), Some("API 429: quota exceeded"));
    }

    #[test]
    fn blocked_empty_answer_is_an_error() {
        let mut events = Vec::new();
        let mut state = StreamState::default();
        let error = {
            let mut sink = |event: DriverEvent| events.push(event);
            assert!(state.handle(&json!({ "candidates": [{ "content": { "parts": [] }, "finishReason": "SAFETY" }] }), &mut sink).is_ok());
            state.finish(&mut sink)
        };
        assert_eq!(error.err().as_deref(), Some("Gemini: ответ остановлен, finishReason=SAFETY"));
    }

    #[test]
    fn a_spoiled_answer_after_some_text_is_still_an_error() {
        let sse = r#"data: {"candidates":[{"content":{"parts":[{"text":"Начинаю "}]}}]}
data: {"candidates":[{"content":{"parts":[]},"finishReason":"MALFORMED_FUNCTION_CALL"}]}
"#;
        assert_eq!(drain(sse).error.as_deref(), Some("Gemini: ответ остановлен, finishReason=MALFORMED_FUNCTION_CALL"));
        let cut = r#"data: {"candidates":[{"content":{"parts":[{"functionCall":{"name":"write_file","args":{"path":"a"}}}]},"finishReason":"MAX_TOKENS"}]}
"#;
        let parsed = drain(cut);
        assert!(parsed.calls.is_empty());
        assert!(parsed.error.is_some());
        let broken = r#"data: {"candidates":[{"content":{"parts":[]}}]}
"#;
        assert_eq!(drain(broken).error.as_deref(), Some("Gemini: поток ответа оборвался"));
    }

    #[test]
    fn calls_get_unique_ids_and_carry_their_signature_back() {
        let sse = r#"data: {"candidates":[{"content":{"parts":[{"functionCall":{"name":"clock","args":{}},"thoughtSignature":"sig-1"}]},"finishReason":"STOP"}],"usageMetadata":{"promptTokenCount":4,"candidatesTokenCount":2,"thoughtsTokenCount":5}}
"#;
        let first = drain(sse);
        let second = drain(sse);
        assert_ne!(first.calls[0].0, second.calls[0].0, "id не повторяются от шага к шагу");
        assert_eq!(first.usage.unwrap().completion, 7, "рассуждение тоже оплачивается");
        let id = first.calls[0].0.clone();
        let messages = vec![
            json!({ "role": "user", "content": "который час" }),
            json!({ "role": "assistant", "content": null, "tool_calls": [{ "id": id, "type": "function", "function": { "name": "clock", "arguments": "{}" } }] }),
            json!({ "role": "tool", "tool_call_id": id, "content": "12:00" }),
        ];
        let (_, contents) = convert_contents(&messages);
        assert_eq!(contents[1]["parts"][0]["thoughtSignature"], "sig-1");
    }
}
