//! Старые рутины из `state.json` переезжают в процессы «триггер → бот» (0.4.11). Здесь только чтение старой записи:
//! публичного типа «рутина» в программе больше нет. Напоминания из того же списка уходят в `reminders.json` (`store`).
//! Вебхук-рутина тоже становится процессом: секрет (`hookToken`) переезжает в секреты, адрес `/hooks/<номер>/<секрет>` продолжает работать.

use serde::Deserialize;
use serde_json::Value;

use crate::schedule::Timing;
use crate::simple_process::{build, set_slot, Spec, BOT, TRIGGER};
use crate::workflows::{LastRun, Stored};

fn last() -> String {
    "last".into()
}

/// Запись рутины в старом `state.json`: только то, что переносится.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", default)]
struct OldRoutine {
    id: String,
    kind: String,
    bot_id: String,
    title: String,
    prompt: String,
    enabled: bool,
    quiet: bool,
    created_at: i64,
    hook: bool,
    hook_token: String,
    hook_events: Vec<String>,
    goal_id: String,
    approval_floor: String,
    check: String,
    catch_up: String,
    last_status: Option<String>,
    last_run: i64,
    next_run: i64,
    failures: u32,
    stop_reason: Option<crate::runs::StopReason>,
    error_paused: bool,
    progress_note: String,
    #[serde(flatten)]
    when: Timing,
}

impl Default for OldRoutine {
    fn default() -> Self {
        Self {
            id: String::new(),
            kind: String::new(),
            bot_id: String::new(),
            title: String::new(),
            prompt: String::new(),
            enabled: true,
            quiet: true,
            created_at: 0,
            hook: false,
            hook_token: String::new(),
            hook_events: Vec::new(),
            goal_id: String::new(),
            approval_floor: String::new(),
            check: String::new(),
            catch_up: last(),
            last_status: None,
            last_run: 0,
            next_run: 0,
            failures: 0,
            stop_reason: None,
            error_paused: false,
            progress_note: String::new(),
            when: Timing::default(),
        }
    }
}

/// Запись списка `state.routines` — напоминание, а не рутина.
pub fn is_reminder(value: &Value) -> bool {
    value.get("kind").and_then(Value::as_str) == Some(crate::reminders::KIND)
}

/// Секрет вебхука старой рутины: он переезжает в секреты раньше процесса.
pub fn hook_secret(value: &Value) -> Option<(String, String, String)> {
    let old: OldRoutine = serde_json::from_value(value.clone()).ok()?;
    old.hook.then(|| (crate::workflows::hook_scope(&old.id), crate::workflows::hook_name(TRIGGER), old.hook_token))
}

