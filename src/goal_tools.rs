//! Цели из чата: бот ставит постоянную задачу, запускает её, ставит на паузу, записывает прогресс.
//! Права те же, что у простых процессов «триггер → бот»: см. `workflow_tools`.

use std::sync::Arc;

use serde_json::{json, Value};

use crate::goals::{clear_pending, current_run, push_progress, Goal, Run, MAX_REPORT};
use crate::workflow_tools::{flag, target_bot, text};
use crate::store::Bot;
use crate::App;

const MAX_GOALS: usize = 100;
const MAX_TITLE: usize = 80;
const MAX_DESCRIPTION: usize = 5_000;

pub fn is_goal_tool(name: &str) -> bool {
    matches!(
        name,
        "create_goal" | "list_goals" | "update_goal" | "run_goal" | "pause_goal" | "resume_goal" | "complete_goal" | "delete_goal" | "report_goal_progress"
    )
}

pub fn defs() -> Vec<Value> {
    let function = |name: &str, description: &str, properties: Value, required: &[&str]| {
        let mut parameters = json!({ "type": "object", "properties": properties });
        if !required.is_empty() {
            parameters["required"] = json!(required);
        }
        json!({ "type": "function", "function": { "name": name, "description": description, "parameters": parameters } })
    };
    let goal_id = json!({ "goal_id": { "type": "string", "description": "Номер цели из list_goals." } });
    vec![
        function(
            "create_goal",
            "Поставить цель: постоянную задачу в этом разговоре. У цели нет своего расписания: её запускают по просьбе (run_goal) \
             или процесс бота (create_workflow с goal_id). Повторяющаяся цель (recurring) не кончается сама: её запуски только записывают прогресс.",
            json!({
                "title": { "type": "string", "description": "Короткое название, до 80 символов." },
                "description": { "type": "string", "description": "Чего добиваемся и как понять, что готово. Полностью: запуск стартует без этого разговора." },
                "recurring": { "type": "boolean", "description": "Повторяющаяся (следить, вести, собирать каждый раз) — true. Разовая, которая когда-то закончится, — false (по умолчанию)." },
                "bot_id": { "type": "string", "description": "Чья цель (ID из list_bots). По умолчанию твоя." },
            }),
            &["title", "description"],
        ),
        function("list_goals", "Показать цели: номер, чья, статус и последняя запись о прогрессе.", json!({}), &[]),
        function(
            "update_goal",
            "Изменить цель. Передавай только то, что меняется.",
            json!({
                "goal_id": { "type": "string", "description": "Номер цели." },
                "title": { "type": "string", "description": "Новое название." },
                "description": { "type": "string", "description": "Новая постановка целиком." },
                "recurring": { "type": "boolean", "description": "Повторяющаяся или разовая." },
            }),
            &["goal_id"],
        ),
        function(
            "run_goal",
            "Запустить цель сейчас: бот возьмёт следующий шаг. Если разговор бота занят, запуск встанет в очередь. На паузе или завершённой цели запуска нет, и в ответе так и сказано.",
            goal_id.clone(),
            &["goal_id"],
        ),
        function("pause_goal", "Поставить цель на паузу: запусков не будет, пока её не возобновят.", goal_id.clone(), &["goal_id"]),
        function("resume_goal", "Снять цель с паузы.", goal_id.clone(), &["goal_id"]),
        function(
            "complete_goal",
            "Завершить цель, когда она достигнута. Повторяющуюся цель запуск завершить не может: это решает человек.",
            goal_id.clone(),
            &["goal_id"],
        ),
        function(
            "delete_goal",
            "Удалить цель насовсем; рутины, привязанные к ней, выключаются. Человек подтверждает каждое удаление. Чтобы просто остановить, поставь на паузу (pause_goal).",
            goal_id,
            &["goal_id"],
        ),
        function(
            "report_goal_progress",
            "Записать, что сделано по цели и что дальше. Это запись в карточку цели, её видно человеку на странице бота.",
            json!({
                "goal_id": { "type": "string", "description": "Номер цели. Во время запуска цели можно не указывать." },
                "text": { "type": "string", "description": "Коротко: что сделано, что дальше." },
            }),
            &["text"],
        ),
    ]
}

/// Нужна ли карточка. Как у рутин: создание и правки — по режиму бота и всегда в ходе без человека.
/// Повторяющуюся цель завершает только человек, поэтому на неё карточка всегда. Запись прогресса —
/// отчёт бота о своей работе, без карточки и в строгом режиме: иначе самопроверка не смогла бы отчитаться.
pub fn needs_approval(app: &App, bot: &Bot, name: &str, args: &Value) -> bool {
    let goal = app.goals.get(text(args, "goal_id"));
    // Чужая цель — отдельный случай: карточка и в режиме «решай сам». Для новой цели смотрим, чей бот указан.
    let another_bot = match name {
        "create_goal" => {
            let wanted = text(args, "bot_id");
            !wanted.is_empty() && wanted != bot.id && app.store.bot(wanted).is_some()
        }
        _ => goal.as_ref().is_some_and(|g| g.bot_id != bot.id),
    };
    match name {
        "list_goals" | "report_goal_progress" => false,
        "delete_goal" => true,
        "complete_goal" => match &goal {
            // Запуск цели и самопроверка повторяющуюся цель не завершают: отказ придёт сам, карточка не нужна.
            Some(g) if g.recurring && current_run(&bot.id).is_some() => false,
            Some(g) if g.recurring => true,
            Some(_) => bot.approval != "auto" || another_bot,
            None => false,
        },
        // Бот не снимает «повторяющаяся»: отказ придёт сам, карточка не нужна.
        "update_goal" if goal.as_ref().is_some_and(|g| g.recurring) && flag(args, "recurring").ok().flatten() == Some(false) => false,
        _ => bot.approval != "auto" || app.turns.is_unattended(&bot.id) || another_bot,
    }
}

