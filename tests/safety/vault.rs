//! Менеджер паролей (V1): подключение, разблокировка и список входов через поддельные `bw` и `op`.
//!
//! Нужны заглушки из `examples/fake_bw.rs` и `examples/fake_op.rs`: перед тестами выполнить
//! `cargo build -q --examples`. Без них тесты падают с «не смог запустить Bitwarden».

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use crate::common::{self, Crew, BIN};
use axum::body::{Body, Bytes};
use axum::extract::State;
use axum::http::StatusCode;
use axum::response::{Html, Response};
use axum::routing::{get, post};
use axum::Router;
use serde_json::{json, Value};

const MASTER: &str = "correct horse";
const SESSION: &str = "fake-session-123";
const OP_TOKEN: &str = "ops_test_token";
const BW_SECRET: &str = "pw-fixture-7731";
const OP_SECRET: &str = "op-pw-5512";

fn fixture() -> Value {
    serde_json::json!({
        "bitwarden": {
            "email": "user@example.com",
            "items": [
                {
                    "id": "bw-good", "type": 1, "name": "Example",
                    "login": {
                        "username": "anna@example.com", "password": BW_SECRET, "totp": "654321",
                        "uris": [{ "uri": "example.com" }]
                    }
                },
                {
                    "id": "bw-evil", "type": 1, "name": "Evil copy",
                    "login": {
                        "username": "anna@example.com", "password": "pw-evil-0002", "totp": "",
                        "uris": [{ "uri": "https://evil-example.com" }]
                    }
                },
                { "id": "bw-note", "type": 2, "name": "Note" }
            ]
        },
        "onepassword": {
            "items": [
                {
                    "id": "opitem1", "title": "Shop", "vault": { "id": "vaultabc" },
                    "urls": [{ "href": "https://login.example.com" }], "additional_information": "anna@example.com"
                },
                {
                    "id": "opitem2", "title": "Other", "vault": { "id": "vaultabc" },
                    "urls": [{ "href": "https://other.test" }], "additional_information": "anna@example.com"
                }
            ],
            "details": {
                "opitem1": {
                    "id": "opitem1", "title": "Shop", "vault": { "id": "vaultabc" },
                    "urls": [{ "href": "https://login.example.com" }],
                    "fields": [
                        { "label": "username", "type": "STRING", "value": "anna@example.com" },
                        { "label": "password", "type": "CONCEAL", "value": OP_SECRET },
                        { "label": "one-time password", "type": "OTP", "value": "112233" }
                    ]
                },
                "opitem2": {
                    "id": "opitem2", "title": "Other", "vault": { "id": "vaultabc" },
                    "urls": [{ "href": "https://other.test" }],
                    "fields": [{ "label": "password", "type": "CONCEAL", "value": "op-pw-other" }]
                }
            }
        }
    })
}

/// Папка для фикстуры и журнала: рядом с данными программы, но не внутри них.
fn scratch(name: &str) -> PathBuf {
    let stamp = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos();
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(format!("vault-{name}-{stamp}"));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn fake_program(name: &str) -> String {
    let suffix = if cfg!(windows) { ".exe" } else { "" };
    Path::new(BIN).parent().unwrap().join("examples").join(format!("{name}{suffix}")).display().to_string()
}

/// Окружение программы: заглушки CLI, фикстура и журнал аргументов.
fn start_env(dir: &Path) -> (String, String, String, String) {
    let fixture_path = dir.join("fixture.json");
    std::fs::write(&fixture_path, fixture().to_string()).unwrap();
    (
        fake_program("fake_bw"),
        fake_program("fake_op"),
        fixture_path.display().to_string(),
        dir.join("argv.log").display().to_string(),
    )
}

/// Есть ли в папке файл с этой строкой. Читает всё, включая вложенные папки.
fn dir_contains(dir: &Path, needle: &str) -> bool {
    let mut stack = vec![dir.to_path_buf()];
    while let Some(path) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&path) else {
            continue;
        };
        for entry in entries.flatten() {
            let child = entry.path();
            if child.is_dir() {
                stack.push(child);
            } else if let Ok(bytes) = std::fs::read(&child) {
                if bytes.windows(needle.len()).any(|window| window == needle.as_bytes()) {
                    return true;
                }
            }
        }
    }
    false
}

fn has_login(logins: &Value, id: &str) -> Option<Value> {
    logins["logins"].as_array()?.iter().find(|login| login["id"] == id).cloned()
}

