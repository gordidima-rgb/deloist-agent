//! Питомцы на рабочем столе: у каждого бота свой процесс `deloist.exe --pets <mode> --bot <id>` и своё окно.
//! Главный цикл окна не трогаем: в desktop.rs у tao много неочевидного, и WebView нельзя
//! создавать внутри обработчика. Поэтому новый питомец — новый процесс, а WebView строится до запуска цикла.

use std::collections::HashMap;
use std::io::{BufRead, Read, Write};
use std::path::PathBuf;
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use tao::dpi::{LogicalSize, PhysicalPosition};
use tao::event::{Event, WindowEvent};
use tao::event_loop::{ControlFlow, EventLoopBuilder, EventLoopProxy};
use tao::platform::run_return::EventLoopExtRunReturn;
use tao::window::{Window, WindowBuilder};
use wry::WebViewBuilderExtWindows;
use wry::{WebContext, WebViewBuilder};

use crate::guard::Guard;

/// Окно ровно по питомцу: аниме-персонаж 124 px и место над ним под значки.
/// Ширина 136, потому что уже Windows окно не сделает, а считать места надо по настоящему размеру.
const CELL: f64 = 136.0;
const CELL_HEIGHT: f64 = 140.0;
const MARGIN: f64 = 12.0;
/// Сколько питомцев можно вывести разом: у каждого свой процесс и своя страница.
pub const MAX_PETS: usize = 8;
const OPEN_EXPIRES: Duration = Duration::from_secs(15);
/// Сколько раз пробуем открыть WebView: профиль мог быть занят только что закрытым процессом.
const WEBVIEW_ATTEMPTS: u32 = 5;
/// Сколько ждать своей очереди на создание WebView: хватает на всех питомцев с большим запасом.
const CREATION_WAIT: Duration = Duration::from_secs(40);
/// Пауза перед закреплением слоя: столько нужно, чтобы отложенные вызовы tao успели пройти.
const LAYER_DELAY: Duration = Duration::from_millis(250);
/// Столько раз подряд слой должен подтвердиться.
const LAYER_CHECKS: u32 = 4;
/// Место записываем, когда окно замерло: при перетаскивании событий идут десятки в секунду.
const SAVE_DELAY: Duration = Duration::from_millis(400);
/// У каждого питомца своя страница, а отдельный процесс на каждую стоил бы слишком дорого: пусть делят один.
/// Первая часть — настройки wry по умолчанию, они при своём списке не добавляются сами.
pub(crate) const BROWSER_ARGS: &str = "--disable-features=msWebOOUI,msPdfOOUI,msSmartScreenProtection --renderer-process-limit=1 --autoplay-policy=no-user-gesture-required";

/// Как часто смотреть, где курсор. Окно получает событие, только когда курсор сдвинулся.
const CURSOR_POLL: Duration = Duration::from_millis(60);
/// Сдвиг меньше этого, в пикселях экрана, не новость: дрожание руки не будит окно.
const CURSOR_STEP: i32 = 3;
/// Неподвижный курсор повторяем так часто. Страница не считает повтор движением, и питомец всё равно задремлет.
const CURSOR_REPEAT: Duration = Duration::from_secs(2);

enum UserEvent {
    Ipc(String),
    /// Курсор в координатах экрана: страница видит мышь только над своим окном, а питомец следит по всему столу.
    Cursor(i32, i32),
    /// Новый режим по stdin: окно меняет слой на месте, без второго WebView на том же профиле.
    Mode(String),
    ServerGone,
    ParentGone,
}

/// Кого показать и где. Без места питомец встанет на первое свободное у правого нижнего угла.
pub struct PetSpec {
    pub id: String,
    pub at: Option<(i32, i32)>,
}

struct Running {
    child: Child,
    stdin: ChildStdin,
    /// Место у угла, которое он занял. Чтобы новый питомец не встал поверх.
    slot: Option<usize>,
}

