//! cli-драйвер: вместо claude/codex подставляем заглушку, которая читает промпт из stdin.

use crate::common;

use std::path::Path;

use common::{Crew, BIN};
use serde_json::json;

#[tokio::test]
async fn cli_gets_the_prompt_and_its_answer_is_saved() {
    let fake = Path::new(BIN).parent().unwrap().join("examples").join("fake_cli.exe");
    assert!(fake.exists(), "нет заглушки: {}", fake.display());

    let crew = Crew::start("cli").await;
    crew.ok(
        "PUT",
        "/api/settings",
        Some(json!({ "driver": "cli", "cliCommand": format!("\"{}\"", fake.display()) })),
    )
    .await;
    crew.listen().await;

    let state = crew.ok("GET", "/api/state", None).await;
    let bot = state["bots"][1]["id"].as_str().unwrap().to_string();
    crew.say(&bot, "проверка маяк").await;

    let done = crew.wait_event("конец хода", |e| e["type"] == "turn_end").await;
    assert_eq!(done["failed"], false, "ход упал: {:?}", crew.find_event(|e| e["type"] == "error"));

    let thread = crew.ok("GET", &format!("/api/bots/{bot}/thread?limit=10"), None).await;
    let answer = thread["messages"].as_array().unwrap().iter().rev().find(|m| m["role"] == "assistant").unwrap().clone();
    let text = answer["text"].as_str().unwrap();
    assert!(text.contains("получено символов"), "неожиданный ответ: {text}");
    assert!(text.contains("есть слово «маяк»: да"), "промпт не дошёл целиком: {text}");
}

#[tokio::test]
async fn missing_cli_is_reported_not_swallowed() {
    let crew = Crew::start("cli-missing").await;
    crew.ok("PUT", "/api/settings", Some(json!({ "driver": "cli", "cliCommand": "crew-no-such-cli -p" }))).await;
    crew.listen().await;

    let state = crew.ok("GET", "/api/state", None).await;
    let bot = state["bots"][0]["id"].as_str().unwrap().to_string();
    crew.say(&bot, "привет").await;

    let done = crew.wait_event("конец хода", |e| e["type"] == "turn_end").await;
    assert_eq!(done["failed"], true);
    let error = crew.find_event(|e| e["type"] == "error").unwrap();
    assert!(error["message"].as_str().unwrap().contains("не найден"), "{error}");
}
