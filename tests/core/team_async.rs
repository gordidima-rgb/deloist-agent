//! Работа команды без ожидания: `assign_task`, `task_status`, `team_status`, `share_file`.
//! Модель-заглушка: ведущий читает «SCRIPT [{name,args}…]» в сообщении человека и по одному вызывает эти инструменты,
//! участники отвечают через паузу, чтобы было видно, что они работают одновременно.

use crate::common;

use axum::body::{Body, Bytes};
use axum::extract::State;
use axum::response::Response;
use axum::routing::post;
use axum::Router;
use common::Crew;
use serde_json::{json, Value};
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

#[derive(Clone, Default)]
struct Model {
    requests: Arc<Mutex<Vec<Value>>>,
    inflight: Arc<AtomicUsize>,
    peak: Arc<AtomicUsize>,
    /// Сколько стоит ответ участника, мс.
    pause: Arc<AtomicUsize>,
}

fn sse(chunks: &[Value]) -> String {
    let mut out: String = chunks.iter().map(|c| format!("data: {c}\n\n")).collect();
    out.push_str("data: [DONE]\n\n");
    out
}

fn text_step(text: &str) -> String {
    sse(&[
        json!({ "choices": [{ "index": 0, "delta": { "role": "assistant", "content": text } }] }),
        json!({ "choices": [{ "index": 0, "delta": {}, "finish_reason": "stop" }], "usage": { "prompt_tokens": 10, "completion_tokens": 5 } }),
    ])
}

fn tool_step(name: &str, args: Value, n: usize) -> String {
    sse(&[
        json!({ "choices": [{ "index": 0, "delta": { "tool_calls": [{ "index": 0, "id": format!("call_{n}"), "type": "function", "function": { "name": name, "arguments": args.to_string() } }] } }] }),
        json!({ "choices": [{ "index": 0, "delta": {}, "finish_reason": "tool_calls" }], "usage": { "prompt_tokens": 100, "completion_tokens": 10 } }),
    ])
}

fn text_of(message: &Value) -> String {
    match &message["content"] {
        Value::String(text) => text.clone(),
        Value::Array(parts) => parts.iter().filter_map(|part| part["text"].as_str()).collect::<Vec<_>>().join("\n"),
        _ => String::new(),
    }
}

async fn completions(State(model): State<Model>, body: Bytes) -> Response {
    let payload: Value = serde_json::from_slice(&body).unwrap_or_else(|_| json!({}));
    model.requests.lock().unwrap().push(payload.clone());
    let messages = payload["messages"].as_array().cloned().unwrap_or_default();
    let system = messages.first().map(text_of).unwrap_or_default();
    // Сценарий — в последнем сообщении человека с «SCRIPT»; шаги считаются по вызовам после него.
    let at = messages.iter().rposition(|m| m["role"] == "user" && text_of(m).contains("SCRIPT "));
    let done_calls = at.map_or(0, |at| messages[at..].iter().filter(|m| m["role"] == "tool").count());
    let script: Option<Vec<Value>> = at.and_then(|at| {
        let t = text_of(&messages[at]);
        t.find("SCRIPT ").and_then(|i| serde_json::Deserializer::from_str(t[i + 7..].trim()).into_iter::<Vec<Value>>().next().and_then(Result::ok))
    });
    let text = if let Some(script) = script {
        match script.get(done_calls) {
            Some(call) => tool_step(call["name"].as_str().unwrap_or_default(), call["args"].clone(), done_calls),
            None => text_step("сценарий закончен"),
        }
    } else {
        // Участник: работает дольше мгновения, и видно, сколько их работает вместе.
        let now = model.inflight.fetch_add(1, Ordering::SeqCst) + 1;
        model.peak.fetch_max(now, Ordering::SeqCst);
        tokio::time::sleep(Duration::from_millis(model.pause.load(Ordering::SeqCst) as u64)).await;
        model.inflight.fetch_sub(1, Ordering::SeqCst);
        let who = ["Бета", "Гамма", "Альфа"].into_iter().find(|n| system.contains(&format!("Ты — {n}"))).unwrap_or("?");
        text_step(&format!("Итог от {who}: всё сделано"))
    };
    Response::builder().header("content-type", "text/event-stream").body(Body::from(text)).unwrap()
}

struct Team {
    crew: Crew,
    alpha: String,
    beta: String,
    gamma: String,
    outsider: String,
    room: String,
    model: Model,
}

async fn team(name: &str, pause_ms: usize) -> Team {
    let model = Model::default();
    model.pause.store(pause_ms, Ordering::SeqCst);
    let router = Router::new().route("/v1/chat/completions", post(completions)).with_state(model.clone());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
    let crew = Crew::start(name).await;
    crew.ok("PUT", "/api/settings", Some(json!({ "driver": "api", "model": "m", "baseUrl": format!("http://127.0.0.1:{port}/v1"), "apiKey": "test-key" }))).await;
    crew.listen().await;
    start_team(crew, model).await
}

