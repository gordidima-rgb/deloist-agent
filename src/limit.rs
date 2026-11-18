//! Сколько осталось у бота: доля бюджета для кольца в доке.
//!
//! У бота бывает до четырёх пределов, и кольцо показывает самый тесный, тот, что кончится первым:
//! - `wallet` — кошелёк «Подписки Deloist» (боты на драйвере `deloist`): сколько осталось от баланса,
//!   который был в начале дня. Баланс программа узнаёт, когда спрашивает сайт (окно аккаунта, пополнение);
//!   по сети ради кольца мы не ходим, поэтому число может отставать.
//! - `plan` — лимит подписки агента (Claude, Codex, Grok по ACP): настоящий остаток, который агент сам
//!   назвал в `/status` или `/usage` (`Providers::subscription_left`). Не назвал — предела нет, а не догадка
//!   по тексту ошибки.
//! - `month` — месячный предел токенов самого бота (`monthly_token_limit`), сброс 1-го числа.
//! - `tokens` — дневной предел токенов команды (`dailyTokenLimit`). Живой разговор он не держит, поэтому
//!   кольцо показывает его, только когда рутина, цель или процесс на нём остановились (`Ledger::daily_stopped`).
//!
//! Нет ни одного предела — `limit` пустой, в доке только тусклый обод цвета бота.

use chrono::{DateTime, Datelike, Local, NaiveDate, NaiveTime, TimeZone};
use serde_json::{json, Value};

/// Один предел: доля 0..1, чем он измерен и когда сбросится (мс, если известно).
#[derive(Clone, Debug, PartialEq)]
pub struct Bound {
    pub source: &'static str,
    pub left: f64,
    pub spent: i64,
    pub cap: i64,
    pub unit: &'static str,
    pub resets_at: Option<i64>,
}

/// Порядок при равной доле: сначала то, что человек может поправить сам (кошелёк), затем подписка,
/// месячный предел бота и дневной предел токенов.
fn rank(source: &str) -> u8 {
    match source {
        "wallet" => 0,
        "plan" => 1,
        "month" => 2,
        _ => 3,
    }
}

/// Самый тесный предел из известных.
pub fn tightest(bounds: Vec<Bound>) -> Option<Bound> {
    bounds
        .into_iter()
        .filter(|b| b.left.is_finite())
        .map(|mut b| {
            b.left = b.left.clamp(0.0, 1.0);
            b
        })
        .min_by(|a, b| a.left.partial_cmp(&b.left).unwrap_or(std::cmp::Ordering::Equal).then(rank(a.source).cmp(&rank(b.source))))
}

/// Баланс кошелька за сегодня: сколько было утром (первый увиденный за день) и сколько сейчас, в копейках.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct WalletDay {
    pub start_kop: i64,
    pub now_kop: i64,
}

pub fn wallet_bound(day: WalletDay) -> Bound {
    // Пополнили днём: «сколько было» растёт вместе с балансом, иначе кольцо показало бы больше 100 %.
    let start = day.start_kop.max(day.now_kop).max(0);
    let left = if start > 0 { day.now_kop.max(0) as f64 / start as f64 } else { 0.0 };
    Bound { source: "wallet", left, spent: (start - day.now_kop.max(0)).max(0), cap: start, unit: "kop", resets_at: None }
}

/// Остаток подписки агента: процент, который он назвал, и когда окно сбросится.
pub fn plan_bound(left_percent: i64, resets_at: Option<i64>) -> Bound {
    Bound { source: "plan", left: left_percent as f64 / 100.0, spent: 0, cap: 0, unit: "", resets_at }
}

pub fn tokens_bound(source: &'static str, spent: i64, cap: i64, resets_at: Option<i64>) -> Option<Bound> {
    (cap > 0).then(|| Bound { source, left: 1.0 - spent.max(0) as f64 / cap as f64, spent: spent.max(0), cap, unit: "tokens", resets_at })
}

/// Полночь по местному времени, мс.
fn midnight_ms(date: NaiveDate) -> Option<i64> {
    Local.from_local_datetime(&date.and_time(NaiveTime::MIN)).earliest().map(|t| t.timestamp_millis())
}

/// Когда начнётся следующий день.
pub fn next_day_ms(now: DateTime<Local>) -> Option<i64> {
    midnight_ms(now.date_naive().succ_opt()?)
}

