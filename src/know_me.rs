//! «Познакомиться быстрее»: чтение локальных диалогов AI-ассистентов, чтобы боты узнали человека.
//!
//! Приватность: opt-in, только реплики человека (user turns), ограничения 300 сообщений / 2000 знаков,
//! секреты вырезаются до модели, конденсация идёт через собственную модель человека.
//!
//! Счётчики источников (`sources()`) кэшируются по отпечаткам файлов (путь, длина, время изменения):
//! повторный вызов не открывает файлы; в кэше только числа, не текст. Домашняя папка берётся из
//! `CREW_HOME`, если она задана (тесты и сценарии), иначе из профиля пользователя.

use std::collections::{HashMap, HashSet};
use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::SystemTime;

use axum::extract::State;
use axum::http::StatusCode;
use axum::Json;
use regex::Regex;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::App;
use crate::memory;
use crate::service;

// ---------------------------------------------------------------------------
// Константы
// ---------------------------------------------------------------------------

pub const MAX_MESSAGES: usize = 300;
pub const MAX_MESSAGE_CHARS: usize = 2000;
pub const MAX_TOPICS: usize = 40;

// ---------------------------------------------------------------------------
// Источники
// ---------------------------------------------------------------------------

#[derive(Clone, Debug, Serialize)]
pub struct SourceInfo {
    pub id: String,
    pub name: String,
    pub found: bool,
    pub count: usize,
    /// Только в отладочной сборке: сколько файлов пересчитано в этом вызове, остальные взяты из кэша.
    #[cfg(debug_assertions)]
    pub recounted: usize,
}

/// Файл, который источник считает: путь, длина и время изменения из листинга каталога.
/// Сам файл не открывается, пока его отпечаток не поменялся.
#[derive(Clone, Debug)]
pub struct FileStat {
    pub path: PathBuf,
    pub len: u64,
    pub modified: Option<SystemTime>,
}

/// Кэш счётчиков хранит только отпечаток и число разговоров в файле, никакого текста.
#[derive(Clone, Copy)]
struct FileMemo {
    len: u64,
    modified: Option<SystemTime>,
    count: usize,
}

