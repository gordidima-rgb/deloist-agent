//! Заглушка агента ACP для тестов: говорит JSON-RPC по stdio и делает то, что написано в сообщении.
//!
//! Последний текстовый блок prompt — команда:
//! - «разреши» — просит разрешение и сообщает, что выбрал человек;
//! - «прочитай <путь>» / «запиши <путь>» — читает или пишет файл через клиента;
//! - «медленно» — ждёт `session/cancel` и заканчивает ход отменой;
//! - «передай <Имя> …» — зовёт инструмент `assign_task` MCP-сервера, запущенного по `mcpServers` сессии;
//! - «рутина <название>» — создаёт рутину и читает список рутин через тот же MCP-сервер;
//! - «спроси <вопрос> | <вариант>, <вариант>» — зовёт `ask_user` MCP-сервера и сообщает ответ человека;
//! - «умри» — падает посреди хода;
//! - «заполни» — сообщает, что окно контекста занято на 90%;
//! - «секреты» — перечисляет переменные окружения, которых у агента быть не должно;
//! - иначе отвечает эхом, своим номером процесса и сессией.
//!
//! `FAKE_ACP_AUTH=1` — без `authenticate` сессию не открывает. `FAKE_ACP_LOAD=1` — умеет `session/load`.
//! `FAKE_ACP_LOG=<файл>` — дописывает туда строку на каждый полученный метод.

use std::io::{BufRead, Write};

use serde_json::{json, Value};

static SERVERS: std::sync::Mutex<Value> = std::sync::Mutex::new(Value::Null);

struct Io {
    lines: std::io::Lines<std::io::StdinLock<'static>>,
    next_id: i64,
}

impl Io {
    fn send(&self, message: Value) {
        let mut out = std::io::stdout().lock();
        writeln!(out, "{message}").unwrap();
        out.flush().unwrap();
    }

    fn read(&mut self) -> Option<Value> {
        loop {
            let line = self.lines.next()?.ok()?;
            if let Ok(value) = serde_json::from_str::<Value>(&line) {
                log(&value);
                return Some(value);
            }
        }
    }

    /// Запрос клиенту и его ответ. Уведомление об отмене по дороге запоминается.
    fn ask(&mut self, method: &str, params: Value, cancelled: &mut bool) -> Value {
        self.next_id += 1;
        let id = self.next_id;
        self.send(json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params }));
        while let Some(message) = self.read() {
            if message["method"] == "session/cancel" {
                *cancelled = true;
                continue;
            }
            if message["id"] == id && message.get("method").is_none() {
                return message;
            }
        }
        std::process::exit(0);
    }
}

fn append_log_line(path: &str, line: &str) {
    #[cfg(windows)]
    {
        use std::os::windows::io::AsRawHandle;
        if let Ok(mut file) = std::fs::OpenOptions::new().create(true).append(true).open(path) {
            let mut overlapped: windows_sys::Win32::System::IO::OVERLAPPED = unsafe { std::mem::zeroed() };
            unsafe {
                windows_sys::Win32::Storage::FileSystem::LockFileEx(
                    file.as_raw_handle() as _,
                    0x00000002, // LOCKFILE_EXCLUSIVE_LOCK
                    0,
                    u32::MAX,
                    u32::MAX,
                    &mut overlapped,
                );
            }
            let _ = writeln!(file, "{line}");
            let _ = file.flush();
            unsafe {
                windows_sys::Win32::Storage::FileSystem::UnlockFileEx(
                    file.as_raw_handle() as _,
                    0,
                    u32::MAX,
                    u32::MAX,
                    &mut overlapped,
                );
            }
        }
    }
    #[cfg(not(windows))]
    {
        if let Ok(mut file) = std::fs::OpenOptions::new().create(true).append(true).open(path) {
            let _ = writeln!(file, "{line}");
        }
    }
}

fn log(value: &Value) {
    let Ok(path) = std::env::var("FAKE_ACP_LOG") else { return };
    let entry = json!({ "pid": std::process::id(), "method": value["method"], "params": value["params"] });
    append_log_line(&path, &entry.to_string());
}

