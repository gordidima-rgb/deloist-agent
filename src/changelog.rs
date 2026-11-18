// Разбор CHANGELOG.md (и его перевода CHANGELOG.en.md): какие версии вышли после указанной.
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct WhatsNewSection {
    pub version: String,
    pub date: String,
    pub markdown: String,
}

pub const CHANGELOG: &str = include_str!("../CHANGELOG.md");
pub const CHANGELOG_EN: &str = include_str!("../CHANGELOG.en.md");

/// Журнал на языке интерфейса: английский для `en`, иначе русский.
pub fn for_lang(lang: &str) -> &'static str {
    if lang == "en" { CHANGELOG_EN } else { CHANGELOG }
}

pub fn parse(text: &str) -> Vec<WhatsNewSection> {
    let lines: Vec<&str> = text.lines().collect();
    let mut sections = Vec::new();
    let mut i = 0;
    while i < lines.len() {
        if let Some((version, date)) = heading(lines[i]) {
            let mut body: Vec<&str> = Vec::new();
            i += 1;
            while i < lines.len() && !lines[i].trim().starts_with("## ") {
                body.push(lines[i]);
                i += 1;
            }
            // Trim trailing blank lines
            while body.last().map(|l| l.trim().is_empty()).unwrap_or(false) {
                body.pop();
            }
            while body.first().map(|l| l.trim().is_empty()).unwrap_or(false) {
                body.remove(0);
            }
            let markdown = body.join("\n");
            if version != "Unreleased" {
                sections.push(WhatsNewSection { version, date, markdown });
            }
            continue;
        }
        i += 1;
    }
    sections
}

fn heading(line: &str) -> Option<(String, String)> {
    let t = line.trim();
    if !t.starts_with("## [") {
        return None;
    }
    let rest = &t[4..];
    let close = rest.find(']')?;
    let version = rest[..close].trim().to_string();
    let after = rest[close + 1..].trim();
    let date = after
        .trim_start_matches('—')
        .trim_start_matches('-')
        .trim()
        .to_string();
    Some((version, date))
}

fn version_parts(v: &str) -> Vec<u64> {
    v.split(|c: char| !c.is_ascii_digit())
        .filter(|s| !s.is_empty())
        .filter_map(|s| s.parse::<u64>().ok())
        .collect()
}

fn cmp_version(a: &str, b: &str) -> std::cmp::Ordering {
    version_parts(a).cmp(&version_parts(b))
}

pub fn since(text: &str, current: &str, from: &str) -> (String, String, Vec<WhatsNewSection>) {
    let all = parse(text);
    let from_known = !from.trim().is_empty() && all.iter().any(|s| s.version == from.trim());
    let filtered = if from_known {
        let from = from.trim();
        all.into_iter()
            .filter(|s| cmp_version(&s.version, from) == std::cmp::Ordering::Greater)
            .filter(|s| cmp_version(&s.version, current) != std::cmp::Ordering::Greater)
            .collect()
    } else {
        all.into_iter()
            .filter(|s| s.version == current)
            .collect()
    };
    (current.to_string(), from.to_string(), filtered)
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = r#"# Changelog

## [Unreleased]

### Добавлено

- новое

## [0.3.1] — 2026-11-12

### Добавлено

- пункт один
- **жирный**

## [0.3.0] — 2026-11-11

- старый пункт
"#;

    #[test]
    fn sections_between_two_versions() {
        let (current, from, sections) = since(SAMPLE, "0.3.1", "0.3.0");
        assert_eq!(current, "0.3.1");
        assert_eq!(from, "0.3.0");
        assert_eq!(sections.len(), 1);
        assert_eq!(sections[0].version, "0.3.1");
    }

    #[test]
    fn english_changelog_has_the_same_releases() {
        let heads = |text| parse(text).into_iter().map(|s| (s.version, s.date)).collect::<Vec<_>>();
        assert_eq!(heads(CHANGELOG_EN), heads(CHANGELOG), "CHANGELOG.en.md must translate every release of CHANGELOG.md");
        assert_eq!(for_lang("en"), CHANGELOG_EN);
        assert_eq!(for_lang("ru"), CHANGELOG);
    }

    #[test]
    fn unreleased_is_skipped() {
        let all = parse(SAMPLE);
        assert!(all.iter().all(|s| s.version != "Unreleased"));
        assert_eq!(all.len(), 2);
    }

    #[test]
    fn unknown_from_gives_current() {
        let (current, _from, sections) = since(SAMPLE, "0.3.1", "9.9.9");
        assert_eq!(current, "0.3.1");
        assert_eq!(sections.len(), 1);
        assert_eq!(sections[0].version, "0.3.1");
    }

    #[test]
    fn between_two_versions_multiple() {
        // from older than multiple releases
        let text = r#"## [0.3.1] — 2026-11-12

- a

## [0.3.0] — 2026-11-11

- b

## [0.2.0] — 2026-11-10

- c
"#;
        let (_c, _f, sections) = since(text, "0.3.1", "0.2.0");
        assert_eq!(sections.len(), 2);
        assert_eq!(sections[0].version, "0.3.1");
        assert_eq!(sections[1].version, "0.3.0");
    }
}