async fn start_team(crew: Crew, model: Model) -> Team {
    let mut ids = Vec::new();
    for (name, role) in [("Альфа", "ведущий"), ("Бета", "картинки"), ("Гамма", "посты"), ("Дельта", "чужая")] {
        let id = crew.ok("POST", "/api/bots", Some(json!({ "name": name, "role": role, "approval": "auto" }))).await["bot"]["id"].as_str().unwrap().to_string();
        ids.push(id);
    }
    let room = crew.ok("POST", "/api/rooms", Some(json!({ "name": "Редакция", "botIds": [ids[0], ids[1], ids[2]], "lead": ids[0] }))).await["room"]["id"].as_str().unwrap().to_string();
    Team { crew, alpha: ids[0].clone(), beta: ids[1].clone(), gamma: ids[2].clone(), outsider: ids[3].clone(), room, model }
}

fn script(calls: &[(&str, Value)]) -> String {
    let list: Vec<Value> = calls.iter().map(|(name, args)| json!({ "name": name, "args": args })).collect();
    format!("SCRIPT {}", Value::Array(list))
}

impl Team {
    /// Ход бота по сценарию; возвращает результаты инструментов по порядку.
    async fn run(&self, bot: &str, calls: &[(&str, Value)]) -> Vec<String> {
        let before = self.crew.count_events(|e| e["type"] == "turn_end" && e["botId"] == bot);
        self.crew.say(bot, &script(calls)).await;
        common::wait_until("конец хода", 30, || async { self.crew.count_events(|e| e["type"] == "turn_end" && e["botId"] == bot) > before }).await;
        let thread = self.crew.ok("GET", &format!("/api/bots/{bot}/thread?limit=200"), None).await;
        // Результаты этого сценария — последние `calls.len()` вызовов в треде.
        let mut results: Vec<String> = thread["messages"]
            .as_array()
            .unwrap()
            .iter()
            .flat_map(|m| m["toolCalls"].as_array().cloned().unwrap_or_default())
            .map(|c| c["result"].as_str().unwrap_or_default().to_string())
            .collect();
        results.split_off(results.len().saturating_sub(calls.len()))
    }

    fn workspace(&self, bot: &str) -> PathBuf {
        self.crew.data_dir.join("workspaces").join(bot)
    }

    fn tasks_file(&self) -> Vec<Value> {
        let text = std::fs::read_to_string(self.crew.data_dir.join("team_tasks.json")).unwrap_or_else(|_| "[]".into());
        serde_json::from_str(&text).unwrap()
    }

    async fn task_state(&self, id: &str) -> String {
        self.tasks_file().iter().find(|t| t["id"] == id).map(|t| t["state"].as_str().unwrap_or_default().to_string()).unwrap_or_default()
    }
}

fn task_id(result: &str) -> String {
    result.split("taskId ").nth(1).and_then(|r| r.split(|c: char| !(c.is_alphanumeric() || c == '_')).next()).unwrap_or_default().to_string()
}

fn is_task_done_for(bot: &str) -> impl Fn(&Value) -> bool + '_ {
    move |e| e["type"] == "task_done" && e["botId"] == bot
}

