//! Адреса рабочих процессов (этап 1). Все они закрыты для ключа программы (`guard::human_only`):
//! бот не создаёт, не меняет, не запускает процессы и не решает согласия.

use std::sync::Arc;

use axum::body::Bytes;
use axum::extract::{Path, Query, State};
use axum::http::{header, HeaderMap, HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post, put};
use axum::{Json, Router};
use serde::Deserialize;
use serde_json::{json, Value};

use crate::store::now;
use crate::workflow_run::{self as run, RunError};
use crate::workflows::{self as wf, Graph, Stored};
use crate::App;

type Shared = State<Arc<App>>;

pub fn routes() -> Router<Arc<App>> {
    Router::new()
        .route("/api/workflows", get(list).post(create))
        .route("/api/workflows/{id}", get(read).patch(rename).delete(remove))
        .route("/api/workflows/{id}/graph", put(save_graph))
        .route("/api/workflows/{id}/duplicate", post(duplicate))
        .route("/api/workflows/{id}/run", post(start))
        .route("/api/workflows/{id}/runs", get(runs))
        .route("/api/workflows/{id}/nodes/{node_id}/webhook", post(rotate_hook))
        .route("/hooks/workflows/{id}/{node_id}/{secret}", post(incoming_hook))
        .route("/api/workflows/{id}/nodes/{node_id}/run", post(start_solo))
        .route("/api/workflows/{id}/versions", get(versions))
        .route("/api/workflows/{id}/versions/{v}", get(version))
        .route("/api/workflows/{id}/versions/{v}/restore", post(restore))
        .route("/api/workflow-runs/{run_id}/stop", post(stop_run))
        .route("/api/workflow-steps/{step_id}/stop", post(stop_step))
        .route("/api/workflow-steps/{step_id}/retry", post(retry_step))
        .route("/api/workflow-steps/{step_id}/decide", post(decide))
}

fn reply(status: StatusCode, body: Value) -> Response {
    (status, [(header::CACHE_CONTROL, HeaderValue::from_static("no-store"))], Json(body)).into_response()
}

fn fail(status: StatusCode, message: &str) -> Response {
    reply(status, json!({ "error": message }))
}

fn not_found() -> Response {
    fail(StatusCode::NOT_FOUND, "процесса нет")
}

fn run_fail(e: RunError) -> Response {
    let status = StatusCode::from_u16(e.status).unwrap_or(StatusCode::BAD_REQUEST);
    let mut body = json!({ "error": e.message });
    if let Some(code) = e.code {
        body["code"] = json!(code);
    }
    reply(status, body)
}

fn body_of(bytes: &Bytes) -> Result<Value, Response> {
    if bytes.is_empty() {
        return Ok(json!({}));
    }
    serde_json::from_slice(bytes).map_err(|_| fail(StatusCode::BAD_REQUEST, "тело не является JSON"))
}

fn graph_of(value: &Value) -> Result<Graph, Response> {
    let graph: Graph = serde_json::from_value(value.clone()).map_err(|e| reply(StatusCode::BAD_REQUEST, json!({ "error": format!("граф не разобран: {e}"), "code": "bad_graph" })))?;
    wf::validate_graph(&graph).map_err(|e| {
        let mut body = json!({ "error": e.message, "code": e.code });
        if let Some(id) = e.node_id {
            body["node_id"] = json!(id);
        }
        if let Some(id) = e.edge_id {
            body["edge_id"] = json!(id);
        }
        reply(StatusCode::BAD_REQUEST, body)
    })?;
    Ok(graph)
}

fn changed(app: &Arc<App>, id: &str) {
    app.turns.emit(json!({ "type": "workflows", "workflow_id": id }));
}

fn title_of(body: &Value) -> Option<String> {
    let title = body.get("title").and_then(Value::as_str)?.trim();
    (!title.is_empty()).then(|| title.chars().take(200).collect())
}

async fn list(State(app): Shared) -> Response {
    let guard = app.workflows.lock();
    let mut items: Vec<&Stored> = guard.items.iter().collect();
    items.sort_by_key(|s| std::cmp::Reverse(s.wf.updated_at));
    reply(StatusCode::OK, json!({ "workflows": items.iter().map(|s| wf::summary(&s.wf)).collect::<Vec<_>>() }))
}

