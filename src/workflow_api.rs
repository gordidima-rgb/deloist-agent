//! Адреса рабочих процессов (этап 1). Все они закрыты для ключа программы (`guard::human_only`):
//! бот не создаёт, не меняет, не запускает процессы и не решает согласия.

use std::sync::Arc;

use axum::body::Bytes;
use axum::extract::{DefaultBodyLimit, Path, Query, State};
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
        .route("/api/workflows/{id}/assets", post(upload_asset).layer(DefaultBodyLimit::max(UPLOAD_BODY_LIMIT)))
        .route("/api/workflows/{id}/files/{*path}", get(serve_file))
        .route("/api/workflows/{id}/nodes/{node_id}/webhook", post(rotate_hook))
        .route("/hooks/workflows/{id}/{node_id}/{secret}", post(incoming_hook))
        .route("/hooks/{id}/{secret}", post(routine_hook))
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

fn graph_of(app: &Arc<App>, value: &Value) -> Result<Graph, Response> {
    let graph: Graph = serde_json::from_value(value.clone()).map_err(|e| reply(StatusCode::BAD_REQUEST, json!({ "error": format!("граф не разобран: {e}"), "code": "bad_graph" })))?;
    wf::validate_graph(&graph).and_then(|()| wf::check_providers(app, &graph)).map_err(|e| {
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
    forget_dead_hooks(app, id);
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
        Some(value) => match graph_of(&app, value) {
            Ok(g) => g,
            Err(r) => return r,
        },
        None => Graph::default(),
    };
    reply(StatusCode::OK, json!({ "workflow": run::create_workflow(&app, &title, graph) }))
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
        .map(|n| json!({ "node_id": n.id, "type": if n.text("trigger").is_empty() { "manual".to_string() } else { n.text("trigger") }, "next_run": stored.slots.get(&n.id).and_then(|s| s.next), "hook": app.connectors.secrets.get(&wf::hook_scope(&id), &wf::hook_name(&n.id)).is_some() }))
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
    let graph = match graph_of(&app, body.get("graph").unwrap_or(&Value::Null)) {
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
        if let Err(e) = wf::validate_graph(&graph).and_then(|()| wf::check_providers(&app, &graph)) {
            return fail(StatusCode::BAD_REQUEST, &format!("версию нельзя восстановить: {}", e.message));
        }
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

/// Новый секрет вебхука узла. Секрет виден только в этом ответе; на диске он лежит лишь в зашифрованных секретах.
async fn rotate_hook(State(app): Shared, headers: HeaderMap, Path((id, node_id)): Path<(String, String)>) -> Response {
    let secret = crate::guard::random_token();
    {
        let guard = app.workflows.lock();
        let Some(stored) = guard.get(&id) else { return not_found() };
        if hook_events(stored, &node_id).is_none() {
            return fail(StatusCode::BAD_REQUEST, "это не запуск вебхуком");
        }
    }
    if let Err(e) = app.connectors.secrets.set(&wf::hook_scope(&id), &wf::hook_name(&node_id), &secret) {
        return fail(StatusCode::INTERNAL_SERVER_ERROR, &e);
    }
    let host = headers.get(header::HOST).and_then(|v| v.to_str().ok()).unwrap_or("127.0.0.1");
    changed(&app, &id);
    reply(StatusCode::OK, json!({ "url": format!("http://{host}/hooks/workflows/{id}/{node_id}/{secret}") }))
}

/// События GitHub, которые узел-вебхук пропускает (`hook_events` в полях узла, через запятую; пусто — все). `None` — узел не вебхук.
fn hook_events(stored: &Stored, node_id: &str) -> Option<Vec<String>> {
    let node = stored.wf.graph.node(node_id).filter(|n| n.kind == "trigger" && n.text("trigger") == "webhook")?;
    Some(node.text("hook_events").split(',').map(str::trim).filter(|e| !e.is_empty()).map(str::to_string).collect())
}

/// Секреты вебхуков узлов, которых больше нет или которые перестали быть вебхуком, стираются: адрес мёртв и не оживёт.
pub(crate) fn forget_dead_hooks(app: &Arc<App>, id: &str) {
    let secrets = &app.connectors.secrets;
    let scope = wf::hook_scope(id);
    let alive: Vec<String> = {
        let guard = app.workflows.lock();
        guard.get(id).map(|s| s.wf.graph.nodes.iter().filter(|n| hook_events(s, &n.id).is_some()).map(|n| wf::hook_name(&n.id)).collect()).unwrap_or_default()
    };
    for name in secrets.names(&scope).into_iter().filter(|name| !alive.contains(name)) {
        let _ = secrets.delete(&scope, &name);
    }
}

const HOOK_BODY_CHARS: usize = 64_000;
const GITHUB_BODY_BYTES: usize = 1024 * 1024;
const GITHUB_SUMMARY_CHARS: usize = 8_000;
const HOOK_DELIVERIES: usize = 200;
const HOOK_MIN_GAP_MS: i64 = 10_000;

/// Старый адрес вебхука рутины `/hooks/<номер>/<секрет>`: рутина стала процессом с тем же номером, вебхук — его узел `t1`.
async fn routine_hook(app: State<Arc<App>>, headers: HeaderMap, Path((id, secret)): Path<(String, String)>, body: Bytes) -> Response {
    incoming_hook(app, headers, Path((id, crate::routine_migrate::TRIGGER.to_string(), secret)), body).await
}

/// Входящий вебхук процесса. Неверный адрес, неверный секрет и выключенный процесс отвечают одинаково.
/// Пускают секрет в адресе и, у GitHub, подпись тела этим же секретом.
async fn incoming_hook(State(app): Shared, headers: HeaderMap, Path((id, node_id, secret)): Path<(String, String, String)>, body: Bytes) -> Response {
    let github = headers.get("x-github-event").and_then(|v| v.to_str().ok()).map(str::to_string);
    if github.is_some() {
        if body.len() > GITHUB_BODY_BYTES {
            return fail(StatusCode::PAYLOAD_TOO_LARGE, "тело больше 1 МБ");
        }
    } else if body.len() > HOOK_BODY_CHARS * 4 || String::from_utf8_lossy(&body).chars().count() > HOOK_BODY_CHARS {
        return fail(StatusCode::PAYLOAD_TOO_LARGE, "тело больше 64 000 знаков");
    }
    let delivery = ["x-delivery-id", "x-github-delivery", "idempotency-key"]
        .iter()
        .find_map(|name| headers.get(*name).and_then(|v| v.to_str().ok()))
        .map(|v| v.trim().chars().take(200).collect::<String>())
        .filter(|v| !v.is_empty());
    let stored_secret = app.connectors.secrets.get(&wf::hook_scope(&id), &wf::hook_name(&node_id));
    let allowed = {
        let guard = app.workflows.lock();
        guard.get(&id).filter(|s| s.wf.enabled).and_then(|s| hook_events(s, &node_id))
    };
    let (Some(stored_secret), Some(allowed)) = (stored_secret, allowed) else { return fail(StatusCode::NOT_FOUND, "нет такого вебхука") };
    if !crate::guard::same(&stored_secret, &secret) {
        return fail(StatusCode::NOT_FOUND, "нет такого вебхука");
    }
    // GitHub: подпись проверяется до разбора JSON.
    let mut data = None;
    if let Some(event) = github.as_ref() {
        let signature = headers.get("x-hub-signature-256").and_then(|v| v.to_str().ok()).unwrap_or("");
        if !verify_github_signature(&stored_secret, &body, signature) {
            return fail(StatusCode::UNAUTHORIZED, "подпись GitHub не сошлась");
        }
        if event == "ping" {
            return reply(StatusCode::OK, json!({ "ok": true, "ping": true }));
        }
        let value: Value = serde_json::from_slice(&body).unwrap_or(Value::Null);
        let action = value.get("action").and_then(Value::as_str).unwrap_or("").to_string();
        let composite = if action.is_empty() { event.clone() } else { format!("{event}.{action}") };
        if !allowed.is_empty() && !allowed.contains(event) && !allowed.contains(&composite) {
            return reply(StatusCode::ACCEPTED, json!({ "ok": true, "skipped": true }));
        }
        let mut summary = github_summary(event, &action, &value);
        if summary.chars().count() > GITHUB_SUMMARY_CHARS {
            summary = summary.chars().take(GITHUB_SUMMARY_CHARS - 1).collect::<String>() + "…";
        }
        data = Some(crate::workflow_run::frame_external(&format!("[GitHub: {composite}]\n{summary}")));
    }
    let data = data.unwrap_or_else(|| crate::workflow_run::frame_external(&String::from_utf8_lossy(&body)));
    // Бот процесса ждал именно этого вебхука (`await_event`): его ход продолжается с данными, нового запуска нет.
    if crate::runs::resume_on_webhook(&app, &id, &data).await {
        return reply(StatusCode::ACCEPTED, json!({ "ok": true, "resumed": true }));
    }
    {
        let guard = app.workflows.lock();
        if let Some(key) = &delivery {
            if guard.deliveries.get(&id).is_some_and(|list| list.iter().any(|k| k == &format!("{node_id}:{key}"))) {
                return reply(StatusCode::OK, json!({ "ok": true, "duplicate": true }));
            }
        }
        if guard.has_open_run(&id) {
            return reply(StatusCode::ACCEPTED, json!({ "skipped": "busy" }));
        }
        // Не чаще одного запуска в 10 секунд на процесс: внешний вызов не должен сжечь токены частотой.
        if guard.last_hook.get(&id).is_some_and(|at| now() - *at < HOOK_MIN_GAP_MS) {
            return fail(StatusCode::TOO_MANY_REQUESTS, "слишком часто: не чаще раза в 10 секунд");
        }
    }
    match run::start_run_with(&app, &id, "webhook", None, None, Some(node_id.clone()), false, Some(data)) {
        Ok(started) => {
            app.workflows.lock().last_hook.insert(id.clone(), now());
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
        Err(e) if e.status == 429 => fail(StatusCode::TOO_MANY_REQUESTS, &e.message),
        Err(_) => fail(StatusCode::NOT_FOUND, "нет такого вебхука"),
    }
}

const MAX_ASSET_BYTES: usize = 20 * 1024 * 1024;
const MAX_ASSETS_TOTAL: u64 = 200 * 1024 * 1024;

fn dir_size(dir: &std::path::Path) -> u64 {
    std::fs::read_dir(dir).map(|entries| entries.flatten().filter_map(|e| e.metadata().ok()).filter(|m| m.is_file()).map(|m| m.len()).sum()).unwrap_or(0)
}
/// Тело с запасом на base64 (4/3) и поля JSON.
const UPLOAD_BODY_LIMIT: usize = 28 * 1024 * 1024;

fn good_mime(mime: &str) -> bool {
    let valid = |part: &str| !part.is_empty() && part.len() <= 64 && part.bytes().all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'+' | b'-'));
    mime.split_once('/').is_some_and(|(a, b)| valid(a) && valid(b))
}

/// Загрузка файла для узла `asset`: JSON `{name, data (base64), mime?}` (охрана пускает тела только как JSON). Не больше 20 МБ.
async fn upload_asset(State(app): Shared, Path(id): Path<String>, bytes: Bytes) -> Response {
    if app.workflows.lock().get(&id).is_none() {
        return not_found();
    }
    let body = match body_of(&bytes) {
        Ok(b) => b,
        Err(r) => return r,
    };
    use base64::Engine;
    let encoded = body.get("data").and_then(Value::as_str).unwrap_or_default();
    if encoded.len() > MAX_ASSET_BYTES / 3 * 4 + 16 {
        return fail(StatusCode::PAYLOAD_TOO_LARGE, "файл больше 20 МБ");
    }
    let Ok(data) = base64::engine::general_purpose::STANDARD.decode(encoded.trim()) else { return fail(StatusCode::BAD_REQUEST, "data не base64") };
    let raw_name = body.get("name").and_then(Value::as_str).unwrap_or_default().to_string();
    let mime = body.get("mime").and_then(Value::as_str).unwrap_or_default().to_string();
    if data.is_empty() {
        return fail(StatusCode::BAD_REQUEST, "пустой файл");
    }
    if data.len() > MAX_ASSET_BYTES {
        return fail(StatusCode::PAYLOAD_TOO_LARGE, "файл больше 20 МБ");
    }
    // Имя из окна — только имя: путь, диск и зарезервированные имена отклоняются.
    let name = match crate::attach::clean_name(&raw_name).and_then(|n| crate::workflow_io::safe_name(&n).map(|()| n)) {
        Ok(n) => n,
        Err(e) => return fail(StatusCode::BAD_REQUEST, &e),
    };
    let mime = if good_mime(mime.trim()) && mime.trim() != "application/octet-stream" {
        mime.trim().to_ascii_lowercase()
    } else {
        crate::attach::image_mime(&name).or_else(|| crate::attach::media_mime(&name)).unwrap_or("application/octet-stream").to_string()
    };
    let (stem, ext) = match name.rsplit_once('.') {
        Some((stem, e)) if !stem.is_empty() => (stem.to_string(), format!(".{e}")),
        _ => (name.clone(), String::new()),
    };
    let size = data.len();
    let (data_dir, wf) = (app.store.data_dir().to_path_buf(), id.clone());
    // Всего файлов на процесс не больше 200 МБ.
    let used = dir_size(&crate::workflow_io::files_dir(&data_dir, &wf).join("assets"));
    if used + size as u64 > MAX_ASSETS_TOTAL {
        return fail(StatusCode::PAYLOAD_TOO_LARGE, "у процесса уже много файлов: всего не больше 200 МБ");
    }
    let saved = tokio::task::spawn_blocking(move || {
        // Занятое имя не затираем: берём следующий номер.
        for counter in 1..=99 {
            let file = if counter == 1 { name.clone() } else { format!("{stem}-{counter}{ext}") };
            let rel = format!("assets/{file}");
            let abs = crate::workflow_io::files_dir(&data_dir, &wf).join("assets").join(&file);
            if abs.exists() {
                continue;
            }
            return crate::workflow_io::write_bytes(&data_dir, &wf, &rel, &data).map(|_| (file, rel));
        }
        Err("слишком много файлов с таким именем".to_string())
    })
    .await;
    match saved {
        Ok(Ok((file, rel))) => {
            changed(&app, &id);
            reply(StatusCode::OK, json!({ "name": file, "path": rel, "mime": mime, "size": size }))
        }
        Ok(Err(e)) => fail(StatusCode::BAD_REQUEST, &e),
        Err(_) => fail(StatusCode::INTERNAL_SERVER_ERROR, "не удалось сохранить файл"),
    }
}

/// Файл из папки процесса для миниатюр и просмотра на холсте. Только внутри папки процесса.
async fn serve_file(State(app): Shared, Path((id, rel)): Path<(String, String)>) -> Response {
    if app.workflows.lock().get(&id).is_none() {
        return not_found();
    }
    let (data_dir, wf, path) = (app.store.data_dir().to_path_buf(), id.clone(), rel.clone());
    let read = tokio::task::spawn_blocking(move || {
        let abs = crate::workflow_io::resolve_file(&data_dir, &wf, &path)?;
        let size = std::fs::metadata(&abs).map_err(|e| e.to_string())?.len() as usize;
        if size > MAX_ASSET_BYTES {
            return Err("файл больше 20 МБ".to_string());
        }
        std::fs::read(&abs).map_err(|e| e.to_string())
    })
    .await;
    let bytes = match read {
        Ok(Ok(b)) => b,
        Ok(Err(e)) => return fail(StatusCode::NOT_FOUND, &e),
        Err(_) => return fail(StatusCode::INTERNAL_SERVER_ERROR, "не удалось прочитать файл"),
    };
    // Только картинки и мультимедиа показываются; всё остальное — вложением, и никакого угадывания типа (nosniff).
    let shown = crate::attach::image_mime(&rel).filter(|m| *m != "image/svg+xml").or_else(|| crate::attach::media_mime(&rel));
    let (mime, disposition) = match shown {
        Some(m) => (m, "inline"),
        None => ("application/octet-stream", "attachment"),
    };
    (
        [
            (header::CONTENT_TYPE, mime.to_string()),
            (header::X_CONTENT_TYPE_OPTIONS, "nosniff".to_string()),
            (header::CONTENT_DISPOSITION, disposition.to_string()),
            (header::CACHE_CONTROL, "no-store".to_string()),
        ],
        bytes,
    )
        .into_response()
}
pub(crate) fn verify_github_signature(secret: &str, body: &[u8], provided_header: &str) -> bool {
    let provided = provided_header.strip_prefix("sha256=").unwrap_or("");
    if provided.len() != 64 || !provided.bytes().all(|b| b.is_ascii_hexdigit()) {
        return false;
    }
    use hmac::{Hmac, Mac};
    type HmacSha256 = Hmac<sha2::Sha256>;
    let mut mac = HmacSha256::new_from_slice(secret.as_bytes()).expect("hmac key");
    mac.update(body);
    let expected = crate::store::hex(&mac.finalize().into_bytes());
    crate::guard::same(&expected, &provided.to_ascii_lowercase())
}

fn github_summary(event: &str, action: &str, value: &Value) -> String {
    let mut lines: Vec<String> = Vec::new();
    if let Some(repo) = value.get("repository").and_then(|r| r.get("full_name")).and_then(Value::as_str) {
        lines.push(format!("repository: {}", repo));
    }
    if !action.is_empty() {
        lines.push(format!("action: {}", action));
    }
    if let Some(sender) = value.get("sender").and_then(|s| s.get("login")).and_then(Value::as_str) {
        lines.push(format!("sender: {}", sender));
    }
    match event {
        "push" => {
            if let Some(r) = value.get("ref").and_then(Value::as_str) { lines.push(format!("ref: {}", r)); }
            if let Some(c) = value.get("compare").and_then(Value::as_str) { lines.push(format!("compare: {}", c)); }
            if let Some(commits) = value.get("commits").and_then(Value::as_array) {
                lines.push(format!("commits ({}):", commits.len()));
                for c in commits.iter().take(10) {
                    let id = c.get("id").and_then(Value::as_str).unwrap_or("");
                    let short = if id.len() >= 7 { &id[..7] } else { id };
                    let msg = c.get("message").and_then(Value::as_str).unwrap_or("");
                    let first_line = msg.lines().next().unwrap_or("").trim();
                    let author = c.get("author").and_then(|a| a.get("name")).and_then(Value::as_str).unwrap_or("");
                    lines.push(format!("- {} {} ({})", short, first_line, author));
                }
            }
        }
        "pull_request" => {
            if let Some(pr) = value.get("pull_request") {
                if let Some(n) = pr.get("number") { lines.push(format!("number: {}", n)); }
                if let Some(t) = pr.get("title").and_then(Value::as_str) { lines.push(format!("title: {}", t)); }
                if let Some(u) = pr.get("html_url").and_then(Value::as_str) { lines.push(format!("url: {}", u)); }
                if let Some(user) = pr.get("user").and_then(|u| u.get("login")).and_then(Value::as_str) { lines.push(format!("user: {}", user)); }
                if let Some(b) = pr.get("base").and_then(|x| x.get("ref")).and_then(Value::as_str) { lines.push(format!("base: {}", b)); }
                if let Some(h) = pr.get("head").and_then(|x| x.get("ref")).and_then(Value::as_str) { lines.push(format!("head: {}", h)); }
                if let Some(body) = pr.get("body").and_then(Value::as_str) {
                    let b: String = body.chars().take(1000).collect();
                    lines.push(format!("body: {}", b));
                }
            }
        }
        "issues" => {
            if let Some(issue) = value.get("issue") {
                issue_summary(&mut lines, issue);
            }
        }
        "issue_comment" => {
            if let Some(issue) = value.get("issue") {
                issue_summary(&mut lines, issue);
            }
            if let Some(comment) = value.get("comment") {
                if let Some(body) = comment.get("body").and_then(Value::as_str) {
                    let b: String = body.chars().take(1000).collect();
                    lines.push(format!("comment: {}", b));
                }
            }
        }
        "release" => {
            if let Some(rel) = value.get("release") {
                if let Some(t) = rel.get("tag_name").and_then(Value::as_str) { lines.push(format!("tag: {}", t)); }
                if let Some(n) = rel.get("name").and_then(Value::as_str) { lines.push(format!("name: {}", n)); }
                if let Some(u) = rel.get("html_url").and_then(Value::as_str) { lines.push(format!("url: {}", u)); }
            }
        }
        _ => {
            let mut keys: Vec<&str> = value.as_object().map(|o| o.keys().map(|s| s.as_str()).collect()).unwrap_or_default();
            keys.sort();
            lines.push(format!("keys: {}", keys.join(", ")));
        }
    }
    lines.join("\n")
}

fn issue_summary(lines: &mut Vec<String>, issue: &Value) {
    if let Some(n) = issue.get("number") { lines.push(format!("number: {}", n)); }
    if let Some(t) = issue.get("title").and_then(Value::as_str) { lines.push(format!("title: {}", t)); }
    if let Some(u) = issue.get("html_url").and_then(Value::as_str) { lines.push(format!("url: {}", u)); }
    if let Some(body) = issue.get("body").and_then(Value::as_str) {
        let b: String = body.chars().take(1000).collect();
        lines.push(format!("body: {}", b));
    }
}

#[cfg(test)]
mod github_signature_tests {
    use super::verify_github_signature;
    #[test]
    fn documented_example() {
        assert!(verify_github_signature(
            "It's a Secret to Everybody",
            b"Hello, World!",
            "sha256=757107ea0eb2509fc211221cce984b8a37570b6d7586c22c46f4379c8b043e17",
        ));
        assert!(!verify_github_signature("secret", b"Hello, World!", "sha256=0000000000000000000000000000000000000000000000000000000000000000"));
    }
}