/// Когда начнётся следующий месяц: счёт токенов бота идёт с 1-го.
pub fn next_month_ms(now: DateTime<Local>) -> Option<i64> {
    let (year, month) = if now.month() == 12 { (now.year() + 1, 1) } else { (now.year(), now.month() + 1) };
    midnight_ms(NaiveDate::from_ymd_opt(year, month, 1)?)
}

/// Время сброса из строки агента → момент (мс). Агенты пишут по-разному: `16:01` (Codex: сегодня или завтра),
/// `Nov 12` (Codex, неделя: время не названо, берём конец того дня), `Nov 13, 11:10am` и `Nov 15, 4pm` (Claude).
/// Часовой пояс в скобках отбрасываем: агент называет время по местному часу. Не поняли строку — `None`.
pub fn parse_reset(text: &str, now: DateTime<Local>) -> Option<i64> {
    let text = text.split(" (").next()?.trim();
    let text = text.strip_prefix("resets").map(str::trim).unwrap_or(text);
    let (date, time) = match text.split_once(',') {
        Some((d, t)) => (Some(d.trim()), Some(t.trim())),
        None if text.starts_with(|c: char| c.is_ascii_alphabetic()) => match text.rsplit_once(' ') {
            // «Nov 12» — одна дата; «Nov 12 4pm» — дата и время.
            Some((d, t)) if d.contains(' ') => (Some(d.trim()), Some(t.trim())),
            _ => (Some(text), None),
        },
        None => (None, Some(text)),
    };
    let time = match time {
        Some(t) => Some(parse_clock(t)?),
        None => None,
    };
    let local = |date: NaiveDate, time: NaiveTime| Local.from_local_datetime(&date.and_time(time)).earliest();
    let at = match date {
        None => {
            let today = local(now.date_naive(), time?)?;
            if today > now { today } else { local(now.date_naive().succ_opt()?, time?)? }
        }
        Some(date) => {
            let mut words = date.split_whitespace();
            let (month, day) = (words.next()?, words.next()?);
            let on = |year: i32| NaiveDate::parse_from_str(&format!("{year} {month} {day}"), "%Y %b %d").ok();
            // Без времени сброс где-то в этот день: берём его конец.
            let build = |date: NaiveDate| match time {
                Some(t) => local(date, t),
                None => local(date.succ_opt()?, NaiveTime::MIN),
            };
            let this_year = build(on(now.year())?)?;
            // Агент в декабре называет январскую дату: прошлое больше чем на сутки — это следующий год.
            if this_year < now - chrono::Duration::days(1) { build(on(now.year() + 1)?)? } else { this_year }
        }
    };
    Some(at.timestamp_millis())
}

/// Ближайшее «ЧЧ:ММ» по местным часам: сегодня, если ещё не прошло, иначе завтра.
pub fn next_clock_ms(text: &str, now: DateTime<Local>) -> Option<i64> {
    let time = parse_clock(text)?;
    let today = Local.from_local_datetime(&now.date_naive().and_time(time)).earliest()?;
    let at = if today > now { today } else { Local.from_local_datetime(&now.date_naive().succ_opt()?.and_time(time)).earliest()? };
    Some(at.timestamp_millis())
}

/// `16:01`, `11:10am`, `4pm`. Пустая строка — `None`.
fn parse_clock(text: &str) -> Option<NaiveTime> {
    let lower = text.trim().to_lowercase().replace(' ', "");
    let (body, pm) = match (lower.strip_suffix("am"), lower.strip_suffix("pm")) {
        (Some(body), _) => (body, Some(false)),
        (_, Some(body)) => (body, Some(true)),
        _ => (lower.as_str(), None),
    };
    let (hour, minute) = body.split_once(':').unwrap_or((body, "0"));
    let (mut hour, minute) = (hour.parse::<u32>().ok()?, minute.parse::<u32>().ok()?);
    if let Some(pm) = pm {
        if !(1..=12).contains(&hour) {
            return None;
        }
        hour = hour % 12 + if pm { 12 } else { 0 };
    }
    NaiveTime::from_hms_opt(hour, minute, 0)
}