/// Первая строка журнала: с какими аргументами и в какой временной папке запущен процесс. Ключа здесь нет и быть не должно.
fn log_start() {
    let Ok(path) = std::env::var("FAKE_ACP_LOG") else { return };
    let args: Vec<String> = std::env::args().skip(1).collect();
    let entry = json!({
        "pid": std::process::id(), "method": "_started",
        "params": { "args": args, "temp": std::env::var("TEMP").unwrap_or_default(), "wsl": std::env::var("WSL_DISTRO_NAME").unwrap_or_default(), "hasKey": std::env::var("GEMINI_API_KEY").is_ok(),
                    "keyLen": std::env::var("GEMINI_API_KEY").map(|k| k.len()).unwrap_or(0) },
    });
    append_log_line(&path, &entry.to_string());
}

fn write_token(pid: u32) {
    let home = std::env::args().find_map(|a| a.strip_prefix("--gemini-dir=").map(str::to_string));
    let Some(home) = home else { return };
    let dir = std::path::Path::new(&home).join("antigravity-acp");
    let _ = std::fs::create_dir_all(&dir);
    let _ = std::fs::write(dir.join("antigravity-oauth-token"), json!({ "token": { "access_token": "fake" }, "auth_method": "oauth", "id_token_email": format!("user{pid}@example.com") }).to_string());
}

fn chunk(io: &Io, session: &str, text: &str) {
    io.send(json!({
        "jsonrpc": "2.0", "method": "session/update",
        "params": { "sessionId": session, "update": { "sessionUpdate": "agent_message_chunk", "content": { "type": "text", "text": text } } },
    }));
}

fn update(io: &Io, session: &str, update: Value) {
    io.send(json!({ "jsonrpc": "2.0", "method": "session/update", "params": { "sessionId": session, "update": update } }));
}

