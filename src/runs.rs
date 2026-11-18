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
/// Законченных внешних действий в записи.
const MAX_EXTERNAL: usize = 50;
/// Сколько последних шагов называем в заметке продолжения.
const NOTE_STEPS: usize = 12;
const LABEL_CHARS: usize = 80;

/// Что шаг делает с миром: по этому выбирается, можно ли повторять его после обрыва.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
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

/// Почему запуск остановился. Задачи про лимиты и повторы добавят свои причины.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StopReason {
    Human,
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
    NeedsHuman { reason: StopReason, text: String },
    Done,
    Failed { reason: StopReason, text: String },
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Step {
    /// Номер вызова у модели или агента: по нему шаг получает итог.
    pub call_id: String,
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
    /// Внешние действия, которые прошли: их после обрыва не повторяют. Шаги вытесняются по счёту, этот список — нет.
    #[serde(default)]
    pub done_external: Vec<String>,
    /// Внешние действия, которые кончились ошибкой: могли и пройти.
    #[serde(default)]
    pub failed_external: Vec<String>,
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
            done_external: Vec::new(),
            failed_external: Vec::new(),
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
        let mut text = format!("Тебя прервал перезапуск программы на шаге «{last}».");
        if !done.is_empty() {
            text.push_str(&format!(" Уже сделано: {}.", done.join("; ")));
        }
        if !self.done_external.is_empty() {
            text.push_str(&format!(" Внешние действия ({}) уже выполнены — не повторяй их.", self.done_external.join("; ")));
        }
        if !self.failed_external.is_empty() {
            text.push_str(&format!(" Внешние действия ({}) завершились ошибкой, но могли пройти — проверь, прежде чем повторять.", self.failed_external.join("; ")));
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

    /// Запись хода, который идёт у бота сейчас.
    pub fn active_id(&self, bot_id: &str) -> Option<String> {
        self.inner.lock().unwrap().active.get(bot_id).cloned()
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
    pub fn step_start(&self, bot_id: &str, call_id: &str, label: &str, effect: Effect) {
        self.on_active(bot_id, |run| {
            push_step(run, call_id, label, effect, StepState::Started);
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
        let (label, effect) = crate::tools::activity_step(row);
        self.on_active(bot_id, |run| {
            let Some(step) = run.steps.iter_mut().rev().find(|s| s.call_id == call_id) else {
                push_step(run, call_id, &label, effect, state);
                return true;
            };
            // Строгий класс остаётся: позднее обновление строки не делает внешнее действие безобидным.
            let stricter = effect > step.effect;
            step.effect = step.effect.max(effect);
            set_step(run, call_id, state) || stricter
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

    /// Занять запись для продолжения одним действием под замком: попытка +1, состояние «идёт».
    /// `from_human` — запись обязана ждать человека, иначе обязана идти (остаток прошлого запуска). Возвращает прежнее состояние.
    fn claim(&self, id: &str, from_human: bool) -> Option<(RunState, Run)> {
        let mut inner = self.inner.lock().unwrap();
        let run = inner.items.iter_mut().find(|r| r.id == id)?;
        if from_human != matches!(run.state, RunState::NeedsHuman { .. }) || (!from_human && !run.is_running()) {
            return None;
        }
        let held = std::mem::replace(&mut run.state, RunState::Running);
        run.attempts += 1;
        run.updated_ms = now();
        let copy = run.clone();
        self.save(&inner.items);
        Some((held, copy))
    }

    /// Ход не стартовал: возвращаем запись, как была, но только если никто её уже не взял.
    fn release(&self, id: &str, held: RunState) {
        let mut inner = self.inner.lock().unwrap();
        if inner.active.values().any(|a| a == id) {
            return;
        }
        if let Some(run) = inner.items.iter_mut().find(|r| r.id == id).filter(|r| r.is_running()) {
            run.state = held;
            run.attempts = run.attempts.saturating_sub(1);
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

fn push_step(run: &mut Run, call_id: &str, label: &str, effect: Effect, state: StepState) {
    run.steps.push(Step { call_id: call_id.into(), label: label.chars().take(LABEL_CHARS).collect(), effect, state });
    note_external(run, call_id);
    let extra = run.steps.len().saturating_sub(MAX_STEPS);
    run.steps.drain(..extra);
}

/// `true`, если состояние шага изменилось.
fn set_step(run: &mut Run, call_id: &str, state: StepState) -> bool {
    match run.steps.iter_mut().rev().find(|s| s.call_id == call_id) {
        Some(step) if step.state != state => {
            step.state = state;
            note_external(run, call_id);
            true
        }
        _ => false,
    }
}

/// Законченное внешнее действие — в отдельный список: шаги вытесняются по счёту, а «не повторяй» должно помниться.
fn note_external(run: &mut Run, call_id: &str) {
    let Some(step) = run.steps.iter().rev().find(|s| s.call_id == call_id) else { return };
    let list = match (step.effect, step.state) {
        (Effect::External, StepState::Ok) => &mut run.done_external,
        (Effect::External, StepState::Err) => &mut run.failed_external,
        _ => return,
    };
    if !list.contains(&step.label) {
        list.push(step.label.clone());
        let extra = list.len().saturating_sub(MAX_EXTERNAL);
        list.drain(..extra);
    }
}

fn prune(items: &mut Vec<Run>) {
    let at = now();
    items.retain(|r| r.is_running() || matches!(r.state, RunState::NeedsHuman { .. }) || at - r.updated_ms < KEEP_MS);
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
        } else if let Some((_, title)) = app.turns.card_for_run(&id) {
            // Карточка переживает перезапуск: ход продолжит решение человека, а не заметка об обрыве.
            needs_human(app, &run, StopReason::Human, &waiting_text(&title)).await;
        } else if let Some(step) = run.unknown_external() {
            let text = format!("Прервали во время: {}. Проверь, прошло ли, и нажми «Продолжить».", step.label);
            needs_human(app, &run, StopReason::Unknown, &text).await;
        } else if run.attempts >= 1 {
            needs_human(app, &run, StopReason::Unknown, "Ход прерывался дважды — нужен ты.").await;
        } else if let Some((_, claimed)) = app.runs.claim(&id, false) {
            if let Err(e) = launch(app, &claimed, &claimed.continuation_note()) {
                needs_human(app, &run, StopReason::Unknown, &format!("Не смог продолжить: {e}")).await;
            }
        }
    }
    // Очередь не стоит после перезапуска: сообщения, которые ждали, идут теперь.
    for bot in app.store.bots() {
        crate::turn::drain_queue(app, &bot.id).await;
    }
}

/// Запустить ход продолжения. Командную задачу запускает её собственный путь (она ждёт в очереди задач).
fn launch(app: &Arc<App>, run: &Run, note: &str) -> Result<(), String> {
    if let Origin::Team { task_id } = &run.origin {
        return crate::team_tasks::requeue(app, task_id);
    }
    crate::turn::start_turn(app, &run.bot_id, note, run.options()).map_err(|StartError { message, .. }| message)
}

/// Человек нажал «Продолжить» на карточке «Нужен ты». Два нажатия подряд: второе получает отказ и записи не касается.
/// Запись, ждущая решения по карточке, так не продолжается: её продолжает само решение.
pub async fn resume_by_human(app: &Arc<App>, id: &str) -> Result<(), String> {
    if app.turns.card_for_run(id).is_some() {
        return Err("ход ждёт решения по карточке".into());
    }
    let run = app.runs.get(id).ok_or("запись уже не ждёт человека")?;
    resume(app, id, &run.continuation_note()).await
}

/// Продолжить ждущую запись с этим первым сообщением (решение человека или заметка об обрыве).
pub async fn resume(app: &Arc<App>, id: &str, note: &str) -> Result<(), String> {
    let (held, run) = app.runs.claim(id, true).ok_or("запись уже не ждёт человека")?;
    launch(app, &run, note).inspect_err(|_| app.runs.release(id, held))
}

/// Человек отказался продолжать: запись закрыта.
pub async fn dismiss(app: &Arc<App>, id: &str) -> Result<(), String> {
    if app.turns.card_for_run(id).is_some() {
        return Err("ход ждёт решения по карточке".into());
    }
    let run = app.runs.get(id).filter(|r| matches!(r.state, RunState::NeedsHuman { .. })).ok_or("запись уже не ждёт человека")?;
    close_stopped(app, &run, "отменено человеком").await;
    Ok(())
}

/// Ход ждёт человека: строка в чате бота, запись в «Нужен ты», рутина и задача команды отмечены.
async fn needs_human(app: &Arc<App>, run: &Run, reason: StopReason, text: &str) {
    app.runs.update(&run.id, |r| r.state = RunState::NeedsHuman { reason, text: text.to_string() });
    let entry = json!({ "id": uid("msg"), "role": "assistant", "text": text, "at": now(), "runId": run.id });
    if let Ok(saved) = app.store.append_message(&run.bot_id, entry).await {
        app.turns.emit(json!({ "type": "message", "botId": run.bot_id, "message": saved }));
    }
    app.turns.emit(json!({ "type": "state_changed" }));
    settle_origin(app, run, "needs", text).await;
}

/// Что записано в запуске, который ждёт решения по карточке.
pub fn waiting_text(title: &str) -> String {
    format!("Ждёт решения человека по: {title}. Ход остановлен до ответа.")
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
            push_step(&mut run, &format!("c{i}"), label, effect, state);
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
    fn finished_external_steps_outlive_the_step_cap_and_do_not_repeat() {
        let mut run = run_with(Vec::new());
        push_step(&mut run, "mail", "письмо клиенту", Effect::External, StepState::Ok);
        push_step(&mut run, "pay", "оплата", Effect::External, StepState::Err);
        push_step(&mut run, "mail2", "письмо клиенту", Effect::External, StepState::Ok);
        for i in 0..40 {
            push_step(&mut run, &format!("r{i}"), "чтение", Effect::Read, StepState::Ok);
        }
        assert!(run.steps.iter().all(|s| s.effect == Effect::Read), "внешние шаги вытеснены по счёту");
        assert_eq!(run.done_external, vec!["письмо клиенту"], "без повтора");
        let note = run.continuation_note();
        assert!(note.contains("Внешние действия (письмо клиенту) уже выполнены"), "{note}");
        assert!(note.contains("(оплата) завершились ошибкой"), "{note}");
    }

    #[test]
    fn a_goal_self_check_keeps_its_run_kind_after_a_restart() {
        let origin = Origin::Goal { id: "checkin_bot".into(), title: "Самопроверка".into(), run: crate::goals::Run::CheckIn };
        let run = Run::new("bot", origin, &TurnOptions::default());
        assert_eq!(run.options().run, Some(crate::goals::Run::CheckIn));
        assert!(run.options().routine.is_some());
    }

    #[test]
    fn steps_keep_only_the_last_ones() {
        let mut run = run_with(Vec::new());
        for i in 0..40 {
            push_step(&mut run, &format!("c{i}"), "x", Effect::Read, StepState::Ok);
        }
        assert_eq!(run.steps.len(), MAX_STEPS);
        assert_eq!(run.steps[0].call_id, "c10");
    }
}