/// (домашняя папка, id источника) → файлы последнего вызова с их отпечатками.
static COUNT_MEMO: OnceLock<Mutex<HashMap<(PathBuf, &'static str), HashMap<PathBuf, FileMemo>>>> = OnceLock::new();

#[derive(Clone, Copy)]
struct Counted {
    found: bool,
    count: usize,
    #[cfg_attr(not(debug_assertions), allow(dead_code))]
    recounted: usize,
}

/// Счётчик — число разговоров (файлов), а не сообщений: история Claude Code бывает на десятки
/// гигабайт, и чтение всех файлов ради цифры держало окно пустым минутами (12.11.2026, 11 ГБ).
/// Повторный вызов берёт числа из кэша по отпечаткам файлов (путь, длина, время изменения):
/// пересчитываются только изменённые и новые файлы, удалённые уходят из счёта.
///
/// Домашняя папка — `CREW_HOME`, если задана (тесты и сценарии), иначе профиль пользователя.
pub fn sources() -> Vec<SourceInfo> {
    sources_in(&home_dir())
}

fn sources_in(home: &Path) -> Vec<SourceInfo> {
    let claude = claude_files(home);
    let codex = codex_files(home);
    let oroot = opencode_root(home);
    let ofound = oroot.exists();
    let opencode = if ofound { collect_files(&oroot, "json") } else { Vec::new() };
    let gemini = gemini_files(home);
    vec![
        info("claude_code", "Claude Code", cached_count(home, "claude_code", !claude.is_empty(), &claude, one_per_file)),
        info("codex", "Codex", cached_count(home, "codex", !codex.is_empty(), &codex, one_per_file)),
        info("opencode", "OpenCode", cached_count(home, "opencode", ofound, &opencode, opencode_counts)),
        info("gemini", "Gemini CLI", cached_count(home, "gemini", !gemini.is_empty(), &gemini, one_per_file)),
        // Cursor — пропускаем в фазе 1.
        info("cursor", "Cursor (скоро)", Counted { found: false, count: 0, recounted: 0 }),
    ]
}

fn info(id: &str, name: &str, c: Counted) -> SourceInfo {
    SourceInfo {
        id: id.into(),
        name: name.into(),
        found: c.found,
        count: c.count,
        #[cfg(debug_assertions)]
        recounted: c.recounted,
    }
}

/// Файл, который источник считает за один разговор, без чтения.
fn one_per_file(_: &Path) -> usize {
    1
}

/// OpenCode: разговор — файл, где есть реплика человека, поэтому файл приходится читать.
fn opencode_counts(path: &Path) -> usize {
    fs::read_to_string(path)
        .ok()
        .and_then(|t| serde_json::from_str::<Value>(&t).ok())
        .map(|v| opencode_user_text(&v).is_some() as usize)
        .unwrap_or(0)
}

/// Считает файлы по кэшу: тот же отпечаток — то же число; иначе файл пересчитывается заново.
fn cached_count(home: &Path, id: &'static str, found: bool, files: &[FileStat], per_file: fn(&Path) -> usize) -> Counted {
    let memo = COUNT_MEMO.get_or_init(Default::default);
    let mut memo = memo.lock().unwrap_or_else(|e| e.into_inner());
    let old = memo.remove(&(home.to_path_buf(), id)).unwrap_or_default();
    let mut fresh = HashMap::with_capacity(files.len());
    let (mut count, mut recounted) = (0, 0);
    for f in files {
        let m = match old.get(&f.path) {
            Some(m) if m.len == f.len && m.modified == f.modified => *m,
            _ => {
                recounted += 1;
                FileMemo { len: f.len, modified: f.modified, count: per_file(&f.path) }
            }
        };
        count += m.count;
        fresh.insert(f.path.clone(), m);
    }
    memo.insert((home.to_path_buf(), id), fresh);
    Counted { found, count, recounted }
}

// ---------------------------------------------------------------------------
// HTTP handlers
// ---------------------------------------------------------------------------

pub async fn http_sources() -> Json<Value> {
    // Обход папок — блокирующий ввод-вывод: не на потоке сервера.
    let list = tokio::task::spawn_blocking(sources).await.unwrap_or_default();
    Json(json!(list))
}

#[derive(Deserialize)]
pub struct ReadBody {
    sources: Option<Vec<String>>,
    zip: Option<String>,
}

/// Какая модель получит темы и локальная ли она: человек видит это до отправки.
fn receiver(app: &App) -> Option<(String, bool)> {
    let bot = service::house_bot(app)?;
    let choice = crate::turn::main_choice(app, &bot, &app.store.settings());
    let name = if choice.model.is_empty() { choice.driver.clone() } else { choice.model.clone() };
    Some((name, is_local_choice(&choice)))
}

pub async fn http_read(State(app): State<Arc<App>>, Json(body): Json<ReadBody>) -> Result<Json<Value>, (StatusCode, String)> {
    let (model, local) = receiver(&app).ok_or_else(|| (StatusCode::SERVICE_UNAVAILABLE, "нет подходящей модели".to_string()))?;
    let sources = body.sources.unwrap_or_default();
    let mut messages: Vec<String> = Vec::new();
    let mut redactions_total: usize = 0;
    for src in &sources {
        let raw = read_source(src, body.zip.as_deref());
        for m in raw {
            let (redacted, n) = redact(&m);
            redactions_total += n;
            messages.push(redacted);
            if messages.len() >= MAX_MESSAGES * 10 {
                break;
            }
        }
    }
    // cap per source already handled in read_source; global cap 300 per source but topics grouping caps 40
    let topics = group_topics(&messages);
    // recompute redactions for topics? redactions_total already counts all messages
    Ok(Json(json!({ "topics": topics, "redactions": redactions_total, "model": model, "local": local })))
}

#[derive(Deserialize)]
pub struct ProposeBody {
    topics: Option<Vec<String>>,
}

pub async fn http_propose(State(app): State<Arc<App>>, Json(body): Json<ProposeBody>) -> Result<Json<Value>, (StatusCode, String)> {
    let topics = body.topics.unwrap_or_default();
    let bot = service::house_bot(&app).ok_or_else(|| (StatusCode::SERVICE_UNAVAILABLE, "нет подходящей модели".to_string()))?;
    let choice = crate::turn::main_choice(&app, &bot, &app.store.settings());
    let model_name = if choice.model.is_empty() { choice.driver.clone() } else { choice.model.clone() };
    let is_local = is_local_choice(&choice);
    let system = "Ты помогаешь составить короткий профиль человека по темам из его диалогов с AI-ассистентами. \
Верни ТОЛЬКО JSON-объект: {\"facts\": [\"короткий факт о человеке\", ...], \"agents\": [{\"name\": \"имя бота\", \"role\": \"одна строка роли\"}, ...]}. \
Фактов 3-6, агентов 2-4. Факты короткие, без секретов. Никакого другого текста.";
    let task = topics.join("\n");
    let reply = service::complete(&app, &bot, system, &task).await.map_err(|e| (StatusCode::BAD_GATEWAY, e))?;
    let obj = service::json_object(&reply).ok_or_else(|| (StatusCode::BAD_GATEWAY, "модель вернула не JSON".to_string()))?;
    let facts = obj.get("facts").and_then(Value::as_array).cloned().unwrap_or_default();
    let agents = obj.get("agents").and_then(Value::as_array).cloned().unwrap_or_default();
    Ok(Json(json!({ "facts": facts, "agents": agents, "model": model_name, "local": is_local })))
}

#[derive(Deserialize)]
pub struct SaveBody {
    facts: Option<Vec<String>>,
}

pub async fn http_save(State(app): State<Arc<App>>, Json(body): Json<SaveBody>) -> Result<Json<Value>, (StatusCode, String)> {
    let facts = body.facts.unwrap_or_default();
    for fact in facts {
        let input = json!({ "title": fact });
        let _ = memory::human_write(&app, &input, None).await.map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e))?;
    }
    Ok(Json(json!({ "saved": true })))
}

