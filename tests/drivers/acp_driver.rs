//! Драйвер ACP: вместо настоящего агента (Claude Code, Codex…) — заглушка `fake_acp`.

use crate::common;

use std::path::{Path, PathBuf};

use common::{Crew, BIN};
use serde_json::{json, Value};

fn fake() -> PathBuf {
    let fake = Path::new(BIN).parent().unwrap().join("examples").join("fake_acp.exe");
    assert!(fake.exists(), "нет заглушки: {}", fake.display());
    fake
}

struct Setup {
    crew: Crew,
    bot: String,
    log: PathBuf,
}

impl Setup {
    async fn new(name: &str, env: Value, approval: &str) -> Self {
        let crew = Crew::start_with_env(name, &[("OPENAI_API_KEY", "sk-test-secret")]).await;
        let log = crew.data_dir.join("fake-acp.log");
        let mut acp_env = json!({ "FAKE_ACP_LOG": log.display().to_string() });
        if let (Some(target), Some(extra)) = (acp_env.as_object_mut(), env.as_object()) {
            target.extend(extra.clone());
        }
        crew.ok(
            "PUT",
            "/api/settings",
            Some(json!({ "driver": "acp", "acpCommand": format!("\"{}\"", fake().display()), "acpEnv": acp_env })),
        )
        .await;
        crew.listen().await;
        let state = crew.ok("GET", "/api/state", None).await;
        let bot = state["bots"][0]["id"].as_str().unwrap().to_string();
        crew.ok("PATCH", &format!("/api/bots/{bot}"), Some(json!({ "approval": approval }))).await;
        Self { crew, bot, log }
    }

    fn workspace(&self) -> PathBuf {
        self.crew.data_dir.join("workspaces").join(&self.bot)
    }

    /// Сообщение боту и ответ хода. Ход обязан закончиться.
    async fn turn(&self, text: &str) -> (Value, String) {
        self.crew.clear_events();
        self.crew.say(&self.bot, text).await;
        let done = self.crew.wait_event("конец хода", |e| e["type"] == "turn_end").await;
        (done, self.last_answer().await)
    }

    async fn last_answer(&self) -> String {
        let thread = self.crew.ok("GET", &format!("/api/bots/{}/thread?limit=10", self.bot), None).await;
        let answer = thread["messages"].as_array().unwrap().iter().rev().find(|m| m["role"] == "assistant").cloned();
        answer.and_then(|a| a["text"].as_str().map(str::to_string)).unwrap_or_default()
    }

    fn methods(&self) -> Vec<Value> {
        std::fs::read_to_string(&self.log)
            .unwrap_or_default()
            .lines()
            .filter_map(|l| serde_json::from_str(l).ok())
            .collect()
    }

    fn count(&self, method: &str) -> usize {
        self.methods().iter().filter(|m| m["method"] == method).count()
    }
}

fn pid_of(answer: &str) -> String {
    answer.split("pid ").nth(1).and_then(|r| r.split_whitespace().next()).unwrap_or_default().to_string()
}

fn session_of(answer: &str) -> String {
    answer.split("сессия ").nth(1).map(str::trim).unwrap_or_default().to_string()
}

#[tokio::test]
async fn unfinished_progress_runs_a_second_prompt_and_stops_on_completion() {
    let s = Setup::new("acp-continuation", json!({}), "ask").await;
    let (done, answer) = s.turn("тест незавершённости: изучи все аккаунты").await;
    assert_eq!(done["failed"], false);
    assert!(answer.contains("Анализ завершён"), "{answer}");
    assert_eq!(s.count("session/prompt"), 2);
}

#[tokio::test]
async fn repeated_unfinished_progress_has_a_fixed_budget() {
    let s = Setup::new("acp-continuation-loop", json!({ "FAKE_ACP_CONTINUE_LOOP": "1" }), "ask").await;
    let (done, _) = s.turn("тест незавершённости: изучи все аккаунты").await;
    assert_eq!(done["failed"], false);
    assert_eq!(s.count("session/prompt"), 5);
    let idle = Setup::new("acp-continuation-idle", json!({ "FAKE_ACP_NO_PROGRESS": "1" }), "ask").await;
    idle.turn("тест незавершённости: изучи все аккаунты").await;
    assert_eq!(idle.count("session/prompt"), 2);
}

