//! Общая обвязка: настоящий exe в режиме `--server`, своя папка данных и свой порт на каждый тест.

#![allow(dead_code)]

use std::net::TcpListener;
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use futures_util::StreamExt;
use serde_json::{json, Value};

pub const BIN: &str = env!("CARGO_BIN_EXE_deloist");

pub fn free_port() -> u16 {
    TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port()
}

pub fn temp_data_dir(name: &str) -> PathBuf {
    let stamp = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos();
    PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(format!("{name}-{stamp}"))
}

pub const BOOT: &str = "test-boot-token";

/// Начало зашифрованного `secrets.json` (как в `src/secrets.rs`).
pub const SEALED_MAGIC: &[u8] = b"DLSEC1\n";

/// Содержимое `secrets.json` расшифрованным текстом: на диске он лежит под DPAPI текущего
/// пользователя. Пустая строка — файла нет. Открытый JSON на диске — провал теста.
pub fn read_secrets(data_dir: &std::path::Path) -> String {
    let Ok(bytes) = std::fs::read(data_dir.join("secrets.json")) else { return String::new() };
    let blob = bytes.strip_prefix(SEALED_MAGIC).unwrap_or_else(|| panic!("secrets.json лежит на диске открытым текстом"));
    String::from_utf8(unseal(blob, data_dir)).unwrap()
}

#[cfg(windows)]
fn unseal(blob: &[u8], _data_dir: &std::path::Path) -> Vec<u8> {
    use windows_sys::Win32::Foundation::LocalFree;
    use windows_sys::Win32::Security::Cryptography::{CryptUnprotectData, CRYPTPROTECT_UI_FORBIDDEN, CRYPT_INTEGER_BLOB};
    const ENTROPY: &[u8] = b"deloist-secrets-v1";
    let data_in = CRYPT_INTEGER_BLOB { cbData: blob.len() as u32, pbData: blob.as_ptr() as *mut u8 };
    let entropy = CRYPT_INTEGER_BLOB { cbData: ENTROPY.len() as u32, pbData: ENTROPY.as_ptr() as *mut u8 };
    let mut out = CRYPT_INTEGER_BLOB { cbData: 0, pbData: std::ptr::null_mut() };
    let done = unsafe { CryptUnprotectData(&data_in, std::ptr::null_mut(), &entropy, std::ptr::null(), std::ptr::null(), CRYPTPROTECT_UI_FORBIDDEN, &mut out) };
    assert!(done != 0 && !out.pbData.is_null(), "secrets.json не расшифровался DPAPI");
    let bytes = unsafe { std::slice::from_raw_parts(out.pbData, out.cbData as usize).to_vec() };
    unsafe { LocalFree(out.pbData as _) };
    bytes
}

/// Файл мастер-ключа для папки данных: рядом с ней, но не внутри (ключ не лежит в `data_dir`).
/// Сервер создаёт его сам с правами 0600 при первом запуске и читает при перезапуске.
pub fn key_file_for(data_dir: &std::path::Path) -> PathBuf {
    let base = data_dir.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    data_dir.with_file_name(format!("{base}-key")).join("secrets.key")
}

#[cfg(not(windows))]
fn unseal(blob: &[u8], data_dir: &std::path::Path) -> Vec<u8> {
    use chacha20poly1305::aead::{Aead, KeyInit, Payload};
    use chacha20poly1305::{Key, XChaCha20Poly1305, XNonce};
    use sha2::{Digest, Sha256};
    const ENTROPY: &[u8] = b"deloist-secrets-v1";
    let master = std::fs::read(key_file_for(data_dir)).expect("нет файла мастер-ключа");
    let blob = blob.strip_prefix(b"DLX1").expect("secrets.json не в формате DLX1");
    let mut hasher = Sha256::new();
    hasher.update(&master);
    hasher.update(b"deloist-secrets-v1");
    hasher.update(ENTROPY);
    let digest: [u8; 32] = hasher.finalize().into();
    let (nonce, sealed) = blob.split_at(24);
    let nonce: [u8; 24] = nonce.try_into().unwrap();
    XChaCha20Poly1305::new(&Key::from(digest)).decrypt(&XNonce::from(nonce), Payload { msg: sealed, aad: ENTROPY }).expect("secrets.json не расшифровался")
}

pub struct Crew {
    child: Child,
    pub port: u16,
    pub base: String,
    pub data_dir: PathBuf,
    pub client: reqwest::Client,
    /// Сессия окна: тесты получают её так же, как приложение, через одноразовую ссылку.
    pub cookie: String,
    pub events: Arc<Mutex<Vec<Value>>>,
    /// Папку данных не удалять при Drop: тест перезапустит сервер на ней же.
    keep_data: bool,
}

