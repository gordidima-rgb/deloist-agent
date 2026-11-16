//! Заявки на добавление команды из файла от командной строки (`deloist team apply`).
//!
//! Ключ программы лежит в `runtime.json`, его может прочитать скрипт бота, поэтому прямой импорт
//! команды (`POST /api/team/import`) ему закрыт. Командная строка кладёт заявку: она проверяется так
//! же, как импорт, лежит только в памяти и ничего не создаёт. Принять или отклонить её может лишь
//! окно (`guard::human_only`), а принятая заявка исполняет тот же `team::import`, что и окно.
//!
//! У заявки два номера. Публичный (`id`) и короткий код для сверки с терминалом видны окну: они идут
//! в события и в список для окна, который ключу закрыт. Закрытый «ручка» (`handle`, 128 бит) получает
//! только подавший заявку в ответе на подачу: по ней читается статус и отзывается заявка. Публичный
//! номер статуса и отзыва не даёт.

use std::collections::{HashMap, VecDeque};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use serde_json::{json, Value};
use sha2::{Digest, Sha256};

use crate::store::{hex, now};

/// Сколько заявка ждёт человека.
pub const DEFAULT_TTL: Duration = Duration::from_secs(10 * 60);
/// Сколько может идти импорт после «Добавить»: дольше — заявка считается проваленной.
pub const DEFAULT_APPLY_TTL: Duration = Duration::from_secs(120);
/// Сколько заявок ждёт ответа одновременно: больше — отказ, чтобы окно не засыпали карточками.
pub const MAX_PENDING: usize = 3;
/// Сколько заявок можно подать за окно времени (в том числе уже закрытых).
const RATE_LIMIT: usize = 8;
const RATE_WINDOW: Duration = Duration::from_secs(10 * 60);
/// Отклонённый файл и «не принимать заявки» держатся час.
pub const BLOCK_FOR: Duration = Duration::from_secs(60 * 60);
const MAX_BLOCKED: usize = 64;
/// Закрытая заявка ещё немного живёт, чтобы командная строка успела прочитать итог.
const KEEP_FINISHED: Duration = Duration::from_secs(10 * 60);
const MAX_ITEMS: usize = 16;
/// Без похожих знаков (I, O, 0, 1): код сверяют глазами.
const CODE_ALPHABET: &[u8] = b"ABCDEFGHJKLMNPQRSTUVWXYZ23456789";
/// Что видит ключ программы вместо текста ошибки: настоящий текст идёт в журнал приложения.
pub const FAILED_TEXT: &str = "не получилось добавить команду; подробности в журнале приложения";

#[derive(Clone, Debug, PartialEq)]
enum State {
    Pending,
    /// Человек нажал «Добавить», импорт идёт: второе нажатие уже ничего не применит.
    Applying(Instant),
    Approved(Value),
    Denied,
    Cancelled,
    Failed,
    Expired,
}

impl State {
    fn name(&self) -> &'static str {
        match self {
            State::Pending | State::Applying(_) => "pending",
            State::Approved(_) => "approved",
            State::Denied => "denied",
            State::Cancelled => "cancelled",
            State::Failed => "failed",
            State::Expired => "expired",
        }
    }
}

struct Item {
    id: String,
    handle: String,
    code: String,
    /// Файл как он был проверен: применяется ровно он.
    team: Value,
    preview: Value,
    digest: String,
    created: Instant,
    created_at: i64,
    closed: Option<Instant>,
    state: State,
}

pub struct Created {
    pub id: String,
    pub handle: String,
    pub code: String,
}

#[derive(Debug, PartialEq)]
pub enum Refusal {
    /// Тот же файл уже ждёт ответа: ручку второму не выдаём.
    Duplicate,
    TooMany,
    RateLimited,
    /// Человека этот файл уже раздражал: отклонён недавно.
    Blocked,
    /// Человек нажал «Отклонить все и не принимать заявки».
    Paused,
}

pub enum Take {
    Taken(Value),
    Missing,
    Expired,
    NotPending,
}

pub struct ImportRequests {
    inner: Mutex<Inner>,
    ttl: Duration,
    apply_ttl: Duration,
    /// Сколько окон человека сейчас слушают события: без них карточку никто не увидит.
    windows: Arc<AtomicUsize>,
}

