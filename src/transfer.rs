//! Перенос ботов между доверенными машинами (ПК ↔ облако) одним нажатием.
//!
//! Основа — тот же файл команды, что и `team export/apply`: описания ботов, групп, навыков и
//! рутин. Сверху едут только доверенные части (brain.md, память, ACL и происхождение). Секреты,
//! ключи, токены, расписания вебхуков и настройки компьютера в пакет не попадают. Если часть
//! больше предела, она пропускается и попадает в отчёт — источник при этом не меняется и не
//! удаляется. Пакет можно применить повторно: второй раз он ничего не дублирует.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

use crate::store::now;
use crate::App;

/// Формат пакета переноса. Отдельный от `deloist-team`: у переноса другие правила доверия.
pub const FORMAT: &str = "deloist-transfer";
pub const VERSION: u32 = 1;
/// Одна часть пакета должна пролезть в запечатанный конверт ретранслятора (256 КБ) с запасом.
pub const MAX_PART_BYTES: usize = 200 * 1024;
/// Весь пакет не больше файла команды из `team::MAX_REQUEST_BYTES`.
pub const MAX_BUNDLE_BYTES: usize = 1024 * 1024;
/// Сколько фактов памяти уходит за один заход.
pub const MAX_FACTS: usize = 200;
/// Сколько символов текста одной части.
const MAX_TEXT: usize = 20_000;
const FILE: &str = "transfer.json";

/// Поля, которые никогда не попадают в пакет, даже если пришли в чужом описании.
const SECRET_KEYS: &[&str] = &[
    "apiKey", "apikey", "api_key", "fallbackApiKey", "imageApiKey", "token", "accessToken", "refreshToken", "hookToken",
    "hook_token", "secret", "secrets", "secretsKey", "gatewayKey", "claimToken", "password", "passwd", "pin", "cvv", "key",
    "privateKey", "sessionKey", "cookie", "authorization", "credentials", "runtimeToken", "claim", "nonce",
];

/// Почему часть не поехала. Показывается в окне: человек видит, что именно осталось на источнике.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Skipped {
    pub part: String,
    pub kind: String,
    pub bytes: usize,
    pub limit: usize,
    pub reason: String,
}
impl Default for Skipped {
    fn default() -> Self {
        Self { part: String::new(), kind: String::new(), bytes: 0, limit: 0, reason: String::new() }
    }
}

fn skipped(part: &str, kind: &str, bytes: usize, limit: usize, reason: &str) -> Skipped {
    Skipped { part: part.into(), kind: kind.into(), bytes, limit, reason: reason.into() }
}

fn bytes(value: &Value) -> usize {
    serde_json::to_vec(value).map(|v| v.len()).unwrap_or(usize::MAX)
}

/// Убрать секреты из любого текста; контакты оставляем — они нужны боту для работы.
pub fn scrub_text(text: &str) -> String {
    crate::sensitive::mask(text, false)
}

/// Рекурсивно вырезать поля-секреты и замаскировать значения. Используется и для чужого пакета.
pub fn scrub(value: &Value) -> Value {
    match value {
        Value::Object(map) => {
            let mut out = serde_json::Map::new();
            for (key, val) in map {
                if SECRET_KEYS.iter().any(|secret| key.eq_ignore_ascii_case(secret)) {
                    continue;
                }
                out.insert(key.clone(), scrub(val));
            }
            Value::Object(out)
        }
        Value::Array(items) => Value::Array(items.iter().map(scrub).collect()),
        Value::String(text) => Value::String(scrub_text(text)),
        other => other.clone(),
    }
}

