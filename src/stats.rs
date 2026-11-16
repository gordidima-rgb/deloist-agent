//! Счётчики сеанса бота — строка под полем ввода: ходы, шаги, скорость, токены, кэш
//! и сколько окна истории занято до сжатия. Сеанс длится до «Очистить чат».

use std::sync::Arc;

use serde_json::{json, Map, Value};

use crate::drivers::Usage;
use crate::store::{now, Thread};
use crate::App;

/// Где счётчики лежат у бота: служебное поле, профиль его не правит.
const KEY: &str = "session";

/// Один шаг хода: запрос к модели и инструменты после него.
#[derive(Debug, Default, Clone, Copy)]
pub struct Step {
    /// Первый шаг хода: с него ход и считается.
    pub first: bool,
    /// Расход шага; у демо и CLI его нет.
    pub usage: Option<Usage>,
    /// Сколько ждали модель целиком.
    pub model_ms: i64,
    /// Сколько ждали первый кусок ответа.
    pub ttft_ms: Option<i64>,
    /// Сколько работали инструменты шага.
    pub tool_ms: i64,
}

fn num(map: &Map<String, Value>, key: &str) -> i64 {
    map.get(key).and_then(Value::as_i64).unwrap_or(0)
}

fn bump(map: &mut Map<String, Value>, key: &str, by: i64) {
    let value = num(map, key) + by;
    map.insert(key.to_string(), json!(value));
}

fn raw(app: &App, bot_id: &str) -> Map<String, Value> {
    app.store
        .bot(bot_id)
        .and_then(|bot| bot.extra.get(KEY).and_then(Value::as_object).cloned())
        .unwrap_or_default()
}

/// Добавить шаг к сеансу. `inflight` — знаки шагов хода, которых ещё нет в треде.
pub async fn record(app: &Arc<App>, bot_id: &str, step: Step, inflight: usize) {
    let mut map = raw(app, bot_id);
    if !map.contains_key("since") {
        map.insert("since".into(), json!(now()));
    }
    if step.first {
        bump(&mut map, "turns", 1);
    }
    bump(&mut map, "steps", 1);
    bump(&mut map, "modelMs", step.model_ms);
    bump(&mut map, "toolMs", step.tool_ms);
    if let Some(ttft) = step.ttft_ms {
        bump(&mut map, "ttftMs", ttft);
        bump(&mut map, "ttftCount", 1);
    }
    if let Some(usage) = step.usage {
        bump(&mut map, "prompt", usage.prompt);
        bump(&mut map, "cached", usage.cached);
        bump(&mut map, "completion", usage.completion);
        // Скорость — по времени генерации, без ожидания первого токена.
        bump(&mut map, "genMs", (step.model_ms - step.ttft_ms.unwrap_or(0)).max(0));
        bump(&mut map, "genTokens", usage.completion);
    }
    let _ = app.store.set_bot_extra(bot_id, KEY, Value::Object(map)).await;
    let stats = snapshot(app, bot_id, inflight).await;
    app.turns.emit(json!({ "type": "session_stats", "botId": bot_id, "stats": stats }));
}

/// «Очистить чат» начинает новый сеанс.
pub async fn reset(app: &Arc<App>, bot_id: &str) {
    let _ = app.store.set_bot_extra(bot_id, KEY, json!({ "since": now() })).await;
}

/// Что показать человеку: суммы и производные (скорость, доля кэша, заполненность окна).
pub async fn snapshot(app: &Arc<App>, bot_id: &str, inflight: usize) -> Value {
    let map = raw(app, bot_id);
    let prompt = num(&map, "prompt");
    let cached = num(&map, "cached");
    let gen_ms = num(&map, "genMs");
    let gen_tokens = num(&map, "genTokens");
    let ttft_count = num(&map, "ttftCount");
    let bot = app.store.bot(bot_id).unwrap_or_default();
    let budget = crate::context::budget_for(app, &bot);
    let window = crate::context::window_for(app, &bot);
    let chars = app.store.window_size(Thread::Bot(bot_id)).await + inflight;
    let percent = if budget > 0 { ((chars as f64 / budget as f64) * 100.0).round().min(100.0) as i64 } else { 0 };
    let settings = app.store.settings();
    let choice = crate::turn::main_choice(app, &bot, &settings);
    let acp = if choice.driver == "acp" || choice.driver.starts_with("acp:") {
        crate::context::acp_usage(bot_id, &choice.model, chars)
    } else {
        None
    };
    let mut out = json!({
        "since": map.get("since").cloned().unwrap_or(Value::Null),
        "turns": num(&map, "turns"),
        "steps": num(&map, "steps"),
        "prompt": prompt,
        "cached": cached,
        "completion": num(&map, "completion"),
        "modelMs": num(&map, "modelMs"),
        "toolMs": num(&map, "toolMs"),
        "ttftMs": if ttft_count > 0 { num(&map, "ttftMs") / ttft_count } else { 0 },
        "tps": if gen_ms > 0 { ((gen_tokens as f64) * 1000.0 / gen_ms as f64).round() as i64 } else { 0 },
        "cacheRate": if prompt > 0 { ((cached as f64) * 100.0 / prompt as f64).round() as i64 } else { 0 },
        // Токены — оценка по знакам. `window` — настоящее окно модели, если оно известно (у агентов ACP и CLI его нет);
        // `compactAt` — с какого размера старая часть истории сожмётся в сводку.
        "context": {
            "chars": chars,
            "budget": budget,
            "percent": percent,
            "tokens": crate::context::approx_tokens(chars),
            "compactAt": crate::context::approx_tokens(budget),
            "window": window.unwrap_or(crate::context::DEFAULT_TOKENS),
            "windowKnown": window.is_some(),
            "mode": crate::context::mode_of(&bot).key(),
            // Порог сжатия в процентах окна: заданный ползунком или равный старому режиму.
            "compactPercent": crate::context::effective_percent(&bot, window),
        },
    });
    // Для ACP-агентов: сколько занято и какое окно, и откуда это известно («agent» — сам агент, «table» — таблица по модели).
    if let Some((used, size, source)) = acp {
        out["context"]["usedTokens"] = json!(used);
        out["context"]["windowTokens"] = json!(size);
        out["context"]["source"] = json!(source);
        out["context"]["windowKnown"] = json!(true);
        out["context"]["window"] = json!(size);
        out["context"]["percent"] = json!(((used as f64 / size as f64) * 100.0).round().min(100.0) as i64);
    }
    out
}