async fn create(State(app): Shared, bytes: Bytes) -> Response {
    let body = match body_of(&bytes) {
        Ok(b) => b,
        Err(r) => return r,
    };
    let Some(title) = title_of(&body) else { return fail(StatusCode::BAD_REQUEST, "нужно название") };
    let graph = match body.get("graph").filter(|g| !g.is_null()) {
        Some(value) => match graph_of(value) {
            Ok(g) => g,
            Err(r) => return r,
        },
        None => Graph::default(),
    };
    let mut stored = wf::new_workflow(&title, graph);
    wf::refresh_slots(&mut stored, now());
    let (id, view) = (stored.wf.id.clone(), stored.wf.clone());
    {
        let mut guard = app.workflows.lock();
        guard.items.push(stored);
        guard.runs.insert(id.clone(), Vec::new());
        guard.save_wf(&id);
    }
    changed(&app, &id);
    reply(StatusCode::OK, json!({ "workflow": view }))
}

async fn read(State(app): Shared, Path(id): Path<String>) -> Response {
    let mut guard = app.workflows.lock();
    let Some(stored) = guard.get_mut(&id) else { return not_found() };
    // Срок нового расписания виден сразу, а не после ближайшего тика.
    let touched = wf::refresh_slots(stored, now());
    let triggers: Vec<Value> = stored
        .wf
        .graph
        .nodes
        .iter()
        .filter(|n| n.kind == "trigger")
        .map(|n| json!({ "node_id": n.id, "type": if n.text("trigger").is_empty() { "manual".to_string() } else { n.text("trigger") }, "next_run": stored.slots.get(&n.id).and_then(|s| s.next) }))
        .collect();
    let view = stored.wf.clone();
    if touched {
        guard.save_wf(&id);
    }
    reply(StatusCode::OK, json!({ "workflow": view, "triggers": triggers }))
}

async fn save_graph(State(app): Shared, Path(id): Path<String>, bytes: Bytes) -> Response {
    let body = match body_of(&bytes) {
        Ok(b) => b,
        Err(r) => return r,
    };
    let Some(base) = body.get("base_revision").and_then(Value::as_u64) else { return fail(StatusCode::BAD_REQUEST, "нужна base_revision") };
    let graph = match graph_of(body.get("graph").unwrap_or(&Value::Null)) {
        Ok(g) => g,
        Err(r) => return r,
    };
    let revision = {
        let mut guard = app.workflows.lock();
        let Some(stored) = guard.get_mut(&id) else { return not_found() };
        if stored.wf.revision != base {
            return reply(StatusCode::CONFLICT, json!({ "conflict": true, "workflow": stored.wf }));
        }
        stored.wf.graph = graph.clone();
        stored.wf.revision += 1;
        stored.wf.updated_at = now();
        wf::refresh_slots(stored, now());
        let revision = stored.wf.revision;
        guard.save_wf(&id);
        guard.push_version(&id, "auto", &graph, revision);
        revision
    };
    changed(&app, &id);
    reply(StatusCode::OK, json!({ "revision": revision }))
}

async fn rename(State(app): Shared, Path(id): Path<String>, bytes: Bytes) -> Response {
    let body = match body_of(&bytes) {
        Ok(b) => b,
        Err(r) => return r,
    };
    let title = title_of(&body);
    let enabled = body.get("enabled").and_then(Value::as_bool);
    if title.is_none() && enabled.is_none() {
        return fail(StatusCode::BAD_REQUEST, "нужно название или enabled");
    }
    let view = {
        let mut guard = app.workflows.lock();
        let Some(stored) = guard.get_mut(&id) else { return not_found() };
        if let Some(title) = title {
            stored.wf.title = title;
        }
        if let Some(enabled) = enabled {
            // После паузы сроки считаются заново: пропущенное за время выключения не запускается разом.
            if enabled && !stored.wf.enabled {
                stored.slots.clear();
                wf::refresh_slots(stored, now());
            }
            stored.wf.enabled = enabled;
        }
        stored.wf.updated_at = now();
        let view = stored.wf.clone();
        guard.save_wf(&id);
        view
    };
    changed(&app, &id);
    reply(StatusCode::OK, json!({ "workflow": view }))
}

async fn remove(State(app): Shared, Path(id): Path<String>) -> Response {
    if run::delete_workflow(&app, &id) {
        reply(StatusCode::OK, json!({ "ok": true }))
    } else {
        not_found()
    }
}

async fn duplicate(State(app): Shared, Path(id): Path<String>) -> Response {
    let (copy_id, view) = {
        let mut guard = app.workflows.lock();
        let Some(source) = guard.get(&id) else { return not_found() };
        let mut copy = wf::new_workflow(&format!("{} (копия)", source.wf.title), source.wf.graph.clone());
        wf::refresh_slots(&mut copy, now());
        let (copy_id, view) = (copy.wf.id.clone(), copy.wf.clone());
        guard.items.push(copy);
        guard.runs.insert(copy_id.clone(), Vec::new());
        guard.save_wf(&copy_id);
        (copy_id, view)
    };
    changed(&app, &copy_id);
    reply(StatusCode::OK, json!({ "workflow": view }))
}