#[tokio::test]
async fn bitwarden_unlock_lists_and_locks_without_leaking_secrets() {
    let dir = scratch("bitwarden");
    let (bw, op, fixture, log) = start_env(&dir);
    let env = [
        ("CREW_TEST_BW", bw.as_str()),
        ("CREW_TEST_OP", op.as_str()),
        ("FAKE_VAULT_FIXTURE", fixture.as_str()),
        ("FAKE_VAULT_LOG", log.as_str()),
    ];
    let crew = Crew::start_with_env("vault-bitwarden", &env).await;

    let (status, state) = crew.send("GET", "/api/vault", None).await;
    assert_eq!(status, 200);
    assert_eq!(state["bitwarden"]["cli"], true);
    assert_eq!(state["bitwarden"]["status"], "locked");
    assert_eq!(state["bitwarden"]["email"], "user@example.com");

    let (status, _) = crew.send("POST", "/api/vault/bitwarden/unlock", Some(json!({ "password": "wrong" }))).await;
    assert_eq!(status, 400, "неверный мастер-пароль должен давать 400");
    let (status, unlocked) = crew.send("POST", "/api/vault/bitwarden/unlock", Some(json!({ "password": MASTER }))).await;
    assert_eq!(status, 200);
    assert_eq!(unlocked["status"], "unlocked");
    assert_eq!(crew.ok("GET", "/api/vault", None).await["bitwarden"]["status"], "unlocked");

    let logins = crew.ok("GET", "/api/vault/logins?origin=https://login.example.com", None).await;
    let good = has_login(&logins, "bw:bw-good").expect("вход на этом сайте не найден");
    assert_eq!(good["hasTotp"], true);
    assert_eq!(good["username"], "anna@example.com");
    assert_eq!(good["source"], "bitwarden");
    assert!(has_login(&logins, "bw:bw-evil").is_none(), "чужой сайт попал в список");

    // Ни один ответ не содержит пароля, кода или ключа сессии.
    for answer in [&state, &unlocked, &logins] {
        let text = answer.to_string();
        for secret in [BW_SECRET, SESSION, MASTER, "654321"] {
            assert!(!text.contains(secret), "секрет {secret} в ответе: {text}");
        }
    }

    // На диске данных их тоже нет, и аргументы программ их не несли.
    for secret in [MASTER, SESSION, BW_SECRET] {
        assert!(!dir_contains(&crew.data_dir, secret), "секрет {secret} оказался в папке данных");
    }
    let argv = std::fs::read_to_string(&log).unwrap_or_default();
    assert!(argv.contains("unlock"), "заглушка bw не вызвана: {argv}");
    for secret in [MASTER, SESSION] {
        assert!(!argv.contains(secret), "секрет попал в аргументы: {argv}");
    }

    // Адрес без https не ищется вовсе.
    let (status, _) = crew.send("GET", "/api/vault/logins?origin=http://login.example.com", None).await;
    assert_eq!(status, 400);

    let (status, _) = crew.send("POST", "/api/vault/bitwarden/lock", None).await;
    assert_eq!(status, 200);
    assert_eq!(crew.ok("GET", "/api/vault", None).await["bitwarden"]["status"], "locked");
    let after = crew.ok("GET", "/api/vault/logins?origin=https://login.example.com", None).await;
    assert!(after["logins"].as_array().unwrap().is_empty(), "после блокировки входы остались");
}

#[tokio::test]
async fn onepassword_token_is_checked_kept_encrypted_and_removable() {
    let dir = scratch("onepassword");
    let (bw, op, fixture, log) = start_env(&dir);
    let env = [
        ("CREW_TEST_BW", bw.as_str()),
        ("CREW_TEST_OP", op.as_str()),
        ("FAKE_VAULT_FIXTURE", fixture.as_str()),
        ("FAKE_VAULT_LOG", log.as_str()),
    ];
    let crew = Crew::start_with_env("vault-onepassword", &env).await;

    let (status, _) = crew.send("PUT", "/api/vault/onepassword", Some(json!({ "token": "ops_wrong" }))).await;
    assert_eq!(status, 400, "чужой токен должен давать 400");
    assert_eq!(crew.ok("GET", "/api/vault", None).await["onepassword"]["connected"], false);

    let (status, connected) = crew.send("PUT", "/api/vault/onepassword", Some(json!({ "token": OP_TOKEN }))).await;
    assert_eq!(status, 200);
    assert_eq!(connected["connected"], true);
    let state = crew.ok("GET", "/api/vault", None).await;
    assert_eq!(state["onepassword"]["connected"], true);
    assert_eq!(state["onepassword"]["cli"], true);

    let logins = crew.ok("GET", "/api/vault/logins?origin=https://login.example.com", None).await;
    let shop = has_login(&logins, "op:vaultabc/opitem1").expect("вход 1Password не найден");
    assert_eq!(shop["hasTotp"], true);
    assert_eq!(shop["username"], "anna@example.com");
    assert_eq!(shop["source"], "1password");
    assert!(has_login(&logins, "op:vaultabc/opitem2").is_none(), "чужой сайт попал в список");

    // Токен не виден ни в одном ответе, ни в state.json; он лежит только в secrets.json.
    let app_state = crew.ok("GET", "/api/state", None).await;
    for answer in [&state, &connected, &logins, &app_state] {
        let text = answer.to_string();
        for secret in [OP_TOKEN, OP_SECRET] {
            assert!(!text.contains(secret), "секрет {secret} в ответе: {text}");
        }
    }
    // Ни один файл папки данных не содержит токен или пароль открытым текстом: токен в зашифрованном хранилище.
    assert!(!dir_contains(&crew.data_dir, OP_TOKEN), "токен 1Password открыто лежит в папке данных");
    assert!(!dir_contains(&crew.data_dir, OP_SECRET), "пароль 1Password попал в папку данных");
    // Заглушка op получает токен только из окружения, в аргументах его быть не должно.
    let argv = std::fs::read_to_string(&log).unwrap_or_default();
    assert!(argv.contains("whoami"), "заглушка op не вызвана: {argv}");
    assert!(!argv.contains(OP_TOKEN) && !argv.contains(OP_SECRET), "токен или пароль попали в аргументы: {argv}");

    let (status, _) = crew.send("DELETE", "/api/vault/onepassword", None).await;
    assert_eq!(status, 200);
    assert_eq!(crew.ok("GET", "/api/vault", None).await["onepassword"]["connected"], false);
    assert!(!dir_contains(&crew.data_dir, OP_TOKEN), "токен остался в папке данных после удаления");
}

