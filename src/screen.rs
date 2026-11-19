//! Экран бота в программе: живая картинка его вкладки, управление человеком, живая оболочка
//! `run_command` и запись «Научи бота». Картинка идёт отдельным SSE-потоком только пока панель
//! открыта: общий поток событий слушают док и питомцы, кадры им ни к чему.

use std::collections::{HashMap, HashSet};
use std::convert::Infallible;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path as FsPath, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, LazyLock, Mutex};
use std::time::{Duration, Instant};

use axum::body::{Body, Bytes};
use axum::extract::{Path, State};
use axum::http::{header, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use futures_util::StreamExt;
use serde_json::{json, Value};
use tokio::sync::{broadcast, mpsc, watch};
use tokio_util::sync::CancellationToken;

use crate::browser::Session;
use crate::teach::{self, Recording};
use crate::App;

/// Подтверждаем кадр не чаще: браузер не шлёт следующий, пока не подтвердили прошлый.
const ACK_EVERY: Duration = Duration::from_millis(150);
/// Столько ждём первый кадр трансляции, потом переходим на снимки.
const FIRST_FRAME_WAIT: Duration = Duration::from_secs(2);
const POLL_EVERY: Duration = Duration::from_millis(700);
/// Больше этого вывод одной команды в панель не идёт: результат модели всё равно обрезан.
const SHELL_LIMIT: usize = 200 * 1024;
/// Столько `browser_human` ждёт, что панель откроется, прежде чем вывести окно Chrome.
const PANEL_WAIT: Duration = Duration::from_millis(1500);
/// Ход без человека (рутина, ночная работа) ждёт удержания не дольше этого, потом управление возвращается боту.
const UNATTENDED_HOLD: Duration = Duration::from_secs(5 * 60);
/// Бот позвал человека (`browser_human`), а тот закрыл экран: столько ждём, что он вернётся
/// (окно ушло в трей, человек смотрит код в телефоне), прежде чем закончить ожидание.
const LEFT_GRACE: Duration = Duration::from_secs(2 * 60);

/// Срок из настройки теста: только в отладочной сборке, чтобы тесты не ждали минутами.
fn test_duration(name: &str, default: Duration) -> Duration {
    if cfg!(debug_assertions) {
        if let Some(ms) = std::env::var(name).ok().and_then(|v| v.parse().ok()) {
            return Duration::from_millis(ms);
        }
    }
    default
}
/// Столько живёт трансляция без единого кадра, прежде чем подстраховаться снимком.
const STALE_FRAME: Duration = Duration::from_secs(5);
/// Хвост вывода команды, который видит панель, открытая посреди команды.
const SHELL_TAIL: usize = 32 * 1024;
/// Вызовы браузера внутри цикла трансляции: зависшая вкладка не должна останавливать картинку и список вкладок.
const LOOP_CALL: Duration = Duration::from_secs(3);

// ---------------------------------------------------------------------------
// Координаты
// ---------------------------------------------------------------------------

/// Метрики кадра трансляции: размер окна в CSS-пикселях и сдвиг сверху.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct FrameMeta {
    pub device_width: f64,
    pub device_height: f64,
    pub offset_top: f64,
}

impl FrameMeta {
    fn from_screencast(metadata: &Value) -> Self {
        let get = |key: &str| metadata[key].as_f64().unwrap_or(0.0);
        Self { device_width: get("deviceWidth"), device_height: get("deviceHeight"), offset_top: get("offsetTop") }
    }

    fn json(&self) -> Value {
        json!({ "deviceWidth": self.device_width, "deviceHeight": self.device_height, "offsetTop": self.offset_top })
    }
}

/// Точка на картинке (доля ширины и высоты, 0…1) → CSS-пиксели окна, как их ждёт
/// `Input.dispatchMouseEvent`. `iw`×`ih` — размер кадра в точках: кадр бывает уменьшен
/// (maxWidth) и снят в физических пикселях (масштаб экрана), поэтому высоту в CSS-пикселях
/// считаем по ширине окна. Без размера кадра — по высоте окна.
pub fn map_point(meta: &FrameMeta, nx: f64, ny: f64, iw: f64, ih: f64) -> (f64, f64) {
    let (nx, ny) = (nx.clamp(0.0, 1.0), ny.clamp(0.0, 1.0));
    let x = nx * meta.device_width;
    let height = if iw > 0.0 && ih > 0.0 { ih * meta.device_width / iw } else { meta.device_height + meta.offset_top };
    let y = (ny * height - meta.offset_top).clamp(0.0, meta.device_height.max(0.0));
    (x, y)
}

// ---------------------------------------------------------------------------
// Состояние экрана бота
// ---------------------------------------------------------------------------

struct BotScreen {
    /// Управление у человека: ход бота ждёт перед каждым следующим инструментом.
    held: watch::Sender<bool>,
    /// Готовые строки SSE для открытых панелей.
    frames: broadcast::Sender<Arc<String>>,
    viewers: usize,
    capture: Option<CancellationToken>,
    /// Поколение трансляции: прежний цикл, уходя, не трогает трансляцию нового.
    capture_gen: u64,
    last_frame: Option<(Arc<String>, FrameMeta)>,
    /// Счётчик кадров: после перехода запись ждёт свежий кадр для миниатюры.
    frame_seq: u64,
    last_tabs: Option<Arc<String>>,
    /// Почему картинки нет: новый зритель видит причину сразу.
    last_unavailable: Option<Arc<String>>,
    /// Управление отдал сам бот через `browser_human`: оно держится, пока человек не ответит.
    human_waiting: bool,
    /// Человек закрыл экран посреди `browser_human` и не вернулся.
    human_left: bool,
    /// Страница входа, о которой человеку уже сказали: одно событие на страницу.
    sign_in_told: Option<String>,
    shell: Option<CancellationToken>,
    /// Идущая команда для панели, открытой посреди неё: начало и хвост вывода.
    shell_start: Option<Arc<String>>,
    shell_tail: String,
    recording: Option<Recording>,
}

impl Default for BotScreen {
    fn default() -> Self {
        Self {
            held: watch::channel(false).0,
            frames: broadcast::channel(16).0,
            viewers: 0,
            capture: None,
            capture_gen: 0,
            last_frame: None,
            frame_seq: 0,
            last_tabs: None,
            last_unavailable: None,
            human_waiting: false,
            human_left: false,
            sign_in_told: None,
            shell: None,
            shell_start: None,
            shell_tail: String::new(),
            recording: None,
        }
    }
}

