//! «Gemini» одной кнопкой: Antigravity CLI из релизов GitHub (подменены своим сервером), заглушка `fake_acp`
//! под именем `antigravity.exe` внутри архива. Несколько аккаунтов, ключ API, обновление с откатом.

use crate::common;

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use axum::extract::State;
use axum::routing::get;
use axum::{Json, Router};
use common::{Crew, BIN};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

const RELEASE_PATH: &str = "/repos/google-antigravity/antigravity-cli/releases/latest";

struct Fake {
    dir: PathBuf,
    state: Arc<Mutex<Value>>,
    base: String,
}

fn file_url(path: &Path) -> String {
    format!("file:///{}", path.display().to_string().replace('\\', "/"))
}

impl Fake {
    async fn start(name: &str) -> Self {
        let dir = common::temp_data_dir(&format!("{name}-releases"));
        std::fs::create_dir_all(&dir).unwrap();
        let state = Arc::new(Mutex::new(json!({})));
        let app = Router::new()
            .route(RELEASE_PATH, get(|State(s): State<Arc<Mutex<Value>>>| async move { Json(s.lock().unwrap().clone()) }))
            .with_state(state.clone());
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        std::fs::write(dir.join("registry.json"), json!({ "version": "1.0.0", "agents": [] }).to_string()).unwrap();
        Self { dir, state, base }
    }

    /// Архив с `antigravity.exe`: заглушка или (если `broken`) файл, который не запускается.
    fn zip(&self, version: &str, broken: bool) -> (PathBuf, String) {
        let folder = self.dir.join(version);
        std::fs::create_dir_all(&folder).unwrap();
        let exe = folder.join("antigravity.exe");
        if broken {
            std::fs::write(&exe, b"not a program").unwrap();
        } else {
            let fake = Path::new(BIN).parent().unwrap().join("examples").join("fake_acp.exe");
            std::fs::copy(&fake, &exe).unwrap();
        }
        let zip = self.dir.join(format!("agy-{version}.zip"));
        let status = std::process::Command::new("tar").args(["-a", "-cf"]).arg(&zip).arg("-C").arg(&folder).arg("antigravity.exe").status().unwrap();
        assert!(status.success(), "не собрал архив");
        let sha: String = Sha256::digest(std::fs::read(&zip).unwrap()).iter().map(|b| format!("{b:02x}")).collect();
        (zip, sha)
    }

    /// Выложить релиз. `digest`: `Some(текст)` — как есть, `None` — поля нет. `url` — адрес архива.
    fn publish(&self, version: &str, url: &str, digest: Option<String>) {
        let asset = |name: &str| {
            let mut a = json!({ "name": name, "browser_download_url": url, "size": 1000 });
            if let Some(d) = &digest {
                a["digest"] = json!(d);
            }
            a
        };
        *self.state.lock().unwrap() = json!({
            "tag_name": format!("v{version}"),
            "assets": [asset("agy_cli_windows_x64.zip"), asset("agy_cli_windows_arm64.zip"), { "name": "agy_cli_linux_x64.tar.gz", "browser_download_url": "https://example.invalid/x" }],
        });
    }

    fn publish_good(&self, version: &str, broken: bool) {
        let (zip, sha) = self.zip(version, broken);
        self.publish(version, &file_url(&zip), Some(format!("sha256:{sha}")));
    }
}

async fn start(name: &str, fake: &Fake, auth: bool) -> (Crew, PathBuf) {
    start_with(name, fake, auth, true).await
}

async fn start_with(name: &str, fake: &Fake, auth: bool, oauth: bool) -> (Crew, PathBuf) {
    start_full(name, fake, auth, oauth, &[]).await
}

async fn start_full(name: &str, fake: &Fake, auth: bool, oauth: bool, extra: &[(&str, &str)]) -> (Crew, PathBuf) {
    let log = fake.dir.join("fake.log");
    let registry = fake.dir.join("registry.json").display().to_string();
    let mut env: Vec<(&str, String)> = vec![
        ("CREW_TEST_ACP_REGISTRY", registry),
        ("CREW_TEST_ANTIGRAVITY_RELEASES", fake.base.clone()),
        ("FAKE_ACP_LOG", log.display().to_string()),
    ];
    if oauth {
        env.push(("FAKE_ACP_OAUTH", "1".into()));
    }
    for (k, v) in extra {
        env.push((k, v.to_string()));
    }
    if auth {
        env.push(("FAKE_ACP_AUTH", "1".into()));
    }
    let pairs: Vec<(&str, &str)> = env.iter().map(|(k, v)| (*k, v.as_str())).collect();
    let crew = Crew::start_with_env(name, &pairs).await;
    crew.listen().await;
    (crew, log)
}

