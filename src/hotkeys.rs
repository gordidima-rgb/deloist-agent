//! Горячие клавиши на всю систему: голос боту, «Покажи на экране», правка выделенного текста.
//!
//! Сочетания регистрирует главный процесс через RegisterHotKey на невидимом окне сообщений в своём
//! потоке: цикл окна программы не трогаем. Человек меняет их в настройках, и они применяются сразу.
//! Занятое другой программой сочетание не роняет ничего: остаётся прежнее рабочее, а рядом с полем
//! пишется, что новое занято. Настройки лежат в обычном `state.json` под ключом `hotkeys`.

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicBool, AtomicIsize, Ordering};
use std::sync::{Arc, Condvar, Mutex, OnceLock};
use std::time::Duration;

use axum::extract::State;
use axum::response::Response;
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::voice::{fail, ok};
use crate::App;
pub use crate::voice::LAST_BOT;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Action {
    Voice,
    Snap,
    Edit,
}

impl Action {
    pub const ALL: [Action; 3] = [Action::Voice, Action::Snap, Action::Edit];

    pub fn key(self) -> &'static str {
        match self {
            Action::Voice => "voice",
            Action::Snap => "snap",
            Action::Edit => "edit",
        }
    }

    fn id(self) -> i32 {
        match self {
            Action::Voice => 1,
            Action::Snap => 2,
            Action::Edit => 3,
        }
    }

    fn from_id(id: i32) -> Option<Self> {
        Self::ALL.into_iter().find(|a| a.id() == id)
    }

    fn from_key(key: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|a| a.key() == key)
    }
}

// ---------------------------------------------------------------------------
// Настройки
// ---------------------------------------------------------------------------

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Binding {
    pub enabled: bool,
    pub keys: String,
}