fn main() {
    let stdin: &'static std::io::Stdin = Box::leak(Box::new(std::io::stdin()));
    let mut io = Io { lines: stdin.lock().lines(), next_id: 1000 };
    let needs_auth = std::env::var("FAKE_ACP_AUTH").is_ok_and(|v| v == "1");
    let can_load = std::env::var("FAKE_ACP_LOAD").is_ok_and(|v| v == "1");
    let mut authed = false;
    let mut sessions = 0;
    let pid = std::process::id();
    // `FAKE_ACP_OAUTH=1` — как Antigravity: вход по ключу идёт первым, вход через Google вторым.
    let auth_methods = if std::env::var("FAKE_ACP_OAUTH").is_ok_and(|v| v == "1") {
        json!([{ "id": "gemini-api-key", "name": "Gemini API key" }, { "id": "oauth-personal", "name": "Sign in with Google" }])
    } else {
        json!([{ "id": "browser-login", "name": "Вход через браузер" }])
    };
    log_start();

    while let Some(message) = io.read() {
        let Some(id) = message.get("id").cloned() else { continue };
        let params = &message["params"];
        if matches!(message["method"].as_str(), Some("session/new" | "session/load")) {
            *SERVERS.lock().unwrap() = params["mcpServers"].clone();
        }
        let result = match message["method"].as_str().unwrap_or_default() {
            "initialize" => json!({
                "protocolVersion": 1,
                "agentCapabilities": { "loadSession": can_load, "promptCapabilities": { "image": false } },
                "agentInfo": { "name": "fake-acp", "version": "0.0.1" },
                "authMethods": auth_methods,
            }),
            "authenticate" => {
                authed = true;
                // Как Antigravity: вход через Google оставляет файл с токеном в своей папке.
                if params["methodId"] == "oauth-personal" {
                    // `FAKE_ACP_SLOW_AUTH=<секунды>` — человек выбирает аккаунт в браузере: слушаем 127.0.0.1,
                    // пишем ссылку входа в stderr и пишем токен только через N секунд.
                    if let Some(seconds) = std::env::var("FAKE_ACP_SLOW_AUTH").ok().and_then(|v| v.parse::<u64>().ok()) {
                        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
                        let port = listener.local_addr().unwrap().port();
                        std::mem::forget(listener);
                        eprintln!("Open this link to sign in: https://accounts.google.com/o/oauth2/auth?state=SECRETLINK123&redirect_uri=http://127.0.0.1:{port}/oauth2callback");
                        log(&json!({ "method": "_signin_wait", "params": { "port": port } }));
                        std::thread::sleep(std::time::Duration::from_secs(seconds));
                    }
                    write_token(pid);
                    log(&json!({ "method": "_token_written", "params": {} }));
                }
                json!({})
            }
            "session/new" => {
                // Test only: hold the separate provider probe while normal bot sessions still work.
                if std::env::current_dir().ok().is_some_and(|p| p.ends_with("_probe"))
                    && std::env::var_os("FAKE_ACP_SLOW_PROBE_FILE").is_some_and(|p| std::path::Path::new(&p).exists()) {
                    std::thread::sleep(std::time::Duration::from_secs(62));
                }
                if needs_auth && !authed {
                    io.send(json!({ "jsonrpc": "2.0", "id": id, "error": { "code": -32000, "message": "Authentication required" } }));
                    continue;
                }
                sessions += 1;
                io.send(json!({
                    "jsonrpc": "2.0", "method": "_auth/status_update",
                    "params": { "authStatus": { "kind": "account", "label": "Fake Plus", "account": { "email": "fake@example.com", "plan": "plus" } } },
                }));
                json!({
                    "sessionId": format!("s-{pid}-{sessions}"),
                    "modes": { "currentModeId": "auto", "availableModes": [{ "id": "auto", "name": "Auto" }, { "id": "read-only", "name": "Read-only" }] },
                    "_commands": "available_commands_update follows",
                    "configOptions": [{
                        "id": "model", "name": "Model", "category": "model", "type": "select", "currentValue": "fake-1",
                        "options": [{ "value": "fake-1", "name": "Fake One" }, { "value": "fake-2", "name": "Fake Two" }],
                    }, {
                        "id": "reasoning_effort", "name": "Effort", "category": "thought_level", "type": "select", "currentValue": "medium",
                        "options": [{ "value": "low", "name": "Low" }, { "value": "medium", "name": "Medium" }, { "value": "high", "name": "High" }],
                    }, {
                        "id": "speed", "name": "Speed", "type": "select", "currentValue": "standard",
                        "options": [{ "value": "standard", "name": "Standard" }, { "value": "fast", "name": "Fast" }],
                    }],
                })
            }
            "session/load" => json!({}),
            "_announce" => json!({}),
            "session/set_mode" => json!({}),
            "logout" => json!({}),
            "session/set_config_option" => json!({ "configOptions": [] }),
            "session/prompt" => {
                let session = params["sessionId"].as_str().unwrap_or_default().to_string();
                let blocks = params["prompt"].as_array().cloned().unwrap_or_default();
                let text = blocks.iter().rev().find_map(|b| b["text"].as_str()).unwrap_or_default().to_string();
                if text.trim() == "/status" {
                    chunk(&io, &session, "**Account:** Fake Plus (fake@example.com)  \n**5h limit:** 86% left (resets 16:01)  \n**Weekly limit:** 57% left (resets Nov 12)\n");
                    io.send(json!({ "jsonrpc": "2.0", "id": id, "result": { "stopReason": "end_turn" } }));
                    continue;
                }
                let stop = prompt(&mut io, &session, &text, pid);
                json!({ "stopReason": stop })
            }
            _ => {
                io.send(json!({ "jsonrpc": "2.0", "id": id, "error": { "code": -32601, "message": "unknown" } }));
                continue;
            }
        };
        let new_session = message["method"] == "session/new";
        io.send(json!({ "jsonrpc": "2.0", "id": id, "result": result.clone() }));
        if new_session {
            if let Some(session) = result["sessionId"].as_str() {
                update(&io, session, json!({ "sessionUpdate": "available_commands_update", "availableCommands": [{ "name": "status", "description": "Show status" }] }));
            }
        }
    }
}

