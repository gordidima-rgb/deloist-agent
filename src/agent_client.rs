//! Клиент живой копии: общий для командной строки и MCP-сервера.
//!
//! Главное здесь — `ask`: дождаться ответа именно на это сообщение. Отметка «бот свободен»
//! этого не гарантирует: группа становится занятой не сразу (сначала выбирает, кому отвечать),
//! а бот освобождается чуть раньше, чем его ответ попадает в тред.

use std::path::Path;
use std::time::{Duration, Instant};

use serde_json::{json, Value};

use crate::runtime;

const REQUEST_TIMEOUT: Duration = Duration::from_secs(30);
const POLL: Duration = Duration::from_millis(400);
/// Сколько последних записей треда смотреть. Один ход столько не пишет.
const THREAD_WINDOW: usize = 200;

pub enum Error {
    NotRunning,
    Timeout(String),
    Failed(String),
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Error::NotRunning => f.write_str("Deloist не запущен: открой приложение и повтори"),
            Error::Timeout(message) | Error::Failed(message) => f.write_str(message),
        }
    }
}

impl From<Error> for String {
    fn from(error: Error) -> Self {
        error.to_string()
    }
}

/// Кому пишем: у бота и группы разные адреса и разное понятие «занят».
#[derive(Clone, Copy)]
pub enum Target<'a> {
    Bot(&'a str),
    Room(&'a str),
}

impl Target<'_> {
    fn base(&self) -> String {
        match self {
            Target::Bot(id) => format!("/api/bots/{id}"),
            Target::Room(id) => format!("/api/rooms/{id}"),
        }
    }
}

pub struct Answer {
    pub text: Option<String>,
    /// Ход закончился ошибкой: сбой модели, а не ответ по существу.
    pub failed: bool,
}

pub struct Client {
    base: String,
    token: String,
    http: reqwest::blocking::Client,
}

impl Client {
    /// `root` — папка приложения: копия живёт в той же папке данных, что и эта программа.
    pub fn open(root: &Path) -> Result<Self, Error> {
        Self::open_with_timeout(root, REQUEST_TIMEOUT)
    }

    pub fn open_with_timeout(root: &Path, timeout: Duration) -> Result<Self, Error> {
        let published = runtime::read(&crate::data_dir(root)).ok_or(Error::NotRunning)?;
        let http = reqwest::blocking::Client::builder()
            .timeout(timeout)
            // Адрес всегда 127.0.0.1: системный прокси здесь только помешает.
            .no_proxy()
            .build()
            .map_err(|e| Error::Failed(e.to_string()))?;
        Ok(Self { base: format!("http://127.0.0.1:{}", published.port), token: published.token, http })
    }

    pub fn get(&self, path: &str) -> Result<Value, Error> {
        self.call(reqwest::Method::GET, path, None, &[])
    }

    pub fn post(&self, path: &str, body: Value) -> Result<Value, Error> {
        self.call(reqwest::Method::POST, path, Some(body), &[])
    }

    /// То же с добавочными заголовками.
    pub fn post_with(&self, path: &str, body: Value, headers: &[(&str, &str)]) -> Result<Value, Error> {
        self.call(reqwest::Method::POST, path, Some(body), headers)
    }

    /// Содержимое файла как есть: картинку нельзя пропускать через текст.
    pub fn get_bytes(&self, path: &str) -> Result<Vec<u8>, Error> {
        let response = self.send(reqwest::Method::GET, path, None, &[])?;
        let status = response.status().as_u16();
        if !(200..300).contains(&status) {
            return Err(Error::Failed(error_text(status, &response.text().unwrap_or_default())));
        }
        let bytes = response.bytes().map_err(|e| Error::Failed(format!("приложение не ответило: {e}")))?;
        Ok(bytes.to_vec())
    }

    fn send(&self, method: reqwest::Method, path: &str, body: Option<Value>, headers: &[(&str, &str)]) -> Result<reqwest::blocking::Response, Error> {
        let mut request = self.http.request(method, format!("{}{}", self.base, path)).bearer_auth(&self.token);
        for (name, value) in headers {
            request = request.header(*name, *value);
        }
        if let Some(body) = body {
            request = request.header("content-type", "application/json").body(body.to_string());
        }
        request.send().map_err(|e| Error::Failed(format!("приложение не ответило: {e}")))
    }

    fn call(&self, method: reqwest::Method, path: &str, body: Option<Value>, headers: &[(&str, &str)]) -> Result<Value, Error> {
        let response = self.send(method, path, body, headers)?;
        let status = response.status().as_u16();
        let text = response.text().unwrap_or_default();
        if !(200..300).contains(&status) {
            return Err(Error::Failed(error_text(status, &text)));
        }
        Ok(serde_json::from_str(&text).unwrap_or_else(|_| json!({ "raw": text })))
    }

    fn busy(&self, target: Target) -> Result<bool, Error> {
        let state = self.get("/api/state")?;
        let (list, id) = match target {
            Target::Bot(id) => ("bots", id),
            Target::Room(id) => ("rooms", id),
        };
        let item = state[list].as_array().and_then(|items| items.iter().find(|item| item["id"] == id));
        Ok(item.is_some_and(|item| item["busy"].as_bool().unwrap_or(false) || item["queued"].as_i64().unwrap_or(0) > 0))
    }

    /// Дождаться, пока бот закончит ход и разберёт очередь.
    pub fn wait_idle(&self, target: Target, timeout: u64) -> Result<(), Error> {
        let started = Instant::now();
        while self.busy(target)? {
            if started.elapsed() > Duration::from_secs(timeout) {
                return Err(Error::Timeout(format!("бот не закончил за {timeout} с")));
            }
            std::thread::sleep(POLL);
        }
        Ok(())
    }

