//! Бот-оптимизатор: читает журнал работы ботов и предлагает ускорить механику скриптом.
//!
//! Служба только читает переписки (`Store::thread_entries`) и ничего не запускает. Пишет она в один
//! файл `optimizer.json` и, если человек ответил «Написать скрипт», один факт в общую память.
//! В файле и в памяти нет аргументов и результатов вызовов: только имена инструментов, числа и
//! фраза человека с закрытыми секретами и контактами.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::{Arc, Mutex};

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::store::{now, uid, Thread};
use crate::App;

/// Шагов одной подписи в одном ходе, при которых хватает и одного хода.
pub const VOLUME_STEPS: usize = 40;
/// Шагов одной подписи в каждом из ≥ 2 ходов, когда работа повторяется.
pub const REPEAT_STEPS: usize = 15;
/// Доля подписи среди всех шагов хода (в процентах): остальное — решения ИИ.
pub const MIN_SHARE_PERCENT: usize = 60;
/// Ходы старше этого срока не считаем.
const WINDOW_MS: i64 = 30 * 24 * 3_600_000;
/// Сколько новых карточек за один просмотр.
const MAX_NEW_PER_SCAN: usize = 3;
/// Первая проверка дня — через пару минут после старта, как у доктора памяти.
const START_DELAY_MS: i64 = 3 * 60_000;
const EXAMPLE_CHARS: usize = 160;

/// Инструменты, где решает сам ИИ или человек: их повторы не механика.
const IGNORED: [&str; 12] = ["ask_user", "remember", "recall", "read_skill", "offer_specialist", "propose_team", "propose_bot_change", "propose_routine", "propose_rule", "search_catalog", "write_file", "run_command"];

static GATE: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());
static STARTED_AT: AtomicI64 = AtomicI64::new(0);
static CHECKED_DAY: Mutex<String> = Mutex::new(String::new());

// ---------------------------------------------------------------------------
// Разбор журнала (чистые функции)
// ---------------------------------------------------------------------------

/// Ход: всё, что сделал бот между двумя сообщениями человека.
#[derive(Debug, Default, Clone, PartialEq)]
pub struct Run {
    pub at: i64,
    /// Просьба человека, уже без секретов и контактов.
    pub ask: String,
    pub steps: usize,
    pub counts: BTreeMap<String, usize>,
}

