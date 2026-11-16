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
const MAX_WORKFLOWS: usize = 50;
/// Текст из чужого файла идёт в каждый запрос к модели: длинный обошёлся бы дорого.
const MAX_TEXT: usize = 20_000;

/// Какие поля берём из файла (ровно то, что пишет `export`). Всё остальное — `approvalFloor`, `goalId`,
/// `kind`, каналы, сайты, папки, команды — из чужого файла не читается вообще.
const BOT_KEYS: &[&str] = &["name", "role", "emoji", "color", "persona", "skin", "skills"];
const SKILL_KEYS: &[&str] = &["name", "description", "body"];
const ROOM_KEYS: &[&str] = &["name", "emoji", "color", "instructions", "mode", "members", "lead"];
const ROUTINE_KEYS: &[&str] = &["bot", "title", "prompt", "everyMinutes", "at", "days", "windowFrom", "windowTo", "quiet"];

/// Поля из файла приводим к безопасному виду: файл описывает команду, а не настройки компьютера.
fn sanitized(item: &Value, allowed: &[&str]) -> Value {
    let mut input = json!({});
    for key in allowed {
        let Some(value) = item.get(*key) else { continue };
        input[*key] = match (value.as_str(), *key) {
            (Some(text), "persona" | "body" | "instructions" | "prompt") => json!(text.chars().take(MAX_TEXT).collect::<String>()),
            _ => value.clone(),
        };
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
    export_with(app, true)
}

/// `workflows: false` — для ключа программы (скрипт бота): графы процессов содержат постановки и адреса, их отдаёт только окно.
pub fn export_with(app: &App, workflows: bool) -> Value {
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
        "workflows": if workflows { export_workflows(app, &name_of) } else { Vec::new() },
        "routines": routines.iter().map(|r| json!({
            "bot": name_of(&r.bot_id), "title": r.title, "prompt": r.prompt,
            "everyMinutes": r.every_minutes, "at": r.at, "days": r.days,
            "windowFrom": r.window_from, "windowTo": r.window_to, "quiet": r.quiet,
        })).collect::<Vec<_>>(),
    })
}

/// Процессы без секретов, запусков и мест доставки: боты записаны именами, а не номерами.
fn export_workflows(app: &App, name_of: &dyn Fn(&str) -> String) -> Vec<Value> {
    let guard = app.workflows.lock();
    guard
        .items
        .iter()
        .map(|stored| {
            let mut graph = stored.wf.graph.clone();
            for node in &mut graph.nodes {
                if let Some(id) = node.fields.remove("bot_id") {
                    let name = name_of(id.as_str().unwrap_or_default());
                    if !name.is_empty() {
                        node.fields.insert("bot_name".into(), json!(name));
                    }
                }
                // Чат или канал человека — не часть команды.
                node.fields.remove("place_id");
            }
            json!({ "title": stored.wf.title, "graph": graph })
        })
        .collect()
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
        let skill: Skill = app.store.add_skill(&sanitized(item, SKILL_KEYS)).await.map_err(|e| e.to_string())?;
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
        let mut input = sanitized(item, BOT_KEYS);
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
        let mut input = sanitized(item, ROOM_KEYS);
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
        let mut input = sanitized(item, ROUTINE_KEYS);
        input["botId"] = json!(bot_id);
        input["enabled"] = json!(false);
        // Вебхук включает только сам человек: адрес из чужого файла не должен появиться незаметно.
        input["hook"] = json!(false);
        app.store.add_routine(&input).await.map_err(|e| e.to_string())?;
        made_routines += 1;
    }

    // Процессы приезжают выключенными и без вебхуков: адрес с секретом включает только сам человек.
    let mut made_workflows = 0;
    for item in team.get("workflows").and_then(Value::as_array).unwrap_or(&empty).iter().take(MAX_WORKFLOWS) {
        let title = item.get("title").and_then(Value::as_str).unwrap_or_default().trim();
        let Some(mut graph) = item.get("graph").and_then(|g| serde_json::from_value::<crate::workflows::Graph>(g.clone()).ok()) else { continue };
        if title.is_empty() {
            continue;
        }
        for node in &mut graph.nodes {
            node.fields.remove("place_id");
            node.fields.remove("bot_id");
            if let Some(name) = node.fields.remove("bot_name") {
                if let Some(id) = name.as_str().and_then(|n| bot_ids.get(n)) {
                    node.fields.insert("bot_id".into(), json!(id));
                }
            }
        }
        if crate::workflows::validate_graph(&graph).and_then(|()| crate::workflows::check_providers(app, &graph)).is_err() {
            continue;
        }
        let mut stored = crate::workflows::new_workflow(&title.chars().take(200).collect::<String>(), graph);
        stored.wf.enabled = false;
        let id = stored.wf.id.clone();
        {
            let mut guard = app.workflows.lock();
            guard.items.push(stored);
            guard.runs.insert(id.clone(), Vec::new());
            guard.save_wf(&id);
        }
        app.turns.emit(json!({ "type": "workflows", "workflow_id": id }));
        made_workflows += 1;
    }

    Ok(json!({
        "bots": made_bots, "skills": made_skills, "rooms": made_rooms, "routines": made_routines, "workflows": made_workflows,
    }))
}

