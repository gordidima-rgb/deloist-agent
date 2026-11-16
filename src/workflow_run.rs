//! Движок запусков рабочих процессов: шаги, согласия, остановка, повтор, расписание.
//! Состояние живёт в `Workflows` и на диске; решения «что дальше» принимает чистый обход `workflow_walk`.

use std::collections::HashMap;
use std::sync::Arc;

use serde_json::json;
use tokio_util::sync::CancellationToken;

use crate::store::{now, uid};
use crate::workflow_walk as walk;
use crate::workflows::{collect_due, is_annotation, Inner, LastRun, Node, Run, Step, KEEP_RUNS};
use crate::App;

pub struct RunError {
    pub status: u16,
    pub message: String,
    pub code: Option<&'static str>,
}

fn err(status: u16, message: impl Into<String>) -> RunError {
    RunError { status, message: message.into(), code: None }
}

/// Что нужно сделать вне блокировки состояния.
enum Spawn {
    Bot { run_id: String, step_id: String, bot_id: String, text: String, unattended: bool, token: CancellationToken },
    Output { run_id: String, step_id: String, title: String, body: String },
    Notice { workflow_id: String, run_id: String, step_id: String, title: String, question: String },
}

type Tokens = HashMap<String, CancellationToken>;

fn clip(text: &str, limit: usize) -> String {
    let flat = text.trim();
    if flat.chars().count() <= limit {
        return flat.to_string();
    }
    format!("{}…", flat.chars().take(limit).collect::<String>().trim_end())
}

// ---------------------------------------------------------------------------
// Ядро: действия над одним запуском под блокировкой
// ---------------------------------------------------------------------------

fn finish(run: &mut Run, index: usize, status: &str, output: Option<String>, error: Option<String>) {
    run.counter += 1;
    let seq = run.counter;
    let step = &mut run.steps[index];
    step.status = status.to_string();
    if let Some(output) = output {
        step.output = output;
    }
    step.error = error;
    step.ended_at = Some(now());
    step.end_seq = seq;
}

fn new_step(run: &mut Run, node_id: &str, input: String, status: &str, attempt: u32) -> usize {
    run.counter += 1;
    let step = Step {
        id: uid("st"), run_id: run.id.clone(), node_id: node_id.to_string(), attempt, status: status.to_string(), input,
        started_at: now(), seq: run.counter, ..Step::default()
    };
    run.steps.push(step);
    run.steps.len() - 1
}

fn input_text(run: &Run, inputs: &[walk::Input]) -> String {
    let parts: Vec<String> = inputs
        .iter()
        .filter_map(|i| run.steps.iter().find(|s| s.id == i.step_id).map(|s| if i.failed { walk::failed_text(s.error.as_deref().unwrap_or("")) } else { s.output.clone() }))
        .collect();
    walk::join_inputs(&parts)
}

/// Начать шаг узла. Мгновенные виды заканчиваются здесь же, долгие уходят в `spawns`.
fn start_node(run: &mut Run, title: &str, node_id: &str, inputs: &[walk::Input], attempt: u32, tokens: &mut Tokens, spawns: &mut Vec<Spawn>) {
    let Some(node) = run.graph.node(node_id).cloned() else { return };
    let input = input_text(run, inputs);
    let kind = node.kind.clone();
    let status = if kind == "approval" { "approval" } else { "running" };
    let index = new_step(run, node_id, input.clone(), status, attempt);
    let step_id = run.steps[index].id.clone();
    match kind.as_str() {
        "trigger" => finish(run, index, "done", Some(String::new()), None),
        "prompt" => finish(run, index, "done", Some(node.text("text")), None),
        "approval" => {
            let question = node.text("question");
            let question = if question.trim().is_empty() { "Продолжить?".to_string() } else { question };
            spawns.push(Spawn::Notice { workflow_id: run.workflow_id.clone(), run_id: run.id.clone(), step_id, title: title.to_string(), question });
        }
        "output" => {
            let target = node.text("target");
            if matches!(target.as_str(), "" | "notify") {
                spawns.push(Spawn::Output { run_id: run.id.clone(), step_id, title: title.to_string(), body: clip(&input, 500) });
            } else {
                finish(run, index, "failed", None, Some("этот вывод будет на этапе 3".into()));
            }
        }
        "bot" => {
            let bot_id = node.text("bot_id");
            if bot_id.trim().is_empty() {
                finish(run, index, "failed", None, Some("в шаге не выбран бот".into()));
                return;
            }
            let text = format!("Рабочий процесс «{title}» поручает тебе шаг.\n\n{}\n\n--- вход ---\n{input}", node.text("task"));
            let token = CancellationToken::new();
            tokens.insert(step_id.clone(), token.clone());
            run.steps[index].thread_id = Some(format!("bot:{bot_id}"));
            spawns.push(Spawn::Bot { run_id: run.id.clone(), step_id, bot_id, text, unattended: run.source != "manual", token });
        }
        _ => finish(run, index, "failed", None, Some("будет на этапе 2".into())),
    }
}

