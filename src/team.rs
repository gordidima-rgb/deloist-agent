//! Команда одним файлом: экспорт и импорт ботов, навыков, групп и рутин.
//! Секреты, ключи и переписка в файл не попадают — только описание команды.

use std::collections::HashMap;
use std::sync::Arc;

use serde_json::{json, Value};

use crate::store::{now, Bot, Skill};
use crate::App;

const MAX_BOTS: usize = 50;
const MAX_SKILLS: usize = 100;
const MAX_ROOMS: usize = 30;
const MAX_ROUTINES: usize = 100;
/// Текст из чужого файла идёт в каждый запрос к модели: длинный обошёлся бы дорого.
const MAX_TEXT: usize = 20_000;

/// Поля из файла приводим к безопасному виду: файл описывает команду, а не настройки компьютера.
fn sanitized(item: &Value) -> Value {
    let mut input = item.clone();
    for key in ["persona", "body", "instructions", "prompt"] {
        if let Some(text) = item.get(key).and_then(Value::as_str) {
            input[key] = json!(text.chars().take(MAX_TEXT).collect::<String>());
        }
    }
    let color_ok = item.get("color").and_then(Value::as_str).is_some_and(|c| {
        c.len() == 7 && c.starts_with('#') && c[1..].chars().all(|d| d.is_ascii_hexdigit())
    });
    if !color_ok {
        if let Some(map) = input.as_object_mut() {
            map.remove("color");
        }
    }
    input
}

/// Всё, что можно перенести на другую машину. Имена вместо идентификаторов:
/// на новой машине они всё равно будут другими.
pub fn export(app: &App) -> Value {
    let bots = app.store.bots();
    let skills = app.store.skills();
    let rooms = app.store.rooms();
    let routines = app.store.routines();
    let name_of = |id: &str| bots.iter().find(|b| b.id == id).map(|b| b.name.clone()).unwrap_or_default();

    json!({
        "format": "deloist-team",
        "version": 1,
        "exportedAt": now(),
        "skills": skills.iter().map(|s| json!({
            "name": s.name, "description": s.description, "body": s.body,
        })).collect::<Vec<_>>(),
        "bots": bots.iter().map(|b| json!({
            "name": b.name, "role": b.role, "emoji": b.emoji, "color": b.color,
            "persona": b.persona, "approval": b.approval, "skin": b.skin,
            "skills": b.skills.iter().filter_map(|id| skills.iter().find(|s| &s.id == id).map(|s| s.name.clone())).collect::<Vec<_>>(),
        })).collect::<Vec<_>>(),
        "rooms": rooms.iter().map(|r| json!({
            "name": r.name, "emoji": r.emoji, "color": r.color, "instructions": r.instructions, "mode": r.mode,
            "members": r.bot_ids.iter().map(|id| name_of(id)).filter(|n| !n.is_empty()).collect::<Vec<_>>(),
            "lead": name_of(&r.lead),
        })).collect::<Vec<_>>(),
        "routines": routines.iter().map(|r| json!({
            "bot": name_of(&r.bot_id), "title": r.title, "prompt": r.prompt,
            "everyMinutes": r.every_minutes, "at": r.at, "days": r.days,
            "windowFrom": r.window_from, "windowTo": r.window_to, "quiet": r.quiet,
        })).collect::<Vec<_>>(),
    })
}

