//! Лимиты подписки Google у Gemini (Antigravity CLI) для кружков в левом меню.
//!
//! Команды `/usage` у агента нет. Квоту, как и сама программа, спрашиваем у сервиса Google Code Assist:
//! `loadCodeAssist` даёт проект аккаунта, `retrieveUserQuotaSummary` — группы квоты («Gemini Models»,
//! «Claude and GPT models») с окнами 5 часов и неделя. Google отвечает только на User-Agent настольной
//! программы Antigravity. Сводку отдаёт только хост `daily-cloudcode-pa`.
//!
//! Токен берём из файла входа, который ведёт сама программа, и только если он ещё живёт: обновлять его
//! нечем, у нас нет секрета клиента. Токен уходит только на два адреса Google ниже и никуда не пишется:
//! ни в журнал, ни в ответы API, ни в ошибки. Вход ключом API квоты не даёт.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{LazyLock, Mutex};
use std::time::{Duration, Instant};

use chrono::{DateTime, Local, Utc};
use serde_json::{json, Value};

const LOAD_URL: &str = "https://cloudcode-pa.googleapis.com/v1internal:loadCodeAssist";
const QUOTA_URL: &str = "https://daily-cloudcode-pa.googleapis.com/v1internal:retrieveUserQuotaSummary";
/// Версия настольной Antigravity: группу «Gemini Models» Google отдаёт только ей, не CLI.
const DESKTOP_VERSION: &str = "2.31.0";
const CALL_TIMEOUT: Duration = Duration::from_secs(10);
/// Квота меняется медленно, а запрос идёт в сеть: не чаще раза в 5 минут на аккаунт.
const FRESH_FOR: Duration = Duration::from_secs(5 * 60);

type Cached = (Instant, Vec<Value>, &'static str, Option<std::time::SystemTime>, Option<i64>);
static CACHE: LazyLock<Mutex<HashMap<PathBuf, Cached>>> = LazyLock::new(|| Mutex::new(HashMap::new()));

pub fn token_file(home: &Path) -> PathBuf {
    home.join("antigravity-acp").join("antigravity-oauth-token")
}

/// Токен доступа из файла входа, если он проживёт ещё минуту. Иначе `None`: программа сама обновит его
/// при следующем ходе.
fn access_token(text: &str, now: DateTime<Utc>) -> Option<String> {
    let value: Value = serde_json::from_str(text).ok()?;
    let token = value["token"]["access_token"].as_str()?.trim();
    let expiry = DateTime::parse_from_rfc3339(value["token"]["expiry"].as_str()?).ok()?.with_timezone(&Utc);
    (!token.is_empty() && expiry > now + chrono::Duration::seconds(60)).then(|| token.to_string())
}

fn user_agent() -> String {
    let os = match std::env::consts::OS {
        "macos" => "darwin",
        other => other,
    };
    let arch = match std::env::consts::ARCH {
        "x86_64" => "amd64",
        "aarch64" => "arm64",
        other => other,
    };
    format!("antigravity/{DESKTOP_VERSION} {os}/{arch}")
}

/// Лимиты аккаунта в виде, который понимает левое меню: `[{label, left, resets}]`. Пустой список —
/// показать нечего (нет входа Google, токен устарел, Google не ответил).
pub async fn limits(home: &Path) -> (Vec<Value>, &'static str, Option<i64>) {
    let modified = std::fs::metadata(token_file(home)).and_then(|m| m.modified()).ok();
    if let Some((at, cached, status, token_modified, read_at)) = CACHE.lock().unwrap().get(home) {
        if at.elapsed() < FRESH_FOR && *token_modified == modified {
            return (cached.clone(), status, *read_at);
        }
    }
    // Токен живёт час и обновляется только ходом Gemini: без свежего токена показываем последнее
    // прочитанное, а не пустые кружки. Неудачу тоже помним, иначе каждое открытие меню шло бы в сеть.
    let (previous, previous_at) = CACHE.lock().unwrap().get(home).map(|(_, rows, _, _, at)| (rows.clone(), *at)).unwrap_or_default();
    let (rows, status, read_at) = match read(home).await {
        Ok(rows) if !rows.is_empty() => (rows, "ok", Some(crate::store::now())),
        Ok(_) => (previous, "unavailable", previous_at),
        Err(status) => (previous, status, previous_at),
    };
    CACHE.lock().unwrap().insert(home.to_path_buf(), (Instant::now(), rows.clone(), status, modified, read_at));
    (rows, status, read_at)
}

async fn read(home: &Path) -> Result<Vec<Value>, &'static str> {
    let text = tokio::fs::read_to_string(token_file(home)).await.map_err(|_| "auth_expired")?;
    let token = access_token(&text, Utc::now()).ok_or("auth_expired")?;
    let client = reqwest::Client::builder().timeout(CALL_TIMEOUT).user_agent(user_agent()).build().map_err(|_| "unavailable")?;
    let call = |url: &'static str, body: Value| {
        let request = client.post(url).bearer_auth(&token).json(&body);
        async move {
            let response = request.send().await.map_err(|_| "unavailable")?;
            if response.status() == reqwest::StatusCode::UNAUTHORIZED { return Err("auth_expired"); }
            if !response.status().is_success() {
                return Err("unavailable");
            }
            response.json::<Value>().await.map_err(|_| "unavailable")
        }
    };
    let loaded = call(LOAD_URL, json!({})).await?;
    let project = loaded["cloudaicompanionProject"].as_str().filter(|p| !p.is_empty()).ok_or("unavailable")?.to_string();
    let summary = call(QUOTA_URL, json!({ "project": project })).await?;
    Ok(limits_of(&summary, Local::now()))
}

