//! Изменения участника команды карточкой: `propose_bot_change` и `propose_routine`
//! (схема: notes/agent-team-tools.md, «Задача D»).
//!
//! Тот же механизм, что у `propose_team`: предложение лежит в `offers`, окно показывает карточку
//! «Применить / Не надо», принять её может только человек (`guard::human_only`: `/api/bots/{id}/offers/…`).
//! По «Применить» программа записывает ровно то, что сохранено в предложении, а не повторный ввод агента.
//! Права (`approval`, `browserSites`, драйвер, модель, команды запуска, папки, лимиты) в аргументах
//! не читаются вообще: у этих инструментов нет для них места.

use std::sync::Arc;

use serde::Serialize;
use serde_json::{json, Value};
use tokio_util::sync::CancellationToken;

use crate::offers::Offer;
use crate::store::{uid, Bot, Room};
use crate::team_proposal::{await_decision, clean_line, hidden_in_instructions, is_bidi_control, is_format_char, same, MAX_INSTRUCTIONS, MAX_ROLE};
use crate::App;

#[derive(Clone, Debug, Serialize)]
pub struct Edit {
    pub before: String,
    pub after: String,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RoomRef {
    pub id: String,
    pub name: String,
    /// Кто станет ведущим, если уходит нынешний. Пусто — ведущий не меняется.
    pub new_lead: String,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BotChange {
    pub host_id: String,
    pub host_name: String,
    pub target_id: String,
    pub target_name: String,
    pub role: Option<Edit>,
    pub instructions: Option<Edit>,
    pub remove_from: Option<RoomRef>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RoutinePlan {
    pub host_id: String,
    pub host_name: String,
    pub target_id: String,
    pub target_name: String,
    pub title: String,
    pub prompt: String,
    /// Ввод для `Store::add_routine_capped`: ровно он будет записан.
    pub input: Value,
    /// «по будням в 09:00».
    pub schedule: String,
    /// «Следующий запуск: …»: считается в момент предложения, в сравнении карточек не участвует.
    #[serde(skip)]
    pub next: String,
    /// У бота «Свобода действий»: рутина выполнится без вопросов.
    pub free_mode: bool,
    /// Тихая рутина: её запуски не попадают в историю бота, человек видит только итог.
    pub quiet: bool,
}

#[derive(Clone, Debug, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Change {
    BotChange(BotChange),
    Routine(RoutinePlan),
}

impl Change {
    /// Одинаковое содержание — одна и та же карточка.
    pub fn dedupe_key(&self) -> String {
        serde_json::to_string(self).unwrap_or_default()
    }

    /// Так предложение видит окно. Тип события тот же, что у карточки команды: окно отличает по `kind`.
    pub fn event_json(&self, offer: &Offer) -> Value {
        match self {
            Change::BotChange(c) => json!({
                "id": offer.id,
                "hostBotId": offer.host_bot_id,
                "kind": "bot_change",
                "hostName": c.host_name,
                "target": { "id": c.target_id, "name": c.target_name },
                "role": c.role,
                "instructions": c.instructions,
                "removeFrom": c.remove_from,
                "at": offer.at,
            }),
            Change::Routine(r) => json!({
                "id": offer.id,
                "hostBotId": offer.host_bot_id,
                "kind": "routine",
                "hostName": r.host_name,
                "target": { "id": r.target_id, "name": r.target_name },
                "title": r.title,
                "prompt": r.prompt,
                "schedule": r.schedule,
                "next": r.next,
                "freeMode": r.free_mode,
                "quiet": r.quiet,
                "at": offer.at,
            }),
        }
    }
}

/// Бот по имени среди тех, с кем `me` состоит в общей команде (и сам `me`, если он разрешён). Остальных не ищем и не выдаём:
/// по ответу нельзя узнать, есть ли бот с таким именем вне команды.
fn find_bot(app: &App, me: &Bot, name: &str, allow_self: bool) -> Result<Bot, String> {
    let wanted = clean_line(name);
    if wanted.is_empty() {
        return Err("не указан бот: нужно имя участника команды".into());
    }
    app.store
        .bots()
        .into_iter()
        .filter(|b| (allow_self && b.id == me.id) || (b.id != me.id && !shared_rooms(app, &me.id, &b.id).is_empty()))
        .find(|b| same(&b.name, &wanted))
        .ok_or_else(|| format!("в твоей команде нет бота «{wanted}»: он не в твоей команде или такого бота нет. Изменить можно только участника команды, где состоишь и ты (команду собирай через propose_team)"))
}

/// Команды (не архивные), где состоят оба бота.
fn shared_rooms(app: &App, a: &str, b: &str) -> Vec<Room> {
    app.store.rooms().into_iter().filter(|r| !r.archived && r.bot_ids.iter().any(|id| id == a) && r.bot_ids.iter().any(|id| id == b)).collect()
}

fn no_hidden(text: &str) -> bool {
    !text.chars().any(|c| is_format_char(c) || is_bidi_control(c))
}

/// Что-то из того, что агент мог бы подсунуть, но чего эти инструменты не принимают.
fn text_arg(args: &Value, key: &str) -> Result<Option<String>, String> {
    match args.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(s)) => Ok(Some(s.trim().to_string())),
        Some(_) => Err(format!("{key}: нужна строка")),
    }
}

