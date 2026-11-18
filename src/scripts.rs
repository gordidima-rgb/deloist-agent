//! Скрипты ботов: рецепт из вызовов инструментов, который программа проходит без модели.
//!
//! Скрипт не код. Это список шагов «инструмент + аргументы» с подстановкой `{{параметр}}` и повтором
//! `for_each` по списку. Каждый шаг идёт через `turn::execute_call_in`, то есть теми же правилами, что и
//! вызов самого бота: карточки согласий, запреты на секреты, режим «без человека», отмена хода. Нового права
//! исполнения у скрипта нет, поэтому код человеку не показывают, а согласия на «ускорить скриптом» хватает.
//!
//! Сохранить скрипт можно только под согласие человека (`optimizer::open_consent`). Остальным ботам он
//! становится известен (факт в общей памяти) после первого успешного запуска, то есть проверки на живом примере.

use std::path::PathBuf;
use std::sync::Arc;

use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};
use tokio_util::sync::CancellationToken;

use crate::store::{now, uid, Bot};
use crate::App;

/// Шаги скрипта только из того, что бот и так делает сам, и без отладчика, входов и передачи управления человеку.
pub const ALLOWED_TOOLS: [&str; 9] = [
    "browser_nav", "browser_page", "browser_mouse", "browser_keyboard", "web_fetch", "web_search", "read_file", "list_files", "recall",
];
const MAX_STEPS: usize = 60;
const MAX_CALLS_PER_RUN: usize = 600;
const MAX_ITEMS: usize = 300;
const MAX_NAME: usize = 60;
const MAX_DESCRIPTION: usize = 500;
const COLLECT_PER_STEP: usize = 3_000;
const COLLECT_TOTAL: usize = 15_000;
/// Сколько знаков результата шага помнит `{{prev}}`.
const PREV_LIMIT: usize = 20_000;

static GATE: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

pub fn is_script_tool(name: &str) -> bool {
    matches!(name, "save_script" | "run_script")
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Param {
    pub name: String,
    pub description: String,
}

/// Достать значение из текста результата шага регулярным выражением (движок `regex` работает за линейное время,
/// зависнуть на нём нельзя). `all` — список всех совпадений для `for_each`, иначе первое совпадение или пустая строка.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Save {
    pub name: String,
    pub regex: String,
    /// Номер группы в скобках; 0 — всё совпадение.
    pub group: usize,
    pub all: bool,
}

/// Условие на переменную: все заданные поля должны сойтись. `var` — параметр, сохранённое значение или `prev`.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Cond {
    pub var: String,
    /// `true` — значение есть и не пусто; `false` — нет или пусто.
    pub exists: Option<bool>,
    pub equals: Option<String>,
    pub contains: Option<String>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Step {
    pub tool: String,
    pub args: Value,
    /// Имя параметра-списка: шаг повторяется для каждого элемента, элемент — `{{item}}`.
    pub for_each: Option<String>,
    /// Результат шага попадёт в ответ `run_script`.
    pub collect: bool,
    /// Достать из результата шага значение для следующих шагов (`{{имя}}`).
    pub save: Option<Save>,
    /// Шаг выполняется, только если условие верно; иначе пропускается.
    pub when: Option<Cond>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Script {
    pub id: String,
    pub name: String,
    pub description: String,
    pub params: Vec<Param>,
    pub steps: Vec<Step>,
    /// Бот-автор и согласие, под которое скрипт построен.
    pub bot_id: String,
    pub decision_key: String,
    /// Прошёл хотя бы один успешный запуск: только такой скрипт знают остальные боты.
    pub verified: bool,
    pub fact_id: String,
    pub runs: u32,
    pub created_at: i64,
}

fn dir(app: &App) -> PathBuf {
    app.store.data_dir().join("scripts")
}

async fn load_all(app: &App) -> Vec<Script> {
    let mut out = Vec::new();
    let Ok(mut rd) = tokio::fs::read_dir(dir(app)).await else { return out };
    while let Ok(Some(item)) = rd.next_entry().await {
        if let Ok(raw) = tokio::fs::read_to_string(item.path()).await {
            if let Ok(script) = serde_json::from_str::<Script>(&raw) {
                out.push(script);
            }
        }
    }
    out.sort_by_key(|s| s.created_at);
    out
}

async fn write(app: &App, script: &Script) -> Result<(), String> {
    tokio::fs::create_dir_all(dir(app)).await.map_err(|e| e.to_string())?;
    let file = dir(app).join(format!("{}.json", script.id));
    let tmp = file.with_extension("json.tmp");
    let text = serde_json::to_string_pretty(script).map_err(|e| e.to_string())?;
    tokio::fs::write(&tmp, text).await.map_err(|e| e.to_string())?;
    tokio::fs::rename(&tmp, &file).await.map_err(|e| e.to_string())
}

fn find<'a>(all: &'a [Script], name: &str) -> Option<&'a Script> {
    let wanted = name.trim().to_lowercase();
    all.iter().find(|s| s.id == name.trim() || s.name.to_lowercase() == wanted)
}

