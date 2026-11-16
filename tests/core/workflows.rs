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
async fn a_condition_without_a_usable_model_fails_the_step() {
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
    assert!(step(&run, "c")["error"].as_str().unwrap().contains("нужна модель"), "{run}");
    assert_eq!(step(&run, "y")["status"], "skipped");
    // Пустой процесс не запускается.
    let empty = crew.ok("POST", "/api/workflows", Some(json!({ "title": "Пусто" }))).await["workflow"]["id"].as_str().unwrap().to_string();
    let (status, body) = crew.send("POST", &format!("/api/workflows/{empty}/run"), Some(json!({}))).await;
    assert_eq!(status, 400, "{body}");
}

// --- этап 2: модель, версии, одиночный шаг -------------------------------

#[derive(Clone, Default)]
struct Model {
    requests: Arc<Mutex<Vec<Value>>>,
    answer: Arc<Mutex<String>>,
}

async fn model_reply(State(fake): State<Model>, body: Bytes) -> Response {
    fake.requests.lock().unwrap().push(serde_json::from_slice(&body).unwrap_or_default());
    let text = fake.answer.lock().unwrap().clone();
    Json(json!({ "choices": [{ "message": { "role": "assistant", "content": text } }] })).into_response()
}

async fn crew_with_model(name: &str, answer: &str) -> (Crew, Model) {
    let fake = Model::default();
    *fake.answer.lock().unwrap() = answer.to_string();
    let router = Router::new().route("/v1/chat/completions", post(model_reply)).with_state(fake.clone());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
    let sink = common::temp_data_dir("wf-model-sink").with_extension("txt");
    let sink_text = sink.to_string_lossy().to_string();
    let crew = Crew::start_with_env(name, &[("CREW_TEST_REMINDER_SINK", &sink_text)]).await;
    crew.ok("PUT", "/api/settings", Some(json!({ "driver": "api", "baseUrl": format!("http://127.0.0.1:{port}/v1"), "apiKey": "k" }))).await;
    (crew, fake)
}

async fn run_wf(crew: &Crew, id: &str) -> String {
    crew.ok("POST", &format!("/api/workflows/{id}/run"), Some(json!({}))).await["run"]["id"].as_str().unwrap().to_string()
}

fn branching() -> Value {
    json!({
        "nodes": [
            node("p", "prompt", json!({ "text": "Заказ на 5000" })),
            node("c", "condition", json!({ "rule": "Сумма больше 1000?" })),
            node("y", "output", json!({ "target": "notify" })),
            node("n", "output", json!({ "target": "notify" })),
        ],
        "edges": [
            edge("e1", "p", "c"),
            { "id": "e2", "from": "c", "to": "y", "branch": "yes" },
            { "id": "e3", "from": "c", "to": "n", "branch": "no" }
        ]
    })
}

#[tokio::test(flavor = "multi_thread")]
async fn a_condition_asks_the_model_and_routes_the_branch() {
    let (crew, fake) = crew_with_model("wf-cond", " Да. ").await;
    let id = create(&crew, "Условие", branching()).await;
    let run_id = run_wf(&crew, &id).await;
    let run = wait_run(&crew, &id, &run_id, "done").await;
    assert_eq!(step(&run, "c")["branch"], "yes");
    assert_eq!(step(&run, "c")["output"], "Заказ на 5000", "условие отдаёт вход как есть");
    assert_eq!(step(&run, "y")["status"], "done");
    assert_eq!(step(&run, "n")["status"], "skipped");
    let sent = fake.requests.lock().unwrap()[0].to_string();
    assert!(sent.contains("Сумма больше 1000?") && sent.contains("Заказ на 5000"), "{sent}");

    *fake.answer.lock().unwrap() = "НЕТ!".into();
    let run = wait_run(&crew, &id, &run_wf(&crew, &id).await, "done").await;
    assert_eq!(step(&run, "n")["status"], "done");
    assert_eq!(step(&run, "y")["status"], "skipped");

    *fake.answer.lock().unwrap() = "Возможно, зависит".into();
    let run = wait_run(&crew, &id, &run_wf(&crew, &id).await, "failed").await;
    assert_eq!(step(&run, "c")["error"], "модель не ответила да/нет");
}