// ---------------------------------------------------------------------------
// propose_bot_change
// ---------------------------------------------------------------------------

pub fn plan_bot_change(app: &App, bot: &Bot, args: &Value) -> Result<BotChange, String> {
    let name = text_arg(args, "bot")?.unwrap_or_default();
    if same(&name, &bot.name) {
        return Err("себя этим инструментом не правят: предложи изменение участнику своей команды".into());
    }
    let target = find_bot(app, bot, &name, false)?;
    let rooms = shared_rooms(app, &bot.id, &target.id);

    let role = match text_arg(args, "role")? {
        None => None,
        Some(raw) => {
            let after = clean_line(&raw);
            if after.is_empty() || after.chars().count() > MAX_ROLE || after.chars().any(char::is_control) || !no_hidden(&after) {
                return Err(format!("role: от 1 до {MAX_ROLE} знаков, одной строкой, без невидимых символов"));
            }
            (after != target.role).then(|| Edit { before: target.role.clone(), after })
        }
    };
    let instructions = match text_arg(args, "instructions")? {
        None => None,
        Some(after) => {
            if after.is_empty() || after.chars().count() > MAX_INSTRUCTIONS {
                return Err(format!("instructions: от 1 до {MAX_INSTRUCTIONS} знаков (это новый текст целиком)"));
            }
            if after.chars().any(hidden_in_instructions) {
                return Err("instructions: невидимые знаки (нулевой ширины, смены направления текста и т. п.) запрещены".into());
            }
            (after != target.persona.trim()).then(|| Edit { before: target.persona.clone(), after })
        }
    };
    let remove_from = match text_arg(args, "removeFromRoom")? {
        None => None,
        Some(room_name) if room_name.is_empty() => None,
        Some(room_name) => {
            let room = rooms.iter().find(|r| same(&r.name, &room_name)).ok_or_else(|| format!("removeFromRoom: «{room_name}» — не команда, где вы с «{}» вместе", target.name))?;
            let new_lead = if room.lead == target.id {
                room.bot_ids.iter().find(|id| **id != target.id).and_then(|id| app.store.bot(id)).map(|b| b.name).unwrap_or_default()
            } else {
                String::new()
            };
            Some(RoomRef { id: room.id.clone(), name: room.name.clone(), new_lead })
        }
    };
    if role.is_none() && instructions.is_none() && remove_from.is_none() {
        return Err("нечего менять: укажи role, instructions или removeFromRoom (и они должны отличаться от нынешних)".into());
    }
    Ok(BotChange { host_id: bot.id.clone(), host_name: bot.name.clone(), target_id: target.id, target_name: target.name, role, instructions, remove_from })
}

// ---------------------------------------------------------------------------
// propose_routine
// ---------------------------------------------------------------------------