// ---------------------------------------------------------------------------
// Проверка и подстановка (чистые функции)
// ---------------------------------------------------------------------------

fn text_of<'a>(args: &'a Value, key: &str) -> &'a str {
    args.get(key).and_then(Value::as_str).unwrap_or_default().trim()
}

/// Рецепт из того, что прислала модель: проверяем всё, модель может прислать что угодно.
pub fn parse(args: &Value) -> Result<(String, String, Vec<Param>, Vec<Step>), String> {
    let name = text_of(args, "name");
    if name.is_empty() || name.chars().count() > MAX_NAME {
        return Err("нужно короткое название скрипта (до 60 знаков)".into());
    }
    let description: String = text_of(args, "description").chars().take(MAX_DESCRIPTION).collect();
    if description.is_empty() {
        return Err("нужно описание: что делает скрипт и какие параметры принимает".into());
    }
    let mut params = Vec::new();
    for p in args["params"].as_array().into_iter().flatten() {
        let pname = text_of(p, "name");
        if pname.is_empty() || !pname.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') || pname == "item" {
            return Err(format!("имя параметра «{pname}» не подходит: латиница, цифры, _ (и не item)"));
        }
        params.push(Param { name: pname.to_string(), description: text_of(p, "description").chars().take(200).collect() });
    }
    let raw_steps = args["steps"].as_array().ok_or("нужен список шагов steps")?;
    if raw_steps.is_empty() || raw_steps.len() > MAX_STEPS {
        return Err(format!("шагов должно быть от 1 до {MAX_STEPS}"));
    }
    let mut steps = Vec::new();
    for (i, s) in raw_steps.iter().enumerate() {
        let tool = text_of(s, "tool");
        if !ALLOWED_TOOLS.contains(&tool) {
            return Err(format!("шаг {}: инструмент «{tool}» в скрипт не берётся (можно: {})", i + 1, ALLOWED_TOOLS.join(", ")));
        }
        let args = s.get("args").cloned().filter(Value::is_object).unwrap_or_else(|| json!({}));
        let for_each = s["for_each"].as_str().map(str::trim).filter(|v| !v.is_empty()).map(str::to_string);
        // Что известно шагу: параметры, значения, сохранённые прошлыми шагами, и `prev`.
        let known = |var: &str| var == "prev" || params.iter().any(|p| p.name == var) || steps.iter().any(|st: &Step| st.save.as_ref().is_some_and(|sv| sv.name == var));
        if let Some(list) = &for_each {
            if !known(list) || list == "prev" {
                return Err(format!("шаг {}: for_each «{list}» — это не параметр и не сохранённое прошлым шагом значение", i + 1));
            }
        }
        let save = match s.get("save").filter(|v| v.is_object()) {
            None => None,
            Some(v) => {
                let save = Save {
                    name: text_of(v, "name").to_string(),
                    regex: text_of(v, "regex").to_string(),
                    group: v["group"].as_u64().unwrap_or(0) as usize,
                    all: v["all"].as_bool().unwrap_or(false),
                };
                if save.name.is_empty() || !save.name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') || matches!(save.name.as_str(), "item" | "prev") {
                    return Err(format!("шаг {}: имя в save «{}» не подходит: латиница, цифры, _ (и не item, prev)", i + 1, save.name));
                }
                if for_each.is_some() {
                    return Err(format!("шаг {}: save нельзя совмещать с for_each", i + 1));
                }
                let re = compile(&save.regex).map_err(|e| format!("шаг {}: регулярное выражение save не подходит: {e}", i + 1))?;
                if save.group >= re.captures_len() {
                    return Err(format!("шаг {}: в выражении нет группы {}", i + 1, save.group));
                }
                Some(save)
            }
        };
        let when = match s.get("when").filter(|v| v.is_object()) {
            None => None,
            Some(v) => {
                let cond = Cond {
                    var: text_of(v, "var").to_string(),
                    exists: v["exists"].as_bool(),
                    equals: v["equals"].as_str().map(str::to_string),
                    contains: v["contains"].as_str().map(str::to_string),
                };
                if !known(&cond.var) {
                    return Err(format!("шаг {}: в when переменная «{}» неизвестна", i + 1, cond.var));
                }
                if cond.exists.is_none() && cond.equals.is_none() && cond.contains.is_none() {
                    return Err(format!("шаг {}: в when нужно exists, equals или contains", i + 1));
                }
                Some(cond)
            }
        };
        steps.push(Step { tool: tool.to_string(), args, for_each, collect: s["collect"].as_bool().unwrap_or(false), save, when });
    }
    // Секреты и контакты в рецепт не кладём: он лежит на диске и виден всем ботам.
    if let Some(reason) = secret_in_recipe(&json!([&name, &description, &params, &steps])) {
        return Err(format!("в рецепте есть секрет ({reason}): вынеси его в параметр, а значение пусть даёт человек"));
    }
    Ok((name.to_string(), description, params, steps))
}

