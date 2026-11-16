//! Эпизоды — законченные задачи: что просили, что сделано и каким способом, результат, ссылка
//! на чат и даты. Лежат рядом с фактами в папке памяти, по файлу на задачу:
//! `memory/episodes/2026-11/2026-11-10 Отчёт Ozon.md`. Объём не ограничен, доктор их не чистит.
//!
//! Модели эпизоды приходят блоком «Из памяти» (1–2 похожих) и через `recall` со `scope: "episodes"`.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use crate::memory::one_line;
use crate::sensitive::mask;
use crate::service::take_chars;
use crate::vault;
use crate::App;

pub const DIR: &str = "episodes";
const TITLE_LIMIT: usize = 120;
const FILE_LIMIT: u64 = 256 * 1024;

/// Запись и выдача номеров идут по одному: два конца задачи разом не получат один номер.
static WRITE: Mutex<()> = Mutex::new(());

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Episode {
    /// `e12`: коротко, чтобы модели было легко сослаться.
    pub id: String,
    pub title: String,
    pub body: String,
    /// Откуда задача: `bot:<id>` или `room:<id>`.
    pub thread: String,
    /// Как чат называется для человека.
    pub chat: String,
    pub started: i64,
    pub ended: i64,
    /// Путь файла от папки памяти, через `/`.
    pub file: String,
    /// Эпизод личного чата видит только этот бот, пока человек не пометит его `shared: true`.
    pub shared: bool,
}

impl Episode {
    /// Кто видит эпизод. `viewer` — `bot:<id>`, `room:<id>` или пусто для внешних агентов.
    /// Эпизоды групп общие, эпизоды личного чата — только этому боту.
    pub fn visible_to(&self, viewer: &str) -> bool {
        self.shared || !self.thread.starts_with("bot:") || self.thread == viewer
    }

    /// Дата для строки блока и поиска: день конца задачи.
    pub fn day(&self) -> String {
        day_of(self.ended)
    }
}

fn day_of(ms: i64) -> String {
    chrono::DateTime::<chrono::Utc>::from_timestamp_millis(ms)
        .map(|t| t.with_timezone(&chrono::Local).format("%Y-%m-%d").to_string())
        .unwrap_or_default()
}

fn month_of(ms: i64) -> String {
    chrono::DateTime::<chrono::Utc>::from_timestamp_millis(ms)
        .map(|t| t.with_timezone(&chrono::Local).format("%Y-%m").to_string())
        .unwrap_or_default()
}

pub fn root(app: &App) -> PathBuf {
    app.store.data_dir().join("memory").join(DIR)
}

fn number(id: &str) -> u64 {
    id.trim_start_matches('e').parse().unwrap_or(0)
}

fn render(episode: &Episode) -> String {
    format!(
        "---\nid: {}\ntitle: {}\nthread: {}\nchat: {}\nstarted: {}\nended: {}\n{}---\n{}\n",
        episode.id,
        vault::quoted(&episode.title),
        vault::quoted(&episode.thread),
        vault::quoted(&episode.chat),
        vault::stamp(episode.started),
        vault::stamp(episode.ended),
        if episode.shared { "shared: true\n" } else { "" },
        episode.body.trim()
    )
}

fn parse(text: &str, file: &str) -> Option<Episode> {
    let (props, body) = vault::front_matter(text).ok()?;
    let get = |key: &str| props.get(key).cloned().unwrap_or_default();
    let id = get("id").trim().to_string();
    if !id.starts_with('e') || number(&id) == 0 {
        return None;
    }
    let stem = file.rsplit('/').next().unwrap_or(file).trim_end_matches(".md");
    let title = one_line(&get("title"));
    Some(Episode {
        id,
        title: if title.is_empty() { stem.to_string() } else { title },
        body: body.trim().to_string(),
        thread: get("thread"),
        chat: get("chat"),
        started: vault::parse_stamp(&get("started")).unwrap_or(0),
        ended: vault::parse_stamp(&get("ended")).unwrap_or(0),
        file: file.to_string(),
        shared: matches!(get("shared").trim(), "true" | "yes" | "да"),
    })
}

fn walk(dir: &Path, rel: &str, out: &mut Vec<String>) {
    let Ok(items) = std::fs::read_dir(dir) else { return };
    for item in items.flatten() {
        let name = item.file_name().to_string_lossy().to_string();
        if name.starts_with('.') {
            continue;
        }
        let child = if rel.is_empty() { name.clone() } else { format!("{rel}/{name}") };
        let path = item.path();
        if path.is_dir() {
            walk(&path, &child, out);
        } else if name.to_lowercase().ends_with(".md") && item.metadata().map(|m| m.len() <= FILE_LIMIT).unwrap_or(false) {
            out.push(child);
        }
    }
}