#[tokio::test(flavor = "multi_thread")]
async fn an_llm_step_makes_one_toolless_call_with_its_own_model() {
    let (crew, fake) = crew_with_model("wf-llm", "Краткое резюме").await;
    let graph = json!({
        "nodes": [
            node("p", "prompt", json!({ "text": "длинный текст" })),
            node("l", "llm", json!({ "prompt": "Сделай резюме", "system": "Ты редактор", "model": "model-x" })),
        ],
        "edges": [edge("e1", "p", "l")]
    });
    let id = create(&crew, "Модель", graph).await;
    let run = wait_run(&crew, &id, &run_wf(&crew, &id).await, "done").await;
    assert_eq!(step(&run, "l")["output"], "Краткое резюме");
    let sent = fake.requests.lock().unwrap()[0].clone();
    assert_eq!(sent["model"], "model-x");
    let text = sent.to_string();
    assert!(text.contains("Ты редактор") && text.contains("Сделай резюме") && text.contains("--- вход ---") && text.contains("длинный текст"), "{text}");
    assert!(sent.get("tools").is_none_or(|t| t.as_array().is_none_or(Vec::is_empty)), "без инструментов");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_solo_step_reuses_unchanged_upstream_answers_and_versions_are_kept() {
    let (crew, fake) = crew_with_model("wf-solo", "ответ модели").await;
    let graph = json!({
        "nodes": [
            node("p", "prompt", json!({ "text": "вводные" })),
            node("l", "llm", json!({ "prompt": "Обработай" })),
            node("o", "output", json!({ "target": "notify" })),
        ],
        "edges": [edge("e1", "p", "l"), edge("e2", "l", "o")]
    });
    let id = create(&crew, "Одиночный", graph.clone()).await;
    let first = run_wf(&crew, &id).await;
    wait_run(&crew, &id, &first, "done").await;
    assert_eq!(fake.requests.lock().unwrap().len(), 1);

    // Одиночный запуск вывода: ответ модели берётся из прошлого запуска, модель не зовётся.
    let solo = crew.ok("POST", &format!("/api/workflows/{id}/nodes/o/run"), Some(json!({ "request_id": "s1" }))).await["run"]["id"].as_str().unwrap().to_string();
    let run = wait_run(&crew, &id, &solo, "done").await;
    assert_eq!(step(&run, "o")["input"], "ответ модели");
    assert_eq!(step(&run, "l")["reused"], true);
    assert_eq!(fake.requests.lock().unwrap().len(), 1, "модель повторно не звали");
    let again = crew.ok("POST", &format!("/api/workflows/{id}/nodes/o/run"), Some(json!({ "request_id": "s1" }))).await;
    assert_eq!(again["run"]["id"], solo.as_str());
    // «Запустить отсюда» тоже берёт неизменный верх.
    let from_l = crew.ok("POST", &format!("/api/workflows/{id}/run"), Some(json!({ "from_node": "l" }))).await["run"]["id"].as_str().unwrap().to_string();
    let run = wait_run(&crew, &id, &from_l, "done").await;
    assert_eq!(step(&run, "p")["reused"], true);
    assert_eq!(step(&run, "o")["status"], "done");

    // Поменяли настройки узла модели: прошлый ответ устарел, вход пуст.
    let mut changed = graph.clone();
    changed["nodes"][1]["fields"]["prompt"] = json!("Другое задание");
    crew.ok("PUT", &format!("/api/workflows/{id}/graph"), Some(json!({ "base_revision": 2, "graph": changed }))).await;
    let solo2 = crew.ok("POST", &format!("/api/workflows/{id}/nodes/o/run"), Some(json!({}))).await["run"]["id"].as_str().unwrap().to_string();
    let run = wait_run(&crew, &id, &solo2, "done").await;
    assert_eq!(step(&run, "o")["input"], "", "устаревший ответ не берём");
    let from = crew.ok("POST", &format!("/api/workflows/{id}/run"), Some(json!({ "from_node": "l" }))).await["run"]["id"].as_str().unwrap().to_string();
    let run = wait_run(&crew, &id, &from, "done").await;
    assert_eq!(step(&run, "p")["reused"], true, "неизменный верх взят из прошлого запуска");
    assert_eq!(step(&run, "l")["status"], "done");

    // Версии: сохранение (раз в пять минут), запуски; восстановление создаёт новую ревизию.
    let list = crew.ok("GET", &format!("/api/workflows/{id}/versions"), None).await["versions"].as_array().unwrap().clone();
    let kinds: Vec<&str> = list.iter().map(|v| v["kind"].as_str().unwrap()).collect();
    assert!(kinds.contains(&"auto") && kinds.contains(&"run"), "{kinds:?}");
    assert_eq!(list.iter().filter(|v| v["kind"] == "auto").count(), 1, "частые сохранения не плодят версии");
    assert_eq!(list[0]["steps"], 3);
    let oldest = list.last().unwrap()["v"].as_u64().unwrap();
    let old = crew.ok("GET", &format!("/api/workflows/{id}/versions/{oldest}"), None).await;
    assert_eq!(old["graph"]["nodes"][1]["fields"]["prompt"], "Обработай");
    let restored = crew.ok("POST", &format!("/api/workflows/{id}/versions/{oldest}/restore"), None).await;
    assert_eq!(restored["workflow"]["graph"]["nodes"][1]["fields"]["prompt"], "Обработай");
    assert_eq!(restored["workflow"]["revision"], 4);
    let after = crew.ok("GET", &format!("/api/workflows/{id}/versions"), None).await["versions"].as_array().unwrap().clone();
    assert_eq!(after[0]["kind"], "restore");
    assert_eq!(crew.send("GET", &format!("/api/workflows/{id}/versions/9999"), None).await.0, 404);
}

#[tokio::test(flavor = "multi_thread")]
async fn strokes_and_annotation_fields_are_validated() {
    let crew = Crew::start("wf-strokes").await;
    let id = create(&crew, "Холст", json!({ "nodes": [], "edges": [] })).await;
    let stroke = |name: &str, points: usize| json!({ "id": name, "brush": "pen", "color": "#ff0000", "width": 3, "points": vec![json!([1, 2]); points] });
    let ok = json!({ "nodes": [json!({ "id": "f", "kind": "shape", "x": 0, "y": 0, "fields": { "shape": "star", "fill": "#fff" } })], "edges": [], "strokes": [stroke("s1", 2000)] });
    let saved = crew.ok("PUT", &format!("/api/workflows/{id}/graph"), Some(json!({ "base_revision": 2, "graph": ok }))).await;
    assert_eq!(saved["revision"], 3);
    let read = crew.ok("GET", &format!("/api/workflows/{id}"), None).await;
    assert_eq!(read["workflow"]["graph"]["strokes"][0]["points"].as_array().unwrap().len(), 2000);
    let many: Vec<Value> = (0..501).map(|i| stroke(&format!("s{i}"), 1)).collect();
    for (graph, code) in [
        (json!({ "nodes": [], "edges": [], "strokes": [stroke("a", 2001)] }), "too_many_points"),
        (json!({ "nodes": [], "edges": [], "strokes": many }), "too_many_strokes"),
        (json!({ "nodes": [], "edges": [], "strokes": [{ "id": "s", "brush": "brush", "color": "#000", "width": 1, "points": [] }] }), "bad_brush"),
        (json!({ "nodes": [json!({ "id": "f", "kind": "shape", "x": 0, "y": 0, "fields": { "shape": "blob" } })], "edges": [] }), "bad_shape"),
    ] {
        let (status, body) = crew.send("PUT", &format!("/api/workflows/{id}/graph"), Some(json!({ "base_revision": 3, "graph": graph }))).await;
        assert_eq!((status, body["code"].as_str().unwrap_or("")), (400, code), "{body}");
    }
}
