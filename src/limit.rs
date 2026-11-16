//! Сколько осталось у бота: доля бюджета для кольца в доке.
//!
//! У бота бывает до трёх пределов, и кольцо показывает самый тесный, тот, что кончится первым:
//! - `wallet` — кошелёк «Подписки Deloist» (боты на драйвере `deloist`): сколько осталось от баланса,
//!   который был в начале дня. Баланс программа узнаёт, когда спрашивает сайт (окно аккаунта, пополнение);
//!   по сети ради кольца мы не ходим, поэтому число может отставать.
//! - `plan` — лимит подписки провайдера (Claude, ChatGPT и др.). Сколько осталось, провайдеры не сообщают;
//!   известно только, что последний ход упёрся в лимит. Тогда доля 0, пока бот не ответит без ошибки.
//! - `tokens` — дневной предел токенов из настроек (`dailyTokenLimit`, им же ограничены рутины)
//!   против расхода всей команды за сегодня из `ledger`.
//!
//! Нет ни одного предела — `limit` пустой, в доке только тусклый обод цвета бота.

use serde_json::{json, Value};

/// Один предел: доля 0..1 и чем он измерен.
#[derive(Clone, Debug, PartialEq)]
pub struct Bound {
    pub source: &'static str,
    pub left: f64,
    pub spent: i64,
    pub cap: i64,
    pub unit: &'static str,
}

/// Порядок при равной доле: сначала то, что человек может поправить сам (кошелёк), затем подписка, затем токены.
fn rank(source: &str) -> u8 {
    match source {
        "wallet" => 0,
        "plan" => 1,
        _ => 2,
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
    Bound { source: "wallet", left, spent: (start - day.now_kop.max(0)).max(0), cap: start, unit: "kop" }
}

pub fn tokens_bound(spent: i64, cap: i64) -> Option<Bound> {
    (cap > 0).then(|| Bound { source: "tokens", left: 1.0 - spent.max(0) as f64 / cap as f64, spent: spent.max(0), cap, unit: "tokens" })
}

/// Последний ход упёрся в лимит или кончились деньги.
pub fn hit_limit(preview: &Value) -> bool {
    if preview.get("error").and_then(Value::as_bool) != Some(true) {
        return false;
    }
    let text = preview.get("text").and_then(Value::as_str).unwrap_or_default();
    let lower = text.to_lowercase();
    crate::drivers::is_limit_error(text) || lower.contains("лимит") || lower.contains("закончились деньги")
}

/// Поле `limit` бота в `/api/state`.
pub fn for_bot(uses_wallet: bool, preview: &Value, wallet: Option<WalletDay>, tokens_spent: i64, token_cap: i64) -> Value {
    let mut bounds = Vec::new();
    let blocked = hit_limit(preview);
    if uses_wallet {
        match wallet {
            Some(day) => bounds.push(wallet_bound(day)),
            None if blocked => bounds.push(Bound { source: "wallet", left: 0.0, spent: 0, cap: 0, unit: "kop" }),
            None => {}
        }
    } else if blocked {
        bounds.push(Bound { source: "plan", left: 0.0, spent: 0, cap: 0, unit: "" });
    }
    if let Some(b) = tokens_bound(tokens_spent, token_cap) {
        bounds.push(b);
    }
    match tightest(bounds) {
        Some(b) => json!({
            "left": (b.left * 1000.0).round() / 1000.0,
            "source": b.source,
            "spent": b.spent,
            "cap": b.cap,
            "unit": b.unit,
            "moneyLeftKop": if uses_wallet { wallet.map(|w| w.now_kop.max(0)) } else { None },
        }),
        None => Value::Null,
    }
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
        let tokens = tokens_bound(500_000, 1_000_000).unwrap();
        let picked = tightest(vec![tokens.clone(), wallet.clone()]).unwrap();
        assert_eq!(picked.source, "wallet");
        assert!((picked.left - 0.25).abs() < 1e-9);
        let even = tokens_bound(750_000, 1_000_000).unwrap();
        assert_eq!(tightest(vec![even, wallet]).unwrap().source, "wallet", "равные доли — кошелёк");
        assert!(tightest(vec![]).is_none());
    }

    #[test]
    fn a_top_up_never_shows_more_than_full_and_overspend_is_zero() {
        assert_eq!(wallet_bound(WalletDay { start_kop: 5_000, now_kop: 9_000 }).left, 1.0);
        assert_eq!(wallet_bound(WalletDay { start_kop: 0, now_kop: 0 }).left, 0.0);
        let over = tightest(vec![tokens_bound(1_500_000, 1_000_000).unwrap()]).unwrap();
        assert_eq!(over.left, 0.0);
        assert!(tokens_bound(10, 0).is_none(), "без предела токенов кольца нет");
    }

    #[test]
    fn a_limit_error_empties_the_ring_of_the_right_kind() {
        let hit = json!({ "error": true, "text": "Claude: usage limit reached, try again at 14:00" });
        let fine = json!({ "error": false, "text": "Готово" });
        assert_eq!(for_bot(false, &hit, None, 0, 0)["source"], "plan");
        assert_eq!(for_bot(false, &hit, None, 0, 0)["left"], 0.0);
        let broke = json!({ "error": true, "text": "На балансе Deloist закончились деньги. Пополни баланс" });
        assert_eq!(for_bot(true, &broke, None, 0, 0)["source"], "wallet");
        assert!(for_bot(false, &fine, None, 0, 0).is_null(), "нет пределов — нет кольца");
        let wallet = Some(WalletDay { start_kop: 20_000, now_kop: 14_600 });
        let item = for_bot(true, &fine, wallet, 100, 1_000_000);
        assert_eq!(item["source"], "wallet");
        assert_eq!(item["left"], 0.73);
        assert_eq!(item["moneyLeftKop"], 14_600);
    }
}
