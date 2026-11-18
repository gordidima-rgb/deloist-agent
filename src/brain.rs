//! `brain.md` — правила от человека в папке бота (с 0.4.3, решение владельца 17.11.2026). Раньше это был
//! раздел «Правила от человека» в конце инструкции бота.
//!
//! Действует одобренная копия, а не файл: она лежит в данных бота (`Bot::brain`), и только её программа
//! кладёт в ход. Файл в папке — её вид для человека. Свою папку бот правит без карточки, в том числе
//! командами, поэтому правка файла мимо «Применить» и ящика бота правил не меняет: ход идёт по одобренной
//! копии, а человек получает одну карточку «brain.md изменён вне карточки: применить?». «Не надо» возвращает
//! файл к одобренному виду.

use std::path::PathBuf;
use std::sync::Arc;

use serde_json::json;

use crate::offers::Offer;
use crate::store::{now, uid, Bot};
use crate::team_changes::{BotChange, Change, Edit, RULES_HEAD};
use crate::App;

pub const FILE: &str = "brain.md";
/// Больше правил в каждом ходе не держим: это постоянный контекст.
pub const LIMIT: usize = 4_000;
const HEAD: &str = "# Правила от человека";

pub fn path(app: &App, bot_id: &str) -> PathBuf {
    app.store.personal_workspace(bot_id).join(FILE)
}

fn rule_text(line: &str) -> Option<&str> {
    let line = line.trim();
    line.strip_prefix("- ").or_else(|| line.strip_prefix("* ")).map(str::trim)
}

fn same(a: &str, b: &str) -> bool {
    a.trim().to_lowercase() == b.trim().to_lowercase()
}

/// Сравнение без разницы в концах строк и пробелах по краям.
pub fn normalize(text: &str) -> String {
    text.replace("\r\n", "\n").lines().map(str::trim_end).collect::<Vec<_>>().join("\n").trim().to_string()
}

/// Текст с добавленным и/или убранным правилом. Остальное, что написал человек, не трогаем.
pub fn with_rule(text: &str, add: &str, remove: &str) -> Result<String, String> {
    let mut lines: Vec<String> = normalize(text).lines().map(str::to_string).collect();
    if !remove.is_empty() {
        let wanted = remove.trim().trim_start_matches("- ").trim();
        let index = lines.iter().position(|l| rule_text(l).is_some_and(|r| r == wanted)).ok_or_else(|| format!("правила «{wanted}» в brain.md нет: передай его текст точно"))?;
        lines.remove(index);
    }
    if !add.is_empty() {
        if lines.iter().any(|l| rule_text(l).is_some_and(|r| same(r, add))) {
            return Err("такое правило уже есть".into());
        }
        if lines.iter().all(|l| l.trim().is_empty()) {
            lines = vec![HEAD.to_string(), String::new()];
        }
        lines.push(format!("- {add}"));
    }
    // Остался один заголовок — правил нет.
    let rest: Vec<&String> = lines.iter().filter(|l| !l.trim().is_empty() && l.trim() != HEAD).collect();
    if rest.is_empty() {
        return Ok(String::new());
    }
    Ok(lines.join("\n"))
}

/// Блок правил в ходе: одобренная копия, сразу после инструкции.
pub fn prompt_block(bot: &Bot) -> Option<String> {
    let text = normalize(&bot.brain);
    if text.is_empty() {
        return None;
    }
    let body = text.strip_prefix(HEAD).map(str::trim_start).unwrap_or(&text);
    Some(format!("# brain.md — правила от человека: следуй им всегда; новое правило добавляй через propose_rule\n{}", crate::service::take_chars(body, LIMIT)))
}

fn file_text(text: &str) -> String {
    let text = normalize(text);
    if text.is_empty() {
        String::new()
    } else {
        format!("{text}\n")
    }
}

fn write_file(app: &App, bot_id: &str, text: &str) -> std::io::Result<()> {
    let path = path(app, bot_id);
    let text = file_text(text);
    if text.is_empty() {
        return match std::fs::remove_file(&path) {
            Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(e),
            _ => Ok(()),
        };
    }
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    std::fs::write(path, text)
}

/// Одобрить правила: копия в данных бота и файл в его папке. Только по слову человека (карточка, ящик бота).
pub async fn set(app: &App, bot_id: &str, text: &str) -> Result<Bot, String> {
    let text = normalize(text);
    if text.chars().count() > LIMIT {
        return Err(format!("brain.md длиннее {LIMIT} знаков: убери устаревшие правила"));
    }
    if text.chars().any(|c| crate::team_proposal::is_bidi_control(c) || (c.is_control() && c != '\n' && c != '\t')) {
        return Err("в правилах есть невидимые знаки".into());
    }
    let bot = app.store.set_bot_brain(bot_id, &text).await.map_err(|e| e.to_string())?.ok_or("бот не найден")?;
    write_file(app, bot_id, &text).map_err(|e| format!("brain.md не записан: {e}"))?;
    Ok(bot)
}

/// Вернуть файл к одобренному виду («Не надо» на карточке о правке вне карточки).
pub fn restore_file(app: &App, bot_id: &str) {
    if let Some(bot) = app.store.bot(bot_id) {
        if let Err(e) = write_file(app, bot_id, &bot.brain) {
            eprintln!("[brain] {}: файл не восстановлен: {e}", bot.name);
        }
    }
}

