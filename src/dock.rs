//! Док агентов у края экрана: узкая плашка с аватарами ботов и кнопкой нового.
//! Как и питомцы, это отдельный процесс `deloist.exe --dock <слой>` со своим прозрачным окном:
//! главный цикл окна не трогаем, а WebView строится до запуска цикла.
//!
//! Окно стоит у края рабочей области во всю её высоту, а ловит мышь только там, где что-то нарисовано:
//! страница присылает прямоугольники дока и карточек, и окно обрезается по ним (SetWindowRgn).
//! Всё остальное окно пропускает клики к программам под ним. Поэтому плашку двигают, прячут
//! и показывают анимацией внутри страницы, а окно не дёргается.

use std::io::{BufRead, Write};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use tao::dpi::LogicalSize;
use tao::event::{Event, WindowEvent};
use tao::event_loop::{ControlFlow, EventLoopBuilder, EventLoopProxy};
use tao::platform::run_return::EventLoopExtRunReturn;
use tao::window::{Window, WindowBuilder};
use wry::WebViewBuilderExtWindows;
use wry::{WebContext, WebViewBuilder};

use crate::guard::Guard;
use crate::pets::{creation_turn, data_dir, watch_server, BROWSER_ARGS};

/// Ширина окна в логических пикселях: сам док, мини-чат 380 рядом с ним и запас под тень.
const WINDOW_WIDTH: f64 = 480.0;
/// Как часто сверять монитор, слой и то, не открыто ли главное окно. Чаще не нужно: это не анимация.
const TICK: Duration = Duration::from_secs(1);
const OPEN_ATTEMPTS: u32 = 5;
/// Клик по «+»: главное окно откроет создание бота. Ботов с таким id не бывает.
pub const NEW_BOT: &str = "+new";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(C)]
pub struct Rect {
    pub left: i32,
    pub top: i32,
    pub right: i32,
    pub bottom: i32,
}

impl Rect {
    fn width(&self) -> i32 {
        self.right - self.left
    }
    fn height(&self) -> i32 {
        self.bottom - self.top
    }
}

/// Монитор, как его видит Windows: имя (`\\.\DISPLAY1`), весь экран и рабочая область без панели задач.
#[derive(Clone, Debug, PartialEq)]
pub struct Screen {
    pub name: String,
    pub area: Rect,
    pub work: Rect,
    pub primary: bool,
}

/// Куда ставить док: выбранный монитор, а если его отключили — туда, где окно программы, иначе на основной.
fn pick_screen<'a>(screens: &'a [Screen], wanted: &str, window_screen: Option<&str>) -> Option<&'a Screen> {
    let by_name = |name: &str| screens.iter().find(|screen| screen.name == name);
    (!wanted.is_empty())
        .then(|| by_name(wanted))
        .flatten()
        .or_else(|| window_screen.and_then(by_name))
        .or_else(|| screens.iter().find(|screen| screen.primary))
        .or_else(|| screens.first())
}

/// Окно дока: во всю высоту рабочей области, прижато к выбранному краю.
fn frame(work: Rect, right: bool, width: i32) -> Rect {
    let width = width.min(work.width()).max(1);
    let left = if right { work.right - width } else { work.left };
    Rect { left, top: work.top, right: left + width, bottom: work.bottom }
}

/// Прямоугольники от страницы: `x,y,w,h;x,y,w,h` в CSS-пикселях. Пустые и битые отбрасываем,
/// а всё, что вылезло за окно, обрезаем: окно и так не нарисует ничего за своими краями.
fn parse_region(text: &str, scale: f64, size: (i32, i32)) -> Vec<Rect> {
    text.split(';')
        .filter_map(|part| {
            let numbers: Vec<f64> = part.split(',').map(|n| n.trim().parse::<f64>()).collect::<Result<_, _>>().ok()?;
            let [x, y, w, h] = numbers[..] else { return None };
            if !(x.is_finite() && y.is_finite() && w.is_finite() && h.is_finite()) || w <= 0.0 || h <= 0.0 {
                return None;
            }
            let rect = Rect {
                left: ((x * scale).floor() as i32).clamp(0, size.0),
                top: ((y * scale).floor() as i32).clamp(0, size.1),
                right: (((x + w) * scale).ceil() as i32).clamp(0, size.0),
                bottom: (((y + h) * scale).ceil() as i32).clamp(0, size.1),
            };
            (rect.width() > 0 && rect.height() > 0).then_some(rect)
        })
        .take(32)
        .collect()
}

