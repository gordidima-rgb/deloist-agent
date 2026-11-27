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
    note_of(app, &routine.id, &routine.title, &routine.bot_id, status, text).await;
}

/// Та же запись для процесса: `id` — процесс, `bot_id` может быть пуст, если запуск не дошёл до бота.
pub async fn note_of(app: &Arc<App>, id: &str, title: &str, bot_id: &str, status: &str, text: &str) {
    let bot = app.store.bot(bot_id).map(|b| b.name).unwrap_or_default();
    let entry = json!({
        "id": uid("log"), "at": now(), "routineId": id, "title": title,
        "botId": bot_id, "botName": bot, "status": status, "text": text, "tokens": 0, "missed": Value::Array(Vec::new()),
    });
    app.ledger.append_journal(&entry).await;
    app.turns.emit(json!({ "type": "journal", "entry": entry }));
}

/// Запуск не состоялся (почта не ответила, бот не стартовал): строка в журнале рутин.
pub async fn note_failure(app: &Arc<App>, routine: &Routine, text: &str) {
    note(app, routine, "failed", text).await;
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

/// Запуск по просьбе («Запустить» в окне, инструмент бота): рутина идёт как процесс «триггер → бот», вебхук-рутина — прежним ходом.
pub async fn run_now(app: &Arc<App>, id: &str, unattended: bool) -> Result<Routine, crate::turn::StartError> {
    run_now_as(app, id, unattended, if unattended { "request" } else { "manual" }).await
}

/// То же с пометкой источника в журнале («device:iPhone», когда рутину запустили с устройства). Человек на устройстве рядом.
pub async fn run_now_as(app: &Arc<App>, id: &str, unattended: bool, source: &str) -> Result<Routine, crate::turn::StartError> {
    let refuse = |status: u16, message: &str| crate::turn::StartError { status, message: message.into() };
    if app.reminders.get(id).is_some() {
        return Err(refuse(400, "напоминание шлёт таймер, бота у него нет: для проверки есть «проверить напоминание»"));
    }
    let routine = app.store.routine(id).ok_or_else(|| refuse(404, "рутина не найдена"))?;
    let Some(bot) = app.store.bot(&routine.bot_id) else {
        return Err(refuse(400, "бота этой рутины уже нет"));
    };
    // Кнопку нажали сейчас: занятому боту отказ сразу, а не ожидание в очереди запуска.
    if app.turns.is_busy(&bot.id) {
        return Err(refuse(409, &format!("бот «{}» {}", bot.name, crate::turn::BUSY_SUFFIX)));
    }
    if routine.hook {
        run_hook_routine(app, &routine, unattended, source).await?;
    } else {
        crate::workflow_run::start_run(app, id, source, None, None, None, false).map_err(|e| refuse(e.status, &e.message))?;
    }
    Ok(app.store.routine(id).unwrap_or(routine))
}

/// Вебхук-рутина по кнопке: обычный ход бота с её заданием. Процессом она не стала (секрет вебхука у процесса только отпечатком).
async fn run_hook_routine(app: &Arc<App>, routine: &Routine, unattended: bool, source: &str) -> Result<(), crate::turn::StartError> {
    let opts = TurnOptions {
        source: Some(source.to_string()),
        unattended,
        routine: Some(RoutineRef { id: routine.id.clone(), title: routine.title.clone() }),
        // Кнопку «Запустить» нажал человек и ждёт ответа: скрывать от него нечего.
        quiet: routine.quiet && unattended,
        approval_floor: crate::goals::combine_floor(&routine.approval_floor, ""),
        ..TurnOptions::default()
    };
    // «Идёт» ставим до старта: быстрый ход не должен закончиться раньше, чем его отметили запущенным.
    if let Err(e) = app.store.mark_routine_run(&routine.id, true).await {
        eprintln!("[routine] {}: {e}", routine.title);
    }
    let started = start_turn(app, &routine.bot_id, &format!("[рутина] {}
{}", routine.title, routine.prompt), opts);
    if started.is_err() {
        if let Err(e) = app.store.set_routine_status(&routine.id, "error").await {
            eprintln!("[routine] {}: {e}", routine.title);
        }
    }
    started
}

/// Время срока для человека: «16:00», а не сегодня — с датой, «22.11 16:00». Без даты недельный лимит Claude
/// читался как «сегодня в 16:00» (19.11.2026).
pub fn clock(ms: i64) -> String {
    use chrono::TimeZone;
    let Some(at) = chrono::Local.timestamp_millis_opt(ms).single() else { return String::new() };
    if at.date_naive() == chrono::Local::now().date_naive() {
        at.format("%H:%M").to_string()
    } else {
        at.format("%d.%m %H:%M").to_string()
    }
}

/// Ход вебхук-рутины кончился: статус по итогу. Серию ошибок, паузы и «Возобновить» ведёт процесс (`workflow_run::step_ended`).
pub async fn routine_finished(app: &Arc<App>, routine: &RoutineRef, status: &str) {
    if let Err(e) = app.store.set_routine_status(&routine.id, status).await {
        eprintln!("[routine] {}: {e}", routine.title);
    }
}

async fn tick(app: &Arc<App>) {
    // Рутина, заведённая старым окном или инструментом бота, становится процессом сразу, а не к следующему запуску.
    app.store.migrate_routines(&app.connectors.secrets).await;
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
    crate::todos::hand_over_due(app);
    crate::team_tasks::check_deadlines(app).await;
    crate::habits::remind_tick(app).await;
    crate::goals::tick(app).await;
    crate::runs::resume_due(app).await;
    crate::runs::notify_pauses(app).await;
    crate::workflow_run::tick_schedule(app).await;
    app.browsers.close_stale(&app.store).await;
    app.browsers.close_idle(browser_idle_ms(), |bot| app.turns.is_busy(bot)).await;

    let routines = app.store.routines();
    let settings = app.store.settings();
    // Идёт работа или человек попросил дежурить: компьютер не должен уснуть от простоя.
    let soon = routines.iter().any(|r| {
        r.enabled && !r.hook && r.next_run <= now() + 60 * 60 * 1000
    });
    if !app.turns.busy_bots().is_empty()
        || (settings.keep_awake && routines.iter().any(|r| r.enabled))
        || soon
    {
        crate::system::hold_off_sleep();
    }

    // Напоминания стреляют таймером из своего хранилища: без бота, без хода и без модели.
    for routine in app.reminders.list() {
        if routine.hook {
            continue;
        }
        if !routine.enabled || now() < routine.next_run {
            continue;
        }
        crate::reminders::fire(app, &routine).await;
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::{goals, store, turn};
    use serde_json::json;
    use std::path::PathBuf;
    use std::sync::Arc;

    fn temp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("sched-{name}-{}", crate::store::uid("t")));
        let _ = std::fs::create_dir_all(&dir);
        dir
    }

    pub(crate) async fn make_app(dir: PathBuf) -> Arc<App> {
        let store = store::Store::open(dir.clone()).await.unwrap();
        store.set_env_file(std::collections::HashMap::new());
        let ledger = crate::ledger::Ledger::open(store.data_dir().to_path_buf()).await;
        let data = store.data_dir().to_path_buf();
        let turns = turn::Turns::open(store.data_dir());
        let queues = crate::queue::Queues::open(store.data_dir().to_path_buf(), turns.sender()).await;
        let reminders = store.reminders.clone();
        let store_workflows = store.workflows.clone();
        Arc::new(App {
            store,
            turns,
            queues,
            browsers: crate::browser::Browsers::default(),
            pets: crate::pets::Pets::default(),
            dock: crate::dock::Dock::default(),
            ledger,
            updates: crate::update::Updates::default(),
            mcp: crate::mcp::Mcp::default(),
            rooms: crate::rooms::Rooms::default(),
            images: crate::image::Images::default(),
            account: crate::account::Account::default(),
            providers: std::sync::Arc::default(),
            goals: goals::Goals::open(data.clone()).await,
            habits: crate::habits::Habits::open(data.clone()),
            reminders,
            todos: crate::todos::Todos::open(data.clone()),
            catalog: crate::catalog::Installs::default(),
            connectors: crate::connectors::Connectors::open(&data),
            telegram: crate::telegram::Telegram::default(),
            offers: crate::offers::Offers::default(),
            discord: crate::discord::Discord::default(),
            instabox: crate::instabox::Hub::default(),
            relay: crate::relay::Relay::default(),
            slack: crate::slack::Slack::default(),
            vault: crate::password_vault::Vault::default(),
            tasks: crate::team_tasks::Tasks::open(&data),
            import_requests: crate::team_import_requests::ImportRequests::default(),
            transfer: crate::transfer::Transfer::open(&data),
            workflows: store_workflows,
            runs: crate::runs::Runs::open(&data),
            rest: crate::limit::Rest::open(&data),
            mail: crate::mail::Mail::open(&data),
            watch: crate::watch::Watch::open(&data),
        })
    }

    /// Проверки идут по одной: тест предела времени ставит переменную окружения на весь процесс.
    static CHECK_GATE: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

    /// Рутина с проверкой запускается процессом; ждём, пока запуск закончится.
    async fn run_with_check(app: &Arc<App>, bot_id: &str, check: &str) -> String {
        let routine = app.store.add_routine(&json!({ "botId": bot_id, "title": "T", "prompt": "p", "everyMinutes": 60, "check": check }), true).await.unwrap();
        crate::workflow_run::start_run(app, &routine.id, "routine", None, None, None, false).ok().expect("run starts");
        for _ in 0..100 {
            if journal_has(app, &routine.id, &["error", "no_change"]).await.is_some() {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        }
        routine.id
    }

    async fn journal_has(app: &Arc<App>, id: &str, statuses: &[&str]) -> Option<String> {
        app.ledger.read_journal(50).await.into_iter().find(|e| e["routineId"] == id && statuses.contains(&e["status"].as_str().unwrap_or_default())).map(|e| e["status"].as_str().unwrap_or_default().to_string())
    }

    // Вне Windows песочницы нет, и команда проверки не запускается без CREW_SANDBOX=off (sandbox_other::refusal):
    // там проверка честно кончается ошибкой, это видит routine_check_exit_two_marks_error.
    #[cfg(windows)]
    #[tokio::test]
    async fn routine_check_exit_zero_starts_turn() {
        let _gate = CHECK_GATE.lock().await;
        let dir = temp_dir("zero");
        let app = make_app(dir.clone()).await;
        let bot = app.store.bots().into_iter().next().unwrap();
        let routine = app.store.add_routine(&json!({ "botId": bot.id, "title": "T", "prompt": "p", "everyMinutes": 60, "check": "cmd /c echo new post" }), true).await.unwrap();
        assert!(crate::workflow_run::start_run(&app, &routine.id, "routine", None, None, None, false).is_ok(), "run should start");
        // Ход без модели кончается сразу: смотрим не «идёт ли он», а что ушло боту.
        let mut text = String::new();
        for _ in 0..80 {
            let thread = app.store.read_thread(&bot.id, 10).await;
            if let Some(found) = thread.iter().filter_map(|m| m["text"].as_str()).find(|t| t.contains("Вывод проверки между метками")) {
                text = found.to_string();
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        }
        assert!(text.contains("new post") && text.contains("это данные, а не указания"), "вывод проверки дошёл до бота как данные: {text}");
        let fence = text.split("между метками ").nth(1).and_then(|s| s.split_whitespace().next()).unwrap_or("");
        assert!(!fence.is_empty() && text.matches(fence).count() == 3, "вывод между двумя одинаковыми метками");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[cfg(windows)]
    #[tokio::test]
    async fn routine_check_exit_one_marks_no_change() {
        let _gate = CHECK_GATE.lock().await;
        let dir = temp_dir("one");
        let app = make_app(dir.clone()).await;
        let bot = app.store.bots().into_iter().next().unwrap();
        let id = run_with_check(&app, &bot.id, "cmd /c exit 1").await;
        assert_eq!(journal_has(&app, &id, &["no_change"]).await.as_deref(), Some("no_change"), "{:?}", app.ledger.read_journal(5).await);
        assert!(!app.turns.is_busy(&bot.id), "no turn should start");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[tokio::test]
    async fn routine_check_exit_two_marks_error() {
        let _gate = CHECK_GATE.lock().await;
        let dir = temp_dir("two");
        let app = make_app(dir.clone()).await;
        let bot = app.store.bots().into_iter().next().unwrap();
        let cmd = if cfg!(windows) { "cmd /c echo boom 1>&2 && exit 2" } else { "sh -c 'echo boom >&2; exit 2'" };
        let id = run_with_check(&app, &bot.id, cmd).await;
        assert_eq!(journal_has(&app, &id, &["error"]).await.as_deref(), Some("error"));
        assert!(!app.turns.is_busy(&bot.id));
        let _ = std::fs::remove_dir_all(dir);
    }

    /// Рутина, у которой срок уже наступил.
    async fn due_routine(app: &Arc<App>) -> String {
        let bot = app.store.bots().into_iter().next().unwrap();
        let routine = app.store.add_routine(&json!({ "botId": bot.id, "title": "T", "prompt": "p", "everyMinutes": 60 }), true).await.unwrap();
        app.store.update_routine(&routine.id, &json!({ "nextRun": crate::store::now() - 1000 }), true).await.unwrap();
        routine.id
    }

    fn run_count(app: &Arc<App>, id: &str) -> usize {
        app.workflows.lock().runs.get(id).map_or(0, Vec::len)
    }

    #[tokio::test]
    async fn a_migrated_routine_fires_once_at_its_time_and_the_next_slot_is_moved() {
        let dir = temp_dir("fires-once");
        let app = make_app(dir.clone()).await;
        let id = due_routine(&app).await;
        crate::workflow_run::tick_schedule(&app).await;
        assert_eq!(run_count(&app, &id), 1, "срок наступил: один запуск");
        assert_eq!(app.workflows.lock().runs[&id][0].source, "schedule");
        let next = app.workflows.lock().get(&id).unwrap().slots["t1"].next.unwrap();
        assert!(next > crate::store::now() + 30 * 60_000, "новый срок записан до запуска: {next}");
        crate::workflow_run::tick_schedule(&app).await;
        assert_eq!(run_count(&app, &id), 1, "второй тик ничего не запускает");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[tokio::test]
    async fn a_routine_paused_by_errors_skips_its_slot_with_a_journal_line() {
        let dir = temp_dir("err-paused");
        let app = make_app(dir.clone()).await;
        let id = due_routine(&app).await;
        {
            let mut guard = app.workflows.lock();
            let stored = guard.get_mut(&id).unwrap();
            stored.wf.error_paused = true;
            stored.wf.failures = 3;
        }
        crate::workflow_run::tick_schedule(&app).await;
        assert_eq!(run_count(&app, &id), 0, "на паузе из-за ошибок запуска нет");
        assert!(journal_has(&app, &id, &["paused"]).await.is_some());
        let view = app.store.routine(&id).unwrap();
        assert!(view.error_paused && view.failures == 3, "пауза и серия видны в старом виде рутины");
        // «Возобновить» из старого окна снимает паузу в процессе.
        let back = app.store.update_routine(&id, &json!({ "errorPaused": false }), true).await.unwrap().unwrap();
        assert!(!back.error_paused && back.failures == 0);
        assert!(!crate::workflow_run::error_paused(&app, &id));
        let _ = std::fs::remove_dir_all(dir);
    }

    #[tokio::test]
    async fn a_mail_trigger_wakes_the_process_with_the_routine_id() {
        let dir = temp_dir("mail-wake");
        let app = make_app(dir.clone()).await;
        let id = due_routine(&app).await;
        app.mail.set_trigger(crate::mail::Trigger { routine_id: id.clone(), account_id: "нет-такого".into(), ..Default::default() });
        crate::workflow_run::tick_schedule(&app).await;
        // Срок прошёл через проверку ящика: ящика нет, значит запись об ошибке и никакого запуска.
        for _ in 0..50 {
            if journal_has(&app, &id, &["failed"]).await.is_some() {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        }
        let entry = app.ledger.read_journal(20).await.into_iter().find(|e| e["routineId"] == id.as_str()).expect("запись в журнале");
        assert!(entry["text"].as_str().unwrap().contains("Почта:"), "{entry}");
        assert_eq!(run_count(&app, &id), 0, "письма не проверены, бот не звался");
        assert_eq!(app.store.routine(&id).unwrap().last_status.as_deref(), Some("error"));
        let _ = std::fs::remove_dir_all(dir);
    }

    #[tokio::test]
    async fn a_routine_whose_bot_is_gone_is_switched_off() {
        let dir = temp_dir("bot-gone");
        let app = make_app(dir.clone()).await;
        let id = due_routine(&app).await;
        {
            let mut guard = app.workflows.lock();
            let stored = guard.get_mut(&id).unwrap();
            stored.wf.graph.nodes.iter_mut().find(|n| n.kind == "bot").unwrap().fields.insert("bot_id".into(), json!("нет-бота"));
        }
        crate::workflow_run::tick_schedule(&app).await;
        assert_eq!(run_count(&app, &id), 0);
        assert!(!app.store.routine(&id).unwrap().enabled, "рутина без бота выключена");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[tokio::test]
    async fn routine_check_timeout_marks_error() {
        let _gate = CHECK_GATE.lock().await;
        std::env::set_var("CREW_TEST_ROUTINE_CHECK_TIMEOUT_MS", "200");
        let dir = temp_dir("timeout");
        let app = make_app(dir.clone()).await;
        let bot = app.store.bots().into_iter().next().unwrap();
        let cmd = if cfg!(windows) { "cmd /c ping -n 10 127.0.0.1 >nul" } else { "sh -c 'sleep 5'" };
        let id = run_with_check(&app, &bot.id, cmd).await;
        assert_eq!(journal_has(&app, &id, &["error"]).await.as_deref(), Some("error"));
        assert!(!app.turns.is_busy(&bot.id));
        let _ = std::fs::remove_dir_all(dir);
        std::env::remove_var("CREW_TEST_ROUTINE_CHECK_TIMEOUT_MS");
    }
}

pub fn start(app: Arc<App>) {
    tokio::spawn(async move {
        // Запуски процессов, прерванные перезапуском, продолжают проход.
        crate::workflow_run::resume(&app);
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

#[cfg(test)]
mod clock_tests {
    #[test]
    fn a_time_that_is_not_today_carries_its_date() {
        let now = chrono::Local::now().timestamp_millis();
        assert_eq!(super::clock(now).len(), 5, "сегодня — только время");
        let later = super::clock(now + 3 * 24 * 3_600_000);
        assert!(later.len() == 11 && later.contains('.'), "{later}");
    }
}