// ---------------------------------------------------------------------------
// Локальная ли модель
// ---------------------------------------------------------------------------

fn is_local_choice(choice: &crate::turn::ModelChoice) -> bool {
    // Внешние агенты (Claude Code, Codex, OpenCode по ACP/CLI) ходят в облако: не локальные.
    // Локальна только модель на этом компьютере (Ollama, LM Studio и свой адрес на localhost).
    if choice.driver == "demo" {
        return true;
    }
    if !choice.driver.starts_with("openai:") {
        return false;
    }
    let base = choice.base_url();
    base.contains("127.0.0.1") || base.contains("localhost")
}

// ---------------------------------------------------------------------------
// Чтение источников
// ---------------------------------------------------------------------------

fn read_source(id: &str, zip_path: Option<&str>) -> Vec<String> {
    let home = home_dir();
    match id {
        "claude_code" => {
            let files = claude_files(&home);
            read_claude(&files)
        }
        "codex" => {
            let files = codex_files(&home);
            read_codex(&files)
        }
        "opencode" => {
            let root = opencode_root(&home);
            if root.exists() { read_opencode(&root) } else { Vec::new() }
        }
        "gemini" => {
            let files = gemini_files(&home);
            read_gemini(&files)
        }
        "zip" => {
            // zip не поддерживается без крейта zip: пробуем как каталог с conversations.json
            if let Some(p) = zip_path { read_zip_or_dir(p) } else { Vec::new() }
        }
        _ => Vec::new(),
    }
}

// --- Claude Code ---

fn claude_files(home: &Path) -> Vec<FileStat> {
    collect_files(&home.join(".claude").join("projects"), "jsonl")
}

pub fn claude_user_text(line: &str) -> Option<String> {
    let v: Value = serde_json::from_str(line).ok()?;
    if v.get("type").and_then(Value::as_str) != Some("user") { return None; }
    let content = v.get("message").and_then(|m| m.get("content"))?;
    extract_content_text(content)
}


fn read_claude(files: &[FileStat]) -> Vec<String> {
    let mut out = Vec::new();
    for f in files {
        if let Ok(text) = fs::read_to_string(&f.path) {
            for line in text.lines() {
                if let Some(m) = claude_user_text(line) {
                    out.push(truncate(&m, MAX_MESSAGE_CHARS));
                    if out.len() >= MAX_MESSAGES { return out; }
                }
            }
        }
    }
    out
}

// --- Codex ---

fn codex_files(home: &Path) -> Vec<FileStat> {
    collect_files(&home.join(".codex").join("sessions"), "jsonl")
}

