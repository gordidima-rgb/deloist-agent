//! Навыки, которые бот учит сам: его поправили, он нашёл рабочий способ или навык подвёл.
//! Режим задаёт настройка `skillLearning`: `auto` — сразу и с отменой, `ask` — через карточку,
//! `off` — инструмента нет вовсе.

use std::sync::Arc;

use serde_json::{json, Value};

use crate::store::{Bot, Settings};
use crate::App;

pub fn learning_on(settings: &Settings) -> bool {
    settings.skill_learning != "off"
}

/// Сохранение навыка спрашивает человека в режиме «Сначала спросить» и у строгого бота.
pub fn save_needs_approval(settings: &Settings, bot: &Bot) -> bool {
    settings.skill_learning == "ask" || bot.approval == "strict"
}

pub fn save_skill_def() -> Value {
    json!({
        "type": "function",
        "function": {
            "name": "save_skill",
            "description": "Сохранить или исправить свой навык — способ, как делать повторяющуюся задачу. \
                Зови, когда человек поправил, как надо, когда нашёлся рабочий способ после проб или когда навык подвёл. \
                Тот же name правит навык, новый name создаёт его.",
            "parameters": {
                "type": "object",
                "properties": {
                    "name": { "type": "string", "description": "Короткое название, до 60 знаков: «Отчёт по продажам Ozon»." },
                    "description": { "type": "string", "description": "Когда применять, одной строкой." },
                    "body": { "type": "string", "description": "Шаги и правила, коротко. Без разовых фактов и паролей." },
                },
                "required": ["name", "body"],
            },
        },
    })
}

pub fn read_skill_def() -> Value {
    json!({
        "type": "function",
        "function": {
            "name": "read_skill",
            "description": "Прочитать свой навык целиком по названию из списка «Твои навыки».",
            "parameters": {
                "type": "object",
                "properties": { "name": { "type": "string", "description": "Название навыка." } },
                "required": ["name"],
            },
        },
    })
}

fn arg<'a>(args: &'a Value, key: &str) -> &'a str {
    args.get(key).and_then(Value::as_str).unwrap_or_default()
}

pub async fn save(app: &Arc<App>, bot: &Bot, args: &Value) -> Result<String, String> {
    if !learning_on(&app.store.settings()) {
        return Err("самообучение навыкам выключено в настройках".into());
    }
    let (skill, created) = app.store.learn_skill(&bot.id, arg(args, "name"), arg(args, "description"), arg(args, "body")).await?;
    app.turns.emit(json!({
        "type": "skill_learned", "botId": bot.id, "skillId": skill.id, "name": skill.name, "created": created,
    }));
    Ok(if created {
        format!("Навык «{}» создан и подключён тебе.", skill.name)
    } else {
        format!("Навык «{}» исправлен.", skill.name)
    })
}

pub fn read(app: &App, bot: &Bot, args: &Value) -> Result<String, String> {
    let wanted = arg(args, "name").trim().to_lowercase();
    let conv = app.turns.conv_of(&bot.id);
    let skills = match conv.as_deref().and_then(|c| c.strip_prefix("room:")) {
        Some(room_id) => app.store.skills_for_bot_in_room(&bot.id, room_id),
        None => app.store.skills_for_bot(&bot.id),
    };
    skills
        .into_iter()
        .find(|s| s.name.to_lowercase() == wanted)
        .map(|s| format!("# {}\n{}", s.name, s.body))
        .ok_or_else(|| format!("у тебя нет навыка «{}»", arg(args, "name")))
}
