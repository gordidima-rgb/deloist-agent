//! Каталог: готовые навыки, агенты и приложения. Один JSON на элемент в папке `catalog/`,
//! встроенной в exe (таблицу собирает build.rs), плюс `index.json` с версиями и хэшами.
//! Формат — `docs/catalog-schema.md`. Манифест ничего не разрешает сам: навык только текст,
//! агент — пресет бота, приложение подключается через модуль коннекторов и согласие человека.

use std::collections::{BTreeMap, HashSet};
use std::sync::{Arc, OnceLock};

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

use crate::connectors_client::{ConnectorError, Connectors};
use crate::store::{now, uid, Bot, Skill};
use crate::App;

const FILES: &[(&str, &str)] = include!(concat!(env!("OUT_DIR"), "/catalog_files.rs"));

pub const KINDS: [&str; 3] = ["skill", "agent", "app"];
pub const CATEGORIES: [&str; 8] = ["productivity", "docs", "research", "business", "marketing", "dev", "personal", "design"];
pub const TIERS: [&str; 2] = ["verified", "public"];
const APPROVALS: [&str; 3] = ["ask", "auto", "strict"];
const RANKS: [&str; 2] = ["lead", "specialist"];
const MODEL_HINTS: [&str; 4] = ["strong", "standard", "fast", "smart"];
const PERSONA_MAX: usize = 12000;
const SPECIALIST_HEADINGS_RU: [&str; 8] = [
    "## Роль",
    "## Когда звать",
    "## Как работаю",
    "## Стандарты качества",
    "## Проверка",
    "## Чего не делаю",
    "## Грабли",
    "## Формат ответа",
];
const SPECIALIST_HEADINGS_EN: [&str; 8] = [
    "## Role",
    "## When to call me",
    "## How I work",
    "## Quality bar",
    "## Self-check",
    "## What I don't do",
    "## Pitfalls",
    "## Answer format",
];

// ---------------------------------------------------------------------------
// Загрузка и проверка
// ---------------------------------------------------------------------------

/// Проверенный элемент каталога.
#[derive(Clone, Debug, Serialize)]
pub struct Item {
    pub id: String,
    pub kind: String,
    pub version: String,
    pub category: String,
    pub tier: String,
    /// sha256 текста файла (переводы строк LF), как в index.json.
    pub hash: String,
    pub manifest: Value,
}

#[derive(Debug, Default)]
pub struct Catalog {
    pub items: Vec<Item>,
    /// Что не прошло проверку: такие элементы в выдачу не попадают.
    pub errors: Vec<String>,
    /// `builtin` — копия внутри exe; иначе путь папки (только для разработки и тестов).
    pub origin: String,
}

impl Catalog {
    pub fn get(&self, id: &str) -> Option<&Item> {
        self.items.iter().find(|item| item.id == id)
    }
}

/// Текст в переводах: строка или `{"ru": …, "en": …}`. Нет нужного языка — русский.
pub fn loc(value: &Value, lang: &str) -> String {
    match value {
        Value::String(s) => s.clone(),
        Value::Object(map) => map
            .get(lang)
            .or_else(|| map.get("ru"))
            .or_else(|| map.values().next())
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string(),
        _ => String::new(),
    }
}

fn has_text(value: Option<&Value>) -> bool {
    value.is_some_and(|v| !loc(v, "ru").trim().is_empty())
}

fn headings_in_order(text: &str, headings: &[&str]) -> bool {
    let mut pos = 0;
    for h in headings {
        match text[pos..].find(h) {
            Some(i) => pos += i + h.len(),
            None => return false,
        }
    }
    true
}

fn validate_triggers(id: &str, triggers: &Value) -> Result<(), String> {
    let Some(obj) = triggers.as_object() else {
        return Err(format!("{id}: agent.triggers должен быть объектом"));
    };
    let files = obj.get("files").ok_or_else(|| format!("{id}: agent.triggers.files обязателен"))?;
    let Some(files) = files.as_array() else {
        return Err(format!("{id}: agent.triggers.files — список строк"));
    };
    if files.is_empty() || files.len() > 40 || !files.iter().all(Value::is_string) {
        return Err(format!("{id}: agent.triggers.files: непустой список строк ≤ 40"));
    }
    let keywords = obj.get("keywords").ok_or_else(|| format!("{id}: agent.triggers.keywords обязателен"))?;
    let Some(kw) = keywords.as_object() else {
        return Err(format!("{id}: agent.triggers.keywords должен быть объектом"));
    };
    for lang in ["ru", "en"] {
        let list = kw.get(lang).ok_or_else(|| format!("{id}: agent.triggers.keywords.{lang} обязателен"))?;
        let Some(list) = list.as_array() else {
            return Err(format!("{id}: agent.triggers.keywords.{lang} — список строк"));
        };
        if list.is_empty() || list.len() > 40 || !list.iter().all(Value::is_string) {
            return Err(format!("{id}: agent.triggers.keywords.{lang}: непустой список строк ≤ 40"));
        }
    }
    Ok(())
}

fn validate_checks(id: &str, checks: &Value) -> Result<(), String> {
    let Some(list) = checks.as_array() else {
        return Err(format!("{id}: agent.checks — список"));
    };
    if list.len() < 2 || list.len() > 3 {
        return Err(format!("{id}: agent.checks: 2–3 элемента"));
    }
    for (i, c) in list.iter().enumerate() {
        let Some(obj) = c.as_object() else {
            return Err(format!("{id}: agent.checks[{i}] должен быть объектом"));
        };
        let Some(task) = obj.get("task").and_then(Value::as_object) else {
            return Err(format!("{id}: agent.checks[{i}].task должен быть объектом {{ru, en}}"));
        };
        for lang in ["ru", "en"] {
            let text = task.get(lang).and_then(Value::as_str).unwrap_or_default();
            if text.trim().is_empty() {
                return Err(format!("{id}: agent.checks[{i}].task.{lang} обязателен"));
            }
        }
        let criteria = obj.get("criteria").ok_or_else(|| format!("{id}: agent.checks[{i}].criteria обязателен"))?;
        let Some(crit) = criteria.as_object() else {
            return Err(format!("{id}: agent.checks[{i}].criteria должен быть объектом"));
        };
        for lang in ["ru", "en"] {
            let arr = crit.get(lang).ok_or_else(|| format!("{id}: agent.checks[{i}].criteria.{lang} обязателен"))?;
            let Some(arr) = arr.as_array() else {
                return Err(format!("{id}: agent.checks[{i}].criteria.{lang} — список строк"));
            };
            if arr.is_empty() || !arr.iter().all(Value::is_string) {
                return Err(format!("{id}: agent.checks[{i}].criteria.{lang} — непустой список строк"));
            }
        }
        let behavior = obj.get("behavior").ok_or_else(|| format!("{id}: agent.checks[{i}].behavior обязателен"))?;
        let Some(behavior) = behavior.as_object() else {
            return Err(format!("{id}: agent.checks[{i}].behavior должен быть объектом"));
        };
        for key in ["require", "forbid"] {
            let Some(patterns) = behavior.get(key).and_then(Value::as_array) else {
                return Err(format!("{id}: agent.checks[{i}].behavior.{key} — список регулярных выражений"));
            };
            if patterns.len() > 4 || (key == "require" && patterns.is_empty()) || !patterns.iter().all(|p| p.as_str().is_some_and(|p| !p.trim().is_empty())) {
                return Err(format!("{id}: agent.checks[{i}].behavior.{key} — непустых строк не больше 4"));
            }
            for pattern in patterns.iter().filter_map(Value::as_str) {
                regex::Regex::new(pattern).map_err(|e| format!("{id}: agent.checks[{i}].behavior.{key}: {pattern}: {e}"))?;
            }
        }
    }
    Ok(())
}

fn str_of<'a>(m: &'a Value, key: &str) -> &'a str {
    m.get(key).and_then(Value::as_str).unwrap_or_default()
}

pub fn hash_text(text: &str) -> String {
    crate::store::hex(&Sha256::digest(text.replace("\r\n", "\n").as_bytes()))
}

fn valid_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 80
        && id.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '.' || c == '-' || c == '_')
        && !id.starts_with('.')
}

