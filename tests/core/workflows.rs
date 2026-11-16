//! Рабочие процессы, этап 1: сохранение графа, запуск, согласие, шаг-бот, повтор, перезапуск.

use crate::common;

use std::sync::{Arc, Mutex};

use axum::body::Bytes;
use axum::extract::State;
use axum::response::{IntoResponse, Response};
use axum::routing::post;
use axum::{Json, Router};
use common::{wait_until, Crew};
use serde_json::{json, Value};

fn node(id: &str, kind: &str, fields: Value) -> Value {
    json!({ "id": id, "kind": kind, "x": 0, "y": 0, "fields": fields })
}

fn edge(id: &str, from: &str, to: &str) -> Value {
    json!({ "id": id, "from": from, "to": to })
}

/// Подсказка → согласие → вывод.
fn approval_graph() -> Value {
    json!({
        "nodes": [
            node("p", "prompt", json!({ "text": "Отчёт готов" })),
            node("a", "approval", json!({ "question": "Отправлять?" })),
            node("o", "output", json!({ "target": "notify" })),
        ],
        "edges": [edge("e1", "p", "a"), edge("e2", "a", "o")],
        "strokes": []
    })
}

async fn create(crew: &Crew, title: &str, graph: Value) -> String {
    let made = crew.ok("POST", "/api/workflows", Some(json!({ "title": title }))).await;
    let id = made["workflow"]["id"].as_str().unwrap().to_string();
    assert_eq!(made["workflow"]["revision"], 1);
    let saved = crew.ok("PUT", &format!("/api/workflows/{id}/graph"), Some(json!({ "base_revision": 1, "graph": graph }))).await;
    assert_eq!(saved["revision"], 2);
    id
}

async fn run_status(crew: &Crew, run_id: &str, workflow: &str) -> Value {
    let runs = crew.ok("GET", &format!("/api/workflows/{workflow}/runs"), None).await;
    runs["runs"].as_array().unwrap().iter().find(|r| r["id"] == run_id).cloned().unwrap()
}

async fn wait_run(crew: &Crew, workflow: &str, run_id: &str, status: &str) -> Value {
    let (c, w, r, s) = (crew, workflow.to_string(), run_id.to_string(), status.to_string());
    wait_until(&format!("запуск {status}"), 20, || async { run_status(c, &r, &w).await["status"] == s.as_str() }).await;
    run_status(crew, run_id, workflow).await
}

fn step<'a>(run: &'a Value, node: &str) -> &'a Value {
    run["steps"].as_array().unwrap().iter().filter(|s| s["node_id"] == node).last().unwrap_or_else(|| panic!("нет шага {node}: {run}"))
}

