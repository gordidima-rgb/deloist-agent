//! «Мой Chrome»: хост Native Messaging, канал до программы и бот в браузере человека.
//!
//! Браузерный тест берёт только одноразовый профиль Edge (`--user-data-dir` во временной папке)
//! с расширением из папки данных программы. Настоящий профиль человека и запущенный у него
//! браузер не трогаются. В реестре пишутся только ключи `NativeMessagingHosts\com.deloist.bots`,
//! и после теста их прежние значения возвращаются.
#![cfg(windows)]

mod common;

use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use axum::body::{Body, Bytes};
use axum::extract::State;
use axum::http::header;
use axum::response::{Html, Response};
use axum::routing::{get, post};
use axum::Router;
use common::{Crew, BIN};
use serde_json::{json, Value};

const OUR_ORIGIN: &str = "chrome-extension://aicphofdeimagdepgjfkookblhldmcja/";

// ---------------------------------------------------------------------------
// Кадры Native Messaging
// ---------------------------------------------------------------------------

fn frame(value: &Value) -> Vec<u8> {
    let body = value.to_string().into_bytes();
    let mut out = (body.len() as u32).to_le_bytes().to_vec();
    out.extend(body);
    out
}

fn read_frame(reader: &mut impl Read) -> Option<Value> {
    let mut head = [0u8; 4];
    reader.read_exact(&mut head).ok()?;
    let mut body = vec![0u8; u32::from_le_bytes(head) as usize];
    reader.read_exact(&mut body).ok()?;
    serde_json::from_slice(&body).ok()
}

/// Хост, как его запускает Chrome: origin первым аргументом, кадры по stdin/stdout.
fn host(origin: &str, data_dir: &Path) -> std::process::Child {
    Command::new(std::env::var("CREW_TEST_BIN").unwrap_or_else(|_| BIN.to_string()))
        .arg(origin)
        .arg("--parent-window=0")
        .env("CREW_DATA_DIR", data_dir)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .expect("хост не запустился")
}

