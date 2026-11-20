//! Голос боту. Страница пишет микрофон в 16 кГц PCM (`web/voice.js`) и присылает сюда, а программа
//! отправляет запись на deloist.ru от имени аккаунта: токен устройства в страницу не попадает.
//! Здесь же разрешение микрофона для своей страницы в WebView2, выбор бота для горячих клавиш
//! и раздача `voice.js` / `snap.js` / `snap.html`.

use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use axum::extract::State;
use axum::http::{header, HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use base64::Engine;
use serde_json::{json, Value};

use crate::App;

// Облачное распознавание (deloist.ru) живёт в private/src/voice_cloud.rs; в публичной сборке там заглушка.
pub use crate::voice_cloud::transcribe;

/// Те же пределы, что у распознавания на сайте (SpeechKit, синхронный режим).
pub const RATES: [u32; 3] = [8000, 16000, 48000];
pub const MAX_BYTES: usize = 1024 * 1024;
pub const MIN_SECONDS: f64 = 0.3;

pub fn ok(data: Value) -> Response {
    (StatusCode::OK, [(header::CACHE_CONTROL, HeaderValue::from_static("no-store"))], Json(data)).into_response()
}

pub fn fail(status: u16, message: impl Into<String>) -> Response {
    let status = StatusCode::from_u16(status).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR);
    (status, [(header::CACHE_CONTROL, HeaderValue::from_static("no-store"))], Json(json!({ "error": message.into() }))).into_response()
}

// ---------------------------------------------------------------------------
// Распознавание
// ---------------------------------------------------------------------------

#[derive(Debug, PartialEq)]
pub struct Clip {
    pub pcm: Vec<u8>,
    pub rate: u32,
    pub lang: &'static str,
}

/// Запись от страницы: `{pcm: base64, rate, lang}`. Окно шлёт только JSON (так требует сторож),
/// поэтому PCM едет в base64. Проверяем всё, что потом проверит сайт, чтобы не платить за мусор.
pub fn clip(body: &Value) -> Result<Clip, (u16, &'static str)> {
    let rate = body.get("rate").and_then(Value::as_u64).unwrap_or(16000) as u32;
    if !RATES.contains(&rate) {
        return Err((422, "Такую частоту записи сайт не принимает"));
    }
    let lang = match body.get("lang").and_then(Value::as_str) {
        Some("en") => "en",
        _ => "ru",
    };
    let text = body.get("pcm").and_then(Value::as_str).unwrap_or_default();
    // Запись больше предела не раскодируем вовсе: base64 длиннее данных на треть.
    if text.len() > MAX_BYTES / 3 * 4 + 8 {
        return Err((413, "Запись длиннее 30 секунд: скажи покороче"));
    }
    let pcm = base64::engine::general_purpose::STANDARD.decode(text).map_err(|_| (400, "Запись повредилась по дороге. Попробуй ещё раз"))?;
    if pcm.len() > MAX_BYTES {
        return Err((413, "Запись длиннее 30 секунд: скажи покороче"));
    }
    // 16-битные отсчёты: нечётная длина значит обрезанную запись.
    if pcm.len() % 2 != 0 {
        return Err((400, "Запись повредилась по дороге. Попробуй ещё раз"));
    }
    if (pcm.len() as f64) < f64::from(rate) * 2.0 * MIN_SECONDS {
        return Err((422, "Слишком коротко: скажи ещё раз"));
    }
    Ok(Clip { pcm, rate, lang })
}

async fn transcribe_route(State(app): State<Arc<App>>, Json(body): Json<Value>) -> Response {
    let clip = match clip(&body) {
        Ok(clip) => clip,
        Err((status, message)) => return fail(status, message),
    };
    // Способ распознавания выбирают настройки: компьютер, Deloist или свой сервер.
    match crate::stt_local::transcribe(&app, clip).await {
        Ok(text) => ok(json!({ "text": text })),
        Err((status, message)) => fail(status, message),
    }
}

// ---------------------------------------------------------------------------
// Горячая клавиша голоса и «отложенное» действие для главного окна
// ---------------------------------------------------------------------------

