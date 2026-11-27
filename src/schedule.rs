//! Когда рутине пора: «каждые N минут» с окном и днями недели или «в 09:00 по будням».
//! Всё считается в местном времени компьютера: человек думает о ночи, а не о UTC.

use chrono::{Datelike, Duration, Local, LocalResult, NaiveDate, NaiveDateTime, NaiveTime, TimeZone, Timelike, Utc};

use serde::{Deserialize, Serialize};


pub fn tz_valid(name: &str) -> bool {
    let name = name.trim();
    name.is_empty() || name.parse::<chrono_tz::Tz>().is_ok()
}

pub trait AsSchedule {
    fn as_schedule(&self) -> Schedule;
}

impl AsSchedule for Timing {
    fn as_schedule(&self) -> Schedule {
        self.schedule()
    }
}

impl AsSchedule for Schedule {
    fn as_schedule(&self) -> Schedule {
        self.clone()
    }
}

/// Расписание «плоским» видом: так его вводит человек в форме, так его правят боты и так оно лежит у напоминаний.
/// Расписание триггера процесса — `Schedule`; оба вида переводятся друг в друга.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", default)]
pub struct Timing {
    pub every_minutes: i64,
    /// Верх случайного интервала: срок выпадает между `every_minutes` и этим числом, чтобы запуски
    /// не шли ровной сеткой. 0 или не больше `every_minutes` — интервал ровный.
    pub every_max_minutes: i64,
    /// «09:00»: запускать в это время, а не каждые N минут.
    pub at: String,
    /// Cron-выражение (5 полей). Если задано, заменяет every/at/days/window.
    pub cron: String,
    /// Часовой пояс IANA для cron. Пусто — местное время ПК.
    pub tz: String,
    /// Дни недели, 1 — понедельник. Пусто — каждый день.
    pub days: Vec<i64>,
    /// Повтор по тому же правилу, что у задач (dates_ru::Recurrence). Пусто — нет правила.
    pub recurrence: Option<crate::dates_ru::Recurrence>,
    /// Не раньше этой даты (YYYY-MM-DD, местное время), пусто — без ограничения.
    pub start_on: String,
    /// Разовый запуск в миллисекундах; 0 — повторяющийся.
    pub once_at: i64,
    /// Окно работы, может переходить через полночь: «22:00»–«07:00». Пусто — всегда.
    pub window_from: String,
    pub window_to: String,
}

impl Default for Timing {
    fn default() -> Self {
        Self { every_minutes: 60, every_max_minutes: 0, at: String::new(), cron: String::new(), tz: String::new(), days: Vec::new(), recurrence: None, start_on: String::new(), once_at: 0, window_from: String::new(), window_to: String::new() }
    }
}

impl Timing {
    pub fn schedule(&self) -> Schedule {
        if !self.cron.trim().is_empty() {
            return Schedule {
                kind: "cron".into(),
                cron: Some(self.cron.clone()),
                tz: if self.tz.trim().is_empty() { None } else { Some(self.tz.clone()) },
                ..Schedule::default()
            };
        }
        if self.once_at > 0 {
            let l = local(self.once_at);
            return Schedule {
                kind: "once".into(),
                date: Some(l.date().format("%Y-%m-%d").to_string()),
                time: Some(l.time().format("%H:%M").to_string()),
                ..Schedule::default()
            };
        }
        let days = if self.days.is_empty() { None } else { Some(self.days.clone()) };
        if parse_time(&self.at).is_some() {
            return Schedule {
                kind: "daily".into(),
                time: Some(self.at.clone()),
                days,
                recurrence: self.recurrence.clone(),
                start_on: if self.start_on.trim().is_empty() { None } else { Some(self.start_on.clone()) },
                ..Schedule::default()
            };
        }
        Schedule {
            kind: "interval".into(),
            every_minutes: Some(self.every_minutes),
            every_max_minutes: if self.every_max_minutes > self.every_minutes { Some(self.every_max_minutes) } else { None },
            days,
            window_from: if self.window_from.trim().is_empty() { None } else { Some(self.window_from.clone()) },
            window_to: if self.window_to.trim().is_empty() { None } else { Some(self.window_to.clone()) },
            ..Schedule::default()
        }
    }

    /// Расписание триггера обратно в плоский вид. Виды, которых в форме нет (их рисует человек на холсте), сводятся к ближайшему.
    pub fn of(s: &Schedule) -> Self {
        let mut t = Self::default();
        let days = s.days.clone().unwrap_or_default();
        match s.kind.as_str() {
            "cron" => {
                t.cron = s.cron.clone().unwrap_or_default();
                t.tz = s.tz.clone().unwrap_or_default();
            }
            "once" => t.once_at = schedule_next(s, 0).unwrap_or(0),
            "interval" => {
                t.every_minutes = s.every_minutes.or_else(|| s.interval_minutes().map(|m| m as i64)).unwrap_or(60);
                t.every_max_minutes = s.every_max_minutes.unwrap_or(0);
                t.window_from = s.window_from.clone().unwrap_or_default();
                t.window_to = s.window_to.clone().unwrap_or_default();
                t.days = days;
            }
            "hourly" => t.every_minutes = 60,
            kind => {
                t.at = s.time.clone().unwrap_or_default();
                t.days = match kind {
                    "weekdays" => vec![1, 2, 3, 4, 5],
                    "weekly" => vec![s.weekday.unwrap_or(1)],
                    _ => days,
                };
                t.recurrence = s.recurrence.clone();
                t.start_on = s.start_on.clone().unwrap_or_default();
            }
        }
        if t.every_minutes == 0 {
            t.every_minutes = 60;
        }
        t
    }

    /// Поля расписания из запроса. Формат времени проверяет сервер, пустая строка снимает значение.
    pub fn apply(&mut self, input: &serde_json::Value) -> Result<(), String> {
        use crate::store::number;
        if let Some(max) = input.get("everyMaxMinutes") {
            self.every_max_minutes = number(Some(max)).filter(|x| x.is_finite() && *x > 0.0).map_or(0, |x| x as i64);
        }
        if let Some(at) = input.get("at").and_then(|v| v.as_str()) {
            self.at = at.trim().to_string();
        }
        if let Some(days) = input.get("days").and_then(|v| v.as_array()) {
            let mut wanted: Vec<i64> = days.iter().filter_map(|d| d.as_i64()).filter(|d| (1..=7).contains(d)).collect();
            wanted.sort_unstable();
            wanted.dedup();
            self.days = wanted;
        }
        if let Some(from) = input.get("windowFrom").and_then(|v| v.as_str()) {
            self.window_from = from.trim().to_string();
        }
        if let Some(to) = input.get("windowTo").and_then(|v| v.as_str()) {
            self.window_to = to.trim().to_string();
        }
        if let Some(v) = input.get("recurrence") {
            if v.is_null() {
                self.recurrence = None;
            } else if let Ok(rec) = serde_json::from_value(v.clone()) {
                self.recurrence = Some(rec);
            }
        }
        if let Some(v) = input.get("startOn").or_else(|| input.get("start_on")) {
            self.start_on = v.as_str().unwrap_or_default().trim().to_string();
        }
        if let Some(v) = input.get("onceAt").or_else(|| input.get("once_at")) {
            self.once_at = v.as_i64().unwrap_or(0);
        }
        if let Some(v) = input.get("cron") {
            let s = v.as_str().ok_or("поле cron должно быть строкой")?.trim().to_string();
            if !s.is_empty() {
                parse_cron(&s).map_err(|e| format!("поле cron: {e}"))?;
            }
            self.cron = s;
        }
        if let Some(v) = input.get("tz") {
            let s = v.as_str().ok_or("поле tz должно быть строкой")?.trim().to_string();
            if !s.is_empty() && !tz_valid(&s) {
                return Err("поле tz: неизвестный часовой пояс".into());
            }
            self.tz = s;
        }
        if !self.cron.is_empty() && self.once_at > 0 {
            return Err("cron несовместим с разовым запуском (onceAt)".into());
        }
        Ok(())
    }
}

/// Последний пропущенный запуск догоняем, если компьютер проспал меньше этого: сводка в 10 ещё нужна, через день нет.
pub const CATCH_UP_MS: i64 = 12 * 3_600_000;

