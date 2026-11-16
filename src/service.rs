//! Служебные вызовы модели: сводка истории, доктор памяти, знакомство с ботом.
//! Один шаг без инструментов. Расход идёт в учёт того бота, чьей моделью шли,
//! а ключ провайдера — только этому провайдеру, как и в обычном ходе.

use std::sync::Arc;
use std::time::Duration;

use serde_json::{json, Value};
use tokio_util::sync::CancellationToken;

use crate::drivers::{is_limit_error, DriverEvent, Usage};
use crate::store::Bot;
use crate::turn::{drive, estimate_usage, fallback_choice, main_choice, ModelChoice};
use crate::App;

const SERVICE_TIMEOUT: Duration = Duration::from_secs(180);

/// Подходит ли драйвер для служебного вызова. Демо отвечает заготовками, а внешние агенты
/// и CLI — это целые программы со своим контекстом и инструментами: сводку им не заказываем.
pub fn usable(driver: &str) -> bool {
    !(driver.is_empty() || driver == "demo" || driver == "cli" || driver == "acp" || driver.starts_with("acp:"))
}

/// Может ли этот бот сделать служебный вызов своей моделью.
pub fn can_serve(app: &App, bot: &Bot) -> bool {
    usable(&main_choice(app, bot, &app.store.settings()).driver)
}

/// Чьей моделью делать общую служебную работу (доктор памяти): модель по умолчанию из настроек,
/// а если там демо или внешний агент — модель первого бота с обычным провайдером.
pub fn house_bot(app: &App) -> Option<Bot> {
    let house = Bot { id: "_memory".into(), name: "Память".into(), role: "служебная работа".into(), ..Bot::default() };
    if can_serve(app, &house) {
        return Some(house);
    }
    app.store.bots().into_iter().find(|bot| can_serve(app, bot))
}

async fn call_once(app: &Arc<App>, bot: &Bot, choice: &ModelChoice, messages: &[Value], kind: &str) -> Result<String, String> {
    let settings = app.store.settings();
    let cancel = CancellationToken::new();
    let mut text = String::new();
    let mut reported: Option<Usage> = None;
    let outcome = {
        let mut on_event = |ev: DriverEvent| match ev {
            DriverEvent::Text(chunk) => text.push_str(&chunk),
            DriverEvent::Usage(usage) => reported = Some(usage),
            _ => {}
        };
        let no_tools = json!([]);
        let run = drive(app, bot, &settings, choice, messages, "", &no_tools, &cancel, &mut on_event);
        match tokio::time::timeout(SERVICE_TIMEOUT, run).await {
            Ok(result) => result,
            Err(_) => {
                cancel.cancel();
                Err("модель не ответила за три минуты".to_string())
            }
        }
    };
    if choice.is_metered() {
        let usage = reported.unwrap_or_else(|| estimate_usage(messages, &text, &[]));
        app.ledger.add_tokens(&bot.id, usage.prompt, usage.completion, usage.cached, reported.is_none()).await;
        // Те же токены видны на экране «Использование» как работа за кадром и в разрезе моделей.
        let spent = usage.prompt + usage.completion;
        app.ledger.add_behind(kind, spent).await;
        app.ledger.add_model_tokens(&choice.label(), choice.provider(), spent).await;
    }
    outcome.map(|()| text.trim().to_string())
}

/// Один служебный запрос: системный текст и задание. Основная упёрлась в лимит — пробуем запасную.
pub async fn complete(app: &Arc<App>, bot: &Bot, system: &str, task: &str) -> Result<String, String> {
    complete_as("helper", app, bot, system, task).await
}

/// Тот же запрос, но с видом закулисной работы для учёта: `memory` (сводки, доктор, извлечение), `helper`, `router`.
pub async fn complete_as(kind: &str, app: &Arc<App>, bot: &Bot, system: &str, task: &str) -> Result<String, String> {
    let settings = app.store.settings();
    let main = main_choice(app, bot, &settings);
    if !usable(&main.driver) {
        return Err("для этой работы нужна модель по API: демо и внешние агенты не подходят".to_string());
    }
    let messages = vec![json!({ "role": "system", "content": system }), json!({ "role": "user", "content": task })];
    match call_once(app, bot, &main, &messages, kind).await {
        Ok(text) if !text.is_empty() => Ok(text),
        Ok(_) => Err("модель вернула пустой ответ".to_string()),
        Err(message) if is_limit_error(&message) => {
            let Some(spare) = fallback_choice(app, &settings).filter(|spare| usable(&spare.driver)) else {
                return Err(message);
            };
            call_once(app, bot, &spare, &messages, kind).await.and_then(|text| {
                if text.is_empty() {
                    Err("модель вернула пустой ответ".to_string())
                } else {
                    Ok(text)
                }
            })
        }
        Err(message) => Err(message),
    }
}

/// Первый JSON-объект в ответе модели: модели любят оборачивать его в ```json и пояснения.
pub fn json_object(reply: &str) -> Option<Value> {
    let start = reply.find('{')?;
    let end = reply.rfind('}')?;
    if end <= start {
        return None;
    }
    serde_json::from_str(&reply[start..=end]).ok().filter(Value::is_object)
}

/// Обрезать по знакам, а не по байтам: русский текст в UTF-8 иначе ломается посередине буквы.
pub fn take_chars(text: &str, limit: usize) -> String {
    text.chars().take(limit).collect()
}

#[cfg(test)]
mod tests {
    use super::{json_object, usable};

    #[test]
    fn json_is_found_inside_fences_and_chatter() {
        let reply = "Вот ответ:\n```json\n{\"brief\": \"a\", \"ops\": []}\n```\nГотово.";
        assert_eq!(json_object(reply).unwrap()["brief"], "a");
        assert!(json_object("нет json").is_none());
        assert!(json_object("[1, 2]").is_none());
    }

    #[test]
    fn agents_and_demo_do_not_do_service_work() {
        for driver in ["demo", "cli", "acp", "acp:claude", ""] {
            assert!(!usable(driver), "{driver}");
        }
        for driver in ["api", "deloist", "anthropic", "gemini", "openai:ollama"] {
            assert!(usable(driver), "{driver}");
        }
    }
}
