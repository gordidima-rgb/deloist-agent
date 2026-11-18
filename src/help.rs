//! Справка для ботов: `docs/bots/api.md` — что в программе есть, как человек это видит и какие инструменты у бота.
//! Инструмент `read_help` отдаёт список разделов или один раздел; тест следит, чтобы каждый инструмент был описан.

use serde_json::{json, Value};

const DOC: &str = include_str!("../docs/bots/api.md");

pub fn tool_def() -> Value {
    json!({
        "type": "function",
        "function": {
            "name": "read_help",
            "description": "Справка по программе: что умеет каждая часть (рутины, процессы, цели, задачи, привычки, команда, память, файлы, браузер, скрипты, каталог), какие у неё инструменты, какие поля и когда нужна карточка. Без topic — список разделов. Читай перед тем, как предлагать или менять настройки.",
            "parameters": {
                "type": "object",
                "properties": { "topic": { "type": "string", "description": "Название раздела или его начало, например «Рутины». Без него — список разделов." } },
            },
        },
    })
}

/// Разделы справки: (заголовок, текст вместе с заголовком).
fn sections() -> Vec<(&'static str, &'static str)> {
    let starts: Vec<usize> = DOC.match_indices("\n## ").map(|(i, _)| i + 1).collect();
    starts
        .iter()
        .enumerate()
        .map(|(n, &from)| {
            let to = starts.get(n + 1).map_or(DOC.len(), |&next| next - 1);
            let body = DOC[from..to].trim_end();
            (body.lines().next().unwrap_or_default().trim_start_matches("## ").trim(), body)
        })
        .collect()
}

fn index() -> String {
    let list: Vec<String> = sections()
        .iter()
        .map(|(title, body)| format!("• {title} — {}", body.lines().skip(1).find(|l| !l.trim().is_empty()).unwrap_or_default().chars().take(140).collect::<String>()))
        .collect();
    format!("Разделы справки (read_help с topic):\n{}", list.join("\n"))
}

/// Текст для модели: раздел по началу названия без учёта регистра, без темы или с неизвестной темой — список.
pub fn read(topic: &str) -> String {
    let wanted = topic.trim().to_lowercase();
    if wanted.is_empty() {
        return index();
    }
    match sections().into_iter().find(|(title, _)| title.to_lowercase().starts_with(&wanted)) {
        Some((_, body)) => body.to_string(),
        None => format!("Нет такого раздела: «{}».\n{}", topic.trim(), index()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Инструменты, которые `tool_defs_with_mcp` добавляет по условиям (навыки, картинки, граф): без запущенных сервисов список не собрать.
    const CONDITIONAL: [&str; 4] = ["read_skill", "workspace_graph", "save_skill", "generate_image"];

    #[test]
    fn every_tool_is_documented() {
        let defs = crate::tools::tool_defs();
        let mut names: Vec<&str> = defs.as_array().unwrap().iter().filter_map(|d| d["function"]["name"].as_str()).collect();
        names.extend(CONDITIONAL);
        let missing: Vec<&str> = names.into_iter().filter(|n| !DOC.contains(&format!("`{n}`"))).collect();
        assert!(missing.is_empty(), "нет в docs/bots/api.md: {missing:?}");
    }

    #[test]
    fn every_tool_has_a_label() {
        let source = include_str!("../web/tool-labels.js");
        let defs = crate::tools::tool_defs();
        let mut names: Vec<&str> = defs.as_array().unwrap().iter().filter_map(|d| d["function"]["name"].as_str()).collect();
        names.extend(CONDITIONAL);
        let missing: Vec<&str> = names.into_iter().filter(|n| {
            !source.contains(&format!("\"{n}\"")) && !source.contains(&format!("{n}:"))
        }).collect();
        assert!(missing.is_empty(), "нет подписи для: {missing:?}");
    }

    #[test]
    fn every_section_is_reachable_and_unknown_topics_list_them() {
        let all = sections();
        assert!(all.len() >= 12, "разделов: {}", all.len());
        for (title, body) in &all {
            assert_eq!(&read(title), body);
            assert_eq!(&read(&title.to_uppercase()), body, "регистр не важен");
        }
        let unknown = read("несуществующий");
        assert!(unknown.contains("Нет такого раздела"));
        assert!(all.iter().all(|(title, _)| unknown.contains(title) && index().contains(title)));
    }

    #[test]
    fn the_tool_is_read_only_and_reaches_agents() {
        assert!(crate::tools::is_read_only("read_help"));
        assert!(crate::tools::is_bridge_tool("read_help"));
        assert!(crate::tools::team_tool_defs().iter().any(|d| d["function"]["name"] == "read_help"));
    }
}