/// Что известно боту для кольца.
#[derive(Default)]
pub struct Inputs {
    /// Кошелёк Deloist, только у ботов на драйвере `deloist`.
    pub wallet: Option<WalletDay>,
    /// Подписка агента: сколько процентов осталось в самом тесном окне и когда оно сбросится.
    pub plan: Option<(i64, Option<i64>)>,
    /// Дневной предел команды: расход за день и предел — только если на нём что-то остановилось.
    pub daily: Option<(i64, i64)>,
    /// Месяц бота: расход и предел.
    pub month: Option<(i64, i64)>,
}

/// Поле `limit` бота в `/api/state`.
pub fn for_bot(inputs: Inputs, now: DateTime<Local>) -> Value {
    let mut bounds = Vec::new();
    bounds.extend(inputs.wallet.map(wallet_bound));
    bounds.extend(inputs.plan.map(|(left, at)| plan_bound(left, at)));
    bounds.extend(inputs.daily.and_then(|(spent, cap)| tokens_bound("tokens", spent, cap, next_day_ms(now))));
    bounds.extend(inputs.month.and_then(|(spent, cap)| tokens_bound("month", spent, cap, next_month_ms(now))));
    match tightest(bounds) {
        Some(b) => json!({
            "left": (b.left * 1000.0).round() / 1000.0,
            "source": b.source,
            "spent": b.spent,
            "cap": b.cap,
            "unit": b.unit,
            "resetsAt": b.resets_at,
            "moneyLeftKop": inputs.wallet.map(|w| w.now_kop.max(0)),
        }),
        None => Value::Null,
    }
}

/// «Отдых» модели после лимита: ключ модели → момент, когда её снова можно спрашивать. Один на компьютер,
/// лежит в `limits-rest.json`, поэтому переживает перезапуск и общий для всех ботов.
pub struct Rest {
    path: std::path::PathBuf,
    map: std::sync::Mutex<std::collections::HashMap<String, i64>>,
}

impl Rest {
    pub fn open(dir: &std::path::Path) -> Self {
        let path = dir.join("limits-rest.json");
        let map = std::fs::read_to_string(&path).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default();
        Self { path, map: std::sync::Mutex::new(map) }
    }

    /// Когда модель снова доступна, если она ещё отдыхает.
    pub fn until(&self, key: &str) -> Option<i64> {
        self.map.lock().unwrap().get(key).copied().filter(|&at| at > crate::store::now())
    }

    pub fn set(&self, key: &str, until: i64) {
        let mut map = self.map.lock().unwrap();
        let at = crate::store::now();
        map.retain(|_, &mut v| v > at);
        map.insert(key.to_string(), until);
        if let Ok(text) = serde_json::to_string(&*map) {
            let _ = std::fs::write(&self.path, text);
        }
    }
}

const BACKOFF_BASE_MS: i64 = 5 * 60_000;
const BACKOFF_MAX_MS: i64 = 24 * 3_600_000;

/// Пауза без названного срока: 5 минут, с каждой подряд идущей паузой вдвое дольше, не больше суток.
pub fn backoff_ms(pauses: u32) -> i64 {
    let base = if cfg!(debug_assertions) {
        std::env::var("CREW_TEST_PAUSE_BASE_MS").ok().and_then(|v| v.parse().ok()).unwrap_or(BACKOFF_BASE_MS)
    } else {
        BACKOFF_BASE_MS
    };
    base.saturating_mul(1i64 << pauses.min(20)).min(BACKOFF_MAX_MS.max(base))
}

/// Срок сброса из текста ошибки провайдера: «… resets 4pm», «… resets Nov 13, 11:10am». Не назван — `None`.
pub fn reset_in_error(message: &str, now: DateTime<Local>) -> Option<i64> {
    let at = message.char_indices().map(|(i, _)| i).find(|&i| message[i..].get(..6).is_some_and(|s| s.eq_ignore_ascii_case("resets")))?;
    parse_reset(&format!("resets{}", &message[at + 6..]), now).filter(|&ms| ms > now.timestamp_millis())
}

/// Ответ вместо вызова модели, когда бот выбрал свой месячный предел токенов.
pub const MONTH_LIMIT_REACHED: &str = "месячный лимит бота исчерпан, лимит можно поднять в карточке";

