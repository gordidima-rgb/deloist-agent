//! Рутины из чата: бот смотрит, создаёт, меняет и удаляет расписания теми же правилами, что и интерфейс.
//! Вебхуки отсюда не заводятся: адрес с секретом включает только сам человек.

use std::sync::Arc;

use serde_json::{json, Map, Value};

use chrono::{Local, NaiveDate};
use crate::dates_ru;
use crate::store::{Bot, Routine};
use crate::{schedule, App};

/// Рутина — это чужое время и токены каждый день: стена, чтобы бот не засыпал список.
pub(crate) const MAX_ROUTINES: usize = 100;
pub(crate) const MAX_PROMPT: usize = 20_000;
pub(crate) const MAX_TITLE: usize = 80;
const MIN_EVERY_MINUTES: i64 = 5;

pub fn is_routine_tool(name: &str) -> bool {
    matches!(name, "list_routines" | "create_routine" | "update_routine" | "run_routine" | "delete_routine")
}

/// Схемы для модели.
pub fn defs() -> Vec<Value> {
    let function = |name: &str, description: &str, properties: Value, required: &[&str]| {
        let mut parameters = json!({ "type": "object", "properties": properties });
        if !required.is_empty() {
            parameters["required"] = json!(required);
        }
        json!({ "type": "function", "function": { "name": name, "description": description, "parameters": parameters } })
    };
    let schedule_fields = || {
        json!({
            "at": { "type": "string", "description": "Время запуска ЧЧ:ММ по часам человека: «09:00». Вместе с days или recurrence — по каким дням. Либо at, либо every_minutes." },
            "days": { "type": "array", "items": { "type": "integer" }, "description": "Дни недели, 1 — понедельник … 7 — воскресенье. Будни — [1,2,3,4,5]; один день — [3]. Пусто или не указано — каждый день." },
            "recurrence": { "type": "object", "description": "Правило повтора: {freq: daily|weekly|monthly|yearly, interval, weekdays:[1..7], monthday, nth (1..4 или -1 для monthly/yearly при одном дне недели), month (1..12 для yearly)}. Примеры: каждый вторник — {freq:weekly, weekdays:[2]}; 3-я суббота сентября — {freq:yearly, month:9, weekdays:[6], nth:3}; раз в год 5 марта — {freq:yearly, month:3, monthday:5}." },
            "start_date": { "type": "string", "description": "YYYY-MM-DD: не раньше этой даты (местное время)." },
            "once_at": { "type": "string", "description": "YYYY-MM-DD HH:MM местного времени, в будущем: запустить один раз, затем выключить. Пример: один раз 2 декабря 10:00." },
            "every_minutes": { "type": "integer", "description": "Повторять каждые N минут, не реже 5: час — 60. Вместо at." },
            "every_max_minutes": { "type": "integer", "description": "Для every_minutes: случайный интервал — каждый раз срок выпадает между every_minutes и этим числом (40 и 80 — от 40 до 80 мин), чтобы запуски не шли ровной сеткой, как у человека. 0 — ровный интервал." },
            "window_from": { "type": "string", "description": "Для every_minutes: работать только с этого часа, ЧЧ:ММ. Окно может переходить через полночь (22:00–07:00). Нужны оба поля окна." },
            "window_to": { "type": "string", "description": "Конец окна ЧЧ:ММ." },
        })
    };
    let with = |extra: Value| {
        let mut all = schedule_fields();
        if let (Some(all), Some(extra)) = (all.as_object_mut(), extra.as_object()) {
            all.extend(extra.clone());
        }
        all
    };
    vec![
        function(
            "list_routines",
            "Показать рутины (расписания ботов): номер, название, чья, когда идёт и когда следующий запуск.",
            json!({}),
            &[],
        ),
        function(
            "create_routine",
            "Создать рутину: в нужное время ты сам проснёшься и выполнишь prompt. \
             Примеры: каждый вторник; 3-я суббота сентября; один раз 2 декабря 10:00. \
             Называй человеку расписание и время следующего запуска из ответа. Без bot_id рутина твоя. \
             Обещаешь сделать что-то позже или регулярно — сначала поставь рутину (или шаг в свою через update_routine), иначе не обещай.",
            with(json!({
                "title": { "type": "string", "description": "Короткое название, до 80 символов." },
                "prompt": { "type": "string", "description": "Полная постановка на каждый запуск: рутина проснётся без этого разговора, «см. выше» не сработает. Можно не указывать, если задан goal_id." },
                "goal_id": { "type": "string", "description": "Номер цели из list_goals: тогда каждый запуск рутины — шаг к этой цели. Цель должна быть того же бота." },
                "bot_id": { "type": "string", "description": "Чей бот выполнит рутину (ID из list_bots). По умолчанию ты сам." },
                "enabled": { "type": "boolean", "description": "Включена ли. По умолчанию да." },
                "quiet": { "type": "boolean", "description": "Писать в чат, только если есть что сказать. По умолчанию да." },
                "catch_up": { "type": "string", "description": "Если пропущен запуск: «last» — выполнить последний, «skip» — пропустить. По умолчанию last." },
                "hook_events": { "type": "array", "items": { "type": "string" }, "description": "Для вебхука: события GitHub, на которые реагировать (push, pull_request.opened, issues, release.published). Пусто — любое." },
            })),
            &["title"],
        ),
        function(
            "update_routine",
            "Изменить рутину по номеру из list_routines. Передавай только то, что меняется. Чтобы перейти с at на every_minutes, \
             передай every_minutes — at снимется сам.",
            with(json!({
                "routine_id": { "type": "string", "description": "Номер рутины из list_routines." },
                "title": { "type": "string", "description": "Новое название." },
                "prompt": { "type": "string", "description": "Новая постановка целиком." },
                "goal_id": { "type": "string", "description": "Привязать рутину к цели (номер из list_goals) или пустая строка, чтобы отвязать." },
                "enabled": { "type": "boolean", "description": "Включить или выключить." },
                "quiet": { "type": "boolean", "description": "Только важное или писать всё." },
                "catch_up": { "type": "string", "description": "Если пропущен запуск: «last» или «skip»." },
                "hook_events": { "type": "array", "items": { "type": "string" }, "description": "Для вебхука: события GitHub, на которые реагировать (push, pull_request.opened). Пусто — любое." },
            })),
            &["routine_id"],
        ),
        function(
            "run_routine",
            "Запустить рутину сейчас, вне расписания: бот рутины выполнит её prompt как обычный запуск. Человек подтверждает карточкой. Напоминания так не запускаются.",
            json!({ "routine_id": { "type": "string", "description": "Номер рутины из list_routines." } }),
            &["routine_id"],
        ),
        function(
            "delete_routine",
            "Удалить рутину по номеру из list_routines. Человек подтверждает каждое удаление. Чтобы просто приостановить, выключи её через update_routine.",
            json!({ "routine_id": { "type": "string", "description": "Номер рутины из list_routines." } }),
            &["routine_id"],
        ),
    ]
}