/// Подпись шага: `имя` или `имя:действие`; все `browser_*` — одно семейство `browser`. Только буквы, цифры, `_` и `-`: она идёт в адрес и в ключ.
fn signature(call: &Value) -> Option<String> {
    let name = call["name"].as_str()?.trim();
    let safe = |text: &str| -> String { text.chars().filter(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-')).take(32).collect() };
    let base = safe(name);
    if base.is_empty() || name.contains(char::is_whitespace) {
        return None;
    }
    // Браузерная работа — это смесь snapshot, click, type, scroll: по одному действию доли не наберётся.
    if base.starts_with("browser_") {
        return Some("browser".into());
    }
    let action = call["args"]["action"].as_str().map(safe).filter(|a| !a.is_empty());
    Some(match action {
        Some(a) => format!("{base}:{a}"),
        None => base,
    })
}

fn ask_of(entry: &Value) -> String {
    let text: String = entry["text"].as_str().unwrap_or_default().chars().take(2_000).collect();
    let masked = crate::sensitive::mask(&text, true);
    let line = crate::memory::one_line(&masked);
    if line.chars().count() > EXAMPLE_CHARS {
        format!("{}…", line.chars().take(EXAMPLE_CHARS).collect::<String>())
    } else {
        line
    }
}

/// Разбить ленту на ходы. Вызовы внешних агентов (`activity`) входят в `steps`, но подписи не получают:
/// у них имя — это текст команды, а решения они принимают сами.
pub fn runs_of(entries: &[Value]) -> Vec<Run> {
    let mut runs: Vec<Run> = Vec::new();
    for entry in entries {
        match entry["role"].as_str() {
            Some("user") => runs.push(Run { at: entry["at"].as_i64().unwrap_or(0), ask: ask_of(entry), ..Run::default() }),
            Some("assistant") => {
                let Some(run) = runs.last_mut() else { continue };
                for call in entry["toolCalls"].as_array().into_iter().flatten() {
                    run.steps += 1;
                    if let Some(sig) = signature(call) {
                        *run.counts.entry(sig).or_default() += 1;
                    }
                }
                run.steps += entry["activity"].as_array().map_or(0, Vec::len);
            }
            _ => {}
        }
    }
    runs
}

fn ignored(sig: &str) -> bool {
    let tool = sig.split(':').next().unwrap_or(sig);
    IGNORED.contains(&tool)
}

#[derive(Debug, Clone, PartialEq)]
pub struct Finding {
    pub sig: String,
    /// `volume` — хватило одного хода, `repeat` — работа повторяется.
    pub kind: &'static str,
    /// Шагов в самом большом ходе.
    pub steps: usize,
    /// Сколько ходов подошло.
    pub runs: usize,
    pub example: String,
}

/// Что в ходах одного бота похоже на механику. Сильные находки первыми.
pub fn findings(runs: &[Run], since: i64) -> Vec<Finding> {
    let mut by_sig: BTreeMap<&str, Vec<&Run>> = BTreeMap::new();
    for run in runs.iter().filter(|r| r.at >= since) {
        for (sig, n) in &run.counts {
            if !ignored(sig) && *n * 100 >= run.steps * MIN_SHARE_PERCENT {
                by_sig.entry(sig).or_default().push(run);
            }
        }
    }
    let mut out = Vec::new();
    for (sig, hits) in by_sig {
        let count = |r: &Run| r.counts.get(sig).copied().unwrap_or(0);
        let Some(best) = hits.iter().copied().max_by_key(|r| count(r)) else { continue };
        let steps = count(best);
        let repeated = hits.iter().filter(|r| count(r) >= REPEAT_STEPS).count();
        let kind = if steps >= VOLUME_STEPS {
            "volume"
        } else if repeated >= 2 {
            "repeat"
        } else {
            continue;
        };
        out.push(Finding { sig: sig.to_string(), kind, steps, runs: if kind == "volume" { hits.len().max(1) } else { repeated }, example: best.ask.clone() });
    }
    out.sort_by(|a, b| b.steps.cmp(&a.steps));
    out
}

fn slug(sig: &str) -> String {
    sig.replace(':', "-")
}

pub fn key_of(bot_id: &str, sig: &str) -> String {
    format!("{bot_id}.{}", slug(sig))
}

// ---------------------------------------------------------------------------
// Файл состояния
// ---------------------------------------------------------------------------

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Proposal {
    pub id: String,
    pub key: String,
    pub bot_id: String,
    pub sig: String,
    pub kind: String,
    pub steps: usize,
    pub runs: usize,
    pub example: String,
    pub at: i64,
}

impl Default for Proposal {
    fn default() -> Self {
        Self { id: String::new(), key: String::new(), bot_id: String::new(), sig: String::new(), kind: String::new(), steps: 0, runs: 0, example: String::new(), at: 0 }
    }
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Decision {
    pub key: String,
    pub bot_id: String,
    pub sig: String,
    /// `script` — человек хочет скрипт, `once` — работа разовая.
    pub verdict: String,
    pub example: String,
    pub steps: usize,
    pub at: i64,
    /// Готовый и проверенный скрипт (только у `script`); пока пусто — бот его ещё строит.
    pub script_id: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
struct State {
    enabled: bool,
    last_day: String,
    decisions: Vec<Decision>,
    pending: Vec<Proposal>,
}

impl Default for State {
    fn default() -> Self {
        Self { enabled: true, last_day: String::new(), decisions: Vec::new(), pending: Vec::new() }
    }
}

fn path(app: &App) -> PathBuf {
    app.store.data_dir().join("optimizer.json")
}

async fn load(app: &App) -> State {
    let raw = tokio::fs::read_to_string(path(app)).await.unwrap_or_default();
    serde_json::from_str(&raw).unwrap_or_default()
}

async fn save(app: &App, state: &State) {
    if let Ok(text) = serde_json::to_string_pretty(state) {
        let tmp = path(app).with_extension("json.tmp");
        if tokio::fs::write(&tmp, text).await.is_ok() {
            let _ = tokio::fs::rename(&tmp, path(app)).await;
        }
    }
}

fn today() -> String {
    chrono::Local::now().format("%Y-%m-%d").to_string()
}

fn card(app: &App, p: &Proposal) -> Value {
    let name = app.store.bot(&p.bot_id).map(|b| b.name).unwrap_or_default();
    json!({
        "id": p.id, "botId": p.bot_id, "botName": name, "sig": p.sig, "kind": p.kind,
        "steps": p.steps, "runs": p.runs, "example": p.example, "at": p.at,
    })
}

// ---------------------------------------------------------------------------
// Работа службы
// ---------------------------------------------------------------------------

/// Один просмотр журналов. Возвращает, сколько новых карточек появилось.
pub async fn scan(app: &Arc<App>) -> usize {
    let _gate = GATE.lock().await;
    let mut state = load(app).await;
    if !state.enabled {
        return 0;
    }
    let since = now() - WINDOW_MS;
    let mut added: Vec<Proposal> = Vec::new();
    for bot in app.store.bots() {
        if added.len() >= MAX_NEW_PER_SCAN {
            break;
        }
        let entries = app.store.thread_entries(Thread::Bot(&bot.id)).await;
        let known = |key: &str| state.decisions.iter().any(|d| d.key == key) || state.pending.iter().any(|p| p.key == key) || added.iter().any(|p| p.key == key);
        let Some(found) = findings(&runs_of(&entries), since).into_iter().find(|f| !known(&key_of(&bot.id, &f.sig))) else { continue };
        added.push(Proposal {
            id: uid("opt"),
            key: key_of(&bot.id, &found.sig),
            bot_id: bot.id.clone(),
            sig: found.sig,
            kind: found.kind.into(),
            steps: found.steps,
            runs: found.runs,
            example: found.example,
            at: now(),
        });
    }
    state.last_day = today();
    state.pending.extend(added.iter().cloned());
    save(app, &state).await;
    for p in &added {
        app.turns.emit(json!({ "type": "optimizer_offer", "botId": p.bot_id, "offer": card(app, p) }));
    }
    added.len()
}

/// Тик планировщика: первая проверка за день, через пару минут после старта.
pub fn tick(app: &Arc<App>) {
    let started = STARTED_AT.load(Ordering::SeqCst);
    if started == 0 {
        STARTED_AT.store(now(), Ordering::SeqCst);
        return;
    }
    if now() - started < START_DELAY_MS {
        return;
    }
    let day = today();
    {
        let mut checked = CHECKED_DAY.lock().unwrap();
        if *checked == day {
            return;
        }
        *checked = day.clone();
    }
    let app = app.clone();
    tokio::spawn(async move {
        if load(&app).await.last_day != day {
            scan(&app).await;
        }
    });
}

/// Всё, что видит окно: включено ли, нерешённое, ответы.
pub async fn view(app: &App) -> Value {
    let state = load(app).await;
    json!({
        "enabled": state.enabled,
        "pending": state.pending.iter().map(|p| card(app, p)).collect::<Vec<_>>(),
        "decisions": state.decisions,
        "scripts": crate::scripts::list(app).await,
    })
}

pub async fn pending_for(app: &App, bot_id: &str) -> Vec<Value> {
    load(app).await.pending.iter().filter(|p| p.bot_id == bot_id).map(|p| card(app, p)).collect()
}

/// Ответ человека. `script` — «Написать скрипт», иначе «Разовая работа».
pub async fn decide(app: &Arc<App>, id: &str, script: bool) -> Result<Value, String> {
    let _gate = GATE.lock().await;
    let mut state = load(app).await;
    let at = state.pending.iter().position(|p| p.id == id).ok_or("предложение уже неактивно")?;
    let p = state.pending[at].clone();
    state.pending.remove(at);
    state.decisions.push(Decision {
        key: p.key,
        bot_id: p.bot_id,
        sig: p.sig,
        verdict: if script { "script" } else { "once" }.into(),
        example: p.example,
        steps: p.steps,
        at: now(),
        script_id: String::new(),
    });
    let decision = state.decisions.last().cloned().unwrap_or_default();
    save(app, &state).await;
    Ok(json!(decision))
}

/// Согласие, под которое бот вправе построить скрипт: решение «скрипт» у этого бота, скрипта ещё нет.
pub async fn open_consent(app: &App, bot_id: &str) -> Option<Decision> {
    load(app).await.decisions.into_iter().rev().find(|d| d.bot_id == bot_id && d.verdict == "script" && d.script_id.is_empty())
}

/// Скрипт построен под это согласие: оно израсходовано.
pub async fn attach_script(app: &App, key: &str, script_id: &str) {
    let _gate = GATE.lock().await;
    let mut state = load(app).await;
    if let Some(d) = state.decisions.iter_mut().find(|d| d.key == key) {
        d.script_id = script_id.to_string();
        save(app, &state).await;
    }
}

/// Задача боту после согласия. Скрипт — рецепт из его же вызовов инструментов, поэтому код человеку не
/// показывают: каждый шаг при запуске идёт через обычные карточки согласий бота.
pub fn build_task(d: &Decision) -> String {
    format!(
        "Человек согласился ускорить скриптом работу, на которой ты делал(а) много однотипных шагов («{sig}», около {steps} за ход; пример просьбы: {example}).\n\
         Сделай по порядку:\n\
         1. Вспомни свои шаги в той работе (они в этом разговоре) и составь из них рецепт для инструмента save_script. Шаги — только browser_nav, browser_page, browser_mouse, browser_keyboard, web_fetch, web_search, read_file, list_files, recall. Что меняется от раза к разу (что искать, кому писать) вынеси в параметры и подставь как {{{{имя}}}}; список повторяй через for_each. Если шагу нужен результат прошлого, бери {{{{prev}}}}; значение из результата достань через save (регулярное выражение, all — список для for_each); шаг, который нужен не всегда, закрой условием when. Результаты, нужные для ответа, пометь collect. Паролей, ключей и телефонов в шагах быть не должно.\n\
         2. ОБЯЗАТЕЛЬНО проверь скрипт на живом примере: запусти run_script на настоящей просьбе человека и сравни результат с тем, как ты делал(а) это руками. Не сходится — поправь рецепт (save_script с тем же именем) и проверь снова.\n\
         3. Когда проверка прошла, скажи человеку одной-двумя фразами: скрипт готов, как называется и что делает. После успешной проверки он автоматически станет известен всем ботам человека.",
        sig = d.sig, steps = d.steps, example = d.example
    )
}

/// Забыть ответ: служба снова может спросить про эту работу.
pub async fn forget(app: &App, key: &str) -> bool {
    let _gate = GATE.lock().await;
    let mut state = load(app).await;
    let before = state.decisions.len();
    state.decisions.retain(|d| d.key != key);
    let changed = state.decisions.len() != before;
    if changed {
        save(app, &state).await;
    }
    changed
}

pub async fn set_enabled(app: &App, enabled: bool) {
    let _gate = GATE.lock().await;
    let mut state = load(app).await;
    state.enabled = enabled;
    save(app, &state).await;
}

#[cfg(test)]
mod tests {
    use super::*;

    fn user(at: i64, text: &str) -> Value {
        json!({ "role": "user", "at": at, "text": text })
    }

    fn calls(name: &str, action: Option<&str>, n: usize) -> Value {
        let args = action.map_or(json!({}), |a| json!({ "action": a }));
        json!({ "role": "assistant", "toolCalls": (0..n).map(|_| json!({ "name": name, "args": args, "result": "secret-result" })).collect::<Vec<_>>() })
    }

    #[test]
    fn long_same_tool_run_is_a_volume_finding() {
        let entries = vec![user(100, "Найди в Telegram всех про кружки"), calls("browser_page", Some("click"), 45), calls("remember", None, 1)];
        let found = findings(&runs_of(&entries), 0);
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].sig, "browser");
        assert_eq!(found[0].kind, "volume");
        assert_eq!(found[0].steps, 45);
    }

    #[test]
    fn repeated_medium_runs_in_two_turns_are_a_repeat_finding() {
        let entries = vec![user(1, "день 1"), calls("web_fetch", None, 18), user(2, "день 2"), calls("web_fetch", None, 16)];
        let found = findings(&runs_of(&entries), 0);
        assert_eq!(found.len(), 1);
        assert_eq!((found[0].kind, found[0].runs), ("repeat", 2));
        // Один такой ход — ещё не повторяемость.
        assert!(findings(&runs_of(&entries[..2]), 0).is_empty());
    }

    #[test]
    fn mixed_work_creative_tools_and_old_runs_are_not_findings() {
        // Доля подписи ниже порога: ИИ решает много других шагов.
        let mixed = vec![user(1, "x"), calls("browser_page", Some("click"), 45), calls("web_fetch", None, 40)];
        assert!(findings(&runs_of(&mixed), 0).is_empty());
        // Кодинг и запись файлов — решает ИИ, механикой не считаем.
        let coding = vec![user(1, "x"), calls("run_command", None, 200), calls("write_file", None, 60)];
        assert!(findings(&runs_of(&coding), 0).is_empty());
        // Слишком давно.
        let old = vec![user(5, "x"), calls("web_fetch", None, 80)];
        assert!(findings(&runs_of(&old), 10).is_empty());
        // Вызовы внешних агентов подписи не получают.
        let acp = vec![user(1, "x"), json!({ "role": "assistant", "activity": (0..90).map(|_| json!({ "name": "Get-ChildItem -Force x" })).collect::<Vec<_>>() })];
        assert!(findings(&runs_of(&acp), 0).is_empty());
    }

    #[test]
    fn example_hides_secrets_and_arguments_never_reach_the_run() {
        let entries = vec![user(1, "Пароль от кабинета Ozon: Qwerty123, зайди и собери"), calls("web_fetch", None, 50)];
        let runs = runs_of(&entries);
        assert!(!runs[0].ask.contains("Qwerty123"), "{}", runs[0].ask);
        assert!(!format!("{runs:?}").contains("secret-result"));
    }

    #[test]
    fn signature_is_safe_for_a_path_and_a_key() {
        assert_eq!(signature(&json!({ "name": "browser_page", "args": { "action": "telegram_results" } })).as_deref(), Some("browser"));
        assert_eq!(signature(&json!({ "name": "Get-ChildItem -Force" })), None);
        assert_eq!(signature(&json!({ "name": "a/b?c", "args": { "action": "x y/z" } })).as_deref(), Some("abc:xyz"));
        assert_eq!(key_of("bot_1", "web_fetch:search"), "bot_1.web_fetch-search");
    }
}
