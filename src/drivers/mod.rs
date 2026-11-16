//! Драйверы — «мозги» бота. Драйвер получает сообщения и отдаёт текст и вызовы инструментов.

pub mod acp;
pub mod anthropic;
pub mod cli;
pub mod demo;
pub mod gemini;
pub mod gigachat;
pub mod openai;

use serde_json::Value;

#[derive(Clone, Debug)]
pub struct ToolCall {
    pub id: String,
    pub name: String,
    pub args: Value,
}

/// Счётчик токенов от провайдера за один шаг.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Usage {
    /// Весь входной промпт.
    pub prompt: i64,
    /// Ответ модели.
    pub completion: i64,
    /// Часть `prompt`, которую провайдер взял из своего кэша: она стоит в разы дешевле.
    pub cached: i64,
}

pub enum DriverEvent {
    Text(String),
    Tool(ToolCall),
    /// Счётчик токенов от провайдера: вход, выход и попадание в кэш этого шага.
    Usage(Usage),
    /// Что делает внешний агент (ACP): строка хода работы `{id, name?, kind?, status?, args?, result?}`.
    /// Инструменты исполняет сам агент, мы только показываем. Повтор того же `id` обновляет строку.
    Activity(Value),
}

pub type Sink<'a> = &'a mut (dyn FnMut(DriverEvent) + Send);

/// Статус из текста ошибки драйвера: `API 429: ...`.
fn api_status(message: &str) -> Option<u16> {
    let rest = message.strip_prefix("API ")?;
    let digits = rest.bytes().take_while(|b| b.is_ascii_digit()).count();
    if digits == 0 || !rest[digits..].starts_with(':') {
        return None;
    }
    rest[..digits].parse().ok()
}

/// Упёрлись в лимит, баланс или перегрузку провайдера: есть смысл идти на запасную модель.
pub fn is_limit_error(message: &str) -> bool {
    // Неверный запрос или ключ — это настройка, а не лимит: переключение тут не поможет.
    if matches!(api_status(message), Some(400 | 401)) {
        return false;
    }
    if matches!(api_status(message), Some(402 | 408 | 429 | 500 | 502 | 503 | 504)) {
        return true;
    }
    let lower = message.to_lowercase();
    const LIMIT_WORDS: [&str; 9] = [
        "usage limit",
        "rate limit",
        "quota",
        "insufficient",
        "payment required",
        "balance",
        "overloaded",
        "reached their",
        "too many requests",
    ];
    if LIMIT_WORDS.iter().any(|word| lower.contains(word)) {
        return true;
    }
    // Ошибки соединения: адрес не отвечает, DNS, таймаут.
    const CONNECTION_WORDS: [&str; 6] = ["connection", "connect", "dns error", "network", "timed out", "timeout"];
    CONNECTION_WORDS.iter().any(|word| lower.contains(word))
}

/// Временный сбой транспорта или сервера. Ошибки учётных данных и запросов не повторяем.
pub fn is_transient_model_error(message: &str) -> bool {
    if let Some(status) = api_status(message) {
        return status == 408 || (500..=599).contains(&status);
    }
    let lower = message.to_lowercase();
    ["error sending request", "connection", "connect error", "dns error", "network", "timed out", "timeout", "error decoding response body", "unexpected eof", "connection reset", "broken pipe"].iter().any(|word| lower.contains(word))
}

pub fn retry_delay(attempt: u32) -> std::time::Duration {
    std::time::Duration::from_secs(match attempt { 0 => 2, 1 => 5, 2 => 10, 3 => 20, 4 => 30, _ => 60 })
}

/// Запрос не влез в окно модели: поможет сжатие истории, а не запасная модель или повтор.
pub fn is_context_error(message: &str) -> bool {
    let lower = message.to_lowercase();
    // «Токенов в минуту» — это лимит скорости, а не размер окна.
    if lower.contains("per minute") || lower.contains("rate limit") {
        return false;
    }
    const CONTEXT_WORDS: [&str; 10] = [
        "context_length_exceeded",
        "context length",
        "maximum context",
        "context window",
        "prompt is too long",
        "input is too long",
        "too many tokens",
        "reduce the length",
        "exceeds the maximum",
        "tokens_limit_exceeded",
    ];
    CONTEXT_WORDS.iter().any(|word| lower.contains(word))
}

#[cfg(test)]
mod context_tests {
    use super::{is_context_error, is_limit_error};

    #[test]
    fn an_overflowing_prompt_is_told_apart_from_a_rate_limit() {
        assert!(is_context_error("API 400: This model's maximum context length is 65536 tokens"));
        assert!(is_context_error("API 400: {\"error\":{\"code\":\"context_length_exceeded\"}}"));
        assert!(is_context_error("API 400: prompt is too long: 210000 tokens > 200000 maximum"));
        assert!(!is_context_error("API 429: Rate limit reached for tokens per minute"));
        assert!(!is_context_error("API 401: invalid key"));
        assert!(!is_limit_error("API 400: This model's maximum context length is 65536 tokens"));
    }
}

#[cfg(test)]
mod tests {
    use super::is_limit_error;

    #[test]
    fn limits_and_connection_failures_are_recognised() {
        assert!(is_limit_error("API 402: Grok Build usage balance"));
        assert!(is_limit_error("The free OpenCode models reached their usage limit"));
        for code in [408, 429, 500, 502, 503, 504] {
            assert!(is_limit_error(&format!("API {code}: провайдер не ответил")), "{code}");
        }
        assert!(is_limit_error("API 429: Rate limit exceeded"));
        assert!(is_limit_error("quota exceeded for this key"));
        assert!(is_limit_error("insufficient balance"));
        assert!(is_limit_error("Payment required"));
        assert!(is_limit_error("server overloaded"));
        assert!(is_limit_error("too many requests"));
        assert!(is_limit_error("error sending request: connection refused"));
        assert!(is_limit_error("dns error: no such host"));
    }

    #[test]
    fn other_errors_do_not_switch_the_model() {
        assert!(!is_limit_error("API 400: invalid request"));
        assert!(!is_limit_error("API 401: Incorrect API key provided"));
        assert!(!is_limit_error("остановлено"));
        assert!(!is_limit_error("CLI «claude» не найден в PATH"));
        assert!(!is_limit_error("нет ключа API: впиши его в настройках или положи в .env"));
        assert!(!is_limit_error("Ошибка инструмента: файл не найден"));
    }
}