fn stop_active(run: &mut Run, tokens: &mut Tokens, reason: Option<&str>) {
    for index in 0..run.steps.len() {
        if matches!(run.steps[index].status.as_str(), "running" | "approval") {
            if let Some(token) = tokens.remove(&run.steps[index].id) {
                token.cancel();
            }
            finish(run, index, "stopped", None, reason.map(str::to_string));
        }
    }
}

fn stop_core(run: &mut Run, tokens: &mut Tokens) {
    stop_active(run, tokens, None);
    run.status = "stopped".into();
    run.ended_at = Some(now());
}

/// Двигать запуск, пока есть что запускать или пропускать.
fn advance_core(run: &mut Run, title: &str, tokens: &mut Tokens, spawns: &mut Vec<Spawn>) {
    if run.status == "stopped" {
        return;
    }
    loop {
        let plan = walk::plan(&run.graph, run.from_node.as_deref(), run.trigger_node.as_deref());
        let next = walk::next(&run.graph, &run.steps, &plan);
        if next.overflow {
            stop_active(run, tokens, Some("запуск остановлен: слишком много кругов"));
            run.status = "failed".into();
            run.error = Some("слишком много кругов".into());
            run.ended_at = Some(now());
            return;
        }
        if next.actions.is_empty() {
            break;
        }
        for action in next.actions {
            match action {
                walk::Action::Skip { node } => {
                    let index = new_step(run, &node, String::new(), "skipped", 1);
                    finish(run, index, "skipped", None, None);
                }
                walk::Action::Start { node, inputs } => start_node(run, title, &node, &inputs, 1, tokens, spawns),
            }
        }
    }
    let running = run.steps.iter().any(|s| s.status == "running");
    let waiting = run.steps.iter().any(|s| s.status == "approval");
    if running {
        run.status = "running".into();
        run.ended_at = None;
    } else if waiting {
        run.status = "approval".into();
        run.ended_at = None;
    } else {
        run.status = walk::outcome(&run.graph, &run.steps).into();
        run.ended_at = Some(now());
    }
}

// ---------------------------------------------------------------------------
// Оболочка: блокировка, сохранение, события, запуск долгих шагов
// ---------------------------------------------------------------------------

fn change_event(app: &Arc<App>, workflow_id: &str, run_id: Option<&str>) {
    let mut event = json!({ "type": "workflows", "workflow_id": workflow_id });
    if let Some(run_id) = run_id {
        event["run_id"] = json!(run_id);
    }
    app.turns.emit(event);
}

