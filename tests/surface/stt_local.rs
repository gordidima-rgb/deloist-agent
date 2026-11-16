//! Речь на компьютере через тот же `/api/voice/transcribe`: настоящий exe в режиме `--server`,
//! фейковые архивы движка и модели с локального сервера (`CREW_TEST_STT_CATALOG`) и процесс
//! распознавателя в режиме заготовки (`CREW_TEST_STT_FAKE=1`): библиотеки sherpa-onnx не грузятся.
//! Плюс свой сервер с OpenAI-совместимым `/audio/transcriptions`.

use crate::common;

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use axum::body::Bytes;
use axum::extract::State;
use axum::http::HeaderMap;
use axum::response::IntoResponse;
use axum::routing::{get, post};
use axum::Router;
use base64::Engine;
use common::{temp_data_dir, Crew};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

const RUNTIME_DIR: &str = "sherpa-onnx-2.0.1";
const MODEL_DIR: &str = "stt-gigaam-v3-2025-12-16";

fn tar_exe() -> PathBuf {
    let root = std::env::var_os("SystemRoot").map(PathBuf::from).unwrap_or_else(|| PathBuf::from(r"C:\Windows"));
    root.join("System32").join("tar.exe")
}

/// tar.bz2 с файлами внутри папки `inner` (как в настоящих архивах sherpa-onnx).
fn archive(work: &Path, name: &str, inner: &str, files: &[&str]) -> Vec<u8> {
    let src = work.join(name);
    let dir = src.join(inner);
    std::fs::create_dir_all(&dir).unwrap();
    for file in files {
        std::fs::write(dir.join(file), format!("fake {file}")).unwrap();
    }
    let out = work.join(format!("{name}.tar.bz2"));
    let top = inner.split('/').next().unwrap();
    let status = std::process::Command::new(tar_exe()).arg("-c").arg("-j").arg("-f").arg(&out).arg("-C").arg(&src).arg(top).status().unwrap();
    assert!(status.success());
    std::fs::read(out).unwrap()
}