async fn start(State(app): Shared, Path(id): Path<String>, bytes: Bytes) -> Response {
    let body = match body_of(&bytes) {
        Ok(b) => b,
        Err(r) => return r,
    };
    let request_id = body.get("request_id").and_then(Value::as_str).filter(|s| !s.is_empty() && s.len() <= 100).map(str::to_string);
    let from_node = body.get("from_node").and_then(Value::as_str).filter(|s| !s.is_empty()).map(str::to_string);
    match run::start_run(&app, &id, "manual", request_id, from_node, None, false) {
        Ok(run) => reply(StatusCode::OK, json!({ "run": run })),
        Err(e) => run_fail(e),
    }
}

#[derive(Deserialize)]
struct RunsQuery {
    before: Option<i64>,
    limit: Option<usize>,
}

async fn runs(State(app): Shared, Path(id): Path<String>, Query(q): Query<RunsQuery>) -> Response {
    let guard = app.workflows.lock();
    if guard.get(&id).is_none() {
        return not_found();
    }
    let limit = q.limit.unwrap_or(20).clamp(1, wf::KEEP_RUNS);
    let mut all: Vec<&wf::Run> = guard.runs.get(&id).map(|l| l.iter().collect()).unwrap_or_default();
    all.sort_by_key(|r| std::cmp::Reverse(r.started_at));
    let rest: Vec<&wf::Run> = all.into_iter().filter(|r| q.before.is_none_or(|b| r.started_at < b)).collect();
    let more = rest.len() > limit;
    let page: Vec<Value> = rest
        .iter()
        .take(limit)
        .map(|r| {
            let mut value = serde_json::to_value(r).unwrap_or_default();
            if let Some(map) = value.as_object_mut() {
                map.remove("graph");
            }
            value
        })
        .collect();
    let next_before = if more { rest.get(limit - 1).map(|r| r.started_at) } else { None };
    reply(StatusCode::OK, json!({ "runs": page, "next_before": next_before }))
}

fn run_reply(result: Result<wf::Run, RunError>) -> Response {
    match result {
        Ok(run) => reply(StatusCode::OK, json!({ "run": run })),
        Err(e) => run_fail(e),
    }
}

async fn stop_run(State(app): Shared, Path(run_id): Path<String>) -> Response {
    run_reply(run::stop_run(&app, &run_id))
}

async fn stop_step(State(app): Shared, Path(step_id): Path<String>) -> Response {
    run_reply(run::stop_step(&app, &step_id))
}

async fn retry_step(State(app): Shared, Path(step_id): Path<String>) -> Response {
    run_reply(run::retry_step(&app, &step_id))
}

async fn decide(State(app): Shared, Path(step_id): Path<String>, bytes: Bytes) -> Response {
    let body = match body_of(&bytes) {
        Ok(b) => b,
        Err(r) => return r,
    };
    let Some(approved) = body.get("approved").and_then(Value::as_bool) else { return fail(StatusCode::BAD_REQUEST, "нужно approved: true или false") };
    run_reply(run::decide(&app, &step_id, approved))
}

async fn start_solo(State(app): Shared, Path((id, node_id)): Path<(String, String)>, bytes: Bytes) -> Response {
    let body = match body_of(&bytes) {
        Ok(b) => b,
        Err(r) => return r,
    };
    let request_id = body.get("request_id").and_then(Value::as_str).filter(|s| !s.is_empty() && s.len() <= 100).map(str::to_string);
    run_reply(run::start_run(&app, &id, "manual", request_id, Some(node_id), None, true))
}

async fn versions(State(app): Shared, Path(id): Path<String>) -> Response {
    let guard = app.workflows.lock();
    if guard.get(&id).is_none() {
        return not_found();
    }
    let list: Vec<Value> = guard
        .versions
        .get(&id)
        .map(|l| {
            l.iter()
                .rev()
                .map(|v| json!({ "v": v.v, "kind": v.kind, "at": v.at, "revision": v.revision, "steps": v.graph.nodes.iter().filter(|n| !wf::is_annotation(&n.kind)).count() }))
                .collect()
        })
        .unwrap_or_default();
    reply(StatusCode::OK, json!({ "versions": list }))
}

async fn version(State(app): Shared, Path((id, v)): Path<(String, u64)>) -> Response {
    let guard = app.workflows.lock();
    match guard.versions.get(&id).and_then(|l| l.iter().find(|x| x.v == v)) {
        Some(found) => reply(StatusCode::OK, json!({ "v": found.v, "kind": found.kind, "at": found.at, "revision": found.revision, "graph": found.graph })),
        None => fail(StatusCode::NOT_FOUND, "такой версии нет"),
    }
}

