//! Командная строка: делает работу без окна и без браузера.
//!
//! Говорит с уже запущенной копией по её паспорту (`data/runtime.json`), поэтому не
//! открывает вторую папку данных и не мешает работе ботов. Всё, что печатается с `--json`,
//! пригодно для скриптов; коды выхода разные, чтобы их можно было различать в CI.

use std::path::{Path, PathBuf};
use std::time::SystemTime;

use serde_json::{json, Value};

use crate::agent_client::{self, find, Answer, Client, Target};

/// Коды выхода: скрипт должен отличать «не запущено» от «не получилось».
pub const OK: i32 = 0;
pub const FAILED: i32 = 1;
pub const USAGE: i32 = 2;
pub const NOT_RUNNING: i32 = 3;
pub const TIMEOUT: i32 = 4;

/// Флаги, после которых идёт значение, а не текст задачи.
const VALUE_FLAGS: &[&str] = &["--bot", "--bot-id", "--room", "--export", "--timeout", "--out", "--wait"];
const NUMBER_FLAGS: &[&str] = &["--timeout", "--wait"];
const PLAIN_FLAGS: &[&str] = &["--json", "--no-wait"];
const MAX_EXPORT_DEPTH: u32 = 16;

const HELP: &str = "\
Deloist из командной строки. Приложение должно быть запущено.

  deloist bots [--json]                       кто в команде
  deloist rooms [--json]                      какие есть группы
  deloist run \"задача\" [--bot ИМЯ] [--room ИМЯ] [--export ПАПКА] [--timeout СЕК] [--json]
  deloist wait --bot ИМЯ [--timeout СЕК]
  deloist team export [--out ФАЙЛ]
  deloist team apply ФАЙЛ [--wait СЕК] [--no-wait] [--json]
                                              попросить добавить команду из файла: решает человек в окне Deloist
                                              (по умолчанию ждёт 300 с; --no-wait печатает номер заявки и выходит);
                                              повторный запуск добавит команду ещё раз

Коды выхода: 0 — получилось, 1 — ошибка, 2 — неверные аргументы,
             3 — приложение не запущено, 4 — не дождались за отведённое время.
";

pub enum Command {
    Bots { json: bool },
    Rooms { json: bool },
    Run { text: String, bot: Option<String>, room: Option<String>, export: Option<PathBuf>, timeout: u64, json: bool },
    Wait { bot: String, timeout: u64 },
    TeamExport { out: Option<PathBuf> },
    TeamApply { file: PathBuf, json: bool, wait: u64, no_wait: bool },
    Help,
}

struct Args<'a> {
    rest: &'a [String],
}

impl<'a> Args<'a> {
    fn new(rest: &'a [String]) -> Self {
        Self { rest }
    }

    fn flag(&self, name: &str) -> Option<String> {
        self.rest.iter().position(|a| a == name).and_then(|i| self.rest.get(i + 1)).cloned()
    }

    fn number(&self, name: &str, fallback: u64) -> u64 {
        self.flag(name).and_then(|v| v.parse().ok()).unwrap_or(fallback)
    }

    /// Опечатка во флаге или флаг без значения не должны молча отдать задачу случайному боту.
    fn well_formed(&self) -> bool {
        let mut i = 0;
        while i < self.rest.len() {
            let arg = self.rest[i].as_str();
            if VALUE_FLAGS.contains(&arg) {
                let Some(value) = self.rest.get(i + 1) else {
                    return false;
                };
                if value.starts_with("--") || (NUMBER_FLAGS.contains(&arg) && value.parse::<u64>().is_err()) {
                    return false;
                }
                i += 2;
                continue;
            }
            if arg.starts_with("--") && !PLAIN_FLAGS.contains(&arg) {
                return false;
            }
            i += 1;
        }
        true
    }

    fn json(&self) -> bool {
        self.rest.iter().any(|a| a == "--json")
    }

    /// Слова без флагов и без их значений: из них берём текст задачи или имя файла.
    fn words(&self) -> Vec<&'a str> {
        let mut out = Vec::new();
        let mut i = 0;
        while i < self.rest.len() {
            let arg = self.rest[i].as_str();
            if VALUE_FLAGS.contains(&arg) {
                i += 2;
                continue;
            }
            if arg.starts_with("--") {
                i += 1;
                continue;
            }
            out.push(arg);
            i += 1;
        }
        out
    }
}

