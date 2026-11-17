//! Учёт: сколько токенов потрачено и что боты делали без человека.
//! Лежит рядом с остальными данными: `usage.json` и `journal.jsonl`, всё можно прочитать руками.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::Mutex;

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tokio::io::AsyncWriteExt;

const KEEP_DAYS: usize = 60;
const JOURNAL_LIMIT_BYTES: usize = 2 * 1024 * 1024;

#[derive(Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct DayUsage {
    pub prompt: i64,
    pub completion: i64,
    /// Сколько входных токенов провайдер отдал из кэша: это самая дешёвая часть расхода.
    pub cached: i64,
    /// Провайдер не прислал счётчик хотя бы раз, часть числа посчитана примерно.
    pub estimated: bool,
}

type Usage = BTreeMap<String, BTreeMap<String, DayUsage>>;

/// Токены по модели за день; провайдер хранится рядом, чтобы экран показал, чья это модель.
#[derive(Clone, Default, Serialize, Deserialize)]
#[serde(default)]
struct ModelDay {
    provider: String,
    tokens: i64,
}

type ModelUsage = BTreeMap<String, BTreeMap<String, ModelDay>>;
/// Дата -> вид закулисной работы (`router`, `memory`, `helper`) -> токены.
type Behind = BTreeMap<String, BTreeMap<String, i64>>;

/// Что ledger знает о боте для экрана «Использование».
pub struct BotInfo {
    pub id: String,
    pub name: String,
    pub engine: String,
    pub model: String,
    pub monthly_limit: Option<i64>,
}

/// Один ответ бота для `receipts.jsonl`.
pub struct Receipt<'a> {
    pub bot_id: &'a str,
    /// `dm` или имя канала.
    pub place: &'a str,
    pub model: &'a str,
    pub provider: &'a str,
    pub seconds: f64,
    pub tokens: i64,
    pub failed: bool,
    /// Ответ или текст ошибки: в строку идут первые 80 знаков.
    pub text: &'a str,
    pub thread_id: &'a str,
}

pub struct Ledger {
    dir: PathBuf,
    usage: Mutex<Usage>,
    models: Mutex<ModelUsage>,
    behind: Mutex<Behind>,
    write_gate: tokio::sync::Mutex<()>,
    /// День, в который что-то без человека остановилось на дневном пределе токенов.
    daily_stop: Mutex<String>,
}

async fn read_json<T: serde::de::DeserializeOwned + Default>(dir: &std::path::Path, name: &str) -> T {
    tokio::fs::read_to_string(dir.join(name)).await.ok().and_then(|text| serde_json::from_str(&text).ok()).unwrap_or_default()
}

fn today() -> String {
    chrono::Local::now().format("%Y-%m-%d").to_string()
}

impl Ledger {
    pub async fn open(dir: PathBuf) -> Self {
        let usage = tokio::fs::read_to_string(dir.join("usage.json"))
            .await
            .ok()
            .and_then(|text| serde_json::from_str(&text).ok())
            .unwrap_or_default();
        let models = read_json(&dir, "usage_models.json").await;
        let behind = read_json(&dir, "usage_behind.json").await;
        let ledger = Self { dir, usage: Mutex::new(usage), models: Mutex::new(models), behind: Mutex::new(behind), write_gate: tokio::sync::Mutex::new(()), daily_stop: Mutex::new(String::new()) };
        ledger.trim_journal().await;
        ledger
    }

    pub async fn add_tokens(&self, bot_id: &str, prompt: i64, completion: i64, cached: i64, estimated: bool) {
        let snapshot = {
            let mut usage = self.usage.lock().unwrap();
            let day = usage.entry(today()).or_default().entry(bot_id.to_string()).or_default();
            day.prompt += prompt;
            day.completion += completion;
            day.cached += cached.min(prompt);
            day.estimated |= estimated;
            while usage.len() > KEEP_DAYS {
                usage.pop_first();
            }
            serde_json::to_string_pretty(&*usage).unwrap_or_default()
        };
        let _gate = self.write_gate.lock().await;
        if let Err(e) = tokio::fs::write(self.dir.join("usage.json"), snapshot).await {
            eprintln!("[ledger] не записал расход: {e}");
        }
    }