fn compile(pattern: &str) -> Result<regex::Regex, String> {
    if pattern.is_empty() || pattern.chars().count() > 300 {
        return Err("выражение пустое или длиннее 300 знаков".into());
    }
    regex::RegexBuilder::new(pattern).size_limit(200_000).build().map_err(|e| e.to_string())
}

/// Вынуть значение из результата шага. Нет совпадения — пустая строка (или пустой список).
pub fn extract(save: &Save, text: &str) -> Value {
    let Ok(re) = compile(&save.regex) else { return Value::Null };
    let pick = |c: regex::Captures| c.get(save.group).map(|m| m.as_str().to_string());
    if save.all {
        Value::Array(re.captures_iter(text).filter_map(pick).take(MAX_ITEMS).map(Value::String).collect())
    } else {
        Value::String(re.captures(text).and_then(pick).unwrap_or_default())
    }
}

fn is_empty_value(v: &Value) -> bool {
    match v {
        Value::Null => true,
        Value::String(s) => s.trim().is_empty(),
        Value::Array(a) => a.is_empty(),
        _ => false,
    }
}

pub fn holds(cond: &Cond, vars: &Map<String, Value>) -> bool {
    let v = vars.get(&cond.var).cloned().unwrap_or(Value::Null);
    let shown = show(&v);
    cond.exists.map_or(true, |want| want != is_empty_value(&v))
        && cond.equals.as_ref().map_or(true, |e| &shown == e)
        && cond.contains.as_ref().map_or(true, |c| shown.contains(c.as_str()))
}

// ---------------------------------------------------------------------------
// Секреты в рецепте
// ---------------------------------------------------------------------------

/// Имя поля, в котором лежит секрет: значением может быть только `{{параметр}}`.
const SECRET_KEYS: [&str; 16] = ["pass", "pwd", "token", "secret", "apikey", "api_key", "api-key", "cookie", "authorization", "auth", "cvv", "otp", "парол", "токен", "ключ", "секрет"];

fn is_placeholder(s: &str) -> bool {
    let t = s.trim();
    t.starts_with("{{") && t.ends_with("}}") && t.matches("{{").count() == 1
}

/// Невидимые знаки, которыми прячут слово от сканера («па\u{200b}роль»).
fn drop_invisible(s: &str) -> String {
    s.chars().filter(|c| !matches!(*c, '\u{200b}'..='\u{200f}' | '\u{202a}'..='\u{202e}' | '\u{2060}' | '\u{feff}' | '\u{00ad}')).collect()
}

/// Длинная непрозрачная строка из букв и цифр: ключ, токен, закодированный пароль.
fn looks_opaque(s: &str) -> bool {
    let t = s.trim();
    t.len() >= 32 && !t.contains(char::is_whitespace) && t.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '+' | '/' | '=' | '.'))
        && t.chars().any(|c| c.is_ascii_digit()) && t.chars().any(|c| c.is_ascii_alphabetic()) && !t.starts_with("http")
}

/// `https://имя:пароль@сайт`: пароль в адресе.
fn userinfo_secret(url: &str) -> bool {
    let rest = url.split_once("://").map_or(url, |(_, r)| r);
    let authority = rest.split(['/', '?', '#']).next().unwrap_or_default();
    authority.split_once('@').is_some_and(|(info, _)| info.contains(':') && !is_placeholder(info.split_once(':').map_or("", |(_, p)| p)))
}

fn query_secret(url: &str) -> bool {
    let Some((_, query)) = url.split_once('?') else { return false };
    query.split(['&', ';']).any(|pair| {
        let (k, v) = pair.split_once('=').unwrap_or((pair, ""));
        let k = k.to_lowercase();
        SECRET_KEYS.iter().any(|s| k.contains(s)) && !v.is_empty() && !is_placeholder(v)
    })
}

