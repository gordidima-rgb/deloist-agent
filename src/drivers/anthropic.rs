//! Драйвер Anthropic Messages API: POST /v1/messages со stream:true.
//! Системный промпт едет отдельным полем, инструменты — блоками tool_use и tool_result.

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
}

/// Версия API: без этого заголовка Anthropic отвечает 400 на любой запрос.
const ANTHROPIC_VERSION: &str = "2023-06-01";
const DEFAULT_BASE_URL: &str = "https://api.anthropic.com";
/// Потолок ответа. Рассуждение у нынешних моделей включено само и считается в этот же предел,
/// поэтому с запасом: длинный write_file не должен обрываться посреди файла.
const MAX_TOKENS: i64 = 64_000;

/// Адрес из настроек: пустой — официальный, а уже вписанный /v1 не дублируем.
fn endpoint(base_url: &str) -> String {
    let base = base_url.trim().trim_end_matches('/');
    let root = if base.is_empty() { DEFAULT_BASE_URL } else { base };
    if root.ends_with("/v1") {
        format!("{root}/messages")
    } else {
        format!("{root}/v1/messages")
    }
}

/// Аргументы инструмента приходят строкой JSON: история собрана в OpenAI-формате.
fn parse_args(raw: &Value) -> Value {
    match raw {
        Value::Object(_) => raw.clone(),
        Value::String(s) if s.is_empty() => json!({}),
        Value::String(s) => serde_json::from_str(s).unwrap_or_else(|_| json!({ "_raw": s })),
        _ => json!({}),
    }
}

/// Anthropic принимает в tool_use только объект: неразобранную строку показываем полем _raw,
/// чтобы модель увидела, что именно пришло, а провайдер не отверг запрос.
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

fn text_block(text: &str) -> Option<Value> {
    if text.trim().is_empty() {
        None
    } else {
        Some(json!({ "type": "text", "text": text }))
    }
}

/// Часть истории OpenAI → блок Anthropic. Картинка в истории лежит data-URL-ом:
/// Anthropic принимает её только как base64, поэтому другие ссылки просто пропускаем.
fn part_block(part: &Value) -> Option<Value> {
    if let Some(text) = part["text"].as_str() {
        return text_block(text);
    }
    let url = part["image_url"]["url"].as_str()?;
    let (meta, data) = url.strip_prefix("data:")?.split_once(',')?;
    let media_type = meta.strip_suffix(";base64")?;
    Some(json!({
        "type": "image",
        "source": { "type": "base64", "media_type": media_type, "data": data },
    }))
}

fn content_blocks(content: &Value) -> Vec<Value> {
    match content {
        Value::String(text) => text_block(text).into_iter().collect(),
        Value::Array(parts) => parts.iter().filter_map(part_block).collect(),
        _ => Vec::new(),
    }
}

/// Anthropic требует чередования ролей: соседние сообщения одной роли склеиваем в один ход.
/// Заодно так собираются все tool_result шага в одно сообщение человека, как ждёт API.
fn push_blocks(messages: &mut Vec<Value>, role: &str, blocks: Vec<Value>) {
    if blocks.is_empty() {
        return;
    }
    if let Some(last) = messages.last_mut() {
        if last["role"] == role {
            if let Some(content) = last["content"].as_array_mut() {
                content.extend(blocks);
                return;
            }
        }
    }
    messages.push(json!({ "role": role, "content": blocks }));
}

/// OpenAI-история → системная строка и сообщения Anthropic.
fn convert_history(messages: &[Value]) -> (String, Vec<Value>) {
    let mut system = String::new();
    let mut out: Vec<Value> = Vec::new();
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
            "user" => push_blocks(&mut out, "user", content_blocks(&message["content"])),
            "assistant" => {
                let mut blocks = content_blocks(&message["content"]);
                for call in message["tool_calls"].as_array().into_iter().flatten() {
                    let name = call["function"]["name"].as_str().unwrap_or_default();
                    if name.is_empty() {
                        continue;
                    }
                    let id = call["id"].as_str().filter(|id| !id.is_empty()).map_or_else(|| uid("call"), str::to_string);
                    blocks.push(json!({
                        "type": "tool_use",
                        "id": id,
                        "name": name,
                        "input": tool_input(&call["function"]["arguments"]),
                    }));
                }
                push_blocks(&mut out, "assistant", blocks);
            }
            "tool" => {
                let block = json!({
                    "type": "tool_result",
                    "tool_use_id": message["tool_call_id"].as_str().unwrap_or_default(),
                    "content": message["content"].as_str().unwrap_or_default(),
                });
                push_blocks(&mut out, "user", vec![block]);
            }
            _ => {}
        }
    }
    (system, out)
}