/// Самый большой файл команды, который принимает заявка с ключа программы.
pub const MAX_REQUEST_BYTES: usize = 1024 * 1024;
const MAX_DEPTH: usize = 8;
const MAX_NAME: usize = 40;
const MAX_ROLE: usize = 80;
const MAX_SKILL_NAME: usize = 60;
const MAX_TITLE: usize = 80;

/// Невидимые знаки и управляющие символы (кроме переводов строк и табуляции) где угодно в файле:
/// они подделывают то, что человек увидит в карточке и в инструкциях бота.
fn hidden_text(value: &Value, depth: usize) -> Result<(), String> {
    if depth > MAX_DEPTH {
        return Err("файл вложен слишком глубоко".into());
    }
    match value {
        Value::String(text) => {
            if text.chars().any(|c| (c.is_control() && !matches!(c, '\n' | '\r' | '\t')) || crate::team_proposal::hidden_in_instructions(c)) {
                return Err("в файле есть управляющие или невидимые символы".into());
            }
            Ok(())
        }
        Value::Array(items) => items.iter().try_for_each(|item| hidden_text(item, depth + 1)),
        Value::Object(map) => map.iter().try_for_each(|(key, item)| hidden_text(&Value::String(key.clone()), depth + 1).and_then(|()| hidden_text(item, depth + 1))),
        _ => Ok(()),
    }
}

fn list_of<'a>(team: &'a Value, key: &str, max: usize) -> Result<&'a [Value], String> {
    match team.get(key) {
        None | Some(Value::Null) => Ok(&[]),
        Some(Value::Array(items)) if items.len() <= max => Ok(items),
        Some(Value::Array(_)) => Err(format!("«{key}»: в файле больше {max} записей")),
        Some(_) => Err(format!("«{key}»: нужен список")),
    }
}

const EXCERPT: usize = 200;
/// Персона или навык длиннее этого — предупреждение на карточке.
const LONG_TEXT: usize = 2000;
const MANY_BOTS: usize = 5;

/// Начало текста для карточки: одна строка, без лишних пробелов, с многоточием.
fn excerpt(raw: &str) -> String {
    let flat = crate::team_proposal::clean_line(raw);
    if flat.chars().count() > EXCERPT {
        format!("{}…", flat.chars().take(EXCERPT).collect::<String>())
    } else {
        flat
    }
}

