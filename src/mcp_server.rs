//! MCP-сервер: управление командой из Claude Desktop, Cursor или любого MCP-клиента.
//!
//! Запускается отдельным процессом (`deloist --mcp`), приложение при этом должно работать.
//! Доступ даёт ключ из паспорта копии (`data/runtime.json`) — тот же, что у командной строки.
//! Список намеренно короткий: посмотреть, прочитать тред, отправить задачу, дождаться ответа,
//! вспомнить и запомнить. Подтверждений, удаления, настроек и ключей здесь нет и не будет.

use std::io::{BufRead, Write};
use std::path::Path;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError};
use std::time::{Duration, Instant};

use serde_json::{json, Value};

use crate::agent_client::{find, Answer, Client, Error, Target};

const MAX_LINE: usize = 1024 * 1024;
const MAX_TEXT: usize = 20_000;
const FINISH_ON_CLOSE: Duration = Duration::from_secs(5);
const BROWSER_WAIT: Duration = Duration::from_secs(40);
type BrowserPending = Arc<Mutex<Option<(String, Receiver<Result<String, String>>)>> >;

/// Строка протокола целиком: ответы идут из разных потоков и не должны перемешаться.
fn send(out: &Mutex<std::io::Stdout>, reply: &Value) {
    let mut out = out.lock().unwrap();
    let _ = writeln!(out, "{reply}");
    let _ = out.flush();
}

fn rpc_error(id: Value, code: i64, message: &str) -> Value {
    json!({ "jsonrpc": "2.0", "id": id, "error": { "code": code, "message": message } })
}

pub fn run(root: &Path, browser_bot: Option<String>) -> i32 {
    let stdin = std::io::stdin();
    let out = Arc::new(Mutex::new(std::io::stdout()));
    let in_flight = Arc::new(AtomicUsize::new(0));
    let browser_pending: BrowserPending = Arc::new(Mutex::new(None));
    let mut locked = stdin.lock();

    loop {
        let mut line = String::new();
        match locked.read_line(&mut line) {
            Ok(0) => {
                // Клиент закрыл ввод, но может ещё читать: короткие ответы успеют уйти.
                // Долгую задачу не ждём: она и так идёт в приложении, а процесс не должен висеть.
                let started = Instant::now();
                while in_flight.load(Ordering::SeqCst) > 0 && started.elapsed() < FINISH_ON_CLOSE {
                    std::thread::sleep(Duration::from_millis(20));
                }
                return 0;
            }
            Ok(_) => {}
            Err(e) => {
                eprintln!("[mcp] {e}");
                return 1;
            }
        }
        // Без ответа клиент ждал бы до своего предела: об ошибке говорим сразу.
        if line.len() > MAX_LINE {
            send(&out, &rpc_error(Value::Null, -32600, "запрос слишком большой"));
            continue;
        }
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        let Ok(request) = serde_json::from_str::<Value>(trimmed) else {
            send(&out, &rpc_error(Value::Null, -32700, "это не JSON"));
            continue;
        };
        // Уведомления от клиента ответа не требуют.
        let Some(id) = request.get("id").cloned() else {
            continue;
        };
        let method = request["method"].as_str().unwrap_or_default();
        let params = request.get("params").cloned().unwrap_or(Value::Null);
        let reply = match method {
            "initialize" => json!({
                "jsonrpc": "2.0", "id": id,
                "result": {
                    "protocolVersion": "2024-11-05",
                    "capabilities": { "tools": {} },
                    "serverInfo": { "name": "deloist", "version": env!("CARGO_PKG_VERSION") },
                }
            }),
            "ping" => json!({ "jsonrpc": "2.0", "id": id, "result": {} }),
            "tools/list" => json!({ "jsonrpc": "2.0", "id": id, "result": { "tools": if browser_bot.is_some() { browser_tools() } else { tools() } } }),
            "tools/call" => {
                // Задача идёт минутами: пока она работает, сервер обязан отвечать на остальное.
                let (out, root, in_flight, bot, pending) = (out.clone(), root.to_path_buf(), in_flight.clone(), browser_bot.clone(), browser_pending.clone());
                in_flight.fetch_add(1, Ordering::SeqCst);
                std::thread::spawn(move || {
                    send(&out, &tool_reply(&root, id, &params, bot.as_deref(), &pending));
                    in_flight.fetch_sub(1, Ordering::SeqCst);
                });
                continue;
            }
            _ => rpc_error(id, -32601, "нет такого метода"),
        };
        send(&out, &reply);
    }
}