fn entries(log: &Path) -> Vec<Value> {
    std::fs::read_to_string(log).unwrap_or_default().lines().filter_map(|l| serde_json::from_str(l).ok()).collect()
}

fn started(log: &Path) -> Vec<Value> {
    entries(log).into_iter().filter(|e| e["method"] == "_started").collect()
}

async fn gemini(crew: &Crew) -> Value {
    crew.ok("GET", "/api/providers/antigravity", None).await
}

async fn wait_gemini(crew: &Crew, label: &str, ready: impl Fn(&Value) -> bool) -> Value {
    let started = std::time::Instant::now();
    loop {
        let view = gemini(crew).await;
        if ready(&view) {
            return view;
        }
        assert!(started.elapsed().as_secs() < 40, "не дождался «{label}»: {view}");
        tokio::time::sleep(std::time::Duration::from_millis(150)).await;
    }
}

fn idle(view: &Value) -> bool {
    !matches!(view["status"].as_str().unwrap_or_default(), "installing" | "verifying" | "unpacking" | "checking" | "connecting")
}

fn connected_accounts(view: &Value) -> usize {
    view["accounts"].as_array().map(|a| a.iter().filter(|x| x["connected"] == true).count()).unwrap_or(0)
}

fn home_arg(entry: &Value) -> String {
    entry["params"]["args"][1].as_str().unwrap_or_default().to_string()
}

#[tokio::test]
async fn one_click_installs_verifies_signs_in_with_google_and_starts_with_exact_args() {
    let fake = Fake::start("gem-install").await;
    fake.publish_good("1.5.7", false);
    let (crew, log) = start("gem-install", &fake, true).await;

    let listed = crew.ok("GET", "/api/providers", None).await;
    let card = listed["providers"].as_array().unwrap().iter().find(|p| p["id"] == "antigravity").cloned().expect("нет карточки Gemini");
    assert_eq!(card["name"], "Gemini");
    assert_eq!(card["status"], "not_installed");
    assert_eq!(card["latest"], "1.5.7");
    // Старый агент из реестра ACP и ключевой Google AI Studio остаются рядом.
    assert!(listed["providers"].as_array().unwrap().iter().any(|p| p["id"] == "gemini" && p["kind"] == "key"));

    crew.ok("POST", "/api/providers/antigravity/install", None).await;
    let view = wait_gemini(&crew, "подключено", |v| v["status"] == "connected").await;
    assert_eq!(view["installed"], "1.5.7");
    assert_eq!(connected_accounts(&view), 1, "{view}");
    let models: Vec<&str> = view["models"].as_array().unwrap().iter().filter_map(|m| m["id"].as_str()).collect();
    assert_eq!(models, ["fake-1", "fake-2"], "модели дошли до карточки");
    assert!(crew.data_dir.join("agents").join("antigravity").join("1.5.7").join("antigravity.exe").exists());
    assert!(crew.find_event(|e| e["type"] == "provider" && e["provider"]["status"] == "verifying").is_some(), "не было проверки подписи");

    // Вход идёт способом oauth-personal, хотя вход по ключу в списке стоит первым.
    let auth: Vec<Value> = entries(&log).into_iter().filter(|e| e["method"] == "authenticate").collect();
    assert!(!auth.is_empty());
    assert!(auth.iter().all(|e| e["params"]["methodId"] == "oauth-personal"), "{auth:?}");

    // Запуск: ровно acp, абсолютная своя папка и своя папка данных; своя TEMP у процесса.
    let runs = started(&log);
    assert!(!runs.is_empty());
    let first = &runs[0];
    let args: Vec<&str> = first["params"]["args"].as_array().unwrap().iter().filter_map(Value::as_str).collect();
    assert_eq!(args.len(), 3, "{args:?}");
    assert_eq!(args[0], "acp");
    let home = args[1].strip_prefix("--gemini-dir=").expect("нет --gemini-dir");
    assert!(Path::new(home).is_absolute(), "{home}");
    assert!(home.ends_with(r"accounts\a1\home"), "{home}");
    assert_eq!(args[2], "--app-data-dir=antigravity-acp");
    assert!(runs.iter().all(|r| r["params"]["wsl"] == "Deloist"), "без WSL_DISTRO_NAME вход уйдёт в общее хранилище Windows");
    let temp = first["params"]["temp"].as_str().unwrap();
    assert!(temp.contains("antigravity") && temp.contains("tmp"), "TEMP не своя: {temp}");
    let temps: std::collections::HashSet<&str> = runs.iter().filter_map(|r| r["params"]["temp"].as_str()).collect();
    assert_eq!(temps.len(), runs.len(), "у каждого процесса своя TEMP");
    let temp_root = crew.data_dir.join("agents").join("antigravity").join("tmp");
    common::wait_until("временные папки убраны", 15, || async { std::fs::read_dir(&temp_root).map(|d| d.count() == 0).unwrap_or(true) }).await;
}