/// Управление дочерними процессами из главного. Пока питомцы выключены, здесь нет ни окон,
/// ни WebView: только пустые поля.
pub struct Pets {
    running: Mutex<HashMap<String, Running>>,
    origin: Mutex<Option<String>>,
    guard: Mutex<Option<Arc<Guard>>>,
    mode: Mutex<String>,
    /// Клик по питомцу, пока главное окно спрятано в трей: оно заберёт просьбу при открытии.
    open: Mutex<Option<(String, Instant)>>,
}

impl Default for Pets {
    fn default() -> Self {
        Self {
            running: Mutex::new(HashMap::new()),
            origin: Mutex::new(None),
            guard: Mutex::new(None),
            mode: Mutex::new("off".into()),
            open: Mutex::new(None),
        }
    }
}

impl Pets {
    /// Адрес и сторож доступны только в оконном режиме: в `--server` питомцев не бывает.
    pub fn attach(&self, origin: String, guard: Arc<Guard>) {
        *self.origin.lock().unwrap() = Some(origin);
        *self.guard.lock().unwrap() = Some(guard);
    }

    pub fn request_open(&self, bot_id: &str) {
        *self.open.lock().unwrap() = Some((bot_id.to_string(), Instant::now()));
    }

    /// Просьба старше нескольких секунд уже не про этот клик.
    pub fn take_open(&self) -> Option<String> {
        let (bot_id, at) = self.open.lock().unwrap().take()?;
        (at.elapsed() < OPEN_EXPIRES).then_some(bot_id)
    }

    /// Привести окна в соответствие с выбором: лишних закрыть, новых открыть, остальным сообщить слой.
    /// Смена слоя идёт по stdin: перезапуск упёрся бы в профиль WebView, который процесс ещё держит.
    pub fn sync(&self, mode: &str, wanted: &[PetSpec], data_dir: &std::path::Path) {
        let mode = match mode {
            "desktop" | "top" => mode,
            _ => "off",
        };
        let origin = self.origin.lock().unwrap().clone();
        let guard = self.guard.lock().unwrap().clone();
        let (Some(origin), Some(guard)) = (origin, guard) else {
            return;
        };
        let mode_changed = {
            let mut current = self.mode.lock().unwrap();
            let changed = *current != mode;
            *current = mode.to_string();
            changed
        };
        let mut running = self.running.lock().unwrap();
        running.retain(|id, pet| {
            let keep = mode != "off" && wanted.iter().any(|spec| spec.id == *id);
            // Упавший процесс тоже убираем: ниже он откроется заново.
            if keep && matches!(pet.child.try_wait(), Ok(None)) {
                return true;
            }
            finish(pet);
            false
        });
        if mode == "off" {
            return;
        }
        if mode_changed {
            running.retain(|_, pet| {
                let sent = pet.stdin.write_all(format!("mode:{mode}\n").as_bytes()).and_then(|()| pet.stdin.flush());
                if sent.is_err() {
                    finish(pet);
                }
                sent.is_ok()
            });
        }
        for spec in wanted {
            if running.contains_key(&spec.id) {
                continue;
            }
            let slot = spec.at.is_none().then(|| first_free_slot(running.values().filter_map(|pet| pet.slot)));
            match spawn(&origin, &guard, mode, spec, slot, data_dir) {
                Ok(pet) => {
                    running.insert(spec.id.clone(), pet);
                }
                Err(message) => eprintln!("[pets] {message}"),
            }
        }
    }

    pub fn stop(&self) {
        for (_, mut pet) in self.running.lock().unwrap().drain() {
            finish(&mut pet);
        }
    }
}

impl Drop for Pets {
    fn drop(&mut self) {
        self.stop();
    }
}

fn finish(pet: &mut Running) {
    let _ = pet.child.kill();
    let _ = pet.child.wait();
}

fn first_free_slot(taken: impl Iterator<Item = usize>) -> usize {
    let taken: Vec<usize> = taken.collect();
    (0..).find(|slot| !taken.contains(slot)).unwrap_or(0)
}