    /// Написать и дождаться ответа на это самое сообщение.
    pub fn ask(&self, target: Target, text: &str, timeout: u64) -> Result<Answer, Error> {
        let base = target.base();
        let latest = self.get(&format!("{base}/thread?limit=1"))?;
        let before = latest["messages"].as_array().and_then(|m| m.last()).and_then(|m| m["id"].as_str()).map(str::to_string);
        self.post(&format!("{base}/messages"), json!({ "text": text }))?;

        let started = Instant::now();
        let mut settled: Option<usize> = None;
        loop {
            if !self.busy(target)? {
                let thread = self.get(&format!("{base}/thread?limit={THREAD_WINDOW}"))?;
                let messages = thread["messages"].as_array().cloned().unwrap_or_default();
                if let Some(reply) = reply_entries(&messages, before.as_deref(), text) {
                    // Записи хода ложатся в тред одна за другой: берём, когда их число перестало меняться.
                    if settled == Some(reply.len()) {
                        return Ok(summary(reply));
                    }
                    settled = Some(reply.len());
                }
            } else {
                settled = None;
            }
            if started.elapsed() > Duration::from_secs(timeout) {
                return Err(Error::Timeout(format!("не дождались ответа за {timeout} с: возможно, бот ждёт подтверждения в приложении")));
            }
            std::thread::sleep(POLL);
        }
    }
}

fn error_text(status: u16, body: &str) -> String {
    serde_json::from_str::<Value>(body)
        .ok()
        .and_then(|data| data["error"].as_str().map(str::to_string))
        .unwrap_or_else(|| format!("HTTP {status}"))
}

/// Записи ответа: всё после нашего сообщения. `None` — ответа ещё нет.
fn reply_entries<'a>(messages: &'a [Value], before: Option<&str>, text: &str) -> Option<&'a [Value]> {
    // Отметки может не быть в окне: тогда всё, что видно, написано уже после неё.
    let start = before.and_then(|id| messages.iter().position(|m| m["id"] == id)).map_or(0, |i| i + 1);
    let fresh = &messages[start..];
    let wanted = text.trim();
    let own = fresh.iter().rposition(|m| m["role"] == "user" && m["text"].as_str().map(str::trim) == Some(wanted))?;
    let reply = &fresh[own + 1..];
    reply.iter().any(|m| m["role"] == "assistant").then_some(reply)
}

fn summary(reply: &[Value]) -> Answer {
    let assistant = || reply.iter().rev().filter(|m| m["role"] == "assistant");
    Answer {
        text: assistant().find_map(|m| m["text"].as_str().filter(|t| !t.is_empty())).map(str::to_string),
        failed: assistant().next().is_some_and(|m| m["error"].as_bool().unwrap_or(false)),
    }
}

/// Найти бота или группу по имени: точное совпадение, потом начало имени.
/// Двое подходящих — ошибка: платную задачу нельзя отдавать наугад.
pub fn find<'a>(list: &'a [Value], name: &str) -> Result<&'a Value, String> {
    let wanted = name.trim().to_lowercase();
    if wanted.is_empty() {
        return Err("нужно имя".into());
    }
    let name_of = |item: &Value| item["name"].as_str().unwrap_or_default().to_lowercase();
    let mut matches: Vec<&Value> = list.iter().filter(|item| name_of(item) == wanted).collect();
    if matches.is_empty() {
        matches = list.iter().filter(|item| name_of(item).starts_with(&wanted)).collect();
    }
    match matches.as_slice() {
        [] => Err(format!("«{}» не найдено", name.trim())),
        [one] => Ok(one),
        many => {
            let names: Vec<&str> = many.iter().filter_map(|item| item["name"].as_str()).collect();
            Err(format!("«{}» подходит сразу нескольким: {}", name.trim(), names.join(", ")))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{find, reply_entries, summary};
    use serde_json::{json, Value};

    #[test]
    fn a_name_must_point_to_exactly_one() {
        let list = vec![json!({ "name": "Аналитик" }), json!({ "name": "Анна" }), json!({ "name": "Редактор" })];
        assert_eq!(find(&list, "аналитик").unwrap()["name"], "Аналитик");
        assert_eq!(find(&list, "ред").unwrap()["name"], "Редактор");
        assert!(find(&list, "ан").unwrap_err().contains("нескольким"));
        assert!(find(&list, "кто").is_err());
        assert!(find(&list, "  ").is_err());

        let twins = vec![json!({ "name": "Бот" }), json!({ "name": "Бот" })];
        assert!(find(&twins, "бот").is_err(), "два одинаковых имени — не повод брать первого");
    }

    fn entry(id: &str, role: &str, text: &str) -> Value {
        json!({ "id": id, "role": role, "text": text })
    }

    #[test]
    fn an_old_answer_is_never_taken_for_a_new_one() {
        let old = vec![entry("1", "user", "привет"), entry("2", "assistant", "старый ответ")];
        assert!(reply_entries(&old, Some("2"), "привет").is_none(), "нашего сообщения ещё нет");

        let mut asked = old.clone();
        asked.push(entry("3", "user", "привет"));
        assert!(reply_entries(&asked, Some("2"), "привет").is_none(), "ответа ещё нет");

        asked.push(entry("4", "assistant", "новый ответ"));
        let reply = reply_entries(&asked, Some("2"), "привет").unwrap();
        assert_eq!(summary(reply).text.as_deref(), Some("новый ответ"));
    }

    #[test]
    fn the_last_entry_decides_whether_the_turn_failed() {
        let failed = vec![entry("1", "assistant", "думаю"), json!({ "id": "2", "role": "assistant", "text": "Не смог", "error": true })];
        assert!(summary(&failed).failed);
        assert!(!summary(&[entry("1", "assistant", "готово")]).failed);
    }
}