    async fn persist(&self, name: &str, snapshot: String) {
        let _gate = self.write_gate.lock().await;
        if let Err(e) = tokio::fs::write(self.dir.join(name), snapshot).await {
            eprintln!("[ledger] не записал {name}: {e}");
        }
    }

    /// Токены ответа по модели: рядом с `add_tokens`, в отдельном файле, старый `usage.json` не меняется.
    pub async fn add_model_tokens(&self, model: &str, provider: &str, tokens: i64) {
        if tokens <= 0 {
            return;
        }
        let snapshot = {
            let mut models = self.models.lock().unwrap();
            let slot = models.entry(today()).or_default().entry(model.to_string()).or_default();
            slot.tokens += tokens;
            slot.provider = provider.to_string();
            while models.len() > KEEP_DAYS {
                models.pop_first();
            }
            serde_json::to_string_pretty(&*models).unwrap_or_default()
        };
        self.persist("usage_models.json", snapshot).await;
    }

    /// Закулисные токены: `router`, `memory` или `helper`.
    pub async fn add_behind(&self, kind: &str, tokens: i64) {
        if tokens <= 0 {
            return;
        }
        let snapshot = {
            let mut behind = self.behind.lock().unwrap();
            *behind.entry(today()).or_default().entry(kind.to_string()).or_default() += tokens;
            while behind.len() > KEEP_DAYS {
                behind.pop_first();
            }
            serde_json::to_string_pretty(&*behind).unwrap_or_default()
        };
        self.persist("usage_behind.json", snapshot).await;
    }

    /// Одна строка на ответ бота: `receipts.jsonl`, из неё строятся экраны «Использование» и «Отчёт».
    pub async fn add_receipt(&self, receipt: &Receipt<'_>) {
        let entry = json!({
            "id": crate::store::uid("rc"),
            "ts": chrono::Local::now().format("%Y-%m-%dT%H:%M:%S").to_string(),
            "botId": receipt.bot_id, "where": receipt.place, "model": receipt.model, "provider": receipt.provider,
            "seconds": (receipt.seconds * 10.0).round() / 10.0, "tokens": receipt.tokens,
            "outcome": if receipt.failed { "failed" } else { "ok" },
            "preview": receipt.text.trim().chars().take(80).collect::<String>(),
            "threadId": receipt.thread_id,
        });
        let mut line = entry.to_string();
        line.push('\n');
        let _gate = self.write_gate.lock().await;
        if let Ok(mut file) = tokio::fs::OpenOptions::new().create(true).append(true).open(self.dir.join("receipts.jsonl")).await {
            if let Err(e) = file.write_all(line.as_bytes()).await {
                eprintln!("[ledger] не записал квитанцию: {e}");
            }
        }
    }

    /// Все квитанции, старые первыми.
    pub async fn read_receipts(&self) -> Vec<Value> {
        let text = tokio::fs::read_to_string(self.dir.join("receipts.jsonl")).await.unwrap_or_default();
        text.lines().filter_map(|line| serde_json::from_str(line).ok()).collect()
    }