pub fn parse_time(text: &str) -> Option<NaiveTime> {
    let (hours, minutes) = text.trim().split_once(':')?;
    if hours.len() != 2 || minutes.len() != 2 {
        return None;
    }
    NaiveTime::from_hms_opt(hours.parse().ok()?, minutes.parse().ok()?, 0)
}

/// Поля времени запроса («at», «windowFrom», «windowTo») — пустые или ЧЧ:ММ. Одна проверка для интерфейса и ботов.
pub fn times_valid(body: &serde_json::Value) -> bool {
    ["at", "windowFrom", "windowTo"].iter().all(|key| match body.get(*key).filter(|v| !v.is_null()) {
        None => true,
        Some(value) => value.as_str().is_some_and(|text| text.trim().is_empty() || parse_time(text).is_some()),
    })
}

/// «catchUp» запроса — нет, «last» или «skip». Одна проверка для интерфейса и ботов.
pub fn catch_up_valid(body: &serde_json::Value) -> bool {
    body.get("catchUp").filter(|v| !v.is_null()).map_or(true, |v| matches!(v.as_str(), Some("last" | "skip")))
}

/// Открыто ли окно расписания сейчас.
pub fn open_now<T: AsSchedule>(x: &T, now_ms: i64) -> bool {
    allowed(x, local(now_ms))
}

fn window(timing: &Timing) -> Option<(NaiveTime, NaiveTime)> {
    let from = parse_time(&timing.window_from)?;
    let to = parse_time(&timing.window_to)?;
    (from != to).then_some((from, to))
}

/// Можно ли работать в этот момент. Ночное окно относится к дню, в который началось.
pub fn allowed<T: AsSchedule>(x: &T, at: NaiveDateTime) -> bool {
    x.as_schedule().allowed(at)
}

/// Следующий срок рутины по часам, которые задаёт тест (без часового пояса компьютера).
#[cfg(test)]
pub(crate) fn next_after(routine: &Timing, now: NaiveDateTime) -> NaiveDateTime {
    if !routine.cron.trim().is_empty() {
        return now + Duration::minutes(routine.every_minutes.max(1));
    }
    let epoch = NaiveDate::from_ymd_opt(1970, 1, 1).unwrap().and_hms_opt(0, 0, 0).unwrap();
    let now_ms = (now - epoch).num_milliseconds();
    let s = routine.schedule();
    match schedule_next_with(&s, now_ms, |ms| epoch + Duration::milliseconds(ms), |t| (t - epoch).num_milliseconds()) {
        Some(ms) => epoch + Duration::milliseconds(ms),
        None => now + Duration::days(1),
    }
}

fn local(ms: i64) -> NaiveDateTime {
    Local.timestamp_millis_opt(ms).single().map_or_else(|| Local::now().naive_local(), |at| at.naive_local())
}

fn to_ms(at: NaiveDateTime) -> i64 {
    // Часа, которого нет из-за перевода часов, не бывает: берём следующий.
    Local
        .from_local_datetime(&at)
        .earliest()
        .or_else(|| Local.from_local_datetime(&(at + Duration::hours(1))).earliest())
        .map_or_else(|| Local::now().timestamp_millis(), |t| t.timestamp_millis())
}

pub fn next_run_ms(routine: &Timing, now_ms: i64) -> i64 {
    if routine.once_at > 0 {
        return routine.once_at;
    }
    let s = routine.schedule();
    schedule_next(&s, now_ms).unwrap_or_else(|| now_ms + routine.every_minutes.clamp(1, 60) * 60_000)
}

/// Интервал до следующего запуска: ровный `every_minutes` или случайный до `every_max_minutes`
/// (`roll` — число из [0, 1)), чтобы запуски не повторялись минута в минуту.
fn roll_minutes(low: i64, high: i64, roll: f64) -> i64 {
    let low = low.max(1);
    if high <= low {
        return low;
    }
    low + ((high - low + 1) as f64 * roll.clamp(0.0, 0.999_999)) as i64
}

/// Час для ожиданий рутин (занятый бот, пауза после ошибок). В отладочной сборке `CREW_TEST_HOUR_MS` ужимает его для проверок.
pub fn hour_ms() -> i64 {
    if cfg!(debug_assertions) {
        if let Some(ms) = std::env::var("CREW_TEST_HOUR_MS").ok().and_then(|v| v.parse().ok()) {
            return ms;
        }
    }
    3_600_000
}

/// Как редко рутина запускается после неудач подряд: от 3 — не чаще раза в час, от 6 — раза в 4 часа.
pub fn failure_gap_ms(failures: u32) -> i64 {
    match failures {
        0..=2 => 0,
        3..=5 => hour_ms(),
        _ => 4 * hour_ms(),
    }
}

const WEEKDAYS_SHORT: [&str; 7] = ["пн", "вт", "ср", "чт", "пт", "сб", "вс"];
const WEEKDAYS_EACH: [&str; 7] = ["понедельникам", "вторникам", "средам", "четвергам", "пятницам", "субботам", "воскресеньям"];

/// Время запуска человеческими словами по местным часам: «пн 11.11 в 09:00».
pub fn describe_moment(ms: i64) -> String {
    let at = local(ms);
    let day = WEEKDAYS_SHORT[at.date().weekday().num_days_from_monday() as usize];
    format!("{day} {} в {}", at.format("%d.%m"), at.format("%H:%M"))
}

/// «Следующий запуск: …» для ответа бота и списка. У выключенной и у вебхука срока нет.
pub fn describe_next(hook: bool, enabled: bool, next_run: i64) -> String {
    if hook {
        "Запускается только вебхуком, по расписанию не идёт.".to_string()
    } else if !enabled {
        "Рутина выключена, запусков не будет.".to_string()
    } else {
        format!("Следующий запуск: {}.", describe_moment(next_run))
    }
}

fn days_text(days: &[i64]) -> String {
    match days {
        [] | [1, 2, 3, 4, 5, 6, 7] => "каждый день".to_string(),
        [1, 2, 3, 4, 5] => "по будням".to_string(),
        [6, 7] => "по выходным".to_string(),
        list => {
            let names: Vec<&str> = list.iter().filter_map(|d| WEEKDAYS_EACH.get((*d - 1) as usize).copied()).collect();
            format!("по {}", names.join(", "))
        }
    }
}

fn describe_cron(expr: &str) -> String {
    let fields: Vec<&str> = expr.split_whitespace().collect();
    if fields.len() != 5 {
        return format!("по cron: {}", expr);
    }
    let (m, h, dom, _mon, dow) = (fields[0], fields[1], fields[2], fields[3], fields[4]);
    let time = || -> Option<String> {
        if m.chars().all(|c| c.is_ascii_digit()) && h.chars().all(|c| c.is_ascii_digit()) {
            let mm: u8 = m.parse().ok()?;
            let hh: u8 = h.parse().ok()?;
            if mm <= 59 && hh <= 23 {
                return Some(format!("{:02}:{:02}", hh, mm));
            }
        }
        None
    };
    // каждые N минут
    if let Some(n) = m.strip_prefix("*/") {
        if h == "*" && dom == "*" && _mon == "*" && dow == "*" {
            if let Ok(n) = n.parse::<u8>() {
                return format!("каждые {} мин", n);
            }
        }
    }
    if let Some(t) = time() {
        if dom == "*" && _mon == "*" && dow == "*" {
            return format!("каждый день в {}", t);
        }
        if dom == "*" && _mon == "*" && (dow == "1-5" || dow == "1,2,3,4,5") {
            return format!("по будням в {}", t);
        }
        if let Ok(d) = dom.parse::<u8>() {
            if _mon == "*" && dow == "*" && (1..=31).contains(&d) {
                return format!("{}-го числа в {}", d, t);
            }
        }
    }
    format!("по cron: {}", expr)
}

/// Расписание словами: «по будням в 09:00», «каждый час», «каждые 30 мин с 22:00 до 07:00».
pub fn describe<T: AsSchedule>(x: &T) -> String {
    x.as_schedule().describe()
}

// ---------------------------------------------------------------------------
// Календарь (.ics): рутины для обычного календаря
// ---------------------------------------------------------------------------

const ICS_DAYS: [&str; 7] = ["MO", "TU", "WE", "TH", "FR", "SA", "SU"];

