//! Демо-драйвер: работает без ключей и без CLI. Это не модель, а сценарий: он проверяет весь
//! контур (тред, инструменты, подтверждения, воркспейс) до подключения настоящих мозгов.

use std::time::Duration;

use regex::Regex;
use serde_json::{json, Value};
use tokio_util::sync::CancellationToken;

use super::{DriverEvent, Sink, ToolCall};
use crate::store::{uid, Bot};

async fn say(text: &str, cancel: &CancellationToken, sink: Sink<'_>) -> Result<(), String> {
    let parts: Vec<&str> = Regex::new(r"\S+\s*").unwrap().find_iter(text).map(|m| m.as_str()).collect();
    if parts.is_empty() {
        sink(DriverEvent::Text(text.to_string()));
        return Ok(());
    }
    let chunk = (parts.len() as f64 / 24.0).round().max(1.0) as usize;
    for piece in parts.chunks(chunk) {
        if cancel.is_cancelled() {
            return Err("остановлено".into());
        }
        tokio::time::sleep(Duration::from_millis(8)).await;
        sink(DriverEvent::Text(piece.concat()));
    }
    Ok(())
}

fn call(name: &str, args: Value) -> ToolCall {
    ToolCall { id: uid("call"), name: name.into(), args }
}

fn pick(text: &str, english: bool) -> Option<(ToolCall, String)> {
    let say = |ru: &str, en: &str| if english { en.to_string() } else { ru.to_string() };
    let t = text.to_lowercase();
    let has = |pattern: &str| Regex::new(pattern).unwrap().is_match(&t);
    if let Some(url) = Regex::new(r"https?://\S+").unwrap().find(text) {
        return Some((
            call("browser_nav", json!({ "action": "open", "url": url.as_str() })),
            say("Открою эту страницу в своём браузере и прочитаю её.", "I'll open this page in my browser and read it."),
        ));
    }
    if has(r"запомни|в память|заметк|remember|\bnote\b") {
        let short: String = text.chars().take(200).collect();
        return Some((
            call("remember", json!({ "text": short })),
            say("Запишу это в свою память — вернусь к этому в следующий раз.", "I'll put this in my memory and come back to it next time."),
        ));
    }
    if has(r"делег|передай|попроси .*бот|другому боту|delegate|another bot") {
        return Some((
            call(
                "assign_task",
                if english {
                    json!({ "bot": "Editor", "text": "Describe in one paragraph why personal memory matters for a bot." })
                } else {
                    json!({ "bot": "Редактор", "text": "Опиши одним абзацем, почему личная память важна для бота." })
                },
            ),
            say("Это не моя работа — передам Редактору. Он ответит в своём чате.", "That's not my job, so I'll pass it to the Editor. The answer will be in that bot's chat."),
        ));
    }
    if has(r"нарисуй|картинк|изображени|\bdraw\b|\bpicture\b") {
        let prompt: String = text.chars().take(200).collect();
        return Some((
            call("generate_image", json!({ "prompt": prompt })),
            say("Сейчас нарисую картинку — это платный запрос, поэтому сперва спрошу.", "I'll draw it now. It's a paid request, so I'll ask first."),
        ));
    }
    if has(r"сохран|запиши|создай файл|черновик|\bsave\b|\bdraft\b|create a file") {
        return Some((
            call(
                "write_file",
                json!({ "path": "draft.md", "content": format!("# Черновик\n\n{text}\n\n— записано в демо-режиме.\n") }),
            ),
            say("Сейчас положу это в файл draft.md в своей папке.", "I'll put this in draft.md in my folder."),
        ));
    }
    if has(r"запусти|выполни|терминал|\bshell\b|node -v|версию|\brun\b|terminal|command") {
        let probe = if cfg!(windows) { "ver" } else { "uname -a" };
        return Some((
            call("run_command", json!({ "command": probe })),
            say("Проверю систему: эта команда только читает, поэтому спрашивать не буду.", "I'll check the system. This command only reads, so I won't ask."),
        ));
    }
    if has(r"файл|папк|список|что у тебя|\bfiles?\b|folder") {
        return Some((
            call("list_files", json!({ "path": "" })),
            say("Посмотрю, что лежит в моей папке.", "Let me see what's in my folder."),
        ));
    }
    None
}

pub async fn run(bot: &Bot, messages: &[Value], english: bool, cancel: &CancellationToken, sink: Sink<'_>) -> Result<(), String> {
    let last = messages.last();
    let content = last.and_then(|m| m.get("content")).and_then(Value::as_str).unwrap_or_default();

    // Сценарий тихой рутины: человек написал «тихо» — сообщать нечего.
    if last.and_then(|m| m.get("role")).and_then(Value::as_str) == Some("user")
        && Regex::new(r"(?i)\bтихо\b").unwrap().is_match(content)
    {
        return say(crate::turn::QUIET_MARK, cancel, sink).await;
    }

    // Второй заход после инструмента: коротко пересказываем результат.
    if last.and_then(|m| m.get("role")).and_then(Value::as_str) == Some("tool") {
        let result: String = content.chars().take(900).collect();
        let text = if english {
            format!("Done. Here's the result:\n\n{result}\n\nWith a real model I would carry on from here.")
        } else {
            format!("Готово. Вот результат:\n\n{result}\n\nДальше я бы продолжил уже с настоящей моделью.")
        };
        return say(&text, cancel, sink).await;
    }

    if let Some((tool, intro)) = pick(content, english) {
        say(&intro, cancel, &mut *sink).await?;
        sink(DriverEvent::Tool(tool));
        return Ok(());
    }

    let quoted: String = content.chars().take(160).collect();
    let reply = if english {
        format!(
            "Got it: \u{201c}{quoted}\u{201d}.\n\nI'm {}: {}. This is demo mode, so I follow a script and don't think for myself.\n\nAsk me to \u{201c}show files\u{201d}, \u{201c}save to a file\u{201d} or \u{201c}run a command\u{201d} to see how it works.\n\nTo get real answers, connect a model in settings.",
            bot.name, bot.role
        )
    } else {
        format!(
            "Принял: «{quoted}».\n\nБот {}: {}. Сейчас демо-режим, я отвечаю по сценарию, а не думаю сам.\n\nПопроси «покажи файлы», «сохрани в файл» или «запусти команду» и посмотри, как это работает.\n\nЧтобы бот отвечал по-настоящему, подключи модель в настройках.",
            bot.name, bot.role
        )
    };
    say(&reply, cancel, sink).await
}
