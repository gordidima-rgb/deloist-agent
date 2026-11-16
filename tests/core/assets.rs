//! Каждый скрипт и стиль, на который ссылается `web/index.html`, программа правда отдаёт.
//!
//! 12.11.2026 голосовой разговор не работал в собранной программе: `voice-chat.js` был в странице,
//! но сервер отвечал на него 404. Тесты логики этого не видели.

use crate::common::Crew;

#[tokio::test]
async fn every_script_and_style_of_the_page_is_served() {
    let crew = Crew::start("assets").await;
    let html = include_str!("../../web/index.html");
    let mut paths: Vec<String> = Vec::new();
    for attr in ["src=\"", "href=\""] {
        for part in html.split(attr).skip(1) {
            let Some(path) = part.split('"').next() else { continue };
            if path.starts_with('/') && !path.starts_with("//") && (path.ends_with(".js") || path.ends_with(".css")) {
                paths.push(path.to_string());
            }
        }
    }
    paths.sort();
    paths.dedup();
    assert!(paths.iter().any(|p| p == "/app.js"), "разбор index.html ничего не нашёл: {paths:?}");
    let mut missing = Vec::new();
    for path in &paths {
        let res = crew.client.get(format!("{}{path}", crew.base)).header("cookie", &crew.cookie).send().await.unwrap();
        if res.status().as_u16() != 200 {
            missing.push(format!("{path} → {}", res.status()));
        }
    }
    assert!(missing.is_empty(), "страница ссылается на файлы, которых сервер не отдаёт: {missing:?}");
}
