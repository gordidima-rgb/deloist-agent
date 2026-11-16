//! `GET /api/usage` и `GET /api/report`: формы ответа, границы недель, старый `usage.json` без моделей.

use crate::common::{self, Crew};
use serde_json::{json, Value};

/// Папка данных с заготовленными файлами учёта, сервер стартует на ней.
async fn seeded(name: &str, files: &[(&str, String)]) -> Crew {
    let dir = common::temp_data_dir(name);
    std::fs::create_dir_all(&dir).unwrap();
    for (file, text) in files {
        std::fs::write(dir.join(file), text).unwrap();
    }
    Crew::start_at(dir, &[]).await
}

fn receipt(id: &str, ts: &str, bot: &str, outcome: &str, tokens: i64, seconds: f64, preview: &str) -> String {
    json!({ "id": id, "ts": ts, "botId": bot, "where": "dm", "model": "m1", "provider": "api", "seconds": seconds,
            "tokens": tokens, "outcome": outcome, "preview": preview, "threadId": format!("bot:{bot}") })
    .to_string()
}

#[tokio::test]
async fn an_empty_month_has_the_full_shape_with_zeros() {
    let crew = Crew::start("usage-empty").await;
    let usage = crew.ok("GET", "/api/usage?month=2026-02", None).await;
    assert_eq!(usage["month"], "2026-02");
    assert_eq!(usage["totalTokens"], 0);
    assert_eq!(usage["replies"], 0);
    assert_eq!(usage["avgSeconds"], 0.0);
    assert_eq!(usage["days"], json!([]));
    assert_eq!(usage["models"], json!([]));
    assert_eq!(usage["receipts"], json!([]));
    assert_eq!(usage["behind"], json!({ "router": 0, "memory": 0, "helper": 0 }));
    // Боты из настроек видны и без расхода; лимит не задан — null.
    let bots = usage["bots"].as_array().unwrap();
    assert!(!bots.is_empty(), "{usage}");
    assert!(bots.iter().all(|b| b["tokens"] == 0 && b["replies"] == 0 && b["monthlyLimit"].is_null()), "{usage}");
    // Текущий месяц по умолчанию и разбор неверного значения.
    let now = crew.ok("GET", "/api/usage", None).await;
    assert_eq!(now["month"].as_str().unwrap().len(), 7);
    let (status, _) = crew.send("GET", "/api/usage?month=junk", None).await;
    assert_eq!(status, 400);
    let report = crew.ok("GET", "/api/report?week=2026-02-10", None).await;
    assert_eq!(report["weekStart"], "2026-02-09");
    assert_eq!(report["weekEnd"], "2026-02-15");
    assert_eq!(report["replies"], 0);
    assert_eq!(report["okPercent"], 0);
    assert_eq!(report["weeks"].as_array().unwrap().len(), 6);
    assert_eq!(report["bots"], json!([]));
    let (status, _) = crew.send("GET", "/api/report?week=junk", None).await;
    assert_eq!(status, 400);
}