fn prompt(io: &mut Io, session: &str, text: &str, pid: u32) -> &'static str {
    let mut cancelled = false;
    if text.contains("finish_turn") || text.contains("итог хода") || text.starts_with("Продолжи незавершённую часть") {
        let continuing = text.starts_with("Продолжи незавершённую часть");
        let (status, note) = if continuing { ("nothing", "x") } else { ("continue", "") };
        let answer = call_finish_turn(status, note);
        chunk(io, session, &format!("finish_turn: {answer}"));
        return "end_turn";
    }
    if text.contains("тест незавершённости") || text.starts_with("Продолжи незавершённую часть") {
        let continuing = text.starts_with("Продолжи незавершённую часть");
        if !continuing || std::env::var("FAKE_ACP_CONTINUE_LOOP").is_ok() || std::env::var("FAKE_ACP_NO_PROGRESS").is_ok() {
            if !continuing || std::env::var("FAKE_ACP_NO_PROGRESS").is_err() {
                update(io, session, json!({ "sessionUpdate": "tool_call", "toolCallId": "work", "title": "Работа", "kind": "execute", "status": "completed" }));
            }
            if std::env::var("FAKE_ACP_CONTINUE_DENY").is_ok() {
                let _ = io.ask("session/request_permission", json!({ "sessionId": session,
                    "toolCall": { "toolCallId": "denied", "title": "Команда", "kind": "execute" },
                    "options": [{ "optionId": "yes", "kind": "allow_once" }, { "optionId": "no", "kind": "reject_once" }] }), &mut cancelled);
            }
            if std::env::var("FAKE_ACP_CONTINUE_BLOCKED").is_ok() {
                chunk(io, session, "Анализ ещё не завершён. Нет доступа к остальным аккаунтам.");
            } else {
                chunk(io, session, "Анализ ещё не завершён: остальные аккаунты нужно изучить.");
            }
        } else {
            chunk(io, session, "Анализ завершён.");
        }
        return "end_turn";
    }
    if text.contains("умри") {
        std::process::exit(3);
    }
    // Задача из передачи приходит с хвостом «Верни итог…»: настоящую строку берём без него.
    let task = text.lines().rev().find(|l| !l.is_empty() && !l.starts_with("Верни итог")).unwrap_or_default().to_string();
    if task.starts_with("рутина ") || task.starts_with("задача от агента: рутина ") {
        let answer = call_routines(&format!("{}
", task.replacen("задача от агента: ", "", 1)));
        chunk(io, session, &format!("рутины: {answer}"));
        return "end_turn";
    }
    if text.lines().last().unwrap_or_default().starts_with("спроси ") {
        let answer = call_ask_user(text);
        chunk(io, session, &format!("вопрос: {answer}"));
        return "end_turn";
    }
    if text.contains("передай") {
        let answer = call_delegate(&task);
        chunk(io, session, &format!("делегирование: {answer}"));
        return "end_turn";
    }
    if text.contains("покажи-env") {
        chunk(io, session, &format!("env: {}", std::env::var("NoDefaultCurrentDirectoryInExePath").unwrap_or_else(|_| "нет".into())));
        return "end_turn";
    }
    if text.contains("молчание") {
        while let Some(message) = io.read() {
            if message["method"] == "session/cancel" {
                return "cancelled";
            }
        }
        std::process::exit(0);
    }
    if text.contains("медленно") {
        chunk(io, session, "начал");
        while let Some(message) = io.read() {
            if message["method"] == "session/cancel" {
                return "cancelled";
            }
        }
        std::process::exit(0);
    }
    if let Some(raw) = text.lines().find(|l| l.contains("частичное-разрешение ")).and_then(|l| l.split("частичное-разрешение ").nth(1)) {
        let call: Value = serde_json::from_str(raw.trim()).unwrap_or_else(|_| json!({}));
        let id = call["toolCallId"].as_str().unwrap_or("sparse");
        let mut full = call.clone();
        full["sessionUpdate"] = json!("tool_call");
        update(io, session, full);
        let reply = io.ask("session/request_permission", json!({"sessionId":session,"toolCall":{"toolCallId":id,"kind":"execute","rawInput":{"arguments":{}}},"options":[
            {"optionId":"yes","kind":"allow_once"},{"optionId":"no","kind":"reject_once"}]}), &mut cancelled);
        let outcome = &reply["result"]["outcome"];
        let picked = outcome["optionId"].as_str().unwrap_or("cancelled");
        chunk(io, session, &format!("разрешение: {picked}"));
        return if cancelled { "cancelled" } else { "end_turn" };
    }
    // После позднего «да» агент повторяет запомненный вызов, а следом такой же с другим скрытым параметром:
    // «первый» разрешён разово, «второй» должен спрашивать заново.
    if text.contains("Человек разрешил:") {
        let last = std::env::var("FAKE_ACP_LOG").ok().and_then(|p| std::fs::read_to_string(format!("{p}.last")).ok());
        if let Some(call) = last.and_then(|t| serde_json::from_str::<Value>(&t).ok()) {
            let mut other = call.clone();
            other["rawInput"]["env"] = json!({ "A": "другой" });
            let mut picks = Vec::new();
            for call in [call, other] {
                let reply = io.ask(
                    "session/request_permission",
                    json!({ "sessionId": session, "toolCall": call, "options": [
                        { "optionId": "yes", "name": "Разрешить", "kind": "allow_once" },
                        { "optionId": "no", "name": "Запретить", "kind": "reject_once" },
                    ] }),
                    &mut cancelled,
                );
                let outcome = &reply["result"]["outcome"];
                picks.push(if outcome["outcome"] == "selected" { outcome["optionId"].as_str().unwrap_or("?").to_string() } else { "cancelled".into() });
            }
            chunk(io, session, &format!("повтор: первый {}, второй {}", picks[0], picks[1]));
            return "end_turn";
        }
    }
    // «разреши-вызов {json}» — запрос разрешения с произвольным `toolCall` (так проверяются карточки и MCP-вызовы).
    if let Some(raw) = text.lines().find(|l| l.contains("разреши-вызов ")).and_then(|l| l.split("разреши-вызов ").nth(1)) {
        let call: Value = serde_json::from_str(raw.trim()).unwrap_or_else(|_| json!({}));
        if let Ok(path) = std::env::var("FAKE_ACP_LOG") {
            let _ = std::fs::write(format!("{path}.last"), call.to_string());
        }
        let reply = io.ask(
            "session/request_permission",
            json!({
                "sessionId": session,
                "toolCall": call,
                "options": [
                    { "optionId": "yes", "name": "Разрешить", "kind": "allow_once" },
                    { "optionId": "no", "name": "Запретить", "kind": "reject_once" },
                ],
            }),
            &mut cancelled,
        );
        let outcome = &reply["result"]["outcome"];
        let picked = if outcome["outcome"] == "selected" { outcome["optionId"].as_str().unwrap_or("?").to_string() } else { "cancelled".into() };
        chunk(io, session, &format!("разрешение: {picked}"));
        return if cancelled { "cancelled" } else { "end_turn" };
    }
    if text.contains("разреши") {
        // «разреши ключи» — команда, которая читает .env на сервере.
        let command = if text.contains("ключи") { "ssh myserver cat /home/user/proj/.env" } else { "echo hi" };
        update(io, session, json!({
            "sessionUpdate": "tool_call", "toolCallId": "cmd1", "title": "Запуск команды", "kind": "execute",
            "status": "pending", "rawInput": { "command": command },
        }));
        let reply = io.ask(
            "session/request_permission",
            json!({
                "sessionId": session,
                "toolCall": { "toolCallId": "cmd1", "title": "Запуск команды", "kind": "execute", "rawInput": { "command": command } },
                "options": [
                    { "optionId": "yes", "name": "Разрешить", "kind": "allow_once" },
                    { "optionId": "no", "name": "Запретить", "kind": "reject_once" },
                ],
            }),
            &mut cancelled,
        );
        let outcome = &reply["result"]["outcome"];
        let picked = if outcome["outcome"] == "selected" { outcome["optionId"].as_str().unwrap_or("?").to_string() } else { "cancelled".into() };
        let status = if picked == "yes" { "completed" } else { "failed" };
        update(io, session, json!({ "sessionUpdate": "tool_call_update", "toolCallId": "cmd1", "status": status }));
        chunk(io, session, &format!("разрешение: {picked}"));
        return if cancelled { "cancelled" } else { "end_turn" };
    }
    if let Some(path) = text.split("прочитай ").nth(1) {
        let reply = io.ask("fs/read_text_file", json!({ "sessionId": session, "path": path.trim() }), &mut cancelled);
        match reply["result"]["content"].as_str() {
            Some(content) => chunk(io, session, &format!("прочитано: {content}")),
            None => chunk(io, session, &format!("ошибка: {}", reply["error"]["message"].as_str().unwrap_or("?"))),
        }
        return "end_turn";
    }
    if let Some(path) = text.split("запиши ").nth(1) {
        let reply = io.ask(
            "fs/write_text_file",
            json!({ "sessionId": session, "path": path.trim(), "content": "привет от агента" }),
            &mut cancelled,
        );
        if reply.get("error").is_some_and(|e| !e.is_null()) {
            chunk(io, session, &format!("ошибка: {}", reply["error"]["message"].as_str().unwrap_or("?")));
        } else {
            chunk(io, session, "записано");
        }
        return "end_turn";
    }
    if text.contains("секреты") {
        let leaked: Vec<String> = std::env::vars()
            .map(|(k, _)| k)
            .filter(|k| k == "OPENAI_API_KEY" || k.to_uppercase().starts_with("CREW_"))
            .collect();
        chunk(io, session, &format!("секреты: {}", if leaked.is_empty() { "нет".to_string() } else { leaked.join(",") }));
        return "end_turn";
    }
    if text.contains("заполни") {
        update(io, session, json!({ "sessionUpdate": "usage_update", "used": 90, "size": 100 }));
    }
    chunk(io, session, &format!("эхо: {} | pid {pid} | сессия {session}", text.lines().last().unwrap_or_default()));
    "end_turn"
}

/// Запускает MCP-сервер `deloist_browser` из `mcpServers` сессии, делает initialize и отдаёт `body` функцию запроса-ответа.
fn with_mcp(body: impl FnOnce(&mut dyn FnMut(Value) -> Value) -> String) -> String {
    use std::process::{Command, Stdio};
    let servers = SERVERS.lock().unwrap().clone();
    let Some(server) = servers.as_array().and_then(|list| list.iter().find(|s| s["name"].as_str().is_some_and(|n| n.starts_with("deloist_browser")))) else {
        return "нет MCP-сервера".into();
    };
    let mut command = Command::new(server["command"].as_str().unwrap_or_default());
    for arg in server["args"].as_array().into_iter().flatten().filter_map(Value::as_str) {
        command.arg(arg);
    }
    for var in server["env"].as_array().into_iter().flatten() {
        command.env(var["name"].as_str().unwrap_or_default(), var["value"].as_str().unwrap_or_default());
    }
    let Ok(mut child) = command.stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::null()).spawn() else {
        return "MCP-сервер не запустился".into();
    };
    let mut stdin = child.stdin.take().unwrap();
    let mut lines = std::io::BufReader::new(child.stdout.take().unwrap()).lines();
    let mut roundtrip = |request: Value| -> Value {
        writeln!(stdin, "{request}").unwrap();
        stdin.flush().unwrap();
        loop {
            let line = lines.next().and_then(|l| l.ok()).unwrap_or_default();
            if let Ok(reply) = serde_json::from_str::<Value>(&line) {
                return reply;
            }
            if line.is_empty() {
                return Value::Null;
            }
        }
    };
    roundtrip(json!({ "jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {} }));
    let out = body(&mut roundtrip);
    let _ = child.kill();
    out
}

fn tool_names(listed: &Value) -> Vec<String> {
    listed["result"]["tools"].as_array().into_iter().flatten().filter_map(|t| t["name"].as_str().map(str::to_string)).collect()
}

fn reply_text(reply: &Value) -> String {
    let result = &reply["result"];
    format!("{} {}", if result["isError"] == true { "ошибка:" } else { "ответ:" }, result["content"][0]["text"].as_str().unwrap_or_default())
}

/// Зовёт `assign_task`: имя бота — слово после «передай».
fn call_delegate(text: &str) -> String {
    let after = text.lines().last().unwrap_or_default().split_once("передай").map(|x| x.1).unwrap_or_default().to_string();
    let target = after.split_whitespace().next().unwrap_or_default().trim_matches(|c: char| !c.is_alphanumeric()).to_string();
    let rest = after.trim().splitn(2, ' ').nth(1).unwrap_or_default().to_string();
    with_mcp(|ask| {
        let names = tool_names(&ask(json!({ "jsonrpc": "2.0", "id": 2, "method": "tools/list" })));
        if !names.iter().any(|n| n == "assign_task") || !names.iter().any(|n| n == "list_bots") {
            return format!("в списке нет командных инструментов: {names:?}");
        }
        reply_text(&ask(json!({ "jsonrpc": "2.0", "id": 3, "method": "tools/call",
            "params": { "name": "assign_task", "arguments": { "bot": target, "text": format!("задача от агента: {rest}") } } })))
    })
}

/// «спроси <вопрос> | <вариант>, <вариант>» — `ask_user` моста с вариантами; ответ человека идёт в текст.
fn call_ask_user(text: &str) -> String {
    let line = text.lines().last().unwrap_or_default().trim_start_matches("спроси ").to_string();
    let (question, choices) = line.split_once('|').unwrap_or((line.as_str(), ""));
    let choices: Vec<String> = choices.split(',').map(|c| c.trim().to_string()).filter(|c| !c.is_empty()).collect();
    with_mcp(|ask| {
        let names = tool_names(&ask(json!({ "jsonrpc": "2.0", "id": 2, "method": "tools/list" })));
        if !names.iter().any(|n| n == "ask_user") {
            return format!("в списке нет ask_user: {names:?}");
        }
        reply_text(&ask(json!({ "jsonrpc": "2.0", "id": 3, "method": "tools/call",
            "params": { "name": "ask_user", "arguments": { "question": question.trim(), "choices": choices } } })))
    })
}

/// Зовёт `finish_turn` моста.
fn call_finish_turn(status: &str, note: &str) -> String {
    with_mcp(|ask| {
        let names = tool_names(&ask(json!({ "jsonrpc": "2.0", "id": 2, "method": "tools/list" })));
        if !names.iter().any(|n| n == "finish_turn") {
            return format!("в списке нет finish_turn: {names:?}");
        }
        let arguments = if note.is_empty() {
            json!({ "status": status })
        } else {
            json!({ "status": status, "note": note })
        };
        reply_text(&ask(json!({ "jsonrpc": "2.0", "id": 3, "method": "tools/call",
            "params": { "name": "finish_turn", "arguments": arguments } })))
    })
}

/// «рутина <название>» — создаёт рутину раз в 30 минут, потом читает список рутин. Ответы обоих вызовов идут в текст.
fn call_routines(text: &str) -> String {
    let title = text.lines().last().unwrap_or_default().split("рутина").nth(1).unwrap_or_default().trim().to_string();
    with_mcp(|ask| {
        let names = tool_names(&ask(json!({ "jsonrpc": "2.0", "id": 2, "method": "tools/list" })));
        for need in ["list_routines", "create_routine", "update_routine", "delete_routine"] {
            if !names.iter().any(|n| n == need) {
                return format!("в списке нет {need}: {names:?}");
            }
        }
        let made = ask(json!({ "jsonrpc": "2.0", "id": 3, "method": "tools/call",
            "params": { "name": "create_routine", "arguments": { "title": title, "prompt": "проверь цены", "every_minutes": 30 } } }));
        let listed = ask(json!({ "jsonrpc": "2.0", "id": 4, "method": "tools/call", "params": { "name": "list_routines", "arguments": {} } }));
        format!("создание {} | список {}", reply_text(&made), reply_text(&listed))
    })
}