/// Разобранные файлы по размеру и времени изменения: неизменённый файл второй раз не читается.
type Cache = HashMap<PathBuf, HashMap<String, (u64, i64, Option<Episode>)>>;
static CACHE: Mutex<Option<Cache>> = Mutex::new(None);

fn stat(path: &Path) -> Option<(u64, i64)> {
    let meta = std::fs::metadata(path).ok()?;
    let modified = meta.modified().ok()?.duration_since(std::time::UNIX_EPOCH).ok()?.as_millis() as i64;
    Some((meta.len(), modified))
}

fn list_sync(root: &Path) -> Vec<Episode> {
    let mut files = Vec::new();
    walk(root, "", &mut files);
    let mut guard = CACHE.lock().unwrap_or_else(|e| e.into_inner());
    let known = guard.get_or_insert_with(HashMap::new).entry(root.to_path_buf()).or_default();
    let mut fresh: HashMap<String, (u64, i64, Option<Episode>)> = HashMap::new();
    for rel in &files {
        let Some((len, modified)) = stat(&root.join(rel)) else { continue };
        let parsed = match known.get(rel) {
            Some((l, m, episode)) if *l == len && *m == modified => episode.clone(),
            _ => std::fs::read_to_string(root.join(rel)).ok().and_then(|text| parse(&text, &format!("{DIR}/{rel}"))),
        };
        fresh.insert(rel.clone(), (len, modified, parsed));
    }
    *known = fresh;
    let mut seen: HashSet<String> = HashSet::new();
    let mut out: Vec<Episode> = files
        .iter()
        .filter_map(|rel| known.get(rel).and_then(|(_, _, e)| e.clone()))
        // Скопированный руками файл с тем же номером не путает ссылки: первый по пути главнее.
        .filter(|e| seen.insert(e.id.clone()))
        .collect();
    out.sort_by_key(|e| number(&e.id));
    out
}

/// Все эпизоды, старые первыми.
pub async fn list(app: &App) -> Vec<Episode> {
    let root = root(app);
    tokio::task::spawn_blocking(move || list_sync(&root)).await.unwrap_or_default()
}

fn file_for(root: &Path, episode: &Episode) -> String {
    let month = month_of(episode.ended);
    let base = vault::sanitize(&format!("{} {}", day_of(episode.ended), episode.title));
    let mut name = format!("{month}/{base}.md");
    if root.join(&name).exists() {
        name = format!("{month}/{base} ({}).md", episode.id);
    }
    name
}

fn write_sync(root: &Path, mut episode: Episode) -> std::io::Result<Episode> {
    let _lock = WRITE.lock().unwrap_or_else(|e| e.into_inner());
    // Эпизод видят другие боты и `recall`: секреты и чужие контакты в нём не хранятся ни
    // из ленты, ни из пересказа модели — на их месте «[скрыто: …]».
    episode.title = take_chars(&one_line(&mask(&episode.title, true)), TITLE_LIMIT);
    episode.body = mask(&episode.body, true);
    let existing = list_sync(root);
    match existing.iter().find(|e| !episode.id.is_empty() && e.id == episode.id) {
        // Правка: файл остаётся на месте, даже если заголовок поменялся, — ссылки на него целы.
        Some(old) => {
            episode.file = old.file.clone();
        }
        None => {
            let next = existing.iter().map(|e| number(&e.id)).max().unwrap_or(0) + 1;
            episode.id = format!("e{next}");
            episode.file = format!("{DIR}/{}", file_for(root, &episode));
        }
    }
    let rel = episode.file.trim_start_matches(&format!("{DIR}/")).to_string();
    vault::write_atomic(&root.join(rel), &render(&episode))?;
    Ok(episode)
}

/// Записать новый эпизод (без `id`) или переписать существующий по `id`.
pub async fn write(app: &App, episode: Episode) -> std::io::Result<Episode> {
    let root = root(app);
    tokio::task::spawn_blocking(move || write_sync(&root, episode)).await.map_err(std::io::Error::other)?
}

/// Лучшие совпадения по словам: заголовок важнее текста.
pub fn rank<'a>(episodes: &'a [Episode], query: &str) -> Vec<&'a Episode> {
    let docs: Vec<Vec<(&str, f64)>> = episodes.iter().map(|e| vec![(e.title.as_str(), 2.0), (e.body.as_str(), 1.0)]).collect();
    crate::search::rank(&docs, query).into_iter().map(|(i, _)| &episodes[i]).collect()
}