#[derive(Default)]
struct Inner {
    items: Vec<Item>,
    submitted: VecDeque<Instant>,
    blocked: HashMap<String, Instant>,
    paused_until: Option<Instant>,
}

/// Окно человека подключено к потоку событий, пока жив этот знак.
pub struct WindowGuard(Arc<AtomicUsize>);

impl Drop for WindowGuard {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::SeqCst);
    }
}

impl Default for ImportRequests {
    fn default() -> Self {
        // Для проверок срока в тестах программы: не короче секунды, не длиннее обычного.
        let secs = |name: &str, max: Duration| {
            std::env::var(name).ok().and_then(|v| v.parse::<u64>().ok()).filter(|s| (1..=max.as_secs()).contains(s)).map(Duration::from_secs)
        };
        Self::with_ttls(secs("CREW_TEAM_IMPORT_TTL_SECS", DEFAULT_TTL).unwrap_or(DEFAULT_TTL), secs("CREW_TEAM_IMPORT_APPLY_TTL_SECS", DEFAULT_APPLY_TTL).unwrap_or(DEFAULT_APPLY_TTL))
    }
}

impl Inner {
    /// Просроченные заявки закрываем, давно закрытые забываем.
    fn sweep(&mut self, ttl: Duration, apply_ttl: Duration) {
        let at = Instant::now();
        for item in &mut self.items {
            match item.state {
                State::Pending if at.duration_since(item.created) >= ttl => {
                    item.state = State::Expired;
                    item.closed = Some(at);
                }
                State::Applying(since) if at.duration_since(since) >= apply_ttl => {
                    eprintln!("[team-import] импорт заявки не закончился за {} с: помечена проваленной", apply_ttl.as_secs());
                    item.state = State::Failed;
                    item.closed = Some(at);
                }
                _ => {}
            }
        }
        self.items.retain(|item| item.closed.map_or(true, |closed| at.duration_since(closed) < KEEP_FINISHED));
        while self.items.len() > MAX_ITEMS {
            match self.items.iter().position(|item| item.closed.is_some()) {
                Some(oldest) => {
                    self.items.remove(oldest);
                }
                None => break,
            }
        }
        self.blocked.retain(|_, until| *until > at);
        if self.paused_until.is_some_and(|until| until <= at) {
            self.paused_until = None;
        }
    }
}

fn digest_of(team: &Value) -> String {
    hex(&Sha256::digest(serde_json::to_vec(team).unwrap_or_default()))
}

fn counts(preview: &Value) -> Value {
    let size = |key: &str| preview[key].as_array().map_or(0, Vec::len);
    json!({ "bots": size("bots"), "skills": size("skills"), "rooms": size("rooms"), "routines": size("routines") })
}

/// Статус для подавшего: номер и ручку не повторяет, ошибка всегда одна и та же строка.
fn view(item: &Item, ttl: Duration) -> Value {
    let mut out = json!({ "status": item.state.name(), "summary": counts(&item.preview) });
    match &item.state {
        State::Approved(done) => out["imported"] = done.clone(),
        State::Failed => out["error"] = json!(FAILED_TEXT),
        State::Pending | State::Applying(_) => out["expiresInSeconds"] = json!(ttl.saturating_sub(item.created.elapsed()).as_secs()),
        _ => {}
    }
    out
}

/// Карточка для окна: публичный номер, код сверки и что именно будет добавлено.
fn event(item: &Item, ttl: Duration) -> Value {
    json!({
        "id": item.id,
        "code": item.code,
        "preview": item.preview,
        "at": item.created_at,
        "expiresInSeconds": ttl.saturating_sub(item.created.elapsed()).as_secs(),
    })
}

fn random_hex(bytes: usize) -> String {
    let mut buf = vec![0u8; bytes];
    rand::RngCore::fill_bytes(&mut rand::thread_rng(), &mut buf);
    hex(&buf)
}

fn random_code() -> String {
    let mut buf = [0u8; 5];
    rand::RngCore::fill_bytes(&mut rand::thread_rng(), &mut buf);
    buf.iter().map(|b| CODE_ALPHABET[(*b as usize) % CODE_ALPHABET.len()] as char).collect()
}