#[tokio::test]
async fn unfinished_progress_with_blocker_or_denied_permission_does_not_continue() {
    let blocked = Setup::new("acp-continuation-blocked", json!({ "FAKE_ACP_CONTINUE_BLOCKED": "1" }), "ask").await;
    blocked.turn("тест незавершённости").await;
    assert_eq!(blocked.count("session/prompt"), 1);

    let denied = Setup::new("acp-continuation-denied", json!({ "FAKE_ACP_CONTINUE_DENY": "1" }), "ask").await;
    denied.crew.clear_events();
    denied.crew.say(&denied.bot, "тест незавершённости").await;
    let card = denied.crew.wait_event("карточка", |e| e["type"] == "approval").await;
    denied.crew.ok("POST", &format!("/api/approvals/{}", card["id"].as_str().unwrap()), Some(json!({ "allow": false }))).await;
    denied.crew.wait_event("конец хода", |e| e["type"] == "turn_end").await;
    assert_eq!(denied.count("session/prompt"), 1);
}

#[tokio::test]
async fn the_agent_answers_and_keeps_one_process_and_session() {
    let s = Setup::new("acp-basic", json!({}), "ask").await;
    let (done, first) = s.turn("привет, маяк").await;
    assert_eq!(done["failed"], false, "ход упал: {:?}", s.crew.find_event(|e| e["type"] == "error"));
    assert!(first.contains("эхо: привет, маяк"), "{first}");
    assert!(s.crew.find_event(|e| e["type"] == "text_delta").is_some(), "ответ не шёл потоком");

    let (_, second) = s.turn("ещё раз").await;
    assert!(second.contains("эхо: ещё раз"), "{second}");
    assert_eq!(pid_of(&first), pid_of(&second), "процесс агента не переиспользован");
    assert_eq!(session_of(&first), session_of(&second), "сессия не переиспользована");
    assert_eq!(s.count("initialize"), 1);
    assert_eq!(s.count("session/new"), 1);
    // Без выбора человека агент переведён в режим, где он спрашивает разрешения.
    let mode = s.methods().into_iter().find(|m| m["method"] == "session/set_mode").expect("режим не выбран");
    assert_eq!(mode["params"]["modeId"], "read-only");

    // Первый prompt новой сессии несёт характер бота, а сессия открыта в его папке.
    let methods = s.methods();
    let new = methods.iter().find(|m| m["method"] == "session/new").unwrap();
    let browser = &new["params"]["mcpServers"][0];
    assert!(browser["name"].as_str().unwrap().starts_with("deloist_browser_"), "имя моста со случайной частью: {browser}");
    assert_eq!(browser["args"][0], "--mcp");
    assert_eq!(browser["args"][1], "--browser-bot");
    assert_eq!(browser["args"][2], s.bot);
    let cwd = new["params"]["cwd"].as_str().unwrap().to_lowercase();
    assert!(cwd.ends_with(&s.bot.to_lowercase()), "cwd не воркспейс бота: {cwd}");
    let prompts: Vec<&Value> = methods.iter().filter(|m| m["method"] == "session/prompt").collect();
    assert!(prompts[0]["params"]["prompt"].as_array().unwrap().len() >= 2, "нет вступления в первом prompt");
    assert_eq!(prompts[1]["params"]["prompt"].as_array().unwrap().len(), 1, "вступление повторилось");
}

#[tokio::test]
async fn the_bot_effort_and_agent_options_reach_the_agent() {
    let s = Setup::new("acp-effort", json!({}), "ask").await;
    s.crew
        .ok(
            "PATCH",
            &format!("/api/bots/{}", s.bot),
            Some(json!({ "effort": "high", "agentOptions": { "speed": "fast", "nope": "x" } })),
        )
        .await;
    let (done, _) = s.turn("привет").await;
    assert_eq!(done["failed"], false);
    let sets: Vec<Value> = s.methods().into_iter().filter(|m| m["method"] == "session/set_config_option").collect();
    let sent = |id: &str, value: &str| sets.iter().any(|m| m["params"]["configId"] == id && m["params"]["value"] == value);
    assert!(sent("reasoning_effort", "high"), "усилие не отправлено: {sets:?}");
    assert!(sent("speed", "fast"), "скорость не отправлена: {sets:?}");
    assert!(!sets.iter().any(|m| m["params"]["configId"] == "nope"), "отправлена необъявленная настройка");
}

