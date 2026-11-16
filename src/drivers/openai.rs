//! Драйвер OpenAI-совместимого API: всё, что умеет /chat/completions со stream:true и tool_calls.

use futures_util::StreamExt;
use serde_json::{json, Value};
use tokio_util::sync::CancellationToken;

use super::{DriverEvent, Sink, ToolCall, Usage};
use crate::store::uid;

pub struct Request<'a> {
    pub model: &'a str,
    pub base_url: &'a str,
    pub api_key: &'a str,
    /// Как передать ключ: пусто — `Bearer`, `api-key` — `Api-Key` (Yandex AI Studio), `none` — без ключа.
    pub auth: &'a str,
    pub messages: &'a [Value],
    pub tools: Value,
    pub temperature: f64,
    /// `reasoning_effort` для моделей, которые его принимают; пусто — поле не шлём.
    pub effort: &'a str,
}

fn parse_args(raw: &Value) -> Value {
    match raw {
        Value::Object(_) => raw.clone(),
        Value::String(s) if s.is_empty() => json!({}),
        Value::String(s) => serde_json::from_str(s).unwrap_or_else(|_| json!({ "_raw": s })),
        _ => json!({}),
    }
}

/// Провайдеры называют попадание в кэш по-разному, но смысл один: столько входных
/// токенов уже лежало в кэше и стоило дешевле.
/// DeepSeek — `prompt_cache_hit_tokens`, OpenAI — `prompt_tokens_details.cached_tokens`,
/// Anthropic-совместимые — `cache_read_input_tokens`.
pub fn cached_tokens(usage: &Value) -> i64 {
    usage["prompt_cache_hit_tokens"]
        .as_i64()
        .or_else(|| usage["prompt_tokens_details"]["cached_tokens"].as_i64())
        .or_else(|| usage["cache_read_input_tokens"].as_i64())
        .or_else(|| usage["cached_tokens"].as_i64())
        .unwrap_or(0)
        .max(0)
}

fn report_usage(parsed: &Value, sink: Sink<'_>) {
    let usage = &parsed["usage"];
    if let (Some(prompt), Some(completion)) = (usage["prompt_tokens"].as_i64(), usage["completion_tokens"].as_i64()) {
        // Больше, чем весь вход, из кэша взять нельзя: иначе в окне появятся проценты выше ста.
        sink(DriverEvent::Usage(Usage { prompt, completion, cached: cached_tokens(usage).min(prompt.max(0)) }));
    }
}

/// Подряд идущие сообщения человека склеиваем: часы, «сейчас» из очереди и реплики других ботов
/// в группе дают такие цепочки, а строгие провайдеры отвечают на них ошибкой. Результат зависит
/// только от истории, поэтому начало запроса между ходами не меняется.
fn merge_successive_user(messages: &[Value]) -> Vec<Value> {
    let mut merged: Vec<Value> = Vec::with_capacity(messages.len());
    for message in messages {
        if let (Some(text), Some(last)) = (message["content"].as_str().filter(|_| message["role"] == "user"), merged.last_mut()) {
            if last["role"] == "user" {
                if let Some(previous) = last["content"].as_str() {
                    let joined = format!("{previous}\n\n{text}");
                    last["content"] = json!(joined);
                    continue;
                }
            }
        }
        merged.push(message.clone());
    }
    merged
}

#[derive(Default)]
struct PartialCall {
    id: String,
    name: String,
    args: String,
}

