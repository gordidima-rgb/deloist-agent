//! Личная память бота — один файл `memory.md` в его папке (с 0.4.3, решение владельца 17.11.2026:
//! «вся память — файлом»). Это та же книга фактов, что и раньше, только личные факты бота лежат не
//! папкой по файлу на факт, а строками одного текста, который человек правит в ящике бота.
//!
//! Запись — строка `- заголовок`, подробности — следующие строки с отступом. Раздел `## Всегда` бот
//! видит в каждом ходе целиком, остальные записи — первой строкой. Номер, тип, автор и время записи
//! в файле не видны: они лежат в индексе рядом с данными (`memory-files.json`), и запись узнаётся по
//! тексту. Правку файла мимо программы (Проводник, Obsidian, команда агента) программа замечает при
//! следующем чтении и отдаёт памяти как изменения — они ложатся в журнал и отменяются.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use serde::{Deserialize, Serialize};

use super::{clean_kind, Fact, BODY_LIMIT, TITLE_LIMIT};
use crate::service;
use crate::vault::Change;

/// Имя файла памяти в папке бота.
pub const FILE: &str = "memory.md";
/// Заголовок раздела, который идёт в ход целиком.
pub const ALWAYS: &str = "Всегда";
const OTHER: &str = "Записи";
/// Больше этого файл не разбираем: так его не раздуть до остановки программы.
const FILE_LIMIT: u64 = 256 * 1024;

static LOCK: Mutex<()> = Mutex::new(());

/// Бот, у которого память лежит файлом.
#[derive(Clone, Debug)]
pub struct BotFile {
    pub id: String,
    pub name: String,
    /// `<папка бота>/memory.md`.
    pub path: PathBuf,
    /// `<папка бота>/MAIN.md` из 0.4.2: при первом переезде его текст становится записью в разделе «Всегда».
    pub main: PathBuf,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
struct Entry {
    len: u64,
    modified: i64,
    facts: Vec<Fact>,
}

#[derive(Debug, Default, Serialize, Deserialize)]
#[serde(default)]
struct Index {
    bots: HashMap<String, Entry>,
}

/// Итог чтения: факты ботов, которые живут файлом, и правки мимо программы.
#[derive(Debug, Default)]
pub struct Overlay {
    pub facts: Vec<Fact>,
    /// Боты, чью личную память отдаёт файл (их факты из папки памяти больше не нужны).
    pub owned: HashSet<String>,
    pub external: Vec<Change>,
    /// Файл надо записать: переезд, удалённый файл или новый номер.
    pub needs_write: bool,
}

/// Запись, как её видно в тексте.
#[derive(Clone, Debug, PartialEq)]
pub struct Line {
    pub pinned: bool,
    pub title: String,
    pub body: String,
}

fn number(id: &str) -> u64 {
    id.trim_start_matches('m').parse().unwrap_or(0)
}

fn stat(path: &Path) -> Option<(u64, i64)> {
    let meta = std::fs::metadata(path).ok()?;
    let modified = meta.modified().ok()?.duration_since(std::time::UNIX_EPOCH).ok()?.as_millis() as i64;
    Some((meta.len(), modified))
}

fn read_index(path: &Path) -> Index {
    std::fs::read_to_string(path).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default()
}

fn save_index(path: &Path, index: &Index) -> std::io::Result<()> {
    let text = serde_json::to_string(index).map_err(std::io::Error::other)?;
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, text)?;
    std::fs::rename(&tmp, path)
}

// ---------------------------------------------------------------------------
// Текст файла
// ---------------------------------------------------------------------------