#[tokio::test]
async fn acp_browser_mcp_uses_the_active_bots_browser() {
    use std::io::{BufRead, BufReader, Write};
    use std::process::{Command, Stdio};

    let s = Setup::new("acp-browser-mcp", json!({}), "auto").await;
    s.crew.say(&s.bot, "медленно").await;
    s.crew.wait_event("агент начал ход", |e| e["type"] == "text_delta").await;
    let data = s.crew.data_dir.clone();
    let bot = s.bot.clone();
    // Ключ хода агент получает в окружении MCP-сервера: путь к файлу, который приложение пишет на время хода.
    let key_file = data.join("browser-turn-keys").join(format!("{bot}.key"));
    assert!(key_file.exists(), "ключ хода не выдан");
    let operation = tokio::task::spawn_blocking(move || {
        let mut child = Command::new(BIN).args(["--mcp", "--browser-bot", &bot])
            .env("CREW_DATA_DIR", data).env("DELOIST_BROWSER_TURN_KEY_FILE", key_file).stdin(Stdio::piped()).stdout(Stdio::piped()).spawn().unwrap();
        let mut input = child.stdin.take().unwrap();
        let mut output = BufReader::new(child.stdout.take().unwrap());
        let mut request = |id, method: &str, params: Value| {
            writeln!(input, "{}", json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params })).unwrap();
            input.flush().unwrap();
            let mut line = String::new();
            output.read_line(&mut line).unwrap();
            serde_json::from_str::<Value>(&line).unwrap()
        };
        let list = request(1, "tools/list", json!({}));
        let call = request(2, "tools/call", json!({ "name": "browser_nav", "arguments": { "action": "tabs" } }));
        let human = request(3, "tools/call", json!({ "name": "browser_human", "arguments": { "message": "Войди в кабинет" } }));
        drop(input);
        child.wait().unwrap();
        (list, call, human)
    });
    let card = s.crew.wait_event("передача браузера человеку",|e| e["type"] == "approval" && e["tool"] == "browser_human").await;
    let id = card["id"].as_str().unwrap();
    s.crew.ok("POST", &format!("/api/approvals/{id}"), Some(json!({ "allow": true }))).await;
    let (list, call, human) = operation.await.unwrap();
    assert!(list["result"]["tools"].as_array().unwrap().iter().any(|t| t["name"] == "browser_human"));
    assert_eq!(call["result"]["isError"], false, "{call}");
    assert_eq!(human["result"]["isError"], false, "{human}");
    assert!(human["result"]["content"][0]["text"].as_str().unwrap().contains("Человек закончил"));
    s.crew.ok("POST", &format!("/api/bots/{}/stop", s.bot), Some(json!({}))).await;
}

#[tokio::test]
async fn a_dead_agent_is_restarted_with_a_new_session_and_a_recap() {
    let s = Setup::new("acp-restart", json!({}), "ask").await;
    let (_, first) = s.turn("запомни слово кит").await;
    let (done, _) = s.turn("умри").await;
    assert_eq!(done["failed"], true);
    let error = s.crew.find_event(|e| e["type"] == "error").unwrap();
    assert!(error["message"].as_str().unwrap().contains("закрылся"), "{error}");

    let (done, third) = s.turn("ты тут?").await;
    assert_eq!(done["failed"], false, "{third}");
    assert_ne!(pid_of(&first), pid_of(&third), "мёртвый процесс не заменён");
    assert_ne!(session_of(&first), session_of(&third));
    let methods = s.methods();
    let last = methods.iter().rev().find(|m| m["method"] == "session/prompt").unwrap();
    let intro = last["params"]["prompt"][0]["text"].as_str().unwrap();
    assert!(intro.contains("запомни слово кит"), "новая сессия не знает разговор: {intro}");
}

#[tokio::test]
async fn a_known_session_is_loaded_when_the_agent_can() {
    let s = Setup::new("acp-load", json!({ "FAKE_ACP_LOAD": "1" }), "ask").await;
    let (_, first) = s.turn("раз").await;
    s.turn("умри").await;
    let (_, third) = s.turn("два").await;
    assert_eq!(s.count("session/load"), 1);
    assert_eq!(session_of(&first), session_of(&third), "сессия не продолжена");
}