/// Разобрать аргументы. `None` значит «это не команда»: приложение запускается как обычно.
pub fn parse(args: &[String]) -> Option<Command> {
    let (first, rest) = args.split_first()?;
    let parsed = Args::new(rest);
    let json = parsed.json();

    let command = match first.as_str() {
        "bots" | "list" => Command::Bots { json },
        "rooms" => Command::Rooms { json },
        "run" => {
            // Текст без кавычек — это вся фраза, а не первое её слово.
            let text = parsed.words().join(" ");
            if text.trim().is_empty() {
                Command::Help
            } else {
                Command::Run {
                    text,
                    bot: parsed.flag("--bot"),
                    room: parsed.flag("--room"),
                    export: parsed.flag("--export").map(PathBuf::from),
                    timeout: parsed.number("--timeout", 600),
                    json,
                }
            }
        }
        "wait" => match parsed.flag("--bot") {
            Some(bot) => Command::Wait { bot, timeout: parsed.number("--timeout", 600) },
            None => Command::Help,
        },
        "team" => match rest.first().map(String::as_str) {
            Some("export") => Command::TeamExport { out: parsed.flag("--out").map(PathBuf::from) },
            Some("apply") => match parsed.words().get(1) {
                Some(file) => Command::TeamApply { file: PathBuf::from(file), json, wait: parsed.number("--wait", 300), no_wait: parsed.rest.iter().any(|a| a == "--no-wait") },
                None => Command::Help,
            },
            _ => Command::Help,
        },
        "help" | "--help" | "-h" => Command::Help,
        _ => return None,
    };
    Some(if parsed.well_formed() { command } else { Command::Help })
}

pub fn run(root: &Path, command: Command) -> i32 {
    if matches!(command, Command::Help) {
        print!("{HELP}");
        return USAGE;
    }
    let client = match Client::open(root) {
        Ok(client) => client,
        Err(error) => {
            let (code, message) = error.into();
            return fail(code, &message);
        }
    };
    let result = match command {
        Command::Bots { json } => bots(&client, json),
        Command::Rooms { json } => rooms(&client, json),
        Command::Run { text, bot, room, export, timeout, json } => run_task(&client, text, bot, room, export, timeout, json),
        Command::Wait { bot, timeout } => wait(&client, &bot, timeout),
        Command::TeamExport { out } => team_export(&client, out),
        Command::TeamApply { file, json, wait, no_wait } => team_apply(&client, &file, json, wait, no_wait),
        Command::Help => Ok(()),
    };
    match result {
        Ok(()) => OK,
        Err((code, message)) => fail(code, &message),
    }
}

fn fail(code: i32, message: &str) -> i32 {
    eprintln!("{message}");
    code
}

type Outcome = Result<(), (i32, String)>;

impl From<agent_client::Error> for (i32, String) {
    fn from(error: agent_client::Error) -> Self {
        let code = match &error {
            agent_client::Error::NotRunning => NOT_RUNNING,
            agent_client::Error::Timeout(_) => TIMEOUT,
            agent_client::Error::Failed(_) => FAILED,
        };
        (code, error.to_string())
    }
}

fn bots(client: &Client, json: bool) -> Outcome {
    let state = client.get("/api/state")?;
    let list = state["bots"].as_array().cloned().unwrap_or_default();
    if json {
        let rows: Vec<Value> = list
            .iter()
            .map(|b| json!({ "id": b["id"], "name": b["name"], "role": b["role"], "busy": b["busy"], "queued": b["queued"] }))
            .collect();
        println!("{}", json!({ "bots": rows }));
    } else {
        for bot in &list {
            let mark = if bot["busy"].as_bool().unwrap_or(false) { "*" } else { " " };
            println!("{mark} {:<20} {}", bot["name"].as_str().unwrap_or("?"), bot["role"].as_str().unwrap_or(""));
        }
    }
    Ok(())
}

fn rooms(client: &Client, json: bool) -> Outcome {
    let state = client.get("/api/state")?;
    let list = state["rooms"].as_array().cloned().unwrap_or_default();
    if json {
        println!("{}", json!({ "rooms": list }));
    } else {
        for room in &list {
            println!("{:<20} {}", room["name"].as_str().unwrap_or("?"), room["id"].as_str().unwrap_or(""));
        }
    }
    Ok(())
}