/// Текст по RFC 5545: обратная косая, точка с запятой, запятая и перевод строки.
fn ics_escape(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars().peekable();
    while let Some(ch) = chars.next() {
        match ch {
            '\\' => out.push_str("\\\\"),
            ';' => out.push_str("\\;"),
            ',' => out.push_str("\\,"),
            '\r' => {
                // CRLF — один перевод строки, а не два.
                if chars.peek() == Some(&'\n') {
                    chars.next();
                }
                out.push_str("\\n");
            }
            '\n' => out.push_str("\\n"),
            _ => out.push(ch),
        }
    }
    out
}

/// Строка не длиннее 75 октетов; продолжение начинается с пробела (он тоже считается).
fn ics_fold(line: &str) -> String {
    let mut out = String::with_capacity(line.len() + 8);
    let (mut used, mut limit) = (0usize, 75usize);
    for ch in line.chars() {
        let len = ch.len_utf8();
        if used + len > limit {
            out.push_str("\r\n ");
            used = 0;
            limit = 74;
        }
        out.push(ch);
        used += len;
    }
    out
}

fn ics_duration(minutes: i64) -> String {
    let minutes = minutes.max(1);
    let (hours, rest) = (minutes / 60, minutes % 60);
    match (hours, rest) {
        (0, rest) => format!("PT{rest}M"),
        (hours, 0) => format!("PT{hours}H"),
        (hours, rest) => format!("PT{hours}H{rest}M"),
    }
}

fn ics_days(days: &[i64]) -> Option<String> {
    let mut days: Vec<i64> = days.iter().copied().filter(|d| (1..=7).contains(d)).collect();
    days.sort_unstable();
    days.dedup();
    (!days.is_empty()).then(|| days.iter().map(|d| ICS_DAYS[(d - 1) as usize]).collect::<Vec<_>>().join(","))
}

fn ics_window_minutes(from: NaiveTime, to: NaiveTime) -> i64 {
    let minutes = to.signed_duration_since(from).num_minutes();
    if minutes > 0 {
        minutes
    } else {
        minutes + 24 * 60
    }
}

fn ics_stamp(now: i64) -> String {
    Utc.timestamp_millis_opt(now).single().unwrap_or_else(Utc::now).format("%Y%m%dT%H%M%SZ").to_string()
}

/// Событие календаря: процесс или напоминание. Выключенные и вебхуки в календарь не попадают.
pub struct IcsItem {
    pub id: String,
    pub title: String,
    /// Имя бота; у напоминания пусто.
    pub bot: String,
    pub prompt: String,
    pub timing: Timing,
}

/// Календарь процессов и напоминаний: по одному событию на каждое, cron пропускаем.
/// Время местное и без TZID: календарь человека сам покажет его в своих часах.
pub fn tasks_ics(items: &[IcsItem], now: i64) -> String {
    let mut lines = vec![
        "BEGIN:VCALENDAR".to_string(),
        "VERSION:2.0".to_string(),
        "PRODID:-//Deloist//Bots//RU".to_string(),
        "CALSCALE:GREGORIAN".to_string(),
        "METHOD:PUBLISH".to_string(),
    ];
    let stamp = format!("DTSTAMP:{}", ics_stamp(now));
    let today = local(now).date();
    for item in items.iter().filter(|i| i.timing.cron.trim().is_empty()) {
        // cron пропускаем: ICS не умеет произвольный cron.
        let routine = &item.timing;
        let mut title = if item.bot.is_empty() {
            item.title.clone()
        } else {
            format!("{}: {}", item.bot, item.title)
        };
        let (start, duration, rule) = if let Some(at) = parse_time(&routine.at) {
            let rule = ics_days(&routine.days).map_or_else(|| "FREQ=DAILY".to_string(), |days| format!("FREQ=WEEKLY;BYDAY={days}"));
            (today.and_time(at), 15, rule)
        } else if let Some((from, to)) = window(routine) {
            // «Каждые N минут внутри окна» календарь не умеет: показываем окно и пишем интервал в названии.
            title = format!("{title} (каждые {} мин)", routine.every_minutes.max(1));
            (today.and_time(from), ics_window_minutes(from, to), "FREQ=DAILY".to_string())
        } else {
            let current = local(now);
            let start = current.with_second(0).unwrap_or(current);
            let rule = format!("FREQ=MINUTELY;INTERVAL={}", routine.every_minutes.max(1));
            (start, 15, rule)
        };
        lines.push("BEGIN:VEVENT".to_string());
        lines.push(format!("UID:{}@deloist", item.id));
        lines.push(stamp.clone());
        lines.push(format!("DTSTART:{}", start.format("%Y%m%dT%H%M%S")));
        lines.push(format!("DURATION:{}", ics_duration(duration)));
        lines.push(format!("RRULE:{rule}"));
        lines.push(format!("SUMMARY:{}", ics_escape(&title)));
        let prompt: String = item.prompt.chars().take(300).collect();
        lines.push(format!("DESCRIPTION:{}", ics_escape(&prompt)));
        lines.push("END:VEVENT".to_string());
    }
    lines.push("END:VCALENDAR".to_string());

    let mut out = String::new();
    for line in &lines {
        out.push_str(&ics_fold(line));
        out.push_str("\r\n");
    }
    out
}

// ---------------------------------------------------------------------------
// Cron (5 полей).
// ---------------------------------------------------------------------------

#[derive(Clone, Debug)]
pub struct Cron {
    pub minutes: Vec<bool>,
    pub hours: Vec<bool>,
    pub dom: Vec<bool>,
    pub months: Vec<bool>,
    pub dow: Vec<bool>,
    pub dom_restricted: bool,
    pub dow_restricted: bool,
}

fn parse_cron_field(s: &str, min: u8, max: u8, map7: bool) -> Result<(Vec<bool>, bool), String> {
    let restricted = s.trim() != "*";
    let mut allowed = vec![false; (max - min + 1) as usize];
    for part in s.split(',') {
        let part = part.trim();
        if part.is_empty() {
            return Err("пустой элемент списка".into());
        }
        let (range, step) = if let Some((r, st)) = part.split_once('/') {
            let step: u8 = st.trim().parse().map_err(|_| "шаг должен быть положительным числом".to_string())?;
            if step == 0 {
                return Err("шаг должен быть больше 0".into());
            }
            (r.trim(), step)
        } else {
            (part, 1)
        };
        let (lo, hi) = if range == "*" {
            (min, max)
        } else if let Some((a, b)) = range.split_once('-') {
            let a: u8 = a.trim().parse().map_err(|_| "неверное число".to_string())?;
            let b: u8 = b.trim().parse().map_err(|_| "неверное число".to_string())?;
            (a, b)
        } else {
            let v: u8 = range.parse().map_err(|_| "неверное число".to_string())?;
            (v, v)
        };
        if lo < min || hi > max || lo > hi {
            return Err(format!("значение вне диапазона {}-{}", min, max));
        }
        let mut v = lo;
        while v <= hi {
            let mut x = v;
            if map7 && x == 7 {
                x = 0;
            }
            if x >= min && x <= max {
                allowed[(x - min) as usize] = true;
            }
            if step as u16 > (hi - v) as u16 {
                break;
            }
            v += step;
            if v == 0 && step > 0 {
                // avoid infinite loop on wrap; not expected
                break;
            }
        }
    }
    if !allowed.iter().any(|x| *x) {
        return Err("ни одно значение не выбрано".into());
    }
    Ok((allowed, restricted))
}

pub fn parse_cron(expr: &str) -> Result<Cron, String> {
    let fields: Vec<&str> = expr.split_whitespace().collect();
    if fields.len() != 5 {
        return Err("нужно 5 полей: минута час день-месяца месяц день-недели".into());
    }
    let (minutes, _) = parse_cron_field(fields[0], 0, 59, false)?;
    let (hours, _) = parse_cron_field(fields[1], 0, 23, false)?;
    let (dom, dom_restricted) = parse_cron_field(fields[2], 1, 31, false)?;
    let (months, _) = parse_cron_field(fields[3], 1, 12, false)?;
    // 0..=7 даёт 8 ячеек, 7 уже сложено в воскресенье (0): оставляем 7.
    let (mut dow, dow_restricted) = parse_cron_field(fields[4], 0, 7, true)?;
    dow.truncate(7);
    let cron = Cron { minutes, hours, dom, months, dow, dom_restricted, dow_restricted };
    // Выражение без единого дня в календаре («0 0 31 2 *») сохранить нельзя: оно не запустится никогда.
    let today = Local::now().date_naive();
    if !(0..CRON_HORIZON_DAYS).any(|d| cron_day_matches(&cron, today + Duration::days(d))) {
        return Err("выражение не срабатывает ни в один день ближайших 8 лет".into());
    }
    Ok(cron)
}