    /// `GET /api/usage`: месяц целиком. `bots` — имя, движок, модель и месячный лимит из настроек ботов.
    pub async fn usage_month(&self, month: &str, bots: &[BotInfo]) -> Value {
        let receipts: Vec<Value> =
            self.read_receipts().await.into_iter().filter(|r| r["ts"].as_str().is_some_and(|ts| ts.starts_with(month))).collect();
        let (days, bot_tokens, total) = {
            let usage = self.usage.lock().unwrap();
            let mut days = Vec::new();
            let mut bot_tokens: BTreeMap<String, i64> = BTreeMap::new();
            let mut total = 0;
            for (date, by_bot) in usage.iter().filter(|(date, _)| date.starts_with(month)) {
                let mut row = serde_json::Map::new();
                for (id, day) in by_bot {
                    let tokens = day.prompt + day.completion;
                    total += tokens;
                    *bot_tokens.entry(id.clone()).or_default() += tokens;
                    row.insert(id.clone(), json!(tokens));
                }
                days.push(json!({ "date": date, "byBot": row }));
            }
            (days, bot_tokens, total)
        };
        let mut by_model: BTreeMap<String, (String, i64)> = BTreeMap::new();
        for (_, day_models) in self.models.lock().unwrap().iter().filter(|(date, _)| date.starts_with(month)) {
            for (id, day) in day_models {
                let slot = by_model.entry(id.clone()).or_default();
                slot.1 += day.tokens;
                if !day.provider.is_empty() {
                    slot.0 = day.provider.clone();
                }
            }
        }
        let mut models: Vec<Value> =
            by_model.into_iter().map(|(id, (provider, tokens))| json!({ "id": id, "provider": provider, "tokens": tokens })).collect();
        models.sort_by_key(|m| std::cmp::Reverse(m["tokens"].as_i64().unwrap_or(0)));
        let mut behind_sum = [0i64; 3];
        for (_, kinds) in self.behind.lock().unwrap().iter().filter(|(date, _)| date.starts_with(month)) {
            for (slot, kind) in behind_sum.iter_mut().zip(["router", "memory", "helper"]) {
                *slot += kinds.get(kind).copied().unwrap_or(0);
            }
        }
        let name_of = |id: &str| bots.iter().find(|b| b.id == id).map_or_else(|| id.to_string(), |b| b.name.clone());
        let mut bot_rows: Vec<Value> = bots
            .iter()
            .map(|b| (b.id.clone(), b.name.clone(), b.engine.clone(), b.model.clone(), b.monthly_limit))
            .chain(
                bot_tokens
                    .keys()
                    .filter(|id| !bots.iter().any(|b| &b.id == *id))
                    .map(|id| (id.clone(), id.clone(), String::new(), String::new(), None)),
            )
            .map(|(id, name, engine, model, limit)| {
                let replies = receipts.iter().filter(|r| r["botId"] == id.as_str()).count();
                json!({ "id": id, "name": name, "engine": engine, "model": model, "replies": replies,
                        "tokens": bot_tokens.get(&id).copied().unwrap_or(0), "monthlyLimit": limit })
            })
            .collect();
        bot_rows.sort_by_key(|b| std::cmp::Reverse(b["tokens"].as_i64().unwrap_or(0)));
        let seconds: f64 = receipts.iter().map(|r| r["seconds"].as_f64().unwrap_or(0.0)).sum();
        let avg = if receipts.is_empty() { 0.0 } else { (seconds / receipts.len() as f64 * 10.0).round() / 10.0 };
        let latest: Vec<Value> = receipts
            .iter()
            .rev()
            .take(20)
            .map(|r| {
                let id = r["botId"].as_str().unwrap_or("");
                json!({ "id": r["id"], "ts": r["ts"], "botId": id, "botName": name_of(id), "where": r["where"], "model": r["model"],
                        "seconds": r["seconds"], "tokens": r["tokens"], "outcome": r["outcome"], "preview": r["preview"], "threadId": r["threadId"] })
            })
            .collect();
        json!({
            "month": month, "totalTokens": total, "replies": receipts.len(), "avgSeconds": avg,
            "days": days, "bots": bot_rows, "models": models,
            "behind": { "router": behind_sum[0], "memory": behind_sum[1], "helper": behind_sum[2] },
            "receipts": latest,
        })
    }

