//! Браузер бота целиком: настоящий exe, настоящий Chrome без окна, поддельная модель по сценарию
//! и локальный сайт. Проверяем то, что видит модель в следующем снимке, а не внутренности.

use crate::common;

use std::process::Command;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use axum::body::{Body, Bytes};
use axum::extract::State;
use axum::http::{header, StatusCode};
use axum::response::{Html, Response};
use axum::routing::{get, post};
use axum::Router;
use common::Crew;
use serde_json::{json, Value};

// ---------------------------------------------------------------------------
// Локальный сайт
// ---------------------------------------------------------------------------

const INDEX: &str = r##"<!doctype html><html lang="ru"><head><meta charset="utf-8"><title>Главная страница</title></head><body>
<h1>Главная</h1>
<a href="/form">Форма</a>
<button onclick="document.getElementById('out').textContent='Показано-OK';console.log('журнал-маяк')">Показать</button>
<button onclick="document.title='paid';document.getElementById('out').textContent='оплачено'">Оплатить</button>
<button onclick="confirm('точно?')">Спросить</button>
<button onclick="document.getElementById('out').textContent='res='+compute(1)">Считать</button>
<a href="/tab" target="_blank">Новая вкладка</a>
<div id="out"></div>
<div style="position:absolute;left:700px;top:500px;width:220px;height:60px;background:#ddd" onmouseover="this.textContent='Наведено'">Навести сюда</div>
<script src="/app.js"></script>
<script>fetch('/api/data').then(function(r){return r.json()})</script>
</body></html>"##;

// Функция compute начинается на строке 2 (с нуля), её `return` стоит на строке 4.
const APP_JS: &str = "// секретный_маркер_кода\nvar APP_VERSION = 1;\nfunction compute(a){\n  var local='переменная-маяк';\n  return a+1;\n}\n";
const COMPUTE_RETURN_LINE: u32 = 4;

const FORM: &str = r##"<!doctype html><html lang="ru"><head><meta charset="utf-8"><title>Форма</title></head><body>
<h1>Форма</h1>
<input id="q" aria-label="Запрос" onkeydown="this.dataset.n=(+this.dataset.n||0)+1;document.getElementById('cnt').textContent='keys='+this.dataset.n">
<div id="cnt">keys=0</div>
<input type="password" aria-label="Пароль" oninput="if(this.value)document.getElementById('leak').textContent='ввод:Пароль'">
<input autocomplete="cc-number" aria-label="Карта" oninput="if(this.value)document.getElementById('leak').textContent='ввод:Карта'">
<div id="leak"></div>
<input type="file" aria-label="Файл" onchange="document.getElementById('fname').textContent='файл:'+this.files[0].name">
<div id="fname"></div>
<form onsubmit="event.preventDefault();document.getElementById('sent').textContent='заявка-отправлена'">
<input aria-label="Тема"><button type="submit">Отправить заявку</button></form>
<div id="sent"></div>
</body></html>"##;

const CANVAS: &str = r##"<!doctype html><html lang="ru"><head><meta charset="utf-8"><title>Холст</title></head><body>
<h1>Холст</h1>
<div id="info">нет</div>
<div id="sc">скролл</div>
<button style="position:absolute;left:300px;top:200px;width:120px;height:50px" onclick="document.getElementById('info').textContent='кнопка-нажата'">Холст</button>
<div id="pad" style="position:absolute;left:100px;top:400px;width:400px;height:200px;background:#eef">площадка</div>
<div style="height:3000px"></div>
<script>
var start=null;
document.getElementById('pad').addEventListener('mousedown',function(e){start=Math.round(e.clientX)+','+Math.round(e.clientY)});
document.addEventListener('mouseup',function(e){if(start){document.getElementById('info').textContent='drag:'+start+'->'+Math.round(e.clientX)+','+Math.round(e.clientY);start=null}});
window.addEventListener('scroll',function(){document.getElementById('sc').textContent='прокручено'});
</script></body></html>"##;

// Редактор ответа, как в X: contenteditable, который ниже окна и сам гасит фокус от мыши (mousedown без действия по умолчанию).
const EDITOR: &str = r##"<!doctype html><html lang="ru"><head><meta charset="utf-8"><title>Редактор</title></head><body>
<h1>Редактор</h1><div id="st">нет-фокуса</div>
<div style="height:2000px"></div>
<div contenteditable="true" role="textbox" aria-label="Ответ" style="height:120px;border:1px solid #888"
 onmousedown="event.preventDefault()" onfocus="document.getElementById('st').textContent='фокус-есть'"></div>
</body></html>"##;

const LOGIN: &str = r##"<!doctype html><html lang="ru"><head><meta charset="utf-8"><title>Вход в кабинет</title></head><body>
<h1>Вход</h1>
<form onsubmit="event.preventDefault()"><input aria-label="Логин"><input type="password" aria-label="Пароль"><button type="submit">Войти</button></form>
<a href="/">Главная</a>
</body></html>"##;

const SHOW_COOKIE: &str = r##"<!doctype html><html lang="ru"><head><meta charset="utf-8"><title>Куки</title></head><body>
<h1>Куки</h1><div id="c"></div>
<script>document.getElementById('c').textContent='cookie:'+document.cookie</script></body></html>"##;

async fn index() -> impl axum::response::IntoResponse {
    ([(header::SET_COOKIE, "tok=sekret-cookie-val; Path=/")], Html(INDEX))
}