/// Горизонт поиска запуска: 29 февраля бывает раз в 4 года, с запасом на пропуск високосного 2100-го.
const CRON_HORIZON_DAYS: i64 = 8 * 366;

fn cron_day_matches(cron: &Cron, date: NaiveDate) -> bool {
    if !cron.months[date.month0() as usize] {
        return false;
    }
    let dom_match = cron.dom[date.day0() as usize];
    let dow_match = cron.dow[date.weekday().num_days_from_sunday() as usize];
    // Как в классическом cron: если ограничены и день месяца, и день недели, достаточно любого из них.
    if cron.dom_restricted && cron.dow_restricted {
        dom_match || dow_match
    } else {
        dom_match && dow_match
    }
}

// ---------------------------------------------------------------------------
// Расписание запускающего узла процесса (бывшее workflows::Schedule).
// ---------------------------------------------------------------------------

pub const MIN_INTERVAL_MINUTES: f64 = 3.0;

const SCHEDULE_KINDS: [&str; 8] = ["hourly", "daily", "weekdays", "weekly", "monthly", "interval", "once", "cron"];

const WEEKDAYS_EACH_WF: [&str; 7] = ["понедельникам", "вторникам", "средам", "четвергам", "пятницам", "субботам", "воскресеньям"];

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct Schedule {
    pub kind: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub time: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub minute: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub weekday: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub day: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub amount: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub unit: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub date: Option<String>,
    // --- routine-derived fields ---
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub days: Option<Vec<i64>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub every_minutes: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub every_max_minutes: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub window_from: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub window_to: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cron: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tz: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub recurrence: Option<crate::dates_ru::Recurrence>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub start_on: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hook: Option<bool>,
}

impl Schedule {
    pub fn validate(&self) -> Result<(), String> {
        if !SCHEDULE_KINDS.contains(&self.kind.as_str()) {
            return Err("неизвестный вид расписания".into());
        }
        let time_ok = || self.time.as_deref().and_then(parse_time).is_some();
        match self.kind.as_str() {
            "hourly" => {
                if self.minute.is_some_and(|m| !(0..=59).contains(&m)) {
                    return Err("минута от 0 до 59".into());
                }
            }
            "daily" | "weekdays" => {
                if !time_ok() {
                    return Err("нужно время ЧЧ:ММ".into());
                }
                if let Some(days) = &self.days {
                    if !days.iter().all(|d| (1..=7).contains(d)) {
                        return Err("день недели от 1 до 7".into());
                    }
                }
                if let Some(rule) = &self.recurrence {
                    let _ = rule;
                }
                if let Some(tz) = &self.tz {
                    if !tz.trim().is_empty() && !tz_valid(tz) { return Err("неверный часовой пояс".into()); }
                }
            }
            "weekly" => {
                if !time_ok() {
                    return Err("нужно время ЧЧ:ММ".into());
                }
                if !self.weekday.is_some_and(|d| (1..=7).contains(&d)) {
                    return Err("день недели от 1 до 7".into());
                }
            }
            "monthly" => {
                if !time_ok() {
                    return Err("нужно время ЧЧ:ММ".into());
                }
                if !self.day.is_some_and(|d| (1..=31).contains(&d)) {
                    return Err("число месяца от 1 до 31".into());
                }
            }
            "interval" => {
                if let Some(low) = self.every_minutes {
                    if low < 1 { return Err("интервал не меньше 1 минуты".into()); }
                    if let Some(high) = self.every_max_minutes {
                        if high > 0 && high < low { return Err("верх интервала не меньше нижнего".into()); }
                    }
                } else {
                    match self.interval_minutes() {
                        None => return Err("нужны количество и единица (минуты, часы, дни)".into()),
                        Some(m) if m < MIN_INTERVAL_MINUTES => return Err("интервал не меньше 3 минут".into()),
                        Some(_) => {}
                    }
                }
                if let Some(days) = &self.days {
                    if !days.iter().all(|d| (1..=7).contains(d)) { return Err("день недели от 1 до 7".into()); }
                }
                for t in [self.window_from.as_deref(), self.window_to.as_deref()].into_iter().flatten() {
                    if !t.trim().is_empty() && parse_time(t).is_none() { return Err("время ЧЧ:ММ".into()); }
                }
            }
            "once" => {
                if !time_ok() {
                    return Err("нужно время ЧЧ:ММ".into());
                }
                if self.date.as_deref().and_then(|d| NaiveDate::parse_from_str(d, "%Y-%m-%d").ok()).is_none() {
                    return Err("нужна дата ГГГГ-ММ-ДД".into());
                }
            }
            "cron" => {
                let expr = self.cron.as_deref().unwrap_or("").trim();
                if expr.is_empty() { return Err("нужно cron-выражение".into()); }
                parse_cron(expr).map_err(|e| e)?;
                if let Some(tz) = &self.tz {
                    if !tz.trim().is_empty() && !tz_valid(tz) { return Err("неверный часовой пояс".into()); }
                }
            }
            _ => {}
        }
        Ok(())
    }

    pub(crate) fn interval_minutes(&self) -> Option<f64> {
        let amount = self.amount?;
        let per = match self.unit.as_deref()? {
            "minutes" => 1.0,
            "hours" => 60.0,
            "days" => 1440.0,
            _ => return None,
        };
        (amount.is_finite() && amount > 0.0).then_some(amount * per)
    }

    pub fn describe(&self) -> String {
        if self.hook == Some(true) {
            return "по вебхуку".to_string();
        }
        if self.kind == "cron" {
            let expr = self.cron.clone().unwrap_or_default();
            let mut text = describe_cron(&expr);
            if let Some(tz) = &self.tz {
                if !tz.trim().is_empty() {
                    text = format!("{} ({})", text, tz.trim());
                }
            }
            return text;
        }
        if self.kind == "once" {
            let time = self.time.clone().unwrap_or_default();
            return format!("один раз {} в {time}", self.date.clone().unwrap_or_default());
        }
        if let Some(time) = &self.time {
            if self.kind == "weekdays" && self.days.is_none() {
                return format!("по будням в {time}");
            }
            if self.kind == "weekly" {
                return format!("по {} в {time}", WEEKDAYS_EACH_WF[(self.weekday.unwrap_or(1).clamp(1, 7) - 1) as usize]);
            }
            if self.kind == "monthly" {
                return format!("каждого {}-го числа в {time}", self.day.unwrap_or(1));
            }
            if self.kind == "hourly" {
                return format!("каждый час в {:02} минут", self.minute.unwrap_or(0));
            }
            let days = self.days.clone().unwrap_or_default();
            let days_txt = days_text(&days);
            if self.recurrence.is_some() {
                return format!("{} в {}", days_txt, time);
            }
            if self.days.is_some() {
                return format!("{} в {}", days_txt, time);
            }
            if self.kind == "daily" {
                return format!("каждый день в {time}");
            }
            // default at-time
            return format!("{} в {}", days_txt, time);
        }
        if self.kind == "hourly" {
            return format!("каждый час в {:02} минут", self.minute.unwrap_or(0));
        }
        if self.kind == "interval" {
            if let Some(low) = self.every_minutes {
                let high = self.every_max_minutes.unwrap_or(0);
                let every = match low {
                    l if high > l => format!("каждые {l}–{high} мин, случайно"),
                    60 => "каждый час".to_string(),
                    m if m > 60 && m % 60 == 0 => format!("каждые {} ч", m / 60),
                    m => format!("каждые {m} мин"),
                };
                let mut text = every;
                let days = self.days.clone().unwrap_or_default();
                if !days.is_empty() {
                    text = format!("{text}, {}", days_text(&days));
                }
                if let (Some(f), Some(t)) = (&self.window_from, &self.window_to) {
                    if !f.trim().is_empty() && !t.trim().is_empty() {
                        text = format!("{text}, с {} до {}", f.trim(), t.trim());
                    }
                }
                return text;
            }
            let amount = self.amount.unwrap_or(0.0);
            let unit_txt = match self.unit.as_deref() { Some("hours") => "ч", Some("days") => "дн", _ => "мин" };
            return format!("каждые {} {}", amount, unit_txt);
        }
        self.kind.clone()
    }