pub fn plan_routine(app: &App, bot: &Bot, args: &Value) -> Result<RoutinePlan, String> {
    let name = text_arg(args, "bot")?.unwrap_or_default();
    let target = if name.is_empty() { bot.clone() } else { find_bot(app, bot, &name, true)? };
    let title = clean_line(&text_arg(args, "title")?.unwrap_or_default());
    if title.is_empty() || title.chars().count() > crate::routine_tools::MAX_TITLE || title.chars().any(char::is_control) || !no_hidden(&title) {
        return Err(format!("title: от 1 до {} знаков, без невидимых символов", crate::routine_tools::MAX_TITLE));
    }
    let prompt = text_arg(args, "prompt")?.unwrap_or_default();
    if prompt.is_empty() || prompt.chars().count() > crate::routine_tools::MAX_PROMPT {
        return Err(format!("prompt: полная постановка на каждый запуск, до {} знаков", crate::routine_tools::MAX_PROMPT));
    }
    if prompt.chars().any(is_bidi_control) {
        return Err("prompt: знаки смены направления текста запрещены".into());
    }
    let schedule = args.get("schedule").filter(|v| v.is_object()).ok_or("schedule: нужен объект {at, days} или {every_minutes, window_from, window_to}")?;
    // Расписание разбирает тот же код, что у create_routine.
    let mut input = crate::routine_tools::schedule_patch(schedule)?;
    let has_at = input.get("at").and_then(Value::as_str).is_some_and(|a| !a.is_empty());
    if !has_at && !input.contains_key("everyMinutes") {
        return Err("schedule: не сказано когда: укажи at (время ЧЧ:ММ) или every_minutes".into());
    }
    crate::routine_tools::check_window(input.get("windowFrom").and_then(Value::as_str).unwrap_or_default(), input.get("windowTo").and_then(Value::as_str).unwrap_or_default())?;
    if app.store.routines().len() >= crate::routine_tools::MAX_ROUTINES {
        return Err(format!("уже {} рутин, новые не создаются", crate::routine_tools::MAX_ROUTINES));
    }
    input.insert("botId".into(), json!(target.id));
    input.insert("title".into(), json!(title));
    input.insert("prompt".into(), json!(prompt));
    input.insert("goalId".into(), json!(""));
    input.insert("enabled".into(), json!(true));
    input.insert("quiet".into(), json!(true));
    let quiet = input.get("quiet").and_then(Value::as_bool).unwrap_or(false);
    let input = Value::Object(input);
    let preview = crate::store::Store::preview_routine(&input);
    Ok(RoutinePlan {
        host_id: bot.id.clone(),
        host_name: bot.name.clone(),
        target_name: target.name.clone(),
        target_id: target.id,
        title,
        prompt,
        input,
        schedule: crate::schedule::describe(&preview),
        next: crate::schedule::describe_next(&preview),
        free_mode: target.approval == "auto",
        quiet,
    })
}

// ---------------------------------------------------------------------------
// Показ и ожидание
// ---------------------------------------------------------------------------

/// Показать карточку и ждать решения: общий путь обоих инструментов.
pub async fn propose(app: &Arc<App>, bot: &Bot, change: Change, cancel: &CancellationToken) -> Result<String, String> {
    // Такая же карточка уже ждёт ответа: второй не показываем, ждём то же решение.
    let (offer_id, rx) = match app.offers.find_change(&bot.id, &change) {
        Some(id) => {
            let rx = app.offers.open_decision(&id);
            (id, rx)
        }
        None => {
            let offer = Offer {
                id: uid("offer"),
                host_bot_id: bot.id.clone(),
                specialist_id: String::new(),
                reason: String::new(),
                task: String::new(),
                at: crate::store::now(),
                team: None,
                change: Some(change.clone()),
            };
            let rx = app.offers.open_decision(&offer.id);
            app.offers.add(offer.clone());
            app.turns.emit(json!({ "type": "team_proposal", "botId": bot.id, "proposal": change.event_json(&offer) }));
            (offer.id, rx)
        }
    };
    let silent = match change {
        Change::BotChange(_) => "человек не ответил на карточку: ничего не изменено",
        Change::Routine(_) => "человек не ответил на карточку: рутина не создана",
    };
    await_decision(app, &bot.id, &offer_id, rx, cancel, silent).await
}

pub async fn propose_bot_change(app: &Arc<App>, bot: &Bot, args: &Value, cancel: &CancellationToken) -> Result<String, String> {
    let plan = plan_bot_change(app, bot, args)?;
    propose(app, bot, Change::BotChange(plan), cancel).await
}

pub async fn propose_routine(app: &Arc<App>, bot: &Bot, args: &Value, cancel: &CancellationToken) -> Result<String, String> {
    let plan = plan_routine(app, bot, args)?;
    propose(app, bot, Change::Routine(plan), cancel).await
}

// ---------------------------------------------------------------------------
// «Применить»
// ---------------------------------------------------------------------------

/// Ровно то, что сохранено в предложении. Итог получает и ждущий инструмент.
pub async fn accept(app: &Arc<App>, offer: &Offer, change: &Change) -> Result<String, String> {
    let result = match change {
        Change::BotChange(c) => apply_bot_change(app, c).await,
        Change::Routine(r) => apply_routine(app, r).await,
    };
    match &result {
        Ok(text) => app.offers.decide(&offer.id, text.clone()),
        Err(message) => app.offers.decide(&offer.id, format!("Не получилось применить: {message}")),
    }
    app.turns.emit(json!({ "type": "state_changed" }));
    result
}