/// Нужна ли карточка. Удаление спрашивает всегда. Остальные правки — по режиму бота, как у `update_bot`;
/// а в ходе без человека (по расписанию, вебхуком, через делегирование) — тоже всегда: рутина, которая
/// молча плодит рутины, ночью не остановится.
pub fn needs_approval(app: &App, bot: &Bot, name: &str, args: &Value) -> bool {
    match name {
        "list_routines" => false,
        "delete_routine" => true,
        // Чужое расписание — отдельный случай: карточка и в режиме «решай сам».
        _ => bot.approval != "auto" || app.turns.is_unattended(&bot.id) || for_another_bot(app, bot, name, args),
    }
}

/// Рутина, которую создают или правят у другого бота.
fn for_another_bot(app: &App, bot: &Bot, name: &str, args: &Value) -> bool {
    match name {
        "create_routine" => {
            let wanted = text(args, "bot_id");
            !wanted.is_empty() && wanted != bot.id && app.store.bot(wanted).is_some()
        }
        _ => app.store.routine(text(args, "routine_id")).is_some_and(|r| r.bot_id != bot.id),
    }
}

pub(crate) fn text<'a>(args: &'a Value, key: &str) -> &'a str {
    args.get(key).and_then(Value::as_str).unwrap_or_default().trim()
}

pub(crate) fn flag(args: &Value, key: &str) -> Result<Option<bool>, String> {
    match args.get(key).filter(|v| !v.is_null()) {
        None => Ok(None),
        Some(v) => v.as_bool().map(Some).ok_or_else(|| format!("поле {key} должно быть true или false")),
    }
}

