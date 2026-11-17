//! Рутины: бот сам просыпается по расписанию. Раз в 30 секунд смотрим, что созрело.
//! Если бот занят, рутина ждёт следующего окна, а не копит очередь.

use std::sync::Arc;
use std::time::Duration;

use serde_json::{json, Value};

use crate::store::{now, uid, Routine};
use crate::turn::{start_turn, RoutineRef, TurnOptions};
use crate::App;

const TICK: Duration = Duration::from_secs(30);

const BROWSER_IDLE_MS: i64 = 10 * 60_000;

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

pub fn over_limit(app: &App) -> bool {
    let limit = app.store.settings().daily_token_limit;
    limit > 0 && app.ledger.tokens_today() >= limit
}

/// Запустить рутину. `unattended` значит, что человека рядом нет и решать за него нельзя.
pub async fn run_routine(app: &Arc<App>, routine: &Routine, unattended: bool) -> Result<(), crate::turn::StartError> {
    run_routine_as(app, routine, unattended, "routine").await
}

/// Тот же запуск, но с другой пометкой источника в журнале (например «device:iPhone», когда рутину запустили с устройства).
pub async fn run_routine_as(app: &Arc<App>, routine: &Routine, unattended: bool, source: &str) -> Result<(), crate::turn::StartError> {
    let mut text = format!("[рутина] {}\n{}", routine.title, routine.prompt);
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
    };
    let started = start_turn(app, &routine.bot_id, &text, opts);
    if let Err(e) = app.store.mark_routine_run(&routine.id, started.is_ok()).await {
        eprintln!("[routine] {}: {e}", routine.title);
    }
    started
}

async fn tick(app: &Arc<App>) {
    crate::update::tick(app);
    crate::memory::daily_tick(app);
    crate::optimizer::tick(app);
    crate::todos::tick(app);
    crate::habits::remind_tick(app).await;
    crate::goals::tick(app).await;
    crate::workflow_run::tick(app);
    app.browsers.close_idle(BROWSER_IDLE_MS, |bot| app.turns.is_busy(bot)).await;

    let routines = app.store.routines();
    let settings = app.store.settings();
    // Идёт работа или человек попросил дежурить: компьютер не должен уснуть от простоя.
    if !app.turns.busy_bots().is_empty() || (settings.keep_awake && routines.iter().any(|r| r.enabled)) {
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
        if !crate::schedule::should_run(&routine, now()) {
            note(app, &routine, "skipped", "Срок прошёл, пока компьютер спал или программа была закрыта.").await;
            let _ = app.store.reschedule_routine(&routine.id, "skipped").await;
            continue;
        }
        // Человек важнее: живая очередь идёт первой. Очередь на паузе рутину не держит: человек мог уйти
        // на всю ночь, и бот не должен простаивать из-за нажатого вечером «стоп».
        let queue_waits = app.queues.count(&routine.bot_id) > 0 && !app.queues.is_paused(&routine.bot_id);
        if app.turns.is_busy(&routine.bot_id) || queue_waits {
            app.store.delay_routine(&routine.id, 5 * 60_000);
            continue;
        }
        if over_limit(app) {
            note(app, &routine, "limit", "Дневной предел токенов исчерпан, запуск пропущен.").await;
            let _ = app.store.reschedule_routine(&routine.id, "limit").await;
            continue;
        }
        if let Err(e) = run_routine(app, &routine, true).await {
            note(app, &routine, "failed", &e.message).await;
        }
    }
}

pub fn start(app: Arc<App>) {
    tokio::spawn(async move {
        // Запуски процессов, прерванные перезапуском, продолжают проход.
        crate::workflow_run::resume(&app);
        let mut timer = tokio::time::interval(tick_period());
        timer.tick().await;
        loop {
            timer.tick().await;
            tick(&app).await;
        }
    });
}
