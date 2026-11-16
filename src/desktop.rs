//! Отдельное приложение: своё окно на WebView2 и значок в трее.
//! Закрытие окна не завершает программу: боты и рутины продолжают работать в фоне.

use tao::dpi::LogicalSize;
use tao::event::{Event, WindowEvent};
use tao::event_loop::{ControlFlow, EventLoopBuilder, EventLoopProxy};
use tao::platform::run_return::EventLoopExtRunReturn;
use tao::window::{Icon as WindowIcon, Window, WindowBuilder};
use tray_icon::menu::{Menu, MenuEvent, MenuItem, PredefinedMenuItem};
use tray_icon::{Icon as TrayIconImage, MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use wry::{WebContext, WebView, WebViewBuilder};

use crate::icon_art::icon_rgba;

const WINDOW_TITLE: &str = "Deloist";
/// Свой класс окна: заголовок «Deloist» есть и у настольного приложения планировщика,
/// и по одному заголовку второй запуск мог бы показать чужое окно.
pub(crate) const WINDOW_CLASS: &str = "DeloistAgentWindow";

enum UserEvent {
    Tray(TrayIconEvent),
    Menu(MenuEvent),
    /// Страница сменила тему: системная шапка окна перекрашивается в цвет рамки.
    TitleBar(String),
}

/// `titlebar:R,G,B;R,G,B;D` от страницы: цвет шапки, цвет надписи (COLORREF 0x00BBGGRR), тёмная ли тема.
fn parse_title_colors(spec: &str) -> Option<(u32, u32, bool)> {
    let mut parts = spec.split(';');
    let color = |part: Option<&str>| -> Option<u32> {
        let rgb: Vec<u8> = part?.split(',').map(|n| n.trim().parse::<u8>().ok()).collect::<Option<_>>()?;
        match rgb[..] {
            [r, g, b] => Some(u32::from(r) | (u32::from(g) << 8) | (u32::from(b) << 16)),
            _ => None,
        }
    };
    let caption = color(parts.next())?;
    let text = color(parts.next())?;
    let dark = match parts.next()? {
        "1" => true,
        "0" => false,
        _ => return None,
    };
    parts.next().is_none().then_some((caption, text, dark))
}

#[cfg(windows)]
fn tint_title_bar(window: &Window, spec: &str) {
    use tao::platform::windows::WindowExtWindows;
    use windows_sys::Win32::Graphics::Dwm::{
        DwmSetWindowAttribute, DWMWA_CAPTION_COLOR, DWMWA_TEXT_COLOR, DWMWA_USE_IMMERSIVE_DARK_MODE,
    };
    let Some((caption, text, dark)) = parse_title_colors(spec) else { return };
    let hwnd = window.hwnd() as _;
    let dark = i32::from(dark);
    // Windows 10 не знает цвета шапки и вернёт ошибку: там останется только тёмный режим, это не страшно.
    unsafe {
        DwmSetWindowAttribute(hwnd, DWMWA_USE_IMMERSIVE_DARK_MODE as _, (&dark as *const i32).cast(), 4);
        DwmSetWindowAttribute(hwnd, DWMWA_CAPTION_COLOR as _, (&caption as *const u32).cast(), 4);
        DwmSetWindowAttribute(hwnd, DWMWA_TEXT_COLOR as _, (&text as *const u32).cast(), 4);
    }
}

#[cfg(not(windows))]
fn tint_title_bar(_window: &Window, _spec: &str) {}

#[cfg(windows)]
pub fn show_error(message: &str) {
    use std::os::windows::ffi::OsStrExt;
    #[link(name = "user32")]
    extern "system" {
        fn MessageBoxW(hwnd: *mut core::ffi::c_void, text: *const u16, caption: *const u16, kind: u32) -> i32;
    }
    let wide = |s: &str| -> Vec<u16> { std::ffi::OsStr::new(s).encode_wide().chain(Some(0)).collect() };
    let (text, caption) = (wide(message), wide("Deloist"));
    // MB_ICONERROR. Указатели живут до конца вызова.
    unsafe {
        MessageBoxW(core::ptr::null_mut(), text.as_ptr(), caption.as_ptr(), 0x10);
    }
}

#[cfg(not(windows))]
pub fn show_error(message: &str) {
    eprintln!("{message}");
}

/// Ссылка из ответа бота открывается в обычном браузере, а не внутри приложения с его сессией.
#[cfg(windows)]
pub(crate) fn open_in_browser(url: &str) {
    use std::os::windows::ffi::OsStrExt;
    #[link(name = "shell32")]
    extern "system" {
        fn ShellExecuteW(
            hwnd: *mut core::ffi::c_void,
            operation: *const u16,
            file: *const u16,
            parameters: *const u16,
            directory: *const u16,
            show: i32,
        ) -> *mut core::ffi::c_void;
    }
    // Только http(s): иначе ссылка могла бы запустить программу или открыть файл.
    let Ok(parsed) = reqwest::Url::parse(url) else { return };
    if !matches!(parsed.scheme(), "http" | "https") {
        return;
    }
    let wide = |s: &str| -> Vec<u16> { std::ffi::OsStr::new(s).encode_wide().chain(Some(0)).collect() };
    let (verb, target) = (wide("open"), wide(parsed.as_str()));
    // SW_SHOWNORMAL = 1. Строки живут до конца вызова.
    unsafe {
        ShellExecuteW(core::ptr::null_mut(), verb.as_ptr(), target.as_ptr(), core::ptr::null(), core::ptr::null(), 1);
    }
}

#[cfg(not(windows))]
pub(crate) fn open_in_browser(url: &str) {
    // Только http(s): иначе ссылка могла бы запустить программу или открыть файл.
    let Ok(parsed) = reqwest::Url::parse(url) else { return };
    if !matches!(parsed.scheme(), "http" | "https") || crate::cloud::active() {
        return;
    }
    let _ = std::process::Command::new("xdg-open")
        .arg(parsed.as_str())
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn();
}

/// Второй запуск: показываем окно уже работающей копии. Копия могла быть свёрнута в трей.
#[cfg(windows)]
pub fn activate_running_window() -> bool {
    use std::os::windows::ffi::OsStrExt;
    #[link(name = "user32")]
    extern "system" {
        fn FindWindowW(class: *const u16, title: *const u16) -> *mut core::ffi::c_void;
        fn ShowWindow(hwnd: *mut core::ffi::c_void, command: i32) -> i32;
        fn SetForegroundWindow(hwnd: *mut core::ffi::c_void) -> i32;
    }
    let wide = |text: &str| -> Vec<u16> { std::ffi::OsStr::new(text).encode_wide().chain(Some(0)).collect() };
    let (class, title) = (wide(WINDOW_CLASS), wide(WINDOW_TITLE));
    // SW_RESTORE = 9. Строки живут до конца вызова, дескриптор проверен на null.
    unsafe {
        let window = FindWindowW(class.as_ptr(), title.as_ptr());
        if window.is_null() {
            return false;
        }
        ShowWindow(window, 9);
        SetForegroundWindow(window);
    }
    true
}

#[cfg(not(windows))]
pub fn activate_running_window() -> bool {
    false
}

fn build_webview(
    window: &Window,
    context: &mut WebContext,
    url: &str,
    origin: &str,
    proxy: EventLoopProxy<UserEvent>,
) -> Result<WebView, String> {
    // Со слэшем: иначе адрес http://127.0.0.1:5000.evil.com тоже подошёл бы как «свой».
    let allowed = format!("{origin}/");
    let builder = WebViewBuilder::new_with_web_context(context)
        .with_url(url)
        // Единственное сообщение главного окна — цвет шапки; остальное отбрасывается.
        .with_ipc_handler(move |request: wry::http::Request<String>| {
            if let Some(spec) = request.body().strip_prefix("titlebar:") {
                let _ = proxy.send_event(UserEvent::TitleBar(spec.to_owned()));
            }
        })
        // Горячая клавиша голоса без дока начинает запись здесь, без щелчка по странице.
        .with_autoplay(true)
        .with_navigation_handler(move |target| target.starts_with(&allowed))
        .with_new_window_req_handler(|target, _| {
            open_in_browser(&target);
            wry::NewWindowResponse::Deny
        })
        .with_devtools(false);
    #[cfg(windows)]
    let built = builder.build(window);
    // На Linux WebKitGTK встраивается в контейнер окна tao: так он работает и под X11, и под Wayland.
    #[cfg(not(windows))]
    let built = {
        use tao::platform::unix::WindowExtUnix;
        use wry::WebViewBuilderExtUnix;
        let vbox = window.default_vbox().ok_or_else(|| "У окна нет контейнера GTK".to_string())?;
        builder.build_gtk(vbox)
    };
    built
        .inspect(|view| crate::voice::allow_microphone(view, origin))
        .map_err(|e| {
            if cfg!(windows) {
                format!("Не удалось открыть окно. Для работы нужен Microsoft Edge WebView2 Runtime.\n\n{e}")
            } else {
                format!("Не удалось открыть окно. Для работы нужен WebKitGTK (libwebkit2gtk-4.1).\n\n{e}")
            }
        })
}

/// Блокирует главный поток до выхода. `entry` выдаёт одноразовую ссылку для входа,
/// `on_exit` останавливает сервер.
pub fn run(
    entry: impl Fn() -> String + 'static,
    origin: String,
    webview_data: std::path::PathBuf,
    hidden: bool,
    english: bool,
    on_exit: impl Fn() + 'static,
) -> Result<(), String> {
    let mut event_loop = EventLoopBuilder::<UserEvent>::with_user_event().build();

    let proxy = event_loop.create_proxy();
    TrayIconEvent::set_event_handler(Some(move |event| {
        let _ = proxy.send_event(UserEvent::Tray(event));
    }));
    let proxy = event_loop.create_proxy();
    MenuEvent::set_event_handler(Some(move |event| {
        let _ = proxy.send_event(UserEvent::Menu(event));
    }));
    let title_proxy = event_loop.create_proxy();

    // Для проверок на машине, где человек занят своим: окно не забирает фокус.
    let take_focus = std::env::var_os("CREW_NO_FOCUS").is_none();
    let builder = WindowBuilder::new()
        .with_title(WINDOW_TITLE)
        .with_inner_size(LogicalSize::new(1200.0, 800.0))
        .with_min_inner_size(LogicalSize::new(720.0, 480.0))
        .with_window_icon(WindowIcon::from_rgba(icon_rgba(64), 64, 64).ok())
        .with_focused(take_focus)
        .with_visible(!hidden);
    #[cfg(windows)]
    let builder = {
        use tao::platform::windows::WindowBuilderExtWindows;
        builder.with_window_classname(WINDOW_CLASS)
    };
    let window = builder
        .build(&event_loop)
        .map_err(|e| e.to_string())?;

    let mut context = WebContext::new(Some(webview_data));
    let mut webview = if hidden {
        crate::system::release_idle_memory();
        None
    } else {
        Some(build_webview(&window, &mut context, &entry(), &origin, title_proxy.clone())?)
    };

    let menu = Menu::new();
    let open_item = MenuItem::new(if english { "Open Deloist" } else { "Открыть Deloist" }, true, None);
    let quit_item = MenuItem::new(if english { "Quit" } else { "Выйти" }, true, None);
    menu.append_items(&[&open_item, &PredefinedMenuItem::separator(), &quit_item])
        .map_err(|e| e.to_string())?;
    let tray = TrayIconBuilder::new()
        .with_menu(Box::new(menu))
        .with_menu_on_left_click(false)
        .with_tooltip(if english { "Deloist: bots keep working in the background" } else { "Deloist: боты работают в фоне" })
        .with_icon(TrayIconImage::from_rgba(icon_rgba(32), 32, 32).map_err(|e| e.to_string())?)
        .build()
        .map_err(|e| e.to_string())?;

    let (open_id, quit_id) = (open_item.id().clone(), quit_item.id().clone());
    let mut quit = false;
    loop {
        // Создание WebView само крутит сообщения Windows. Внутри обработчика это повторный вход в цикл
        // событий и падение, поэтому цикл останавливается, окно создаётся снаружи и цикл идёт дальше.
        let mut need_webview = false;
        event_loop.run_return(|event, _, control_flow| {
            let show = |window: &Window| {
                window.set_visible(true);
                window.set_minimized(false);
                window.set_focus();
            };
            match event {
                Event::WindowEvent { event: WindowEvent::CloseRequested, .. } => {
                    // WebView занимает почти всю память программы. В трее он не нужен: боты и рутины
                    // работают на сервере, а при открытии интерфейс просто загрузится заново.
                    window.set_visible(false);
                    webview = None;
                    crate::system::release_idle_memory();
                }
                Event::UserEvent(UserEvent::Tray(TrayIconEvent::Click {
                    button: MouseButton::Left,
                    button_state: MouseButtonState::Up,
                    ..
                })) => show(&window),
                Event::UserEvent(UserEvent::Tray(TrayIconEvent::DoubleClick { button: MouseButton::Left, .. })) => {
                    show(&window)
                }
                Event::UserEvent(UserEvent::Menu(item)) => {
                    if item.id == open_id {
                        show(&window);
                    } else if item.id == quit_id {
                        quit = true;
                        *control_flow = ControlFlow::Exit;
                        return;
                    }
                }
                Event::UserEvent(UserEvent::TitleBar(spec)) => tint_title_bar(&window, &spec),
                _ => {}
            }

            // Окно могли показать и снаружи: повторный запуск программы делает это через WinAPI.
            if webview.is_none() && window.is_visible() {
                // tao помнит видимость отдельно от Windows. Без этого следующее закрытие не спрячет окно:
                // оно решит, что и так скрыто, а WebView тут же создастся заново.
                window.set_visible(true);
                need_webview = true;
                *control_flow = ControlFlow::Exit;
                return;
            }
            // Без таймеров: показ окна снаружи и так приходит событием, а сработавший таймер
            // разбудил бы цикл посреди создания WebView, когда обработчика нет.
            *control_flow = ControlFlow::Wait;
        });
        if quit {
            break;
        }
        if need_webview {
            match build_webview(&window, &mut context, &entry(), &origin, title_proxy.clone()) {
                Ok(view) => webview = Some(view),
                Err(message) => {
                    window.set_visible(false);
                    show_error(&message);
                }
            }
        }
    }
    on_exit();
    // Значок убираем сами, пока цикл ещё жив: иначе он повисит в трее до наведения мыши.
    drop(tray);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::parse_title_colors;

    #[test]
    fn title_colors_are_parsed_as_colorref() {
        assert_eq!(parse_title_colors("27,27,27;244,244,244;1"), Some((0x1b1b1b, 0xf4f4f4, true)));
        assert_eq!(parse_title_colors("236,235,232;38,41,49;0"), Some((0x00e8ebec, 0x00312926, false)));
    }

    #[test]
    fn bad_title_colors_are_ignored() {
        for bad in ["", "1,2;3,4,5;1", "300,0,0;0,0,0;1", "1,2,3;4,5,6;2", "1,2,3;4,5,6;1;x", "a,b,c;1,2,3;0"] {
            assert_eq!(parse_title_colors(bad), None, "{bad}");
        }
    }
}