impl Crew {
    pub async fn start(name: &str) -> Self {
        Self::start_with_env(name, &[]).await
    }

    pub async fn start_with_env(name: &str, env: &[(&str, &str)]) -> Self {
        Self::start_at(temp_data_dir(name), env).await
    }

    /// Запуск на уже существующей папке данных: так проверяется перезапуск.
    pub async fn start_at(data_dir: PathBuf, env: &[(&str, &str)]) -> Self {
        if env.iter().any(|(k, v)| *k == "CREW_CLOUD" && *v == "1") {
            return Self::start_cloud_at(data_dir, env).await;
        }
        let mut crew = Self::spawn_at(data_dir, env).await;
        let res = crew.client.get(format!("{}/?boot={BOOT}", crew.base)).send().await.unwrap();
        assert_eq!(res.status().as_u16(), 303, "вход по одноразовой ссылке");
        let set_cookie = res.headers().get("set-cookie").expect("нет сессии").to_str().unwrap();
        crew.cookie = set_cookie.split(';').next().unwrap().to_string();
        crew
    }

    /// Облачный режим: одноразовой ссылки нет, вход — подписанный пропуск шлюза на `/auth/gateway`.
    /// Пара ключей создаётся здесь же и нигде не сохраняется.
    async fn start_cloud_at(data_dir: PathBuf, env: &[(&str, &str)]) -> Self {
        use base64::Engine;
        use ed25519_dalek::{Signer, SigningKey};
        use rand::RngCore;
        const ORIGIN: &str = "https://u-test.app.example.org";
        const PUBLIC_HOST: &str = "u-test.app.example.org";
        const USER: &str = "user_test-1";
        let hex = |bytes: &[u8]| bytes.iter().map(|b| format!("{b:02x}")).collect::<String>();
        let mut seed = [0u8; 32];
        rand::thread_rng().fill_bytes(&mut seed);
        let key = SigningKey::from_bytes(&seed);
        let pubkey = hex(key.verifying_key().as_bytes());
        // Вне Windows облако без ключа шифрования секретов не стартует (на Windows ключ даёт DPAPI); свой ключ теста важнее.
        let master = env.iter().find(|(k, _)| *k == "CREW_SECRETS_KEY").map(|(_, v)| v.to_string()).unwrap_or_else(|| {
            let mut bytes = [0u8; 32];
            rand::thread_rng().fill_bytes(&mut bytes);
            hex(&bytes)
        });
        // `read_secrets` ищет ключ в файле рядом с папкой данных; облачный сервер берёт его из окружения.
        #[cfg(not(windows))]
        {
            let bytes: Vec<u8> = (0..32).map(|i| u8::from_str_radix(&master[2 * i..2 * i + 2], 16).unwrap()).collect();
            let file = key_file_for(&data_dir);
            std::fs::create_dir_all(file.parent().unwrap()).unwrap();
            std::fs::write(&file, bytes).unwrap();
        }
        let mut vars: Vec<(&str, &str)> =
            vec![("CREW_GATEWAY_PUBKEY", pubkey.as_str()), ("CREW_USER_ID", USER), ("CREW_PUBLIC_ORIGIN", ORIGIN), ("CREW_SECRETS_KEY", master.as_str())];
        vars.extend(env.iter().copied());
        let mut crew = Self::spawn_at(data_dir, &vars).await;

        let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs() as i64;
        let mut nonce = [0u8; 12];
        rand::thread_rng().fill_bytes(&mut nonce);
        let payload = json!({ "uid": USER, "aud": ORIGIN, "iat": now, "exp": now + 30, "nonce": hex(&nonce) }).to_string();
        let b64 = base64::engine::general_purpose::URL_SAFE_NO_PAD;
        let token = format!("{}.{}", b64.encode(&payload), b64.encode(key.sign(&[b"deloist-gw-v1\n".as_slice(), payload.as_bytes()].concat()).to_bytes()));
        let res = crew
            .client
            .get(format!("{}/auth/gateway?t={token}", crew.base))
            .header("host", PUBLIC_HOST)
            .header("sec-fetch-site", "cross-site")
            .send()
            .await
            .unwrap();
        assert_eq!(res.status().as_u16(), 303, "вход через шлюз");
        let set_cookie = res.headers().get("set-cookie").expect("нет сессии").to_str().unwrap();
        crew.cookie = set_cookie.split(';').next().unwrap().to_string();
        crew
    }