/// Оставить в копии файла команды только выбранных ботов и всё, что с ними связано.
pub fn select_team(team: &Value, selected: &[String]) -> Value {
    let want: HashSet<&str> = selected.iter().map(String::as_str).collect();
    let names: HashSet<String> = team
        .get("bots")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter(|bot| bot.get("name").and_then(Value::as_str).is_some_and(|n| want.contains(n)))
        .filter_map(|bot| bot.get("name").and_then(Value::as_str).map(str::to_string))
        .collect();
    let mut out = team.clone();
    let mut take = |key: &str, keep: &dyn Fn(&Value) -> bool| {
        let kept: Vec<Value> = team.get(key).and_then(Value::as_array).into_iter().flatten().filter(|v| keep(v)).cloned().collect();
        out[key] = json!(kept);
    };
    take("bots", &|v| v.get("name").and_then(Value::as_str).is_some_and(|n| names.contains(n)));
    let skills: HashSet<String> = team
        .get("bots")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter(|v| v.get("name").and_then(Value::as_str).is_some_and(|n| names.contains(n)))
        .flat_map(|v| v.get("skills").and_then(Value::as_array).into_iter().flatten().filter_map(Value::as_str).map(str::to_string))
        .collect();
    take("skills", &|v| v.get("name").and_then(Value::as_str).is_some_and(|n| skills.contains(n)));
    take("rooms", &|v| {
        v.get("members").and_then(Value::as_array).is_some_and(|members| members.iter().any(|m| m.as_str().is_some_and(|n| names.contains(n))))
    });
    take("routines", &|v| v.get("bot").and_then(Value::as_str).is_some_and(|n| names.contains(n)));
    take("workflows", &|_| false);
    out
}

/// Откуда приехал пакет. Имя машины — не секрет; оно помогает человеку узнать источник в отчёте.
pub fn origin() -> Value {
    let name = std::env::var("COMPUTERNAME")
        .or_else(|_| std::env::var("HOSTNAME"))
        .unwrap_or_default()
        .chars()
        .take(80)
        .collect::<String>();
    json!({
        "machine": name,
        "kind": if crate::cloud::active() { "cloud" } else { "pc" },
        "at": now(),
    })
}

/// Контрольная сумма пакета без самого поля `checksum`.
pub fn checksum(bundle: &Value) -> String {
    let mut value = bundle.clone();
    if let Some(map) = value.as_object_mut() {
        map.remove("checksum");
    }
    let canonical = serde_json::to_vec(&value).unwrap_or_default();
    let mut hasher = Sha256::new();
    hasher.update(canonical);
    crate::store::hex(&hasher.finalize())
}

/// Собрать пакет из копии файла команды и частей. Крупные части уходят в отчёт, а не молча.
/// Источник не меняется: вызывающий только читает данные.
pub fn build(mut team: Value, mut parts: Vec<Value>, selected: &[String], origin: &Value, direction: &str) -> Result<(Value, Vec<Skipped>), String> {
    if !matches!(direction, "to-cloud" | "to-pc") {
        return Err("неизвестное направление переноса".into());
    }
    team = scrub(&select_team(&team, selected));
    let mut report = Vec::new();
    let id = crate::store::uid("tr");
    let mut kept = Vec::new();
    for part in parts.drain(..) {
        let part = scrub(&part);
        let name = part.get("part").and_then(Value::as_str).unwrap_or("").to_string();
        let kind = part.get("kind").and_then(Value::as_str).unwrap_or("").to_string();
        let size = bytes(&part);
        if size > MAX_PART_BYTES {
            report.push(skipped(&name, &kind, size, MAX_PART_BYTES, "часть больше предела: осталась на источнике"));
        } else {
            kept.push(part);
        }
    }
    let mut bundle = json!({
        "format": FORMAT,
        "version": VERSION,
        "id": id,
        "direction": direction,
        "createdAt": now(),
        "origin": origin,
        "team": team,
        "parts": kept,
        "skipped": report.iter().map(|s| serde_json::to_value(s).unwrap_or(Value::Null)).collect::<Vec<_>>(),
    });
    // Весь пакет тоже ограничен: сначала отдаём части, потом ботов. Ничего не удаляем из источника.
    loop {
        let size = bytes(&bundle);
        if size <= MAX_BUNDLE_BYTES {
            break;
        }
        let dropped = {
            let map = bundle.as_object_mut().ok_or("пакет должен быть объектом")?;
            let parts = map.get_mut("parts").and_then(Value::as_array_mut);
            match parts.and_then(|p| p.pop()) {
                Some(part) => Some(part),
                None => map.get_mut("team").and_then(|t| t.get_mut("bots")).and_then(Value::as_array_mut).and_then(|b| b.pop()),
            }
        };
        let Some(part) = dropped else { return Err("пакет слишком большой даже без частей: выбери меньше ботов".into()) };
        let name = part.get("part").and_then(Value::as_str).or_else(|| part.get("name").and_then(Value::as_str)).unwrap_or("bot").to_string();
        let kind = part.get("kind").and_then(Value::as_str).unwrap_or("team").to_string();
        let size = bytes(&part);
        let skipped_part = skipped(&name, &kind, size, MAX_BUNDLE_BYTES, "пакет больше предела: осталось на источнике");
        bundle["skipped"].as_array_mut().ok_or("нет списка пропущенных")?.push(serde_json::to_value(&skipped_part).unwrap_or(Value::Null));
        report.push(skipped_part);
    }
    let sum = checksum(&bundle);
    bundle["checksum"] = json!(sum);
    Ok((bundle, report))
}

