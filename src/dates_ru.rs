//! Даты, время и повторы из русского текста: кодом, без модели.
//!
//! Порт из старого планировщика: подсветка (`ruParser.ts`), разбор без модели
//! (`fallback_parser.py`), следующий срок и подписи повтора (`recurrence.ts`,
//! `recurrence.py`). Правило одно: ничего не выдумывать. Ложное время хуже
//! отсутствующего, поэтому голое число временем не становится: нужны минуты,
//! часть суток или слово «часов».
#![allow(dead_code)] // маршрут и тесты пока единственные потребители; рутины подключат остальное

use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};

use chrono::{Datelike, Days, NaiveDate, NaiveTime};
use regex::{Captures, Match, Regex};
use serde::{Deserialize, Deserializer, Serialize};
use serde_json::{json, Value};

// ---------- повтор ----------

/// Частота повтора. В JSON — строкой в нижнем регистре, как в старом планировщике.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Freq {
    Daily,
    Weekly,
    Monthly,
    Yearly,
}

/// Правило повтора. `weekdays`: 1 = понедельник … 7 = воскресенье.
/// При чтении из JSON значения приводятся к допустимым: интервал 1..=365,
/// дни недели без дубля и вне диапазона, число месяца 1..=31.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Recurrence {
    pub freq: Freq,
    #[serde(default = "one", deserialize_with = "de_interval")]
    pub interval: u32,
    #[serde(default, deserialize_with = "de_weekdays")]
    pub weekdays: Vec<u8>,
    #[serde(default, deserialize_with = "de_monthday")]
    pub monthday: Option<u8>,
}

fn one() -> u32 {
    1
}

fn de_interval<'de, D: Deserializer<'de>>(d: D) -> Result<u32, D::Error> {
    Ok(i64::deserialize(d)?.clamp(1, 365) as u32)
}

fn de_weekdays<'de, D: Deserializer<'de>>(d: D) -> Result<Vec<u8>, D::Error> {
    let raw = Option::<Vec<i64>>::deserialize(d)?.unwrap_or_default();
    Ok(norm_weekdays(raw))
}

fn de_monthday<'de, D: Deserializer<'de>>(d: D) -> Result<Option<u8>, D::Error> {
    Ok(Option::<i64>::deserialize(d)?
        .filter(|m| (1..=31).contains(m))
        .map(|m| m as u8))
}

/// Дни недели: 1..=7, без повторов, по возрастанию.
fn norm_weekdays(raw: impl IntoIterator<Item = i64>) -> Vec<u8> {
    let mut out: Vec<u8> = raw
        .into_iter()
        .filter(|d| (1..=7).contains(d))
        .map(|d| d as u8)
        .collect();
    out.sort_unstable();
    out.dedup();
    out
}

fn norm_of(rule: &Recurrence) -> Vec<u8> {
    norm_weekdays(rule.weekdays.iter().map(|d| i64::from(*d)))
}

// ---------- календарь ----------

fn wd_num(d: NaiveDate) -> u8 {
    d.weekday().number_from_monday() as u8
}

fn plus_days(d: NaiveDate, n: i64) -> NaiveDate {
    let moved = if n >= 0 {
        d.checked_add_days(Days::new(n as u64))
    } else {
        d.checked_sub_days(Days::new(n.unsigned_abs()))
    };
    moved.expect("дата в допустимом диапазоне")
}

/// День `day` в месяце; если его нет (31 февраля), последний день месяца.
fn clamp_day(year: i32, month: u32, day: u32) -> NaiveDate {
    (1..=day.clamp(1, 31))
        .rev()
        .find_map(|d| NaiveDate::from_ymd_opt(year, month, d))
        .expect("месяц от 1 до 12")
}

fn add_months(d: NaiveDate, months: u32, day: u32) -> NaiveDate {
    let total = d.year() * 12 + d.month0() as i32 + months as i32;
    clamp_day(total.div_euclid(12), total.rem_euclid(12) as u32 + 1, day)
}

fn weekdays_of(rule: &Recurrence, reference: NaiveDate) -> Vec<u8> {
    let wd = norm_of(rule);
    if wd.is_empty() {
        vec![wd_num(reference)]
    } else {
        wd
    }
}

/// Первый срок ряда, не раньше `today`.
pub fn first_due(rule: &Recurrence, today: NaiveDate) -> NaiveDate {
    match rule.freq {
        Freq::Daily | Freq::Yearly => today,
        Freq::Weekly => {
            let wd = weekdays_of(rule, today);
            (0..7)
                .map(|i| plus_days(today, i))
                .find(|c| wd.contains(&wd_num(*c)))
                .unwrap_or(today)
        }
        Freq::Monthly => {
            let day = rule.monthday.map_or(today.day(), u32::from);
            let this = clamp_day(today.year(), today.month(), day);
            if this >= today {
                this
            } else {
                add_months(today, 1, day)
            }
        }
    }
}

/// Следующий срок строго после `after`. Считает от `after`, как от последнего срока ряда.
/// Пропущенные сроки не догоняются: если `after` давно в прошлом, см. `next_occurrence_not_before`.
pub fn next_occurrence(rule: &Recurrence, after: NaiveDate) -> NaiveDate {
    next_occurrence_not_before(rule, after, after)
}

/// Следующий срок строго после `after` и не раньше `today`. Пропущенные сроки
/// не догоняются: результат — первый срок ряда, который не в прошлом.
pub fn next_occurrence_not_before(rule: &Recurrence, after: NaiveDate, today: NaiveDate) -> NaiveDate {
    let step = rule.interval.max(1);
    match rule.freq {
        Freq::Daily => {
            let mut c = plus_days(after, i64::from(step));
            while c < today {
                c = plus_days(c, i64::from(step));
            }
            c
        }
        Freq::Weekly => {
            let wd = weekdays_of(rule, after);
            let week0 = plus_days(after, -i64::from(after.weekday().num_days_from_monday()));
            let mut c = plus_days(after, 1);
            loop {
                let weeks = (c - week0).num_days() / 7;
                if wd.contains(&wd_num(c)) && weeks % i64::from(step) == 0 && c >= today {
                    return c;
                }
                c = plus_days(c, 1);
            }
        }
        Freq::Monthly => {
            let day = rule.monthday.map_or(after.day(), u32::from);
            let mut c = add_months(after, step, day);
            while c <= after || c < today {
                c = add_months(c, step, day);
            }
            c
        }
        Freq::Yearly => {
            let year = |y: i32| clamp_day(y, after.month(), after.day());
            let mut c = year(after.year() + step as i32);
            while c <= after || c < today {
                c = year(c.year() + step as i32);
            }
            c
        }
    }
}

