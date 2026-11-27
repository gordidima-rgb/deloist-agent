//! Процесс «триггер → бот» (бывшая рутина): заведение, правка и карточка для интерфейса.
//! Граф из двух узлов: триггер `t1` (расписание или вебхук) и шаг-бот `b1`. Всё, что у него есть сверх графа
//! (срок, серия ошибок, заметка), лежит в самом процессе; своего типа «рутина» нет.

use std::collections::HashMap;

use serde_json::{json, Map, Value};

use crate::schedule::{self, Schedule, Timing};
use crate::store::{as_string, minutes, now, take_chars, text_or, truthy, uid};
use crate::workflows::{self as wf, Edge, Graph, Node, Slot, Stored, Workflow, Workflows};

pub(crate) const TRIGGER: &str = "t1";
pub(crate) const BOT: &str = "b1";

fn text(value: &str) -> Value {
    Value::String(value.to_string())
}

pub(crate) fn sig_of(schedule: &Schedule) -> String {
    serde_json::to_string(schedule).unwrap_or_default()
}

fn invalid(message: impl Into<String>) -> std::io::Error {
    std::io::Error::new(std::io::ErrorKind::InvalidInput, message.into())
}

/// Что нужно, чтобы собрать процесс «триггер → бот».
pub(crate) struct Spec {
    pub id: String,
    pub title: String,
    pub bot_id: String,
    pub prompt: String,
    pub enabled: bool,
    pub quiet: bool,
    pub catch_up: String,
    pub goal_id: String,
    pub approval_floor: String,
    pub check: String,
    pub created_at: i64,
    /// Вебхук вместо расписания; события GitHub, на которые он реагирует.
    pub hook_events: Option<Vec<String>>,
    pub timing: Timing,
}

/// Сам процесс. Срок первого запуска — по расписанию от «сейчас», если он не задан.
pub(crate) fn build(spec: &Spec) -> Stored {
    let schedule = spec.timing.schedule();
    let hook = spec.hook_events.is_some();
    let mut trigger_fields = Map::new();
    if let Some(events) = &spec.hook_events {
        trigger_fields.insert("trigger".into(), text("webhook"));
        if !events.is_empty() {
            trigger_fields.insert("hook_events".into(), text(&events.join(",")));
        }
    } else {
        trigger_fields.insert("trigger".into(), text("schedule"));
        trigger_fields.insert("catch_up".into(), text(&spec.catch_up));
    }
    let mut bot_fields = Map::new();
    bot_fields.insert("bot_id".into(), text(&spec.bot_id));
    bot_fields.insert("task".into(), text(&spec.prompt));
    bot_fields.insert("quiet".into(), text(if spec.quiet { "true" } else { "false" }));
    for (key, value) in [("approval_floor", &spec.approval_floor), ("goal_id", &spec.goal_id), ("check", &spec.check)] {
        if !value.is_empty() {
            bot_fields.insert(key.into(), text(value));
        }
    }
    let graph = Graph {
        nodes: vec![
            Node { id: TRIGGER.into(), kind: "trigger".into(), x: 0.0, y: 0.0, fields: trigger_fields, schedule: (!hook).then(|| schedule.clone()), ..Node::default() },
            Node { id: BOT.into(), kind: "bot".into(), x: 320.0, y: 0.0, fields: bot_fields, ..Node::default() },
        ],
        edges: vec![Edge { id: "e1".into(), from: TRIGGER.into(), to: BOT.into(), ..Edge::default() }],
        strokes: Vec::new(),
    };
    Stored {
        wf: Workflow {
            id: spec.id.clone(),
            title: spec.title.clone(),
            enabled: spec.enabled,
            revision: 1,
            graph,
            created_at: spec.created_at,
            updated_at: now(),
            simple: true,
            ..Workflow::default()
        },
        slots: HashMap::new(),
    }
}

/// Срок триггера: тот же момент, что записан, а если его нет — по расписанию от `now_ms`.
pub(crate) fn set_slot(stored: &mut Stored, next: Option<i64>) {
    let schedule = stored.wf.graph.node(TRIGGER).and_then(|n| n.schedule.clone());
    if let Some(schedule) = schedule {
        stored.slots.insert(TRIGGER.into(), Slot { sig: sig_of(&schedule), next });
    }
}

impl Stored {
    fn trigger_node(&self) -> Option<&Node> {
        self.wf.graph.node(TRIGGER)
    }

    fn bot_node(&self) -> Option<&Node> {
        self.wf.graph.node(BOT)
    }

    /// Процесс «триггер → бот».
    pub fn is_simple(&self) -> bool {
        self.wf.simple
    }