/// Проверка одного манифеста без оглядки на соседей.
pub fn validate(m: &Value) -> Result<(), String> {
    if m.get("schema").and_then(Value::as_i64) != Some(1) {
        return Err("schema должна быть 1".into());
    }
    let id = str_of(m, "id");
    if !valid_id(id) {
        return Err(format!("плохой id «{id}»: строчные латинские буквы, цифры, точка, дефис"));
    }
    let kind = str_of(m, "kind");
    if !KINDS.contains(&kind) {
        return Err(format!("{id}: kind должен быть skill, agent или app"));
    }
    if !id.starts_with(&format!("{kind}.")) {
        return Err(format!("{id}: id должен начинаться с «{kind}.»"));
    }
    if str_of(m, "version").trim().is_empty() {
        return Err(format!("{id}: нет version"));
    }
    if !CATEGORIES.contains(&str_of(m, "category")) {
        return Err(format!("{id}: неизвестная category «{}»", str_of(m, "category")));
    }
    if !TIERS.contains(&str_of(m, "tier")) {
        return Err(format!("{id}: tier должен быть verified или public"));
    }
    if !has_text(m.get("title")) {
        return Err(format!("{id}: нет title.ru"));
    }
    if !has_text(m.get("summary")) {
        return Err(format!("{id}: нет summary.ru"));
    }
    let source = m.get("source").cloned().unwrap_or(Value::Null);
    if str_of(&source, "license").trim().is_empty() {
        return Err(format!("{id}: нет source.license"));
    }
    if str_of(&source, "name").trim().is_empty() {
        return Err(format!("{id}: нет source.name"));
    }
    let url = str_of(&source, "url");
    if !url.is_empty() && !url.starts_with("https://") {
        return Err(format!("{id}: source.url должен быть https"));
    }
    match kind {
        "skill" => {
            let body = m.pointer("/skill/body");
            if !has_text(body) {
                return Err(format!("{id}: нет skill.body.ru"));
            }
        }
        "agent" => {
            let agent = m.get("agent").ok_or_else(|| format!("{id}: нет раздела agent"))?;
            if !has_text(agent.get("persona")) {
                return Err(format!("{id}: нет agent.persona"));
            }
            // Длина persona: 12000 символов на язык.
            if let Some(persona) = agent.get("persona") {
                match persona {
                    Value::String(s) => {
                        if s.chars().count() > PERSONA_MAX {
                            return Err(format!("{id}: agent.persona слишком длинный ({} > {PERSONA_MAX})", s.chars().count()));
                        }
                    }
                    Value::Object(map) => {
                        for lang in ["ru", "en"] {
                            if let Some(Value::String(s)) = map.get(lang) {
                                if s.chars().count() > PERSONA_MAX {
                                    return Err(format!("{id}: agent.persona.{lang} слишком длинный ({} > {PERSONA_MAX})", s.chars().count()));
                                }
                            }
                        }
                    }
                    _ => {}
                }
            }
            let approval = str_of(agent, "approval");
            if !approval.is_empty() && !APPROVALS.contains(&approval) {
                return Err(format!("{id}: agent.approval должен быть ask, auto или strict"));
            }
            if let Some(skills) = agent.get("skills") {
                if !skills.as_array().is_some_and(|list| list.iter().all(Value::is_string)) {
                    return Err(format!("{id}: agent.skills — список id навыков"));
                }
            }
            // model_hint
            let hint = str_of(agent, "model_hint");
            if hint.is_empty() || !MODEL_HINTS.contains(&hint) {
                return Err(format!("{id}: agent.model_hint должен быть strong, standard, fast (или старое smart)"));
            }
            // effort (optional, default medium)
            if let Some(effort) = agent.get("effort") {
                if !effort.as_str().is_some_and(|e| crate::providers::EFFORTS.contains(&e)) {
                    return Err(format!("{id}: agent.effort должен быть low, medium или high"));
                }
            }
            // rank (optional, default lead)
            if let Some(rank) = agent.get("rank") {
                if !rank.as_str().is_some_and(|r| RANKS.contains(&r)) {
                    return Err(format!("{id}: agent.rank должен быть lead или specialist"));
                }
            }
            let rank = agent.get("rank").and_then(Value::as_str).unwrap_or("lead");
            // specialist-specific
            if rank == "specialist" {
                // persona headings in order; у специалиста оба языка лежат в объекте persona.
                let Some(persona) = agent.get("persona").and_then(Value::as_object) else {
                    return Err(format!("{id}: у специалиста agent.persona должен быть объектом {{ru, en}}"));
                };
                let ru_text = persona.get("ru").and_then(Value::as_str).unwrap_or_default();
                let en_text = persona.get("en").and_then(Value::as_str).unwrap_or_default();
                if !headings_in_order(ru_text, &SPECIALIST_HEADINGS_RU) {
                    return Err(format!("{id}: agent.persona.ru не содержит заголовков specialist в нужном порядке"));
                }
                if !headings_in_order(en_text, &SPECIALIST_HEADINGS_EN) {
                    return Err(format!("{id}: agent.persona.en не содержит заголовков specialist в нужном порядке"));
                }
                // triggers required
                let triggers = agent.get("triggers").ok_or_else(|| format!("{id}: specialist требует agent.triggers"))?;
                validate_triggers(id, triggers)?;
            }
            // Проверочные задачи нужны и ведущим: релизный прогон гоняет всех ботам каталога.
            let checks = agent.get("checks").ok_or_else(|| format!("{id}: agent требует agent.checks"))?;
            validate_checks(id, checks)?;
        }
        _ => validate_app(id, m.get("app").ok_or_else(|| format!("{id}: нет раздела app"))?)?,
    }
    Ok(())
}

fn validate_app(id: &str, app: &Value) -> Result<(), String> {
    let transport = str_of(app, "transport");
    let cabinet = app.get("web_cabinet");
    if transport.is_empty() {
        let Some(cabinet) = cabinet else {
            return Err(format!("{id}: у приложения нет ни transport, ни web_cabinet"));
        };
        if !str_of(cabinet, "url").starts_with("https://") {
            return Err(format!("{id}: web_cabinet.url должен быть https"));
        }
        return Ok(());
    }
    match transport {
        "http" => {
            if !str_of(app, "url").starts_with("https://") {
                return Err(format!("{id}: app.url должен быть https"));
            }
        }
        "stdio" => {
            if str_of(app, "command").trim().is_empty() {
                return Err(format!("{id}: у stdio-приложения нет command"));
            }
        }
        _ => return Err(format!("{id}: transport должен быть stdio или http")),
    }
    // Версии сторонних пакетов закреплены: `@latest` тянет чужой код в момент запуска.
    let install = app.get("install").cloned().unwrap_or(Value::Null);
    match str_of(&install, "kind") {
        "" | "none" => {}
        "npx" | "uvx" => {
            let pin = str_of(&install, "pin");
            if pin.is_empty() || pin == "latest" || pin.contains('*') || str_of(&install, "package").is_empty() {
                return Err(format!("{id}: npx/uvx без закреплённой версии (install.package, install.pin)"));
            }
        }
        "binary" => {
            let sha = str_of(&install, "sha256");
            if sha.len() != 64 || !sha.chars().all(|c| c.is_ascii_hexdigit()) {
                return Err(format!("{id}: у бинаря нет sha256"));
            }
        }
        other => return Err(format!("{id}: неизвестный install.kind «{other}»")),
    }
    if let Some(fields) = app.pointer("/auth/fields") {
        let Some(fields) = fields.as_array() else {
            return Err(format!("{id}: auth.fields — список"));
        };
        for field in fields {
            if str_of(field, "name").trim().is_empty() || !has_text(field.get("label")) {
                return Err(format!("{id}: у поля auth.fields нет name или label"));
            }
        }
    }
    Ok(())
}