/// Разобрать текст. Строка `- …` начинает запись, строка с отступом продолжает её; пустая строка
/// или заголовок запись закрывают. Текст до первого раздела `##` — пояснение, кроме строк `- …`.
pub fn parse(text: &str) -> Vec<Line> {
    let mut out: Vec<Line> = Vec::new();
    let mut section: Option<bool> = None;
    let mut open = false;
    for raw in text.lines() {
        let line = raw.trim_end();
        let trimmed = line.trim_start();
        if trimmed.is_empty() {
            open = false;
            continue;
        }
        if let Some(head) = trimmed.strip_prefix('#') {
            open = false;
            let level = 1 + head.chars().take_while(|c| *c == '#').count();
            if level >= 2 {
                let name = head.trim_start_matches('#').trim();
                section = Some(name.eq_ignore_ascii_case(ALWAYS) || name.to_lowercase() == ALWAYS.to_lowercase());
            }
            continue;
        }
        let indented = line.starts_with("  ") || line.starts_with('\t');
        let item = if indented { None } else { trimmed.strip_prefix("- ").or_else(|| trimmed.strip_prefix("* ")) };
        if let Some(rest) = item {
            out.push(Line { pinned: section.unwrap_or(false), title: rest.trim().to_string(), body: String::new() });
            open = true;
            continue;
        }
        if open {
            let last = out.last_mut().expect("открытая запись");
            let piece = line.strip_prefix("  ").or_else(|| line.strip_prefix('\t')).unwrap_or(line);
            if !last.body.is_empty() {
                last.body.push('\n');
            }
            last.body.push_str(piece.trim_end());
            continue;
        }
        // Свободная строка в разделе — тоже запись: человек мог не поставить «- ».
        if let Some(pinned) = section {
            out.push(Line { pinned, title: trimmed.to_string(), body: String::new() });
            open = true;
        }
    }
    out.into_iter()
        .filter(|l| !l.title.is_empty())
        .map(|mut l| {
            if l.title.chars().count() > TITLE_LIMIT {
                let head = service::take_chars(&l.title, TITLE_LIMIT);
                let tail: String = l.title.chars().skip(TITLE_LIMIT).collect();
                l.body = if l.body.is_empty() { tail.trim().to_string() } else { format!("{}\n{}", tail.trim(), l.body) };
                l.title = head;
            }
            l.body = service::take_chars(l.body.trim(), BODY_LIMIT);
            l
        })
        .collect()
}

fn push_fact(out: &mut String, fact: &Fact) {
    out.push_str("- ");
    out.push_str(fact.title.trim());
    out.push('\n');
    for line in fact.body.lines().map(str::trim_end).filter(|l| !l.trim().is_empty()) {
        out.push_str("  ");
        out.push_str(line);
        out.push('\n');
    }
}

/// Текст файла: пояснение, «Всегда», остальное — в порядке номеров (новое в конце).
pub fn render(name: &str, facts: &[Fact]) -> String {
    let mut sorted: Vec<&Fact> = facts.iter().collect();
    sorted.sort_by_key(|f| number(&f.id));
    let mut out = format!(
        "# Память: {name}\n\nКаждая запись начинается с «- », подробности — строками с отступом в два пробела. \
         Раздел «{ALWAYS}» бот видит в каждом ходе целиком, остальные записи — первой строкой.\n\n## {ALWAYS}\n\n"
    );
    for fact in sorted.iter().filter(|f| f.pinned) {
        push_fact(&mut out, fact);
    }
    out.push_str(&format!("\n## {OTHER}\n\n"));
    for fact in sorted.iter().filter(|f| !f.pinned) {
        push_fact(&mut out, fact);
    }
    out
}

/// Сопоставить записи текста со старыми фактами бота. Та же запись (заголовок и подробности) — тот же
/// факт; тот же заголовок с другими подробностями — правка; остальное — удалено или новое.
/// Новые и правленые записи получают автора `author`.
pub fn merge(lines: &[Line], old: &[Fact], scope: &str, next: &mut u64, author: &str, at: i64) -> (Vec<Fact>, Vec<Change>) {
    let mut used: HashSet<usize> = HashSet::new();
    let mut slots: Vec<Option<usize>> = vec![None; lines.len()];
    let same = |a: &str, b: &str| a.trim() == b.trim();
    for (i, line) in lines.iter().enumerate() {
        if let Some(j) = old.iter().enumerate().position(|(j, f)| !used.contains(&j) && same(&f.title, &line.title) && same(&f.body, &line.body)) {
            used.insert(j);
            slots[i] = Some(j);
        }
    }
    for (i, line) in lines.iter().enumerate() {
        if slots[i].is_some() {
            continue;
        }
        if let Some(j) = old.iter().enumerate().position(|(j, f)| !used.contains(&j) && same(&f.title, &line.title)) {
            used.insert(j);
            slots[i] = Some(j);
        }
    }
    let mut facts = Vec::new();
    let mut changes = Vec::new();
    for (i, line) in lines.iter().enumerate() {
        match slots[i] {
            Some(j) => {
                let before = &old[j];
                let mut fact = before.clone();
                fact.pinned = line.pinned;
                if !same(&before.body, &line.body) {
                    fact.body = line.body.clone();
                    fact.author = author.to_string();
                }
                if fact != *before {
                    fact.updated_at = at.max(before.updated_at + 1);
                    changes.push((Some(before.clone()), Some(fact.clone())));
                }
                facts.push(fact);
            }
            None => {
                *next += 1;
                let fact = Fact {
                    id: format!("m{next}"),
                    scope: scope.to_string(),
                    kind: clean_kind("", scope),
                    title: line.title.clone(),
                    body: line.body.clone(),
                    author: author.to_string(),
                    created_at: at,
                    updated_at: at,
                    pinned: line.pinned,
                    ..Fact::default()
                };
                changes.push((None, Some(fact.clone())));
                facts.push(fact);
            }
        }
    }
    for (j, fact) in old.iter().enumerate() {
        if !used.contains(&j) {
            changes.push((Some(fact.clone()), None));
        }
    }
    // Номера — не позиции: текст мог переставить записи, а порядок памяти — по номерам.
    facts.sort_by_key(|f| number(&f.id));
    (facts, changes)
}