#[tokio::test]
async fn old_usage_json_without_models_still_counts_and_receipts_fill_the_screens() {
    // Старый формат: ни usage_models.json, ни usage_behind.json.
    let usage_json = json!({
        "2026-03-02": { "b1": { "prompt": 100, "completion": 20, "cached": 0, "estimated": false } },
        "2026-03-03": { "b1": { "prompt": 10, "completion": 5 }, "b2": { "prompt": 1, "completion": 1 } },
        "2026-04-01": { "b1": { "prompt": 999, "completion": 1 } },
    });
    let lines = [
        receipt("r1", "2026-03-02T10:00:00", "b1", "ok", 120, 2.0, "привет"),
        receipt("r2", "2026-03-03T11:00:00", "b1", "failed", 0, 4.0, "Не смог ответить: 429"),
        receipt("r3", "2026-03-03T12:00:00", "b2", "ok", 2, 6.0, "ответ"),
        // Граница: воскресенье 2026-03-08 ещё в той же неделе, понедельник 03-09 уже в следующей.
        receipt("r4", "2026-03-08T23:59:59", "b2", "ok", 10, 1.0, "вс"),
        receipt("r5", "2026-03-09T00:00:00", "b2", "failed", 0, 1.0, "пн"),
    ]
    .join("\n");
    let behind = json!({ "2026-03-02": { "memory": 50, "helper": 7 }, "2026-03-03": { "memory": 5 } });
    let models = json!({ "2026-03-02": { "m1": { "provider": "api", "tokens": 120 } }, "2026-03-03": { "m1": { "provider": "api", "tokens": 2 } } });
    let crew = seeded(
        "usage-seeded",
        &[
            ("usage.json", usage_json.to_string()),
            ("receipts.jsonl", lines),
            ("usage_behind.json", behind.to_string()),
            ("usage_models.json", models.to_string()),
        ],
    )
    .await;

    let usage = crew.ok("GET", "/api/usage?month=2026-03", None).await;
    assert_eq!(usage["totalTokens"], 137, "{usage}");
    assert_eq!(usage["replies"], 5);
    assert_eq!(usage["avgSeconds"], 2.8);
    assert_eq!(usage["days"].as_array().unwrap().len(), 2);
    assert_eq!(usage["days"][0], json!({ "date": "2026-03-02", "byBot": { "b1": 120 } }));
    assert_eq!(usage["models"], json!([{ "id": "m1", "provider": "api", "tokens": 122 }]));
    assert_eq!(usage["behind"], json!({ "router": 0, "memory": 55, "helper": 7 }));
    let receipts = usage["receipts"].as_array().unwrap();
    assert_eq!(receipts.len(), 5);
    assert_eq!(receipts[0]["id"], "r5", "свежие первыми");
    assert_eq!(receipts[0]["outcome"], "failed");
    for key in ["id", "ts", "botId", "botName", "where", "model", "seconds", "tokens", "outcome", "preview", "threadId"] {
        assert!(receipts[0].get(key).is_some(), "нет поля {key}");
    }
    // Бот, которого нет в настройках, виден по id; расход у него из usage.json.
    let b1 = usage["bots"].as_array().unwrap().iter().find(|b| b["id"] == "b1").cloned().expect("b1");
    assert_eq!((b1["tokens"].clone(), b1["replies"].clone()), (json!(135), json!(2)));
    // Апрель не смешивается с мартом.
    let april = crew.ok("GET", "/api/usage?month=2026-04", None).await;
    assert_eq!(april["totalTokens"], 1000);
    assert_eq!(april["replies"], 0);

    // Неделя 2026-03-02..08: 4 ответа, 1 ошибка; следующая начинается с понедельника.
    let week: Value = crew.ok("GET", "/api/report?week=2026-03-05", None).await;
    assert_eq!((week["weekStart"].clone(), week["weekEnd"].clone()), (json!("2026-03-02"), json!("2026-03-08")));
    assert_eq!((week["replies"].clone(), week["okCount"].clone(), week["failedCount"].clone(), week["okPercent"].clone()), (json!(4), json!(3), json!(1), json!(75)));
    let weeks = week["weeks"].as_array().unwrap();
    assert_eq!(weeks[5], json!({ "start": "2026-03-02", "ok": 3, "failed": 1 }));
    assert_eq!(weeks[4]["start"], "2026-02-23");
    let b1 = week["bots"].as_array().unwrap().iter().find(|b| b["id"] == "b1").cloned().unwrap();
    assert_eq!(b1["replies"], 2);
    assert_eq!(b1["okPercent"], 50);
    assert_eq!(b1["avgTokens"], 60);
    assert_eq!(b1["avgSeconds"], 3.0);
    assert_eq!(b1["failures"], json!([{ "ts": "2026-03-03T11:00:00", "preview": "Не смог ответить: 429" }]));
    let next = crew.ok("GET", "/api/report?week=2026-03-09", None).await;
    assert_eq!((next["replies"].clone(), next["failedCount"].clone(), next["okPercent"].clone()), (json!(1), json!(1), json!(0)));
    assert_eq!(next["weeks"][4]["start"], "2026-03-02");
    assert_eq!(next["weeks"][4]["ok"], 3);
}
