//! Рутина = процесс «триггер → бот» (0.4.11, шаг 4a). Здесь два перевода между старой рутиной и процессом:
//! `to_stored` (переезд данных) и `to_routine` (вид процесса как рутины для старых адресов, пока их не убрал шаг 4b),
//! плюс `apply` — запись правки рутины обратно в процесс.
//! Вебхук-рутина тоже становится процессом: узел `t1` — вебхук, его секрет (`hook_token`) переезжает в секреты (`store::move_routines`),
//! старый адрес `/hooks/<номер>/<секрет>` продолжает работать.

use std::collections::HashMap;

use serde_json::{Map, Value};

use crate::schedule::{schedule_next, Schedule};
use crate::store::{now, Routine};
use crate::workflows::{Edge, Graph, LastRun, Node, Slot, Stored, Workflow};

pub(crate) const TRIGGER: &str = "t1";
const BOT: &str = "b1";

fn text(value: &str) -> Value {
    Value::String(value.to_string())
}

fn sig_of(schedule: &Schedule) -> String {
    serde_json::to_string(schedule).unwrap_or_default()
}

/// Процесс из рутины с тем же номером. Срок `next_run`, пауза, серия ошибок и заметка переезжают с ней.
pub fn to_stored(routine: &Routine) -> Stored {
    let schedule = routine.schedule();
    let mut trigger_fields = Map::new();
    if routine.hook {
        trigger_fields.insert("trigger".into(), text("webhook"));
        if !routine.hook_events.is_empty() {
            trigger_fields.insert("hook_events".into(), text(&routine.hook_events.join(",")));
        }
    } else {
        trigger_fields.insert("trigger".into(), text("schedule"));
        trigger_fields.insert("catch_up".into(), text(&routine.catch_up));
    }
    let mut bot_fields = Map::new();
    bot_fields.insert("bot_id".into(), text(&routine.bot_id));
    bot_fields.insert("task".into(), text(&routine.prompt));
    bot_fields.insert("quiet".into(), text(if routine.quiet { "true" } else { "false" }));
    for (key, value) in [("approval_floor", &routine.approval_floor), ("goal_id", &routine.goal_id), ("check", &routine.check)] {
        if !value.is_empty() {
            bot_fields.insert(key.into(), text(value));
        }
    }
    let graph = Graph {
        nodes: vec![
            Node { id: TRIGGER.into(), kind: "trigger".into(), x: 0.0, y: 0.0, fields: trigger_fields, schedule: (!routine.hook).then(|| schedule.clone()), ..Node::default() },
            Node { id: BOT.into(), kind: "bot".into(), x: 320.0, y: 0.0, fields: bot_fields, ..Node::default() },
        ],
        edges: vec![Edge { id: "e1".into(), from: TRIGGER.into(), to: BOT.into(), ..Edge::default() }],
        strokes: Vec::new(),
    };
    let mut stored = Stored {
        wf: Workflow {
            id: routine.id.clone(),
            title: routine.title.clone(),
            enabled: routine.enabled,
            revision: 1,
            graph,
            created_at: routine.created_at,
            updated_at: now(),
            from_routine: true,
            ..Workflow::default()
        },
        slots: HashMap::new(),
    };
    write_state(&mut stored, routine);
    stored
}

/// Всё, что рутина помнит о себе и не лежит в самом графе: срок, итог последнего запуска, серия ошибок, заметка.
fn write_state(stored: &mut Stored, routine: &Routine) {
    let wf = &mut stored.wf;
    wf.last_run = routine.last_status.clone().map(|status| LastRun { status, at: routine.last_run });
    wf.failures = routine.failures;
    wf.stop_reason = routine.stop_reason;
    wf.error_paused = routine.error_paused;
    wf.notes.clear();
    if !routine.progress_note.is_empty() {
        wf.notes.insert(BOT.into(), routine.progress_note.clone());
    }
    stored.slots.clear();
    // Срок рутины остаётся сроком процесса: тот же момент, второго запуска при переезде нет. Нет срока — его посчитает тик.
    let schedule = stored.wf.graph.node(TRIGGER).and_then(|n| n.schedule.as_ref()).map(sig_of);
    if let Some(sig) = schedule.filter(|_| routine.next_run > 0 && !routine.hook) {
        stored.slots.insert(TRIGGER.into(), Slot { sig, next: Some(routine.next_run) });
    }
}