fn status_word(goal: &Goal) -> &'static str {
    match goal.status.as_str() {
        "paused" => "на паузе",
        "completed" => "завершена",
        _ => "идёт",
    }
}

fn line(app: &App, goal: &Goal) -> String {
    let who = app.store.bot(&goal.bot_id).map_or_else(|| "бот удалён".to_string(), |b| b.name);
    let kind = if goal.recurring { "повторяющаяся" } else { "разовая" };
    let last = goal.progress.last().map_or("прогресса ещё нет".to_string(), |p| {
        format!("последний прогресс ({}): {}", crate::schedule::describe_moment(p.at), p.text)
    });
    let waiting = if goal.pending_run { ", запуск ждёт, пока разговор освободится" } else { "" };
    format!("{} · «{}» — {}: {}, {kind}{waiting}. {last}", goal.id, goal.title, who, status_word(goal))
}

fn find(app: &App, args: &Value) -> Result<Goal, String> {
    let id = text(args, "goal_id");
    if id.is_empty() {
        return Err("нужен номер цели из list_goals".into());
    }
    app.goals.get(id).ok_or_else(|| "цели с таким номером нет, посмотри list_goals".to_string())
}

fn changed(app: &Arc<App>) {
    app.turns.emit(json!({ "type": "state_changed" }));
}