/// Собрать каталог из файлов (путь, текст). Ошибочные элементы отбрасываются с записью в errors.
pub fn build(files: &[(String, String)], origin: &str) -> Catalog {
    let mut catalog = Catalog { origin: origin.to_string(), ..Catalog::default() };
    let mut index: Option<Value> = None;
    let mut seen = HashSet::new();
    let mut paths: BTreeMap<String, String> = BTreeMap::new();
    for (path, text) in files {
        let value: Value = match serde_json::from_str(text) {
            Ok(v) => v,
            Err(e) => {
                catalog.errors.push(format!("{path}: не JSON: {e}"));
                continue;
            }
        };
        // index.json — публичный индекс, index.private.json — индекс private/catalog (только официальная сборка): сверяем их сумму.
        if path == "index.json" || path == "index.private.json" {
            match index.as_mut().and_then(|i: &mut Value| i.get_mut("items")).and_then(Value::as_array_mut) {
                Some(items) => items.extend(value.get("items").and_then(Value::as_array).cloned().unwrap_or_default()),
                None => index = Some(value),
            }
            continue;
        }
        if let Err(e) = validate(&value) {
            catalog.errors.push(format!("{path}: {e}"));
            continue;
        }
        // Нормализация на загрузке: старые model_hint → новые, необязательные поля получают
        // значение по умолчанию, чтобы остальное приложение видело только новые значения.
        let mut value = value;
        if value.get("kind").and_then(Value::as_str) == Some("agent") {
            if let Some(agent) = value.get_mut("agent").and_then(Value::as_object_mut) {
                if let Some(hint) = agent.get("model_hint").and_then(Value::as_str) {
                    let norm = match hint {
                        "smart" => "strong",
                        other => other,
                    };
                    agent.insert("model_hint".to_string(), Value::String(norm.to_string()));
                }
                if !agent.contains_key("effort") {
                    agent.insert("effort".to_string(), Value::String("medium".to_string()));
                }
                if !agent.contains_key("rank") {
                    agent.insert("rank".to_string(), Value::String("lead".to_string()));
                }
            }
        }
        let id = str_of(&value, "id").to_string();
        if !seen.insert(id.clone()) {
            catalog.errors.push(format!("{path}: id «{id}» уже занят"));
            continue;
        }
        paths.insert(id.clone(), path.clone());
        catalog.items.push(Item {
            kind: str_of(&value, "kind").into(),
            version: str_of(&value, "version").into(),
            category: str_of(&value, "category").into(),
            tier: str_of(&value, "tier").into(),
            hash: hash_text(text),
            id,
            manifest: value,
        });
    }
    // Агент ссылается только на навыки, которые есть в этом же каталоге.
    let skills: HashSet<String> = catalog.items.iter().filter(|i| i.kind == "skill").map(|i| i.id.clone()).collect();
    let mut broken = Vec::new();
    for item in catalog.items.iter().filter(|i| i.kind == "agent") {
        for skill in item.manifest.pointer("/agent/skills").and_then(Value::as_array).into_iter().flatten() {
            let skill = skill.as_str().unwrap_or_default();
            if !skills.contains(skill) {
                broken.push((item.id.clone(), format!("{}: агент ссылается на навык «{skill}», которого нет в каталоге", item.id)));
            }
        }
    }
    for (id, message) in broken {
        catalog.items.retain(|i| i.id != id);
        catalog.errors.push(message);
    }
    if let Some(index) = index {
        check_index(&mut catalog, &index);
    }
    catalog.items.sort_by(|a, b| a.id.cmp(&b.id));
    catalog
}

/// index.json перечисляет те же элементы, с теми же версиями и хэшами.
fn check_index(catalog: &mut Catalog, index: &Value) {
    let listed: Vec<&Value> = index.get("items").and_then(Value::as_array).map(|a| a.iter().collect()).unwrap_or_default();
    let mut ids = HashSet::new();
    for entry in &listed {
        let id = str_of(entry, "id");
        ids.insert(id.to_string());
        match catalog.items.iter().find(|i| i.id == id) {
            None => catalog.errors.push(format!("index.json: «{id}» есть в индексе, но не в каталоге")),
            Some(item) => {
                if str_of(entry, "version") != item.version {
                    catalog.errors.push(format!("index.json: у «{id}» другая версия"));
                }
                let hash = str_of(entry, "hash");
                if !hash.is_empty() && hash != item.hash {
                    catalog.errors.push(format!("index.json: у «{id}» другой хэш (node tools/catalog_index.mjs)"));
                }
            }
        }
    }
    for item in &catalog.items {
        if !ids.contains(&item.id) {
            catalog.errors.push(format!("index.json: «{}» нет в индексе", item.id));
        }
    }
}

fn embedded_files() -> Vec<(String, String)> {
    FILES.iter().map(|(path, text)| (path.to_string(), text.to_string())).collect()
}

fn dir_files(root: &std::path::Path) -> Vec<(String, String)> {
    fn walk(dir: &std::path::Path, root: &std::path::Path, out: &mut Vec<(String, String)>) {
        let Ok(entries) = std::fs::read_dir(dir) else { return };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                walk(&path, root, out);
            } else if path.extension().and_then(|e| e.to_str()) == Some("json") {
                if let Ok(text) = std::fs::read_to_string(&path) {
                    let rel = path.strip_prefix(root).unwrap_or(&path).to_string_lossy().replace('\\', "/");
                    out.push((rel, text));
                }
            }
        }
    }
    let mut out = Vec::new();
    walk(root, root, &mut out);
    out.sort();
    out
}

static CATALOG: OnceLock<Catalog> = OnceLock::new();

/// Каталог этой копии. В debug-сборке `CREW_CATALOG_DIR` подменяет встроенный целиком:
/// так тесты проверяют обновление, а разработчик — 500 элементов.
pub fn catalog() -> &'static Catalog {
    CATALOG.get_or_init(|| {
        let catalog = match crate::update::test_env("CREW_CATALOG_DIR") {
            Some(dir) => build(&dir_files(std::path::Path::new(&dir)), &dir),
            None => build(&embedded_files(), "builtin"),
        };
        for error in &catalog.errors {
            eprintln!("каталог: {error}");
        }
        catalog
    })
}

// ---------------------------------------------------------------------------
// Обновление с deloist.ru (крючок, скачивание ещё не сделано)
// ---------------------------------------------------------------------------

/// Подписанный индекс с сайта: `index.json` и подпись ed25519 тем же ключом, что у выпусков.
/// Скачивание (ETag, офлайн = встроенная копия) появится позже; когда появится, оно должно:
/// 1) проверить подпись этой функцией, 2) скачать файлы и сверить каждый с хэшем из индекса,
/// 3) собрать каталог через `build` и показать человеку diff текстов перед «Обновить».
#[allow(dead_code)]
pub fn verify_signed_index(index: &[u8], signature_hex: &[u8]) -> Result<Value, String> {
    crate::update::verify(index, signature_hex)?;
    serde_json::from_slice(index).map_err(|e| format!("index.json не JSON: {e}"))
}

// ---------------------------------------------------------------------------
// Установки: что именно добавил каталог
// ---------------------------------------------------------------------------

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Install {
    pub kind: String,
    pub version: String,
    /// Язык текстов при установке: обновление берёт тот же.
    pub lang: String,
    pub at: i64,
    /// Навык: id в state.skills.
    pub skill: String,
    /// Агент: созданный бот, навыки, которых до установки не было, разрешённые сайты.
    pub bot: String,
    pub skills_added: Vec<String>,
    pub sites_added: Vec<String>,
    /// Приложение: id подключения в модуле коннекторов и его состояние.
    pub connector: String,
    pub status: String,
}

#[derive(Debug, Default, Serialize, Deserialize)]
#[serde(default)]
struct InstallsFile {
    items: BTreeMap<String, Install>,
}

/// Учёт установок в `catalog-installs.json`. Все операции каталога идут по очереди под этим замком:
/// двойной щелчок «Добавить» не создаёт двух копий.
#[derive(Default)]
pub struct Installs {
    file: tokio::sync::Mutex<Option<InstallsFile>>,
}

fn installs_path(app: &App) -> std::path::PathBuf {
    app.store.data_dir().join("catalog-installs.json")
}

async fn load_installs(app: &App) -> InstallsFile {
    match tokio::fs::read_to_string(installs_path(app)).await {
        Ok(text) => serde_json::from_str(&text).unwrap_or_default(),
        Err(_) => InstallsFile::default(),
    }
}

async fn save_installs(app: &App, file: &InstallsFile) -> Result<(), Failure> {
    let text = serde_json::to_string_pretty(file).map_err(|e| Failure::internal(e.to_string()))?;
    let path = installs_path(app);
    let tmp = path.with_extension("json.tmp");
    tokio::fs::write(&tmp, text).await.map_err(|e| Failure::internal(e.to_string()))?;
    tokio::fs::rename(&tmp, &path).await.map_err(|e| Failure::internal(e.to_string()))
}

/// Ошибка операции каталога с HTTP-статусом для API.
#[derive(Debug)]
pub struct Failure {
    pub status: u16,
    pub message: String,
    /// Подробности для интерфейса: diff при конфликте правок, карточка согласия и т. п.
    pub data: Value,
}