fn wait_exit(child: &mut std::process::Child, seconds: u64) -> Option<i32> {
    let deadline = Instant::now() + Duration::from_secs(seconds);
    while Instant::now() < deadline {
        if let Ok(Some(status)) = child.try_wait() {
            return status.code();
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    let _ = child.kill();
    None
}

#[test]
fn the_host_refuses_when_the_app_is_not_running() {
    let dir = common::temp_data_dir("uc-host-off");
    std::fs::create_dir_all(&dir).unwrap();
    let mut child = host(OUR_ORIGIN, &dir);
    let reply = read_frame(child.stdout.as_mut().unwrap()).expect("хост должен ответить кадром");
    assert_eq!(reply["type"], "error", "{reply}");
    assert_eq!(reply["code"], "app_not_running", "{reply}");
    assert!(reply["text"].as_str().unwrap().contains("не запущена"), "{reply}");
    assert_eq!(wait_exit(&mut child, 10), Some(1));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn the_host_refuses_a_foreign_extension() {
    let dir = common::temp_data_dir("uc-host-foreign");
    std::fs::create_dir_all(&dir).unwrap();
    for origin in ["chrome-extension://abcdefghijklmnopabcdefghijklmnop/", "chrome-extension://aicphofdeimagdepgjfkookblhldmcja/x"] {
        let mut child = host(origin, &dir);
        let reply = read_frame(child.stdout.as_mut().unwrap()).expect("хост должен ответить кадром");
        assert_eq!(reply["code"], "origin", "{origin}: {reply}");
        assert_eq!(wait_exit(&mut child, 10), Some(1), "{origin}");
    }
    let _ = std::fs::remove_dir_all(&dir);
}

async fn status(crew: &Crew) -> Value {
    crew.ok("GET", "/api/user-chrome", None).await
}

async fn wait_connected(crew: &Crew, want: bool, seconds: u64) -> Value {
    let deadline = Instant::now() + Duration::from_secs(seconds);
    loop {
        let st = status(crew).await;
        if st["connected"] == want {
            return st;
        }
        assert!(Instant::now() < deadline, "connected не стало {want} за {seconds} с: {st}");
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

async fn wait_waiting(crew: &Crew, want: bool, seconds: u64) -> Value {
    let deadline = Instant::now() + Duration::from_secs(seconds);
    loop {
        let st = status(crew).await;
        if st["waitingForPairing"] == want {
            return st;
        }
        assert!(Instant::now() < deadline, "waitingForPairing не стало {want} за {seconds} с: {st}");
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn the_host_relays_between_the_extension_and_the_app() {
    let crew = Crew::start("uc-relay").await;
    assert_eq!(status(&crew).await["connected"], false);
    let mut child = host(OUR_ORIGIN, &crew.data_dir);
    let mut stdin = child.stdin.take().unwrap();
    let mut stdout = child.stdout.take().unwrap();
    // Расширение здоровается, программа отвечает через хост.
    stdin.write_all(&frame(&json!({ "type": "hello", "salt": "0a".repeat(16), "version": "0.3.1", "browser": "chrome" }))).unwrap();
    stdin.flush().unwrap();
    let reader = std::thread::spawn(move || {
        let mut got = Vec::new();
        while let Some(message) = read_frame(&mut stdout) {
            let done = message["type"] == "status";
            got.push(message);
            if done {
                break;
            }
        }
        got
    });
    let got = tokio::task::spawn_blocking(move || reader.join().unwrap()).await.unwrap();
    // Без пары программа шлёт только hello с солью и status, подписанных кадров нет.
    assert_eq!(got[0]["type"], "hello", "{got:?}");
    assert_eq!(got[0]["salt"].as_str().map(str::len), Some(32), "{got:?}");
    assert!(got.iter().any(|m| m["type"] == "status" && m["paired"] == false), "{got:?}");
    assert!(got.iter().all(|m| m.get("mac").is_none()), "{got:?}");
    // Без пары подключение есть, но «подключено» оно не считается: ждёт пары.
    let st = wait_waiting(&crew, true, 10).await;
    assert_eq!(st["connected"], false, "{st}");
    assert_eq!(st["links"][0]["browser"], "chrome", "{st}");
    // Кадр больше предела от расширения рвёт связь, а не кладёт программу.
    let mut huge = (64u32 * 1024 * 1024).to_le_bytes().to_vec();
    huge.extend_from_slice(b"{}");
    let _ = stdin.write_all(&huge);
    drop(stdin);
    assert_eq!(wait_exit(&mut child, 10), Some(0), "хост выходит, когда Chrome закрыл канал");
    wait_waiting(&crew, false, 10).await;
    crew.ok("GET", "/api/state", None).await;
}

#[cfg(windows)]
#[tokio::test(flavor = "multi_thread")]
async fn the_app_refuses_a_pipe_client_without_the_key() {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let crew = Crew::start("uc-badkey").await;
    let runtime: Value = serde_json::from_str(&std::fs::read_to_string(crew.data_dir.join("runtime.json")).unwrap()).unwrap();
    let name = format!(r"\\.\pipe\deloist-user-chrome-{}", runtime["pid"]);
    let mut pipe = tokio::net::windows::named_pipe::ClientOptions::new().open(&name).expect("канал программы");
    pipe.write_all(&frame(&json!({ "type": "host", "token": "чужой", "origin": OUR_ORIGIN }))).await.unwrap();
    let mut head = [0u8; 4];
    pipe.read_exact(&mut head).await.unwrap();
    let mut body = vec![0u8; u32::from_le_bytes(head) as usize];
    pipe.read_exact(&mut body).await.unwrap();
    let reply: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(reply["code"], "refused", "{reply}");
    assert_eq!(status(&crew).await["connected"], false);
    // С верным ключом, но чужим origin — тоже нет.
    let mut pipe = tokio::net::windows::named_pipe::ClientOptions::new().open(&name).expect("канал программы");
    let hello = json!({ "type": "host", "token": runtime["token"], "origin": "chrome-extension://abcdefghijklmnopabcdefghijklmnop/" });
    pipe.write_all(&frame(&hello)).await.unwrap();
    pipe.read_exact(&mut head).await.unwrap();
    let mut body = vec![0u8; u32::from_le_bytes(head) as usize];
    pipe.read_exact(&mut body).await.unwrap();
    assert_eq!(serde_json::from_slice::<Value>(&body).unwrap()["code"], "refused");
}

#[tokio::test(flavor = "multi_thread")]
async fn bot_settings_need_the_window_and_unattended_is_off_by_default() {
    let crew = Crew::start("uc-settings").await;
    let st = status(&crew).await;
    assert_eq!(st["default"], "own");
    let bot = st["bots"][0]["id"].as_str().unwrap().to_string();
    assert_eq!(st["bots"][0]["unattended"], false);
    let st = crew.ok("PUT", &format!("/api/user-chrome/bots/{bot}"), Some(json!({ "mode": "mine" }))).await;
    assert_eq!(st["bots"][0]["mine"], true);
    assert_eq!(crew.send("PUT", &format!("/api/user-chrome/bots/{bot}"), Some(json!({ "mode": "evil" }))).await.0, 400);
    // Программа по ключу runtime.json (MCP, командная строка) пустить бота в Chrome человека не может.
    let runtime: Value = serde_json::from_str(&std::fs::read_to_string(crew.data_dir.join("runtime.json")).unwrap()).unwrap();
    let res = reqwest::Client::new()
        .put(format!("{}/api/user-chrome/bots/{bot}", crew.base))
        .bearer_auth(runtime["token"].as_str().unwrap())
        .json(&json!({ "unattended": true }))
        .send()
        .await
        .unwrap();
    assert_eq!(res.status().as_u16(), 403, "только человек в окне");
    assert_eq!(status(&crew).await["bots"][0]["unattended"], false);
}

// ---------------------------------------------------------------------------
// Бот в браузере человека: одноразовый профиль Edge с расширением
// ---------------------------------------------------------------------------

const PAGE: &str = r##"<!doctype html><html lang="ru"><head><meta charset="utf-8"><title>Кабинет</title></head><body>
<h1>Кабинет продавца</h1>
<button onclick="document.getElementById('out').textContent='Показано-OK'">Показать</button>
<div id="out"></div>
</body></html>"##;

async fn start_site() -> u16 {
    let app = Router::new().route("/", get(|| async { ([(header::SET_COOKIE, "tok=sekret-cookie-val; Path=/")], Html(PAGE)) }));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    port
}

type Step = Arc<dyn Fn(&str) -> (String, Value) + Send + Sync>;

#[derive(Clone, Default)]
struct Fake(Arc<Mutex<(Vec<Step>, usize, usize)>>);

fn call(name: &str, args: Value) -> Step {
    let name = name.to_string();
    Arc::new(move |_| (name.clone(), args.clone()))
}

fn with(name: &str, build: impl Fn(&str) -> Value + Send + Sync + 'static) -> Step {
    let name = name.to_string();
    Arc::new(move |last| (name.clone(), build(last)))
}

fn ref_of(snapshot: &str, label: &str) -> u64 {
    let quoted = format!("\"{label}\"");
    snapshot
        .lines()
        .filter(|line| line.starts_with('[') && line.contains(&quoted))
        .find_map(|line| line[1..line.find(']')?].parse().ok())
        .unwrap_or(9999)
}

async fn completions(State(fake): State<Fake>, body: Bytes) -> Response {
    let payload: Value = serde_json::from_slice(&body).unwrap();
    let last = payload["messages"]
        .as_array()
        .and_then(|m| m.iter().rev().find(|m| m["role"] == "tool"))
        .and_then(|m| m["content"].as_str())
        .unwrap_or_default()
        .to_string();
    let (step, id) = {
        let mut script = fake.0.lock().unwrap();
        let step = script.0.get(script.1).cloned();
        script.1 += 1;
        script.2 += 1;
        (step, script.2)
    };
    let delta = match step {
        Some(step) => {
            let (name, args) = step(&last);
            json!({ "tool_calls": [{ "index": 0, "id": format!("call_{id}"), "type": "function", "function": { "name": name, "arguments": args.to_string() } }] })
        }
        None => json!({ "content": "готово" }),
    };
    let text = format!("data: {}\n\ndata: [DONE]\n\n", json!({ "choices": [{ "index": 0, "delta": delta }] }));
    Response::builder().header("content-type", "text/event-stream").body(Body::from(text)).unwrap()
}

struct Run {
    updates: Vec<Value>,
    approvals: Vec<Value>,
}

impl Run {
    fn text(&self, i: usize) -> &str {
        self.updates.get(i).and_then(|u| u["result"].as_str()).unwrap_or_else(|| panic!("нет шага {i}: {:?}", self.updates))
    }
    fn status(&self, i: usize) -> &str {
        self.updates[i]["status"].as_str().unwrap_or_default()
    }
    fn ok(&self, i: usize, needle: &str) {
        assert_eq!(self.status(i), "ok", "шаг {i}: {}", self.text(i));
        assert!(self.text(i).contains(needle), "шаг {i}: нет «{needle}» в {}", self.text(i));
    }
    fn err(&self, i: usize, needle: &str) {
        assert_eq!(self.status(i), "error", "шаг {i}: {}", self.text(i));
        assert!(self.text(i).contains(needle), "шаг {i}: нет «{needle}» в {}", self.text(i));
    }
    fn asked(&self, tool: &str) -> usize {
        self.approvals.iter().filter(|a| a["tool"] == tool).count()
    }
}

/// Один ход: человек в чате или рутина без человека. `answer` решает карточки.
async fn run(crew: &Crew, fake: &Fake, bot: &str, steps: Vec<Step>, routine: Option<&str>, answer: impl Fn(&Value) -> &'static str) -> Run {
    {
        let mut script = fake.0.lock().unwrap();
        script.0 = steps;
        script.1 = 0;
    }
    let mut seen = crew.events.lock().unwrap().len();
    match routine {
        Some(id) => {
            crew.ok("POST", &format!("/api/workflows/{id}/run"), Some(json!({ "attended": false }))).await;
        }
        None => crew.say(bot, "поехали").await,
    }
    let started = Instant::now();
    let mut out = Run { updates: Vec::new(), approvals: Vec::new() };
    loop {
        let fresh: Vec<Value> = {
            let events = crew.events.lock().unwrap();
            let slice = events[seen..].to_vec();
            seen = events.len();
            slice
        };
        for event in fresh {
            if event["botId"] != bot {
                continue;
            }
            match event["type"].as_str().unwrap_or_default() {
                "approval" => {
                    let note = json!({ "tool": event["tool"], "title": event["title"], "detail": event["detail"] });
                    let reply = answer(&note);
                    out.approvals.push(note);
                    let id = event["id"].as_str().unwrap();
                    crew.ok("POST", &format!("/api/approvals/{id}"), Some(json!({ "answer": reply }))).await;
                }
                "tool_update" => out.updates.push(event.clone()),
                "turn_end" => return out,
                _ => {}
            }
        }
        assert!(started.elapsed() < Duration::from_secs(120), "ход не закончился: {:?}", out.updates);
        tokio::time::sleep(Duration::from_millis(30)).await;
    }
}

fn edge_exe() -> Option<PathBuf> {
    [std::env::var_os("ProgramFiles(x86)"), std::env::var_os("ProgramFiles")]
        .into_iter()
        .flatten()
        .map(|base| PathBuf::from(base).join(r"Microsoft\Edge\Application\msedge.exe"))
        .find(|path| path.is_file())
}

/// Подключи в HKCU: только ключи хоста Deloist.
const KEYS: [&str; 2] = [r"Software\Google\Chrome\NativeMessagingHosts\com.deloist.bots", r"Software\Microsoft\Edge\NativeMessagingHosts\com.deloist.bots"];

fn wide(text: &str) -> Vec<u16> {
    text.encode_utf16().chain(std::iter::once(0)).collect()
}

fn reg_value(key: &str) -> Option<String> {
    use windows_sys::Win32::System::Registry::{RegGetValueW, HKEY_CURRENT_USER, RRF_RT_REG_SZ};
    let key = wide(key);
    let mut buffer = vec![0u16; 2048];
    let mut size = (buffer.len() * 2) as u32;
    let code = unsafe { RegGetValueW(HKEY_CURRENT_USER, key.as_ptr(), std::ptr::null(), RRF_RT_REG_SZ, std::ptr::null_mut(), buffer.as_mut_ptr().cast(), &mut size) };
    (code == 0).then(|| String::from_utf16_lossy(&buffer[..(size as usize / 2).saturating_sub(1)]))
}

fn reg_restore(key: &str, value: Option<&str>) {
    use windows_sys::Win32::System::Registry::{RegDeleteKeyW, RegSetKeyValueW, HKEY_CURRENT_USER, REG_SZ};
    let subkey = wide(key);
    unsafe {
        match value {
            Some(value) => {
                let data = wide(value);
                RegSetKeyValueW(HKEY_CURRENT_USER, subkey.as_ptr(), std::ptr::null(), REG_SZ, data.as_ptr().cast(), (data.len() * 2) as u32);
            }
            None => {
                RegDeleteKeyW(HKEY_CURRENT_USER, subkey.as_ptr());
            }
        }
    }
}

/// Ключи хоста до теста; после теста — как было (обычно ключей просто не было).
struct RegistryGuard(Vec<(String, Option<String>)>);

impl RegistryGuard {
    fn backup() -> Self {
        Self(KEYS.iter().map(|key| (key.to_string(), reg_value(key))).collect())
    }
}

impl Drop for RegistryGuard {
    fn drop(&mut self) {
        for (key, value) in &self.0 {
            reg_restore(key, value.as_deref());
        }
    }
}

/// Одноразовый профиль браузера: процесс и папка убираются после теста.
struct TestBrowser {
    child: std::process::Child,
    profile: PathBuf,
}

impl Drop for TestBrowser {
    fn drop(&mut self) {
        let _ = self.child.kill();
        // Дочерние процессы Edge с этим профилем — только наши, по пути профиля в командной строке.
        let needle = self.profile.display().to_string().replace('\'', "''");
        let script = format!("Get-CimInstance Win32_Process -Filter \"Name='msedge.exe'\" | Where-Object {{ $_.CommandLine -like '*{needle}*' }} | ForEach-Object {{ Stop-Process -Id $_.ProcessId -Force -ErrorAction SilentlyContinue }}");
        let _ = Command::new("powershell").args(["-NoProfile", "-Command", &script]).output();
        std::thread::sleep(Duration::from_millis(500));
        let _ = std::fs::remove_dir_all(&self.profile);
    }
}


// ---------------------------------------------------------------------------
// Связь расширения без человека: только через порт отладки одноразового профиля теста
// ---------------------------------------------------------------------------

fn free_port() -> u16 {
    std::net::TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port()
}

/// Выполнить выражение в service worker расширения через CDP тестового Edge и вернуть результат.
async fn eval_in_extension(port: u16, expression: &str) -> Value {
    use futures_util::{SinkExt, StreamExt};
    let client = reqwest::Client::new();
    let deadline = Instant::now() + Duration::from_secs(30);
    let ws_url = loop {
        if let Ok(list) = client.get(format!("http://127.0.0.1:{port}/json/list")).send().await {
            if let Ok(targets) = list.json::<Vec<Value>>().await {
                let found = targets.iter().find(|t| t["type"] == "service_worker" && t["url"].as_str().unwrap_or_default().contains(OUR_ORIGIN.trim_start_matches("chrome-extension://").trim_end_matches('/')));
                if let Some(url) = found.and_then(|t| t["webSocketDebuggerUrl"].as_str()) {
                    break url.to_string();
                }
            }
        }
        assert!(Instant::now() < deadline, "service worker расширения не найден на порту отладки {port} за 30 с");
        tokio::time::sleep(Duration::from_millis(250)).await;
    };
    let (mut ws, _) = tokio_tungstenite::connect_async(ws_url).await.expect("подключиться к service worker расширения");
    let request = json!({ "id": 1, "method": "Runtime.evaluate", "params": { "expression": expression, "awaitPromise": true, "returnByValue": true } });
    ws.send(request.to_string().into()).await.unwrap();
    let reply = tokio::time::timeout(Duration::from_secs(30), async {
        while let Some(Ok(message)) = ws.next().await {
            if let tokio_tungstenite::tungstenite::Message::Text(text) = message {
                let value: Value = serde_json::from_str(text.as_str()).unwrap();
                if value["id"] == 1 {
                    return value;
                }
            }
        }
        panic!("service worker закрыл канал отладки без ответа");
    })
    .await
    .expect("service worker не ответил за 30 с");
    assert!(reply["result"]["exceptionDetails"].is_null(), "выражение в расширении упало: {reply}");
    reply["result"]["result"]["value"].clone()
}

/// Связать расширение кодом и впустить в него сайты, как человек в окне расширения (popup.js):
/// тот же KDF, что в protocol.js `deriveK`, те же ключи `pairingKey` и `allowedOrigins`.
async fn pair_extension(port: u16, code: &str, origins: &[String]) {
    let expression = format!(
        r#"(async () => {{
  const enc = new TextEncoder();
  const material = await crypto.subtle.importKey('raw', enc.encode({code}), 'PBKDF2', false, ['deriveBits']);
  const bits = await crypto.subtle.deriveBits({{ name: 'PBKDF2', salt: enc.encode('deloist-user-chrome-v1'), iterations: 200000, hash: 'SHA-256' }}, material, 256);
  const key = [...new Uint8Array(bits)].map((b) => b.toString(16).padStart(2, '0')).join('');
  await chrome.storage.local.set({{ pairingKey: key, allowedOrigins: {origins} }});
  return key.length;
}})()"#,
        code = Value::String(code.to_string()),
        origins = Value::from(origins.to_vec()),
    );
    assert_eq!(eval_in_extension(port, &expression).await, 64, "ключ связи не записан");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_bot_works_in_a_test_profile_of_the_persons_browser() {
    let Some(edge) = edge_exe() else {
        eprintln!("Edge не найден: браузерная проверка «моего Chrome» пропущена");
        return;
    };
    let _registry = RegistryGuard::backup();
    let fake = Fake::default();
    let router = Router::new().route("/v1/chat/completions", post(completions)).with_state(fake.clone());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let model = listener.local_addr().unwrap().port();
    tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
    let site = start_site().await;

    let crew = Crew::start_with_env("uc-e2e", &[("CREW_TEST_ALLOW_LOOPBACK", "1"), ("CREW_TEST_NO_EXPLORER", "1"), ("CREW_TEST_UNATTENDED_WAIT_MS", "300"), ("CREW_USER_CHROME_TRACE", "1")]).await;
    crew.ok("PUT", "/api/settings", Some(json!({ "driver": "api", "baseUrl": format!("http://127.0.0.1:{model}/v1"), "apiKey": "k" }))).await;
    crew.listen().await;
    let st = status(&crew).await;
    let bot = st["bots"][0]["id"].as_str().unwrap().to_string();
    crew.ok("PUT", &format!("/api/user-chrome/bots/{bot}"), Some(json!({ "mode": "mine" }))).await;

    // Без расширения бот честно говорит, что делать.
    let r = run(&crew, &fake, &bot, vec![call("browser_nav", json!({ "action": "open", "url": format!("http://127.0.0.1:{site}/") }))], None, |_| "deny").await;
    r.err(0, "расширение Deloist сейчас не подключено");

    // «Подключить»: манифест хоста, ключи реестра, файлы расширения в папке данных.
    let st = crew.ok("POST", "/api/user-chrome/connect", None).await;
    assert_eq!(st["registered"], true, "{st}");
    let ext = PathBuf::from(st["extensionDir"].as_str().unwrap());
    assert!(ext.join("manifest.json").is_file());
    for key in KEYS {
        let manifest = reg_value(key).expect(key);
        let text: Value = serde_json::from_str(&std::fs::read_to_string(&manifest).unwrap()).unwrap();
        assert_eq!(text["allowed_origins"], json!([OUR_ORIGIN]), "{text}");
    }

    // Код связи нужен до запуска браузера; расширение свяжем через порт отладки этого профиля.
    let code = crew.ok("POST", "/api/user-chrome/pairing", None).await["code"].as_str().unwrap().to_string();
    assert_eq!(code.split(' ').count(), 8, "код из восьми слов");
    let debug_port = free_port();
    let profile = common::temp_data_dir("uc-edge-profile");
    std::fs::create_dir_all(&profile).unwrap();
    let child = Command::new(&edge)
        .arg(format!("--user-data-dir={}", profile.display()))
        .arg(format!("--remote-debugging-port={debug_port}"))
        .arg(format!("--load-extension={}", ext.display()))
        .args(["--no-first-run", "--no-default-browser-check", "--headless=new", "--disable-features=msEdgeSidebarV2", "about:blank"])
        // Хост, которого запустит браузер, найдёт программу этого теста.
        .env("CREW_DATA_DIR", &crew.data_dir)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("Edge не запустился");
    let _browser = TestBrowser { child, profile };
    // Расширение пришло, но без пары не «подключено».
    let st = wait_waiting(&crew, true, 45).await;
    assert_eq!(st["connected"], false, "{st}");
    let here = format!("http://127.0.0.1:{site}/");
    let other = format!("http://localhost:{site}/");
    pair_extension(debug_port, &code, &[here.trim_end_matches('/').to_string(), other.trim_end_matches('/').to_string()]).await;
    let st = wait_connected(&crew, true, 45).await;
    assert_eq!(st["links"][0]["browser"], "edge", "{st}");
    assert_eq!(st["links"][0]["ready"], true, "{st}");

    // Запрещённый сайт (вход в аккаунт Google) закрыт и с разрешением «один раз».
    let r = run(&crew, &fake, &bot, vec![call("browser_nav", json!({ "action": "open", "url": "https://accounts.google.com/" }))], None, |_| "once").await;
    r.err(0, "запрещён");
    assert_eq!(r.asked("user_browser"), 0, "для запрещённого сайта карточки нет: {:?}", r.approvals);

    let mut secrets = 0;
    let secrets_answer = Mutex::new(0);
    let r = run(
        &crew,
        &fake,
        &bot,
        vec![
            call("browser_nav", json!({ "action": "open", "url": here })),
            with("browser_mouse", |last| json!({ "action": "click", "ref": ref_of(last, "Показать") })),
            call("browser_devtools", json!({ "action": "cookies" })),
            call("browser_devtools", json!({ "action": "eval", "expression": "document.cookie" })),
            call("browser_devtools", json!({ "action": "cookies" })),
            call("browser_nav", json!({ "action": "open", "url": other })),
        ],
        None,
        |card| match card["tool"].as_str().unwrap_or_default() {
            // Первый сайт — «всегда», второй — один раз.
            "user_browser" if card["title"].as_str().unwrap_or_default().contains("127.0.0.1") => "always",
            "browser_secrets" => {
                let mut n = secrets_answer.lock().unwrap();
                *n += 1;
                if *n == 1 { "deny" } else { "once" }
            }
            _ => "once",
        },
    )
    .await;
    if r.status(0) != "ok" {
        let trace = std::fs::read_to_string(crew.data_dir.join("user-chrome-trace.log")).unwrap_or_default();
        eprintln!("--- журнал кадров ---
{trace}");
    }
    r.ok(0, "Кабинет продавца");
    r.ok(1, "Показано-OK");
    r.err(2, "не разрешил");
    r.err(3, "закрыты");
    // Человек в приложении разрешил, но сам браузер закрывает cookies, пока человек не нажмёт «разрешить один раз» в окне расширения: второй замок.
    r.err(4, "закрыты");
    r.ok(5, "Кабинет продавца");
    assert_eq!(r.asked("user_browser"), 2, "каждый сайт — своя карточка: {:?}", r.approvals);
    assert!(r.approvals.iter().any(|a| a["detail"].as_str().unwrap_or_default().contains("настоящий Chrome")), "{:?}", r.approvals);
    secrets += r.asked("browser_secrets");
    assert_eq!(secrets, 2);
    assert!(status(&crew).await["bots"][0]["active"] == true);

    // Без человека: пока не включено боту — нельзя вовсе.
    let body = json!({ "botId": bot, "title": "Ночью", "prompt": "проверь кабинет", "everyMinutes": 60 });
    let (code, data) = crew.send("POST", "/api/workflows", Some(body)).await;
    assert_eq!(code, 201, "{data}");
    let routine = data["process"]["id"].as_str().unwrap().to_string();
    let night = |url: &str| vec![call("browser_nav", json!({ "action": "open", "url": url }))];
    let r = run(&crew, &fake, &bot, night(&here), Some(&routine), |_| "deny").await;
    r.err(0, "без человека");
    // Включено: только сайт с правилом «всегда», без карточек.
    crew.ok("PUT", &format!("/api/user-chrome/bots/{bot}"), Some(json!({ "unattended": true }))).await;
    let r = run(&crew, &fake, &bot, night(&here), Some(&routine), |_| "deny").await;
    r.ok(0, "Кабинет продавца");
    let r = run(&crew, &fake, &bot, night(&other), Some(&routine), |_| "once").await;
    r.err(0, "«всегда»");
    assert!(r.approvals.is_empty(), "ночью карточек нет: {:?}", r.approvals);

    // «Остановить всё»: сессии закрыты, расширение отпустило вкладки.
    crew.ok("POST", "/api/user-chrome/stop", None).await;
    assert!(status(&crew).await["bots"].as_array().unwrap().iter().all(|b| b["active"] == false));

    // «Отвязать»: подключения рвутся, следующий вызов бота получает отказ.
    crew.ok("POST", "/api/user-chrome/unpair", None).await;
    let st = wait_connected(&crew, false, 20).await;
    assert_eq!(st["paired"], false, "{st}");
    let r = run(&crew, &fake, &bot, vec![call("browser_nav", json!({ "action": "open", "url": here }))], None, |_| "once").await;
    r.err(0, "расширение Deloist сейчас не подключено");

    // «Отключить» убирает ключи реестра.
    crew.ok("POST", "/api/user-chrome/disconnect", None).await;
    for key in KEYS {
        assert!(reg_value(key).is_none(), "{key} должен быть удалён");
    }
}