/// OpenAI-совместимая заглушка: сводку отдаёт по системному промпту сводчика, обычный ход — «готово».
async fn api_completions(body: axum::body::Bytes) -> axum::response::Response {
    let payload: Value = serde_json::from_slice(&body).unwrap_or_else(|_| json!({}));
    let system: String = payload["messages"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|m| m["role"] == "system")
        .filter_map(|m| m["content"].as_str())
        .collect::<Vec<_>>()
        .join("\n");
    let text = if system.contains("сворачиваешь") { "СВОДКА: решили выпустить маяк." } else { "готово" };
    let stream = format!(
        "data: {}\n\ndata: {}\n\ndata: [DONE]\n\n",
        json!({ "choices": [{ "index": 0, "delta": { "role": "assistant", "content": text } }] }),
        json!({ "choices": [{ "index": 0, "delta": {}, "finish_reason": "stop" }] }),
    );
    axum::response::Response::builder()
        .header("content-type", "text/event-stream")
        .body(axum::body::Body::from(stream))
        .unwrap()
}

#[tokio::test]
async fn a_full_window_starts_a_fresh_session_with_the_summary() {
    let app = axum::Router::new().route("/v1/chat/completions", axum::routing::post(api_completions));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });

    let s = Setup::new("acp-window", json!({ "FAKE_ACP_LOAD": "1" }), "ask").await;
    // Сводку пишет модель по API: два хода и «Сжать историю», как это делает человек.
    s.crew
        .ok(
            "PUT",
            "/api/settings",
            Some(json!({
                "driver": "api", "model": "fake-model",
                "baseUrl": format!("http://127.0.0.1:{port}/v1"), "apiKey": "test-key",
            })),
        )
        .await;
    s.turn("раз").await;
    s.turn("два").await;
    let compacted = s.crew.ok("POST", &format!("/api/bots/{}/compact", s.bot), Some(json!({}))).await;
    assert_eq!(compacted["summarized"], true, "{compacted}");
    // Бот уходит на агента ACP, а модель по API остаётся в настройках: ею пишется сводка.
    s.crew.ok("PATCH", &format!("/api/bots/{}", s.bot), Some(json!({ "driver": "acp" }))).await;

    // Агент говорит, что окно занято на 90%.
    let (_, first) = s.turn("заполни окно").await;
    assert_eq!(s.count("session/new"), 1);
    let (done, second) = s.turn("второй").await;
    assert_eq!(done["failed"], false, "{second}");
    assert_eq!(s.count("session/new"), 2, "заполненную сессию не заменили");
    assert_eq!(s.count("session/load"), 0, "продолжили сессию с полным окном");
    assert_ne!(session_of(&first), session_of(&second), "номер сессии не сменился");
    let prompts: Vec<Value> = s.methods().into_iter().filter(|m| m["method"] == "session/prompt").collect();
    let opening = prompts.last().unwrap()["params"]["prompt"][0]["text"].as_str().unwrap();
    // Сводка — отдельный первый блок, а не строка внутри системного промпта.
    assert_eq!(opening, "# Сводка прошлого разговора\nСВОДКА: решили выпустить маяк.", "первый блок новой сессии — не сводка");
    let compacted = {
        let events = s.crew.events.lock().unwrap();
        events.iter().filter(|e| e["type"] == "compacted" && e["botId"] == s.bot.as_str()).count()
    };
    assert_eq!(compacted, 1, "чат не узнал, что история сжата");

    // Заполненность забыта: следующий ход продолжает новую сессию, а не открывает ещё одну.
    let (_, third) = s.turn("третий").await;
    assert_eq!(s.count("session/new"), 2, "порог сработал повторно");
    assert_eq!(session_of(&third), session_of(&second));
}