/// Текст MAIN.md из 0.4.2 — одна запись «Всегда»: он и раньше был в каждом ходе.
fn main_fact(text: &str, scope: &str, next: &mut u64, at: i64) -> Option<Fact> {
    let text = text.trim();
    if text.is_empty() {
        return None;
    }
    let mut body = service::take_chars(text, BODY_LIMIT);
    if text.chars().count() > BODY_LIMIT {
        body = service::take_chars(text, BODY_LIMIT.saturating_sub(40));
        body.push_str("\n…полностью — в MAIN.md.old");
    }
    *next += 1;
    Some(Fact {
        id: format!("m{next}"),
        scope: scope.to_string(),
        kind: "project".into(),
        title: "Главное по работе: задачи, критерии, рабочие файлы (было в MAIN.md)".into(),
        body,
        author: "migrated".into(),
        created_at: at,
        updated_at: at,
        pinned: true,
        ..Fact::default()
    })
}

// ---------------------------------------------------------------------------
// Чтение и запись
// ---------------------------------------------------------------------------

/// Прочитать файлы памяти ботов. `vault_facts` — факты, которые ещё лежат в папке памяти: из них
/// переезжает бот, у которого файла памяти ещё нет.
pub fn read(index_path: &Path, bots: &[BotFile], vault_facts: &[Fact], next: &mut u64, at: i64) -> std::io::Result<Overlay> {
    let _lock = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let mut index = read_index(index_path);
    let mut out = Overlay::default();
    let mut index_dirty = false;
    // Номера не повторяются: следующий — после всех известных, и из папки, и из индекса.
    let known = index.bots.values().flat_map(|e| e.facts.iter()).map(|f| number(&f.id)).max().unwrap_or(0);
    *next = (*next).max(known);
    for bot in bots {
        out.owned.insert(bot.id.clone());
        let file = stat(&bot.path);
        match index.bots.get(&bot.id).cloned() {
            None => {
                // Переезд: личные факты из папки памяти, MAIN.md — записью «Всегда», и то, что человек уже положил в memory.md.
                let mut facts: Vec<Fact> = vault_facts.iter().filter(|f| f.scope == bot.id).cloned().collect();
                if let Ok(text) = std::fs::read_to_string(&bot.main) {
                    facts.extend(main_fact(&text, &bot.id, next, at));
                }
                if file.is_some_and(|(len, _)| len <= FILE_LIMIT) {
                    if let Ok(text) = std::fs::read_to_string(&bot.path) {
                        let lines = parse(&text);
                        let fresh: Vec<Line> = lines.into_iter().filter(|l| !facts.iter().any(|f| f.title.trim() == l.title.trim())).collect();
                        let (added, changes) = merge(&fresh, &[], &bot.id, next, "human", at);
                        out.external.extend(changes);
                        facts.extend(added);
                    }
                }
                if !facts.is_empty() || bot.path.exists() {
                    out.needs_write = true;
                }
                out.facts.extend(facts);
            }
            Some(entry) => {
                let Some((len, modified)) = file else {
                    // Файл пропал (удалили руками или командой): память не теряем, файл вернётся при записи.
                    out.needs_write |= !entry.facts.is_empty();
                    out.facts.extend(entry.facts);
                    continue;
                };
                if len == entry.len && modified == entry.modified {
                    out.facts.extend(entry.facts);
                    continue;
                }
                let text = if len > FILE_LIMIT { None } else { std::fs::read_to_string(&bot.path).ok() };
                let Some(text) = text else {
                    eprintln!("[memory] {} не прочитался (больше 256 КБ или не UTF-8): беру прежнюю память", bot.path.display());
                    out.needs_write = true;
                    out.facts.extend(entry.facts);
                    continue;
                };
                let (facts, changes) = merge(&parse(&text), &entry.facts, &bot.id, next, "files", at);
                out.external.extend(changes);
                index.bots.insert(bot.id.clone(), Entry { len, modified, facts: facts.clone() });
                index_dirty = true;
                out.facts.extend(facts);
            }
        }
    }
    // Бота удалили — его запись в индексе больше не нужна.
    let ids: HashSet<&str> = bots.iter().map(|b| b.id.as_str()).collect();
    let before = index.bots.len();
    index.bots.retain(|id, _| ids.contains(id.as_str()));
    if index_dirty || index.bots.len() != before {
        save_index(index_path, &index)?;
    }
    Ok(out)
}