    /// Бот простого процесса; у остальных — первый шаг-бот графа.
    pub fn bot_id(&self) -> String {
        let node = self.bot_node().or_else(|| self.wf.graph.nodes.iter().find(|n| n.kind == "bot"));
        node.map(|n| n.text("bot_id")).unwrap_or_default()
    }

    pub fn prompt(&self) -> String {
        self.bot_node().map(|n| n.text("task")).unwrap_or_default()
    }

    pub fn goal_id(&self) -> String {
        self.bot_node().map(|n| n.text("goal_id")).unwrap_or_default()
    }

    /// Осторожность создателя, если процесс завёл другой бот.
    pub fn approval_floor(&self) -> String {
        self.bot_node().map(|n| n.text("approval_floor")).unwrap_or_default()
    }

    /// Запускается вебхуком, а не по расписанию.
    pub fn is_hook(&self) -> bool {
        self.trigger_node().is_some_and(|n| n.text("trigger") == "webhook")
    }

    /// Расписание триггера в плоском виде формы.
    pub fn timing(&self) -> Timing {
        self.trigger_node().and_then(|n| n.schedule.as_ref()).map(Timing::of).unwrap_or_default()
    }

    /// Следующий запуск; у выключенного и у вебхука его нет.
    pub fn next_run(&self) -> i64 {
        if self.wf.enabled { self.slots.get(TRIGGER).and_then(|slot| slot.next).unwrap_or(0) } else { 0 }
    }
}

/// Карточка простого процесса для интерфейса и ботов: плоский вид со всеми полями формы расписания.
/// Секрета вебхука здесь нет.
pub fn card(stored: &Stored) -> Value {
    let wf = &stored.wf;
    let trigger = stored.trigger_node();
    let bot = stored.bot_node();
    let field = |node: Option<&Node>, key: &str| node.map(|n| n.text(key)).unwrap_or_default();
    let mut out = serde_json::to_value(stored.timing()).unwrap_or_else(|_| json!({}));
    let catch_up = field(trigger, "catch_up");
    out["id"] = json!(wf.id);
    out["botId"] = json!(stored.bot_id());
    out["title"] = json!(wf.title);
    out["prompt"] = json!(stored.prompt());
    out["enabled"] = json!(wf.enabled);
    out["hook"] = json!(stored.is_hook());
    out["quiet"] = json!(field(bot, "quiet") != "false");
    out["createdAt"] = json!(wf.created_at);
    out["goalId"] = json!(stored.goal_id());
    out["approvalFloor"] = json!(field(bot, "approval_floor"));
    out["check"] = json!(field(bot, "check"));
    out["catchUp"] = json!(if catch_up.is_empty() { "last".to_string() } else { catch_up });
    out["lastStatus"] = json!(wf.last_run.as_ref().map(|run| if run.status == "done" { "ok" } else { run.status.as_str() }));
    out["lastRun"] = json!(wf.last_run.as_ref().map_or(0, |run| run.at));
    out["nextRun"] = json!(stored.next_run());
    out["progressNote"] = json!(wf.notes.get(BOT).cloned().unwrap_or_default());
    if wf.failures > 0 {
        out["failures"] = json!(wf.failures);
    }
    if let Some(reason) = wf.stop_reason {
        out["stopReason"] = json!(reason);
    }
    if wf.error_paused {
        out["errorPaused"] = json!(true);
    }
    out
}

/// Простые процессы одного бота или всех, как карточки.
pub fn cards(workflows: &Workflows, bot_id: Option<&str>) -> Vec<Value> {
    let guard = workflows.lock();
    guard.items.iter().filter(|s| s.is_simple() && bot_id.is_none_or(|b| s.bot_id() == b)).map(card).collect()
}

/// Заведение простого процесса из тела запроса (форма интерфейса, инструмент бота, импорт команды).
/// `None` — достигнут предел `cap` простых процессов.
pub fn create(workflows: &Workflows, input: &Value, cap: usize, human: bool) -> std::io::Result<Option<Stored>> {
    let mut timing = Timing { every_minutes: minutes(input.get("everyMinutes")), ..Timing::default() };
    timing.apply(input).map_err(invalid)?;
    if input.get("hook").is_some_and(truthy) {
        return Err(invalid("вебхук включается у процесса на холсте"));
    }
    let spec = Spec {
        id: uid("rt"),
        title: take_chars(&text_or(input, "title", "Рутина"), 80),
        bot_id: text_or(input, "botId", ""),
        prompt: text_or(input, "prompt", ""),
        enabled: input.get("enabled").map_or(true, |v| v != &Value::Bool(false)),
        quiet: input.get("quiet").map_or(true, truthy),
        catch_up: text_or(input, "catchUp", "last"),
        goal_id: text_or(input, "goalId", ""),
        approval_floor: text_or(input, "approvalFloor", ""),
        check: if human { text_or(input, "check", "") } else { String::new() },
        created_at: now(),
        hook_events: None,
        timing: timing.clone(),
    };
    let mut stored = build(&spec);
    set_slot(&mut stored, Some(schedule::next_run_ms(&timing, now())));
    let mut guard = workflows.lock();
    if guard.items.iter().filter(|s| s.is_simple()).count() >= cap {
        return Ok(None);
    }
    guard.insert_new(stored.clone())?;
    Ok(Some(stored))
}