/// Текущий выбор из настроек — в окна. Зовут при запуске и после любой правки выбора.
pub fn sync_with_store(app: &crate::App) {
    let wanted: Vec<PetSpec> = app
        .store
        .bots()
        .into_iter()
        .filter(|bot| bot.pet)
        .take(MAX_PETS)
        .map(|bot| PetSpec { at: bot.pet_x.zip(bot.pet_y), id: bot.id })
        .collect();
    app.pets.sync(&app.store.settings().pets_mode, &wanted, app.store.data_dir());
}

fn spawn(origin: &str, guard: &Guard, mode: &str, spec: &PetSpec, slot: Option<usize>, data_dir: &std::path::Path) -> Result<Running, String> {
    let exe = std::env::current_exe().map_err(|e| format!("не нашёл себя: {e}"))?;
    let mut command = Command::new(exe);
    command
        .arg("--pets")
        .arg(mode)
        .arg("--bot")
        .arg(&spec.id)
        .arg("--slot")
        .arg(slot.unwrap_or(0).to_string())
        .env("CREW_DATA_DIR", data_dir)
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        // Окна у него может не быть, и причину отказа иначе не узнать.
        .stderr(std::fs::File::options().create(true).append(true).open(data_dir.join("pets.log")).map_or_else(|_| Stdio::null(), Stdio::from));
    if let Some((x, y)) = spec.at {
        command.arg("--at").arg(format!("{x},{y}"));
    }
    // Отладочный порт главного окна занят им самим: у питомцев для проверок свой.
    if cfg!(debug_assertions) {
        if let Ok(port) = std::env::var("CREW_TEST_PETS_DEBUG_PORT") {
            command.env("WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS", format!("--remote-debugging-port={port} {BROWSER_ARGS}"));
        }
    }
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        // Без консольного окна: у debug-сборки оно есть.
        command.creation_flags(0x0800_0000);
    }
    let mut child = command.spawn().map_err(|e| format!("не смог запустить питомца: {e}"))?;
    let mut stdin = child.stdin.take().ok_or("у процесса питомца нет stdin")?;
    // Адрес одноразовый и только по stdin: аргументы видны другим программам.
    let entry = format!("{origin}/?boot={}&to=/pet.html", guard.issue_boot());
    stdin.write_all(format!("{entry}\n").as_bytes()).map_err(|e| e.to_string())?;
    stdin.flush().map_err(|e| e.to_string())?;
    Ok(Running { child, stdin, slot })
}

pub(crate) fn data_dir() -> PathBuf {
    if let Some(dir) = std::env::var_os("CREW_DATA_DIR") {
        return PathBuf::from(dir);
    }
    let exe = std::env::current_exe().unwrap_or_else(|_| PathBuf::from("deloist"));
    let dir = exe.parent().map(PathBuf::from).unwrap_or_else(|| PathBuf::from("."));
    dir.ancestors()
        .find(|p| p.join("Cargo.toml").exists())
        .map(|p| p.join("data"))
        .unwrap_or_else(|| dir.join("data"))
}

/// Рабочая область экрана: правый нижний угол считается от неё, а не от всего монитора.
#[cfg(windows)]
fn work_area() -> (i32, i32, i32, i32) {
    #[repr(C)]
    struct Rect {
        left: i32,
        top: i32,
        right: i32,
        bottom: i32,
    }
    #[link(name = "user32")]
    extern "system" {
        fn SystemParametersInfoW(action: u32, param: u32, data: *mut core::ffi::c_void, win_ini: u32) -> i32;
    }
    let mut rect = Rect { left: 0, top: 0, right: 1280, bottom: 800 };
    // SPI_GETWORKAREA = 0x0030.
    unsafe {
        SystemParametersInfoW(0x0030, 0, &mut rect as *mut Rect as *mut core::ffi::c_void, 0);
    }
    (rect.left, rect.top, rect.right, rect.bottom)
}

#[cfg(not(windows))]
fn work_area() -> (i32, i32, i32, i32) {
    (0, 0, 1280, 800)
}

