//! Аутентификация канала «Мой Chrome» (протокол v1).

use crate::native_host::hmac_sha256;
use serde_json::{json, Value};

pub const DIR_APP_TO_EXT: u8 = 0x01;
pub const DIR_EXT_TO_APP: u8 = 0x02;

/// PBKDF2-HMAC-SHA256 поверх существующего hmac_sha256.
pub fn pbkdf2_sha256(password: &[u8], salt: &[u8], iterations: u32, out: &mut [u8]) {
    let blocks = (out.len() + 31) / 32;
    for block in 1..=blocks {
        let mut u = Vec::with_capacity(salt.len() + 4);
        u.extend_from_slice(salt);
        u.extend_from_slice(&(block as u32).to_be_bytes());
        let mut t = hmac_sha256(password, &u);
        let mut prev = t;
        for _ in 2..=iterations {
            prev = hmac_sha256(password, &prev);
            for i in 0..32 {
                t[i] ^= prev[i];
            }
        }
        let start = (block - 1) * 32;
        let end = (start + 32).min(out.len());
        out[start..end].copy_from_slice(&t[..end - start]);
    }
}

pub fn derive_k(code: &str) -> [u8; 32] {
    // Нормализация: lowercase, одинарные пробелы.
    let mut collapsed = String::new();
    let mut prev_space = false;
    for c in code.trim().to_lowercase().chars() {
        if c.is_whitespace() {
            if !prev_space {
                collapsed.push(' ');
            }
            prev_space = true;
        } else {
            collapsed.push(c);
            prev_space = false;
        }
    }
    let password = collapsed.trim().as_bytes();
    let salt = b"deloist-user-chrome-v1";
    let mut out = [0u8; 32];
    pbkdf2_sha256(password, salt, 200_000, &mut out);
    out
}

pub fn session_key(k: &[u8], salt_app: &[u8], salt_ext: &[u8]) -> [u8; 32] {
    let mut msg = Vec::with_capacity(7 + salt_app.len() + salt_ext.len());
    msg.extend_from_slice(b"session");
    msg.extend_from_slice(salt_app);
    msg.extend_from_slice(salt_ext);
    hmac_sha256(k, &msg)
}

pub fn frame_mac(s: &[u8], dir: u8, n: u64, body: &str) -> [u8; 32] {
    let body_bytes = body.as_bytes();
    let mut msg = Vec::with_capacity(1 + 8 + body_bytes.len());
    msg.push(dir);
    msg.extend_from_slice(&n.to_be_bytes());
    msg.extend_from_slice(body_bytes);
    hmac_sha256(s, &msg)
}

pub fn hex_encode(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{:02x}", b)).collect()
}

pub fn hex_decode(s: &str) -> Option<Vec<u8>> {
    crate::native_host::hex_decode(s)
}