impl ImportRequests {
    pub fn with_ttl(ttl: Duration) -> Self {
        Self::with_ttls(ttl, DEFAULT_APPLY_TTL)
    }

    pub fn with_ttls(ttl: Duration, apply_ttl: Duration) -> Self {
        Self { inner: Mutex::new(Inner::default()), ttl, apply_ttl, windows: Arc::new(AtomicUsize::new(0)) }
    }

    pub fn window_connected(&self) -> WindowGuard {
        self.windows.fetch_add(1, Ordering::SeqCst);
        WindowGuard(self.windows.clone())
    }

    pub fn windows(&self) -> usize {
        self.windows.load(Ordering::SeqCst)
    }

    fn sweep<'a>(&self, inner: &'a mut Inner) -> &'a mut Inner {
        inner.sweep(self.ttl, self.apply_ttl);
        inner
    }

    /// Новая заявка.
    pub fn create(&self, team: Value, preview: Value) -> Result<Created, Refusal> {
        let digest = digest_of(&team);
        let mut guard = self.inner.lock().unwrap();
        let inner = self.sweep(&mut guard);
        if inner.paused_until.is_some() {
            return Err(Refusal::Paused);
        }
        if inner.blocked.contains_key(&digest) {
            return Err(Refusal::Blocked);
        }
        if inner.items.iter().any(|i| i.state == State::Pending && i.digest == digest) {
            return Err(Refusal::Duplicate);
        }
        if inner.items.iter().filter(|i| matches!(i.state, State::Pending | State::Applying(_))).count() >= MAX_PENDING {
            return Err(Refusal::TooMany);
        }
        let at = Instant::now();
        while inner.submitted.front().is_some_and(|t| at.duration_since(*t) >= RATE_WINDOW) {
            inner.submitted.pop_front();
        }
        if inner.submitted.len() >= RATE_LIMIT {
            return Err(Refusal::RateLimited);
        }
        inner.submitted.push_back(at);
        let created = Created { id: format!("tir_{}", random_hex(16)), handle: format!("tih_{}", random_hex(16)), code: random_code() };
        inner.items.push(Item {
            id: created.id.clone(),
            handle: created.handle.clone(),
            code: created.code.clone(),
            team,
            preview,
            digest,
            created: at,
            created_at: now(),
            closed: None,
            state: State::Pending,
        });
        Ok(created)
    }

    /// Статус заявки для того, кто знает её ручку (публичный номер не подходит).
    pub fn status(&self, handle: &str) -> Option<Value> {
        let mut guard = self.inner.lock().unwrap();
        let inner = self.sweep(&mut guard);
        inner.items.iter().find(|i| same_secret(&i.handle, handle)).map(|i| view(i, self.ttl))
    }

    /// Ждущие заявки для окна: что именно будет добавлено.
    pub fn pending(&self) -> Vec<Value> {
        let mut guard = self.inner.lock().unwrap();
        let inner = self.sweep(&mut guard);
        inner.items.iter().filter(|i| i.state == State::Pending).map(|i| event(i, self.ttl)).collect()
    }

    pub fn event_for(&self, id: &str) -> Option<Value> {
        let inner = self.inner.lock().unwrap();
        inner.items.iter().find(|i| i.id == id).map(|i| event(i, self.ttl))
    }

    /// «Добавить»: заявка переходит из ожидания в работу ровно один раз.
    pub fn take_for_apply(&self, id: &str) -> Take {
        let mut guard = self.inner.lock().unwrap();
        let inner = self.sweep(&mut guard);
        match inner.items.iter_mut().find(|i| i.id == id) {
            None => Take::Missing,
            Some(item) if item.state == State::Expired => Take::Expired,
            Some(item) if item.state != State::Pending => Take::NotPending,
            Some(item) => {
                item.state = State::Applying(Instant::now());
                Take::Taken(item.team.clone())
            }
        }
    }

    /// Итог импорта. Ошибку ключу не показываем: настоящий текст пишется в журнал.
    pub fn finish(&self, id: &str, result: Result<Value, String>) {
        let mut inner = self.inner.lock().unwrap();
        if let Some(item) = inner.items.iter_mut().find(|i| i.id == id && matches!(i.state, State::Applying(_))) {
            item.state = match result {
                Ok(done) => State::Approved(done),
                Err(message) => {
                    eprintln!("[team-import] заявка {id}: {message}");
                    State::Failed
                }
            };
            item.closed = Some(Instant::now());
        }
    }

    /// Отказ человека (кнопка или Esc): тот же файл час не принимается.
    pub fn deny(&self, id: &str) -> bool {
        let mut guard = self.inner.lock().unwrap();
        let inner = self.sweep(&mut guard);
        let Some(item) = inner.items.iter_mut().find(|i| i.id == id && i.state == State::Pending) else {
            return false;
        };
        item.state = State::Denied;
        item.closed = Some(Instant::now());
        let digest = item.digest.clone();
        if inner.blocked.len() < MAX_BLOCKED {
            inner.blocked.insert(digest, Instant::now() + BLOCK_FOR);
        }
        true
    }

    /// Отзыв командной строкой своей заявки (по ручке). Id заявки возвращается, чтобы погасить карточку.
    pub fn cancel(&self, handle: &str) -> Option<String> {
        let mut guard = self.inner.lock().unwrap();
        let inner = self.sweep(&mut guard);
        let item = inner.items.iter_mut().find(|i| i.state == State::Pending && same_secret(&i.handle, handle))?;
        item.state = State::Cancelled;
        item.closed = Some(Instant::now());
        Some(item.id.clone())
    }

    /// «Отклонить все и не принимать заявки час»: ждущие закрываются, новые не принимаются.
    pub fn reject_all(&self) -> Vec<String> {
        let mut guard = self.inner.lock().unwrap();
        let inner = self.sweep(&mut guard);
        let at = Instant::now();
        let mut ids = Vec::new();
        for item in inner.items.iter_mut().filter(|i| i.state == State::Pending) {
            item.state = State::Denied;
            item.closed = Some(at);
            ids.push(item.id.clone());
        }
        inner.paused_until = Some(at + BLOCK_FOR);
        ids
    }
}