    pub fn allowed(&self, at: NaiveDateTime) -> bool {
        let (date, time) = (at.date(), at.time());
        let day_allowed = |d: NaiveDate| -> bool {
            match &self.days {
                Some(days) if !days.is_empty() => days.contains(&i64::from(d.weekday().number_from_monday())),
                _ => true,
            }
        };
        let window = match (&self.window_from, &self.window_to) {
            (Some(f), Some(t)) => {
                let from = parse_time(f);
                let to = parse_time(t);
                match (from, to) {
                    (Some(f), Some(t)) if f != t => Some((f, t)),
                    _ => None,
                }
            }
            _ => None,
        };
        match window {
            None => day_allowed(date),
            Some((from, to)) if from < to => time >= from && time < to && day_allowed(date),
            Some((from, to)) => {
                (time >= from && day_allowed(date))
                    || (time < to && date.pred_opt().is_some_and(|before| day_allowed(before)))
            }
        }
    }
}

fn days_in_month(date: NaiveDate) -> u32 {
    let (y, m) = if date.month() == 12 { (date.year() + 1, 1) } else { (date.year(), date.month() + 1) };
    NaiveDate::from_ymd_opt(y, m, 1).and_then(|first| first.pred_opt()).map_or(28, |last| last.day())
}

fn cron_next_ms(s: &Schedule, now_ms: i64) -> Option<i64> {
    let expr = s.cron.as_deref().unwrap_or("").trim();
    if expr.is_empty() { return None; }
    let Ok(cron) = parse_cron(expr) else { return None; };
    let tz: Option<chrono_tz::Tz> = s.tz.as_deref().unwrap_or("").trim().parse().ok();
    let to_local = |ms: i64| -> NaiveDateTime {
        match (tz, chrono::DateTime::from_timestamp_millis(ms)) {
            (Some(t), Some(d)) => d.with_timezone(&t).naive_local(),
            _ => local(ms),
        }
    };
    let to_utc_ms = |naive: NaiveDateTime| -> Option<i64> {
        match tz {
            Some(t) => match t.from_local_datetime(&naive) {
                LocalResult::Single(dt) => Some(dt.timestamp_millis()),
                LocalResult::Ambiguous(first, _) => Some(first.timestamp_millis()),
                LocalResult::None => None,
            },
            None => Some(to_ms(naive)),
        }
    };
    let start = to_local(now_ms).with_second(0).and_then(|t| t.with_nanosecond(0)).unwrap_or_else(|| to_local(now_ms));
    let mut cur = start + Duration::minutes(1);
    let end = start + Duration::days(CRON_HORIZON_DAYS);
    while cur <= end {
        if !cron_day_matches(&cron, cur.date()) {
            cur = cur.date().succ_opt().map_or(end + Duration::minutes(1), |d| d.and_time(NaiveTime::MIN));
        } else if !cron.hours[cur.hour() as usize] {
            cur = cur.with_minute(0).unwrap_or(cur) + Duration::hours(1);
        } else {
            if cron.minutes[cur.minute() as usize] {
                if let Some(ms) = to_utc_ms(cur).filter(|ms| *ms > now_ms) {
                    return Some(ms);
                }
            }
            cur += Duration::minutes(1);
        }
    }
    None
}

/// Ближайший срок строго после `now_ms`. `None` — больше срока нет (разовый уже прошёл).
pub fn schedule_next(s: &Schedule, now_ms: i64) -> Option<i64> {
    schedule_next_with(s, now_ms, local, to_ms)
}