// ---------- подписи ----------

const WD_NAME: [&str; 7] = [
    "понедельникам", "вторникам", "средам", "четвергам", "пятницам", "субботам", "воскресеньям",
];
const WD_SHORT: [&str; 7] = ["пн", "вт", "ср", "чт", "пт", "сб", "вс"];

/// «по будням», «по выходным», «по субботам», «по пн–сб», «по пн, ср, пт».
/// Пустая строка: дни не заданы или их все семь.
fn weekdays_phrase(wd: &[u8]) -> String {
    if wd.is_empty() || wd.len() == 7 {
        return String::new();
    }
    if wd == [1u8, 2, 3, 4, 5] {
        return "по будням".to_string();
    }
    if wd == [6u8, 7] {
        return "по выходным".to_string();
    }
    if wd.len() == 1 {
        return format!("по {}", WD_NAME[wd[0] as usize - 1]);
    }
    let contiguous = wd.windows(2).all(|p| p[1] == p[0] + 1);
    if contiguous {
        return format!(
            "по {}–{}",
            WD_SHORT[wd[0] as usize - 1],
            WD_SHORT[wd[wd.len() - 1] as usize - 1]
        );
    }
    let names: Vec<&str> = wd.iter().map(|d| WD_SHORT[*d as usize - 1]).collect();
    format!("по {}", names.join(", "))
}

/// Подпись повтора. Пока только русский: `_lang` не используется, перевод подключит шаг с интерфейсом.
pub fn label(rule: &Recurrence, _lang: &str) -> String {
    let iv = rule.interval.max(1);
    match rule.freq {
        Freq::Daily => {
            if iv == 1 {
                "Каждый день".to_string()
            } else {
                format!("Каждые {iv} дн.")
            }
        }
        Freq::Weekly => {
            let wd = norm_of(rule);
            let days = weekdays_phrase(&wd);
            if iv == 1 {
                if wd.len() == 7 {
                    "Каждый день".to_string()
                } else if days == "по будням" {
                    "По будням".to_string()
                } else if days == "по выходным" {
                    "По выходным".to_string()
                } else if days.is_empty() {
                    "Каждую неделю".to_string()
                } else {
                    format!("Каждую неделю {days}")
                }
            } else {
                let prefix = if iv == 2 {
                    "Раз в 2 недели".to_string()
                } else {
                    format!("Каждые {iv} нед.")
                };
                format!("{prefix} {days}").trim_end().to_string()
            }
        }
        Freq::Monthly => {
            let tail = rule.monthday.map(|d| format!(", {d}-го")).unwrap_or_default();
            if iv == 1 {
                format!("Каждый месяц{tail}")
            } else {
                format!("Раз в {iv} мес.{tail}")
            }
        }
        Freq::Yearly => {
            if iv == 1 {
                "Каждый год".to_string()
            } else {
                format!("Раз в {iv} г.")
            }
        }
    }
}

// ---------- разбор текста ----------

/// Что нашли в тексте.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SpanKind {
    Date,
    Time,
    Repeat,
}

/// Кусок текста, который распознан. `start`/`end` — номера символов (не байт).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Span {
    pub start: usize,
    pub end: usize,
    pub kind: SpanKind,
}

#[derive(Debug, PartialEq, Eq)]
pub struct Parsed {
    /// Текст без распознанных кусков; при «ничего не нашли» — исходный текст.
    pub title: String,
    pub due_date: Option<NaiveDate>,
    pub due_time: Option<NaiveTime>,
    pub recurrence: Option<Recurrence>,
    pub spans: Vec<Span>,
    /// Код нашёл всё, что просила фраза, и в ней не осталось неясного. Тогда модель не нужна.
    pub complete: bool,
}

enum Payload {
    Date(NaiveDate),
    Time(NaiveTime),
    Repeat(Recurrence),
}

/// Совпадение в байтах текста; переводится в символы только при выдаче.
struct Hit {
    start: usize,
    end: usize,
    payload: Payload,
}

const WD_ALT: &str = r"понедельник\p{L}*|вторник\p{L}*|сред\p{L}*|четверг\p{L}*|пятниц\p{L}*|суббот\p{L}*|воскресень\p{L}*";
const NUM: &str = r"\d{1,3}|два|две|три|четыре|пять|шесть|семь|восемь|девять|десять|одиннадцать|двенадцать";
const UNIT: &str = r"дн(?:ей|я)|день|недел\p{L}*|месяц\p{L}*|год\p{L}*|лет";

const P_LIST: &str = r"(?i)кажд\p{L}*\s+((?:{WD})(?:\s*(?:,|и)\s*(?:{WD}))*)";
const P_WD_ONLY: &str = r"(?i)(?:{WD})";
const P_WD_PO: &str = r"(?i)по\s+({WD})";
const P_WORK: &str = r"(?i)по\s+будням";
const P_WEEKEND: &str = r"(?i)по\s+выходным";
const P_DAILY: &str = r"(?i)ежедневн\p{L}*";
const P_WEEKLY: &str = r"(?i)еженедельн\p{L}*";
const P_YEARLY: &str = r"(?i)ежегодн\p{L}*";
const P_MONTH: &str = r"(?i)(?:ежемесячн\p{L}*|кажд\p{L}*\s+месяц\p{L}*)(?:\s+(\d{1,2})-?го)?";
const P_EVERY: &str = r"(?i)кажд\p{L}*\s+(?:({NUM})\s+)?({UNIT})";
const P_RAZ: &str = r"(?i)раз\p{L}*\s+в\s+(?:({NUM})\s+)?({UNIT})";
const P_IN: &str = r"(?i)через\s+(\d+\s+)?(дн\p{L}*|недел\p{L}*|месяц\p{L}*)";
const P_WEEKDAY: &str = r"(?i)в(?:о)?\s+((?:следующ\p{L}*|этот|эту|ближайш\p{L}*)\s+)?({WD})";
const P_REL: &str = r"(?i)(сегодня|завтра|послезавтра)";
const P_MONTHNAME: &str = r"(?i)(\d{1,2})\s+(январ\p{L}*|феврал\p{L}*|март\p{L}*|апрел\p{L}*|ма[йяе]\p{L}*|июн\p{L}*|июл\p{L}*|август\p{L}*|сентябр\p{L}*|октябр\p{L}*|ноябр\p{L}*|декабр\p{L}*)";
const P_DATE_YEAR: &str = r"(?i)(?:начиная\s+с\s+)?(\d{1,2})[./](\d{1,2})[./](\d{2,4})(?:\s*г\.?)?";
const P_DATE_NOYEAR: &str = r"(?i)(?:начиная\s+с\s+)?(\d{1,2})\.(\d{1,2})";
const P_NAMED: &str = r"(?i)(?:в\s+)?(полдень|полночь)";
const P_CLOCK: &str = r"(?i)(?:в|к)\s+(\d{1,2})(?::(\d{2}))?(?:\s*(утра|дня|вечера|ночи))?";
const P_HOURS: &str = r"(?i)в\s+(\d{1,2})\s*(?:часов|часа|час|ч)";
const P_BARE: &str = r"(?i)(?:в\s+)?(\d{1,2}):(\d{2})";
const P_RANGE: &str = r"(?i)с\s+(\d{1,2}):(\d{2})\s+до\s+\d{1,2}:\d{2}";
const P_STRAY: &str = r"(?i)(?:^|\s)(?:в|во|к)\s+\d";

