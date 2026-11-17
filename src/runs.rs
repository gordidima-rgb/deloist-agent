//! Журнал запусков: каждый ход бота (человек, рутина, цель, задача команды, вебхук) получает запись в `runs.json`
//! до старта, обновляется по шагам и закрывается в конце. После перезапуска программы незакрытые записи
//! продолжаются один раз сами или честно переходят в «Нужен ты» (`recover`).
//!
//! Шаги процессов (`run_workflow_step`) и делегаты здесь не ведутся: у процессов свой путь, а делегат — часть хода начальника.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::store::{now, uid};
use crate::turn::{RoutineRef, StartError, TurnOptions};
use crate::App;

/// Закрытых записей храним не больше этого и не старше `KEEP_MS`.
const KEEP_CLOSED: usize = 200;
const KEEP_MS: i64 = 7 * 24 * 3600 * 1000;
/// Шагов в записи: старые отбрасываются, нужны последние.
const MAX_STEPS: usize = 30;
/// Сколько последних шагов называем в заметке продолжения.
const NOTE_STEPS: usize = 12;
const LABEL_CHARS: usize = 80;

/// Что шаг делает с миром: по этому выбирается, можно ли повторять его после обрыва.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Effect {
    /// Только читает.
    Read,
    /// Меняет файлы и состояние на этом компьютере.
    Local,
    /// Меняет что-то снаружи (отправка, публикация, оплата, действия на сайте): повторять вслепую нельзя.
    External,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StepState {
    Started,
    Ok,
    Err,
}