/// Процесс из старой записи с тем же номером. Срок `next_run`, пауза, серия ошибок и заметка переезжают с ним.
pub fn to_stored(value: &Value) -> Result<Stored, serde_json::Error> {
    let old: OldRoutine = serde_json::from_value(value.clone())?;
    let mut stored = build(&Spec {
        id: old.id.clone(),
        title: old.title.clone(),
        bot_id: old.bot_id.clone(),
        prompt: old.prompt.clone(),
        enabled: old.enabled,
        quiet: old.quiet,
        catch_up: old.catch_up.clone(),
        goal_id: old.goal_id.clone(),
        approval_floor: old.approval_floor.clone(),
        check: old.check.clone(),
        created_at: old.created_at,
        hook_events: old.hook.then(|| old.hook_events.clone()),
        timing: old.when.clone(),
    });
    let wf = &mut stored.wf;
    wf.last_run = old.last_status.map(|status| LastRun { status, at: old.last_run });
    wf.failures = old.failures;
    wf.stop_reason = old.stop_reason;
    wf.error_paused = old.error_paused;
    if !old.progress_note.is_empty() {
        wf.notes.insert(BOT.into(), old.progress_note);
    }
    // Срок рутины остаётся сроком процесса: тот же момент, второго запуска при переезде нет. Нет срока — его посчитает тик.
    if old.next_run > 0 && !old.hook {
        set_slot(&mut stored, Some(old.next_run));
    }
    Ok(stored)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn routine(id: &str, extra: Value) -> Value {
        let mut base = json!({ "id": id, "botId": "b", "title": "Сводка", "prompt": "Сделай сводку", "enabled": true, "quiet": true, "createdAt": 5, "nextRun": 1_900_000_000_000i64 });
        for (key, value) in extra.as_object().unwrap() {
            base[key] = value.clone();
        }
        base
    }

    #[test]
    fn each_schedule_kind_keeps_its_id_and_next_run() {
        let cases = [
            (routine("rt_a", json!({ "everyMinutes": 30, "everyMaxMinutes": 45, "windowFrom": "09:00", "windowTo": "18:00", "days": [1, 2, 3] })), "interval"),
            (routine("rt_b", json!({ "at": "09:30", "days": [1, 5] })), "daily"),
            (routine("rt_c", json!({ "cron": "0 9 * * 1", "tz": "Europe/Moscow" })), "cron"),
            (routine("rt_d", json!({ "onceAt": 1_900_000_000_000i64 })), "once"),
            (routine("rt_e", json!({ "at": "08:00", "startOn": "2030-01-01" })), "daily"),
        ];
        for (value, kind) in cases {
            let stored = to_stored(&value).unwrap();
            let id = value["id"].as_str().unwrap();
            assert_eq!(stored.wf.id, id);
            let schedule = stored.wf.graph.node(TRIGGER).unwrap().schedule.clone().unwrap();
            assert_eq!(schedule.kind, kind, "{id}");
            let slot = &stored.slots[TRIGGER];
            assert_eq!(slot.next, Some(1_900_000_000_000), "{id}: срок тот же");
            assert_eq!(slot.sig, crate::simple_process::sig_of(&schedule), "{id}: подпись та же, тик не пересчитает срок");
            assert_eq!(stored.next_run(), 1_900_000_000_000, "{id}");
            assert_eq!((stored.wf.title.as_str(), stored.prompt().as_str(), stored.bot_id().as_str()), ("Сводка", "Сделай сводку", "b"));
            assert!(crate::workflows::validate_graph(&stored.wf.graph).is_ok(), "{id}: граф годен");
        }
    }

    #[test]
    fn old_at_with_days_and_the_old_once_field_read_as_the_same_schedule() {
        // Записи до общего правила повтора: `at` + `days` без `recurrence`, разовый срок полем `onceAt`.
        let weekly = to_stored(&routine("rt_w", json!({ "at": "09:00", "days": [1, 3] }))).unwrap();
        assert_eq!(weekly.timing().days, vec![1, 3]);
        assert_eq!(weekly.timing().at, "09:00");
        let once = to_stored(&routine("rt_o", json!({ "onceAt": 1_800_000_000_000i64 }))).unwrap();
        assert_eq!(once.timing().once_at, 1_800_000_000_000);
    }

    #[test]
    fn state_of_the_routine_travels_with_it() {
        let value = routine(
            "rt_s",
            json!({ "enabled": false, "quiet": false, "goalId": "g1", "approvalFloor": "ask", "check": "echo 1", "catchUp": "skip", "failures": 4, "errorPaused": true, "stopReason": "login", "lastStatus": "failed", "lastRun": 77, "progressNote": "заметка", "everyMinutes": 60 }),
        );
        let stored = to_stored(&value).unwrap();
        assert!(!stored.wf.enabled && stored.wf.error_paused && stored.wf.failures == 4 && stored.wf.simple);
        assert_eq!(stored.wf.notes[BOT], "заметка");
        let bot = stored.wf.graph.node(BOT).unwrap();
        assert_eq!((bot.text("quiet").as_str(), bot.text("goal_id").as_str(), bot.text("approval_floor").as_str(), bot.text("check").as_str()), ("false", "g1", "ask", "echo 1"));
        assert_eq!(stored.wf.graph.node(TRIGGER).unwrap().text("catch_up"), "skip");
        assert_eq!(stored.wf.last_run.as_ref().map(|r| (r.status.as_str(), r.at)), Some(("failed", 77)));
        assert_eq!(stored.next_run(), 0, "у выключенной срока нет");
    }

    #[test]
    fn a_webhook_routine_keeps_its_secret_for_the_secrets_store() {
        let value = routine("rt_h", json!({ "hook": true, "hookToken": "secret-of-rt_h", "hookEvents": ["push"] }));
        let (scope, name, token) = hook_secret(&value).unwrap();
        assert_eq!((scope.as_str(), token.as_str()), ("wf.rt_h", "secret-of-rt_h"));
        assert_eq!(name, crate::workflows::hook_name(TRIGGER));
        let stored = to_stored(&value).unwrap();
        assert!(stored.is_hook() && stored.wf.graph.node(TRIGGER).unwrap().schedule.is_none());
        assert!(!stored.slots.contains_key(TRIGGER));
        assert!(hook_secret(&routine("rt_n", json!({}))).is_none());
    }
}
