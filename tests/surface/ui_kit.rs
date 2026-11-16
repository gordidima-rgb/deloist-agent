//! Собранные Svelte-компоненты (web/ui) отдаются из exe, как остальной интерфейс, и закрыты сессией.

use crate::common;

use common::Crew;

#[tokio::test]
async fn built_svelte_files_are_served_from_the_exe() {
    let crew = Crew::start("ui-kit").await;
    for (path, kind) in [("/ui/kit.js", "text/javascript; charset=utf-8"), ("/ui/kit.css", "text/css; charset=utf-8")] {
        let res = crew.client.get(format!("{}{path}", crew.base)).header("cookie", &crew.cookie).send().await.unwrap();
        assert_eq!(res.status().as_u16(), 200, "{path}");
        assert_eq!(res.headers()["content-type"].to_str().unwrap(), kind, "{path}");
        assert!(!res.bytes().await.unwrap().is_empty(), "{path} пустой");
    }
    // Служебный файл с хэшем исходников в exe не попадает.
    let res = crew.client.get(format!("{}/ui/.source-hash", crew.base)).header("cookie", &crew.cookie).send().await.unwrap();
    assert_eq!(res.status().as_u16(), 404);
    // Страница-витрина есть только в отладочной сборке: тесты идут в ней.
    let res = crew.client.get(format!("{}/ui-demo.html", crew.base)).header("cookie", &crew.cookie).send().await.unwrap();
    assert_eq!(res.status().as_u16(), if cfg!(debug_assertions) { 200 } else { 404 });
}

#[tokio::test]
async fn svelte_files_need_a_session() {
    let stranger = Crew::spawn("ui-kit-security", &[]).await;
    for path in ["/ui/kit.js", "/ui/kit.css", "/ui-demo.html"] {
        let status = stranger.client.get(format!("{}{path}", stranger.base)).send().await.unwrap().status();
        assert_eq!(status.as_u16(), 403, "{path} открыт без сессии");
    }
}