/// Последний бот, с кем говорили: у кого самое свежее сообщение. Нет переписки — первый в списке.
pub async fn last_bot(app: &App) -> Option<String> {
    let mut best: Option<(i64, String)> = None;
    let bots = app.store.bots();
    for bot in &bots {
        let at = app.store.last_preview(&bot.id).await.get("at").and_then(Value::as_i64).unwrap_or(0);
        if best.as_ref().is_none_or(|(seen, _)| at > *seen) {
            best = Some((at, bot.id.clone()));
        }
    }
    best.map(|(_, id)| id).or_else(|| bots.first().map(|b| b.id.clone()))
}

/// Главное окно могло ещё грузиться, когда пришло событие: оно заберёт действие при запуске.
static PENDING: Mutex<Option<(Instant, Value)>> = Mutex::new(None);
/// Окно, закрытое в трей, пересоздаёт WebView заново: на медленной машине это не одна секунда.
const PENDING_LIFE: Duration = Duration::from_secs(60);

/// Где показать: в доке, если он включён, иначе в главном окне (его надо открыть).
#[cfg(feature = "desktop")]
pub fn announce(app: &App, mut event: Value) {
    let dock = app.store.settings().dock_enabled;
    event["target"] = json!(if dock { "dock" } else { "main" });
    if dock {
        // Мини-чат возьмёт фокус: право на это — только процессу дока.
        if let Some(pid) = app.dock.pid() {
            crate::hotkeys::allow_foreground(pid);
        }
    } else {
        *PENDING.lock().unwrap() = Some((Instant::now(), event.clone()));
        if !crate::hotkeys::test_hooks() {
            crate::desktop::activate_running_window();
        }
    }
    app.turns.emit(event);
}

/// Кому уходит голос: бот из настройки «Голос уходит», а если там `last` или бота уже нет —
/// последний, с кем говорили.
#[cfg(feature = "desktop")]
pub async fn voice_bot(app: &App) -> Option<String> {
    let target = crate::hotkeys::voice_target(app);
    if target != crate::hotkeys::LAST_BOT && app.store.bot(&target).is_some() {
        return Some(target);
    }
    last_bot(app).await
}

#[cfg(feature = "desktop")]
pub async fn on_hotkey(app: &Arc<App>) {
    let Some(bot) = voice_bot(app).await else { return };
    announce(app, json!({ "type": "voice_hotkey", "botId": bot }));
}

/// Микрофон на доке (не в чате бота): кому записывать.
async fn target_route(State(app): State<Arc<App>>) -> Response {
    ok(json!({ "botId": voice_bot(&app).await }))
}

// ---------------------------------------------------------------------------
// Состояние записи для дока: `voice_state`
// ---------------------------------------------------------------------------

/// Шаги голоса, которые видит док. Текста записи в событии нет никогда.
pub const VOICE_STATES: [&str; 5] = ["recording", "transcribing", "sending", "done", "error"];
/// Уровень микрофона не чаще 20 раз в секунду.
pub const LEVEL_EVERY: Duration = Duration::from_millis(50);

/// События только для окон человека: программа с ключом их не получает.
/// Заявки командной строки на добавление команды (`team_import_*`): номер и состав карточки видит только окно.
pub fn window_only(event: &Value) -> bool {
    matches!(event.get("type").and_then(Value::as_str), Some("voice_state" | "team_import_request" | "team_import_closed" | "device_pair_request" | "devices_changed" | "remote_event"))
}

/// Пропустить ли ещё одно событие: смена шага — всегда, уровень того же шага — не чаще `LEVEL_EVERY`.
pub fn should_emit(last: Option<(Instant, &str, &str)>, now: Instant, state: &str, bot: &str) -> bool {
    match last {
        Some((at, last_state, last_bot)) if last_state == state && last_bot == bot => now.duration_since(at) >= LEVEL_EVERY,
        _ => true,
    }
}

static LAST_STATE: Mutex<Option<(Instant, String, String)>> = Mutex::new(None);