impl Failure {
    fn new(status: u16, message: impl Into<String>) -> Self {
        Self { status, message: message.into(), data: Value::Null }
    }
    fn internal(message: impl Into<String>) -> Self {
        Self::new(500, message)
    }
    fn io(err: std::io::Error) -> Self {
        Self::internal(err.to_string())
    }
}

impl From<ConnectorError> for Failure {
    fn from(err: ConnectorError) -> Self {
        let status = if err.status == 0 { 502 } else { err.status };
        Self { status, message: err.message, data: err.data }
    }
}

/// Содержимое навыка, по которому видно, правили ли его.
fn skill_hash(name: &str, description: &str, body: &str) -> String {
    hash_text(&format!("{name}\n{description}\n{body}"))
}

fn bot_hash(role: &str, persona: &str) -> String {
    hash_text(&format!("{role}\n{persona}"))
}

struct SkillText {
    name: String,
    description: String,
    body: String,
}

fn skill_text(item: &Item, lang: &str) -> SkillText {
    let m = &item.manifest;
    SkillText {
        name: loc(&m["title"], lang).chars().take(60).collect(),
        description: loc(&m["summary"], lang).chars().take(160).collect(),
        body: loc(&m["skill"]["body"], lang).chars().take(20_000).collect(),
    }
}

fn lang_of(body: &Value) -> String {
    match str_of(body, "lang") {
        "en" => "en".into(),
        _ => "ru".into(),
    }
}

fn catalog_skill(app: &App, catalog_id: &str) -> Option<Skill> {
    app.store.skills().into_iter().find(|s| s.catalog_id == catalog_id)
}

fn catalog_extra(bot: &Bot, key: &str) -> String {
    bot.extra.get(key).and_then(Value::as_str).unwrap_or_default().to_string()
}

// ---------------------------------------------------------------------------
// Выдача
// ---------------------------------------------------------------------------

#[derive(Debug, Default, Deserialize)]
pub struct Query {
    pub kind: Option<String>,
    pub category: Option<String>,
    pub tier: Option<String>,
    pub q: Option<String>,
}

fn matches_text(item: &Item, words: &[String]) -> bool {
    if words.is_empty() {
        return true;
    }
    let m = &item.manifest;
    let hay = [
        loc(&m["title"], "ru"),
        loc(&m["title"], "en"),
        loc(&m["summary"], "ru"),
        loc(&m["summary"], "en"),
        str_of(&m["source"], "name").to_string(),
        item.id.clone(),
    ]
    .join(" ")
    .to_lowercase();
    words.iter().all(|w| hay.contains(w.as_str()))
}

/// Что из каталога уже стоит у человека: по элементу — версия, правки, боты.
fn installed_state(app: &App, item: &Item, file: &InstallsFile) -> Value {
    let record = file.items.get(&item.id);
    match item.kind.as_str() {
        "skill" => match catalog_skill(app, &item.id) {
            Some(skill) => {
                let bots: Vec<String> = app.store.bots().into_iter().filter(|b| b.skills.contains(&skill.id)).map(|b| b.id).collect();
                json!({
                    "installed": true,
                    "version": skill.catalog_version,
                    "update": skill.catalog_version != item.version,
                    "edited": skill_hash(&skill.name, &skill.description, &skill.body) != skill.catalog_hash,
                    "skillId": skill.id,
                    "bots": bots,
                })
            }
            None => json!({ "installed": false }),
        },
        "agent" => match record.and_then(|r| app.store.bot(&r.bot)) {
            Some(bot) => {
                let version = catalog_extra(&bot, "catalogVersion");
                json!({
                    "installed": true,
                    "version": version,
                    "update": version != item.version,
                    "edited": bot_hash(&bot.role, &bot.persona) != catalog_extra(&bot, "catalogHash"),
                    "botId": bot.id,
                })
            }
            None => json!({ "installed": false }),
        },
        _ => match record {
            Some(r) if !r.bot.is_empty() && app.store.bot(&r.bot).is_some() => {
                json!({ "installed": true, "version": r.version, "update": r.version != item.version, "botId": r.bot, "status": "ready" })
            }
            Some(r) if !r.connector.is_empty() => json!({
                "installed": r.status == "ready",
                "version": r.version,
                "update": r.status == "ready" && r.version != item.version,
                "connectorId": r.connector,
                "status": r.status,
            }),
            _ => json!({ "installed": false }),
        },
    }
}

/// Что из каталога уже стоит у человека: только чтение (для поиска ботами, `team_proposal`).
pub async fn state_of(app: &App, item: &Item) -> Value {
    let file = load_installs(app).await;
    installed_state(app, item, &file)
}

pub async fn list(app: &App, query: &Query) -> Value {
    let catalog = catalog();
    let words: Vec<String> = query.q.as_deref().unwrap_or_default().to_lowercase().split_whitespace().map(str::to_string).collect();
    let filter = |value: &Option<String>, actual: &str| value.as_deref().filter(|v| !v.is_empty() && *v != "all").is_none_or(|v| v == actual);
    let file = load_installs(app).await;
    let items: Vec<Value> = catalog
        .items
        .iter()
        .filter(|item| filter(&query.kind, &item.kind) && filter(&query.category, &item.category) && filter(&query.tier, &item.tier))
        .filter(|item| matches_text(item, &words))
        .map(|item| {
            let mut out = item.manifest.clone();
            out["hash"] = json!(item.hash);
            out["state"] = installed_state(app, item, &file);
            if item.kind == "app" {
                out["mode"] = json!(app_mode(item));
            }
            out
        })
        .collect();
    json!({
        "items": items,
        "categories": CATEGORIES,
        "origin": catalog.origin,
        "errors": catalog.errors.len(),
    })
}

/// `connector` — подключается через модуль коннекторов, `cabinet` — только сайт в браузере бота.
fn app_mode(item: &Item) -> &'static str {
    if str_of(&item.manifest["app"], "transport").is_empty() {
        "cabinet"
    } else {
        "connector"
    }
}

/// Карточка согласия из манифеста: что запустится, куда уйдёт ключ, что спросим.
pub fn consent_card(item: &Item, lang: &str) -> Value {
    let app = &item.manifest["app"];
    let fields: Vec<Value> = app
        .pointer("/auth/fields")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .map(|f| {
            json!({
                "name": str_of(f, "name"),
                "label": loc(&f["label"], lang),
                "secret": f.get("secret").and_then(Value::as_bool).unwrap_or(true),
                "help": loc(&f["help"], lang),
                "helpUrl": str_of(f, "help_url"),
            })
        })
        .collect();
    json!({
        "title": loc(&item.manifest["title"], lang),
        "transport": str_of(app, "transport"),
        "command": str_of(app, "command"),
        "args": app.get("args").cloned().unwrap_or(json!([])),
        "url": str_of(app, "url"),
        "install": app.get("install").cloned().unwrap_or(json!({ "kind": "none" })),
        "egress": app.get("egress").cloned().unwrap_or(json!([])),
        "auth": {
            "kind": str_of(&app["auth"], "kind"),
            "fields": fields,
            "oauth": app.pointer("/auth/oauth").cloned().unwrap_or(Value::Null),
        },
        "toolsHint": app.get("tools_hint").cloned().unwrap_or(json!([])),
        "autoApprove": false,
    })
}

// ---------------------------------------------------------------------------
// Установка, удаление, обновление
// ---------------------------------------------------------------------------

fn need_item(id: &str) -> Result<&'static Item, Failure> {
    catalog().get(id).ok_or_else(|| Failure::new(404, "в каталоге нет такого элемента"))
}

fn bot_list(app: &App, body: &Value) -> Result<Vec<String>, Failure> {
    let ids: Vec<String> = body.get("bots").and_then(Value::as_array).into_iter().flatten().filter_map(Value::as_str).map(str::to_string).collect();
    for id in &ids {
        if app.store.bot(id).is_none() {
            return Err(Failure::new(400, "такого бота нет"));
        }
    }
    Ok(ids)
}