/// Точка `x,y` в CSS-пикселях от страницы.
fn parse_point(text: &str) -> Option<(f64, f64)> {
    let (x, y) = text.split_once(',')?;
    let (x, y) = (x.trim().parse::<f64>().ok()?, y.trim().parse::<f64>().ok()?);
    (x.is_finite() && y.is_finite()).then_some((x, y))
}

/// Док отпустили в точке `x` экрана: к какой стене он ближе.
fn nearer_right(work: Rect, x: f64) -> bool {
    x * 2.0 >= f64::from(work.left + work.right)
}

/// Настройки, которые решают, где окно. Страница присылает их при каждой перемене состояния.
#[derive(Clone, Debug, PartialEq)]
struct Place {
    top: bool,
    right: bool,
    display: String,
}

impl Place {
    fn from_args(layer: &str, edge: Option<&str>, display: Option<&str>) -> Self {
        Self {
            top: layer != "normal",
            right: edge != Some("left"),
            display: display.filter(|name| *name != "window").unwrap_or_default().to_string(),
        }
    }

    fn from_config(text: &str) -> Option<Self> {
        let value: serde_json::Value = serde_json::from_str(text).ok()?;
        Some(Self {
            top: value.get("layer").and_then(|v| v.as_str()) != Some("normal"),
            right: value.get("edge").and_then(|v| v.as_str()) != Some("left"),
            display: value.get("display").and_then(|v| v.as_str()).unwrap_or_default().to_string(),
        })
    }
}

// ---------------------------------------------------------------------------
// Главный процесс: запустить, остановить
// ---------------------------------------------------------------------------

struct Running {
    child: Child,
    /// Держим открытым: когда главная программа закроется, поток оборвётся и док уйдёт сам.
    _stdin: ChildStdin,
}

#[derive(Default)]
pub struct Dock {
    running: Mutex<Option<Running>>,
    origin: Mutex<Option<String>>,
    guard: Mutex<Option<Arc<Guard>>>,
}

impl Dock {
    /// Адрес и сторож есть только в оконном режиме: в `--server` дока не бывает.
    pub fn attach(&self, origin: String, guard: Arc<Guard>) {
        *self.origin.lock().unwrap() = Some(origin);
        *self.guard.lock().unwrap() = Some(guard);
    }

    /// Включён — процесс должен жить, выключен — его нет. Прочие настройки док берёт сам со страницы.
    pub fn sync(&self, settings: &crate::store::Settings, data_dir: &std::path::Path) {
        let origin = self.origin.lock().unwrap().clone();
        let guard = self.guard.lock().unwrap().clone();
        let (Some(origin), Some(guard)) = (origin, guard) else {
            return;
        };
        let mut running = self.running.lock().unwrap();
        if let Some(dock) = running.as_mut() {
            // Упавший процесс откроем заново.
            if !settings.dock_enabled || !matches!(dock.child.try_wait(), Ok(None)) {
                finish(dock);
                *running = None;
            }
        }
        if settings.dock_enabled && running.is_none() {
            match spawn(&origin, &guard, settings, data_dir) {
                Ok(dock) => *running = Some(dock),
                Err(message) => eprintln!("[dock] {message}"),
            }
        }
    }

    /// Процесс дока, если он запущен: горячая клавиша отдаёт ему право взять фокус.
    pub fn pid(&self) -> Option<u32> {
        self.running.lock().unwrap().as_ref().map(|dock| dock.child.id())
    }

    pub fn stop(&self) {
        if let Some(mut dock) = self.running.lock().unwrap().take() {
            finish(&mut dock);
        }
    }
}

impl Drop for Dock {
    fn drop(&mut self) {
        self.stop();
    }
}

fn finish(dock: &mut Running) {
    let _ = dock.child.kill();
    let _ = dock.child.wait();
}