    /// Сервер без входа: для проверок того, что снаружи ничего не открывается.
    pub async fn spawn(name: &str, env: &[(&str, &str)]) -> Self {
        Self::spawn_at(temp_data_dir(name), env).await
    }

    pub async fn spawn_at(data_dir: PathBuf, env: &[(&str, &str)]) -> Self {
        let port = free_port();
        // Папка сборки может нести метку Low integrity (песочница Codex помечает рабочие папки):
        // браузер бота оттуда не стартует. `CREW_TEST_BIN` — копия exe в обычной папке.
        let bin = env
            .iter()
            .find(|(k, _)| *k == "CREW_TEST_BIN")
            .map(|(_, v)| v.to_string())
            .or_else(|| std::env::var("CREW_TEST_BIN").ok())
            .unwrap_or_else(|| BIN.to_string());
        let child = Command::new(bin)
            .arg("--server")
            .env("PORT", port.to_string())
            .env("HOST", "127.0.0.1")
            .env("CREW_DATA_DIR", &data_dir)
            .env("CREW_BOOT_TOKEN", BOOT)
            // Тесты ждут прежних стартовых ботов; новая установка с оркестратором включается через `("CREW_LEGACY_STARTERS", "0")`.
            .env("CREW_LEGACY_STARTERS", "1")
            .env("CREW_SECRETS_KEY_FILE", key_file_for(&data_dir))
            // Уведомления Windows из тестов — в файлы, а не на экран человека (тест может переопределить путь).
            .env("CREW_TEST_TOAST_SINK", data_dir.join("test-toasts.jsonl"))
            .env("CREW_TEST_REMINDER_SINK", data_dir.join("test-reminders.jsonl"))
            .envs(env.iter().copied())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .expect("запуск crew");
        let client = reqwest::Client::builder()
            .no_proxy()
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .unwrap();
        let crew = Self {
            child,
            port,
            base: format!("http://127.0.0.1:{port}"),
            data_dir,
            client,
            cookie: String::new(),
            events: Arc::new(Mutex::new(Vec::new())),
            keep_data: false,
        };
        wait_until("сервер поднялся", 20, || async {
            std::net::TcpStream::connect(("127.0.0.1", port)).is_ok()
        })
        .await;
        crew
    }

    /// Номер процесса сервера: для проверок, что его память и окружение закрыты.
    #[allow(dead_code)]
    pub fn pid(&self) -> u32 {
        self.child.id()
    }

    /// Оставить папку данных после остановки сервера.
    pub fn keep_data(&mut self) {
        self.keep_data = true;
    }

    /// Подписка на поток событий, как у интерфейса.
    pub async fn listen(&self) {
        let res = self.client.get(format!("{}/api/events", self.base)).header("cookie", &self.cookie).send().await.unwrap();
        assert_eq!(res.status().as_u16(), 200, "поток событий");
        let events = self.events.clone();
        tokio::spawn(async move {
            let mut stream = res.bytes_stream();
            let mut buf: Vec<u8> = Vec::new();
            while let Some(Ok(chunk)) = stream.next().await {
                buf.extend_from_slice(&chunk);
                while let Some(i) = buf.windows(2).position(|w| w == b"\n\n") {
                    let part: Vec<u8> = buf.drain(..i + 2).collect();
                    for line in String::from_utf8_lossy(&part).lines() {
                        if let Some(data) = line.strip_prefix("data:") {
                            if let Ok(event) = serde_json::from_str(data.trim()) {
                                events.lock().unwrap().push(event);
                            }
                        }
                    }
                }
            }
        });
        self.wait_event("hello", |e| e["type"] == "hello").await;
    }

    pub async fn send(&self, method: &str, path: &str, body: Option<Value>) -> (u16, Value) {
        let method = reqwest::Method::from_bytes(method.as_bytes()).unwrap();
        let mut req = self.client.request(method, format!("{}{path}", self.base)).header("cookie", &self.cookie);
        if let Some(body) = body {
            req = req.header("content-type", "application/json").body(body.to_string());
        }
        let res = req.send().await.unwrap();
        let status = res.status().as_u16();
        let text = res.text().await.unwrap_or_default();
        (status, serde_json::from_str(&text).unwrap_or_else(|_| json!({})))
    }