/// Почему запуск остановился. Объявлено здесь для задач про лимиты и повторы; пока ставятся только `Human` и `Unknown`.
#[allow(dead_code)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StopReason {
    Limit,
    Network,
    Login,
    Human,
    Budget,
    Check,
    Stuck,
    Unknown,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Origin {
    Human,
    Routine { id: String, title: String },
    Goal { id: String, title: String, run: crate::goals::Run },
    Webhook { id: String, title: String },
    Team { task_id: String },
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum RunState {
    Running,
    /// Ждёт срока (сброс лимита): заполняют задачи про лимиты.
    #[allow(dead_code)]
    Paused { until_ms: i64, reason: StopReason },
    NeedsHuman { reason: StopReason, text: String },
    Done,
    Failed { reason: StopReason, text: String },
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Step {
    /// Номер вызова у модели или агента: по нему шаг получает итог.
    pub call_id: String,
    pub tool: String,
    /// Короткая подпись для человека, без секретов.
    pub label: String,
    pub effect: Effect,
    pub state: StepState,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Run {
    pub id: String,
    pub bot_id: String,
    pub origin: Origin,
    /// Параметры хода, чтобы повторить его так же: источник, «без человека», тихий режим, пол осторожности.
    pub source: String,
    pub unattended: bool,
    pub quiet: bool,
    pub floor: Option<String>,
    pub started_ms: i64,
    pub updated_ms: i64,
    pub state: RunState,
    /// Сколько раз ход уже продолжали после обрыва.
    pub attempts: u32,
    pub steps: Vec<Step>,
    /// Заметка прогресса (ложится в следующий запуск).
    #[serde(default)]
    pub note: String,
}

impl Run {
    pub fn new(bot_id: &str, origin: Origin, opts: &TurnOptions) -> Self {
        let at = now();
        Self {
            id: uid("run"),
            bot_id: bot_id.to_string(),
            origin,
            source: opts.source.clone().unwrap_or_else(|| "chat".into()),
            unattended: opts.unattended,
            quiet: opts.quiet,
            floor: opts.approval_floor.clone(),
            started_ms: at,
            updated_ms: at,
            state: RunState::Running,
            attempts: 0,
            steps: Vec::new(),
            note: String::new(),
        }
    }

    fn is_running(&self) -> bool {
        matches!(self.state, RunState::Running)
    }

    /// Шаг, у которого результат неизвестен, а действие внешнее.
    fn unknown_external(&self) -> Option<&Step> {
        self.steps.iter().rev().find(|s| s.state == StepState::Started && s.effect == Effect::External)
    }

    /// Параметры продолжающего хода: те же, что у прерванного.
    fn options(&self) -> TurnOptions {
        let (routine, run) = match &self.origin {
            Origin::Routine { id, title } | Origin::Webhook { id, title } => (Some(RoutineRef { id: id.clone(), title: title.clone() }), None),
            Origin::Goal { id, title, run } => (Some(RoutineRef { id: id.clone(), title: title.clone() }), Some(run.clone())),
            Origin::Human | Origin::Team { .. } => (None, None),
        };
        TurnOptions {
            source: Some(self.source.clone()),
            unattended: self.unattended,
            routine,
            quiet: self.quiet,
            approval_floor: self.floor.clone(),
            run,
            origin: Some(self.origin.clone()),
            resume: Some(self.id.clone()),
            ..TurnOptions::default()
        }
    }

    /// Первое сообщение продолжения: что прервало, что уже сделано, что нельзя повторять.
    pub fn continuation_note(&self) -> String {
        let last = self.steps.last().map(|s| s.label.as_str()).unwrap_or("начало хода");
        let tail = self.steps.len().saturating_sub(NOTE_STEPS);
        let done: Vec<&str> = self.steps[tail..].iter().filter(|s| s.state == StepState::Ok).map(|s| s.label.as_str()).collect();
        let mut external: Vec<&str> = self.steps.iter().filter(|s| s.state == StepState::Ok && s.effect == Effect::External).map(|s| s.label.as_str()).collect();
        external.dedup();
        let mut text = format!("Тебя прервал перезапуск программы на шаге «{last}».");
        if !done.is_empty() {
            text.push_str(&format!(" Уже сделано: {}.", done.join("; ")));
        }
        if !external.is_empty() {
            text.push_str(&format!(" Внешние действия ({}) уже выполнены — не повторяй их.", external.join("; ")));
        }
        if let Some(step) = self.unknown_external() {
            text.push_str(&format!(" Шаг «{}» оборвался на полпути, прошёл ли он — неизвестно: сначала проверь, и повторяй, только если он не прошёл.", step.label));
        }
        text.push_str(" Проверь локальные изменения и доведи задание до конца.");
        text
    }
}

#[derive(Default)]
struct Inner {
    items: Vec<Run>,
    /// Бот → запись хода, который идёт сейчас (в памяти: после перезапуска пусто).
    active: HashMap<String, String>,
}

pub struct Runs {
    path: PathBuf,
    inner: Mutex<Inner>,
    /// Что было незакрыто при запуске программы: их и разбирает `recover`.
    stale: Vec<String>,
}

impl Runs {
    pub fn open(dir: &Path) -> Self {
        let path = dir.join("runs.json");
        let mut items: Vec<Run> = std::fs::read_to_string(&path).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default();
        prune(&mut items);
        let stale = items.iter().filter(|r| r.is_running()).map(|r| r.id.clone()).collect();
        Self { path, inner: Mutex::new(Inner { items, active: HashMap::new() }), stale }
    }

    fn save(&self, items: &[Run]) {
        let Ok(text) = serde_json::to_string(items) else { return };
        let tmp = self.path.with_extension("json.tmp");
        if std::fs::write(&tmp, text).and_then(|_| std::fs::rename(&tmp, &self.path)).is_err() {
            eprintln!("[runs] не смог записать runs.json");
        }
    }

    /// Изменить запись и сохранить. Возвращает её копию.
    pub fn update(&self, id: &str, change: impl FnOnce(&mut Run)) -> Option<Run> {
        let mut inner = self.inner.lock().unwrap();
        let run = inner.items.iter_mut().find(|r| r.id == id)?;
        change(run);
        run.updated_ms = now();
        let copy = run.clone();
        self.save(&inner.items);
        Some(copy)
    }

    pub fn get(&self, id: &str) -> Option<Run> {
        self.inner.lock().unwrap().items.iter().find(|r| r.id == id).cloned()
    }

    /// Новая запись: ход начинается.
    pub fn begin(&self, run: Run) -> String {
        let mut inner = self.inner.lock().unwrap();
        let id = run.id.clone();
        inner.active.insert(run.bot_id.clone(), id.clone());
        inner.items.push(run);
        self.save(&inner.items);
        id
    }

    /// Продолжающий ход берёт уже существующую запись.
    pub fn activate(&self, id: &str) -> bool {
        let mut inner = self.inner.lock().unwrap();
        let Some(bot_id) = inner.items.iter().find(|r| r.id == id).map(|r| r.bot_id.clone()) else { return false };
        inner.active.insert(bot_id, id.to_string());
        true
    }

    /// Ход закончился: запись закрыта, шаги закрытой записи не нужны.
    pub fn close(&self, bot_id: &str, state: RunState) {
        let mut inner = self.inner.lock().unwrap();
        let Some(id) = inner.active.remove(bot_id) else { return };
        if let Some(run) = inner.items.iter_mut().find(|r| r.id == id) {
            if matches!(state, RunState::Done) {
                run.steps.clear();
            }
            run.state = state;
            run.updated_ms = now();
        }
        prune(&mut inner.items);
        self.save(&inner.items);
    }

    /// Шаг начался. Вызывается до выполнения: если процесс умрёт, запись знает, что шло.
    pub fn step_start(&self, bot_id: &str, call_id: &str, tool: &str, label: &str, effect: Effect) {
        self.on_active(bot_id, |run| {
            push_step(run, call_id, tool, label, effect, StepState::Started);
            true
        });
    }

    pub fn step_end(&self, bot_id: &str, call_id: &str, ok: bool) {
        self.on_active(bot_id, |run| set_step(run, call_id, if ok { StepState::Ok } else { StepState::Err }));
    }

    /// Строка хода внешнего агента (ACP): новый шаг или смена состояния прежнего.
    pub fn activity(&self, bot_id: &str, row: &Value) {
        let Some(call_id) = row["id"].as_str() else { return };
        let state = match row["status"].as_str() {
            Some("ok") => StepState::Ok,
            Some("error" | "denied") => StepState::Err,
            _ => StepState::Started,
        };
        let (tool, label, effect) = crate::tools::activity_step(row);
        self.on_active(bot_id, |run| {
            if run.steps.iter().any(|s| s.call_id == call_id) {
                return set_step(run, call_id, state);
            }
            push_step(run, call_id, &tool, &label, effect, state);
            true
        });
    }

    /// Изменить запись идущего хода бота; `change` отвечает, было ли что менять (без изменения файл не пишем).
    fn on_active(&self, bot_id: &str, change: impl FnOnce(&mut Run) -> bool) {
        let mut inner = self.inner.lock().unwrap();
        let Some(id) = inner.active.get(bot_id).cloned() else { return };
        let Some(run) = inner.items.iter_mut().find(|r| r.id == id) else { return };
        if change(run) {
            run.updated_ms = now();
            self.save(&inner.items);
        }
    }

    /// Запись, ждущая человека: для «Нужен ты».
    pub fn needs(&self) -> Vec<Run> {
        self.inner.lock().unwrap().items.iter().filter(|r| matches!(r.state, RunState::NeedsHuman { .. })).cloned().collect()
    }

    /// Прерванный ход командной задачи, который уже решено продолжать: его подхватит `team_tasks::run`.
    pub fn pending_for_task(&self, task_id: &str) -> Option<Run> {
        let inner = self.inner.lock().unwrap();
        inner
            .items
            .iter()
            .find(|r| r.is_running() && matches!(&r.origin, Origin::Team { task_id: t } if t == task_id) && !inner.active.values().any(|a| *a == r.id))
            .cloned()
    }
}

fn push_step(run: &mut Run, call_id: &str, tool: &str, label: &str, effect: Effect, state: StepState) {
    run.steps.push(Step { call_id: call_id.into(), tool: tool.into(), label: label.chars().take(LABEL_CHARS).collect(), effect, state });
    let extra = run.steps.len().saturating_sub(MAX_STEPS);
    run.steps.drain(..extra);
}

/// `true`, если состояние шага изменилось.
fn set_step(run: &mut Run, call_id: &str, state: StepState) -> bool {
    match run.steps.iter_mut().rev().find(|s| s.call_id == call_id) {
        Some(step) if step.state != state => {
            step.state = state;
            true
        }
        _ => false,
    }
}

fn prune(items: &mut Vec<Run>) {
    let at = now();
    items.retain(|r| r.is_running() || matches!(r.state, RunState::NeedsHuman { .. } | RunState::Paused { .. }) || at - r.updated_ms < KEEP_MS);
    let closed = items.iter().filter(|r| !r.is_running()).count();
    let mut drop = closed.saturating_sub(KEEP_CLOSED);
    items.retain(|r| {
        let old = drop > 0 && matches!(r.state, RunState::Done | RunState::Failed { .. });
        if old {
            drop -= 1;
        }
        !old
    });
}

// ---------------------------------------------------------------------------
// Старт программы и продолжение
// ---------------------------------------------------------------------------

/// Один раз при запуске, до расписания и очередей: каждая запись, оставшаяся «идёт», либо продолжается,
/// либо честно ждёт человека. Командные задачи отсюда только решаются: ход запускает `team_tasks::resume`.
pub async fn recover(app: &Arc<App>) {
    for id in app.runs.stale.clone() {
        let Some(run) = app.runs.get(&id).filter(Run::is_running) else { continue };
        if app.store.bot(&run.bot_id).is_none() {
            close_stopped(app, &run, "бота больше нет").await;
        } else if let Some(step) = run.unknown_external() {
            let text = format!("Прервали во время: {}. Проверь, прошло ли, и нажми «Продолжить».", step.label);
            needs_human(app, &run, &text).await;
        } else if run.attempts >= 1 {
            needs_human(app, &run, "Ход прерывался дважды — нужен ты.").await;
        } else if let Err(e) = continue_run(app, &id).await {
            needs_human(app, &run, &format!("Не смог продолжить: {e}")).await;
        }
    }
    // Очередь не стоит после перезапуска: сообщения, которые ждали, идут теперь.
    for bot in app.store.bots() {
        crate::turn::drain_queue(app, &bot.id).await;
    }
}

/// Продолжить запись: счётчик попыток растёт, ход идёт с заметкой «что уже сделано».
/// Командную задачу запускает её собственный путь (она ждёт в очереди задач).
pub async fn continue_run(app: &Arc<App>, id: &str) -> Result<(), String> {
    let run = app.runs.update(id, |r| {
        r.attempts += 1;
        r.state = RunState::Running;
    });
    let Some(run) = run else { return Err("запись не найдена".into()) };
    if let Origin::Team { task_id } = &run.origin {
        return crate::team_tasks::requeue(app, task_id);
    }
    let note = run.continuation_note();
    match crate::turn::start_turn(app, &run.bot_id, &note, run.options()) {
        Ok(()) => Ok(()),
        Err(StartError { message, .. }) => {
            app.runs.update(id, |r| r.attempts = r.attempts.saturating_sub(1));
            Err(message)
        }
    }
}

/// Человек нажал «Продолжить» на карточке «Нужен ты».
pub async fn resume_by_human(app: &Arc<App>, id: &str) -> Result<(), String> {
    let run = app.runs.get(id).filter(|r| matches!(r.state, RunState::NeedsHuman { .. })).ok_or("запись уже не ждёт человека")?;
    let held = run.state.clone();
    continue_run(app, id).await.inspect_err(|_| {
        app.runs.update(id, |r| r.state = held);
    })
}

/// Человек отказался продолжать: запись закрыта.
pub async fn dismiss(app: &Arc<App>, id: &str) -> Result<(), String> {
    let run = app.runs.get(id).filter(|r| matches!(r.state, RunState::NeedsHuman { .. })).ok_or("запись уже не ждёт человека")?;
    close_stopped(app, &run, "отменено человеком").await;
    Ok(())
}

/// Ход ждёт человека: строка в чате бота, запись в «Нужен ты», рутина и задача команды отмечены.
async fn needs_human(app: &Arc<App>, run: &Run, text: &str) {
    app.runs.update(&run.id, |r| r.state = RunState::NeedsHuman { reason: StopReason::Unknown, text: text.to_string() });
    let entry = json!({ "id": uid("msg"), "role": "assistant", "text": text, "at": now(), "runId": run.id });
    if let Ok(saved) = app.store.append_message(&run.bot_id, entry).await {
        app.turns.emit(json!({ "type": "message", "botId": run.bot_id, "message": saved }));
    }
    app.turns.emit(json!({ "type": "state_changed" }));
    settle_origin(app, run, "needs", text).await;
}

/// Запись закрыта без продолжения.
async fn close_stopped(app: &Arc<App>, run: &Run, why: &str) {
    app.runs.update(&run.id, |r| r.state = RunState::Failed { reason: StopReason::Human, text: why.to_string() });
    app.turns.emit(json!({ "type": "state_changed" }));
    settle_origin(app, run, "stopped", why).await;
}

/// Что было связано с запуском (рутина, цель, задача команды), больше не «идёт».
async fn settle_origin(app: &Arc<App>, run: &Run, status: &str, text: &str) {
    match &run.origin {
        Origin::Routine { id, title } | Origin::Webhook { id, title } | Origin::Goal { id, title, .. } => {
            let routine = RoutineRef { id: id.clone(), title: title.clone() };
            crate::scheduler::routine_finished(app, &routine, &run.bot_id, status, run.started_ms).await;
            if let Origin::Goal { run: goal_run, .. } = &run.origin {
                crate::goals::finished(app, &run.bot_id, &Some(goal_run.clone()), status).await;
            }
        }
        Origin::Team { task_id } => crate::team_tasks::finish_interrupted(app, task_id, text).await,
        Origin::Human => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run_with(steps: Vec<(&str, Effect, StepState)>) -> Run {
        let mut run = Run::new("bot", Origin::Human, &TurnOptions::default());
        for (i, (label, effect, state)) in steps.into_iter().enumerate() {
            push_step(&mut run, &format!("c{i}"), "t", label, effect, state);
        }
        run
    }

    #[test]
    fn external_step_without_result_is_found() {
        let run = run_with(vec![("чтение", Effect::Read, StepState::Ok), ("отправка", Effect::External, StepState::Started)]);
        assert_eq!(run.unknown_external().unwrap().label, "отправка");
        let run = run_with(vec![("запись", Effect::Local, StepState::Started)]);
        assert!(run.unknown_external().is_none());
    }

    #[test]
    fn note_names_done_steps_and_external_ones() {
        let run = run_with(vec![
            ("чтение файла", Effect::Read, StepState::Ok),
            ("письмо клиенту", Effect::External, StepState::Ok),
            ("запись файла", Effect::Local, StepState::Started),
        ]);
        let note = run.continuation_note();
        assert!(note.contains("на шаге «запись файла»"), "{note}");
        assert!(note.contains("Уже сделано: чтение файла; письмо клиенту"), "{note}");
        assert!(note.contains("Внешние действия (письмо клиенту) уже выполнены — не повторяй"), "{note}");
        assert!(!note.contains("неизвестно"), "{note}");
    }

    #[test]
    fn steps_keep_only_the_last_ones() {
        let mut run = run_with(Vec::new());
        for i in 0..40 {
            push_step(&mut run, &format!("c{i}"), "t", "x", Effect::Read, StepState::Ok);
        }
        assert_eq!(run.steps.len(), MAX_STEPS);
        assert_eq!(run.steps[0].call_id, "c10");
    }
}