/// Запись правки рутины (поля, срок, серия ошибок) обратно в её процесс; остальной граф не трогаем.
pub fn apply(stored: &mut Stored, routine: &Routine) {
    stored.wf.title = routine.title.clone();
    stored.wf.enabled = routine.enabled;
    stored.wf.updated_at = now();
    let schedule = routine.schedule();
    if let Some(node) = stored.wf.graph.nodes.iter_mut().find(|n| n.id == TRIGGER) {
        node.schedule = Some(schedule);
        node.fields.insert("catch_up".into(), text(&routine.catch_up));
    }
    if let Some(node) = stored.wf.graph.nodes.iter_mut().find(|n| n.id == BOT) {
        node.fields.insert("bot_id".into(), text(&routine.bot_id));
        node.fields.insert("task".into(), text(&routine.prompt));
        node.fields.insert("quiet".into(), text(if routine.quiet { "true" } else { "false" }));
        for (key, value) in [("approval_floor", &routine.approval_floor), ("goal_id", &routine.goal_id), ("check", &routine.check)] {
            if value.is_empty() {
                node.fields.remove(key);
            } else {
                node.fields.insert(key.into(), text(value));
            }
        }
    }
    write_state(stored, routine);
}

/// Процесс глазами старого окна рутин. `None` — это не рутина (обычный процесс).
pub fn to_routine(stored: &Stored) -> Option<Routine> {
    let wf = &stored.wf;
    if !wf.from_routine {
        return None;
    }
    let trigger = wf.graph.node(TRIGGER)?;
    let bot = wf.graph.node(BOT)?;
    let schedule = trigger.schedule.clone().unwrap_or_default();
    let mut routine = Routine {
        id: wf.id.clone(),
        bot_id: bot.text("bot_id"),
        title: wf.title.clone(),
        prompt: bot.text("task"),
        enabled: wf.enabled,
        hook: trigger.text("trigger") == "webhook",
        quiet: bot.text("quiet") != "false",
        created_at: wf.created_at,
        goal_id: bot.text("goal_id"),
        approval_floor: bot.text("approval_floor"),
        check: bot.text("check"),
        catch_up: if trigger.text("catch_up").is_empty() { "last".into() } else { trigger.text("catch_up") },
        failures: wf.failures,
        stop_reason: wf.stop_reason,
        error_paused: wf.error_paused,
        last_status: wf.last_run.as_ref().map(|run| if run.status == "done" { "ok".to_string() } else { run.status.clone() }),
        last_run: wf.last_run.as_ref().map_or(0, |run| run.at),
        next_run: if wf.enabled { stored.slots.get(TRIGGER).and_then(|slot| slot.next).unwrap_or(0) } else { 0 },
        progress_note: wf.notes.get(BOT).cloned().unwrap_or_default(),
        ..Routine::default()
    };
    read_schedule(&mut routine, &schedule);
    Some(routine)
}