/// Предел бота за месяц: `limit <= 0` — без предела, иначе упёрлись, когда расход дошёл до него.
pub fn month_blocked(used: i64, limit: i64) -> bool {
    limit > 0 && used >= limit
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(month: u32, day: u32, hour: u32, minute: u32) -> DateTime<Local> {
        Local.with_ymd_and_hms(2026, month, day, hour, minute, 0).unwrap()
    }

    #[test]
    fn the_month_limit_blocks_at_and_above_but_not_below_or_when_unset() {
        assert!(!month_blocked(999, 1_000));
        assert!(month_blocked(1_000, 1_000));
        assert!(month_blocked(5_000, 1_000));
        assert!(!month_blocked(9_999_999, 0), "0 — без предела");
        assert!(!month_blocked(9_999_999, -5));
    }

    #[tokio::test]
    async fn the_month_total_counts_only_this_month_and_this_bot() {
        let dir = std::env::temp_dir().join(format!("deloist-month-{}-{}", std::process::id(), line!()));
        std::fs::create_dir_all(&dir).unwrap();
        let day = |p: i64, c: i64| json!({ "prompt": p, "completion": c });
        let usage = json!({
            "2026-10-31": { "bot_a": day(400, 100) },
            "2026-11-01": { "bot_a": day(10, 5), "bot_b": day(900, 900) },
            "2026-11-14": { "bot_a": day(100, 50) },
        });
        std::fs::write(dir.join("usage.json"), usage.to_string()).unwrap();
        let ledger = crate::ledger::Ledger::open(dir.clone()).await;
        assert_eq!(ledger.tokens_in_month("bot_a", "2026-11"), 165);
        assert_eq!(ledger.tokens_in_month("bot_a", "2026-10"), 500, "прошлый месяц считается отдельно: на 1-е число счёт с нуля");
        assert_eq!(ledger.tokens_in_month("bot_b", "2026-11"), 1_800);
        assert_eq!(ledger.tokens_in_month("nobody", "2026-11"), 0);
        // Живой вызов берёт текущий месяц и растёт на запись «сегодня».
        let before = ledger.tokens_month("bot_a");
        ledger.add_tokens("bot_a", 7, 3, 0, false).await;
        assert_eq!(ledger.tokens_month("bot_a"), before + 10);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_tightest_bound_wins_and_ties_go_to_the_wallet() {
        let wallet = wallet_bound(WalletDay { start_kop: 10_000, now_kop: 2_500 });
        let tokens = tokens_bound("tokens", 500_000, 1_000_000, None).unwrap();
        let picked = tightest(vec![tokens.clone(), wallet.clone()]).unwrap();
        assert_eq!(picked.source, "wallet");
        assert!((picked.left - 0.25).abs() < 1e-9);
        let even = tokens_bound("tokens", 750_000, 1_000_000, None).unwrap();
        assert_eq!(tightest(vec![even, wallet]).unwrap().source, "wallet", "равные доли — кошелёк");
        assert!(tightest(vec![]).is_none());
    }

    #[test]
    fn a_top_up_never_shows_more_than_full_and_overspend_is_zero() {
        assert_eq!(wallet_bound(WalletDay { start_kop: 5_000, now_kop: 9_000 }).left, 1.0);
        assert_eq!(wallet_bound(WalletDay { start_kop: 0, now_kop: 0 }).left, 0.0);
        let over = tightest(vec![tokens_bound("tokens", 1_500_000, 1_000_000, None).unwrap()]).unwrap();
        assert_eq!(over.left, 0.0);
        assert!(tokens_bound("tokens", 10, 0, None).is_none(), "без предела токенов кольца нет");
    }

    #[test]
    fn the_ring_takes_the_real_numbers_and_never_guesses_from_an_error() {
        let now = at(11, 17, 12, 0);
        // Нет ни подписки, ни кошелька, ни предела — кольца нет, что бы ни было в последнем ответе.
        assert!(for_bot(Inputs::default(), now).is_null());
        // Подписка агента: остаток и момент сброса идут в кольцо как есть.
        let reset = at(11, 17, 15, 0).timestamp_millis();
        let item = for_bot(Inputs { plan: Some((42, Some(reset))), ..Inputs::default() }, now);
        assert_eq!((item["source"].as_str(), item["left"].as_f64(), item["resetsAt"].as_i64()), (Some("plan"), Some(0.42), Some(reset)));
        let wallet = Some(WalletDay { start_kop: 20_000, now_kop: 14_600 });
        let item = for_bot(Inputs { wallet, ..Inputs::default() }, now);
        assert_eq!((item["source"].as_str(), item["left"].as_f64(), item["moneyLeftKop"].as_i64()), (Some("wallet"), Some(0.73), Some(14_600)));
    }

    #[test]
    fn the_month_limit_is_in_the_ring_and_resets_on_the_first() {
        let now = at(11, 17, 12, 0);
        let item = for_bot(Inputs { month: Some((250, 1_000)), ..Inputs::default() }, now);
        assert_eq!((item["source"].as_str(), item["left"].as_f64(), item["cap"].as_i64()), (Some("month"), Some(0.75), Some(1_000)));
        assert_eq!(item["resetsAt"].as_i64(), Some(at(12, 1, 0, 0).timestamp_millis()));
        assert_eq!(next_month_ms(at(12, 31, 23, 0)), Some(Local.with_ymd_and_hms(2027, 1, 1, 0, 0, 0).unwrap().timestamp_millis()));
        assert!(for_bot(Inputs { month: Some((250, 0)), ..Inputs::default() }, now).is_null(), "без предела бота кольца нет");
        // Самый тесный из двух: месяц бота против подписки.
        let tight = for_bot(Inputs { month: Some((250, 1_000)), plan: Some((10, None)), ..Inputs::default() }, now);
        assert_eq!(tight["source"], "plan");
    }

    #[test]
    fn agent_reset_strings_become_moments() {
        let now = at(11, 17, 12, 0);
        let ms = |month, day, hour, minute| Some(at(month, day, hour, minute).timestamp_millis());
        assert_eq!(parse_reset("16:01", now), ms(11, 17, 16, 1), "время впереди — сегодня");
        assert_eq!(parse_reset("09:30", now), ms(11, 18, 9, 30), "время уже прошло — завтра");
        assert_eq!(parse_reset("Nov 13, 11:10am", now), Some(Local.with_ymd_and_hms(2027, 11, 13, 11, 10, 0).unwrap().timestamp_millis()), "прошло больше суток — следующий год");
        assert_eq!(parse_reset("Nov 19, 11:10am (Asia/Bangkok)", now), ms(11, 19, 11, 10), "пояс в скобках не нужен");
        assert_eq!(parse_reset("Nov 20, 4pm", now), ms(11, 20, 16, 0));
        assert_eq!(parse_reset("resets Nov 18, 12am", now), ms(11, 18, 0, 0));
        assert_eq!(parse_reset("Nov 18, 12pm", now), ms(11, 18, 12, 0));
        assert_eq!(parse_reset("Nov 19", now), ms(11, 20, 0, 0), "дата без времени — конец того дня");
        assert_eq!(parse_reset("Nov 19 4pm", now), ms(11, 19, 16, 0));
        // Январская дата из декабря — следующий год.
        let december = Local.with_ymd_and_hms(2026, 12, 30, 12, 0, 0).unwrap();
        assert_eq!(parse_reset("Jan 2, 9am", december), Some(Local.with_ymd_and_hms(2027, 1, 2, 9, 0, 0).unwrap().timestamp_millis()));
        for junk in ["", "soon", "Nov", "25:99", "Foo 12"] {
            assert_eq!(parse_reset(junk, now), None, "{junk:?}");
        }
    }

    #[test]
    fn the_reset_is_found_inside_an_error_text() {
        let now = at(11, 12, 10, 0);
        let expected = at(11, 12, 16, 0).timestamp_millis();
        assert_eq!(reset_in_error("API 429: usage limit reached, resets 4pm (Europe/Moscow)", now), Some(expected));
        assert_eq!(reset_in_error("Resets 16:00", now), Some(expected));
        assert_eq!(reset_in_error("API 429: rate limit exceeded", now), None);
    }

    #[test]
    fn next_clock_is_today_or_tomorrow() {
        let now = at(11, 12, 10, 0);
        assert_eq!(next_clock_ms("14:30", now), Some(at(11, 12, 14, 30).timestamp_millis()));
        assert_eq!(next_clock_ms("09:00", now), Some(at(11, 13, 9, 0).timestamp_millis()));
        assert_eq!(next_clock_ms("soon", now), None);
    }

    #[test]
    fn backoff_doubles_up_to_a_day() {
        assert_eq!(backoff_ms(0), 5 * 60_000);
        assert_eq!(backoff_ms(1), 10 * 60_000);
        assert_eq!(backoff_ms(3), 40 * 60_000);
        assert_eq!(backoff_ms(20), 24 * 3_600_000);
    }
}