fn tool_reply(root: &Path, id: Value, params: &Value, browser_bot: Option<&str>, pending: &BrowserPending) -> Value {
    let name = params["name"].as_str().unwrap_or_default();
    let args = params.get("arguments").cloned().unwrap_or(json!({}));
    let result = match browser_bot {
        Some(bot) => browser_request(root, bot, name, &args, pending),
        None => call(root, name, &args),
    };
    let (text, is_error) = match result {
        Ok(text) => (text, false),
        Err(message) => (message, true),
    };
    json!({
        "jsonrpc": "2.0", "id": id,
        "result": { "content": [{ "type": "text", "text": text }], "isError": is_error }
    })
}

fn browser_tools() -> Value {
    let mut defs: Vec<Value> = crate::browser_tools::tool_defs().into_iter().map(|def| {
        let f = &def["function"];
        json!({ "name": f["name"], "description": f["description"], "inputSchema": f["parameters"] })
    }).collect();
    defs.extend(crate::tools::team_tool_defs().into_iter().map(|def| {
        let f = &def["function"];
        json!({ "name": f["name"], "description": f["description"], "inputSchema": f["parameters"] })
    }));
    defs.push(json!({ "name": "browser_wait", "description": "Продолжить ожидание предыдущего действия браузера по job. Вызывай снова, пока действие ожидает человека; других действий в это время не запускай.", "inputSchema": { "type": "object", "properties": { "job": { "type": "string" } }, "required": ["job"] } }));
    Value::Array(defs)
}

fn browser_request(root: &Path, bot: &str, name: &str, args: &Value, pending: &BrowserPending) -> Result<String, String> {
    let mut slot = pending.lock().map_err(|_| "ожидание браузера прервано")?;
    if name == "browser_wait" {
        if !slot.as_ref().is_some_and(|(id, _)| Some(id.as_str()) == args["job"].as_str()) {
            return Err("ожидаемое действие браузера не найдено".into());
        }
    } else {
        if slot.is_some() { return Err("Действие браузера ещё выполняется: продолжай ожидать его через browser_wait".into()); }
        if !crate::tools::is_bridge_tool(name) {
            return Err("неизвестный инструмент".into());
        }
        let (tx, rx) = mpsc::channel();
        let (root, bot, name, args) = (root.to_path_buf(), bot.to_string(), name.to_string(), args.clone());
        let job = crate::guard::random_token();
        std::thread::spawn(move || { let _ = tx.send(call_browser(&root, &bot, &name, &args)); });
        *slot = Some((job, rx));
    }
    wait_browser(&mut slot, BROWSER_WAIT)
}

fn wait_browser(slot: &mut Option<(String, Receiver<Result<String, String>>)>, timeout: Duration) -> Result<String, String> {
    let (job, rx) = slot.as_ref().unwrap();
    match rx.recv_timeout(timeout) {
        Ok(result) => { *slot = None; result }
        Err(RecvTimeoutError::Timeout) => Ok(format!("Действие ещё выполняется, возможно ждёт человека. Вызови browser_wait с job: {job}. Не запускай действие повторно.")),
        Err(RecvTimeoutError::Disconnected) => { *slot = None; Err("соединение с браузером прервано".into()) }
    }
}

#[cfg(test)]
mod browser_wait_tests {
    use super::*;

    #[test]
    fn the_bridge_offers_ask_user_with_the_native_schema() {
        let list = browser_tools();
        let ask = list.as_array().unwrap().iter().find(|t| t["name"] == "ask_user").expect("в мосте нет ask_user");
        let props = &ask["inputSchema"]["properties"];
        assert!(props["question"].is_object() && props["choices"].is_object() && props["form"].is_object(), "{ask}");
        assert_eq!(ask["inputSchema"]["required"], json!(["question"]));
    }

    #[test]
    fn the_bridge_offers_the_async_team_tools_to_acp_agents() {
        let list = browser_tools();
        for name in crate::team_tasks::TOOLS {
            let tool = list.as_array().unwrap().iter().find(|t| t["name"] == name).unwrap_or_else(|| panic!("в мосте нет {name}"));
            assert!(tool["inputSchema"]["properties"].is_object(), "{tool}");
            assert!(crate::tools::is_bridge_tool(name), "{name}");
        }
        let assign = list.as_array().unwrap().iter().find(|t| t["name"] == "assign_task").unwrap();
        assert_eq!(assign["inputSchema"]["required"], json!(["bot", "text"]));
        assert!(assign["inputSchema"]["properties"]["files"].is_object());
    }