/// Сравнение без раннего выхода: время не подсказывает, сколько байт совпало.
pub fn ct_eq(a: &[u8], b: &[u8]) -> bool {
    a.len() == b.len() && a.iter().zip(b.iter()).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

/// Соль пира: ровно 32 hex-символа (16 байт).
fn parse_salt(value: Option<&Value>) -> Option<Vec<u8>> {
    let text = value?.as_str()?;
    if text.len() != 32 {
        return None;
    }
    hex_decode(text).filter(|bytes| bytes.len() == 16)
}

/// Что делать с входящим сообщением.
#[derive(Debug, PartialEq)]
pub enum Action {
    /// Молча пропустить.
    Ignore,
    /// Закрыть подключение.
    Close,
    /// Проверенное сообщение пира (в том числе его первый `ready`).
    Deliver(Value),
    /// Сессионный ключ выведен: пора отправить свой `ready` (через `seal`).
    SessionReady,
}

/// Состояние одного подключения: рукопожатие, ключ сессии, счётчики кадров.
/// Каждое подключение начинается с нуля; ничего из этого не хранится между подключениями.
pub struct Conn {
    k: Option<[u8; 32]>,
    local_salt: [u8; 16],
    remote_salt: Option<Vec<u8>>,
    s: Option<[u8; 32]>,
    last_in: u64,
    out_n: u64,
    hello_seen: bool,
    local_ready_sent: bool,
    peer_ready: bool,
    dir_in: u8,
    dir_out: u8,
    /// Hello пира целиком (версия, браузер): он не заверен, годится только для показа.
    pub peer_hello: Option<Value>,
    /// Последний status пира (например `paired:false`): не заверен, только для показа.
    pub peer_status: Option<Value>,
}

impl Conn {
    pub fn new(k: Option<[u8; 32]>, local_salt: [u8; 16], dir_in: u8, dir_out: u8) -> Self {
        Conn { k, local_salt, remote_salt: None, s: None, last_in: 0, out_n: 1, hello_seen: false, local_ready_sent: false, peer_ready: false, dir_in, dir_out, peer_hello: None, peer_status: None }
    }

    /// Сторона программы: принимает кадры расширения, шлёт свои с меткой app→ext.
    pub fn app(k: Option<[u8; 32]>, local_salt: [u8; 16]) -> Self {
        Self::new(k, local_salt, DIR_EXT_TO_APP, DIR_APP_TO_EXT)
    }

    pub fn is_paired(&self) -> bool {
        self.k.is_some()
    }

    /// Пир прислал верный `ready`: подключением можно пользоваться.
    pub fn is_ready(&self) -> bool {
        self.s.is_some() && self.peer_ready
    }

    pub fn hello_text(&self) -> String {
        json!({ "type": "hello", "salt": hex_encode(&self.local_salt) }).to_string()
    }

    pub fn on_message(&mut self, raw: &str) -> Action {
        let Ok(value) = serde_json::from_str::<Value>(raw) else {
            return Action::Close;
        };
        let Some(obj) = value.as_object() else {
            return Action::Close;
        };

        if obj.get("type").and_then(Value::as_str) == Some("hello") {
            if self.hello_seen {
                return Action::Close;
            }
            self.hello_seen = true;
            let Some(salt) = parse_salt(obj.get("salt")) else {
                return Action::Close;
            };
            self.peer_hello = Some(value.clone());
            self.remote_salt = Some(salt.clone());
            let Some(k) = self.k else {
                return Action::Ignore;
            };
            let s = if self.dir_out == DIR_APP_TO_EXT {
                session_key(&k, &self.local_salt, &salt)
            } else {
                session_key(&k, &salt, &self.local_salt)
            };
            self.s = Some(s);
            return Action::SessionReady;
        }

        // status без подписи — справка для человека: его только запоминаем, ни закрывать, ни действовать.
        if obj.get("type").and_then(Value::as_str) == Some("status") {
            self.peer_status = Some(value.clone());
            return Action::Ignore;
        }

        // Без пары обрабатываются только hello и status.
        if self.k.is_none() {
            return Action::Ignore;
        }
        let Some(s) = self.s else {
            return Action::Close;
        };

        // Дальше только кадры {n, body, mac}; всё остальное рвёт связь.
        let (Some(n), Some(body), Some(mac_hex)) =
            (obj.get("n").and_then(Value::as_u64), obj.get("body").and_then(Value::as_str), obj.get("mac").and_then(Value::as_str))
        else {
            return Action::Close;
        };
        let Some(mac) = hex_decode(mac_hex).filter(|m| m.len() == 32) else {
            return Action::Close;
        };
        if self.last_in.checked_add(1) != Some(n) {
            return Action::Close;
        }
        let calc = frame_mac(&s, self.dir_in, n, body);
        if !ct_eq(&calc, &mac) {
            return Action::Close;
        }
        self.last_in = n;
        let Ok(parsed) = serde_json::from_str::<Value>(body) else {
            return Action::Close;
        };
        if parsed.get("type").and_then(Value::as_str) == Some("ready") {
            if self.peer_ready {
                return Action::Ignore;
            }
            self.peer_ready = true;
            return Action::Deliver(parsed);
        }
        if !self.peer_ready {
            return Action::Ignore;
        }
        Action::Deliver(parsed)
    }

    /// Кадр для отправки: n назначается здесь, поэтому вызывать под тем же замком, что и отправку.
    /// До собственного `ready` уходит только `ready`.
    pub fn seal(&mut self, message: &Value) -> Option<String> {
        let s = self.s?;
        let is_ready = message.get("type").and_then(Value::as_str) == Some("ready");
        if !is_ready && !self.local_ready_sent {
            return None;
        }
        let n = self.out_n;
        self.out_n = n.checked_add(1)?;
        let body = message.to_string();
        let mac = hex_encode(&frame_mac(&s, self.dir_out, n, &body));
        if is_ready {
            self.local_ready_sent = true;
        }
        Some(json!({ "n": n, "body": body, "mac": mac }).to_string())
    }

    /// Кадр не ушёл (не влез в предел): вернуть номер, чтобы у пира не появилась дыра.
    pub fn undo_seal(&mut self) {
        self.out_n = self.out_n.saturating_sub(1).max(1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hmac_sha256_rfc4231_case1() {
        let key = vec![0x0bu8; 20];
        let mac = hmac_sha256(&key, b"Hi There");
        assert_eq!(hex_encode(&mac), "b0344c61d8db38535ca8afceaf0bf12b881dc200c9833da726e9376c2e32cff7");
    }

    #[test]
    fn pbkdf2_known_answers() {
        let mut out = [0u8; 32];
        pbkdf2_sha256(b"password", b"salt", 1, &mut out);
        assert_eq!(hex_encode(&out), "120fb6cffcf8b32c43e7225256c4f837a86548c92ccc35480805987cb70be17b");
        pbkdf2_sha256(b"password", b"salt", 2, &mut out);
        assert_eq!(hex_encode(&out), "ae4d0c95af6b46d32d0adff928f06dd02a303f8ef3c251dfd6e2d85a95474c43");
    }

    #[test]
    fn vectors_match() {
        let raw = include_str!("../extension/protocol-vectors.json");
        let v: Value = serde_json::from_str(raw).unwrap();
        for case in v["cases"].as_array().unwrap() {
            let k = derive_k(case["code"].as_str().unwrap());
            assert_eq!(hex_encode(&k), case["K"].as_str().unwrap(), "K mismatch");
            let salt_app = hex_decode(case["salt_app"].as_str().unwrap()).unwrap();
            let salt_ext = hex_decode(case["salt_ext"].as_str().unwrap()).unwrap();
            let s = session_key(&k, &salt_app, &salt_ext);
            assert_eq!(hex_encode(&s), case["S"].as_str().unwrap(), "S mismatch");
            let mac = frame_mac(&s, case["dir"].as_u64().unwrap() as u8, case["n"].as_u64().unwrap(), case["body"].as_str().unwrap());
            assert_eq!(hex_encode(&mac), case["mac"].as_str().unwrap(), "mac mismatch");
        }
    }

    const KEY: [u8; 32] = [7u8; 32];

    fn ext_conn(k: Option<[u8; 32]>, salt: u8) -> Conn {
        Conn::new(k, [salt; 16], DIR_APP_TO_EXT, DIR_EXT_TO_APP)
    }

    /// Обе стороны обменялись hello и своими ready.
    fn ready_pair() -> (Conn, Conn) {
        let mut app = Conn::app(Some(KEY), [1u8; 16]);
        let mut ext = ext_conn(Some(KEY), 2);
        assert_eq!(app.on_message(&ext.hello_text()), Action::SessionReady);
        assert_eq!(ext.on_message(&app.hello_text()), Action::SessionReady);
        let app_ready = app.seal(&json!({ "type": "ready" })).unwrap();
        let ext_ready = ext.seal(&json!({ "type": "ready" })).unwrap();
        assert!(matches!(ext.on_message(&app_ready), Action::Deliver(_)));
        assert!(matches!(app.on_message(&ext_ready), Action::Deliver(_)));
        assert!(app.is_ready() && ext.is_ready());
        (app, ext)
    }

    #[test]
    fn a_real_round_trip_both_ways() {
        let (mut app, mut ext) = ready_pair();
        let to_ext = app.seal(&json!({ "type": "state", "bots": [] })).unwrap();
        assert_eq!(ext.on_message(&to_ext), Action::Deliver(json!({ "type": "state", "bots": [] })));
        let to_app = ext.seal(&json!({ "type": "cdp", "bot": "b1" })).unwrap();
        assert_eq!(app.on_message(&to_app), Action::Deliver(json!({ "type": "cdp", "bot": "b1" })));
        let more = ext.seal(&json!({ "type": "stopped" })).unwrap();
        assert_eq!(app.on_message(&more), Action::Deliver(json!({ "type": "stopped" })));
    }

    #[test]
    fn a_second_hello_closes() {
        let mut app = Conn::app(Some(KEY), [1u8; 16]);
        let ext = ext_conn(Some(KEY), 2);
        assert_eq!(app.on_message(&ext.hello_text()), Action::SessionReady);
        assert_eq!(app.on_message(&ext.hello_text()), Action::Close);
        // и без соли тоже
        let mut app = Conn::app(Some(KEY), [1u8; 16]);
        assert_eq!(app.on_message(&ext.hello_text()), Action::SessionReady);
        assert_eq!(app.on_message(r#"{"type":"hello"}"#), Action::Close);
    }

    #[test]
    fn a_frame_before_ready_is_ignored() {
        let mut app = Conn::app(Some(KEY), [1u8; 16]);
        let mut ext = ext_conn(Some(KEY), 2);
        app.on_message(&ext.hello_text());
        ext.on_message(&app.hello_text());
        // ext ещё не прислал ready, а сразу шлёт cdp: seal не даёт, собираем кадр руками
        assert!(ext.seal(&json!({ "type": "cdp" })).is_none(), "до своего ready уходит только ready");
        let s = session_key(&KEY, &[1u8; 16], &[2u8; 16]);
        let body = json!({ "type": "cdp" }).to_string();
        let mac = hex_encode(&frame_mac(&s, DIR_EXT_TO_APP, 1, &body));
        let frame = json!({ "n": 1, "body": body, "mac": mac }).to_string();
        assert_eq!(app.on_message(&frame), Action::Ignore);
        assert!(!app.is_ready());
    }

    #[test]
    fn unpaired_processes_only_hello_and_status() {
        let mut app = Conn::app(None, [1u8; 16]);
        let ext = ext_conn(Some(KEY), 2);
        assert_eq!(app.on_message(&ext.hello_text()), Action::Ignore);
        assert_eq!(app.on_message(r#"{"type":"status","paired":false}"#), Action::Ignore);
        assert_eq!(app.on_message(r#"{"type":"cdp","bot":"b"}"#), Action::Ignore);
        assert_eq!(app.on_message(r#"{"n":1,"body":"{}","mac":"00"}"#), Action::Ignore);
        assert!(!app.is_ready());
        assert!(app.seal(&json!({ "type": "ready" })).is_none(), "без пары кадров нет");
    }

    #[test]
    fn a_bad_mac_closes() {
        let (mut app, mut ext) = ready_pair();
        let frame = ext.seal(&json!({ "type": "cdp" })).unwrap();
        let mut value: Value = serde_json::from_str(&frame).unwrap();
        value["body"] = json!(r#"{"type":"stopped"}"#);
        assert_eq!(app.on_message(&value.to_string()), Action::Close);
    }

    #[test]
    fn a_reflected_frame_closes() {
        let (mut app, _ext) = ready_pair();
        let own = app.seal(&json!({ "type": "state" })).unwrap();
        assert_eq!(app.on_message(&own), Action::Close, "своя метка направления не годится для входящего");
    }

    #[test]
    fn n_out_of_order_closes() {
        let (mut app, mut ext) = ready_pair();
        let first = ext.seal(&json!({ "type": "cdp", "i": 1 })).unwrap();
        let _skipped = ext.seal(&json!({ "type": "cdp", "i": 2 })).unwrap();
        let third = ext.seal(&json!({ "type": "cdp", "i": 3 })).unwrap();
        assert!(matches!(app.on_message(&first), Action::Deliver(_)));
        assert_eq!(app.on_message(&third), Action::Close, "n пропущен");

        let (mut app, mut ext) = ready_pair();
        let one = ext.seal(&json!({ "type": "cdp" })).unwrap();
        assert!(matches!(app.on_message(&one), Action::Deliver(_)));
        assert_eq!(app.on_message(&one), Action::Close, "n повторён");
    }

    #[test]
    fn an_invalid_salt_closes() {
        for hello in [
            r#"{"type":"hello","salt":"0102"}"#,
            r#"{"type":"hello","salt":"0102030405060708"}"#,
            r#"{"type":"hello","salt":"zz02030405060708090a0b0c0d0e0f10"}"#,
            r#"{"type":"hello","salt":"0102030405060708090a0b0c0d0e0f1011"}"#,
            r#"{"type":"hello","salt":5}"#,
            r#"{"type":"hello"}"#,
        ] {
            let mut app = Conn::app(Some(KEY), [1u8; 16]);
            assert_eq!(app.on_message(hello), Action::Close, "{hello}");
        }
    }

    #[test]
    fn status_after_hello_is_shown_not_acted_on() {
        let mut app = Conn::app(Some(KEY), [1u8; 16]);
        let mut ext = ext_conn(Some(KEY), 2);
        app.on_message(&ext.hello_text());
        assert_eq!(app.on_message(r#"{"type":"status","paired":false}"#), Action::Ignore);
        assert_eq!(app.peer_status, Some(json!({ "type": "status", "paired": false })));
        assert!(!app.is_ready());
        // связь при этом продолжает работать
        ext.on_message(&app.hello_text());
        let ready = ext.seal(&json!({ "type": "ready" })).unwrap();
        assert!(matches!(app.on_message(&ready), Action::Deliver(_)));
    }

    #[test]
    fn a_non_frame_message_after_hello_closes_when_paired() {
        let mut app = Conn::app(Some(KEY), [1u8; 16]);
        let ext = ext_conn(Some(KEY), 2);
        app.on_message(&ext.hello_text());
        assert_eq!(app.on_message(r#"{"type":"cdp","bot":"b"}"#), Action::Close);
    }
}