pub fn sync_with_store(app: &crate::App) {
    app.dock.sync(&app.store.settings(), app.store.data_dir());
}

fn spawn(origin: &str, guard: &Guard, settings: &crate::store::Settings, data_dir: &std::path::Path) -> Result<Running, String> {
    let exe = std::env::current_exe().map_err(|e| format!("не нашёл себя: {e}"))?;
    let display = if settings.dock_display.is_empty() { "window" } else { settings.dock_display.as_str() };
    let mut command = Command::new(exe);
    command
        .arg("--dock")
        .arg(&settings.dock_layer)
        .arg("--edge")
        .arg(&settings.dock_edge)
        .arg("--display")
        .arg(display)
        .env("CREW_DATA_DIR", data_dir)
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(std::fs::File::options().create(true).append(true).open(data_dir.join("dock.log")).map_or_else(|_| Stdio::null(), Stdio::from));
    // Профиль WebView общий с питомцами, а с ним и браузер: отладочный порт тот же.
    if cfg!(debug_assertions) {
        if let Ok(port) = std::env::var("CREW_TEST_PETS_DEBUG_PORT") {
            command.env("WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS", format!("--remote-debugging-port={port} {BROWSER_ARGS}"));
        }
    }
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x0800_0000);
    }
    let mut child = command.spawn().map_err(|e| format!("не смог запустить док: {e}"))?;
    let mut stdin = child.stdin.take().ok_or("у процесса дока нет stdin")?;
    // Адрес одноразовый и только по stdin: аргументы видны другим программам.
    let entry = format!("{origin}/?boot={}&to=/dock.html", guard.issue_boot());
    stdin.write_all(format!("{entry}\n").as_bytes()).map_err(|e| e.to_string())?;
    stdin.flush().map_err(|e| e.to_string())?;
    Ok(Running { child, _stdin: stdin })
}

// ---------------------------------------------------------------------------
// Мониторы и окно: Windows
// ---------------------------------------------------------------------------

#[cfg(windows)]
mod win {
    use super::{Rect, Screen};

    #[repr(C)]
    struct MonitorInfoEx {
        size: u32,
        monitor: Rect,
        work: Rect,
        flags: u32,
        device: [u16; 32],
    }

    #[link(name = "user32")]
    extern "system" {
        fn EnumDisplayMonitors(hdc: isize, clip: *const Rect, callback: unsafe extern "system" fn(isize, isize, *mut Rect, isize) -> i32, data: isize) -> i32;
        fn GetMonitorInfoW(monitor: isize, info: *mut MonitorInfoEx) -> i32;
        fn MonitorFromWindow(hwnd: isize, flags: u32) -> isize;
        fn FindWindowW(class: *const u16, title: *const u16) -> *mut core::ffi::c_void;
        fn GetForegroundWindow() -> isize;
        fn SetWindowPos(hwnd: isize, after: isize, x: i32, y: i32, width: i32, height: i32, flags: u32) -> i32;
        fn GetWindowLongW(hwnd: isize, index: i32) -> i32;
        fn SetWindowLongW(hwnd: isize, index: i32, value: i32) -> i32;
        fn SetWindowRgn(hwnd: isize, region: isize, redraw: i32) -> i32;
        fn SetForegroundWindow(hwnd: *mut core::ffi::c_void) -> i32;
    }
    // Библиотеки импорта gdi32 у тулчейна gnullvm нет: связываемся с DLL напрямую, как windows-sys.
    #[link(name = "gdi32", kind = "raw-dylib")]
    extern "system" {
        fn CreateRectRgn(left: i32, top: i32, right: i32, bottom: i32) -> isize;
        fn CombineRgn(dest: isize, a: isize, b: isize, mode: i32) -> i32;
        fn DeleteObject(object: isize) -> i32;
    }