/// Навык каталога в хранилище: уже есть — тот же, нет — новый. Второе значение: создан ли сейчас.
async fn ensure_skill(app: &App, item: &Item, lang: &str) -> Result<(Skill, bool), Failure> {
    if let Some(skill) = catalog_skill(app, &item.id) {
        return Ok((skill, false));
    }
    let text = skill_text(item, lang);
    let skill = Skill {
        id: uid("sk"),
        catalog_hash: skill_hash(&text.name, &text.description, &text.body),
        name: text.name,
        description: text.description,
        body: text.body,
        created_at: now(),
        updated_at: now(),
        origin: "catalog".into(),
        catalog_id: item.id.clone(),
        catalog_version: item.version.clone(),
        ..Skill::default()
    };
    Ok((app.store.push_skill(skill).await.map_err(Failure::io)?, true))
}

async fn attach_skill(app: &App, bot_id: &str, skill_id: &str) -> Result<(), Failure> {
    let skill_id = skill_id.to_string();
    app.store
        .edit_bot(bot_id, |bot| {
            if !bot.skills.contains(&skill_id) {
                bot.skills.push(skill_id);
            }
        })
        .await
        .map_err(Failure::io)?;
    Ok(())
}

pub async fn install(app: &Arc<App>, id: &str, body: &Value) -> Result<Value, Failure> {
    let item = need_item(id)?;
    let mut guard = app.catalog.file.lock().await;
    if guard.is_none() {
        *guard = Some(load_installs(app).await);
    }
    let file = guard.as_mut().unwrap();
    let lang = lang_of(body);
    let result = match item.kind.as_str() {
        "skill" => install_skill(app, item, body, &lang, file).await?,
        "agent" => install_agent(app, item, body, &lang, file).await?,
        _ => install_app(app, item, body, &lang, file).await?,
    };
    save_installs(app, file).await?;
    app.turns.emit(json!({ "type": "state_changed" }));
    Ok(result)
}

async fn install_skill(app: &App, item: &Item, body: &Value, lang: &str, file: &mut InstallsFile) -> Result<Value, Failure> {
    let bots = bot_list(app, body)?;
    let (skill, _) = ensure_skill(app, item, lang).await?;
    for bot in &bots {
        attach_skill(app, bot, &skill.id).await?;
    }
    let record = file.items.entry(item.id.clone()).or_default();
    record.kind = "skill".into();
    record.skill = skill.id.clone();
    if record.version.is_empty() {
        record.version = skill.catalog_version.clone();
        record.lang = lang.into();
        record.at = now();
    }
    Ok(json!({ "skill": skill, "bots": bots }))
}

/// Модель не навязывается: пусто — модель человека по умолчанию; подсказку видно в профиле.
async fn install_agent(app: &App, item: &Item, body: &Value, lang: &str, file: &mut InstallsFile) -> Result<Value, Failure> {
    if let Some(bot) = file.items.get(&item.id).and_then(|r| app.store.bot(&r.bot)) {
        return Ok(json!({ "bot": bot, "already": true }));
    }
    let agent = &item.manifest["agent"];
    let mut skill_ids = Vec::new();
    let mut skills_added = Vec::new();
    for skill_ref in agent["skills"].as_array().into_iter().flatten().filter_map(Value::as_str) {
        let skill_item = need_item(skill_ref)?;
        let (skill, created) = ensure_skill(app, skill_item, lang).await?;
        if created {
            skills_added.push(skill.id.clone());
            file.items.insert(
                skill_item.id.clone(),
                Install { kind: "skill".into(), version: skill_item.version.clone(), lang: lang.into(), at: now(), skill: skill.id.clone(), ..Install::default() },
            );
        }
        skill_ids.push(skill.id);
    }
    let avatar = &agent["avatar"];
    let role = loc(&agent["role"], lang);
    let persona = loc(&agent["persona"], lang);
    // `forceApproval: "ask"` — поставка по карточке команды: режим из манифеста не берётся вовсе.
    // Иначе агент из каталога ставится с «спрашивать» или «строго»: «решай сам» из описания каталога
    // не принимаем, этот режим включает только человек (ключ программы тоже может ставить каталог).
    let approval = match (str_of(body, "forceApproval"), str_of(agent, "approval")) {
        ("ask", _) => "ask",
        (_, "strict") => "strict",
        _ => "ask",
    };
    let mut input = json!({
        "name": loc(&item.manifest["title"], lang),
        "role": role,
        "persona": persona,
        "emoji": str_of(avatar, "emoji"),
        "color": avatar.get("color").and_then(Value::as_str).filter(|c| valid_color(c)).unwrap_or("#7a8595"),
        "approval": approval,
        "skills": skill_ids,
        "driver": str_of(body, "driver"),
        "model": str_of(body, "model"),
        // Подпись каталога: лид или специалист. Старые манифесты без rank остаются лидами.
        "rank": agent.get("rank").and_then(Value::as_str).unwrap_or("lead"),
    });
    for key in ["skin"] {
        if let Some(v) = avatar.get(key).filter(|v| v.is_i64()) {
            input[key] = v.clone();
        }
    }
    let bot = app.store.add_bot(&input).await.map_err(Failure::io)?;
    // Сайты агента — только с согласия человека: без него бот там ничего не нажимает.
    let wanted: Vec<String> = agent["sites"].as_array().into_iter().flatten().filter_map(Value::as_str).map(str::to_string).collect();
    let sites_added = if body.get("sites").and_then(Value::as_bool) == Some(true) { wanted } else { Vec::new() };
    let (stored_role, stored_persona) = (bot.role.clone(), bot.persona.clone());
    let version = item.version.clone();
    // Манифест уже нормализован на загрузке: здесь только strong / standard / fast и low / medium / high.
    let level = str_of(agent, "model_hint").to_string();
    let effort = str_of(agent, "effort").to_string();
    let catalog_id = item.id.clone();
    let sites = sites_added.clone();
    let bot = app
        .store
        .edit_bot(&bot.id, move |bot| {
            for site in sites {
                if !bot.browser_sites.contains(&site) {
                    bot.browser_sites.push(site);
                }
            }
            bot.extra.insert("catalogId".into(), json!(catalog_id));
            bot.extra.insert("catalogVersion".into(), json!(version));
            bot.extra.insert("catalogHash".into(), json!(bot_hash(&stored_role, &stored_persona)));
            if !level.is_empty() {
                bot.extra.insert("modelLevel".into(), json!(level));
            }
            if !effort.is_empty() {
                bot.extra.insert("effort".into(), json!(effort));
            }
        })
        .await
        .map_err(Failure::io)?
        .ok_or_else(|| Failure::internal("бот пропал сразу после создания"))?;
    file.items.insert(
        item.id.clone(),
        Install {
            kind: "agent".into(),
            version: item.version.clone(),
            lang: lang.into(),
            at: now(),
            bot: bot.id.clone(),
            skills_added,
            sites_added,
            ..Install::default()
        },
    );
    Ok(json!({ "bot": bot }))
}

fn valid_color(c: &str) -> bool {
    c.len() == 7 && c.starts_with('#') && c[1..].chars().all(|ch| ch.is_ascii_hexdigit())
}

async fn install_app(app: &Arc<App>, item: &Item, body: &Value, lang: &str, file: &mut InstallsFile) -> Result<Value, Failure> {
    if app_mode(item) == "cabinet" {
        return install_cabinet(app, item, body, lang, file).await;
    }
    let card = consent_card(item, lang);
    if let Some(record) = file.items.get(&item.id).filter(|r| !r.connector.is_empty()) {
        return Ok(json!({ "card": card, "connector": record.connector, "status": record.status, "already": true }));
    }
    let client = Connectors::from_env();
    let installed = client.install(&item.id, &item.manifest).await.map_err(|e| {
        let mut failure = Failure::from(e);
        failure.data = json!({ "card": card });
        failure
    })?;
    file.items.insert(
        item.id.clone(),
        Install {
            kind: "app".into(),
            version: item.version.clone(),
            lang: lang.into(),
            at: now(),
            connector: installed.id.clone(),
            status: "pending".into(),
            ..Install::default()
        },
    );
    Ok(json!({ "card": card, "remote": installed.card, "connector": installed.id, "status": "pending" }))
}