pub fn codex_user_text(line: &str) -> Option<String> {
    let v: Value = serde_json::from_str(line).ok()?;
    // response_item with message role user
    if v.get("type").and_then(Value::as_str) == Some("response_item") {
        if let Some(p) = v.get("payload") {
            if p.get("type").and_then(Value::as_str) == Some("message") && p.get("role").and_then(Value::as_str) == Some("user") {
                if let Some(c) = p.get("content") { return extract_content_text(c); }
            }
        }
    }
    if v.get("type").and_then(Value::as_str) == Some("event_msg") {
        if let Some(p) = v.get("payload") {
            if p.get("type").and_then(Value::as_str) == Some("user_message") {
                if let Some(m) = p.get("message").and_then(Value::as_str) { return Some(m.to_string()); }
            }
        }
    }
    // generic role user
    if v.get("role").and_then(Value::as_str) == Some("user") {
        if let Some(c) = v.get("content") { return extract_content_text(c); }
        if let Some(t) = v.get("text").and_then(Value::as_str) { return Some(t.to_string()); }
    }
    if v.get("type").and_then(Value::as_str) == Some("user") {
        if let Some(c) = v.get("content") { return extract_content_text(c); }
        if let Some(t) = v.get("text").and_then(Value::as_str) { return Some(t.to_string()); }
    }
    None
}


fn read_codex(files: &[FileStat]) -> Vec<String> {
    let mut out = Vec::new();
    for f in files {
        if let Ok(text) = fs::read_to_string(&f.path) {
            for line in text.lines() {
                if let Some(m) = codex_user_text(line) {
                    out.push(truncate(&m, MAX_MESSAGE_CHARS));
                    if out.len() >= MAX_MESSAGES { return out; }
                }
            }
        }
    }
    out
}

// --- OpenCode ---

fn opencode_root(home: &Path) -> PathBuf {
    home.join(".local").join("share").join("opencode")
}

pub fn opencode_user_text(v: &Value) -> Option<String> {
    if v.get("role").and_then(Value::as_str) != Some("user") { return None; }
    if let Some(c) = v.get("content") { if let Some(t) = extract_content_text(c) { return Some(t); } }
    if let Some(p) = v.get("parts") { if let Some(t) = extract_content_text(p) { return Some(t); } }
    if let Some(t) = v.get("text").and_then(Value::as_str) { return Some(t.to_string()); }
    None
}

fn read_opencode(root: &Path) -> Vec<String> {
    let mut out = Vec::new();
    for f in collect_files(root, "json") {
        if let Ok(text) = fs::read_to_string(&f.path) {
            if let Ok(v) = serde_json::from_str::<Value>(&text) {
                if let Some(m) = opencode_user_text(&v) {
                    out.push(truncate(&m, MAX_MESSAGE_CHARS));
                    if out.len() >= MAX_MESSAGES { return out; }
                }
            }
        }
    }
    out
}

// --- Gemini ---

fn gemini_files(home: &Path) -> Vec<FileStat> {
    let root = home.join(".gemini").join("tmp");
    let mut files: Vec<FileStat> = Vec::new();
    // logs.json anywhere
    for f in collect_files(&root, "json") {
        if f.path.file_name().and_then(|n| n.to_str()).map(|n| n == "logs.json").unwrap_or(false) {
            files.push(f);
        }
    }
    // chats/*.json
    if root.exists() {
        for f in collect_files(&root, "json") {
            let pathstr = f.path.to_string_lossy().to_lowercase();
            if pathstr.contains("\\chats\\") || pathstr.contains("/chats/") {
                if !files.iter().any(|x| x.path == f.path) { files.push(f); }
            }
        }
    }
    files
}

pub fn gemini_collect(value: &Value, out: &mut Vec<String>) {
    match value {
        Value::Array(arr) => {
            for item in arr { gemini_collect(item, out); }
        }
        Value::Object(map) => {
            // if this object is a user message, extract
            let is_user = map.get("role").and_then(Value::as_str) == Some("user")
                || map.get("type").and_then(Value::as_str) == Some("user")
                || map.get("author").and_then(Value::as_str) == Some("user");
            if is_user {
                if let Some(t) = extract_gemini_text(value) {
                    out.push(t);
                    return;
                }
            }
            for (_k, v) in map {
                gemini_collect(v, out);
            }
        }
        _ => {}
    }
}

fn extract_gemini_text(v: &Value) -> Option<String> {
    if let Some(s) = v.get("content").and_then(Value::as_str) { return Some(s.to_string()); }
    if let Some(s) = v.get("text").and_then(Value::as_str) { return Some(s.to_string()); }
    if let Some(s) = v.get("message").and_then(Value::as_str) { return Some(s.to_string()); }
    if let Some(arr) = v.get("parts").and_then(Value::as_array) {
        let mut parts = Vec::new();
        for p in arr {
            if let Some(s) = p.get("text").and_then(Value::as_str) { parts.push(s.to_string()); }
            else if let Some(s) = p.as_str() { parts.push(s.to_string()); }
        }
        if !parts.is_empty() { return Some(parts.join("\n")); }
    }
    if let Some(arr) = v.get("content").and_then(Value::as_array) {
        let mut parts = Vec::new();
        for p in arr {
            if let Some(s) = p.get("text").and_then(Value::as_str) { parts.push(s.to_string()); }
            else if let Some(s) = p.as_str() { parts.push(s.to_string()); }
        }
        if !parts.is_empty() { return Some(parts.join("\n")); }
    }
    None
}


