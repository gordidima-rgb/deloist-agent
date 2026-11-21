//! Экран ПК на телефоне (0.4.10): просмотр, «Перехватить», «Вернуть ботам» и автопауза через 5 минут.
//!
//! Работает поверх E2E-конвертов `relay` без изменения сервера-почтальона: кадр снимается через
//! `capture::win::grab_screen()`, уменьшается до ширины ≤ 720 px и сжимается в PNG (base64 ≤ 180 КБ,
//! чтобы гарантированно влезать в конверт 256 КБ). Пока человек в режиме «Перехватить», компьютерные
//! и браузерные действия ботов ставятся на паузу; по кнопке «Вернуть ботам» или при остановке
//! трансляции боты снова продолжают работу. Через 5 минут без касаний трансляция автоматически
//! ставится на паузу ради трафика, а по касанию продолжается.

use std::sync::{Mutex, OnceLock};

#[cfg(all(windows, feature = "desktop"))]
use base64::Engine;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

/// Пауза трансляции ради трафика через 5 минут без касаний.
pub const IDLE_PAUSE_MS: i64 = 5 * 60 * 1000;
/// Максимальная ширина кадра для телефона (чтобы PNG + base64 влезали в 256 КБ конверт relay).
#[cfg(any(test, all(windows, feature = "desktop")))]
pub const MAX_WIDTH: usize = 720;
/// Предел закодированного PNG в байтах (~170 КБ base64).
#[cfg(any(test, all(windows, feature = "desktop")))]
pub const MAX_PNG_BYTES: usize = 128 * 1024;
/// Ограничение длины текста за один ввод с телефона.
pub const MAX_TEXT_CHARS: usize = 200;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Mode {
    Off,
    Watching,
    Takeover,
    Paused,
}