impl Default for Binding {
    fn default() -> Self {
        Self { enabled: true, keys: String::new() }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Config {
    pub voice: Binding,
    pub snap: Binding,
    pub edit: Binding,
    /// Распознанный текст сразу уходит боту, без проверки в поле ввода.
    pub send_now: bool,
    /// «Голос уходит»: id бота или `last` — последнему, с кем говорили.
    pub voice_target: String,
}

/// Ключ в настройках (`state.json`).
const SETTINGS_KEY: &str = "hotkeys";

impl Default for Config {
    fn default() -> Self {
        let on = |keys: &str| Binding { enabled: true, keys: keys.to_string() };
        Self { voice: on("Ctrl + Space"), snap: on("Alt + S"), edit: on("Alt + E"), send_now: false, voice_target: LAST_BOT.to_string() }
    }
}

impl Config {
    pub fn binding(&self, action: Action) -> &Binding {
        match action {
            Action::Voice => &self.voice,
            Action::Snap => &self.snap,
            Action::Edit => &self.edit,
        }
    }

    fn binding_mut(&mut self, action: Action) -> &mut Binding {
        match action {
            Action::Voice => &mut self.voice,
            Action::Snap => &mut self.snap,
            Action::Edit => &mut self.edit,
        }
    }

    /// Из настроек программы. Пустое или испорченное сочетание — как по умолчанию.
    pub fn from_settings(value: Option<&Value>) -> Self {
        let mut config = value.and_then(|v| serde_json::from_value::<Config>(v.clone()).ok()).unwrap_or_default();
        let defaults = Config::default();
        if config.voice_target.trim().is_empty() {
            config.voice_target = LAST_BOT.to_string();
        }
        for action in Action::ALL {
            let binding = config.binding_mut(action);
            match validate(&binding.keys) {
                Ok(combo) => binding.keys = combo.to_string(),
                Err(_) => binding.keys = defaults.binding(action).keys.clone(),
            }
        }
        config
    }

    /// Правка из окна настроек: только известные поля, сочетание проверено и приведено к одному виду.
    pub fn patch(&mut self, patch: &Value) -> Result<(), String> {
        for action in Action::ALL {
            let Some(part) = patch.get(action.key()) else { continue };
            if let Some(on) = part.get("enabled").and_then(Value::as_bool) {
                self.binding_mut(action).enabled = on;
            }
            if let Some(keys) = part.get("keys").and_then(Value::as_str) {
                self.binding_mut(action).keys = validate(keys)?.to_string();
            }
        }
        if let Some(on) = patch.get("sendNow").and_then(Value::as_bool) {
            self.send_now = on;
        }
        if let Some(target) = patch.get("voiceTarget").and_then(Value::as_str) {
            let target = target.trim();
            // Есть ли такой бот, проверяет тот, кто знает ботов (put_hotkeys); здесь — только форма.
            if target.is_empty() || target.len() > 100 || target.chars().any(char::is_control) {
                return Err("Выбери бота, которому уходит голос".to_string());
            }
            self.voice_target = target.to_string();
        }
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Сочетание клавиш
// ---------------------------------------------------------------------------

pub const MOD_ALT: u32 = 0x1;
pub const MOD_CONTROL: u32 = 0x2;
pub const MOD_SHIFT: u32 = 0x4;
pub const MOD_WIN: u32 = 0x8;
const MOD_NOREPEAT: u32 = 0x4000;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Combo {
    pub mods: u32,
    pub vk: u32,
}

const VK_TAB: u32 = 0x09;
const VK_SPACE: u32 = 0x20;
const VK_DELETE: u32 = 0x2E;
const VK_F4: u32 = 0x73;
const VK_F12: u32 = 0x7B;
const VK_F13: u32 = 0x7C;

const NAMED: &[(&str, u32)] = &[
    ("Space", VK_SPACE),
    ("Tab", VK_TAB),
    ("PageUp", 0x21),
    ("PageDown", 0x22),
    ("End", 0x23),
    ("Home", 0x24),
    ("Insert", 0x2D),
    ("Delete", VK_DELETE),
    ("Del", VK_DELETE),
    ("Pause", 0x13),
];

fn is_function_key(vk: u32) -> bool {
    (0x70..=0x87).contains(&vk)
}

fn key_name(vk: u32) -> String {
    match vk {
        0x30..=0x39 | 0x41..=0x5A => char::from_u32(vk).map(String::from).unwrap_or_default(),
        _ if is_function_key(vk) => format!("F{}", vk - 0x70 + 1),
        _ => NAMED.iter().find(|(_, code)| *code == vk).map(|(name, _)| name.to_string()).unwrap_or_else(|| format!("0x{vk:02X}")),
    }
}

fn key_code(name: &str) -> Option<u32> {
    let upper = name.to_ascii_uppercase();
    if upper.len() == 1 {
        let c = upper.as_bytes()[0];
        if c.is_ascii_uppercase() || c.is_ascii_digit() {
            return Some(u32::from(c));
        }
    }
    if let Some(n) = upper.strip_prefix('F').and_then(|n| n.parse::<u32>().ok()) {
        if (1..=24).contains(&n) {
            return Some(0x70 + n - 1);
        }
    }
    NAMED.iter().find(|(known, _)| known.eq_ignore_ascii_case(name)).map(|(_, code)| *code)
}

/// `Ctrl + Space`, `ctrl+space`, `Alt+S`, `F9` → модификаторы и код клавиши. Только разбор.
pub fn parse(text: &str) -> Result<Combo, String> {
    let mut mods = 0;
    let mut vk = None;
    for part in text.split('+').map(str::trim) {
        if part.is_empty() {
            return Err(format!("Непонятное сочетание: {text}"));
        }
        let flag = match part.to_ascii_lowercase().as_str() {
            "ctrl" | "control" => Some(MOD_CONTROL),
            "alt" => Some(MOD_ALT),
            "shift" => Some(MOD_SHIFT),
            "win" | "super" | "meta" => Some(MOD_WIN),
            _ => None,
        };
        match flag {
            Some(flag) => mods |= flag,
            None if vk.is_none() => vk = Some(key_code(part).ok_or_else(|| format!("Неизвестная клавиша: {part}"))?),
            None => return Err(format!("В сочетании две обычные клавиши: {text}")),
        }
    }
    let vk = vk.ok_or_else(|| format!("В сочетании нет обычной клавиши: {text}"))?;
    Ok(Combo { mods, vk })
}

/// Сочетания, которые нельзя забирать: системные (Alt+Tab, Alt+F4, Ctrl+Alt+Del, Win+L)
/// и правка текста, которой человек пользуется каждую минуту.
const RESERVED: &[(u32, u32)] = &[
    (MOD_ALT, VK_TAB),
    (MOD_ALT, VK_F4),
    // Alt+Space открывает системное меню окна.
    (MOD_ALT, VK_SPACE),
    (MOD_CONTROL | MOD_ALT, VK_DELETE),
    (MOD_WIN, b'L' as u32),
    (MOD_CONTROL, b'C' as u32),
    (MOD_CONTROL, b'V' as u32),
    (MOD_CONTROL, b'X' as u32),
    (MOD_CONTROL, b'Z' as u32),
    (MOD_CONTROL, b'A' as u32),
    (MOD_CONTROL, b'S' as u32),
    (MOD_CONTROL, b'Q' as u32),
    (MOD_CONTROL, b'W' as u32),
];

/// Разбор и проверка: модификатор и клавиша либо одна из F13–F24. F1–F12 без модификатора — это
/// обычные команды программ (F1 справка, F5 обновить), а Shift без Ctrl, Alt или Win с буквой —
/// просто заглавная буква: такое отняло бы у человека ввод.
pub fn validate(text: &str) -> Result<Combo, String> {
    let combo = parse(text)?;
    if combo.vk == VK_F12 {
        return Err("F12 Windows держит для отладчика: выбери другую клавишу".to_string());
    }
    let strong = combo.mods & (MOD_CONTROL | MOD_ALT | MOD_WIN) != 0;
    if !strong && !(combo.mods == 0 && combo.vk >= VK_F13 && is_function_key(combo.vk)) {
        return Err("Нужна клавиша с Ctrl, Alt или Win либо одна из F13–F24".to_string());
    }
    if RESERVED.contains(&(combo.mods, combo.vk)) {
        return Err(format!("{combo} нужно Windows и программам, его не забираем. Выбери другое"));
    }
    Ok(combo)
}

impl std::fmt::Display for Combo {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let mut parts = Vec::new();
        for (flag, name) in [(MOD_CONTROL, "Ctrl"), (MOD_ALT, "Alt"), (MOD_SHIFT, "Shift"), (MOD_WIN, "Win")] {
            if self.mods & flag != 0 {
                parts.push(name.to_string());
            }
        }
        parts.push(key_name(self.vk));
        f.write_str(&parts.join(" + "))
    }
}

// ---------------------------------------------------------------------------
// Регистрация
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Status {
    /// Сочетание работает.
    Ok,
    /// Его уже держит Windows, другая программа или другое действие Deloist.
    Taken,
    /// Сочетание не разобралось.
    Invalid,
    /// Действие выключено.
    Off,
    /// Окна программы нет (режим `--server`): горячих клавиш тоже нет.
    Unavailable,
}

/// Кто на самом деле регистрирует сочетания: Windows или проверка.
pub trait Registrar {
    fn register(&mut self, id: i32, combo: Combo) -> bool;
    fn unregister(&mut self, id: i32);
}

/// Снять все сочетания и поставить заново по настройкам. Неудача одного не мешает остальным.
pub fn apply(config: &Config, registrar: &mut impl Registrar) -> BTreeMap<Action, Status> {
    let mut out = BTreeMap::new();
    for action in Action::ALL {
        registrar.unregister(action.id());
    }
    for action in Action::ALL {
        let binding = config.binding(action);
        let status = if !binding.enabled {
            Status::Off
        } else {
            match validate(&binding.keys) {
                Err(_) => Status::Invalid,
                Ok(combo) if registrar.register(action.id(), combo) => Status::Ok,
                Ok(_) => Status::Taken,
            }
        };
        out.insert(action, status);
    }
    out
}

/// Правка сочетаний применилась, но какое-то новое оказалось занято: возвращаем ему прежнее.
/// Возвращает действия, которые пришлось откатить.
pub fn keep_working(before: &Config, after: &mut Config, status: &BTreeMap<Action, Status>) -> Vec<Action> {
    let mut reverted = Vec::new();
    for action in Action::ALL {
        let changed = before.binding(action).keys != after.binding(action).keys;
        if changed && status.get(&action) == Some(&Status::Taken) {
            after.binding_mut(action).keys = before.binding(action).keys.clone();
            reverted.push(action);
        }
    }
    reverted
}

// ---------------------------------------------------------------------------
// Состояние процесса
// ---------------------------------------------------------------------------

struct Hotkeys {
    /// Копия настроек для потока окна: читать `state.json` на каждое нажатие незачем.
    config: Mutex<Config>,
    /// Итог последней регистрации и её номер: настройки ждут, пока поток окна применит правку.
    status: Mutex<(u64, BTreeMap<Action, Status>)>,
    applied: Condvar,
    /// Окно сообщений потока горячих клавиш; 0 — потока нет.
    hwnd: AtomicIsize,
}

static STATE: OnceLock<Hotkeys> = OnceLock::new();
/// Правки сочетаний по одной: две одновременные откатывали бы друг друга.
static EDITING: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

fn state(app: &App) -> &'static Hotkeys {
    STATE.get_or_init(|| {
        let config = Config::from_settings(app.store.settings().extra.get(SETTINGS_KEY));
        let status = Action::ALL.into_iter().map(|a| (a, Status::Unavailable)).collect();
        Hotkeys { config: Mutex::new(config), status: Mutex::new((0, status)), applied: Condvar::new(), hwnd: AtomicIsize::new(0) }
    })
}

/// Одно действие за раз: второе нажатие, пока открыт экран выбора или подсказка, ничего не делает.
pub struct Busy(&'static AtomicBool);

impl Busy {
    pub fn take(flag: &'static AtomicBool) -> Option<Self> {
        (!flag.swap(true, Ordering::SeqCst)).then_some(Self(flag))
    }
}

impl Drop for Busy {
    fn drop(&mut self) {
        self.0.store(false, Ordering::SeqCst);
    }
}

/// Запуск в оконном режиме: поток с окном сообщений, регистрация, разбор нажатий.
pub fn start(app: Arc<App>, runtime: tokio::runtime::Handle) {
    let hotkeys = state(&app);
    let spawned = std::thread::Builder::new().name("hotkeys".into()).spawn(move || win::run(hotkeys, app, runtime));
    if let Err(e) = spawned {
        eprintln!("[hotkeys] не запустил поток: {e}");
    }
}

fn record(hotkeys: &Hotkeys, status: BTreeMap<Action, Status>) {
    let mut guard = hotkeys.status.lock().unwrap();
    guard.0 += 1;
    guard.1 = status;
    hotkeys.applied.notify_all();
}

/// Нажали сочетание. Поток окна сообщений не ждёт: всё долгое уходит в свои потоки.
fn fire(app: &Arc<App>, runtime: &tokio::runtime::Handle, action: Action, mods: u32) {
    // Отпущенный после Alt+E одиночный Alt открыл бы меню программы, а Win — меню «Пуск».
    if mods & (MOD_ALT | MOD_WIN) != 0 {
        win::mask_menu();
    }
    // Право взять фокус программа отдаёт адресно: доку или подсказке, когда знает их процесс.
    match action {
        Action::Voice => {
            let app = app.clone();
            runtime.spawn(async move { crate::voice::on_hotkey(&app).await });
        }
        Action::Snap => {
            let (app, runtime) = (app.clone(), runtime.clone());
            let _ = std::thread::Builder::new().name("snap".into()).spawn(move || crate::capture::on_hotkey(app, runtime));
        }
        Action::Edit => {
            let (app, runtime) = (app.clone(), runtime.clone());
            let _ = std::thread::Builder::new().name("edit".into()).spawn(move || crate::selection::on_hotkey(app, runtime));
        }
    }
}

/// Программа только что получила нажатие и может отдать фокус окну другого своего процесса.
pub fn allow_foreground(pid: u32) {
    win::allow_foreground(pid);
}

// ---------------------------------------------------------------------------
// API настроек
// ---------------------------------------------------------------------------

fn snapshot(hotkeys: &Hotkeys, rejected: &[Action]) -> Value {
    let config = hotkeys.config.lock().unwrap().clone();
    let status = hotkeys.status.lock().unwrap().1.clone();
    let mut out = serde_json::to_value(&config).unwrap_or_else(|_| json!({}));
    out["status"] = json!(status.iter().map(|(a, s)| (a.key(), *s)).collect::<BTreeMap<_, _>>());
    out["defaults"] = serde_json::to_value(Config::default()).unwrap_or_default();
    // Новое сочетание было занято, осталось прежнее: окно скажет это рядом с полем.
    out["rejected"] = json!(rejected.iter().map(|a| (a.key(), "taken")).collect::<BTreeMap<_, _>>());
    out
}

async fn save(app: &App, hotkeys: &Hotkeys, config: Config) -> Result<(), String> {
    let value = serde_json::to_value(&config).map_err(|e| e.to_string())?;
    app.store.set_setting_extra(SETTINGS_KEY, value).await.map_err(|e| format!("Не сохранил настройки: {e}"))?;
    *hotkeys.config.lock().unwrap() = config;
    Ok(())
}

/// Попросить поток окна перерегистрировать сочетания и дождаться итога. `None` — потока нет.
async fn reregister(hotkeys: &'static Hotkeys) -> Option<BTreeMap<Action, Status>> {
    let seen = hotkeys.status.lock().unwrap().0;
    if !win::reapply(hotkeys) {
        return None;
    }
    // Поток окна применит правку за миллисекунды; дольше секунды не ждём.
    tokio::task::spawn_blocking(move || {
        let guard = hotkeys.status.lock().unwrap();
        let (guard, _) = hotkeys.applied.wait_timeout_while(guard, Duration::from_secs(1), |s| s.0 == seen).unwrap();
        guard.1.clone()
    })
    .await
    .ok()
}

async fn get_hotkeys(State(app): State<Arc<App>>) -> Response {
    ok(snapshot(state(&app), &[]))
}

async fn put_hotkeys(State(app): State<Arc<App>>, Json(patch): Json<Value>) -> Response {
    let _one_at_a_time = EDITING.lock().await;
    let hotkeys = state(&app);
    let before = hotkeys.config.lock().unwrap().clone();
    let mut next = before.clone();
    if let Err(message) = next.patch(&patch) {
        return fail(400, message);
    }
    if next.voice_target != before.voice_target && next.voice_target != LAST_BOT && app.store.bot(&next.voice_target).is_none() {
        return fail(400, "Такого бота нет");
    }
    if let Err(message) = save(&app, hotkeys, next.clone()).await {
        return fail(500, message);
    }
    let mut rejected = Vec::new();
    if let Some(status) = reregister(hotkeys).await {
        rejected = keep_working(&before, &mut next, &status);
        if !rejected.is_empty() {
            if let Err(message) = save(&app, hotkeys, next).await {
                return fail(500, message);
            }
            reregister(hotkeys).await;
        }
    }
    ok(snapshot(hotkeys, &rejected))
}

/// Проверки: нажать сочетание без Windows. Только отладочная сборка с `CREW_TEST_HOTKEYS=1`.
pub fn test_hooks() -> bool {
    cfg!(debug_assertions) && std::env::var("CREW_TEST_HOTKEYS").is_ok_and(|v| v == "1")
}

async fn test_fire(State(app): State<Arc<App>>, Json(body): Json<Value>) -> Response {
    if !test_hooks() {
        return fail(404, "не найдено");
    }
    let action = body.get("action").and_then(Value::as_str).and_then(Action::from_key);
    match action {
        Some(Action::Voice) => {
            crate::voice::on_hotkey(&app).await;
            ok(json!({ "fired": "voice" }))
        }
        Some(Action::Snap) => match crate::capture::test_snap(&app).await {
            Ok(saved) => ok(json!({ "fired": "snap", "path": saved["path"], "attachment": saved })),
            Err(message) => fail(500, message),
        },
        Some(Action::Edit) => {
            let text = body.get("text").and_then(Value::as_str).unwrap_or_default();
            match crate::selection::test_begin(text, body.get("pasteFails").and_then(Value::as_str)) {
                Ok(id) => ok(json!({ "fired": "edit", "id": id })),
                Err(message) => fail(400, message),
            }
        }
        None => fail(400, "нет такого действия"),
    }
}

pub fn routes() -> Router<Arc<App>> {
    Router::new()
        .route("/api/hotkeys", get(get_hotkeys).put(put_hotkeys))
        .route("/api/hotkeys/test/fire", post(test_fire))
}

// ---------------------------------------------------------------------------
// Windows
// ---------------------------------------------------------------------------

#[cfg(windows)]
mod win {
    use super::*;
    use windows_sys::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
    use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
    use windows_sys::Win32::UI::Input::KeyboardAndMouse::{RegisterHotKey, SendInput, UnregisterHotKey, INPUT, INPUT_0, INPUT_KEYBOARD, KEYBDINPUT, KEYEVENTF_KEYUP};
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        AllowSetForegroundWindow, CreateWindowExW, DefWindowProcW, DispatchMessageW, GetMessageW, KillTimer, PostMessageW, RegisterClassW, SetTimer, TranslateMessage, HWND_MESSAGE, MSG, WM_APP, WM_HOTKEY, WM_TIMER, WNDCLASSW,
    };

    const WM_REAPPLY: u32 = WM_APP + 41;
    /// После перезапуска с обновлением прежняя копия ещё держит сочетания: занятые пробуем снова.
    const RETRY_TIMER: usize = 1;
    const RETRY_EVERY_MS: u32 = 1000;
    const RETRIES: u32 = 5;

    struct System(HWND);

    impl Registrar for System {
        fn register(&mut self, id: i32, combo: Combo) -> bool {
            // MOD_NOREPEAT: зажатая клавиша не стреляет очередью.
            unsafe { RegisterHotKey(self.0, id, combo.mods | MOD_NOREPEAT, combo.vk) != 0 }
        }
        fn unregister(&mut self, id: i32) {
            unsafe {
                UnregisterHotKey(self.0, id);
            }
        }
    }

    unsafe extern "system" fn proc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
        unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) }
    }

    pub fn run(hotkeys: &'static Hotkeys, app: Arc<App>, runtime: tokio::runtime::Handle) {
        let class: Vec<u16> = "DeloistHotkeys\0".encode_utf16().collect();
        let hwnd = unsafe {
            let instance = GetModuleHandleW(std::ptr::null());
            let wc = WNDCLASSW { lpfnWndProc: Some(proc), hInstance: instance, lpszClassName: class.as_ptr(), ..std::mem::zeroed() };
            RegisterClassW(&wc);
            CreateWindowExW(0, class.as_ptr(), std::ptr::null(), 0, 0, 0, 0, 0, HWND_MESSAGE, std::ptr::null_mut(), instance, std::ptr::null())
        };
        if hwnd.is_null() {
            eprintln!("[hotkeys] не создал окно сообщений");
            return;
        }
        hotkeys.hwnd.store(hwnd as isize, Ordering::SeqCst);
        let mut system = System(hwnd);
        let config = hotkeys.config.lock().unwrap().clone();
        let status = apply(&config, &mut system);
        let mut retries = 0;
        if status.values().any(|s| *s == Status::Taken) {
            retries = RETRIES;
            unsafe { SetTimer(hwnd, RETRY_TIMER, RETRY_EVERY_MS, None) };
        }
        record(hotkeys, status);

        let mut msg: MSG = unsafe { std::mem::zeroed() };
        // Окно и регистрация живут до конца процесса: выход из программы закроет их сам.
        while unsafe { GetMessageW(&mut msg, std::ptr::null_mut(), 0, 0) } > 0 {
            match msg.message {
                WM_HOTKEY => {
                    if let Some(action) = Action::from_id(msg.wParam as i32) {
                        let enabled = hotkeys.config.lock().unwrap().binding(action).enabled;
                        if enabled {
                            // Младшее слово lParam — модификаторы нажатого сочетания.
                            fire(&app, &runtime, action, (msg.lParam & 0xFFFF) as u32);
                        }
                    }
                }
                WM_REAPPLY => {
                    let config = hotkeys.config.lock().unwrap().clone();
                    record(hotkeys, apply(&config, &mut system));
                }
                WM_TIMER if msg.wParam == RETRY_TIMER => {
                    let config = hotkeys.config.lock().unwrap().clone();
                    let status = apply(&config, &mut system);
                    retries = retries.saturating_sub(1);
                    let taken = status.values().any(|s| *s == Status::Taken);
                    if !taken || retries == 0 {
                        unsafe { KillTimer(hwnd, RETRY_TIMER) };
                        for (action, status) in &status {
                            if *status == Status::Taken {
                                eprintln!("[hotkeys] сочетание для «{}» занято другой программой", action.key());
                            }
                        }
                    }
                    record(hotkeys, status);
                }
                _ => unsafe {
                    TranslateMessage(&msg);
                    DispatchMessageW(&msg);
                },
            }
        }
    }

    /// Попросить поток окна перечитать настройки. `false` — потока нет, ждать нечего.
    pub fn reapply(hotkeys: &Hotkeys) -> bool {
        let hwnd = hotkeys.hwnd.load(Ordering::SeqCst);
        hwnd != 0 && unsafe { PostMessageW(hwnd as HWND, WM_REAPPLY, 0, 0) } != 0
    }

    /// Отдать право взять фокус одному процессу: доку или подсказке.
    pub fn allow_foreground(pid: u32) {
        unsafe {
            AllowSetForegroundWindow(pid);
        }
    }

    /// Нажать и отпустить неназначенную клавишу (0xE8), пока Alt или Win ещё зажаты: тогда Windows
    /// не считает их отпущенными «в одиночку» и не открывает меню. Так же делают AutoHotkey и PowerToys.
    pub fn mask_menu() {
        let key = |up: bool| INPUT {
            r#type: INPUT_KEYBOARD,
            Anonymous: INPUT_0 { ki: KEYBDINPUT { wVk: 0xE8, wScan: 0, dwFlags: if up { KEYEVENTF_KEYUP } else { 0 }, time: 0, dwExtraInfo: 0 } },
        };
        let inputs = [key(false), key(true)];
        unsafe {
            SendInput(inputs.len() as u32, inputs.as_ptr(), std::mem::size_of::<INPUT>() as i32);
        }
    }
}

