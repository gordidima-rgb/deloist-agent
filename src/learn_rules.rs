//! Правила обучения памяти: что извлекатель и бот вправе записать после задачи.
//!
//! Чистые помощники без диска (цитата человека, ключ забытого факта, «это правило, а не память»),
//! список «забытого» (человек удалил факт — извлекатель не выучит его заново) и учёт
//! размышления бота о памяти (`Reflection`): пока оно идёт, `remember` бота проверяет те же правила,
//! что и JSON-извлечение.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Mutex;

use crate::App;

/// Короче такая цитата не доказывает ничего: «да», «ок», «хорошо».
const QUOTE_MIN: usize = 8;
/// Сколько удалённых заголовков помним.
const FORGOTTEN_KEEP: usize = 1_000;

/// Правила записи для одного прохода извлекателя.
pub struct Policy<'a> {
    /// Реплики самого человека из этой задачи. `None` — цитаты не проверяем (только тесты и карточка контакта,
    /// которую человек уже одобрил).
    pub said: Option<&'a [String]>,
    /// Ключи заголовков, которые человек удалил (`title_key`).
    pub forgotten: &'a [String],
}

impl Policy<'static> {
    #[cfg(test)]
    pub const fn none() -> Self {
        Policy { said: None, forgotten: &[] }
    }
}

/// Регистр и пробелы не важны.
fn fold(text: &str) -> String {
    text.to_lowercase().split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Цитата дословно есть в одной из реплик человека (без учёта регистра, пробелов и кавычек по краям).
pub fn quote_found(quote: &str, said: &[String]) -> bool {
    let trimmed = quote.trim().trim_matches(|c: char| matches!(c, '«' | '»' | '"' | '“' | '”' | '\'' | '`'));
    let wanted = fold(trimmed);
    wanted.chars().count() >= QUOTE_MIN && said.iter().any(|message| fold(message).contains(&wanted))
}

/// Ключ заголовка для списка забытого: буквы и цифры в нижнем регистре, FNV-1a. Хэш, а не текст:
/// в файле нет самих фактов.
pub fn title_key(title: &str) -> String {
    let flat: String = title.to_lowercase().chars().map(|c| if c.is_alphanumeric() { c } else { ' ' }).collect();
    let norm = flat.split_whitespace().collect::<Vec<_>>().join(" ");
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in norm.as_bytes() {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x0100_0000_01b3);
    }
    format!("{hash:016x}")
}

pub fn is_forgotten(forgotten: &[String], title: &str) -> bool {
    !title.trim().is_empty() && forgotten.contains(&title_key(title))
}

const RULE_STARTS: [&str; 6] = ["всегда ", "никогда ", "always ", "never ", "пропиши ", "запиши в правила"];

/// Постоянное правило («всегда…», «никогда…», «пропиши в правила») — не память: это предложение для `propose_rule`.
pub fn looks_like_rule(title: &str, body: &str) -> bool {
    let title = fold(title);
    let head: String = fold(body).chars().take(60).collect();
    let starts = |text: &str| RULE_STARTS.iter().any(|word| text.starts_with(word));
    starts(&title) || title.contains("в правила") || head.contains("в правила")
}

/// Строка со словами человека в конце факта: откуда это известно.
pub fn with_quote(body: &str, quote: &str) -> String {
    let quote: String = quote.trim().trim_matches(|c: char| matches!(c, '«' | '»' | '"')).chars().take(200).collect();
    let note = format!("Слова человека: «{quote}»");
    let joined = if body.trim().is_empty() { note } else { format!("{}\n{note}", body.trim()) };
    joined.chars().take(crate::memory::BODY_LIMIT).collect()
}

// ---------------------------------------------------------------------------
// Забытое
// ---------------------------------------------------------------------------

static FORGOTTEN_GATE: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

fn forgotten_path(app: &App) -> PathBuf {
    app.store.data_dir().join("memory").join("forgotten.json")
}

pub async fn forgotten(app: &App) -> Vec<String> {
    let Ok(raw) = tokio::fs::read_to_string(forgotten_path(app)).await else { return Vec::new() };
    serde_json::from_str(&raw).unwrap_or_default()
}