async fn apply_bot_change(app: &Arc<App>, c: &BotChange) -> Result<String, String> {
    // Мир мог поменяться, пока карточка висела: человек видел «было → стало», а не то, что там теперь.
    let target = app.store.bot(&c.target_id).ok_or_else(|| format!("бота «{}» больше нет", c.target_name))?;
    // Предлагал участник команды: если бота убрали из команды, пока карточка висела, права на правку он потерял.
    if app.store.bot(&c.host_id).is_none() || shared_rooms(app, &c.host_id, &c.target_id).is_empty() {
        return Err(format!("«{}» больше не в одной команде с «{}»: предложи заново", c.host_name, target.name));
    }
    if c.role.as_ref().is_some_and(|e| e.before != target.role) || c.instructions.as_ref().is_some_and(|e| e.before != target.persona) {
        return Err(format!("у «{}» роль или инструкции уже изменились, пока карточка висела: предложи заново", target.name));
    }
    let room = match &c.remove_from {
        Some(r) => {
            let room = app.store.room(&r.id).ok_or_else(|| format!("команды «{}» больше нет", r.name))?;
            if target.orchestrator {
                return Err(format!("«{}» ведёт команду «{}»: оркестратора из команды не убирают", target.name, room.name));
            }
            if !room.bot_ids.contains(&c.target_id) || !room.bot_ids.contains(&c.host_id) {
                return Err(format!("«{}» уже не состоит в команде «{}» вместе с «{}»", target.name, room.name, c.host_name));
            }
            let new_lead = if room.lead == c.target_id {
                room.bot_ids.iter().find(|id| **id != c.target_id).and_then(|id| app.store.bot(id)).map(|b| b.name).unwrap_or_default()
            } else {
                String::new()
            };
            if new_lead != r.new_lead {
                return Err(format!("состав команды «{}» изменился, пока карточка висела: предложи заново", room.name));
            }
            Some(room)
        }
        None => None,
    };

    // Только эти два поля: права, драйвер, модель, папки и лимиты в патч не попадают.
    let mut patch = serde_json::Map::new();
    if let Some(e) = &c.role {
        patch.insert("role".into(), json!(e.after));
    }
    if let Some(e) = &c.instructions {
        patch.insert("persona".into(), json!(e.after));
    }
    let mut done = Vec::new();
    if !patch.is_empty() {
        app.store.update_bot(&c.target_id, &Value::Object(patch)).await.map_err(|e| e.to_string())?.ok_or("бот уже удалён")?;
        if c.role.is_some() {
            done.push("роль".to_string());
        }
        if c.instructions.is_some() {
            done.push("инструкции".to_string());
        }
    }
    if let Some(room) = room {
        let ids: Vec<&String> = room.bot_ids.iter().filter(|id| **id != c.target_id).collect();
        app.store.update_room(&room.id, &json!({ "botIds": ids })).await.map_err(|e| e.to_string())?;
        done.push(format!("убран из команды «{}»", room.name));
    }
    Ok(format!("Человек согласился. У «{}» изменено: {}.", c.target_name, done.join(", ")))
}

async fn apply_routine(app: &Arc<App>, r: &RoutinePlan) -> Result<String, String> {
    let target = app.store.bot(&r.target_id).ok_or_else(|| format!("бота «{}» больше нет", r.target_name))?;
    // Карточка предупреждала про «Свободу действий» (или нет): если режим сменился, она устарела.
    if (target.approval == "auto") != r.free_mode {
        return Err(format!("у «{}» сменился режим подтверждений, пока карточка висела: предложи заново", target.name));
    }
    if r.target_id != r.host_id && (app.store.bot(&r.host_id).is_none() || shared_rooms(app, &r.host_id, &r.target_id).is_empty()) {
        return Err(format!("«{}» больше не в одной команде с «{}»: предложи заново", r.host_name, target.name));
    }
    // Рутина для бота без «Свободы действий» не должна стать смелее него позже: пол осторожности — его режим сейчас.
    // Пустым он остаётся только у «решай сам» (карточка про это предупреждала).
    let mut input = r.input.clone();
    if target.approval != "auto" {
        input["approvalFloor"] = json!(target.approval);
    }
    let made = app
        .store
        .add_routine_capped(&input, crate::routine_tools::MAX_ROUTINES)
        .await
        .map_err(|e| e.to_string())?
        .ok_or_else(|| format!("уже {} рутин, новые не создаются", crate::routine_tools::MAX_ROUTINES))?;
    Ok(format!("Человек согласился. Рутина создана и включена: «{}» у «{}», {}. {} Номер: {}.", made.title, target.name, crate::schedule::describe(&made), crate::schedule::describe_next(&made), made.id))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn routine(next: &str) -> Change {
        Change::Routine(RoutinePlan {
            host_name: "Х".into(),
            target_id: "b".into(),
            target_name: "Б".into(),
            title: "т".into(),
            prompt: "п".into(),
            input: json!({ "at": "09:00" }),
            schedule: "каждый день в 09:00".into(),
            next: next.into(),
            free_mode: false,
            quiet: true,
            host_id: "h".into(),
        })
    }

    #[test]
    fn the_next_run_does_not_make_a_repeat_look_like_a_new_card() {
        assert_eq!(routine("пн 01.01 в 09:00").dedupe_key(), routine("вт 02.01 в 09:00").dedupe_key());
    }
}