#[tokio::test]
async fn a_bad_missing_or_insecure_download_is_refused() {
    let fake = Fake::start("gem-refuse").await;
    let (zip, sha) = fake.zip("1.5.7", false);
    let (crew, _) = start("gem-refuse", &fake, false).await;
    let good_url = file_url(&zip);
    let cases: Vec<(&str, String, Option<String>, &str)> = vec![
        ("неверная сумма", good_url.clone(), Some(format!("sha256:{}", "0".repeat(64))), "контрольной суммой"),
        ("суммы нет", good_url.clone(), None, "нет контрольной суммы"),
        ("сумма не sha256", good_url.clone(), Some("md5:abc".into()), "нет контрольной суммы"),
        ("http", "http://127.0.0.1:9/agy.zip".into(), Some(format!("sha256:{sha}")), "не по https"),
    ];
    for (label, url, digest, why) in cases {
        fake.publish("1.5.7", &url, digest);
        // Свежий ответ: кэш релиза живёт 6 часов.
        crew.ok("POST", "/api/providers/antigravity/refresh", None).await;
        crew.ok("POST", "/api/providers/antigravity/install", None).await;
        let view = wait_gemini(&crew, label, |v| idle(v) && v["error"].is_string()).await;
        assert!(view["error"].as_str().unwrap().contains(why), "{label}: {view}");
        assert!(view["installed"].is_null(), "{label}: поставилось");
        assert!(!crew.data_dir.join("agents").join("antigravity").join("1.5.7").exists(), "{label}: остался файл");
    }
}

#[tokio::test]
async fn an_update_keeps_the_old_version_until_the_new_one_starts() {
    let fake = Fake::start("gem-update").await;
    fake.publish_good("1.5.7", false);
    let (crew, _) = start("gem-update", &fake, false).await;
    crew.ok("POST", "/api/providers/antigravity/install", None).await;
    wait_gemini(&crew, "подключено", |v| v["status"] == "connected").await;
    let agents = crew.data_dir.join("agents").join("antigravity");

    // Вышла 1.5.8, но не запускается: остаётся 1.5.7, новую папку убираем.
    fake.publish_good("1.5.8", true);
    let view = crew.ok("POST", "/api/providers/antigravity/refresh", None).await;
    assert_eq!(view["latest"], "1.5.8");
    assert_eq!(view["update"], true, "{view}");
    crew.ok("POST", "/api/providers/antigravity/install", None).await;
    let view = wait_gemini(&crew, "откат", |v| idle(v) && v["error"].is_string()).await;
    assert_eq!(view["installed"], "1.5.7", "{view}");
    assert!(view["error"].as_str().unwrap().contains("оставил прежнюю"), "{view}");
    assert!(agents.join("1.5.7").join("antigravity.exe").exists());
    assert!(!agents.join("1.5.8").exists());

    // Рабочая 1.5.9 ставится, и только потом уходит 1.5.7.
    fake.publish_good("1.5.9", false);
    crew.ok("POST", "/api/providers/antigravity/refresh", None).await;
    crew.ok("POST", "/api/providers/antigravity/install", None).await;
    let view = wait_gemini(&crew, "обновилось", |v| idle(v) && v["installed"] == "1.5.9").await;
    assert_eq!(view["update"], false, "{view}");
    assert!(agents.join("1.5.9").join("antigravity.exe").exists());
    assert!(!agents.join("1.5.7").exists(), "старая версия должна уйти после первого запуска новой");
}

