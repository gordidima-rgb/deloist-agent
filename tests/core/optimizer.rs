use crate::common::Crew;
use serde_json::{json, Value};

/// Журнал бота: одна просьба и столько же вызовов одного инструмента. В аргументах и результате лежат метки,
/// которых нигде не должно оказаться.
fn write_thread(crew: &Crew, bot: &str, asks: &[(&str, usize)]) {
    let mut lines = Vec::new();
    for (k, (ask, calls)) in asks.iter().enumerate() {
        let at = now_ms() - 1_000 * (asks.len() - k) as i64;
        lines.push(json!({ "id": format!("u{k}"), "role": "user", "at": at, "text": ask }));
        let tool_calls: Vec<Value> = (0..*calls)
            .map(|i| json!({ "id": format!("c{k}-{i}"), "name": "browser_page", "args": { "action": "click", "selector": "ARG-MARKER" }, "status": "ok", "result": "RESULT-MARKER" }))
            .collect();
        lines.push(json!({ "id": format!("a{k}"), "role": "assistant", "text": "готово", "toolCalls": tool_calls, "at": at + 1 }));
    }
    let text: String = lines.iter().map(|l| format!("{l}\n")).collect();
    std::fs::create_dir_all(crew.data_dir.join("threads")).unwrap();
    std::fs::write(crew.data_dir.join("threads").join(format!("{bot}.jsonl")), text).unwrap();
}

#[tokio::test]
async fn optimizer_offers_a_script_once_and_remembers_the_answer() {
    let crew = Crew::start("optimizer-flow").await;
    let state = crew.ok("GET", "/api/state", None).await;
    let bot = state["bots"][0]["id"].as_str().unwrap().to_string();
    write_thread(&crew, &bot, &[("Собери выдачу Telegram, пароль: Qwerty123 для входа", 45)]);

    let scanned = crew.ok("POST", "/api/optimizer/scan", None).await;
    assert_eq!(scanned["added"], 1);
    let offers = crew.ok("GET", &format!("/api/bots/{bot}/optimizer"), None).await;
    let offer = offers["offers"][0].clone();
    assert_eq!(offer["sig"], "browser");
    assert_eq!(offer["steps"], 45);
    assert!(!offer["example"].as_str().unwrap().contains("Qwerty123"), "секрет в карточке: {offer}");

    // Пока человек не ответил, второй просмотр про то же не спрашивает.
    assert_eq!(crew.ok("POST", "/api/optimizer/scan", None).await["added"], 0);

    let id = offer["id"].as_str().unwrap();
    let decision = crew.ok("POST", &format!("/api/optimizer/proposals/{id}/accept"), None).await;
    assert_eq!(decision["verdict"], "script");
    let (gone, _) = crew.send("POST", &format!("/api/optimizer/proposals/{id}/accept"), None).await;
    assert_eq!(gone, 404, "ответ засчитан дважды");

    // Ответ запомнился: ни в файле, ни в общей памяти нет аргументов, результатов и секрета.
    // В общую память скрипт попадёт позже, после проверки на живом примере: до неё остальным ботам знать нечего.
    assert_eq!(crew.ok("POST", "/api/optimizer/scan", None).await["added"], 0);
    let memory = crew.ok("GET", "/api/memory", None).await;
    assert!(memory["facts"].as_array().unwrap().is_empty(), "память заполнена до проверки скрипта: {memory}");
    let saved = std::fs::read_to_string(crew.data_dir.join("optimizer.json")).unwrap();
    for marker in ["ARG-MARKER", "RESULT-MARKER", "Qwerty123"] {
        assert!(!saved.contains(marker), "{marker} попал на диск: {saved}");
    }

    // Забыть ответ можно: служба снова может спросить.
    let key = decision["key"].as_str().unwrap();
    let (status, _) = crew.send("DELETE", &format!("/api/optimizer/decisions/{key}"), None).await;
    assert_eq!(status, 204);
    assert_eq!(crew.ok("POST", "/api/optimizer/scan", None).await["added"], 1);
}

#[tokio::test]
async fn one_off_answer_is_kept_without_a_memory_fact_and_switch_stops_the_scan() {
    let crew = Crew::start("optimizer-once").await;
    let state = crew.ok("GET", "/api/state", None).await;
    let bot = state["bots"][0]["id"].as_str().unwrap().to_string();
    write_thread(&crew, &bot, &[("первый день", 16), ("второй день", 17)]);

    crew.ok("POST", "/api/optimizer/enabled", Some(json!({ "enabled": false }))).await;
    assert_eq!(crew.ok("POST", "/api/optimizer/scan", None).await["added"], 0, "выключенная служба смотрит журналы");
    crew.ok("POST", "/api/optimizer/enabled", Some(json!({ "enabled": true }))).await;

    let scanned = crew.ok("POST", "/api/optimizer/scan", None).await;
    assert_eq!(scanned["added"], 1, "повтор в двух ходах");
    let offer = crew.ok("GET", "/api/optimizer", None).await["pending"][0].clone();
    assert_eq!(offer["kind"], "repeat");
    let id = offer["id"].as_str().unwrap();
    let decision = crew.ok("POST", &format!("/api/optimizer/proposals/{id}/decline"), None).await;
    assert_eq!(decision["verdict"], "once");
    let memory = crew.ok("GET", "/api/memory", None).await;
    assert!(memory["facts"].as_array().unwrap().is_empty(), "разовая работа раздувает память: {memory}");
    assert_eq!(crew.ok("POST", "/api/optimizer/scan", None).await["added"], 0);
}

#[tokio::test]
async fn short_or_mixed_work_gets_no_offer() {
    let crew = Crew::start("optimizer-quiet").await;
    let state = crew.ok("GET", "/api/state", None).await;
    let bot = state["bots"][0]["id"].as_str().unwrap().to_string();
    write_thread(&crew, &bot, &[("мало", 10), ("тоже мало", 12)]);
    assert_eq!(crew.ok("POST", "/api/optimizer/scan", None).await["added"], 0);
}

fn now_ms() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_millis() as i64
}