/// Сравнение ручек без раннего выхода: по времени ответа ручку не подобрать.
fn same_secret(a: &str, b: &str) -> bool {
    a.len() == b.len() && a.bytes().zip(b.bytes()).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

#[cfg(test)]
mod tests {
    use super::*;

    fn team(n: usize) -> Value {
        json!({ "format": "deloist-team", "bots": [{ "name": format!("Бот {n}") }] })
    }

    fn make(requests: &ImportRequests, n: usize) -> Created {
        requests.create(team(n), json!({})).ok().expect("заявка принята")
    }

    #[test]
    fn a_request_is_applied_once_and_a_second_identical_file_gets_no_handle() {
        let requests = ImportRequests::with_ttl(Duration::from_secs(60));
        let first = make(&requests, 1);
        assert_eq!(first.code.len(), 5);
        assert!(first.code.bytes().all(|b| CODE_ALPHABET.contains(&b)));
        assert_eq!(requests.create(team(1), json!({})).err(), Some(Refusal::Duplicate));
        assert!(matches!(requests.take_for_apply(&first.id), Take::Taken(_)));
        assert!(matches!(requests.take_for_apply(&first.id), Take::NotPending), "второе «Добавить» ничего не применяет");
        requests.finish(&first.id, Ok(json!({ "bots": 1 })));
        assert_eq!(requests.status(&first.handle).unwrap()["status"], "approved");
        assert!(!requests.deny(&first.id), "принятую заявку не отклонить");
    }

    #[test]
    fn the_public_id_gives_neither_status_nor_cancel() {
        let requests = ImportRequests::with_ttl(Duration::from_secs(60));
        let made = make(&requests, 1);
        assert!(requests.status(&made.id).is_none());
        assert!(requests.cancel(&made.id).is_none());
        assert!(requests.status(&made.handle).is_some());
        assert_eq!(requests.cancel(&made.handle).as_deref(), Some(made.id.as_str()));
        assert_eq!(requests.status(&made.handle).unwrap()["status"], "cancelled");
    }

    #[test]
    fn at_most_three_wait_at_once_closed_ones_do_not_hold_the_cap_and_the_rate_is_limited() {
        let requests = ImportRequests::with_ttl(Duration::from_secs(60));
        let made: Vec<Created> = (0..MAX_PENDING).map(|n| make(&requests, n)).collect();
        assert_eq!(requests.create(team(99), json!({})).err(), Some(Refusal::TooMany));
        assert!(requests.deny(&made[0].id));
        let next = make(&requests, 50);
        requests.cancel(&next.handle);
        for n in 10..20 {
            if let Ok(created) = requests.create(team(n), json!({})) {
                requests.cancel(&created.handle);
            }
        }
        assert_eq!(requests.create(team(500), json!({})).err(), Some(Refusal::RateLimited));
    }

    #[test]
    fn a_denied_file_is_blocked_and_reject_all_pauses_everything() {
        let requests = ImportRequests::with_ttl(Duration::from_secs(60));
        let a = make(&requests, 1);
        let b = make(&requests, 2);
        assert!(requests.deny(&a.id));
        assert_eq!(requests.create(team(1), json!({})).err(), Some(Refusal::Blocked));
        assert!(requests.create(team(3), json!({})).is_ok(), "другой файл не заблокирован");
        let closed = requests.reject_all();
        assert!(closed.contains(&b.id) && closed.len() == 2, "{closed:?}");
        assert!(requests.pending().is_empty());
        assert_eq!(requests.status(&b.handle).unwrap()["status"], "denied");
        assert_eq!(requests.create(team(4), json!({})).err(), Some(Refusal::Paused));
    }

    #[test]
    fn an_expired_request_cannot_be_applied() {
        let requests = ImportRequests::with_ttl(Duration::from_millis(20));
        let made = make(&requests, 1);
        std::thread::sleep(Duration::from_millis(40));
        assert!(matches!(requests.take_for_apply(&made.id), Take::Expired));
        assert_eq!(requests.status(&made.handle).unwrap()["status"], "expired");
        assert!(requests.pending().is_empty());
    }

    #[test]
    fn a_request_stuck_in_applying_fails_after_its_timeout_with_a_fixed_error() {
        let requests = ImportRequests::with_ttls(Duration::from_secs(60), Duration::from_millis(20));
        let made = make(&requests, 1);
        assert!(matches!(requests.take_for_apply(&made.id), Take::Taken(_)));
        assert_eq!(requests.status(&made.handle).unwrap()["status"], "pending");
        std::thread::sleep(Duration::from_millis(40));
        let status = requests.status(&made.handle).unwrap();
        assert_eq!((status["status"].as_str(), status["error"].as_str()), (Some("failed"), Some(FAILED_TEXT)), "{status}");
        // Запоздавший итог уже закрытой заявки ничего не меняет.
        requests.finish(&made.id, Ok(json!({ "bots": 9 })));
        assert_eq!(requests.status(&made.handle).unwrap()["status"], "failed");
    }

    #[test]
    fn the_status_never_carries_the_file_the_id_or_the_real_error() {
        let requests = ImportRequests::with_ttl(Duration::from_secs(60));
        let made = requests
            .create(json!({ "format": "deloist-team", "bots": [{ "name": "Секрет-имя", "persona": "ТОКЕН-123" }] }), json!({ "bots": [{ "name": "Секрет-имя" }] }))
            .ok()
            .unwrap();
        assert!(matches!(requests.take_for_apply(&made.id), Take::Taken(_)));
        requests.finish(&made.id, Err("C:\\Users\\Секрет\\state.json: отказано в доступе".into()));
        let text = requests.status(&made.handle).unwrap().to_string();
        for leak in ["ТОКЕН-123", "Секрет-имя", "Секрет", "state.json", made.id.as_str()] {
            assert!(!text.contains(leak), "{leak}: {text}");
        }
    }

    #[test]
    fn windows_are_counted_while_they_listen() {
        let requests = ImportRequests::default();
        assert_eq!(requests.windows(), 0);
        let one = requests.window_connected();
        let two = requests.window_connected();
        assert_eq!(requests.windows(), 2);
        drop(one);
        drop(two);
        assert_eq!(requests.windows(), 0);
    }
}