#[tokio::test]
async fn up_to_three_accounts_each_with_its_own_home_process_and_bots() {
    let fake = Fake::start("gem-accounts").await;
    fake.publish_good("1.5.7", false);
    let (crew, log) = start("gem-accounts", &fake, true).await;
    crew.ok("POST", "/api/providers/antigravity/install", None).await;
    wait_gemini(&crew, "первый аккаунт", |v| connected_accounts(v) == 1 && idle(v)).await;
    for want in [2, 3] {
        let (status, body) = crew.send("POST", "/api/providers/antigravity/accounts", None).await;
        assert_eq!(status, 200, "{body}");
        wait_gemini(&crew, "аккаунт", |v| connected_accounts(v) == want && idle(v)).await;
    }
    // Четвёртый не берём, говорим простыми словами.
    let (status, body) = crew.send("POST", "/api/providers/antigravity/accounts", None).await;
    assert_eq!(status, 400, "{body}");
    assert!(body["error"].as_str().unwrap_or_default().contains("не больше 3"), "{body}");

    // У каждого аккаунта своя папка и свой процесс.
    let runs = started(&log);
    let homes: std::collections::HashSet<String> = runs.iter().map(home_arg).collect();
    assert_eq!(homes.len(), 3, "{homes:?}");
    let pids: std::collections::HashSet<i64> = runs.iter().filter_map(|r| r["pid"].as_i64()).collect();
    assert_eq!(pids.len(), runs.len(), "процессы разные");

    // В выборе модели есть отдельная запись на второй и третий аккаунт.
    let list = crew.ok("GET", "/api/providers", None).await;
    let drivers: Vec<&str> = list["providers"].as_array().unwrap().iter().filter(|p| p["id"].as_str().unwrap_or_default().starts_with("antigravity")).filter_map(|p| p["driver"].as_str()).collect();
    assert!(drivers.contains(&"acp:antigravity") && drivers.contains(&"acp:antigravity@a2") && drivers.contains(&"acp:antigravity@a3"), "{drivers:?}");
    assert!(list["providers"].as_array().unwrap().iter().filter(|p| p["pickerOnly"] == true).all(|p| p["id"].as_str().unwrap().starts_with("antigravity@")));

    // Бот на втором аккаунте запускает агента с папкой второго аккаунта.
    let state = crew.ok("GET", "/api/state", None).await;
    let bot = state["bots"][0]["id"].as_str().unwrap().to_string();
    crew.ok("PATCH", &format!("/api/bots/{bot}"), Some(json!({ "driver": "acp:antigravity@a2" }))).await;
    let before = started(&log).len();
    crew.clear_events();
    crew.say(&bot, "привет").await;
    let done = crew.wait_event("конец хода", |e| e["type"] == "turn_end").await;
    assert_eq!(done["failed"], false, "{:?}", crew.find_event(|e| e["type"] == "error"));
    let after = started(&log);
    assert!(after.len() > before, "агент не запускался");
    assert!(home_arg(after.last().unwrap()).ends_with(r"accounts\a2\home"), "{:?}", after.last());

    // Удаление аккаунта: выход у программы, папка стёрта, бот возвращается к первому.
    let home2 = crew.data_dir.join("agents").join("antigravity").join("accounts").join("a2");
    assert!(home2.exists());
    crew.ok("DELETE", "/api/providers/antigravity/accounts/a2", None).await;
    assert!(!home2.exists(), "папка аккаунта осталась");
    assert!(entries(&log).iter().any(|e| e["method"] == "logout"), "программа не получила logout");
    let state = crew.ok("GET", "/api/state", None).await;
    let driver = state["bots"].as_array().unwrap().iter().find(|b| b["id"] == bot.as_str()).unwrap()["driver"].clone();
    assert_eq!(driver, "acp:antigravity", "бот остался на удалённом аккаунте");
    let view = gemini(&crew).await;
    assert_eq!(view["accounts"].as_array().unwrap().len(), 2);

    // «Отключить»: все аккаунты, программа остаётся.
    crew.ok("POST", "/api/providers/antigravity/logout", None).await;
    let view = gemini(&crew).await;
    assert_eq!(view["accounts"].as_array().unwrap().len(), 0, "{view}");
    assert_eq!(view["installed"], "1.5.7");
    assert!(crew.data_dir.join("agents").join("antigravity").join("1.5.7").join("antigravity.exe").exists());
}