#[tokio::test(flavor = "multi_thread")]
async fn a_workflow_goes_prompt_approval_output_and_the_graph_save_is_guarded() {
    let sink = common::temp_data_dir("wf-sink").with_extension("txt");
    let sink_text = sink.to_string_lossy().to_string();
    let crew = Crew::start_with_env("wf-flow", &[("CREW_TEST_REMINDER_SINK", &sink_text)]).await;
    crew.listen().await;
    let id = create(&crew, "Утренний отчёт", approval_graph()).await;

    // Устаревшая ревизия не затирает чужое сохранение.
    let (status, body) = crew.send("PUT", &format!("/api/workflows/{id}/graph"), Some(json!({ "base_revision": 1, "graph": { "nodes": [], "edges": [] } }))).await;
    assert_eq!(status, 409, "{body}");
    assert_eq!(body["conflict"], true);
    assert_eq!(body["workflow"]["revision"], 2);
    assert_eq!(body["workflow"]["graph"]["nodes"].as_array().unwrap().len(), 3);

    // Неверный граф: 400 с кодом и id ребра.
    let bad = json!({ "nodes": [node("a", "prompt", json!({})), node("b", "bot", json!({}))], "edges": [json!({ "id": "x", "from": "a", "to": "b", "branch": "yes" })] });
    let (status, body) = crew.send("PUT", &format!("/api/workflows/{id}/graph"), Some(json!({ "base_revision": 2, "graph": bad }))).await;
    assert_eq!(status, 400, "{body}");
    assert_eq!(body["code"], "edge_branch_forbidden");
    assert_eq!(body["edge_id"], "x");

    let list = crew.ok("GET", "/api/workflows", None).await;
    assert_eq!(list["workflows"][0]["steps"], 3);
    let read = crew.ok("GET", &format!("/api/workflows/{id}"), None).await;
    assert_eq!(read["triggers"], json!([]));

    crew.clear_events();
    let started = crew.ok("POST", &format!("/api/workflows/{id}/run"), Some(json!({ "request_id": "r1" }))).await;
    let run_id = started["run"]["id"].as_str().unwrap().to_string();
    let again = crew.ok("POST", &format!("/api/workflows/{id}/run"), Some(json!({ "request_id": "r1" }))).await;
    assert_eq!(again["run"]["id"], run_id.as_str(), "тот же request_id — тот же запуск");
    let run = wait_run(&crew, &id, &run_id, "approval").await;
    assert_eq!(step(&run, "p")["output"], "Отчёт готов");
    let waiting = step(&run, "a");
    assert_eq!(waiting["status"], "approval");
    assert_eq!(waiting["input"], "Отчёт готов");
    assert!(run.get("graph").is_none(), "в списке запусков графа нет");
    crew.wait_event("уведомление о согласии", |e| e["type"] == "workflow_approval" && e["question"] == "Отправлять?").await;
    crew.wait_event("событие процесса", |e| e["type"] == "workflows" && e["workflow_id"] == id.as_str() && e["run_id"] == run_id.as_str()).await;

    let step_id = waiting["id"].as_str().unwrap();
    let decided = crew.ok("POST", &format!("/api/workflow-steps/{step_id}/decide"), Some(json!({ "approved": true }))).await;
    assert!(decided["run"]["graph"].is_object());
    let done = wait_run(&crew, &id, &run_id, "done").await;
    assert_eq!(step(&done, "o")["status"], "done");
    assert_eq!(step(&done, "o")["input"], "Отчёт готов", "согласие передаёт текст дальше");
    let (status, _) = crew.send("POST", &format!("/api/workflow-steps/{step_id}/decide"), Some(json!({ "approved": true }))).await;
    assert_eq!(status, 409, "решение принимается один раз");

    let shown = std::fs::read_to_string(&sink).unwrap_or_default();
    assert!(shown.contains("Отчёт готов") && shown.contains("Утренний отчёт"), "уведомление показано: {shown}");
    let read = crew.ok("GET", &format!("/api/workflows/{id}"), None).await;
    assert_eq!(read["workflow"]["last_run"]["status"], "done");

    // Копия и удаление.
    let copy = crew.ok("POST", &format!("/api/workflows/{id}/duplicate"), None).await;
    assert_eq!(copy["workflow"]["title"], "Утренний отчёт (копия)");
    crew.ok("DELETE", &format!("/api/workflows/{id}"), None).await;
    assert_eq!(crew.send("GET", &format!("/api/workflows/{id}"), None).await.0, 404);
    assert_eq!(crew.ok("GET", "/api/workflows", None).await["workflows"].as_array().unwrap().len(), 1);
    let _ = std::fs::remove_file(sink);
}