pub async fn run(req: Request<'_>, cancel: &CancellationToken, sink: Sink<'_>) -> Result<(), String> {
    if req.api_key.is_empty() {
        return Err("нет ключа API: впиши его в настройках или положи в .env (OPENAI_API_KEY)".into());
    }

    let model = if req.model.is_empty() { "gpt-4o-mini" } else { req.model };
    let mut body = json!({
        "model": model,
        "messages": merge_successive_user(req.messages),
        "temperature": req.temperature,
        "stream": true,
    });
    if req.tools.as_array().is_some_and(|t| !t.is_empty()) {
        body["tools"] = req.tools.clone();
        body["tool_choice"] = json!("auto");
    }
    // Только для моделей из allowlist (см. providers::takes_effort): остальным это поле даёт 400.
    if !req.effort.is_empty() {
        body["reasoning_effort"] = json!(req.effort);
    }

    // Без этого в потоке нет счётчика токенов. Старые провайдеры на такое поле отвечают 400: тогда без него.
    body["stream_options"] = json!({ "include_usage": true });

    let url = format!("{}/chat/completions", req.base_url.trim_end_matches('/'));
    let client = reqwest::Client::new();
    let res = loop {
        let mut send = client.post(&url);
        send = match req.auth {
            "none" => send,
            "api-key" => send.header("authorization", format!("Api-Key {}", req.api_key)),
            _ => send.bearer_auth(req.api_key),
        };
        let send = send
            .header("content-type", "application/json")
            .header("user-agent", concat!("deloist/", env!("CARGO_PKG_VERSION")))
            .body(body.to_string())
            .send();
        let res = tokio::select! {
            _ = cancel.cancelled() => return Err("остановлено".into()),
            r = send => r.map_err(|e| e.to_string())?,
        };
        if res.status().is_success() {
            break res;
        }
        let status = res.status().as_u16();
        let detail = res.text().await.unwrap_or_default();
        if status == 400 && detail.contains("stream_options") && body.get("stream_options").is_some() {
            body.as_object_mut().map(|fields| fields.remove("stream_options"));
            continue;
        }
        let short: String = detail.chars().take(600).collect();
        return Err(format!("API {status}: {short}"));
    };

    // Некоторые провайдеры игнорируют stream:true и отвечают обычным JSON.
    let kind = res.headers().get("content-type").and_then(|v| v.to_str().ok()).unwrap_or_default();
    if !kind.contains("event-stream") {
        let raw = res.text().await.map_err(|e| e.to_string())?;
        let parsed: Value = serde_json::from_str(&raw).map_err(|e| e.to_string())?;
        let msg = &parsed["choices"][0]["message"];
        if let Some(text) = msg["content"].as_str().filter(|t| !t.is_empty()) {
            sink(DriverEvent::Text(text.to_string()));
        }
        for call in msg["tool_calls"].as_array().into_iter().flatten() {
            let id = call["id"].as_str().map_or_else(|| uid("call"), str::to_string);
            let name = call["function"]["name"].as_str().unwrap_or_default().to_string();
            sink(DriverEvent::Tool(ToolCall { id, name, args: parse_args(&call["function"]["arguments"]) }));
        }
        report_usage(&parsed, sink);
        return Ok(());
    }

    let mut stream = res.bytes_stream();
    let mut buf: Vec<u8> = Vec::new();
    let mut calls: Vec<PartialCall> = Vec::new();
    'read: loop {
        let chunk = tokio::select! {
            _ = cancel.cancelled() => return Err("остановлено".into()),
            c = stream.next() => c,
        };
        let Some(chunk) = chunk else { break };
        buf.extend_from_slice(&chunk.map_err(|e| e.to_string())?);

        while let Some(nl) = buf.iter().position(|b| *b == b'\n') {
            let raw: Vec<u8> = buf.drain(..=nl).collect();
            let line = String::from_utf8_lossy(&raw);
            let line = line.trim();
            let Some(payload) = line.strip_prefix("data:") else { continue };
            let payload = payload.trim();
            if payload == "[DONE]" {
                break 'read;
            }
            let Ok(parsed) = serde_json::from_str::<Value>(payload) else { continue };
            report_usage(&parsed, sink);
            let delta = &parsed["choices"][0]["delta"];
            if let Some(text) = delta["content"].as_str().filter(|t| !t.is_empty()) {
                sink(DriverEvent::Text(text.to_string()));
            }
            for tc in delta["tool_calls"].as_array().into_iter().flatten() {
                let idx = tc["index"].as_u64().unwrap_or(0) as usize;
                if calls.len() <= idx {
                    calls.resize_with(idx + 1, PartialCall::default);
                }
                let slot = &mut calls[idx];
                if let Some(id) = tc["id"].as_str() {
                    slot.id = id.to_string();
                }
                if let Some(name) = tc["function"]["name"].as_str() {
                    slot.name.push_str(name);
                }
                if let Some(args) = tc["function"]["arguments"].as_str() {
                    slot.args.push_str(args);
                }
            }
        }
    }

    for call in calls {
        if call.name.is_empty() {
            continue;
        }
        let id = if call.id.is_empty() { uid("call") } else { call.id };
        sink(DriverEvent::Tool(ToolCall { id, name: call.name, args: parse_args(&Value::String(call.args)) }));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::{cached_tokens, merge_successive_user};

    #[test]
    fn successive_user_messages_are_merged_but_images_stay_apart() {
        let picture = json!([{ "type": "text", "text": "снимок" }]);
        let merged = merge_successive_user(&[
            json!({ "role": "system", "content": "s" }),
            json!({ "role": "user", "content": "часы" }),
            json!({ "role": "user", "content": "вопрос" }),
            json!({ "role": "assistant", "content": "ответ" }),
            json!({ "role": "user", "content": picture }),
            json!({ "role": "user", "content": "ещё" }),
        ]);
        assert_eq!(merged.len(), 5, "{merged:?}");
        assert_eq!(merged[1]["content"], "часы\n\nвопрос");
        assert_eq!(merged[3]["content"], picture);
        assert_eq!(merged[4]["content"], "ещё");
    }

    #[test]
    fn cache_hits_are_read_from_every_provider_shape() {
        assert_eq!(cached_tokens(&json!({ "prompt_cache_hit_tokens": 12 })), 12);
        assert_eq!(cached_tokens(&json!({ "prompt_tokens_details": { "cached_tokens": 34 } })), 34);
        assert_eq!(cached_tokens(&json!({ "cache_read_input_tokens": 56 })), 56);
        assert_eq!(cached_tokens(&json!({ "cached_tokens": 78 })), 78);
        assert_eq!(cached_tokens(&json!({ "prompt_tokens": 100 })), 0, "нет поля кэша — ноль, а не выдумка");
        assert_eq!(cached_tokens(&json!({ "prompt_cache_hit_tokens": -5 })), 0, "минус не бывает");
    }
}