/// Импорт: боты и навыки создаются заново, рутины приходят выключенными.
pub async fn import(app: &Arc<App>, team: &Value) -> Result<Value, String> {
    if team.get("format").and_then(Value::as_str) != Some("deloist-team") {
        return Err("это не файл команды Deloist".into());
    }

    let empty = Vec::new();
    let skill_items = team.get("skills").and_then(Value::as_array).unwrap_or(&empty);
    let bot_items = team.get("bots").and_then(Value::as_array).unwrap_or(&empty);
    let room_items = team.get("rooms").and_then(Value::as_array).unwrap_or(&empty);
    let routine_items = team.get("routines").and_then(Value::as_array).unwrap_or(&empty);
    if bot_items.len() > MAX_BOTS {
        return Err(format!("в файле больше {MAX_BOTS} ботов"));
    }
    if bot_items.is_empty() {
        return Err("в файле нет ни одного бота".into());
    }

    // Навыки: существующий с тем же именем не дублируем.
    let mut skill_ids: HashMap<String, String> = HashMap::new();
    for skill in app.store.skills() {
        skill_ids.insert(skill.name.clone(), skill.id);
    }
    let mut made_skills = 0;
    for item in skill_items.iter().take(MAX_SKILLS) {
        let name = item.get("name").and_then(Value::as_str).unwrap_or_default().trim();
        if name.is_empty() || skill_ids.contains_key(name) {
            continue;
        }
        let skill: Skill = app.store.add_skill(&sanitized(item)).await.map_err(|e| e.to_string())?;
        skill_ids.insert(skill.name.clone(), skill.id);
        made_skills += 1;
    }

    // Боты: имена в файле уникальны, иначе непонятно, кого брать в группу.
    let mut bot_ids: HashMap<String, String> = HashMap::new();
    let mut made_bots = 0;
    for item in bot_items {
        let name = item.get("name").and_then(Value::as_str).unwrap_or_default().trim();
        if name.is_empty() || bot_ids.contains_key(name) {
            continue;
        }
        let mut input = sanitized(item);
        // Чем отвечать, решает хозяин компьютера: чужой файл не запускает его программы и не тратит его деньги.
        input["driver"] = json!("");
        input["model"] = json!("");
        if let Some(skills) = item.get("skills").and_then(Value::as_array) {
            let ids: Vec<String> = skills
                .iter()
                .filter_map(Value::as_str)
                .filter_map(|name| skill_ids.get(name).cloned())
                .collect();
            input["skills"] = json!(ids);
        }
        // Импортированный бот по умолчанию спрашивает разрешение, как бы он ни был настроен дома.
        input["approval"] = json!("ask");
        let bot: Bot = app.store.add_bot(&input).await.map_err(|e| e.to_string())?;
        bot_ids.insert(bot.name.clone(), bot.id);
        made_bots += 1;
    }

    let mut made_rooms = 0;
    for item in room_items.iter().take(MAX_ROOMS) {
        let name = item.get("name").and_then(Value::as_str).unwrap_or_default().trim();
        if name.is_empty() {
            continue;
        }
        let members: Vec<String> = item
            .get("members")
            .and_then(Value::as_array)
            .map(|list| list.iter().filter_map(Value::as_str).filter_map(|n| bot_ids.get(n).cloned()).collect())
            .unwrap_or_default();
        if members.is_empty() {
            continue;
        }
        let mut input = sanitized(item);
        input["botIds"] = json!(members);
        let lead_name = item.get("lead").and_then(Value::as_str).unwrap_or_default();
        input["lead"] = json!(bot_ids.get(lead_name).cloned().unwrap_or_default());
        app.store.add_room(&input).await.map_err(|e| e.to_string())?;
        made_rooms += 1;
    }

    // Рутины приезжают выключенными: чужое расписание не должно начать работать само.
    let mut made_routines = 0;
    for item in routine_items.iter().take(MAX_ROUTINES) {
        let bot_name = item.get("bot").and_then(Value::as_str).unwrap_or_default();
        let Some(bot_id) = bot_ids.get(bot_name).cloned() else {
            continue;
        };
        let prompt = item.get("prompt").and_then(Value::as_str).unwrap_or_default().trim();
        if prompt.is_empty() {
            continue;
        }
        let mut input = sanitized(item);
        input["botId"] = json!(bot_id);
        input["enabled"] = json!(false);
        // Вебхук включает только сам человек: адрес из чужого файла не должен появиться незаметно.
        input["hook"] = json!(false);
        app.store.add_routine(&input).await.map_err(|e| e.to_string())?;
        made_routines += 1;
    }

    Ok(json!({
        "bots": made_bots, "skills": made_skills, "rooms": made_rooms, "routines": made_routines,
    }))
}