/// Что похоже на секрет в рецепте: слова-метки, поля с секретными именами, ключи в адресе, длинные непрозрачные строки.
/// Читаем и «как есть», и без невидимых знаков, и без разметки: JSON, HTML и кавычки словам не мешают.
pub fn secret_in_recipe(value: &Value) -> Option<String> {
    fn walk(value: &Value, key: &str, out: &mut Option<String>, text: &mut String) {
        if out.is_some() {
            return;
        }
        match value {
            Value::String(s) => {
                let s = drop_invisible(s);
                if !key.is_empty() && SECRET_KEYS.iter().any(|k| key.to_lowercase().contains(k)) && !s.trim().is_empty() && !is_placeholder(&s) {
                    *out = Some(format!("поле «{key}»"));
                } else if s.starts_with("http") && (query_secret(&s) || userinfo_secret(&s)) {
                    *out = Some("ключ или пароль в адресе".into());
                } else if looks_opaque(&s) {
                    *out = Some("длинная непрозрачная строка, похожая на ключ".into());
                }
                // Адреса проверены своими правилами выше: сканер слов принимает `api?page=2` за имя ключа.
                if !s.starts_with("http") {
                    text.push_str(&s);
                    text.push('\n');
                }
            }
            Value::Array(a) => a.iter().for_each(|v| walk(v, key, out, text)),
            Value::Object(o) => o.iter().for_each(|(k, v)| {
                text.push_str(k);
                text.push(' ');
                walk(v, k, out, text);
            }),
            _ => {}
        }
    }
    let mut found = None;
    let mut text = String::new();
    walk(value, "", &mut found, &mut text);
    if found.is_some() {
        return found;
    }
    // Разметку заменяем пробелами, а «имя: значение» оставляем: сканер ищет метку и слово рядом.
    let plain: String = text.chars().map(|c| if matches!(c, '"' | '\'' | '{' | '}' | '[' | ']' | '<' | '>' | '\\' | '(' | ')' | '`') { ' ' } else { c }).collect();
    crate::sensitive::secret_in(&text).or_else(|| crate::sensitive::secret_in(&plain)).map(str::to_string)
}

fn show(value: &Value) -> String {
    match value {
        Value::String(s) => s.clone(),
        other => other.to_string(),
    }
}

/// `{{имя}}` в строке заменяется значением; если вся строка — одно `{{имя}}`, значение подставляется как есть (число, список).
pub fn fill(template: &Value, vars: &Map<String, Value>) -> Value {
    match template {
        Value::String(s) => {
            if let Some(key) = s.strip_prefix("{{").and_then(|r| r.strip_suffix("}}")) {
                if let Some(v) = vars.get(key.trim()) {
                    return v.clone();
                }
            }
            let mut out = s.clone();
            for (k, v) in vars {
                out = out.replace(&format!("{{{{{k}}}}}"), &show(v));
            }
            Value::String(out)
        }
        Value::Array(a) => Value::Array(a.iter().map(|v| fill(v, vars)).collect()),
        Value::Object(o) => Value::Object(o.iter().map(|(k, v)| (k.clone(), fill(v, vars))).collect()),
        other => other.clone(),
    }
}

fn clip(text: &str, limit: usize) -> String {
    if text.chars().count() <= limit {
        text.to_string()
    } else {
        format!("{}…", text.chars().take(limit).collect::<String>())
    }
}

// ---------------------------------------------------------------------------
// Инструменты бота
// ---------------------------------------------------------------------------