    #[test]
    fn the_bridge_offers_the_team_change_tools_and_lets_them_through() {
        let list = browser_tools();
        for name in ["propose_bot_change", "propose_routine"] {
            let tool = list.as_array().unwrap().iter().find(|t| t["name"] == name).unwrap_or_else(|| panic!("в мосте нет {name}"));
            assert!(tool["inputSchema"]["properties"].is_object(), "{tool}");
            assert!(crate::tools::is_bridge_tool(name), "{name} не пропускается мостом");
        }
        let change = list.as_array().unwrap().iter().find(|t| t["name"] == "propose_bot_change").unwrap();
        let props = change["inputSchema"]["properties"].as_object().unwrap();
        for key in ["approval", "browserSites", "driver", "model", "folders", "monthlyTokenLimit"] {
            assert!(!props.contains_key(key), "в схеме есть привилегированное поле {key}");
        }
    }

    #[test]
    fn waiting_preserves_the_same_action_until_it_finishes() {
        let (tx, rx) = mpsc::channel();
        let mut pending = Some(("job-1".to_string(), rx));
        let first = wait_browser(&mut pending, Duration::ZERO).unwrap();
        assert!(first.contains("job-1"));
        assert!(pending.is_some());
        tx.send(Ok("готово".into())).unwrap();
        assert_eq!(wait_browser(&mut pending, Duration::ZERO).unwrap(), "готово");
        assert!(pending.is_none());
    }
}

fn call_browser(root: &Path, bot: &str, name: &str, args: &Value) -> Result<String, String> {
    if !crate::tools::is_bridge_tool(name) {
        return Err("неизвестный инструмент".into());
    }
    // Человек может входить в кабинет до 15 минут.
    let client = Client::open_with_timeout(root, Duration::from_secs(20 * 60))?;
    // Ключ хода лежит в файле, путь к нему передан через окружение: агент живёт дольше одного хода,
    // а ключ у каждого хода свой, поэтому читаем его заново при каждом вызове.
    let key_file = std::env::var_os(crate::drivers::acp::TURN_KEY_ENV).ok_or("нет ключа хода: браузер доступен только агенту во время хода")?;
    let key = std::fs::read_to_string(key_file).map_err(|_| "нет ключа хода: ход закончился")?;
    let result = client.post_with(&format!("/api/bots/{bot}/browser-tool"), json!({ "name": name, "args": args }), &[("x-deloist-turn-token", key.trim())])?;
    if result["status"] == "ok" {
        Ok(result["result"].as_str().unwrap_or_default().to_string())
    } else {
        Err(result["result"].as_str().unwrap_or("ошибка браузера").to_string())
    }
}