impl Mode {
    pub fn as_str(self) -> &'static str {
        match self {
            Mode::Off => "off",
            Mode::Watching => "watching",
            Mode::Takeover => "takeover",
            Mode::Paused => "paused",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Session {
    pub mode: Mode,
    pub device_id: String,
    pub device_name: String,
    pub last_touch_ms: i64,
    /// Предыдущий режим до автопаузы (Watching или Takeover), чтобы при возврате после паузы
    /// безопасно вернуться в просмотр (ввод с телефона после паузы не включается сам).
    pub resumed_mode: Mode,
}

impl Default for Session {
    fn default() -> Self {
        Self { mode: Mode::Off, device_id: String::new(), device_name: String::new(), last_touch_ms: 0, resumed_mode: Mode::Watching }
    }
}

impl Session {
    pub fn view(&self) -> Value {
        json!({
            "active": self.mode != Mode::Off,
            "mode": self.mode.as_str(),
            "deviceId": if self.mode == Mode::Off { "" } else { &self.device_id },
            "deviceName": if self.mode == Mode::Off { "" } else { &self.device_name },
            "botsPaused": self.mode == Mode::Takeover,
            "idlePauseMs": IDLE_PAUSE_MS,
        })
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum InputAction {
    Click { x: f64, y: f64, button: MouseButton, double: bool },
    Scroll { x: f64, y: f64, dy: i32 },
    Text { text: String },
    Key { key: SpecialKey },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MouseButton {
    Left,
    Right,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SpecialKey {
    Enter,
    Backspace,
    Escape,
    Tab,
    Up,
    Down,
    Left,
    Right,
    Space,
}

/// Разбор входного события с телефона. Координаты строго в `0.0..=1.0`, спецклавиши — только из белого списка.
pub fn parse_input(req: &Value) -> Result<InputAction, &'static str> {
    let kind = req["kind"].as_str().unwrap_or_default();
    let coord = |key: &str| -> Result<f64, &'static str> {
        let v = req[key].as_f64().ok_or("нужны координаты x и y")?;
        if !v.is_finite() || !(0.0..=1.0).contains(&v) {
            return Err("координаты должны быть от 0 до 1");
        }
        Ok(v)
    };
    match kind {
        "click" | "dblclick" | "rclick" => {
            let x = coord("x")?;
            let y = coord("y")?;
            let button = if kind == "rclick" { MouseButton::Right } else { MouseButton::Left };
            Ok(InputAction::Click { x, y, button, double: kind == "dblclick" })
        }
        "scroll" => {
            let x = coord("x")?;
            let y = coord("y")?;
            let dy = req["dy"].as_i64().unwrap_or(0).clamp(-1200, 1200) as i32;
            if dy == 0 {
                return Err("нужно направление прокрутки dy");
            }
            Ok(InputAction::Scroll { x, y, dy })
        }
        "text" => {
            let raw = req["text"].as_str().unwrap_or_default();
            let clean: String = raw.chars().filter(|c| !c.is_control() || *c == '\n').take(MAX_TEXT_CHARS).collect();
            if clean.is_empty() {
                return Err("пустой текст");
            }
            Ok(InputAction::Text { text: clean })
        }
        "key" => {
            let key = match req["key"].as_str().unwrap_or_default() {
                "Enter" => SpecialKey::Enter,
                "Backspace" => SpecialKey::Backspace,
                "Escape" => SpecialKey::Escape,
                "Tab" => SpecialKey::Tab,
                "ArrowUp" => SpecialKey::Up,
                "ArrowDown" => SpecialKey::Down,
                "ArrowLeft" => SpecialKey::Left,
                "ArrowRight" => SpecialKey::Right,
                "Space" => SpecialKey::Space,
                _ => return Err("неизвестная клавиша"),
            };
            Ok(InputAction::Key { key })
        }
        _ => Err("неизвестный вид ввода"),
    }
}

/// Состояние удалённого экрана на этом ПК (чистый автомат, легко проверяется в тестах).
#[derive(Default)]
pub struct Controller {
    session: Mutex<Session>,
}

impl Controller {
    pub fn snapshot(&self) -> Session {
        self.session.lock().unwrap().clone()
    }

    pub fn bots_paused(&self) -> bool {
        self.session.lock().unwrap().mode == Mode::Takeover
    }

    pub fn start(&self, device_id: &str, device_name: &str, now_ms: i64) -> Result<Session, &'static str> {
        let mut s = self.session.lock().unwrap();
        if s.mode != Mode::Off && s.device_id != device_id {
            return Err("экран уже смотрит другое устройство");
        }
        s.mode = Mode::Watching;
        s.resumed_mode = Mode::Watching;
        s.device_id = device_id.to_string();
        s.device_name = device_name.to_string();
        s.last_touch_ms = now_ms;
        Ok(s.clone())
    }

    pub fn stop(&self, device_id: Option<&str>) -> Session {
        let mut s = self.session.lock().unwrap();
        if let Some(id) = device_id {
            if s.mode != Mode::Off && s.device_id != id {
                return s.clone();
            }
        }
        *s = Session::default();
        s.clone()
    }

    pub fn takeover(&self, device_id: &str, now_ms: i64) -> Result<Session, &'static str> {
        let mut s = self.session.lock().unwrap();
        if s.mode == Mode::Off || s.device_id != device_id {
            return Err("сначала откройте просмотр экрана");
        }
        s.mode = Mode::Takeover;
        s.resumed_mode = Mode::Watching;
        s.last_touch_ms = now_ms;
        Ok(s.clone())
    }

    pub fn return_to_bots(&self, device_id: &str, now_ms: i64) -> Result<Session, &'static str> {
        let mut s = self.session.lock().unwrap();
        if s.mode == Mode::Off || s.device_id != device_id {
            return Err("экран не активен");
        }
        s.mode = Mode::Watching;
        s.resumed_mode = Mode::Watching;
        s.last_touch_ms = now_ms;
        Ok(s.clone())
    }

    /// Касание на телефоне: сбрасывает 5-минутный таймер и возобновляет трансляцию после паузы.
    pub fn touch(&self, device_id: &str, now_ms: i64) -> Result<(Session, bool), &'static str> {
        let mut s = self.session.lock().unwrap();
        if s.mode == Mode::Off || s.device_id != device_id {
            return Err("экран не активен");
        }
        let was_paused = s.mode == Mode::Paused;
        if was_paused {
            // После автопаузы возвращаемся в безопасный просмотр; перехват человек включает явно.
            s.mode = Mode::Watching;
        }
        s.last_touch_ms = now_ms;
        Ok((s.clone(), was_paused))
    }

    /// Проверка автопаузы по таймауту (5 минут без касаний). Возвращает `Some(session)`, если только что перешли в `Paused`.
    pub fn check_idle(&self, now_ms: i64) -> Option<Session> {
        let mut s = self.session.lock().unwrap();
        if matches!(s.mode, Mode::Watching | Mode::Takeover) && now_ms - s.last_touch_ms >= IDLE_PAUSE_MS {
            s.mode = Mode::Paused;
            return Some(s.clone());
        }
        None
    }