/// День недели числом или словом: 3, "3", "ср", "wed".
fn day_of(value: &Value) -> Option<i64> {
    if let Some(n) = value.as_i64() {
        return (1..=7).contains(&n).then_some(n);
    }
    let word = value.as_str()?.trim().to_lowercase();
    if let Ok(n) = word.parse::<i64>() {
        return (1..=7).contains(&n).then_some(n);
    }
    const NAMES: [[&str; 2]; 7] = [["пн", "mon"], ["вт", "tue"], ["ср", "wed"], ["чт", "thu"], ["пт", "fri"], ["сб", "sat"], ["вс", "sun"]];
    NAMES.iter().position(|names| names.iter().any(|n| word.starts_with(n))).map(|i| i as i64 + 1)
}

/// Расписание из аргументов бота в поля, которые понимает хранилище. Пустой результат — расписание не трогали.
pub(crate) fn schedule_patch(args: &Value) -> Result<Map<String, Value>, String> {
    let mut out = Map::new();
    if let Some(at) = args.get("at").filter(|v| !v.is_null()) {
        out.insert("at".into(), json!(at.as_str().ok_or("поле at должно быть строкой ЧЧ:ММ")?.trim()));
    }
    if let Some(list) = args.get("days").filter(|v| !v.is_null()) {
        let items = list.as_array().ok_or("поле days должно быть списком чисел 1–7")?;
        let days = items.iter().map(|d| day_of(d).ok_or_else(|| format!("день недели {d} не понят: нужно 1 (пн) … 7 (вс)"))).collect::<Result<Vec<_>, _>>()?;
        out.insert("days".into(), json!(days));
    }
    if let Some(every) = args.get("every_minutes").filter(|v| !v.is_null()) {
        let minutes = every.as_i64().ok_or("every_minutes должно быть целым числом минут")?;
        if minutes < MIN_EVERY_MINUTES {
            return Err(format!("чаще раза в {MIN_EVERY_MINUTES} минут нельзя"));
        }
        out.insert("everyMinutes".into(), json!(minutes));
    }
    if let Some(max) = args.get("every_max_minutes").filter(|v| !v.is_null()) {
        let max = max.as_i64().filter(|m| *m >= 0).ok_or("every_max_minutes должно быть целым числом минут, 0 — без случайности")?;
        let low = out.get("everyMinutes").and_then(Value::as_i64);
        if max != 0 && low.is_some_and(|low| max <= low) {
            return Err("every_max_minutes должно быть больше every_minutes".into());
        }
        out.insert("everyMaxMinutes".into(), json!(max));
    }
    for (key, field) in [("window_from", "windowFrom"), ("window_to", "windowTo")] {
        if let Some(v) = args.get(key).filter(|v| !v.is_null()) {
            out.insert(field.into(), json!(v.as_str().ok_or_else(|| format!("поле {key} должно быть строкой ЧЧ:ММ"))?.trim()));
        }
    }
    if let Some(v) = args.get("recurrence").filter(|v| !v.is_null()) {
        let rec: dates_ru::Recurrence = serde_json::from_value(v.clone()).map_err(|e| format!("поле recurrence: {e}"))?;
        dates_ru::validate_recurrence(&rec)?;
        out.insert("recurrence".into(), v.clone());
    }
    if let Some(v) = args.get("start_date").filter(|v| !v.is_null()) {
        let s = v.as_str().ok_or("поле start_date должно быть строкой YYYY-MM-DD")?.trim();
        let date = NaiveDate::parse_from_str(s, "%Y-%m-%d").map_err(|_| "поле start_date: формат YYYY-MM-DD".to_string())?;
        let today = Local::now().date_naive();
        if date < today {
            return Err("start_date не может быть в прошлом".into());
        }
        out.insert("startOn".into(), json!(s));
    }
    if let Some(v) = args.get("once_at").filter(|v| !v.is_null()) {
        let ms = if let Some(s) = v.as_str() {
            let s = s.trim();
            let naive = chrono::NaiveDateTime::parse_from_str(s, "%Y-%m-%d %H:%M").map_err(|_| "поле once_at: формат «YYYY-MM-DD HH:MM»".to_string())?;
            let local_dt = naive.and_local_timezone(Local).single().ok_or("поле once_at: неоднозначное местное время")?;
            let ms = local_dt.timestamp_millis();
            if ms <= chrono::Utc::now().timestamp_millis() {
                return Err("once_at должно быть в будущем".into());
            }
            ms
        } else if let Some(n) = v.as_i64() {
            if n <= chrono::Utc::now().timestamp_millis() {
                return Err("once_at должно быть в будущем".into());
            }
            n
        } else {
            return Err("поле once_at: строка «YYYY-MM-DD HH:MM» или число миллисекунд".into());
        };
        out.insert("onceAt".into(), json!(ms));
    }
    if let Some(v) = args.get("catch_up").filter(|v| !v.is_null()) {
        out.insert("catchUp".into(), json!(v.as_str().ok_or("поле catch_up должно быть строкой: last или skip")?.trim()));
    }
    if !schedule::catch_up_valid(&Value::Object(out.clone())) {
        return Err("поле catch_up: только last или skip".into());
    }
    if !schedule::times_valid(&Value::Object(out.clone())) {
        return Err("время нужно в формате ЧЧ:ММ, например 09:00".into());
    }
    let has_at = out.get("at").and_then(Value::as_str).is_some_and(|a| !a.is_empty());
    if has_at && out.contains_key("everyMinutes") {
        return Err("укажи либо at (время), либо every_minutes (интервал), не оба".into());
    }
    Ok(out)
}

