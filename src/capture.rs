//! «Покажи на экране»: снимок экрана, выбор области и PNG во вложения бота.
//!
//! Сначала снимаем весь виртуальный экран (все мониторы) через GDI BitBlt, потом поверх всего
//! показываем этот же снимок затемнённым: человек обводит область мышью, Esc отменяет, Enter берёт
//! монитор под курсором. Так экран выбора никогда не попадает в кадр. Окно одно на один снимок
//! и исчезает сразу после него. Поток снимка помечен Per-Monitor DPI v2: координаты везде
//! в физических пикселях, и мониторы с разным масштабом склеиваются без сдвигов.

use std::sync::atomic::AtomicBool;
use std::sync::Arc;

use serde_json::{json, Value};

use crate::App;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Rect {
    pub left: i32,
    pub top: i32,
    pub right: i32,
    pub bottom: i32,
}

impl Rect {
    pub fn width(&self) -> i32 {
        self.right - self.left
    }
    pub fn height(&self) -> i32 {
        self.bottom - self.top
    }
    fn intersect(&self, other: &Rect) -> Option<Rect> {
        let r = Rect { left: self.left.max(other.left), top: self.top.max(other.top), right: self.right.min(other.right), bottom: self.bottom.min(other.bottom) };
        (r.width() > 0 && r.height() > 0).then_some(r)
    }
}

/// Кадр: пиксели BGRA сверху вниз и где он лежит на виртуальном экране.
#[derive(Clone, Debug, PartialEq)]
pub struct Shot {
    pub left: i32,
    pub top: i32,
    pub width: usize,
    pub height: usize,
    pub bgra: Vec<u8>,
}

/// Меньше этого выделение считаем случайным щелчком, а не областью.
const MIN_SIDE: i32 = 8;

// ---------------------------------------------------------------------------
// Геометрия
// ---------------------------------------------------------------------------

/// Виртуальный экран — общий прямоугольник всех мониторов; левый или верхний бывает в минусе.
pub fn union(rects: &[Rect]) -> Option<Rect> {
    let first = *rects.first()?;
    Some(rects.iter().fold(first, |acc, r| Rect { left: acc.left.min(r.left), top: acc.top.min(r.top), right: acc.right.max(r.right), bottom: acc.bottom.max(r.bottom) }))
}

/// Тянули мышь из `a` в `b`: прямоугольник в любую сторону, обрезанный по кадру.
pub fn selection(a: (i32, i32), b: (i32, i32), bounds: Rect) -> Option<Rect> {
    let rect = Rect { left: a.0.min(b.0), top: a.1.min(b.1), right: a.0.max(b.0), bottom: a.1.max(b.1) };
    rect.intersect(&bounds)
}

fn big_enough(rect: &Rect) -> bool {
    rect.width() >= MIN_SIDE && rect.height() >= MIN_SIDE
}

/// Что перерисовать, когда рамка сменилась: старая и новая вместе с запасом под толщину рамки.
pub fn dirty(old: Option<Rect>, new: Option<Rect>, pad: i32) -> Option<Rect> {
    let both: Vec<Rect> = [old, new].into_iter().flatten().collect();
    union(&both).map(|r| Rect { left: r.left - pad, top: r.top - pad, right: r.right + pad, bottom: r.bottom + pad })
}

/// Окно размером `size` (логические пиксели) рядом с курсором: правее и ниже, а у края экрана —
/// с другой стороны. Масштаб монитора переводит размер в физические пиксели.
pub fn place_near(cursor: (i32, i32), size: (f64, f64), scale: f64, area: Rect) -> Rect {
    let (w, h) = ((size.0 * scale).round() as i32, (size.1 * scale).round() as i32);
    let gap = (16.0 * scale).round() as i32;
    let mut left = cursor.0 + gap;
    let mut top = cursor.1 + gap;
    if left + w > area.right {
        left = cursor.0 - gap - w;
    }
    if top + h > area.bottom {
        top = cursor.1 - gap - h;
    }
    left = left.clamp(area.left, (area.right - w).max(area.left));
    top = top.clamp(area.top, (area.bottom - h).max(area.top));
    Rect { left, top, right: left + w, bottom: top + h }
}