pub async fn exec(app: &Arc<App>, bot: &Bot, name: &str, args: &Value) -> Result<String, String> {
    match name {
        "list_goals" => {
            let mut goals = app.goals.list();
            if goals.is_empty() {
                return Ok("Целей нет.".into());
            }
            goals.reverse();
            Ok(goals.iter().take(50).map(|g| line(app, g)).collect::<Vec<_>>().join("\n"))
        }

        "create_goal" => {
            let target = target_bot(app, bot, args)?;
            let title = text(args, "title");
            let description = text(args, "description");
            if title.is_empty() || title.chars().count() > MAX_TITLE {
                return Err(format!("название нужно, до {MAX_TITLE} символов"));
            }
            if description.is_empty() || description.chars().count() > MAX_DESCRIPTION {
                return Err(format!("нужно описание цели, до {MAX_DESCRIPTION} символов"));
            }
            // Цель для чужого бота не должна выйти смелее того, кто её поставил: как при делегировании.
            let floor = if target.id != bot.id { crate::tools::stricter(&bot.approval, &target.approval) } else { String::new() };
            let goal = Goal {
                bot_id: target.id,
                title: title.to_string(),
                description: description.to_string(),
                recurring: flag(args, "recurring")?.unwrap_or(false),
                approval_floor: floor,
                ..Goal::default()
            };
            let made = app.goals.add_limited(goal, MAX_GOALS).await?;
            changed(app);
            Ok(format!("Цель поставлена.\n{}", line(app, &made)))
        }

        "update_goal" => {
            let goal = find(app, args)?;
            let mut title = None;
            let mut description = None;
            for (key, slot, limit) in [("title", &mut title, MAX_TITLE), ("description", &mut description, MAX_DESCRIPTION)] {
                if let Some(v) = args.get(key).filter(|v| !v.is_null()) {
                    let value = v.as_str().ok_or_else(|| format!("поле {key} должно быть строкой"))?.trim();
                    if value.is_empty() || value.chars().count() > limit {
                        return Err(format!("{key}: от 1 до {limit} символов"));
                    }
                    *slot = Some(value.to_string());
                }
            }
            let recurring = flag(args, "recurring")?;
            if title.is_none() && description.is_none() && recurring.is_none() {
                return Err("не указано, что менять".into());
            }
            let updated = app
                .goals
                .try_edit(&goal.id, |g| {
                    if g.status == "completed" {
                        return Err("цель уже завершена".to_string());
                    }
                    // Повторяющуюся цель бот в разовую не превращает: иначе её запуск мог бы сам себя завершить.
                    if g.recurring && recurring == Some(false) {
                        return Err("повторяющуюся цель в разовую превращает только человек, на странице бота".to_string());
                    }
                    if let Some(t) = title {
                        g.title = t;
                    }
                    if let Some(d) = description {
                        g.description = d;
                    }
                    if let Some(r) = recurring {
                        g.recurring = r;
                    }
                    // Правка чужой цели добавляет осторожность правящего, как у edit_workflow:
                    // прежний пол остаётся, строже можно, мягче нельзя.
                    if g.bot_id != bot.id {
                        if let Some(floor) = crate::goals::combine_floor(&g.approval_floor, &bot.approval) {
                            g.approval_floor = floor;
                        }
                    }
                    Ok(())
                })
                .await?;
            changed(app);
            Ok(format!("Цель изменена.\n{}", line(app, &updated)))
        }

        "run_goal" => {
            let goal = find(app, args)?;
            match goal.status.as_str() {
                "paused" => return Ok(format!("Запуска нет: цель «{}» на паузе. Возобнови её через resume_goal, если нужно.", goal.title)),
                "completed" => return Ok(format!("Запуска нет: цель «{}» завершена.", goal.title)),
                _ => {}
            }
            if app.store.bot(&goal.bot_id).is_none() {
                return Err("бота этой цели уже нет".into());
            }
            // Тот, кто просит, уже занят этим ходом: свой разговор не освободится, пока он не закончит.
            let floor = Some(bot.approval.clone());
            let busy = goal.bot_id == bot.id || crate::goals::conversation_busy(app, &goal.bot_id);
            if !busy {
                let unattended = app.turns.is_unattended(&bot.id);
                match crate::goals::start_goal_run(app, &goal, unattended, floor.clone()).await {
                    Ok(()) => {
                        changed(app);
                        return Ok(format!("Запустил цель «{}». Итог она запишет в прогресс.", goal.title));
                    }
                    Err(e) if e.status != 409 => return Err(e.message),
                    Err(_) => {}
                }
            }
            let floor = floor.unwrap_or_default();
            let queued = app
                .goals
                .try_edit(&goal.id, |g| {
                    if g.status != "active" {
                        return Err("цель только что поставили на паузу или завершили, запуска нет".to_string());
                    }
                    g.pending_run = true;
                    g.pending_since = crate::store::now();
                    g.pending_floor = floor;
                    Ok(())
                })
                .await;
            if let Err(why) = queued {
                return Ok(format!("Запуска нет: {why}."));
            }
            changed(app);
            Ok(format!("Разговор бота занят, запуск цели «{}» стоит в очереди и начнётся, как он освободится.", goal.title))
        }

        "delete_goal" => {
            let goal = find(app, args)?;
            crate::goals::delete(app, &goal.id).await;
            Ok(format!("Цель «{}» удалена.", goal.title))
        }

        "pause_goal" | "resume_goal" => {
            let goal = find(app, args)?;
            let pause = name == "pause_goal";
            let updated = app
                .goals
                .try_edit(&goal.id, |g| {
                    if g.status == "completed" {
                        return Err("цель уже завершена".to_string());
                    }
                    g.status = if pause { "paused" } else { "active" }.into();
                    clear_pending(g);
                    Ok(())
                })
                .await?;
            changed(app);
            Ok(format!("{}\n{}", if pause { "Цель на паузе." } else { "Цель снова идёт." }, line(app, &updated)))
        }

        "complete_goal" => {
            let goal = find(app, args)?;
            if goal.status == "completed" {
                return Ok(format!("Цель «{}» уже завершена.", goal.title));
            }
            let in_run = matches!(current_run(&bot.id), Some(Run::Goal(_) | Run::CheckIn));
            let updated = app
                .goals
                .try_edit(&goal.id, |g| {
                    // Ход без человека (запуск цели, самопроверка) повторяющуюся цель не кончает: это решение человека.
                    if g.recurring && in_run {
                        return Err("это повторяющаяся цель: её запуск не завершает цель, завершить её может только человек. Запиши прогресс через report_goal_progress.".to_string());
                    }
                    g.status = "completed".into();
                    clear_pending(g);
                    g.completed_at = crate::store::now();
                    Ok(())
                })
                .await?;
            crate::goals::release_routines(app, &updated.id).await;
            changed(app);
            Ok(format!("Цель завершена.\n{}", line(app, &updated)))
        }

        "report_goal_progress" => {
            let note = text(args, "text");
            if note.is_empty() {
                return Err("нужен текст записи".into());
            }
            // Запись попадёт в каждый следующий запрос: длиннее предела обрезается.
            let note: String = note.chars().take(MAX_REPORT).collect();
            let id = match text(args, "goal_id") {
                "" => match current_run(&bot.id) {
                    Some(Run::Goal(id)) => id,
                    _ => return Err("укажи номер цели из list_goals".into()),
                },
                id => id.to_string(),
            };
            let owner = bot.id.clone();
            let updated = app
                .goals
                .try_edit(&id, |g| {
                    if g.bot_id != owner {
                        return Err("прогресс записывает тот бот, чья это цель".to_string());
                    }
                    if g.status == "completed" {
                        return Err("цель уже завершена".to_string());
                    }
                    push_progress(g, &note);
                    Ok(())
                })
                .await?;
            changed(app);
            Ok(format!("Прогресс по «{}» записан.", updated.title))
        }

        other => Err(format!("неизвестный инструмент: {other}")),
    }
}
