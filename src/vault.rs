//! Память на диске — папка Markdown, которую можно открыть в Obsidian и править руками.
//!
//! Факт — файл `<заголовок>.md` со свойствами в шапке (номер, тип, сущности, кто и когда записал)
//! и разделом «Связи» из ссылок `[[…]]`. Общая память лежит в `shared/`, личная бота — в
//! `bots/<имя бота>/`. Программа держит рядом индекс (`memory-index.json` вне папки): размер и время
//! каждого файла и разобранный факт, поэтому неизменённый файл второй раз не читается.
//!
//! Правки руками программа замечает при следующем чтении и отдаёт памяти как изменения «мимо
//! программы» — они ложатся в журнал и отменяются. Файл, который не разобрался, уезжает в
//! `_unparsed/` с причиной в индексе; память от этого не ломается.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::UNIX_EPOCH;

use serde::{Deserialize, Serialize};

use crate::memory::{clean_kind, normalize_entities, one_line, Fact, SHARED};

pub const SHARED_DIR: &str = "shared";
pub const BOTS_DIR: &str = "bots";
pub const UNPARSED_DIR: &str = "_unparsed";
const LINKS_HEADING: &str = "## Связи";
const STEM_LIMIT: usize = 80;
const FILE_LIMIT: u64 = 64 * 1024;
const UNPARSED_KEEP: usize = 50;

/// Чтение и запись папки идут по одному: иначе два чтения разом оба заметили бы одну правку руками.
static VAULT: Mutex<()> = Mutex::new(());