    /// Разрешён ли ввод с этого устройства прямо сейчас (только `Takeover` и не истёк таймаут).
    pub fn allow_input(&self, device_id: &str, now_ms: i64) -> Result<(), &'static str> {
        let mut s = self.session.lock().unwrap();
        if s.mode != Mode::Takeover || s.device_id != device_id {
            return Err("управление не перехвачено: нажмите «Перехватить»");
        }
        if now_ms - s.last_touch_ms >= IDLE_PAUSE_MS {
            s.mode = Mode::Paused;
            return Err("трансляция на паузе после 5 минут без касаний");
        }
        s.last_touch_ms = now_ms;
        Ok(())
    }
}

static GLOBAL: OnceLock<Controller> = OnceLock::new();

pub fn controller() -> &'static Controller {
    GLOBAL.get_or_init(Controller::default)
}

/// Подождать, пока человек вернёт управление ботам (или сработает отмена / 5-минутная автопауза).
pub async fn wait_released(cancel: &tokio_util::sync::CancellationToken) {
    let ctrl = controller();
    while ctrl.bots_paused() {
        let _ = ctrl.check_idle(crate::store::now());
        if !ctrl.bots_paused() {
            break;
        }
        tokio::select! {
            _ = cancel.cancelled() => break,
            _ = tokio::time::sleep(std::time::Duration::from_millis(200)) => {}
        }
    }
}

pub fn status_json() -> Value {
    controller().snapshot().view()
}

/// Уменьшить BGRA-кадр до ширины ≤ `max_w` (ближайший сосед) и закодировать в компактный PNG.
#[cfg(any(test, all(windows, feature = "desktop")))]
pub fn encode_frame_png(bgra: &[u8], src_w: usize, src_h: usize, max_w: usize) -> Result<(usize, usize, Vec<u8>), String> {
    if src_w == 0 || src_h == 0 || bgra.len() < src_w * src_h * 4 {
        return Err("пустой кадр экрана".into());
    }
    let scale = if src_w > max_w { max_w as f64 / src_w as f64 } else { 1.0 };
    let dst_w = ((src_w as f64 * scale).round() as usize).max(1);
    let dst_h = ((src_h as f64 * scale).round() as usize).max(1);
    let mut rgb = Vec::with_capacity(dst_w * dst_h * 3);
    for y in 0..dst_h {
        let sy = (y * src_h) / dst_h;
        for x in 0..dst_w {
            let sx = (x * src_w) / dst_w;
            let i = (sy * src_w + sx) * 4;
            // Лёгкое квантование по 4 уровня (маска 0xFC) делает PNG экрана в 2–3 раза компактнее без заметной потери читаемости.
            rgb.push(bgra[i + 2] & 0xFC);
            rgb.push(bgra[i + 1] & 0xFC);
            rgb.push(bgra[i] & 0xFC);
        }
    }
    let mut out = Vec::new();
    {
        let mut encoder = png::Encoder::new(&mut out, dst_w as u32, dst_h as u32);
        encoder.set_color(png::ColorType::Rgb);
        encoder.set_depth(png::BitDepth::Eight);
        encoder.set_compression(png::Compression::Fast);
        let mut writer = encoder.write_header().map_err(|e| e.to_string())?;
        writer.write_image_data(&rgb).map_err(|e| e.to_string())?;
    }
    Ok((dst_w, dst_h, out))
}

/// Снять текущий экран и упаковать в JSON для конверта `relay` (`{ mime, width, height, data }`).
/// Если кадр крупнее `MAX_PNG_BYTES`, автоматически пробует меньшую ширину (480 px).
pub fn capture_frame() -> Result<Value, String> {
    #[cfg(all(windows, feature = "desktop"))]
    {
        let screen = crate::capture::win::grab_screen().ok_or_else(|| "не удалось снять экран".to_string())?;
        let bounds = screen.bounds();
        let (src_w, src_h) = (bounds.width() as usize, bounds.height() as usize);
        let bgra = screen.pixels();
        for width in [MAX_WIDTH, 480, 360] {
            let (w, h, bytes) = encode_frame_png(bgra, src_w, src_h, width)?;
            if bytes.len() <= MAX_PNG_BYTES || width == 360 {
                let b64 = base64::engine::general_purpose::STANDARD.encode(&bytes);
                return Ok(json!({ "mime": "image/png", "width": w, "height": h, "data": b64 }));
            }
        }
        Err("кадр слишком большой".into())
    }
    #[cfg(not(all(windows, feature = "desktop")))]
    {
        Err("экран недоступен в серверной сборке без окна".into())
    }
}