/// Вырезать прямоугольник (экранные координаты) из кадра `width`×`height`, лежащего в (`left`, `top`).
fn crop_pixels(bgra: &[u8], width: usize, height: usize, left: i32, top: i32, rect: Rect) -> Option<Shot> {
    let bounds = Rect { left, top, right: left + width as i32, bottom: top + height as i32 };
    let rect = rect.intersect(&bounds)?;
    let (x0, y0) = ((rect.left - left) as usize, (rect.top - top) as usize);
    let (w, h) = (rect.width() as usize, rect.height() as usize);
    let mut out = Vec::with_capacity(w * h * 4);
    for row in y0..y0 + h {
        let start = (row * width + x0) * 4;
        out.extend_from_slice(&bgra[start..start + w * 4]);
    }
    Some(Shot { left: rect.left, top: rect.top, width: w, height: h, bgra: out })
}

/// Затемнить кадр на месте: фон экрана выбора. Отдельную копию большого экрана не держим.
fn dim_in_place(bgra: &mut [u8]) {
    for px in bgra.chunks_exact_mut(4) {
        for c in &mut px[..3] {
            *c = (u16::from(*c) * 9 / 20) as u8;
        }
    }
}

impl Shot {
    /// Вырезать прямоугольник в экранных координатах.
    #[cfg(test)]
    pub fn crop(&self, rect: Rect) -> Option<Shot> {
        crop_pixels(&self.bgra, self.width, self.height, self.left, self.top, rect)
    }

    #[cfg(test)]
    pub fn pixel(&self, x: usize, y: usize) -> [u8; 3] {
        let i = (y * self.width + x) * 4;
        [self.bgra[i + 2], self.bgra[i + 1], self.bgra[i]]
    }
}

/// PNG без альфы: у снимка экрана она пустая, а RGB на четверть меньше.
pub fn encode_png(shot: &Shot) -> Result<Vec<u8>, String> {
    let mut rgb = Vec::with_capacity(shot.width * shot.height * 3);
    for px in shot.bgra.chunks_exact(4) {
        rgb.extend_from_slice(&[px[2], px[1], px[0]]);
    }
    let mut out = Vec::new();
    {
        let mut encoder = png::Encoder::new(&mut out, shot.width as u32, shot.height as u32);
        encoder.set_color(png::ColorType::Rgb);
        encoder.set_depth(png::BitDepth::Eight);
        // Вложение не больше 10 МБ: сжимаем как следует, на 4K это всё равно доли секунды.
        encoder.set_compression(png::Compression::Default);
        let mut writer = encoder.write_header().map_err(|e| e.to_string())?;
        writer.write_image_data(&rgb).map_err(|e| e.to_string())?;
    }
    Ok(out)
}

// ---------------------------------------------------------------------------
// Сохранить и показать в чате
// ---------------------------------------------------------------------------

/// Снимок — обычное вложение бота (`attachments/<дата>/screen-<время>.png`): окно и док прикладывают
/// его к сообщению, а модель видит саму картинку.
async fn deliver(app: &App, png: Vec<u8>) -> Result<Value, String> {
    // Снимок и голосовой вопрос к нему уходят одному боту.
    let bot = crate::voice::voice_bot(app).await.ok_or("нет ни одного бота")?;
    let name = format!("screen-{}.png", chrono::Local::now().format("%H%M%S"));
    let saved = crate::attach::save(&app.store, std::slice::from_ref(&bot), &name, &png).await?;
    crate::voice::announce(app, json!({ "type": "snap", "botId": bot, "attachment": saved }));
    Ok(saved)
}

static BUSY: AtomicBool = AtomicBool::new(false);

/// Нажали «Покажи на экране». Работает в своём потоке: у экрана выбора свой цикл сообщений.
pub fn on_hotkey(app: Arc<App>, runtime: tokio::runtime::Handle) {
    let Some(_busy) = crate::hotkeys::Busy::take(&BUSY) else { return };
    let english = app.store.settings().ui_lang == "en";
    let Some(screen) = win::grab_screen() else {
        eprintln!("[snap] не снял экран");
        return;
    };
    let Some(rect) = win::choose(&screen, english) else { return };
    let Some(part) = screen.crop(rect) else { return };
    // Весь экран больше не нужен: кодируем только вырезанное.
    drop(screen);
    // Огромный кадр с пустыми рамками кодировать долго, но это доли секунды даже для 4K.
    match encode_png(&part) {
        Ok(png) => {
            if let Err(message) = runtime.block_on(deliver(&app, png)) {
                eprintln!("[snap] {message}");
            }
        }
        Err(e) => eprintln!("[snap] PNG: {e}"),
    }
}

