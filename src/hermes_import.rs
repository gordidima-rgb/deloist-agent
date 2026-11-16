//! Перенос ботов из Hermes Agent: профиль становится ботом, `SOUL.md` — его персоной,
//! строки памяти — его фактами.
//!
//! Корень Hermes: `%LOCALAPPDATA%\hermes` на Windows, иначе `~/.hermes`; `HERMES_HOME`
//! перекрывает оба. Сам корень — профиль `default`, остальные лежат в `<root>/profiles/<name>/`.
//! Секреты Hermes (`.env`, `auth.json`) и его базу (`state.db`) не читаем никогда: профиль
//! определяется их соседством, а не содержимым.

use std::collections::HashSet;
use std::path::{Component, Path, PathBuf};
use std::sync::Arc;

use serde::Serialize;
use serde_json::{json, Value};

use crate::store::Bot;
use crate::App;

/// Сколько знаков персоны переносим в бота: остальное в запрос к модели всё равно не влезет.
pub const MAX_PERSONA_CHARS: usize = 12_000;
/// Предел одной записи памяти Hermes.
pub const MAX_MEMORY_ENTRY_CHARS: usize = 2_000;
/// Больше записей с одного профиля не берём.
pub const MAX_MEMORY_ENTRIES: usize = 200;
/// Строка, которая разделяет записи памяти Hermes.
const MEMORY_SEPARATOR: &str = "§";
/// По одному из этих файлов папка считается профилем. `state.db` только проверяем на
/// существование и никогда не открываем.
const PROFILE_MARKERS: [&str; 4] = ["config.yaml", "SOUL.md", "profile.yaml", "state.db"];

/// Профиль для списка в интерфейсе: только счётчики, без содержимого.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Profile {
    /// Имя папки; у корня — `default`.
    pub name: String,
    /// Длина `SOUL.md` в знаках.
    pub persona_chars: usize,
    /// Сколько записей нашлось в `memories/MEMORY.md` и `memories/USER.md`.
    pub memory_entries: usize,
}

/// Профиль вместе с прочитанным текстом: нужен переносу, наружу отдаются только счётчики.
struct ProfileFiles {
    profile: Profile,
    persona: String,
    memories: Vec<String>,
}

/// Корень Hermes на этой машине.
pub fn root() -> Option<PathBuf> {
    if let Some(home) = std::env::var_os("HERMES_HOME").filter(|value| !value.is_empty()) {
        return Some(PathBuf::from(home));
    }
    #[cfg(windows)]
    if let Some(local) = std::env::var_os("LOCALAPPDATA").filter(|value| !value.is_empty()) {
        return Some(PathBuf::from(local).join("hermes"));
    }
    std::env::var_os("HOME")
        .filter(|value| !value.is_empty())
        .map(|home| PathBuf::from(home).join(".hermes"))
}

/// Все профили Hermes под корнем. `found` в ответе API — это непустой список.
pub fn profiles(root: &Path) -> Vec<Profile> {
    scan(root).into_iter().map(|item| item.profile).collect()
}

/// Чистый разбор файла памяти: записи разделены строкой из одного `§`.
/// Каждую запись обрезаем по краям, пустые выбрасываем, длинную режем по знакам.
pub fn parse_memory(text: &str) -> Vec<String> {
    let mut entries = Vec::new();
    let mut current = String::new();
    for line in text.lines() {
        if entries.len() >= MAX_MEMORY_ENTRIES {
            break;
        }
        if line.trim() == MEMORY_SEPARATOR {
            push_memory(&mut entries, &current);
            current.clear();
            continue;
        }
        if !current.is_empty() {
            current.push('\n');
        }
        current.push_str(line);
    }
    if entries.len() < MAX_MEMORY_ENTRIES {
        push_memory(&mut entries, &current);
    }
    entries
}

fn push_memory(entries: &mut Vec<String>, raw: &str) {
    if entries.len() >= MAX_MEMORY_ENTRIES {
        return;
    }
    let entry = raw.trim();
    if entry.is_empty() {
        return;
    }
    entries.push(crate::service::take_chars(entry, MAX_MEMORY_ENTRY_CHARS));
}

fn is_profile(dir: &Path) -> bool {
    PROFILE_MARKERS.iter().any(|name| dir.join(name).exists())
}