const MONTH_STEMS: [&str; 12] = [
    "январ", "феврал", "март", "апрел", "ма", "июн", "июл", "август", "сентябр", "октябр", "ноябр", "декабр",
];

/// Скомпилированный шаблон (кэшируется). Подставляет куски, которые повторяются.
fn re(pattern: &'static str) -> Regex {
    static CACHE: OnceLock<Mutex<HashMap<&'static str, Regex>>> = OnceLock::new();
    let cache = CACHE.get_or_init(|| Mutex::new(HashMap::new()));
    let mut map = cache.lock().unwrap_or_else(|e| e.into_inner());
    map.entry(pattern)
        .or_insert_with(|| {
            let src = pattern
                .replace("{WD}", WD_ALT)
                .replace("{NUM}", NUM)
                .replace("{UNIT}", UNIT);
            Regex::new(&src).expect("шаблон разбора дат собран верно")
        })
        .clone()
}

fn is_word(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

/// Граница слова: по обе стороны совпадения не буква, не цифра и не `_`.
/// Заменяет `(?<!…)` и `(?!…)`, которых нет в движке regex.
fn bounded(text: &str, start: usize, end: usize) -> bool {
    let before = text[..start].chars().next_back();
    let after = text[end..].chars().next();
    !before.is_some_and(is_word) && !after.is_some_and(is_word)
}

fn scan(text: &str, pattern: &'static str, out: &mut Vec<Hit>, mut f: impl FnMut(&Captures<'_>) -> Option<Payload>) {
    for caps in re(pattern).captures_iter(text) {
        let whole = caps.get(0).expect("группа 0 есть всегда");
        let (start, end) = (whole.start(), whole.end());
        if !bounded(text, start, end) {
            continue;
        }
        if let Some(payload) = f(&caps) {
            out.push(Hit { start, end, payload });
        }
    }
}

fn wd_of_word(word: &str) -> Option<u8> {
    const STEMS: [(&str, u8); 7] = [
        ("понедельник", 1),
        ("вторник", 2),
        ("сред", 3),
        ("четверг", 4),
        ("пятниц", 5),
        ("суббот", 6),
        ("воскресень", 7),
    ];
    let w = word.to_lowercase();
    STEMS.iter().find(|(s, _)| w.starts_with(s)).map(|(_, i)| *i)
}

fn month_of_word(word: &str) -> Option<u32> {
    let w = word.to_lowercase();
    MONTH_STEMS.iter().position(|s| w.starts_with(s)).map(|i| i as u32 + 1)
}

fn weekday_list(s: &str) -> Vec<u8> {
    norm_weekdays(
        re(P_WD_ONLY)
            .find_iter(s)
            .filter_map(|m| wd_of_word(m.as_str()).map(i64::from)),
    )
}

fn num(m: Match<'_>) -> Option<u32> {
    m.as_str().parse().ok()
}

fn at(h: u32, m: u32) -> Option<Payload> {
    NaiveTime::from_hms_opt(h, m, 0).map(Payload::Time)
}

fn repeat(freq: Freq, interval: u32, weekdays: Vec<u8>, monthday: Option<u8>) -> Payload {
    Payload::Repeat(Recurrence { freq, interval, weekdays, monthday })
}

fn freq_of_unit(unit: &str) -> Freq {
    let low = unit.to_lowercase();
    if low.starts_with("дн") || low.starts_with("день") {
        Freq::Daily
    } else if low.starts_with("недел") {
        Freq::Weekly
    } else if low.starts_with("месяц") {
        Freq::Monthly
    } else {
        Freq::Yearly
    }
}

fn parse_interval(raw: &str) -> u32 {
    let low = raw.to_lowercase();
    if let Ok(n) = low.parse::<u32>() {
        return n.clamp(1, 365);
    }
    const WORDS: [(&str, u32); 12] = [
        ("два", 2), ("две", 2), ("три", 3), ("четыре", 4), ("пять", 5), ("шесть", 6),
        ("семь", 7), ("восемь", 8), ("девять", 9), ("десять", 10), ("одиннадцать", 11), ("двенадцать", 12),
    ];
    WORDS.iter().find(|(w, _)| *w == low).map(|(_, n)| *n).unwrap_or(1)
}

fn repeat_hits(text: &str, out: &mut Vec<Hit>) {
    // «каждую среду», «каждый понедельник и среду» — раньше одиночных дней
    scan(text, P_LIST, out, |c| {
        let days = weekday_list(c.get(1)?.as_str());
        (!days.is_empty()).then(|| repeat(Freq::Weekly, 1, days, None))
    });
    scan(text, P_WD_PO, out, |c| {
        let day = wd_of_word(c.get(1)?.as_str())?;
        Some(repeat(Freq::Weekly, 1, vec![day], None))
    });
    scan(text, P_WORK, out, |_| Some(repeat(Freq::Weekly, 1, vec![1, 2, 3, 4, 5], None)));
    scan(text, P_WEEKEND, out, |_| Some(repeat(Freq::Weekly, 1, vec![6, 7], None)));
    scan(text, P_DAILY, out, |_| Some(repeat(Freq::Daily, 1, vec![], None)));
    scan(text, P_WEEKLY, out, |_| Some(repeat(Freq::Weekly, 1, vec![], None)));
    scan(text, P_YEARLY, out, |_| Some(repeat(Freq::Yearly, 1, vec![], None)));
    // раньше P_EVERY: «каждый месяц 5-го» — месячный повтор с числом
    scan(text, P_MONTH, out, |c| {
        let md = c
            .get(1)
            .and_then(|m| m.as_str().parse::<u8>().ok())
            .filter(|d| (1..=31).contains(d));
        Some(repeat(Freq::Monthly, 1, vec![], md))
    });
    scan(text, P_EVERY, out, |c| Some(interval_unit(c.get(1), c.get(2)?.as_str())));
    scan(text, P_RAZ, out, |c| Some(interval_unit(c.get(1), c.get(2)?.as_str())));
}

