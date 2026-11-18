//! Когда рутине пора: «каждые N минут» с окном и днями недели или «в 09:00 по будням».
//! Всё считается в местном времени компьютера: человек думает о ночи, а не о UTC.

use chrono::{Datelike, Duration, Local, NaiveDate, NaiveDateTime, NaiveTime, TimeZone, Timelike, Utc};

use crate::store::{Bot, Routine};

/// Последний пропущенный запуск догоняем, если компьютер проспал меньше этого: сводка в 10 ещё нужна, через день нет.
pub const CATCH_UP_MS: i64 = 12 * 3_600_000;

/// Запуски, которые должны были начаться, пока компьютер был выключен или спал.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Missed {
    pub count: u32,
    pub last_slot_ms: Option<i64>,
}

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
pub fn open_now(routine: &Routine, now_ms: i64) -> bool {
    allowed(routine, local(now_ms))
}

fn day_allowed(routine: &Routine, date: NaiveDate) -> bool {
    routine.days.is_empty() || routine.days.contains(&i64::from(date.weekday().number_from_monday()))
}

fn window(routine: &Routine) -> Option<(NaiveTime, NaiveTime)> {
    let from = parse_time(&routine.window_from)?;
    let to = parse_time(&routine.window_to)?;
    (from != to).then_some((from, to))
}

/// Можно ли работать в этот момент. Ночное окно относится к дню, в который началось.
pub fn allowed(routine: &Routine, at: NaiveDateTime) -> bool {
    let (date, time) = (at.date(), at.time());
    match window(routine) {
        None => day_allowed(routine, date),
        Some((from, to)) if from < to => time >= from && time < to && day_allowed(routine, date),
        Some((from, to)) => {
            (time >= from && day_allowed(routine, date))
                || (time < to && date.pred_opt().is_some_and(|before| day_allowed(routine, before)))
        }
    }
}

fn next_allowed_start(routine: &Routine, after: NaiveDateTime) -> NaiveDateTime {
    let start = window(routine).map_or(NaiveTime::MIN, |(from, _)| from);
    let mut date = after.date();
    for _ in 0..=8 {
        let candidate = date.and_time(start);
        if candidate > after && day_allowed(routine, date) {
            return candidate;
        }
        date = date.succ_opt().unwrap_or(date);
    }
    after + Duration::days(1)
}