// ---------------------------------------------------------------------------
// Вход сохранённым паролем в настоящем Chrome (V2)
// ---------------------------------------------------------------------------

/// Секреты фикстуры. В ответах инструментов, событиях, журнале и данных программы их быть не должно.
const LOCAL_SECRET: &str = "pw-local-6604";
const LOCAL_TOTP: &str = "654321";
const LOCAL_LOGIN: &str = "bw:bw-local";

/// Страница входа: логин, пароль, код. Страница сама присылает значение поля пароля в `/report`,
/// чтобы тест увидел, что программа вставила именно сохранённый пароль.
const SIGN_IN: &str = r##"<!doctype html><html lang="ru"><head><meta charset="utf-8"><title>Вход в кабинет</title></head><body>
<h1>Вход в кабинет</h1>
<form onsubmit="event.preventDefault()">
<input aria-label="Логин" name="login">
<input type="password" aria-label="Пароль" name="password" id="pw">
<input autocomplete="one-time-code" aria-label="Код" name="otp" id="otp">
<input type="password" aria-label="Секрет" id="pw2">
<button type="button" id="show" onclick="document.getElementById('pw2').type='text'">Показать</button>
<input type="text" aria-label="Пароль поиска" id="srch">
<input type="password" aria-label="Прыгающий пароль" id="jump" onfocus="document.getElementById('vis').focus()">
<input type="text" aria-label="Видимое поле" id="vis">
<button type="submit">Войти</button>
</form>
<script>
// Страница выдаёт фокус полем «Видимое поле», пока в фокусе «Прыгающий пароль»: событие focus в тестовом Chrome
// без фокуса окна не приходит, поэтому подмена идёт через activeElement, а не через обработчик.
var realActive = Object.getOwnPropertyDescriptor(Document.prototype, 'activeElement').get;
Object.defineProperty(document, 'activeElement', { get: function(){ var a = realActive.call(document); return a && a.id === 'jump' ? document.getElementById('vis') : a; } });
['pw', 'otp', 'pw2', 'vis', 'srch', 'jump'].forEach(function(id){
  document.getElementById(id).addEventListener('input', function(){ fetch('/report?f=' + id, {method:'POST', body: this.value}); });
});
</script>
</body></html>"##;

/// Страница другого origin с полем пароля: её встраивают во фрейм.
const FRAMED_PASSWORD: &str = r##"<!doctype html><html><head><meta charset="utf-8"></head><body>
<input type="password" aria-label="Пароль во фрейме" id="fp">
<script>document.getElementById('fp').addEventListener('input', function(){ fetch('/report?f=fp', {method:'POST', body: this.value}); });</script>
</body></html>"##;

/// Значения, которые страница получила в полях входа (вставленные программой). Свои на каждый стенд.
type Reported = Arc<Mutex<Vec<String>>>;

static CHROMES: tokio::sync::Semaphore = tokio::sync::Semaphore::const_new(4);

/// Шаг сценария модели: по тексту последнего ответа инструмента выбирает вызов.
type Step = Arc<dyn Fn(&str) -> (String, Value) + Send + Sync>;

#[derive(Default)]
struct Script {
    steps: Vec<Step>,
    next: usize,
    total: usize,
    payloads: Vec<Value>,
}

#[derive(Clone, Default)]
struct Fake(Arc<Mutex<Script>>);