fn tools() -> Value {
    let object = |properties: Value, required: Vec<&str>| {
        let mut schema = json!({ "type": "object", "properties": properties });
        if !required.is_empty() {
            schema["required"] = json!(required);
        }
        schema
    };
    json!([
        {
            "name": "list_bots",
            "description": "Кто в команде Deloist: имя, роль, занят ли.",
            "inputSchema": object(json!({}), vec![]),
        },
        {
            "name": "list_rooms",
            "description": "Группы Deloist: имя, состав, кто отвечает.",
            "inputSchema": object(json!({}), vec![]),
        },
        {
            "name": "read_thread",
            "description": "Последние сообщения личного чата с ботом.",
            "inputSchema": object(json!({
                "bot": { "type": "string", "description": "Имя бота." },
                "limit": { "type": "number", "description": "Сколько сообщений, по умолчанию 20." },
            }), vec!["bot"]),
        },
        {
            "name": "send_message",
            "description": "Дать боту задачу и дождаться ответа. Пока бот работает, вызов не возвращается.",
            "inputSchema": object(json!({
                "bot": { "type": "string", "description": "Имя бота." },
                "text": { "type": "string", "description": "Что сделать. Полная постановка." },
                "timeout": { "type": "number", "description": "Сколько секунд ждать ответ, по умолчанию 300." },
            }), vec!["bot", "text"]),
        },
        {
            "name": "send_to_room",
            "description": "Написать в группу и дождаться ответа того, кто в ней отвечает.",
            "inputSchema": object(json!({
                "room": { "type": "string", "description": "Имя группы." },
                "text": { "type": "string", "description": "Что сделать." },
                "timeout": { "type": "number", "description": "Сколько секунд ждать ответ, по умолчанию 300." },
            }), vec!["room", "text"]),
        },
        {
            "name": "list_routines",
            "description": "Расписания ботов: что, когда и включено ли.",
            "inputSchema": object(json!({}), vec![]),
        },
        {
            "name": "recall",
            "description": "Память Deloist о человеке: id — факт (m12) или прошлая задача (e3) целиком; query — поиск по словам;                             scope episodes — среди прошлых задач: что просили, как сделали, чем кончилось. Без аргументов — оглавление.",
            "inputSchema": object(json!({
                "id": { "type": "string", "description": "Номер факта или эпизода." },
                "query": { "type": "string", "description": "Слова для поиска." },
                "scope": { "type": "string", "enum": ["facts", "episodes"], "description": "Где искать, по умолчанию facts." },
            }), vec![]),
        },
        {
            "name": "remember",
            "description": "Записать факт о человеке в общую память Deloist: его узнают все боты. Один факт — одна мысль;                             тот же заголовок правит факт, а не дублирует. Запомненное перестало работать, а ты нашёл, что работает, — исправь этот же факт сразу; ошибку записывай вместе с обходом. Пароли, коды, карты и токены не пишутся.",
            "inputSchema": object(json!({
                "title": { "type": "string", "description": "Суть одной строкой." },
                "body": { "type": "string", "description": "Подробности, если есть." },
                "kind": { "type": "string", "enum": ["user", "feedback", "project", "reference"], "description": "user — о человеке; feedback — как делать и почему; project — дела и решения; reference — ссылки и кабинеты." },
                "entities": { "type": "array", "items": { "type": "string" }, "description": "Люди, компании, проекты, кабинеты." },
                "id": { "type": "string", "description": "Номер факта, чтобы исправить его." },
            }), vec!["title"]),
        },
    ])
}