fn interval_unit(num: Option<Match<'_>>, unit: &str) -> Payload {
    let interval = num.map_or(1, |m| parse_interval(m.as_str()));
    repeat(freq_of_unit(unit), interval, vec![], None)
}

/// Ближайшая (сегодня или будущая) дата с этим днём и месяцем.
fn next_dated(today: NaiveDate, month: u32, day: u32) -> Option<NaiveDate> {
    let this = NaiveDate::from_ymd_opt(today.year(), month, day)?;
    if this < today {
        NaiveDate::from_ymd_opt(today.year() + 1, month, day)
    } else {
        Some(this)
    }
}

fn date_hits(text: &str, today: NaiveDate, out: &mut Vec<Hit>) {
    // «через N дней/недель/месяцев», «через неделю» — N по умолчанию 1
    scan(text, P_IN, out, |c| {
        let n = match c.get(1) {
            Some(m) => m.as_str().trim().parse::<i64>().ok()?,
            None => 1,
        };
        if !(0..=10_000).contains(&n) {
            return None;
        }
        let unit = c.get(2)?.as_str().to_lowercase();
        let date = if unit.starts_with("дн") {
            plus_days(today, n)
        } else if unit.starts_with("недел") {
            plus_days(today, n * 7)
        } else {
            add_months(today, n as u32, today.day())
        };
        Some(Payload::Date(date))
    });
    scan(text, P_MONTHNAME, out, |c| {
        let day: u32 = c.get(1)?.as_str().parse().ok()?;
        let month = month_of_word(c.get(2)?.as_str())?;
        if !(1..=31).contains(&day) {
            return None;
        }
        Some(Payload::Date(next_dated(today, month, day)?))
    });
    scan(text, P_DATE_YEAR, out, |c| {
        let d: u32 = c.get(1)?.as_str().parse().ok()?;
        let m: u32 = c.get(2)?.as_str().parse().ok()?;
        let y: i32 = c.get(3)?.as_str().parse().ok()?;
        let y = if y < 100 { 2000 + y } else { y };
        Some(Payload::Date(NaiveDate::from_ymd_opt(y, m, d)?))
    });
    scan(text, P_DATE_NOYEAR, out, |c| {
        // «1.7.2026» и «25.000» — не «день.месяц»: за числом не должно идти «.» или цифра
        let end = c.get(0)?.end();
        let next = text[end..].trim_start().chars().next();
        if matches!(next, Some(ch) if ch == '.' || ch.is_numeric()) {
            return None;
        }
        let d: u32 = c.get(1)?.as_str().parse().ok()?;
        let m: u32 = c.get(2)?.as_str().parse().ok()?;
        Some(Payload::Date(next_dated(today, m, d)?))
    });
    scan(text, P_WEEKDAY, out, |c| {
        let idx = wd_of_word(c.get(2)?.as_str())?;
        let modifier = c.get(1).map_or(String::new(), |m| m.as_str().to_lowercase());
        let mut delta = (i64::from(idx) - i64::from(wd_num(today)) + 7) % 7;
        if modifier.starts_with("следующ") {
            delta += 7;
        }
        Some(Payload::Date(plus_days(today, delta)))
    });
    scan(text, P_REL, out, |c| {
        let offset = match c.get(1)?.as_str().to_lowercase().as_str() {
            "сегодня" => 0,
            "завтра" => 1,
            _ => 2,
        };
        Some(Payload::Date(plus_days(today, offset)))
    });
}

fn time_hits(text: &str, out: &mut Vec<Hit>) {
    scan(text, P_NAMED, out, |c| {
        if c.get(1)?.as_str().to_lowercase() == "полдень" {
            at(12, 0)
        } else {
            at(0, 0)
        }
    });
    // «в 9 утра», «в 9 вечера», «в 18:00», «в 18». Голый час — только от 13 и выше.
    scan(text, P_CLOCK, out, |c| {
        let mut h = num(c.get(1)?)?;
        let min: Option<u32> = c.get(2).and_then(|m| m.as_str().parse().ok());
        let part = c.get(3).map(|m| m.as_str().to_lowercase());
        if h > 23 || min.is_some_and(|m| m > 59) {
            return None;
        }
        if min.is_none() && part.is_none() && h < 13 {
            return None;
        }
        match part.as_deref() {
            Some("вечера") if h < 12 => h += 12,
            Some("дня") if (1..=5).contains(&h) => h += 12,
            Some("ночи") if h == 12 => h = 0,
            _ => {}
        }
        at(h, min.unwrap_or(0))
    });
    // «в 11 часов» — час словом только с явным «часов/часа/час/ч»
    scan(text, P_HOURS, out, |c| {
        let h = num(c.get(1)?)?;
        if h > 23 {
            None
        } else {
            at(h, 0)
        }
    });
    scan(text, P_BARE, out, |c| at(num(c.get(1)?)?, num(c.get(2)?)?));
    // «с 14:00 до 15:00» — берём начало; диапазон раньше своих чисел (он начинается раньше)
    scan(text, P_RANGE, out, |c| at(num(c.get(1)?)?, num(c.get(2)?)?));
}

