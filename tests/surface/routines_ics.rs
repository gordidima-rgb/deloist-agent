//! Рутины файлом календаря: маршрут отдаёт .ics и не открывается без сессии окна.

use crate::common;

use common::Crew;
use serde_json::json;

#[tokio::test]
async fn routines_are_served_as_a_calendar_file() {
    let crew = Crew::start("routines-ics").await;
    let state = crew.ok("GET", "/api/state", None).await;
    let bot = state["bots"][0]["id"].as_str().unwrap().to_string();
    crew.ok(
        "POST",
        "/api/routines",
        Some(json!({
            "botId": bot,
            "title": "Утренняя сводка",
            "prompt": "Собери темы дня",
            "at": "09:00",
            "days": [1, 2, 3, 4, 5],
        })),
    )
    .await;

    let anonymous = crew.client.get(format!("{}/api/routines.ics", crew.base)).send().await.unwrap();
    assert_eq!(anonymous.status().as_u16(), 403, "файл открыт без сессии окна");

    let res = crew
        .client
        .get(format!("{}/api/routines.ics", crew.base))
        .header("cookie", &crew.cookie)
        .send()
        .await
        .unwrap();
    assert_eq!(res.status().as_u16(), 200);
    let kind = res.headers()["content-type"].to_str().unwrap().to_string();
    assert!(kind.starts_with("text/calendar"), "тип ответа: {kind}");
    assert_eq!(
        res.headers()["content-disposition"].to_str().unwrap(),
        "attachment; filename=\"deloist-routines.ics\""
    );
    let text = res.text().await.unwrap();
    assert!(text.contains("BEGIN:VCALENDAR"), "{text}");
    assert!(text.contains("RRULE:FREQ=WEEKLY;BYDAY=MO,TU,WE,TH,FR"), "{text}");
}