async fn start_site() -> String {
    let app = Router::new()
        .route("/", get(index))
        .route("/app.js", get(|| async { ([(header::CONTENT_TYPE, "application/javascript; charset=utf-8")], APP_JS) }))
        .route(
            "/api/data",
            get(|| async { ([(header::CONTENT_TYPE, "application/json; charset=utf-8")], r#"{"ответ":"тело-маяк"}"#) }),
        )
        .route("/form", get(|| async { Html(FORM) }))
        .route("/canvas", get(|| async { Html(CANVAS) }))
        .route("/editor", get(|| async { Html(EDITOR) }))
        .route(
            "/tab",
            get(|| async { Html(r#"<!doctype html><html><head><meta charset="utf-8"><title>Вкладка</title></head><body><h1>Вторая вкладка</h1></body></html>"#) }),
        )
        .route(
            "/redirect",
            get(|| async { (StatusCode::FOUND, [(header::LOCATION, "http://169.254.169.254/latest/meta-data/")], "") }),
        )
        .route(
            "/download",
            get(|| async {
                (
                    [(header::CONTENT_DISPOSITION, "attachment; filename=report.txt"), (header::CONTENT_TYPE, "text/plain; charset=utf-8")],
                    "содержимое-отчёта",
                )
            }),
        )
        .route(
            "/setcookie",
            get(|| async {
                (
                    [(header::SET_COOKIE, "sid=bot0-secret; Path=/; Max-Age=86400")],
                    Html(r#"<!doctype html><html><head><meta charset="utf-8"><title>Кука</title></head><body><h1>Кука поставлена</h1></body></html>"#),
                )
            }),
        )
        .route("/login", get(|| async { Html(LOGIN) }))
        .route("/showcookie", get(|| async { Html(SHOW_COOKIE) }));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    format!("http://127.0.0.1:{port}")
}

// ---------------------------------------------------------------------------
// Поддельная модель по сценарию
// ---------------------------------------------------------------------------

/// Шаг сценария: по тексту последнего ответа инструмента выбирает вызов.
type Step = Arc<dyn Fn(&str) -> (String, Value) + Send + Sync>;

#[derive(Default)]
struct Script {
    steps: Vec<Step>,
    next: usize,
    total: usize,
    payloads: Vec<Value>,
    /// Ответ на служебный вызов «Научи бота»: JSON навыка.
    service: Option<String>,
}

#[derive(Clone, Default)]
struct Fake(Arc<Mutex<Script>>);

impl Fake {
    fn set_script(&self, steps: Vec<Step>) {
        let mut script = self.0.lock().unwrap();
        script.steps = steps;
        script.next = 0;
    }

    fn set_service(&self, reply: &str) {
        self.0.lock().unwrap().service = Some(reply.to_string());
    }

    fn payloads(&self) -> Vec<Value> {
        self.0.lock().unwrap().payloads.clone()
    }

    fn last_payload(&self) -> Value {
        self.0.lock().unwrap().payloads.last().cloned().expect("модель не вызывалась")
    }
}

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
    let teach = payload.to_string().contains("Запись действий человека");
    let (step, id, service) = {
        let mut script = fake.0.lock().unwrap();
        script.payloads.push(payload);
        if teach {
            let reply = script.service.clone().unwrap_or_else(|| "не json".into());
            (None, 0, Some(reply))
        } else {
            let step = script.steps.get(script.next).cloned();
            script.next += 1;
            script.total += 1;
            (step, script.total, None)
        }
    };
    let delta = match step {
        _ if service.is_some() => json!({ "content": service.unwrap() }),
        Some(step) => {
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
    eprintln!("в снимке нет элемента «{label}»:\n{snapshot}");
    9999
}

/// Идентификатор запроса из списка network: первая колонка строки с нужным адресом.
fn request_id(listing: &str, needle: &str) -> String {
    listing
        .lines()
        .find(|line| line.contains(needle))
        .and_then(|line| line.split(" · ").next())
        .map(str::to_string)
        .unwrap_or_else(|| {
            eprintln!("в списке запросов нет {needle}:\n{listing}");
            "нет-такого".into()
        })
}

// ---------------------------------------------------------------------------
// Стенд
// ---------------------------------------------------------------------------

/// Каждый тест поднимает настоящий Chrome. Больше четырёх сразу не укладываются в свои сроки на обычном
/// компьютере, и падают разные тесты вперемешку.
static CHROMES: tokio::sync::Semaphore = tokio::sync::Semaphore::const_new(4);

struct Env {
    crew: Crew,
    fake: Fake,
    site: String,
    bot: String,
    _chrome: tokio::sync::SemaphorePermit<'static>,
}

async fn setup(name: &str) -> Env {
    setup_with(name, &[("CREW_BROWSER_HEADLESS", "1"), ("CREW_TEST_ALLOW_LOOPBACK", "1")]).await
}

async fn setup_with(name: &str, env: &[(&str, &str)]) -> Env {
    let chrome = CHROMES.acquire().await.unwrap();
    let fake = Fake::default();
    let router = Router::new().route("/v1/chat/completions", post(completions)).with_state(fake.clone());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
    let site = start_site().await;

    let crew = Crew::start_with_env(name, env).await;
    crew.ok(
        "PUT",
        "/api/settings",
        Some(json!({ "driver": "api", "baseUrl": format!("http://127.0.0.1:{port}/v1"), "apiKey": "k" })),
    )
    .await;
    crew.listen().await;
    let state = crew.ok("GET", "/api/state", None).await;
    let bot = state["bots"][0]["id"].as_str().unwrap().to_string();
    Env { crew, fake, site, bot, _chrome: chrome }
}

struct Run {
    updates: Vec<Value>,
    approvals: Vec<Value>,
    events: Vec<Value>,
}

impl Run {
    fn upd(&self, i: usize) -> &Value {
        self.updates.get(i).unwrap_or_else(|| panic!("нет результата шага {i}; всего {}\n{}", self.updates.len(), self.dump()))
    }

    fn text(&self, i: usize) -> &str {
        self.upd(i)["result"].as_str().unwrap_or_default()
    }

    fn status(&self, i: usize) -> &str {
        self.upd(i)["status"].as_str().unwrap_or_default()
    }

    fn dump(&self) -> String {
        self.updates.iter().enumerate().map(|(i, u)| format!("--- шаг {i} [{}]\n{}", u["status"], u["result"].as_str().unwrap_or_default())).collect::<Vec<_>>().join("\n")
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

    fn lacks(&self, i: usize, needle: &str) {
        assert!(!self.text(i).contains(needle), "шаг {i} [{}]: лишнее «{needle}» в:\n{}", self.status(i), self.text(i));
    }

    fn asked(&self, tool: &str) -> usize {
        self.approvals.iter().filter(|a| a["tool"] == tool).count()
    }

    /// Сколько миллисекунд шаг ждал ответа инструмента.
    fn ms(&self, i: usize) -> i64 {
        let id = self.upd(i)["callId"].clone();
        let started = self.events.iter().find(|e| e["type"] == "tool_start" && e["callId"] == id).map_or(0, |e| e["at"].as_i64().unwrap_or(0));
        self.upd(i)["at"].as_i64().unwrap_or(0) - started
    }
}

impl Env {
    fn url(&self, path: &str) -> String {
        format!("{}{path}", self.site)
    }

    fn workspace(&self, bot: &str) -> std::path::PathBuf {
        self.crew.data_dir.join("workspaces").join(bot)
    }

    async fn new_bot(&self, name: &str) -> String {
        let res = self.crew.ok("POST", "/api/bots", Some(json!({ "name": name }))).await;
        res["bot"]["id"].as_str().unwrap_or_else(|| panic!("нет id бота: {res}")).to_string()
    }

    async fn sites_of(&self, bot: &str) -> Vec<String> {
        let state = self.crew.ok("GET", "/api/state", None).await;
        let found = state["bots"].as_array().unwrap().iter().find(|b| b["id"] == bot).cloned().expect("нет бота");
        found["browserSites"].as_array().cloned().unwrap_or_default().iter().filter_map(|s| s.as_str().map(str::to_string)).collect()
    }

    async fn run(&self, steps: Vec<Step>, decide: impl Fn(&Value) -> bool) -> Run {
        self.run_bot(&self.bot, steps, decide).await
    }

    /// Один ход бота по сценарию. Карточки подтверждения отвечаются сами по решению `decide`.
    async fn run_bot(&self, bot: &str, steps: Vec<Step>, decide: impl Fn(&Value) -> bool) -> Run {
        self.fake.set_script(steps);
        let mut seen = self.crew.events.lock().unwrap().len();
        self.crew.say(bot, "поехали").await;
        let started = Instant::now();
        let mut run = Run { updates: Vec::new(), approvals: Vec::new(), events: Vec::new() };
        loop {
            let fresh: Vec<Value> = {
                let events = self.crew.events.lock().unwrap();
                let slice = events[seen..].to_vec();
                seen = events.len();
                slice
            };
            let mut finished = false;
            for event in fresh {
                if event["botId"] != bot {
                    continue;
                }
                match event["type"].as_str().unwrap_or_default() {
                    "approval" => {
                        let note = json!({ "tool": event["tool"], "title": event["title"], "detail": event["detail"], "allowLabel": event["allowLabel"], "denyLabel": event["denyLabel"] });
                        let allow = decide(&note);
                        run.approvals.push(note);
                        let id = event["id"].as_str().unwrap();
                        self.crew.ok("POST", &format!("/api/approvals/{id}"), Some(json!({ "allow": allow }))).await;
                    }
                    "tool_update" => run.updates.push(event.clone()),
                    "turn_end" => finished = true,
                    _ => {}
                }
                run.events.push(event);
            }
            if finished {
                return run;
            }
            if started.elapsed() > Duration::from_secs(90) {
                let types: Vec<String> = run.events.iter().map(|e| e["type"].to_string()).collect();
                panic!("ход не закончился за 90 с; события: {}\nрезультаты:\n{}", types.join(", "), run.dump());
            }
            tokio::time::sleep(Duration::from_millis(30)).await;
        }
    }
}

fn open(env: &Env, path: &str) -> Step {
    call("browser_nav", json!({ "action": "open", "url": env.url(path) }))
}

fn click(label: &'static str) -> Step {
    with("browser_mouse", move |last| json!({ "action": "click", "ref": ref_of(last, label) }))
}

fn snapshot() -> Step {
    call("browser_page", json!({ "action": "snapshot" }))
}

// ---------------------------------------------------------------------------
// Тесты
// ---------------------------------------------------------------------------

/// Настоящие открытые сайты в настоящем окне браузера за краем экрана, только чтение. Нужен интернет,
/// поэтому вручную: cargo test --test surface browser:: real_public_sites -- --ignored --nocapture
#[tokio::test(flavor = "multi_thread")]
#[ignore = "ходит в интернет"]
async fn real_public_sites_read_only() {
    let env = setup_with("real-sites", &[]).await;
    let sites = [
        ("https://example.com/", "Example Domain"),
        ("https://ru.wikipedia.org/wiki/Rust_(%D1%8F%D0%B7%D1%8B%D0%BA_%D0%BF%D1%80%D0%BE%D0%B3%D1%80%D0%B0%D0%BC%D0%BC%D0%B8%D1%80%D0%BE%D0%B2%D0%B0%D0%BD%D0%B8%D1%8F)", "Rust"),
        ("https://github.com/rust-lang/rust", "rust"),
        ("https://www.python.org/", "Python"),
        ("https://habr.com/ru/articles/", "Хабр"),
        ("https://www.ozon.ru/", "OZON"),
    ];
    let mut steps = Vec::new();
    for (url, _) in sites {
        steps.push(call("browser_nav", json!({ "action": "open", "url": url })));
        steps.push(snapshot());
    }
    steps.push(call("browser_page", json!({ "action": "screenshot" })));
    let run = env.run(steps, |_| false).await;
    assert!(run.approvals.is_empty(), "чтение не должно спрашивать разрешения: {:?}", run.approvals);
    for (i, (url, needle)) in sites.iter().enumerate() {
        let page = run.text(i * 2 + 1);
        let found = page.contains(needle);
        let head: String = page.chars().take(110).collect();
        println!("[{}] найдено «{needle}»: {found}; {} симв.; {url}\n    {}", run.status(i * 2 + 1), page.chars().count(), head.replace('\n', " / "));
    }
    println!("снимок: [{}] {}", run.status(sites.len() * 2), run.text(sites.len() * 2).chars().take(120).collect::<String>());

    let dir = env.crew.data_dir.display().to_string();
    let script = format!("(Get-CimInstance Win32_Process -Filter \"name='chrome.exe' or name='msedge.exe'\" | Where-Object {{ $_.CommandLine -like '*{dir}*' }} | Measure-Object WorkingSetSize -Sum).Sum / 1MB");
    let memory = std::process::Command::new("pwsh").args(["-NoProfile", "-Command", &script]).output().unwrap();
    println!("память браузера, МБ: {}", String::from_utf8_lossy(&memory.stdout).trim());
    env.crew.ok("POST", &format!("/api/bots/{}/browser", env.bot), None).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn open_snapshot_click_and_site_permission_asked_once() {
    let env = setup("br-open").await;
    let run = env.run(vec![open(&env, "/"), click("Показать"), click("Показать")], |_| true).await;

    run.ok(0);
    run.expect(0, "Главная");
    run.expect(0, "[1] ");
    run.expect(0, "button \"Показать\"");
    run.ok(1);
    run.expect(1, "Показано-OK");
    run.ok(2);
    assert_eq!(run.approvals.len(), 1, "{:?}", run.approvals);
    assert_eq!(run.asked("browser_site"), 1, "{:?}", run.approvals);
    // Сайт запоминается правилом «всегда здесь» (старое «да» на карточке сайта значит именно это).
    let rules = env.crew.ok("GET", "/api/permissions", None).await["rules"].as_array().cloned().unwrap();
    let remembered = rules.iter().any(|r| r["botId"] == env.bot.as_str() && r["tool"] == "browser_site" && r["place"].as_str().is_some_and(|p| p.starts_with("http://127.0.0.1:")));
    assert!(remembered, "сайт не запомнился: {rules:?}");
}

/// «Свобода действий»: обычный клик без карточки сайта, а оплата всё равно спрашивается.
#[tokio::test(flavor = "multi_thread")]
async fn auto_mode_clicks_without_a_site_card_but_still_asks_for_payment() {
    let env = setup("br-auto-site").await;
    env.crew.ok("PATCH", &format!("/api/bots/{}", env.bot), Some(json!({ "approval": "auto" }))).await;
    let run = env.run(vec![open(&env, "/"), click("Показать"), click("Оплатить")], |_| false).await;

    run.ok(1);
    run.expect(1, "Показано-OK");
    assert_eq!(run.asked("browser_site"), 0, "{:?}", run.approvals);
    assert_eq!(run.asked("browser_risky"), 1, "{:?}", run.approvals);
    run.err(2);
}

#[tokio::test(flavor = "multi_thread")]
async fn denied_site_leaves_the_page_untouched() {
    let env = setup("br-deny-site").await;
    let run = env.run(vec![open(&env, "/"), click("Показать"), snapshot()], |a| a["tool"] != "browser_site").await;

    run.ok(0);
    run.err(1);
    run.expect(1, "не разрешил");
    run.ok(2);
    run.lacks(2, "Показано-OK");
    assert_eq!(run.asked("browser_site"), 1);
    assert!(env.sites_of(&env.bot).await.is_empty(), "отказ не должен запоминаться");
}

#[tokio::test(flavor = "multi_thread")]
async fn risky_click_is_asked_every_time() {
    let env = setup("br-risky").await;
    let risky_seen = AtomicUsize::new(0);
    let steps = vec![
        open(&env, "/"),
        click("Оплатить"),
        snapshot(),
        click("Оплатить"),
        call("browser_page", json!({ "action": "wait", "seconds": 0.5 })),
    ];
    // Сайт разрешён, первую оплату отклоняем, вторую разрешаем.
    let run = env
        .run(steps, |a| match a["tool"].as_str() {
            Some("browser_risky") => risky_seen.fetch_add(1, Ordering::SeqCst) > 0,
            _ => true,
        })
        .await;

    assert_eq!(run.asked("browser_site"), 1, "{:?}", run.approvals);
    assert_eq!(run.asked("browser_risky"), 2, "{:?}", run.approvals);
    run.err(1);
    run.ok(2);
    run.lacks(2, "оплачено");
    run.lacks(2, "· paid ·");
    run.ok(3);
    run.expect(4, "оплачено");
    run.expect(4, "· paid ·");
}

#[tokio::test(flavor = "multi_thread")]
async fn keyboard_types_cyrillic_and_refuses_passwords_and_cards() {
    let env = setup("br-keys").await;
    let steps = vec![
        open(&env, "/form"),
        with("browser_keyboard", |last| json!({ "action": "type", "ref": ref_of(last, "Запрос"), "text": "привет мир" })),
        call("browser_keyboard", json!({ "action": "press", "keys": "Control+A" })),
        call("browser_keyboard", json!({ "action": "press", "keys": "Backspace" })),
        with("browser_keyboard", |last| json!({ "action": "type", "ref": ref_of(last, "Пароль"), "text": "hunter2" })),
        snapshot(),
        with("browser_keyboard", |last| json!({ "action": "type", "ref": ref_of(last, "Карта"), "text": "4111111111111111" })),
        snapshot(),
    ];
    let run = env.run(steps, |_| true).await;

    run.ok(1);
    run.expect(1, "textbox \"Запрос\" = \"привет мир\"");
    run.expect(1, "keys=10");
    run.ok(2);
    run.ok(3);
    run.lacks(3, "привет");
    run.err(4);
    run.expect(4, "browser_human");
    run.err(6);
    run.expect(6, "browser_human");
    run.ok(7);
    run.lacks(7, "ввод:");
    // Единственный вопрос за весь ход: разрешение на сайт при первой печати.
    assert_eq!(run.approvals.len(), 1, "{:?}", run.approvals);
    assert_eq!(run.asked("browser_site"), 1);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_click_on_an_editor_below_the_fold_gives_it_focus() {
    let env = setup("br-editor-focus").await;
    let steps = vec![
        open(&env, "/editor"),
        with("browser_mouse", |last| json!({ "action": "click", "ref": ref_of(last, "Ответ") })),
        with("browser_keyboard", |last| json!({ "action": "type", "ref": ref_of(last, "Ответ"), "text": "привет" })),
    ];
    let run = env.run(steps, |_| true).await;
    run.ok(1);
    run.expect(1, "фокус-есть");
    run.ok(2);
}

#[tokio::test(flavor = "multi_thread")]
async fn hover_needs_no_approval() {
    let env = setup("br-hover").await;
    let run = env.run(vec![open(&env, "/"), call("browser_mouse", json!({ "action": "move", "x": 760, "y": 530 }))], |_| true).await;
    run.ok(1);
    run.expect(1, "Наведено");
    assert!(run.approvals.is_empty(), "наведение не должно ничего спрашивать: {:?}", run.approvals);
}

#[tokio::test(flavor = "multi_thread")]
async fn mouse_by_coordinates_drag_and_wheel() {
    let env = setup("br-mouse").await;
    let steps = vec![
        open(&env, "/canvas"),
        call("browser_mouse", json!({ "action": "click", "x": 360, "y": 225 })),
        call("browser_mouse", json!({ "action": "drag", "x": 150, "y": 450, "to_x": 400, "to_y": 550 })),
        call("browser_mouse", json!({ "action": "wheel", "dy": 500 })),
        call("browser_page", json!({ "action": "wait", "text": "прокручено", "seconds": 5 })),
    ];
    let run = env.run(steps, |_| true).await;

    run.ok(1);
    run.expect(1, "кнопка-нажата");
    run.ok(2);
    run.expect(2, "drag:150,450->400,550");
    run.ok(3);
    run.ok(4);
    run.expect(4, "прокручено");
    assert_eq!(run.asked("browser_site"), 1, "{:?}", run.approvals);
}

#[tokio::test(flavor = "multi_thread")]
async fn link_with_target_blank_opens_a_working_tab() {
    let env = setup("br-popup").await;
    let steps = vec![open(&env, "/"), click("Новая вкладка"), call("browser_nav", json!({ "action": "tabs" }))];
    let run = env.run(steps, |_| true).await;

    run.ok(1);
    run.expect(1, "Вторая вкладка");
    run.expect(1, "новая вкладка");
    run.expect(2, "1. ");
    run.expect(2, "2. ");
}

#[tokio::test(flavor = "multi_thread")]
async fn tabs_switch_and_close() {
    let env = setup("br-tabs").await;
    let steps = vec![
        open(&env, "/"),
        call("browser_nav", json!({ "action": "new_tab", "url": env.url("/tab") })),
        call("browser_nav", json!({ "action": "tabs" })),
        call("browser_nav", json!({ "action": "switch_tab", "tab": 1 })),
        call("browser_nav", json!({ "action": "switch_tab", "tab": 2 })),
        call("browser_nav", json!({ "action": "close_tab" })),
        call("browser_nav", json!({ "action": "tabs" })),
    ];
    let run = env.run(steps, |_| true).await;

    run.ok(1);
    run.expect(1, "Вторая вкладка");
    run.expect(2, "1. ");
    run.expect(2, "2. ");
    run.ok(3);
    run.expect(3, "Главная");
    run.ok(4);
    run.expect(4, "Вторая вкладка");
    run.ok(5);
    run.expect(5, "Главная");
    run.expect(6, "1. ");
    run.lacks(6, "2. ");
}

#[tokio::test(flavor = "multi_thread")]
async fn confirm_dialog_is_reported_and_answered() {
    let env = setup("br-dialog").await;
    let steps = vec![open(&env, "/"), click("Спросить"), call("browser_page", json!({ "action": "dialog", "accept": false }))];
    let run = env.run(steps, |_| true).await;

    run.expect(1, "Открыт диалог confirm");
    run.expect(1, "точно?");
    run.ok(2);
    run.lacks(2, "Открыт диалог");
    run.expect(2, "Главная");
}

#[tokio::test(flavor = "multi_thread")]
async fn devtools_reading() {
    let env = setup("br-devtools").await;
    let app_js = env.url("/app.js");
    let steps = vec![
        open(&env, "/"),
        click("Показать"),
        call("browser_devtools", json!({ "action": "sources" })),
        call("browser_devtools", json!({ "action": "source", "url": app_js })),
        call("browser_devtools", json!({ "action": "search", "query": "секретный_маркер_кода" })),
        call("browser_devtools", json!({ "action": "html" })),
        call("browser_devtools", json!({ "action": "console" })),
        call("browser_devtools", json!({ "action": "network", "query": "/api/data" })),
        with("browser_devtools", |last| json!({ "action": "request", "id": request_id(last, "/api/data") })),
        call("browser_devtools", json!({ "action": "network", "query": "/api/data" })),
        with("browser_devtools", |last| json!({ "action": "response", "id": request_id(last, "/api/data") })),
    ];
    let run = env.run(steps, |_| true).await;

    run.ok(1);
    run.ok(2);
    run.expect(2, "/app.js");
    run.ok(3);
    run.expect(3, "секретный_маркер_кода");
    run.expect(3, "function compute");
    run.ok(4);
    run.expect(4, "/app.js");
    run.ok(5);
    run.expect(5, "Главная");
    run.ok(6);
    run.expect(6, "журнал-маяк");
    run.ok(7);
    run.expect(7, "/api/data");
    run.ok(8);
    run.expect(8, "/api/data");
    run.lacks(8, "sekret-cookie-val");
    run.ok(9);
    run.ok(10);
    run.expect(10, "тело-маяк");
}

#[tokio::test(flavor = "multi_thread")]
async fn debugger_pauses_inspects_and_resumes() {
    let env = setup("br-debugger").await;
    let app_js = env.url("/app.js");
    let steps = vec![
        open(&env, "/"),
        call("browser_devtools", json!({ "action": "breakpoint", "url": app_js, "line": COMPUTE_RETURN_LINE })),
        snapshot(),
        click("Считать"),
        call("browser_devtools", json!({ "action": "paused" })),
        call("browser_devtools", json!({ "action": "eval", "expression": "local" })),
        call("browser_devtools", json!({ "action": "eval", "expression": "a+1" })),
        call("browser_devtools", json!({ "action": "resume" })),
    ];
    let run = env.run(steps, |_| true).await;
    let click_ms = run.ms(3);
    let click_text = format!("[{}] {}", run.status(3), run.text(3));

    run.ok(1);
    run.expect(1, "Точка останова");
    run.expect(1, "мест: 1");
    run.ok(2);
    // Результат клика не проверяем: важно, что страница встала.
    run.ok(4);
    run.expect(4, "compute");
    run.expect(4, "local = \"переменная-маяк\"");
    run.ok(5);
    run.expect(5, "переменная-маяк");
    run.ok(6);
    run.expect(6, "2");
    assert_eq!(run.asked("browser_eval"), 2, "eval должен спрашивать каждый раз: {:?}", run.approvals);
    run.ok(7);
    run.expect(7, "res=2");

    // Ход закончился на остановке: страницу не оставляют висеть.
    let run = env.run(vec![open(&env, "/"), click("Считать")], |_| true).await;
    run.ok(0);
    let run = env.run(vec![snapshot()], |_| true).await;
    run.ok(0);
    run.lacks(0, "точке останова");
    run.expect(0, "res=2");

    // Клик, после которого страница встала на точку останова, должен сразу сказать об этом, а не висеть.
    assert!(click_ms < 10_000, "клик по кнопке с точкой останова ждал {click_ms} мс: {click_text}");
}

#[tokio::test(flavor = "multi_thread")]
async fn eval_cookies_and_storage_always_ask() {
    let env = setup("br-secrets").await;
    let steps = vec![
        open(&env, "/"),
        call("browser_devtools", json!({ "action": "eval", "expression": "document.title='hacked'" })),
        snapshot(),
        call("browser_devtools", json!({ "action": "cookies" })),
        call("browser_devtools", json!({ "action": "storage" })),
    ];
    let run = env.run(steps, |a| a["tool"] != "browser_eval").await;

    run.err(1);
    run.expect(1, "не разрешил");
    run.ok(2);
    run.lacks(2, "hacked");
    assert_eq!(run.asked("browser_eval"), 1, "{:?}", run.approvals);
    assert_eq!(run.asked("browser_secrets"), 2, "{:?}", run.approvals);
    run.ok(3);
    run.expect(3, "tok");
    run.ok(4);
    run.expect(4, "local");
    assert_eq!(run.approvals.len(), 3, "{:?}", run.approvals);
}

#[tokio::test(flavor = "multi_thread")]
async fn blocked_addresses_fail_fast() {
    let env = setup("br-blocked").await;
    let bad = ["http://169.254.169.254/", "http://10.0.0.1/", "file:///C:/Windows/win.ini", "chrome://settings"];
    let mut steps = vec![open(&env, "/")];
    for url in bad {
        steps.push(call("browser_nav", json!({ "action": "open", "url": url })));
    }
    steps.push(open(&env, "/redirect"));
    let run = env.run(steps, |_| true).await;

    run.ok(0);
    for (i, url) in bad.iter().enumerate() {
        run.err(i + 1);
        assert!(run.ms(i + 1) < 5_000, "{url}: ответ занял {} мс", run.ms(i + 1));
    }
    run.expect(1, "закрыт");
    run.expect(2, "закрыт");
    let redirect = bad.len() + 1;
    let text = run.text(redirect).to_lowercase();
    assert!(!text.contains("ami-id") && !text.contains("instance-id"), "метаданные просочились:\n{text}");
    assert!(
        run.status(redirect) == "error" || text.contains("blocked") || text.contains("заблок") || text.contains("err_"),
        "редирект на закрытый адрес не помечен как сбой [{}]:\n{text}",
        run.status(redirect)
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn screenshot_is_written_to_the_workspace() {
    let env = setup("br-shot").await;
    let run = env.run(vec![open(&env, "/"), call("browser_page", json!({ "action": "screenshot" }))], |_| true).await;
    run.ok(1);
    run.expect(1, "screenshots/shot-");
    let folder = env.workspace(&env.bot).join("screenshots");
    let files: Vec<_> = std::fs::read_dir(&folder).unwrap_or_else(|e| panic!("нет {}: {e}", folder.display())).flatten().collect();
    assert_eq!(files.len(), 1, "{files:?}");
    assert!(files[0].metadata().unwrap().len() > 1000, "снимок пустой");
}

#[tokio::test(flavor = "multi_thread")]
async fn download_lands_in_the_workspace() {
    let env = setup("br-download").await;
    let steps = vec![open(&env, "/"), open(&env, "/download"), call("browser_page", json!({ "action": "wait", "seconds": 4 }))];
    let run = env.run(steps, |_| true).await;

    eprintln!("[download] open /download: [{}] {}", run.status(1), run.text(1));
    let file = env.workspace(&env.bot).join("downloads").join("report.txt");
    assert!(file.is_file(), "нет {}", file.display());
    assert_eq!(std::fs::read_to_string(&file).unwrap(), "содержимое-отчёта");
    let mentioned = run.text(1).contains("report.txt") || run.text(2).contains("report.txt");
    assert!(mentioned, "снимок не упомянул загрузку.\nopen [{}]: {}\nwait [{}]: {}", run.status(1), run.text(1), run.status(2), run.text(2));
    run.ok(2);
}

#[tokio::test(flavor = "multi_thread")]
async fn upload_asks_and_stays_in_the_workspace() {
    let env = setup("br-upload").await;
    let workspace = env.workspace(&env.bot);
    std::fs::create_dir_all(&workspace).unwrap();
    std::fs::write(workspace.join("up.txt"), "данные").unwrap();

    let steps = vec![
        open(&env, "/form"),
        with("browser_page", |last| json!({ "action": "upload", "ref": ref_of(last, "Файл"), "path": "up.txt" })),
        with("browser_page", |last| json!({ "action": "upload", "ref": ref_of(last, "Файл"), "path": "../../state.json" })),
        snapshot(),
        with("browser_page", |last| json!({ "action": "upload", "ref": ref_of(last, "Файл"), "path": "C:\\Windows\\win.ini" })),
    ];
    let run = env.run(steps, |_| true).await;

    run.ok(1);
    run.expect(1, "файл:up.txt");
    assert_eq!(run.asked("browser_upload"), 1, "{:?}", run.approvals);
    assert!(run.approvals[0]["detail"].as_str().unwrap().contains("up.txt"));
    run.err(2);
    run.expect(2, "вне воркспейса");
    run.err(4);
    run.expect(4, "вне воркспейса");
    assert_eq!(run.asked("browser_upload"), 1, "путь вне воркспейса не должен доходить до вопроса");
}

#[tokio::test(flavor = "multi_thread")]
async fn handoff_to_the_human() {
    let env = setup("br-human").await;
    let human = || call("browser_human", json!({ "message": "войди в кабинет" }));

    let run = env.run(vec![open(&env, "/"), human()], |_| true).await;
    assert_eq!(run.approvals.len(), 1, "{:?}", run.approvals);
    assert_eq!(run.approvals[0]["tool"], "browser_human");
    assert_eq!(run.approvals[0]["allowLabel"], "Готово");
    assert_eq!(run.approvals[0]["denyLabel"], "Не буду");
    run.ok(1);
    assert!(run.text(1).starts_with("Человек закончил"), "{}", run.text(1));
    run.expect(1, "Главная");

    let run = env.run(vec![human()], |_| false).await;
    run.err(0);

    let (status, body) = env.crew.send("POST", &format!("/api/bots/{}/browser", env.bot), None).await;
    eprintln!("[handoff] POST /browser → {status} {body}");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_sign_in_page_asks_to_open_the_screen_once() {
    let env = setup("br-signin").await;
    let needs = |run: &Run| run.events.iter().filter(|e| e["type"] == "browser_needs_human").cloned().collect::<Vec<_>>();

    // Страница входа: одно событие, хотя бот смотрит её дважды. Ничего не нажато, управление не взято.
    let run = env.run(vec![open(&env, "/login"), snapshot(), open(&env, "/"), open(&env, "/login")], |_| true).await;
    let events = needs(&run);
    assert_eq!(events.len(), 2, "один раз на страницу, снова после ухода и возврата: {events:?}");
    assert_eq!(events[0]["botId"], env.bot.as_str());
    assert!(events[0]["url"].as_str().unwrap().ends_with("/login"), "{}", events[0]);
    assert!(events[0]["reason"].as_str().unwrap().contains("вход"), "{}", events[0]);
    assert!(run.events.iter().all(|e| e["type"] != "screen_control"), "панель открывается без захвата управления");
    assert!(run.approvals.is_empty(), "{:?}", run.approvals);

    // Обычная страница сигнала не даёт.
    let run = env.run(vec![open(&env, "/form")], |_| true).await;
    assert!(needs(&run).is_empty(), "{:?}", run.events);
}

#[tokio::test(flavor = "multi_thread")]
async fn show_to_human_only_opens_the_panel() {
    let env = setup("br-show").await;
    let run = env.run(vec![call("browser_show_to_human", json!({ "reason": "смотри, QR-код" }))], |_| true).await;
    run.ok(0);
    assert!(run.approvals.is_empty(), "карточек нет: инструмент прав не даёт {:?}", run.approvals);
    let event = run.events.iter().find(|e| e["type"] == "browser_needs_human").expect("событие для панели");
    assert_eq!(event["reason"], "смотри, QR-код");
    assert_eq!(event["asked"], true);
    assert!(run.events.iter().all(|e| e["type"] != "screen_control"), "управление не берётся");
}

#[tokio::test(flavor = "multi_thread")]
async fn old_snapshots_are_dropped_from_the_model_context() {
    let env = setup("br-tokens").await;
    let run = env.run(vec![open(&env, "/"), snapshot(), snapshot()], |_| true).await;
    run.ok(2);

    let payload = env.fake.last_payload();
    let tools: Vec<String> = payload["messages"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|m| m["role"] == "tool")
        .map(|m| m["content"].as_str().unwrap_or_default().to_string())
        .collect();
    assert_eq!(tools.len(), 3, "{tools:?}");
    assert_eq!(tools[0], "[снимок страницы устарел]");
    assert_eq!(tools[1], "[снимок страницы устарел]");
    assert!(tools[2].contains("Главная") && tools[2].contains("[1] "), "{}", tools[2]);
}

#[tokio::test(flavor = "multi_thread")]
async fn bots_do_not_share_cookies() {
    let env = setup("br-isolation").await;
    let second = env.new_bot("Второй").await;

    let run = env.run(vec![open(&env, "/setcookie"), open(&env, "/showcookie")], |_| true).await;
    run.ok(1);
    run.expect(1, "cookie:sid=bot0-secret");

    let run = env.run_bot(&second, vec![open(&env, "/showcookie")], |_| true).await;
    run.ok(0);
    run.expect(0, "cookie:");
    run.lacks(0, "bot0-secret");
    run.lacks(0, "sid=");
}

#[tokio::test(flavor = "multi_thread")]
async fn profile_folder_stays_after_a_run() {
    let env = setup("br-profile").await;
    let run = env.run(vec![open(&env, "/setcookie")], |_| true).await;
    run.ok(0);
    let profile = env.crew.data_dir.join("browser").join(&env.bot);
    assert!(profile.is_dir(), "нет профиля {}", profile.display());
    assert!(std::fs::read_dir(&profile).unwrap().next().is_some(), "профиль пуст");
}

fn chrome_running_with(needle: &str) -> bool {
    let script = "Get-CimInstance Win32_Process -Filter \"Name='chrome.exe'\" | ForEach-Object { $_.CommandLine }";
    let out = Command::new("powershell").args(["-NoProfile", "-Command", script]).output().expect("powershell");
    String::from_utf8_lossy(&out.stdout).contains(needle)
}

#[tokio::test(flavor = "multi_thread")]
async fn removing_a_bot_closes_its_browser_and_deletes_the_profile() {
    let env = setup("br-remove").await;
    let second = env.new_bot("Лишний").await;
    let run = env.run_bot(&second, vec![open(&env, "/")], |_| true).await;
    run.ok(0);

    let profile = env.crew.data_dir.join("browser").join(&second);
    assert!(profile.is_dir(), "профиль не создался");
    let needle = format!("browser\\{second}");
    assert!(chrome_running_with(&needle), "браузер бота не запущен");

    let (status, body) = env.crew.send("DELETE", &format!("/api/bots/{second}"), None).await;
    assert_eq!(status, 200, "{body}");
    assert!(!profile.exists(), "профиль остался: {}", profile.display());
    let started = Instant::now();
    while chrome_running_with(&needle) {
        assert!(started.elapsed() < Duration::from_secs(8), "браузер удалённого бота ещё жив");
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
}

// ---------------------------------------------------------------------------
// Лёгкий браузер Obscura
// ---------------------------------------------------------------------------

/// Папка с obscura.exe и obscura-worker.exe. Без неё сценарии с настоящим Obscura пропускаются:
/// cargo test --test surface browser:: obscura (с CREW_TEST_OBSCURA_DIR=<папка распакованного архива>)
fn obscura_source() -> Option<std::path::PathBuf> {
    let dir = std::path::PathBuf::from(std::env::var_os("CREW_TEST_OBSCURA_DIR")?);
    (dir.join("obscura.exe").is_file() && dir.join("obscura-worker.exe").is_file()).then_some(dir)
}

/// Стенд, где Obscura «уже скачан»: файлы кладутся туда же, куда их положила бы установка.
async fn setup_obscura(name: &str) -> Option<Env> {
    let Some(source) = obscura_source() else {
        eprintln!("[{name}] пропущен: нет CREW_TEST_OBSCURA_DIR");
        return None;
    };
    let env = setup(name).await;
    let target = env.crew.data_dir.join("engines").join("obscura-0.2.6");
    std::fs::create_dir_all(&target).unwrap();
    for file in ["obscura.exe", "obscura-worker.exe"] {
        std::fs::copy(source.join(file), target.join(file)).unwrap();
    }
    let status = env.crew.ok("GET", "/api/browser/obscura", None).await;
    assert_eq!(status["installed"], true, "{status}");
    let saved = env.crew.ok("PUT", "/api/settings", Some(json!({ "browserEngine": "obscura" }))).await;
    assert_eq!(saved["settings"]["browserEngine"], "obscura");
    Some(env)
}

#[tokio::test(flavor = "multi_thread")]
async fn obscura_cannot_be_chosen_before_it_is_downloaded() {
    let env = setup("br-obscura-missing").await;
    let status = env.crew.ok("GET", "/api/browser/obscura", None).await;
    assert_eq!(status["installed"], false, "{status}");
    assert_eq!(status["state"], "idle", "{status}");

    let (code, body) = env.crew.send("PUT", "/api/settings", Some(json!({ "browserEngine": "obscura" }))).await;
    assert_eq!(code, 400, "{body}");
    assert!(body.to_string().contains("скачай"), "{body}");
    let (code, body) = env.crew.send("PUT", "/api/settings", Some(json!({ "browserEngine": "opera" }))).await;
    assert_eq!(code, 400, "{body}");

    let state = env.crew.ok("PUT", "/api/settings", Some(json!({ "browserEngine": "chrome" }))).await;
    assert_eq!(state["settings"]["browserEngine"], "chrome");
    // Chrome по умолчанию остаётся рабочим.
    let run = env.run(vec![open(&env, "/")], |_| true).await;
    run.ok(0);
    run.expect(0, "Главная");
}

#[tokio::test(flavor = "multi_thread")]
async fn obscura_opens_clicks_and_types() {
    let Some(env) = setup_obscura("br-obscura-work").await else { return };
    let steps = vec![
        open(&env, "/"),
        click("Показать"),
        open(&env, "/form"),
        with("browser_keyboard", |last| json!({ "action": "type", "ref": ref_of(last, "Запрос"), "text": "привет мир" })),
        with("browser_keyboard", |last| json!({ "action": "type", "ref": ref_of(last, "Пароль"), "text": "hunter2" })),
        call("browser_page", json!({ "action": "screenshot" })),
    ];
    let run = env.run(steps, |_| true).await;

    run.ok(0);
    run.expect(0, "Главная");
    run.ok(1);
    run.expect(1, "Показано-OK");
    run.ok(3);
    run.expect(3, "keys=10");
    run.err(4);
    run.expect(4, "browser_human");
    run.ok(5);
    let shots = env.workspace(&env.bot).join("screenshots");
    assert_eq!(std::fs::read_dir(&shots).unwrap().count(), 1);
    assert!(env.crew.data_dir.join("browser-obscura").join(&env.bot).is_dir(), "профиль Obscura не создался");
}

#[tokio::test(flavor = "multi_thread")]
async fn obscura_keeps_closed_addresses_closed() {
    let Some(env) = setup_obscura("br-obscura-blocked").await else { return };
    let bad = ["http://169.254.169.254/", "http://10.0.0.1/"];
    let mut steps = vec![open(&env, "/")];
    for url in bad {
        steps.push(call("browser_nav", json!({ "action": "open", "url": url })));
    }
    let run = env.run(steps, |_| true).await;
    run.ok(0);
    for i in 1..=bad.len() {
        run.err(i);
        run.expect(i, "закрыт");
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn obscura_sends_the_human_to_chrome_and_removal_switches_back() {
    let Some(env) = setup_obscura("br-obscura-human").await else { return };
    let run = env.run(vec![open(&env, "/"), call("browser_human", json!({ "message": "войди в кабинет" }))], |_| true).await;
    run.ok(0);
    run.err(1);
    run.expect(1, "Chrome");
    assert_eq!(run.asked("browser_human"), 0, "без окна человека звать некуда: {:?}", run.approvals);

    let (code, body) = env.crew.send("POST", &format!("/api/bots/{}/browser", env.bot), None).await;
    assert_eq!(code, 500, "{body}");
    assert!(body.to_string().contains("Chrome"), "{body}");

    let removed = env.crew.ok("DELETE", "/api/browser/obscura", None).await;
    assert_eq!(removed["settings"]["browserEngine"], "chrome", "{removed}");
    assert_eq!(removed["status"]["installed"], false, "{removed}");
    assert!(!env.crew.data_dir.join("engines").join("obscura-0.2.6").exists());

    let run = env.run(vec![open(&env, "/")], |_| true).await;
    run.ok(0);
    run.expect(0, "Главная");
}

// ---------------------------------------------------------------------------
// Лёгкий браузер Moli
// ---------------------------------------------------------------------------

fn moli_exe_name() -> &'static str {
    if cfg!(windows) {
        "moli.exe"
    } else {
        "moli"
    }
}

/// Папка с moli(.exe) из распакованного архива. Без неё сценарии с настоящим Moli пропускаются:
/// cargo test --test surface browser:: moli (с CREW_TEST_MOLI_DIR=<папка moli-v1.1.24-…>)
fn moli_source() -> Option<std::path::PathBuf> {
    let dir = std::path::PathBuf::from(std::env::var_os("CREW_TEST_MOLI_DIR")?);
    dir.join(moli_exe_name()).is_file().then_some(dir)
}

/// Стенд, где Moli «уже скачан»: файл кладётся туда же, куда его положила бы установка.
async fn setup_moli(name: &str) -> Option<Env> {
    let Some(source) = moli_source() else {
        eprintln!("[{name}] пропущен: нет CREW_TEST_MOLI_DIR");
        return None;
    };
    let env = setup(name).await;
    let target = env.crew.data_dir.join("engines").join("moli-1.1.24");
    std::fs::create_dir_all(&target).unwrap();
    std::fs::copy(source.join(moli_exe_name()), target.join(moli_exe_name())).unwrap();
    let status = env.crew.ok("GET", "/api/browser/moli", None).await;
    assert_eq!(status["installed"], true, "{status}");
    let saved = env.crew.ok("PUT", "/api/settings", Some(json!({ "browserEngine": "moli" }))).await;
    assert_eq!(saved["settings"]["browserEngine"], "moli");
    Some(env)
}

#[tokio::test(flavor = "multi_thread")]
async fn moli_cannot_be_chosen_before_it_is_downloaded() {
    let env = setup("br-moli-missing").await;
    let status = env.crew.ok("GET", "/api/browser/moli", None).await;
    assert_eq!(status["installed"], false, "{status}");
    assert_eq!(status["state"], "idle", "{status}");

    let (code, body) = env.crew.send("PUT", "/api/settings", Some(json!({ "browserEngine": "moli" }))).await;
    assert_eq!(code, 400, "{body}");
    assert!(body.to_string().contains("скачай"), "{body}");
    let (code, body) = env.crew.send("PUT", "/api/settings", Some(json!({ "browserEngine": "opera" }))).await;
    assert_eq!(code, 400, "{body}");

    let state = env.crew.ok("PUT", "/api/settings", Some(json!({ "browserEngine": "chrome" }))).await;
    assert_eq!(state["settings"]["browserEngine"], "chrome");
    // Chrome по умолчанию остаётся рабочим.
    let run = env.run(vec![open(&env, "/")], |_| true).await;
    run.ok(0);
    run.expect(0, "Главная");
}

#[tokio::test(flavor = "multi_thread")]
async fn moli_opens_clicks_and_types() {
    let Some(env) = setup_moli("br-moli-work").await else { return };
    let steps = vec![
        open(&env, "/"),
        click("Показать"),
        open(&env, "/form"),
        with("browser_keyboard", |last| json!({ "action": "type", "ref": ref_of(last, "Запрос"), "text": "привет мир" })),
        with("browser_keyboard", |last| json!({ "action": "type", "ref": ref_of(last, "Пароль"), "text": "hunter2" })),
    ];
    let run = env.run(steps, |_| true).await;

    run.ok(0);
    run.expect(0, "Главная");
    run.ok(1);
    run.expect(1, "Показано-OK");
    run.ok(3);
    run.expect(3, "keys=10");
    run.err(4);
    run.expect(4, "browser_human");
    assert!(env.crew.data_dir.join("browser-moli").join(&env.bot).is_dir(), "хранилище Moli не создалось");
}

#[tokio::test(flavor = "multi_thread")]
async fn moli_keeps_closed_addresses_closed() {
    let Some(env) = setup_moli("br-moli-blocked").await else { return };
    let bad = ["http://169.254.169.254/", "http://10.0.0.1/"];
    let mut steps = vec![open(&env, "/")];
    for url in bad {
        steps.push(call("browser_nav", json!({ "action": "open", "url": url })));
    }
    let run = env.run(steps, |_| true).await;
    run.ok(0);
    for i in 1..=bad.len() {
        run.err(i);
        run.expect(i, "закрыт");
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn moli_sends_the_human_to_chrome_and_removal_switches_back() {
    let Some(env) = setup_moli("br-moli-human").await else { return };
    let run = env.run(vec![open(&env, "/"), call("browser_human", json!({ "message": "войди в кабинет" }))], |_| true).await;
    run.ok(0);
    run.err(1);
    run.expect(1, "Chrome");
    assert_eq!(run.asked("browser_human"), 0, "без окна человека звать некуда: {:?}", run.approvals);

    let (code, body) = env.crew.send("POST", &format!("/api/bots/{}/browser", env.bot), None).await;
    assert_eq!(code, 500, "{body}");
    assert!(body.to_string().contains("Chrome"), "{body}");

    let removed = env.crew.ok("DELETE", "/api/browser/moli", None).await;
    assert_eq!(removed["settings"]["browserEngine"], "chrome", "{removed}");
    assert_eq!(removed["status"]["installed"], false, "{removed}");
    assert!(!env.crew.data_dir.join("engines").join("moli-1.1.24").exists());

    let run = env.run(vec![open(&env, "/")], |_| true).await;
    run.ok(0);
    run.expect(0, "Главная");
}

// ---------------------------------------------------------------------------
// Экран бота и «Научи бота»
// ---------------------------------------------------------------------------

/// Страница с элементами на известных местах: клик по координатам проверяется по тому, что нажалось.
const TEACH: &str = r##"<!doctype html><html lang="ru"><head><meta charset="utf-8"><title>Кабинет</title>
<style>body{margin:0}.b{position:absolute;box-sizing:border-box}</style></head><body>
<button class="b" style="left:100px;top:100px;width:200px;height:60px" onclick="document.getElementById('st').textContent='цель-нажата'">Кнопка-цель</button>
<button class="b" style="left:320px;top:100px;width:200px;height:60px" onclick="document.getElementById('st').textContent='мимо'">Соседняя</button>
<input class="b" id="order" aria-label="Номер заказа" style="left:100px;top:250px;width:300px;height:40px">
<input class="b" type="password" aria-label="Пароль" style="left:100px;top:330px;width:300px;height:40px">
<button class="b" data-testid="export" style="left:100px;top:420px;width:200px;height:50px" onclick="document.getElementById('st').textContent='экспорт:'+document.getElementById('order').value">Экспорт</button>
<div class="b" id="st" style="left:100px;top:520px">ждём</div>
</body></html>"##;

async fn start_teach_site() -> String {
    let app = Router::new().route("/teach", get(|| async { Html(TEACH) }));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    format!("http://127.0.0.1:{port}/teach")
}

fn b64(input: &str) -> Vec<u8> {
    let value = |c: u8| match c {
        b'A'..=b'Z' => Some(c - b'A'),
        b'a'..=b'z' => Some(c - b'a' + 26),
        b'0'..=b'9' => Some(c - b'0' + 52),
        b'+' => Some(62),
        b'/' => Some(63),
        _ => None,
    };
    let (mut out, mut buffer, mut bits) = (Vec::new(), 0u32, 0u32);
    for c in input.bytes().filter_map(value) {
        buffer = (buffer << 6) | c as u32;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((buffer >> bits) as u8);
        }
    }
    out
}

/// Размер JPEG в точках: из маркера SOF.
fn jpeg_size(bytes: &[u8]) -> (f64, f64) {
    let mut i = 2;
    while i + 9 < bytes.len() {
        if bytes[i] != 0xFF {
            i += 1;
            continue;
        }
        let marker = bytes[i + 1];
        let len = u16::from_be_bytes([bytes[i + 2], bytes[i + 3]]) as usize;
        if matches!(marker, 0xC0 | 0xC1 | 0xC2) {
            let h = u16::from_be_bytes([bytes[i + 5], bytes[i + 6]]) as f64;
            let w = u16::from_be_bytes([bytes[i + 7], bytes[i + 8]]) as f64;
            return (w, h);
        }
        i += 2 + len;
    }
    panic!("в кадре нет размера JPEG");
}

/// Поток экрана бота, как его читает панель.
async fn screen_feed(env: &Env) -> Arc<Mutex<Vec<Value>>> {
    screen_feed_task(env).await.0
}

/// То же, но с задачей чтения: её остановка — это закрытая панель.
async fn screen_feed_task(env: &Env) -> (Arc<Mutex<Vec<Value>>>, tokio::task::JoinHandle<()>) {
    let res = env.crew.client.get(format!("{}/api/bots/{}/screen", env.crew.base, env.bot)).header("cookie", &env.crew.cookie).send().await.unwrap();
    assert_eq!(res.status().as_u16(), 200, "поток экрана");
    let events = Arc::new(Mutex::new(Vec::new()));
    let sink = events.clone();
    let task = tokio::spawn(async move {
        use futures_util::StreamExt;
        let mut stream = res.bytes_stream();
        let mut buf: Vec<u8> = Vec::new();
        while let Some(Ok(chunk)) = stream.next().await {
            buf.extend_from_slice(&chunk);
            while let Some(i) = buf.windows(2).position(|w| w == b"\n\n") {
                let part: Vec<u8> = buf.drain(..i + 2).collect();
                for line in String::from_utf8_lossy(&part).lines() {
                    if let Some(data) = line.strip_prefix("data:") {
                        if let Ok(event) = serde_json::from_str::<Value>(data.trim()) {
                            sink.lock().unwrap().push(event);
                        }
                    }
                }
            }
        }
    });
    (events, task)
}

async fn wait_screen(events: &Arc<Mutex<Vec<Value>>>, label: &str, pred: impl Fn(&Value) -> bool) -> Value {
    let started = Instant::now();
    loop {
        if let Some(found) = events.lock().unwrap().iter().rev().find(|e| pred(e)).cloned() {
            return found;
        }
        if started.elapsed() > Duration::from_secs(30) {
            let seen: Vec<String> = events.lock().unwrap().iter().map(|e| e["type"].to_string()).collect();
            panic!("экран: не дождался {label}; было: {}", seen.join(", "));
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

/// Кадр и доля картинки для точки страницы в CSS-пикселях — так считает панель.
struct Frame {
    meta: Value,
    iw: f64,
    ih: f64,
}

impl Frame {
    fn at(&self, x: f64, y: f64) -> Value {
        let width = self.meta["deviceWidth"].as_f64().unwrap();
        let top = self.meta["offsetTop"].as_f64().unwrap_or(0.0);
        // Высота картинки в CSS-пикселях: по ширине, как у сервера.
        let height = self.ih * width / self.iw;
        json!({ "x": x / width, "y": (y + top) / height, "iw": self.iw, "ih": self.ih })
    }
}

async fn first_frame(events: &Arc<Mutex<Vec<Value>>>) -> Frame {
    let frame = wait_screen(events, "кадр", |e| e["type"] == "frame").await;
    let (iw, ih) = jpeg_size(&b64(frame["data"].as_str().unwrap()));
    assert!(iw > 100.0 && ih > 100.0, "кадр {iw}×{ih}");
    Frame { meta: frame["meta"].clone(), iw, ih }
}

impl Env {
    async fn screen(&self, path: &str, body: Value) -> (u16, Value) {
        self.crew.send("POST", &format!("/api/bots/{}/screen/{path}", self.bot), Some(body)).await
    }

    async fn screen_ok(&self, path: &str, body: Value) -> Value {
        let (status, res) = self.screen(path, body).await;
        assert_eq!(status, 200, "screen/{path}: {res}");
        res
    }

    async fn human_click(&self, frame: &Frame, x: f64, y: f64) {
        let mut body = frame.at(x, y);
        body["kind"] = json!("click");
        self.screen_ok("input", body).await;
    }

    async fn human_types(&self, text: &str) {
        for ch in text.chars() {
            let code = if ch.is_ascii_digit() { format!("Digit{ch}") } else { format!("Key{}", ch.to_ascii_uppercase()) };
            let body = json!({ "kind": "key", "key": ch.to_string(), "code": code, "keyCode": ch.to_ascii_uppercase() as u32 });
            self.screen_ok("input", body).await;
        }
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn screen_streams_frames_and_forwards_the_human_click() {
    let env = setup("br-screen").await;
    let page = start_teach_site().await;
    let run = env.run(vec![call("browser_nav", json!({ "action": "open", "url": page }))], |_| true).await;
    run.ok(0);
    // Строка «что делает бот» приходит в общий поток.
    assert!(env.crew.find_event(|e| e["type"] == "screen_step" && e["text"].as_str().is_some_and(|t| t.starts_with("Открывает"))).is_some());

    let feed = screen_feed(&env).await;
    let frame = first_frame(&feed).await;
    let tabs = wait_screen(&feed, "вкладки", |e| e["type"] == "tabs" && e["url"].as_str().is_some_and(|u| u.ends_with("/teach"))).await;
    assert_eq!(tabs["title"], "Кабинет", "{tabs}");

    // Без управления ввод не проходит.
    let mut body = frame.at(200.0, 130.0);
    body["kind"] = json!("click");
    let (status, res) = env.screen("input", body).await;
    assert_eq!(status, 409, "{res}");

    // Человек держит управление — ход бота ждёт перед следующим инструментом.
    env.screen_ok("control", json!({ "hold": true })).await;
    let before = env.crew.count_events(|e| e["type"] == "tool_update" && e["botId"] == env.bot.as_str());
    let human = async {
        env.crew.wait_event("шаг ждёт", |e| e["type"] == "tool_start" && e["botId"] == env.bot.as_str() && e["name"] == "browser_page").await;
        tokio::time::sleep(Duration::from_millis(1200)).await;
        let finished = env.crew.count_events(|e| e["type"] == "tool_update" && e["botId"] == env.bot.as_str());
        env.human_click(&frame, 200.0, 130.0).await;
        tokio::time::sleep(Duration::from_millis(300)).await;
        env.screen_ok("control", json!({ "hold": false })).await;
        finished
    };
    let (run, during) = tokio::join!(env.run(vec![snapshot()], |_| true), human);
    assert_eq!(during, before, "пока управление у человека, инструмент не должен был выполниться");
    run.ok(0);
    assert!(run.ms(0) >= 1000, "шаг не ждал человека: {} мс", run.ms(0));
    run.expect(0, "цель-нажата");
    run.lacks(0, "мимо");
}

#[tokio::test(flavor = "multi_thread")]
async fn teach_records_steps_hides_the_password_and_saves_a_skill() {
    let env = setup("br-teach").await;
    let page = start_teach_site().await;
    env.fake.set_service(
        &json!({
            "name": "Выгрузка заказа",
            "description": "когда нужен отчёт по заказу",
            "steps": [
                { "do": "Открой страницу кабинета", "check": "видна кнопка «Экспорт»" },
                { "do": "Введи номер заказа в поле «Номер заказа»", "check": "номер в поле" },
                { "do": "Пароль вводит человек: browser_human" },
                { "do": "Нажми «Экспорт»", "check": "появилось «экспорт:» с номером" },
            ],
            "human": ["ввести пароль"],
        })
        .to_string(),
    );
    let run = env.run(vec![call("browser_nav", json!({ "action": "open", "url": page }))], |_| true).await;
    run.ok(0);
    let feed = screen_feed(&env).await;
    let frame = first_frame(&feed).await;

    let (status, _) = env.screen("teach/start", json!({})).await;
    assert_eq!(status, 409, "запись без управления не начинается");
    env.screen_ok("control", json!({ "hold": true })).await;
    let started = env.screen_ok("teach/start", json!({})).await;
    assert_eq!(started["steps"][0]["kind"], "open", "{started}");

    env.human_click(&frame, 250.0, 270.0).await;
    env.human_types("87654321").await;
    env.human_click(&frame, 250.0, 350.0).await;
    env.human_types("hunter2").await;
    env.screen_ok("teach/note", json!({ "text": "пароль от кабинета" })).await;
    env.human_click(&frame, 200.0, 445.0).await;
    tokio::time::sleep(Duration::from_millis(400)).await;

    let done = env.screen_ok("teach/stop", json!({})).await;
    let text = done.to_string();
    assert!(!text.contains("hunter2"), "пароль попал в запись: {text}");
    let steps = done["steps"].as_array().unwrap();
    let kinds: Vec<&str> = steps.iter().map(|s| s["kind"].as_str().unwrap()).collect();
    assert_eq!(kinds, ["open", "click", "type", "click", "secret", "click"], "{text}");
    let brief: Vec<String> = steps.iter().map(|s| format!("{} {} {}", s["kind"], s["target"], s["value"])).collect();
    assert_eq!(steps[1]["target"]["label"], "Номер заказа", "{}", brief.join("
"));
    assert_eq!(steps[1]["target"]["selector"], "#order");
    assert_eq!(steps[2]["value"], "87654321");
    assert_eq!(steps[4]["value"], "");
    assert_eq!(steps[4]["target"]["secret"], "пароль");
    assert_eq!(steps[4]["note"], "пароль от кабинета");
    assert_eq!(steps[5]["target"]["label"], "Экспорт");
    assert_eq!(steps[5]["target"]["selector"], "[data-testid=\"export\"]");
    assert!(steps.iter().all(|s| s["thumb"].as_str().is_some_and(|t| t.len() > 100)), "у шага нет кадра");

    // Модель видела запись без пароля и без разового номера.
    let asked = env.fake.payloads().into_iter().find(|p| p.to_string().contains("Запись действий человека")).expect("служебного вызова не было");
    let asked = asked.to_string();
    assert!(!asked.contains("hunter2") && !asked.contains("87654321"), "{asked}");
    assert!(asked.contains("<номер>") && asked.contains("человек ввёл пароль"), "{asked}");

    let draft = &done["draft"];
    assert_eq!(draft["name"], "Выгрузка заказа", "{done}");
    assert!(draft["body"].as_str().unwrap().contains("1. Открой страницу кабинета\n   Проверить: видна кнопка «Экспорт»"), "{draft}");
    assert!(draft["body"].as_str().unwrap().contains("Где нужен человек:\n- ввести пароль"), "{draft}");
    assert!(done["warning"].is_null(), "{done}");

    // Запись живёт до сохранения: оборвался ответ — «Стоп» можно нажать ещё раз, шаги те же.
    let again = env.screen_ok("teach/stop", json!({})).await;
    assert_eq!(again["steps"].as_array().unwrap().len(), steps.len());
    let saved = env.screen_ok("teach/save", draft.clone()).await;
    let (status, _) = env.screen("teach/stop", json!({})).await;
    assert_eq!(status, 409, "после сохранения записи нет");
    let skill_id = saved["skill"]["id"].as_str().unwrap().to_string();
    assert_eq!(saved["created"], true);
    let state = env.crew.ok("GET", "/api/state", None).await;
    let bot = state["bots"].as_array().unwrap().iter().find(|b| b["id"] == env.bot.as_str()).unwrap().clone();
    assert!(bot["skills"].as_array().unwrap().iter().any(|s| s == skill_id.as_str()), "навык не подключён боту: {bot}");
    assert!(env.crew.find_event(|e| e["type"] == "skill_learned" && e["skillId"] == skill_id.as_str()).is_some());
    env.screen_ok("control", json!({ "hold": false })).await;

    // Бот выполняет новый навык на той же странице: навык у него в промпте, шаги проходят.
    let steps = vec![
        call("browser_nav", json!({ "action": "open", "url": page })),
        with("browser_keyboard", |last| json!({ "action": "type", "ref": ref_of(last, "Номер заказа"), "text": "555" })),
        click("Экспорт"),
    ];
    let run = env.run(steps, |_| true).await;
    run.ok(1);
    run.ok(2);
    run.expect(2, "экспорт:555");
    let system = env.fake.last_payload()["messages"][0]["content"].as_str().unwrap_or_default().to_string();
    assert!(system.contains("Выгрузка заказа"), "навыка нет в промпте бота");
}

#[tokio::test(flavor = "multi_thread")]
async fn run_command_output_streams_to_the_screen_only() {
    let env = setup("br-shell").await;
    let feed = screen_feed(&env).await;
    let run = env.run(vec![call("run_command", json!({ "command": "echo screen-beacon" }))], |_| true).await;
    run.ok(0);
    let start = wait_screen(&feed, "shell_start", |e| e["type"] == "shell_start").await;
    assert_eq!(start["command"], "echo screen-beacon");
    let end = wait_screen(&feed, "shell_end", |e| e["type"] == "shell_end").await;
    assert_eq!(end["code"], 0, "{end}");
    let streamed: String = feed
        .lock()
        .unwrap()
        .iter()
        .filter(|e| e["type"] == "shell_output")
        .map(|e| e["text"].as_str().unwrap_or_default().to_string())
        .collect();
    assert!(streamed.contains("screen-beacon"), "вывод не дошёл до экрана: {streamed:?}");
    // В общий поток (док, питомцы, внешние клиенты) ни команда, ни вывод не попадают.
    let leaked = env.crew.events.lock().unwrap().iter().any(|e| {
        e["type"].as_str().is_some_and(|t| t.starts_with("shell_") && t != "shell_done")
    });
    assert!(!leaked, "команда ушла в общий поток событий");
    assert!(env.crew.find_event(|e| e["type"] == "shell_done" && e["botId"] == env.bot.as_str()).is_some());
    let (status, _) = env.screen("shell/stop", json!({})).await;
    assert_eq!(status, 404, "команда уже закончилась");
}

#[tokio::test(flavor = "multi_thread")]
async fn shell_stop_ends_only_the_command() {
    let env = setup("br-shell-stop").await;
    let feed = screen_feed(&env).await;
    let stopper = async {
        wait_screen(&feed, "shell_start", |e| e["type"] == "shell_start").await;
        tokio::time::sleep(Duration::from_millis(500)).await;
        env.screen_ok("shell/stop", json!({})).await;
    };
    let steps = vec![call("run_command", json!({ "command": "ping -n 30 127.0.0.1" })), call("list_bots", json!({}))];
    let (run, ()) = tokio::join!(env.run(steps, |_| true), stopper);
    run.expect(0, "остановлена пользователем");
    // Ход продолжился: следующий инструмент выполнился.
    run.ok(1);
    let end = wait_screen(&feed, "shell_end", |e| e["type"] == "shell_end").await;
    assert_eq!(end["stopped"], true, "{end}");
}

#[tokio::test(flavor = "multi_thread")]
async fn obscura_screen_shows_a_picture_or_says_it_is_unavailable() {
    let Some(env) = setup_obscura("br-obscura-screen").await else { return };
    let run = env.run(vec![open(&env, "/")], |_| true).await;
    run.ok(0);
    let feed = screen_feed(&env).await;
    let seen = wait_screen(&feed, "кадр или отказ", |e| e["type"] == "frame" || e["type"] == "unavailable").await;
    eprintln!("[obscura-screen] {}", seen["type"]);
    if seen["type"] == "unavailable" {
        assert!(seen["message"].as_str().unwrap().contains("недоступен"), "{seen}");
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn stop_while_the_human_holds_control_runs_nothing() {
    let env = setup("br-hold-stop").await;
    let feed = screen_feed(&env).await;
    first_frame(&feed).await;
    env.screen_ok("control", json!({ "hold": true })).await;
    let marker = env.workspace(&env.bot).join("marker.txt");
    let stopper = async {
        env.crew.wait_event("шаг ждёт", |e| e["type"] == "tool_start" && e["botId"] == env.bot.as_str()).await;
        tokio::time::sleep(Duration::from_millis(500)).await;
        env.crew.ok("POST", &format!("/api/bots/{}/stop", env.bot), None).await;
    };
    let (run, ()) = tokio::join!(env.run(vec![call("run_command", json!({ "command": "echo x > marker.txt" }))], |_| true), stopper);
    assert_ne!(run.status(0), "ok", "{}", run.dump());
    run.expect(0, "Остановлено пользователем");
    tokio::time::sleep(Duration::from_millis(500)).await;
    assert!(!marker.exists(), "команда выполнилась после «Стоп»");
}

#[tokio::test(flavor = "multi_thread")]
async fn control_needs_an_open_screen_and_ends_when_it_closes() {
    let env = setup("br-hold-viewer").await;
    let (status, body) = env.screen("control", json!({ "hold": true })).await;
    assert_eq!(status, 409, "без открытого экрана управление не берётся: {body}");

    let (feed, task) = screen_feed_task(&env).await;
    first_frame(&feed).await;
    env.screen_ok("control", json!({ "hold": true })).await;
    env.crew.wait_event("управление у человека", |e| e["type"] == "screen_control" && e["botId"] == env.bot.as_str() && e["held"] == true).await;
    env.crew.clear_events();
    // Панель закрыли, не вернув управление: оно возвращается боту само.
    task.abort();
    env.crew.wait_event("управление вернулось", |e| e["type"] == "screen_control" && e["botId"] == env.bot.as_str() && e["held"] == false).await;
    let run = env.run(vec![snapshot()], |_| true).await;
    run.ok(0);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_new_tab_shows_up_on_the_screen() {
    let env = setup("br-screen-tab").await;
    let page = start_teach_site().await;
    let run = env.run(vec![call("browser_nav", json!({ "action": "open", "url": page }))], |_| true).await;
    run.ok(0);
    let feed = screen_feed(&env).await;
    first_frame(&feed).await;
    let second = page.replace("/teach", "/teach?tab=2");
    let run = env.run(vec![call("browser_nav", json!({ "action": "new_tab", "url": second }))], |_| true).await;
    run.ok(0);
    let tabs = wait_screen(&feed, "две вкладки", |e| e["type"] == "tabs" && e["tabs"].as_array().is_some_and(|t| t.len() == 2) && e["url"].as_str().is_some_and(|u| u.ends_with("tab=2"))).await;
    assert_eq!(tabs["tabs"][1]["current"], true, "{tabs}");
    let at = feed.lock().unwrap().iter().position(|e| e == &tabs).unwrap();
    let started = Instant::now();
    loop {
        if feed.lock().unwrap()[at..].iter().any(|e| e["type"] == "frame") {
            break;
        }
        assert!(started.elapsed() < Duration::from_secs(15), "у новой вкладки нет кадров");
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn closing_the_screen_during_browser_human_ends_the_wait() {
    let env = setup_with(
        "br-human-left",
        &[("CREW_BROWSER_HEADLESS", "1"), ("CREW_TEST_ALLOW_LOOPBACK", "1"), ("CREW_TEST_SCREEN_GRACE_MS", "1500")],
    )
    .await;
    let (feed, task) = screen_feed_task(&env).await;
    first_frame(&feed).await;
    env.fake.set_script(vec![open(&env, "/"), call("browser_human", json!({ "message": "войди в кабинет" }))]);
    env.crew.say(&env.bot, "поехали").await;
    let card = env.crew.wait_event("карточка человека", |e| e["type"] == "approval" && e["tool"] == "browser_human").await;
    env.crew.wait_event("управление у человека", |e| e["type"] == "screen_control" && e["held"] == true).await;
    // Человек закрыл экран и не вернулся: карточка закрывается, ход не висит.
    task.abort();
    env.crew.wait_event("карточка закрыта", |e| e["type"] == "approval_closed" && e["id"] == card["id"]).await;
    let update = env
        .crew
        .wait_event("ответ инструмента", |e| e["type"] == "tool_update" && e["result"].as_str().is_some_and(|r| r.contains("закрыл экран")))
        .await;
    assert_eq!(update["status"], "error", "{update}");
    env.crew.wait_event("ход закончился", |e| e["type"] == "turn_end" && e["botId"] == env.bot.as_str()).await;
    assert!(env.crew.find_event(|e| e["type"] == "screen_control" && e["held"] == false).is_some(), "управление не вернулось боту");
}

#[tokio::test(flavor = "multi_thread")]
async fn an_unattended_turn_does_not_wait_for_a_held_screen_forever() {
    let env = setup_with(
        "br-hold-unattended",
        &[("CREW_BROWSER_HEADLESS", "1"), ("CREW_TEST_ALLOW_LOOPBACK", "1"), ("CREW_TEST_UNATTENDED_HOLD_MS", "1500"), ("CREW_TEST_TICK_MS", "100")],
    )
    .await;
    let feed = screen_feed(&env).await;
    first_frame(&feed).await;
    env.screen_ok("control", json!({ "hold": true })).await;
    env.fake.set_script(vec![snapshot()]);
    // Рутина по расписанию — ход без человека.
    let body = json!({ "botId": env.bot, "title": "Проверка", "prompt": "проверь сайт", "everyMinutes": 60 });
    let (status, data) = env.crew.send("POST", "/api/routines", Some(body)).await;
    assert_eq!(status, 201, "{data}");
    let id = data["routine"]["id"].as_str().unwrap().to_string();
    env.crew.ok("PATCH", &format!("/api/routines/{id}"), Some(json!({ "nextRun": common::due_now() }))).await;
    let update = env.crew.wait_event("шаг рутины", |e| e["type"] == "tool_update" && e["botId"] == env.bot.as_str()).await;
    assert_eq!(update["status"], "ok", "{update}");
    assert!(env.crew.find_event(|e| e["type"] == "screen_control" && e["held"] == false).is_some(), "удержание не снято по сроку");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_site_rule_does_not_act_while_the_human_holds_control() {
    let env = setup("br-hold-rule").await;
    // Сайт разрешён «всегда здесь»: дальше на нём карточек нет.
    let run = env.run(vec![open(&env, "/"), click("Показать")], |_| true).await;
    run.ok(1);
    assert_eq!(run.asked("browser_site"), 1, "{:?}", run.approvals);
    let feed = screen_feed(&env).await;
    first_frame(&feed).await;
    env.screen_ok("control", json!({ "hold": true })).await;
    let bot = env.bot.clone();
    let releaser = async {
        env.crew.wait_event("шаг ждёт", |e| e["type"] == "tool_start" && e["botId"] == bot.as_str()).await;
        tokio::time::sleep(Duration::from_millis(1500)).await;
        // Правило не пропускает шаг мимо удержания: шаг всё ещё ждёт человека.
        let done = env.crew.find_event(|e| e["type"] == "tool_update" && e["botId"] == bot.as_str());
        assert!(done.is_none(), "шаг выполнился, пока управление у человека: {done:?}");
        env.screen_ok("control", json!({ "hold": false })).await;
    };
    env.crew.clear_events();
    let (run, ()) = tokio::join!(env.run(vec![open(&env, "/"), click("Показать")], |_| true), releaser);
    run.ok(1);
    assert!(run.approvals.is_empty(), "{:?}", run.approvals);
}

/// Браузер бота стартует и с Chrome, и с Edge, из обычной папки данных (не `C:\X` с меткой
/// Low integrity). Регрессия 11.11.2026: владелец видел «инструмент браузера завершился с
/// ошибкой»; причина была у ACP-агента (его собственный плагин браузера), но и обычный запуск
/// после `--browser-bot` и экрана бота должен остаться рабочим для обоих браузеров.
#[tokio::test(flavor = "multi_thread")]
async fn bot_browser_starts_with_chrome_and_edge() {
    let candidates = [
        (r"C:\Program Files\Google\Chrome\Application\chrome.exe", "chrome"),
        (r"C:\Program Files (x86)\Google\Chrome\Application\chrome.exe", "chrome"),
        (r"C:\Program Files (x86)\Microsoft\Edge\Application\msedge.exe", "edge"),
        (r"C:\Program Files\Microsoft\Edge\Application\msedge.exe", "edge"),
    ];
    let mut tried = Vec::new();
    for (path, kind) in candidates {
        if tried.contains(&kind) || !std::path::Path::new(path).is_file() {
            continue;
        }
        tried.push(kind);
        let env = setup(&format!("br-start-{kind}")).await;
        assert!(!env.crew.data_dir.starts_with(r"C:\X"), r"проверка не должна идти из C:\X");
        env.crew.ok("PUT", "/api/settings", Some(json!({ "browserPath": path }))).await;
        let run = env.run(vec![open(&env, "/")], |_| true).await;
        assert_eq!(run.status(0), "ok", "{kind}: браузер не запустился:\n{}", run.dump());
        run.expect(0, "Главная");
        let dir = env.crew.data_dir.display().to_string();
        let exe = if kind == "edge" { "msedge.exe" } else { "chrome.exe" };
        let script = format!("@(Get-CimInstance Win32_Process -Filter \"name='{exe}'\" | Where-Object {{ $_.CommandLine -like '*{dir}*' }}).Count");
        let found = Command::new("pwsh").args(["-NoProfile", "-Command", &script]).output().unwrap();
        let count: usize = String::from_utf8_lossy(&found.stdout).trim().parse().unwrap_or(0);
        assert!(count > 0, "{kind}: бот открыл страницу, но не через {exe}");
    }
    assert!(!tried.is_empty(), "на машине нет ни Chrome, ни Edge");
}

/// Отпечаток браузера бота на локальной странице: что видит сайт (navigator.webdriver, окно,
/// следы CDP). Вручную: FP_URL=http://127.0.0.1:<порт>/fp.html cargo test --test browser
/// fingerprint_probe -- --ignored --nocapture. Окно настоящее, за краем экрана.
#[tokio::test(flavor = "multi_thread")]
#[ignore = "ручной замер"]
async fn fingerprint_probe() {
    let url = std::env::var("FP_URL").expect("FP_URL");
    let wait = std::env::var("FP_WAIT").unwrap_or_else(|_| "FP {".into());
    let env = setup_with("fp-probe", &[("CREW_TEST_ALLOW_LOOPBACK", "1")]).await;
    let run = env.run(vec![call("browser_nav", json!({ "action": "open", "url": url })), call("browser_page", json!({ "action": "wait", "text": wait.clone(), "seconds": 20 }))], |_| true).await;
    let text = run.text(1);
    let line = text.lines().find(|l| l.contains(wait.as_str())).unwrap_or(text);
    println!("{}", line.chars().take(1500).collect::<String>());
}