fn read_gemini(files: &[FileStat]) -> Vec<String> {
    let mut out = Vec::new();
    for f in files {
        if let Ok(text) = fs::read_to_string(&f.path) {
            if let Ok(v) = serde_json::from_str::<Value>(&text) {
                let mut msgs = Vec::new();
                gemini_collect(&v, &mut msgs);
                for m in msgs {
                    out.push(truncate(&m, MAX_MESSAGE_CHARS));
                    if out.len() >= MAX_MESSAGES { return out; }
                }
            }
        }
    }
    out
}

// --- zip / export ---

fn read_zip_or_dir(path: &str) -> Vec<String> {
    let p = Path::new(path);
    if p.is_dir() {
        // look for conversations.json
        let conv = p.join("conversations.json");
        if conv.exists() {
            if let Ok(text) = fs::read_to_string(&conv) {
                if let Ok(v) = serde_json::from_str::<Value>(&text) {
                    return parse_export(&v);
                }
            }
        }
        // maybe file directly
    }
    if p.is_file() && p.extension().and_then(|e| e.to_str()) == Some("json") {
        if let Ok(text) = fs::read_to_string(p) {
            if let Ok(v) = serde_json::from_str::<Value>(&text) {
                return parse_export(&v);
            }
        }
    }
    Vec::new()
}

pub fn parse_export(value: &Value) -> Vec<String> {
    let mut out = Vec::new();
    // ChatGPT style: array of conversations
    let convs: Vec<&Value> = if let Some(arr) = value.as_array() { arr.iter().collect() } else { vec![value] };
    for conv in convs {
        // ChatGPT mapping
        if let Some(mapping) = conv.get("mapping") {
            if let Some(obj) = mapping.as_object() {
                for (_k, node) in obj {
                    if let Some(msg) = node.get("message") {
                        let role = msg.get("author").and_then(|a| a.get("role")).and_then(Value::as_str).unwrap_or_default();
                        if role == "user" {
                            if let Some(c) = msg.get("content") {
                                if let Some(t) = extract_content_text(c) {
                                    out.push(truncate(&t, MAX_MESSAGE_CHARS));
                                }
                            }
                        }
                    }
                }
            }
        }
        // Claude style chat_messages
        if let Some(arr) = conv.get("chat_messages").and_then(Value::as_array) {
            for m in arr {
                let sender = m.get("sender").and_then(Value::as_str).unwrap_or_default();
                if sender == "human" || sender == "user" {
                    if let Some(t) = m.get("text").and_then(Value::as_str) { out.push(truncate(t, MAX_MESSAGE_CHARS)); }
                    else if let Some(c) = m.get("content") {
                        if let Some(t) = extract_content_text(c) { out.push(truncate(&t, MAX_MESSAGE_CHARS)); }
                    }
                }
            }
        }
        // generic messages array
        if let Some(arr) = conv.get("messages").and_then(Value::as_array) {
            for m in arr {
                let role = m.get("role").and_then(Value::as_str).unwrap_or_default();
                if role == "user" || role == "human" {
                    if let Some(c) = m.get("content") {
                        if let Some(t) = extract_content_text(c) { out.push(truncate(&t, MAX_MESSAGE_CHARS)); }
                    } else if let Some(t) = m.get("text").and_then(Value::as_str) {
                        out.push(truncate(t, MAX_MESSAGE_CHARS));
                    }
                }
            }
        }
    }
    // cap
    if out.len() > MAX_MESSAGES { out.truncate(MAX_MESSAGES); }
    out
}

// ---------------------------------------------------------------------------
// Общие утилиты
// ---------------------------------------------------------------------------

/// Домашняя папка с историями ассистентов: `CREW_HOME`, если задана, иначе профиль пользователя.
fn home_dir() -> PathBuf {
    match env::var("CREW_HOME") {
        Ok(h) if !h.trim().is_empty() => PathBuf::from(h),
        _ => PathBuf::from(env::var("USERPROFILE").unwrap_or_default()),
    }
}