/// Проверка чужого пакета перед применением: формат, версия, контрольная сумма, размер.
pub fn verify(bundle: &Value) -> Result<(), String> {
    if bytes(bundle) > MAX_BUNDLE_BYTES {
        return Err("пакет больше 1 МБ".into());
    }
    if bundle.get("format").and_then(Value::as_str) != Some(FORMAT) {
        return Err("это не пакет переноса Deloist".into());
    }
    if bundle.get("version").and_then(Value::as_u64) != Some(VERSION as u64) {
        return Err("неизвестная версия пакета переноса".into());
    }
    let want = bundle.get("checksum").and_then(Value::as_str).unwrap_or_default();
    if want.is_empty() || want != checksum(bundle) {
        return Err("пакет повреждён: контрольная сумма не совпала".into());
    }
    let team = bundle.get("team");
    if team.and_then(|t| t.get("format")).and_then(Value::as_str) != Some("deloist-team") {
        return Err("в пакете нет файла команды Deloist".into());
    }
    Ok(())
}

/// Перемаркировать пакет при возврате: он собран в облаке, но применяется на ПК.
pub fn retarget(mut bundle: Value, direction: &str) -> Result<Value, String> {
    if !matches!(direction, "to-cloud" | "to-pc") {
        return Err("неизвестное направление переноса".into());
    }
    bundle["direction"] = json!(direction);
    let sum = checksum(&bundle);
    bundle["checksum"] = json!(sum);
    verify(&bundle)?;
    Ok(bundle)
}

/// Слить след переноса с уже сохранённым: старые ACL остаются, новые поля добавляются.
fn merge_bots(state: &mut State, names: &[String], origin: &Value, acl: &Value) {
    for name in names {
        let entry = state.bots.entry(name.clone()).or_insert_with(|| json!({}));
        entry["origin"] = origin.clone();
        if let Some(new) = acl.get(name).and_then(Value::as_object) {
            for (key, value) in new {
                entry["acl"][key.as_str()] = value.clone();
            }
        } else if let Some(new) = acl.get(name) {
            entry["acl"] = new.clone();
        }
        entry["at"] = json!(now());
    }
}

/// Устойчивый след переноса: чтобы повтор не создал копию, а сбой не стёр происхождение и ACL.
#[derive(Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
struct State {
    applied: HashMap<String, Value>,
    bots: HashMap<String, Value>,
    last: Option<Value>,
}

#[derive(Default)]
pub struct Transfer {
    state: Mutex<State>,
}

impl Transfer {
    pub fn open(dir: &Path) -> Self {
        let text = std::fs::read_to_string(dir.join(FILE)).unwrap_or_default();
        let state = serde_json::from_str(&text).unwrap_or_default();
        Self { state: Mutex::new(state) }
    }