/// Выполнить ввод с телефона на экране Windows (только в режиме `Takeover`).
pub fn perform_input(action: &InputAction) -> Result<(), String> {
    #[cfg(all(windows, feature = "desktop"))]
    {
        win_input::apply(action)
    }
    #[cfg(not(all(windows, feature = "desktop")))]
    {
        let _ = action;
        Err("управление экраном доступно только на ПК с окном".into())
    }
}

#[cfg(all(windows, feature = "desktop"))]
mod win_input {
    use super::{InputAction, MouseButton, SpecialKey};
    use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
        SendInput, INPUT, INPUT_0, INPUT_KEYBOARD, INPUT_MOUSE, KEYBDINPUT, KEYEVENTF_KEYUP, KEYEVENTF_UNICODE,
        MOUSEEVENTF_ABSOLUTE, MOUSEEVENTF_LEFTDOWN, MOUSEEVENTF_LEFTUP, MOUSEEVENTF_MOVE, MOUSEEVENTF_RIGHTDOWN,
        MOUSEEVENTF_RIGHTUP, MOUSEEVENTF_VIRTUALDESK, MOUSEEVENTF_WHEEL, MOUSEINPUT, VK_BACK, VK_DOWN, VK_ESCAPE,
        VK_LEFT, VK_RETURN, VK_RIGHT, VK_SPACE, VK_TAB, VK_UP,
    };

    fn send(inputs: &[INPUT]) -> Result<(), String> {
        let sent = unsafe { SendInput(inputs.len() as u32, inputs.as_ptr(), std::mem::size_of::<INPUT>() as i32) };
        if sent == 0 {
            Err("Windows не приняла ввод (возможно, активное окно запущено от администратора)".into())
        } else {
            Ok(())
        }
    }

    fn mouse_abs(x: f64, y: f64, flags: u32, wheel: i32) -> INPUT {
        INPUT {
            r#type: INPUT_MOUSE,
            Anonymous: INPUT_0 {
                mi: MOUSEINPUT {
                    dx: (x.clamp(0.0, 1.0) * 65535.0).round() as i32,
                    dy: (y.clamp(0.0, 1.0) * 65535.0).round() as i32,
                    mouseData: wheel as u32,
                    dwFlags: flags | MOUSEEVENTF_ABSOLUTE | MOUSEEVENTF_VIRTUALDESK,
                    time: 0,
                    dwExtraInfo: 0,
                },
            },
        }
    }

    fn key_vk(vk: u16, up: bool) -> INPUT {
        INPUT {
            r#type: INPUT_KEYBOARD,
            Anonymous: INPUT_0 {
                ki: KEYBDINPUT { wVk: vk, wScan: 0, dwFlags: if up { KEYEVENTF_KEYUP } else { 0 }, time: 0, dwExtraInfo: 0 },
            },
        }
    }

    pub fn apply(action: &InputAction) -> Result<(), String> {
        match action {
            InputAction::Click { x, y, button, double } => {
                let (down, up) = match button {
                    MouseButton::Left => (MOUSEEVENTF_LEFTDOWN, MOUSEEVENTF_LEFTUP),
                    MouseButton::Right => (MOUSEEVENTF_RIGHTDOWN, MOUSEEVENTF_RIGHTUP),
                };
                let mut seq = vec![mouse_abs(*x, *y, MOUSEEVENTF_MOVE, 0), mouse_abs(*x, *y, down, 0), mouse_abs(*x, *y, up, 0)];
                if *double {
                    seq.push(mouse_abs(*x, *y, down, 0));
                    seq.push(mouse_abs(*x, *y, up, 0));
                }
                send(&seq)
            }
            InputAction::Scroll { x, y, dy } => {
                let seq = [mouse_abs(*x, *y, MOUSEEVENTF_MOVE, 0), mouse_abs(*x, *y, MOUSEEVENTF_WHEEL, *dy)];
                send(&seq)
            }
            InputAction::Text { text } => {
                let mut seq = Vec::new();
                for ch in text.encode_utf16() {
                    if ch == b'\n' as u16 {
                        seq.push(key_vk(VK_RETURN, false));
                        seq.push(key_vk(VK_RETURN, true));
                    } else {
                        seq.push(INPUT {
                            r#type: INPUT_KEYBOARD,
                            Anonymous: INPUT_0 { ki: KEYBDINPUT { wVk: 0, wScan: ch, dwFlags: KEYEVENTF_UNICODE, time: 0, dwExtraInfo: 0 } },
                        });
                        seq.push(INPUT {
                            r#type: INPUT_KEYBOARD,
                            Anonymous: INPUT_0 { ki: KEYBDINPUT { wVk: 0, wScan: ch, dwFlags: KEYEVENTF_UNICODE | KEYEVENTF_KEYUP, time: 0, dwExtraInfo: 0 } },
                        });
                    }
                }
                if seq.is_empty() { Ok(()) } else { send(&seq) }
            }
            InputAction::Key { key } => {
                let vk = match key {
                    SpecialKey::Enter => VK_RETURN,
                    SpecialKey::Backspace => VK_BACK,
                    SpecialKey::Escape => VK_ESCAPE,
                    SpecialKey::Tab => VK_TAB,
                    SpecialKey::Up => VK_UP,
                    SpecialKey::Down => VK_DOWN,
                    SpecialKey::Left => VK_LEFT,
                    SpecialKey::Right => VK_RIGHT,
                    SpecialKey::Space => VK_SPACE,
                };
                send(&[key_vk(vk, false), key_vk(vk, true)])
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lifecycle_takeover_return_and_five_minute_idle_pause() {
        let c = Controller::default();
        assert!(!c.bots_paused());

        // Старт просмотра: боты не на паузе, ввод запрещён.
        let s = c.start("dev1", "iPhone", 1_000).unwrap();
        assert_eq!(s.mode, Mode::Watching);
        assert!(!c.bots_paused());
        assert!(c.allow_input("dev1", 1_100).is_err());
        assert!(c.start("dev2", "iPad", 1_200).is_err(), "второе устройство не перебивает активную сессию");

        // Перехват: боты на паузе, ввод разрешён только этому устройству.
        let s = c.takeover("dev1", 2_000).unwrap();
        assert_eq!(s.mode, Mode::Takeover);
        assert!(c.bots_paused());
        assert!(c.allow_input("dev1", 2_500).is_ok());
        assert!(c.allow_input("dev2", 2_600).is_err());

        // 4 мин 59 с без касаний — ещё активно; 5 минут — автопауза, боты сняты с паузы.
        assert!(c.check_idle(2_500 + IDLE_PAUSE_MS - 1).is_none());
        let paused = c.check_idle(2_500 + IDLE_PAUSE_MS).expect("должна сработать автопауза");
        assert_eq!(paused.mode, Mode::Paused);
        assert!(!c.bots_paused());
        assert!(c.allow_input("dev1", 2_500 + IDLE_PAUSE_MS + 10).is_err());

        // Касание после паузы возвращает просмотр (но не автоматический перехват).
        let (resumed, was_paused) = c.touch("dev1", 400_000).unwrap();
        assert!(was_paused);
        assert_eq!(resumed.mode, Mode::Watching);
        assert!(!c.bots_paused());

        // Перехват → «Вернуть ботам» → Стоп.
        c.takeover("dev1", 401_000).unwrap();
        assert!(c.bots_paused());
        let back = c.return_to_bots("dev1", 402_000).unwrap();
        assert_eq!(back.mode, Mode::Watching);
        assert!(!c.bots_paused());
        let off = c.stop(Some("dev1"));
        assert_eq!(off.mode, Mode::Off);
    }

    #[test]
    fn input_validation_rejects_out_of_bounds_and_unknown_keys() {
        assert!(parse_input(&json!({ "kind": "click", "x": 0.5, "y": 0.25 })).is_ok());
        assert!(parse_input(&json!({ "kind": "click", "x": -0.1, "y": 0.5 })).is_err());
        assert!(parse_input(&json!({ "kind": "click", "x": 1.2, "y": 0.5 })).is_err());
        assert!(parse_input(&json!({ "kind": "key", "key": "Enter" })).is_ok());
        assert!(parse_input(&json!({ "kind": "key", "key": "Win" })).is_err());
        assert!(parse_input(&json!({ "kind": "text", "text": "привет" })).is_ok());
        assert!(parse_input(&json!({ "kind": "text", "text": "\u{0000}" })).is_err());
    }

    #[test]
    fn downscaled_frame_encodes_to_compact_png() {
        let (w, h) = (1920, 1080);
        let mut bgra = vec![0u8; w * h * 4];
        for y in 0..h {
            for x in 0..w {
                let i = (y * w + x) * 4;
                bgra[i] = (x / 16) as u8;
                bgra[i + 1] = (y / 16) as u8;
                bgra[i + 2] = 180;
                bgra[i + 3] = 255;
            }
        }
        let (dw, dh, png) = encode_frame_png(&bgra, w, h, MAX_WIDTH).unwrap();
        assert_eq!(dw, MAX_WIDTH);
        assert_eq!(dh, 405);
        assert!(png.len() < MAX_PNG_BYTES, "png too large: {}", png.len());
    }
}