fn call(name: &str, args: Value) -> Step {
    let name = name.to_string();
    Arc::new(move |_| (name.clone(), args.clone()))
}

fn with(name: &str, build: impl Fn(&str) -> Value + Send + Sync + 'static) -> Step {
    let name = name.to_string();
    Arc::new(move |last| (name.clone(), build(last)))
}

async fn completions(State(fake): State<Fake>, body: Bytes) -> Response {
    let payload: Value = serde_json::from_slice(&body).unwrap();
    let last = payload["messages"]
        .as_array()
        .and_then(|m| m.iter().rev().find(|m| m["role"] == "tool"))
        .and_then(|m| m["content"].as_str())
        .unwrap_or_default()
        .to_string();
    let step = {
        let mut script = fake.0.lock().unwrap();
        script.payloads.push(payload);
        let step = script.steps.get(script.next).cloned();
        script.next += 1;
        script.total += 1;
        step.map(|step| (step, script.total))
    };
    let delta = match step {
        Some((step, id)) => {
            let (name, args) = step(&last);
            json!({ "tool_calls": [{ "index": 0, "id": format!("call_{id}"), "type": "function", "function": {
                "name": name, "arguments": args.to_string(),
            } }] })
        }
        None => json!({ "content": "готово" }),
    };
    let text = format!("data: {}\n\ndata: [DONE]\n\n", json!({ "choices": [{ "index": 0, "delta": delta }] }));
    Response::builder().header("content-type", "text/event-stream").body(Body::from(text)).unwrap()
}

/// Номер элемента `[n]` по подписи из последнего снимка.
fn ref_of(snapshot: &str, label: &str) -> u64 {
    let quoted = format!("\"{label}\"");
    for line in snapshot.lines() {
        if line.starts_with('[') && line.contains(&quoted) {
            if let Some(end) = line.find(']') {
                if let Ok(number) = line[1..end].parse() {
                    return number;
                }
            }
        }
    }
    panic!("в снимке нет элемента «{label}»:\n{snapshot}");
}

fn open(site: &str, path: &str) -> Step {
    call("browser_nav", json!({ "action": "open", "url": format!("{site}{path}") }))
}

fn snapshot() -> Step {
    call("browser_page", json!({ "action": "snapshot" }))
}

/// Стенд: настоящий exe, настоящий Chrome без окна, поддельный `bw` с фикстурой и поддельная модель.
struct Vault {
    crew: Crew,
    fake: Fake,
    site: String,
    bot: String,
    reported: Reported,
    _chrome: tokio::sync::SemaphorePermit<'static>,
}

struct VRun {
    updates: Vec<Value>,
    approvals: Vec<Value>,
}

impl VRun {
    fn text(&self, i: usize) -> String {
        let update = self.updates.get(i).unwrap_or_else(|| panic!("нет результата шага {i}; всего {}", self.updates.len()));
        update["result"].as_str().unwrap_or_default().to_string()
    }

    fn status(&self, i: usize) -> String {
        self.updates[i]["status"].as_str().unwrap_or_default().to_string()
    }

    fn ok(&self, i: usize) {
        assert_eq!(self.status(i), "ok", "шаг {i} должен пройти:\n{}", self.text(i));
    }

    fn err(&self, i: usize) {
        assert_eq!(self.status(i), "error", "шаг {i} должен кончиться ошибкой:\n{}", self.text(i));
    }

    fn expect(&self, i: usize, needle: &str) {
        assert!(self.text(i).contains(needle), "шаг {i} [{}]: нет «{needle}» в:\n{}", self.status(i), self.text(i));
    }

    fn asked(&self, tool: &str) -> usize {
        self.approvals.iter().filter(|a| a["tool"] == tool).count()
    }
}

