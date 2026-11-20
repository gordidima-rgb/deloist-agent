//! Фокусированные проверки переноса ботов и флага «только в облаке».
use common::Crew;
use serde_json::{json, Value};

use crate::common;

async fn agent_call(crew: &Crew, method: reqwest::Method, path: &str, body: Option<Value>) -> u16 {
    let token = std::fs::read_to_string(crew.data_dir.join("runtime.json"))
        .ok()
        .and_then(|text| serde_json::from_str::<Value>(&text).ok())
        .and_then(|json| json["token"].as_str().map(str::to_string))
        .unwrap_or_default();
    let mut req = crew.client.request(method, format!("{}{path}", crew.base)).bearer_auth(&token);
    if let Some(body) = body {
        req = req.json(&body);
    }
    req.send().await.unwrap().status().as_u16()
}

#[tokio::test]
async fn transfer_bundle_round_trips_without_secrets_and_without_duplicates() {
    let crew = Crew::start("transfer-roundtrip").await;
    let state = crew.ok("GET", "/api/state", None).await;
    let bot_id = state["bots"][0]["id"].as_str().unwrap().to_string();
    let name = state["bots"][0]["name"].as_str().unwrap().to_string();
    // Секрет в инструкции: в пакет он попасть не должен.
    crew.ok("PATCH", &format!("/api/bots/{bot_id}"), Some(json!({ "persona": "секрет sk-transfer-marker-1" }))).await;

    let exported = crew.ok("GET", &format!("/api/transfer/export?bots={name}"), None).await;
    let bundle = exported["bundle"].clone();
    assert_eq!(bundle["format"], "deloist-transfer");
    assert_eq!(bundle["origin"]["kind"], "pc");
    assert_eq!(bundle["team"]["bots"][0]["name"], name);
    assert!(!bundle.to_string().contains("sk-transfer-marker-1"), "секрет попал в пакет: {bundle}");
    assert!(bundle["parts"].as_array().is_some_and(|parts| parts.iter().any(|p| p["kind"] == "brain" || p["kind"] == "acl")));

    let before = crew.ok("GET", "/api/state", None).await["bots"].as_array().unwrap().len();
    let first = crew.ok("POST", "/api/transfer/import", Some(json!({ "bundle": bundle }))).await;
    assert_eq!(first["report"]["applied"]["bots"], 1, "первый импорт должен создать бота");
    let after = crew.ok("GET", "/api/state", None).await["bots"].as_array().unwrap().len();
    assert_eq!(after, before + 1);

    let second = crew.ok("POST", "/api/transfer/import", Some(json!({ "bundle": bundle }))).await;
    assert_eq!(second["report"]["applied"]["duplicate"], true, "повтор не должен дублировать ботов");
    assert_eq!(crew.ok("GET", "/api/state", None).await["bots"].as_array().unwrap().len(), after);
    // Отчёт последнего переноса доступен окну и после повторного запроса.
    assert!(!crew.ok("GET", "/api/transfer/status", None).await["report"].is_null());
}

#[tokio::test]
async fn cloud_only_flag_is_enforced_by_the_real_dispatch() {
    let mut crew = Crew::start("transfer-cloud-only").await;
    let state = crew.ok("GET", "/api/state", None).await;
    let bot = state["bots"][0]["id"].as_str().unwrap().to_string();
    crew.listen().await;
    crew.ok("PATCH", &format!("/api/bots/{bot}"), Some(json!({ "cloudOnly": true }))).await;
    // Перезапуск: флаг должен читаться из сохранённого состояния, а не из памяти окна.
    crew.keep_data();
    let dir = crew.data_dir.clone();
    drop(crew);
    let crew = Crew::start_at(dir, &[]).await;
    let state = crew.ok("GET", "/api/state", None).await;
    assert_eq!(state["bots"][0]["cloudOnly"], true, "флаг не пережил перезапуск");
    let (status, error) = crew.send("POST", &format!("/api/bots/{bot}/messages"), Some(json!({ "text": "привет" }))).await;
    assert_eq!(status, 409, "ход облачного бота должен быть отказан на ПК");
    assert!(error["error"].as_str().unwrap_or_default().contains("только в облаке"), "не то сообщение: {error}");
}

#[tokio::test]
async fn transfer_routes_are_not_reachable_by_the_program_key() {
    let crew = Crew::start("transfer-guard").await;
    let name = crew.ok("GET", "/api/state", None).await["bots"][0]["name"].as_str().unwrap().to_string();
    for (method, path) in [
        (reqwest::Method::GET, "/api/transfer/export".to_string()),
        (reqwest::Method::GET, "/api/transfer/status".to_string()),
        (reqwest::Method::POST, "/api/transfer/import".to_string()),
        (reqwest::Method::POST, "/api/transfer/to-cloud".to_string()),
        (reqwest::Method::POST, "/api/transfer/from-cloud".to_string()),
        (reqwest::Method::POST, "/api/transfer/to-pc".to_string()),
    ] {
        let status = agent_call(&crew, method.clone(), &path, Some(json!({ "bots": [name] }))).await;
        assert_eq!(status, 403, "ключ программы прошёл на {path}");
    }
}