pub fn defs() -> Vec<Value> {
    let function = |name: &str, description: &str, properties: Value, required: &[&str]| {
        let mut parameters = json!({ "type": "object", "properties": properties });
        if !required.is_empty() {
            parameters["required"] = json!(required);
        }
        json!({ "type": "function", "function": { "name": name, "description": description, "parameters": parameters } })
    };
    vec![
        function(
            "save_script",
            "Сохранить скрипт: рецепт из твоих вызовов инструментов, который программа пройдёт без модели в разы быстрее. \
             Только после согласия человека ускорить работу скриптом (его приходит просить задача). Шаги — browser_nav, browser_page, \
             browser_mouse, browser_keyboard, web_fetch, web_search, read_file, list_files, recall. В args можно писать {{параметр}}; \
             for_each — имя параметра-списка, шаг повторится на каждый элемент ({{item}}); collect — результат шага попадёт в ответ. \
             {{prev}} — результат прошлого шага; save {name, regex, group, all} достаёт из результата значение (all — список всех совпадений для for_each); \
             when {var, exists | equals | contains} пропускает шаг, если условие неверно. \
             Секретов в шагах быть не должно. Скрипт с тем же названием заменяется. После сохранения проверь его run_script на живом примере.",
            json!({
                "name": { "type": "string", "description": "Короткое название." },
                "description": { "type": "string", "description": "Что делает и когда его брать." },
                "params": { "type": "array", "description": "Что меняется между запусками.", "items": { "type": "object", "properties": {
                    "name": { "type": "string" }, "description": { "type": "string" } }, "required": ["name"] } },
                "steps": { "type": "array", "description": "Шаги по порядку, до 60.", "items": { "type": "object", "properties": {
                    "tool": { "type": "string" }, "args": { "type": "object" },
                    "for_each": { "type": "string", "description": "Имя параметра-списка." },
                    "collect": { "type": "boolean" },
                    "save": { "type": "object", "description": "Достать значение из результата: {name, regex, group, all}.", "properties": { "name": { "type": "string" }, "regex": { "type": "string" }, "group": { "type": "integer" }, "all": { "type": "boolean" } }, "required": ["name", "regex"] },
                    "when": { "type": "object", "description": "Условие шага: {var, exists | equals | contains}.", "properties": { "var": { "type": "string" }, "exists": { "type": "boolean" }, "equals": { "type": "string" }, "contains": { "type": "string" } }, "required": ["var"] } }, "required": ["tool"] } },
            }),
            &["name", "description", "steps"],
        ),
        function(
            "run_script",
            "Запустить сохранённый скрипт по названию, params — значения параметров. Каждый шаг идёт теми же правилами и карточками, что твой вызов. \
             Без названия — список скриптов, которые есть у человека. Первый успешный запуск проверяет скрипт и делает его известным остальным ботам.",
            json!({
                "name": { "type": "string", "description": "Название или номер скрипта." },
                "params": { "type": "object", "description": "Значения параметров: {имя: значение}." },
            }),
            &[],
        ),
    ]
}

pub async fn exec(app: &Arc<App>, bot: &Bot, name: &str, args: &Value, depth: u32, cancel: &CancellationToken) -> Result<String, String> {
    match name {
        "save_script" => save(app, bot, args).await,
        "run_script" => run(app, bot, args, depth, cancel).await,
        other => Err(format!("неизвестный инструмент: {other}")),
    }
}

async fn save(app: &Arc<App>, bot: &Bot, args: &Value) -> Result<String, String> {
    let _gate = GATE.lock().await;
    let all = load_all(app).await;
    let (name, description, params, steps) = parse(args)?;
    // Правка своего же скрипта согласия не тратит; новый скрипт строится под открытое согласие человека.
    let existing = find(&all, &name).filter(|s| s.bot_id == bot.id).cloned();
    let (id, decision_key, created_at, fact_id, runs) = match existing {
        Some(old) => (old.id, old.decision_key, old.created_at, old.fact_id, 0),
        None => {
            let consent = crate::optimizer::open_consent(app, &bot.id)
                .await
                .ok_or("человек не просил скрипт для этой работы: сначала его согласие (карточка оптимизатора)")?;
            let id = uid("scr");
            crate::optimizer::attach_script(app, &consent.key, &id).await;
            (id, consent.key, now(), String::new(), 0)
        }
    };
    // Правка меняет рецепт: он снова не проверен.
    let script = Script { id: id.clone(), name: name.clone(), description, params, steps, bot_id: bot.id.clone(), decision_key, verified: false, fact_id, runs, created_at };
    write(app, &script).await?;
    Ok(format!("Скрипт «{name}» сохранён ({id}), шагов: {}. Теперь проверь его: run_script на живом примере.", script.steps.len()))
}