/// Изменить запуск под блокировкой, сохранить, послать событие и запустить долгие шаги.
fn with_run<T>(app: &Arc<App>, run_id: &str, f: impl FnOnce(&mut Run, &str, &mut Tokens, &mut Vec<Spawn>) -> T) -> Option<T> {
    let mut spawns = Vec::new();
    let (value, wf_id) = {
        let mut guard = app.workflows.lock();
        let inner: &mut Inner = &mut guard;
        let wf_id = inner.runs.iter().find(|(_, list)| list.iter().any(|r| r.id == run_id)).map(|(k, _)| k.clone())?;
        let title = inner.get(&wf_id).map(|s| s.wf.title.clone()).unwrap_or_default();
        let (value, status, at, newest) = {
            let Inner { runs, tokens, .. } = &mut *inner;
            let list = runs.get_mut(&wf_id)?;
            let index = list.iter().position(|r| r.id == run_id)?;
            let newest = index + 1 == list.len();
            let run = &mut list[index];
            let value = f(run, &title, tokens, &mut spawns);
            (value, run.status.clone(), run.ended_at.unwrap_or(run.started_at), newest)
        };
        inner.save_runs(&wf_id);
        if newest {
            if let Some(stored) = inner.get_mut(&wf_id) {
                stored.wf.last_run = Some(LastRun { status, at });
            }
            inner.save_wf(&wf_id);
        }
        (value, wf_id)
    };
    change_event(app, &wf_id, Some(run_id));
    spawn_all(app, spawns);
    Some(value)
}

fn complete(app: &Arc<App>, run_id: &str, step_id: &str, result: Result<String, String>) {
    with_run(app, run_id, |run, title, tokens, spawns| {
        tokens.remove(step_id);
        let Some(index) = run.steps.iter().position(|s| s.id == step_id && s.status == "running") else { return };
        match result {
            Ok(output) => finish(run, index, "done", Some(output), None),
            Err(error) => finish(run, index, "failed", None, Some(error)),
        }
        advance_core(run, title, tokens, spawns);
    });
}

fn spawn_all(app: &Arc<App>, spawns: Vec<Spawn>) {
    for spawn in spawns {
        let app = app.clone();
        tokio::spawn(async move {
            match spawn {
                Spawn::Bot { run_id, step_id, bot_id, text, unattended, token } => {
                    let result = crate::turn::run_workflow_step(&app, &bot_id, &text, unattended, token).await;
                    complete(&app, &run_id, &step_id, result);
                }
                Spawn::Output { run_id, step_id, title, body } => {
                    let list = crate::reminders::send_text(&app, &format!("Рабочий процесс «{title}»"), &body, &["windows".to_string()]).await;
                    let error = list.into_iter().find_map(|d| d.error);
                    complete(&app, &run_id, &step_id, error.map_or_else(|| Ok(String::new()), |e| Err(format!("не удалось показать уведомление: {e}"))));
                }
                Spawn::Notice { workflow_id, run_id, step_id, title, question } => {
                    app.turns.emit(json!({ "type": "workflow_approval", "workflow_id": workflow_id, "run_id": run_id, "step_id": step_id, "title": title, "question": question }));
                    let _ = crate::reminders::send_text(&app, &format!("Рабочий процесс «{title}»: нужно решение"), &clip(&question, 300), &["windows".to_string()]).await;
                }
            }
        });
    }
}

fn snapshot(app: &Arc<App>, run_id: &str) -> Result<Run, RunError> {
    app.workflows.lock().run(run_id).cloned().ok_or_else(|| err(404, "запуска нет"))
}

fn run_of_step(app: &Arc<App>, step_id: &str) -> Result<String, RunError> {
    app.workflows.lock().run_of_step(step_id).ok_or_else(|| err(404, "шага нет"))
}

// ---------------------------------------------------------------------------
// Команды
// ---------------------------------------------------------------------------