#[tokio::test]
async fn permission_goes_to_the_human_card() {
    let s = Setup::new("acp-permission", json!({}), "ask").await;
    for (allow, expected) in [(true, "разрешение: yes"), (false, "разрешение: no")] {
        s.crew.clear_events();
        s.crew.say(&s.bot, "разреши команду").await;
        let card = s.crew.wait_event("карточка", |e| e["type"] == "approval").await;
        assert!(card["detail"].as_str().unwrap().contains("echo hi"), "{card}");
        let id = card["id"].as_str().unwrap();
        s.crew.ok("POST", &format!("/api/approvals/{id}"), Some(json!({ "allow": allow }))).await;
        s.crew.wait_event("конец хода", |e| e["type"] == "turn_end").await;
        let answer = s.last_answer().await;
        assert!(answer.contains(expected), "{answer}");
        // Ход агента виден строкой и сохранён в треде отдельно от наших инструментов.
        let row = s.crew.find_event(|e| e["type"] == "activity").expect("нет строки хода агента");
        assert_eq!(row["call"]["name"], "Запуск команды");
        let thread = s.crew.ok("GET", &format!("/api/bots/{}/thread?limit=4", s.bot), None).await;
        let saved = thread["messages"].as_array().unwrap().iter().rev().find(|m| m["role"] == "assistant").unwrap().clone();
        assert_eq!(saved["activity"][0]["status"], if allow { "ok" } else { "error" }, "{saved}");
        assert!(saved.get("toolCalls").is_none(), "{saved}");
    }
}

#[tokio::test]
async fn reading_project_keys_asks_a_secret_card_even_in_auto_mode() {
    let s = Setup::new("acp-secret", json!({}), "auto").await;
    s.crew.clear_events();
    s.crew.say(&s.bot, "разреши ключи").await;
    let card = s.crew.wait_event("карточка", |e| e["type"] == "approval").await;
    assert_eq!(card["title"], "Бот хочет прочитать файл с ключами: .env на myserver", "{card}");
    assert_eq!(card["scope"]["kind"], "secret", "{card}");
    assert_eq!(card["always"], false, "{card}");
    let id = card["id"].as_str().unwrap();
    s.crew.ok("POST", &format!("/api/approvals/{id}"), Some(json!({ "answer": "deny" }))).await;
    s.crew.wait_event("конец хода", |e| e["type"] == "turn_end").await;
    assert!(s.last_answer().await.contains("разрешение: no"));
}

#[tokio::test]
async fn auto_mode_allows_without_a_card() {
    let s = Setup::new("acp-auto", json!({}), "auto").await;
    let (_, answer) = s.turn("разреши команду").await;
    assert!(answer.contains("разрешение: yes"), "{answer}");
    assert!(s.crew.find_event(|e| e["type"] == "approval").is_none());
}

#[tokio::test]
async fn files_inside_the_workspace_work_and_outside_are_refused() {
    let s = Setup::new("acp-files", json!({}), "auto").await;
    std::fs::create_dir_all(s.workspace()).unwrap();
    std::fs::write(s.workspace().join("note.txt"), "заметка").unwrap();
    let secret = s.crew.data_dir.join("secret.txt");
    std::fs::write(&secret, "тайна").unwrap();

    let (_, answer) = s.turn("прочитай note.txt").await;
    assert!(answer.contains("прочитано: заметка"), "{answer}");
    let absolute = s.workspace().join("note.txt");
    let (_, answer) = s.turn(&format!("прочитай {}", absolute.display())).await;
    assert!(answer.contains("прочитано: заметка"), "абсолютный путь внутри воркспейса: {answer}");

    for path in [secret.display().to_string(), "../../secret.txt".to_string(), "..\\..\\secret.txt".to_string()] {
        let (_, answer) = s.turn(&format!("прочитай {path}")).await;
        assert!(answer.contains("ошибка: путь вне папки бота"), "{path}: {answer}");
        assert!(!answer.contains("тайна"), "{path}: {answer}");
    }

    let (_, answer) = s.turn("запиши sub/out.txt").await;
    assert!(answer.contains("записано"), "{answer}");
    assert_eq!(std::fs::read_to_string(s.workspace().join("sub").join("out.txt")).unwrap(), "привет от агента");

    let outside = s.crew.data_dir.join("pwned.txt");
    let (_, answer) = s.turn(&format!("запиши {}", outside.display())).await;
    assert!(answer.contains("ошибка"), "{answer}");
    assert!(!outside.exists(), "агент записал файл вне воркспейса");
}

#[tokio::test]
async fn strict_mode_asks_before_the_agent_writes_a_file() {
    let s = Setup::new("acp-strict", json!({}), "strict").await;
    s.crew.clear_events();
    s.crew.say(&s.bot, "запиши blocked.txt").await;
    let card = s.crew.wait_event("карточка", |e| e["type"] == "approval").await;
    s.crew.ok("POST", &format!("/api/approvals/{}", card["id"].as_str().unwrap()), Some(json!({ "allow": false }))).await;
    s.crew.wait_event("конец хода", |e| e["type"] == "turn_end").await;
    let answer = s.last_answer().await;
    assert!(answer.contains("ошибка: человек не разрешил"), "{answer}");
    assert!(!s.workspace().join("blocked.txt").exists());
}