/// Раздаёт архивы по имени и считает запросы.
async fn mirror(files: Vec<(&'static str, Vec<u8>)>) -> (String, Arc<Mutex<usize>>) {
    let hits = Arc::new(Mutex::new(0usize));
    let mut app = Router::new();
    for (name, bytes) in files {
        let hits = hits.clone();
        app = app.route(
            &format!("/{name}"),
            get(move || {
                let bytes = bytes.clone();
                let hits = hits.clone();
                async move {
                    *hits.lock().unwrap() += 1;
                    // Медленная раздача: страница успевает увидеть «скачивается».
                    tokio::time::sleep(Duration::from_millis(300)).await;
                    bytes
                }
            }),
        );
    }
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    (format!("http://127.0.0.1:{port}"), hits)
}

fn pin(url: String, bytes: &[u8]) -> Value {
    let sum: String = Sha256::digest(bytes).iter().map(|b| format!("{b:02x}")).collect();
    json!({ "url": url, "size": bytes.len(), "sha256": sum })
}

/// Каталог с фейковыми архивами. `bad_model` — неверная сумма у модели.
async fn catalog(name: &str, bad_model: bool) -> (PathBuf, Arc<Mutex<usize>>) {
    let work = temp_data_dir(&format!("{name}-mirror"));
    std::fs::create_dir_all(&work).unwrap();
    let runtime = archive(&work, "rt", "sherpa-onnx-v2.0.1-win-x64-shared-MT-Release-no-tts-lib/lib", &["sherpa-onnx-c-api.dll", "onnxruntime.dll", "onnxruntime_providers_shared.dll"]);
    let model = archive(&work, "giga", "sherpa-onnx-nemo-transducer-punct-giga-am-v3-russian-2025-12-16", &["encoder.int8.onnx", "decoder.onnx", "joiner.onnx", "tokens.txt"]);
    let (base, hits) = mirror(vec![("rt.tar.bz2", runtime.clone()), ("giga.tar.bz2", model.clone())]).await;
    let mut model_pin = pin(format!("{base}/giga.tar.bz2"), &model);
    if bad_model {
        model_pin["sha256"] = json!("00".repeat(32));
    }
    let pins = json!({ RUNTIME_DIR: pin(format!("{base}/rt.tar.bz2"), &runtime), MODEL_DIR: model_pin });
    let path = work.join("catalog.json");
    std::fs::write(&path, pins.to_string()).unwrap();
    (path, hits)
}

fn pcm(seconds: f64) -> String {
    base64::engine::general_purpose::STANDARD.encode(vec![0u8; (seconds * 32000.0) as usize])
}

async fn wait_job(crew: &Crew, done: &[&str]) -> Value {
    let started = Instant::now();
    loop {
        let job = crew.ok("GET", "/api/voice/local", None).await;
        if done.contains(&job["job"]["state"].as_str().unwrap_or_default()) {
            return job;
        }
        assert!(started.elapsed() < Duration::from_secs(60), "подготовка не кончилась: {job}");
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn first_press_downloads_the_model_then_speech_is_recognised_on_this_computer() {
    let (catalog, hits) = catalog("stt-flow", false).await;
    let catalog = catalog.to_string_lossy().to_string();
    let crew = Crew::start_with_env("stt-flow", &[("CREW_TEST_STT_CATALOG", &catalog), ("CREW_TEST_STT_FAKE", "1")]).await;
    crew.listen().await;

    // По умолчанию — этот компьютер, бесплатно; модели ещё нет.
    let settings = crew.ok("GET", "/api/voice/settings", None).await;
    assert_eq!(settings["engine"], "local");
    assert_eq!(settings["model"], "gigaam-v3");
    // В 0.3.1 голос только на компьютере: платного пути через сайт нет.
    assert_eq!(settings["cloud"], false);
    let (status, _) = crew.send("PUT", "/api/voice/settings", Some(json!({ "engine": "deloist" }))).await;
    assert_eq!(status, 400, "облачный голос выключен");
    assert!(settings["models"].as_array().unwrap().iter().all(|m| m["installed"] == false), "{settings}");
    let (status, body) = crew.send("POST", "/api/voice/transcribe", Some(json!({ "pcm": pcm(1.0) }))).await;
    assert_eq!(status, 409, "{body}");

    // Проверка без скачивания ничего не начинает.
    let dry = crew.ok("POST", "/api/voice/prepare", Some(json!({ "dryRun": true }))).await;
    assert_eq!(dry["ready"], false);
    assert_eq!(*hits.lock().unwrap(), 0);

    // Первое нажатие: скачивание с прогрессом, загрузка, готово.
    let first = crew.ok("POST", "/api/voice/prepare", Some(json!({}))).await;
    assert_eq!(first["ready"], false);
    assert_eq!(first["job"]["state"], "downloading");
    let job = wait_job(&crew, &["ready", "error"]).await;
    assert_eq!(job["job"]["state"], "ready", "{job}");
    assert_eq!(job["loaded"], "gigaam-v3");
    assert_eq!(*hits.lock().unwrap(), 2, "движок и модель");
    crew.wait_event("скачивается", |e| e["type"] == "voice_state" && e["state"] == "downloading").await;
    crew.wait_event("загружается", |e| e["type"] == "voice_state" && e["state"] == "loading").await;
    crew.wait_event("готово", |e| e["type"] == "voice_state" && e["state"] == "ready" && e["progress"] == 100).await;
    let engines = crew.data_dir.join("engines");
    assert!(engines.join(RUNTIME_DIR).join("sherpa-onnx-c-api.dll").is_file());
    assert!(engines.join(MODEL_DIR).join("tokens.txt").is_file());

    // Второе нажатие: модель в памяти, слушаем сразу.
    let again = crew.ok("POST", "/api/voice/prepare", Some(json!({}))).await;
    assert_eq!(again["ready"], true, "{again}");

    // Запись идёт в процесс модели, а не на сайт.
    let answer = crew.ok("POST", "/api/voice/transcribe", Some(json!({ "pcm": pcm(1.0), "rate": 16000, "lang": "ru" }))).await;
    assert_eq!(answer, json!({ "text": "распознано на компьютере: 1.0 с" }));
    let answer = crew.ok("POST", "/api/voice/transcribe", Some(json!({ "pcm": pcm(2.0), "rate": 48000 }))).await;
    assert_eq!(answer["text"], "распознано на компьютере: 0.7 с", "48 кГц пересчитано в 16 кГц");

    // Удаление из настроек: модель и библиотеки уходят, запись снова просит скачать.
    let after = crew.ok("DELETE", "/api/voice/local/gigaam-v3", None).await;
    assert!(after["models"].as_array().unwrap().iter().all(|m| m["installed"] == false));
    assert!(!engines.join(MODEL_DIR).exists());
    assert!(!engines.join(RUNTIME_DIR).exists(), "без моделей движок не нужен");
    let (status, _) = crew.send("POST", "/api/voice/transcribe", Some(json!({ "pcm": pcm(1.0) }))).await;
    assert_eq!(status, 409);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_failed_download_says_so_and_leaves_nothing() {
    let (catalog, _) = catalog("stt-bad", true).await;
    let catalog = catalog.to_string_lossy().to_string();
    let crew = Crew::start_with_env("stt-bad", &[("CREW_TEST_STT_CATALOG", &catalog), ("CREW_TEST_STT_FAKE", "1")]).await;
    crew.ok("POST", "/api/voice/prepare", Some(json!({}))).await;
    let job = wait_job(&crew, &["ready", "error"]).await;
    assert_eq!(job["job"]["state"], "error");
    let error = job["job"]["error"].as_str().unwrap();
    assert!(error.starts_with("Не удалось скачать модель речи") && error.contains("контрольная сумма"), "{error}");
    let engines = crew.data_dir.join("engines");
    assert!(!engines.join(MODEL_DIR).exists());
    let leftovers: Vec<String> = std::fs::read_dir(&engines).unwrap().map(|e| e.unwrap().file_name().to_string_lossy().to_string()).collect();
    assert_eq!(leftovers, vec![RUNTIME_DIR.to_string()], "движок скачался, модели и обрывков нет");
}

#[tokio::test(flavor = "multi_thread")]
async fn cancel_stops_the_download() {
    let (catalog, _) = catalog("stt-cancel", false).await;
    let catalog = catalog.to_string_lossy().to_string();
    let crew = Crew::start_with_env("stt-cancel", &[("CREW_TEST_STT_CATALOG", &catalog), ("CREW_TEST_STT_FAKE", "1")]).await;
    let first = crew.ok("POST", "/api/voice/prepare", Some(json!({}))).await;
    assert_eq!(first["job"]["state"], "downloading");
    let cancelled = crew.ok("POST", "/api/voice/local/cancel", Some(json!({}))).await;
    assert_eq!(cancelled["cancelled"], true);
    let job = wait_job(&crew, &["idle", "ready", "error"]).await;
    assert_eq!(job["job"]["state"], "idle", "{job}");
    assert!(job["loaded"].is_null());
}

#[derive(Clone, Default)]
struct Heard(Arc<Mutex<Vec<(String, String)>>>);

async fn openai(State(heard): State<Heard>, headers: HeaderMap, body: Bytes) -> axum::response::Response {
    let auth = headers.get("authorization").and_then(|v| v.to_str().ok()).unwrap_or_default().to_string();
    heard.0.lock().unwrap().push((auth, String::from_utf8_lossy(&body).to_string()));
    axum::Json(json!({ "text": " текст со своего сервера " })).into_response()
}

#[tokio::test(flavor = "multi_thread")]
async fn own_server_gets_a_wav_with_the_key_and_the_cloud_is_one_setting_away() {
    let heard = Heard::default();
    let app = Router::new().route("/v1/audio/transcriptions", post(openai)).with_state(heard.clone());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });

    let crew = Crew::start("stt-server").await;
    let (status, body) = crew.send("PUT", "/api/voice/settings", Some(json!({ "engine": "server", "serverUrl": "ftp://nope" }))).await;
    assert_eq!(status, 400, "{body}");
    let saved = crew
        .ok("PUT", "/api/voice/settings", Some(json!({ "engine": "server", "serverUrl": format!("http://127.0.0.1:{port}/v1/"), "serverKey": "sk-test", "language": "ru" })))
        .await;
    assert_eq!(saved["serverKey"], true);
    assert!(!saved.to_string().contains("sk-test"), "ключ в ответ не попадает");
    let ready = crew.ok("POST", "/api/voice/prepare", Some(json!({}))).await;
    assert_eq!(ready["ready"], true, "свой сервер ничего не скачивает");

    let answer = crew.ok("POST", "/api/voice/transcribe", Some(json!({ "pcm": pcm(1.0) }))).await;
    assert_eq!(answer, json!({ "text": "текст со своего сервера" }));
    let seen = heard.0.lock().unwrap().clone();
    assert_eq!(seen.len(), 1);
    assert_eq!(seen[0].0, "Bearer sk-test");
    assert!(seen[0].1.contains("RIFF") && seen[0].1.contains("name=\"model\"") && seen[0].1.contains("whisper-1"));
    assert!(seen[0].1.contains("name=\"language\"\r\n\r\nru"));

    // Ключ в secrets.json, а не в state.json.
    let state = std::fs::read_to_string(crew.data_dir.join("state.json")).unwrap();
    assert!(!state.contains("sk-test"));
    assert!(common::read_secrets(&crew.data_dir).contains("sk-test"));
}