    fn path(&self, dir: &Path) -> PathBuf {
        dir.join(FILE)
    }

    fn save(&self, dir: &Path) {
        let Ok(text) = serde_json::to_string_pretty(&*self.state.lock().unwrap()) else { return };
        let path = self.path(dir);
        let tmp = path.with_extension("json.tmp");
        if std::fs::write(&tmp, text.as_bytes()).is_ok() {
            let _ = std::fs::rename(&tmp, &path);
        }
    }

    pub fn already_applied(&self, id: &str) -> bool {
        self.state.lock().unwrap().applied.contains_key(id)
    }

    pub fn bot_meta(&self, name: &str) -> Option<Value> {
        self.state.lock().unwrap().bots.get(name).cloned()
    }

    /// Запомнить результат. При ошибке части вызывающий кладёт failed в отчёт, но origin и ACL
    /// остаются в файле: следующий заход дополнит перенос, а не начнёт с чистого листа.
    pub fn remember(&self, dir: &Path, id: &str, names: &[String], origin: &Value, acl: &Value, report: &Value) {
        {
            let mut state = self.state.lock().unwrap();
            state.applied.insert(id.into(), json!({ "at": now(), "report": report }));
            state.last = Some(json!({ "at": now(), "report": report }));
            merge_bots(&mut state, names, origin, acl);
        }
        self.save(dir);
    }

    /// Сбой: запоминаем происхождение и ACL, но не помечаем пакет применённым — повтор возможен.
    pub fn remember_failed(&self, dir: &Path, names: &[String], origin: &Value, acl: &Value, report: &Value) {
        {
            let mut state = self.state.lock().unwrap();
            merge_bots(&mut state, names, origin, acl);
            state.last = Some(json!({ "at": now(), "report": report }));
        }
        self.save(dir);
    }

    pub fn last(&self) -> Value {
        let state = self.state.lock().unwrap();
        state
            .last
            .clone()
            .or_else(|| state.applied.values().max_by_key(|v| v["at"].as_i64().unwrap_or(0)).cloned())
            .unwrap_or(Value::Null)
    }
}

/// Последний отчёт для окна: что уехало, что пропущено, что не применилось.
pub fn report_from_last(app: &App) -> Value {
    let last = app.transfer.last();
    if last.is_null() {
        return json!({ "report": Value::Null });
    }
    json!({ "report": last.get("report").cloned().unwrap_or(Value::Null), "at": last.get("at").cloned().unwrap_or(Value::Null) })
}

/// Собрать часть с текстом brain.md бота.
pub fn brain_part(app: &App, name: &str, title: &str) -> Value {
    let text = app.store.bots().into_iter().find(|b| b.name == name).map(|b| b.brain).unwrap_or_default();
    json!({ "part": title, "kind": "brain", "bot": name, "text": scrub_text(&text).chars().take(MAX_TEXT).collect::<String>() })
}

/// Собрать часть с памятью бота, общей памятью и памятью его команд.
pub async fn memory_parts(app: &Arc<App>, name: &str, title: &str) -> Vec<Value> {
    let mut scopes = vec![crate::memory::SHARED.to_string()];
    if let Some(bot) = app.store.bots().into_iter().find(|b| b.name == name) {
        scopes.push(bot.id.clone());
        scopes.extend(crate::memory::team_scopes(app, &bot.id));
    }
    let mut out = Vec::new();
    for (index, scope) in scopes.into_iter().enumerate() {
        let facts = crate::memory::facts(app, &scope).await;
        if facts.is_empty() {
            continue;
        }
        let facts = facts.into_iter().take(MAX_FACTS).map(|f| serde_json::to_value(scrub(&serde_json::to_value(f).unwrap_or(Value::Null))).unwrap_or(Value::Null)).collect::<Vec<_>>();
        out.push(json!({ "part": format!("{title}/memory/{index}"), "kind": "memory", "bot": name, "scope": scope, "facts": facts }));
    }
    out
}

