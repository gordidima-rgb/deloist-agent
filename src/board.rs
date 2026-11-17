//! Доска «Нужен ты / В работе / Ждёт»: что сейчас делают боты и что и почему стоит в очереди.
//! Только чтение: ни ключей, ни текстов сообщений; у задач и очередей — короткое начало, как в самой очереди.

use serde_json::{json, Value};

use crate::store::{now, Bot};
use crate::App;

/// Почему сообщение бота ждёт в очереди. Код, а не текст: слова подбирает интерфейс.
/// Порядок: пауза, лимит месяца, карточка без ответа, занятость бота. `None` — ничто не держит.
pub fn hold_reason(app: &App, bot: &Bot) -> Option<&'static str> {
    if app.queues.is_paused(&bot.id) {
        return Some("paused_manual");
    }
    // Та же проверка, что в `turn::drive`: бот с исчерпанным пределом месяца не начнёт ход.
    if crate::limit::month_blocked(app.ledger.tokens_month(&bot.id), bot.monthly_token_limit) {
        return Some("limit");
    }
    if !app.turns.pending_for(&bot.id).is_empty() {
        return Some("card");
    }
    app.turns.is_busy(&bot.id).then_some("busy")
}

fn preview(text: &str) -> String {
    let flat = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if flat.chars().count() <= 80 {
        return flat;
    }
    format!("{}…", flat.chars().take(80).collect::<String>().trim_end())
}

/// «В работе» и «Ждёт» одним чтением: ходы ботов, очереди сообщений и командные задачи.
pub fn build(app: &App) -> (Vec<Value>, Vec<Value>) {
    let bots = app.store.bots();
    let rooms: std::collections::HashMap<String, String> = app.store.rooms().into_iter().map(|r| (r.id, r.name)).collect();
    let at = now();
    let mut working = Vec::new();
    let mut waiting = Vec::new();
    for bot in &bots {
        if let Some(since) = app.turns.busy_since(&bot.id) {
            let room = app.turns.conv_of(&bot.id).and_then(|c| c.strip_prefix("room:").map(str::to_string));
            let mut item = json!({ "kind": "turn", "botId": bot.id, "botName": bot.name, "since": since, "ms": (at - since).max(0) });
            if let Some(room) = room.filter(|r| rooms.contains_key(r)) {
                item["roomName"] = json!(rooms.get(&room));
                item["roomId"] = json!(room);
            }
            working.push(item);
        }
        let queue = app.queues.get(&bot.id);
        if !queue.items.is_empty() {
            let first = queue.items.first().and_then(|i| i["text"].as_str()).unwrap_or_default();
            waiting.push(json!({
                "kind": "queue", "botId": bot.id, "botName": bot.name, "count": queue.items.len(),
                "reason": hold_reason(app, bot), "text": preview(first),
            }));
        }
    }
    for task in app.tasks.all().into_iter().filter(|t| t.is_open()) {
        let running = task.state == "running";
        let mut item = json!({
            "kind": "task", "id": task.id, "botId": task.to, "botName": task.to_name, "fromName": task.from_name,
            "text": preview(&task.text),
        });
        if task.room != "" && rooms.contains_key(&task.room) {
            item["roomId"] = json!(task.room);
            item["roomName"] = json!(rooms.get(&task.room));
        }
        if running {
            let since = if task.started > 0 { task.started } else { task.created };
            item["since"] = json!(since);
            item["ms"] = json!((at - since).max(0));
            working.push(item);
        } else {
            // Задача ждёт, пока получатель не освободится.
            item["reason"] = json!("busy");
            waiting.push(item);
        }
    }
    (working, waiting)
}