static SCREENS: LazyLock<Mutex<HashMap<String, BotScreen>>> = LazyLock::new(Default::default);
/// Удалённые боты: запоздалое завершение трансляции или зрителя не заводит их экран заново.
static GONE: LazyLock<Mutex<HashSet<String>>> = LazyLock::new(Default::default);

fn with<R>(bot: &str, f: impl FnOnce(&mut BotScreen) -> R) -> R {
    if GONE.lock().unwrap().contains(bot) {
        return f(&mut BotScreen::default());
    }
    let mut map = SCREENS.lock().unwrap();
    f(map.entry(bot.to_string()).or_default())
}

/// Бот удалён: трансляцию и команду остановить, последний кадр и состояние экрана отпустить
/// (аудит 19.11.2026: экран удалённого бота с последним кадром жил до конца процесса).
pub fn forget_bot(bot: &str) {
    GONE.lock().unwrap().insert(bot.to_string());
    if let Some(screen) = SCREENS.lock().unwrap().remove(bot) {
        screen.capture.iter().chain(screen.shell.iter()).for_each(CancellationToken::cancel);
    }
}

fn sse(value: &Value) -> Arc<String> {
    Arc::new(format!("data: {value}\n\n"))
}

pub fn is_held(bot: &str) -> bool {
    with(bot, |s| *s.held.borrow())
}

fn set_held(app: &App, bot: &str, held: bool) {
    let changed = with(bot, |s| s.held.send_replace(held) != held);
    if changed {
        app.turns.emit(json!({ "type": "screen_control", "botId": bot, "held": held }));
    }
}

/// Что бот делает сейчас: строка над картинкой.
pub fn step(app: &App, bot: &str, text: &str) {
    if !text.is_empty() {
        app.turns.emit(json!({ "type": "screen_step", "botId": bot, "text": text }));
    }
}

/// Человек держит управление — ход ждёт. Останов хода ожидание прерывает.
pub async fn wait_released(app: &App, bot: &str, cancel: &CancellationToken) {
    let mut rx = with(bot, |s| s.held.subscribe());
    if !*rx.borrow_and_update() {
        return;
    }
    step(app, bot, "Ждёт: управление у человека");
    // Никто не смотрит за ходом (рутина): ждём не бесконечно, потом управление возвращается боту.
    // С человеком рядом удержание снимается само, когда закрывают экран (см. `Viewer`).
    let limit = if app.turns.is_unattended(bot) {
        test_duration("CREW_TEST_UNATTENDED_HOLD_MS", UNATTENDED_HOLD)
    } else {
        Duration::from_secs(100 * 365 * 24 * 3600)
    };
    let deadline = tokio::time::sleep(limit);
    tokio::pin!(deadline);
    loop {
        tokio::select! {
            changed = rx.changed() => {
                if changed.is_err() || !*rx.borrow_and_update() {
                    return;
                }
            }
            _ = cancel.cancelled() => return,
            _ = &mut deadline => {
                with(bot, |s| s.human_waiting = false);
                set_held(app, bot, false);
                return;
            }
        }
    }
}