    /// Процесс «вебхук → бот» с выданным секретом: номер и секрет (он же последняя часть адреса `/hooks/workflows/<номер>/t1/<секрет>`).
    pub async fn webhook_process(&self, bot_id: &str, title: &str, task: &str, events: &[&str]) -> (String, String) {
        let graph = json!({
            "nodes": [
                { "id": "t1", "kind": "trigger", "x": 0, "y": 0, "fields": { "trigger": "webhook", "hook_events": events.join(",") } },
                { "id": "b1", "kind": "bot", "x": 300, "y": 0, "fields": { "bot_id": bot_id, "task": task } }
            ],
            "edges": [{ "id": "e1", "from": "t1", "to": "b1" }]
        });
        let made = self.ok("POST", "/api/workflows", Some(json!({ "title": title, "graph": graph }))).await;
        let id = made["workflow"]["id"].as_str().unwrap().to_string();
        let url = self.ok("POST", &format!("/api/workflows/{id}/nodes/t1/webhook"), None).await["url"].as_str().unwrap().to_string();
        let secret = url.rsplit('/').next().unwrap().to_string();
        (id, secret)
    }

    /// Запрос, который обязан пройти.
    pub async fn ok(&self, method: &str, path: &str, body: Option<Value>) -> Value {
        let (status, data) = self.send(method, path, body).await;
        assert!((200..300).contains(&status), "{method} {path} → {status} {data}");
        data
    }

    pub async fn say(&self, bot_id: &str, text: &str) {
        self.ok("POST", &format!("/api/bots/{bot_id}/messages"), Some(json!({ "text": text }))).await;
    }

    pub fn find_event(&self, pred: impl Fn(&Value) -> bool) -> Option<Value> {
        self.events.lock().unwrap().iter().find(|e| pred(e)).cloned()
    }

