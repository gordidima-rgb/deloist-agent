use crate::common;

use common::{Crew, BOOT};
use serde_json::json;
use std::fs;
use std::io::Write;
use std::net::TcpListener;
use axum::body::{Body, Bytes};
use axum::extract::State;
use axum::response::Response;
use axum::routing::post;
use axum::Router;

#[derive(Clone)]
struct Fake;
async fn completions(State(_fake): State<Fake>, _body: Bytes) -> Response {
    let text = "data: {\"choices\":[{\"index\":0,\"delta\":{\"content\":\"{\\\"facts\\\":[\\\"Любит Rust\\\"],\\\"agents\\\":[{\\\"name\\\":\\\"Скаут\\\",\\\"role\\\":\\\"ищет источники\\\"}]}\"}}]}\n\ndata: [DONE]\n\n";
    Response::builder().header("content-type", "text/event-stream").body(Body::from(text)).unwrap()
}

#[tokio::test]
async fn know_me_read_propose_save() {
    // Prepare source files under a fake USERPROFILE.
    let data_dir = common::temp_data_dir("km");
    let profile = data_dir.join("profile");
    let claude = profile.join(".claude").join("projects").join("p");
    fs::create_dir_all(&claude).unwrap();
    let mut f = fs::File::create(claude.join("s.jsonl")).unwrap();
    writeln!(f, r#"{{"type":"user","message":{{"content":"I love Rust"}}}}"#).unwrap();
    writeln!(f, r#"{{"type":"assistant","message":{{"content":"ok"}}}}"#).unwrap();

    // Fake model server.
    let fake = Fake;
    let app = Router::new().route("/v1/chat/completions", post(completions)).with_state(fake);
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    drop(listener);
    let listener = tokio::net::TcpListener::bind(format!("127.0.0.1:{port}")).await.unwrap();
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });

    let mut crew = Crew::spawn("km", &[("USERPROFILE", profile.to_str().unwrap())]).await;
    // enter session
    let res = crew.client.get(format!("{}/?boot={}", crew.base, BOOT)).send().await.unwrap();
    let cookie = res.headers().get("set-cookie").unwrap().to_str().unwrap().split(';').next().unwrap().to_string();
    crew.cookie = cookie;

    crew.ok("PUT", "/api/settings", Some(json!({ "driver": "api", "baseUrl": format!("http://127.0.0.1:{port}/v1"), "apiKey": "k", "model": "m" }))).await;

    // sources
    let srcs = crew.ok("GET", "/api/know-me/sources", None).await;
    let arr = srcs.as_array().unwrap();
    let claude_src = arr.iter().find(|s| s["id"]=="claude_code").unwrap();
    assert!(claude_src["found"].as_bool().unwrap());
    assert!(claude_src["count"].as_u64().unwrap() >= 1);

    // read
    let read = crew.ok("POST", "/api/know-me/read", Some(json!({ "sources": ["claude_code"] }))).await;
    let topics = read["topics"].as_array().unwrap();
    assert!(!topics.is_empty());
    // cancel baseline memory count
    let mem_before = crew.ok("GET", "/api/memory", None).await;
    let facts_before = mem_before["facts"].as_array().map(|a| a.len()).unwrap_or(0);

    // propose
    let prop = crew.ok("POST", "/api/know-me/propose", Some(json!({ "topics": topics }))).await;
    let facts = prop["facts"].as_array().unwrap().clone();
    let agents = prop["agents"].as_array().unwrap().clone();
    assert!(!facts.is_empty());

    // save
    crew.ok("POST", "/api/know-me/save", Some(json!({ "facts": facts, "agents": agents }))).await;
    let mem_after = crew.ok("GET", "/api/memory", None).await;
    let facts_after = mem_after["facts"].as_array().map(|a| a.len()).unwrap_or(0);
    assert!(facts_after > facts_before, "facts not saved");

    // cancel scenario: no save -> unchanged (already covered by not calling save again)
    let _ = data_dir;
}

#[tokio::test]
async fn know_me_sources_read_crew_home_and_answer_from_cache() {
    // CREW_HOME указывает на подготовленную папку историй; профиль пользователя не нужен.
    let data_dir = common::temp_data_dir("km-home");
    let home = data_dir.join("home");
    let claude = home.join(".claude").join("projects").join("p");
    fs::create_dir_all(&claude).unwrap();
    for i in 0..3 {
        fs::write(claude.join(format!("s{i}.jsonl")), "{\"type\":\"user\",\"message\":{\"content\":\"hi\"}}\n").unwrap();
    }

    let mut crew = Crew::spawn("km-home", &[("CREW_HOME", home.to_str().unwrap())]).await;
    let res = crew.client.get(format!("{}/?boot={}", crew.base, BOOT)).send().await.unwrap();
    let cookie = res.headers().get("set-cookie").unwrap().to_str().unwrap().split(';').next().unwrap().to_string();
    crew.cookie = cookie;

    let claude_entry = |v: &serde_json::Value| v.as_array().unwrap().iter().find(|s| s["id"] == "claude_code").unwrap().clone();

    let first = claude_entry(&crew.ok("GET", "/api/know-me/sources", None).await);
    assert!(first["found"].as_bool().unwrap());
    assert_eq!(first["count"].as_u64().unwrap(), 3);

    let second = claude_entry(&crew.ok("GET", "/api/know-me/sources", None).await);
    assert_eq!(second["count"].as_u64().unwrap(), 3);
    // Отладочная сборка считает пересчитанные файлы: во втором вызове их нет.
    if cfg!(debug_assertions) {
        assert_eq!(first["recounted"].as_u64().unwrap(), 3);
        assert_eq!(second["recounted"].as_u64().unwrap(), 0);
    }
}