/// Записать файлы памяти ботов: только те, чей текст изменился. `facts` — личные факты этих ботов.
pub fn write(index_path: &Path, bots: &[BotFile], facts: &[Fact]) -> std::io::Result<()> {
    let _lock = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let mut index = read_index(index_path);
    let mut dirty = false;
    for bot in bots {
        let mine: Vec<Fact> = facts.iter().filter(|f| f.scope == bot.id).cloned().collect();
        let first = !index.bots.contains_key(&bot.id);
        if first && mine.is_empty() && !bot.path.exists() {
            continue;
        }
        let text = render(&bot.name, &mine);
        let current = std::fs::read_to_string(&bot.path).ok();
        if current.as_deref() != Some(text.as_str()) {
            if let Some(dir) = bot.path.parent() {
                std::fs::create_dir_all(dir)?;
            }
            let tmp = bot.path.with_extension("md.tmp");
            std::fs::write(&tmp, &text)?;
            std::fs::rename(&tmp, &bot.path)?;
        }
        let (len, modified) = stat(&bot.path).unwrap_or_default();
        let entry = Entry { len, modified, facts: mine };
        let changed = index.bots.get(&bot.id).is_none_or(|e| e.len != entry.len || e.modified != entry.modified || e.facts != entry.facts);
        if changed {
            index.bots.insert(bot.id.clone(), entry);
            dirty = true;
        }
        // MAIN.md переехал в память: оригинал остаётся рядом как .old, в ход он больше не идёт.
        if first && bot.main.is_file() {
            let mut aside = bot.main.clone().into_os_string();
            aside.push(".old");
            let _ = std::fs::rename(&bot.main, PathBuf::from(aside));
        }
    }
    if dirty {
        save_index(index_path, &index)?;
    }
    Ok(())
}

pub async fn read_async(index_path: PathBuf, bots: Vec<BotFile>, vault_facts: Vec<Fact>, mut next: u64, at: i64) -> std::io::Result<(Overlay, u64)> {
    tokio::task::spawn_blocking(move || read(&index_path, &bots, &vault_facts, &mut next, at).map(|o| (o, next)))
        .await
        .map_err(std::io::Error::other)?
}