fn run_task(
    client: &Client,
    text: String,
    bot: Option<String>,
    room: Option<String>,
    export: Option<PathBuf>,
    timeout: u64,
    json: bool,
) -> Outcome {
    // Всё, что можно отвергнуть, отвергаем до отправки: потом задача уже в работе.
    if room.is_some() && bot.is_some() {
        return Err((USAGE, "нужно что-то одно: --bot или --room".into()));
    }
    if room.is_some() && export.is_some() {
        return Err((USAGE, "--export работает только для бота: у группы нет своего воркспейса".into()));
    }

    let state = client.get("/api/state")?;
    let bots = state["bots"].as_array().cloned().unwrap_or_default();
    let rooms = state["rooms"].as_array().cloned().unwrap_or_default();
    let picked = match (&room, &bot) {
        (Some(name), _) => find(&rooms, name).map_err(|e| (USAGE, e))?,
        (None, Some(name)) => find(&bots, name).map_err(|e| (USAGE, e))?,
        // Без имени берём первого свободного: скрипт не должен выбирать наугад.
        (None, None) => bots
            .iter()
            .find(|b| !b["busy"].as_bool().unwrap_or(false))
            .ok_or((FAILED, "все боты заняты".to_string()))?,
    };
    let id = picked["id"].as_str().unwrap_or_default();
    let target = if room.is_some() { Target::Room(id) } else { Target::Bot(id) };

    let started = SystemTime::now();
    let Answer { text: answer, failed } = client.ask(target, &text, timeout)?;

    // Файлы забираем и после сбоя: часть работы могла быть сделана.
    let mut exported = Vec::new();
    let export_error = export.and_then(|dir| export_new_files(client, id, &dir, started, &mut exported).err());

    if json {
        let ok = !failed && export_error.is_none();
        println!("{}", json!({ "ok": ok, "answer": answer, "exported": exported }));
    } else {
        println!("{}", answer.as_deref().unwrap_or("(бот не ответил)"));
        if !exported.is_empty() {
            eprintln!("сохранено файлов: {}", exported.len());
        }
    }
    if failed {
        return Err((FAILED, "бот не смог выполнить задачу".into()));
    }
    export_error.map_or(Ok(()), Err)
}

fn wait(client: &Client, name: &str, timeout: u64) -> Outcome {
    let state = client.get("/api/state")?;
    let bots = state["bots"].as_array().cloned().unwrap_or_default();
    let found = find(&bots, name).map_err(|e| (USAGE, e))?;
    let id = found["id"].as_str().unwrap_or_default();
    Ok(client.wait_idle(Target::Bot(id), timeout)?)
}

/// Скопировать на диск человека файлы, которые бот изменил за время задачи.
fn export_new_files(client: &Client, bot_id: &str, dir: &Path, since: SystemTime, copied: &mut Vec<String>) -> Outcome {
    let cutoff = since.duration_since(SystemTime::UNIX_EPOCH).map(|d| d.as_millis() as i64).unwrap_or(0);
    walk(client, bot_id, "", dir, cutoff, copied, 0)
}

fn walk(
    client: &Client,
    bot_id: &str,
    rel: &str,
    dir: &Path,
    cutoff: i64,
    copied: &mut Vec<String>,
    depth: u32,
) -> Result<(), (i32, String)> {
    if depth > MAX_EXPORT_DEPTH {
        return Err((FAILED, format!("вложенность глубже {MAX_EXPORT_DEPTH} папок: {rel}")));
    }
    let listing = client.get(&format!("/api/bots/{bot_id}/files?path={}", urlencode(rel)))?;
    for item in listing["items"].as_array().cloned().unwrap_or_default() {
        let path = item["path"].as_str().unwrap_or_default().to_string();
        if item["dir"].as_bool().unwrap_or(false) {
            walk(client, bot_id, &path, dir, cutoff, copied, depth + 1)?;
            continue;
        }
        // Файлы, к которым бот не прикасался, не копируем: экспорт — это результат задачи.
        if item["modified"].as_i64().unwrap_or(0) < cutoff {
            continue;
        }
        // Картинку нельзя пропускать через текст, поэтому файл забираем как есть.
        let bytes = client.get_bytes(&format!("/api/bots/{bot_id}/file/raw?path={}", urlencode(&path)))?;
        let target = dir.join(path.replace('/', std::path::MAIN_SEPARATOR_STR));
        if let Some(parent) = target.parent() {
            std::fs::create_dir_all(parent).map_err(|e| (FAILED, e.to_string()))?;
        }
        std::fs::write(&target, bytes).map_err(|e| (FAILED, e.to_string()))?;
        copied.push(path);
    }
    Ok(())
}