/// Только `SOUL.md` и файлы памяти: `.env`, `auth.json` и `state.db` не открываются.
fn read_text(path: &Path) -> String {
    std::fs::read_to_string(path).unwrap_or_default()
}

fn read_memories(dir: &Path) -> Vec<String> {
    let mut entries = Vec::new();
    for name in ["MEMORY.md", "USER.md"] {
        entries.extend(parse_memory(&read_text(&dir.join("memories").join(name))));
        if entries.len() >= MAX_MEMORY_ENTRIES {
            break;
        }
    }
    entries.truncate(MAX_MEMORY_ENTRIES);
    entries
}

fn read_profile(dir: &Path, name: &str) -> ProfileFiles {
    let persona = read_text(&dir.join("SOUL.md"));
    let memories = read_memories(dir);
    ProfileFiles {
        profile: Profile {
            name: name.to_string(),
            persona_chars: persona.chars().count(),
            memory_entries: memories.len(),
        },
        persona,
        memories,
    }
}

/// Корень и его `profiles/<name>/`. Порядок постоянный: `default` первым, остальные по имени.
fn scan(root: &Path) -> Vec<ProfileFiles> {
    let mut found = Vec::new();
    let mut seen = HashSet::new();
    if is_profile(root) {
        found.push(read_profile(root, "default"));
        seen.insert("default".to_string());
    }
    let Ok(entries) = std::fs::read_dir(root.join("profiles")) else {
        return found;
    };
    let mut dirs: Vec<(String, PathBuf)> = entries
        .filter_map(|entry| {
            let entry = entry.ok()?;
            if !entry.file_type().ok()?.is_dir() {
                return None;
            }
            let name = entry.file_name().into_string().ok()?;
            Some((name, entry.path()))
        })
        .collect();
    dirs.sort_by(|left, right| left.0.cmp(&right.0));
    for (name, dir) in dirs {
        if seen.contains(&name) || !is_profile(&dir) {
            continue;
        }
        seen.insert(name.clone());
        found.push(read_profile(&dir, &name));
    }
    found
}

/// Имя профиля — ровно одно обычное имя папки: без `..`, слэшей и абсолютных путей.
fn safe_profile_name(name: &str) -> bool {
    if name.is_empty() || name.contains('/') || name.contains('\\') || name == "." || name == ".." || Path::new(name).is_absolute() {
        return false;
    }
    let mut parts = Path::new(name).components();
    matches!(parts.next(), Some(Component::Normal(_))) && parts.next().is_none()
}

/// Одна запись памяти — один факт бота. Короткую строку кладём в заголовок, длинную или
/// многострочную — ещё и в текст, чтобы `remember` не потерял её конец.
fn memory_fact(entry: &str) -> Value {
    let single_line = !entry.contains('\n');
    if single_line && entry.chars().count() <= crate::memory::TITLE_LIMIT {
        return json!({ "title": entry });
    }
    let title = entry.lines().next().unwrap_or(entry).trim();
    json!({ "title": title, "body": entry })
}

/// Создать по боту на каждый выбранный профиль. Неизвестное или небезопасное имя — ошибка
/// до создания первого бота.
pub async fn import(app: &Arc<App>, names: &[String]) -> Result<Vec<String>, String> {
    if names.is_empty() {
        return Err("Выбери хотя бы один профиль Hermes".into());
    }
    let root = root().ok_or_else(|| "Не нашёл папку Hermes на этом компьютере".to_string())?;
    let found = scan(&root);
    let mut selected: Vec<&ProfileFiles> = Vec::new();
    for name in names {
        if !safe_profile_name(name) {
            return Err(format!("Недопустимое имя профиля Hermes: «{name}»"));
        }
        let profile = found
            .iter()
            .find(|item| item.profile.name == *name)
            .ok_or_else(|| format!("Профиль Hermes «{name}» не найден"))?;
        if !selected.iter().any(|item| item.profile.name == profile.profile.name) {
            selected.push(profile);
        }
    }

    let mut ids = Vec::with_capacity(selected.len());
    for profile in selected {
        let name = if profile.profile.name == "default" {
            format!("Hermes · {}", profile.profile.name)
        } else {
            profile.profile.name.clone()
        };
        let input = json!({
            "name": name,
            "role": "перенесён из Hermes",
            "persona": crate::service::take_chars(&profile.persona, MAX_PERSONA_CHARS),
        });
        let bot: Bot = app
            .store
            .add_bot(&input)
            .await
            .map_err(|error| format!("Не смог создать бота из Hermes: {error}"))?;
        for entry in &profile.memories {
            // Строка с секретом в память не пройдёт: профиль переносится без неё.
            let _ = crate::memory::remember(app, &bot, &memory_fact(entry)).await;
        }
        ids.push(bot.id);
    }
    Ok(ids)
}