    /// `GET /api/report`: неделя Пн–Вс и пять предыдущих по квитанциям. `names` — id и имя бота.
    pub async fn report_week(&self, any_day: chrono::NaiveDate, names: &[(String, String)]) -> Value {
        use chrono::Datelike;
        let start = any_day - chrono::Duration::days(any_day.weekday().num_days_from_monday() as i64);
        let day = |d: chrono::NaiveDate| d.format("%Y-%m-%d").to_string();
        let end = start + chrono::Duration::days(6);
        let receipts = self.read_receipts().await;
        let in_range = |r: &Value, from: chrono::NaiveDate, to: chrono::NaiveDate| {
            r["ts"].as_str().and_then(|ts| ts.get(..10)).is_some_and(|d| d >= day(from).as_str() && d <= day(to).as_str())
        };
        let week: Vec<&Value> = receipts.iter().filter(|r| in_range(r, start, end)).collect();
        let ok_count = week.iter().filter(|r| r["outcome"] == "ok").count();
        let percent = |ok: usize, all: usize| if all == 0 { 0 } else { (ok as f64 / all as f64 * 100.0).round() as i64 };
        let weeks: Vec<Value> = (0..6)
            .rev()
            .map(|i| {
                let from = start - chrono::Duration::days(7 * i);
                let rows: Vec<&Value> = receipts.iter().filter(|r| in_range(r, from, from + chrono::Duration::days(6))).collect();
                let ok = rows.iter().filter(|r| r["outcome"] == "ok").count();
                json!({ "start": day(from), "ok": ok, "failed": rows.len() - ok })
            })
            .collect();
        let mut order: Vec<&str> = Vec::new();
        for r in &week {
            let id = r["botId"].as_str().unwrap_or("");
            if !order.contains(&id) {
                order.push(id);
            }
        }
        let mut bots: Vec<Value> = order
            .into_iter()
            .map(|id| {
                let rows: Vec<&&Value> = week.iter().filter(|r| r["botId"] == id).collect();
                let ok = rows.iter().filter(|r| r["outcome"] == "ok").count();
                let n = rows.len() as f64;
                let tokens: f64 = rows.iter().map(|r| r["tokens"].as_f64().unwrap_or(0.0)).sum();
                let seconds: f64 = rows.iter().map(|r| r["seconds"].as_f64().unwrap_or(0.0)).sum();
                let failures: Vec<Value> =
                    rows.iter().rev().filter(|r| r["outcome"] != "ok").take(10).map(|r| json!({ "ts": r["ts"], "preview": r["preview"] })).collect();
                let name = names.iter().find(|(bot, _)| bot == id).map_or(id, |(_, name)| name.as_str());
                json!({ "id": id, "name": name, "replies": rows.len(), "okPercent": percent(ok, rows.len()),
                        "avgTokens": (tokens / n).round() as i64, "avgSeconds": (seconds / n * 10.0).round() / 10.0, "failures": failures })
            })
            .collect();
        bots.sort_by_key(|b| std::cmp::Reverse(b["replies"].as_i64().unwrap_or(0)));
        json!({
            "weekStart": day(start), "weekEnd": day(end), "replies": week.len(), "okCount": ok_count,
            "failedCount": week.len() - ok_count, "okPercent": percent(ok_count, week.len()), "weeks": weeks, "bots": bots,
        })
    }

    /// Рутина, цель или процесс остановились на дневном пределе токенов: кольцо ботов покажет предел сегодня.
    pub fn note_daily_stop(&self) {
        *self.daily_stop.lock().unwrap() = today();
    }

    pub fn daily_stopped(&self) -> bool {
        *self.daily_stop.lock().unwrap() == today()
    }

    /// Расход ботов за сегодня для дневного лимита рутин. Служебная память (`_memory`: извлечение и разбор после
    /// задачи) сюда не входит: её траты не должны останавливать рутины; в отчёте она видна отдельной строкой.
    pub fn tokens_today(&self) -> i64 {
        self.usage
            .lock()
            .unwrap()
            .get(&today())
            .map_or(0, |bots| bots.iter().filter(|(id, _)| id.as_str() != crate::extract::LEDGER_ID).map(|(_, d)| d.prompt + d.completion).sum())
    }

    /// Для интерфейса: всего за сегодня, по ботам и доля кэша.
    pub fn today_json(&self) -> Value {
        let usage = self.usage.lock().unwrap();
        let bots = usage.get(&today()).cloned().unwrap_or_default();
        let total: i64 = bots.values().map(|d| d.prompt + d.completion).sum();
        let estimated = bots.values().any(|d| d.estimated);
        let prompt: i64 = bots.values().map(|d| d.prompt).sum();
        let cached: i64 = bots.values().map(|d| d.cached).sum();
        // Доля кэша считается от входа: выход в кэш не попадает никогда.
        let rate = if prompt > 0 { (cached as f64 / prompt as f64 * 100.0).round() as i64 } else { 0 };
        json!({ "tokens": total, "estimated": estimated, "cached": cached, "cacheRate": rate, "bots": bots })
    }