    fn info(monitor: isize) -> Option<Screen> {
        let mut info = MonitorInfoEx {
            size: std::mem::size_of::<MonitorInfoEx>() as u32,
            monitor: Rect { left: 0, top: 0, right: 0, bottom: 0 },
            work: Rect { left: 0, top: 0, right: 0, bottom: 0 },
            flags: 0,
            device: [0; 32],
        };
        // Структура живёт до конца вызова, размер указан.
        if unsafe { GetMonitorInfoW(monitor, &mut info) } == 0 {
            return None;
        }
        let end = info.device.iter().position(|&c| c == 0).unwrap_or(info.device.len());
        Some(Screen {
            name: String::from_utf16_lossy(&info.device[..end]),
            area: info.monitor,
            work: info.work,
            // MONITORINFOF_PRIMARY = 1.
            primary: info.flags & 1 != 0,
        })
    }

    unsafe extern "system" fn collect(monitor: isize, _hdc: isize, _rect: *mut Rect, data: isize) -> i32 {
        let list = unsafe { &mut *(data as *mut Vec<Screen>) };
        if let Some(screen) = info(monitor) {
            list.push(screen);
        }
        1
    }

    pub fn screens() -> Vec<Screen> {
        let mut list: Vec<Screen> = Vec::new();
        // Список живёт до конца вызова: Windows зовёт обработчик синхронно.
        unsafe {
            EnumDisplayMonitors(0, core::ptr::null(), collect, &mut list as *mut Vec<Screen> as isize);
        }
        list
    }

    fn main_window() -> isize {
        use std::os::windows::ffi::OsStrExt;
        let class: Vec<u16> = std::ffi::OsStr::new(crate::desktop::WINDOW_CLASS).encode_wide().chain(Some(0)).collect();
        // Только по классу: заголовок «Deloist» совпадает с приложением планировщика.
        unsafe { FindWindowW(class.as_ptr(), core::ptr::null()) as isize }
    }

    /// Монитор окна программы. Спрятанное в трей окно тоже где-то стоит: Windows помнит его место.
    pub fn main_window_screen() -> Option<String> {
        let hwnd = main_window();
        if hwnd == 0 {
            return None;
        }
        // MONITOR_DEFAULTTONEAREST = 2.
        info(unsafe { MonitorFromWindow(hwnd, 2) }).map(|screen| screen.name)
    }

    pub fn main_window_active() -> bool {
        let hwnd = main_window();
        hwnd != 0 && unsafe { GetForegroundWindow() } == hwnd
    }

    // SWP_NOZORDER | SWP_NOACTIVATE.
    const PLACE: u32 = 0x0004 | 0x0010;

    pub fn place(hwnd: isize, rect: Rect) {
        unsafe {
            SetWindowPos(hwnd, 0, rect.left, rect.top, rect.right - rect.left, rect.bottom - rect.top, PLACE);
        }
    }

    /// Только сдвинуть: док несут по экрану, размер тот же.
    pub fn move_to(hwnd: isize, x: i32, y: i32) {
        // SWP_NOSIZE.
        unsafe {
            SetWindowPos(hwnd, 0, x, y, 0, 0, PLACE | 0x0001);
        }
    }

    /// Мини-чат открыт: окно берёт фокус, чтобы в поле можно было печатать. Человек только что
    /// кликнул по доку, поэтому Windows отдаёт фокус без возражений.
    pub fn take_focus(hwnd: isize) {
        unsafe {
            SetForegroundWindow(hwnd as *mut core::ffi::c_void);
        }
    }

    const EX_STYLE: i32 = -20;
    const EX_TOPMOST: i32 = 0x8;
    const EX_TOOLWINDOW: i32 = 0x80;
    const EX_APPWINDOW: i32 = 0x4_0000;
    const EX_NOACTIVATE: i32 = 0x0800_0000;

    /// Вне Alt+Tab и без фокуса: клик по доку не должен уводить человека из программы, где он работает.
    /// Фокус окно берёт, только пока открыт мини-чат. tao заново собирает стиль при смене своих флагов,
    /// поэтому стиль сверяется на каждом такте.
    pub fn keep_style(hwnd: isize, focusable: bool) {
        unsafe {
            let style = GetWindowLongW(hwnd, EX_STYLE);
            let base = (style | EX_TOOLWINDOW) & !EX_APPWINDOW;
            let wanted = if focusable { base & !EX_NOACTIVATE } else { base | EX_NOACTIVATE };
            if wanted != style {
                SetWindowLongW(hwnd, EX_STYLE, wanted);
            }
        }
    }