/// Расписание триггера обратно в поля рутины. Виды, которых у рутины нет (рисовал человек на холсте), сводятся к ближайшему.
fn read_schedule(routine: &mut Routine, s: &Schedule) {
    let days = s.days.clone().unwrap_or_default();
    match s.kind.as_str() {
        "cron" => {
            routine.cron = s.cron.clone().unwrap_or_default();
            routine.tz = s.tz.clone().unwrap_or_default();
        }
        "once" => routine.once_at = schedule_next(s, 0).unwrap_or(0),
        "interval" => {
            routine.every_minutes = s.every_minutes.or_else(|| s.interval_minutes().map(|m| m as i64)).unwrap_or(60);
            routine.every_max_minutes = s.every_max_minutes.unwrap_or(0);
            routine.window_from = s.window_from.clone().unwrap_or_default();
            routine.window_to = s.window_to.clone().unwrap_or_default();
            routine.days = days;
        }
        "hourly" => routine.every_minutes = 60,
        kind => {
            routine.at = s.time.clone().unwrap_or_default();
            routine.days = match kind {
                "weekdays" => vec![1, 2, 3, 4, 5],
                "weekly" => vec![s.weekday.unwrap_or(1)],
                _ => days,
            };
            routine.recurrence = s.recurrence.clone();
            routine.start_on = s.start_on.clone().unwrap_or_default();
        }
    }
    if routine.every_minutes == 0 {
        routine.every_minutes = 60;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn routine(id: &str) -> Routine {
        Routine { id: id.into(), bot_id: "b".into(), title: "Сводка".into(), prompt: "Сделай сводку".into(), enabled: true, quiet: true, created_at: 5, next_run: 1_900_000_000_000, ..Routine::default() }
    }

    #[test]
    fn each_schedule_kind_keeps_its_id_and_next_run_and_reads_back() {
        let cases = [
            Routine { every_minutes: 30, every_max_minutes: 45, window_from: "09:00".into(), window_to: "18:00".into(), days: vec![1, 2, 3], ..routine("rt_a") },
            Routine { at: "09:30".into(), days: vec![1, 5], ..routine("rt_b") },
            Routine { cron: "0 9 * * 1".into(), tz: "Europe/Moscow".into(), ..routine("rt_c") },
            Routine { once_at: 1_900_000_000_000, ..routine("rt_d") },
            Routine { at: "08:00".into(), start_on: "2030-01-01".into(), ..routine("rt_e") },
        ];
        for r in cases {
            let stored = to_stored(&r);
            assert_eq!(stored.wf.id, r.id);
            let slot = &stored.slots[TRIGGER];
            assert_eq!(slot.next, Some(r.next_run), "{}: срок тот же", r.id);
            assert_eq!(slot.sig, sig_of(&r.schedule()), "{}: подпись та же, тик не пересчитает срок", r.id);
            let back = to_routine(&stored).unwrap();
            assert_eq!(back.schedule(), r.schedule(), "{}: расписание читается обратно", r.id);
            assert_eq!((back.next_run, back.title.as_str(), back.prompt.as_str(), back.bot_id.as_str()), (r.next_run, "Сводка", "Сделай сводку", "b"));
            assert!(crate::workflows::validate_graph(&stored.wf.graph).is_ok(), "{}: граф годен", r.id);
        }
    }

    #[test]
    fn state_of_the_routine_travels_with_it() {
        let r = Routine {
            enabled: false,
            quiet: false,
            goal_id: "g1".into(),
            approval_floor: "ask".into(),
            check: "echo 1".into(),
            catch_up: "skip".into(),
            failures: 4,
            error_paused: true,
            stop_reason: Some(crate::runs::StopReason::Login),
            last_status: Some("failed".into()),
            last_run: 77,
            progress_note: "заметка".into(),
            every_minutes: 60,
            ..routine("rt_s")
        };
        let stored = to_stored(&r);
        assert!(!stored.wf.enabled && stored.wf.error_paused && stored.wf.failures == 4 && stored.wf.from_routine);
        assert_eq!(stored.wf.notes[BOT], "заметка");
        let bot = stored.wf.graph.node(BOT).unwrap();
        assert_eq!((bot.text("quiet").as_str(), bot.text("goal_id").as_str(), bot.text("approval_floor").as_str(), bot.text("check").as_str()), ("false", "g1", "ask", "echo 1"));
        assert_eq!(stored.wf.graph.node(TRIGGER).unwrap().text("catch_up"), "skip");
        let back = to_routine(&stored).unwrap();
        assert_eq!((back.enabled, back.quiet, back.catch_up.as_str(), back.last_status.as_deref(), back.last_run, back.failures, back.error_paused), (false, false, "skip", Some("failed"), 77, 4, true));
        assert_eq!((back.goal_id.as_str(), back.check.as_str(), back.approval_floor.as_str(), back.progress_note.as_str()), ("g1", "echo 1", "ask", "заметка"));
        assert_eq!(back.next_run, 0, "у выключенной срока нет");
    }

    #[test]
    fn apply_writes_the_edit_back_and_keeps_the_rest_of_the_graph() {
        let mut stored = to_stored(&Routine { every_minutes: 30, ..routine("rt_p") });
        stored.wf.graph.nodes.push(Node { id: "extra".into(), kind: "note".into(), ..Node::default() });
        let mut r = to_routine(&stored).unwrap();
        r.prompt = "Новое".into();
        r.at = "10:00".into();
        r.next_run = 1_900_000_100_000;
        apply(&mut stored, &r);
        assert_eq!(stored.wf.graph.node(BOT).unwrap().text("task"), "Новое");
        assert_eq!(stored.slots[TRIGGER].next, Some(1_900_000_100_000));
        assert_eq!(stored.slots[TRIGGER].sig, sig_of(&r.schedule()));
        assert!(stored.wf.graph.node("extra").is_some());
    }

    #[test]
    fn an_ordinary_process_is_not_a_routine_and_a_webhook_stays_put() {
        let plain = crate::workflows::new_workflow("Т", Graph::default());
        assert!(to_routine(&plain).is_none());
    }
}