/// «Кабинет»: сайт добавляется в разрешённые сайты выбранного бота, только с согласия.
async fn install_cabinet(app: &App, item: &Item, body: &Value, lang: &str, file: &mut InstallsFile) -> Result<Value, Failure> {
    let bot_id = str_of(body, "bot").to_string();
    if app.store.bot(&bot_id).is_none() {
        return Err(Failure::new(400, "выбери бота, который будет работать в кабинете"));
    }
    if body.get("consent").and_then(Value::as_bool) != Some(true) {
        return Err(Failure::new(400, "нужно согласие: бот будет действовать на этом сайте"));
    }
    if file.items.get(&item.id).is_some_and(|r| r.bot == bot_id) {
        return Ok(json!({ "bot": app.store.bot(&bot_id), "already": true }));
    }
    // Кабинет переезжает к другому боту: прежнему возвращаем его сайты.
    if let Some(old) = file.items.get(&item.id).filter(|r| !r.bot.is_empty()).cloned() {
        let sites = old.sites_added.clone();
        app.store.edit_bot(&old.bot, move |bot| bot.browser_sites.retain(|s| !sites.contains(s))).await.map_err(Failure::io)?;
    }
    let cabinet = &item.manifest["app"]["web_cabinet"];
    let mut sites: Vec<String> = cabinet["sites"].as_array().into_iter().flatten().filter_map(Value::as_str).map(str::to_string).collect();
    if sites.is_empty() {
        if let Some(host) = host_of(str_of(cabinet, "url")) {
            sites.push(host);
        }
    }
    let before = app.store.bot(&bot_id).map(|b| b.browser_sites).unwrap_or_default();
    let added: Vec<String> = sites.into_iter().filter(|s| !before.contains(s)).collect();
    let to_add = added.clone();
    let bot = app
        .store
        .edit_bot(&bot_id, move |bot| bot.browser_sites.extend(to_add))
        .await
        .map_err(Failure::io)?;
    file.items.insert(
        item.id.clone(),
        Install { kind: "app".into(), version: item.version.clone(), lang: lang.into(), at: now(), bot: bot_id, sites_added: added, status: "ready".into(), ..Install::default() },
    );
    Ok(json!({ "bot": bot, "url": str_of(cabinet, "url") }))
}

fn host_of(url: &str) -> Option<String> {
    let rest = url.strip_prefix("https://")?;
    let host = rest.split(['/', '?', '#']).next()?.trim_start_matches("www.");
    (!host.is_empty()).then(|| host.to_string())
}

/// Шаг подключения приложения: `confirm`, `secret`, `oauth`, `check`. Секрет каталог не хранит и не
/// пишет в журнал: он уходит прямо в модуль коннекторов.
pub async fn connect(app: &Arc<App>, id: &str, body: &Value) -> Result<Value, Failure> {
    let item = need_item(id)?;
    if item.kind != "app" || app_mode(item) != "connector" {
        return Err(Failure::new(400, "этот элемент не подключается как приложение"));
    }
    let mut guard = app.catalog.file.lock().await;
    if guard.is_none() {
        *guard = Some(load_installs(app).await);
    }
    let file = guard.as_mut().unwrap();
    let Some(record) = file.items.get_mut(&item.id).filter(|r| !r.connector.is_empty()) else {
        return Err(Failure::new(409, "сначала нажми «Подключить»"));
    };
    let client = Connectors::from_env();
    let connector = record.connector.clone();
    let result = match str_of(body, "step") {
        "confirm" => client.confirm(&connector).await?,
        "secret" => {
            let name = str_of(body, "name");
            let allowed = item.manifest.pointer("/app/auth/fields").and_then(Value::as_array).into_iter().flatten().any(|f| str_of(f, "name") == name);
            if !allowed {
                return Err(Failure::new(400, "у этого приложения нет такого поля"));
            }
            client.secret(&connector, name, str_of(body, "value")).await?
        }
        "oauth" => {
            let mut login = json!({});
            for key in ["clientId", "clientSecret", "redirectPort"] {
                if let Some(value) = body.get(key).filter(|v| v.is_string() || v.is_number()) {
                    login[key] = value.clone();
                }
            }
            let (url, host) = client.oauth_start(&connector, login).await?;
            json!({ "url": url, "authHost": host })
        }
        "check" => {
            let tools = client.check(&connector).await?;
            record.status = "ready".into();
            json!({ "tools": tools })
        }
        _ => return Err(Failure::new(400, "шаг должен быть confirm, secret, oauth или check")),
    };
    save_installs(app, file).await?;
    if record_ready(file, &item.id) {
        app.turns.emit(json!({ "type": "state_changed" }));
    }
    Ok(result)
}

fn record_ready(file: &InstallsFile, id: &str) -> bool {
    file.items.get(id).is_some_and(|r| r.status == "ready")
}

pub async fn uninstall(app: &Arc<App>, id: &str) -> Result<Value, Failure> {
    let item = need_item(id)?;
    let mut guard = app.catalog.file.lock().await;
    if guard.is_none() {
        *guard = Some(load_installs(app).await);
    }
    let file = guard.as_mut().unwrap();
    let record = file.items.remove(&item.id).unwrap_or_default();
    let mut removed = json!({});
    match item.kind.as_str() {
        "skill" => {
            if let Some(skill) = catalog_skill(app, &item.id) {
                app.store.remove_skill(&skill.id).await.map_err(Failure::io)?;
                removed["skill"] = json!(skill.id);
            }
        }
        "agent" => {
            if !record.bot.is_empty() && app.store.bot(&record.bot).is_some() {
                crate::server::drop_bot(app, &record.bot).await.map_err(Failure::io)?;
                removed["bot"] = json!(record.bot);
            }
            // Навыки, которые принёс агент, уходят, только если ими больше никто не пользуется.
            let mut gone = Vec::new();
            for skill_id in &record.skills_added {
                let used = app.store.bots().iter().any(|b| b.skills.contains(skill_id))
                    || file.items.values().any(|r| r.kind == "agent" && r.skills_added.contains(skill_id));
                if !used && app.store.remove_skill(skill_id).await.map_err(Failure::io)? {
                    gone.push(skill_id.clone());
                }
            }
            file.items.retain(|_, r| !(r.kind == "skill" && gone.contains(&r.skill)));
            removed["skills"] = json!(gone);
        }
        _ => {
            if !record.connector.is_empty() {
                Connectors::from_env().remove(&record.connector).await?;
                removed["connector"] = json!(record.connector);
            }
            if !record.bot.is_empty() {
                let sites = record.sites_added.clone();
                app.store.edit_bot(&record.bot, move |bot| bot.browser_sites.retain(|s| !sites.contains(s))).await.map_err(Failure::io)?;
                removed["sites"] = json!(record.sites_added);
            }
        }
    }
    save_installs(app, file).await?;
    app.turns.emit(json!({ "type": "state_changed" }));
    Ok(json!({ "removed": removed }))
}

/// Обновить до версии каталога. Правки человека не затираются: если текст меняли, без `resolve`
/// приходит 409 с обеими версиями; `resolve: "mine"` оставляет правку, `"theirs"` берёт новую,
/// а правка человека уходит в историю навыка (её можно вернуть «Отменить»).
pub async fn update(app: &Arc<App>, id: &str, body: &Value) -> Result<Value, Failure> {
    let item = need_item(id)?;
    let resolve = str_of(body, "resolve").to_string();
    if !["", "mine", "theirs"].contains(&resolve.as_str()) {
        return Err(Failure::new(400, "resolve должен быть mine или theirs"));
    }
    let mut guard = app.catalog.file.lock().await;
    if guard.is_none() {
        *guard = Some(load_installs(app).await);
    }
    let file = guard.as_mut().unwrap();
    let lang = file.items.get(&item.id).map(|r| r.lang.clone()).filter(|l| !l.is_empty()).unwrap_or_else(|| lang_of(body));
    let result = match item.kind.as_str() {
        "skill" => {
            let skill = catalog_skill(app, &item.id).ok_or_else(|| Failure::new(409, "навык не установлен"))?;
            let outcome = plan_skill(&skill, item, &lang);
            if !outcome.conflict.is_null() && resolve.is_empty() {
                return Err(conflict(vec![outcome.conflict]));
            }
            let skill = apply_skill(app, &skill, item, &lang, &resolve).await?;
            json!({ "skill": skill })
        }
        "agent" => update_agent(app, item, &lang, &resolve, file).await?,
        _ => {
            // У приложения нечего править руками: новая версия манифеста просто записывается.
            json!({ "ok": true })
        }
    };
    if let Some(record) = file.items.get_mut(&item.id) {
        record.version = item.version.clone();
    }
    save_installs(app, file).await?;
    app.turns.emit(json!({ "type": "state_changed" }));
    Ok(result)
}

