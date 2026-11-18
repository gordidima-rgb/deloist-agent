//! Сводка прав бота словами для окна: «что можно без спроса, о чём спросит, чего не будет».
//! Решения берутся из тех же функций, что решают на деле (`tools::mode_needs_approval_for`, `always_asks`,
//! `reach::release_command`), а не из своей таблицы. Секретов в ответе нет.

use serde_json::{json, Value};

use crate::store::Bot;
use crate::{tools, App};

/// Вызовы, по которым видно действие режима: подпись и пример вызова для `mode_needs_approval_for`.
fn probes() -> Vec<(&'static str, &'static str, Value)> {
    vec![
        ("читать файлы и искать", "read_file", json!({ "path": "note.txt" })),
        ("писать файлы в своей папке", "write_file", json!({ "path": "note.txt" })),
        ("запускать простые команды в своей папке", "run_command", json!({ "command": "echo hi" })),
        ("создавать и править ботов", "create_bot", json!({})),
        ("создавать и запускать рутины", "create_routine", json!({})),
        ("тратить деньги на картинки", "generate_image", json!({})),
        ("передавать файлы другим ботам", "share_file", json!({})),
    ]
}

fn mode_text(mode: &str) -> &'static str {
    match mode {
        "strict" => "Строгий: спрашивает перед всем, что что-то меняет.",
        "auto" => "Свобода действий: решает сам, спрашивает только о том, что в списках ниже.",
        _ => "Спрашивать: свою папку и чтение берёт без вопроса, опасное и чужое спрашивает.",
    }
}

fn level_text(level: &str) -> &'static str {
    match level {
        "allow" => "можно",
        "deny" => "нельзя",
        _ => "спросит",
    }
}

pub fn summary(app: &App, bot: &Bot) -> Value {
    // Режим без списка «Всегда спрашивать»: он показан отдельно, а здесь видно, что решает сам режим.
    let effective = app.turns.effective_approval(&app.store, &bot.id, &bot.approval);
    let probe_bot = Bot { approval: effective.clone(), always_ask: Vec::new(), ..bot.clone() };
    let mut allowed = Vec::new();
    let mut asks = Vec::new();
    for (label, name, args) in probes() {
        if tools::mode_needs_approval_for(app, &probe_bot, name, &args) { asks.push(label.to_string()) } else { allowed.push(label.to_string()) }
    }
    // Что спрашивается в любом режиме, даже в «Свободе действий».
    asks.push("в любом режиме: выпуск наружу (push, релиз, зеркало), файлы с ключами и ключи SSH, автозапуск, .git и программы".into());
    for (id, label) in tools::ALWAYS_ASK_KINDS {
        if bot.always_ask.iter().any(|k| k == id) {
            asks.push(format!("всегда спрашивает (по вашему списку): {label}"));
        }
    }
    let never = vec!["служебные файлы и секреты программы: команды, которые к ним тянутся, отклоняются".to_string()];
    let messengers: Vec<Value> = app.turns.messengers.entries(&bot.id).into_iter().map(|m| {
        let rules = crate::messengers::RULES.iter().map(|r| format!("{} — {}", r.label().to_lowercase(), level_text(m["rules"][r.key()].as_str().unwrap_or("ask")))).collect::<Vec<_>>();
        json!(format!("{}: {}", m["name"].as_str().unwrap_or_default(), rules.join("; ")))
    }).collect();
    let stored = app.turns.perms.rules().iter().filter(|r| r.bot_id == bot.id).count();
    json!({
        "mode": bot.approval,
        "modeText": mode_text(&bot.approval),
        "floorText": (effective != bot.approval).then(|| format!("Сейчас строже: задачу передал бот с режимом «{}», ниже его режима бот не опускается.", mode_text(&effective).split(':').next().unwrap_or_default())),
        "allowed": allowed,
        "asks": asks,
        "never": never,
        "sites": bot.browser_sites,
        "messengers": messengers,
        "storedRules": stored,
        "alwaysAsk": tools::ALWAYS_ASK_KINDS.iter().map(|(id, label)| json!({ "id": id, "label": label, "on": bot.always_ask.iter().any(|k| k == id) })).collect::<Vec<_>>(),
    })
}