#[derive(Clone, Debug)]
pub struct BotRef {
    pub id: String,
    pub name: String,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", default)]
struct Entry {
    len: u64,
    modified: i64,
    fact: Fact,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", default)]
pub struct Unparsed {
    /// Где файл теперь, относительно папки памяти.
    pub file: String,
    /// Где он лежал.
    pub from: String,
    pub reason: String,
    pub at: i64,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
struct Index {
    next: u64,
    /// Путь файла (через `/`, от папки памяти) → что в нём было при последнем чтении или записи.
    files: HashMap<String, Entry>,
    unparsed: Vec<Unparsed>,
}

/// Изменение одного факта: `None` — факта не было или не стало.
pub type Change = (Option<Fact>, Option<Fact>);

#[derive(Debug, Default)]
pub struct Loaded {
    pub next: u64,
    pub facts: Vec<Fact>,
    /// Изменения мимо программы: правка, переименование, удаление или новый файл руками.
    pub external: Vec<Change>,
}

fn number(id: &str) -> u64 {
    id.trim_start_matches('m').parse().unwrap_or(0)
}

fn now() -> i64 {
    crate::store::now()
}

// ---------------------------------------------------------------------------
// Имена файлов и папок
// ---------------------------------------------------------------------------

/// Имя файла или папки из заголовка: без знаков, запрещённых Windows и ломающих ссылки Obsidian.
pub fn sanitize(name: &str) -> String {
    let cleaned: String = name.chars().map(|c| if c.is_control() || "<>:\"/\\|?*#^[]".contains(c) { ' ' } else { c }).collect();
    let mut out = one_line(&cleaned);
    if out.chars().count() > STEM_LIMIT {
        out = out.chars().take(STEM_LIMIT).collect();
    }
    let out = out.trim_end_matches(['.', ' ']).trim_start_matches('.').trim().to_string();
    let upper = out.to_uppercase();
    let reserved = ["CON", "PRN", "AUX", "NUL"].contains(&upper.as_str())
        || ((upper.starts_with("COM") || upper.starts_with("LPT")) && upper.len() == 4 && upper.as_bytes()[3].is_ascii_digit());
    if reserved {
        format!("{out}_")
    } else {
        out
    }
}

/// Папка каждого бота: его имя, а у одинаковых или пустых имён — с номером бота.
fn bot_folders(bots: &[BotRef]) -> HashMap<String, String> {
    let mut taken: HashSet<String> = HashSet::new();
    let mut out = HashMap::new();
    for bot in bots {
        let base = sanitize(&bot.name);
        let name = if base.is_empty() || taken.contains(&base.to_lowercase()) {
            if base.is_empty() { sanitize(&bot.id) } else { format!("{base} ({})", sanitize(&bot.id)) }
        } else {
            base
        };
        taken.insert(name.to_lowercase());
        out.insert(bot.id.clone(), name);
    }
    out
}

fn folder_of(scope: &str, folders: &HashMap<String, String>) -> String {
    if scope == SHARED {
        SHARED_DIR.to_string()
    } else {
        format!("{BOTS_DIR}/{}", folders.get(scope).cloned().unwrap_or_else(|| sanitize(scope)))
    }
}

/// Где должен лежать каждый факт. Одинаковые заголовки в одной папке — с номером в имени.
fn plan_paths(facts: &[Fact], bots: &[BotRef]) -> HashMap<String, String> {
    let folders = bot_folders(bots);
    let mut ordered: Vec<&Fact> = facts.iter().collect();
    ordered.sort_by_key(|f| number(&f.id));
    let mut taken: HashSet<String> = HashSet::new();
    let mut out = HashMap::new();
    for fact in ordered {
        let folder = folder_of(&fact.scope, &folders);
        let base = sanitize(&fact.title);
        let mut stem = if base.is_empty() { fact.id.clone() } else { base };
        if taken.contains(&format!("{folder}/{stem}").to_lowercase()) {
            stem = format!("{stem} ({})", fact.id);
        }
        let path = format!("{folder}/{stem}.md");
        taken.insert(format!("{folder}/{stem}").to_lowercase());
        out.insert(fact.id.clone(), path);
    }
    out
}

fn stem_of(path: &str) -> &str {
    let name = path.rsplit('/').next().unwrap_or(path);
    name.strip_suffix(".md").unwrap_or(name)
}

/// Чем ссылаться на файл: просто именем, если оно в папке одно, иначе путём.
fn link_targets(paths: &HashMap<String, String>) -> HashMap<String, String> {
    let mut count: HashMap<String, usize> = HashMap::new();
    for path in paths.values() {
        *count.entry(stem_of(path).to_lowercase()).or_default() += 1;
    }
    paths
        .iter()
        .map(|(id, path)| {
            let stem = stem_of(path);
            let target = if count.get(&stem.to_lowercase()) == Some(&1) { stem.to_string() } else { path.trim_end_matches(".md").to_string() };
            (id.clone(), target)
        })
        .collect()
}

// ---------------------------------------------------------------------------
// Файл факта: запись и разбор
// ---------------------------------------------------------------------------

pub(crate) fn stamp(ms: i64) -> String {
    chrono::DateTime::<chrono::Utc>::from_timestamp_millis(ms)
        .map(|t| t.to_rfc3339_opts(chrono::SecondsFormat::Millis, true))
        .unwrap_or_default()
}

pub(crate) fn parse_stamp(text: &str) -> Option<i64> {
    chrono::DateTime::parse_from_rfc3339(text.trim()).ok().map(|t| t.timestamp_millis())
}

pub(crate) fn quoted(text: &str) -> String {
    serde_json::to_string(text).unwrap_or_default()
}

/// Текст файла факта. `targets` — чем ссылаться на другие факты (`link_targets`).
fn render(fact: &Fact, targets: &HashMap<String, String>) -> String {
    let entities = serde_json::to_string(&fact.entities).unwrap_or_else(|_| "[]".into());
    let mut out = format!(
        "---\nid: {}\ntitle: {}\nkind: {}\nscope: {}\nentities: {}\nauthor: {}\ncreated: {}\nupdated: {}\n---\n",
        fact.id,
        quoted(&fact.title),
        fact.kind,
        quoted(&fact.scope),
        entities,
        quoted(&fact.author),
        stamp(fact.created_at),
        stamp(fact.updated_at),
    );
    if fact.pinned {
        // Только у закреплённых: файлы остальных фактов не меняются.
        out = out.replacen("
---
", "
pinned: true
---
", 1);
    }
    if !fact.body.trim().is_empty() {
        out.push('\n');
        out.push_str(fact.body.trim());
        out.push('\n');
    }
    let links: Vec<String> = fact.links.iter().filter_map(|id| targets.get(id)).map(|t| format!("- [[{t}]]")).collect();
    if !links.is_empty() {
        out.push_str(&format!("\n{LINKS_HEADING}\n\n{}\n", links.join("\n")));
    }
    out
}

#[derive(Debug, Clone, PartialEq)]
enum Prop {
    One(String),
    Many(Vec<String>),
}

impl Prop {
    fn text(&self) -> String {
        match self {
            Prop::One(text) => text.clone(),
            Prop::Many(list) => list.join(", "),
        }
    }

    fn list(&self) -> Vec<String> {
        match self {
            Prop::One(text) if text.is_empty() => Vec::new(),
            Prop::One(text) => vec![text.clone()],
            Prop::Many(list) => list.clone(),
        }
    }
}

fn unquote(raw: &str) -> String {
    let raw = raw.trim();
    if raw.starts_with('"') {
        if let Ok(text) = serde_json::from_str::<String>(raw) {
            return text;
        }
        return raw.trim_matches('"').to_string();
    }
    if raw.len() >= 2 && raw.starts_with('\'') && raw.ends_with('\'') {
        return raw[1..raw.len() - 1].replace("''", "'");
    }
    if raw == "~" || raw == "null" {
        return String::new();
    }
    raw.to_string()
}

/// Значение свойства: JSON-строка, строка в кавычках YAML, голая строка или список `[a, b]`.
fn prop_value(raw: &str) -> Prop {
    let raw = raw.trim();
    if raw.starts_with('[') {
        if let Ok(list) = serde_json::from_str::<Vec<String>>(raw) {
            return Prop::Many(list);
        }
        let inner = raw.trim_start_matches('[').trim_end_matches(']');
        return Prop::Many(inner.split(',').map(unquote).filter(|s| !s.is_empty()).collect());
    }
    Prop::One(unquote(raw))
}

/// Шапка YAML в том виде, в каком её пишем мы и переписывает Obsidian: `ключ: значение`
/// и списки строками `  - элемент`. Ошибка — шапка открыта, но не закрыта.
/// Шапка свойств как текст и тело файла — для соседних файлов памяти (эпизоды).
pub fn front_matter(text: &str) -> Result<(HashMap<String, String>, String), String> {
    let (props, body) = split_front(text)?;
    Ok((props.into_iter().map(|(k, v)| (k, v.text())).collect(), body))
}

fn split_front(text: &str) -> Result<(HashMap<String, Prop>, String), String> {
    let text = text.trim_start_matches('\u{feff}').replace("\r\n", "\n");
    let Some(rest) = text.strip_prefix("---\n") else {
        return Ok((HashMap::new(), text));
    };
    let Some(end) = rest.find("\n---\n").map(|i| (i, i + 5)).or_else(|| rest.strip_suffix("\n---").map(|r| (r.len(), rest.len()))) else {
        return Err("шапка свойств не закрыта строкой ---".into());
    };
    let (head, body) = (&rest[..end.0], &rest[end.1..]);
    let mut props: HashMap<String, Prop> = HashMap::new();
    let mut open: Option<String> = None;
    for line in head.lines() {
        if line.trim().is_empty() || line.trim_start().starts_with('#') {
            continue;
        }
        let item = line.trim_start();
        if line.starts_with([' ', '\t']) || item.starts_with("- ") || item == "-" {
            if let (Some(key), Some(value)) = (&open, item.strip_prefix('-')) {
                let value = unquote(value);
                match props.entry(key.clone()).or_insert_with(|| Prop::Many(Vec::new())) {
                    Prop::Many(list) => list.push(value),
                    other => *other = Prop::Many(vec![value]),
                }
            }
            continue;
        }
        let Some((key, value)) = line.split_once(':') else {
            return Err(format!("строка шапки без «:» — {}", line.trim()));
        };
        let key = key.trim().to_string();
        if value.trim().is_empty() {
            open = Some(key.clone());
            props.insert(key, Prop::Many(Vec::new()));
        } else {
            open = None;
            props.insert(key, prop_value(value));
        }
    }
    Ok((props, body.to_string()))
}

/// Ссылки `[[цель|подпись]]` и `[[цель#раздел]]` → цель.
fn wikilinks(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut rest = text;
    while let Some(start) = rest.find("[[") {
        let after = &rest[start + 2..];
        let Some(end) = after.find("]]") else { break };
        let target = after[..end].split(['|', '#']).next().unwrap_or_default().trim();
        if !target.is_empty() && !out.iter().any(|t: &String| t == target) {
            out.push(target.to_string());
        }
        rest = &after[end + 2..];
    }
    out
}

/// Тело без раздела «Связи», который пишет программа. Раздел — только если в нём одни ссылки.
fn split_links(body: &str) -> (String, Vec<String>) {
    if let Some(at) = body.rfind(&format!("\n{LINKS_HEADING}\n")).or_else(|| body.starts_with(&format!("{LINKS_HEADING}\n")).then_some(0)) {
        let tail = &body[at..];
        let lines: Vec<&str> = tail.lines().map(str::trim).filter(|l| !l.is_empty() && *l != LINKS_HEADING).collect();
        if lines.iter().all(|l| l.starts_with('-') && l.contains("[[")) {
            let text = body[..at].trim().to_string();
            let mut links = wikilinks(tail);
            for link in wikilinks(&text) {
                if !links.contains(&link) {
                    links.push(link);
                }
            }
            return (text, links);
        }
    }
    let text = body.trim().to_string();
    let links = wikilinks(&text);
    (text, links)
}

/// Разобранный файл: факт без связей (их разрешаем, когда известны все файлы) и цели ссылок.
#[derive(Debug, Clone)]
struct Parsed {
    fact: Fact,
    /// Номера в шапке не было: факт создан руками.
    had_id: bool,
    /// `scope` из шапки — на случай, если папку бота не узнали по имени.
    scope_hint: String,
    targets: Vec<String>,
    /// Были ли в шапке дата создания и автор: без них правленый факт берёт прежние.
    has_created: bool,
    has_author: bool,
}

/// Файл на диске показывает факт как надо: тот же номер и те же ссылки в разделе «Связи».
fn file_matches(text: &str, fact: &Fact, rendered: &str) -> bool {
    let Ok((props, body)) = split_front(text) else { return false };
    let id = props.get("id").map(Prop::text).unwrap_or_default();
    let shown = split_links(&body).1;
    let wanted = split_front(rendered).map(|(_, b)| split_links(&b).1).unwrap_or_default();
    id.trim() == fact.id && shown == wanted
}

fn parse(text: &str, stem: &str, modified: i64) -> Result<Parsed, String> {
    let (props, body) = split_front(text)?;
    let get = |key: &str| props.get(key).map(Prop::text).unwrap_or_default();
    let id = get("id").trim().trim_matches(['[', ']']).to_string();
    let had_id = id.starts_with('m') && number(&id) > 0;
    // Заголовок — из шапки; если файл переименовали руками, имя файла главнее.
    let title_prop = one_line(&get("title"));
    let title = if title_prop.is_empty() {
        stem.to_string()
    } else {
        let expected = sanitize(&title_prop);
        if stem == expected || stem == format!("{expected} ({id})") || (expected.is_empty() && stem == id) {
            title_prop
        } else {
            stem.to_string()
        }
    };
    let (body, targets) = split_links(&body);
    let created = parse_stamp(&get("created")).unwrap_or(modified);
    let updated = parse_stamp(&get("updated")).unwrap_or(modified);
    let author = get("author");
    Ok(Parsed {
        fact: Fact {
            id: if had_id { id } else { String::new() },
            scope: String::new(),
            kind: get("kind"),
            title,
            body,
            entities: normalize_entities(&props.get("entities").map(Prop::list).unwrap_or_default()),
            links: Vec::new(),
            author: if author.is_empty() { "human".into() } else { author },
            created_at: created,
            updated_at: updated,
            pinned: get("pinned").trim() == "true",
        },
        had_id,
        scope_hint: get("scope"),
        targets,
        has_created: props.contains_key("created"),
        has_author: !get("author").is_empty(),
    })
}

// ---------------------------------------------------------------------------
// Диск
// ---------------------------------------------------------------------------

fn stat(path: &Path) -> Option<(u64, i64)> {
    let meta = std::fs::metadata(path).ok()?;
    let modified = meta.modified().ok()?.duration_since(UNIX_EPOCH).ok()?.as_millis() as i64;
    Some((meta.len(), modified))
}

fn rel_to_path(root: &Path, rel: &str) -> PathBuf {
    rel.split('/').fold(root.to_path_buf(), |path, part| path.join(part))
}

/// Все `.md` в `shared/` и `bots/`, кроме служебных файлов, начинающихся с точки.
fn walk(root: &Path) -> Vec<String> {
    fn visit(dir: &Path, rel: &str, out: &mut Vec<String>) {
        let Ok(items) = std::fs::read_dir(dir) else { return };
        for item in items.flatten() {
            let name = item.file_name().to_string_lossy().to_string();
            if name.starts_with('.') {
                continue;
            }
            let path = item.path();
            let child = format!("{rel}/{name}");
            if path.is_dir() {
                visit(&path, &child, out);
            } else if name.to_lowercase().ends_with(".md") {
                out.push(child);
            }
        }
    }
    let mut out = Vec::new();
    for top in [SHARED_DIR, BOTS_DIR] {
        visit(&root.join(top), top, &mut out);
    }
    out.sort();
    out
}

fn read_index(path: &Path) -> Option<Index> {
    let text = std::fs::read_to_string(path).ok()?;
    serde_json::from_str(&text).ok()
}

pub(crate) fn write_atomic(path: &Path, text: &str) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let name = path.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
    let tmp = path.with_file_name(format!(".{name}.tmp"));
    std::fs::write(&tmp, text)?;
    std::fs::rename(&tmp, path)
}

fn save_index(path: &Path, index: &Index) -> std::io::Result<()> {
    let text = serde_json::to_string(index).map_err(std::io::Error::other)?;
    write_atomic(path, &text)
}

/// Убрать файл в `_unparsed/` и записать причину в индекс.
fn set_aside(root: &Path, index: &mut Index, rel: &str, reason: &str) {
    let from = rel_to_path(root, rel);
    let name = rel.rsplit('/').next().unwrap_or(rel).to_string();
    let stem = name.strip_suffix(".md").unwrap_or(&name).to_string();
    let _ = std::fs::create_dir_all(root.join(UNPARSED_DIR));
    let mut target = format!("{UNPARSED_DIR}/{name}");
    let mut n = 2;
    while rel_to_path(root, &target).exists() {
        target = format!("{UNPARSED_DIR}/{stem} ({n}).md");
        n += 1;
    }
    if std::fs::rename(&from, rel_to_path(root, &target)).is_err() {
        return;
    }
    index.files.remove(rel);
    index.unparsed.push(Unparsed { file: target, from: rel.to_string(), reason: reason.to_string(), at: now() });
    if index.unparsed.len() > UNPARSED_KEEP {
        let extra = index.unparsed.len() - UNPARSED_KEEP;
        index.unparsed.drain(..extra);
    }
}

/// Чья память по пути файла: `shared/…` — общая, `bots/<папка>/…` — бота.
fn scope_of(rel: &str, hint: &str, bots: &[BotRef], folders: &HashMap<String, String>) -> Result<String, String> {
    let parts: Vec<&str> = rel.split('/').collect();
    if parts.first() == Some(&SHARED_DIR) {
        return Ok(SHARED.to_string());
    }
    if parts.len() < 3 {
        return Err("файл лежит прямо в bots/, а не в папке бота".into());
    }
    let folder = parts[1];
    if let Some((id, _)) = folders.iter().find(|(_, name)| name.to_lowercase() == folder.to_lowercase()) {
        return Ok(id.clone());
    }
    let hint = hint.trim();
    if !hint.is_empty() && (hint == folder || sanitize(hint) == folder || bots.iter().any(|b| b.id == hint)) {
        return Ok(hint.to_string());
    }
    Err(format!("нет бота с папкой «{folder}»"))
}

/// Связи по ссылкам: сначала путь, потом имя файла, если оно одно на всю память.
fn resolve(targets: &[String], paths: &HashMap<String, String>) -> Vec<String> {
    let mut by_path: HashMap<String, String> = HashMap::new();
    let mut by_stem: HashMap<String, Vec<String>> = HashMap::new();
    for (rel, id) in paths {
        by_path.insert(rel.trim_end_matches(".md").to_lowercase(), id.clone());
        by_stem.entry(stem_of(rel).to_lowercase()).or_default().push(id.clone());
    }
    let mut out: Vec<String> = Vec::new();
    for target in targets {
        let key = target.trim_end_matches(".md").to_lowercase();
        let id = by_path.get(&key).cloned().or_else(|| by_stem.get(&key).filter(|ids| ids.len() == 1).map(|ids| ids[0].clone()));
        if let Some(id) = id {
            if !out.contains(&id) {
                out.push(id);
            }
        }
    }
    out
}

/// Есть ли уже память в виде папки (или её индекс).
pub fn exists(root: &Path, index_path: &Path) -> bool {
    index_path.exists() || root.join(SHARED_DIR).is_dir() || root.join(BOTS_DIR).is_dir()
}

/// Прочитать папку. `None` — папки ещё нет: память надо перенести из старого вида.
pub fn read_sync(root: &Path, index_path: &Path, bots: &[BotRef]) -> std::io::Result<Option<Loaded>> {
    let _lock = VAULT.lock().unwrap_or_else(|e| e.into_inner());
    if !exists(root, index_path) {
        return Ok(None);
    }
    let previous = read_index(index_path);
    let had_index = previous.is_some();
    let mut index = previous.unwrap_or_default();
    let folders = bot_folders(bots);
    let before: HashMap<String, Fact> = index.files.values().map(|e| (e.fact.id.clone(), e.fact.clone())).collect();

    // Файлы, которые не менялись с прошлого раза, берём из индекса, остальные разбираем.
    let mut kept: Vec<(String, Fact)> = Vec::new();
    let mut parsed: Vec<(String, Parsed, (u64, i64))> = Vec::new();
    let mut touched = false;
    for rel in walk(root) {
        let path = rel_to_path(root, &rel);
        let Some(meta) = stat(&path) else { continue };
        if let Some(entry) = index.files.get(&rel).filter(|e| e.len == meta.0 && e.modified == meta.1) {
            kept.push((rel, entry.fact.clone()));
            continue;
        }
        touched = true;
        if meta.0 > FILE_LIMIT {
            set_aside(root, &mut index, &rel, "файл больше 64 КБ");
            continue;
        }
        let text = match std::fs::read(&path).map(String::from_utf8) {
            Ok(Ok(text)) => text,
            Ok(Err(_)) => {
                set_aside(root, &mut index, &rel, "файл не в UTF-8");
                continue;
            }
            Err(_) => continue,
        };
        match parse(&text, stem_of(&rel), meta.1).and_then(|p| scope_of(&rel, &p.scope_hint, bots, &folders).map(|scope| (p, scope))) {
            Ok((mut p, scope)) => {
                p.fact.kind = clean_kind(&p.fact.kind, &scope);
                p.fact.scope = scope;
                parsed.push((rel, p, meta));
            }
            Err(reason) => set_aside(root, &mut index, &rel, &reason),
        }
    }
    let gone = index.files.keys().any(|rel| !rel_to_path(root, rel).exists());
    if !touched && !gone {
        let mut facts: Vec<Fact> = kept.into_iter().map(|(_, f)| f).collect();
        facts.sort_by_key(|f| number(&f.id));
        let next = index.next.max(facts.iter().map(|f| number(&f.id)).max().unwrap_or(0));
        return Ok(Some(Loaded { next, facts, external: Vec::new() }));
    }

    // Номера: у неизменённых файлов свои; у новых, скопированных (номер повторился) — следующие.
    let mut next = index.next.max(before.keys().map(|id| number(id)).max().unwrap_or(0));
    let mut used: HashSet<String> = kept.iter().map(|(_, f)| f.id.clone()).collect();
    let mut renumbered: HashSet<String> = HashSet::new();
    for (rel, p, _) in &mut parsed {
        next = next.max(number(&p.fact.id));
        if !p.had_id || used.contains(&p.fact.id) {
            p.fact.id = String::new();
            renumbered.insert(rel.clone());
        } else {
            used.insert(p.fact.id.clone());
        }
    }
    for (rel, p, _) in &mut parsed {
        if p.fact.id.is_empty() {
            next += 1;
            p.fact.id = format!("m{next}");
            debug_assert!(renumbered.contains(rel));
        }
    }

    let paths: HashMap<String, String> = kept.iter().map(|(rel, f)| (rel.clone(), f.id.clone())).chain(parsed.iter().map(|(rel, p, _)| (rel.clone(), p.fact.id.clone()))).collect();
    let mut files: HashMap<String, Entry> = HashMap::new();
    for (rel, fact) in kept {
        let meta = index.files.get(&rel).map(|e| (e.len, e.modified)).unwrap_or_default();
        files.insert(rel, Entry { len: meta.0, modified: meta.1, fact });
    }
    for (rel, mut p, meta) in parsed {
        p.fact.links = resolve(&p.targets, &paths).into_iter().filter(|id| *id != p.fact.id).collect();
        if let Some(old) = before.get(&p.fact.id).filter(|_| !renumbered.contains(&rel)) {
            if !p.has_created {
                p.fact.created_at = old.created_at;
            }
            if !p.has_author {
                p.fact.author = old.author.clone();
            }
            // Правка руками без смены даты в шапке: дата — время файла, иначе доктор её не заметит.
            if *old != p.fact && p.fact.updated_at <= old.updated_at {
                p.fact.updated_at = meta.1.max(old.updated_at + 1);
            }
        }
        files.insert(rel, Entry { len: meta.0, modified: meta.1, fact: p.fact });
    }

    let mut facts: Vec<Fact> = files.values().map(|e| e.fact.clone()).collect();
    facts.sort_by_key(|f| number(&f.id));
    let mut external = Vec::new();
    if had_index {
        let after: HashMap<String, Fact> = facts.iter().map(|f| (f.id.clone(), f.clone())).collect();
        let mut ids: Vec<String> = before.keys().chain(after.keys()).cloned().collect::<HashSet<_>>().into_iter().collect();
        ids.sort_by_key(|id| number(id));
        for id in ids {
            let change = (before.get(&id).cloned(), after.get(&id).cloned());
            if change.0 != change.1 {
                external.push(change);
            }
        }
    }
    index.files = files;
    index.next = next;
    save_index(index_path, &index)?;
    // Новому или скопированному файлу нужен свой номер в шапке — дописываем, остальное не трогаем.
    if !renumbered.is_empty() {
        write_locked(root, index_path, &facts, next, bots)?;
    }
    Ok(Some(Loaded { next, facts, external }))
}

/// Записать память: только факты, которые изменились или переехали; удалённые — убрать.
pub fn write_sync(root: &Path, index_path: &Path, facts: &[Fact], next: u64, bots: &[BotRef]) -> std::io::Result<()> {
    let _lock = VAULT.lock().unwrap_or_else(|e| e.into_inner());
    write_locked(root, index_path, facts, next, bots)
}

fn write_locked(root: &Path, index_path: &Path, facts: &[Fact], next: u64, bots: &[BotRef]) -> std::io::Result<()> {
    let mut index = read_index(index_path).unwrap_or_default();
    let planned = plan_paths(facts, bots);
    let targets = link_targets(&planned);
    let current: HashMap<String, String> = index.files.iter().map(|(rel, e)| (e.fact.id.clone(), rel.clone())).collect();
    let alive: HashSet<&str> = facts.iter().map(|f| f.id.as_str()).collect();
    // Файл правили руками после нашего последнего чтения?
    let edited = |index: &Index, rel: &str| -> bool {
        match (index.files.get(rel), stat(&rel_to_path(root, rel))) {
            (Some(e), Some(meta)) => (e.len, e.modified) != meta,
            _ => false,
        }
    };

    for (id, rel) in &current {
        if alive.contains(id.as_str()) {
            continue;
        }
        if edited(&index, rel) {
            set_aside(root, &mut index, rel, "факт удалён в программе, а файл в это время правили");
        } else {
            let _ = std::fs::remove_file(rel_to_path(root, rel));
            index.files.remove(rel);
        }
    }

    let mut ordered: Vec<&Fact> = facts.iter().collect();
    ordered.sort_by_key(|f| number(&f.id));
    // Пути, уже записанные в этот проход: чужой старый путь мог занять другой факт.
    let mut claimed: HashSet<String> = HashSet::new();
    for fact in ordered {
        let rel = planned[&fact.id].clone();
        let old = current.get(&fact.id).cloned();
        let text = render(fact, &targets);
        let path = rel_to_path(root, &rel);
        if old.as_deref() == Some(rel.as_str()) {
            let same = index.files.get(&rel).is_some_and(|e| e.fact == *fact);
            // Правка руками не повод переписывать файл; номер в шапке и ссылки на соседей — повод.
            let shown = std::fs::read_to_string(&path).map(|t| file_matches(&t, fact, &text)).unwrap_or(false);
            if same && (shown || edited(&index, &rel)) {
                claimed.insert(rel);
                continue;
            }
            if edited(&index, &rel) {
                set_aside(root, &mut index, &rel, "файл правили руками одновременно с правкой в программе");
            }
        } else if path.exists() {
            match index.files.get(&rel).map(|e| e.fact.id.clone()) {
                // Место занимал другой живой факт: он сейчас переедет, его файл перепишем.
                Some(owner) if alive.contains(owner.as_str()) && !edited(&index, &rel) => {}
                Some(_) => set_aside(root, &mut index, &rel, "имя файла заняли, пока программа писала свой факт"),
                None => {
                    // Свой же факт (повтор переезда старой памяти) перезаписываем, чужой файл убираем.
                    let theirs = std::fs::read_to_string(&path).ok().and_then(|t| parse(&t, stem_of(&rel), 0).ok());
                    if theirs.map(|p| p.fact.id) != Some(fact.id.clone()) {
                        set_aside(root, &mut index, &rel, "имя файла заняли, пока программа писала свой факт");
                    }
                }
            }
        }
        write_atomic(&path, &text)?;
        claimed.insert(rel.clone());
        if let Some(old) = old.filter(|o| *o != rel && !claimed.contains(o)) {
            if !edited(&index, &old) {
                let _ = std::fs::remove_file(rel_to_path(root, &old));
            }
            index.files.remove(&old);
        }
        let meta = stat(&path).unwrap_or_default();
        index.files.insert(rel, Entry { len: meta.0, modified: meta.1, fact: fact.clone() });
    }
    std::fs::create_dir_all(root.join(SHARED_DIR))?;
    index.next = next.max(index.next);
    save_index(index_path, &index)
}

/// Файлы, которые не разобрались (для экрана памяти).
pub fn unparsed(index_path: &Path) -> Vec<Unparsed> {
    read_index(index_path).map(|i| i.unparsed).unwrap_or_default()
}

pub async fn read(root: PathBuf, index_path: PathBuf, bots: Vec<BotRef>) -> std::io::Result<Option<Loaded>> {
    tokio::task::spawn_blocking(move || read_sync(&root, &index_path, &bots)).await.map_err(std::io::Error::other)?
}

pub async fn write(root: PathBuf, index_path: PathBuf, facts: Vec<Fact>, next: u64, bots: Vec<BotRef>) -> std::io::Result<()> {
    tokio::task::spawn_blocking(move || write_sync(&root, &index_path, &facts, next, &bots)).await.map_err(std::io::Error::other)?
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Tmp(PathBuf);

    impl Tmp {
        fn new(name: &str) -> Self {
            let dir = std::env::temp_dir().join(format!("deloist-vault-{name}-{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&dir);
            std::fs::create_dir_all(&dir).unwrap();
            Self(dir)
        }
        fn root(&self) -> PathBuf {
            self.0.join("memory")
        }
        fn index(&self) -> PathBuf {
            self.0.join("memory-index.json")
        }
        fn read(&self, bots: &[BotRef]) -> Loaded {
            read_sync(&self.root(), &self.index(), bots).unwrap().expect("папка есть")
        }
        fn write(&self, facts: &[Fact], next: u64, bots: &[BotRef]) {
            write_sync(&self.root(), &self.index(), facts, next, bots).unwrap();
        }
        fn file(&self, rel: &str) -> PathBuf {
            rel_to_path(&self.root(), rel)
        }
    }

    impl Drop for Tmp {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn bots() -> Vec<BotRef> {
        vec![BotRef { id: "bot_1".into(), name: "Рен".into() }, BotRef { id: "bot_2".into(), name: "Мира: аналитик".into() }]
    }

    fn fact(id: &str, scope: &str, title: &str) -> Fact {
        Fact {
            id: id.into(),
            scope: scope.into(),
            kind: if scope == SHARED { "user".into() } else { "project".into() },
            title: title.into(),
            author: "bot_1".into(),
            created_at: 1_762_750_000_123,
            updated_at: 1_762_750_000_456,
            ..Fact::default()
        }
    }

    fn sample() -> Vec<Fact> {
        vec![
            Fact { body: "по утрам, без сахара".into(), entities: vec!["Ваня".into()], links: vec!["m2".into()], ..fact("m1", SHARED, "Пьёт кофе: крепкий?") },
            Fact { entities: vec!["Ozon".into(), "ООО \"Ромашка\"".into()], links: vec!["m1".into()], ..fact("m2", "bot_1", "Отчёт Ozon по понедельникам") },
            fact("m3", "bot_2", "Отчёт Ozon по понедельникам"),
            fact("m4", SHARED, "CON"),
        ]
    }

    #[test]
    fn facts_survive_the_round_trip_through_files() {
        let tmp = Tmp::new("round");
        tmp.write(&sample(), 4, &bots());
        let loaded = tmp.read(&bots());
        assert_eq!(loaded.facts, sample());
        assert_eq!(loaded.next, 4);
        assert!(loaded.external.is_empty());
        assert!(tmp.file("shared/Пьёт кофе крепкий.md").exists(), "знаки, запрещённые Windows, убраны из имени");
        assert!(tmp.file("bots/Рен/Отчёт Ozon по понедельникам.md").exists());
        assert!(tmp.file("bots/Мира аналитик/Отчёт Ozon по понедельникам.md").exists());
        assert!(tmp.file("shared/CON_.md").exists());
        let text = std::fs::read_to_string(tmp.file("shared/Пьёт кофе крепкий.md")).unwrap();
        assert!(text.contains("\n## Связи\n\n- [[bots/Рен/Отчёт Ozon по понедельникам]]\n"), "одинаковые имена — ссылкой с путём: {text}");
        // Второе чтение — из индекса, файлы не разбираются заново.
        assert_eq!(tmp.read(&bots()).facts, sample());
        // Без индекса всё разбирается из самих файлов, без потерь, и это не правка руками.
        std::fs::remove_file(tmp.index()).unwrap();
        let again = tmp.read(&bots());
        assert_eq!(again.facts, sample());
        assert_eq!(again.next, 4);
        assert!(again.external.is_empty());
    }

    #[test]
    fn a_title_taking_a_neighbours_file_name_moves_the_neighbour() {
        let tmp = Tmp::new("swap");
        let facts = vec![fact("m3", SHARED, "Б"), fact("m7", SHARED, "А")];
        tmp.write(&facts, 7, &bots());
        let mut renamed = facts.clone();
        renamed[0].title = "А".into();
        tmp.write(&renamed, 7, &bots());
        assert!(!tmp.file(UNPARSED_DIR).exists(), "ничего не ушло в неразобранное");
        assert!(!tmp.file("shared/Б.md").exists());
        assert!(std::fs::read_to_string(tmp.file("shared/А.md")).unwrap().starts_with("---\nid: m3\n"));
        assert!(std::fs::read_to_string(tmp.file("shared/А (m7).md")).unwrap().starts_with("---\nid: m7\n"));
        assert_eq!(tmp.read(&bots()).facts, renamed);
        std::fs::remove_file(tmp.index()).unwrap();
        assert_eq!(tmp.read(&bots()).facts, renamed, "имя с номером узнаётся и без индекса");
    }

    #[test]
    fn hand_edits_in_obsidian_are_noticed() {
        let tmp = Tmp::new("hand");
        tmp.write(&sample(), 4, &bots());
        tmp.read(&bots());
        // Obsidian переписывает шапку по-своему: списки строками, строки без кавычек.
        let path = tmp.file("shared/Пьёт кофе крепкий.md");
        std::fs::write(
            &path,
            "---\nid: m1\ntitle: \"Пьёт кофе: крепкий?\"\nkind: user\nentities:\n  - Ваня\n  - Кофейня\nupdated: 2025-11-10T10:00:00.456Z\n---\nпо утрам, с молоком\r\n\r\n## Связи\r\n\r\n- [[Отчёт Ozon по понедельникам|отчёт]]\r\n",
        )
        .unwrap();
        let loaded = tmp.read(&bots());
        let m1 = loaded.facts.iter().find(|f| f.id == "m1").unwrap();
        assert_eq!(m1.body, "по утрам, с молоком");
        assert_eq!(m1.entities, vec!["Ваня", "Кофейня"]);
        assert!(m1.links.is_empty(), "имя файла не одно на всю память — ссылка без пути не разрешается");
        assert!(m1.updated_at > sample()[0].updated_at, "дата правки сдвинута вперёд");
        assert_eq!(loaded.external.len(), 1);
        assert_eq!(loaded.external[0].0.as_ref().unwrap().body, "по утрам, без сахара");
        // Файл руками не переписываем: человек может как раз его править.
        assert!(std::fs::read_to_string(&path).unwrap().contains("Кофейня"));
        assert!(tmp.read(&bots()).external.is_empty(), "правка замечается один раз");
    }

    #[test]
    fn a_renamed_file_renames_the_fact_and_a_new_note_gets_a_number() {
        let tmp = Tmp::new("rename");
        tmp.write(&sample(), 4, &bots());
        tmp.read(&bots());
        std::fs::rename(tmp.file("shared/CON_.md"), tmp.file("shared/Кабинет WB.md")).unwrap();
        std::fs::write(tmp.file("shared/Любит чай.md"), "Зелёный, вечером. См. [[Кабинет WB]]").unwrap();
        let loaded = tmp.read(&bots());
        let m4 = loaded.facts.iter().find(|f| f.id == "m4").unwrap();
        assert_eq!(m4.title, "Кабинет WB");
        let new = loaded.facts.iter().find(|f| f.title == "Любит чай").unwrap();
        assert_eq!(new.id, "m5");
        assert_eq!(new.author, "human");
        assert_eq!(new.kind, "user");
        assert_eq!(new.links, vec!["m4"]);
        assert_eq!(loaded.next, 5);
        assert_eq!(loaded.external.len(), 2);
        let text = std::fs::read_to_string(tmp.file("shared/Любит чай.md")).unwrap();
        assert!(text.starts_with("---\nid: m5\n"), "новому файлу дописан номер: {text}");
    }

    #[test]
    fn a_copied_file_gets_its_own_number() {
        let tmp = Tmp::new("copy");
        tmp.write(&sample(), 4, &bots());
        tmp.read(&bots());
        std::fs::copy(tmp.file("shared/CON_.md"), tmp.file("shared/CON копия.md")).unwrap();
        let loaded = tmp.read(&bots());
        let ids: Vec<&str> = loaded.facts.iter().map(|f| f.id.as_str()).collect();
        assert_eq!(ids, vec!["m1", "m2", "m3", "m4", "m5"]);
    }

    #[test]
    fn broken_files_go_aside_instead_of_breaking_memory() {
        let tmp = Tmp::new("broken");
        tmp.write(&sample(), 4, &bots());
        tmp.read(&bots());
        std::fs::write(tmp.file("shared/Сломан.md"), "---\nid: m9\ntitle: без конца").unwrap();
        std::fs::create_dir_all(tmp.file("bots/Чужой")).unwrap();
        std::fs::write(tmp.file("bots/Чужой/Факт.md"), "текст").unwrap();
        let loaded = tmp.read(&bots());
        assert_eq!(loaded.facts.len(), 4);
        assert!(tmp.file("_unparsed/Сломан.md").exists());
        assert!(tmp.file("_unparsed/Факт.md").exists());
        let aside = unparsed(&tmp.index());
        assert_eq!(aside.len(), 2);
        assert!(aside.iter().any(|u| u.from == "bots/Чужой/Факт.md" && u.reason.contains("Чужой")), "{aside:?}");
    }

    #[test]
    fn deleting_and_moving_facts_moves_their_files() {
        let tmp = Tmp::new("move");
        tmp.write(&sample(), 4, &bots());
        let mut facts = sample();
        facts.retain(|f| f.id != "m3");
        facts[1].scope = SHARED.into();
        tmp.write(&facts, 4, &bots());
        assert!(!tmp.file("bots/Мира аналитик/Отчёт Ozon по понедельникам.md").exists());
        assert!(!tmp.file("bots/Рен/Отчёт Ozon по понедельникам.md").exists());
        assert!(tmp.file("shared/Отчёт Ozon по понедельникам.md").exists());
        let text = std::fs::read_to_string(tmp.file("shared/Пьёт кофе крепкий.md")).unwrap();
        assert!(text.contains("- [[Отчёт Ozon по понедельникам]]"), "имя стало единственным — ссылка без пути: {text}");
        assert_eq!(tmp.read(&bots()).facts, facts);
    }

    #[test]
    fn a_bot_rename_keeps_its_facts() {
        let tmp = Tmp::new("botname");
        tmp.write(&sample(), 4, &bots());
        let renamed = vec![BotRef { id: "bot_1".into(), name: "Рен Старший".into() }, bots()[1].clone()];
        let loaded = tmp.read(&renamed);
        assert_eq!(loaded.facts, sample(), "старая папка узнана по scope в шапке");
        tmp.write(&loaded.facts, loaded.next, &renamed);
        assert!(tmp.file("bots/Рен Старший/Отчёт Ozon по понедельникам.md").exists());
        assert!(!tmp.file("bots/Рен/Отчёт Ozon по понедельникам.md").exists());
    }

    #[test]
    fn names_are_safe_for_windows_and_obsidian() {
        assert_eq!(sanitize("a/b\\c: d?"), "a b c d");
        assert_eq!(sanitize("  ...точки... "), "точки");
        assert_eq!(sanitize("lpt1"), "lpt1_");
        assert_eq!(sanitize(&"я".repeat(200)).chars().count(), STEM_LIMIT);
        assert_eq!(wikilinks("a [[x|y]] b [[z#h]] [[x]]"), vec!["x", "z"]);
    }
}