#[cfg(not(windows))]
mod win {
    use super::*;
    pub fn run(_hotkeys: &'static Hotkeys, _app: Arc<App>, _runtime: tokio::runtime::Handle) {}
    pub fn reapply(_hotkeys: &Hotkeys) -> bool {
        false
    }
    pub fn allow_foreground(_pid: u32) {}
    pub fn mask_menu() {}
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn combos_parse_in_any_spelling_and_print_with_spaces() {
        let combo = validate("ctrl+space").unwrap();
        assert_eq!(combo, Combo { mods: MOD_CONTROL, vk: 0x20 });
        assert_eq!(combo.to_string(), "Ctrl + Space");
        assert_eq!(validate("Ctrl + Space").unwrap(), combo, "свой же вид читается обратно");
        assert_eq!(validate("alt+s").unwrap().to_string(), "Alt + S");
        assert_eq!(validate("Alt+Ctrl+s").unwrap().to_string(), "Ctrl + Alt + S");
        assert_eq!(validate("Win+Shift+F11").unwrap(), Combo { mods: MOD_WIN | MOD_SHIFT, vk: 0x7A });
        assert_eq!(validate("Control+7").unwrap().vk, u32::from(b'7'));
        assert_eq!(validate("Ctrl+PageDown").unwrap().to_string(), "Ctrl + PageDown");
    }

    #[test]
    fn only_f13_to_f24_go_bare_and_f12_is_never_taken() {
        assert_eq!(validate("F13").unwrap(), Combo { mods: 0, vk: 0x7C });
        assert_eq!(validate("f24").unwrap().to_string(), "F24");
        assert!(validate("F9").is_err(), "F1–F12 без модификатора — команды программ");
        assert!(validate("F1").is_err());
        assert!(validate("Shift+F2").is_err(), "Shift не делает F2 своей");
        assert_eq!(validate("Ctrl+F9").unwrap().to_string(), "Ctrl + F9");
        for f12 in ["F12", "Ctrl+F12", "Alt+Shift+F12"] {
            assert!(validate(f12).unwrap_err().contains("отладчика"), "{f12}");
        }
        assert!(validate("S").is_err(), "одна буква");
        assert!(validate("Shift+S").is_err(), "это заглавная буква");
        assert!(validate("Space").is_err());
        assert!(validate("Ctrl+Alt").is_err(), "нет обычной клавиши");
        assert!(validate("Ctrl+A+B").is_err(), "две обычные клавиши");
        assert!(validate("Ctrl+F25").is_err());
        assert!(validate("Ctrl+Хэ").is_err());
        assert!(validate("Ctrl++").is_err());
        assert!(validate("").is_err());
    }

    #[test]
    fn system_and_editing_combos_are_never_grabbed() {
        for combo in ["Alt+Tab", "Alt+F4", "Alt+Space", "Ctrl+Alt+Del", "Ctrl+Alt+Delete", "Win+L", "Ctrl+C", "Ctrl+V", "Ctrl+X", "Ctrl+Z", "Ctrl+A", "Ctrl+S", "Ctrl+Q", "Ctrl+W"] {
            let error = validate(combo).unwrap_err();
            assert!(error.contains("не забираем"), "{combo}: {error}");
            assert!(parse(combo).is_ok(), "{combo} разбирается, но не годится");
        }
        // Соседние сочетания с другим набором модификаторов — можно.
        assert!(validate("Ctrl+Shift+C").is_ok());
        assert!(validate("Alt+C").is_ok());
        assert!(validate("Win+Shift+L").is_ok());
    }

    #[test]
    fn defaults_are_short_two_key_combos() {
        let config = Config::default();
        assert_eq!(config.voice.keys, "Ctrl + Space");
        assert_eq!(config.snap.keys, "Alt + S");
        assert_eq!(config.edit.keys, "Alt + E");
        for action in Action::ALL {
            assert!(validate(&config.binding(action).keys).is_ok(), "{action:?}");
        }
    }

    #[derive(Default)]
    struct Fake {
        held: Vec<(u32, u32)>,
        mine: BTreeMap<i32, Combo>,
    }

    impl Registrar for Fake {
        fn register(&mut self, id: i32, combo: Combo) -> bool {
            let taken = self.held.contains(&(combo.mods, combo.vk)) || self.mine.values().any(|c| *c == combo);
            if !taken {
                self.mine.insert(id, combo);
            }
            !taken
        }
        fn unregister(&mut self, id: i32) {
            self.mine.remove(&id);
        }
    }

    #[test]
    fn a_taken_combo_is_reported_and_the_others_still_work() {
        // Другая программа держит Alt+S.
        let mut fake = Fake { held: vec![(MOD_ALT, u32::from(b'S'))], ..Fake::default() };
        let status = apply(&Config::default(), &mut fake);
        assert_eq!(status[&Action::Voice], Status::Ok);
        assert_eq!(status[&Action::Snap], Status::Taken);
        assert_eq!(status[&Action::Edit], Status::Ok);

        // Выключенное снимается, битое не регистрируется, совпавшее с соседним — свободно, пока сосед выключен.
        let mut config = Config::default();
        config.voice.enabled = false;
        config.snap.keys = "nonsense".into();
        config.edit.keys = "Ctrl + Space".into();
        let status = apply(&config, &mut fake);
        assert_eq!(status[&Action::Voice], Status::Off);
        assert_eq!(status[&Action::Snap], Status::Invalid);
        assert_eq!(status[&Action::Edit], Status::Ok);
        assert_eq!(fake.mine.len(), 1);

        config.voice.enabled = true;
        let status = apply(&config, &mut fake);
        assert_eq!(status[&Action::Voice], Status::Ok);
        assert_eq!(status[&Action::Edit], Status::Taken, "то же сочетание у голоса");
    }

    #[test]
    fn a_new_combo_that_turns_out_taken_gives_way_to_the_old_working_one() {
        let mut fake = Fake { held: vec![(MOD_CONTROL | MOD_SHIFT, u32::from(b'K'))], ..Fake::default() };
        let before = Config::default();
        apply(&before, &mut fake);
        let mut after = before.clone();
        after.patch(&json!({ "snap": { "keys": "ctrl+shift+k" }, "edit": { "keys": "Alt+R" } })).unwrap();
        let status = apply(&after, &mut fake);
        assert_eq!(status[&Action::Snap], Status::Taken);
        let reverted = keep_working(&before, &mut after, &status);
        assert_eq!(reverted, vec![Action::Snap]);
        assert_eq!(after.snap.keys, "Alt + S", "прежнее рабочее");
        assert_eq!(after.edit.keys, "Alt + R", "свободное новое остаётся");
        let status = apply(&after, &mut fake);
        assert!(status.values().all(|s| *s == Status::Ok), "{status:?}");
        // Занятое, но не менявшееся (другая программа взяла его раньше) не откатывается: откатывать некуда.
        let mut same = after.clone();
        let taken = BTreeMap::from([(Action::Voice, Status::Taken)]);
        assert!(keep_working(&after, &mut same, &taken).is_empty());
    }

    #[test]
    fn settings_patch_normalizes_and_rejects_bad_combos() {
        let mut config = Config::default();
        assert!(config.patch(&json!({ "voice": { "keys": "F9" } })).is_err(), "голая F9 не годится");
        config.patch(&json!({ "snap": { "keys": "alt+ctrl+x", "enabled": false }, "sendNow": true, "junk": 1 })).unwrap();
        assert_eq!(config.snap, Binding { enabled: false, keys: "Ctrl + Alt + X".into() });
        assert!(config.send_now);
        let before = config.clone();
        assert!(config.patch(&json!({ "voice": { "keys": "Q" } })).is_err());
        assert!(config.patch(&json!({ "voice": { "keys": "Ctrl+C" } })).is_err());
        assert_eq!(config.voice, before.voice);
    }

    #[test]
    fn settings_from_the_store_fall_back_to_defaults() {
        assert_eq!(Config::from_settings(None), Config::default());
        let stored = json!({ "voice": { "enabled": false }, "snap": { "keys": "Ctrl+C" }, "edit": { "keys": "f18" } });
        let config = Config::from_settings(Some(&stored));
        assert!(!config.voice.enabled);
        assert_eq!(config.voice.keys, "Ctrl + Space", "пустое — по умолчанию");
        assert_eq!(config.snap.keys, "Alt + S", "запрещённое — по умолчанию");
        assert_eq!(config.edit.keys, "F18");
        assert_eq!(Config::from_settings(Some(&json!("мусор"))), Config::default());
    }
}
