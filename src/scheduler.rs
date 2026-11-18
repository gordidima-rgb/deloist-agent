//! Рутины: бот сам просыпается по расписанию. Раз в 30 секунд смотрим, что созрело.
//! Если бот занят, рутина ждёт следующего окна, а не копит очередь.

use std::sync::Arc;
use std::sync::atomic::{AtomicI64, Ordering};
use std::time::Duration;

use serde_json::{json, Value};

use crate::store::{now, uid, Routine};
use crate::turn::{start_turn, RoutineRef, TurnOptions};
use crate::App;

const TICK: Duration = Duration::from_secs(30);
static PRUNE_LAST: AtomicI64 = AtomicI64::new(0);

const BROWSER_IDLE_MS: i64 = 10 * 60_000;
/// В облаке память на сервере общая: браузер бота засыпает через 5 минут бездействия (владелец, 14.11.2026).
const CLOUD_BROWSER_IDLE_MS: i64 = 5 * 60_000;

fn browser_idle_ms() -> i64 {
    if crate::cloud::active() {
        CLOUD_BROWSER_IDLE_MS
    } else {
        BROWSER_IDLE_MS
    }
}

fn tick_period() -> Duration {
    if cfg!(debug_assertions) {
        if let Some(ms) = std::env::var("CREW_TEST_TICK_MS").ok().and_then(|v| v.parse().ok()) {
            return Duration::from_millis(ms);
        }
    }
    TICK
}

/// Запись журнала о том, что запуск не состоялся, и сообщение интерфейсу.
async fn note(app: &Arc<App>, routine: &Routine, status: &str, text: &str) {
    let bot = app.store.bot(&routine.bot_id).map(|b| b.name).unwrap_or_default();
    let entry = json!({
        "id": uid("log"), "at": now(), "routineId": routine.id, "title": routine.title,
        "botId": routine.bot_id, "botName": bot, "status": status, "text": text, "tokens": 0, "missed": Value::Array(Vec::new()),
    });
    app.ledger.append_journal(&entry).await;
    app.turns.emit(json!({ "type": "journal", "entry": entry }));
}

/// Дневной предел токенов исчерпан. Спрашивают только те, кто идёт без человека (рутины, цели, процессы):
/// упёрся — это и есть остановка, её запоминаем для кольца в доке.
pub fn over_limit(app: &App) -> bool {
    let limit = app.store.settings().daily_token_limit;
    let over = limit > 0 && app.ledger.tokens_today() >= limit;
    if over {
        app.ledger.note_daily_stop();
    }
    over
}

/// Запуск по просьбе («Запустить» в окне, инструмент бота): те же проверки, что и перед ходом по расписанию.
pub async fn run_now(app: &Arc<App>, id: &str, unattended: bool) -> Result<Routine, crate::turn::StartError> {
    let refuse = |status: u16, message: &str| crate::turn::StartError { status, message: message.into() };
    let routine = app.store.routine(id).ok_or_else(|| refuse(404, "рутина не найдена"))?;
    if crate::reminders::is_reminder(&routine) {
        return Err(refuse(400, "напоминание шлёт таймер, бота у него нет: для проверки есть «проверить напоминание»"));
    }
    if app.store.bot(&routine.bot_id).is_none() {
        return Err(refuse(400, "бота этой рутины уже нет"));
    }
    run_routine(app, &routine, unattended).await?;
    Ok(app.store.routine(id).unwrap_or(routine))
}

/// Запустить рутину. `unattended` значит, что человека рядом нет и решать за него нельзя.
pub async fn run_routine(app: &Arc<App>, routine: &Routine, unattended: bool) -> Result<(), crate::turn::StartError> {
    run_routine_as(app, routine, unattended, "routine", 0).await
}