fn files_under(dir: &Path, out: &mut Vec<PathBuf>) {
    for entry in std::fs::read_dir(dir).into_iter().flatten().flatten() {
        let path = entry.path();
        if path.is_dir() {
            files_under(&path, out);
        } else {
            out.push(path);
        }
    }
}

#[tokio::test]
async fn the_api_key_goes_only_through_the_environment_and_never_comes_back() {
    let fake = Fake::start("gem-key").await;
    fake.publish_good("1.5.7", false);
    let (crew, log) = start("gem-key", &fake, true).await;
    crew.ok("POST", "/api/providers/antigravity/install", None).await;
    wait_gemini(&crew, "первый аккаунт", |v| connected_accounts(v) == 1 && idle(v)).await;

    let key = "AIzaSyFAKEKEY0123456789abcdWXYZ";
    let (status, body) = crew.send("PUT", "/api/providers/antigravity/key", Some(json!({ "apiKey": key }))).await;
    assert_eq!(status, 200, "{body}");
    let view = wait_gemini(&crew, "вход по ключу", |v| connected_accounts(v) == 2 && idle(v)).await;
    assert_eq!(view["hasKey"], true);
    assert_eq!(view["keyTail"], "WXYZ");
    let row = view["accounts"].as_array().unwrap().iter().find(|a| a["kind"] == "key").cloned().expect("нет записи по ключу");
    assert_eq!(row["connected"], true);

    // Вход по ключу выбирает метод с ключом, вход через Google остаётся для остальных аккаунтов.
    let methods: Vec<Value> = entries(&log).into_iter().filter(|e| e["method"] == "authenticate").map(|e| e["params"]["methodId"].clone()).collect();
    assert!(methods.contains(&json!("gemini-api-key")) && methods.contains(&json!("oauth-personal")), "{methods:?}");
    let runs = started(&log);
    let with_key: Vec<&Value> = runs.iter().filter(|r| r["params"]["hasKey"] == true).collect();
    assert!(!with_key.is_empty(), "ключ не дошёл до программы");
    assert!(with_key.iter().all(|r| r["params"]["keyLen"] == key.len() && home_arg(r).ends_with(r"accounts\key\home")), "ключ ушёл не туда: {with_key:?}");
    assert!(runs.iter().filter(|r| home_arg(r).ends_with(r"accounts\a1\home")).all(|r| r["params"]["hasKey"] == false), "аккаунт Google получил ключ");

    // Ключа нет ни в одном ответе, ни в состоянии, ни в журнале заглушки, ни в файлах данных открытым текстом.
    let list = crew.ok("GET", "/api/providers", None).await;
    let state = crew.ok("GET", "/api/state", None).await;
    for (what, text) in [("список", list.to_string()), ("карточка", gemini(&crew).await.to_string()), ("state", state.to_string()), ("ответ PUT", body.to_string())] {
        assert!(!text.contains(key), "ключ в ответе: {what}");
    }
    assert!(!std::fs::read_to_string(&log).unwrap().contains(key), "ключ в argv/журнале заглушки");
    let mut files = Vec::new();
    files_under(&crew.data_dir, &mut files);
    for path in files {
        let bytes = std::fs::read(&path).unwrap_or_default();
        assert!(!bytes.windows(key.len()).any(|w| w == key.as_bytes()), "ключ открытым текстом в {}", path.display());
    }

    // Убрали ключ — запись по ключу отключается, аккаунт Google остаётся.
    let view = crew.ok("DELETE", "/api/providers/antigravity/key", None).await;
    assert_eq!(view["hasKey"], false, "{view}");
    assert!(view["accounts"].as_array().unwrap().iter().all(|a| a["kind"] != "key"), "{view}");
    assert_eq!(connected_accounts(&view), 1);
    // Битый ключ не принимаем.
    let (status, _) = crew.send("PUT", "/api/providers/antigravity/key", Some(json!({ "apiKey": "x y" }))).await;
    assert_eq!(status, 400);
}

#[tokio::test]
async fn a_model_list_without_a_saved_google_token_is_not_a_sign_in() {
    // Настоящая программа отдаёт модели и без входа: «подключено» только когда в папке аккаунта есть токен.
    let fake = Fake::start("gem-notoken").await;
    fake.publish_good("1.5.7", false);
    let (crew, _) = start_with("gem-notoken", &fake, false, false).await;
    crew.ok("POST", "/api/providers/antigravity/install", None).await;
    let view = wait_gemini(&crew, "вход не удался", |v| idle(v) && v["error"].is_string()).await;
    assert_eq!(view["installed"], "1.5.7");
    assert_eq!(connected_accounts(&view), 0, "{view}");
    assert_ne!(view["status"], "connected", "{view}");
    assert_eq!(view["accounts"].as_array().unwrap().len(), 0, "аккаунт без входа не остаётся: {view}");
}