    /// Слой: поверх всех или обычное окно. Отложенные вызовы tao приходят в своём порядке и могут
    /// перебить флаг, поэтому сначала смотрим, какой он сейчас, и правим только расхождение.
    pub fn keep_layer(hwnd: isize, top: bool) {
        // HWND_TOPMOST = -1, HWND_NOTOPMOST = -2; SWP_NOSIZE | SWP_NOMOVE | SWP_NOACTIVATE.
        unsafe {
            let now = GetWindowLongW(hwnd, EX_STYLE) & EX_TOPMOST != 0;
            if now != top {
                SetWindowPos(hwnd, if top { -1 } else { -2 }, 0, 0, 0, 0, 0x0001 | 0x0002 | 0x0010);
            }
        }
    }

    /// Окно ловит мышь и рисует только в этих прямоугольниках. Пустой список — пустая область.
    pub fn set_region(hwnd: isize, rects: &[Rect]) {
        unsafe {
            let region = CreateRectRgn(0, 0, 0, 0);
            if region == 0 {
                return;
            }
            for rect in rects {
                let part = CreateRectRgn(rect.left, rect.top, rect.right, rect.bottom);
                if part != 0 {
                    // RGN_OR = 2.
                    CombineRgn(region, region, part, 2);
                    DeleteObject(part);
                }
            }
            // После удачного вызова областью владеет система, удалять её нельзя.
            if SetWindowRgn(hwnd, region, 1) == 0 {
                DeleteObject(region);
            }
        }
    }
}

#[cfg(not(windows))]
mod win {
    use super::{Rect, Screen};
    pub fn screens() -> Vec<Screen> {
        let area = Rect { left: 0, top: 0, right: 1280, bottom: 800 };
        vec![Screen { name: "screen".into(), area, work: area, primary: true }]
    }
    pub fn main_window_screen() -> Option<String> {
        None
    }
    pub fn main_window_active() -> bool {
        false
    }
    pub fn place(_hwnd: isize, _rect: Rect) {}
    pub fn move_to(_hwnd: isize, _x: i32, _y: i32) {}
    pub fn take_focus(_hwnd: isize) {}
    pub fn keep_style(_hwnd: isize, _focusable: bool) {}
    pub fn keep_layer(_hwnd: isize, _top: bool) {}
    pub fn set_region(_hwnd: isize, _rects: &[Rect]) {}
}

/// Мониторы для выбора в настройках.
pub fn screens() -> Vec<Screen> {
    win::screens()
}

// ---------------------------------------------------------------------------
// Процесс дока
// ---------------------------------------------------------------------------

enum UserEvent {
    Ipc(String),
    Tick,
    Gone,
}

#[cfg(windows)]
fn hwnd_of(window: &Window) -> isize {
    use tao::platform::windows::WindowExtWindows;
    window.hwnd()
}

#[cfg(not(windows))]
fn hwnd_of(_window: &Window) -> isize {
    0
}

/// Масштаб монитора по имени: у tao он есть, у WinAPI без shcore нет.
fn scale_of(window: &Window, name: &str) -> f64 {
    window
        .available_monitors()
        .find(|monitor| monitor.name().as_deref() == Some(name))
        .map(|monitor| monitor.scale_factor())
        .unwrap_or(1.0)
}

/// Где окну стоять сейчас и с каким масштабом.
fn target(window: &Window, place: &Place) -> Option<(Rect, f64)> {
    let screens = win::screens();
    let window_screen = win::main_window_screen();
    let screen = pick_screen(&screens, &place.display, window_screen.as_deref())?;
    let scale = scale_of(window, &screen.name);
    Some((frame(screen.work, place.right, (WINDOW_WIDTH * scale).round() as i32), scale))
}