    pub async fn wait_event(&self, label: &str, pred: impl Fn(&Value) -> bool) -> Value {
        let started = Instant::now();
        loop {
            if let Some(event) = self.find_event(&pred) {
                return event;
            }
            if started.elapsed() > Duration::from_secs(25) {
                let seen: Vec<String> = self.events.lock().unwrap().iter().map(|e| e["type"].to_string()).collect();
                panic!("не дождался: {label}; события: {}", seen.join(", "));
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    }

    pub fn count_events(&self, pred: impl Fn(&Value) -> bool) -> usize {
        self.events.lock().unwrap().iter().filter(|e| pred(e)).count()
    }

    pub fn clear_events(&self) {
        self.events.lock().unwrap().clear();
    }
}

impl Drop for Crew {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
        if !self.keep_data {
            let _ = std::fs::remove_dir_all(&self.data_dir);
            if let Some(dir) = key_file_for(&self.data_dir).parent() {
                let _ = std::fs::remove_dir_all(dir);
            }
        }
    }
}

pub async fn wait_until<F, Fut>(label: &str, seconds: u64, check: F)
where
    F: Fn() -> Fut,
    Fut: std::future::Future<Output = bool>,
{
    let started = Instant::now();
    while started.elapsed() < Duration::from_secs(seconds) {
        if check().await {
            return;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    panic!("не дождался: {label}");
}

/// Сервер-заглушка дистрибутива Node.js и сборка zip с фальшивым `node.exe`.
#[cfg(windows)]
pub mod node_dist {
    use super::{temp_data_dir, BIN};

    use std::collections::HashMap;
    use std::path::{Path, PathBuf};
    use std::sync::{Arc, Mutex};

    use axum::body::Body;
    use axum::extract::{Path as AxumPath, State};
    use axum::response::Response;
    use axum::routing::get;
    use axum::Router;
    use serde_json::{json, Value};
    use sha2::{Digest, Sha256};

    #[derive(Clone)]
    pub struct Dist {
        pub base: String,
        index: Arc<Mutex<Value>>,
        shasums: Arc<Mutex<Option<String>>>,
        zip: Arc<Mutex<Vec<u8>>>,
        hits: Arc<Mutex<HashMap<String, usize>>>,
    }

    impl Dist {
        pub fn set_shasums(&self, text: Option<String>) {
            *self.shasums.lock().unwrap() = text;
        }

        pub fn hits(&self, path: &str) -> usize {
            self.hits.lock().unwrap().get(path).copied().unwrap_or(0)
        }

        pub fn total_hits(&self) -> usize {
            self.hits.lock().unwrap().values().sum()
        }
    }

    pub fn fake_node() -> PathBuf {
        Path::new(BIN).parent().unwrap().join("examples").join("fake_node.exe")
    }

    /// PATH без node: системный Node «не установлен», как на чистом компьютере.
    pub fn without_node() -> String {
        let path = std::env::var_os("PATH").unwrap_or_default();
        let dirs: Vec<PathBuf> = std::env::split_paths(&path)
            .filter(|dir| !dir.join("node.exe").is_file() && !dir.join("node").is_file())
            .collect();
        std::env::join_paths(dirs).unwrap().to_string_lossy().into_owned()
    }

    /// PATH, где первым стоит папка с фальшивым системным node.exe и npm-cli.js.
    pub fn with_node(dir: &Path) -> String {
        let path = std::env::var_os("PATH").unwrap_or_default();
        let dirs = std::iter::once(dir.to_path_buf()).chain(std::env::split_paths(&path));
        std::env::join_paths(dirs).unwrap().to_string_lossy().into_owned()
    }

    /// Папка «системного» Node: node.exe и npm-cli.js рядом, как у обычной установки.
    pub fn fake_system_node(name: &str) -> PathBuf {
        let dir = temp_data_dir(name);
        let npm = dir.join("node_modules").join("npm").join("bin");
        std::fs::create_dir_all(&npm).unwrap();
        std::fs::copy(fake_node(), dir.join("node.exe")).unwrap();
        std::fs::write(npm.join("npm-cli.js"), "// fake npm\n").unwrap();
        std::fs::write(npm.join("npx-cli.js"), "// fake npx\n").unwrap();
        dir
    }

    fn build_zip(version: &str) -> Vec<u8> {
        let work = temp_data_dir("node-dist-zip");
        let inner = work.join(format!("node-{version}-win-x64"));
        let npm = inner.join("node_modules").join("npm").join("bin");
        std::fs::create_dir_all(&npm).unwrap();
        std::fs::copy(fake_node(), inner.join("node.exe")).unwrap();
        std::fs::write(inner.join("npm.cmd"), "@echo off\r\n").unwrap();
        std::fs::write(inner.join("npx.cmd"), "@echo off\r\n").unwrap();
        std::fs::write(npm.join("npm-cli.js"), "// fake npm\n").unwrap();
        std::fs::write(npm.join("npx-cli.js"), "// fake npx\n").unwrap();
        let zip = work.join("node.zip");
        let status = std::process::Command::new("tar")
            .args(["-a", "-cf"])
            .arg(&zip)
            .arg("-C")
            .arg(&work)
            .arg(inner.file_name().unwrap())
            .status()
            .unwrap();
        assert!(status.success(), "не собрал zip Node.js");
        let bytes = std::fs::read(&zip).unwrap();
        let _ = std::fs::remove_dir_all(&work);
        bytes
    }

    pub async fn start(version: &str) -> Dist {
        let zip = build_zip(version);
        let sha: String = Sha256::digest(&zip).iter().map(|b| format!("{b:02x}")).collect();
        let file = format!("node-{version}-win-x64.zip");
        let index = json!([
            { "version": "v27.0.0", "lts": false },
            { "version": version, "lts": "Lithium" }
        ]);
        let dist = Dist {
            base: String::new(),
            index: Arc::new(Mutex::new(index)),
            shasums: Arc::new(Mutex::new(Some(format!("{sha}  {file}\n")))),
            zip: Arc::new(Mutex::new(zip)),
            hits: Arc::new(Mutex::new(HashMap::new())),
        };
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let mut dist = dist;
        dist.base = base;
        let app = Router::new()
            .route("/index.json", get(index_route))
            .route("/{version}/{file}", get(file_route))
            .with_state(dist.clone());
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        dist
    }

    fn reply(bytes: Vec<u8>) -> Response {
        Response::builder().header("content-type", "application/octet-stream").body(Body::from(bytes)).unwrap()
    }

    async fn index_route(State(dist): State<Dist>) -> Response {
        *dist.hits.lock().unwrap().entry("index.json".to_string()).or_default() += 1;
        reply(dist.index.lock().unwrap().to_string().into_bytes())
    }

    async fn file_route(State(dist): State<Dist>, AxumPath((version, file)): AxumPath<(String, String)>) -> Response {
        let path = format!("{version}/{file}");
        *dist.hits.lock().unwrap().entry(path.clone()).or_default() += 1;
        if file == "SHASUMS256.txt" {
            if let Some(text) = dist.shasums.lock().unwrap().clone() {
                return reply(text.into_bytes());
            }
        }
        if file == format!("node-{version}-win-x64.zip") {
            return reply(dist.zip.lock().unwrap().clone());
        }
        Response::builder().status(404).body(Body::empty()).unwrap()
    }
}


/// Срок рутины «только что»: запуск пора делать, но это не пропущенный запуск (их догон — отдельно, с записью в журнал).
pub fn due_now() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_millis() as i64 - 1_000
}