#[tokio::test(flavor = "multi_thread")]
async fn two_tasks_run_in_parallel_and_finish_without_waking_the_lead() {
    let t = team("async-parallel", 1500).await;
    let started = std::time::Instant::now();
    let out = t
        .run(&t.alpha, &[
            ("assign_task", json!({ "bot": "Бета", "text": "Нарисуй обложку" })),
            ("assign_task", json!({ "bot": "Гамма", "text": "Напиши пост" })),
        ])
        .await;
    assert!(started.elapsed() < Duration::from_millis(1400), "assign_task не должен ждать участника: {:?}", started.elapsed());
    let (first, second) = (task_id(&out[0]), task_id(&out[1]));
    assert!(first.starts_with("task_") && second.starts_with("task_") && first != second, "{out:?}");

    t.crew.wait_event("первая закончила", is_task_done_for(&t.alpha)).await;
    common::wait_until("обе закончили", 30, || async { t.crew.count_events(is_task_done_for(&t.alpha)) == 2 }).await;
    assert_eq!(t.model.peak.load(Ordering::SeqCst), 2, "участники работали одновременно");
    assert_eq!(t.task_state(&first).await, "done");
    assert_eq!(t.task_state(&second).await, "done");

    // Ведущий не просыпался: ход у него был один (тот, что раздал задачи), новых запросов к модели от него нет.
    let lead_turns = t.crew.count_events(|e| e["type"] == "turn_start" && e["botId"] == t.alpha.as_str());
    let lead_requests = t.model.requests.lock().unwrap().iter().filter(|r| r["messages"][0]["content"].as_str().is_some_and(|s| s.contains("Ты — Альфа"))).count();
    tokio::time::sleep(Duration::from_millis(1500)).await;
    assert_eq!(lead_turns, 1);
    assert_eq!(t.crew.count_events(|e| e["type"] == "turn_start" && e["botId"] == t.alpha.as_str()), 1, "завершение задачи запустило ход ведущего");
    let after = t.model.requests.lock().unwrap().iter().filter(|r| r["messages"][0]["content"].as_str().is_some_and(|s| s.contains("Ты — Альфа"))).count();
    assert_eq!(lead_requests, after);

    // В ленте ведущего короткая строка о каждой задаче, и она переживает перезагрузку чата.
    let thread = t.crew.ok("GET", &format!("/api/bots/{}/thread?limit=200", t.alpha), None).await;
    let lines: Vec<&Value> = thread["messages"].as_array().unwrap().iter().filter(|m| m["role"] == "task").collect();
    assert_eq!(lines.len(), 2, "{thread}");
    assert!(lines.iter().all(|l| l["state"] == "done" && l["text"].as_str().unwrap().contains("Итог от")), "{lines:?}");
    let event = t.crew.find_event(is_task_done_for(&t.alpha)).unwrap();
    assert_eq!(event["message"]["role"], "task");

    // Итог читается через task_status и team_status.
    let status = t.run(&t.alpha, &[("task_status", json!({ "taskId": first })), ("task_status", json!({})), ("team_status", json!({}))]).await;
    assert!(status[0].contains("готово") && status[0].contains("Итог от Бета"), "{}", status[0]);
    assert!(status[1].contains(&first) && status[1].contains(&second), "{}", status[1]);
    assert!(status[2].contains("Редакция") && status[2].contains("ведёт: Альфа") && status[2].contains("Бета") && status[2].contains("Итог от Бета"), "{}", status[2]);
    assert!(!status[2].contains("Дельта"), "чужой бот не в списке: {}", status[2]);
}