/// Проверки: вместо настоящего экрана — полосатая картинка, дальше тот же путь.
pub async fn test_snap(app: &App) -> Result<Value, String> {
    let (width, height) = (64usize, 48usize);
    let mut bgra = Vec::with_capacity(width * height * 4);
    for y in 0..height {
        for x in 0..width {
            bgra.extend_from_slice(&[(x * 4) as u8, (y * 5) as u8, if x < width / 2 { 255 } else { 0 }, 255]);
        }
    }
    let png = encode_png(&Shot { left: 0, top: 0, width, height, bgra })?;
    deliver(app, png).await
}

// ---------------------------------------------------------------------------
// Windows: снимок, окно выбора
// ---------------------------------------------------------------------------

#[cfg(windows)]
pub mod win {
    use super::{big_enough, crop_pixels, dim_in_place, dirty, selection, Rect, Shot};
    use std::ptr::{null, null_mut};
    use windows_sys::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, RECT, WPARAM};
    use windows_sys::Win32::Graphics::Gdi::{
        BeginPaint, BitBlt, CreateCompatibleDC, CreateDIBSection, CreateSolidBrush, DeleteDC, DeleteObject, DrawTextW, EndPaint, FillRect, FrameRect, GetDC, GetMonitorInfoW, GetStockObject, InvalidateRect, MonitorFromPoint, ReleaseDC, SelectObject, SetBkMode, SetTextColor, BITMAPINFO, BITMAPINFOHEADER, BI_RGB,
        CAPTUREBLT, DEFAULT_GUI_FONT, DIB_RGB_COLORS, DT_CENTER, DT_SINGLELINE, DT_VCENTER, HBITMAP, HDC, HGDIOBJ, MONITORINFO, MONITOR_DEFAULTTONEAREST, PAINTSTRUCT, SRCCOPY, TRANSPARENT,
    };
    use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
    use windows_sys::Win32::UI::HiDpi::{SetThreadDpiAwarenessContext, DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2};
    use windows_sys::Win32::UI::Input::KeyboardAndMouse::{ReleaseCapture, SetCapture, VK_ESCAPE, VK_RETURN};
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        CreateWindowExW, DefWindowProcW, DestroyWindow, DispatchMessageW, GetCursorPos, GetMessageW, GetSystemMetrics, GetWindowLongPtrW, KillTimer, LoadCursorW, RegisterClassW, SetForegroundWindow, SetTimer, SetWindowLongPtrW, ShowWindow, TranslateMessage, GWLP_USERDATA, IDC_CROSS, MSG,
        SM_CXSCREEN, SM_CXVIRTUALSCREEN, SM_CYSCREEN, SM_CYVIRTUALSCREEN, SM_XVIRTUALSCREEN, SM_YVIRTUALSCREEN, SW_SHOW, WM_ACTIVATEAPP, WM_DPICHANGED, WM_ERASEBKGND, WM_KEYDOWN, WM_LBUTTONDOWN, WM_LBUTTONUP, WM_MOUSEMOVE, WM_PAINT, WM_RBUTTONDOWN, WM_TIMER, WNDCLASSW, WS_EX_TOOLWINDOW,
        WS_EX_TOPMOST, WS_POPUP,
    };

    /// Кадр в памяти: DIB-секция в своём контексте рисования.
    pub struct Canvas {
        dc: HDC,
        bitmap: HBITMAP,
        old: HGDIOBJ,
        bits: *mut u8,
        width: i32,
        height: i32,
    }

    impl Canvas {
        pub fn new(width: i32, height: i32) -> Option<Self> {
            unsafe {
                let screen = GetDC(null_mut());
                let dc = CreateCompatibleDC(screen);
                ReleaseDC(null_mut(), screen);
                if dc.is_null() {
                    return None;
                }
                let mut info: BITMAPINFO = std::mem::zeroed();
                info.bmiHeader = BITMAPINFOHEADER {
                    biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
                    biWidth: width,
                    // Минус — строки сверху вниз, как в PNG.
                    biHeight: -height,
                    biPlanes: 1,
                    biBitCount: 32,
                    biCompression: BI_RGB,
                    ..std::mem::zeroed()
                };
                let mut bits: *mut core::ffi::c_void = null_mut();
                let bitmap = CreateDIBSection(dc, &info, DIB_RGB_COLORS, &mut bits, null_mut(), 0);
                if bitmap.is_null() || bits.is_null() {
                    DeleteDC(dc);
                    return None;
                }
                let old = SelectObject(dc, bitmap);
                Some(Self { dc, bitmap, old, bits: bits as *mut u8, width, height })
            }
        }

        fn len(&self) -> usize {
            self.width as usize * self.height as usize * 4
        }

        fn bytes(&self) -> &[u8] {
            // Буфер DIB-секции живёт, пока жив Canvas; размер посчитан по её заголовку.
            unsafe { std::slice::from_raw_parts(self.bits, self.len()) }
        }

        fn bytes_mut(&mut self) -> &mut [u8] {
            unsafe { std::slice::from_raw_parts_mut(self.bits, self.len()) }
        }

        #[cfg(test)]
        pub fn pixels(&self) -> Vec<u8> {
            self.bytes().to_vec()
        }
    }

    /// Весь виртуальный экран одним кадром прямо в DIB-секции: её же показывает экран выбора.
    pub struct Screen {
        canvas: Canvas,
        left: i32,
        top: i32,
    }

    impl Screen {
        pub fn crop(&self, rect: Rect) -> Option<Shot> {
            crop_pixels(self.canvas.bytes(), self.canvas.width as usize, self.canvas.height as usize, self.left, self.top, rect)
        }

        pub fn bounds(&self) -> Rect {
            Rect { left: self.left, top: self.top, right: self.left + self.canvas.width, bottom: self.top + self.canvas.height }
        }

        pub fn pixels(&self) -> &[u8] {
            self.canvas.bytes()
        }
    }

    impl Drop for Canvas {
        fn drop(&mut self) {
            unsafe {
                SelectObject(self.dc, self.old);
                DeleteObject(self.bitmap);
                DeleteDC(self.dc);
            }
        }
    }

    /// Поток снимка в физических пикселях: так мониторы с разным масштабом не съезжают.
    fn per_monitor_dpi() {
        unsafe {
            SetThreadDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
        }
    }

    /// Все мониторы одним кадром.
    pub fn grab_screen() -> Option<Screen> {
        per_monitor_dpi();
        let (left, top, width, height) = unsafe {
            (GetSystemMetrics(SM_XVIRTUALSCREEN), GetSystemMetrics(SM_YVIRTUALSCREEN), GetSystemMetrics(SM_CXVIRTUALSCREEN), GetSystemMetrics(SM_CYVIRTUALSCREEN))
        };
        if width <= 0 || height <= 0 {
            return None;
        }
        let canvas = Canvas::new(width, height)?;
        let copied = unsafe {
            let screen = GetDC(null_mut());
            // CAPTUREBLT: слоистые окна (подсказки, док) тоже попадают в кадр.
            let ok = BitBlt(canvas.dc, 0, 0, width, height, screen, left, top, SRCCOPY | CAPTUREBLT);
            ReleaseDC(null_mut(), screen);
            ok != 0
        };
        copied.then_some(Screen { canvas, left, top })
    }

    #[cfg(test)]
    #[link(name = "user32", kind = "raw-dylib")]
    extern "system" {
        fn PrintWindow(hwnd: HWND, hdc: HDC, flags: u32) -> i32;
    }

    #[cfg(test)]
    /// Снимок одного окна, даже за краем экрана: окно рисует себя само в наш кадр.
    pub fn grab_window(hwnd: HWND, width: i32, height: i32) -> Option<Shot> {
        let canvas = Canvas::new(width, height)?;
        let ok = unsafe { PrintWindow(hwnd, canvas.dc, 0) } != 0;
        ok.then(|| Shot { left: 0, top: 0, width: width as usize, height: height as usize, bgra: canvas.pixels() })
    }

    struct Picker {
        /// Сам снимок (его DIB-секция у `Screen`) и затемнённая копия.
        bright: HDC,
        dim: Canvas,
        /// Экран в координатах окна: окно стоит ровно на виртуальном экране.
        bounds: Rect,
        origin: (i32, i32),
        anchor: Option<(i32, i32)>,
        current: Option<Rect>,
        hint: Vec<u16>,
        hint_rect: RECT,
        border: isize,
        hint_fill: isize,
        done: Option<Option<Rect>>,
    }

    const TIMER_GIVE_UP: usize = 1;
    const GIVE_UP_MS: u32 = 120_000;

    fn point(lparam: LPARAM) -> (i32, i32) {
        ((lparam & 0xFFFF) as i16 as i32, ((lparam >> 16) & 0xFFFF) as i16 as i32)
    }

    fn win_rect(r: Rect) -> RECT {
        RECT { left: r.left, top: r.top, right: r.right, bottom: r.bottom }
    }

    /// Монитор под курсором в координатах окна: Enter берёт его целиком.
    fn monitor_under_cursor(origin: (i32, i32)) -> Option<Rect> {
        unsafe {
            let mut pt = POINT { x: 0, y: 0 };
            GetCursorPos(&mut pt);
            let monitor = MonitorFromPoint(pt, MONITOR_DEFAULTTONEAREST);
            let mut info: MONITORINFO = std::mem::zeroed();
            info.cbSize = std::mem::size_of::<MONITORINFO>() as u32;
            if GetMonitorInfoW(monitor, &mut info) == 0 {
                return None;
            }
            let m = info.rcMonitor;
            Some(Rect { left: m.left - origin.0, top: m.top - origin.1, right: m.right - origin.0, bottom: m.bottom - origin.1 })
        }
    }

    unsafe extern "system" fn proc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
        let picker = unsafe { GetWindowLongPtrW(hwnd, GWLP_USERDATA) } as *mut Picker;
        if picker.is_null() {
            return unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) };
        }
        // Указатель поставлен до показа окна и живёт дольше него: окно разрушают раньше Picker.
        let p = unsafe { &mut *picker };
        let redraw = |p: &Picker, old: Option<Rect>| {
            if let Some(r) = dirty(old, p.current, 4) {
                unsafe { InvalidateRect(hwnd, &win_rect(r), 0) };
            }
        };
        match msg {
            WM_ERASEBKGND => return 1,
            WM_PAINT => unsafe {
                let mut ps: PAINTSTRUCT = std::mem::zeroed();
                let dc = BeginPaint(hwnd, &mut ps);
                let rc = ps.rcPaint;
                BitBlt(dc, rc.left, rc.top, rc.right - rc.left, rc.bottom - rc.top, p.dim.dc, rc.left, rc.top, SRCCOPY);
                let paint = Rect { left: rc.left, top: rc.top, right: rc.right, bottom: rc.bottom };
                if let Some(sel) = p.current {
                    if let Some(part) = sel.intersect(&paint) {
                        BitBlt(dc, part.left, part.top, part.width(), part.height(), p.bright, part.left, part.top, SRCCOPY);
                    }
                    let outer = win_rect(Rect { left: sel.left - 2, top: sel.top - 2, right: sel.right + 2, bottom: sel.bottom + 2 });
                    let inner = win_rect(Rect { left: sel.left - 1, top: sel.top - 1, right: sel.right + 1, bottom: sel.bottom + 1 });
                    FrameRect(dc, &outer, p.border as _);
                    FrameRect(dc, &inner, p.border as _);
                } else {
                    FillRect(dc, &p.hint_rect, p.hint_fill as _);
                    SetBkMode(dc, TRANSPARENT as i32);
                    SetTextColor(dc, 0x00FF_FFFF);
                    let old = SelectObject(dc, GetStockObject(DEFAULT_GUI_FONT));
                    let mut text_rect = p.hint_rect;
                    DrawTextW(dc, p.hint.as_ptr(), p.hint.len() as i32, &mut text_rect, DT_CENTER | DT_VCENTER | DT_SINGLELINE);
                    SelectObject(dc, old);
                }
                EndPaint(hwnd, &ps);
                return 0;
            },
            WM_LBUTTONDOWN => {
                let at = point(lparam);
                p.anchor = Some(at);
                let old = p.current;
                p.current = None;
                unsafe {
                    SetCapture(hwnd);
                    // Подсказка пропадает, как только начали тянуть.
                    InvalidateRect(hwnd, &p.hint_rect, 0);
                }
                redraw(p, old);
            }
            WM_MOUSEMOVE => {
                if let Some(anchor) = p.anchor {
                    let old = p.current;
                    p.current = selection(anchor, point(lparam), p.bounds);
                    redraw(p, old);
                }
            }
            WM_LBUTTONUP => {
                unsafe { ReleaseCapture() };
                if let Some(anchor) = p.anchor.take() {
                    match selection(anchor, point(lparam), p.bounds).filter(big_enough) {
                        Some(rect) => p.done = Some(Some(rect)),
                        None => {
                            // Просто щелчок: ждём настоящую область.
                            let old = p.current.take();
                            redraw(p, old);
                            unsafe { InvalidateRect(hwnd, &p.hint_rect, 0) };
                        }
                    }
                }
            }
            WM_RBUTTONDOWN => p.done = Some(None),
            WM_KEYDOWN => match wparam as u16 {
                VK_ESCAPE => p.done = Some(None),
                VK_RETURN => p.done = Some(monitor_under_cursor(p.origin).and_then(|m| m.intersect(&p.bounds)).or(Some(p.bounds))),
                _ => {}
            },
            // Человек ушёл в другую программу (Alt+Tab, Win): экран выбора не держит его.
            WM_ACTIVATEAPP if wparam == 0 => p.done = Some(None),
            WM_TIMER if wparam == TIMER_GIVE_UP => p.done = Some(None),
            // Окно во весь виртуальный экран: масштаб монитора его размер не меняет.
            WM_DPICHANGED => return 0,
            _ => {}
        }
        unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) }
    }

    /// Показать снимок затемнённым и дождаться выбора. Возвращает область в экранных координатах.
    pub fn choose(shot: &Screen, english: bool) -> Option<Rect> {
        per_monitor_dpi();
        let (w, h) = (shot.canvas.width, shot.canvas.height);
        // Две копии экрана, не больше: снимок и затемнённый фон, который темнеет на месте.
        let mut dim = Canvas::new(w, h)?;
        dim.bytes_mut().copy_from_slice(shot.canvas.bytes());
        dim_in_place(dim.bytes_mut());
        let origin = (shot.left, shot.top);
        // Подсказка сверху посередине основного монитора: он всегда начинается в (0, 0) экрана.
        let (pw, _) = unsafe { (GetSystemMetrics(SM_CXSCREEN), GetSystemMetrics(SM_CYSCREEN)) };
        let (hint_w, hint_h) = (560, 40);
        let hx = -origin.0 + (pw - hint_w) / 2;
        let hy = -origin.1 + 24;
        let hint = if english { "Drag to select an area · Enter: whole screen · Esc: cancel" } else { "Обведи область мышью · Enter — весь экран · Esc — отмена" };
        let mut picker = Box::new(Picker {
            bright: shot.canvas.dc,
            dim,
            bounds: Rect { left: 0, top: 0, right: w, bottom: h },
            origin,
            anchor: None,
            current: None,
            hint: hint.encode_utf16().collect(),
            hint_rect: RECT { left: hx, top: hy, right: hx + hint_w, bottom: hy + hint_h },
            border: unsafe { CreateSolidBrush(0x00F0_A040) } as isize,
            hint_fill: unsafe { CreateSolidBrush(0x0030_2a26) } as isize,
            done: None,
        });

        let class: Vec<u16> = "DeloistSnap\0".encode_utf16().collect();
        let hwnd = unsafe {
            let instance = GetModuleHandleW(null());
            let wc = WNDCLASSW { lpfnWndProc: Some(proc), hInstance: instance, lpszClassName: class.as_ptr(), hCursor: LoadCursorW(null_mut(), IDC_CROSS), ..std::mem::zeroed() };
            RegisterClassW(&wc);
            CreateWindowExW(WS_EX_TOPMOST | WS_EX_TOOLWINDOW, class.as_ptr(), null(), WS_POPUP, shot.left, shot.top, w, h, null_mut(), null_mut(), instance, null())
        };
        let cleanup = |picker: &Picker| unsafe {
            DeleteObject(picker.border as _);
            DeleteObject(picker.hint_fill as _);
        };
        if hwnd.is_null() {
            cleanup(&picker);
            return None;
        }
        unsafe {
            SetWindowLongPtrW(hwnd, GWLP_USERDATA, &mut *picker as *mut Picker as isize);
            ShowWindow(hwnd, SW_SHOW);
            SetTimer(hwnd, TIMER_GIVE_UP, GIVE_UP_MS, None);
            // Права на фокус есть, пока программа только что получила нажатие. Не вышло — экран выбора
            // висел бы поверх всего без клавиатуры: сразу убираем его.
            if SetForegroundWindow(hwnd) == 0 {
                picker.done = Some(None);
            }
        }
        let mut msg: MSG = unsafe { std::mem::zeroed() };
        while picker.done.is_none() && unsafe { GetMessageW(&mut msg, null_mut(), 0, 0) } > 0 {
            unsafe {
                TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }
        }
        unsafe {
            KillTimer(hwnd, TIMER_GIVE_UP);
            SetWindowLongPtrW(hwnd, GWLP_USERDATA, 0);
            DestroyWindow(hwnd);
        }
        cleanup(&picker);
        let chosen = picker.done.flatten()?;
        Some(Rect { left: chosen.left + origin.0, top: chosen.top + origin.1, right: chosen.right + origin.0, bottom: chosen.bottom + origin.1 })
    }
}