fn alive(pid: i64) -> bool {
    let out = std::process::Command::new("tasklist").args(["/FI", &format!("PID eq {pid}"), "/NH"]).output().unwrap();
    String::from_utf8_lossy(&out.stdout).contains(&pid.to_string())
}

#[tokio::test]
async fn a_slow_google_sign_in_keeps_its_process_and_link_until_the_token_appears() {
    // Человек выбирает аккаунт в браузере 20 секунд: процесс с портом на 127.0.0.1 должен дожить до токена.
    let fake = Fake::start("gem-slow").await;
    fake.publish_good("1.5.7", false);
    let (crew, log) = start_full("gem-slow", &fake, true, true, &[("FAKE_ACP_SLOW_AUTH", "20")]).await;
    crew.ok("POST", "/api/providers/antigravity/install", None).await;
    let view = wait_gemini(&crew, "ссылка входа", |v| v["status"] == "connecting" && v["signinLink"] == true).await;
    assert_eq!(view["installed"], "1.5.7");
    let runs = started(&log);
    assert_eq!(runs.len(), 1, "одна программа для входа");
    let pid = runs[0]["pid"].as_i64().unwrap();

    // Пока идёт вход: процесс жив, карточка «вход», обновление и проверка его не трогают.
    for round in 0..4 {
        assert!(alive(pid), "процесс входа погиб на шаге {round}");
        let view = gemini(&crew).await;
        assert_eq!(view["status"], "connecting", "{view}");
        assert!(view["accounts"].as_array().unwrap().len() <= 1);
        match round {
            0 => assert_eq!(crew.send("POST", "/api/providers/antigravity/refresh", None).await.0, 200),
            1 => assert_eq!(crew.send("POST", "/api/providers/antigravity/check", None).await.0, 400, "проверка во время входа"),
            2 => {
                assert_eq!(crew.send("POST", "/api/providers/antigravity/logout", None).await.0, 400, "выход во время входа");
                assert_eq!(crew.send("DELETE", "/api/providers/antigravity/accounts/a1", None).await.0, 400);
            }
            _ => {
                let list = crew.ok("GET", "/api/providers", None).await;
                assert!(list["providers"].as_array().unwrap().iter().any(|p| p["id"] == "antigravity"));
            }
        }
        tokio::time::sleep(std::time::Duration::from_millis(700)).await;
    }
    assert_eq!(started(&log).len(), 1, "процесс входа перезапущен");

    // Ссылка есть для окна, но не в списке и не в файлах.
    let (status, body) = crew.send("GET", "/api/providers/antigravity/signin-link", None).await;
    assert_eq!(status, 200, "{body}");
    let link = body["link"].as_str().unwrap().to_string();
    assert!(link.starts_with("https://accounts.google.com/") && link.contains("SECRETLINK123"), "{link}");
    let list = crew.ok("GET", "/api/providers", None).await.to_string();
    assert!(!list.contains("SECRETLINK123"), "ссылка в списке провайдеров");
    assert!(!gemini(&crew).await.to_string().contains("SECRETLINK123"), "ссылка в карточке");

    // Токен появился — подключено, тем же процессом.
    let view = wait_gemini(&crew, "подключено", |v| v["status"] == "connected").await;
    assert_eq!(connected_accounts(&view), 1, "{view}");
    assert_eq!(started(&log).len(), 1);
    assert!(entries(&log).iter().any(|e| e["method"] == "_token_written"));
    assert_eq!(view["signinLink"], false);
    assert_eq!(crew.send("GET", "/api/providers/antigravity/signin-link", None).await.0, 400, "после входа ссылки нет");

    // Нигде на диске и в журнале заглушки ссылки нет.
    let mut files = Vec::new();
    files_under(&crew.data_dir, &mut files);
    files.push(log.clone());
    for path in files.into_iter().filter(|p| p.extension().is_none_or(|e| e != "exe")) {
        let bytes = std::fs::read(&path).unwrap_or_default();
        assert!(!bytes.windows(13).any(|w| w == b"SECRETLINK123"), "ссылка входа в {}", path.display());
    }
}
