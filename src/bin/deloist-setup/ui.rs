//! Окна загрузчика: вопросы (MessageBox) и окно с полосой загрузки.

use std::ptr::{null, null_mut};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Mutex;

use windows_sys::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
use windows_sys::Win32::Graphics::Gdi::{GetStockObject, COLOR_WINDOW, DEFAULT_GUI_FONT};
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::UI::Controls::{InitCommonControlsEx, ICC_PROGRESS_CLASS, INITCOMMONCONTROLSEX, PBM_SETPOS, PBM_SETRANGE32};
use windows_sys::Win32::UI::WindowsAndMessaging::*;

use crate::w;

pub const YES: i32 = 6;
pub const NO: i32 = 7;

const WM_PROGRESS: u32 = WM_APP + 1;
const WM_FINISHED: u32 = WM_APP + 2;

static LABEL: Mutex<String> = Mutex::new(String::new());
static TEXT_HWND: AtomicUsize = AtomicUsize::new(0);
static BAR_HWND: AtomicUsize = AtomicUsize::new(0);

pub fn ask(text: &str, title: &str, style: u32) -> i32 {
    unsafe { MessageBoxW(null_mut(), w(text).as_ptr(), w(title).as_ptr(), style | MB_SETFOREGROUND) }
}

pub fn info(text: &str, title: &str) {
    ask(text, title, MB_OK | MB_ICONINFORMATION);
}

pub fn error(text: &str, title: &str) {
    ask(text, title, MB_OK | MB_ICONERROR);
}

unsafe extern "system" fn proc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    match msg {
        WM_PROGRESS => {
            let text = LABEL.lock().map(|t| t.clone()).unwrap_or_default();
            SetWindowTextW(TEXT_HWND.load(Ordering::Relaxed) as HWND, w(&text).as_ptr());
            SendMessageW(BAR_HWND.load(Ordering::Relaxed) as HWND, PBM_SETPOS, wparam, 0);
            0
        }
        WM_FINISHED => {
            DestroyWindow(hwnd);
            0
        }
        // Пока идёт установка, закрыть окно нельзя: оборванная копия оставила бы полфайла.
        WM_CLOSE => 0,
        WM_DESTROY => {
            PostQuitMessage(0);
            0
        }
        _ => DefWindowProcW(hwnd, msg, wparam, lparam),
    }
}

/// Сообщает окну о ходе работы из другого потока.
#[derive(Clone, Copy)]
pub struct Reporter(usize);

impl Reporter {
    pub fn report(&self, percent: u32, text: &str) {
        if let Ok(mut label) = LABEL.lock() {
            *label = text.to_string();
        }
        unsafe { PostMessageW(self.0 as HWND, WM_PROGRESS, percent as WPARAM, 0) };
    }
}

/// Показывает окно с полосой и крутит сообщения, пока `work` не вернётся. Возвращает её результат.
pub fn with_progress<T: Send + 'static>(title: &str, work: impl FnOnce(Reporter) -> Result<T, String> + Send + 'static) -> Result<T, String> {
    unsafe {
        InitCommonControlsEx(&INITCOMMONCONTROLSEX { dwSize: std::mem::size_of::<INITCOMMONCONTROLSEX>() as u32, dwICC: ICC_PROGRESS_CLASS });
        let instance = GetModuleHandleW(null());
        let class = w("DeloistSetupWindow");
        let wc = WNDCLASSW {
            style: 0,
            lpfnWndProc: Some(proc),
            cbClsExtra: 0,
            cbWndExtra: 0,
            hInstance: instance,
            hIcon: LoadIconW(null_mut(), IDI_APPLICATION),
            hCursor: LoadCursorW(null_mut(), IDC_ARROW),
            hbrBackground: (COLOR_WINDOW + 1) as usize as _,
            lpszMenuName: null(),
            lpszClassName: class.as_ptr(),
        };
        RegisterClassW(&wc);
        let scale = |n: i32| n * (windows_sys::Win32::UI::HiDpi::GetDpiForSystem() as i32) / 96;
        let (width, height) = (scale(440), scale(140));
        let x = (GetSystemMetrics(SM_CXSCREEN) - width) / 2;
        let y = (GetSystemMetrics(SM_CYSCREEN) - height) / 2;
        let hwnd = CreateWindowExW(
            0,
            class.as_ptr(),
            w(title).as_ptr(),
            WS_OVERLAPPED | WS_CAPTION | WS_SYSMENU | WS_VISIBLE,
            x,
            y,
            width,
            height,
            null_mut(),
            null_mut(),
            instance,
            null(),
        );
        let text = CreateWindowExW(0, w("STATIC").as_ptr(), w("...").as_ptr(), WS_CHILD | WS_VISIBLE, scale(16), scale(16), scale(392), scale(24), hwnd, null_mut(), instance, null());
        let bar = CreateWindowExW(0, w("msctls_progress32").as_ptr(), null(), WS_CHILD | WS_VISIBLE, scale(16), scale(48), scale(392), scale(22), hwnd, null_mut(), instance, null());
        SendMessageW(bar, PBM_SETRANGE32, 0, 100);
        let font = GetStockObject(DEFAULT_GUI_FONT);
        SendMessageW(text, WM_SETFONT, font as WPARAM, 1);
        TEXT_HWND.store(text as usize, Ordering::Relaxed);
        BAR_HWND.store(bar as usize, Ordering::Relaxed);
        SetForegroundWindow(hwnd);

        let handle = hwnd as usize;
        let worker = std::thread::spawn(move || {
            // Паника в работе не должна оставить окно висеть: закрываем его в любом случае.
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| work(Reporter(handle)))).unwrap_or_else(|_| Err("внутренняя ошибка установщика".to_string()));
            PostMessageW(handle as HWND, WM_FINISHED, 0, 0);
            result
        });
        let mut message: MSG = std::mem::zeroed();
        while GetMessageW(&mut message, null_mut(), 0, 0) > 0 {
            TranslateMessage(&message);
            DispatchMessageW(&message);
        }
        worker.join().unwrap_or_else(|_| Err("внутренняя ошибка установщика".to_string()))
    }
}