async fn save_forgotten(app: &App, list: &[String]) {
    let path = forgotten_path(app);
    if let Some(parent) = path.parent() {
        let _ = tokio::fs::create_dir_all(parent).await;
    }
    if let Ok(text) = serde_json::to_string(list) {
        if let Err(e) = tokio::fs::write(&path, text).await {
            eprintln!("[memory] список забытого не записан: {e}");
        }
    }
}

/// Человек удалил факт: этот заголовок извлекатель и боты заново не запишут.
pub async fn forget_title(app: &App, title: &str) {
    if title.trim().is_empty() {
        return;
    }
    let _gate = FORGOTTEN_GATE.lock().await;
    let mut list = forgotten(app).await;
    let key = title_key(title);
    if list.contains(&key) {
        return;
    }
    list.push(key);
    if list.len() > FORGOTTEN_KEEP {
        list.drain(..list.len() - FORGOTTEN_KEEP);
    }
    save_forgotten(app, &list).await;
}

/// Человек сам записал такой факт заново: запрет снимается.
pub async fn unforget_title(app: &App, title: &str) {
    let _gate = FORGOTTEN_GATE.lock().await;
    let mut list = forgotten(app).await;
    let key = title_key(title);
    let before = list.len();
    list.retain(|k| *k != key);
    if list.len() != before {
        save_forgotten(app, &list).await;
    }
}

// ---------------------------------------------------------------------------
// Размышление бота о памяти
// ---------------------------------------------------------------------------

struct Reflection {
    said: Vec<String>,
    /// Сколько фактов бот видит: правок за проход не больше четверти.
    pool: usize,
    edits: usize,
}

static REFLECTING: Mutex<Option<HashMap<String, Reflection>>> = Mutex::new(None);

/// Пока жив, `remember` этого бота проверяет цитаты, чужие факты человека и предел правок.
pub struct ReflectionGuard(String);

impl Drop for ReflectionGuard {
    fn drop(&mut self) {
        if let Some(map) = REFLECTING.lock().unwrap().as_mut() {
            map.remove(&self.0);
        }
    }
}

pub fn is_reflecting(bot_id: &str) -> bool {
    REFLECTING.lock().unwrap().as_ref().is_some_and(|map| map.contains_key(bot_id))
}

pub fn begin_reflection(bot_id: &str, said: Vec<String>, pool: usize) -> ReflectionGuard {
    REFLECTING.lock().unwrap().get_or_insert_with(HashMap::new).insert(bot_id.to_string(), Reflection { said, pool, edits: 0 });
    ReflectionGuard(bot_id.to_string())
}

/// Предел правок существующих фактов за один проход: четверть, но не меньше одной.
pub fn edit_cap(pool: usize) -> usize {
    pool.div_ceil(4).max(1)
}

/// Что проверить в `remember`, пока идёт размышление. `Ok(None)` — размышления нет, правил нет.
/// `kind` — уже определённый вид (по умолчанию его даёт память); `existing` — факт, который запись правит.
pub struct Existing<'a> {
    pub author: &'a str,
    pub kind: &'a str,
}