/// Тот же запуск, но с другой пометкой источника в журнале (например «device:iPhone», когда рутину запустили с устройства).
pub async fn run_routine_as(app: &Arc<App>, routine: &Routine, unattended: bool, source: &str, late_ms: i64) -> Result<(), crate::turn::StartError> {
    let late_prefix = if late_ms > 0 {
        let h = late_ms / 3_600_000;
        let m = (late_ms % 3_600_000) / 60_000;
        format!("Запуск опоздал на {h} ч {m} мин: компьютер был выключен или спал.\n")
    } else {
        String::new()
    };
    let mut text = format!("{late_prefix}[рутина] {}\n{}", routine.title, routine.prompt);
    let mut run = None;
    let mut goal_floor = String::new();
    if !routine.goal_id.is_empty() {
        match crate::goals::routine_run(app, routine) {
            Ok(plan) => {
                text = plan.text;
                run = Some(plan.run);
                goal_floor = plan.floor;
            }
            // Цель на паузе или завершена: срок пропускаем честно, а не молча «запускаем» пустоту.
            Err(why) => {
                // Одна запись о пропуске, а не по строке на каждый срок.
                if routine.last_status.as_deref() != Some("skipped") {
                    note(app, routine, "skipped", &why).await;
                }
                crate::goals::release_if_dead(app, routine).await;
                let _ = app.store.reschedule_routine(&routine.id, "skipped").await;
                return Ok(());
            }
        }
    }
    if routine.goal_id.is_empty() && !routine.progress_note.is_empty() {
        text.push_str("\n\nЗаметка с прошлого запуска:\n");
        text.push_str(&routine.progress_note);
    }
    let opts = TurnOptions {
        source: Some(source.to_string()),
        depth: 0,
        unattended,
        routine: Some(RoutineRef { id: routine.id.clone(), title: routine.title.clone() }),
        // Кнопку «Запустить» нажал человек и ждёт ответа: скрывать от него нечего.
        quiet: routine.quiet && unattended,
        // Рутину, которую завёл осторожный бот для чужого, смелый получатель не запускает смелее него.
        approval_floor: crate::goals::combine_floor(&routine.approval_floor, &goal_floor),
        window: false,
        voice: false,
        run,
        origin: None,
        resume: None,
    };
    // «Идёт» ставим до старта: быстрый ход не должен закончиться раньше, чем его отметили запущенным.
    if let Err(e) = app.store.mark_routine_run(&routine.id, true).await {
        eprintln!("[routine] {}: {e}", routine.title);
    }
    let started = start_turn(app, &routine.bot_id, &text, opts);
    if started.is_err() {
        if let Err(e) = app.store.set_routine_status(&routine.id, "error").await {
            eprintln!("[routine] {}: {e}", routine.title);
        }
    }
    started
}

pub fn clock(ms: i64) -> String {
    use chrono::TimeZone;
    chrono::Local.timestamp_millis_opt(ms).single().map(|d| d.format("%H:%M").to_string()).unwrap_or_default()
}

/// Ход рутины кончился: статус по итогу, ошибка растит серию. Реже — после 3 и 6 ошибок подряд, а три «вечные» ошибки
/// ставят рутину на паузу: человеку одна строка в чате на всю серию, дальше «Возобновить».
pub async fn routine_finished(app: &Arc<App>, routine: &RoutineRef, bot_id: &str, status: &str, reason: Option<crate::runs::StopReason>) {
    use crate::store::RoutineOutcome;
    let outcome = match app.store.finish_routine_run(&routine.id, status, reason).await {
        Ok(outcome) => outcome,
        Err(e) => {
            eprintln!("[routine] {}: {e}", routine.title);
            return;
        }
    };
    let english = app.store.settings().ui_lang == "en";
    let title = &routine.title;
    let Some(r) = app.store.routine(&routine.id) else { return };
    let why = r.stop_reason.map(|reason| reason.label(english)).unwrap_or_default();
    match outcome {
        RoutineOutcome::Done => {}
        RoutineOutcome::Failed { failures: n @ (3 | 6) } => {
            let gap = if n == 3 { if english { "once an hour" } else { "раз в час" } } else if english { "once in 4 hours" } else { "раз в 4 часа" };
            let text = if english { format!("{n} failures in a row ({why}): runs no more often than {gap}.") } else { format!("Ошибок подряд: {n} ({why}) — запускаю не чаще, чем {gap}.") };
            note(app, &r, "paused", &text).await;
        }
        RoutineOutcome::Failed { .. } => {}
        RoutineOutcome::PausedByErrors { failures } => {
            let text = if english {
                format!("Routine “{title}” is paused: {failures} failures in a row — {why}. Press “Resume” in the routines list when it is fixed.")
            } else {
                format!("Рутина «{title}» на паузе: {failures} ошибки подряд — {why}. Когда исправишь, нажми «Возобновить» в списке рутин.")
            };
            note(app, &r, "paused", &text).await;
            let entry = json!({ "id": uid("msg"), "role": "assistant", "text": text, "at": now(), "error": true, "routineId": routine.id });
            if let Err(e) = app.store.append_message(bot_id, entry.clone()).await {
                eprintln!("[routine] {title}: {e}");
            }
            app.turns.emit(json!({ "type": "message", "botId": bot_id, "message": entry }));
        }
    }
}