/// Правка простого процесса из тела запроса. `None` — такого простого процесса нет.
pub fn patch(workflows: &Workflows, id: &str, patch: &Value, human: bool) -> std::io::Result<Option<Stored>> {
    let mut guard = workflows.lock();
    let Some(stored) = guard.get_mut(id).filter(|s| s.is_simple()) else { return Ok(None) };
    if stored.is_hook() {
        return Err(invalid("вебхук-процесс правится на холсте"));
    }
    if patch.get("hook").is_some_and(truthy) {
        return Err(invalid("вебхук включается у процесса на холсте"));
    }
    let before = serde_json::to_string(&stored.wf.graph).unwrap_or_default();
    let mut timing = stored.timing();
    if patch.get("everyMinutes").is_some_and(|v| !v.is_null()) {
        timing.every_minutes = minutes(patch.get("everyMinutes"));
    }
    timing.apply(patch).map_err(invalid)?;
    let set = |key: &str| patch.get(key).filter(|v| !v.is_null());
    let rescheduled = ["everyMinutes", "everyMaxMinutes", "at", "days", "windowFrom", "windowTo", "recurrence", "startOn", "onceAt", "cron", "tz"].iter().any(|key| set(key).is_some());
    if let Some(v) = set("title") {
        stored.wf.title = as_string(v);
    }
    for node in &mut stored.wf.graph.nodes {
        if node.id == TRIGGER {
            if rescheduled {
                node.schedule = Some(timing.schedule());
            }
            if let Some(v) = set("catchUp") {
                node.fields.insert("catch_up".into(), text(&as_string(v)));
            }
        } else if node.id == BOT {
            for (key, field, only_human) in [("prompt", "task", false), ("goalId", "goal_id", false), ("approvalFloor", "approval_floor", false), ("check", "check", true)] {
                if let Some(v) = set(key).filter(|_| human || !only_human) {
                    let value = as_string(v);
                    if value.is_empty() && field != "task" {
                        node.fields.remove(field);
                    } else {
                        node.fields.insert(field.into(), text(&value));
                    }
                }
            }
            if let Some(v) = set("quiet") {
                node.fields.insert("quiet".into(), text(if truthy(v) { "true" } else { "false" }));
            }
        }
    }
    if let Some(v) = set("progressNote") {
        stored.wf.notes.insert(BOT.into(), as_string(v));
    }
    // «Возобновить» после паузы из-за ошибок.
    if let Some(v) = set("errorPaused") {
        stored.wf.error_paused = truthy(v);
        if !stored.wf.error_paused {
            stored.wf.failures = 0;
            stored.wf.stop_reason = None;
        }
    }
    let was_enabled = stored.wf.enabled;
    if let Some(v) = set("enabled") {
        stored.wf.enabled = truthy(v);
    }
    // Человек выключил процесс или поменял расписание: прежняя серия ошибок больше не нужна.
    if rescheduled || !stored.wf.enabled {
        stored.wf.failures = 0;
        stored.wf.stop_reason = None;
    }
    if rescheduled {
        set_slot(stored, Some(schedule::next_run_ms(&timing, now())));
    } else if stored.wf.enabled && !was_enabled {
        // После паузы сроки считаются заново: пропущенное за время выключения не запускается разом.
        stored.slots.clear();
        wf::refresh_slots(stored, now());
    }
    // Только для проверок: приблизить срок, не дожидаясь его.
    if cfg!(debug_assertions) {
        if let Some(at) = crate::store::number(patch.get("nextRun")) {
            set_slot(stored, Some(at as i64));
        }
    }
    if serde_json::to_string(&stored.wf.graph).unwrap_or_default() != before {
        stored.wf.revision += 1;
    }
    stored.wf.updated_at = now();
    let updated = stored.clone();
    guard.save_wf(id);
    Ok(Some(updated))
}

/// Как процесс выглядит на карточке подтверждения: расписание и ближайший запуск по тому же вводу, без записи.
pub fn preview(input: &Value) -> (String, String) {
    let mut timing = Timing { every_minutes: minutes(input.get("everyMinutes")), ..Timing::default() };
    let _ = timing.apply(input);
    let enabled = input.get("enabled").map_or(true, |v| v != &Value::Bool(false));
    (schedule::describe(&timing), schedule::describe_next(false, enabled, schedule::next_run_ms(&timing, now())))
}
