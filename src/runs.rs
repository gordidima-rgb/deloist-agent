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

/// Почему запуск остановился: от причины зависит, ждать, повторять или звать человека.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StopReason {
    /// Лимит провайдера или окно подписки.
    Limit,
    /// Дневной предел токенов команды или месячный предел бота.
    Budget,
    /// Бот сам ждёт времени или вебхука (`await_event`).
    Event,
    Human,
    /// Связь, таймаут, 5xx, перегрузка: пройдёт само.
    Network,
    /// Нужен вход в провайдера (агент просит войти, ключ не подошёл).
    Login,
    /// Проверка не прошла: бот сам закрыл ход итогом «не получилось».
    Check,
    /// Застрял: агент молчал или запуск шёл дольше положенного.
    Stuck,
    Unknown,
}

impl StopReason {
    /// Ошибка, которая сама не пройдёт: серия таких в рутине ставит её на паузу.
    pub fn is_lasting(self) -> bool {
        matches!(self, Self::Login | Self::Check | Self::Unknown)
    }

    /// Ошибка хода (а не пауза, лимит или ожидание человека): растит серию неудач рутины.
    pub fn is_failure(self) -> bool {
        matches!(self, Self::Network | Self::Login | Self::Check | Self::Stuck | Self::Unknown)
    }

    /// Причина словами для человека (строка в чате, журнал).
    pub fn label(self, english: bool) -> &'static str {
        match (self, english) {
            (Self::Network, false) => "сеть",
            (Self::Network, true) => "network",
            (Self::Login, false) => "нужен вход",
            (Self::Login, true) => "sign-in needed",
            (Self::Check, false) => "проверка не прошла",
            (Self::Check, true) => "a check failed",
            (Self::Stuck, false) => "застрял",
            (Self::Stuck, true) => "stuck",
            (_, false) => "неизвестная ошибка",
            (_, true) => "unknown error",
        }
    }
}

/// Запуск шёл дольше положенного (`TurnOptions`: рутина 3 ч, цель 12 ч).
pub const RUN_TIMEOUT: &str = "Превышено время запуска";

/// Причина остановки по тексту ошибки хода — единственное место, где ошибку разбирают.
pub fn reason_of(error: &str) -> StopReason {
    use crate::drivers::{api_status, is_limit_error, is_transient_model_error};
    let lower = error.to_lowercase();
    if error.starts_with(RUN_TIMEOUT) || lower.contains("молчал") {
        StopReason::Stuck
    } else if is_limit_error(error) {
        StopReason::Limit
    } else if matches!(api_status(error), Some(401 | 403)) || ["нужен вход", "not logged in", "needs auth"].iter().any(|w| lower.contains(w)) {
        StopReason::Login
    } else if is_transient_model_error(error) {
        StopReason::Network
    } else {
        StopReason::Unknown
    }
}

/// Через сколько повторить после обрыва связи: 1, 5, 15 минут, дальше решает человек.
pub fn network_retry_ms(retries: u32) -> Option<i64> {
    let minute = crate::schedule::hour_ms() / 60;
    [1, 5, 15].get(retries as usize).map(|m| m * minute)
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
    /// Ход ждёт срока, а не человека: сбросится лимит, придёт событие. Сам продолжится в `resume_due`.
    Paused {
        until_ms: i64,
        reason: StopReason,
        /// Номер вебхук-рутины бота, чей вызов тоже будит запись раньше срока.
        #[serde(default)]
        webhook: Option<String>,
        /// Для человека: чей лимит (провайдер) или что ждёт бот.
        #[serde(default)]
        note: String,
    },
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
    /// Сколько раз подряд ход вставал на паузу из-за лимита: пятая подряд — «Нужен ты».
    #[serde(default)]
    pub pauses: u32,
    /// Человек нажал «Продолжить сейчас»: этот ход не останавливают заранее ни отдых модели, ни почти пустое окно подписки.
    #[serde(default)]
    pub forced: bool,
    /// Первое сообщение продолжения, если ход ещё ничего не успел: постановка рутины вместо «продолжай».
    #[serde(default)]
    pub resume_text: Option<String>,
    /// Об этой паузе уже сказано уведомлением Windows.
    #[serde(default)]
    pub notified: bool,
    /// Ход уже что-то менял (шаг не из чтения): после обрыва связи его не повторяют сами.
    #[serde(default)]
    pub acted: bool,
    /// Сколько раз ход уже вставал на паузу из-за связи: после третьей — «Нужен ты».
    #[serde(default)]
    pub retries: u32,
}