pub fn start_run(app: &Arc<App>, wf_id: &str, source: &str, request_id: Option<String>, from_node: Option<String>, trigger_node: Option<String>) -> Result<Run, RunError> {
    let run_id = {
        let mut guard = app.workflows.lock();
        let inner: &mut Inner = &mut guard;
        let Some(stored) = inner.get(wf_id) else { return Err(err(404, "процесса нет")) };
        if let Some(request_id) = &request_id {
            if let Some(found) = inner.runs.get(wf_id).and_then(|list| list.iter().find(|r| r.request_id.as_deref() == Some(request_id.as_str()))) {
                return Ok(found.clone());
            }
        }
        let graph = stored.wf.graph.clone();
        if !graph.nodes.iter().any(|n| !is_annotation(&n.kind)) {
            return Err(RunError { status: 400, message: "в процессе нет шагов".into(), code: Some("empty_workflow") });
        }
        if let Some(from) = &from_node {
            if !graph.node(from).is_some_and(|n| !is_annotation(&n.kind)) {
                return Err(RunError { status: 400, message: "узла нет или с него нельзя начать".into(), code: Some("bad_node") });
            }
        }
        if source != "manual" && inner.has_open_run(wf_id) {
            return Err(err(409, "у процесса уже идёт запуск"));
        }
        let run = Run {
            id: uid("run"), workflow_id: wf_id.to_string(), status: "running".into(), source: source.to_string(), started_at: now(),
            revision: stored.wf.revision, graph, request_id, from_node, trigger_node, ..Run::default()
        };
        let id = run.id.clone();
        let list = inner.runs.entry(wf_id.to_string()).or_default();
        list.push(run);
        while list.len() > KEEP_RUNS {
            let Some(old) = list.iter().position(|r| !r.is_open()) else { break };
            list.remove(old);
        }
        id
    };
    with_run(app, &run_id, |run, title, tokens, spawns| advance_core(run, title, tokens, spawns));
    snapshot(app, &run_id)
}

pub fn stop_run(app: &Arc<App>, run_id: &str) -> Result<Run, RunError> {
    with_run(app, run_id, |run, _, tokens, _| {
        if run.is_open() {
            stop_core(run, tokens);
        }
    })
    .ok_or_else(|| err(404, "запуска нет"))?;
    snapshot(app, run_id)
}

pub fn stop_step(app: &Arc<App>, step_id: &str) -> Result<Run, RunError> {
    let run_id = run_of_step(app, step_id)?;
    let done = with_run(app, &run_id, |run, title, tokens, spawns| {
        let Some(index) = run.steps.iter().position(|s| s.id == step_id && matches!(s.status.as_str(), "running" | "approval")) else { return false };
        if let Some(token) = tokens.remove(step_id) {
            token.cancel();
        }
        finish(run, index, "stopped", None, None);
        advance_core(run, title, tokens, spawns);
        true
    });
    if done != Some(true) {
        return Err(err(409, "шаг уже не идёт"));
    }
    snapshot(app, &run_id)
}

pub fn retry_step(app: &Arc<App>, step_id: &str) -> Result<Run, RunError> {
    let run_id = run_of_step(app, step_id)?;
    // Новая попытка берёт текущие настройки узла из процесса.
    let fresh: Option<Node> = {
        let guard = app.workflows.lock();
        let run = guard.run(&run_id);
        run.and_then(|r| guard.get(&r.workflow_id).and_then(|s| r.steps.iter().find(|st| st.id == step_id).and_then(|st| s.wf.graph.node(&st.node_id).cloned())))
    };
    let result = with_run(app, &run_id, |run, title, tokens, spawns| -> Result<(), RunError> {
        let Some(old) = run.steps.iter().find(|s| s.id == step_id).cloned() else { return Err(err(404, "шага нет")) };
        if !walk::terminal(&old.status) {
            return Err(err(409, "шаг ещё идёт"));
        }
        let newest = run.steps.iter().filter(|s| s.node_id == old.node_id).map(|s| s.seq).max();
        if newest != Some(old.seq) {
            return Err(err(409, "повторить можно только последнюю попытку"));
        }
        if let Some(fresh) = fresh.filter(|n| run.graph.node(&n.id).is_some_and(|cur| cur.kind == n.kind)) {
            if let Some(slot) = run.graph.nodes.iter_mut().find(|n| n.id == fresh.id) {
                *slot = fresh;
            }
        }
        run.status = "running".into();
        run.ended_at = None;
        run.error = None;
        let inputs = walk::inputs_of(&run.graph, &run.steps, &old.node_id);
        start_node(run, title, &old.node_id, &inputs, old.attempt + 1, tokens, spawns);
        advance_core(run, title, tokens, spawns);
        Ok(())
    });
    match result {
        None => Err(err(404, "запуска нет")),
        Some(Err(e)) => Err(e),
        Some(Ok(())) => snapshot(app, &run_id),
    }
}