#[tokio::test]
async fn stopping_the_turn_cancels_the_agent() {
    let s = Setup::new("acp-cancel", json!({}), "ask").await;
    s.crew.clear_events();
    s.crew.say(&s.bot, "медленно").await;
    s.crew.wait_event("агент начал", |e| e["type"] == "text_delta").await;
    s.crew.ok("POST", &format!("/api/bots/{}/stop", s.bot), None).await;
    let done = s.crew.wait_event("конец хода", |e| e["type"] == "turn_end").await;
    assert_eq!(done["failed"], true);
    assert_eq!(s.count("session/cancel"), 1, "агенту не сказали об отмене");
    assert_eq!(s.count("session/prompt"), 1);
    // Процесс жив и отвечает дальше.
    let (done, answer) = s.turn("снова").await;
    assert_eq!(done["failed"], false, "{answer}");
    assert_eq!(s.count("initialize"), 1);
}

#[tokio::test]
async fn sign_in_is_requested_when_the_agent_needs_it() {
    let s = Setup::new("acp-auth", json!({ "FAKE_ACP_AUTH": "1" }), "ask").await;
    let (done, answer) = s.turn("привет").await;
    assert_eq!(done["failed"], false, "{answer}");
    assert_eq!(s.count("authenticate"), 1);
    let auth = s.methods().into_iter().find(|m| m["method"] == "authenticate").unwrap();
    assert_eq!(auth["params"]["methodId"], "browser-login");
}

#[tokio::test]
async fn the_agent_does_not_see_our_secrets() {
    let s = Setup::new("acp-env", json!({}), "ask").await;
    let (_, answer) = s.turn("секреты").await;
    assert!(answer.contains("секреты: нет"), "{answer}");
}

#[tokio::test]
async fn a_missing_agent_is_reported() {
    let crew = Crew::start("acp-missing").await;
    crew.ok("PUT", "/api/settings", Some(json!({ "driver": "acp", "acpCommand": "crew-no-such-agent --acp" }))).await;
    crew.listen().await;
    let state = crew.ok("GET", "/api/state", None).await;
    let bot = state["bots"][0]["id"].as_str().unwrap().to_string();
    crew.say(&bot, "привет").await;
    let done = crew.wait_event("конец хода", |e| e["type"] == "turn_end").await;
    assert_eq!(done["failed"], true);
    let error = crew.find_event(|e| e["type"] == "error").unwrap();
    let message = error["message"].as_str().unwrap();
    assert!(message.contains("crew-no-such-agent"), "{message}");
}

#[tokio::test]
async fn the_agent_hands_a_task_to_another_bot_through_its_team_tools() {
    let s = Setup::new("acp-team", json!({}), "strict").await;
    let other = s.crew.ok("POST", "/api/bots", Some(json!({ "name": "Второй", "role": "р", "approval": "auto" }))).await["bot"]["id"].as_str().unwrap().to_string();

    // Человек упомянул бота: карточки нет, задача доходит, ответ возвращается агенту.
    s.crew.clear_events();
    s.crew.say(&s.bot, "передай @Второй проверку").await;
    s.crew.wait_event("конец хода", |e| e["type"] == "turn_end" && e["botId"] == s.bot.as_str()).await;
    assert!(s.crew.find_event(|e| e["type"] == "approval").is_none(), "упомянутому боту карточка не нужна");
    let answer = s.last_answer().await;
    // Бриф делегату кончается требованием к итогу (`DELEGATION_TAIL`), заглушка отвечает эхом
    // последней строки. Что задача дошла, проверяет тред второго бота ниже.
    assert!(answer.contains("делегирование: ответ:") && answer.contains("эхо: Верни итог не длиннее 20 строк"), "{answer}");
    let thread = s.crew.ok("GET", &format!("/api/bots/{other}/thread?limit=10"), None).await;
    assert!(thread.to_string().contains("задача от агента"), "второй бот не получил задачу: {thread}");

    // Без упоминания карточки тоже нет: передача сообщения другому боту ничего не разрешает, получатель работает по своим правилам.
    s.crew.clear_events();
    s.crew.say(&s.bot, "передай Второй ещё").await;
    s.crew.wait_event("конец хода", |e| e["type"] == "turn_end" && e["botId"] == s.bot.as_str()).await;
    assert!(s.crew.find_event(|e| e["type"] == "approval").is_none(), "на передачу карточки нет");
    let answer = s.last_answer().await;
    assert!(answer.contains("делегирование: ответ:"), "{answer}");
}