async fn state_route(State(app): State<Arc<App>>, Json(body): Json<Value>) -> Response {
    let state = body.get("state").and_then(Value::as_str).unwrap_or_default();
    if !VOICE_STATES.contains(&state) {
        return fail(400, "нет такого шага");
    }
    let bot = body.get("botId").and_then(Value::as_str).unwrap_or_default();
    if app.store.bot(bot).is_none() {
        return fail(404, "бот не найден");
    }
    let now = Instant::now();
    {
        let mut last = LAST_STATE.lock().unwrap();
        let seen = last.as_ref().map(|(at, s, b)| (*at, s.as_str(), b.as_str()));
        if !should_emit(seen, now, state, bot) {
            return ok(json!({ "sent": false }));
        }
        *last = Some((now, state.to_string(), bot.to_string()));
    }
    let mut event = json!({ "type": "voice_state", "state": state, "botId": bot });
    if let Some(level) = body.get("level").and_then(Value::as_f64).filter(|l| l.is_finite()) {
        // Два знака хватит, чтобы рисовать: точнее незачем гонять по потоку.
        event["level"] = json!((level.clamp(0.0, 1.0) * 100.0).round() / 100.0);
    }
    app.turns.emit(event);
    ok(json!({ "sent": true }))
}

async fn take_pending() -> Response {
    let taken = PENDING.lock().unwrap().take().filter(|(at, _)| at.elapsed() < PENDING_LIFE);
    ok(json!({ "event": taken.map(|(_, event)| event) }))
}

// ---------------------------------------------------------------------------
// Микрофон в WebView2
// ---------------------------------------------------------------------------

/// Микрофон разрешён только своей странице: окно и так не уходит со своего адреса,
/// но проверка адреса остаётся на случай чужого кадра.
pub fn mic_allowed(uri: &str, origin: &str) -> bool {
    uri == origin || uri.starts_with(&format!("{origin}/"))
}

/// Без этого WebView2 спрашивает доступ к микрофону при каждой записи.
#[cfg(all(windows, feature = "desktop"))]
pub fn allow_microphone(webview: &wry::WebView, origin: &str) {
    use webview2_com::Microsoft::Web::WebView2::Win32::{COREWEBVIEW2_PERMISSION_KIND, COREWEBVIEW2_PERMISSION_KIND_MICROPHONE, COREWEBVIEW2_PERMISSION_STATE_ALLOW};
    use webview2_com::PermissionRequestedEventHandler;
    use wry::WebViewExtWindows;

    let origin = origin.trim_end_matches('/').to_string();
    let handler = PermissionRequestedEventHandler::create(Box::new(move |_, args| {
        let Some(args) = args else { return Ok(()) };
        let mut kind = COREWEBVIEW2_PERMISSION_KIND::default();
        unsafe { args.PermissionKind(&mut kind)? };
        if kind != COREWEBVIEW2_PERMISSION_KIND_MICROPHONE {
            return Ok(());
        }
        let mut uri = windows_core::PWSTR::null();
        unsafe { args.Uri(&mut uri)? };
        let uri = webview2_com::take_pwstr(uri);
        if mic_allowed(&uri, &origin) {
            unsafe { args.SetState(COREWEBVIEW2_PERMISSION_STATE_ALLOW)? };
        }
        Ok(())
    }));
    let mut token = 0i64;
    let core = webview.webview();
    if let Err(e) = unsafe { core.add_PermissionRequested(&handler, &mut token) } {
        eprintln!("[voice] не настроил микрофон: {e}");
    }
}

#[cfg(all(not(windows), feature = "desktop"))]
pub fn allow_microphone(_webview: &wry::WebView, _origin: &str) {}

// ---------------------------------------------------------------------------
// Маршруты и файлы страницы
// ---------------------------------------------------------------------------

const VOICE_JS: &str = include_str!("../web/voice.js");
const SNAP_JS: &str = include_str!("../web/snap.js");
const SNAP_HTML: &str = include_str!("../web/snap.html");

fn asset(body: &'static str, kind: &'static str) -> Response {
    ([(header::CONTENT_TYPE, kind), (header::CACHE_CONTROL, "no-cache")], body).into_response()
}

pub fn routes() -> Router<Arc<App>> {
    Router::new()
        .route("/voice.js", get(|| async { asset(VOICE_JS, "text/javascript; charset=utf-8") }))
        .route("/snap.js", get(|| async { asset(SNAP_JS, "text/javascript; charset=utf-8") }))
        .route("/snap.html", get(|| async { asset(SNAP_HTML, "text/html; charset=utf-8") }))
        .route("/api/voice/transcribe", post(transcribe_route))
        .route("/api/voice/pending", post(take_pending))
        .route("/api/voice/target", post(target_route))
        .route("/api/voice/state", post(state_route))
        .merge(desktop_routes())
}