/// Файлы с расширением `ext` под `root` с длиной и временем изменения. Метаданные берутся из
/// листинга каталога (на Windows без отдельного обращения к каждому файлу), сами файлы не открываются.
fn collect_files(root: &Path, ext: &str) -> Vec<FileStat> {
    let mut out = Vec::new();
    if !root.exists() { return out; }
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        if let Ok(entries) = fs::read_dir(&dir) {
            for e in entries.flatten() {
                let path = e.path();
                // Ссылки разрешаем как прежде: is_dir() ходит по ним, file_type() нет.
                let is_dir = match e.file_type() {
                    Ok(t) if !t.is_symlink() => t.is_dir(),
                    _ => path.is_dir(),
                };
                if is_dir {
                    stack.push(path);
                } else if path.extension().and_then(|x| x.to_str()) == Some(ext) {
                    let meta = e.metadata().ok();
                    out.push(FileStat {
                        len: meta.as_ref().map(|m| m.len()).unwrap_or(0),
                        modified: meta.and_then(|m| m.modified().ok()),
                        path,
                    });
                }
            }
        }
    }
    out
}

fn extract_content_text(v: &Value) -> Option<String> {
    match v {
        Value::String(s) => Some(s.clone()),
        Value::Array(arr) => {
            let mut parts = Vec::new();
            for item in arr {
                if let Some(s) = item.as_str() { parts.push(s.to_string()); continue; }
                if let Some(t) = item.get("text").and_then(Value::as_str) { parts.push(t.to_string()); continue; }
                if let Some(c) = item.get("content").and_then(Value::as_str) { parts.push(c.to_string()); continue; }
            }
            if parts.is_empty() { None } else { Some(parts.join("\n")) }
        }
        Value::Object(map) => {
            if let Some(arr) = map.get("parts").and_then(Value::as_array) {
                let mut parts = Vec::new();
                for item in arr {
                    if let Some(s) = item.as_str() { parts.push(s.to_string()); }
                    else if let Some(t) = item.get("text").and_then(Value::as_str) { parts.push(t.to_string()); }
                }
                if !parts.is_empty() { return Some(parts.join("\n")); }
            }
            if let Some(s) = map.get("text").and_then(Value::as_str) { return Some(s.to_string()); }
            if let Some(s) = map.get("content").and_then(Value::as_str) { return Some(s.to_string()); }
            None
        }
        _ => None,
    }
}

fn truncate(s: &str, n: usize) -> String {
    s.chars().take(n).collect()
}

// ---------------------------------------------------------------------------
// Redaction
// ---------------------------------------------------------------------------

pub fn redact(input: &str) -> (String, usize) {
    let mut text = input.to_string();
    let mut n = 0;
    // NAME=value lines
    let (redacted, pairs) = crate::keys::redact_pairs(&text);
    text = redacted;
    n += pairs.len();

    // private key blocks
    let re = Regex::new(r"-----BEGIN [A-Z ]*PRIVATE KEY-----[\s\S]*?-----END [A-Z ]*PRIVATE KEY-----").unwrap();
    let c = re.find_iter(&text).count();
    if c > 0 { n += c; text = re.replace_all(&text, "[REDACTED PRIVATE KEY]").to_string(); }

    // key shapes
    let patterns: &[(&str, &str)] = &[
        (r"sk-[A-Za-z0-9_-]{20,}", "[REDACTED KEY]"),
        (r"ghp_[A-Za-z0-9]{36,}", "[REDACTED KEY]"),
        (r"github_pat_[A-Za-z0-9_]{20,}", "[REDACTED KEY]"),
        (r"xox[bp]-[A-Za-z0-9-]{10,}", "[REDACTED KEY]"),
        (r"AKIA[0-9A-Z]{16}", "[REDACTED KEY]"),
    ];
    for (pat, repl) in patterns {
        let re = Regex::new(pat).unwrap();
        let c = re.find_iter(&text).count();
        if c > 0 { n += c; text = re.replace_all(&text, *repl).to_string(); }
    }
    // long base64/hex runs >=32
    for pat in [r"[A-Za-z0-9+/]{32,}={0,2}", r"[0-9a-fA-F]{32,}"] {
        let re = Regex::new(pat).unwrap();
        let c = re.find_iter(&text).count();
        if c > 0 { n += c; text = re.replace_all(&text, "[REDACTED]").to_string(); }
    }
    // emails: keep domain
    let re = Regex::new(r"\b[A-Za-z0-9._%+-]+@([A-Za-z0-9.-]+\.[A-Za-z]{2,})\b").unwrap();
    let c = re.find_iter(&text).count();
    if c > 0 {
        n += c;
        text = re.replace_all(&text, |caps: &regex::Captures| format!("***@{}", &caps[1])).to_string();
    }
    // phone numbers
    let re = Regex::new(r"\+?\d[\d\s\-()]{8,}\d").unwrap();
    let c = re.find_iter(&text).count();
    if c > 0 { n += c; text = re.replace_all(&text, "[PHONE]").to_string(); }

    (text, n)
}

