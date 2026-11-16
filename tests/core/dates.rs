//! Разбор дат по-русски: маршрут GET /api/dates/parse. Код, без модели.

use crate::common;

use common::Crew;
use serde_json::json;

/// Проценты для query-строки: кириллица и пробелы.
fn enc(s: &str) -> String {
    s.bytes()
        .map(|b| {
            if b.is_ascii_alphanumeric() || b"-_.~".contains(&b) {
                (b as char).to_string()
            } else {
                format!("%{b:02X}")
            }
        })
        .collect()
}

#[tokio::test]
async fn parses_a_repeat_phrase_without_a_model() {
    let crew = Crew::start("dates-repeat").await;
    let (status, body) = crew
        .send("GET", &format!("/api/dates/parse?text={}", enc("каждую среду вынести мусор")), None)
        .await;
    assert_eq!(status, 200);
    assert_eq!(body["title"], "Вынести мусор");
    assert_eq!(body["recurrence"], json!({"freq": "weekly", "interval": 1, "weekdays": [3], "monthday": null}));
    assert_eq!(body["label"], "Каждую неделю по средам");
    assert_eq!(body["complete"], true);
}

#[tokio::test]
async fn parses_relative_day_and_time() {
    let crew = Crew::start("dates-relative").await;
    let tomorrow = (chrono::Local::now().date_naive() + chrono::Days::new(1)).format("%Y-%m-%d").to_string();
    let (status, body) = crew
        .send("GET", &format!("/api/dates/parse?text={}", enc("позвонить маме завтра в 18:00")), None)
        .await;
    assert_eq!(status, 200);
    assert_eq!(body["due_date"], tomorrow);
    assert_eq!(body["due_time"], "18:00");
    assert_eq!(body["title"], "Позвонить маме");
}

#[tokio::test]
async fn empty_text_gives_empty_result() {
    let crew = Crew::start("dates-empty").await;
    let (status, body) = crew.send("GET", "/api/dates/parse", None).await;
    assert_eq!(status, 200);
    assert_eq!(body["title"], "");
    assert_eq!(body["complete"], false);
}