/// ACL бота — это его права доступа в программе. В пакет идут только описательные поля,
/// а не секреты и не настройки компьютера.
pub fn acl_part(app: &App, name: &str, title: &str) -> Value {
    let acl = app
        .store
        .bots()
        .into_iter()
        .find(|b| b.name == name)
        .map(|b| {
            json!({
                "approval": b.approval,
                "browserSites": b.browser_sites,
                "alwaysAsk": b.always_ask,
                "team": b.team,
            })
        })
        .unwrap_or_else(|| json!({}));
    json!({ "part": title, "kind": "acl", "bot": name, "acl": scrub(&acl) })
}

/// Собрать пакет для выбранных ботов. Возврат пакета и список пропущенных частей.
pub async fn build_for_bots(app: &Arc<App>, selected: &[String], origin: &Value, direction: &str) -> Result<(Value, Vec<Skipped>), String> {
    let team = crate::team::export_with(app, false);
    let mut parts = Vec::new();
    for name in selected {
        let title = format!("bot/{name}");
        parts.push(brain_part(app, name, &title));
        parts.extend(memory_parts(app, name, &title).await);
        parts.push(acl_part(app, name, &title));
    }
    build(team, parts, selected, origin, direction)
}

/// Отчёт о применении: что доехало, что пропущено, что не удалось применить.
pub fn report(applied: &Value, skipped: &[Skipped], failed: &[String]) -> Value {
    json!({
        "applied": applied,
        "skipped": skipped.iter().map(|s| serde_json::to_value(s).unwrap_or(Value::Null)).collect::<Vec<_>>(),
        "failed": failed,
    })
}