pub fn decide(app: &Arc<App>, step_id: &str, approved: bool) -> Result<Run, RunError> {
    let run_id = run_of_step(app, step_id)?;
    let done = with_run(app, &run_id, |run, title, tokens, spawns| {
        let Some(index) = run.steps.iter().position(|s| s.id == step_id && s.status == "approval") else { return false };
        if approved {
            let input = run.steps[index].input.clone();
            finish(run, index, "done", Some(input), None);
            advance_core(run, title, tokens, spawns);
        } else {
            finish(run, index, "stopped", None, Some("отклонено".into()));
            stop_core(run, tokens);
        }
        true
    });
    if done != Some(true) {
        return Err(err(409, "решение уже принято"));
    }
    snapshot(app, &run_id)
}

/// Остановить открытые запуски процесса, убрать его файлы. `false` — такого процесса нет.
pub fn delete_workflow(app: &Arc<App>, wf_id: &str) -> bool {
    let open: Vec<String> = {
        let guard = app.workflows.lock();
        if guard.get(wf_id).is_none() {
            return false;
        }
        guard.runs.get(wf_id).map(|list| list.iter().filter(|r| r.is_open()).map(|r| r.id.clone()).collect()).unwrap_or_default()
    };
    for id in open {
        let _ = stop_run(app, &id);
    }
    {
        let mut guard = app.workflows.lock();
        guard.items.retain(|s| s.wf.id != wf_id);
        guard.runs.remove(wf_id);
        guard.remove_files(wf_id);
    }
    change_event(app, wf_id, None);
    true
}

/// После перезапуска: шаги, которые шли, прерваны; запуски продолжают проход, согласия ждут дальше.
pub fn resume(app: &Arc<App>) {
    let open: Vec<String> = {
        let mut guard = app.workflows.lock();
        let inner: &mut Inner = &mut guard;
        let mut open = Vec::new();
        let mut changed: Vec<String> = Vec::new();
        for (wf_id, list) in inner.runs.iter_mut() {
            for run in list.iter_mut().filter(|r| r.is_open()) {
                if run.status == "stopping" {
                    run.status = "stopped".into();
                    run.ended_at = Some(now());
                } else {
                    for index in 0..run.steps.len() {
                        if run.steps[index].status == "running" {
                            finish(run, index, "failed", None, Some("прервано перезапуском".into()));
                        }
                    }
                    open.push(run.id.clone());
                }
                if !changed.contains(wf_id) {
                    changed.push(wf_id.clone());
                }
            }
        }
        for wf_id in changed {
            inner.save_runs(&wf_id);
        }
        open
    };
    for run_id in open {
        with_run(app, &run_id, |run, title, tokens, spawns| advance_core(run, title, tokens, spawns));
    }
}

/// Раз в тик планировщика: сработавшие запускающие узлы по расписанию.
pub fn tick(app: &Arc<App>) {
    let at = now();
    let fire: Vec<(String, String)> = {
        let mut guard = app.workflows.lock();
        let mut fire = Vec::new();
        let mut changed = Vec::new();
        for stored in guard.items.iter_mut() {
            let (due, touched) = collect_due(stored, at);
            if touched {
                changed.push(stored.wf.id.clone());
            }
            fire.extend(due.into_iter().map(|node| (stored.wf.id.clone(), node)));
        }
        // Срок сдвинут и записан до запуска: сбой запуска не повторит его каждые полминуты.
        for id in changed {
            guard.save_wf(&id);
        }
        fire
    };
    for (wf_id, node) in fire {
        // Пока у процесса есть открытый запуск, новый по расписанию не стартует (слот сгорает).
        match start_run(app, &wf_id, "schedule", None, None, Some(node)) {
            Ok(run) => change_event(app, &wf_id, Some(&run.id)),
            Err(e) => eprintln!("[workflows] запуск по расписанию не начат: {}", e.message),
        }
    }
}