#[cfg(test)]
mod tests {
    use std::path::{Path, PathBuf};
    use std::time::{SystemTime, UNIX_EPOCH};

    use super::{parse_memory, profiles, MAX_MEMORY_ENTRIES, MAX_MEMORY_ENTRY_CHARS};

    fn temp_root(label: &str) -> PathBuf {
        let stamp = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos();
        Path::new(env!("CARGO_MANIFEST_DIR")).join("target").join(format!("hermes-import-{label}-{stamp}"))
    }

    fn write(path: &Path, text: &str) {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).unwrap();
        }
        std::fs::write(path, text).unwrap();
    }

    #[test]
    fn memory_parser_splits_trims_and_drops_empty_entries() {
        let text = "\r\n  первая строка  \r\n § \r\n\r\n вторая\r\nстрока \r\n§\r\n   \r\n§\r\n\r\nтретья\r\n";
        assert_eq!(parse_memory(text), vec!["первая строка", "вторая\nстрока", "третья"]);
        assert!(parse_memory("").is_empty());
        assert!(parse_memory("  \n § \n \n").is_empty());
    }

    #[test]
    fn memory_entries_are_capped_by_length_and_count() {
        let long = "я".repeat(MAX_MEMORY_ENTRY_CHARS + 500);
        let parsed = parse_memory(&long);
        assert_eq!(parsed.len(), 1);
        assert_eq!(parsed[0].chars().count(), MAX_MEMORY_ENTRY_CHARS);

        let many = (0..MAX_MEMORY_ENTRIES + 50).map(|index| format!("запись {index}")).collect::<Vec<_>>().join("\n§\n");
        let parsed = parse_memory(&many);
        assert_eq!(parsed.len(), MAX_MEMORY_ENTRIES);
        assert_eq!(parsed[0], "запись 0");
        assert_eq!(parsed[MAX_MEMORY_ENTRIES - 1], format!("запись {}", MAX_MEMORY_ENTRIES - 1));
    }

    #[test]
    fn profiles_find_the_root_and_named_folders() {
        let root = temp_root("profiles");
        write(&root.join("SOUL.md"), "персона по умолчанию");
        write(&root.join("memories").join("MEMORY.md"), "любит кофе\n§\nживёт в Новосибирске");
        write(&root.join("memories").join("USER.md"), "работает над Deloist");
        write(&root.join("profiles").join("work").join("config.yaml"), "name: work\n");
        write(&root.join("profiles").join("work").join("SOUL.md"), "рабочая персона");
        write(&root.join("profiles").join("work").join("memories").join("MEMORY.md"), "ведёт проект");
        // Только state.db: профиль есть, персоны и памяти нет. Читать базу нельзя.
        write(&root.join("profiles").join("state").join("state.db"), "not a database");
        // Секреты рядом, но без файлов-маркеров: это не профиль, и внутрь мы не смотрим.
        write(&root.join("profiles").join("ignored").join(".env"), "TOKEN=sk-lost-17");
        write(&root.join("profiles").join("ignored").join("auth.json"), "{\"token\":\"sk-lost-17\"}");
        write(&root.join("profiles").join("not-a-profile").join("readme.txt"), "пусто");

        let found = profiles(&root);
        let names: Vec<&str> = found.iter().map(|profile| profile.name.as_str()).collect();
        assert_eq!(names, vec!["default", "state", "work"]);

        assert_eq!(found[0].persona_chars, "персона по умолчанию".chars().count());
        assert_eq!(found[0].memory_entries, 3);
        assert_eq!(found[1].persona_chars, 0);
        assert_eq!(found[1].memory_entries, 0);
        assert_eq!(found[2].persona_chars, "рабочая персона".chars().count());
        assert_eq!(found[2].memory_entries, 1);

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn missing_root_has_no_profiles() {
        let root = temp_root("missing");
        assert!(profiles(&root).is_empty());
    }
}