/// Применить пакет на доверенной машине. Повтор не дублирует ботов; сбой одной части не
/// отменяет остальные и не стирает происхождение и ACL.
pub async fn apply(app: &Arc<App>, bundle: &Value) -> Result<Value, String> {
    verify(bundle)?;
    let id = bundle.get("id").and_then(Value::as_str).unwrap_or_default();
    let skipped: Vec<Skipped> = bundle
        .get("skipped")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|v| serde_json::from_value(v.clone()).ok())
        .collect();
    if app.transfer.already_applied(id) {
        return Ok(json!({
            "applied": { "bots": 0, "skills": 0, "rooms": 0, "routines": 0, "duplicate": true },
            "skipped": skipped.iter().map(|s| serde_json::to_value(s).unwrap_or(Value::Null)).collect::<Vec<_>>(),
            "failed": [],
        }));
    }
    let origin = scrub(bundle.get("origin").unwrap_or(&Value::Null));
    let team = bundle.get("team").cloned().ok_or("в пакете нет команды")?;
    let names: Vec<String> = team
        .get("bots")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|b| b.get("name").and_then(Value::as_str).map(str::to_string))
        .collect();
    let mut failed = Vec::new();
    // Возврат и повтор после сбоя не должны плодить копий: боты, которые уже приехали когда-то
    // этим же переносом, обновляются на месте, а не создаются заново.
    let existing: HashMap<String, crate::store::Bot> = app.store.bots().into_iter().map(|b| (b.name.clone(), b)).collect();
    let reuse: Vec<String> = names
        .iter()
        .filter(|name| {
            existing.get(*name).is_some_and(|bot| app.transfer.bot_meta(name).is_some() || bot.cloud_only)
        })
        .cloned()
        .collect();
    let fresh: Vec<String> = names.iter().filter(|name| !reuse.contains(name)).cloned().collect();
    let mut applied = json!({ "bots": reuse.len(), "skills": 0, "rooms": 0, "routines": 0, "workflows": 0 });
    let mut acl_all = json!({});
    if !fresh.is_empty() {
        match crate::team::import(app, &select_team(&team, &fresh)).await {
            Ok(summary) => applied = summary,
            Err(e) => {
                // Даже полный сбой импорта не стирает след: повтор дополнит перенос, а не начнёт с нуля.
                let result = report(&applied, &skipped, &[format!("импорт команды: {e}")]);
                app.transfer.remember_failed(app.store.data_dir(), &names, &origin, &acl_all, &result);
                return Err(e);
            }
        }
    }

    // Имена в файле команды — не id; после импорта находим новые id по именам.
    let bots: HashMap<String, crate::store::Bot> = app.store.bots().into_iter().map(|b| (b.name.clone(), b)).collect();
    let direction = bundle.get("direction").and_then(Value::as_str).unwrap_or("to-cloud");
    for name in &names {
        let Some(bot) = bots.get(name) else {
            failed.push(format!("бот {name}: не создался"));
            continue;
        };
        acl_all[name.as_str()] = json!({});
        for part in bundle.get("parts").and_then(Value::as_array).into_iter().flatten() {
            if part.get("bot").and_then(Value::as_str) != Some(name.as_str()) {
                continue;
            }
            let kind = part.get("kind").and_then(Value::as_str).unwrap_or_default();
            let result: Result<(), String> = match kind {
                "brain" => match part.get("text").and_then(Value::as_str) {
                    Some(text) => crate::brain::set(app, &bot.id, text).await.map(|_| ()),
                    None => Err("нет текста brain".into()),
                },
                "memory" => {
                    let facts: Vec<crate::memory::Fact> = part.get("facts").and_then(Value::as_array).into_iter().flatten().filter_map(|f| serde_json::from_value(scrub(f).clone()).ok()).collect();
                    let (_, errors) = crate::memory::import_transfer_facts(app, &facts).await;
                    if errors.is_empty() { Ok(()) } else { Err(errors.join("; ")) }
                }
                "acl" => {
                    let acl = part.get("acl").cloned().unwrap_or(Value::Null);
                    acl_all[name.as_str()] = acl.clone();
                    let mut patch = json!({});
                    if let Some(obj) = acl.as_object() {
                        for key in ["approval", "browserSites", "alwaysAsk", "team"] {
                            if let Some(v) = obj.get(key) {
                                patch[key] = v.clone();
                            }
                        }
                    }
                    app.store.update_bot(&bot.id, &patch).await.map(|_| ()).map_err(|e| e.to_string())
                }
                _ => Err(format!("неизвестная часть {kind}")),
            };
            if let Err(e) = result {
                failed.push(format!("бот {name}, {kind}: {e}"));
            }
        }
        // Включённые рутины и cloud-only восстанавливаем после всех частей: сбой одной из них
        // не должен оставить бота в половинчатом состоянии.
        let want_cloud = direction == "to-cloud";
        if let Err(e) = app.store.update_bot(&bot.id, &json!({ "cloudOnly": want_cloud })).await {
            failed.push(format!("бот {name}, cloudOnly: {e}"));
        }
        for routine in team.get("routines").and_then(Value::as_array).into_iter().flatten() {
            if routine.get("bot").and_then(Value::as_str) != Some(name.as_str()) {
                continue;
            }
            let title = routine.get("title").and_then(Value::as_str).unwrap_or_default().to_string();
            if let Some(found) = app.store.routines().into_iter().find(|r| r.bot_id == bot.id && r.title == title) {
                if let Err(e) = app.store.update_routine(&found.id, &json!({ "enabled": true }), false).await {
                    failed.push(format!("рутина {title}: {e}"));
                }
            }
        }
    }
    let result = report(&applied, &skipped, &failed);
    app.transfer.remember(app.store.data_dir(), id, &names, &origin, &acl_all, &result);
    app.turns.emit(json!({ "type": "transfer_finished", "report": result.clone(), "direction": direction }));
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn team() -> Value {
        json!({
            "format": "deloist-team", "version": 1,
            "bots": [
                { "name": "А", "persona": "пароль: sk-secret-1", "skills": ["s"] },
                { "name": "Б", "persona": "б" }
            ],
            "skills": [{ "name": "s", "body": "b" }],
            "rooms": [
                { "name": "R1", "members": ["А"] },
                { "name": "R2", "members": ["Б"] }
            ],
            "routines": [
                { "bot": "А", "title": "t", "prompt": "p" },
                { "bot": "Б", "title": "t2", "prompt": "p2" }
            ],
            "workflows": [{ "title": "w", "graph": {} }],
        })
    }

    #[test]
    fn select_team_keeps_only_the_bot_and_its_relations() {
        let got = select_team(&team(), &["А".to_string()]);
        assert_eq!(got["bots"].as_array().unwrap().len(), 1);
        assert_eq!(got["bots"][0]["name"], "А");
        assert_eq!(got["rooms"].as_array().unwrap().len(), 1);
        assert_eq!(got["routines"].as_array().unwrap().len(), 1);
        assert!(got["workflows"].as_array().unwrap().is_empty());
    }

    #[test]
    fn bundle_has_no_secrets_and_reports_oversized_parts() {
        let origin = json!({ "machine": "pc-1", "kind": "pc" });
        let big = json!({ "part": "bot/А/memory/0", "kind": "memory", "facts": vec!["x".repeat(MAX_PART_BYTES + 1)] });
        let (bundle, skipped) = build(team(), vec![big], &["А".to_string()], &origin, "to-cloud").unwrap();
        assert!(!bundle.to_string().contains("sk-secret-1"), "секрет попал в пакет: {bundle}");
        assert_eq!(skipped.len(), 1, "крупная часть должна быть в отчёте");
        assert_eq!(skipped[0].limit, MAX_PART_BYTES);
        assert_eq!(bundle["skipped"].as_array().unwrap().len(), 1);
        assert_eq!(bundle["origin"]["machine"], "pc-1");
        verify(&bundle).unwrap();
    }

    #[test]
    fn verify_rejects_a_changed_bundle() {
        let (mut bundle, _) = build(team(), vec![], &["А".to_string()], &json!({}), "to-pc").unwrap();
        verify(&bundle).unwrap();
        bundle["team"]["bots"][0]["persona"] = json!("подмена");
        assert!(verify(&bundle).is_err(), "подмену нужно ловить по сумме");
    }

    #[test]
    fn repeated_failure_keeps_origin_and_merges_acl() {
        let dir = std::env::temp_dir().join(format!("deloist-transfer-test-{}", crate::store::uid("x")));
        std::fs::create_dir_all(&dir).unwrap();
        let state = Transfer::open(&dir);
        let report = json!({ "failed": ["bot/А/memory: сбой"], "applied": {} });
        state.remember(&dir, "tr-1", &["А".to_string()], &json!({ "machine": "pc" }), &json!({ "А": { "approval": "ask" } }), &report);
        state.remember(&dir, "tr-2", &["А".to_string()], &json!({ "machine": "cloud" }), &json!({ "А": { "browserSites": ["site"] } }), &report);
        let meta = state.bot_meta("А").unwrap();
        assert_eq!(meta["origin"]["machine"], "cloud");
        assert_eq!(meta["acl"]["approval"], "ask", "старое правило ACL потерялось при повторе");
        assert_eq!(meta["acl"]["browserSites"][0], "site");
        assert!(state.already_applied("tr-1"));
        let reopened = Transfer::open(&dir);
        assert_eq!(reopened.bot_meta("А").unwrap()["acl"]["approval"], "ask", "след переноса не пережил перезапуск");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn secret_fields_are_removed_at_any_depth() {
        let value = json!({ "a": { "apiKey": "sk-1", "hookToken": "t", "name": "ok", "text": "пароль: hunter2" }, "b": [{ "token": "x", "keep": 1 }] });
        let clean = scrub(&value);
        let text = clean.to_string();
        assert!(!text.contains("sk-1") && !text.contains("hunter2") && !text.contains("hookToken"), "{text}");
        assert_eq!(clean["a"]["name"], "ok");
        assert_eq!(clean["b"][0]["keep"], 1);
    }
}