#[cfg(windows)]
fn cursor_position() -> Option<(i32, i32)> {
    #[repr(C)]
    struct Point {
        x: i32,
        y: i32,
    }
    #[link(name = "user32")]
    extern "system" {
        fn GetCursorPos(point: *mut Point) -> i32;
    }
    let mut point = Point { x: 0, y: 0 };
    // На экране блокировки и в UAC курсора нет: вызов не удаётся, и мы просто молчим.
    (unsafe { GetCursorPos(&mut point) } != 0).then_some((point.x, point.y))
}

#[cfg(not(windows))]
fn cursor_position() -> Option<(i32, i32)> {
    None
}

/// Курсор сдвинулся настолько, чтобы сообщить окну.
fn cursor_moved(last: Option<(i32, i32)>, now: (i32, i32)) -> bool {
    last.is_none_or(|(x, y)| (now.0 - x).abs() >= CURSOR_STEP || (now.1 - y).abs() >= CURSOR_STEP)
}

/// Сдвиг курсора от середины окна: столько странице и нужно, чтобы наклониться в его сторону.
fn cursor_offset(cursor: (i32, i32), window: (i32, i32), size: (u32, u32)) -> (i32, i32) {
    (cursor.0 - (window.0 + size.0 as i32 / 2), cursor.1 - (window.1 + size.1 as i32 / 2))
}

fn watch_cursor(proxy: EventLoopProxy<UserEvent>) {
    std::thread::spawn(move || {
        let mut last = None;
        let mut sent = Instant::now();
        loop {
            std::thread::sleep(CURSOR_POLL);
            let Some(now) = cursor_position() else { continue };
            // Изредка повторяем и неподвижный курсор: страница могла загрузиться позже первого сообщения.
            if !cursor_moved(last, now) && sent.elapsed() < CURSOR_REPEAT {
                continue;
            }
            last = Some(now);
            sent = Instant::now();
            // Цикл окна закрылся: и следить больше не за кого.
            if proxy.send_event(UserEvent::Cursor(now.0, now.1)).is_err() {
                return;
            }
        }
    });
}

/// Закрепляет слой прямым вызовом и говорит, встал ли он. Отложенные вызовы tao приходят в своём порядке и могут
/// перекрыть результат, поэтому цикл окна спрашивает ещё раз, пока итог не подтвердится.
#[cfg(windows)]
fn apply_layer(window: &Window, top: bool) -> bool {
    use tao::platform::windows::WindowExtWindows;
    #[link(name = "user32")]
    extern "system" {
        fn SetWindowPos(hwnd: isize, after: isize, x: i32, y: i32, width: i32, height: i32, flags: u32) -> i32;
        fn GetWindowLongW(hwnd: isize, index: i32) -> i32;
    }
    // GWL_EXSTYLE и WS_EX_TOPMOST.
    const EX_STYLE: i32 = -20;
    const EX_TOPMOST: i32 = 0x8;
    const TOPMOST: isize = -1;
    const NOT_TOPMOST: isize = -2;
    const BOTTOM: isize = 1;
    // SWP_NOSIZE | SWP_NOMOVE | SWP_NOACTIVATE
    const KEEP: u32 = 0x0001 | 0x0002 | 0x0010;
    let hwnd = window.hwnd();
    unsafe {
        if top {
            SetWindowPos(hwnd, TOPMOST, 0, 0, 0, 0, KEEP);
        } else {
            SetWindowPos(hwnd, NOT_TOPMOST, 0, 0, 0, 0, KEEP);
            SetWindowPos(hwnd, BOTTOM, 0, 0, 0, 0, KEEP);
        }
        (GetWindowLongW(hwnd, EX_STYLE) & EX_TOPMOST != 0) == top
    }
}

#[cfg(not(windows))]
fn apply_layer(window: &Window, top: bool) -> bool {
    window.set_always_on_top(top);
    true
}