#[tokio::test(flavor = "multi_thread")]
async fn inside_a_team_anyone_and_outside_only_lead_to_lead() {
    let t = team("async-teammates", 100).await;
    let out = t.run(&t.alpha, &[("assign_task", json!({ "bot": "Дельта", "text": "чужому" })), ("assign_task", json!({ "bot": "Альфа", "text": "себе" }))]).await;
    assert!(out[0].contains("не в твоей команде") && out[0].contains("propose_team"), "{}", out[0]);
    assert!(out[1].contains("не в твоей команде"), "самому себе нельзя: {}", out[1]);
    assert!(t.tasks_file().is_empty(), "задача чужому не создана");

    // Вторая команда: Дельта ведёт, Эпсилон — участник.
    let eps = t.crew.ok("POST", "/api/bots", Some(json!({ "name": "Эпсилон", "role": "вёрстка", "approval": "auto" }))).await["bot"]["id"].as_str().unwrap().to_string();
    t.crew.ok("POST", "/api/rooms", Some(json!({ "name": "Соседи", "botIds": [t.outsider, eps], "lead": t.outsider }))).await;
    // Ведущий — ведущему другой команды можно; ведущий — участнику чужой команды нельзя.
    let out = t.run(&t.alpha, &[("assign_task", json!({ "bot": "Дельта", "text": "коллеге" })), ("assign_task", json!({ "bot": "Эпсилон", "text": "через голову" }))]).await;
    assert!(task_id(&out[0]).starts_with("task_"), "ведущий ведущему: {}", out[0]);
    assert!(out[1].contains("не в твоей команде"), "{}", out[1]);
    // Участник не обращается к ведущему чужой команды, и чужой участник — к нашему ведущему.
    let out = t.run(&t.beta, &[("assign_task", json!({ "bot": "Дельта", "text": "мимо своего" }))]).await;
    assert!(out[0].contains("не в твоей команде"), "{}", out[0]);
    let out = t.run(&eps, &[("assign_task", json!({ "bot": "Альфа", "text": "мимо своего" }))]).await;
    assert!(out[0].contains("не в твоей команде"), "{}", out[0]);

    // Внутри команды участник даёт задачу и соседу, и ведущему (владелец, 16.11.2026).
    let out = t.run(&t.beta, &[("assign_task", json!({ "bot": "Гамма", "text": "соседу" })), ("assign_task", json!({ "bot": "Альфа", "text": "ведущему" }))]).await;
    assert!(task_id(&out[0]).starts_with("task_"), "{}", out[0]);
    assert!(task_id(&out[1]).starts_with("task_"), "{}", out[1]);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_repeat_gets_the_same_id_and_the_fourth_open_task_is_refused() {
    let t = team("async-cap", 3000).await;
    let out = t
        .run(&t.alpha, &[
            ("assign_task", json!({ "bot": "Бета", "text": "первая" })),
            ("assign_task", json!({ "bot": "Бета", "text": "  Первая " })),
            ("assign_task", json!({ "bot": "Бета", "text": "вторая" })),
            ("assign_task", json!({ "bot": "Бета", "text": "третья" })),
            ("assign_task", json!({ "bot": "Бета", "text": "четвёртая" })),
            ("assign_task", json!({ "bot": "Гамма", "text": "четвёртая, но другому" })),
        ])
        .await;
    let first = task_id(&out[0]);
    assert!(out[1].contains("уже идёт") && task_id(&out[1]) == first, "повтор — тот же taskId: {}", out[1]);
    assert!(out[4].contains("уже 3 незакрытые") && out[4].contains("task_status"), "{}", out[4]);
    assert!(task_id(&out[5]).starts_with("task_"), "лимит у каждого участника свой: {}", out[5]);
    assert_eq!(t.tasks_file().len(), 4, "создано четыре: три Бете и одна Гамме");
    // Бета занят одной задачей, остальные ждут.
    common::wait_until("одна идёт, две ждут", 15, || async {
        let f = t.tasks_file();
        let of_beta = |s: &str| f.iter().filter(|x| x["toName"] == "Бета" && x["state"] == s).count();
        of_beta("running") == 1 && of_beta("queued") == 2
    })
    .await;
    // Вторая очередь закрыта: после завершения освобождается место.
    common::wait_until("первая готова", 30, || async { t.task_state(&first).await == "done" }).await;
    let out = t.run(&t.alpha, &[("assign_task", json!({ "bot": "Бета", "text": "четвёртая" }))]).await;
    assert!(task_id(&out[0]).starts_with("task_"), "{}", out[0]);
}

#[tokio::test(flavor = "multi_thread")]
async fn another_bots_task_id_is_not_found_and_a_running_task_resumes_after_a_restart() {
    let mut t = team("async-restart", 12_000).await;
    let out = t.run(&t.alpha, &[("assign_task", json!({ "bot": "Бета", "text": "долго" }))]).await;
    let id = task_id(&out[0]);
    common::wait_until("идёт", 20, || async { t.task_state(&id).await == "running" }).await;
    // Чужой номер не отличим от несуществующего.
    let other = t.run(&t.gamma, &[("task_status", json!({ "taskId": id })), ("task_status", json!({ "taskId": "task_nope" }))]).await;
    assert!(other[0].contains("у тебя нет") && !other[0].contains("долго"), "{}", other[0]);
    assert!(other[1].contains("у тебя нет"));
    let mine = t.run(&t.alpha, &[("task_status", json!({ "taskId": id }))]).await;
    assert!(mine[0].contains("в работе"), "{}", mine[0]);

    // Перезапуск программы посреди задачи.
    let data_dir = t.crew.data_dir.clone();
    t.crew.keep_data();
    let model = t.model.clone();
    let Team { crew, .. } = t;
    drop(crew);
    // Задача старше шести часов: прежний предел возраста не действует, решает журнал запусков.
    let path = data_dir.join("team_tasks.json");
    let mut tasks: Vec<serde_json::Value> = serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
    for task in tasks.iter_mut() {
        task["created"] = json!(task["created"].as_i64().unwrap() - 10 * 3600 * 1000);
    }
    std::fs::write(&path, serde_json::to_vec(&tasks).unwrap()).unwrap();
    let crew = Crew::start_at(data_dir, &[]).await;
    crew.listen().await;
    let state = crew.ok("GET", "/api/state", None).await;
    let alpha = state["bots"].as_array().unwrap().iter().find(|b| b["name"] == "Альфа").unwrap()["id"].as_str().unwrap().to_string();
    // Задача, что шла, встаёт в очередь заново, а не помечается прерванной, как бы давно она ни началась.
    let tasks: Vec<serde_json::Value> = serde_json::from_str(&std::fs::read_to_string(crew.data_dir.join("team_tasks.json")).unwrap()).unwrap();
    let task = tasks.iter().find(|t| t["id"] == id).unwrap_or_else(|| panic!("задачи {id} нет: {tasks:?}"));
    assert_ne!(task["state"], "interrupted", "{task}");
    crew.say(&alpha, &script(&[("task_status", json!({}))])).await;
    crew.wait_event("конец", |e| e["type"] == "turn_end").await;
    let thread = crew.ok("GET", &format!("/api/bots/{alpha}/thread?limit=200"), None).await;
    let result = thread["messages"].as_array().unwrap().iter().flat_map(|m| m["toolCalls"].as_array().cloned().unwrap_or_default()).last().unwrap()["result"].as_str().unwrap().to_string();
    // task_status у ведущего показывает задачу как продолжающуюся, а не оборванную.
    assert!(!result.contains("прервана перезапуском"), "{result}");
    assert!(result.contains("ждёт") || result.contains("в работе") || result.contains("готово") || result.contains("ошибка"), "{result}");
    drop(model);
}

#[tokio::test(flavor = "multi_thread")]
async fn files_are_copied_only_from_the_own_folder_under_the_rules() {
    let t = team("async-files", 100).await;
    let ws = t.workspace(&t.alpha);
    std::fs::create_dir_all(ws.join("real")).unwrap();
    std::fs::write(ws.join("report.txt"), "отчёт").unwrap();
    std::fs::write(ws.join("real").join("inner.txt"), "внутри").unwrap();
    std::fs::write(ws.join(".env"), "KEY=1").unwrap();
    std::fs::write(ws.join("big.bin"), vec![0u8; 21 * 1024 * 1024]).unwrap();
    std::fs::write(t.crew.data_dir.join("outside.txt"), "снаружи").unwrap();
    // Junction внутри папки: ведёт в её же подпапку, но ссылки не передаём.
    let junction = std::process::Command::new("cmd").args(["/C", "mklink", "/J", &ws.join("link").to_string_lossy(), &ws.join("real").to_string_lossy()]).output().unwrap();
    assert!(junction.status.success(), "junction не создан: {}", String::from_utf8_lossy(&junction.stdout));
    let absolute = ws.join("report.txt").to_string_lossy().to_string();
    let outside = t.crew.data_dir.join("outside.txt").to_string_lossy().to_string();
    let out = t
        .run(&t.alpha, &[
            ("share_file", json!({ "bot": "Бета", "path": "report.txt" })),
            ("share_file", json!({ "bot": "Бета", "path": absolute })),
            ("share_file", json!({ "bot": "Бета", "path": ".env" })),
            ("share_file", json!({ "bot": "Бета", "path": "..\\other\\x.txt" })),
            ("share_file", json!({ "bot": "Бета", "path": "real/../../escape.txt" })),
            ("share_file", json!({ "bot": "Бета", "path": outside })),
            ("share_file", json!({ "bot": "Бета", "path": "big.bin" })),
            ("share_file", json!({ "bot": "Бета", "path": "link/inner.txt" })),
            ("share_file", json!({ "bot": "Бета", "path": "real" })),
            ("share_file", json!({ "bot": "Дельта", "path": "report.txt" })),
        ])
        .await;
    assert!(out[0].contains("attachments/") && out[0].contains("report.txt"), "{}", out[0]);
    assert!(out[1].contains("attachments/") && out[1].contains("report-2.txt"), "второй файл с тем же именем не затирает первый: {}", out[1]);
    assert!(out[2].contains("не передаём") || out[2].contains("секрет"), ".env: {}", out[2]);
    assert!(out[3].contains("«..»"), "{}", out[3]);
    assert!(out[4].contains("«..»"), "{}", out[4]);
    assert!(out[5].contains("только файлы из своей рабочей папки"), "{}", out[5]);
    assert!(out[6].contains("больше 20 МБ"), "{}", out[6]);
    assert!(out[7].contains("ссылки"), "junction: {}", out[7]);
    assert!(out[8].contains("не файл"), "{}", out[8]);
    assert!(out[9].contains("не в твоей команде"), "{}", out[9]);

    let attachments = t.workspace(&t.beta).join("attachments");
    // Новая схема: attachments/<дата>/from-<отправитель>/.
    let date_dir = std::fs::read_dir(&attachments).unwrap().map(|e| e.unwrap().path()).find(|p| p.is_dir()).unwrap();
    let from_dir = std::fs::read_dir(&date_dir).unwrap().map(|e| e.unwrap().path()).find(|p| p.is_dir()).unwrap();
    let names: Vec<String> = std::fs::read_dir(&from_dir).unwrap().map(|e| e.unwrap().file_name().to_string_lossy().to_string()).collect();
    assert_eq!(names.len(), 2, "скопированы только два допустимых: {names:?}");
    assert!(names.iter().all(|n| n == "report.txt" || n == "report-2.txt"), "{names:?}");
    for name in &names {
        assert_eq!(std::fs::read_to_string(from_dir.join(name)).unwrap(), "отчёт");
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn assign_task_with_files_copies_them_and_tells_the_assignee_where_they_are() {
    let t = team("async-files-brief", 100).await;
    std::fs::create_dir_all(t.workspace(&t.alpha)).unwrap();
    std::fs::write(t.workspace(&t.alpha).join("brief.md"), "ТЗ").unwrap();
    let out = t.run(&t.alpha, &[("assign_task", json!({ "bot": "Гамма", "text": "Прочитай ТЗ", "files": ["brief.md"] })), ("assign_task", json!({ "bot": "Гамма", "text": "Плохой файл", "files": [".env"] }))]).await;
    let id = task_id(&out[0]);
    assert!(id.starts_with("task_"), "{}", out[0]);
    assert!(out[1].contains("задача не выдана"), "{}", out[1]);
    common::wait_until("готово", 20, || async { t.task_state(&id).await == "done" }).await;
    let brief_seen = t.model.requests.lock().unwrap().iter().any(|r| {
        r["messages"].as_array().is_some_and(|m| m.iter().any(|x| text_of(x).contains("Прочитай ТЗ") && text_of(x).contains("attachments/") && text_of(x).contains("brief.md")))
    });
    assert!(brief_seen, "в брифе получателя нет пути к файлу");
    assert_eq!(t.tasks_file().len(), 1, "задача с плохим файлом не создана");
}

#[tokio::test(flavor = "multi_thread")]
async fn result_files_are_listed_and_secret_names_are_dropped() {
    // Результат — файлы, появившиеся в папке получателя за время задачи; секретные имена в список не попадают.
    let t = team("async-result-files", 1200).await;
    let out = t.run(&t.alpha, &[("assign_task", json!({ "bot": "Бета", "text": "сделай файл" }))]).await;
    let id = task_id(&out[0]);
    common::wait_until("идёт", 20, || async { t.task_state(&id).await == "running" }).await;
    let ws = t.workspace(&t.beta);
    std::fs::create_dir_all(ws.join("out")).unwrap();
    std::fs::write(ws.join("cover.png"), "png").unwrap();
    std::fs::write(ws.join("out").join("post.md"), "пост").unwrap();
    std::fs::write(ws.join(".env"), "SECRET=1").unwrap();
    std::fs::write(ws.join("secrets.json"), "{}").unwrap();
    std::fs::write(ws.join("out").join("secrets.key"), "k").unwrap();
    std::fs::write(ws.join("out").join(".env.local"), "K=2").unwrap();
    common::wait_until("готово", 20, || async { t.task_state(&id).await == "done" }).await;
    let status = t.run(&t.alpha, &[("task_status", json!({ "taskId": id }))]).await;
    assert!(status[0].contains("cover.png"), "файл результата пропал из списка: {}", status[0]);
    assert!(status[0].contains("post.md"), "файл из подпапки пропал: {}", status[0]);
    for secret in [".env", "secrets.json", "secrets.key"] {
        assert!(!status[0].contains(secret), "секретный файл {secret} в результате: {}", status[0]);
    }
    let _ = &t.gamma;
    let _ = &t.outsider;
}

// ---------------------------------------------------------------------------
// Раунд правок после проверки
// ---------------------------------------------------------------------------

fn request_texts(t: &Team, who: &str) -> Vec<String> {
    t.model
        .requests
        .lock()
        .unwrap()
        .iter()
        .filter(|r| r["messages"][0]["content"].as_str().is_some_and(|s| s.contains(&format!("Ты — {who}"))))
        .flat_map(|r| r["messages"].as_array().cloned().unwrap_or_default())
        .map(|m| format!("{}: {}", m["role"].as_str().unwrap_or_default(), text_of(&m)))
        .collect()
}

#[tokio::test(flavor = "multi_thread")]
async fn queued_task_keeps_the_assigners_floor_and_depth() {
    let t = team("async-floor", 4000).await;
    t.crew.ok("PATCH", &format!("/api/bots/{}", t.alpha), Some(json!({ "approval": "ask" }))).await;
    let outside = r"C:\Users\Public\wave2-floor-test.txt";
    let _ = std::fs::remove_file(outside);
    let write = script(&[("write_file", json!({ "path": outside, "content": "x" }))]);
    let out = t
        .run(&t.alpha, &[("assign_task", json!({ "bot": "Бета", "text": "первая, держит занятой" })), ("assign_task", json!({ "bot": "Бета", "text": write }))])
        .await;
    let second = task_id(&out[1]);
    // Ведущий стал смелее, пока вторая задача стояла в очереди: пол осторожности остался прежним.
    t.crew.ok("PATCH", &format!("/api/bots/{}", t.alpha), Some(json!({ "approval": "auto" }))).await;
    let stored = t.tasks_file().into_iter().find(|x| x["id"] == second.as_str()).expect("задача записана");
    assert_eq!(stored["floor"], "ask", "пол осторожности не сохранён при выдаче: {stored}");
    assert_eq!(stored["depth"], 0);
    let card = t.crew.wait_event("карточка у Беты", |e| e["type"] == "approval" && e["botId"] == t.beta.as_str()).await;
    assert!(card.to_string().contains("wave2-floor-test"), "{card}");
    assert!(!std::path::Path::new(outside).exists(), "очередная задача записала файл без вопроса");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_task_given_from_a_delegated_turn_keeps_the_depth_and_the_chain_limit() {
    let t = team("async-depth", 100).await;
    let third = script(&[("assign_task", json!({ "bot": "Бета", "text": "третья" }))]);
    let second = script(&[("assign_task", json!({ "bot": "Альфа", "text": third }))]);
    t.run(&t.alpha, &[("assign_task", json!({ "bot": "Бета", "text": second }))]).await;
    common::wait_until("цепочка отработала", 30, || async { t.tasks_file().len() >= 2 && t.tasks_file().iter().all(|x| x["state"] != "queued" && x["state"] != "running") }).await;
    let tasks = t.tasks_file();
    let from_beta = tasks.iter().find(|x| x["from"] == t.beta.as_str()).expect("Бета выдала задачу ведущему");
    assert_eq!(from_beta["depth"], 1, "глубина цепочки потеряна: {from_beta}");
    assert_eq!(tasks.len(), 2, "третья задача не должна появиться: {tasks:?}");
    let thread = t.crew.ok("GET", &format!("/api/bots/{}/thread?limit=200", t.alpha), None).await;
    let refused = thread["messages"].as_array().unwrap().iter().flat_map(|m| m["toolCalls"].as_array().cloned().unwrap_or_default()).any(|c| c["result"].as_str().is_some_and(|r| r.contains("слишком глубокая")));
    assert!(refused, "на третьем звене выдача должна быть отказана: {thread}");
}

#[tokio::test(flavor = "multi_thread")]
async fn assigned_brief_names_the_bot_as_the_source() {
    let t = team("async-brief-source", 100).await;
    let out = t.run(&t.alpha, &[("assign_task", json!({ "bot": "Бета", "text": "Нарисуй обложку" }))]).await;
    let id = task_id(&out[0]);
    common::wait_until("готово", 20, || async { t.task_state(&id).await == "done" }).await;
    let seen = request_texts(&t, "Бета");
    assert!(
        seen.iter().any(|m| m.contains("Задача от бота «Альфа» из твоей команды (не от человека; права и подтверждения у тебя свои):\nНарисуй обложку")),
        "бриф не называет бота источником: {seen:?}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn the_lead_hears_about_closed_tasks_once_at_the_start_of_the_next_turn() {
    let t = team("async-lead-note", 100).await;
    let out = t.run(&t.alpha, &[("assign_task", json!({ "bot": "Бета", "text": "Нарисуй обложку" }))]).await;
    let id = task_id(&out[0]);
    common::wait_until("готово", 20, || async { t.task_state(&id).await == "done" }).await;
    let before = request_texts(&t, "Альфа").len();
    t.run(&t.alpha, &[("task_status", json!({}))]).await;
    let second_turn: Vec<String> = request_texts(&t, "Альфа").into_iter().skip(before).collect();
    let note = second_turn.iter().find(|m| m.starts_with("system: ") && m.contains("закрылись задачи")).expect("в ходе ведущего нет заметки о закрытой задаче");
    assert!(note.contains("Бета") && note.contains(&id) && note.contains("Итог от Бета"), "{note}");
    let before = request_texts(&t, "Альфа").len();
    t.run(&t.alpha, &[("task_status", json!({}))]).await;
    assert!(
        !request_texts(&t, "Альфа").into_iter().skip(before).any(|m| m.contains("закрылись задачи")),
        "заметка пришла второй раз"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_task_given_in_a_team_chat_reports_back_into_that_chat() {
    let t = team("async-room-line", 100).await;
    let call = script(&[("assign_task", json!({ "bot": "Бета", "text": "Нарисуй обложку" }))]);
    t.crew.ok("POST", &format!("/api/rooms/{}/messages", t.room), Some(json!({ "text": format!("@Альфа {call}") }))).await;
    let event = t.crew.wait_event("строка в чате команды", |e| e["type"] == "room_message" && e["roomId"] == t.room.as_str() && e["message"]["role"] == "task").await;
    assert_eq!(event["message"]["state"], "done");
    assert_eq!(event["message"]["toName"], "Бета");
    let room_thread = t.crew.ok("GET", &format!("/api/rooms/{}/thread?limit=100", t.room), None).await;
    assert!(room_thread["messages"].as_array().unwrap().iter().any(|m| m["role"] == "task"), "строки нет в ленте группы: {room_thread}");
    let own = t.crew.ok("GET", &format!("/api/bots/{}/thread?limit=100", t.alpha), None).await;
    assert!(own["messages"].as_array().unwrap().iter().all(|m| m["role"] != "task"), "строка попала в личный чат ведущего");
    assert_eq!(t.crew.count_events(is_task_done_for(&t.alpha)), 0);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_queued_task_fails_when_the_assignee_left_the_team() {
    let t = team("async-left-team", 3000).await;
    let out = t
        .run(&t.alpha, &[("assign_task", json!({ "bot": "Бета", "text": "первая" })), ("assign_task", json!({ "bot": "Бета", "text": "вторая, в очереди" }))])
        .await;
    let second = task_id(&out[1]);
    t.crew.ok("PATCH", &format!("/api/rooms/{}", t.room), Some(json!({ "botIds": [t.alpha.clone(), t.gamma.clone()] }))).await;
    common::wait_until("вторая закрыта", 20, || async { t.task_state(&second).await == "error" }).await;
    let stored = t.tasks_file().into_iter().find(|x| x["id"] == second.as_str()).unwrap();
    assert!(stored["summary"].as_str().unwrap().contains("не в команде"), "{stored}");
}

#[tokio::test(flavor = "multi_thread")]
async fn team_status_hides_task_texts_of_other_bots_from_members() {
    let t = team("async-status-privacy", 4000).await;
    t.run(&t.alpha, &[("assign_task", json!({ "bot": "Гамма", "text": "секретная задача про выпуск" }))]).await;
    let as_member = t.run(&t.beta, &[("team_status", json!({}))]).await;
    assert!(as_member[0].contains("Гамма") && as_member[0].contains("занят"), "{}", as_member[0]);
    assert!(!as_member[0].contains("секретная"), "участник увидел чужую задачу: {}", as_member[0]);
    let as_lead = t.run(&t.alpha, &[("team_status", json!({}))]).await;
    assert!(as_lead[0].contains("секретная задача"), "ведущий не видит задачу: {}", as_lead[0]);
}

#[tokio::test(flavor = "multi_thread")]
async fn share_file_refuses_planted_executables_and_autorun_names() {
    let t = team("async-share-planted", 100).await;
    let ws = t.workspace(&t.alpha);
    std::fs::create_dir_all(&ws).unwrap();
    for name in ["run.bat", "tool.ps1", "a.js", "x.lnk", "setup.exe", "m.vbs", "GIT~1.txt", "ok.txt"] {
        std::fs::write(ws.join(name), "x").unwrap();
    }
    let names = ["run.bat", "tool.ps1", "a.js", "x.lnk", "setup.exe", "m.vbs", "GIT~1.txt", "ok.txt:evil.exe", "ok.txt"];
    let calls: Vec<(&str, Value)> = names.iter().map(|n| ("share_file", json!({ "bot": "Бета", "path": n }))).collect();
    let out = t.run(&t.alpha, &calls).await;
    for (name, result) in names.iter().zip(&out).take(names.len() - 1) {
        assert!(!result.contains("Файл передан"), "{name} передан: {result}");
    }
    assert!(out[names.len() - 1].contains("Файл передан"), "обычный файл должен передаваться: {}", out[names.len() - 1]);
    let attachments = t.workspace(&t.beta).join("attachments");
    let mut copied: Vec<String> = Vec::new();
    if let Ok(dates) = std::fs::read_dir(&attachments) {
        for d in dates.flatten() {
            if !d.path().is_dir() { continue; }
            if let Ok(froms) = std::fs::read_dir(d.path()) {
                for f in froms.flatten() {
                    if !f.path().is_dir() { continue; }
                    if let Ok(files) = std::fs::read_dir(f.path()) {
                        for x in files.flatten() {
                            copied.push(x.file_name().to_string_lossy().to_string());
                        }
                    }
                }
            }
        }
    }
    assert_eq!(copied.len(), 1, "скопирован только обычный файл: {copied:?}");
    // Файлы той же природы не проходят и вместе с задачей.
    let with_task = t.run(&t.alpha, &[("assign_task", json!({ "bot": "Бета", "text": "возьми", "files": ["run.bat"] }))]).await;
    assert!(with_task[0].contains("задача не выдана"), "{}", with_task[0]);
}

#[tokio::test(flavor = "multi_thread")]
async fn share_file_asks_the_human_unless_the_sender_is_in_free_mode() {
    let t = team("async-share-approval", 100).await;
    let ws = t.workspace(&t.alpha);
    std::fs::create_dir_all(&ws).unwrap();
    std::fs::write(ws.join("note.txt"), "x").unwrap();
    t.crew.ok("PATCH", &format!("/api/bots/{}", t.alpha), Some(json!({ "approval": "ask" }))).await;
    t.crew.say(&t.alpha, &script(&[("share_file", json!({ "bot": "Бета", "path": "note.txt" }))])).await;
    let card = t.crew.wait_event("карточка передачи файла", |e| e["type"] == "approval" && e["botId"] == t.alpha.as_str()).await;
    assert!(card.to_string().contains("share_file") || card.to_string().contains("note.txt"), "{card}");
    assert!(!t.workspace(&t.beta).join("attachments").exists(), "файл скопирован до ответа человека");
}