    pub async fn append_journal(&self, entry: &Value) {
        let mut line = entry.to_string();
        line.push('\n');
        let _gate = self.write_gate.lock().await;
        let file = tokio::fs::OpenOptions::new().create(true).append(true).open(self.dir.join("journal.jsonl")).await;
        match file {
            Ok(mut file) => {
                if let Err(e) = file.write_all(line.as_bytes()).await {
                    eprintln!("[ledger] не записал журнал: {e}");
                }
            }
            Err(e) => eprintln!("[ledger] не открыл журнал: {e}"),
        }
    }

    /// Свежие записи первыми.
    pub async fn read_journal(&self, limit: usize) -> Vec<Value> {
        let text = tokio::fs::read_to_string(self.dir.join("journal.jsonl")).await.unwrap_or_default();
        text.lines().rev().filter_map(|line| serde_json::from_str(line).ok()).take(limit).collect()
    }

    /// Журнал решений о разрешениях: кто, что, где и каким правилом. Отдельно от журнала рутин,
    /// чтобы клики не тонули в отчётах ночной смены и не зажигали их отметку «новое».
    pub async fn append_activity(&self, entry: &Value) {
        let mut line = entry.to_string();
        line.push('\n');
        let _gate = self.write_gate.lock().await;
        let file = tokio::fs::OpenOptions::new().create(true).append(true).open(self.dir.join("activity.jsonl")).await;
        if let Ok(mut file) = file {
            if let Err(e) = file.write_all(line.as_bytes()).await {
                eprintln!("[ledger] не записал журнал решений: {e}");
            }
        }
    }

    /// Свежие решения бота первыми.
    pub async fn read_activity(&self, bot_id: &str, limit: usize) -> Vec<Value> {
        let text = tokio::fs::read_to_string(self.dir.join("activity.jsonl")).await.unwrap_or_default();
        text.lines()
            .rev()
            .filter_map(|line| serde_json::from_str::<Value>(line).ok())
            .filter(|entry| entry["botId"] == bot_id)
            .take(limit)
            .collect()
    }

    /// Журнал не должен расти вечно: при запуске оставляем свежую половину.
    async fn trim_journal(&self) {
        self.trim_file("activity.jsonl").await;
        self.trim_file("journal.jsonl").await;
        self.trim_file("receipts.jsonl").await;
    }

    async fn trim_file(&self, name: &str) {
        let path = self.dir.join(name);
        let Ok(text) = tokio::fs::read_to_string(&path).await else { return };
        if text.len() <= JOURNAL_LIMIT_BYTES {
            return;
        }
        let mut cut = text.len() - JOURNAL_LIMIT_BYTES / 2;
        while !text.is_char_boundary(cut) {
            cut += 1;
        }
        let tail = text[cut..].split_once('\n').map_or("", |(_, rest)| rest);
        let _ = tokio::fs::write(&path, tail).await;
    }
}

impl Ledger {
    /// Токены бота (вход и выход) за текущий календарный месяц.
    pub fn tokens_month(&self, bot_id: &str) -> i64 {
        self.tokens_in_month(bot_id, &chrono::Local::now().format("%Y-%m").to_string())
    }

    /// То же за месяц `YYYY-MM`: ключи учёта — даты `YYYY-MM-DD`.
    pub fn tokens_in_month(&self, bot_id: &str, month: &str) -> i64 {
        let usage = self.usage.lock().unwrap();
        usage.iter().filter(|(day, _)| day.starts_with(month)).filter_map(|(_, bots)| bots.get(bot_id)).map(|d| d.prompt + d.completion).sum()
    }
}

#[cfg(test)]
mod tests {
    use super::Ledger;

    #[tokio::test]
    async fn memory_housekeeping_does_not_count_toward_the_daily_limit() {
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("target").join("test-tmp").join(crate::store::uid("ledger"));
        std::fs::create_dir_all(&dir).unwrap();
        let ledger = Ledger::open(dir.clone()).await;
        ledger.add_tokens("bot_a", 100, 20, 0, false).await;
        ledger.add_tokens(crate::extract::LEDGER_ID, 5_000, 500, 0, false).await;
        assert_eq!(ledger.tokens_today(), 120);
        assert_eq!(ledger.today_json()["tokens"], 5_620, "в отчёте служебная память видна");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