/// Окно работы — оба края или ни одного: с половиной окна расписание молча ломается.
pub(crate) fn check_window(from: &str, to: &str) -> Result<(), String> {
    match (from.trim().is_empty(), to.trim().is_empty()) {
        (true, true) => Ok(()),
        (false, false) if from.trim() == to.trim() => Err("окно работы пустое: начало и конец совпадают".into()),
        (false, false) => Ok(()),
        _ => Err("окно работы задаётся двумя полями: window_from и window_to".into()),
    }
}

pub(crate) fn target_bot(app: &App, me: &Bot, args: &Value) -> Result<Bot, String> {
    let wanted = text(args, "bot_id");
    if wanted.is_empty() || wanted == me.id {
        return Ok(me.clone());
    }
    // Как у update_bot: правила команды позволяют настраивать коллегу по ID. Он будет работать по своим разрешениям.
    app.store.bot(wanted).ok_or_else(|| "бота с таким ID нет, посмотри list_bots".to_string())
}

fn line(app: &App, routine: &Routine) -> String {
    let who = app.store.bot(&routine.bot_id).map_or_else(|| "бот удалён".to_string(), |b| b.name);
    format!("{} · «{}» — {}: {}. {}", routine.id, routine.title, who, schedule::describe(routine), schedule::describe_next(routine))
}

/// Цель для рутины: должна быть и принадлежать тому же боту, что и рутина.
fn goal_of(app: &App, id: &str, bot_id: &str) -> Result<crate::goals::Goal, String> {
    let goal = app.goals.get(id).ok_or("цели с таким номером нет, посмотри list_goals")?;
    if goal.bot_id != bot_id {
        return Err("цель другого бота: рутина запускает цель своего бота".into());
    }
    Ok(goal)
}

fn find(app: &App, args: &Value) -> Result<Routine, String> {
    let id = text(args, "routine_id");
    if id.is_empty() {
        return Err("нужен номер рутины из list_routines".into());
    }
    app.store.routine(id).ok_or_else(|| "рутины с таким номером нет, посмотри list_routines".to_string())
}