/// `browser_human`: панель открывается сама, управление сразу у человека. `false` — панель так
/// и не открылась за полторы секунды, тогда вызывающий выводит настоящее окно браузера.
pub async fn hand_to_human(app: &Arc<App>, bot: &str, message: &str) -> bool {
    with(bot, |s| {
        s.human_waiting = true;
        s.human_left = false;
    });
    set_held(app, bot, true);
    app.turns.emit(json!({ "type": "screen_open", "botId": bot, "message": message }));
    let started = Instant::now();
    while started.elapsed() < PANEL_WAIT {
        if with(bot, |s| s.viewers > 0) {
            return true;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    with(bot, |s| s.viewers > 0)
}

/// Страница входа в браузере бота: просим открыть экран, один раз на страницу. Управление не берём,
/// на странице ничего не нажимаем: человек сам решит, что делать.
pub fn needs_human(app: &App, bot: &str, url: &str, reason: &str) {
    let key = page_key(url);
    let fresh = with(bot, |s| {
        let fresh = s.sign_in_told.as_deref() != Some(key.as_str());
        s.sign_in_told = Some(key);
        fresh
    });
    if fresh {
        app.turns.emit(json!({ "type": "browser_needs_human", "botId": bot, "url": url, "reason": reason }));
    }
}

/// Бот ушёл со страницы входа: к ней можно вернуться и сказать снова.
pub fn page_is_not_sign_in(bot: &str) {
    with(bot, |s| s.sign_in_told = None);
}

/// Адрес без `#…`: смена якоря не новая страница.
fn page_key(url: &str) -> String {
    url.split('#').next().unwrap_or(url).to_string()
}

/// `browser_show_to_human`: бот просит посмотреть. Только открывает панель и пишет строку в чат:
/// управление не берёт, прав не даёт, ход не ждёт.
pub fn show_to_human(app: &App, bot: &str, reason: &str) -> String {
    let reason: String = reason.trim().chars().take(300).collect();
    let reason = if reason.is_empty() { "посмотреть на экран браузера".to_string() } else { reason };
    app.turns.emit(json!({ "type": "browser_needs_human", "botId": bot, "url": "", "reason": reason, "asked": true }));
    "Экран бота показан человеку. Управление у тебя, ход не ждёт его. Если нужно, чтобы человек что-то сделал сам (вход, код, капча), вызови browser_human.".to_string()
}

/// Ожидание `browser_human` кончилось: управление снова у бота. `true` — человек закрыл экран,
/// не закончив, и ожидание сняла программа.
pub fn take_back(app: &App, bot: &str) -> bool {
    let left = with(bot, |s| {
        s.human_waiting = false;
        std::mem::take(&mut s.human_left)
    });
    set_held(app, bot, false);
    left
}

/// Человек закрыл экран посреди `browser_human`. Вернулся за отведённое время — ждём дальше,
/// нет — снимаем удержание и закрываем карточку «Готово / Не буду», чтобы ход не висел.
async fn human_gone(app: Arc<App>, bot: String) {
    tokio::time::sleep(test_duration("CREW_TEST_SCREEN_GRACE_MS", LEFT_GRACE)).await;
    let gone = with(&bot, |s| {
        let gone = s.viewers == 0 && s.human_waiting;
        if gone {
            s.human_left = true;
        }
        gone
    });
    if !gone {
        return;
    }
    for card in app.turns.pending_for(&bot) {
        if card["kind"] == "approval" && card["tool"] == "browser_human" {
            if let Some(id) = card["id"].as_str() {
                crate::turn::decide_approval(&app, id, crate::turn::Reply::Deny).await;
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Трансляция
// ---------------------------------------------------------------------------

/// Зритель потока. Ушёл последний — трансляция стоит, а удержание снимается, чтобы ход не завис.
struct Viewer {
    app: Arc<App>,
    bot: String,
}

impl Drop for Viewer {
    fn drop(&mut self) {
        let release = with(&self.bot, |s| {
            s.viewers = s.viewers.saturating_sub(1);
            if s.viewers > 0 {
                return false;
            }
            if let Some(stop) = s.capture.take() {
                stop.cancel();
            }
            !s.human_waiting
        });
        if release {
            set_held(&self.app, &self.bot, false);
        } else if let Ok(runtime) = tokio::runtime::Handle::try_current() {
            runtime.spawn(human_gone(self.app.clone(), self.bot.clone()));
        }
    }
}

fn publish_frame(bot: &str, data: &str, meta: FrameMeta) {
    let chunk = sse(&json!({ "type": "frame", "data": data, "meta": meta.json() }));
    with(bot, |s| {
        s.last_unavailable = None;
        s.last_frame = Some((Arc::new(data.to_string()), meta));
        s.frame_seq += 1;
        let _ = s.frames.send(chunk);
    });
}

fn unavailable(bot: &str, message: &str) {
    let chunk = sse(&json!({ "type": "unavailable", "message": message }));
    with(bot, |s| {
        s.last_unavailable = Some(chunk.clone());
        let _ = s.frames.send(chunk);
    });
}

/// Вызов браузера с коротким сроком: внутри цикла трансляции ждать по 20 с нельзя.
async fn quick(session: &Session, sid: &str, method: &str, params: Value) -> Result<Value, String> {
    match tokio::time::timeout(LOOP_CALL, session.cdp.call(Some(sid), method, params)).await {
        Ok(result) => result,
        Err(_) => Err(format!("браузер не ответил на {method} за {} с", LOOP_CALL.as_secs())),
    }
}

fn tabs_json(session: &Session) -> Value {
    let tabs = session.tabs.lock().unwrap();
    let mine = tabs.mine();
    let list: Vec<Value> = mine
        .iter()
        .filter_map(|id| {
            let page = tabs.pages.get(id)?;
            Some(json!({ "title": page.title, "url": page.url, "current": tabs.current.as_ref() == Some(id) }))
        })
        .collect();
    let current = tabs.current.as_ref().and_then(|id| tabs.pages.get(id));
    json!({
        "type": "tabs",
        "url": current.map(|p| p.url.clone()).unwrap_or_default(),
        "title": current.map(|p| p.title.clone()).unwrap_or_default(),
        "tabs": list,
        "engine": session.backend(),
    })
}

fn publish_tabs(bot: &str, session: &Session) {
    let chunk = sse(&tabs_json(session));
    with(bot, |s| {
        if s.last_tabs.as_deref() != Some(&*chunk) {
            s.last_tabs = Some(chunk.clone());
            let _ = s.frames.send(chunk);
        }
    });
}

fn current_url(session: &Session) -> String {
    session.with_page(|page| page.url.clone()).unwrap_or_default()
}

/// Размер окна в CSS-пикселях: для снимков вместо трансляции и для ввода до первого кадра.
async fn layout_meta(session: &Session, sid: &str) -> Result<FrameMeta, String> {
    let metrics = quick(session, sid, "Page.getLayoutMetrics", json!({})).await?;
    let view = if metrics["cssVisualViewport"].is_object() { &metrics["cssVisualViewport"] } else { &metrics["layoutViewport"] };
    let meta = FrameMeta { device_width: view["clientWidth"].as_f64().unwrap_or(0.0), device_height: view["clientHeight"].as_f64().unwrap_or(0.0), offset_top: 0.0 };
    if meta.device_width <= 0.0 {
        return Err("браузер не сообщил размер окна".into());
    }
    Ok(meta)
}

async fn poll_frame(session: &Session, sid: &str) -> Result<(String, FrameMeta), String> {
    let shot = quick(session, sid, "Page.captureScreenshot", json!({ "format": "jpeg", "quality": 60 })).await?;
    let data = shot["data"].as_str().filter(|d| !d.is_empty()).ok_or("браузер не вернул снимок")?.to_string();
    let meta = layout_meta(session, sid).await?;
    Ok((data, meta))
}

async fn capture(app: Arc<App>, bot: String, stop: CancellationToken, generation: u64) {
    let session = match app.browsers.session(&app.store, &bot).await {
        Ok(session) => session,
        Err(e) => {
            unavailable(&bot, &e);
            with(&bot, |s| {
                if s.capture_gen == generation {
                    s.capture = None;
                }
            });
            return;
        }
    };
    let (tx, mut rx) = mpsc::unbounded_channel();
    session.tabs.lock().unwrap().frame_tx = Some(tx.clone());
    let mut current: Option<String> = None;
    let mut started = Instant::now();
    let mut got_frame = false;
    let mut polling = false;
    let mut failures = 0;
    let mut last_ack = Instant::now();
    let mut last_poll = Instant::now() - POLL_EVERY;
    let mut last_frame_at = Instant::now();
    let mut tick = tokio::time::interval(Duration::from_millis(250));
    loop {
        tokio::select! {
            _ = stop.cancelled() => break,
            frame = rx.recv() => {
                let Some((sid, params)) = frame else { break };
                // Подтверждение с задержкой: так браузер сам держит не больше ~6 кадров в секунду.
                let wait = ACK_EVERY.saturating_sub(last_ack.elapsed());
                last_ack = Instant::now() + wait;
                let (cdp, ack_sid, ack) = (session.cdp.clone(), sid.clone(), params["sessionId"].clone());
                tokio::spawn(async move {
                    tokio::time::sleep(wait).await;
                    let _ = cdp.call(Some(&ack_sid), "Page.screencastFrameAck", json!({ "sessionId": ack })).await;
                });
                if current.as_deref() != Some(sid.as_str()) {
                    continue;
                }
                if let Some(data) = params["data"].as_str() {
                    got_frame = true;
                    last_frame_at = Instant::now();
                    publish_frame(&bot, data, FrameMeta::from_screencast(&params["metadata"]));
                }
            }
            _ = tick.tick() => {
                if session.cdp.is_closed() {
                    unavailable(&bot, "Браузер бота закрыт. Открой панель заново: он запустится снова.");
                    break;
                }
                // Браузер команды общий: человеку показываем вкладку этого бота, если им сейчас никто не работает.
                if session.view_as(&bot).is_none() {
                    // Браузер сейчас у другого бота команды: его вкладку этому человеку не показываем.
                    if let Some(old) = current.take() {
                        let _ = quick(&session, &old, "Page.stopScreencast", json!({})).await;
                    }
                    continue;
                }
                let now_sid = session.current_session();
                if now_sid != current {
                    if let Some(old) = current.take() {
                        let _ = quick(&session, &old, "Page.stopScreencast", json!({})).await;
                    }
                    current = now_sid;
                    (got_frame, polling, failures, started) = (false, false, 0, Instant::now());
                    if let Some(sid) = &current {
                        let params = json!({ "format": "jpeg", "quality": 60, "maxWidth": 1280, "maxHeight": 900, "everyNthFrame": 1 });
                        polling = quick(&session, sid, "Page.startScreencast", params).await.is_err();
                    }
                }
                if current.is_some() && !got_frame && !polling && started.elapsed() > FIRST_FRAME_WAIT {
                    // Свёрнутое окно или движок без трансляции: дальше обычные снимки.
                    polling = true;
                }
                // Трансляция молчит дольше обычного: страница могла просто не меняться, а могла и
                // перестать отдавать кадры. Один снимок освежит картинку в любом случае.
                let stale = got_frame && !polling && last_frame_at.elapsed() > STALE_FRAME && last_poll.elapsed() > STALE_FRAME;
                if let (true, Some(sid)) = ((polling && last_poll.elapsed() >= POLL_EVERY) || stale, current.as_deref()) {
                    last_poll = Instant::now();
                    match poll_frame(&session, sid).await {
                        Ok((data, meta)) => {
                            failures = 0;
                            if stale {
                                last_frame_at = Instant::now();
                            }
                            publish_frame(&bot, &data, meta);
                        }
                        Err(e) => {
                            failures += 1;
                            if failures == 3 {
                                let engine = if session.is_obscura() {
                                    "лёгкий браузер Obscura не отдаёт картинку"
                                } else if session.is_moli() {
                                    "лёгкий браузер Moli не отдаёт картинку"
                                } else {
                                    "браузер не отдаёт картинку"
                                };
                                unavailable(&bot, &format!("Экран бота недоступен: {engine} ({e}). Бот при этом работает как обычно."));
                            }
                        }
                    }
                }
                publish_tabs(&bot, &session);
                let url = current_url(&session);
                with(&bot, |s| {
                    if let Some(rec) = s.recording.as_mut().filter(|rec| !rec.stopped) {
                        rec.page_is(&url);
                    }
                });
            }
        }
    }
    // Новый цикл уже мог начать трансляцию на той же вкладке: её не останавливаем.
    let newest = with(&bot, |s| s.capture_gen == generation);
    if let (Some(sid), true) = (current, newest) {
        let _ = quick(&session, &sid, "Page.stopScreencast", json!({})).await;
    }
    {
        let mut tabs = session.tabs.lock().unwrap();
        if tabs.frame_tx.as_ref().is_some_and(|mine| mine.same_channel(&tx)) {
            tabs.frame_tx = None;
        }
    }
    // Цикл кончился сам (браузер закрыт): следующий зритель запустит трансляцию заново.
    // Свой токен не отменён — значит, в s.capture лежит именно он: живой цикл никто не подменяет.
    if !stop.is_cancelled() {
        stop.cancel();
        with(&bot, |s| {
            if s.capture_gen == generation {
                s.capture = None;
            }
        });
    }
}

async fn stream(State(app): State<Arc<App>>, Path(id): Path<String>) -> Response {
    if app.store.bot(&id).is_none() {
        return fail(StatusCode::NOT_FOUND, "бот не найден");
    }
    let (rx, first) = with(&id, |s| {
        s.viewers += 1;
        if s.capture.is_none() {
            let stop = CancellationToken::new();
            s.capture = Some(stop.clone());
            s.capture_gen += 1;
            tokio::spawn(capture(app.clone(), id.clone(), stop, s.capture_gen));
        }
        let active = s.recording.as_ref().is_some_and(|rec| !rec.stopped);
        let mut first = vec![sse(&json!({ "type": "control", "held": *s.held.borrow(), "recording": active }))];
        if let Some((data, meta)) = &s.last_frame {
            first.push(sse(&json!({ "type": "frame", "data": **data, "meta": meta.json() })));
        }
        first.extend(s.last_tabs.clone());
        first.extend(s.last_unavailable.clone());
        if let Some(start) = &s.shell_start {
            first.push(start.clone());
            if !s.shell_tail.is_empty() {
                first.push(sse(&json!({ "type": "shell_output", "stream": "out", "text": s.shell_tail })));
            }
        }
        (s.frames.subscribe(), first)
    });
    let viewer = Viewer { app: app.clone(), bot: id };
    let hello = futures_util::stream::iter(first.into_iter().map(|chunk| (*chunk).clone()));
    let live = futures_util::stream::unfold((rx, viewer), |(mut rx, viewer)| async move {
        loop {
            match tokio::time::timeout(Duration::from_secs(15), rx.recv()).await {
                Ok(Ok(chunk)) => return Some(((*chunk).clone(), (rx, viewer))),
                Ok(Err(broadcast::error::RecvError::Lagged(_))) => continue,
                Ok(Err(broadcast::error::RecvError::Closed)) => return None,
                Err(_) => return Some((": ping\n\n".to_string(), (rx, viewer))),
            }
        }
    });
    Response::builder()
        .header(header::CONTENT_TYPE, "text/event-stream; charset=utf-8")
        .header(header::CACHE_CONTROL, "no-cache, no-transform")
        .header("x-accel-buffering", "no")
        .body(Body::from_stream(hello.chain(live).map(Ok::<String, Infallible>)))
        .unwrap()
}

// ---------------------------------------------------------------------------
// Ввод человека
// ---------------------------------------------------------------------------

fn fail(status: StatusCode, message: impl Into<String>) -> Response {
    (status, Json(json!({ "error": message.into() }))).into_response()
}

fn ok(data: Value) -> Response {
    (StatusCode::OK, Json(data)).into_response()
}

fn body_json(body: &Bytes) -> Value {
    serde_json::from_slice(body).unwrap_or_else(|_| json!({}))
}

fn s<'a>(body: &'a Value, key: &str) -> &'a str {
    body.get(key).and_then(Value::as_str).unwrap_or_default()
}

fn f(body: &Value, key: &str) -> f64 {
    body.get(key).and_then(Value::as_f64).unwrap_or(0.0)
}

const NOT_HELD: &str = "Сначала возьми управление: кнопка «Взять управление».";

/// Бот есть, управление у человека — дай его браузер.
/// Вместе с сессией возвращается замок браузера (если он свободен): пока обработчик не закончил, другой бот команды
/// не возьмёт браузер и действие человека не попадёт в его вкладку.
async fn held_session(app: &Arc<App>, id: &str) -> Result<(Arc<Session>, Option<tokio::sync::OwnedMutexGuard<()>>), Response> {
    if app.store.bot(id).is_none() {
        return Err(fail(StatusCode::NOT_FOUND, "бот не найден"));
    }
    if !is_held(id) {
        return Err(fail(StatusCode::CONFLICT, NOT_HELD));
    }
    let session = app.browsers.session(&app.store, id).await.map_err(|e| fail(StatusCode::INTERNAL_SERVER_ERROR, e))?;
    let Some(guard) = session.view_as(id) else {
        return Err(fail(StatusCode::CONFLICT, TEAM_BUSY));
    };
    Ok((session, guard))
}

const TEAM_BUSY: &str = "Браузер команды сейчас занят другим ботом. Подожди, пока он закончит шаг.";

/// Запись идёт: «Стоп» ещё не нажат.
fn recording(bot: &str) -> bool {
    with(bot, |s| s.recording.as_ref().is_some_and(|rec| !rec.stopped))
}

/// Кадр к шагу записи — последний кадр трансляции. Отдельный уменьшенный снимок (`clip.scale`)
/// брать нельзя: браузер на время снимка меняет масштаб вкладки, кадры трансляции приходят
/// уменьшенными, и клики человека после этого попадают мимо.
async fn thumb(bot: &str, session: &Session) -> String {
    if let Some(data) = with(bot, |s| s.last_frame.as_ref().map(|(data, _)| (**data).clone())) {
        return data;
    }
    match session.call("Page.captureScreenshot", json!({ "format": "jpeg", "quality": 40 })).await {
        Ok(shot) => shot["data"].as_str().unwrap_or_default().to_string(),
        Err(_) => String::new(),
    }
}

async fn frame_meta(bot: &str, session: &Session) -> Result<FrameMeta, String> {
    if let Some(meta) = with(bot, |s| s.last_frame.as_ref().map(|(_, meta)| *meta)).filter(|m| m.device_width > 0.0) {
        return Ok(meta);
    }
    let sid = session.session_id().await?;
    layout_meta(session, &sid).await
}

fn modifiers_of(body: &Value) -> i64 {
    body.get("modifiers").and_then(Value::as_i64).unwrap_or(0) & 15
}

/// Название сочетания для записи: «Control+A», «Enter».
fn combo(key: &str, modifiers: i64) -> String {
    let mut parts = Vec::new();
    for (bit, name) in [(2, "Control"), (1, "Alt"), (8, "Shift"), (4, "Meta")] {
        if modifiers & bit != 0 && key != name {
            parts.push(name.to_string());
        }
    }
    parts.push(if key.chars().count() == 1 { key.to_uppercase() } else { key.to_string() });
    parts.join("+")
}

async fn click(bot: &str, session: &Session, body: &Value) -> Result<(), String> {
    let meta = frame_meta(bot, session).await?;
    let (x, y) = map_point(&meta, f(body, "x"), f(body, "y"), f(body, "iw"), f(body, "ih"));
    // Второй клик двойного — тот же шаг, в запись не идёт.
    if recording(bot) && body.get("count").and_then(Value::as_i64).unwrap_or(1) <= 1 {
        let target = teach::describe_point(session, x, y).await;
        let shot = thumb(bot, session).await;
        let url = current_url(session);
        with(bot, |s| s.recording.as_mut().filter(|rec| !rec.stopped).map(|rec| rec.click(&url, target, shot)));
    }
    let button = match s(body, "button") {
        "right" => "right",
        "middle" => "middle",
        _ => "left",
    };
    let mask = match button {
        "right" => 2,
        "middle" => 4,
        _ => 1,
    };
    // Номер клика в серии, как его даёт браузер человека: двойной клик — это второй клик с count 2.
    let count = body.get("count").and_then(Value::as_i64).unwrap_or(1).clamp(1, 3);
    let modifiers = modifiers_of(body);
    session.input("Input.dispatchMouseEvent", json!({ "type": "mouseMoved", "x": x, "y": y, "button": "none", "buttons": 0 })).await?;
    let at = json!({ "x": x, "y": y, "button": button, "clickCount": count, "modifiers": modifiers });
    let mut pressed = at.clone();
    pressed["type"] = json!("mousePressed");
    pressed["buttons"] = json!(mask);
    session.input("Input.dispatchMouseEvent", pressed).await?;
    let mut released = at;
    released["type"] = json!("mouseReleased");
    released["buttons"] = json!(0);
    session.input("Input.dispatchMouseEvent", released).await?;
    session.with_page(|page| page.mouse = (x, y))?;
    Ok(())
}

async fn wheel(bot: &str, session: &Session, body: &Value) -> Result<(), String> {
    let meta = frame_meta(bot, session).await?;
    let (x, y) = map_point(&meta, f(body, "x"), f(body, "y"), f(body, "iw"), f(body, "ih"));
    let params = json!({ "type": "mouseWheel", "x": x, "y": y, "deltaX": f(body, "dx"), "deltaY": f(body, "dy") });
    session.input("Input.dispatchMouseEvent", params).await
}

async fn key(bot: &str, session: &Session, body: &Value) -> Result<(), String> {
    let key = s(body, "key");
    if key.is_empty() {
        return Err("нет клавиши".into());
    }
    let modifiers = modifiers_of(body);
    let enter = key == "Enter";
    // Печатный символ без Control, Alt и Meta. Shift уже учтён в самом символе.
    let printable = modifiers & 7 == 0 && key.chars().count() == 1;
    let text = if enter { "\r".to_string() } else if printable { key.to_string() } else { String::new() };

    if recording(bot) {
        let target = teach::describe_focused(session).await;
        let url = current_url(session);
        if printable {
            let fresh = with(bot, |s| s.recording.as_ref().is_some_and(|rec| !rec.continues_typing(&target)));
            let shot = if fresh { thumb(bot, session).await } else { String::new() };
            with(bot, |s| s.recording.as_mut().filter(|rec| !rec.stopped).map(|rec| rec.typed(&url, target, key, shot)));
        } else if key == "Backspace" && modifiers == 0 {
            with(bot, |s| s.recording.as_mut().filter(|rec| !rec.stopped).map(|rec| rec.backspace(&target)));
        } else if enter || key == "Tab" || key == "Escape" || modifiers & 6 != 0 {
            let shot = thumb(bot, session).await;
            let keys = combo(key, modifiers);
            with(bot, |s| s.recording.as_mut().filter(|rec| !rec.stopped).map(|rec| rec.press(&url, target, &keys, shot)));
        }
    }

    let code = s(body, "code");
    let vk = body.get("keyCode").and_then(Value::as_i64).unwrap_or(0);
    let mut down = json!({
        "type": if text.is_empty() { "rawKeyDown" } else { "keyDown" },
        "key": key, "code": code, "windowsVirtualKeyCode": vk, "nativeVirtualKeyCode": vk, "modifiers": modifiers,
    });
    if !text.is_empty() {
        down["text"] = json!(text);
        down["unmodifiedText"] = json!(text);
    }
    // Правку текста сочетаниями браузер выполняет только с явной командой.
    if modifiers & 7 == 2 {
        let command = match key.to_lowercase().as_str() {
            "a" => Some("selectAll"),
            "c" => Some("copy"),
            "x" => Some("cut"),
            "v" => Some("paste"),
            "z" => Some("undo"),
            _ => None,
        };
        if let Some(command) = command {
            down["commands"] = json!([command]);
        }
    }
    session.input("Input.dispatchKeyEvent", down).await?;
    let up = json!({ "type": "keyUp", "key": key, "code": code, "windowsVirtualKeyCode": vk, "nativeVirtualKeyCode": vk, "modifiers": modifiers });
    session.input("Input.dispatchKeyEvent", up).await
}

fn steps_brief(bot: &str) -> Value {
    with(bot, |s| match &s.recording {
        Some(rec) if !rec.stopped => json!(rec.steps.len()),
        _ => Value::Null,
    })
}

async fn input(State(app): State<Arc<App>>, Path(id): Path<String>, body: Bytes) -> Response {
    let (session, _held) = match held_session(&app, &id).await {
        Ok(pair) => pair,
        Err(r) => return r,
    };
    let body = body_json(&body);
    let done = match s(&body, "kind") {
        "click" => click(&id, &session, &body).await,
        "wheel" => wheel(&id, &session, &body).await,
        "key" => key(&id, &session, &body).await,
        other => Err(format!("неизвестный ввод: {other}")),
    };
    match done {
        Ok(()) => ok(json!({ "ok": true, "recorded": steps_brief(&id) })),
        Err(e) => fail(StatusCode::BAD_REQUEST, e),
    }
}

async fn nav(State(app): State<Arc<App>>, Path(id): Path<String>, body: Bytes) -> Response {
    let (session, _held) = match held_session(&app, &id).await {
        Ok(pair) => pair,
        Err(r) => return r,
    };
    let body = body_json(&body);
    let done: Result<(), String> = async {
        if let Some(index) = body.get("tab").and_then(Value::as_u64) {
            let mut tabs = session.tabs.lock().unwrap();
            let target = (index as usize).checked_sub(1).and_then(|i| tabs.mine().get(i).cloned()).ok_or("нет такой вкладки")?;
            tabs.current = Some(target);
            return Ok(());
        }
        match s(&body, "action") {
            "back" | "forward" => {
                let history = session.call("Page.getNavigationHistory", json!({})).await?;
                let index = history["currentIndex"].as_i64().unwrap_or(0) + if s(&body, "action") == "back" { -1 } else { 1 };
                let entry = history["entries"].as_array().and_then(|e| e.get(index.max(0) as usize)).filter(|_| index >= 0).ok_or("дальше истории нет")?;
                session.call("Page.navigateToHistoryEntry", json!({ "entryId": entry["id"] })).await.map(|_| ())
            }
            "reload" => session.call("Page.reload", json!({})).await.map(|_| ()),
            _ => {
                let typed = s(&body, "url").trim().to_string();
                // Адрес без схемы — как в адресной строке браузера: сначала https, не открылся — http.
                let tries = if typed.contains("://") { vec![typed.clone()] } else { vec![format!("https://{typed}"), format!("http://{typed}")] };
                let mut opened = None;
                let mut failure = String::new();
                for candidate in tries {
                    crate::browser_tools::check_address(&candidate).await?;
                    let result = session.call("Page.navigate", json!({ "url": candidate })).await?;
                    match result["errorText"].as_str() {
                        Some(error) => failure = format!("страница не открылась: {error}"),
                        None => {
                            opened = Some(candidate);
                            break;
                        }
                    }
                }
                let url = opened.ok_or(failure)?;
                if recording(&id) {
                    // Миниатюра — уже новой страницы: ждём свежий кадр, но не дольше полутора секунд.
                    let seq = with(&id, |s| s.frame_seq);
                    let started = Instant::now();
                    tokio::time::sleep(Duration::from_millis(300)).await;
                    while with(&id, |s| s.frame_seq) <= seq + 1 && started.elapsed() < Duration::from_millis(1500) {
                        tokio::time::sleep(Duration::from_millis(100)).await;
                    }
                    let shot = thumb(&id, &session).await;
                    with(&id, |s| s.recording.as_mut().filter(|rec| !rec.stopped).map(|rec| rec.open(&url, shot)));
                }
                Ok(())
            }
        }
    }
    .await;
    match done {
        Ok(()) => ok(json!({ "ok": true, "recorded": steps_brief(&id) })),
        Err(e) => fail(StatusCode::BAD_REQUEST, e),
    }
}

async fn control(State(app): State<Arc<App>>, Path(id): Path<String>, body: Bytes) -> Response {
    if app.store.bot(&id).is_none() {
        return fail(StatusCode::NOT_FOUND, "бот не найден");
    }
    let hold = body_json(&body).get("hold").and_then(Value::as_bool).unwrap_or(false);
    // Без открытой панели взять управление нельзя: иначе его некому вернуть, и ход ждал бы вечно.
    if hold && with(&id, |s| s.viewers == 0) {
        return fail(StatusCode::CONFLICT, "Открой экран бота, чтобы взять управление.");
    }
    if !hold {
        with(&id, |s| s.human_waiting = false);
    }
    set_held(&app, &id, hold);
    ok(json!({ "held": hold }))
}

// ---------------------------------------------------------------------------
// Живая оболочка
// ---------------------------------------------------------------------------

/// Команда, вывод которой идёт в панель. Ставится на время `run_command`.
pub struct Shell {
    app: Arc<App>,
    bot: String,
    sent: AtomicUsize,
}

tokio::task_local! {
    pub static SHELL: Arc<Shell>;
}

impl Shell {
    pub fn begin(app: &Arc<App>, bot: &str, command: &str, stop: CancellationToken) -> Arc<Self> {
        // Команда и её вывод идут только в открытый экран этого бота, не в общий поток событий:
        // его слушают док, питомцы и внешние клиенты, а в выводе бывают ключи.
        let start = sse(&json!({ "type": "shell_start", "command": command }));
        with(bot, |s| {
            s.shell = Some(stop);
            s.shell_start = Some(start.clone());
            s.shell_tail.clear();
            let _ = s.frames.send(start);
        });
        Arc::new(Self { app: app.clone(), bot: bot.to_string(), sent: AtomicUsize::new(0) })
    }

    pub fn end(&self, code: i32, stopped: bool) {
        let end = sse(&json!({ "type": "shell_end", "code": code, "stopped": stopped }));
        with(&self.bot, |s| {
            s.shell = None;
            s.shell_start = None;
            s.shell_tail.clear();
            let _ = s.frames.send(end);
        });
        // В общий поток — только то, что команда кончилась, без её текста и вывода.
        self.app.turns.emit(json!({ "type": "shell_done", "botId": self.bot }));
    }

    fn output(&self, stream: &str, text: &str) {
        if text.is_empty() {
            return;
        }
        let before = self.sent.fetch_add(text.len(), Ordering::Relaxed);
        if before >= SHELL_LIMIT {
            return;
        }
        let mut text = text.to_string();
        if before + text.len() >= SHELL_LIMIT {
            text.push_str("\n[дальше вывод в панели не показывается]\n");
        }
        let chunk = sse(&json!({ "type": "shell_output", "stream": stream, "text": text }));
        with(&self.bot, |s| {
            s.shell_tail.push_str(&text);
            if s.shell_tail.len() > SHELL_TAIL {
                let mut cut = s.shell_tail.len() - SHELL_TAIL;
                while !s.shell_tail.is_char_boundary(cut) {
                    cut += 1;
                }
                s.shell_tail.drain(..cut);
            }
            let _ = s.frames.send(chunk);
        });
    }
}

#[derive(Default)]
struct TailFile {
    path: PathBuf,
    offset: u64,
    carry: Vec<u8>,
}

/// Новые байты файла как текст. Недописанная буква UTF-8 ждёт следующего раза.
fn read_new(file: &mut TailFile, last: bool) -> String {
    let Ok(mut handle) = std::fs::File::open(&file.path) else { return String::new() };
    if handle.seek(SeekFrom::Start(file.offset)).is_err() {
        return String::new();
    }
    let mut fresh = Vec::new();
    let Ok(read) = handle.read_to_end(&mut fresh) else { return String::new() };
    file.offset += read as u64;
    file.carry.extend_from_slice(&fresh);
    split_utf8(&mut file.carry, last)
}

/// Готовый текст из буфера; хвост с недописанной буквой остаётся в буфере.
fn split_utf8(buffer: &mut Vec<u8>, last: bool) -> String {
    let keep = match std::str::from_utf8(buffer) {
        Ok(_) => 0,
        Err(e) if e.error_len().is_none() && !last => buffer.len() - e.valid_up_to(),
        Err(_) => 0,
    };
    let ready: Vec<u8> = buffer.drain(..buffer.len() - keep).collect();
    String::from_utf8_lossy(&ready).into_owned()
}

/// Читает файлы вывода команды, пока она идёт. При удалении дочитывает остаток.
pub struct Tail {
    shell: Arc<Shell>,
    files: Arc<Mutex<[TailFile; 2]>>,
    stop: CancellationToken,
}

/// Хвост файлов вывода, если команду запустил `run_command` (задан [`SHELL`]).
pub fn tail_files(out: &FsPath, err: &FsPath) -> Option<Tail> {
    let shell = SHELL.try_with(Arc::clone).ok()?;
    let files = Arc::new(Mutex::new([
        TailFile { path: out.to_path_buf(), ..TailFile::default() },
        TailFile { path: err.to_path_buf(), ..TailFile::default() },
    ]));
    let stop = CancellationToken::new();
    {
        let (shell, files, stop) = (shell.clone(), files.clone(), stop.clone());
        tokio::spawn(async move {
            loop {
                tokio::select! {
                    _ = stop.cancelled() => return,
                    _ = tokio::time::sleep(Duration::from_millis(250)) => {}
                }
                let chunks: Vec<String> = files.lock().unwrap().iter_mut().map(|file| read_new(file, false)).collect();
                shell.output("out", &chunks[0]);
                shell.output("err", &chunks[1]);
            }
        });
    }
    Some(Tail { shell, files, stop })
}

impl Drop for Tail {
    fn drop(&mut self) {
        self.stop.cancel();
        let chunks: Vec<String> = self.files.lock().unwrap().iter_mut().map(|file| read_new(file, true)).collect();
        self.shell.output("out", &chunks[0]);
        self.shell.output("err", &chunks[1]);
    }
}

async fn shell_stop(State(app): State<Arc<App>>, Path(id): Path<String>) -> Response {
    if app.store.bot(&id).is_none() {
        return fail(StatusCode::NOT_FOUND, "бот не найден");
    }
    match with(&id, |s| s.shell.clone()) {
        Some(stop) => {
            stop.cancel();
            ok(json!({ "ok": true }))
        }
        None => fail(StatusCode::NOT_FOUND, "команда уже закончилась"),
    }
}

// ---------------------------------------------------------------------------
// «Научи бота»
// ---------------------------------------------------------------------------

fn steps_json(bot: &str) -> Value {
    with(bot, |s| match &s.recording {
        Some(rec) => json!({ "recording": !rec.stopped, "stopped": rec.stopped, "steps": rec.steps }),
        None => json!({ "recording": false, "stopped": false, "steps": [] }),
    })
}

fn no_bot(app: &App, id: &str) -> Option<Response> {
    app.store.bot(id).is_none().then(|| fail(StatusCode::NOT_FOUND, "бот не найден"))
}

async fn teach_get(State(app): State<Arc<App>>, Path(id): Path<String>) -> Response {
    if let Some(r) = no_bot(&app, &id) {
        return r;
    }
    ok(steps_json(&id))
}

async fn teach_start(State(app): State<Arc<App>>, Path(id): Path<String>) -> Response {
    let (session, _held) = match held_session(&app, &id).await {
        Ok(pair) => pair,
        Err(r) => return r,
    };
    let url = current_url(&session);
    let shot = thumb(&id, &session).await;
    with(&id, |s| {
        let mut rec = Recording::default();
        if url.starts_with("http") {
            rec.open(&url, shot);
        }
        s.recording = Some(rec);
    });
    app.turns.emit(json!({ "type": "screen_record", "botId": id, "recording": true }));
    ok(steps_json(&id))
}

async fn teach_note(State(app): State<Arc<App>>, Path(id): Path<String>, body: Bytes) -> Response {
    if let Some(r) = no_bot(&app, &id) {
        return r;
    }
    let body = body_json(&body);
    let added = with(&id, |s| s.recording.as_mut().filter(|rec| !rec.stopped).map(|rec| rec.note(s_of(&body)))).is_some();
    if !added {
        return fail(StatusCode::CONFLICT, "запись не идёт");
    }
    ok(steps_json(&id))
}

fn s_of(body: &Value) -> &str {
    s(body, "text")
}

async fn teach_cancel(State(app): State<Arc<App>>, Path(id): Path<String>) -> Response {
    if let Some(r) = no_bot(&app, &id) {
        return r;
    }
    with(&id, |s| s.recording = None);
    app.turns.emit(json!({ "type": "screen_record", "botId": id, "recording": false }));
    ok(json!({ "ok": true }))
}

async fn teach_stop(State(app): State<Arc<App>>, Path(id): Path<String>) -> Response {
    let Some(bot) = app.store.bot(&id) else { return fail(StatusCode::NOT_FOUND, "бот не найден") };
    // Запись живёт до «Сохранить» или «Не сохранять»: оборвался вызов модели — «Стоп» можно нажать снова.
    let Some(steps) = with(&id, |s| {
        s.recording.as_mut().map(|rec| {
            rec.stopped = true;
            rec.steps.clone()
        })
    }) else {
        return fail(StatusCode::CONFLICT, "запись не идёт");
    };
    app.turns.emit(json!({ "type": "screen_record", "botId": id, "recording": false }));
    let acted = steps.iter().any(|step| step.kind != "open" && step.kind != "note");
    if !acted {
        with(&id, |s| s.recording = None);
        return fail(StatusCode::BAD_REQUEST, "Ничего не записано: покажи действия в панели, пока идёт запись.");
    }
    let (draft, warning) = teach::draft(&app, &bot, &steps).await;
    ok(json!({ "steps": steps, "draft": draft, "warning": warning }))
}

async fn teach_save(State(app): State<Arc<App>>, Path(id): Path<String>, body: Bytes) -> Response {
    if app.store.bot(&id).is_none() {
        return fail(StatusCode::NOT_FOUND, "бот не найден");
    }
    let body = body_json(&body);
    match app.store.learn_skill(&id, s(&body, "name"), s(&body, "description"), s(&body, "body")).await {
        Ok((skill, created)) => {
            with(&id, |s| s.recording = None);
            app.turns.emit(json!({
                "type": "skill_learned", "botId": id, "skillId": skill.id, "name": skill.name, "created": created,
            }));
            app.turns.emit(json!({ "type": "state_changed" }));
            ok(json!({ "skill": skill, "created": created }))
        }
        Err(e) => fail(StatusCode::BAD_REQUEST, e),
    }
}

pub fn routes() -> Router<Arc<App>> {
    Router::new()
        .route("/api/bots/{id}/screen", get(stream))
        .route("/api/bots/{id}/screen/control", post(control))
        .route("/api/bots/{id}/screen/input", post(input))
        .route("/api/bots/{id}/screen/nav", post(nav))
        .route("/api/bots/{id}/screen/shell/stop", post(shell_stop))
        .route("/api/bots/{id}/screen/teach", get(teach_get))
        .route("/api/bots/{id}/screen/teach/start", post(teach_start))
        .route("/api/bots/{id}/screen/teach/note", post(teach_note))
        .route("/api/bots/{id}/screen/teach/stop", post(teach_stop))
        .route("/api/bots/{id}/screen/teach/cancel", post(teach_cancel))
        .route("/api/bots/{id}/screen/teach/save", post(teach_save))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn point_maps_to_css_pixels_of_the_window() {
        let meta = FrameMeta { device_width: 1280.0, device_height: 900.0, offset_top: 0.0 };
        // Кадр в полный размер.
        assert_eq!(map_point(&meta, 0.5, 0.5, 1280.0, 900.0), (640.0, 450.0));
        // Кадр уменьшен вдвое (maxWidth) — те же CSS-пиксели.
        assert_eq!(map_point(&meta, 0.25, 0.5, 640.0, 450.0), (320.0, 450.0));
        // Экран с масштабом 2: кадр 2560×1800 точек, CSS-пиксели прежние.
        assert_eq!(map_point(&meta, 0.1, 0.2, 2560.0, 1800.0), (128.0, 180.0));
        // Без размера кадра — по высоте окна; за краем — край.
        assert_eq!(map_point(&meta, 1.5, -1.0, 0.0, 0.0), (1280.0, 0.0));
    }

    #[test]
    fn offset_top_is_taken_out_of_the_frame() {
        let meta = FrameMeta { device_width: 800.0, device_height: 560.0, offset_top: 40.0 };
        // Кадр 800×600: сверху полоса 40 CSS-пикселей, страница начинается под ней.
        assert_eq!(map_point(&meta, 0.5, 0.5, 800.0, 600.0), (400.0, 260.0));
        assert_eq!(map_point(&meta, 0.0, 0.0, 800.0, 600.0), (0.0, 0.0));
    }

    #[test]
    fn combos_are_named_for_the_recording() {
        assert_eq!(combo("Enter", 0), "Enter");
        assert_eq!(combo("a", 2), "Control+A");
        assert_eq!(combo("Tab", 8), "Shift+Tab");
    }

    #[test]
    fn utf8_split_waits_for_the_rest_of_a_letter() {
        let word = "жук".as_bytes();
        let mut buffer = word[..3].to_vec(); // «ж» и половина «у»
        assert_eq!(split_utf8(&mut buffer, false), "ж");
        assert_eq!(buffer.len(), 1);
        buffer.extend_from_slice(&word[3..]);
        assert_eq!(split_utf8(&mut buffer, false), "ук");
        assert!(buffer.is_empty());
    }
}

#[cfg(test)]
mod forget_tests {
    use super::*;

    #[test]
    fn a_deleted_bot_keeps_no_screen_and_late_events_do_not_bring_it_back() {
        let bot = format!("bot_gone_{}", crate::guard::random_token());
        let stop = CancellationToken::new();
        with(&bot, |s| {
            s.capture = Some(stop.clone());
            s.last_frame = Some((Arc::new("кадр".into()), FrameMeta::default()));
        });
        forget_bot(&bot);
        assert!(stop.is_cancelled(), "трансляция остановлена");
        unavailable(&bot, "браузер закрыт");
        with(&bot, |s| s.viewers = s.viewers.saturating_sub(1));
        assert!(!SCREENS.lock().unwrap().contains_key(&bot), "экран не завёлся заново");
    }
}