/// Инструменты OpenAI `{type:function, function:{…}}` → Anthropic `{name, description, input_schema}`.
fn convert_tools(tools: &Value) -> Value {
    let mut converted = Vec::new();
    for tool in tools.as_array().into_iter().flatten() {
        let function = &tool["function"];
        let name = function["name"].as_str().unwrap_or_default();
        if name.is_empty() {
            continue;
        }
        let mut item = json!({ "name": name });
        if let Some(description) = function["description"].as_str().filter(|text| !text.is_empty()) {
            item["description"] = json!(description);
        }
        // Anthropic ждёт JSON Schema в input_schema; без схемы — пустой объект, а не null.
        item["input_schema"] = match &function["parameters"] {
            schema @ Value::Object(_) => schema.clone(),
            _ => json!({ "type": "object", "properties": {} }),
        };
        converted.push(item);
    }
    Value::Array(converted)
}

fn request_body(req: &Request<'_>) -> Value {
    let (system, messages) = convert_history(req.messages);
    // temperature не шлём: нынешние модели отвечают на него 400. Кэш включаем целиком для запроса:
    // без отметки Anthropic его не ведёт, и каждый шаг платил бы за весь вход полную цену.
    let mut body = json!({
        "model": req.model.trim(),
        "max_tokens": MAX_TOKENS,
        "messages": messages,
        "stream": true,
        "cache_control": { "type": "ephemeral" },
    });
    if !system.is_empty() {
        body["system"] = json!(system);
    }
    let tools = convert_tools(&req.tools);
    if tools.as_array().is_some_and(|list| !list.is_empty()) {
        body["tools"] = tools;
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

/// Anthropic умеет присылать ошибку прямо в потоке: HTTP-статуса у неё нет, но код нужен —
/// по нему решается, идти ли на запасную модель.
fn stream_error(error: &Value) -> String {
    let status = match error["type"].as_str().unwrap_or_default() {
        "invalid_request_error" => 400,
        "authentication_error" => 401,
        "permission_error" => 403,
        "not_found_error" => 404,
        "rate_limit_error" => 429,
        "overloaded_error" => 529,
        _ => 500,
    };
    format!("API {status}: {}", short_text(error["message"].as_str().unwrap_or("ошибка потока"), 300))
}

/// Вход в кэше Anthropic считает отдельно: `input_tokens` его не включает.
fn read_cached(usage: &Value) -> i64 {
    usage["cache_read_input_tokens"].as_i64().unwrap_or(0).max(0)
}

/// В `Usage.prompt` нужен весь вход: свежие токены, прочитанные из кэша и записанные в кэш.
fn input_tokens(usage: &Value) -> i64 {
    let fresh = usage["input_tokens"].as_i64().unwrap_or(0).max(0);
    let created = usage["cache_creation_input_tokens"].as_i64().unwrap_or(0).max(0);
    fresh + read_cached(usage) + created
}

#[derive(Default)]
struct ToolBlock {
    id: String,
    name: String,
    json: String,
    /// Готовый input из content_block_start, если провайдер прислал его целиком.
    input: Value,
}

/// Собранный вызов инструмента: пустой input — это `{}`, а не null.
fn take_tool(block: &mut ToolBlock) -> Option<ToolCall> {
    if block.name.is_empty() {
        return None;
    }
    let block = std::mem::take(block);
    let args = if block.json.trim().is_empty() {
        if block.input.is_null() {
            json!({})
        } else {
            block.input
        }
    } else {
        parse_args(&Value::String(block.json))
    };
    let id = if block.id.is_empty() { uid("call") } else { block.id };
    Some(ToolCall { id, name: block.name, args })
}

fn emit_tool(block: &mut ToolBlock, sink: Sink<'_>) {
    if let Some(call) = take_tool(block) {
        sink(DriverEvent::Tool(call));
    }
}

#[derive(Default)]
struct StreamState {
    prompt: i64,
    cached: i64,
    completion: i64,
    /// Блоки по индексу события: текстовые остаются пустыми, tool_use копят JSON.
    blocks: Vec<ToolBlock>,
    /// Почему модель остановилась: end_turn, tool_use, max_tokens, refusal.
    stop_reason: String,
    /// Подробности отказа: категория из stop_details.
    refusal: String,
    /// Пришёл message_stop: поток закончился как положено, а не оборвался.
    finished: bool,
    /// Что-то уже отдано человеку: текст или вызов.
    produced: bool,
}

impl StreamState {
    fn slot(&mut self, index: usize) -> &mut ToolBlock {
        if self.blocks.len() <= index {
            self.blocks.resize_with(index + 1, ToolBlock::default);
        }
        &mut self.blocks[index]
    }

    fn report(&self, sink: Sink<'_>) {
        if self.prompt == 0 && self.completion == 0 {
            return;
        }
        // Из кэша не могло прийти больше, чем весь вход: иначе в окне будут проценты выше ста.
        let usage = Usage { prompt: self.prompt, completion: self.completion, cached: self.cached.min(self.prompt) };
        sink(DriverEvent::Usage(usage));
    }

    /// Обрабатывает одно SSE-событие. `true` — поток закончился.
    fn handle(&mut self, event: &Value, sink: Sink<'_>) -> Result<bool, String> {
        match event["type"].as_str().unwrap_or_default() {
            "message_start" => {
                let usage = &event["message"]["usage"];
                self.prompt = input_tokens(usage);
                self.cached = read_cached(usage);
                self.report(&mut *sink);
            }
            "content_block_start" => {
                let index = event["index"].as_u64().unwrap_or(0) as usize;
                let block = &event["content_block"];
                match block["type"].as_str().unwrap_or_default() {
                    "text" => {
                        if let Some(text) = block["text"].as_str().filter(|text| !text.is_empty()) {
                            self.produced = true;
                            sink(DriverEvent::Text(text.to_string()));
                        }
                    }
                    "tool_use" => {
                        let slot = self.slot(index);
                        slot.id = block["id"].as_str().unwrap_or_default().to_string();
                        slot.name = block["name"].as_str().unwrap_or_default().to_string();
                        if block["input"].is_object() {
                            slot.input = block["input"].clone();
                        }
                    }
                    // thinking и служебные блоки в ответ человеку не отдаём.
                    _ => {}
                }
            }
            "content_block_delta" => {
                let index = event["index"].as_u64().unwrap_or(0) as usize;
                let delta = &event["delta"];
                match delta["type"].as_str().unwrap_or_default() {
                    "text_delta" => {
                        if let Some(text) = delta["text"].as_str().filter(|text| !text.is_empty()) {
                            self.produced = true;
                            sink(DriverEvent::Text(text.to_string()));
                        }
                    }
                    // Аргументы инструмента приходят по кусочкам: склеиваем и разбираем на stop.
                    "input_json_delta" => {
                        if let Some(json) = delta["partial_json"].as_str() {
                            self.slot(index).json.push_str(json);
                        }
                    }
                    _ => {}
                }
            }
            "content_block_stop" => {
                let index = event["index"].as_u64().unwrap_or(0) as usize;
                if let Some(block) = self.blocks.get_mut(index) {
                    if !block.name.is_empty() {
                        self.produced = true;
                    }
                    emit_tool(block, &mut *sink);
                }
            }
            "message_delta" => {
                if let Some(completion) = event["usage"]["output_tokens"].as_i64() {
                    self.completion = completion.max(0);
                }
                if let Some(reason) = event["delta"]["stop_reason"].as_str() {
                    self.stop_reason = reason.to_string();
                }
                if let Some(category) = event["delta"]["stop_details"]["category"].as_str() {
                    self.refusal = category.to_string();
                }
                self.report(&mut *sink);
            }
            "message_stop" => {
                self.finished = true;
                return Ok(true);
            }
            "error" => return Err(stream_error(&event["error"])),
            // ping и незнакомые события не мешают: их просто не показываем.
            _ => {}
        }
        Ok(false)
    }

    /// Итог потока. Недособранный вызов инструмента не отдаём никогда: с обрезанными аргументами
    /// он записал бы половину файла. Обрыв, отказ и упор в предел — ошибки, тогда сработает и запасная модель.
    fn finish(&mut self) -> Result<(), String> {
        let unfinished = self.blocks.iter().any(|block| !block.name.is_empty());
        if self.stop_reason == "refusal" {
            let category = if self.refusal.is_empty() { String::new() } else { format!(" ({})", self.refusal) };
            return Err(format!("Модель отказалась отвечать{category}"));
        }
        if self.stop_reason == "max_tokens" && (unfinished || !self.produced) {
            return Err("ответ упёрся в предел длины и оборвался".into());
        }
        if !self.finished && (unfinished || !self.produced) {
            return Err("поток ответа оборвался".into());
        }
        Ok(())
    }
}

/// Ответ без stream:true — та же форма, но все блоки целиком.
fn emit_message(message: &Value, sink: Sink<'_>) -> Result<(), String> {
    match message["stop_reason"].as_str().unwrap_or_default() {
        "refusal" => return Err("Модель отказалась отвечать".into()),
        "max_tokens" if message["content"].as_array().is_some_and(|list| list.iter().any(|c| c["type"] == "tool_use")) => {
            return Err("ответ упёрся в предел длины и оборвался".into());
        }
        _ => {}
    }
    for content in message["content"].as_array().into_iter().flatten() {
        match content["type"].as_str().unwrap_or_default() {
            "text" => {
                if let Some(text) = content["text"].as_str().filter(|text| !text.is_empty()) {
                    sink(DriverEvent::Text(text.to_string()));
                }
            }
            "tool_use" => {
                let mut block = ToolBlock {
                    id: content["id"].as_str().unwrap_or_default().to_string(),
                    name: content["name"].as_str().unwrap_or_default().to_string(),
                    json: String::new(),
                    input: content["input"].clone(),
                };
                emit_tool(&mut block, &mut *sink);
            }
            _ => {}
        }
    }
    let usage = &message["usage"];
    let prompt = input_tokens(usage);
    let completion = usage["output_tokens"].as_i64().unwrap_or(0).max(0);
    if prompt > 0 || completion > 0 {
        sink(DriverEvent::Usage(Usage { prompt, completion, cached: read_cached(usage).min(prompt) }));
    }
    Ok(())
}

pub async fn run(req: Request<'_>, cancel: &CancellationToken, sink: Sink<'_>) -> Result<(), String> {
    if req.api_key.trim().is_empty() {
        return Err("нет ключа Anthropic: впиши его в настройках или положи в .env как ANTHROPIC_API_KEY".into());
    }
    if req.model.trim().is_empty() {
        return Err("не задана модель Anthropic: впиши её в настройках или в профиле бота, например claude-sonnet-5-5".into());
    }

    let url = endpoint(req.base_url);
    let body = request_body(&req);
    let client = reqwest::Client::new();
    let send = client
        .post(&url)
        .header("x-api-key", req.api_key.trim())
        .header("anthropic-version", ANTHROPIC_VERSION)
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

    // Прокси иногда отвечает обычным JSON, не глядя на stream:true: тогда читаем ответ целиком.
    let kind = res.headers().get("content-type").and_then(|value| value.to_str().ok()).unwrap_or_default();
    if !kind.contains("event-stream") {
        let raw = res.text().await.map_err(|error| error.to_string())?;
        let parsed: Value = serde_json::from_str(&raw).map_err(|error| error.to_string())?;
        if parsed["type"] == "error" {
            return Err(stream_error(&parsed["error"]));
        }
        return emit_message(&parsed, sink);
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
            if state.handle(&event, &mut *sink)? {
                break 'read;
            }
        }
    }
    state.finish()
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
                match state.handle(&event, &mut sink) {
                    Ok(true) => break,
                    Ok(false) => {}
                    Err(message) => {
                        error = Some(message);
                        break;
                    }
                }
            }
            if error.is_none() {
                error = state.finish().err();
            }
        }
        let mut parsed = split(events);
        parsed.error = error;
        parsed
    }

    fn collect_message(message: &Value) -> Parsed {
        let mut events = Vec::new();
        {
            let mut sink = |event: DriverEvent| events.push(event);
            emit_message(message, &mut sink).unwrap();
        }
        split(events)
    }

    #[test]
    fn request_body_has_system_tools_and_stream() {
        let messages = vec![json!({ "role": "system", "content": "Ты — бот" }), json!({ "role": "user", "content": "привет" })];
        let tools = json!([{
            "type": "function",
            "function": { "name": "read_file", "description": "Читает файл", "parameters": { "type": "object", "properties": {} } },
        }]);
        let request = Request {
            model: " claude-sonnet-4-5 ",
            base_url: "",
            api_key: "sk-ant-secret",
            messages: &messages,
            tools,
        };
        let body = request_body(&request);
        assert_eq!(body["model"], "claude-sonnet-4-5", "модель обрезается по краям");
        assert_eq!(body["stream"], true);
        assert_eq!(body["max_tokens"], 64_000);
        assert!(body.get("temperature").is_none(), "нынешние модели отвечают на temperature ошибкой 400");
        assert_eq!(body["cache_control"]["type"], "ephemeral", "без отметки кэш не ведётся");
        assert_eq!(body["system"], "Ты — бот", "system уходит отдельным полем");
        assert_eq!(body["messages"].as_array().map(Vec::len), Some(1));
        assert_eq!(body["tools"][0]["name"], "read_file");
        assert_eq!(body["tools"][0]["input_schema"]["type"], "object");
    }

    #[test]
    fn tools_are_converted_to_input_schema() {
        let tools = json!([{
            "type": "function",
            "function": {
                "name": "read_file",
                "description": "Читает файл",
                "parameters": { "type": "object", "properties": { "path": { "type": "string" } }, "required": ["path"] },
            },
        }]);
        let converted = convert_tools(&tools);
        assert_eq!(converted[0]["name"], "read_file");
        assert_eq!(converted[0]["description"], "Читает файл");
        assert_eq!(converted[0]["input_schema"]["properties"]["path"]["type"], "string");
        assert!(converted[0].get("function").is_none(), "формат OpenAI не должен протекать в запрос");

        // Без схемы Anthropic всё равно ждёт объект: пустая схема, а не null.
        let without = convert_tools(&json!([{ "type": "function", "function": { "name": "clock" } }]));
        assert_eq!(without[0]["input_schema"]["type"], "object");
    }

    #[test]
    fn history_is_split_into_system_and_turns() {
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
        let (system, converted) = convert_history(&messages);
        assert_eq!(system, "Ты — бот");
        assert_eq!(converted.len(), 3, "{converted:?}");
        assert_eq!(converted[0]["role"], "user");
        assert_eq!(converted[0]["content"][0]["text"], "прочитай файл");
        assert_eq!(converted[1]["role"], "assistant");
        assert_eq!(converted[1]["content"][0]["text"], "сейчас");
        assert_eq!(converted[1]["content"][1]["type"], "tool_use");
        assert_eq!(converted[1]["content"][1]["input"]["path"], "a.txt");
        // Результат инструмента и следующая реплика человека — один ход user.
        assert_eq!(converted[2]["role"], "user");
        assert_eq!(converted[2]["content"][0]["type"], "tool_result");
        assert_eq!(converted[2]["content"][0]["tool_use_id"], "call_1");
        assert_eq!(converted[2]["content"][1]["text"], "спасибо");
    }

    #[test]
    fn base_url_gets_the_version_segment_once() {
        assert_eq!(endpoint(""), "https://api.anthropic.com/v1/messages");
        assert_eq!(endpoint("https://api.anthropic.com/"), "https://api.anthropic.com/v1/messages");
        assert_eq!(endpoint("https://api.anthropic.com/v1"), "https://api.anthropic.com/v1/messages");
        assert_eq!(endpoint(" http://localhost:8080/proxy/v1/ "), "http://localhost:8080/proxy/v1/messages");
    }

    #[test]
    fn sse_stream_yields_text_tool_call_and_cached_usage() {
        let sse = r#"event: message_start
data: {"type":"message_start","message":{"usage":{"input_tokens":10,"cache_read_input_tokens":7,"cache_creation_input_tokens":2,"output_tokens":1}}}

event: content_block_start
data: {"type":"content_block_start","index":0,"content_block":{"type":"text","text":""}}

event: content_block_delta
data: {"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"Сейчас "}}

data: {"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"прочитаю."}}

event: content_block_stop
data: {"type":"content_block_stop","index":0}

event: content_block_start
data: {"type":"content_block_start","index":1,"content_block":{"type":"tool_use","id":"toolu_1","name":"read_file","input":{}}}

event: content_block_delta
data: {"type":"content_block_delta","index":1,"delta":{"type":"input_json_delta","partial_json":"{\"path\":"}}

event: content_block_delta
data: {"type":"content_block_delta","index":1,"delta":{"type":"input_json_delta","partial_json":"\"a.txt\"}"}}

event: content_block_stop
data: {"type":"content_block_stop","index":1}

event: message_delta
data: {"type":"message_delta","delta":{"stop_reason":"tool_use"},"usage":{"output_tokens":25}}

event: message_stop
data: {"type":"message_stop"}
"#;
        let parsed = drain(sse);
        assert!(parsed.error.is_none(), "{:?}", parsed.error);
        assert_eq!(parsed.text, "Сейчас прочитаю.");
        assert_eq!(parsed.calls.len(), 1);
        assert_eq!(parsed.calls[0].0, "toolu_1");
        assert_eq!(parsed.calls[0].1, "read_file");
        assert_eq!(parsed.calls[0].2, json!({ "path": "a.txt" }));
        // 10 свежих + 7 из кэша + 2 записано в кэш = весь вход.
        assert_eq!(parsed.usage, Some(Usage { prompt: 19, completion: 25, cached: 7 }));
    }

    #[test]
    fn tool_without_arguments_becomes_empty_object() {
        let sse = r#"data: {"type":"content_block_start","index":0,"content_block":{"type":"tool_use","id":"toolu_9","name":"clock","input":{}}}
data: {"type":"content_block_stop","index":0}
"#;
        let parsed = drain(sse);
        assert!(parsed.error.is_none());
        assert_eq!(parsed.calls[0].2, json!({}), "пустой JSON — это {{}}, а не null");
    }

    #[test]
    fn message_without_stream_is_read_whole() {
        let message = json!({
            "type": "message",
            "content": [
                { "type": "text", "text": "Готово" },
                { "type": "tool_use", "id": "toolu_2", "name": "write_file", "input": { "path": "a.txt" } },
            ],
            "usage": { "input_tokens": 3, "cache_read_input_tokens": 1, "output_tokens": 4 },
        });
        let parsed = collect_message(&message);
        assert_eq!(parsed.text, "Готово");
        assert_eq!(parsed.calls[0].1, "write_file");
        assert_eq!(parsed.usage, Some(Usage { prompt: 4, completion: 4, cached: 1 }));
    }

    #[test]
    fn unauthorized_error_is_short_and_has_no_key() {
        let request = Request {
            model: "claude-sonnet-4-5",
            base_url: "",
            api_key: "sk-ant-secret",
            messages: &[],
            tools: json!([]),
        };
        let message = api_error(401, r#"{"type":"error","error":{"type":"authentication_error","message":"invalid x-api-key"}}"#);
        assert_eq!(message, "API 401: invalid x-api-key");
        assert!(!message.contains(request.api_key), "ключ не должен попадать в текст ошибки");

        // Тело-простыня обрезается, чтобы не раздувать интерфейс и журнал.
        let long = api_error(500, &"ошибка ".repeat(500));
        assert!(long.starts_with("API 500: "));
        assert!(long.chars().count() < 700, "{}", long.chars().count());
    }

    #[test]
    fn a_refusal_and_a_cut_stream_are_errors_not_answers() {
        let refusal = r#"data: {"type":"message_start","message":{"usage":{"input_tokens":5}}}
data: {"type":"message_delta","delta":{"stop_reason":"refusal","stop_details":{"type":"refusal","category":"cyber"}},"usage":{"output_tokens":1}}
data: {"type":"message_stop"}
"#;
        assert_eq!(drain(refusal).error.as_deref(), Some("Модель отказалась отвечать (cyber)"));

        // Поток оборвался посреди аргументов инструмента: вызова нет, есть ошибка.
        let cut = r#"data: {"type":"content_block_start","index":0,"content_block":{"type":"tool_use","id":"toolu_1","name":"write_file","input":{}}}
data: {"type":"content_block_delta","index":0,"delta":{"type":"input_json_delta","partial_json":"{\"path\":\"a.txt\",\"content\":\"нача"}}
"#;
        let parsed = drain(cut);
        assert!(parsed.calls.is_empty(), "обрезанный вызов не должен выполниться: {:?}", parsed.calls);
        assert_eq!(parsed.error.as_deref(), Some("поток ответа оборвался"));

        // Упор в предел посреди вызова — тоже ошибка.
        let long = r#"data: {"type":"content_block_start","index":0,"content_block":{"type":"tool_use","id":"toolu_2","name":"write_file","input":{}}}
data: {"type":"content_block_delta","index":0,"delta":{"type":"input_json_delta","partial_json":"{\"path\":"}}
data: {"type":"message_delta","delta":{"stop_reason":"max_tokens"},"usage":{"output_tokens":64000}}
data: {"type":"message_stop"}
"#;
        let parsed = drain(long);
        assert!(parsed.calls.is_empty());
        assert_eq!(parsed.error.as_deref(), Some("ответ упёрся в предел длины и оборвался"));
    }

    #[test]
    fn stream_error_keeps_status_for_fallback() {
        assert_eq!(stream_error(&json!({ "type": "overloaded_error", "message": "Overloaded" })), "API 529: Overloaded");
        assert_eq!(stream_error(&json!({ "type": "rate_limit_error" })), "API 429: ошибка потока");
    }
}