fn collapse(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Обрезка по краям и заглавная первая буква.
fn tidy(title: &str) -> String {
    let t = title.trim_matches(|c: char| c.is_whitespace() || " -–—,;:.".contains(c));
    let mut chars = t.chars();
    match chars.next() {
        None => String::new(),
        Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
    }
}

fn cut500(s: &str) -> String {
    s.chars().take(500).collect()
}

fn char_idx(text: &str, byte: usize) -> usize {
    text[..byte].chars().count()
}

/// Фраза с двумя делами или с чек-листом: код её целиком не решает.
fn looks_like_several(text: &str, leftover: &str) -> bool {
    if text.contains('\n') || text.contains(';') {
        return true;
    }
    let words: Vec<String> = leftover.split_whitespace().map(|w| w.to_lowercase()).collect();
    words.windows(2).any(|p| {
        p[0] == "и"
            && ["ть", "ться", "ти", "чь"]
                .iter()
                .any(|e| p[1].ends_with(e) && p[1].chars().count() > e.chars().count())
    })
}

/// Разбор фразы: даты, время, повтор и название без них. Без модели и без сети.
/// `_now` пока не нужен правилам (часы без минут времени не дают); оставлен в сигнатуре по ТЗ.
pub fn parse(text: &str, today: NaiveDate, _now: NaiveTime) -> Parsed {
    let mut hits: Vec<Hit> = Vec::new();
    repeat_hits(text, &mut hits);
    date_hits(text, today, &mut hits);
    time_hits(text, &mut hits);

    // длиннее — раньше при одинаковом начале; при равенстве выигрывает добавленный раньше
    hits.sort_by(|a, b| a.start.cmp(&b.start).then((b.end - b.start).cmp(&(a.end - a.start))));
    let mut kept: Vec<Hit> = Vec::new();
    for h in hits {
        if kept.iter().any(|k| h.start < k.end && k.start < h.end) {
            continue;
        }
        kept.push(h);
    }

    let mut due_date = None;
    let mut due_time = None;
    let mut recurrence = None;
    let mut spans = Vec::with_capacity(kept.len());
    for h in &kept {
        let kind = match &h.payload {
            Payload::Date(d) => {
                if due_date.is_none() {
                    due_date = Some(*d);
                }
                SpanKind::Date
            }
            Payload::Time(t) => {
                if due_time.is_none() {
                    due_time = Some(*t);
                }
                SpanKind::Time
            }
            Payload::Repeat(r) => {
                if recurrence.is_none() {
                    recurrence = Some(r.clone());
                }
                SpanKind::Repeat
            }
        };
        spans.push(Span { start: char_idx(text, h.start), end: char_idx(text, h.end), kind });
    }

    let mut rest = String::new();
    let mut prev = 0;
    for h in &kept {
        rest.push_str(&text[prev..h.start]);
        rest.push(' ');
        prev = h.end;
    }
    rest.push_str(&text[prev..]);
    let leftover = collapse(&rest);
    let tidied = tidy(&leftover);
    let touched = !kept.is_empty();
    let title_ok = touched && tidied.chars().count() >= 3;
    let title = if title_ok { cut500(&tidied) } else { cut500(text.trim()) };
    let complete = title_ok
        && !re(P_STRAY).is_match(&leftover)
        && !looks_like_several(text, &leftover);

    Parsed { title, due_date, due_time, recurrence, spans, complete }
}

/// JSON для маршрута: даты `YYYY-MM-DD`, время `HH:MM`, повтор как в `Recurrence`.
pub fn to_json(p: &Parsed) -> Value {
    let spans: Vec<Value> = p
        .spans
        .iter()
        .map(|s| {
            let kind = match s.kind {
                SpanKind::Date => "date",
                SpanKind::Time => "time",
                SpanKind::Repeat => "repeat",
            };
            json!({ "start": s.start, "end": s.end, "kind": kind })
        })
        .collect();
    json!({
        "title": p.title,
        "due_date": p.due_date.map(|d| d.format("%Y-%m-%d").to_string()),
        "due_time": p.due_time.map(|t| t.format("%H:%M").to_string()),
        "recurrence": p.recurrence,
        "label": p.recurrence.as_ref().map(|r| label(r, "ru")),
        "spans": spans,
        "complete": p.complete,
    })
}

/// Короткая заметка для модели: какие сроки человек назвал словами и какие это точные даты.
/// `None`, если в тексте нет ни даты, ни времени, ни повтора.
pub fn note(text: &str, today: NaiveDate, now: NaiveTime) -> Option<String> {
    let p = parse(text, today, now);
    let chars: Vec<char> = text.chars().collect();
    let words = |kinds: &[SpanKind]| -> String {
        p.spans
            .iter()
            .filter(|s| kinds.contains(&s.kind))
            .map(|s| chars[s.start.min(chars.len())..s.end.min(chars.len())].iter().collect::<String>())
            .collect::<Vec<_>>()
            .join(" ")
    };
    let mut parts: Vec<String> = Vec::new();
    if p.due_date.is_some() || p.due_time.is_some() {
        let said = words(&[SpanKind::Date, SpanKind::Time]);
        let exact = match (p.due_date, p.due_time) {
            (Some(d), Some(t)) => format!("{} {} ({})", d.format("%Y-%m-%d"), t.format("%H:%M"), weekday_ru(d)),
            (Some(d), None) => format!("{} ({})", d.format("%Y-%m-%d"), weekday_ru(d)),
            (None, Some(t)) => format!("время {}", t.format("%H:%M")),
            (None, None) => unreachable!(),
        };
        parts.push(format!("«{said}» = {exact}"));
    }
    if let Some(r) = &p.recurrence {
        parts.push(format!("«{}» = повтор: {}", words(&[SpanKind::Repeat]), label(r, "ru")));
    }
    if parts.is_empty() {
        return None;
    }
    Some(format!("В сообщении сроки: {}. Сегодня {} ({}).", parts.join("; "), today.format("%Y-%m-%d"), weekday_ru(today)))
}

fn weekday_ru(d: NaiveDate) -> &'static str {
    ["Пн", "Вт", "Ср", "Чт", "Пт", "Сб", "Вс"][d.weekday().num_days_from_monday() as usize]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn d(y: i32, m: u32, day: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(y, m, day).unwrap()
    }

    fn tm(h: u32, m: u32) -> NaiveTime {
        NaiveTime::from_hms_opt(h, m, 0).unwrap()
    }

    fn rr(freq: Freq, interval: u32, wd: &[u8], md: Option<u8>) -> Recurrence {
        Recurrence { freq, interval, weekdays: wd.to_vec(), monthday: md }
    }

    /// Опорная дата TS-тестов: 2026-06-18, четверг.
    fn p(text: &str) -> Parsed {
        parse(text, d(2026, 6, 18), tm(10, 0))
    }

    /// Опорная дата Python-тестов: 2026-08-21, пятница.
    fn pf(text: &str) -> Parsed {
        parse(text, d(2026, 8, 21), tm(10, 0))
    }

    // ---- ruParser.test.ts: относительные дни ----

    #[test]
    fn ts_relative_today() {
        let r = p("купить хлеб сегодня");
        assert_eq!(r.due_date, Some(d(2026, 6, 18)));
        assert_eq!(r.spans, vec![Span { start: 12, end: 19, kind: SpanKind::Date }]);
    }

    #[test]
    fn ts_relative_tomorrow() {
        assert_eq!(p("позвонить маме завтра").due_date, Some(d(2026, 6, 19)));
    }

    #[test]
    fn ts_relative_day_after_tomorrow() {
        assert_eq!(p("послезавтра встреча").due_date, Some(d(2026, 6, 20)));
    }

    #[test]
    fn ts_no_time_words_no_matches() {
        let r = p("просто задача");
        assert_eq!(r.due_date, None);
        assert!(r.spans.is_empty());
    }

    // ---- ruParser.test.ts: «через N …» ----

    #[test]
    fn ts_in_days() {
        assert_eq!(p("сдать через 3 дня").due_date, Some(d(2026, 6, 21)));
    }

    #[test]
    fn ts_in_week_without_number_is_one() {
        assert_eq!(p("отпуск через неделю").due_date, Some(d(2026, 6, 25)));
    }

    #[test]
    fn ts_in_two_weeks() {
        assert_eq!(p("через 2 недели").due_date, Some(d(2026, 7, 2)));
    }

    #[test]
    fn ts_in_month() {
        assert_eq!(p("счёт через месяц").due_date, Some(d(2026, 7, 18)));
    }

    // ---- ruParser.test.ts: дни недели ----

    #[test]
    fn ts_weekday_friday_is_nearest() {
        assert_eq!(p("встреча в пятницу").due_date, Some(d(2026, 6, 19)));
    }

    #[test]
    fn ts_weekday_tuesday() {
        assert_eq!(p("во вторник созвон").due_date, Some(d(2026, 6, 23)));
    }

    #[test]
    fn ts_weekday_today_is_today() {
        assert_eq!(p("в четверг дедлайн").due_date, Some(d(2026, 6, 18)));
    }

    #[test]
    fn ts_weekday_next_friday_is_plus_seven() {
        assert_eq!(p("в следующую пятницу").due_date, Some(d(2026, 6, 26)));
    }

    // ---- ruParser.test.ts: явные даты ----

    #[test]
    fn ts_month_name_future_day() {
        assert_eq!(p("отчёт 1 июля").due_date, Some(d(2026, 7, 1)));
    }

    #[test]
    fn ts_month_name_past_goes_to_next_year() {
        assert_eq!(p("день рождения 10 июня").due_date, Some(d(2027, 6, 10)));
    }

    #[test]
    fn ts_numeric_day_month() {
        assert_eq!(p("дедлайн 01.07").due_date, Some(d(2026, 7, 1)));
    }

    #[test]
    fn ts_numeric_with_year() {
        assert_eq!(p("1.7.2026 старт").due_date, Some(d(2026, 7, 1)));
    }

    // ---- ruParser.test.ts: время ----

    #[test]
    fn ts_time_hhmm() {
        assert_eq!(p("созвон в 18:00").due_time, Some(tm(18, 0)));
    }

    #[test]
    fn ts_time_hour_without_minutes_from_13() {
        assert_eq!(p("в 18 встреча").due_time, Some(tm(18, 0)));
    }

    #[test]
    fn ts_time_morning() {
        assert_eq!(p("подъём в 9 утра").due_time, Some(tm(9, 0)));
    }

    #[test]
    fn ts_time_evening_adds_twelve() {
        assert_eq!(p("в 9 вечера").due_time, Some(tm(21, 0)));
    }

    #[test]
    fn ts_time_noon() {
        assert_eq!(p("обед в полдень").due_time, Some(tm(12, 0)));
    }

    #[test]
    fn ts_time_midnight() {
        assert_eq!(p("в полночь").due_time, Some(tm(0, 0)));
    }

    #[test]
    fn ts_time_minutes_phrase_is_not_time() {
        assert_eq!(p("задача в 5 минут").due_time, None);
    }

    #[test]
    fn ts_date_and_time_together() {
        let r = p("позвонить маме завтра в 18:00");
        assert_eq!(r.due_date, Some(d(2026, 6, 19)));
        assert_eq!(r.due_time, Some(tm(18, 0)));
        assert_eq!(r.spans.len(), 2);
    }

    #[test]
    fn ts_time_three_days_is_fifteen() {
        assert_eq!(p("в 3 дня").due_time, Some(tm(15, 0)));
    }

    #[test]
    fn ts_time_five_days_is_seventeen() {
        assert_eq!(p("в 5 дня").due_time, Some(tm(17, 0)));
    }

    #[test]
    fn ts_time_ten_days_stays_morning() {
        assert_eq!(p("встреча в 10 дня").due_time, Some(tm(10, 0)));
    }

    // ---- ruParser.test.ts: повторы ----

    #[test]
    fn ts_repeat_every_day() {
        assert_eq!(p("зарядка каждый день").recurrence, Some(rr(Freq::Daily, 1, &[], None)));
    }

    #[test]
    fn ts_repeat_daily_word() {
        assert_eq!(p("ежедневно пить воду").recurrence.map(|r| r.freq), Some(Freq::Daily));
    }

    #[test]
    fn ts_repeat_every_friday() {
        assert_eq!(p("отчёт каждую пятницу").recurrence, Some(rr(Freq::Weekly, 1, &[5], None)));
    }

    #[test]
    fn ts_repeat_on_fridays() {
        assert_eq!(p("по пятницам бассейн").recurrence.map(|r| r.weekdays), Some(vec![5]));
    }

    #[test]
    fn ts_repeat_workdays() {
        assert_eq!(p("по будням стендап").recurrence.map(|r| r.weekdays), Some(vec![1, 2, 3, 4, 5]));
    }

    #[test]
    fn ts_repeat_weekends() {
        assert_eq!(p("по выходным прогулка").recurrence.map(|r| r.weekdays), Some(vec![6, 7]));
    }

    #[test]
    fn ts_repeat_every_week_without_days() {
        assert_eq!(p("каждую неделю отчёт").recurrence, Some(rr(Freq::Weekly, 1, &[], None)));
    }

    #[test]
    fn ts_repeat_monthly_on_fifth() {
        assert_eq!(p("ежемесячно 5-го платёж").recurrence, Some(rr(Freq::Monthly, 1, &[], Some(5))));
    }

    #[test]
    fn ts_repeat_every_month() {
        assert_eq!(p("каждый месяц аренда").recurrence.map(|r| r.freq), Some(Freq::Monthly));
    }

    #[test]
    fn ts_repeat_every_year() {
        assert_eq!(p("каждый год ТО").recurrence.map(|r| r.freq), Some(Freq::Yearly));
    }

    #[test]
    fn ts_repeat_weekly_word() {
        assert_eq!(p("еженедельно созвон").recurrence, Some(rr(Freq::Weekly, 1, &[], None)));
    }

    #[test]
    fn ts_repeat_yearly_word() {
        assert_eq!(p("ежегодно ТО").recurrence.map(|r| r.freq), Some(Freq::Yearly));
    }

    // ---- test_fallback_parser.py ----

    #[test]
    fn py_injection_keeps_repeat_and_date() {
        let r = pf("каждые четыре недели начиная с 27.08.26г. напоминай о необходимости сделать укол козантикс");
        assert_eq!(r.recurrence, Some(rr(Freq::Weekly, 4, &[], None)));
        assert_eq!(r.due_date, Some(d(2026, 8, 27)));
        assert_eq!(r.title, "Напоминай о необходимости сделать укол козантикс");
    }

    #[test]
    fn py_weekday_list_and_time_range() {
        let r = pf("каждый понедельник и среду с 14:00 до 15:00 у меня ученица");
        assert_eq!(r.recurrence, Some(rr(Freq::Weekly, 1, &[1, 3], None)));
        assert_eq!(r.due_time, Some(tm(14, 0)));
        assert!(!r.title.contains("14:00"));
        assert!(!r.title.split_whitespace().any(|w| w == "до"));
    }

    #[test]
    fn py_tomorrow_and_hour_in_words() {
        let r = pf("Позвонить завтра в 11 часов и записаться");
        assert_eq!(r.due_date, Some(d(2026, 8, 22)));
        assert_eq!(r.due_time, Some(tm(11, 0)));
        assert!(r.title.starts_with("Позвонить"));
        assert!(!r.title.to_lowercase().contains("завтра"));
    }

    #[test]
    fn py_time_with_minutes() {
        let r = pf("Позвонить Олегу в 15:30");
        assert_eq!(r.due_time, Some(tm(15, 30)));
        assert_eq!(r.recurrence, None);
        assert_eq!(r.title, "Позвонить Олегу");
    }

    #[test]
    fn py_daily_words() {
        for text in ["ежедневно пить воду", "каждый день пить воду"] {
            let r = pf(text);
            assert_eq!(r.recurrence, Some(rr(Freq::Daily, 1, &[], None)), "{text}");
            assert_eq!(r.title, "Пить воду", "{text}");
        }
    }

    #[test]
    fn py_once_a_month() {
        let r = pf("раз в месяц платить за интернет");
        assert_eq!(r.recurrence.map(|x| x.freq), Some(Freq::Monthly));
        assert_eq!(r.title, "Платить за интернет");
    }

    #[test]
    fn py_bare_date_without_year_takes_coming_one() {
        let r = pf("Сдать отчёт 05.09");
        assert_eq!(r.due_date, Some(d(2026, 9, 5)));
        assert_eq!(r.title, "Сдать отчёт");
    }

    #[test]
    fn py_short_text_survives_untouched() {
        let r = pf("Дело 1");
        assert_eq!((r.title.as_str(), r.due_date, r.due_time, r.recurrence), ("Дело 1", None, None, None));
    }

    #[test]
    fn py_does_not_invent_time_from_bare_number() {
        let r = pf("Составить список из 3 клиник по отопластике");
        assert_eq!(r.due_time, None);
        assert_eq!(r.recurrence, None);
        assert_eq!(r.title, "Составить список из 3 клиник по отопластике");
    }

    #[test]
    fn py_does_not_invent_date_from_price() {
        assert_eq!(pf("Купить монитор за 25.000 рублей").due_date, None);
    }

    #[test]
    fn py_empty_leftover_falls_back_to_whole_phrase() {
        let r = pf("завтра в 10:00");
        assert_eq!(r.due_date, Some(d(2026, 8, 22)));
        assert_eq!(r.due_time, Some(tm(10, 0)));
        assert_eq!(r.title, "завтра в 10:00");
    }

    // ---- test_recurrence.py ----

    #[test]
    fn py_rec_clean_weekdays() {
        let r: Recurrence = serde_json::from_value(json!({"freq": "weekly", "weekdays": [5, 1, 1, 9, -1]})).unwrap();
        assert_eq!(r.interval, 1);
        assert_eq!(r.weekdays, vec![1, 5]);
    }

    #[test]
    fn py_first_due_daily_is_today() {
        assert_eq!(first_due(&rr(Freq::Daily, 1, &[], None), d(2026, 6, 14)), d(2026, 6, 14));
    }

    #[test]
    fn py_first_due_weekly_workdays() {
        assert_eq!(first_due(&rr(Freq::Weekly, 1, &[1, 2, 3, 4, 5], None), d(2026, 6, 14)), d(2026, 6, 15));
    }

    #[test]
    fn py_first_due_monthly_rolls_when_day_passed() {
        assert_eq!(first_due(&rr(Freq::Monthly, 1, &[], Some(5)), d(2026, 6, 14)), d(2026, 7, 5));
    }

    #[test]
    fn py_first_due_monthly_same_month_when_ahead() {
        assert_eq!(first_due(&rr(Freq::Monthly, 1, &[], Some(20)), d(2026, 6, 14)), d(2026, 6, 20));
    }

    #[test]
    fn py_first_due_monthly_clamps_short_month() {
        assert_eq!(first_due(&rr(Freq::Monthly, 1, &[], Some(31)), d(2026, 2, 1)), d(2026, 2, 28));
    }

    #[test]
    fn py_next_daily_interval() {
        let r = rr(Freq::Daily, 3, &[], None);
        assert_eq!(next_occurrence_not_before(&r, d(2026, 6, 14), d(2026, 6, 1)), d(2026, 6, 17));
    }

    #[test]
    fn py_next_daily_skips_past_to_today() {
        let r = rr(Freq::Daily, 1, &[], None);
        assert_eq!(next_occurrence_not_before(&r, d(2026, 6, 1), d(2026, 6, 14)), d(2026, 6, 14));
    }

    #[test]
    fn py_next_weekly_workdays() {
        let r = rr(Freq::Weekly, 1, &[1, 2, 3, 4, 5], None);
        assert_eq!(next_occurrence_not_before(&r, d(2026, 6, 15), d(2026, 6, 1)), d(2026, 6, 16));
    }

    #[test]
    fn py_next_weekly_every_two_weeks() {
        let r = rr(Freq::Weekly, 2, &[1], None);
        assert_eq!(next_occurrence_not_before(&r, d(2026, 6, 15), d(2026, 6, 1)), d(2026, 6, 29));
    }

    #[test]
    fn py_next_monthly_same_day() {
        let r = rr(Freq::Monthly, 1, &[], Some(15));
        assert_eq!(next_occurrence_not_before(&r, d(2026, 6, 15), d(2026, 6, 1)), d(2026, 7, 15));
    }

    #[test]
    fn py_next_monthly_clamps_february() {
        let r = rr(Freq::Monthly, 1, &[], Some(31));
        assert_eq!(next_occurrence_not_before(&r, d(2026, 1, 31), d(2026, 1, 1)), d(2026, 2, 28));
    }

    #[test]
    fn py_next_yearly_clamps_leap_day() {
        let r = rr(Freq::Yearly, 1, &[], None);
        assert_eq!(next_occurrence_not_before(&r, d(2024, 2, 29), d(2024, 3, 1)), d(2025, 2, 28));
    }

    #[test]
    fn py_label_strings() {
        assert_eq!(label(&rr(Freq::Daily, 1, &[], None), "ru"), "Каждый день");
        assert_eq!(label(&rr(Freq::Weekly, 1, &[1, 2, 3, 4, 5], None), "ru"), "По будням");
        assert_eq!(label(&rr(Freq::Weekly, 1, &[1], None), "ru"), "Каждую неделю по понедельникам");
        assert_eq!(label(&rr(Freq::Weekly, 2, &[1], None), "ru"), "Раз в 2 недели по понедельникам");
        assert_eq!(label(&rr(Freq::Monthly, 1, &[], Some(5)), "ru"), "Каждый месяц, 5-го");
        assert_eq!(label(&rr(Freq::Yearly, 1, &[], None), "ru"), "Каждый год");
    }

    #[test]
    fn py_next_biweekly_in_contract() {
        // after — сам срок (вторник); следующий двухнедельный вторник через две недели
        let r = rr(Freq::Weekly, 2, &[2], None);
        assert_eq!(next_occurrence_not_before(&r, d(2026, 6, 16), d(2026, 6, 1)), d(2026, 6, 30));
    }

    #[test]
    fn py_label_weekday_sets_are_readable() {
        assert_eq!(label(&rr(Freq::Weekly, 1, &[1, 2, 3, 4, 5, 6], None), "ru"), "Каждую неделю по пн–сб");
        assert_eq!(label(&rr(Freq::Weekly, 1, &[1, 3, 5], None), "ru"), "Каждую неделю по пн, ср, пт");
        assert_eq!(label(&rr(Freq::Weekly, 1, &[6, 7], None), "ru"), "По выходным");
        assert_eq!(label(&rr(Freq::Weekly, 1, &[1, 2, 3, 4, 5, 6, 7], None), "ru"), "Каждый день");
        assert_eq!(label(&rr(Freq::Weekly, 2, &[1, 3, 5], None), "ru"), "Раз в 2 недели по пн, ср, пт");
    }

    // ---- recurrence.test.ts (даты и подписи; пресеты интерфейса не переносились) ----

    #[test]
    fn ts_label_saturdays() {
        assert_eq!(label(&rr(Freq::Weekly, 1, &[6], None), "ru"), "Каждую неделю по субботам");
    }

    #[test]
    fn ts_label_workdays() {
        assert_eq!(label(&rr(Freq::Weekly, 1, &[1, 2, 3, 4, 5], None), "ru"), "По будням");
    }

    #[test]
    fn ts_label_monthly_third() {
        assert_eq!(label(&rr(Freq::Monthly, 1, &[], Some(3)), "ru"), "Каждый месяц, 3-го");
    }

    #[test]
    fn ts_first_due_weekly_on_or_after_today() {
        assert_eq!(first_due(&rr(Freq::Weekly, 1, &[6], None), d(2026, 6, 15)), d(2026, 6, 20));
    }

    #[test]
    fn ts_first_due_monthly_this_or_next() {
        let r = rr(Freq::Monthly, 1, &[], Some(3));
        assert_eq!(first_due(&r, d(2026, 6, 1)), d(2026, 6, 3));
        assert_eq!(first_due(&r, d(2026, 6, 10)), d(2026, 7, 3));
    }

    #[test]
    fn ts_next_yearly_due_is_next_future_date() {
        // nextYearlyDue(8, 2, today): после последнего срока 2026-03-08 ряд даёт следующий
        let r = rr(Freq::Yearly, 1, &[], None);
        assert_eq!(next_occurrence_not_before(&r, d(2026, 3, 8), d(2026, 6, 1)), d(2027, 3, 8));
        assert_eq!(next_occurrence_not_before(&r, d(2025, 3, 8), d(2026, 1, 1)), d(2026, 3, 8));
    }

    #[test]
    fn ts_label_range_is_dash() {
        assert_eq!(label(&rr(Freq::Weekly, 1, &[1, 2, 3, 4, 5, 6], None), "ru"), "Каждую неделю по пн–сб");
    }

    #[test]
    fn ts_label_scattered_list() {
        assert_eq!(label(&rr(Freq::Weekly, 1, &[1, 3, 5], None), "ru"), "Каждую неделю по пн, ср, пт");
    }

    #[test]
    fn ts_label_weekends_and_all_days() {
        assert_eq!(label(&rr(Freq::Weekly, 1, &[6, 7], None), "ru"), "По выходным");
        assert_eq!(label(&rr(Freq::Weekly, 1, &[1, 2, 3, 4, 5, 6, 7], None), "ru"), "Каждый день");
    }

    #[test]
    fn ts_label_keeps_interval() {
        assert_eq!(label(&rr(Freq::Weekly, 2, &[1, 3, 5], None), "ru"), "Раз в 2 недели по пн, ср, пт");
    }

    // ---- признаки «полный разбор» и вывод для маршрута ----

    #[test]
    fn complete_for_plain_repeat_and_task() {
        let r = p("каждую среду вынести мусор");
        assert!(r.complete);
        assert_eq!(r.title, "Вынести мусор");
    }

    #[test]
    fn not_complete_for_checklist_or_several_tasks() {
        assert!(!p("купить хлеб;молоко завтра").complete);
        assert!(!pf("Позвонить завтра в 11 часов и записаться").complete);
    }

    #[test]
    fn stray_hour_without_minutes_keeps_phrase_incomplete() {
        // «в 10» без минут и части суток не время: код его не понял, значит фраза не полная
        let r = p("позвонить маме завтра в 10");
        assert_eq!(r.due_time, None);
        assert!(!r.complete);
    }

    #[test]
    fn json_shape_for_route() {
        let v = to_json(&p("позвонить маме завтра в 18:00"));
        assert_eq!(v["due_date"], "2026-06-19");
        assert_eq!(v["due_time"], "18:00");
        assert_eq!(v["recurrence"], Value::Null);
        assert_eq!(v["spans"][0]["kind"], "date");
    }
    #[test]
    fn note_gives_exact_dates() {
        let n = note("завтра в 10:00 сделать отчёт", d(2026, 11, 16), tm(9, 0)).unwrap();
        assert!(n.contains("«завтра в 10:00» = 2026-11-17 10:00 (Вт)"), "{n}");
        assert!(note("просто текст", d(2026, 11, 16), tm(9, 0)).is_none());
    }
}