/// Точка входа процесса дока. Адрес входа читается из stdin одной строкой.
pub fn run(layer: &str, edge: Option<String>, display: Option<String>) -> Result<(), String> {
    let mut place = Place::from_args(layer, edge.as_deref(), display.as_deref());

    let mut reader = std::io::BufReader::new(std::io::stdin());
    let mut entry = String::new();
    reader.read_line(&mut entry).map_err(|e| format!("не прочитал адрес: {e}"))?;
    let entry = entry.trim().to_string();
    let url = reqwest::Url::parse(&entry).map_err(|e| format!("адрес не разобрался: {e}"))?;
    let origin = url.origin().ascii_serialization();

    let event_loop = EventLoopBuilder::<UserEvent>::with_user_event().build();
    let proxy = event_loop.create_proxy();
    watch_parent(proxy.clone(), reader);
    {
        let proxy = proxy.clone();
        watch_server(url, move || {
            let _ = proxy.send_event(UserEvent::Gone);
        });
    }
    tick(proxy.clone());

    let builder = WindowBuilder::new()
        .with_title("Deloist Dock")
        .with_decorations(false)
        .with_transparent(true)
        .with_resizable(false)
        .with_always_on_top(place.top)
        .with_focused(false)
        .with_visible(true)
        .with_inner_size(LogicalSize::new(WINDOW_WIDTH, 600.0));
    #[cfg(windows)]
    let builder = {
        use tao::platform::windows::WindowBuilderExtWindows;
        builder.with_skip_taskbar(true).with_undecorated_shadow(false)
    };
    let window = builder.build(&event_loop).map_err(|e| e.to_string())?;
    let hwnd = hwnd_of(&window);
    win::keep_style(hwnd, false);
    let (mut rect, mut scale) = target(&window, &place).unwrap_or((Rect { left: 0, top: 0, right: 420, bottom: 600 }, 1.0));
    win::place(hwnd, rect);
    // Пока страница не прислала свою область, окно ловит мышь только узкой полосой у края, где встанет док.
    let strip = (80.0 * scale) as i32;
    let first = if place.right { Rect { left: rect.width() - strip, top: 0, right: rect.width(), bottom: rect.height() } } else { Rect { left: 0, top: 0, right: strip, bottom: rect.height() } };
    win::set_region(hwnd, &[first]);

    let allowed = format!("{origin}/");
    let mut context = WebContext::new(Some(data_dir().join("webview-pets")));
    let creating = creation_turn();
    let mut attempt = 0;
    let webview = loop {
        attempt += 1;
        let ipc_proxy = proxy.clone();
        let allowed = allowed.clone();
        let built = WebViewBuilder::new_with_web_context(&mut context)
            .with_url(&entry)
            .with_additional_browser_args(BROWSER_ARGS)
            .with_transparent(true)
            .with_navigation_handler(move |target| target.starts_with(&allowed))
            // Ссылки из ответов бота открываются в браузере человека, а не новым окном внутри дока.
            .with_new_window_req_handler(|target, _| {
                crate::desktop::open_in_browser(&target);
                wry::NewWindowResponse::Deny
            })
            .with_ipc_handler(move |request: wry::http::Request<String>| {
                let _ = ipc_proxy.send_event(UserEvent::Ipc(request.body().clone()));
            })
            .with_devtools(false)
            .build(&window);
        match built {
            Ok(webview) => break webview,
            Err(e) if attempt >= OPEN_ATTEMPTS => return Err(format!("не удалось открыть окно дока: {e}")),
            Err(_) => std::thread::sleep(Duration::from_secs(1)),
        }
    };
    drop(creating);
    crate::voice::allow_microphone(&webview, &origin);

    let fit = |webview: &wry::WebView, rect: Rect| {
        let _ = webview.set_bounds(wry::Rect {
            position: wry::dpi::PhysicalPosition::new(0, 0).into(),
            size: wry::dpi::PhysicalSize::new(rect.width().max(1) as u32, rect.height().max(1) as u32).into(),
        });
    };
    fit(&webview, rect);

    let mut main_active = None;
    // Мини-чат открыт: окно может взять фокус.
    let mut focusable = false;
    // Док отлепили от стены и несут: где внутри окна его держат, в CSS-пикселях.
    let mut carrying: Option<(f64, f64)> = None;
    let mut event_loop = event_loop;
    event_loop.run_return(|event, _, control_flow| {
        *control_flow = ControlFlow::Wait;
        let mut relayout = false;
        match event {
            Event::UserEvent(UserEvent::Tick) => {
                relayout = true;
                let active = win::main_window_active();
                if main_active != Some(active) {
                    main_active = Some(active);
                    let _ = webview.evaluate_script(&format!("window.dockMainActive?.({active})"));
                }
            }
            Event::UserEvent(UserEvent::Ipc(message)) => {
                if let Some(config) = message.strip_prefix("config:") {
                    if let Some(next) = Place::from_config(config) {
                        relayout = next != place;
                        place = next;
                    }
                } else if let Some(region) = message.strip_prefix("region:") {
                    win::set_region(hwnd, &parse_region(region, scale, (rect.width(), rect.height())));
                } else if message == "open" {
                    crate::desktop::activate_running_window();
                } else if let Some(flag) = message.strip_prefix("focus:") {
                    focusable = flag == "1";
                    win::keep_style(hwnd, focusable);
                    if focusable {
                        win::take_focus(hwnd);
                        let _ = webview.focus();
                    }
                } else if let Some(grab) = message.strip_prefix("carry:").and_then(parse_point) {
                    carrying = Some(grab);
                } else if let Some((x, y)) = message.strip_prefix("move:").and_then(parse_point) {
                    if let Some((gx, gy)) = carrying {
                        win::move_to(hwnd, ((x - gx) * scale).round() as i32, ((y - gy) * scale).round() as i32);
                    }
                } else if let Some((x, y)) = message.strip_prefix("drop:").and_then(parse_point) {
                    carrying = None;
                    // Прилипает к ближайшей стене того экрана, где стоит док.
                    let screens = win::screens();
                    let window_screen = win::main_window_screen();
                    if let Some(screen) = pick_screen(&screens, &place.display, window_screen.as_deref()) {
                        place.right = nearer_right(screen.work, x * scale);
                    }
                    if let Some((next, next_scale)) = target(&window, &place) {
                        rect = next;
                        scale = next_scale;
                        win::place(hwnd, rect);
                        fit(&webview, rect);
                        let edge = if place.right { "right" } else { "left" };
                        let (left, top) = (x - f64::from(rect.left) / scale, y - f64::from(rect.top) / scale);
                        let _ = webview.evaluate_script(&format!("window.dockDropped?.('{edge}',{left},{top})"));
                    }
                }
            }
            Event::WindowEvent { event: WindowEvent::ScaleFactorChanged { .. }, .. } => relayout = true,
            Event::WindowEvent { event: WindowEvent::CloseRequested, .. } | Event::UserEvent(UserEvent::Gone) => {
                *control_flow = ControlFlow::Exit;
                return;
            }
            _ => {}
        }
        // Пока док несут, окно ведёт человек: сверять место нельзя, иначе оно прыгнет обратно к стене.
        if relayout && carrying.is_none() {
            win::keep_style(hwnd, focusable);
            win::keep_layer(hwnd, place.top);
            // Монитор отключили, окно программы переехало или Windows сама подвинула окно: ставим заново.
            if let Some((next, next_scale)) = target(&window, &place) {
                let actual = window.outer_position().ok().map(|p| (p.x, p.y)).zip(Some(window.outer_size()));
                let stands = actual.is_some_and(|((x, y), size)| x == next.left && y == next.top && size.width as i32 == next.width() && size.height as i32 == next.height());
                if next != rect || next_scale != scale || !stands {
                    rect = next;
                    scale = next_scale;
                    win::place(hwnd, rect);
                    fit(&webview, rect);
                    // Страница пересчитает свою область под новый размер и пришлёт её.
                    let _ = webview.evaluate_script("window.dockResized?.()");
                }
            }
        }
    });
    Ok(())
}