// ---------------------------------------------------------------------------
// Topics grouping
// ---------------------------------------------------------------------------

pub fn group_topics(messages: &[String]) -> Vec<String> {
    let mut seen: HashSet<String> = HashSet::new();
    let mut out: Vec<String> = Vec::new();
    for msg in messages {
        // skip code blocks: extract first non-code line
        let mut first = String::new();
        let mut in_code = false;
        for line in msg.lines() {
            let t = line.trim();
            if t.starts_with("```") { in_code = !in_code; continue; }
            if in_code { continue; }
            if t.is_empty() { continue; }
            // skip indented code-ish lines
            if line.starts_with("    ") || line.starts_with("\t") { continue; }
            first = t.to_string();
            break;
        }
        if first.is_empty() { continue; }
        if first.len() > 160 { first = first.chars().take(160).collect(); }
        let key = first.to_lowercase();
        if seen.contains(&key) { continue; }
        seen.insert(key);
        out.push(first);
        if out.len() >= MAX_TOPICS { break; }
    }
    out
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn redact_masks_sk() {
        let (out, n) = redact("token sk-abcdefghijklmnopqrstuvwxyz012345");
        assert!(out.contains("[REDACTED KEY]"), "{out}");
        assert!(!out.contains("sk-abc"), "{out}");
        assert!(n >= 1);
    }
    #[test]
    fn redact_masks_ghp() {
        let (out, _) = redact("ghp_abcdefghijklmnopqrstuvwxyz0123456789");
        assert!(out.contains("[REDACTED KEY]"), "{out}");
    }
    #[test]
    fn redact_masks_github_pat() {
        let (out, _) = redact("github_pat_11ABCDEFG_abcdefghijklmnopqrstuvwxyz");
        assert!(out.contains("[REDACTED KEY]"), "{out}");
    }
    #[test]
    fn redact_masks_slack() {
        let (out, _) = redact("xoxb-1234567890-abcdefghij");
        assert!(out.contains("[REDACTED KEY]"), "{out}");
    }
    #[test]
    fn redact_masks_aws() {
        let (out, _) = redact("AKIAIOSFODNN7EXAMPLE");
        assert!(out.contains("[REDACTED KEY]"), "{out}");
    }
    #[test]
    fn redact_masks_long_hex() {
        let (out, _) = redact("hash 0123456789abcdef0123456789abcdef");
        assert!(out.contains("[REDACTED]"), "{out}");
    }
    #[test]
    fn redact_masks_private_key() {
        let input = "-----BEGIN RSA PRIVATE KEY-----\nsecret\n-----END RSA PRIVATE KEY-----";
        let (out, _) = redact(input);
        assert!(out.contains("[REDACTED PRIVATE KEY]"), "{out}");
        assert!(!out.contains("secret"), "{out}");
    }
    #[test]
    fn redact_masks_email() {
        let (out, _) = redact("contact me at user@example.com please");
        assert!(out.contains("***@example.com"), "{out}");
        assert!(!out.contains("user@"), "{out}");
    }
    #[test]
    fn redact_masks_phone() {
        let (out, _) = redact("call +7 (999) 123-45-67 now");
        assert!(out.contains("[PHONE]"), "{out}");
    }
    #[test]
    fn redact_pairs_reuse() {
        let (out, _) = redact("API_KEY=secretvalue123");
        assert!(out.contains("{{secret:API_KEY}}"), "{out}");
    }

    #[test]
    fn claude_parser_keeps_user_only() {
        let line = r#"{"type":"user","message":{"content":"hi"}}"#;
        assert_eq!(claude_user_text(line), Some("hi".into()));
        let line2 = r#"{"type":"assistant","message":{"content":"no"}}"#;
        assert!(claude_user_text(line2).is_none());
        let bad = "not json";
        assert!(claude_user_text(bad).is_none());
    }
    #[test]
    fn codex_parser() {
        let line = r#"{"type":"response_item","payload":{"type":"message","role":"user","content":[{"type":"input_text","text":"hello"}]}}"#;
        assert_eq!(codex_user_text(line), Some("hello".into()));
        let line2 = r#"{"type":"event_msg","payload":{"type":"user_message","message":"yo"}}"#;
        assert_eq!(codex_user_text(line2), Some("yo".into()));
    }
    #[test]
    fn opencode_parser() {
        let v: Value = serde_json::from_str(r#"{"role":"user","parts":[{"text":"hi"}]}"#).unwrap();
        assert_eq!(opencode_user_text(&v), Some("hi".into()));
    }
    #[test]
    fn export_parser() {
        let v: Value = serde_json::from_str(r#"[{"mapping":{"a":{"message":{"author":{"role":"user"},"content":{"parts":["hi"]}}}},"chat_messages":[{"sender":"human","text":"yo"}],"messages":[{"role":"user","content":"hey"}]}]"#).unwrap();
        let msgs = parse_export(&v);
        assert!(msgs.contains(&"hi".to_string()));
        assert!(msgs.contains(&"yo".to_string()));
        assert!(msgs.contains(&"hey".to_string()));
    }
    #[test]
    fn group_topics_dedupe_and_cap() {
        let msgs = vec!["hello\nworld".into(), "hello".into(), "```rust\ncode\n```".into(), "second".into()];
        let t = group_topics(&msgs);
        assert_eq!(t, vec!["hello", "second"]);
    }

    fn scratch_home(name: &str) -> PathBuf {
        let stamp = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos();
        let home = env::temp_dir().join(format!("know-me-{name}-{}-{stamp}", std::process::id()));
        fs::create_dir_all(&home).unwrap();
        home
    }

    fn write_claude_history(home: &Path, n: usize) -> PathBuf {
        let dir = home.join(".claude").join("projects").join("big");
        fs::create_dir_all(&dir).unwrap();
        for i in 0..n {
            fs::write(dir.join(format!("s{i}.jsonl")), format!("{{\"type\":\"user\",\"message\":{{\"content\":\"topic {i}\"}}}}\n")).unwrap();
        }
        dir
    }

    #[cfg(debug_assertions)]
    fn claude_entry(list: &[SourceInfo]) -> (usize, usize) {
        let s = list.iter().find(|s| s.id == "claude_code").unwrap();
        (s.count, s.recounted)
    }

    #[test]
    #[cfg(debug_assertions)]
    fn sources_second_call_comes_from_cache() {
        let home = scratch_home("warm");
        write_claude_history(&home, 2000);
        let started = std::time::Instant::now();
        let first = sources_in(&home);
        let first_ms = started.elapsed().as_millis();
        assert_eq!(claude_entry(&first), (2000, 2000));
        let started = std::time::Instant::now();
        let second = sources_in(&home);
        let second_ms = started.elapsed().as_millis();
        // Счёт файлов, а не время: второй вызов не пересчитал ни одного файла.
        assert_eq!(claude_entry(&second), (2000, 0));
        eprintln!("know_me sources on 2000 files: first {first_ms} ms, second {second_ms} ms");
        fs::remove_dir_all(&home).ok();
    }

    #[test]
    #[cfg(debug_assertions)]
    fn sources_changed_file_is_recounted_alone() {
        let home = scratch_home("touch");
        let dir = write_claude_history(&home, 2000);
        sources_in(&home);
        // Новая длина — новый отпечаток.
        fs::write(dir.join("s5.jsonl"), "{\"type\":\"user\",\"message\":{\"content\":\"changed text here\"}}\n").unwrap();
        assert_eq!(claude_entry(&sources_in(&home)), (2000, 1));
        assert_eq!(claude_entry(&sources_in(&home)), (2000, 0));
        fs::remove_dir_all(&home).ok();
    }

    #[test]
    #[cfg(debug_assertions)]
    fn sources_deleted_file_leaves_the_count() {
        let home = scratch_home("delete");
        let dir = write_claude_history(&home, 2000);
        sources_in(&home);
        fs::remove_file(dir.join("s7.jsonl")).unwrap();
        assert_eq!(claude_entry(&sources_in(&home)), (1999, 0));
        fs::remove_dir_all(&home).ok();
    }

    #[test]
    fn sources_home_override_reads_only_that_folder() {
        let home = scratch_home("empty");
        let list = sources_in(&home);
        let claude = list.iter().find(|s| s.id == "claude_code").unwrap();
        assert!(!claude.found);
        assert_eq!(claude.count, 0);
        fs::remove_dir_all(&home).ok();
    }
}