/// Строка эпизода для блока и поиска: номер, дата, заголовок и начало текста.
pub fn line(episode: &Episode, limit: usize) -> String {
    let head = format!("[{}] эпизод {}: {}", episode.id, episode.day(), episode.title);
    let room = limit.saturating_sub(head.chars().count() + 3);
    let body = one_line(&episode.body);
    if room < 20 || body.is_empty() {
        return take_chars(&head, limit);
    }
    let body = if body.chars().count() > room { format!("{}…", take_chars(&body, room - 1)) } else { body };
    format!("{head}\n  {body}")
}

/// Эпизод целиком для `recall` по номеру.
pub fn full(episode: &Episode) -> String {
    let mut text = format!("[{}] эпизод {}: {}", episode.id, episode.day(), episode.title);
    if !episode.chat.is_empty() {
        text.push_str(&format!("\nЧат: {}", episode.chat));
    }
    text.push('\n');
    text.push_str(episode.body.trim());
    text
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("deloist-episodes-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        dir
    }

    #[test]
    fn episodes_get_numbers_files_by_month_and_survive_a_rename() {
        let root = tmp("write");
        let at = 1_762_760_000_000; // 2025-11-10
        let first = write_sync(&root, Episode { title: "Отчёт Ozon: продажи".into(), body: "## Что просили\nотчёт".into(), thread: "bot:b1".into(), started: at, ended: at, ..Episode::default() }).unwrap();
        assert_eq!(first.id, "e1");
        assert!(first.file.starts_with("episodes/2025-11/2025-11-") && first.file.ends_with("Отчёт Ozon продажи.md"), "{}", first.file);
        let second = write_sync(&root, Episode { title: "Отчёт Ozon: продажи".into(), started: at, ended: at, ..Episode::default() }).unwrap();
        assert_eq!(second.id, "e2");
        assert_ne!(second.file, first.file, "одинаковые заголовки не перетирают друг друга");

        let renamed = write_sync(&root, Episode { title: "Недельный отчёт Ozon".into(), ..first.clone() }).unwrap();
        assert_eq!(renamed.file, first.file, "правка по номеру остаётся в своём файле");
        let all = list_sync(&root);
        assert_eq!(all.len(), 2);
        assert_eq!(all[0].title, "Недельный отчёт Ozon");
        assert_eq!(all[0].thread, "bot:b1");
        assert_eq!(all[0].ended, at);
        assert_eq!(rank(&all, "отчёты озон").len(), 2);
        assert!(line(&all[0], 500).starts_with("[e1] эпизод "), "{}", line(&all[0], 500));
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn episodes_hide_secrets_and_contacts_and_stay_with_their_bot() {
        let root = tmp("mask");
        let body = "## Что просили\n- войди в кабинет, пароль: Qwerty123, бухгалтер Оля +7 913 123-45-67";
        let saved = write_sync(&root, Episode { title: "Позвони +7 913 123-45-67".into(), body: body.into(), thread: "bot:b1".into(), ended: 1, ..Episode::default() }).unwrap();
        let text = std::fs::read_to_string(root.join(saved.file.trim_start_matches("episodes/"))).unwrap();
        assert!(!text.contains("Qwerty123") && !text.contains("123-45-67"), "{text}");
        assert!(text.contains("[скрыто: пароль или код]") && text.contains("[скрыто: телефон]"), "{text}");

        let all = list_sync(&root);
        assert!(all[0].visible_to("bot:b1") && !all[0].visible_to("bot:b2") && !all[0].visible_to("room:r1") && !all[0].visible_to(""));
        let room = Episode { thread: "room:r1".into(), ..all[0].clone() };
        assert!(room.visible_to("bot:b2") && room.visible_to(""), "эпизоды групп общие");
        // Человек пометил эпизод общим — его видят все; список перечитывает изменённый файл.
        let path = root.join(saved.file.trim_start_matches("episodes/"));
        std::fs::write(&path, text.replacen("---\n", "---\nshared: true\n", 1)).unwrap();
        assert!(list_sync(&root)[0].visible_to("bot:b2"));
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn a_long_body_is_cut_to_the_line_limit() {
        let episode = Episode { id: "e3".into(), title: "Сверка".into(), body: "слово ".repeat(400), ended: 1, ..Episode::default() };
        let text = line(&episode, 500);
        assert!(text.chars().count() <= 500, "{}", text.chars().count());
        assert!(text.ends_with('…'));
    }
}