/// Проверка файла команды для заявки: те же пределы, что у `import`, но лишнее не отрезается молча,
/// а отклоняется. Возвращает описание для карточки: только проверенные и подрезанные строки,
/// ровно то, что создаст `import`, с началом каждой персоны, навыка и рутины и общими размерами.
pub fn validate(app: &App, team: &Value) -> Result<Value, String> {
    if serde_json::to_vec(team).map_or(true, |bytes| bytes.len() > MAX_REQUEST_BYTES) {
        return Err("файл команды больше 1 МБ".into());
    }
    if team.get("format").and_then(Value::as_str) != Some("deloist-team") {
        return Err("это не файл команды Deloist".into());
    }
    hidden_text(team, 0)?;
    let skill_items = list_of(team, "skills", MAX_SKILLS)?;
    let bot_items = list_of(team, "bots", MAX_BOTS)?;
    let room_items = list_of(team, "rooms", MAX_ROOMS)?;
    let routine_items = list_of(team, "routines", MAX_ROUTINES)?;
    if bot_items.is_empty() {
        return Err("в файле нет ни одного бота".into());
    }
    let text = |item: &Value, key: &str| item.get(key).and_then(Value::as_str).unwrap_or_default().trim().to_string();
    let object = |item: &Value, what: &str| if item.is_object() { Ok(()) } else { Err(format!("«{what}»: каждая запись должна быть объектом")) };
    // Имена и роли — одна строка без переводов строк и табуляции: на карточке и в данных они совпадают.
    let line = |value: &str, max: usize, what: &str| {
        if value.chars().count() > max || value.chars().any(char::is_control) {
            Err(format!("{what}: до {max} знаков в одну строку, без табуляции и переводов строк"))
        } else {
            Ok(())
        }
    };
    let chars = |value: &str| value.chars().count();
    let mut total = 0usize;
    let mut long = false;

    let mut existing_skills: std::collections::HashSet<String> = app.store.skills().into_iter().map(|s| s.name).collect();
    let mut skills = Vec::new();
    for item in skill_items {
        object(item, "skills")?;
        let name = text(item, "name");
        line(&name, MAX_SKILL_NAME, "имя навыка")?;
        if !name.is_empty() && existing_skills.insert(name.clone()) {
            let body = text(item, "body");
            let size = chars(&body).min(MAX_TEXT);
            total += size;
            long |= size > LONG_TEXT;
            skills.push(json!({ "name": name, "excerpt": excerpt(&body), "chars": size }));
        }
    }

    let mut bot_names: Vec<String> = Vec::new();
    let mut bots = Vec::new();
    for item in bot_items {
        object(item, "bots")?;
        let name = text(item, "name");
        if name.is_empty() {
            return Err(format!("у каждого бота нужно имя до {MAX_NAME} знаков в одну строку"));
        }
        line(&name, MAX_NAME, "имя бота")?;
        line(&text(item, "role"), MAX_ROLE, "роль бота")?;
        if bot_names.contains(&name) {
            continue;
        }
        bot_names.push(name.clone());
        let persona = text(item, "persona");
        let size = chars(&persona).min(MAX_TEXT);
        total += size;
        long |= size > LONG_TEXT;
        bots.push(json!({ "name": name, "role": text(item, "role"), "excerpt": excerpt(&persona), "chars": size }));
    }

    let mut rooms = Vec::new();
    for item in room_items {
        object(item, "rooms")?;
        let name = text(item, "name");
        line(&name, MAX_NAME, "имя команды")?;
        let members = item.get("members").and_then(Value::as_array).map_or(0, |list| list.iter().filter_map(Value::as_str).filter(|n| bot_names.iter().any(|b| b == n)).count());
        if !name.is_empty() && members > 0 {
            rooms.push(json!({ "name": name, "members": members }));
        }
    }

    let mut routines = Vec::new();
    for item in routine_items {
        object(item, "routines")?;
        let title = text(item, "title");
        line(&title, MAX_TITLE, "название рутины")?;
        let owner = text(item, "bot");
        let prompt = text(item, "prompt");
        if bot_names.contains(&owner) && !prompt.is_empty() {
            let size = chars(&prompt).min(MAX_TEXT);
            total += size;
            routines.push(json!({ "bot": owner, "title": title, "excerpt": excerpt(&prompt), "chars": size }));
        }
    }

    Ok(json!({
        "bots": bots, "skills": skills, "rooms": rooms, "routines": routines,
        "totalChars": total,
        "warnings": { "longText": long, "manyBots": bots.len() > MANY_BOTS },
        // Что импорт делает всегда, как бы ни был настроен файл: карточка говорит это прямо.
        "defaults": { "approval": "ask", "browserSites": 0, "folders": 0, "routinesEnabled": false },
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn imported_routines_keep_only_the_exported_fields() {
        let item = json!({
            "bot": "А", "title": "Т", "prompt": "П", "everyMinutes": 5, "at": "09:00",
            "enabled": true, "hook": true, "approvalFloor": "none", "goalId": "g1", "kind": "reminder", "channels": ["telegram"], "botId": "чужой",
        });
        let input = sanitized(&item, ROUTINE_KEYS);
        let keys: Vec<&str> = input.as_object().unwrap().keys().map(String::as_str).collect();
        assert!(keys.iter().all(|k| ROUTINE_KEYS.contains(k)), "{keys:?}");
        for banned in ["approvalFloor", "goalId", "kind", "channels", "hook", "enabled", "botId"] {
            assert!(input.get(banned).is_none(), "{banned} прошёл из чужого файла");
        }
    }

    #[test]
    fn imported_bots_do_not_carry_rights_fields() {
        let item = json!({ "name": "Б", "persona": "п", "approval": "auto", "browserSites": ["x"], "driver": "cli", "model": "m", "monthlyTokenLimit": 1, "color": "javascript:1" });
        let input = sanitized(&item, BOT_KEYS);
        for banned in ["approval", "browserSites", "driver", "model", "monthlyTokenLimit", "color"] {
            assert!(input.get(banned).is_none(), "{banned}");
        }
        assert_eq!(input["name"], "Б");
    }
}