/// Карточка о правке brain.md вне карточки.
pub fn drift_change(bot: &Bot, file: &str) -> Change {
    Change::BotChange(BotChange {
        host_id: bot.id.clone(),
        host_name: bot.name.clone(),
        target_id: bot.id.clone(),
        target_name: bot.name.clone(),
        role: None,
        instructions: None,
        remove_from: None,
        rule: None,
        brain: Some(Edit { before: normalize(&bot.brain), after: normalize(file) }),
        drift: true,
    })
}

/// Перед ходом: файл совпадает с одобренным? Нет файла — вернуть; другой текст — одна карточка человеку.
/// Ход в любом случае идёт по одобренной копии.
pub fn check(app: &Arc<App>, bot: &Bot) {
    let Some(stored) = app.store.bot(&bot.id) else { return };
    let path = path(app, &stored.id);
    let file = match std::fs::read(&path) {
        Ok(bytes) => String::from_utf8_lossy(&bytes[..bytes.len().min(LIMIT * 8)]).to_string(),
        Err(_) => {
            if !normalize(&stored.brain).is_empty() {
                restore_file(app, &stored.id);
            }
            return;
        }
    };
    if normalize(&file) == normalize(&stored.brain) {
        return;
    }
    let change = drift_change(&stored, &file);
    if app.offers.find_change(&stored.id, &change).is_some() {
        return;
    }
    let offer = Offer {
        id: uid("offer"),
        host_bot_id: stored.id.clone(),
        specialist_id: String::new(),
        reason: String::new(),
        task: String::new(),
        at: now(),
        team: None,
        change: Some(change.clone()),
    };
    app.offers.add(offer.clone());
    app.turns.emit(json!({ "type": "team_proposal", "botId": stored.id, "proposal": change.event_json(&offer) }));
}

/// Раздел «Правила от человека» из инструкции и текст инструкции без него.
pub fn split_persona(persona: &str) -> (String, Vec<String>) {
    let lines: Vec<&str> = persona.lines().collect();
    let Some(start) = lines.iter().position(|l| l.trim() == RULES_HEAD) else {
        return (persona.to_string(), Vec::new());
    };
    let end = lines.iter().skip(start + 1).position(|l| l.trim().is_empty()).map(|i| i + start + 1).unwrap_or(lines.len());
    let rules: Vec<String> = lines[start + 1..end].iter().filter_map(|l| rule_text(l).map(str::to_string)).filter(|r| !r.is_empty()).collect();
    let before = lines[..start].join("\n");
    let after = lines[end..].join("\n");
    let text = [before.trim(), after.trim()].into_iter().filter(|p| !p.is_empty()).collect::<Vec<_>>().join("\n\n");
    (text, rules)
}

/// При первом запуске 0.4.3: правила из инструкций — в brain.md; у кого правила уже есть, а файла нет, — файл.
pub async fn migrate_all(app: &App) {
    for bot in app.store.bots() {
        let (persona, rules) = split_persona(&bot.persona);
        if !rules.is_empty() {
            let mut text = bot.brain.clone();
            for rule in &rules {
                text = with_rule(&text, rule, "").unwrap_or(text);
            }
            if let Err(e) = set(app, &bot.id, &text).await {
                eprintln!("[brain] {}: правила не перенесены: {e}", bot.name);
                continue;
            }
            if let Err(e) = app.store.update_bot(&bot.id, &json!({ "persona": persona })).await {
                eprintln!("[brain] {}: инструкция без правил не записана: {e}", bot.name);
            }
        } else if !normalize(&bot.brain).is_empty() && !path(app, &bot.id).exists() {
            restore_file(app, &bot.id);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rules_are_added_and_removed_without_touching_the_rest() {
        let one = with_rule("", "Отчитывайся Редактору кратко", "").unwrap();
        assert_eq!(one, "# Правила от человека\n\n- Отчитывайся Редактору кратко");
        let two = with_rule(&one, "Не комментируй посты про игры", "").unwrap();
        assert!(with_rule(&two, "отчитывайся редактору кратко", "").is_err(), "дубль");
        let human = format!("{two}\n\nЗаметка человека без маркера");
        let removed = with_rule(&human, "", "Отчитывайся Редактору кратко").unwrap();
        assert_eq!(removed, "# Правила от человека\n\n- Не комментируй посты про игры\n\nЗаметка человека без маркера");
        assert!(with_rule(&removed, "", "нет такого").is_err());
        assert_eq!(with_rule(&one, "", "Отчитывайся Редактору кратко").unwrap(), "", "пустой заголовок не остаётся");
    }

    #[test]
    fn persona_rules_split_off_and_the_prompt_block_uses_the_approved_copy() {
        let (persona, rules) = split_persona("Ты замеряешь посты.\n\nПравила от человека:\n- Первое\n- Второе\n\nХвост инструкции.");
        assert_eq!(persona, "Ты замеряешь посты.\n\nХвост инструкции.");
        assert_eq!(rules, ["Первое", "Второе"]);
        let (same_persona, none) = split_persona("Без правил.");
        assert!(none.is_empty() && same_persona == "Без правил.");
        let bot = Bot { brain: "# Правила от человека\n\n- Первое".into(), ..Bot::default() };
        let block = prompt_block(&bot).unwrap();
        assert!(block.starts_with("# brain.md — правила от человека") && block.ends_with("\n- Первое"), "{block}");
        assert!(prompt_block(&Bot::default()).is_none());
    }
}