/// Рутина числится «идёт», а живого хода у неё нет: запись журнала о нём потеряна (старая версия, стёртый файл).
/// Ход продолжает или закрывает `runs::recover`, здесь остаётся только не оставить рутину «идущей» навсегда.
pub async fn settle_orphans(app: &Arc<App>) {
    for routine in app.store.routines() {
        if routine.last_status.as_deref() != Some("running") || crate::reminders::is_reminder(&routine) || app.turns.is_busy(&routine.bot_id) {
            continue;
        }
        note(app, &routine, "failed", "Программа закрылась посреди хода.").await;
        let _ = app.store.set_routine_status(&routine.id, "error").await;
    }
}

async fn tick(app: &Arc<App>) {
    let now_ms = crate::store::now();
    let last = PRUNE_LAST.load(Ordering::Relaxed);
    if now_ms - last >= 24 * 3600 * 1000 {
        crate::attach::prune_copies(&app.store, now_ms);
        PRUNE_LAST.store(now_ms, Ordering::Relaxed);
    }
    crate::update::tick(app);
    crate::memory::daily_tick(app);
    crate::memory::team_review::ensure_routines(app).await;
    crate::optimizer::tick(app);
    crate::todos::tick(app);
    crate::habits::remind_tick(app).await;
    crate::goals::tick(app).await;
    crate::runs::resume_due(app).await;
    crate::runs::notify_pauses(app).await;
    crate::workflow_run::tick(app);
    app.browsers.close_idle(browser_idle_ms(), |bot| app.turns.is_busy(bot)).await;

    let routines = app.store.routines();
    let settings = app.store.settings();
    // Идёт работа или человек попросил дежурить: компьютер не должен уснуть от простоя.
    let soon = routines.iter().any(|r| {
        r.enabled && !r.hook && !crate::reminders::is_reminder(r) && r.next_run <= now() + 60 * 60 * 1000
    });
    if !app.turns.busy_bots().is_empty()
        || (settings.keep_awake && routines.iter().any(|r| r.enabled))
        || soon
    {
        crate::system::hold_off_sleep();
    }

    for routine in routines {
        // Вебхук запускает внешний вызов, а не тикер.
        if routine.hook {
            continue;
        }
        if !routine.enabled || now() < routine.next_run {
            continue;
        }
        // Напоминание стреляет таймером: без бота, без хода и без модели.
        if crate::reminders::is_reminder(&routine) {
            crate::reminders::fire(app, &routine).await;
            continue;
        }
        if app.store.bot(&routine.bot_id).is_none() {
            let _ = app.store.update_routine(&routine.id, &json!({ "enabled": false })).await;
            continue;
        }
        // Три «вечные» ошибки подряд: ждём «Возобновить», срок только отмечаем.
        if routine.error_paused {
            if routine.last_status.as_deref() != Some("skipped") {
                note(app, &routine, "paused", "Запуск пропущен: рутина на паузе из-за ошибок подряд.").await;
            }
            let _ = app.store.reschedule_routine(&routine.id, "skipped").await;
            continue;
        }
        // Пропущенные запуски — по правилу рутины.
        let mut late_ms = 0;
        {
            let missed = crate::schedule::missed(&routine, now());
            let catch_up = routine.catch_up == "last" && missed.last_slot_ms.is_some_and(|slot| now() - slot <= crate::schedule::CATCH_UP_MS);
            let dropped = missed.count - u32::from(catch_up);
            if dropped > 0 {
                note(app, &routine, "skipped", &format!("Пропущено запусков: {dropped}. Компьютер был выключен или спал.")).await;
            }
            if catch_up {
                late_ms = missed.last_slot_ms.map_or(0, |slot| now() - slot);
            } else if missed.count > 0 || !crate::schedule::open_now(&routine, now()) {
                if missed.count == 0 {
                    note(app, &routine, "skipped", "Срок прошёл, пока компьютер спал или программа была закрыта.").await;
                }
                let _ = app.store.reschedule_routine(&routine.id, "skipped").await;
                continue;
            }
        }
        // Человек важнее: живая очередь идёт первой. Очередь на паузе рутину не держит: человек мог уйти
        // на всю ночь, и бот не должен простаивать из-за нажатого вечером «стоп».
        let queue_waits = app.queues.count(&routine.bot_id) > 0 && !app.queues.is_paused(&routine.bot_id);
        if app.turns.is_busy(&routine.bot_id) || queue_waits {
            // Ждём бота по 5 минут, но не дольше часа: дальше запуск пропущен, а в журнале сказано почему.
            if !app.store.wait_for_bot(&routine.id) {
                note(app, &routine, "busy", "Бот был занят больше часа: запуск пропущен, жду следующего срока.").await;
                let _ = app.store.reschedule_routine(&routine.id, "busy").await;
            }
            continue;
        }
        if over_limit(app) {
            pause_for_budget(app, &routine).await;
            continue;
        }
        if let Err(e) = run_routine_as(app, &routine, true, "routine", late_ms).await {
            note(app, &routine, "failed", &e.message).await;
        }
    }
}