pub fn next_after(routine: &Routine, now: NaiveDateTime) -> NaiveDateTime {
    if let Some(at) = parse_time(&routine.at) {
        let mut date = now.date();
        for _ in 0..=8 {
            let candidate = date.and_time(at);
            if candidate > now && day_allowed(routine, date) {
                return candidate;
            }
            date = date.succ_opt().unwrap_or(date);
        }
        return now + Duration::days(1);
    }
    let candidate = now + Duration::minutes(routine.every_minutes.max(1));
    if allowed(routine, candidate) {
        candidate
    } else {
        next_allowed_start(routine, candidate)
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

pub fn next_run_ms(routine: &Routine, now_ms: i64) -> i64 {
    next_run_with(routine, now_ms, local, to_ms)
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

/// Расчёт срока с подставляемыми часами пояса: так его можно проверить на переводе часов в любой стране.
/// Интервал считается в настоящем времени (мс), а в местные часы переводится только для окна и дней:
/// иначе после перевода часов назад срок «через 5 минут» мог оказаться в прошлом и тикер крутился бы час.
fn next_run_with(
    routine: &Routine,
    now_ms: i64,
    local_of: impl Fn(i64) -> NaiveDateTime,
    ms_of: impl Fn(NaiveDateTime) -> i64,
) -> i64 {
    let next = if parse_time(&routine.at).is_some() {
        ms_of(next_after(routine, local_of(now_ms)))
    } else {
        let target = now_ms + routine.every_minutes.max(1) * 60_000;
        let at = local_of(target);
        if allowed(routine, at) {
            target
        } else {
            ms_of(next_allowed_start(routine, at))
        }
    };
    // Срок всегда впереди: неоднозначный местный час при переводе часов не должен вернуть прошлое.
    if next > now_ms { next } else { now_ms + routine.every_minutes.clamp(1, 60) * 60_000 }
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
pub fn describe_next(routine: &Routine) -> String {
    if routine.hook {
        "Запускается только вебхуком, по расписанию не идёт.".to_string()
    } else if !routine.enabled {
        "Рутина выключена, запусков не будет.".to_string()
    } else {
        format!("Следующий запуск: {}.", describe_moment(routine.next_run))
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

/// Расписание словами: «по будням в 09:00», «каждый час», «каждые 30 мин с 22:00 до 07:00».
pub fn describe(routine: &Routine) -> String {
    if routine.hook {
        return "по вебхуку".to_string();
    }
    if parse_time(&routine.at).is_some() {
        return format!("{} в {}", days_text(&routine.days), routine.at.trim());
    }
    let every = match routine.every_minutes {
        60 => "каждый час".to_string(),
        m if m > 60 && m % 60 == 0 => format!("каждые {} ч", m / 60),
        m => format!("каждые {m} мин"),
    };
    let mut text = every;
    if !routine.days.is_empty() {
        text = format!("{text}, {}", days_text(&routine.days));
    }
    if window(routine).is_some() {
        text = format!("{text}, с {} до {}", routine.window_from.trim(), routine.window_to.trim());
    }
    text
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

/// Календарь рутин: по одному событию на включённую рутину, вебхуки пропускаем.
/// Время местное и без TZID: календарь человека сам покажет его в своих часах.
pub fn routines_ics(routines: &[Routine], bots: &[Bot], now: i64) -> String {
    let mut lines = vec![
        "BEGIN:VCALENDAR".to_string(),
        "VERSION:2.0".to_string(),
        "PRODID:-//Deloist//Bots//RU".to_string(),
        "CALSCALE:GREGORIAN".to_string(),
        "METHOD:PUBLISH".to_string(),
    ];
    let stamp = format!("DTSTAMP:{}", ics_stamp(now));
    let today = local(now).date();
    for routine in routines.iter().filter(|r| r.enabled && !r.hook) {
        let bot = bots.iter().find(|b| b.id == routine.bot_id).map_or("", |b| b.name.as_str());
        let mut title = if bot.is_empty() {
            routine.title.clone()
        } else {
            format!("{bot}: {}", routine.title)
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
        lines.push(format!("UID:{}@deloist", routine.id));
        lines.push(stamp.clone());
        lines.push(format!("DTSTART:{}", start.format("%Y%m%dT%H%M%S")));
        lines.push(format!("DURATION:{}", ics_duration(duration)));
        lines.push(format!("RRULE:{rule}"));
        lines.push(format!("SUMMARY:{}", ics_escape(&title)));
        let prompt: String = routine.prompt.chars().take(300).collect();
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

/// Слоты расписания, которые должны были начаться между `routine.next_run` и `now`, но не начались.
/// Считаются только полностью прошедшие интервалы (чтобы обычный 30-секундный тик не считался пропуском).
/// Считаются только последние семь дней: долго выключенный ПК не делает расчёт дорогим, а последний пропуск — настоящий.
pub fn missed(routine: &Routine, now_ms: i64) -> Missed {
    const WEEK_MS: i64 = 7 * 24 * 3_600_000;
    if routine.next_run <= 0 {
        return Missed { count: 0, last_slot_ms: None };
    }
    let interval_ms = routine.every_minutes.max(1) * 60_000;
    let mut slot = if routine.next_run >= now_ms - WEEK_MS { routine.next_run } else { to_ms(next_after(routine, local(now_ms - WEEK_MS))) };
    let (mut count, mut last) = (0, None);
    while slot + interval_ms <= now_ms {
        count += 1;
        last = Some(slot);
        let next = to_ms(next_after(routine, local(slot)));
        if next <= slot {
            break;
        }
        slot = next;
    }
    Missed { count, last_slot_ms: last }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(day: u32, hour: u32, minute: u32) -> NaiveDateTime {
        // Ноябрь 2026: 2-е — понедельник.
        NaiveDate::from_ymd_opt(2026, 11, day).unwrap().and_hms_opt(hour, minute, 0).unwrap()
    }

    fn routine(every: i64) -> Routine {
        Routine { every_minutes: every, ..Routine::default() }
    }

    #[test]
    fn plain_interval_just_adds_minutes() {
        assert_eq!(next_after(&routine(30), at(2, 10, 0)), at(2, 10, 30));
    }

    #[test]
    fn daily_time_today_or_tomorrow() {
        let r = Routine { at: "09:00".into(), ..routine(60) };
        assert_eq!(next_after(&r, at(2, 8, 0)), at(2, 9, 0));
        assert_eq!(next_after(&r, at(2, 9, 0)), at(3, 9, 0), "ровно в срок значит уже следующий день");
    }

    #[test]
    fn weekdays_skip_the_weekend() {
        let r = Routine { at: "09:00".into(), days: vec![1, 2, 3, 4, 5], ..routine(60) };
        assert_eq!(next_after(&r, at(6, 10, 0)), at(9, 9, 0), "после пятницы идёт понедельник");
    }

    #[test]
    fn night_window_crosses_midnight() {
        let r = Routine { window_from: "22:00".into(), window_to: "07:00".into(), ..routine(60) };
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
        let r = Routine { window_from: "22:00".into(), window_to: "07:00".into(), days: vec![5], ..routine(60) };
        assert!(allowed(&r, at(6, 23, 0)));
        assert!(allowed(&r, at(7, 3, 0)), "утро субботы ещё пятничная ночь");
        assert!(!allowed(&r, at(7, 23, 0)));
        assert_eq!(next_after(&r, at(7, 6, 30)), at(13, 22, 0));
    }

    #[test]
    fn day_window_stays_inside() {
        let r = Routine { window_from: "09:00".into(), window_to: "18:00".into(), ..routine(60) };
        assert_eq!(next_after(&r, at(2, 17, 30)), at(3, 9, 0));
        assert_eq!(next_after(&r, at(2, 7, 0)), at(2, 9, 0));
    }

    #[test]
    fn a_daily_run_missed_for_three_days() {
        let r = Routine { at: "09:00".into(), next_run: to_ms(at(2, 9, 0)), ..routine(60) };
        let m = missed(&r, to_ms(at(5, 9, 0)));
        assert_eq!((m.count, m.last_slot_ms), (3, Some(to_ms(at(4, 9, 0)))));
    }

    #[test]
    fn a_daily_run_two_hours_late_is_one_missed() {
        let r = Routine { at: "09:00".into(), next_run: to_ms(at(3, 9, 0)), ..routine(60) };
        let m = missed(&r, to_ms(at(3, 11, 0)));
        assert_eq!((m.count, m.last_slot_ms), (1, Some(to_ms(at(3, 9, 0)))));
    }

    #[test]
    fn a_night_window_missed_whole() {
        let r = Routine { window_from: "22:00".into(), window_to: "07:00".into(), next_run: to_ms(at(2, 22, 0)), ..routine(60) };
        let m = missed(&r, to_ms(at(3, 7, 0)));
        assert_eq!((m.count, m.last_slot_ms), (9, Some(to_ms(at(3, 6, 0)))), "22:00…06:00");
    }

    #[test]
    fn a_normal_tick_is_not_a_miss() {
        let r = Routine { next_run: to_ms(at(3, 9, 0)), ..routine(60) };
        assert_eq!(missed(&r, to_ms(at(3, 9, 0)) + 40_000).count, 0);
    }

    #[test]
    fn missed_runs_skip_days_off() {
        let r = Routine { at: "09:00".into(), days: vec![1, 2, 3, 4, 5], next_run: to_ms(at(2, 9, 0)), ..routine(60) };
        let m = missed(&r, to_ms(at(9, 9, 0)));
        assert_eq!((m.count, m.last_slot_ms), (5, Some(to_ms(at(6, 9, 0)))), "пн–пт, выходные не считаются");
    }

    #[test]
    fn after_two_weeks_off_the_last_miss_is_recent() {
        let r = Routine { at: "09:00".into(), next_run: to_ms(at(2, 9, 0)), ..routine(60) };
        let m = missed(&r, to_ms(at(20, 10, 30)));
        assert_eq!((m.count, m.last_slot_ms), (7, Some(to_ms(at(20, 9, 0)))), "считаем неделю до сейчас, а не неделю от старого срока");
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
        assert_eq!(next_run_with(&r, now_ms, &local_of, &ms_of), now_ms + 5 * 60_000);
        // И в самом конце «первого» часа тоже.
        let before = switch - 2 * 60_000;
        assert_eq!(next_run_with(&r, before, &local_of, &ms_of), before + 5 * 60_000);
    }

    #[test]
    fn moments_and_schedules_read_in_words() {
        assert_eq!(describe_moment(to_ms(at(9, 9, 0))), "пн 09.11 в 09:00");
        assert_eq!(describe_moment(to_ms(at(13, 18, 5))), "пт 13.11 в 18:05");
        let daily = Routine { at: "09:00".into(), ..routine(60) };
        assert_eq!(describe(&daily), "каждый день в 09:00");
        let weekdays = Routine { days: vec![1, 2, 3, 4, 5], ..daily.clone() };
        assert_eq!(describe(&weekdays), "по будням в 09:00");
        let monday = Routine { days: vec![1], ..daily };
        assert_eq!(describe(&monday), "по понедельникам в 09:00");
        assert_eq!(describe(&routine(60)), "каждый час");
        assert_eq!(describe(&routine(30)), "каждые 30 мин");
        assert_eq!(describe(&routine(180)), "каждые 3 ч");
        let night = Routine { window_from: "22:00".into(), window_to: "07:00".into(), ..routine(60) };
        assert_eq!(describe(&night), "каждый час, с 22:00 до 07:00");
        let off = Routine { enabled: false, ..routine(60) };
        assert!(describe_next(&off).contains("выключена"));
    }

    fn ics_now() -> i64 {
        Local.with_ymd_and_hms(2026, 11, 2, 8, 0, 0).single().unwrap().timestamp_millis()
    }

    fn ics_bot() -> Bot {
        Bot { id: "bot_1".into(), name: "Бот".into(), ..Bot::default() }
    }

    fn ics_routine(title: &str) -> Routine {
        Routine {
            id: "rt_1".into(),
            bot_id: "bot_1".into(),
            title: title.into(),
            prompt: "Собери темы дня".into(),
            at: "09:00".into(),
            ..Routine::default()
        }
    }

    #[test]
    fn routines_ics_ends_lines_with_crlf_and_folds_a_long_cyrillic_line_by_octets() {
        let title = "Ж".repeat(80);
        let out = routines_ics(&[ics_routine(&title)], &[ics_bot()], ics_now());
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
    fn routines_ics_escapes_backslash_semicolon_comma_and_newline() {
        let routine = Routine {
            title: "Сводка; день, ночь\\тест".into(),
            prompt: "Первая\nВторая".into(),
            ..ics_routine("")
        };
        let out = routines_ics(&[routine], &[ics_bot()], ics_now());
        assert!(out.contains("SUMMARY:Бот: Сводка\\; день\\, ночь\\\\тест\r\n"), "{out}");
        assert!(out.contains("DESCRIPTION:Первая\\nВторая\r\n"), "{out}");
    }

    #[test]
    fn routines_ics_daily_at_uses_days_as_weekly_byday() {
        let routine = Routine { days: vec![5, 1, 3, 3], ..ics_routine("Сводка") };
        let out = routines_ics(&[routine], &[ics_bot()], ics_now());
        assert!(out.contains("UID:rt_1@deloist\r\n"), "{out}");
        assert!(out.contains("DTSTART:20261102T090000\r\n"), "{out}");
        assert!(out.contains("DURATION:PT15M\r\n"), "{out}");
        assert!(out.contains("RRULE:FREQ=WEEKLY;BYDAY=MO,WE,FR\r\n"), "{out}");
    }

    #[test]
    fn routines_ics_daily_at_without_days_repeats_daily() {
        let out = routines_ics(&[ics_routine("Сводка")], &[ics_bot()], ics_now());
        assert!(out.contains("RRULE:FREQ=DAILY\r\n"), "{out}");
    }

    #[test]
    fn routines_ics_interval_without_a_window_is_minutely() {
        let routine = Routine { at: String::new(), every_minutes: 30, ..ics_routine("Проверка") };
        let out = routines_ics(&[routine], &[ics_bot()], ics_now());
        assert!(out.contains("DTSTART:20261102T080000\r\n"), "{out}");
        assert!(out.contains("RRULE:FREQ=MINUTELY;INTERVAL=30\r\n"), "{out}");
    }

    #[test]
    fn routines_ics_interval_with_a_window_is_a_daily_block() {
        let routine = Routine {
            at: String::new(),
            every_minutes: 30,
            window_from: "22:00".into(),
            window_to: "07:00".into(),
            ..ics_routine("Ночная")
        };
        let out = routines_ics(&[routine], &[ics_bot()], ics_now());
        assert!(out.contains("DTSTART:20261102T220000\r\n"), "{out}");
        assert!(out.contains("DURATION:PT9H\r\n"), "{out}");
        assert!(out.contains("RRULE:FREQ=DAILY\r\n"), "{out}");
        assert!(!out.contains("FREQ=MINUTELY"), "{out}");
        assert!(out.contains("(каждые 30 мин)"), "{out}");
    }

    #[test]
    fn routines_ics_skips_webhooks_and_disabled_routines() {
        let normal = Routine { id: "rt_normal".into(), ..ics_routine("Обычная") };
        let hook = Routine { id: "rt_hook".into(), hook: true, ..ics_routine("Вебхук") };
        let off = Routine { id: "rt_off".into(), enabled: false, ..ics_routine("Выключена") };
        let out = routines_ics(&[normal, hook, off], &[ics_bot()], ics_now());
        assert!(out.contains("UID:rt_normal@deloist"), "{out}");
        assert!(!out.contains("rt_hook"), "{out}");
        assert!(!out.contains("rt_off"), "{out}");
        assert_eq!(out.matches("BEGIN:VEVENT").count(), 1, "{out}");
    }

    #[test]
    fn failures_slow_a_routine_down() {
        assert_eq!([0, 2, 3, 5, 6, 40].map(failure_gap_ms), [0, 0, 3_600_000, 3_600_000, 4 * 3_600_000, 4 * 3_600_000]);
    }
}
