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

fn clock(ms: i64) -> String {
    use chrono::TimeZone;
    chrono::Local.timestamp_millis_opt(ms).single().map(|d| d.format("%H:%M").to_string()).unwrap_or_default()
}

/// Ход рутины кончился (или оборвался перезапуском): статус по итогу, повтор по лестнице,
/// через сутки неудач — ошибка. Всё, что не «выполнено», человек видит строкой в чате бота.
pub async fn routine_finished(app: &Arc<App>, routine: &RoutineRef, bot_id: &str, status: &str, started_at: i64) {
    use crate::store::RoutineOutcome;
    let outcome = match app.store.finish_routine_run(&routine.id, status, started_at).await {
        Ok(outcome) => outcome,
        Err(e) => {
            eprintln!("[routine] {}: {e}", routine.title);
            return;
        }
    };
    let english = app.store.settings().ui_lang == "en";
    let title = &routine.title;
    let interrupted = status == "interrupted";
    let text = match &outcome {
        RoutineOutcome::Done => return,
        RoutineOutcome::Retry { at, attempt } => {
            let at = clock(*at);
            let n = attempt + 1;
            match (interrupted, english) {
                (true, false) => format!("Рутина «{title}» прервалась: программа закрылась посреди хода. Повторю в {at} (попытка {n})."),
                (true, true) => format!("Routine “{title}” was cut off: the app closed mid-turn. Retrying at {at} (attempt {n})."),
                (false, false) => format!("Рутина «{title}» не выполнена. Повторю в {at} (попытка {n})."),
                (false, true) => format!("Routine “{title}” did not finish. Retrying at {at} (attempt {n})."),
            }
        }
        RoutineOutcome::GaveUp { attempts } => {
            let text = if english {
                format!("Routine “{title}” has not finished for a day, attempts: {attempts}. Marked as an error; the next run follows the schedule.")
            } else {
                format!("Рутина «{title}» не выполнилась за сутки, попыток: {attempts}. Ставлю ошибку, дальше — по расписанию.")
            };
            if let Some(r) = app.store.routine(&routine.id) {
                note(app, &r, "gave_up", &text).await;
            }
            text
        }
    };
    let entry = json!({
        "id": uid("msg"), "role": "assistant", "text": text, "at": now(),
        "error": matches!(outcome, RoutineOutcome::GaveUp { .. }), "routineId": routine.id,
    });
    if let Err(e) = app.store.append_message(bot_id, entry.clone()).await {
        eprintln!("[routine] {title}: {e}");
    }
    app.turns.emit(json!({ "type": "message", "botId": bot_id, "message": entry }));
}

/// Программа закрылась, пока рутина шла: её ход пропал целиком. При запуске отмечаем обрыв и ставим повтор.
pub async fn recover_interrupted(app: &Arc<App>) {
    for routine in app.store.routines() {
        if routine.last_status.as_deref() != Some("running") || crate::reminders::is_reminder(&routine) {
            continue;
        }
        note(app, &routine, "interrupted", "Программа закрылась посреди хода.").await;
        let rref = RoutineRef { id: routine.id.clone(), title: routine.title.clone() };
        routine_finished(app, &rref, &routine.bot_id, "interrupted", routine.last_run).await;
    }
}

async fn tick(app: &Arc<App>) {
    crate::update::tick(app);
    crate::memory::daily_tick(app);
    crate::memory::team_review::ensure_routines(app).await;
    crate::optimizer::tick(app);
    crate::todos::tick(app);
    crate::habits::remind_tick(app).await;
    crate::goals::tick(app).await;
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
        // Пропущенные запуски — по правилу рутины. Повтор после неудачи идёт по своему сроку, даже вне окна.
        let mut late_ms = 0;
        if routine.retry_attempt == 0 {
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
            app.store.delay_routine(&routine.id, 5 * 60_000);
            continue;
        }
        if over_limit(app) {
            note(app, &routine, "limit", "Дневной предел токенов исчерпан, запуск пропущен.").await;
            let _ = app.store.reschedule_routine(&routine.id, "limit").await;
            continue;
        }
        if let Err(e) = run_routine_as(app, &routine, true, "routine", late_ms).await {
            note(app, &routine, "failed", &e.message).await;
        }
    }
}

pub fn start(app: Arc<App>) {
    tokio::spawn(async move {
        // Запуски процессов, прерванные перезапуском, продолжают проход.
        crate::workflow_run::resume(&app);
        recover_interrupted(&app).await;
        let mut timer = tokio::time::interval(tick_period());
        timer.tick().await;
        loop {
            timer.tick().await;
            tick(&app).await;
        }
    });
}