/// Ответ `retrieveUserQuotaSummary` → строки лимитов. Группа Gemini идёт первой и без подписи
/// (её окна — главные кружки), прочие группы подписаны: «5h limit · Claude and GPT models».
pub fn limits_of(summary: &Value, now: DateTime<Local>) -> Vec<Value> {
    let mut groups: Vec<(bool, String, Vec<Value>)> = Vec::new();
    for group in summary["groups"].as_array().into_iter().flatten() {
        let name = group["displayName"].as_str().unwrap_or_default().trim().to_string();
        if name.is_empty() {
            continue;
        }
        let gemini = name.to_lowercase().starts_with("gemini");
        // Окно → самое тесное ведро этого окна в группе.
        let mut windows: Vec<(&'static str, f64, String)> = Vec::new();
        for bucket in group["buckets"].as_array().into_iter().flatten() {
            let Some(fraction) = bucket["remainingFraction"].as_f64() else { continue };
            let label = format!("{} {}", bucket["window"].as_str().unwrap_or_default(), bucket["displayName"].as_str().unwrap_or_default()).to_lowercase();
            let window = if label.contains("week") {
                "Weekly limit"
            } else if label.contains("5h") || label.contains("5-h") || label.contains("5 h") || label.contains("five") {
                "5h limit"
            } else {
                continue;
            };
            let resets = bucket["resetTime"].as_str().and_then(|t| DateTime::parse_from_rfc3339(t).ok()).map(|t| reset_text(t.with_timezone(&Local), now)).unwrap_or_default();
            match windows.iter_mut().find(|(w, _, _)| *w == window) {
                Some(existing) if existing.1 <= fraction => {}
                Some(existing) => *existing = (window, fraction, resets),
                None => windows.push((window, fraction, resets)),
            }
        }
        // Сначала 5 часов, потом неделя.
        windows.sort_by_key(|(w, _, _)| if *w == "5h limit" { 0 } else { 1 });
        let rows = windows
            .into_iter()
            .map(|(window, fraction, resets)| {
                let label = if gemini { window.to_string() } else { format!("{window} · {name}") };
                json!({ "label": label, "left": (fraction * 100.0).round().clamp(0.0, 100.0) as i64, "resets": resets })
            })
            .collect::<Vec<_>>();
        if !rows.is_empty() {
            groups.push((gemini, name, rows));
        }
    }
    groups.sort_by_key(|(gemini, _, _)| !*gemini);
    groups.into_iter().flat_map(|(_, _, rows)| rows).collect()
}

/// «14:48» сегодня, «19.11 12:02» позже.
fn reset_text(at: DateTime<Local>, now: DateTime<Local>) -> String {
    if at.date_naive() == now.date_naive() {
        at.format("%H:%M").to_string()
    } else {
        at.format("%d.%m %H:%M").to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    #[test]
    fn only_a_live_token_is_used() {
        let now = Utc.with_ymd_and_hms(2026, 11, 13, 10, 0, 0).unwrap();
        let file = |exp: &str| format!(r#"{{"token":{{"access_token":"ya29.abc","expiry":"{exp}"}},"id_token_email":"a@b.c"}}"#);
        assert_eq!(access_token(&file("2026-11-13T11:00:00.5+00:00"), now).as_deref(), Some("ya29.abc"));
        assert_eq!(access_token(&file("2026-11-13T10:00:30Z"), now), None, "истекает меньше чем через минуту");
        assert_eq!(access_token(&file("2026-11-13T09:00:00Z"), now), None, "истёк");
        assert_eq!(access_token(r#"{"token":{}}"#, now), None);
        assert_eq!(access_token("not json", now), None);
    }

    #[tokio::test]
    async fn expired_token_is_an_auth_state_not_an_unsupported_provider() {
        let home = std::env::temp_dir().join(format!("gemini-quota-{}", std::process::id()));
        let file = token_file(&home);
        std::fs::create_dir_all(file.parent().unwrap()).unwrap();
        std::fs::write(&file, r#"{"token":{"access_token":"redacted","expiry":"2020-01-01T00:00:00Z"}}"#).unwrap();
        let (rows, status, _) = limits(&home).await;
        assert!(rows.is_empty());
        assert_eq!(status, "auth_expired");
        // CLI заменил файл: закэшированная ошибка должна быть отброшена сразу.
        CACHE.lock().unwrap().get_mut(&home).unwrap().3 = Some(std::time::SystemTime::UNIX_EPOCH);
        std::fs::write(&file, r#"{"token":{"access_token":"redacted","expiry":"2020-01-02T00:00:00Z"}}"#).unwrap();
        let (_, _, _) = limits(&home).await;
        assert_ne!(CACHE.lock().unwrap().get(&home).unwrap().3, Some(std::time::SystemTime::UNIX_EPOCH));
        let _ = std::fs::remove_dir_all(home);
    }

    #[test]
    fn quota_groups_become_limit_rows() {
        let now = Local.with_ymd_and_hms(2026, 11, 13, 10, 0, 0).unwrap();
        let today = Local.with_ymd_and_hms(2026, 11, 13, 14, 35, 0).unwrap().to_rfc3339();
        let later = Local.with_ymd_and_hms(2026, 11, 19, 12, 2, 0).unwrap().to_rfc3339();
        let summary = json!({ "groups": [
            { "displayName": "Claude and GPT models", "description": "Models within this group: Claude Opus, GPT-OSS",
              "buckets": [{ "displayName": "Weekly Limit Remaining", "window": "weekly", "resetTime": later, "remainingFraction": 1 }] },
            { "displayName": "Gemini Models",
              "buckets": [
                { "displayName": "Weekly Limit Remaining", "window": "weekly", "resetTime": later, "remainingFraction": 0.99 },
                { "displayName": "5h Limit Remaining", "window": "5h", "resetTime": today, "remainingFraction": 0.71 },
                { "displayName": "5h Limit Remaining", "window": "5h", "resetTime": today, "remainingFraction": 0.9 } ] },
            { "displayName": "", "buckets": [{ "window": "weekly", "remainingFraction": 0.1 }] }
        ] });
        let rows = limits_of(&summary, now);
        assert_eq!(rows, vec![
            json!({ "label": "5h limit", "left": 71, "resets": "14:35" }),
            json!({ "label": "Weekly limit", "left": 99, "resets": "19.11 12:02" }),
            json!({ "label": "Weekly limit · Claude and GPT models", "left": 100, "resets": "19.11 12:02" }),
        ]);
        assert!(limits_of(&json!({}), now).is_empty());
    }

    #[test]
    fn the_user_agent_names_the_desktop_app() {
        let ua = user_agent();
        assert!(ua.starts_with("antigravity/2.31.0 "), "{ua}");
        assert!(!ua.contains("x86_64"), "{ua}");
    }
}