#[tokio::test(flavor = "multi_thread")]
async fn rejecting_an_approval_stops_the_whole_run() {
    let crew = Crew::start("wf-reject").await;
    let id = create(&crew, "Отказ", approval_graph()).await;
    let run_id = crew.ok("POST", &format!("/api/workflows/{id}/run"), Some(json!({}))).await["run"]["id"].as_str().unwrap().to_string();
    let run = wait_run(&crew, &id, &run_id, "approval").await;
    let step_id = step(&run, "a")["id"].as_str().unwrap().to_string();
    crew.ok("POST", &format!("/api/workflow-steps/{step_id}/decide"), Some(json!({ "approved": false }))).await;
    let stopped = wait_run(&crew, &id, &run_id, "stopped").await;
    assert_eq!(step(&stopped, "a")["status"], "stopped");
    assert!(stopped["steps"].as_array().unwrap().iter().all(|s| s["node_id"] != "o"), "после отказа вывода не было");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_pending_approval_survives_a_restart() {
    let mut crew = Crew::start("wf-restart").await;
    let data = crew.data_dir.clone();
    let id = create(&crew, "После перезапуска", approval_graph()).await;
    let run_id = crew.ok("POST", &format!("/api/workflows/{id}/run"), Some(json!({}))).await["run"]["id"].as_str().unwrap().to_string();
    let run = wait_run(&crew, &id, &run_id, "approval").await;
    let step_id = step(&run, "a")["id"].as_str().unwrap().to_string();
    crew.keep_data();
    drop(crew);

    let sink = data.with_extension("txt");
    let sink_text = sink.to_string_lossy().to_string();
    let crew = Crew::start_at(data, &[("CREW_TEST_REMINDER_SINK", &sink_text)]).await;
    let run = run_status(&crew, &run_id, &id).await;
    assert_eq!(run["status"], "approval", "согласие ждёт дальше: {run}");
    crew.ok("POST", &format!("/api/workflow-steps/{step_id}/decide"), Some(json!({ "approved": true }))).await;
    wait_run(&crew, &id, &run_id, "done").await;
}

// --- шаг-бот -------------------------------------------------------------

#[derive(Clone, Default)]
struct Fake {
    requests: Arc<Mutex<Vec<Value>>>,
}

async fn completions(State(fake): State<Fake>, body: Bytes) -> Response {
    fake.requests.lock().unwrap().push(serde_json::from_slice(&body).unwrap_or_default());
    Json(json!({ "choices": [{ "message": { "role": "assistant", "content": "ответ бота" } }] })).into_response()
}

#[tokio::test(flavor = "multi_thread")]
async fn a_bot_step_runs_a_real_turn_and_a_failed_step_can_be_retried() {
    let fake = Fake::default();
    let router = Router::new().route("/v1/chat/completions", post(completions)).with_state(fake.clone());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });

    let sink = common::temp_data_dir("wf-bot-sink").with_extension("txt");
    let sink_text = sink.to_string_lossy().to_string();
    let crew = Crew::start_with_env("wf-bot", &[("CREW_TEST_REMINDER_SINK", &sink_text)]).await;
    crew.ok("PUT", "/api/settings", Some(json!({ "driver": "api", "baseUrl": format!("http://127.0.0.1:{port}/v1"), "apiKey": "k" }))).await;
    let bot = crew.ok("GET", "/api/state", None).await["bots"][0]["id"].as_str().unwrap().to_string();

    // Бот не выбран: шаг падает с понятной причиной, а следующий шаг пропускается.
    let broken = json!({
        "nodes": [
            node("p", "prompt", json!({ "text": "вводные" })),
            node("b", "bot", json!({ "task": "Сделай сводку" })),
            node("o", "output", json!({ "target": "notify" })),
        ],
        "edges": [edge("e1", "p", "b"), edge("e2", "b", "o")]
    });
    let id = create(&crew, "С ботом", broken).await;
    let run_id = crew.ok("POST", &format!("/api/workflows/{id}/run"), Some(json!({}))).await["run"]["id"].as_str().unwrap().to_string();
    let run = wait_run(&crew, &id, &run_id, "failed").await;
    assert_eq!(step(&run, "b")["status"], "failed");
    assert_eq!(step(&run, "b")["error"], "в шаге не выбран бот");
    assert_eq!(step(&run, "o")["status"], "skipped");

    // Человек выбирает бота и повторяет шаг: новая попытка с текущими настройками, проход идёт дальше.
    let fixed = json!({
        "nodes": [
            node("p", "prompt", json!({ "text": "вводные" })),
            node("b", "bot", json!({ "task": "Сделай сводку", "bot_id": bot })),
            node("o", "output", json!({ "target": "notify" })),
        ],
        "edges": [edge("e1", "p", "b"), edge("e2", "b", "o")]
    });
    crew.ok("PUT", &format!("/api/workflows/{id}/graph"), Some(json!({ "base_revision": 2, "graph": fixed }))).await;
    let failed_id = step(&run, "b")["id"].as_str().unwrap().to_string();
    crew.ok("POST", &format!("/api/workflow-steps/{failed_id}/retry"), None).await;
    let run = wait_run(&crew, &id, &run_id, "done").await;
    let bot_step = step(&run, "b");
    assert_eq!(bot_step["attempt"], 2);
    assert_eq!(bot_step["output"], "ответ бота");
    assert!(bot_step["thread_id"].is_string());
    assert_eq!(step(&run, "o")["status"], "done");
    assert_eq!(step(&run, "o")["input"], "ответ бота");

    // Бот получил задачу и вход процесса.
    let sent = fake.requests.lock().unwrap().iter().map(|r| r.to_string()).collect::<Vec<_>>().join("\n");
    assert!(sent.contains("поручает тебе шаг") && sent.contains("Сделай сводку") && sent.contains("вводные"), "{sent}");
    assert!(std::fs::read_to_string(&sink).unwrap_or_default().contains("ответ бота"));
    let _ = std::fs::remove_file(sink);
}

#[tokio::test(flavor = "multi_thread")]
async fn llm_and_condition_steps_fail_with_a_stage_two_note_and_validation_covers_them() {
    let crew = Crew::start("wf-stage2").await;
    let graph = json!({
        "nodes": [
            node("p", "prompt", json!({ "text": "x" })),
            node("c", "condition", json!({ "rule": "да?" })),
            node("y", "output", json!({})),
        ],
        "edges": [edge("e1", "p", "c"), json!({ "id": "e2", "from": "c", "to": "y", "branch": "yes" })]
    });
    let id = create(&crew, "Условие", graph).await;
    let run_id = crew.ok("POST", &format!("/api/workflows/{id}/run"), Some(json!({}))).await["run"]["id"].as_str().unwrap().to_string();
    let run = wait_run(&crew, &id, &run_id, "failed").await;
    assert_eq!(step(&run, "c")["error"], "будет на этапе 2");
    assert_eq!(step(&run, "y")["status"], "skipped");
    // Пустой процесс не запускается.
    let empty = crew.ok("POST", "/api/workflows", Some(json!({ "title": "Пусто" }))).await["workflow"]["id"].as_str().unwrap().to_string();
    let (status, body) = crew.send("POST", &format!("/api/workflows/{empty}/run"), Some(json!({}))).await;
    assert_eq!(status, 400, "{body}");
}