pub fn check_reflective_write(bot_id: &str, title: &str, body: &str, kind: &str, quote: Option<&str>, existing: Option<Existing<'_>>) -> Result<Option<String>, String> {
    let mut guard = REFLECTING.lock().unwrap();
    let Some(state) = guard.as_mut().and_then(|map| map.get_mut(bot_id)) else { return Ok(None) };
    if looks_like_rule(title, body) {
        return Err("это постоянное правило, а не память: предложи его через propose_rule, человек решит сам".into());
    }
    if existing.as_ref().is_some_and(|e| e.author == "human") {
        return Err("этот факт записал человек: его не правят и не заменяют, скажи человеку, если он устарел".into());
    }
    let effective = existing.as_ref().map(|e| e.kind).filter(|_| kind.is_empty()).unwrap_or(kind);
    let about_human = effective == "user";
    let mut quoted = None;
    if about_human {
        match quote {
            Some(q) if quote_found(q, &state.said) => quoted = Some(q.to_string()),
            _ => return Err("факт о человеке записывается с дословной цитатой его слов: передай quote — кусок его сообщения без изменений".into()),
        }
    }
    if existing.is_some() {
        if state.edits >= edit_cap(state.pool) {
            return Err("за один проход правится не больше четверти фактов: остальное — в следующий раз".into());
        }
        state.edits += 1;
    }
    Ok(quoted)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_quote_must_be_in_the_human_words_verbatim() {
        let said = vec!["Привет!  Я работаю в Ozon, мой кабинет — Рога и Копыта".to_string(), "ок".to_string()];
        assert!(quote_found("работаю в ozon", &said), "регистр и пробелы не важны");
        assert!(quote_found("«Мой кабинет — Рога и Копыта»", &said), "кавычки по краям не мешают");
        assert!(!quote_found("работает в Ozon", &said), "пересказ не годится");
        assert!(!quote_found("ок", &said), "слишком коротко: ничего не доказывает");
        assert!(!quote_found("", &said));
    }

    #[test]
    fn the_title_key_ignores_case_and_punctuation_but_not_words() {
        assert_eq!(title_key("Любит крепкий кофе!"), title_key("  любит  КРЕПКИЙ кофе"));
        assert_ne!(title_key("Любит крепкий кофе"), title_key("Любит чай"));
        let list = vec![title_key("Живёт в Новосибирске")];
        assert!(is_forgotten(&list, "живёт в новосибирске."));
        assert!(!is_forgotten(&list, "Живёт в Омске"));
        assert!(!is_forgotten(&list, ""));
    }

    #[test]
    fn standing_rules_are_not_memory() {
        assert!(looks_like_rule("Всегда отвечай коротко", ""));
        assert!(looks_like_rule("Никогда не пиши Маше", ""));
        assert!(looks_like_rule("Формат ответа", "Пропиши в правила: без вступлений"));
        assert!(!looks_like_rule("Отчёт Ozon по понедельникам", "всегда к 10:00 — это расписание, а не правило бота"));
        assert!(!looks_like_rule("Живёт в Новосибирске", ""));
    }

    #[test]
    fn reflection_checks_quotes_human_facts_and_the_quarter_limit() {
        let said = vec!["Запомни: меня зовут Аркадий, я продаю кружки".to_string()];
        let _guard = begin_reflection("bot_reflect", said, 8);
        let check = |title: &str, kind: &str, quote: Option<&str>, existing: Option<Existing<'_>>| check_reflective_write("bot_reflect", title, "", kind, quote, existing);
        // Факт о человеке: без цитаты и с выдуманной — нельзя, с настоящей — можно.
        assert!(check("Зовут Аркадий", "user", None, None).is_err());
        assert!(check("Зовут Аркадий", "user", Some("меня зовут Игорь"), None).is_err());
        assert_eq!(check("Зовут Аркадий", "user", Some("меня зовут Аркадий"), None).unwrap().as_deref(), Some("меня зовут Аркадий"));
        // Рабочая заметка цитаты не требует.
        assert_eq!(check("Выгрузка Ozon за 7 дней", "feedback", None, None).unwrap(), None);
        // Правило — не память.
        assert!(check("Всегда пиши коротко", "feedback", None, None).is_err());
        // Факт человека не правят.
        assert!(check("Любит чай", "feedback", None, Some(Existing { author: "human", kind: "feedback" })).is_err());
        // Четверть от 8 — две правки.
        let mine = || Some(Existing { author: "bot_reflect", kind: "feedback" });
        assert!(check("Правка один", "feedback", None, mine()).is_ok());
        assert!(check("Правка два", "feedback", None, mine()).is_ok());
        assert!(check("Правка три", "feedback", None, mine()).is_err());
        drop(_guard);
        // После размышления проверок нет.
        assert_eq!(check_reflective_write("bot_reflect", "Всегда пиши коротко", "", "user", None, None).unwrap(), None);
    }

    #[test]
    fn the_edit_cap_is_a_quarter_but_at_least_one() {
        assert_eq!((edit_cap(0), edit_cap(1), edit_cap(4), edit_cap(5), edit_cap(40)), (1, 1, 1, 2, 10));
    }
}