#[cfg(feature = "desktop")]
fn desktop_routes() -> Router<Arc<App>> {
    Router::new().merge(crate::hotkeys::routes()).merge(crate::selection::routes())
}

#[cfg(not(feature = "desktop"))]
fn desktop_routes() -> Router<Arc<App>> {
    Router::new()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn b64(bytes: &[u8]) -> String {
        base64::engine::general_purpose::STANDARD.encode(bytes)
    }

    #[test]
    fn pcm_is_framed_like_the_site_expects() {
        let second = vec![0u8; 32000];
        let got = clip(&json!({ "pcm": b64(&second), "rate": 16000, "lang": "ru" })).unwrap();
        assert_eq!(got, Clip { pcm: second.clone(), rate: 16000, lang: "ru" });
        assert_eq!(clip(&json!({ "pcm": b64(&second) })).unwrap().rate, 16000, "по умолчанию 16 кГц");
        assert_eq!(clip(&json!({ "pcm": b64(&second), "lang": "de" })).unwrap().lang, "ru");
        assert_eq!(clip(&json!({ "pcm": b64(&second), "lang": "en" })).unwrap().lang, "en");
    }

    #[test]
    fn bad_clips_are_stopped_before_the_paid_call() {
        // 0,3 с на 16 кГц — 9600 байт: на байт меньше уже «слишком коротко».
        assert!(clip(&json!({ "pcm": b64(&vec![0; 9600]) })).is_ok());
        assert_eq!(clip(&json!({ "pcm": b64(&vec![0; 9598]) })).unwrap_err().0, 422);
        assert_eq!(clip(&json!({ "pcm": b64(&vec![0; 9601]) })).unwrap_err().0, 400, "нечётная длина");
        assert_eq!(clip(&json!({ "pcm": b64(&vec![0; MAX_BYTES + 2]) })).unwrap_err().0, 413);
        assert_eq!(clip(&json!({ "pcm": "@@@" })).unwrap_err().0, 400);
        assert_eq!(clip(&json!({ "pcm": b64(&vec![0; 32000]), "rate": 44100 })).unwrap_err().0, 422);
        // 30 секунд на 16 кГц влезают в предел сайта.
        assert!(clip(&json!({ "pcm": b64(&vec![0; 30 * 16000 * 2]) })).is_ok());
    }

    #[test]
    fn the_level_is_throttled_but_a_new_step_always_goes_through() {
        let start = Instant::now();
        let ms = |n| start + Duration::from_millis(n);
        assert!(should_emit(None, start, "recording", "a"));
        let last = Some((start, "recording", "a"));
        assert!(!should_emit(last, ms(20), "recording", "a"), "чаще 20 Гц");
        assert!(should_emit(last, ms(50), "recording", "a"));
        assert!(should_emit(last, ms(1), "transcribing", "a"), "смена шага");
        assert!(should_emit(last, ms(1), "recording", "b"), "другой бот");
        assert!(window_only(&json!({ "type": "voice_state", "state": "recording" })));
        assert!(!window_only(&json!({ "type": "voice_hotkey" })));
        assert!(window_only(&json!({ "type": "team_import_request" })) && window_only(&json!({ "type": "team_import_closed" })));
        assert!(window_only(&json!({ "type": "device_pair_request", "req": "r" })) && window_only(&json!({ "type": "remote_event", "pc": "p" })), "плашка знакомства и события чужого ПК только окну");
        assert!(window_only(&json!({ "type": "devices_changed" })), "смена списка устройств только окну");
    }

    #[test]
    fn the_microphone_is_only_for_our_own_page() {
        let origin = "http://127.0.0.1:18899";
        assert!(mic_allowed("http://127.0.0.1:18899/dock.html", origin));
        assert!(mic_allowed("http://127.0.0.1:18899", origin));
        assert!(!mic_allowed("http://127.0.0.1:18899.evil.com/", origin));
        assert!(!mic_allowed("https://example.com/", origin));
    }
}