async fn restore(State(app): Shared, Path((id, v)): Path<(String, u64)>) -> Response {
    let view = {
        let mut guard = app.workflows.lock();
        let Some(graph) = guard.versions.get(&id).and_then(|l| l.iter().find(|x| x.v == v)).map(|x| x.graph.clone()) else {
            return fail(StatusCode::NOT_FOUND, "такой версии нет");
        };
        let Some(stored) = guard.get_mut(&id) else { return not_found() };
        stored.wf.graph = graph.clone();
        stored.wf.revision += 1;
        stored.wf.updated_at = now();
        wf::refresh_slots(stored, now());
        let (view, revision) = (stored.wf.clone(), stored.wf.revision);
        guard.save_wf(&id);
        guard.push_version(&id, "restore", &graph, revision);
        view
    };
    changed(&app, &id);
    reply(StatusCode::OK, json!({ "workflow": view }))
}

/// Новый секрет вебхука узла. Секрет виден только в этом ответе; на диске остаётся отпечаток.
async fn rotate_hook(State(app): Shared, headers: HeaderMap, Path((id, node_id)): Path<(String, String)>) -> Response {
    let secret = crate::guard::random_token();
    {
        let mut guard = app.workflows.lock();
        let Some(stored) = guard.get_mut(&id) else { return not_found() };
        if !stored.wf.graph.node(&node_id).is_some_and(|n| n.kind == "trigger" && n.text("trigger") == "webhook") {
            return fail(StatusCode::BAD_REQUEST, "это не запуск вебхуком");
        }
        stored.hooks.insert(node_id.clone(), wf::hook_digest(&secret));
        guard.save_wf(&id);
    }
    let host = headers.get(header::HOST).and_then(|v| v.to_str().ok()).unwrap_or("127.0.0.1");
    changed(&app, &id);
    reply(StatusCode::OK, json!({ "url": format!("http://{host}/hooks/workflows/{id}/{node_id}/{secret}") }))
}

const HOOK_BODY_CHARS: usize = 64_000;
const HOOK_DELIVERIES: usize = 200;

/// Входящий вебхук процесса. Неверный адрес, неверный секрет и выключенный процесс отвечают одинаково.
async fn incoming_hook(State(app): Shared, headers: HeaderMap, Path((id, node_id, secret)): Path<(String, String, String)>, body: Bytes) -> Response {
    if body.len() > HOOK_BODY_CHARS * 4 || String::from_utf8_lossy(&body).chars().count() > HOOK_BODY_CHARS {
        return fail(StatusCode::PAYLOAD_TOO_LARGE, "тело больше 64 000 знаков");
    }
    let delivery = ["x-delivery-id", "x-github-delivery", "idempotency-key"]
        .iter()
        .find_map(|name| headers.get(*name).and_then(|v| v.to_str().ok()))
        .map(|v| v.trim().chars().take(200).collect::<String>())
        .filter(|v| !v.is_empty());
    {
        let mut guard = app.workflows.lock();
        let found = guard.get(&id).is_some_and(|s| {
            let digest = wf::hook_digest(&secret);
            s.wf.enabled
                && s.wf.graph.node(&node_id).is_some_and(|n| n.kind == "trigger" && n.text("trigger") == "webhook")
                && s.hooks.get(&node_id).is_some_and(|stored| crate::guard::same(stored, &digest))
        });
        if !found {
            return fail(StatusCode::NOT_FOUND, "нет такого вебхука");
        }
        if let Some(key) = &delivery {
            if guard.deliveries.get(&id).is_some_and(|list| list.iter().any(|k| k == &format!("{node_id}:{key}"))) {
                return reply(StatusCode::OK, json!({ "ok": true, "duplicate": true }));
            }
        }
        if guard.has_open_run(&id) {
            return reply(StatusCode::ACCEPTED, json!({ "skipped": "busy" }));
        }
    }
    let data = crate::workflow_run::frame_external(&String::from_utf8_lossy(&body));
    match run::start_run_with(&app, &id, "webhook", None, None, Some(node_id.clone()), false, Some(data)) {
        Ok(started) => {
            if let Some(key) = delivery {
                let mut guard = app.workflows.lock();
                let list = guard.deliveries.entry(id).or_default();
                list.push_back(format!("{node_id}:{key}"));
                while list.len() > HOOK_DELIVERIES {
                    list.pop_front();
                }
            }
            reply(StatusCode::ACCEPTED, json!({ "ok": true, "run_id": started.id }))
        }
        Err(e) if e.status == 409 => reply(StatusCode::ACCEPTED, json!({ "skipped": "busy" })),
        Err(_) => fail(StatusCode::NOT_FOUND, "нет такого вебхука"),
    }
}