async fn setup_vault(name: &str, unlocked: bool) -> Vault {
    let chrome = CHROMES.acquire().await.unwrap();
    // Адрес страницы входа: http на 127.0.0.1. Программа принимает его только в тестовой сборке и только точно.
    let site_listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let site = format!("http://127.0.0.1:{}", site_listener.local_addr().unwrap().port());
    let reported: Reported = Arc::default();
    // Второй сервер на другом порту: для браузера это другой origin (но тот же процесс, не OOPIF).
    let other_listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let other_port = other_listener.local_addr().unwrap().port();
    let other_app = Router::new().route("/frame", get(|| async { Html(FRAMED_PASSWORD) }));
    tokio::spawn(async move { axum::serve(other_listener, other_app).await.unwrap() });
    let framed_page = format!(
        r#"<!doctype html><html lang="ru"><head><meta charset="utf-8"><title>Страница с фреймом</title></head><body><h1>Страница с фреймом</h1><iframe src="http://127.0.0.1:{other_port}/frame" width="400" height="120"></iframe></body></html>"#
    );
    let site_app = Router::new()
        .route("/login", get(|| async { Html(SIGN_IN) }))
        .route("/framed", get(move || { let page = framed_page.clone(); async move { Html(page) } }))
        .route(
            "/report",
            post(|State(reported): State<Reported>, body: Bytes| async move {
                reported.lock().unwrap().push(String::from_utf8_lossy(&body).to_string());
                StatusCode::NO_CONTENT
            }),
        )
        .with_state(reported.clone());
    tokio::spawn(async move { axum::serve(site_listener, site_app).await.unwrap() });

    let fake = Fake::default();
    let model = Router::new().route("/v1/chat/completions", post(completions)).with_state(fake.clone());
    let model_listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let model_port = model_listener.local_addr().unwrap().port();
    tokio::spawn(async move { axum::serve(model_listener, model).await.unwrap() });

    let dir = scratch(name);
    let fixture = json!({
        "bitwarden": {
            "email": "user@example.com",
            "items": [{
                "id": "bw-local", "type": 1, "name": "Local",
                "login": { "username": "anna@example.com", "password": LOCAL_SECRET, "totp": LOCAL_TOTP, "uris": [{ "uri": site }] }
            }]
        },
        "onepassword": { "items": [], "details": {} }
    });
    let fixture_path = dir.join("fixture.json").display().to_string();
    std::fs::write(&fixture_path, fixture.to_string()).unwrap();
    let log = dir.join("argv.log").display().to_string();
    let bw = fake_program("fake_bw");
    let env = [
        ("CREW_BROWSER_HEADLESS", "1"),
        ("CREW_TEST_ALLOW_LOOPBACK", "1"),
        ("CREW_TEST_MODE", "1"),
        ("CREW_TEST_VAULT_ORIGIN", site.as_str()),
        ("CREW_TEST_BW", bw.as_str()),
        ("FAKE_VAULT_FIXTURE", fixture_path.as_str()),
        ("FAKE_VAULT_LOG", log.as_str()),
        ("CREW_TEST_TICK_MS", "100"),
    ];
    let crew = Crew::start_with_env(name, &env).await;
    crew.ok(
        "PUT",
        "/api/settings",
        Some(json!({ "driver": "api", "baseUrl": format!("http://127.0.0.1:{model_port}/v1"), "apiKey": "k" })),
    )
    .await;
    crew.listen().await;
    let state = crew.ok("GET", "/api/state", None).await;
    let bot = state["bots"][0]["id"].as_str().unwrap().to_string();
    if unlocked {
        let (status, _) = crew.send("POST", "/api/vault/bitwarden/unlock", Some(json!({ "password": MASTER }))).await;
        assert_eq!(status, 200, "менеджер должен разблокироваться");
    }
    Vault { crew, fake, site, bot, reported, _chrome: chrome }
}

impl Vault {
    /// Один ход бота по сценарию. Карточки отвечаются сами по решению `decide`.
    async fn run(&self, steps: Vec<Step>, decide: impl Fn(&Value) -> bool) -> VRun {
        {
            let mut script = self.fake.0.lock().unwrap();
            script.steps = steps;
            script.next = 0;
        }
        let mut seen = self.crew.events.lock().unwrap().len();
        self.crew.say(&self.bot, "поехали").await;
        let started = Instant::now();
        let mut run = VRun { updates: Vec::new(), approvals: Vec::new() };
        loop {
            let fresh: Vec<Value> = {
                let events = self.crew.events.lock().unwrap();
                let slice = events[seen..].to_vec();
                seen = events.len();
                slice
            };
            let mut finished = false;
            for event in fresh {
                if event["botId"] != self.bot.as_str() {
                    continue;
                }
                match event["type"].as_str().unwrap_or_default() {
                    "approval" => {
                        let note = json!({ "tool": event["tool"], "title": event["title"], "detail": event["detail"] });
                        let allow = decide(&note);
                        run.approvals.push(note);
                        let id = event["id"].as_str().unwrap();
                        self.crew.ok("POST", &format!("/api/approvals/{id}"), Some(json!({ "allow": allow }))).await;
                    }
                    "tool_update" => run.updates.push(event.clone()),
                    "turn_end" => finished = true,
                    _ => {}
                }
            }
            if finished {
                return run;
            }
            assert!(started.elapsed() < Duration::from_secs(90), "ход не закончился за 90 с: {:?}", run.updates);
            tokio::time::sleep(Duration::from_millis(30)).await;
        }
    }

    /// Все следы ответов: события потока, запросы к модели и папка данных не содержат секрет.
    fn assert_no_secret(&self, secrets: &[&str]) {
        let events = serde_json::to_string(&*self.crew.events.lock().unwrap()).unwrap();
        let model = serde_json::to_string(&self.fake.0.lock().unwrap().payloads).unwrap();
        for secret in secrets {
            assert!(!events.contains(secret), "секрет {secret} в потоке событий");
            assert!(!model.contains(secret), "секрет {secret} в запросах к модели");
            assert!(!dir_contains(&self.crew.data_dir, secret), "секрет {secret} в папке данных (в том числе threads)");
        }
    }
}