async fn run(app: &Arc<App>, bot: &Bot, args: &Value, depth: u32, cancel: &CancellationToken) -> Result<String, String> {
    let all = load_all(app).await;
    let wanted = text_of(args, "name");
    if wanted.is_empty() {
        let lines: Vec<String> = all
            .iter()
            .filter(|s| s.verified || s.bot_id == bot.id)
            .map(|s| {
                let ps: Vec<&str> = s.params.iter().map(|p| p.name.as_str()).collect();
                format!("{} — {}{} (параметры: {})", s.id, s.name, if s.verified { "" } else { " [не проверен]" }, ps.join(", "))
            })
            .collect();
        return Ok(if lines.is_empty() { "Скриптов пока нет.".into() } else { lines.join("\n") });
    }
    let script = find(&all, wanted).filter(|s| s.verified || s.bot_id == bot.id).cloned().ok_or_else(|| format!("скрипта «{wanted}» нет"))?;
    let given = args.get("params").and_then(Value::as_object).cloned().unwrap_or_default();
    let missing: Vec<&str> = script.params.iter().filter(|p| !given.contains_key(&p.name)).map(|p| p.name.as_str()).collect();
    if !missing.is_empty() {
        return Err(format!("не хватает параметров: {}", missing.join(", ")));
    }
    let mut vars: Map<String, Value> = script.params.iter().filter_map(|p| given.get(&p.name).map(|v| (p.name.clone(), v.clone()))).collect();

    let mut calls = 0usize;
    let mut skipped = 0usize;
    let mut collected = String::new();
    for (index, step) in script.steps.iter().enumerate() {
        if step.when.as_ref().is_some_and(|cond| !holds(cond, &vars)) {
            skipped += 1;
            continue;
        }
        let rounds: Vec<Map<String, Value>> = match &step.for_each {
            None => vec![vars.clone()],
            Some(list) => {
                let items = vars.get(list).and_then(Value::as_array).ok_or_else(|| format!("шаг {}: «{list}» должен быть списком", index + 1))?;
                if items.len() > MAX_ITEMS {
                    return Err(format!("в списке «{list}» больше {MAX_ITEMS} элементов"));
                }
                items.iter().map(|item| { let mut v = vars.clone(); v.insert("item".into(), item.clone()); v }).collect()
            }
        };
        for round in rounds {
            if calls >= MAX_CALLS_PER_RUN {
                return Err(format!("скрипт остановлен: больше {MAX_CALLS_PER_RUN} вызовов за запуск"));
            }
            calls += 1;
            let call = crate::drivers::ToolCall { id: uid("call"), name: step.tool.clone(), args: fill(&step.args, &round) };
            let (status, text) = Box::pin(crate::turn::execute_call_in(app, bot, &call, depth, cancel, false)).await;
            if status != "ok" {
                return Err(format!("скрипт остановился на шаге {} ({}): {}. Выполнено вызовов: {}.", index + 1, step.tool, clip(&text, 600), calls - 1));
            }
            // Результат прошлого шага и то, что из него достали, видят следующие шаги.
            vars.insert("prev".into(), Value::String(clip(&text, PREV_LIMIT)));
            if let Some(save) = &step.save {
                vars.insert(save.name.clone(), extract(save, &text));
            }
            if step.collect && collected.chars().count() < COLLECT_TOTAL {
                collected.push_str(&format!("[{} #{}] {}
", step.tool, calls, clip(&text, COLLECT_PER_STEP)));
            }
        }
    }
    verified(app, &script).await;
    let skip = if skipped > 0 { format!(", пропущено по условию: {skipped}") } else { String::new() };
    let tail = format!("Скрипт «{}» выполнен: вызовов {calls}{skip}, моделью ни одного.", script.name);
    Ok(if collected.is_empty() { tail } else { format!("{tail}\n\n{}", clip(&collected, COLLECT_TOTAL)) })
}

/// Успешный запуск: скрипт проверен, о нём узнают все боты (общая память), согласие записывает, что скрипт есть.
async fn verified(app: &Arc<App>, script: &Script) {
    let _gate = GATE.lock().await;
    let all = load_all(app).await;
    let Some(mut current) = all.into_iter().find(|s| s.id == script.id) else { return };
    current.runs += 1;
    if !current.verified {
        let ps: Vec<String> = current.params.iter().map(|p| if p.description.is_empty() { p.name.clone() } else { format!("{} — {}", p.name, p.description) }).collect();
        let input = json!({
            "scope": "shared",
            "kind": "project",
            "title": format!("Есть скрипт «{}»", current.name),
            "body": format!(
                "{} Запуск: run_script, name «{}»; параметры: {}. Скрипт идёт без модели, в разы быстрее ручной работы: бери его, когда просят такое же.",
                current.description, current.name, if ps.is_empty() { "нет".into() } else { ps.join("; ") }
            ),
        });
        let id = if current.fact_id.is_empty() { None } else { Some(current.fact_id.as_str()) };
        // Не записалась память — скрипт всё равно проверен и работает; боты найдут его через run_script без названия.
        if let Ok(fact) = crate::memory::human_write(app, &input, id).await {
            current.fact_id = fact.id;
        }
        current.verified = true;
    }
    let _ = write(app, &current).await;
}

/// Скрипты для окна и тестов: без шагов с аргументами не отдаём, только краткое описание.
pub async fn list(app: &App) -> Vec<Value> {
    load_all(app)
        .await
        .into_iter()
        .map(|s| json!({ "id": s.id, "name": s.name, "description": s.description, "botId": s.bot_id, "verified": s.verified, "runs": s.runs, "steps": s.steps.len() }))
        .collect()
}

/// Человек удалил скрипт: файл и факт в общей памяти уходят, согласие снова открыто для нового скрипта.
pub async fn remove(app: &Arc<App>, id: &str) -> bool {
    let _gate = GATE.lock().await;
    let Some(script) = load_all(app).await.into_iter().find(|s| s.id == id) else { return false };
    let _ = tokio::fs::remove_file(dir(app).join(format!("{}.json", script.id))).await;
    if !script.fact_id.is_empty() {
        let _ = crate::memory::human_delete(app, &script.fact_id).await;
    }
    crate::optimizer::forget(app, &script.decision_key).await;
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    fn recipe() -> Value {
        json!({
            "name": "Поиск в Telegram", "description": "Собирает выдачу поиска",
            "params": [{ "name": "queries" }, { "name": "topic" }],
            "steps": [
                { "tool": "browser_nav", "args": { "action": "open", "url": "https://web.telegram.org/k/" } },
                { "tool": "browser_page", "args": { "action": "type", "text": "{{item}} {{topic}}" }, "for_each": "queries", "collect": true },
            ],
        })
    }

    #[test]
    fn a_good_recipe_parses() {
        let (name, _, params, steps) = parse(&recipe()).unwrap();
        assert_eq!((name.as_str(), params.len(), steps.len()), ("Поиск в Telegram", 2, 2));
        assert_eq!(steps[1].for_each.as_deref(), Some("queries"));
        assert!(steps[1].collect);
    }

    #[test]
    fn dangerous_or_broken_recipes_are_refused() {
        for (patch, why) in [
            (json!({ "steps": [{ "tool": "run_command", "args": { "command": "dir" } }] }), "оболочка"),
            (json!({ "steps": [{ "tool": "browser_devtools", "args": { "action": "eval" } }] }), "код на странице"),
            (json!({ "steps": [{ "tool": "run_script", "args": {} }] }), "рекурсия"),
            (json!({ "steps": [{ "tool": "assign_task", "args": {} }] }), "передача"),
            (json!({ "steps": [{ "tool": "web_fetch", "for_each": "nope" }] }), "чужой for_each"),
            (json!({ "steps": [] }), "пусто"),
            (json!({ "params": [{ "name": "item" }] }), "зарезервированное имя"),
            (json!({ "params": [{ "name": "a b" }] }), "плохое имя"),
            (json!({ "steps": [{ "tool": "web_fetch", "args": { "url": "https://x.ru", "note": "пароль: Qwerty123" } }] }), "секрет"),
        ] {
            let mut r = recipe();
            for (k, v) in patch.as_object().unwrap() {
                r[k] = v.clone();
            }
            assert!(parse(&r).is_err(), "принят рецепт: {why}");
        }
    }

    #[test]
    fn placeholders_fill_whole_values_and_substrings() {
        let vars: Map<String, Value> = json!({ "item": "кружки", "n": 3, "list": ["a", "b"] }).as_object().unwrap().clone();
        let out = fill(&json!({ "q": "{{item}} дёшево", "limit": "{{n}}", "all": "{{list}}", "k": 1, "deep": [{ "x": "{{item}}" }] }), &vars);
        assert_eq!(out, json!({ "q": "кружки дёшево", "limit": 3, "all": ["a", "b"], "k": 1, "deep": [{ "x": "кружки" }] }));
        // Неизвестный параметр остаётся как есть: шаг получит видимую ошибку, а не пустоту.
        assert_eq!(fill(&json!("{{ghost}}"), &vars), json!("{{ghost}}"));
    }
    #[test]
    fn save_pulls_values_out_of_a_result_and_when_branches() {
        let first = Save { name: "link".into(), regex: r"https?://[^\s]+".into(), group: 0, all: false };
        let all = Save { name: "links".into(), regex: r"https?://[^\s]+".into(), group: 0, all: true };
        let text = "see https://a.ru/1 and https://b.ru/2 end";
        assert_eq!(extract(&first, text), json!("https://a.ru/1"));
        assert_eq!(extract(&all, text), json!(["https://a.ru/1", "https://b.ru/2"]));
        assert_eq!(extract(&first, "nothing"), json!(""));
        let id = Save { name: "id".into(), regex: r"id=(\d+)".into(), group: 1, all: false };
        assert_eq!(extract(&id, "x id=42 y"), json!("42"));

        let vars: Map<String, Value> = json!({ "link": "", "n": "5", "prev": "ошибка 404" }).as_object().unwrap().clone();
        assert!(!holds(&Cond { var: "link".into(), exists: Some(true), ..Cond::default() }, &vars));
        assert!(holds(&Cond { var: "link".into(), exists: Some(false), ..Cond::default() }, &vars));
        assert!(holds(&Cond { var: "n".into(), equals: Some("5".into()), ..Cond::default() }, &vars));
        assert!(holds(&Cond { var: "prev".into(), contains: Some("404".into()), ..Cond::default() }, &vars));
        assert!(!holds(&Cond { var: "ghost".into(), exists: Some(true), ..Cond::default() }, &vars));
    }

    #[test]
    fn branching_recipes_are_checked() {
        let ok = json!({
            "name": "Открыть первое", "description": "Ищет и открывает первую ссылку",
            "params": [{ "name": "query" }],
            "steps": [
                { "tool": "web_search", "args": { "query": "{{query}}" }, "save": { "name": "first", "regex": r"https?://\S+" } },
                { "tool": "browser_nav", "args": { "action": "open", "url": "{{first}}" }, "when": { "var": "first", "exists": true } },
                { "tool": "browser_page", "args": { "action": "read" }, "when": { "var": "prev", "contains": "ok" }, "collect": true },
            ],
        });
        assert!(parse(&ok).is_ok());
        for (patch, why) in [
            (json!([{ "tool": "web_fetch", "save": { "name": "x", "regex": "(" } }]), "битое выражение"),
            (json!([{ "tool": "web_fetch", "save": { "name": "x", "regex": "a(b)", "group": 3 } }]), "нет группы"),
            (json!([{ "tool": "web_fetch", "save": { "name": "prev", "regex": "a" } }]), "зарезервированное имя"),
            (json!([{ "tool": "web_fetch", "when": { "var": "nope", "exists": true } }]), "неизвестная переменная"),
            (json!([{ "tool": "web_fetch", "when": { "var": "prev" } }]), "условие без проверки"),
            (json!([{ "tool": "web_fetch", "for_each": "later" }, { "tool": "web_fetch", "save": { "name": "later", "regex": "a" } }]), "список позже, чем нужен"),
        ] {
            let mut r = ok.clone();
            r["steps"] = patch;
            assert!(parse(&r).is_err(), "принят рецепт: {why}");
        }
        // Список, найденный прошлым шагом, можно обойти.
        let mut r = ok.clone();
        r["steps"] = json!([
            { "tool": "web_search", "args": { "query": "q" }, "save": { "name": "links", "regex": r"https?://\S+", "all": true } },
            { "tool": "browser_nav", "args": { "url": "{{item}}" }, "for_each": "links" },
        ]);
        assert!(parse(&r).is_ok());
    }

    #[test]
    fn disguised_secrets_in_a_recipe_are_caught() {
        let with = |value: Value| {
            let mut r = recipe();
            r["steps"] = json!([{ "tool": "web_fetch", "args": value }]);
            parse(&r)
        };
        assert!(with(json!({ "url": "https://x.ru", "note": "па\u{200b}роль: Qwerty123" })).is_err(), "невидимый знак");
        assert!(with(json!({ "html": "<input value='пароль: Qwerty123'>" })).is_err(), "разметка");
        assert!(with(json!({ "password": "hunter2x" })).is_err(), "поле password");
        assert!(with(json!({ "headers": { "Authorization": "Bearer abc" } })).is_err(), "заголовок");
        assert!(with(json!({ "url": "https://x.ru/api?api_key=zzz" })).is_err(), "ключ в адресе");
        assert!(with(json!({ "x": "ZXlKaGJHY2lPaUpJVXpJMU5pSjkuZXlKemRXSWk" })).is_err(), "длинная непрозрачная строка");
        // Параметры и обычные адреса проходят.
        assert!(with(json!({ "url": "https://user:hunter22@x.ru/" })).is_err(), "пароль в адресе");
        assert!(with(json!({ "url": "https://web.telegram.org/k/#@name", "query": "{{item}}" })).is_ok());
        let r = with(json!({ "url": "https://x.ru/api?page=2&q={{item}}" }));
        assert!(r.is_ok(), "{r:?}");
    }
}
