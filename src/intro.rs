//! Знакомство с новым ботом: приветствие и три стартовые задачи в пустом чате.
//! Строятся один раз моделью бота и хранятся у него; без модели — заготовка на языке интерфейса.

use std::sync::Arc;

use serde_json::{json, Value};

use crate::service;
use crate::store::Bot;
use crate::App;

const TASK_LIMIT: usize = 120;
const GREETING_LIMIT: usize = 400;

fn fallback(bot: &Bot, english: bool) -> Value {
    if english {
        json!({
            "greeting": format!("Hi, I'm {}. Tell me what you need, or pick a task below.", bot.name),
            "tasks": [
                "Tell me what you can help me with",
                "Help me sort out my tasks for today",
                "What should you know about me to work better?",
            ],
            "generated": false,
        })
    } else {
        json!({
            // Роль у ботов записана от третьего лица («пишет тексты»), в «Я …» она не встаёт.
            "greeting": format!("Привет, я {}. Расскажи, с чем помочь, или выбери задачу ниже.", bot.name),
            "tasks": [
                "Расскажи, чем ты можешь мне помочь",
                "Помоги разобрать мои задачи на сегодня",
                "Что тебе важно знать обо мне, чтобы работать лучше?",
            ],
            "generated": false,
        })
    }
}

/// Ответ модели годится: приветствие и ровно три непустые задачи.
fn clean(answer: &Value) -> Option<Value> {
    let greeting = answer["greeting"].as_str().map(str::trim).filter(|g| !g.is_empty())?;
    let tasks: Vec<String> = answer["tasks"]
        .as_array()?
        .iter()
        .filter_map(Value::as_str)
        .map(str::trim)
        .filter(|t| !t.is_empty())
        .map(|t| service::take_chars(t, TASK_LIMIT))
        .take(3)
        .collect();
    if tasks.len() < 3 {
        return None;
    }
    Some(json!({ "greeting": service::take_chars(greeting, GREETING_LIMIT), "tasks": tasks, "generated": true }))
}

async fn generate(app: &Arc<App>, bot: &Bot, english: bool) -> Result<Value, String> {
    let mut who = if english { format!("You are {}, {}.", bot.name, bot.role) } else { format!("Ты — {}, {}.", bot.name, bot.role) };
    if !bot.persona.trim().is_empty() {
        who.push(' ');
        who.push_str(bot.persona.trim());
    }
    let task = if english {
        "Write a greeting for the person who has just added you: one or two warm, informal sentences, no emoji, \
         saying how you will help. Then three starter tasks the person can give you right now: in their voice, \
         specific to your role, up to 70 characters each. Reply with JSON only: {\"greeting\": \"...\", \"tasks\": [\"...\", \"...\", \"...\"]}"
    } else {
        "Напиши приветствие человеку, который только что тебя добавил: одно-два предложения, тепло, на «ты», без эмодзи, \
         скажи, чем поможешь. Потом три стартовые задачи, которые он может дать тебе прямо сейчас: от его лица, конкретно \
         по твоей роли, каждая до 70 знаков. Ответ — только JSON: {\"greeting\": \"...\", \"tasks\": [\"...\", \"...\", \"...\"]}"
    };
    let reply = service::complete(app, bot, &who, task).await?;
    service::json_object(&reply).as_ref().and_then(clean).ok_or_else(|| "модель вернула не то".to_string())
}

/// Знакомство для пустого чата. Заготовку не сохраняем: подключится модель — построим настоящее.
pub async fn for_bot(app: &Arc<App>, bot: &Bot) -> Value {
    if let Some(saved) = bot.extra.get("intro").and_then(clean) {
        return saved;
    }
    let english = app.store.settings().ui_lang == "en";
    if service::can_serve(app, bot) {
        match generate(app, bot, english).await {
            Ok(intro) => {
                if let Err(e) = app.store.set_bot_extra(&bot.id, "intro", intro.clone()).await {
                    eprintln!("[intro] не сохранил знакомство: {e}");
                }
                return intro;
            }
            Err(e) => eprintln!("[intro] {}: {e}", bot.name),
        }
    }
    fallback(bot, english)
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::clean;

    #[test]
    fn an_intro_needs_a_greeting_and_three_tasks() {
        assert!(clean(&json!({ "greeting": "Привет", "tasks": ["a", "b", "c", "d"] })).is_some_and(|v| v["tasks"].as_array().unwrap().len() == 3));
        assert!(clean(&json!({ "greeting": "Привет", "tasks": ["a", " "] })).is_none());
        assert!(clean(&json!({ "greeting": "", "tasks": ["a", "b", "c"] })).is_none());
    }
}