pub fn schedule_next_with(
    s: &Schedule,
    now_ms: i64,
    local_of: impl Fn(i64) -> NaiveDateTime,
    ms_of: impl Fn(NaiveDateTime) -> i64,
) -> Option<i64> {
    let now = local_of(now_ms);

    // cron
    if s.kind == "cron" {
        return cron_next_ms(s, now_ms);
    }

    // interval
    if s.kind == "interval" {
        if let Some(low) = s.every_minutes {
            let every = roll_minutes(low, s.every_max_minutes.unwrap_or(0), rand::random::<f64>());
            let target = now_ms + every * 60_000;
            let at = local_of(target);
            if s.allowed(at) {
                return Some(target);
            }
            // next allowed start
            let start = s.window_from.as_deref().and_then(parse_time).unwrap_or(NaiveTime::MIN);
            let mut date = at.date();
            for _ in 0..=8 {
                let candidate = date.and_time(start);
                if candidate > at && s.allowed(candidate) {
                    // ensure day allowed
                    return Some(ms_of(candidate));
                }
                date = date.succ_opt()?;
            }
            return Some(ms_of(at.date().succ_opt().unwrap_or(at.date()).and_time(start)));
        }
        // workflow interval amount/unit
        let mins = s.interval_minutes().unwrap_or(0.0).max(MIN_INTERVAL_MINUTES);
        return Some(now_ms + (mins * 60_000.0) as i64);
    }

    // hourly
    if s.kind == "hourly" {
        let minute = s.minute.unwrap_or(0).clamp(0, 59) as u32;
        let mut at = now.date().and_hms_opt(now.hour(), minute, 0)?;
        if at <= now {
            at += Duration::hours(1);
        }
        return Some(ms_of(at));
    }

    // recurrence (routine dates_ru)
    if let Some(rule) = &s.recurrence {
        if let Some(time_str) = &s.time {
            if let Some(at) = parse_time(time_str) {
                let start_on = s.start_on.clone().unwrap_or_default();
                let start_date = if start_on.trim().is_empty() {
                    None
                } else {
                    NaiveDate::parse_from_str(start_on.trim(), "%Y-%m-%d").ok()
                };
                let base = start_date.map_or(now.date(), |d| d.max(now.date()));
                let cand_date = if crate::dates_ru::is_occurrence(rule, base)
                    && (base > now.date() || at > now.time())
                {
                    base
                } else {
                    let prev = base.pred_opt().unwrap_or(base);
                    crate::dates_ru::next_occurrence_not_before(rule, prev, base)
                };
                let cand_date = if cand_date < base { base } else { cand_date };
                let cand = cand_date.and_time(at);
                if cand > now {
                    return Some(ms_of(cand));
                }
                // fallback next day
                return Some(ms_of((now.date().succ_opt().unwrap_or(now.date())).and_time(at)));
            }
        }
    }

    // once
    if s.kind == "once" {
        let time = s.time.as_deref().and_then(parse_time)?;
        let date = NaiveDate::parse_from_str(s.date.as_deref()?, "%Y-%m-%d").ok()?;
        let at = date.and_time(time);
        return (at > now).then(|| ms_of(at));
    }

    // at-time daily/weekdays/weekly/monthly with optional days
    if let Some(time_str) = &s.time {
        if let Some(at) = parse_time(time_str) {
            let days = s.days.clone().unwrap_or_default();
            let mut date = now.date();
            for _ in 0..=400 {
                let day_ok = match s.kind.as_str() {
                    "daily" => days.is_empty() || days.contains(&i64::from(date.weekday().number_from_monday())),
                    "weekdays" => date.weekday().number_from_monday() <= 5,
                    "weekly" => i64::from(date.weekday().number_from_monday()) == s.weekday.unwrap_or(1),
                    "monthly" => i64::from(date.day()) == s.day.unwrap_or(1).min(i64::from(days_in_month(date))),
                    _ => days.is_empty() || days.contains(&i64::from(date.weekday().number_from_monday())),
                };
                let cand = date.and_time(at);
                if day_ok && cand > now {
                    return Some(ms_of(cand));
                }
                match date.succ_opt() {
                    Some(d) => date = d,
                    None => break,
                }
            }
            return None;
        }
    }

    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(day: u32, hour: u32, minute: u32) -> NaiveDateTime {
        // Ноябрь 2026: 2-е — понедельник.
        NaiveDate::from_ymd_opt(2026, 11, day).unwrap().and_hms_opt(hour, minute, 0).unwrap()
    }

    fn routine(every: i64) -> Timing {
        Timing { every_minutes: every, ..Timing::default() }
    }

    fn cron_at(routine: &Timing, now_ms: i64) -> i64 {
        cron_next_ms(&routine.schedule(), now_ms).unwrap_or(i64::MAX)
    }

    #[test]
    fn a_random_interval_stays_inside_its_range_and_reaches_both_ends() {
        let r = Timing { every_minutes: 40, every_max_minutes: 80, ..Timing::default() };
        assert_eq!(roll_minutes(40, 80, 0.0), 40);
        assert_eq!(roll_minutes(40, 80, 0.999_999_9), 80);
        assert_eq!(roll_minutes(40, 80, 0.5), 60);
        for _ in 0..200 {
            assert!((40..=80).contains(&roll_minutes(40, 80, rand::random::<f64>())));
        }
        // Верх не больше низа — интервал ровный.
        assert_eq!(roll_minutes(50, 30, 0.9), 50);
        assert_eq!(roll_minutes(50, 0, 0.9), 50);
        assert_eq!(describe(&r), "каждые 40–80 мин, случайно");
    }

    #[test]
    fn plain_interval_just_adds_minutes() {
        assert_eq!(next_after(&routine(30), at(2, 10, 0)), at(2, 10, 30));
    }

    #[test]
    fn daily_time_today_or_tomorrow() {
        let r = Timing { at: "09:00".into(), ..routine(60) };
        assert_eq!(next_after(&r, at(2, 8, 0)), at(2, 9, 0));
        assert_eq!(next_after(&r, at(2, 9, 0)), at(3, 9, 0), "ровно в срок значит уже следующий день");
    }

    #[test]
    fn weekdays_skip_the_weekend() {
        let r = Timing { at: "09:00".into(), days: vec![1, 2, 3, 4, 5], ..routine(60) };
        assert_eq!(next_after(&r, at(6, 10, 0)), at(9, 9, 0), "после пятницы идёт понедельник");
    }

    #[test]
    fn night_window_crosses_midnight() {
        let r = Timing { window_from: "22:00".into(), window_to: "07:00".into(), ..routine(60) };
        assert!(allowed(&r, at(2, 23, 0)));
        assert!(allowed(&r, at(3, 3, 0)));
        assert!(!allowed(&r, at(3, 7, 0)));
        assert!(!allowed(&r, at(3, 12, 0)));
        assert_eq!(next_after(&r, at(3, 6, 30)), at(3, 22, 0), "конец окна: ждём следующую ночь");
        assert_eq!(next_after(&r, at(3, 12, 0)), at(3, 22, 0));
        assert_eq!(next_after(&r, at(2, 23, 0)), at(3, 0, 0));
    }

    #[test]
    fn night_belongs_to_the_day_it_started() {
        // Только ночь с пятницы на субботу.
        let r = Timing { window_from: "22:00".into(), window_to: "07:00".into(), days: vec![5], ..routine(60) };
        assert!(allowed(&r, at(6, 23, 0)));
        assert!(allowed(&r, at(7, 3, 0)), "утро субботы ещё пятничная ночь");
        assert!(!allowed(&r, at(7, 23, 0)));
        assert_eq!(next_after(&r, at(7, 6, 30)), at(13, 22, 0));
    }

    #[test]
    fn day_window_stays_inside() {
        let r = Timing { window_from: "09:00".into(), window_to: "18:00".into(), ..routine(60) };
        assert_eq!(next_after(&r, at(2, 17, 30)), at(3, 9, 0));
        assert_eq!(next_after(&r, at(2, 7, 0)), at(2, 9, 0));
    }

    #[test]
    fn catch_up_takes_only_last_or_skip() {
        assert!(catch_up_valid(&serde_json::json!({})));
        assert!(catch_up_valid(&serde_json::json!({ "catchUp": "skip" })));
        assert!(!catch_up_valid(&serde_json::json!({ "catchUp": "all" })));
        assert!(!catch_up_valid(&serde_json::json!({ "catchUp": 1 })));
    }

    #[test]
    fn time_format_is_strict() {
        assert!(parse_time("09:30").is_some());
        for bad in ["9:30", "24:00", "09:60", "0930", "", "ab:cd"] {
            assert!(parse_time(bad).is_none(), "{bad}");
        }
    }

    /// Пояс, где в 03:00 часы возвращаются на 02:00 (как осенью в Европе): местный час 02:00–02:59 бывает дважды.
    fn fall_back_zone() -> (impl Fn(i64) -> NaiveDateTime, impl Fn(NaiveDateTime) -> i64, i64) {
        let switch = 1_000_000_000_000_i64;
        let utc = |ms: i64| chrono::DateTime::from_timestamp_millis(ms).unwrap().naive_utc();
        let local_of = move |ms: i64| utc(ms) + Duration::hours(if ms < switch { 3 } else { 2 });
        // Неоднозначное местное время берётся по первому разу, как у `earliest()`.
        let ms_of = move |at: NaiveDateTime| {
            let first = at.and_utc().timestamp_millis() - 3 * 3_600_000;
            if first < switch { first } else { at.and_utc().timestamp_millis() - 2 * 3_600_000 }
        };
        (local_of, ms_of, switch)
    }

    #[test]
    fn a_short_interval_does_not_land_in_the_past_when_the_clocks_go_back() {
        let (local_of, ms_of, switch) = fall_back_zone();
        let r = routine(5);
        // Через десять минут после перевода: местное время 02:10 второго прохода.
        let now_ms = switch + 10 * 60_000;
        // Старый расчёт: местные 02:15 понимаются как первый проход, то есть час назад.
        let naive = ms_of(local_of(now_ms) + Duration::minutes(5));
        assert!(naive < now_ms, "демонстрация: наивный расчёт даёт прошлое");
        assert_eq!(schedule_next_with(&r.schedule(), now_ms, &local_of, &ms_of).unwrap(), now_ms + 5 * 60_000);
        // И в самом конце «первого» часа тоже.
        let before = switch - 2 * 60_000;
        assert_eq!(schedule_next_with(&r.schedule(), before, &local_of, &ms_of).unwrap(), before + 5 * 60_000);
    }

    #[test]
    fn moments_and_schedules_read_in_words() {
        assert_eq!(describe_moment(to_ms(at(9, 9, 0))), "пн 09.11 в 09:00");
        assert_eq!(describe_moment(to_ms(at(13, 18, 5))), "пт 13.11 в 18:05");
        let daily = Timing { at: "09:00".into(), ..routine(60) };
        assert_eq!(describe(&daily), "каждый день в 09:00");
        let weekdays = Timing { days: vec![1, 2, 3, 4, 5], ..daily.clone() };
        assert_eq!(describe(&weekdays), "по будням в 09:00");
        let monday = Timing { days: vec![1], ..daily };
        assert_eq!(describe(&monday), "по понедельникам в 09:00");
        assert_eq!(describe(&routine(60)), "каждый час");
        assert_eq!(describe(&routine(30)), "каждые 30 мин");
        assert_eq!(describe(&routine(180)), "каждые 3 ч");
        let night = Timing { window_from: "22:00".into(), window_to: "07:00".into(), ..routine(60) };
        assert_eq!(describe(&night), "каждый час, с 22:00 до 07:00");
        assert!(describe_next(false, false, 0).contains("выключена"));
        assert!(describe_next(true, true, 0).contains("вебхуком"));
    }

    fn ics_now() -> i64 {
        Local.with_ymd_and_hms(2026, 11, 2, 8, 0, 0).single().unwrap().timestamp_millis()
    }

    fn ics_routine(title: &str) -> IcsItem {
        IcsItem { id: "rt_1".into(), title: title.into(), bot: "Бот".into(), prompt: "Собери темы дня".into(), timing: Timing { at: "09:00".into(), ..Timing::default() } }
    }

    #[test]
    fn tasks_ics_ends_lines_with_crlf_and_folds_a_long_cyrillic_line_by_octets() {
        let title = "Ж".repeat(80);
        let out = tasks_ics(&[ics_routine(&title)], ics_now());
        assert!(out.ends_with("\r\n"), "календарь не кончается CRLF");
        assert!(out.contains("\r\n "), "длинная строка не сложена");
        for line in out.split("\r\n") {
            assert!(line.len() <= 75, "строка {} октетов: {line:?}", line.len());
        }
        assert!(!out.replace("\r\n", "").contains('\n'), "в календаре голый LF");
        let unfolded = out.replace("\r\n ", "");
        assert!(unfolded.contains(&format!("SUMMARY:Бот: {title}")), "сводка потерялась при складывании");
    }

    #[test]
    fn tasks_ics_escapes_backslash_semicolon_comma_and_newline() {
        let routine = IcsItem {
            title: "Сводка; день, ночь\\тест".into(),
            prompt: "Первая\nВторая".into(),
            ..ics_routine("")
        };
        let out = tasks_ics(&[routine], ics_now());
        assert!(out.contains("SUMMARY:Бот: Сводка\\; день\\, ночь\\\\тест\r\n"), "{out}");
        assert!(out.contains("DESCRIPTION:Первая\\nВторая\r\n"), "{out}");
    }

    #[test]
    fn tasks_ics_daily_at_uses_days_as_weekly_byday() {
        let routine = IcsItem { timing: Timing { days: vec![5, 1, 3, 3], ..ics_routine("").timing }, ..ics_routine("Сводка") };
        let out = tasks_ics(&[routine], ics_now());
        assert!(out.contains("UID:rt_1@deloist\r\n"), "{out}");
        assert!(out.contains("DTSTART:20261102T090000\r\n"), "{out}");
        assert!(out.contains("DURATION:PT15M\r\n"), "{out}");
        assert!(out.contains("RRULE:FREQ=WEEKLY;BYDAY=MO,WE,FR\r\n"), "{out}");
    }

    #[test]
    fn tasks_ics_daily_at_without_days_repeats_daily() {
        let out = tasks_ics(&[ics_routine("Сводка")], ics_now());
        assert!(out.contains("RRULE:FREQ=DAILY\r\n"), "{out}");
    }

    #[test]
    fn tasks_ics_interval_without_a_window_is_minutely() {
        let routine = IcsItem { timing: Timing { at: String::new(), every_minutes: 30, ..Timing::default() }, ..ics_routine("Проверка") };
        let out = tasks_ics(&[routine], ics_now());
        assert!(out.contains("DTSTART:20261102T080000\r\n"), "{out}");
        assert!(out.contains("RRULE:FREQ=MINUTELY;INTERVAL=30\r\n"), "{out}");
    }

    #[test]
    fn tasks_ics_interval_with_a_window_is_a_daily_block() {
        let timing = Timing { at: String::new(), every_minutes: 30, window_from: "22:00".into(), window_to: "07:00".into(), ..Timing::default() };
        let out = tasks_ics(&[IcsItem { timing, ..ics_routine("Ночная") }], ics_now());
        assert!(out.contains("DTSTART:20261102T220000\r\n"), "{out}");
        assert!(out.contains("DURATION:PT9H\r\n"), "{out}");
        assert!(out.contains("RRULE:FREQ=DAILY\r\n"), "{out}");
        assert!(!out.contains("FREQ=MINUTELY"), "{out}");
        assert!(out.contains("(каждые 30 мин)"), "{out}");
    }

    #[test]
    fn tasks_ics_skips_cron_which_a_calendar_cannot_hold() {
        let normal = IcsItem { id: "rt_normal".into(), ..ics_routine("Обычная") };
        let cron = IcsItem { id: "rt_cron".into(), timing: Timing { cron: "0 9 * * 1".into(), ..Timing::default() }, ..ics_routine("Cron") };
        let out = tasks_ics(&[normal, cron], ics_now());
        assert!(out.contains("UID:rt_normal@deloist"), "{out}");
        assert!(!out.contains("rt_cron"), "{out}");
        assert_eq!(out.matches("BEGIN:VEVENT").count(), 1, "{out}");
    }

    #[test]
    fn failures_slow_a_routine_down() {
        assert_eq!([0, 2, 3, 5, 6, 40].map(failure_gap_ms), [0, 0, 3_600_000, 3_600_000, 4 * 3_600_000, 4 * 3_600_000]);
    }

    fn moment(y: i32, m: u32, day: u32, hour: u32, minute: u32) -> NaiveDateTime {
        NaiveDate::from_ymd_opt(y, m, day).unwrap().and_hms_opt(hour, minute, 0).unwrap()
    }

    fn weekly_wednesday() -> crate::dates_ru::Recurrence {
        crate::dates_ru::Recurrence {
            freq: crate::dates_ru::Freq::Weekly,
            interval: 1,
            weekdays: vec![3],
            monthday: None,
            nth: None,
            month: None,
        }
    }

    fn cron_routine(cron: &str, tz: &str) -> Timing {
        Timing { cron: cron.into(), tz: tz.into(), ..Timing::default() }
    }

    fn utc_ms(y: i32, mo: u32, d: u32, h: u32, mi: u32) -> i64 {
        NaiveDate::from_ymd_opt(y, mo, d).unwrap().and_hms_opt(h, mi, 0).unwrap().and_utc().timestamp_millis()
    }

    fn naive(y: i32, mo: u32, d: u32, h: u32, mi: u32) -> NaiveDateTime {
        NaiveDate::from_ymd_opt(y, mo, d).unwrap().and_hms_opt(h, mi, 0).unwrap()
    }

    fn set(v: &[bool]) -> Vec<usize> {
        v.iter().enumerate().filter(|(_, on)| **on).map(|(i, _)| i).collect()
    }

    #[test]
    fn cron_parser_reads_steps_lists_ranges_and_sunday() {
        assert_eq!(set(&parse_cron("*/15 * * * *").unwrap().minutes), vec![0, 15, 30, 45]);
        assert_eq!(set(&parse_cron("1-30/5 * * * *").unwrap().minutes), vec![1, 6, 11, 16, 21, 26]);
        assert_eq!(set(&parse_cron("0,10,20 8-10 * * *").unwrap().minutes), vec![0, 10, 20]);
        assert_eq!(set(&parse_cron("0 8-10 * * *").unwrap().hours), vec![8, 9, 10]);
        assert_eq!(set(&parse_cron("0 0 * * 0").unwrap().dow), vec![0]);
        assert_eq!(set(&parse_cron("0 0 * * 7").unwrap().dow), vec![0]);
        assert_eq!(set(&parse_cron("0 0 * * 1-5").unwrap().dow), vec![1, 2, 3, 4, 5]);
        for bad in ["61 * * * *", "* * *", "a b c d e", "* 24 * * *", "* * 0 * *", "* * * 13 *", "*/0 * * * *", "5-1 * * * *"] {
            assert!(parse_cron(bad).is_err(), "{bad} должно отклоняться");
        }
    }

    #[test]
    fn start_on_holds_the_first_run_of_a_weekly_rule() {
        let r = Timing {
            at: "10:00".into(),
            start_on: "2026-12-02".into(),
            recurrence: Some(weekly_wednesday()),
            ..routine(60)
        };
        // 18 ноября — среда: без startOn срок был бы 25 ноября.
        assert_eq!(next_after(&r, moment(2026, 11, 18, 8, 0)), moment(2026, 12, 2, 10, 0));
    }

    #[test]
    fn a_past_start_on_does_not_hold_the_first_run() {
        let r = Timing {
            at: "10:00".into(),
            start_on: "2026-01-01".into(),
            recurrence: Some(weekly_wednesday()),
            ..routine(60)
        };
        assert_eq!(next_after(&r, moment(2026, 11, 18, 8, 0)), moment(2026, 11, 18, 10, 0));
    }

    #[test]
    fn once_at_is_the_moment_itself() {
        let moment_ms = to_ms(moment(2026, 12, 2, 10, 0));
        let r = Timing { once_at: moment_ms, ..routine(60) };
        assert_eq!(next_run_ms(&r, to_ms(moment(2026, 11, 18, 8, 0))), moment_ms);
    }

    #[test]
    fn cron_day_of_month_or_day_of_week_when_both_are_set() {
        let c = parse_cron("0 9 13 * 5").unwrap();
        // 13 ноября 2026 — пятница; 20-е — тоже пятница; 13 декабря — воскресенье.
        assert!(cron_day_matches(&c, naive(2026, 11, 20, 9, 0).date()), "пятница");
        assert!(cron_day_matches(&c, naive(2026, 12, 13, 9, 0).date()), "13-е");
        assert!(!cron_day_matches(&c, naive(2026, 11, 18, 9, 0).date()), "среда, не 13-е");
        let next = cron_at(&cron_routine("0 9 13 * 5", "UTC"), utc_ms(2026, 12, 8, 0, 0));
        assert_eq!(next, utc_ms(2026, 12, 11, 9, 0), "ближайшая пятница раньше 13-го");
    }

    #[test]
    fn cron_monthly_and_leap_day_do_not_fall_back_to_every_minute() {
        let now = to_ms(naive(2026, 11, 18, 12, 0));
        assert_eq!(cron_at(&cron_routine("30 10 1 * *", ""), now), to_ms(naive(2026, 12, 1, 10, 30)));
        assert_eq!(cron_at(&cron_routine("0 0 29 2 *", ""), now), to_ms(naive(2028, 2, 29, 0, 0)));
    }

    #[test]
    fn cron_without_a_day_in_eight_years_is_rejected() {
        assert!(parse_cron("0 0 31 2 *").is_err());
        assert!(parse_cron("0 0 29 2 *").is_ok());
        assert_eq!(cron_at(&cron_routine("not a cron", ""), 1), i64::MAX);
    }

    #[test]
    fn cron_skips_the_hour_that_does_not_exist_and_runs_a_doubled_one_once() {
        let berlin = cron_routine("30 2 * * *", "Europe/Berlin");
        // 28.03.2027 02:30 в Берлине не бывает: переходим на следующий день, 02:30 CEST = 00:30 UTC.
        assert_eq!(cron_at(&berlin, utc_ms(2027, 3, 28, 0, 0)), utc_ms(2027, 3, 29, 0, 30));
        // 25.10.2026 02:30 бывает дважды: берём раннее (CEST, 00:30 UTC), а после него — уже следующий день.
        let first = cron_at(&berlin, utc_ms(2026, 10, 24, 22, 0));
        assert_eq!(first, utc_ms(2026, 10, 25, 0, 30));
        assert_eq!(cron_at(&berlin, first), utc_ms(2026, 10, 26, 1, 30));
    }

    #[test]
    fn cron_uses_the_zone_of_the_routine() {
        let ny = cron_routine("0 9 * * *", "America/New_York");
        assert_eq!(cron_at(&ny, utc_ms(2026, 11, 18, 12, 0)), utc_ms(2026, 11, 18, 14, 0));
        assert!(tz_valid("Europe/Berlin") && tz_valid(""));
        assert!(!tz_valid("Mars/Base"));
    }

    #[test]
    fn cron_is_described_in_words_with_the_zone() {
        assert_eq!(describe(&cron_routine("*/15 * * * *", "")), "каждые 15 мин");
        assert_eq!(describe(&cron_routine("0 9 * * 1-5", "")), "по будням в 09:00");
        assert_eq!(describe(&cron_routine("30 10 1 * *", "")), "1-го числа в 10:30");
        assert_eq!(describe(&cron_routine("0 9 * * 1-5", "Europe/Berlin")), "по будням в 09:00 (Europe/Berlin)");
        assert_eq!(describe(&cron_routine("0 9 13 * 5", "")), "по cron: 0 9 13 * 5");
    }

    // ---- портированные тесты расписания процесса ----

    #[test]
    fn schedule_rules() {
        let daily = Schedule { kind: "daily".into(), time: Some("09:00".into()), ..Schedule::default() };
        assert!(daily.validate().is_ok());
        let fast = Schedule { kind: "interval".into(), amount: Some(2.0), unit: Some("minutes".into()), ..Schedule::default() };
        assert!(fast.validate().is_err(), "интервал меньше трёх минут");
        let okay = Schedule { amount: Some(3.0), ..fast };
        assert!(okay.validate().is_ok());
        let weekly = Schedule { kind: "weekly".into(), time: Some("09:00".into()), weekday: Some(8), ..Schedule::default() };
        assert!(weekly.validate().is_err());
    }

    fn at_ymd(y: i32, m: u32, d: u32, h: u32, mi: u32) -> NaiveDateTime {
        NaiveDate::from_ymd_opt(y, m, d).unwrap().and_hms_opt(h, mi, 0).unwrap()
    }

    fn next(s: &Schedule, now: NaiveDateTime) -> Option<NaiveDateTime> {
        let epoch = NaiveDate::from_ymd_opt(1970, 1, 1).unwrap().and_hms_opt(0, 0, 0).unwrap();
        let now_ms = (now - epoch).num_milliseconds();
        schedule_next_with(s, now_ms, |ms| epoch + Duration::milliseconds(ms), |t| (t - epoch).num_milliseconds()).map(|ms| epoch + Duration::milliseconds(ms))
    }

    #[test]
    fn next_slots() {
        // 2026-11-16 — понедельник.
        let time = Some("09:00".to_string());
        let daily = Schedule { kind: "daily".into(), time: time.clone(), ..Schedule::default() };
        assert_eq!(next(&daily, at_ymd(2026, 11, 16, 8, 0)), Some(at_ymd(2026, 11, 16, 9, 0)));
        assert_eq!(next(&daily, at_ymd(2026, 11, 16, 9, 0)), Some(at_ymd(2026, 11, 17, 9, 0)));
        let weekdays = Schedule { kind: "weekdays".into(), time: time.clone(), ..Schedule::default() };
        assert_eq!(next(&weekdays, at_ymd(2026, 11, 20, 10, 0)), Some(at_ymd(2026, 11, 23, 9, 0)), "пятница вечером — понедельник");
        let weekly = Schedule { kind: "weekly".into(), time: time.clone(), weekday: Some(3), ..Schedule::default() };
        assert_eq!(next(&weekly, at_ymd(2026, 11, 16, 12, 0)), Some(at_ymd(2026, 11, 18, 9, 0)));
        let monthly = Schedule { kind: "monthly".into(), time: time.clone(), day: Some(31), ..Schedule::default() };
        assert_eq!(next(&monthly, at_ymd(2026, 11, 16, 12, 0)), Some(at_ymd(2026, 11, 30, 9, 0)), "в ноябре 31-го нет: последний день");
        let hourly = Schedule { kind: "hourly".into(), minute: Some(15), ..Schedule::default() };
        assert_eq!(next(&hourly, at_ymd(2026, 11, 16, 8, 20)), Some(at_ymd(2026, 11, 16, 9, 15)));
        let once = Schedule { kind: "once".into(), time, date: Some("2026-11-20".into()), ..Schedule::default() };
        assert_eq!(next(&once, at_ymd(2026, 11, 16, 8, 0)), Some(at_ymd(2026, 11, 20, 9, 0)));
        assert_eq!(next(&once, at_ymd(2026, 11, 21, 8, 0)), None);
    }

    #[test]
    fn routine_schedule_gives_same_next_run_for_each_kind() {
        let now_ms = to_ms(at(2, 8, 0));
        // time + days
        let r = Timing { at: "09:00".into(), days: vec![1, 3, 5], ..routine(60) };
        assert_eq!(next_run_ms(&r, now_ms), schedule_next(&r.schedule(), now_ms).unwrap());
        // window
        let r = Timing { window_from: "22:00".into(), window_to: "07:00".into(), ..routine(60) };
        assert_eq!(next_run_ms(&r, now_ms), schedule_next(&r.schedule(), now_ms).unwrap());
        // random interval bounds
        let r = Timing { every_minutes: 40, every_max_minutes: 80, ..Timing::default() };
        let v = roll_minutes(r.every_minutes, r.every_max_minutes, 0.5);
        assert!((40..=80).contains(&v));
        assert_eq!(describe(&r), describe(&r.schedule()));
        // cron
        let r = cron_routine("0 9 * * 1-5", "Europe/Berlin");
        assert_eq!(next_run_ms(&r, utc_ms(2026, 11, 18, 12, 0)), schedule_next(&r.schedule(), utc_ms(2026, 11, 18, 12, 0)).unwrap());
        // tz describe
        assert!(describe(&r).contains("Europe/Berlin"));
        // allowed parity
        let r = Timing { window_from: "09:00".into(), window_to: "18:00".into(), ..routine(60) };
        assert_eq!(allowed(&r, at(2, 12, 0)), r.schedule().allowed(at(2, 12, 0)));
    }
}