fn call(root: &Path, name: &str, args: &Value) -> Result<String, String> {
    let client = Client::open(root)?;
    match name {
        "list_bots" => {
            let state = client.get("/api/state")?;
            let mut lines = Vec::new();
            for bot in state["bots"].as_array().cloned().unwrap_or_default() {
                let busy = if bot["busy"].as_bool().unwrap_or(false) { " (занят)" } else { "" };
                lines.push(format!(
                    "{} — {}{}",
                    bot["name"].as_str().unwrap_or("?"),
                    bot["role"].as_str().unwrap_or(""),
                    busy
                ));
            }
            Ok(lines.join("\n"))
        }
        "list_rooms" => {
            let state = client.get("/api/state")?;
            let bots = state["bots"].as_array().cloned().unwrap_or_default();
            let name_of = |id: &str| {
                bots.iter()
                    .find(|b| b["id"] == id)
                    .and_then(|b| b["name"].as_str())
                    .unwrap_or("?")
                    .to_string()
            };
            let mut lines = Vec::new();
            for room in state["rooms"].as_array().cloned().unwrap_or_default() {
                let members: Vec<String> = room["botIds"]
                    .as_array()
                    .cloned()
                    .unwrap_or_default()
                    .iter()
                    .filter_map(Value::as_str)
                    .map(name_of)
                    .collect();
                lines.push(format!(
                    "{} — {}, отвечает {}",
                    room["name"].as_str().unwrap_or("?"),
                    members.join(", "),
                    name_of(room["lead"].as_str().unwrap_or_default())
                ));
            }
            Ok(if lines.is_empty() { "Групп нет.".into() } else { lines.join("\n") })
        }
        "read_thread" => {
            let state = client.get("/api/state")?;
            let bots = state["bots"].as_array().cloned().unwrap_or_default();
            let bot = find(&bots, args["bot"].as_str().unwrap_or_default())?;
            let id = bot["id"].as_str().unwrap_or_default().to_string();
            let limit = args["limit"].as_i64().unwrap_or(20).clamp(1, 200);
            let thread = client.get(&format!("/api/bots/{id}/thread?limit={limit}"))?;
            let mut lines = Vec::new();
            for message in thread["messages"].as_array().cloned().unwrap_or_default() {
                let who = if message["role"] == "user" { "человек" } else { "бот" };
                let text = message["text"].as_str().unwrap_or_default();
                let files: Vec<&str> = message["attachments"]
                    .as_array()
                    .map(|a| a.iter().filter_map(|f| f["name"].as_str()).collect())
                    .unwrap_or_default();
                if text.is_empty() && files.is_empty() {
                    continue;
                }
                let attached = if files.is_empty() { String::new() } else { format!(" [вложения: {}]", files.join(", ")) };
                lines.push(format!("{who}: {}{attached}", clip(text, 1500)));
            }
            Ok(lines.join("\n"))
        }
        "send_message" => {
            let state = client.get("/api/state")?;
            let bots = state["bots"].as_array().cloned().unwrap_or_default();
            let bot = find(&bots, args["bot"].as_str().unwrap_or_default())?;
            let id = bot["id"].as_str().unwrap_or_default().to_string();
            let text = required(args, "text")?;
            answer(client.ask(Target::Bot(&id), &text, timeout_of(args)), "(бот не ответил)")
        }
        "send_to_room" => {
            let state = client.get("/api/state")?;
            let rooms = state["rooms"].as_array().cloned().unwrap_or_default();
            let room = find(&rooms, args["room"].as_str().unwrap_or_default())?;
            let id = room["id"].as_str().unwrap_or_default().to_string();
            let text = required(args, "text")?;
            answer(client.ask(Target::Room(&id), &text, timeout_of(args)), "(никто не ответил)")
        }
        "list_routines" => {
            let state = client.get("/api/state")?;
            let bots = state["bots"].as_array().cloned().unwrap_or_default();
            let mut lines = Vec::new();
            for routine in state["routines"].as_array().cloned().unwrap_or_default() {
                let bot = bots
                    .iter()
                    .find(|b| b["id"] == routine["botId"])
                    .and_then(|b| b["name"].as_str())
                    .unwrap_or("?");
                let when = if routine["hook"].as_bool().unwrap_or(false) {
                    "по вебхуку".to_string()
                } else if routine["at"].as_str().is_some_and(|a| !a.is_empty()) {
                    format!("в {}", routine["at"].as_str().unwrap_or(""))
                } else {
                    format!("каждые {} мин", routine["everyMinutes"])
                };
                let state_text = if routine["enabled"].as_bool().unwrap_or(false) { "" } else { " (выключена)" };
                let next = serde_json::from_value::<crate::store::Routine>(routine.clone())
                    .map(|r| format!(". {}", crate::schedule::describe_next(&r)))
                    .unwrap_or_default();
                lines.push(format!("{} — {}: {}{state_text}{next}", routine["title"].as_str().unwrap_or("?"), bot, when));
            }
            Ok(if lines.is_empty() { "Рутин нет.".into() } else { lines.join("\n") })
        }
        "recall" => {
            let reply = client.post("/api/memory/recall", args.clone())?;
            Ok(reply["text"].as_str().unwrap_or_default().to_string())
        }
        "remember" => {
            required(args, "title")?;
            let reply = client.post("/api/memory/remember", args.clone())?;
            Ok(reply["text"].as_str().unwrap_or_default().to_string())
        }
        other => Err(format!("нет такого инструмента: {other}")),
    }
}

fn required(args: &Value, key: &str) -> Result<String, String> {
    let value = args[key].as_str().unwrap_or_default().trim();
    if value.is_empty() {
        return Err(format!("нужно поле «{key}»"));
    }
    Ok(clip(value, MAX_TEXT))
}

fn clip(text: &str, limit: usize) -> String {
    if text.chars().count() <= limit {
        text.to_string()
    } else {
        format!("{}…", text.chars().take(limit).collect::<String>())
    }
}

/// Схема обещает просто число: 60.0 от клиента — такие же 60 секунд.
fn timeout_of(args: &Value) -> u64 {
    args["timeout"].as_f64().map_or(300, |seconds| seconds as u64).clamp(5, 3600)
}

/// Сбой хода — ошибка инструмента, чтобы клиент не принял его за результат работы.
fn answer(outcome: Result<Answer, Error>, nobody: &str) -> Result<String, String> {
    let reply = outcome?;
    let text = reply.text.map_or_else(|| nobody.to_string(), |t| clip(&t, MAX_TEXT));
    if reply.failed {
        Err(text)
    } else {
        Ok(text)
    }
}