/// Ждёт, пока страница пришлёт значение поля (вставка идёт асинхронно).
async fn reported_value(reported: &Reported, value: &str) -> bool {
    let started = Instant::now();
    loop {
        if reported.lock().unwrap().iter().any(|v| v == value) {
            return true;
        }
        if started.elapsed() > Duration::from_secs(10) {
            return false;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn browser_fills_the_saved_password_and_the_secret_stays_out_of_the_model_and_the_log() {
    let v = setup_vault("vault-fill-ok", true).await;
    let steps = vec![
        open(&v.site, "/login"),
        call("browser_logins", json!({})),
        snapshot(),
        with("browser_login", |last| json!({ "loginId": LOCAL_LOGIN, "ref": ref_of(last, "Пароль") })),
        snapshot(),
        with("browser_login", |last| json!({ "loginId": LOCAL_LOGIN, "ref": ref_of(last, "Код"), "field": "totp" })),
    ];
    let run = v.run(steps, |_| true).await;

    run.ok(0);
    run.expect(1, LOCAL_LOGIN);
    assert!(!run.text(1).contains(LOCAL_SECRET) && !run.text(1).contains(LOCAL_TOTP), "список входов: {}", run.text(1));
    run.ok(3);
    run.expect(3, "\"filled\":true");
    assert!(!run.text(3).contains(LOCAL_SECRET), "результат входа: {}", run.text(3));
    run.ok(5);
    assert!(!run.text(5).contains(LOCAL_TOTP), "результат кода: {}", run.text(5));
    assert_eq!(run.asked("browser_login"), 2, "{:?}", run.approvals);
    let card = run.approvals.iter().find(|a| a["tool"] == "browser_login").unwrap();
    assert!(card["title"].as_str().unwrap().contains("anna@example.com"), "{card}");
    assert!(card["title"].as_str().unwrap().contains("Bitwarden"), "{card}");

    // Страница получила ровно сохранённый пароль; программа его модели не показала.
    assert!(reported_value(&v.reported, LOCAL_SECRET).await, "страница не получила пароль: {:?}", v.reported.lock().unwrap());
    assert!(reported_value(&v.reported, LOCAL_TOTP).await, "страница не получила код");
    v.assert_no_secret(&[LOCAL_SECRET, LOCAL_TOTP]);
}

#[tokio::test(flavor = "multi_thread")]
async fn wrong_field_and_code_run_are_refused_until_the_browser_restarts() {
    let v = setup_vault("vault-fill-refusals", true).await;
    let steps = vec![
        open(&v.site, "/login"),
        // Логин — не поле пароля: отказ до всякой карточки.
        with("browser_login", |last| json!({ "loginId": LOCAL_LOGIN, "ref": ref_of(last, "Логин") })),
        // Код на странице: после него вход на этой странице не делаем.
        call("browser_devtools", json!({ "action": "eval", "expression": "document.title = 'изменено'" })),
        snapshot(),
        with("browser_login", |last| json!({ "loginId": LOCAL_LOGIN, "ref": ref_of(last, "Пароль") })),
        // Перезагрузка отметку origin не снимает: документ мог остаться в другой вкладке или в истории.
        open(&v.site, "/login"),
        with("browser_login", |last| json!({ "loginId": LOCAL_LOGIN, "ref": ref_of(last, "Пароль") })),
        // Отметка живёт до перезапуска браузера (service worker переживёт вкладки): закрыли все вкладки, открыли заново.
        call("browser_nav", json!({ "action": "new_tab" })),
        call("browser_nav", json!({ "action": "close_tab", "tab": 1 })),
        open(&v.site, "/login"),
        with("browser_login", |last| json!({ "loginId": LOCAL_LOGIN, "ref": ref_of(last, "Пароль") })),
    ];
    let run = v.run(steps, |_| true).await;

    run.ok(0);
    run.err(1);
    run.expect(1, "Это не поле для пароля");
    run.ok(2);
    run.err(4);
    run.expect(4, "На этом сайте выполнялся код бота");
    run.ok(5);
    run.err(6);
    run.expect(6, "На этом сайте выполнялся код бота");
    run.ok(7);
    run.ok(8);
    run.ok(9);
    run.err(10);
    run.expect(10, "На этом сайте выполнялся код бота");
    assert_eq!(run.asked("browser_login"), 0, "карточек входа быть не должно: {:?}", run.approvals);
    v.assert_no_secret(&[LOCAL_SECRET]);
}

/// Вход, который должен быть отказан после подготовки страницы: без карточки входа и без вставки.
async fn refused_login(name: &str, build: impl FnOnce(&str) -> Vec<Step>, last: usize, expect: &str, cards: usize) -> (Vault, VRun) {
    let v = setup_vault(name, true).await;
    let run = v.run(build(&v.site), |_| true).await;
    run.err(last);
    run.expect(last, expect);
    assert_eq!(run.asked("browser_login"), cards, "карточек входа: {:?}", run.approvals);
    v.assert_no_secret(&[LOCAL_SECRET, LOCAL_TOTP]);
    assert!(!v.reported.lock().unwrap().iter().any(|value| value == LOCAL_SECRET), "страница получила пароль");
    (v, run)
}

fn login_at(label: &'static str) -> Step {
    with("browser_login", move |last| json!({ "loginId": LOCAL_LOGIN, "ref": ref_of(last, label) }))
}

#[tokio::test(flavor = "multi_thread")]
async fn push_state_does_not_clear_the_eval_mark() {
    // H1: адрес меняется внутри документа, код на нём остался.
    refused_login(
        "vault-pushstate",
        |site| {
            vec![
                open(site, "/login"),
                call("browser_devtools", json!({ "action": "eval", "expression": "history.pushState({}, '', '?x')" })),
                snapshot(),
                login_at("Пароль"),
            ]
        },
        3,
        "На этом сайте выполнялся код бота",
        0,
    )
    .await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_tab_opened_by_page_code_inherits_the_eval_mark() {
    // H2: код открывает вторую вкладку того же origin по клику и сохраняет к ней доступ.
    let (_v, run) = refused_login(
        "vault-window-open",
        |site| {
            vec![
                open(site, "/login"),
                call(
                    "browser_devtools",
                    json!({ "action": "eval", "expression": "document.addEventListener('click', function(){ window.open(location.href); })" }),
                ),
                call("browser_mouse", json!({ "action": "click", "x": 5, "y": 5 })),
                call("browser_nav", json!({ "action": "switch_tab", "tab": 2 })),
                login_at("Пароль"),
            ]
        },
        4,
        "На этом сайте выполнялся код бота",
        0,
    )
    .await;
    run.ok(3);
    run.expect(3, "Вкладка 2 из 2");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_conditional_breakpoint_marks_the_origin_like_eval() {
    refused_login(
        "vault-breakpoint",
        |site| {
            vec![
                open(site, "/login"),
                call("browser_devtools", json!({ "action": "breakpoint", "url": format!("{site}/app.js"), "line": 0, "condition": "document.title = 'x'" })),
                snapshot(),
                login_at("Пароль"),
            ]
        },
        3,
        "На этом сайте выполнялся код бота",
        0,
    )
    .await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_field_inside_a_foreign_frame_is_refused() {
    // M1: поле пароля в iframe другого origin (другой порт).
    refused_login(
        "vault-frame",
        |site| vec![open(site, "/framed"), snapshot(), login_at("Пароль во фрейме")],
        2,
        "вложенном кадре",
        0,
    )
    .await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_field_that_steals_focus_gets_nothing() {
    // M2: поле пароля при фокусе передаёт фокус видимому текстовому полю.
    let (v, _run) = refused_login(
        "vault-focus-jump",
        |site| vec![open(site, "/login"), snapshot(), login_at("Прыгающий пароль")],
        2,
        "забрала фокус",
        1,
    )
    .await;
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert!(v.reported.lock().unwrap().is_empty(), "в видимое поле ничего не должно попасть: {:?}", v.reported.lock().unwrap());
}

#[tokio::test(flavor = "multi_thread")]
async fn a_text_field_named_password_is_not_a_password_field() {
    // M3: подпись «Пароль поиска» у обычного текстового поля.
    refused_login(
        "vault-search-field",
        |site| vec![open(site, "/login"), snapshot(), login_at("Пароль поиска")],
        2,
        "Это не поле для пароля",
        0,
    )
    .await;
}

#[tokio::test(flavor = "multi_thread")]
async fn clicking_a_filled_field_leaks_no_label_and_the_snapshot_hides_the_code() {
    // H3 и H4: после вставки пароля и кода клик по полю и новый снимок не показывают значения.
    let v = setup_vault("vault-labels", true).await;
    let steps = vec![
        open(&v.site, "/login"),
        snapshot(),
        login_at("Пароль"),
        snapshot(),
        with("browser_login", |last| json!({ "loginId": LOCAL_LOGIN, "ref": ref_of(last, "Код"), "field": "totp" })),
        snapshot(),
        with("browser_mouse", |last| json!({ "action": "click", "ref": ref_of(last, "Код") })),
        snapshot(),
        with("browser_mouse", |last| json!({ "action": "click", "ref": ref_of(last, "Пароль") })),
        snapshot(),
    ];
    let run = v.run(steps, |_| true).await;
    for i in [2, 4, 6, 8] {
        run.ok(i);
    }
    for i in [3, 5, 7, 9] {
        assert!(!run.text(i).contains(LOCAL_TOTP) && !run.text(i).contains(LOCAL_SECRET), "снимок {i} показал секрет:\n{}", run.text(i));
    }
    assert!(reported_value(&v.reported, LOCAL_TOTP).await, "страница не получила код");
    v.assert_no_secret(&[LOCAL_SECRET, LOCAL_TOTP]);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_password_field_switched_to_text_stays_masked() {
    // H4: страница переключает поле на type=text после вставки.
    let v = setup_vault("vault-toggle", true).await;
    let steps = vec![
        open(&v.site, "/login"),
        snapshot(),
        login_at("Секрет"),
        snapshot(),
        with("browser_mouse", |last| json!({ "action": "click", "ref": ref_of(last, "Показать") })),
        snapshot(),
    ];
    let run = v.run(steps, |_| true).await;
    run.ok(2);
    run.ok(4);
    assert!(reported_value(&v.reported, LOCAL_SECRET).await, "страница не получила пароль");
    assert!(!run.text(5).contains(LOCAL_SECRET), "поле, переключённое на text, показало пароль:\n{}", run.text(5));
    v.assert_no_secret(&[LOCAL_SECRET]);
}

#[tokio::test(flavor = "multi_thread")]
async fn unattended_run_refuses_a_login_without_a_saved_always() {
    let v = setup_vault("vault-unattended", true).await;
    // Человек открыл страницу входа в обычном ходе.
    let run = v.run(vec![open(&v.site, "/login")], |_| true).await;
    run.ok(0);

    // Рутина по расписанию — ход без человека: вход без «всегда» отказом, без карточки.
    let before = v.crew.count_events(|e| e["type"] == "turn_end" && e["botId"] == v.bot.as_str());
    v.fake.0.lock().unwrap().steps = vec![
        snapshot(),
        with("browser_login", |last| json!({ "loginId": LOCAL_LOGIN, "ref": ref_of(last, "Пароль") })),
    ];
    v.fake.0.lock().unwrap().next = 0;
    let body = json!({ "botId": v.bot, "title": "Вход", "prompt": "войди на сайт", "everyMinutes": 60 });
    let (status, data) = v.crew.send("POST", "/api/routines", Some(body)).await;
    assert_eq!(status, 201, "{data}");
    let id = data["routine"]["id"].as_str().unwrap().to_string();
    v.crew.ok("PATCH", &format!("/api/routines/{id}"), Some(json!({ "nextRun": common::due_now() }))).await;
    let started = Instant::now();
    while v.crew.count_events(|e| e["type"] == "turn_end" && e["botId"] == v.bot.as_str()) <= before {
        assert!(started.elapsed() < Duration::from_secs(90), "рутина не закончилась");
        tokio::time::sleep(Duration::from_millis(50)).await;
    }

    assert_eq!(v.crew.count_events(|e| e["type"] == "approval" && e["tool"] == "browser_login"), 0, "без человека карточки входа быть не должно");
    let last = v.fake.0.lock().unwrap().payloads.last().cloned().expect("модель не вызывалась");
    assert!(last.to_string().contains("Без тебя бот входит только туда, где ты разрешил"), "{last}");
    v.assert_no_secret(&[LOCAL_SECRET]);
}

#[tokio::test(flavor = "multi_thread")]
async fn locked_manager_says_so_instead_of_listing_nothing() {
    let v = setup_vault("vault-locked", false).await;
    let run = v.run(vec![open(&v.site, "/login"), call("browser_logins", json!({}))], |_| true).await;
    run.ok(1);
    run.expect(1, "Менеджер паролей не подключён");
    v.assert_no_secret(&[LOCAL_SECRET]);
}

#[tokio::test(flavor = "multi_thread")]
async fn screenshots_are_refused_while_a_filled_code_is_on_screen() {
    // N2: код входа лежит в обычном текстовом поле, маска есть только в дереве страницы.
    let v = setup_vault("vault-shot", true).await;
    let steps = vec![
        open(&v.site, "/login"),
        snapshot(),
        with("browser_login", |last| json!({ "loginId": LOCAL_LOGIN, "ref": ref_of(last, "Код"), "field": "totp" })),
        call("browser_page", json!({ "action": "screenshot" })),
        // Переход на новую страницу снимает запрет.
        open(&v.site, "/login"),
        call("browser_page", json!({ "action": "screenshot" })),
    ];
    let run = v.run(steps, |_| true).await;
    run.ok(2);
    run.err(3);
    run.expect(3, "снимок экрана недоступен");
    run.ok(4);
    run.ok(5);
    v.assert_no_secret(&[LOCAL_TOTP]);
}