#[cfg(not(windows))]
pub mod win {
    use super::{Rect, Shot};
    pub struct Screen;
    impl Screen {
        pub fn crop(&self, _rect: Rect) -> Option<Shot> {
            None
        }
    }
    pub fn grab_screen() -> Option<Screen> {
        None
    }
    pub fn choose(_shot: &Screen, _english: bool) -> Option<Rect> {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn r(left: i32, top: i32, right: i32, bottom: i32) -> Rect {
        Rect { left, top, right, bottom }
    }

    #[test]
    fn the_virtual_screen_spans_monitors_left_and_above_the_primary() {
        // Основной 1920×1080 с масштабом 100 %, слева 4K со 150 % (в физических пикселях 3840 шириной), сверху ноутбук.
        let monitors = [r(0, 0, 1920, 1080), r(-3840, -400, 0, 1760), r(200, -1200, 2120, 0)];
        assert_eq!(union(&monitors), Some(r(-3840, -1200, 2120, 1760)));
        assert_eq!(union(&[]), None);
    }

    #[test]
    fn a_drag_in_any_direction_becomes_a_clipped_rectangle() {
        let bounds = r(0, 0, 100, 80);
        assert_eq!(selection((10, 10), (50, 40), bounds), Some(r(10, 10, 50, 40)));
        assert_eq!(selection((50, 40), (10, 10), bounds), Some(r(10, 10, 50, 40)), "справа налево и снизу вверх");
        assert_eq!(selection((90, 70), (300, -20), bounds), Some(r(90, 0, 100, 70)), "вылезло за край");
        assert_eq!(selection((5, 5), (5, 30), bounds), None, "нулевая ширина");
        assert!(!big_enough(&r(0, 0, 7, 100)), "щелчок, а не область");
        assert!(big_enough(&r(0, 0, 8, 8)));
    }

    #[test]
    fn only_the_changed_frame_is_repainted() {
        assert_eq!(dirty(None, None, 4), None);
        assert_eq!(dirty(Some(r(10, 10, 20, 20)), None, 4), Some(r(6, 6, 24, 24)));
        assert_eq!(dirty(Some(r(10, 10, 20, 20)), Some(r(15, 5, 40, 18)), 2), Some(r(8, 3, 42, 22)));
    }

    #[test]
    fn the_prompt_stays_on_screen_near_the_cursor_at_any_scale() {
        let area = r(0, 0, 1920, 1080);
        assert_eq!(place_near((100, 100), (400.0, 150.0), 1.0, area), r(116, 116, 516, 266));
        // 150 %: размер в физических пикселях в полтора раза больше, отступ тоже.
        assert_eq!(place_near((100, 100), (400.0, 150.0), 1.5, area), r(124, 124, 724, 349));
        // У правого нижнего угла окно встаёт слева сверху от курсора.
        assert_eq!(place_near((1900, 1070), (400.0, 150.0), 1.0, area), r(1484, 904, 1884, 1054));
        // Второй монитор слева, в минусовых координатах.
        let left = r(-2560, 0, 0, 1440);
        let placed = place_near((-10, 5), (400.0, 150.0), 1.25, left);
        assert!(placed.right <= 0 && placed.left >= -2560 && placed.top >= 0, "{placed:?}");
        // Экран меньше окна: прижато к левому верхнему углу.
        assert_eq!(place_near((10, 10), (400.0, 150.0), 1.0, r(0, 0, 300, 100)).left, 0);
    }

    fn shot(width: usize, height: usize, left: i32, top: i32) -> Shot {
        let mut bgra = Vec::new();
        for y in 0..height {
            for x in 0..width {
                bgra.extend_from_slice(&[x as u8, y as u8, 7, 0]);
            }
        }
        Shot { left, top, width, height, bgra }
    }

    #[test]
    fn crop_uses_screen_coordinates_even_left_of_the_primary_monitor() {
        let full = shot(40, 30, -20, -10);
        let part = full.crop(r(-15, -5, -5, 0)).unwrap();
        assert_eq!((part.width, part.height, part.left, part.top), (10, 5, -15, -5));
        // Пиксель (−15, −5) экрана — это (5, 5) кадра: синий = x, зелёный = y.
        assert_eq!(part.pixel(0, 0), [7, 5, 5]);
        assert_eq!(part.pixel(9, 4), [7, 9, 14]);
        assert!(full.crop(r(100, 100, 120, 120)).is_none(), "мимо кадра");
        assert_eq!(full.crop(r(-30, -30, 100, 100)).unwrap().width, 40, "обрезано по кадру");
    }

    #[test]
    fn png_round_trips_the_pixels() {
        let original = shot(13, 9, 0, 0);
        let bytes = encode_png(&original).unwrap();
        let decoder = png::Decoder::new(std::io::Cursor::new(bytes));
        let mut reader = decoder.read_info().unwrap();
        let mut buf = vec![0; reader.output_buffer_size()];
        let info = reader.next_frame(&mut buf).unwrap();
        assert_eq!((info.width, info.height, info.color_type), (13, 9, png::ColorType::Rgb));
        assert_eq!(&buf[(4 * 13 + 6) * 3..(4 * 13 + 6) * 3 + 3], &original.pixel(6, 4));
    }

    #[test]
    fn the_dim_copy_keeps_alpha_and_darkens_colour() {
        let mut one = vec![200, 100, 20, 255, 10, 0, 255, 7];
        dim_in_place(&mut one);
        assert_eq!(one, vec![90, 45, 9, 255, 4, 0, 114, 7]);
    }

    /// Окно за краем экрана рисует себя само: проверяем снимок окна без настоящего рабочего стола.
    #[cfg(windows)]
    #[test]
    fn an_off_screen_window_is_captured_with_its_own_pixels() {
        use std::ptr::{null, null_mut};
        use windows_sys::Win32::Foundation::{HWND, LPARAM, LRESULT, RECT, WPARAM};
        use windows_sys::Win32::Graphics::Gdi::{BeginPaint, CreateSolidBrush, DeleteObject, EndPaint, FillRect, HDC, PAINTSTRUCT};
        use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
        use windows_sys::Win32::UI::WindowsAndMessaging::{CreateWindowExW, DefWindowProcW, DestroyWindow, RegisterClassW, ShowWindow, SW_SHOWNOACTIVATE, WM_PAINT, WM_PRINTCLIENT, WNDCLASSW, WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW, WS_POPUP};

        const W: i32 = 120;
        const H: i32 = 60;

        fn paint(dc: HDC) {
            unsafe {
                // Левая половина красная, правая синяя; COLORREF — 0x00BBGGRR.
                let red = CreateSolidBrush(0x0000_00FF);
                let blue = CreateSolidBrush(0x00FF_0000);
                FillRect(dc, &RECT { left: 0, top: 0, right: W / 2, bottom: H }, red);
                FillRect(dc, &RECT { left: W / 2, top: 0, right: W, bottom: H }, blue);
                DeleteObject(red);
                DeleteObject(blue);
            }
        }

        unsafe extern "system" fn proc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
            match msg {
                WM_PAINT => unsafe {
                    let mut ps: PAINTSTRUCT = std::mem::zeroed();
                    let dc = BeginPaint(hwnd, &mut ps);
                    paint(dc);
                    EndPaint(hwnd, &ps);
                    0
                },
                WM_PRINTCLIENT => {
                    paint(wparam as HDC);
                    0
                }
                _ => unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) },
            }
        }

        let class: Vec<u16> = "DeloistSnapTest\0".encode_utf16().collect();
        let hwnd = unsafe {
            let instance = GetModuleHandleW(null());
            let wc = WNDCLASSW { lpfnWndProc: Some(proc), hInstance: instance, lpszClassName: class.as_ptr(), ..std::mem::zeroed() };
            RegisterClassW(&wc);
            // Далеко за краями любого рабочего стола и без фокуса: человек его не увидит.
            CreateWindowExW(WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE, class.as_ptr(), null(), WS_POPUP, -30000, -30000, W, H, null_mut(), null_mut(), instance, null())
        };
        assert!(!hwnd.is_null());
        unsafe { ShowWindow(hwnd, SW_SHOWNOACTIVATE) };
        let shot = win::grab_window(hwnd, W, H);
        unsafe { DestroyWindow(hwnd) };
        let shot = shot.expect("PrintWindow");
        assert_eq!(shot.pixel(5, 5), [255, 0, 0], "слева красный");
        assert_eq!(shot.pixel(W as usize - 5, H as usize - 5), [0, 0, 255], "справа синий");
        // Дальше тот же путь, что у снимка экрана: вырезать и закодировать.
        let right = shot.crop(r(W / 2, 0, W, H)).unwrap();
        let png = encode_png(&right).unwrap();
        let mut reader = png::Decoder::new(std::io::Cursor::new(png)).read_info().unwrap();
        let mut buf = vec![0; reader.output_buffer_size()];
        reader.next_frame(&mut buf).unwrap();
        assert_eq!(&buf[..3], &[0, 0, 255]);
    }
}