fn conflict(conflicts: Vec<Value>) -> Failure {
    Failure {
        status: 409,
        message: "текст правили: выбери, оставить свою версию или взять новую".into(),
        data: json!({ "conflicts": conflicts }),
    }
}

struct SkillPlan {
    conflict: Value,
}

fn plan_skill(skill: &Skill, item: &Item, lang: &str) -> SkillPlan {
    let edited = skill_hash(&skill.name, &skill.description, &skill.body) != skill.catalog_hash;
    let theirs = skill_text(item, lang);
    let same = skill.body == theirs.body && skill.name == theirs.name && skill.description == theirs.description;
    let conflict = if edited && !same {
        json!({
            "what": "skill",
            "id": item.id,
            "title": theirs.name,
            "mine": { "name": skill.name, "description": skill.description, "body": skill.body },
            "theirs": { "name": theirs.name, "description": theirs.description, "body": theirs.body },
        })
    } else {
        Value::Null
    };
    SkillPlan { conflict }
}

async fn apply_skill(app: &App, skill: &Skill, item: &Item, lang: &str, resolve: &str) -> Result<Skill, Failure> {
    let edited = skill_hash(&skill.name, &skill.description, &skill.body) != skill.catalog_hash;
    let keep_mine = edited && resolve == "mine";
    let theirs = skill_text(item, lang);
    let version = item.version.clone();
    let updated = app
        .store
        .edit_skill(&skill.id, move |s| {
            s.catalog_version = version;
            if keep_mine {
                return;
            }
            if s.body != theirs.body || s.name != theirs.name || s.description != theirs.description {
                s.previous.push(crate::store::SkillVersion { name: s.name.clone(), description: s.description.clone(), body: s.body.clone(), at: now() });
            }
            s.catalog_hash = skill_hash(&theirs.name, &theirs.description, &theirs.body);
            s.name = theirs.name;
            s.description = theirs.description;
            s.body = theirs.body;
            s.updated_at = now();
        })
        .await
        .map_err(Failure::io)?;
    updated.ok_or_else(|| Failure::new(404, "навык не найден"))
}

async fn update_agent(app: &App, item: &Item, lang: &str, resolve: &str, file: &mut InstallsFile) -> Result<Value, Failure> {
    let record = file.items.get(&item.id).cloned().ok_or_else(|| Failure::new(409, "агент не установлен"))?;
    let bot = app.store.bot(&record.bot).ok_or_else(|| Failure::new(409, "бот агента удалён"))?;
    let agent = &item.manifest["agent"];
    let role = loc(&agent["role"], lang).chars().take(80).collect::<String>();
    let persona = loc(&agent["persona"], lang);
    let bot_edited = bot_hash(&bot.role, &bot.persona) != catalog_extra(&bot, "catalogHash");
    let bot_same = bot.role == role && bot.persona == persona;
    let mut conflicts = Vec::new();
    if bot_edited && !bot_same {
        conflicts.push(json!({
            "what": "persona",
            "id": item.id,
            "title": bot.name,
            "mine": { "name": bot.role, "body": bot.persona },
            "theirs": { "name": role, "body": persona },
        }));
    }
    let refs: Vec<&str> = agent["skills"].as_array().into_iter().flatten().filter_map(Value::as_str).collect();
    for skill_ref in &refs {
        let skill_item = need_item(skill_ref)?;
        if let Some(skill) = catalog_skill(app, skill_ref).filter(|s| s.catalog_version != skill_item.version) {
            let plan = plan_skill(&skill, skill_item, lang);
            if !plan.conflict.is_null() {
                conflicts.push(plan.conflict);
            }
        }
    }
    if !conflicts.is_empty() && resolve.is_empty() {
        return Err(conflict(conflicts));
    }
    // Навыки: новые ставятся и подключаются, старые обновляются тем же правилом.
    let mut added = record.skills_added.clone();
    for skill_ref in &refs {
        let skill_item = need_item(skill_ref)?;
        let (skill, created) = ensure_skill(app, skill_item, lang).await?;
        if created {
            added.push(skill.id.clone());
        } else if skill.catalog_version != skill_item.version {
            apply_skill(app, &skill, skill_item, lang, resolve).await?;
        }
        attach_skill(app, &bot.id, &skill.id).await?;
    }
    let take_theirs = !bot_edited || resolve == "theirs";
    let version = item.version.clone();
    let bot = app
        .store
        .edit_bot(&bot.id, move |bot| {
            if take_theirs {
                bot.role = role;
                bot.persona = persona;
                bot.extra.insert("catalogHash".into(), json!(bot_hash(&bot.role, &bot.persona)));
            }
            bot.extra.insert("catalogVersion".into(), json!(version));
        })
        .await
        .map_err(Failure::io)?;
    if let Some(r) = file.items.get_mut(&item.id) {
        r.skills_added = added;
    }
    Ok(json!({ "bot": bot }))
}