/// Окно не для переключения: иначе каждый питомец занимал бы место в Alt+Tab. tao заново собирает стиль
/// при смене своих флагов, поэтому зовём и после смены слоя.
#[cfg(windows)]
fn hide_from_alt_tab(window: &Window) {
    use tao::platform::windows::WindowExtWindows;
    #[link(name = "user32")]
    extern "system" {
        fn SetWindowLongW(hwnd: isize, index: i32, value: i32) -> i32;
        fn GetWindowLongW(hwnd: isize, index: i32) -> i32;
    }
    // GWL_EXSTYLE, WS_EX_TOOLWINDOW и WS_EX_APPWINDOW: второй возвращает окно в список, даже если есть первый.
    const EX_STYLE: i32 = -20;
    const EX_TOOLWINDOW: i32 = 0x80;
    const EX_APPWINDOW: i32 = 0x4_0000;
    let hwnd = window.hwnd();
    unsafe {
        SetWindowLongW(hwnd, EX_STYLE, (GetWindowLongW(hwnd, EX_STYLE) | EX_TOOLWINDOW) & !EX_APPWINDOW);
    }
}

#[cfg(not(windows))]
fn hide_from_alt_tab(_window: &Window) {}

/// Куда встать тому, кого ещё не двигали: ряд справа налево от правого нижнего угла, дальше следующий ряд выше.
fn default_spot(area: (i32, i32, i32, i32), size: (i32, i32), margin: i32, slot: usize) -> (i32, i32) {
    let (left, top, right, bottom) = area;
    let step_x = size.0 + margin;
    let step_y = size.1 + margin;
    let columns = ((right - left - margin) / step_x).max(1) as usize;
    let column = (slot % columns) as i32;
    let row = (slot / columns) as i32;
    let x = right - (column + 1) * step_x;
    let y = bottom - (row + 1) * step_y;
    (x.max(left), y.max(top))
}

/// Монитор могли отключить: питомец вне экранов потерялся бы навсегда, поэтому смотрим, где его середина.
fn on_screen(spot: (i32, i32), size: (i32, i32), monitors: &[(i32, i32, i32, i32)]) -> bool {
    let x = spot.0 + size.0 / 2;
    let y = spot.1 + size.1 / 2;
    monitors.iter().any(|&(left, top, width, height)| x >= left && y >= top && x < left + width && y < top + height)
}

fn parse_spot(text: &str) -> Option<(i32, i32)> {
    let (x, y) = text.split_once(',')?;
    Some((x.parse().ok()?, y.parse().ok()?))
}

/// Одна живая проверка: сервер ответил хоть чем-то. 403 тоже ответ, нам важно, что он есть.
fn alive(addrs: &[std::net::SocketAddr], host: &str) -> bool {
    for addr in addrs {
        let Ok(mut stream) = std::net::TcpStream::connect_timeout(addr, Duration::from_millis(700)) else {
            continue;
        };
        let _ = stream.set_read_timeout(Some(Duration::from_millis(700)));
        let _ = stream.set_write_timeout(Some(Duration::from_millis(700)));
        let request = format!("GET /api/health HTTP/1.1\r\nHost: {host}\r\nConnection: close\r\n\r\n");
        if stream.write_all(request.as_bytes()).is_err() {
            continue;
        }
        let mut head = [0u8; 16];
        if let Ok(n) = stream.read(&mut head) {
            if n > 0 && head.starts_with(b"HTTP/") {
                return true;
            }
        }
    }
    false
}

/// Сторож главной программы: три пропуска подряд значат, что её больше нет, и окну пора закрыться.
pub(crate) fn watch_server(url: reqwest::Url, gone: impl FnOnce() + Send + 'static) {
    let host = format!(
        "{}:{}",
        url.host_str().unwrap_or("127.0.0.1"),
        url.port_or_known_default().unwrap_or(80)
    );
    let Ok(addrs) = url.socket_addrs(|| Some(80)) else {
        return;
    };
    std::thread::spawn(move || {
        let mut misses = 0;
        loop {
            std::thread::sleep(Duration::from_secs(2));
            if alive(&addrs, &host) {
                misses = 0;
            } else {
                misses += 1;
                // Три неудачи подряд: главной программы больше нет, хвост не нужен.
                if misses >= 3 {
                    gone();
                    return;
                }
            }
        }
    });
}

