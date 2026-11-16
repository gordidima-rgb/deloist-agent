//! Ветка разговора: новый бот с настройками исходного и тредом до выбранного сообщения.
use crate::common;
use common::Crew;
use serde_json::{json, Value};

/// Записи треда пишем напрямую: сервер читает файл на каждый запрос, как и в жизни.
fn write_thread(crew: &Crew, bot_id: &str, lines: &[Value]) {
    let dir = crew.data_dir.join("threads");
    std::fs::create_dir_all(&dir).unwrap();
    let text = lines.iter().map(|line| line.to_string()).collect::<Vec<_>>().join("\n") + "\n";
    std::fs::write(dir.join(format!("{bot_id}.jsonl")), text).unwrap();
}

async fn new_source(crew: &Crew) -> String {
    let made = crew
        .ok(
            "POST",
            "/api/bots",
            Some(json!({
                "name": "Аня",
                "role": "редактор",
                "emoji": "🦊",
                "color": "#123456",
                "persona": "Ты — редактор. Пиши коротко.",
                "driver": "api",
                "model": "model-x",
                "modelLevel": "strong",
                "effort": "high",
                "approval": "auto",
                "skills": ["skill-1"],
                "rank": "specialist",
                "skin": 9
            })),
        )
        .await;
    made["bot"]["id"].as_str().unwrap().to_string()
}

#[tokio::test]
async fn a_branch_copies_settings_and_the_thread_up_to_the_message() {
    let crew = Crew::start("branch-message").await;
    let source = new_source(&crew).await;
    crew.ok(
        "POST",
        "/api/memory/facts",
        Some(json!({ "scope": source, "kind": "project", "title": "Факт источника", "body": "В ветку не переходит" })),
    )
    .await;
    let lines = vec![
        json!({ "id": "m1", "role": "user", "text": "первый" }),
        json!({ "id": "m2", "role": "assistant", "text": "второй" }),
        json!({ "id": "m3", "role": "user", "text": "третий" }),
        json!({ "id": "m4", "role": "assistant", "text": "четвёртый" }),
    ];
    write_thread(&crew, &source, &lines);

    let made = crew
        .ok("POST", &format!("/api/bots/{source}/branch"), Some(json!({ "messageId": "m2" })))
        .await;
    let branch = &made["bot"];
    let branch_id = branch["id"].as_str().unwrap().to_string();
    assert_ne!(branch_id, source, "ветка — отдельный бот");
    assert_eq!(branch["name"], "Аня · ветка");
    for (field, expected) in [
        ("role", json!("редактор")),
        ("emoji", json!("🦊")),
        ("color", json!("#123456")),
        ("persona", json!("Ты — редактор. Пиши коротко.")),
        ("driver", json!("api")),
        ("model", json!("model-x")),
        ("modelLevel", json!("strong")),
        ("effort", json!("high")),
        ("approval", json!("auto")),
        ("rank", json!("specialist")),
        ("skin", json!(9)),
    ] {
        assert_eq!(branch[field], expected, "не скопировано поле {field}: {branch}");
    }
    assert_eq!(branch["skills"], json!(["skill-1"]), "навыки переходят: {branch}");
    assert_eq!(branch["pet"], false, "питомец не копируется");
    assert_eq!(branch["browserSites"], json!([]), "сайты браузера не копируются");

    let copied = crew.ok("GET", &format!("/api/bots/{branch_id}/thread"), None).await;
    let copied = copied["messages"].as_array().unwrap();
    assert_eq!(copied.len(), 2, "в ветке только переписка до сообщения: {copied:?}");
    assert_eq!(copied[0]["text"], "первый");
    assert_eq!(copied[1]["text"], "второй");
    assert_ne!(copied[0]["id"], "m1", "у записей ветки свои id");
    assert_ne!(copied[1]["id"], "m2", "у записей ветки свои id");

    let original = crew.ok("GET", &format!("/api/bots/{source}/thread"), None).await;
    let original = original["messages"].as_array().unwrap();
    assert_eq!(original.len(), 4, "исходный тред не тронут: {original:?}");
    assert_eq!(original[0]["id"], "m1");
    assert_eq!(original[1]["id"], "m2");
    assert_eq!(original[2]["id"], "m3");
    assert_eq!(original[3]["id"], "m4");

    let memory = crew.ok("GET", &format!("/api/bots/{branch_id}/memory"), None).await;
    assert_eq!(memory["facts"], json!([]), "память не копируется: {memory}");

    let another = crew
        .ok("POST", &format!("/api/bots/{source}/branch"), Some(json!({ "messageId": "m2" })))
        .await;
    assert_eq!(another["bot"]["name"], "Аня · ветка 2", "занятое имя получает номер");

    let (status, answer) = crew
        .send("POST", &format!("/api/bots/{source}/branch"), Some(json!({ "messageId": "nope" })))
        .await;
    assert_eq!(status, 404, "{answer}");
    assert!(answer["error"].as_str().unwrap().contains("не найдено"), "{answer}");
}

#[tokio::test]
async fn a_branch_carries_the_compaction_summary() {
    let crew = Crew::start("branch-anchor").await;
    let source = new_source(&crew).await;
    write_thread(
        &crew,
        &source,
        &[
            json!({ "id": "m1", "role": "user", "text": "старый вопрос" }),
            json!({ "id": "m2", "role": "assistant", "text": "старый ответ" }),
            json!({ "id": "a1", "role": "anchor", "from": 2, "at": 10, "dropped": 2, "summary": "Договорились о плане." }),
            json!({ "id": "m3", "role": "user", "text": "новый вопрос" }),
        ],
    );

    let made = crew
        .ok("POST", &format!("/api/bots/{source}/branch"), Some(json!({ "messageId": "m3" })))
        .await;
    let branch_id = made["bot"]["id"].as_str().unwrap();
    let copied = crew.ok("GET", &format!("/api/bots/{branch_id}/thread"), None).await;
    let copied = copied["messages"].as_array().unwrap();
    assert_eq!(copied.len(), 4, "сводка едет вместе с перепиской до якоря: {copied:?}");
    assert_eq!(copied[0]["text"], "старый вопрос");
    assert_eq!(copied[1]["text"], "старый ответ");
    assert_eq!(copied[2]["role"], "compacted");
    assert_eq!(copied[2]["text"], "Договорились о плане.");
    assert_eq!(copied[3]["text"], "новый вопрос");
}