/// Подробности элемента для боковой панели: манифест, состояние и навыки агента целиком.
pub async fn detail(app: &App, id: &str, lang: &str) -> Result<Value, Failure> {
    let item = need_item(id)?;
    let file = load_installs(app).await;
    let mut out = item.manifest.clone();
    out["hash"] = json!(item.hash);
    out["state"] = installed_state(app, item, &file);
    if item.kind == "agent" {
        let skills: Vec<Value> = item.manifest["agent"]["skills"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
            .filter_map(|s| catalog().get(s))
            .map(|s| json!({ "id": s.id, "title": s.manifest["title"], "summary": s.manifest["summary"], "icon": s.manifest["icon"] }))
            .collect();
        out["skillItems"] = json!(skills);
    }
    if item.kind == "app" {
        out["mode"] = json!(app_mode(item));
        if app_mode(item) == "connector" {
            out["card"] = consent_card(item, lang);
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn skill(id: &str) -> Value {
        json!({
            "schema": 1, "id": id, "kind": "skill", "version": "1.0.0", "category": "research", "tier": "public",
            "title": { "ru": "Навык" }, "summary": { "ru": "Коротко" },
            "source": { "name": "Мы", "license": "MIT" },
            "skill": { "body": { "ru": "Шаги" } }
        })
    }

    fn files(values: &[Value]) -> Vec<(String, String)> {
        values.iter().enumerate().map(|(i, v)| (format!("{i}.json"), v.to_string())).collect()
    }

    fn persona(headings: &[&str]) -> String {
        headings.iter().map(|h| format!("{h}\nСодержание раздела.")).collect::<Vec<_>>().join("\n\n")
    }

    fn specialist() -> Value {
        json!({
            "schema": 1, "id": "agent.spec", "kind": "agent", "version": "1.0.0", "category": "dev", "tier": "public",
            "title": { "ru": "Специалист" }, "summary": { "ru": "Коротко" },
            "source": { "name": "Мы", "license": "MIT" },
            "agent": {
                "rank": "specialist",
                "persona": {
                    "ru": persona(&SPECIALIST_HEADINGS_RU),
                    "en": persona(&SPECIALIST_HEADINGS_EN)
                },
                "skills": ["skill.a"],
                "model_hint": "standard",
                "triggers": {
                    "files": ["*.rs", "Cargo.toml"],
                    "keywords": { "ru": ["rust"], "en": ["rust"] }
                },
                "checks": [
                    { "task": { "ru": "Собрать", "en": "Build" }, "criteria": { "ru": ["зелено"], "en": ["green"] }, "behavior": { "require": ["сборк|build"], "forbid": [] } },
                    { "task": { "ru": "Проверить", "en": "Check" }, "criteria": { "ru": ["ок"], "en": ["ok"] }, "behavior": { "require": ["провер|check"], "forbid": [] } }
                ]
            }
        })
    }

    #[test]
    fn the_builtin_catalog_is_valid_and_indexed() {
        let catalog = build(&embedded_files(), "builtin");
        assert!(catalog.errors.is_empty(), "ошибки встроенного каталога: {:?}", catalog.errors);
        assert!(embedded_files().iter().any(|(p, _)| p == "index.json"), "нет index.json");
        for kind in KINDS {
            assert!(catalog.items.iter().any(|i| i.kind == kind), "нет ни одного элемента вида {kind}");
        }
    }

    #[test]
    fn a_manifest_without_a_license_fails() {
        let mut m = skill("skill.a");
        m["source"].as_object_mut().unwrap().remove("license");
        assert!(validate(&m).unwrap_err().contains("source.license"));
        m["source"]["license"] = json!("  ");
        assert!(validate(&m).is_err());
        let catalog = build(&files(&[m]), "test");
        assert!(catalog.items.is_empty());
        assert_eq!(catalog.errors.len(), 1);
    }

    #[test]
    fn ids_are_unique_and_agents_need_existing_skills() {
        let agent = json!({
            "schema": 1, "id": "agent.x", "kind": "agent", "version": "1", "category": "research", "tier": "public",
            "title": { "ru": "Агент" }, "summary": { "ru": "Коротко" }, "source": { "name": "Мы", "license": "MIT" },
            "agent": {
                "persona": { "ru": "Ты агент" }, "skills": ["skill.a", "skill.missing"], "model_hint": "standard",
                "checks": [
                    { "task": { "ru": "Ответить", "en": "Answer" }, "criteria": { "ru": ["ок"], "en": ["ok"] }, "behavior": { "require": ["ок|ok"], "forbid": [] } },
                    { "task": { "ru": "Проверить", "en": "Check" }, "criteria": { "ru": ["ок"], "en": ["ok"] }, "behavior": { "require": ["ок|ok"], "forbid": [] } }
                ]
            }
        });
        let catalog = build(&files(&[skill("skill.a"), skill("skill.a"), agent]), "test");
        assert_eq!(catalog.items.len(), 1, "{:?}", catalog.errors);
        assert!(catalog.errors.iter().any(|e| e.contains("уже занят")));
        assert!(catalog.errors.iter().any(|e| e.contains("skill.missing")));
    }

    #[test]
    fn a_lead_without_checks_is_rejected_too() {
        let mut lead = json!({
            "schema": 1, "id": "agent.lead", "kind": "agent", "version": "1.0.0", "category": "dev", "tier": "public",
            "title": { "ru": "Лидер" }, "summary": { "ru": "Коротко" }, "source": { "name": "Мы", "license": "MIT" },
            "agent": {
                "persona": { "ru": "Ты лидер", "en": "Lead" }, "skills": [], "model_hint": "standard", "effort": "medium",
                "checks": [
                    { "task": { "ru": "Ответить", "en": "Answer" }, "criteria": { "ru": ["по делу"], "en": ["to the point"] }, "behavior": { "require": ["ответ|answer"], "forbid": [] } },
                    { "task": { "ru": "Проверить", "en": "Check" }, "criteria": { "ru": ["точно"], "en": ["accurate"] }, "behavior": { "require": ["провер|check"], "forbid": [] } }
                ]
            }
        });
        assert!(validate(&lead).is_ok(), "{:?}", validate(&lead));
        lead["agent"].as_object_mut().unwrap().remove("checks");
        assert!(validate(&lead).unwrap_err().contains("checks"));
    }

    #[test]
    fn unpinned_packages_and_unknown_fields_are_refused() {
        let mut app = json!({
            "schema": 1, "id": "app.x", "kind": "app", "version": "1", "category": "dev", "tier": "public",
            "title": { "ru": "Приложение" }, "summary": { "ru": "Коротко" }, "source": { "name": "Мы", "license": "MIT" },
            "app": { "transport": "stdio", "command": "npx", "install": { "kind": "npx", "package": "x-mcp", "pin": "latest" } }
        });
        assert!(validate(&app).unwrap_err().contains("закреплённой"));
        app["app"]["install"]["pin"] = json!("1.2.3");
        assert!(validate(&app).is_ok());
        app["kind"] = json!("widget");
        assert!(validate(&app).is_err());
        let mut wrong_prefix = skill("agent.a");
        wrong_prefix["kind"] = json!("skill");
        assert!(validate(&wrong_prefix).is_err());
    }

    #[test]
    fn the_index_must_match_the_files() {
        let a = skill("skill.a");
        let text = a.to_string();
        let good = json!({ "items": [{ "id": "skill.a", "version": "1.0.0", "hash": hash_text(&text) }] });
        let catalog = build(&[("a.json".into(), text.clone()), ("index.json".into(), good.to_string())], "test");
        assert!(catalog.errors.is_empty(), "{:?}", catalog.errors);
        let stale = json!({ "items": [{ "id": "skill.a", "version": "1.0.0", "hash": "00" }, { "id": "skill.b", "version": "1" }] });
        let catalog = build(&[("a.json".into(), text), ("index.json".into(), stale.to_string())], "test");
        assert_eq!(catalog.errors.len(), 2, "{:?}", catalog.errors);
    }

    #[test]
    fn an_unsigned_remote_index_is_refused() {
        assert!(verify_signed_index(b"{\"schema\":1}", b"00").is_err());
        assert!(verify_signed_index(b"{\"schema\":1}", "ab".repeat(64).as_bytes()).is_err());
    }

    #[test]
    fn localized_text_falls_back_to_russian() {
        assert_eq!(loc(&json!({ "ru": "да", "en": "yes" }), "en"), "yes");
        assert_eq!(loc(&json!({ "ru": "да" }), "en"), "да");
        assert_eq!(loc(&json!("строка"), "en"), "строка");
        assert_eq!(host_of("https://www.gosuslugi.ru/path"), Some("gosuslugi.ru".into()));
    }

    #[test]
    fn an_old_model_hint_is_normalised_on_load() {
        let mut agent = specialist();
        agent["agent"]["model_hint"] = json!("smart");
        let catalog = build(&files(&[skill("skill.a"), agent]), "test");
        assert!(catalog.errors.is_empty(), "{:?}", catalog.errors);
        let item = catalog.get("agent.spec").expect("специалист не загрузился");
        assert_eq!(item.manifest["agent"]["model_hint"], "strong");
        assert_eq!(item.manifest["agent"]["effort"], "medium");
        assert_eq!(item.manifest["agent"]["rank"], "specialist");
    }

    #[test]
    fn a_specialist_without_headings_triggers_or_checks_is_rejected() {
        let good = specialist();
        assert!(validate(&good).is_ok(), "{:?}", validate(&good));

        let mut no_heading = good.clone();
        no_heading["agent"]["persona"]["ru"] = json!(persona(&SPECIALIST_HEADINGS_RU[..7]));
        assert!(validate(&no_heading).unwrap_err().contains("заголовк"));

        let mut no_triggers = good.clone();
        no_triggers["agent"].as_object_mut().unwrap().remove("triggers");
        assert!(validate(&no_triggers).unwrap_err().contains("triggers"));

        let mut no_checks = good.clone();
        no_checks["agent"].as_object_mut().unwrap().remove("checks");
        assert!(validate(&no_checks).unwrap_err().contains("checks"));

        let mut no_behavior = good.clone();
        no_behavior["agent"]["checks"][0].as_object_mut().unwrap().remove("behavior");
        assert!(validate(&no_behavior).unwrap_err().contains("behavior"));

        let mut bad_pattern = good.clone();
        bad_pattern["agent"]["checks"][0]["behavior"]["require"] = json!(["["]);
        assert!(validate(&bad_pattern).unwrap_err().contains("behavior.require"));

        let mut too_long = good.clone();
        too_long["agent"]["persona"]["en"] = json!("x".repeat(PERSONA_MAX + 1));
        assert!(validate(&too_long).unwrap_err().contains("слишком длинный"));
    }

    #[test]
    fn the_reference_rust_specialist_loads() {
        let catalog = build(&embedded_files(), "builtin");
        let item = catalog.get("agent.rust-developer").unwrap_or_else(|| panic!("нет эталонного специалиста: {:?}", catalog.errors));
        let agent = &item.manifest["agent"];
        assert_eq!(agent["rank"], "specialist");
        assert_eq!(agent["model_hint"], "standard");
        assert_eq!(agent["effort"], "medium");
        assert_eq!(item.category, "dev");
        assert_eq!(item.tier, "verified");
        let ru = agent["persona"]["ru"].as_str().unwrap();
        let en = agent["persona"]["en"].as_str().unwrap();
        assert!(headings_in_order(ru, &SPECIALIST_HEADINGS_RU), "в persona.ru нет разделов специалиста");
        assert!(headings_in_order(en, &SPECIALIST_HEADINGS_EN), "в persona.en нет разделов специалиста");
        assert!(agent["triggers"]["files"].as_array().is_some_and(|files| files.iter().any(|f| f.as_str() == Some("*.rs"))));
        assert_eq!(agent["checks"].as_array().unwrap().len(), 2);
    }
}