#[tokio::test]
async fn the_agent_asks_the_human_with_options_through_the_bridge() {
    let s = Setup::new("acp-ask", json!({}), "strict").await;
    s.crew.clear_events();
    s.crew.say(&s.bot, "спроси Какой отчёт? | Продажи, Остатки").await;
    let card = s.crew.wait_event("карточка вопроса", |e| e["type"] == "question").await;
    assert_eq!(card["botId"], s.bot.as_str());
    assert_eq!(card["choices"], json!(["Продажи", "Остатки"]), "{card}");
    assert!(s.crew.find_event(|e| e["type"] == "approval").is_none(), "сам вопрос карточки согласия не требует");
    s.crew.ok("POST", &format!("/api/questions/{}", card["id"].as_str().unwrap()), Some(json!({ "answer": "Остатки" }))).await;
    s.crew.wait_event("конец хода", |e| e["type"] == "turn_end" && e["botId"] == s.bot.as_str()).await;
    let answer = s.last_answer().await;
    assert!(answer.contains("вопрос: ответ: Ответ человека: Остатки"), "{answer}");
}

#[tokio::test]
async fn the_agent_manages_routines_through_the_bridge_under_the_usual_approval() {
    let s = Setup::new("acp-routines", json!({}), "strict").await;

    // Создание рутины спрашивает человека, как у API-бота; после «да» рутина есть и в списке агента, и в приложении.
    s.crew.clear_events();
    s.crew.say(&s.bot, "рутина Цены").await;
    let card = s.crew.wait_event("карточка", |e| e["type"] == "approval").await;
    assert_eq!(card["tool"], "create_routine");
    s.crew.ok("POST", &format!("/api/approvals/{}", card["id"].as_str().unwrap()), Some(json!({ "allow": true }))).await;
    s.crew.wait_event("конец хода", |e| e["type"] == "turn_end" && e["botId"] == s.bot.as_str()).await;
    let answer = s.last_answer().await;
    assert!(answer.contains("создание ответ: Рутина создана") && answer.contains("список ответ:") && answer.contains("Цены"), "{answer}");
    let state = s.crew.ok("GET", "/api/state", None).await;
    let routines = state["routines"].as_array().unwrap();
    assert_eq!(routines.len(), 1, "{state}");
    assert_eq!(routines[0]["title"], "Цены");
    assert_eq!(routines[0]["everyMinutes"], 30);
    assert_eq!(routines[0]["botId"], s.bot.as_str());

    // Отказ человека доходит до агента ошибкой и рутину не создаёт.
    s.crew.clear_events();
    s.crew.say(&s.bot, "рутина Лишняя").await;
    let card = s.crew.wait_event("карточка", |e| e["type"] == "approval").await;
    s.crew.ok("POST", &format!("/api/approvals/{}", card["id"].as_str().unwrap()), Some(json!({ "allow": false }))).await;
    s.crew.wait_event("конец хода", |e| e["type"] == "turn_end" && e["botId"] == s.bot.as_str()).await;
    let answer = s.last_answer().await;
    assert!(answer.contains("создание ошибка:") && answer.contains("не разрешил"), "{answer}");
    let state = s.crew.ok("GET", "/api/state", None).await;
    assert_eq!(state["routines"].as_array().unwrap().len(), 1, "{state}");
}

#[tokio::test]
async fn a_full_window_keeps_the_session_when_nobody_can_write_a_summary() {
    // Только агент, модели по API нет: новая сессия началась бы с чистого листа, поэтому прежняя остаётся.
    let s = Setup::new("acp-window-alone", json!({ "FAKE_ACP_LOAD": "1" }), "ask").await;
    let (_, first) = s.turn("заполни окно").await;
    let (done, second) = s.turn("второй").await;
    assert_eq!(done["failed"], false, "{second}");
    assert_eq!(s.count("session/new"), 1, "сессию заменили без сводки");
    assert_eq!(session_of(&first), session_of(&second), "агент потерял разговор");
}