/// Дневной предел токенов исчерпан: рутина не пропадает, а ждёт полуночи записью в журнале запусков.
/// Рутина цели ведётся самой целью: её срок просто пропускается.
async fn pause_for_budget(app: &Arc<App>, routine: &Routine) {
    if !routine.goal_id.is_empty() {
        note(app, routine, "limit", "Дневной предел токенов исчерпан, запуск пропущен.").await;
        let _ = app.store.reschedule_routine(&routine.id, "limit").await;
        return;
    }
    let until = crate::limit::next_day_ms(chrono::Local::now()).unwrap_or_else(|| now() + crate::limit::backoff_ms(0));
    let text = format!("[рутина] {}\n{}", routine.title, routine.prompt);
    let opts = TurnOptions {
        source: Some("routine".into()),
        unattended: true,
        quiet: routine.quiet,
        approval_floor: crate::goals::combine_floor(&routine.approval_floor, ""),
        ..TurnOptions::default()
    };
    let origin = crate::runs::Origin::Routine { id: routine.id.clone(), title: routine.title.clone() };
    crate::runs::pause_unstarted(app, &routine.bot_id, origin, &opts, &text, until, crate::runs::StopReason::Budget, "дневной предел");
    note(app, routine, "paused", &format!("Дневной предел токенов исчерпан: запуск на паузе до {}.", clock(until))).await;
    let _ = app.store.reschedule_routine(&routine.id, "paused").await;
}

pub fn start(app: Arc<App>) {
    tokio::spawn(async move {
        // Запуски процессов, прерванные перезапуском, продолжают проход.
        crate::workflow_run::resume(&app);
        settle_orphans(&app).await;
        crate::attach::prune_copies(&app.store, crate::store::now());
        PRUNE_LAST.store(crate::store::now(), std::sync::atomic::Ordering::Relaxed);
        let mut timer = tokio::time::interval(tick_period());
        timer.tick().await;
        loop {
            timer.tick().await;
            tick(&app).await;
        }
    });
}