fn watch_stdin(proxy: EventLoopProxy<UserEvent>, mut reader: impl BufRead + Send + 'static) {
    std::thread::spawn(move || {
        loop {
            let mut line = String::new();
            match reader.read_line(&mut line) {
                Ok(0) | Err(_) => break,
                Ok(_) => {}
            }
            if let Some(mode) = line.trim().strip_prefix("mode:") {
                let _ = proxy.send_event(UserEvent::Mode(mode.to_string()));
            }
        }
        let _ = proxy.send_event(UserEvent::ParentGone);
    });
}

/// Очередь на создание WebView: питомцы стартуют разом, и когда несколько процессов открывают один профиль
/// одновременно, WebView2 отвечает «Параметр задан неверно» даже на повторных попытках. Замок снимется сам,
/// если процесс упадёт. Если очередь застряла, идём без неё: лучше риск отказа, чем питомец, который не появится.
pub(crate) fn creation_turn() -> Option<std::fs::File> {
    let file = std::fs::File::create(data_dir().join("webview-pets.lock")).ok()?;
    let deadline = Instant::now() + CREATION_WAIT;
    while Instant::now() < deadline {
        if file.try_lock().is_ok() {
            return Some(file);
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    None
}

/// Точка входа процесса питомца. Адрес входа читается из stdin одной строкой.
pub fn run(mode: &str, bot: Option<String>, at: Option<String>, slot: Option<String>) -> Result<(), String> {
    if mode != "desktop" && mode != "top" {
        return Ok(());
    }
    let Some(bot) = bot.filter(|id| !id.is_empty()) else {
        return Err("не указан бот".into());
    };
    let top = mode == "top";
    let at = at.as_deref().and_then(parse_spot);
    let slot: usize = slot.and_then(|text| text.parse().ok()).unwrap_or(0);

    let mut reader = std::io::BufReader::new(std::io::stdin());
    let mut entry = String::new();
    reader.read_line(&mut entry).map_err(|e| format!("не прочитал адрес: {e}"))?;
    let entry = entry.trim().to_string();
    let url = reqwest::Url::parse(&entry).map_err(|e| format!("адрес не разобрался: {e}"))?;
    let origin = url.origin().ascii_serialization();

    let event_loop = EventLoopBuilder::<UserEvent>::with_user_event().build();
    let proxy = event_loop.create_proxy();
    watch_stdin(proxy.clone(), reader);
    {
        let proxy = proxy.clone();
        watch_server(url, move || {
            let _ = proxy.send_event(UserEvent::ServerGone);
        });
    }
    watch_cursor(proxy.clone());

    let scale = event_loop.primary_monitor().map(|monitor| monitor.scale_factor()).unwrap_or(1.0);
    let size = ((CELL * scale).round() as i32, (CELL_HEIGHT * scale).round() as i32);
    let monitors: Vec<(i32, i32, i32, i32)> = event_loop
        .available_monitors()
        .map(|monitor| {
            let (place, area) = (monitor.position(), monitor.size());
            (place.x, place.y, area.width as i32, area.height as i32)
        })
        .collect();
    let spot = at
        .filter(|&spot| on_screen(spot, size, &monitors))
        .unwrap_or_else(|| default_spot(work_area(), size, (MARGIN * scale).round() as i32, slot));

    let builder = WindowBuilder::new()
        // Не «Deloist»: по этому заголовку клик по питомцу ищет главное окно.
        .with_title("Deloist Pets")
        .with_decorations(false)
        .with_transparent(true)
        .with_resizable(false)
        .with_always_on_top(top)
        .with_focused(false)
        .with_visible(true)
        .with_inner_size(LogicalSize::new(CELL, CELL_HEIGHT))
        .with_position(PhysicalPosition::new(spot.0, spot.1));
    #[cfg(windows)]
    let builder = {
        use tao::platform::windows::WindowBuilderExtWindows;
        // Тень у окон без рамки в tao включена сама: она и рисовала вокруг питомцев скруглённую рамку.
        let builder = builder.with_skip_taskbar(true).with_undecorated_shadow(false);
        if top {
            builder
        } else {
            builder.with_always_on_bottom(true)
        }
    };
    let window = builder.build(&event_loop).map_err(|e| e.to_string())?;
    hide_from_alt_tab(&window);

    let allowed = format!("{origin}/");
    let mut context = WebContext::new(Some(data_dir().join("webview-pets")));
    let creating = creation_turn();
    let mut attempt = 0;
    // Страница одна на всех питомцев: кого показать, она узнаёт отсюда.
    let who = format!("window.PET_BOT = {};", serde_json::to_string(&bot).map_err(|e| e.to_string())?);
    let webview = loop {
        attempt += 1;
        let ipc_proxy = proxy.clone();
        let allowed = allowed.clone();
        let built = WebViewBuilder::new_with_web_context(&mut context)
            .with_url(&entry)
            .with_initialization_script(&who)
            .with_additional_browser_args(BROWSER_ARGS)
            .with_transparent(true)
            .with_navigation_handler(move |target| target.starts_with(&allowed))
            .with_ipc_handler(move |request: wry::http::Request<String>| {
                let _ = ipc_proxy.send_event(UserEvent::Ipc(request.body().clone()));
            })
            .with_devtools(false)
            .build(&window);
        match built {
            Ok(webview) => break webview,
            Err(e) if attempt >= WEBVIEW_ATTEMPTS => return Err(format!("не удалось открыть окно питомца: {e}")),
            Err(_) => std::thread::sleep(Duration::from_secs(1)),
        }
    };

    drop(creating);

    // Окно могло получить настоящий размер уже после того, как WebView его запомнил.
    let _ = webview.set_bounds(wry::Rect {
        position: wry::dpi::PhysicalPosition::new(0, 0).into(),
        size: window.inner_size().into(),
    });

    // Когда, какой слой и сколько подтверждений ещё нужно. Сразу после запуска тоже: слой только что задан
    // при создании окна, а отложенные вызовы tao могут его перебить.
    let mut layer_due: Option<(Instant, bool, u32)> = Some((Instant::now() + LAYER_DELAY, top, LAYER_CHECKS));
    // Человек хоть раз потащил окно: только такие перемещения считаем его выбором места.
    let mut dragged = false;
    let mut save_due: Option<Instant> = None;
    let mut event_loop = event_loop;
    event_loop.run_return(|event, _, control_flow| {
        // Просыпаться незачем: всё, что меняет окно, приходит событиями. Исключения — слой и запись места.
        *control_flow = ControlFlow::Wait;
        if let Some((due, top, left)) = layer_due {
            if Instant::now() >= due {
                // Даже удачный вызов проверяем ещё раз: поздний вызов tao мог снять флаг позже.
                let held = apply_layer(&window, top);
                hide_from_alt_tab(&window);
                layer_due = (left > 0).then(|| (Instant::now() + LAYER_DELAY, top, if held { left - 1 } else { left.min(LAYER_CHECKS) }));
            }
        }
        if save_due.is_some_and(|due| Instant::now() >= due) {
            save_due = None;
            if let Ok(place) = window.outer_position() {
                // Записывает страница: у неё есть вход на сервер, а у процесса его нет.
                let _ = webview.evaluate_script(&format!("window.petMoved?.({},{})", place.x, place.y));
            }
        }
        match event {
            Event::UserEvent(UserEvent::Cursor(x, y)) => {
                if let Ok(place) = window.outer_position() {
                    let size = window.outer_size();
                    let (dx, dy) = cursor_offset((x, y), (place.x, place.y), (size.width, size.height));
                    let _ = webview.evaluate_script(&format!("window.petCursor?.({dx},{dy})"));
                }
            }
            Event::UserEvent(UserEvent::Mode(mode)) => {
                let top = mode == "top";
                window.set_always_on_top(top);
                window.set_always_on_bottom(!top);
                layer_due = Some((Instant::now() + LAYER_DELAY, top, LAYER_CHECKS));
            }
            Event::WindowEvent { event: WindowEvent::Moved(_), .. } if dragged => {
                save_due = Some(Instant::now() + SAVE_DELAY);
            }
            Event::WindowEvent { event: WindowEvent::CloseRequested, .. } => {
                *control_flow = ControlFlow::Exit;
            }
            Event::UserEvent(UserEvent::Ipc(message)) => match message.as_str() {
                "drag" => {
                    dragged = true;
                    let _ = window.drag_window();
                }
                "open" => {
                    crate::desktop::activate_running_window();
                }
                _ => {}
            },
            Event::UserEvent(UserEvent::ServerGone) | Event::UserEvent(UserEvent::ParentGone) => {
                *control_flow = ControlFlow::Exit;
            }
            _ => {}
        }
        if *control_flow != ControlFlow::Exit {
            let next = layer_due.map(|(due, _, _)| due).into_iter().chain(save_due).min();
            if let Some(next) = next {
                *control_flow = ControlFlow::WaitUntil(next);
            }
        }
    });
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const AREA: (i32, i32, i32, i32) = (0, 0, 1000, 800);

    #[test]
    fn the_pet_hears_of_the_cursor_only_when_it_really_moved() {
        assert!(cursor_moved(None, (10, 10)), "первое положение сообщаем всегда");
        assert!(!cursor_moved(Some((10, 10)), (12, 9)), "дрожание руки окно не будит");
        assert!(cursor_moved(Some((10, 10)), (13, 10)));
        assert!(cursor_moved(Some((10, 10)), (10, 6)));
    }

    #[test]
    fn the_cursor_is_measured_from_the_middle_of_the_pet() {
        assert_eq!(cursor_offset((500, 300), (400, 200), (136, 140)), (32, 30));
        // Курсор левее и выше окна, на другом мониторе слева.
        assert_eq!(cursor_offset((-1500, 0), (100, 100), (136, 140)), (-1668, -170));
    }

    #[test]
    fn untouched_pets_stand_in_a_row_from_the_corner_and_wrap() {
        assert_eq!(default_spot(AREA, (100, 120), 10, 0), (890, 670));
        assert_eq!(default_spot(AREA, (100, 120), 10, 1), (780, 670));
        // В ряд на 1000 px помещается девять, десятый идёт выше.
        assert_eq!(default_spot(AREA, (100, 120), 10, 9), (890, 540));
    }

    #[test]
    fn a_tiny_screen_does_not_push_a_pet_outside() {
        assert_eq!(default_spot((0, 0, 50, 50), (100, 120), 10, 3), (0, 0));
    }

    #[test]
    fn a_pet_left_on_a_missing_monitor_is_not_lost() {
        let monitors = [(0, 0, 1920, 1080), (-1280, 0, 1280, 1024)];
        assert!(on_screen((100, 100), (100, 100), &monitors));
        assert!(on_screen((-500, 100), (100, 100), &monitors), "второй монитор слева");
        assert!(!on_screen((3000, 100), (100, 100), &monitors), "монитор справа отключили");
        assert!(!on_screen((-1400, 100), (100, 100), &monitors));
    }

    #[test]
    fn a_new_pet_takes_the_first_free_place() {
        assert_eq!(first_free_slot([].into_iter()), 0);
        assert_eq!(first_free_slot([0, 1, 3].into_iter()), 2);
    }

    #[test]
    fn spots_are_read_only_from_two_numbers() {
        assert_eq!(parse_spot("-20,300"), Some((-20, 300)));
        assert_eq!(parse_spot("20"), None);
        assert_eq!(parse_spot("a,b"), None);
    }
}