pub async fn exec(app: &Arc<App>, bot: &Bot, name: &str, args: &Value) -> Result<String, String> {
    let store = &app.store;
    match name {
        "list_routines" => {
            let routines = store.routines();
            if routines.is_empty() {
                return Ok("Рутин нет.".into());
            }
            Ok(routines.iter().map(|r| line(app, r)).collect::<Vec<_>>().join("\n"))
        }

        "create_routine" => {
            let target = target_bot(app, bot, args)?;
            let title = text(args, "title");
            let prompt = text(args, "prompt");
            if title.is_empty() || title.chars().count() > MAX_TITLE {
                return Err(format!("название нужно, до {MAX_TITLE} символов"));
            }
            let goal_id = text(args, "goal_id");
            let goal = if goal_id.is_empty() { None } else { Some(goal_of(app, goal_id, &target.id)?) };
            if (prompt.is_empty() && goal.is_none()) || prompt.chars().count() > MAX_PROMPT {
                return Err(format!("нужна постановка prompt (или goal_id), до {MAX_PROMPT} символов"));
            }
            let prompt = match &goal {
                Some(g) if prompt.is_empty() => format!("Шаг к цели «{}»", g.title),
                _ => prompt.to_string(),
            };
            let mut input = schedule_patch(args)?;
            let has_at = input.get("at").and_then(Value::as_str).is_some_and(|a| !a.is_empty());
            let has_once = input.contains_key("onceAt");
            if !has_at && !input.contains_key("everyMinutes") && !has_once {
                return Err("не сказано когда: укажи at (время ЧЧ:ММ), every_minutes или once_at".into());
            }
            check_window(input.get("windowFrom").and_then(Value::as_str).unwrap_or_default(), input.get("windowTo").and_then(Value::as_str).unwrap_or_default())?;
            input.insert("botId".into(), json!(target.id));
            input.insert("title".into(), json!(title));
            input.insert("prompt".into(), json!(prompt));
            input.insert("goalId".into(), json!(goal_id));
            input.insert("enabled".into(), json!(flag(args, "enabled")?.unwrap_or(true)));
            input.insert("quiet".into(), json!(flag(args, "quiet")?.unwrap_or(true)));
            if let Some(v) = args.get("hook_events").filter(|v| !v.is_null()) {
                input.insert("hookEvents".into(), v.clone());
            }
            // Рутина для чужого бота не должна выйти смелее того, кто её завёл: как при делегировании.
            if target.id != bot.id {
                let floor = crate::tools::stricter(&bot.approval, &target.approval);
                input.insert("approvalFloor".into(), json!(floor));
            }
            let made = store
                .add_routine_capped(&Value::Object(input), MAX_ROUTINES)
                .await
                .map_err(|e| e.to_string())?
                .ok_or_else(|| format!("уже {MAX_ROUTINES} рутин, новые не создаются"))?;
            app.turns.emit(json!({ "type": "state_changed" }));
            Ok(format!("Рутина создана.\n{}", line(app, &made)))
        }

        "update_routine" => {
            let routine = find(app, args)?;
            let mut patch = schedule_patch(args)?;
            if let Some(v) = args.get("hook_events").filter(|v| !v.is_null()) {
                patch.insert("hookEvents".into(), v.clone());
            }
            if routine.hook && patch.keys().any(|k| k != "hookEvents") {
                return Err("эта рутина запускается вебхуком, у неё нет расписания".into());
            }
            let new_at = patch.get("at").and_then(Value::as_str).map(str::to_string);
            // Интервал вместо времени: пока `at` задан, он главнее интервала.
            if patch.contains_key("everyMinutes") && new_at.is_none() {
                patch.insert("at".into(), json!(""));
            }
            if new_at.as_deref() == Some("") && !patch.contains_key("everyMinutes") {
                return Err("если снять время at, укажи every_minutes".into());
            }
            let window_from = patch.get("windowFrom").and_then(Value::as_str).map_or(routine.window_from.clone(), str::to_string);
            let window_to = patch.get("windowTo").and_then(Value::as_str).map_or(routine.window_to.clone(), str::to_string);
            check_window(&window_from, &window_to)?;
            if !routine.hook {
                for key in ["title", "prompt"] {
                    if let Some(v) = args.get(key).filter(|v| !v.is_null()) {
                        let value = v.as_str().ok_or_else(|| format!("поле {key} должно быть строкой"))?.trim();
                        let limit = if key == "title" { MAX_TITLE } else { MAX_PROMPT };
                        if value.is_empty() || value.chars().count() > limit {
                            return Err(format!("{key}: от 1 до {limit} символов"));
                        }
                        patch.insert(key.into(), json!(value));
                    }
                }
            }
            if let Some(v) = args.get("goal_id").filter(|v| !v.is_null()) {
                let id = v.as_str().ok_or("поле goal_id должно быть строкой")?.trim();
                if !id.is_empty() {
                    goal_of(app, id, &routine.bot_id)?;
                }
                patch.insert("goalId".into(), json!(id));
            }
            if !routine.hook {
                for key in ["enabled", "quiet"] {
                    if let Some(v) = flag(args, key)? {
                        patch.insert(key.into(), json!(v));
                    }
                }
            }
            if patch.is_empty() {
                return Err("не указано, что менять".into());
            }
            // Правка чужой рутины добавляет осторожность правящего: прежний пол остаётся, строже можно, мягче нельзя.
            if routine.bot_id != bot.id {
                if let Some(floor) = crate::goals::combine_floor(&routine.approval_floor, &bot.approval) {
                    patch.insert("approvalFloor".into(), json!(floor));
                }
            }
            // Включённая заново рутина должна получить свежий срок, а не старый, давно прошедший.
            if patch.get("enabled") == Some(&json!(true)) && !["at", "everyMinutes", "days", "windowFrom", "windowTo"].iter().any(|k| patch.contains_key(*k)) {
                patch.insert("at".into(), json!(routine.at));
            }
            let updated = store.update_routine(&routine.id, &Value::Object(patch)).await.map_err(|e| e.to_string())?
                .ok_or("рутина уже удалена")?;
            app.turns.emit(json!({ "type": "state_changed" }));
            Ok(format!("Рутина изменена.\n{}", line(app, &updated)))
        }

        "run_routine" => {
            let routine = find(app, args)?;
            let started = crate::scheduler::run_now(app, &routine.id, app.turns.is_unattended(&bot.id)).await.map_err(|e| e.message)?;
            Ok(format!("Рутина «{}» запущена.", started.title))
        }

        "delete_routine" => {
            let routine = find(app, args)?;
            store.remove_routine(&routine.id).await.map_err(|e| e.to_string())?;
            app.turns.emit(json!({ "type": "state_changed" }));
            Ok(format!("Рутина «{}» удалена.", routine.title))
        }

        other => Err(format!("неизвестный инструмент: {other}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn days_come_as_numbers_or_words() {
        assert_eq!(day_of(&json!(3)), Some(3));
        assert_eq!(day_of(&json!("5")), Some(5));
        assert_eq!(day_of(&json!("Пн")), Some(1));
        assert_eq!(day_of(&json!("sun")), Some(7));
        assert_eq!(day_of(&json!(8)), None);
        assert_eq!(day_of(&json!("завтра")), None);
    }

    #[test]
    fn schedule_arguments_are_checked_like_the_interface_does() {
        let ok = schedule_patch(&json!({ "at": "09:00", "days": [1, 2, 3, 4, 5] })).unwrap();
        assert_eq!(ok["at"], "09:00");
        assert_eq!(ok["days"], json!([1, 2, 3, 4, 5]));
        assert!(schedule_patch(&json!({ "at": "9:00" })).is_err());
        assert!(schedule_patch(&json!({ "at": "09:00", "every_minutes": 60 })).is_err());
        assert!(schedule_patch(&json!({ "every_minutes": 1 })).is_err());
        assert!(schedule_patch(&json!({ "every_minutes": 80, "every_max_minutes": 40 })).is_err());
        assert_eq!(schedule_patch(&json!({ "every_minutes": 40, "every_max_minutes": 80 })).unwrap()["everyMaxMinutes"], 80);
        assert!(schedule_patch(&json!({ "days": [9] })).is_err());
        assert!(schedule_patch(&json!({})).unwrap().is_empty());
        assert!(check_window("22:00", "").is_err());
        assert!(check_window("22:00", "07:00").is_ok());
        assert!(check_window("", "").is_ok());
    }

    #[test]
    fn yearly_nth_weekday_patch_gives_the_september_saturday() {
        let rule = json!({ "freq": "yearly", "month": 9, "weekdays": [6], "nth": 3 });
        let patch = schedule_patch(&json!({ "at": "09:00", "recurrence": rule.clone() })).unwrap();
        assert_eq!(patch["at"], "09:00");
        assert_eq!(patch["recurrence"], rule);
        let rec: dates_ru::Recurrence = serde_json::from_value(patch["recurrence"].clone()).unwrap();
        let r = Routine { at: "09:00".into(), recurrence: Some(rec), ..Routine::default() };
        let now = NaiveDate::from_ymd_opt(2026, 1, 1).unwrap().and_hms_opt(10, 0, 0).unwrap();
        let expected = NaiveDate::from_ymd_opt(2026, 9, 19).unwrap().and_hms_opt(9, 0, 0).unwrap();
        assert_eq!(schedule::next_after(&r, now), expected);
    }

    #[test]
    fn start_and_once_dates_in_the_past_are_refused() {
        let yesterday = Local::now().date_naive().pred_opt().unwrap().format("%Y-%m-%d").to_string();
        let past_start = schedule_patch(&json!({ "at": "09:00", "start_date": yesterday })).unwrap_err();
        assert!(past_start.contains("прошлом"), "{past_start}");
        let past_once = (Local::now() - chrono::Duration::days(1)).format("%Y-%m-%d %H:%M").to_string();
        let past_once_error = schedule_patch(&json!({ "once_at": past_once })).unwrap_err();
        assert!(past_once_error.contains("будущем"), "{past_once_error}");
    }
}