/// Сообщение в группу и ответ бота в её треде.
async fn room_turn(s: &Setup, room: &str, text: &str) -> String {
    s.crew.clear_events();
    s.crew.ok("POST", &format!("/api/rooms/{room}/messages"), Some(json!({ "text": text }))).await;
    s.crew.wait_event("конец хода группы", |e| e["type"] == "room_turn_end" && e["roomId"] == room).await;
    let thread = s.crew.ok("GET", &format!("/api/rooms/{room}/thread?limit=10"), None).await;
    let answer = thread["messages"].as_array().unwrap().iter().rev().find(|m| m["role"] == "assistant").cloned();
    answer.and_then(|a| a["text"].as_str().map(str::to_string)).unwrap_or_default()
}

async fn new_room(s: &Setup, name: &str, instructions: &str) -> String {
    let room = s
        .crew
        .ok("POST", "/api/rooms", Some(json!({ "name": name, "instructions": instructions, "botIds": [s.bot], "lead": s.bot, "mode": "lead" })))
        .await;
    room["room"]["id"].as_str().unwrap().to_string()
}

/// Весь текст prompt-блоков, который агент получил в этой сессии.
fn prompts_of(s: &Setup, session: &str) -> Vec<String> {
    s.methods()
        .iter()
        .filter(|m| m["method"] == "session/prompt" && m["params"]["sessionId"] == session)
        .map(|m| m["params"]["prompt"].as_array().unwrap().iter().filter_map(|b| b["text"].as_str()).collect::<Vec<_>>().join("\n"))
        .collect()
}

#[tokio::test]
async fn each_team_and_the_personal_chat_get_their_own_agent_session() {
    let s = Setup::new("acp-thread-sessions", json!({}), "ask").await;
    let alpha = new_room(&s, "Альфа", "МЕТКА-АЛЬФА-ПРАВИЛО").await;
    let beta = new_room(&s, "Бета", "МЕТКА-БЕТА-ПРАВИЛО").await;

    let (_, personal) = s.turn("личное: секрет-личный").await;
    let a1 = room_turn(&s, &alpha, "в альфе: секрет-альфа").await;
    let b1 = room_turn(&s, &beta, "в бете: секрет-бета").await;
    let a2 = room_turn(&s, &alpha, "альфа ещё раз").await;
    let (_, personal2) = s.turn("личное ещё раз").await;

    let (sp, sa, sb) = (session_of(&personal), session_of(&a1), session_of(&b1));
    assert!(!sp.is_empty() && !sa.is_empty() && !sb.is_empty(), "{personal} / {a1} / {b1}");
    assert_ne!(sp, sa, "личный чат и команда делят сессию");
    assert_ne!(sp, sb, "личный чат и команда делят сессию");
    assert_ne!(sa, sb, "две команды делят сессию");
    assert_eq!(sa, session_of(&a2), "вторая реплика в той же команде не продолжила её сессию");
    assert_eq!(sp, session_of(&personal2), "личный чат потерял свою сессию");
    // Один процесс агента на бота, три сессии.
    assert_eq!(pid_of(&personal), pid_of(&b1));
    assert_eq!(s.count("initialize"), 1);
    assert_eq!(s.count("session/new"), 3);

    // Каждая сессия получила свой текст и ничего чужого.
    let own = |session: &str| prompts_of(&s, session).join("\n");
    let (tp, ta, tb) = (own(&sp), own(&sa), own(&sb));
    assert!(ta.contains("МЕТКА-АЛЬФА-ПРАВИЛО") && !ta.contains("МЕТКА-БЕТА-ПРАВИЛО"), "система альфы: {ta}");
    assert!(tb.contains("МЕТКА-БЕТА-ПРАВИЛО") && !tb.contains("МЕТКА-АЛЬФА-ПРАВИЛО"), "система беты: {tb}");
    assert!(!tp.contains("МЕТКА-") && !tp.contains("секрет-альфа") && !tp.contains("секрет-бета"), "личная сессия: {tp}");
    assert!(ta.contains("секрет-альфа") && !ta.contains("секрет-бета") && !ta.contains("секрет-личный"), "разговор альфы: {ta}");
    assert!(tb.contains("секрет-бета") && !tb.contains("секрет-альфа") && !tb.contains("секрет-личный"), "разговор беты: {tb}");
    assert!(tp.contains("секрет-личный"), "разговор личного чата: {tp}");
}