fn urlencode(text: &str) -> String {
    let mut out = String::new();
    for byte in text.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' | b'/' => out.push(byte as char),
            _ => out.push_str(&format!("%{byte:02X}")),
        }
    }
    out
}

fn team_export(client: &Client, out: Option<PathBuf>) -> Outcome {
    let data = client.get("/api/team/export")?;
    let text = serde_json::to_string_pretty(&data["team"]).map_err(|e| (FAILED, e.to_string()))?;
    match out {
        Some(path) => std::fs::write(&path, text).map_err(|e| (FAILED, e.to_string()))?,
        None => println!("{text}"),
    }
    Ok(())
}

/// Ключ программы может прочитать скрипт бота, поэтому напрямую добавить команду им нельзя:
/// командная строка подаёт заявку, а принимает её человек в окне Deloist.
fn team_apply(client: &Client, file: &Path, json: bool, wait: u64, no_wait: bool) -> Outcome {
    let text = std::fs::read_to_string(file).map_err(|e| (USAGE, format!("{}: {e}", file.display())))?;
    let team: Value = serde_json::from_str(&text).map_err(|e| (USAGE, format!("это не JSON: {e}")))?;
    let team = if team.get("team").is_some() { team["team"].clone() } else { team };
    let request = client.post("/api/team/import-request", json!({ "team": team }))?;
    // Ручка видна только нам: по ней читается статус и отзывается заявка. Код — для сверки с карточкой.
    let handle = request["handle"].as_str().ok_or((FAILED, "приложение не вернуло номер заявки".to_string()))?.to_string();
    let code = request["code"].as_str().unwrap_or("?").to_string();
    let summary = &request["summary"];
    let windows = request["windows"].as_u64().unwrap_or(0);
    if windows == 0 {
        let note = "Окно Deloist не открыто: подтвердить некому. Открой окно или веб-интерфейс.";
        if no_wait {
            eprintln!("{note}");
        } else {
            let _ = client.post(&format!("/api/team/import-request/{handle}/cancel"), json!({}));
            return Err((USAGE, note.to_string()));
        }
    }
    if no_wait {
        if json {
            println!("{}", json!({ "status": "pending", "handle": handle, "code": code, "summary": summary }));
        } else {
            println!("заявка ждёт решения в окне Deloist (карточка «Добавить команду из файла»), код {code}");
        }
        eprintln!("не жду ответа: результат читается запросом GET /api/team/import-request/{handle}");
        return Ok(());
    }
    eprintln!("Жду подтверждения в окне Deloist… (карточка «Добавить команду из файла», код {code}: сверь его с карточкой)");
    let started = std::time::Instant::now();
    loop {
        let status = client.get(&format!("/api/team/import-request/{handle}"))?;
        match status["status"].as_str().unwrap_or("") {
            "approved" => {
                let imported = &status["imported"];
                if json {
                    println!("{}", json!({ "status": "approved", "imported": imported }));
                } else {
                    println!(
                        "добавлено: {} ботов, {} навыков, {} групп, {} рутин",
                        imported["bots"], imported["skills"], imported["rooms"], imported["routines"]
                    );
                }
                return Ok(());
            }
            "pending" => {}
            other => {
                let reason = match other {
                    "denied" => "человек отклонил заявку: ничего не добавлено".to_string(),
                    "expired" => "срок заявки вышел, пока её не приняли: ничего не добавлено".to_string(),
                    "cancelled" => "заявка отозвана: ничего не добавлено".to_string(),
                    _ => format!("не получилось добавить команду: {}", status["error"].as_str().unwrap_or("ошибка приложения")),
                };
                if json {
                    println!("{}", json!({ "status": other, "imported": Value::Null }));
                }
                return Err((FAILED, reason));
            }
        }
        if started.elapsed().as_secs() >= wait {
            // Заявка не должна висеть в окне после того, как её никто не ждёт.
            let _ = client.post(&format!("/api/team/import-request/{handle}/cancel"), json!({}));
            if json {
                println!("{}", json!({ "status": "timeout", "imported": Value::Null }));
            }
            return Err((TIMEOUT, format!("человек не ответил за {wait} с: заявка снята, ничего не добавлено")));
        }
        std::thread::sleep(std::time::Duration::from_millis(500));
    }
}