/// Из какого состояния запись берут на продолжение.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Held {
    /// Остаток прошлого запуска (программу закрыли посреди хода).
    Stale,
    Human,
    Paused,
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
            pauses: 0,
            forced: false,
            resume_text: None,
            notified: false,
            acted: false,
            retries: 0,
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

    /// Что записано в состоянии «ждёт человека».
    pub fn wait_text(&self) -> String {
        match &self.state {
            RunState::NeedsHuman { text, .. } => text.clone(),
            _ => String::new(),
        }
    }

    /// Первое сообщение продолжения: что прервало, что уже сделано, что нельзя повторять.
    pub fn continuation_note(&self) -> String {
        let last = self.steps.last().map(|s| s.label.as_str()).unwrap_or("начало хода");
        let tail = self.steps.len().saturating_sub(NOTE_STEPS);
        let done: Vec<&str> = self.steps[tail..].iter().filter(|s| s.state == StepState::Ok).map(|s| s.label.as_str()).collect();
        let cause = match &self.state {
            RunState::NeedsHuman { reason: StopReason::Network, .. } => "Связь с провайдером оборвалась",
            RunState::NeedsHuman { reason: StopReason::Login, .. } => "Нужен был вход в провайдера, теперь он выполнен",
            _ => "Тебя прервал перезапуск программы",
        };
        let mut text = format!("{cause} на шаге «{last}».");
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
    /// Бот → пауза, которую он попросил сам (`await_event`) и которая ждёт конца шага.
    waits: HashMap<String, RunState>,
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
        Self { path, inner: Mutex::new(Inner { items, ..Inner::default() }), stale }
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
            match state {
                RunState::Paused { reason: StopReason::Limit, .. } => run.pauses += 1,
                RunState::Paused { reason: StopReason::Network, .. } => run.retries += 1,
                _ => {}
            }
            run.forced = false;
            run.resume_text = None;
            run.notified = false;
            run.state = state;
            run.updated_ms = now();
        }
        // Ход встал на паузу, а у рутины уже есть другая: старая устарела, проснуться должна одна.
        if let Some(run) = inner.items.iter().find(|r| r.id == id).filter(|r| matches!(r.state, RunState::Paused { .. })) {
            if let Some(routine) = origin_routine(&run.origin).map(str::to_string) {
                drop_pauses_of(&mut inner.items, &routine, Some(&id));
            }
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
            run.acted |= step.effect > Effect::Read;
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
    /// Запись обязана быть в состоянии `from`. Пауза попыткой не считается: она не обрыв. Возвращает прежнее состояние.
    fn claim(&self, id: &str, from: Held) -> Option<(RunState, Run)> {
        let mut inner = self.inner.lock().unwrap();
        let run = inner.items.iter_mut().find(|r| r.id == id)?;
        let state_is = match from {
            Held::Stale => run.is_running(),
            Held::Human => matches!(run.state, RunState::NeedsHuman { .. }),
            Held::Paused => matches!(run.state, RunState::Paused { .. }),
        };
        if !state_is {
            return None;
        }
        let held = std::mem::replace(&mut run.state, RunState::Running);
        if from != Held::Paused {
            run.attempts += 1;
        }
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
            if !matches!(held, RunState::Paused { .. }) {
                run.attempts = run.attempts.saturating_sub(1);
            }
            run.state = held;
            self.save(&inner.items);
        }
    }

    /// Сколько раз идущий ход бота уже вставал на паузу из-за лимита.
    pub fn pauses(&self, bot_id: &str) -> u32 {
        self.on_active_read(bot_id, |r| r.pauses).unwrap_or(0)
    }

    /// Ход идущего бота: менял ли он что-то и сколько раз уже ждал связь.
    pub fn network_state(&self, bot_id: &str) -> (bool, u32) {
        self.on_active_read(bot_id, |r| (r.acted, r.retries)).unwrap_or((true, 0))
    }

    /// Какой-то запуск уже ждёт входа в этого провайдера: второй карточки «Нужен ты» не нужно.
    pub fn login_waiting(&self, text: &str) -> bool {
        let inner = self.inner.lock().unwrap();
        inner.items.iter().any(|r| matches!(&r.state, RunState::NeedsHuman { reason: StopReason::Login, text: t } if t == text))
    }

    /// Человек велел идущему ходу продолжать, не глядя на отдых модели.
    pub fn is_forced(&self, bot_id: &str) -> bool {
        self.on_active_read(bot_id, |r| r.forced).unwrap_or(false)
    }

    fn on_active_read<T>(&self, bot_id: &str, read: impl FnOnce(&Run) -> T) -> Option<T> {
        let inner = self.inner.lock().unwrap();
        let id = inner.active.get(bot_id)?;
        inner.items.iter().find(|r| r.id == *id).map(read)
    }

    /// Бот вызвал `await_event`: ход закончится паузой, когда шаг дойдёт до конца.
    pub fn request_wait(&self, bot_id: &str, state: RunState) -> bool {
        let mut inner = self.inner.lock().unwrap();
        let active = inner.active.contains_key(bot_id);
        if active {
            inner.waits.insert(bot_id.to_string(), state);
        }
        active
    }

    pub fn take_wait(&self, bot_id: &str) -> Option<RunState> {
        self.inner.lock().unwrap().waits.remove(bot_id)
    }

    /// Записи на паузе.
    pub fn paused(&self) -> Vec<Run> {
        self.inner.lock().unwrap().items.iter().filter(|r| matches!(r.state, RunState::Paused { .. })).cloned().collect()
    }

    /// Подошёл новый срок рутины: её прежние запуски, вставшие на паузу из-за лимита, сети или предела токенов,
    /// устарели. Без этого каждый срок, пока модель на лимите, добавлял ещё одну паузу, а потом все они
    /// просыпались по одной раз в 30 с (Маркетолог X, 19.11.2026: 12 пауз, пять — одной рутины).
    /// Ожидание, которое бот поставил сам (`await_event`), не трогаем.
    pub fn drop_stale_pauses(&self, routine_id: &str) -> usize {
        let mut inner = self.inner.lock().unwrap();
        let dropped = drop_pauses_of(&mut inner.items, routine_id, None);
        if dropped > 0 {
            self.save(&inner.items);
        }
        dropped
    }

    /// Лимит у рутины ещё не кончился: пауза ждёт срока, и новый срок рутины ход не начинает (иначе каждый час
    /// в тред писалась строка «ждёт лимит», а в журнал — ещё одна пауза: Маркетолог Reddit, 26.11.2026).
    pub fn limit_pause_ahead(&self, routine_id: &str) -> bool {
        let at = now();
        self.inner.lock().unwrap().items.iter().any(|r| {
            origin_routine(&r.origin) == Some(routine_id) && matches!(r.state, RunState::Paused { reason: StopReason::Limit, until_ms, .. } if until_ms > at)
        })
    }

    /// Рутина ждёт решения человека: карточка без ответа или остановка, которую человек не снял.
    pub fn routine_waits_for_human(&self, routine_id: &str) -> bool {
        self.inner.lock().unwrap().items.iter().any(|r| matches!(r.origin, Origin::Routine { .. }) && origin_routine(&r.origin) == Some(routine_id) && matches!(r.state, RunState::NeedsHuman { .. }))
    }

    /// Запись, которая сразу на паузе (ход ещё не начинался).
    fn add(&self, run: Run) {
        let mut inner = self.inner.lock().unwrap();
        inner.items.push(run);
        self.save(&inner.items);
    }

    /// Запись, ждущая человека: для «Нужен ты».
    pub fn needs(&self) -> Vec<Run> {
        self.inner.lock().unwrap().items.iter().filter(|r| matches!(r.state, RunState::NeedsHuman { .. })).cloned().collect()
    }

    /// Ход командной задачи, который остановился и ждёт человека (карточка без ответа).
    pub fn waiting_for_task(&self, task_id: &str) -> Option<Run> {
        let inner = self.inner.lock().unwrap();
        inner.items.iter().find(|r| matches!(r.state, RunState::NeedsHuman { .. }) && matches!(&r.origin, Origin::Team { task_id: t } if t == task_id)).cloned()
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

/// Паузы рутины (лимит, сеть, предел токенов) закрываются, кроме `keep`: у одной рутины живёт одна пауза.
fn drop_pauses_of(items: &mut [Run], routine_id: &str, keep: Option<&str>) -> usize {
    let mut dropped = 0;
    for run in items.iter_mut().filter(|r| origin_routine(&r.origin) == Some(routine_id) && Some(r.id.as_str()) != keep) {
        if matches!(run.state, RunState::Paused { reason: StopReason::Limit | StopReason::Network | StopReason::Budget, .. }) {
            run.state = RunState::Done;
            run.steps.clear();
            run.updated_ms = now();
            dropped += 1;
        }
    }
    dropped
}

fn push_step(run: &mut Run, call_id: &str, label: &str, effect: Effect, state: StepState) {
    run.acted |= effect > Effect::Read;
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
    let open = |r: &Run| r.is_running() || matches!(r.state, RunState::NeedsHuman { .. } | RunState::Paused { .. });
    items.retain(|r| open(r) || at - r.updated_ms < KEEP_MS);
    let closed = items.iter().filter(|r| !open(r)).count();
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
        } else if let Some((_, claimed)) = app.runs.claim(&id, Held::Stale) {
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

/// Запустить ход продолжения с первым сообщением `text`. Командную задачу запускает её собственный путь (она ждёт в очереди задач).
fn launch(app: &Arc<App>, run: &Run, text: &str) -> Result<(), String> {
    if let Origin::Team { task_id } = &run.origin {
        return crate::team_tasks::requeue(app, task_id, text);
    }
    crate::turn::start_turn(app, &run.bot_id, text, run.options()).map_err(|StartError { message, .. }| message)
}

/// Человек нажал «Продолжить» на карточке «Нужен ты». Два нажатия подряд: второе получает отказ и записи не касается.
/// Запись, ждущая решения по карточке, так не продолжается: её продолжает само решение.
pub async fn resume_by_human(app: &Arc<App>, id: &str) -> Result<(), String> {
    if app.turns.card_for_run(id).is_some() {
        return Err("ход ждёт решения по карточке".into());
    }
    let run = app.runs.get(id).ok_or("запись уже не ждёт человека")?;
    continue_with(app, id, &run.continuation_note()).await
}

/// Продолжить запись, ждущую человека, с первым сообщением `note`: решение по карточке или заметка об обрыве.
pub async fn continue_with(app: &Arc<App>, id: &str, note: &str) -> Result<(), String> {
    let (held, run) = app.runs.claim(id, Held::Human).ok_or("запись уже не ждёт человека")?;
    launch(app, &run, note).inspect_err(|_| app.runs.release(id, held))
}

// ---------------------------------------------------------------------------
// Паузы: лимит, предел токенов, ожидание события
// ---------------------------------------------------------------------------

// ---------------------------------------------------------------------------
// Инструмент `await_event`: бот сам встаёт на паузу
// ---------------------------------------------------------------------------

/// Дольше недели ждать нельзя: дальше это уже не ожидание, а рутина.
const MAX_WAIT_MS: i64 = 7 * 24 * 3600 * 1000;

pub fn is_wait_tool(name: &str) -> bool {
    name == "await_event"
}

pub fn wait_def() -> Value {
    json!({ "type": "function", "function": {
        "name": "await_event",
        "description": "Остановить это задание и продолжить позже: когда пройдёт время или придёт вызов вебхука. Ход после этого вызова кончается, \
            программа сама разбудит тебя и дошлёт данные. Ждать можно до 7 суток. Используй вместо цикла проверок: «проверю, когда придёт ответ».",
        "parameters": { "type": "object", "properties": {
            "in_minutes": { "type": "integer", "description": "Через сколько минут продолжить." },
            "at": { "type": "string", "description": "Во сколько продолжить, ЧЧ:ММ по часам человека (ближайшее такое время)." },
            "webhook": { "type": "string", "description": "Номер рутины-вебхука этого бота (list_routines): продолжить, когда его вызовут. Вместе с in_minutes или at — это крайний срок." },
            "note": { "type": "string", "description": "Чего ждёшь, коротко: человек увидит это в списке." },
        } },
    } })
}

/// Вызов `await_event`. Пауза начинается, когда шаг дойдёт до конца: тут только записывается просьба.
pub fn exec_wait(app: &Arc<App>, bot: &crate::store::Bot, args: &Value) -> Result<String, String> {
    let text = |key: &str| args[key].as_str().unwrap_or_default().trim();
    let at = now();
    let webhook = Some(text("webhook")).filter(|w| !w.is_empty());
    if let Some(id) = webhook {
        let hook = app.store.routine(id).filter(|r| r.hook && r.bot_id == bot.id);
        hook.ok_or("webhook: нужен номер рутины-вебхука этого бота из list_routines")?;
    }
    let until = if let Some(minutes) = args["in_minutes"].as_i64() {
        at + minutes.max(1).saturating_mul(60_000)
    } else if !text("at").is_empty() {
        crate::limit::next_clock_ms(text("at"), chrono::Local::now()).ok_or("at: время ЧЧ:ММ, например 14:30")?
    } else if webhook.is_some() {
        at + MAX_WAIT_MS
    } else {
        return Err("нужно in_minutes, at или webhook".into());
    };
    let until = until.min(at + MAX_WAIT_MS);
    let note: String = text("note").chars().take(200).collect();
    let state = RunState::Paused { until_ms: until, reason: StopReason::Event, webhook: webhook.map(str::to_string), note };
    if !app.runs.request_wait(&bot.id, state) {
        return Err("ждать можно только в запущенном задании".into());
    }
    Ok(format!("Ожидание принято: продолжу в {}. Заверши ход.", crate::scheduler::clock(until)))
}

/// Рутина, цель или вебхук, из которых вырос запуск.
pub fn origin_routine(origin: &Origin) -> Option<&str> {
    match origin {
        Origin::Routine { id, .. } | Origin::Webhook { id, .. } | Origin::Goal { id, .. } => Some(id),
        Origin::Human | Origin::Team { .. } => None,
    }
}

/// Первое сообщение продолжения после паузы.
fn resume_text(run: &Run, data: Option<&str>) -> String {
    if let Some(text) = &run.resume_text {
        return text.clone();
    }
    match (&run.state, data) {
        (_, Some(data)) => format!("Сработал вебхук, которого ты ждал.
{data}"),
        (RunState::Paused { reason: StopReason::Event, .. }, None) => "Срок ожидания вышел — продолжай задание с того места, где остановился.".into(),
        (RunState::Paused { reason: StopReason::Budget, .. }, None) => "Предел токенов сбросился — продолжай задание с того места, где остановился.".into(),
        (RunState::Paused { reason: StopReason::Network, .. }, None) => "Связь с провайдером оборвалась и вернулась — продолжай задание с того места, где остановился.".into(),
        _ => "Лимит сбросился — продолжай задание с того места, где остановился.".into(),
    }
}

/// Взять паузу на продолжение. `data` — тело вебхука, который её разбудил. `forced` — решил человек: отдых модели не мешает.
async fn resume(app: &Arc<App>, id: &str, forced: bool, data: Option<&str>) -> Result<(), String> {
    let (held, run) = app.runs.claim(id, Held::Paused).ok_or("запись уже не на паузе")?;
    let text = resume_text(&Run { state: held.clone(), ..run.clone() }, data);
    if forced {
        app.runs.update(id, |r| r.forced = true);
    }
    launch(app, &run, &text).inspect_err(|_| app.runs.release(id, held))?;
    if let Some(routine) = origin_routine(&run.origin) {
        let _ = app.store.set_routine_status(routine, "running").await;
    }
    app.turns.emit(json!({ "type": "state_changed" }));
    Ok(())
}

/// «Продолжить сейчас» на паузе.
pub async fn resume_now(app: &Arc<App>, id: &str) -> Result<(), String> {
    resume(app, id, true, None).await
}

/// Тик планировщика: паузы, чей срок вышел, продолжаются. Занятый бот ждёт следующего тика.
pub async fn resume_due(app: &Arc<App>) {
    let at = now();
    for run in app.runs.paused() {
        if matches!(run.state, RunState::Paused { until_ms, .. } if until_ms <= at) && !app.turns.is_busy(&run.bot_id) {
            // Рутина на паузе из-за ошибок: её старые ожидания лимита не просыпаются, пока человек не нажал «Возобновить».
            if origin_routine(&run.origin).and_then(|id| app.store.routine(id)).is_some_and(|r| r.error_paused) {
                app.runs.drop_stale_pauses(origin_routine(&run.origin).unwrap_or_default());
                continue;
            }
            if let Err(e) = resume(app, &run.id, false, None).await {
                eprintln!("[runs] пауза не продолжена: {e}");
            }
        }
    }
}

/// Пришёл вебхук рутины: ход этого бота, который его ждал, продолжается с данными вместо нового хода. `false` — никто не ждал.
pub async fn resume_on_webhook(app: &Arc<App>, routine_id: &str, bot_id: &str, data: &str) -> bool {
    let waiting = app.runs.paused().into_iter().find(|r| {
        r.bot_id == bot_id && matches!(&r.state, RunState::Paused { webhook: Some(w), .. } if w == routine_id)
    });
    match waiting {
        Some(run) => resume(app, &run.id, false, Some(data)).await.is_ok(),
        None => false,
    }
}

/// Ход ещё не начат, а ему уже нельзя (предел токенов): запись ждёт срока и потом идёт с этим текстом.
/// Рутина — одна запись: повторный срок, пока она ждёт, новой не заводит. Вызовы вебхука — каждый свой.
pub fn pause_unstarted(app: &Arc<App>, bot_id: &str, origin: Origin, opts: &TurnOptions, text: &str, until_ms: i64, reason: StopReason, note: &str) {
    let key = origin_routine(&origin).map(str::to_string).filter(|_| matches!(origin, Origin::Routine { .. }));
    if key.is_some() && app.runs.paused().iter().any(|r| r.bot_id == bot_id && origin_routine(&r.origin) == key.as_deref()) {
        return;
    }
    let mut run = Run::new(bot_id, origin, opts);
    run.state = RunState::Paused { until_ms, reason, webhook: None, note: note.to_string() };
    run.resume_text = Some(text.to_string());
    app.runs.add(run);
    app.turns.emit(json!({ "type": "state_changed" }));
}

/// Одно уведомление на окно сброса, а не на каждого бота: паузы с одним провайдером и одним сроком идут вместе.
pub async fn notify_pauses(app: &Arc<App>) {
    let mut groups: std::collections::BTreeMap<(String, i64), Vec<String>> = Default::default();
    for run in app.runs.paused().into_iter().filter(|r| !r.notified) {
        if let RunState::Paused { until_ms, reason: StopReason::Limit | StopReason::Budget, note, .. } = &run.state {
            groups.entry((note.clone(), *until_ms)).or_default().push(run.id.clone());
        }
    }
    for ((note, until), ids) in groups {
        for id in &ids {
            app.runs.update(id, |r| r.notified = true);
        }
        let body = format!("{note} — ждут ботов: {}, сброс в {}", ids.len(), crate::scheduler::clock(until));
        let _ = crate::reminders::send_text(app, "Задания на паузе", &body, &["windows".to_string()]).await;
    }
}

/// Человек отказался продолжать: запись закрыта.
pub async fn dismiss(app: &Arc<App>, id: &str) -> Result<(), String> {
    if app.turns.card_for_run(id).is_some() {
        return Err("ход ждёт решения по карточке".into());
    }
    let run = app.runs.get(id).filter(|r| matches!(r.state, RunState::NeedsHuman { .. } | RunState::Paused { .. })).ok_or("запись уже не ждёт человека")?;
    close_stopped(app, &run, "Отменено человеком").await;
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
    app.turns.drop_once(&run.bot_id, Some(&run.id));
    app.runs.update(&run.id, |r| r.state = RunState::Failed { reason: StopReason::Human, text: why.to_string() });
    app.turns.emit(json!({ "type": "state_changed" }));
    settle_origin(app, run, "stopped", why).await;
}

/// Что было связано с запуском (рутина, цель, задача команды), больше не «идёт».
async fn settle_origin(app: &Arc<App>, run: &Run, status: &str, text: &str) {
    match &run.origin {
        Origin::Routine { id, title } | Origin::Webhook { id, title } | Origin::Goal { id, title, .. } => {
            let routine = RoutineRef { id: id.clone(), title: title.clone() };
            crate::scheduler::routine_finished(app, &routine, &run.bot_id, status, None).await;
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
    fn the_error_text_tells_why_a_run_stopped() {
        let table = [
            ("API 429: Rate limit exceeded", StopReason::Limit),
            ("API 402: Grok Build usage balance", StopReason::Limit),
            ("The free OpenCode models reached their usage limit", StopReason::Limit),
            ("quota exceeded for this key", StopReason::Limit),
            ("API 503: провайдер не ответил", StopReason::Network),
            ("API 408: timeout", StopReason::Network),
            ("error sending request: connection refused", StopReason::Network),
            ("dns error: no such host", StopReason::Network),
            ("server overloaded", StopReason::Network),
            ("API 401: Incorrect API key provided", StopReason::Login),
            ("API 403: forbidden", StopReason::Login),
            ("Агенту нужен вход (claude): вход не завершён за 5 минут", StopReason::Login),
            ("нет ключа Anthropic: впиши его в настройках", StopReason::Unknown),
            ("Агент молчал 20 минут — ход остановлен.", StopReason::Stuck),
            (RUN_TIMEOUT, StopReason::Stuck),
            ("API 400: invalid request", StopReason::Unknown),
            ("Ошибка инструмента: файл не найден", StopReason::Unknown),
            ("остановлено", StopReason::Unknown),
        ];
        for (text, reason) in table {
            assert_eq!(reason_of(text), reason, "{text}");
        }
    }

    #[test]
    fn only_an_untouched_run_waits_for_the_network_on_its_own() {
        let mut run = run_with(vec![("чтение", Effect::Read, StepState::Ok)]);
        assert!(!run.acted, "чтение ничего не меняет");
        push_step(&mut run, "w", "запись", Effect::Local, StepState::Started);
        assert!(run.acted);
        let minute = crate::schedule::hour_ms() / 60;
        assert_eq!([0, 1, 2, 3].map(network_retry_ms), [Some(minute), Some(5 * minute), Some(15 * minute), None]);
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

#[cfg(test)]
mod stale_pause_tests {
    use super::*;

    fn paused(routine: &str, reason: StopReason) -> Run {
        let origin = Origin::Routine { id: routine.into(), title: "Комментарии".into() };
        let mut run = Run::new("bot", origin, &TurnOptions::default());
        run.state = RunState::Paused { until_ms: now() + 3 * 24 * 3_600_000, reason, webhook: None, note: "claude-acp".into() };
        run
    }

    #[test]
    fn a_new_routine_term_replaces_its_stale_pauses_but_keeps_the_bots_own_wait() {
        let dir = std::env::temp_dir().join(format!("runs-stale-{}", crate::guard::random_token()));
        std::fs::create_dir_all(&dir).unwrap();
        let runs = Runs::open(&dir);
        for run in [paused("rt_a", StopReason::Limit), paused("rt_a", StopReason::Network), paused("rt_a", StopReason::Event), paused("rt_b", StopReason::Limit)] {
            runs.add(run);
        }
        assert_eq!(runs.drop_stale_pauses("rt_a"), 2);
        let left: Vec<_> = runs.paused().into_iter().map(|r| (origin_routine(&r.origin).unwrap().to_string(), r.state)).collect();
        assert_eq!(left.len(), 2, "{left:?}");
        assert!(left.iter().any(|(id, s)| id == "rt_a" && matches!(s, RunState::Paused { reason: StopReason::Event, .. })), "своё ожидание бота остаётся");
        assert!(left.iter().any(|(id, _)| id == "rt_b"), "чужая рутина не трогается");
        let _ = std::fs::remove_dir_all(dir);
    }

    fn temp_runs(name: &str) -> (Runs, std::path::PathBuf) {
        let dir = std::env::temp_dir().join(format!("runs-{name}-{}", crate::guard::random_token()));
        std::fs::create_dir_all(&dir).unwrap();
        (Runs::open(&dir), dir)
    }

    #[test]
    fn a_routine_with_a_future_limit_pause_or_a_waiting_card_does_not_start_again() {
        let (runs, dir) = temp_runs("ahead");
        runs.add(paused("rt_a", StopReason::Limit));
        assert!(runs.limit_pause_ahead("rt_a"));
        assert!(!runs.limit_pause_ahead("rt_b"), "чужая рутина");
        let mut over = paused("rt_c", StopReason::Limit);
        over.state = RunState::Paused { until_ms: now() - 1, reason: StopReason::Limit, webhook: None, note: String::new() };
        runs.add(over);
        assert!(!runs.limit_pause_ahead("rt_c"), "срок вышел: пауза проснётся сама, держать нечего");
        let mut waiting = paused("rt_w", StopReason::Limit);
        waiting.state = RunState::NeedsHuman { reason: StopReason::Human, text: "Ждёт решения человека по: действие.".into() };
        runs.add(waiting);
        assert!(runs.routine_waits_for_human("rt_w"));
        assert!(!runs.routine_waits_for_human("rt_a"));
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn a_new_pause_of_a_routine_replaces_its_older_one() {
        let (runs, dir) = temp_runs("one-pause");
        runs.add(paused("rt_a", StopReason::Limit));
        runs.add(paused("rt_a", StopReason::Event));
        let origin = Origin::Routine { id: "rt_a".into(), title: "Комментарии".into() };
        let id = runs.begin(Run::new("bot", origin, &TurnOptions::default()));
        runs.close("bot", RunState::Paused { until_ms: now() + 60_000, reason: StopReason::Limit, webhook: None, note: String::new() });
        let left: Vec<_> = runs.paused().into_iter().map(|r| (r.id, r.state)).collect();
        assert_eq!(left.len(), 2, "{left:?}");
        assert!(left.iter().any(|(rid, _)| *rid == id), "новая пауза осталась");
        assert!(left.iter().any(|(_, s)| matches!(s, RunState::Paused { reason: StopReason::Event, .. })), "ожидание бота не трогается");
        let _ = std::fs::remove_dir_all(dir);
    }
}