fn tick(proxy: EventLoopProxy<UserEvent>) {
    std::thread::spawn(move || loop {
        std::thread::sleep(TICK);
        if proxy.send_event(UserEvent::Tick).is_err() {
            return;
        }
    });
}

fn watch_parent(proxy: EventLoopProxy<UserEvent>, mut reader: impl BufRead + Send + 'static) {
    std::thread::spawn(move || {
        let mut line = String::new();
        // Главная программа ничего больше не пишет: конец потока значит, что её нет.
        while matches!(reader.read_line(&mut line), Ok(n) if n > 0) {
            line.clear();
        }
        let _ = proxy.send_event(UserEvent::Gone);
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn screen(name: &str, left: i32, primary: bool) -> Screen {
        let area = Rect { left, top: 0, right: left + 1920, bottom: 1080 };
        Screen { name: name.into(), area, work: Rect { bottom: 1040, ..area }, primary }
    }

    #[test]
    fn the_dock_goes_to_the_chosen_screen_or_falls_back_to_the_window() {
        let screens = [screen("A", 0, true), screen("B", 1920, false), screen("C", -1920, false)];
        assert_eq!(pick_screen(&screens, "B", Some("C")).unwrap().name, "B");
        assert_eq!(pick_screen(&screens, "", Some("C")).unwrap().name, "C", "вместе с окном программы");
        assert_eq!(pick_screen(&screens, "Z", Some("C")).unwrap().name, "C", "монитор отключили: туда, где окно");
        assert_eq!(pick_screen(&screens, "Z", None).unwrap().name, "A", "окна нет: на основной");
        assert_eq!(pick_screen(&screens, "", Some("Z")).unwrap().name, "A");
        assert!(pick_screen(&[], "", None).is_none());
    }

    #[test]
    fn the_window_hugs_the_chosen_edge_of_the_work_area() {
        let work = Rect { left: 1920, top: 0, right: 3840, bottom: 1040 };
        assert_eq!(frame(work, true, 480), Rect { left: 3360, top: 0, right: 3840, bottom: 1040 });
        assert_eq!(frame(work, false, 480), Rect { left: 1920, top: 0, right: 2400, bottom: 1040 });
        // Экран уже окна: окно не вылезает за него.
        let tiny = Rect { left: 0, top: 0, right: 300, bottom: 500 };
        assert_eq!(frame(tiny, true, 480), tiny);
    }

    #[test]
    fn the_region_is_scaled_clipped_and_cleaned() {
        let rects = parse_region("10,20,30,40;0,0,0,5;bad;-5,-5,10,10;400,0,50,50", 1.5, (630, 1000));
        assert_eq!(
            rects,
            vec![
                Rect { left: 15, top: 30, right: 60, bottom: 90 },
                Rect { left: 0, top: 0, right: 8, bottom: 8 },
                Rect { left: 600, top: 0, right: 630, bottom: 75 },
            ]
        );
        assert!(parse_region("", 1.0, (10, 10)).is_empty());
        assert!(parse_region("1,1,NaN,1", 1.0, (10, 10)).is_empty());
    }

    #[test]
    fn a_dropped_dock_sticks_to_the_nearer_wall() {
        let work = Rect { left: 0, top: 0, right: 1872, bottom: 1080 };
        assert!(nearer_right(work, 1500.0));
        assert!(!nearer_right(work, 300.0));
        assert!(nearer_right(work, 936.0), "ровно посередине — направо, где док по умолчанию");
        let second = Rect { left: 1920, top: 0, right: 3840, bottom: 1080 };
        assert!(!nearer_right(second, 2500.0));
        assert_eq!(parse_point("12.5, -3"), Some((12.5, -3.0)));
        assert_eq!(parse_point("1,NaN"), None);
        assert_eq!(parse_point("1"), None);
    }

    #[test]
    fn the_place_is_read_from_args_and_from_the_page() {
        let place = Place::from_args("normal", Some("left"), Some("window"));
        assert_eq!(place, Place { top: false, right: false, display: String::new() });
        let place = Place::from_args("top", None, Some(r"\\.\DISPLAY2"));
        assert_eq!(place, Place { top: true, right: true, display: r"\\.\DISPLAY2".into() });
        let place = Place::from_config(r#"{"layer":"top","edge":"left","display":"X"}"#).unwrap();
        assert_eq!(place, Place { top: true, right: false, display: "X".into() });
        assert!(Place::from_config("nope").is_none());
    }
}