pub async fn write_async(index_path: PathBuf, bots: Vec<BotFile>, facts: Vec<Fact>) -> std::io::Result<()> {
    tokio::task::spawn_blocking(move || write(&index_path, &bots, &facts)).await.map_err(std::io::Error::other)?
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fact(id: &str, title: &str, body: &str, pinned: bool, author: &str) -> Fact {
        Fact { id: id.into(), scope: "b1".into(), kind: "project".into(), title: title.into(), body: body.into(), author: author.into(), pinned, created_at: 1, updated_at: 1, ..Fact::default() }
    }

    fn bot(dir: &Path) -> BotFile {
        BotFile { id: "b1".into(), name: "Скаут".into(), path: dir.join(FILE), main: dir.join("MAIN.md") }
    }

    #[test]
    fn render_and_parse_round_trip() {
        let facts = vec![fact("m2", "Отчёт по понедельникам", "к 10:00\n- в канал", true, "b1"), fact("m1", "Кабинет Ozon", "", false, "human")];
        let text = render("Скаут", &facts);
        assert!(text.find("## Всегда").unwrap() < text.find("Отчёт по понедельникам").unwrap());
        assert!(text.find("## Записи").unwrap() < text.find("Кабинет Ozon").unwrap());
        let lines = parse(&text);
        assert_eq!(lines, vec![
            Line { pinned: true, title: "Отчёт по понедельникам".into(), body: "к 10:00\n- в канал".into() },
            Line { pinned: false, title: "Кабинет Ozon".into(), body: String::new() },
        ]);
        let mut next = 2;
        let (merged, changes) = merge(&lines, &facts, "b1", &mut next, "files", 5);
        assert!(changes.is_empty(), "тот же текст — без изменений: {changes:?}");
        assert_eq!(merged.iter().map(|f| f.id.as_str()).collect::<Vec<_>>(), ["m1", "m2"]);
        assert_eq!(merged[0].author, "human", "автор не теряется");
    }

    #[test]
    fn human_text_without_markers_and_free_sections() {
        let text = "пояснение не запись\n- но эта до раздела — запись\n\n## Всегда\nЗовут Петя\n\n## Мои заметки\n- сайт x.com\n  кнопка справа\nвторая строка тела\n";
        let lines = parse(text);
        assert_eq!(lines.len(), 3, "{lines:?}");
        assert!(!lines[0].pinned && lines[0].title == "но эта до раздела — запись");
        assert!(lines[1].pinned && lines[1].title == "Зовут Петя");
        assert_eq!((lines[2].pinned, lines[2].body.as_str()), (false, "кнопка справа\nвторая строка тела"));
    }

    #[test]
    fn merge_finds_edits_removals_and_new_lines() {
        let old = vec![fact("m1", "Отчёт", "к 10", false, "b1"), fact("m2", "Старое", "", false, "b1"), fact("m3", "Пин", "", true, "b1")];
        let lines = vec![
            Line { pinned: false, title: "Отчёт".into(), body: "к 11".into() },
            Line { pinned: false, title: "Пин".into(), body: String::new() },
            Line { pinned: true, title: "Новое".into(), body: String::new() },
        ];
        let mut next = 3;
        let (facts, changes) = merge(&lines, &old, "b1", &mut next, "human", 9);
        assert_eq!(facts.iter().map(|f| (f.id.as_str(), f.author.as_str(), f.pinned)).collect::<Vec<_>>(), [("m1", "human", false), ("m3", "b1", false), ("m4", "human", true)]);
        assert_eq!(changes.len(), 4, "правка, откреп, новое, удалённое: {changes:?}");
        assert!(changes.iter().any(|(b, a)| b.as_ref().is_some_and(|f| f.id == "m2") && a.is_none()));
    }

    #[test]
    fn first_read_moves_vault_facts_and_main_md_then_notices_hand_edits() {
        // Рядом со сборкой, а не в %TEMP%: у папки с меткой Low временная папка закрыта на запись.
        let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("target").join("test-tmp").join(crate::store::uid("botfile"));
        std::fs::create_dir_all(&dir).unwrap();
        let index = dir.join("memory-files.json");
        std::fs::write(dir.join("MAIN.md"), "Задача: комментарии в X\nКритерии: x-goal.md\n").unwrap();
        let bots = vec![bot(&dir)];
        let vault = vec![fact("m5", "Кабинет Ozon", "", false, "b1"), Fact { scope: "shared".into(), ..fact("m6", "Чужое", "", false, "b1") }];
        let mut next = 6;
        let over = read(&index, &bots, &vault, &mut next, 10).unwrap();
        assert!(over.needs_write && over.owned.contains("b1"));
        assert_eq!(over.facts.len(), 2, "свой факт и MAIN.md, без общего: {:?}", over.facts);
        let main = over.facts.iter().find(|f| f.author == "migrated").unwrap();
        assert!(main.pinned && main.body.contains("x-goal.md") && main.id == "m7");
        write(&index, &bots, &over.facts).unwrap();
        assert!(!dir.join("MAIN.md").exists() && dir.join("MAIN.md.old").exists());
        let text = std::fs::read_to_string(dir.join(FILE)).unwrap();
        assert!(text.contains("- Кабинет Ozon") && text.contains("Критерии: x-goal.md"), "{text}");

        // Без правок — те же факты, без журнала.
        let again = read(&index, &bots, &[], &mut next, 11).unwrap();
        assert!(again.external.is_empty() && !again.needs_write && again.facts.len() == 2);

        // Правка руками: строка дописана, другая удалена.
        std::thread::sleep(std::time::Duration::from_millis(20));
        std::fs::write(dir.join(FILE), text.replace("- Кабинет Ozon\n", "- Кабинет WB\n")).unwrap();
        let edited = read(&index, &bots, &[], &mut next, 12).unwrap();
        assert_eq!(edited.external.len(), 2, "{:?}", edited.external);
        assert!(edited.facts.iter().any(|f| f.title == "Кабинет WB" && f.author == "files" && f.id == "m8"));

        // Файл удалили — память не пропадает, его надо записать заново.
        std::fs::remove_file(dir.join(FILE)).unwrap();
        let gone = read(&index, &bots, &[], &mut next, 13).unwrap();
        assert!(gone.needs_write && gone.facts.len() == 2);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
