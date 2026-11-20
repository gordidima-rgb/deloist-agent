//! Речь в текст на этом компьютере, бесплатно. Движок — sherpa-onnx (Apache-2.0) поверх
//! onnxruntime (MIT): готовые библиотеки скачиваются с GitHub проекта при первом нажатии на
//! микрофон, вместе с моделью. Всё закреплено, как у Obscura (`obscura.rs`): точный адрес,
//! размер и sha256. Другой файл под тем же именем не установится.
//!
//! Модель работает в отдельном процессе той же программы (`deloist.exe --stt-worker`):
//! падение чужой библиотеки не роняет приложение, а выгрузка по простою отдаёт память целиком.
//! Процесс получает 16-битный PCM 16 кГц по stdin и отвечает строкой JSON в stdout.
//!
//! Здесь же выбор движка для `/api/voice/transcribe`: этот компьютер (по умолчанию),
//! Deloist (платный SpeechKit через сайт) или свой сервер с OpenAI-совместимым API.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

use axum::extract::{Path as UrlPath, State};
use axum::response::Response;
use axum::routing::{delete, get, post};
use axum::{Json, Router};
use futures_util::StreamExt;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt};
use tokio_util::sync::CancellationToken;

use crate::voice::{fail, ok, Clip};
use crate::App;

// ---------------------------------------------------------------------------
// Что скачиваем
// ---------------------------------------------------------------------------

/// Один закреплённый архив с GitHub-выпуска sherpa-onnx.
#[derive(Clone, Debug)]
pub struct Pack {
    /// Папка в `data/engines`: имя и версия, новая версия ляжет рядом.
    pub dir: String,
    pub archive: String,
    pub size: u64,
    pub sha256: String,
    /// Сначала через API: с некоторых сетей прямые ссылки на выпуски GitHub отвечают 504.
    pub urls: Vec<String>,
    /// Папка внутри архива, чьё содержимое становится папкой движка.
    pub inner: String,
    /// Без этих файлов установка не считается.
    pub files: Vec<String>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Kind {
    /// NeMo-трансдьюсер: Parakeet, GigaAM. Язык модель выбирает сама или знает один.
    Transducer,
}

/// Какие языки модель понимает и можно ли его задать.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Langs {
    /// Только русский.
    Russian,
    /// Сама определяет язык, задать нельзя.
    Auto,
}

#[derive(Clone, Debug)]
pub struct Model {
    pub id: &'static str,
    pub title: &'static str,
    /// Строка для настроек: что умеет и чем хороша.
    pub note: &'static str,
    pub kind: Kind,
    pub langs: Langs,
    pub license: &'static str,
    pub source: &'static str,
    pub pack: Pack,
}

pub const RUNTIME_VERSION: &str = "2.0.1";
const RELEASE: &str = "https://github.com/k2-fsa/sherpa-onnx/releases/download";
const API: &str = "https://api.github.com/repos/k2-fsa/sherpa-onnx/releases/assets";

fn pack(dir: &str, archive: &str, size: u64, sha256: &str, asset: u64, tag: &str, inner: &str, files: &[&str]) -> Pack {
    Pack {
        dir: dir.to_string(),
        archive: archive.to_string(),
        size,
        sha256: sha256.to_string(),
        urls: vec![format!("{API}/{asset}"), format!("{RELEASE}/{tag}/{archive}")],
        inner: inner.to_string(),
        files: files.iter().map(|f| f.to_string()).collect(),
    }
}

/// Библиотеки sherpa-onnx и onnxruntime. Сборка MT: CRT вшит, Visual C++ Redistributable не нужен.
pub fn runtime_pack() -> Pack {
    pack(
        &format!("sherpa-onnx-{RUNTIME_VERSION}"),
        "sherpa-onnx-v2.0.1-win-x64-shared-MT-Release-no-tts-lib.tar.bz2",
        7_655_517,
        "21da6a8b92d63ca02ad41dbfc120012343aaaec2e5e02b14819c6b4f38004509",
        678_775_274,
        "v2.0.1",
        "sherpa-onnx-v2.0.1-win-x64-shared-MT-Release-no-tts-lib/lib",
        &["sherpa-onnx-c-api.dll", "onnxruntime.dll", "onnxruntime_providers_shared.dll"],
    )
}

/// Модели по порядку показа. Первая — по умолчанию.
pub fn models() -> Vec<Model> {
    vec![
        Model {
            id: "gigaam-v3",
            title: "GigaAM v3",
            note: "Лучшая для русского: быстрая, со знаками препинания. Только русский язык.",
            kind: Kind::Transducer,
            langs: Langs::Russian,
            license: "MIT (Sber, GigaChat Team)",
            source: "https://github.com/salute-developers/GigaAM",
            pack: pack(
                "stt-gigaam-v3-2025-12-16",
                "sherpa-onnx-nemo-transducer-punct-giga-am-v3-russian-2025-12-16.tar.bz2",
                170_197_019,
                "f9620a0099019c6afcee26525ef9ed3297fa50dd5691c1902af0c948fc1a470b",
                329_230_860,
                "asr-models",
                "sherpa-onnx-nemo-transducer-punct-giga-am-v3-russian-2025-12-16",
                &["encoder.int8.onnx", "decoder.onnx", "joiner.onnx", "tokens.txt"],
            ),
        },
        Model {
            id: "parakeet-v3",
            title: "Parakeet TDT 0.6B v3",
            note: "Сама определяет язык: русский, английский и ещё 23 европейских.",
            kind: Kind::Transducer,
            langs: Langs::Auto,
            license: "CC-BY-4.0 (NVIDIA)",
            source: "https://huggingface.co/nvidia/parakeet-tdt-0.6b-v3",
            pack: pack(
                "stt-parakeet-tdt-0.6b-v3-int8",
                "sherpa-onnx-nemo-parakeet-tdt-0.6b-v3-int8.tar.bz2",
                487_211_443,
                "1495e91b59d7fc53ddca91f3431758ec99f8fa253fdb0e1e851be0f72b14b0f9",
                719_769_366,
                "asr-models",
                "sherpa-onnx-nemo-parakeet-tdt-0.6b-v3-int8",
                &["encoder.int8.onnx", "decoder.int8.onnx", "joiner.int8.onnx", "tokens.txt"],
            ),
        },
    ]
}

pub fn model(id: &str) -> Option<Model> {
    models().into_iter().find(|m| m.id == id)
}

pub fn default_model() -> &'static str {
    "gigaam-v3"
}

/// Отладочная сборка подменяет адреса, размеры и суммы из JSON-файла `CREW_TEST_STT_CATALOG`:
/// `{"<dir>": {"url": ..., "size": ..., "sha256": ...}}`. Выпуск этого не умеет.
fn with_test_pins(mut pack: Pack) -> Pack {
    if !cfg!(debug_assertions) {
        return pack;
    }
    let Some(path) = std::env::var_os("CREW_TEST_STT_CATALOG") else { return pack };
    let Ok(text) = std::fs::read_to_string(path) else { return pack };
    let Ok(pins) = serde_json::from_str::<Value>(&text) else { return pack };
    if let Some(pin) = pins.get(&pack.dir) {
        if let Some(url) = pin["url"].as_str() {
            pack.urls = vec![url.to_string()];
        }
        if let Some(size) = pin["size"].as_u64() {
            pack.size = size;
        }
        if let Some(sum) = pin["sha256"].as_str() {
            pack.sha256 = sum.to_string();
        }
    }
    pack
}

pub fn engines_dir(data_dir: &Path) -> PathBuf {
    data_dir.join("engines")
}

pub fn pack_dir(data_dir: &Path, pack: &Pack) -> PathBuf {
    engines_dir(data_dir).join(&pack.dir)
}

pub fn installed(data_dir: &Path, pack: &Pack) -> bool {
    let dir = pack_dir(data_dir, pack);
    pack.files.iter().all(|name| dir.join(name).is_file())
}

// ---------------------------------------------------------------------------
// Скачивание: докачка, отмена, сверка суммы
// ---------------------------------------------------------------------------

pub const CANCELLED: &str = "скачивание отменено";

fn client() -> Result<reqwest::Client, String> {
    reqwest::Client::builder()
        .user_agent(concat!("deloist/", env!("CARGO_PKG_VERSION")))
        .connect_timeout(Duration::from_secs(15))
        .read_timeout(Duration::from_secs(60))
        .build()
        .map_err(|e| e.to_string())
}

fn to_hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// Сумма уже скачанной части: докачка продолжает ту же сумму.
async fn hash_file(path: &Path, hasher: &mut Sha256) -> Result<u64, String> {
    let mut file = tokio::fs::File::open(path).await.map_err(|e| e.to_string())?;
    let mut buf = vec![0u8; 1 << 20];
    let mut total = 0u64;
    loop {
        let n = file.read(&mut buf).await.map_err(|e| e.to_string())?;
        if n == 0 {
            return Ok(total);
        }
        hasher.update(&buf[..n]);
        total += n as u64;
    }
}

/// Архив в `part` со сверкой размера и суммы. Отмена и обрыв оставляют `part`:
/// следующая попытка докачает с того же места, если сервер умеет Range.
/// `progress` получает число байт архива, которые уже на диске.
pub async fn download(pack: &Pack, url: &str, part: &Path, cancel: &CancellationToken, progress: &(dyn Fn(u64) + Send + Sync)) -> Result<(), String> {
    let mut hasher = Sha256::new();
    let mut have = match tokio::fs::metadata(part).await {
        Ok(meta) if meta.len() <= pack.size => hash_file(part, &mut hasher).await?,
        Ok(_) => {
            let _ = tokio::fs::remove_file(part).await;
            0
        }
        Err(_) => 0,
    };
    progress(have);
    if have < pack.size {
        let mut request = client()?.get(url).header("Accept", "application/octet-stream");
        if have > 0 {
            request = request.header("Range", format!("bytes={have}-"));
        }
        let response = tokio::select! {
            _ = cancel.cancelled() => return Err(CANCELLED.into()),
            sent = request.send() => sent.map_err(|e| e.to_string())?,
        };
        let status = response.status().as_u16();
        let mut file = match status {
            206 if have > 0 => tokio::fs::OpenOptions::new().append(true).open(part).await.map_err(|e| e.to_string())?,
            200 => {
                // Сервер не умеет докачку: начинаем заново.
                hasher = Sha256::new();
                have = 0;
                progress(0);
                tokio::fs::File::create(part).await.map_err(|e| format!("нет права записи в папку данных: {e}"))?
            }
            _ => return Err(format!("HTTP {status}")),
        };
        let mut stream = response.bytes_stream();
        loop {
            let chunk = tokio::select! {
                _ = cancel.cancelled() => {
                    let _ = file.flush().await;
                    return Err(CANCELLED.into());
                }
                next = stream.next() => next,
            };
            let Some(chunk) = chunk else { break };
            let chunk = match chunk {
                Ok(chunk) => chunk,
                Err(e) => {
                    // Обрыв сети: скачанное сохраняем для докачки.
                    let _ = file.flush().await;
                    return Err(e.to_string());
                }
            };
            have += chunk.len() as u64;
            if have > pack.size {
                drop(file);
                let _ = tokio::fs::remove_file(part).await;
                return Err("архив больше ожидаемого".into());
            }
            hasher.update(&chunk);
            file.write_all(&chunk).await.map_err(|e| e.to_string())?;
            progress(have);
        }
        file.flush().await.map_err(|e| e.to_string())?;
    }
    if have != pack.size {
        return Err("архив скачался не целиком".into());
    }
    let got = to_hex(&hasher.finalize());
    if !got.eq_ignore_ascii_case(&pack.sha256) {
        let _ = tokio::fs::remove_file(part).await;
        return Err(format!("контрольная сумма не сошлась: ждали {}, пришла {got}", pack.sha256));
    }
    Ok(())
}

/// Встроенный в Windows tar (с 2018 года) понимает tar.bz2. Полный путь: чужой tar из PATH не нужен.
fn tar_exe() -> PathBuf {
    if !cfg!(windows) {
        return PathBuf::from("tar");
    }
    let root = std::env::var_os("SystemRoot").map(PathBuf::from).unwrap_or_else(|| PathBuf::from(r"C:\Windows"));
    root.join("System32").join("tar.exe")
}

async fn extract(archive: &Path, into: &Path) -> Result<(), String> {
    crate::cloud::deny(crate::cloud::NO_FEATURE)?;
    let mut command = tokio::process::Command::new(tar_exe());
    command.arg("-x").arg("-f").arg(archive).arg("-C").arg(into).stdin(std::process::Stdio::null());
    #[cfg(windows)]
    command.creation_flags(0x0800_0000);
    let output = command.output().await.map_err(|e| format!("не запустился tar: {e}"))?;
    if !output.status.success() {
        return Err(format!("архив не распаковался: {}", String::from_utf8_lossy(&output.stderr).trim()));
    }
    Ok(())
}

/// Скачать, сверить, распаковать и положить в `engines/<dir>`. Уже стоящий пакет не трогает.
pub async fn install(pack: &Pack, data_dir: &Path, cancel: &CancellationToken, progress: &(dyn Fn(u64) + Send + Sync)) -> Result<(), String> {
    if installed(data_dir, pack) {
        progress(pack.size);
        return Ok(());
    }
    let engines = engines_dir(data_dir);
    tokio::fs::create_dir_all(&engines).await.map_err(|e| e.to_string())?;
    let part = engines.join(format!("{}.part", pack.archive));
    let mut last_error = String::from("нет адреса для скачивания");
    let mut downloaded = false;
    for url in &pack.urls {
        match download(pack, url, &part, cancel, progress).await {
            Ok(()) => {
                downloaded = true;
                break;
            }
            // Отмена и неверная сумма — не сбой сети: второй адрес не поможет.
            Err(e) if e == CANCELLED || e.contains("контрольная сумма") => return Err(e),
            Err(e) => last_error = e,
        }
    }
    if !downloaded {
        return Err(last_error);
    }
    let staging = engines.join(format!("{}.staging", pack.dir));
    let _ = tokio::fs::remove_dir_all(&staging).await;
    tokio::fs::create_dir_all(&staging).await.map_err(|e| e.to_string())?;
    let unpacked = async {
        extract(&part, &staging).await?;
        let inner = staging.join(&pack.inner);
        for name in &pack.files {
            if !inner.join(name).is_file() {
                return Err(format!("в архиве нет {name}"));
            }
        }
        let _ = tokio::fs::remove_dir_all(inner.join("test_wavs")).await;
        let target = pack_dir(data_dir, pack);
        let _ = tokio::fs::remove_dir_all(&target).await;
        tokio::fs::rename(&inner, &target).await.map_err(|e| format!("не смог переложить файлы: {e}"))
    }
    .await;
    let _ = tokio::fs::remove_dir_all(&staging).await;
    let _ = tokio::fs::remove_file(&part).await;
    unpacked
}

// ---------------------------------------------------------------------------
// Подготовка: скачать недостающее и загрузить модель, с прогрессом для кнопки микрофона
// ---------------------------------------------------------------------------

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Job {
    /// idle, downloading, loading, ready или error.
    pub state: &'static str,
    pub model: String,
    pub received: u64,
    pub total: u64,
    pub error: String,
}

impl Job {
    fn idle() -> Self {
        Job { state: "idle", model: String::new(), received: 0, total: 0, error: String::new() }
    }

    pub fn percent(&self) -> u64 {
        (self.received * 100).checked_div(self.total).unwrap_or(0).min(100)
    }

    fn event(&self) -> Value {
        json!({ "type": "voice_state", "state": self.state, "model": self.model, "progress": self.percent(), "error": self.error })
    }
}

struct Jobs {
    job: Job,
    cancel: Option<CancellationToken>,
}

static JOB: Mutex<Jobs> = Mutex::new(Jobs {
    job: Job { state: "idle", model: String::new(), received: 0, total: 0, error: String::new() },
    cancel: None,
});

pub fn job() -> Job {
    JOB.lock().unwrap().job.clone()
}

fn busy() -> bool {
    matches!(job().state, "downloading" | "loading")
}

type Emit = Arc<dyn Fn(Value) + Send + Sync>;

fn update(emit: &Emit, change: impl FnOnce(&mut Job)) {
    let event = {
        let mut jobs = JOB.lock().unwrap();
        let before = (jobs.job.state, jobs.job.percent());
        change(&mut jobs.job);
        // Событие на каждый процент — много: кнопке хватает шага в 1 %, смена состояния идёт всегда.
        if before == (jobs.job.state, jobs.job.percent()) {
            return;
        }
        jobs.job.event()
    };
    emit(event);
}

/// Отменить скачивание. Скачанная часть остаётся для докачки.
pub fn cancel() -> bool {
    let jobs = JOB.lock().unwrap();
    match &jobs.cancel {
        Some(token) if matches!(jobs.job.state, "downloading" | "loading") => {
            token.cancel();
            true
        }
        _ => false,
    }
}

/// Скачать недостающее и загрузить модель. Возвращает сразу; ход работы — в `job()` и событиях.
pub fn start_prepare(data_dir: PathBuf, model: Model, emit: Emit) -> Job {
    let token = CancellationToken::new();
    {
        let mut jobs = JOB.lock().unwrap();
        if matches!(jobs.job.state, "downloading" | "loading") {
            return jobs.job.clone();
        }
        jobs.cancel = Some(token.clone());
    }
    let packs: Vec<Pack> = [runtime_pack(), model.pack.clone()].into_iter().map(with_test_pins).collect();
    let missing: Vec<Pack> = packs.into_iter().filter(|p| !installed(&data_dir, p)).collect();
    let total: u64 = missing.iter().map(|p| p.size).sum();
    let first = if missing.is_empty() { "loading" } else { "downloading" };
    update(&emit, |job| *job = Job { state: first, model: model.id.to_string(), received: 0, total, error: String::new() });
    let snapshot = job();
    tokio::spawn(async move {
        let result = prepare(&data_dir, &model, &missing, &token, &emit).await;
        update(&emit, |job| match result {
            Ok(()) => job.state = "ready",
            Err(e) if e == CANCELLED => *job = Job::idle(),
            Err(e) => {
                job.state = "error";
                job.error = e;
            }
        });
        // «ready» остаётся до следующей подготовки: страница, опрашивающая ход работы, его увидит.
        JOB.lock().unwrap().cancel = None;
    });
    snapshot
}

async fn prepare(data_dir: &Path, model: &Model, missing: &[Pack], cancel: &CancellationToken, emit: &Emit) -> Result<(), String> {
    let mut done = 0u64;
    for pack in missing {
        let base = done;
        let emit_progress = emit.clone();
        let progress = move |have: u64| {
            update(&emit_progress, |job| {
                job.received = base + have;
                // Всё скачано — дальше распаковка bz2 (до минуты): кнопка показывает загрузку, а не застывшие 100 %.
                if job.received >= job.total {
                    job.state = "loading";
                }
            })
        };
        install(pack, data_dir, cancel, &progress).await.map_err(|e| if e == CANCELLED { e } else { format!("Не удалось скачать модель речи: {e}") })?;
        done += pack.size;
    }
    update(emit, |job| {
        job.state = "loading";
        job.received = job.total;
    });
    tokio::select! {
        _ = cancel.cancelled() => Err(CANCELLED.into()),
        loaded = engine().load(data_dir, model) => loaded,
    }
}

/// Удалить модель. Последняя модель уносит с собой и библиотеки движка.
pub async fn remove(data_dir: &Path, id: &str) -> Result<(), String> {
    if busy() {
        return Err("дождись конца скачивания".into());
    }
    let model = model(id).ok_or("нет такой модели")?;
    engine().unload_model(id).await;
    let dir = pack_dir(data_dir, &model.pack);
    if dir.exists() {
        tokio::fs::remove_dir_all(&dir).await.map_err(|e| format!("не смог удалить {}: {e}", dir.display()))?;
    }
    let _ = tokio::fs::remove_file(engines_dir(data_dir).join(format!("{}.part", model.pack.archive))).await;
    if !models().iter().any(|m| installed(data_dir, &m.pack)) {
        engine().unload().await;
        let runtime = pack_dir(data_dir, &runtime_pack());
        if runtime.exists() {
            tokio::fs::remove_dir_all(&runtime).await.map_err(|e| format!("не смог удалить {}: {e}", runtime.display()))?;
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Движок: отдельный процесс, один на модель, выгружается по простою
// ---------------------------------------------------------------------------

/// Через столько простоя процесс модели закрывается и отдаёт память.
pub const IDLE_UNLOAD: Duration = Duration::from_secs(10 * 60);
/// Запись до 30 секунд распознаётся за секунды; минута — с запасом на медленный компьютер.
const DECODE_TIMEOUT: Duration = Duration::from_secs(60);
const LOAD_TIMEOUT: Duration = Duration::from_secs(120);

struct Worker {
    key: String,
    child: tokio::process::Child,
    stdin: tokio::process::ChildStdin,
    stdout: tokio::io::BufReader<tokio::process::ChildStdout>,
    last: Instant,
}

pub struct Engine {
    worker: tokio::sync::Mutex<Option<Worker>>,
}

pub fn engine() -> &'static Engine {
    static ENGINE: OnceLock<Engine> = OnceLock::new();
    ENGINE.get_or_init(|| {
        if tokio::runtime::Handle::try_current().is_ok() {
            tokio::spawn(async {
                let mut tick = tokio::time::interval(Duration::from_secs(30));
                loop {
                    tick.tick().await;
                    engine().unload_idle(IDLE_UNLOAD).await;
                }
            });
        }
        Engine { worker: tokio::sync::Mutex::new(None) }
    })
}

/// Описание модели для процесса: что грузить и как.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Spec {
    pub runtime: PathBuf,
    pub kind: String,
    pub encoder: PathBuf,
    pub decoder: PathBuf,
    pub joiner: PathBuf,
    pub tokens: PathBuf,
    pub language: String,
    pub threads: i32,
    /// Только отладочная сборка: процесс отвечает заготовкой, библиотеки не грузит.
    pub fake: bool,
}

fn threads() -> i32 {
    let cores = std::thread::available_parallelism().map(|n| n.get()).unwrap_or(4);
    (cores / 2).clamp(1, 4) as i32
}

pub fn spec(data_dir: &Path, model: &Model) -> Spec {
    let dir = pack_dir(data_dir, &with_test_pins(model.pack.clone()));
    let file = |i: usize| model.pack.files.get(i).map(|f| dir.join(f)).unwrap_or_default();
    Spec {
        runtime: pack_dir(data_dir, &runtime_pack()),
        kind: match model.kind {
            Kind::Transducer => "nemo_transducer".into(),
        },
        encoder: file(0),
        decoder: file(1),
        joiner: if model.kind == Kind::Transducer { file(2) } else { PathBuf::new() },
        tokens: dir.join("tokens.txt"),
        language: String::new(),
        threads: threads(),
        fake: cfg!(debug_assertions) && std::env::var("CREW_TEST_STT_FAKE").is_ok_and(|v| v == "1"),
    }
}

impl Engine {
    /// Запустить процесс модели и дождаться готовности. Та же модель уже работает — ничего не делает.
    pub async fn load(&self, data_dir: &Path, model: &Model) -> Result<(), String> {
        let spec = spec(data_dir, model);
        let key = format!("{}|{}", model.id, spec.language);
        let mut slot = self.worker.lock().await;
        if let Some(worker) = slot.as_mut() {
            if worker.key == key && worker.child.try_wait().ok().flatten().is_none() {
                worker.last = Instant::now();
                return Ok(());
            }
        }
        *slot = None;
        *slot = Some(spawn_worker(key, &spec).await?);
        Ok(())
    }

    pub async fn transcribe(&self, data_dir: &Path, model: &Model, pcm: &[u8]) -> Result<String, String> {
        self.load(data_dir, model).await?;
        let mut slot = self.worker.lock().await;
        let worker = slot.as_mut().ok_or("модель не загрузилась")?;
        let result = tokio::time::timeout(DECODE_TIMEOUT, ask(worker, pcm)).await.unwrap_or_else(|_| Err("модель не ответила за минуту".into()));
        match result {
            Ok(text) => {
                worker.last = Instant::now();
                Ok(text)
            }
            Err(e) => {
                // Процесс в неизвестном состоянии: следующая запись начнёт с нового.
                *slot = None;
                Err(e)
            }
        }
    }

    pub async fn unload(&self) {
        *self.worker.lock().await = None;
    }

    async fn unload_model(&self, id: &str) {
        let mut slot = self.worker.lock().await;
        if slot.as_ref().is_some_and(|w| w.key.split('|').next() == Some(id)) {
            *slot = None;
        }
    }

    pub async fn unload_idle(&self, after: Duration) {
        let mut slot = self.worker.lock().await;
        if slot.as_ref().is_some_and(|w| w.last.elapsed() >= after) {
            *slot = None;
        }
    }

    pub async fn loaded(&self) -> Option<String> {
        self.worker.lock().await.as_ref().map(|w| w.key.split('|').next().unwrap_or_default().to_string())
    }
}

/// Exe процесса модели. Проверки подставляют собранный deloist.exe: сами они — другой exe.
fn worker_exe() -> Result<PathBuf, String> {
    if cfg!(debug_assertions) {
        if let Some(path) = std::env::var_os("CREW_TEST_STT_WORKER") {
            return Ok(PathBuf::from(path));
        }
    }
    std::env::current_exe().map_err(|e| e.to_string())
}

async fn spawn_worker(key: String, spec: &Spec) -> Result<Worker, String> {
    crate::cloud::deny(crate::cloud::NO_FEATURE)?;
    let mut command = tokio::process::Command::new(worker_exe()?);
    command
        .arg("--stt-worker")
        .arg(serde_json::to_string(spec).map_err(|e| e.to_string())?)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .kill_on_drop(true);
    #[cfg(windows)]
    command.creation_flags(0x0800_0000);
    let mut child = command.spawn().map_err(|e| format!("не запустился распознаватель: {e}"))?;
    let stdin = child.stdin.take().ok_or("нет stdin у распознавателя")?;
    let mut stdout = tokio::io::BufReader::new(child.stdout.take().ok_or("нет stdout у распознавателя")?);
    let line = tokio::time::timeout(LOAD_TIMEOUT, read_line(&mut stdout)).await.map_err(|_| "модель грузится слишком долго".to_string())??;
    let ready: Value = serde_json::from_str(&line).map_err(|_| "распознаватель ответил непонятно".to_string())?;
    if let Some(error) = ready["error"].as_str() {
        return Err(format!("модель не загрузилась: {error}"));
    }
    if ready["ready"] != json!(true) {
        return Err("распознаватель ответил непонятно".into());
    }
    Ok(Worker { key, child, stdin, stdout, last: Instant::now() })
}

async fn read_line(stdout: &mut tokio::io::BufReader<tokio::process::ChildStdout>) -> Result<String, String> {
    let mut line = String::new();
    let n = stdout.read_line(&mut line).await.map_err(|e| e.to_string())?;
    if n == 0 {
        return Err("распознаватель закрылся".into());
    }
    Ok(line)
}

async fn ask(worker: &mut Worker, pcm: &[u8]) -> Result<String, String> {
    let samples = (pcm.len() / 2) as u32;
    worker.stdin.write_all(&samples.to_le_bytes()).await.map_err(|e| e.to_string())?;
    worker.stdin.write_all(&pcm[..samples as usize * 2]).await.map_err(|e| e.to_string())?;
    worker.stdin.flush().await.map_err(|e| e.to_string())?;
    let line = read_line(&mut worker.stdout).await?;
    let answer: Value = serde_json::from_str(&line).map_err(|_| "распознаватель ответил непонятно".to_string())?;
    if let Some(error) = answer["error"].as_str() {
        return Err(error.to_string());
    }
    Ok(answer["text"].as_str().unwrap_or_default().trim().to_string())
}

// ---------------------------------------------------------------------------
// Процесс модели (`deloist.exe --stt-worker <spec>`)
// ---------------------------------------------------------------------------

/// Тело процесса модели. Возвращает код выхода.
pub fn run_worker(spec: &str) -> i32 {
    use std::io::{BufWriter, Read, Write};
    let reply = |out: &mut BufWriter<std::io::StdoutLock>, value: Value| {
        let _ = writeln!(out, "{value}");
        let _ = out.flush();
    };
    let stdout = std::io::stdout();
    let mut out = BufWriter::new(stdout.lock());
    let spec: Spec = match serde_json::from_str(spec) {
        Ok(spec) => spec,
        Err(e) => {
            reply(&mut out, json!({ "error": format!("плохое описание модели: {e}") }));
            return 2;
        }
    };
    let started = Instant::now();
    let recognizer: Box<dyn FnMut(&[f32]) -> Result<String, String>> = if spec.fake && cfg!(debug_assertions) {
        Box::new(|samples: &[f32]| Ok(format!("распознано на компьютере: {:.1} с", samples.len() as f64 / 16000.0)))
    } else {
        match sherpa::Recognizer::load(&spec) {
            Ok(rec) => Box::new(move |samples: &[f32]| rec.transcribe(samples)),
            Err(e) => {
                reply(&mut out, json!({ "error": e }));
                return 1;
            }
        }
    };
    let mut recognizer = recognizer;
    reply(&mut out, json!({ "ready": true, "loadMs": started.elapsed().as_millis() as u64 }));
    let mut input = std::io::stdin().lock();
    loop {
        let mut head = [0u8; 4];
        if input.read_exact(&mut head).is_err() {
            return 0;
        }
        let count = u32::from_le_bytes(head) as usize;
        // 30 секунд записи — 480 тысяч отсчётов; больше не бывает, это сбой канала.
        if count > 16000 * 120 {
            reply(&mut out, json!({ "error": "слишком длинная запись" }));
            return 3;
        }
        let mut bytes = vec![0u8; count * 2];
        if input.read_exact(&mut bytes).is_err() {
            return 0;
        }
        let samples: Vec<f32> = bytes.chunks_exact(2).map(|b| f32::from(i16::from_le_bytes([b[0], b[1]])) / 32768.0).collect();
        match recognizer(&samples) {
            Ok(text) => reply(&mut out, json!({ "text": text })),
            Err(e) => reply(&mut out, json!({ "error": e })),
        }
    }
}

/// Тонкая обёртка над C API sherpa-onnx. Библиотека грузится во время работы, не при сборке:
/// C/C++ здесь не собирается. Раскладка структур — по c-api.h версии [`RUNTIME_VERSION`];
/// другая версия библиотеки отклоняется до создания распознавателя.
mod sherpa {
    use super::{Spec, RUNTIME_VERSION};
    use std::ffi::{c_char, c_void, CStr, CString};

    /// Начало `SherpaOnnxOfflineRecognizerConfig` до `model_type` включительно. Дальше нули:
    /// C API подставляет значения по умолчанию вместо нулей (greedy_search, cpu и т. п.).
    /// Хвост с запасом больше настоящей структуры, чтобы библиотека не читала за её край.
    #[repr(C)]
    struct Config {
        sample_rate: i32,
        feature_dim: i32,
        tr_encoder: *const c_char,
        tr_decoder: *const c_char,
        tr_joiner: *const c_char,
        paraformer: *const c_char,
        nemo_ctc: *const c_char,
        wh_encoder: *const c_char,
        wh_decoder: *const c_char,
        wh_language: *const c_char,
        wh_task: *const c_char,
        wh_tail_paddings: i32,
        wh_token_ts: i32,
        wh_segment_ts: i32,
        tdnn: *const c_char,
        tokens: *const c_char,
        num_threads: i32,
        debug: i32,
        provider: *const c_char,
        model_type: *const c_char,
        rest: [u64; 1024],
    }

    type Create = unsafe extern "C" fn(*const Config) -> *const c_void;
    type Destroy = unsafe extern "C" fn(*const c_void);
    type CreateStream = unsafe extern "C" fn(*const c_void) -> *const c_void;
    type Accept = unsafe extern "C" fn(*const c_void, i32, *const f32, i32);
    type Decode = unsafe extern "C" fn(*const c_void, *const c_void);
    type GetResult = unsafe extern "C" fn(*const c_void) -> *const c_void;
    type Version = unsafe extern "C" fn() -> *const c_char;

    pub struct Recognizer {
        recognizer: *const c_void,
        destroy: Destroy,
        create_stream: CreateStream,
        destroy_stream: Destroy,
        accept: Accept,
        decode: Decode,
        get_result: GetResult,
        destroy_result: Destroy,
    }

    #[cfg(windows)]
    fn open(path: &std::path::Path) -> Result<impl Fn(&str) -> Option<usize>, String> {
        use std::os::windows::ffi::OsStrExt;
        use windows_sys::Win32::System::LibraryLoader::{GetProcAddress, LoadLibraryExW, LOAD_WITH_ALTERED_SEARCH_PATH};
        let wide: Vec<u16> = path.as_os_str().encode_wide().chain([0]).collect();
        // С этим флагом зависимости (onnxruntime.dll) ищутся рядом с библиотекой, а не в PATH.
        let module = unsafe { LoadLibraryExW(wide.as_ptr(), std::ptr::null_mut(), LOAD_WITH_ALTERED_SEARCH_PATH) };
        if module.is_null() {
            return Err(format!("не загрузилась {}: код {}", path.display(), std::io::Error::last_os_error()));
        }
        Ok(move |name: &str| {
            let name = CString::new(name).ok()?;
            unsafe { GetProcAddress(module, name.as_ptr() as *const u8) }.map(|f| f as usize)
        })
    }

    #[cfg(not(windows))]
    fn open(_path: &std::path::Path) -> Result<impl Fn(&str) -> Option<usize>, String> {
        Err::<fn(&str) -> Option<usize>, _>("распознавание на компьютере есть только в Windows".into())
    }

    fn cstr(path: &std::path::Path, keep: &mut Vec<CString>) -> Result<*const c_char, String> {
        let text = CString::new(path.to_string_lossy().as_bytes()).map_err(|_| "путь с нулевым байтом".to_string())?;
        let ptr = text.as_ptr();
        keep.push(text);
        Ok(ptr)
    }

    impl Recognizer {
        pub fn load(spec: &Spec) -> Result<Self, String> {
            let symbol = open(&spec.runtime.join("sherpa-onnx-c-api.dll"))?;
            let get = |name: &str| symbol(name).ok_or(format!("в библиотеке нет {name}"));
            unsafe {
                let version: Version = std::mem::transmute::<usize, Version>(get("SherpaOnnxGetVersionStr")?);
                let found = CStr::from_ptr(version()).to_string_lossy().into_owned();
                if found != RUNTIME_VERSION {
                    return Err(format!("библиотека sherpa-onnx {found}, нужна {RUNTIME_VERSION}"));
                }
                let mut keep = Vec::new();
                let mut config: Box<Config> = Box::new(std::mem::zeroed());
                config.sample_rate = 16000;
                config.tokens = cstr(&spec.tokens, &mut keep)?;
                config.num_threads = spec.threads.max(1);
                config.tr_encoder = cstr(&spec.encoder, &mut keep)?;
                config.tr_decoder = cstr(&spec.decoder, &mut keep)?;
                config.tr_joiner = cstr(&spec.joiner, &mut keep)?;
                let kind = CString::new(spec.kind.as_str()).unwrap_or_default();
                config.model_type = kind.as_ptr();
                keep.push(kind);
                let create: Create = std::mem::transmute::<usize, Create>(get("SherpaOnnxCreateOfflineRecognizer")?);
                let recognizer = create(&*config);
                drop(keep);
                if recognizer.is_null() {
                    return Err("sherpa-onnx не принял модель: файлы повреждены или не те".into());
                }
                Ok(Recognizer {
                    recognizer,
                    destroy: std::mem::transmute::<usize, Destroy>(get("SherpaOnnxDestroyOfflineRecognizer")?),
                    create_stream: std::mem::transmute::<usize, CreateStream>(get("SherpaOnnxCreateOfflineStream")?),
                    destroy_stream: std::mem::transmute::<usize, Destroy>(get("SherpaOnnxDestroyOfflineStream")?),
                    accept: std::mem::transmute::<usize, Accept>(get("SherpaOnnxAcceptWaveformOffline")?),
                    decode: std::mem::transmute::<usize, Decode>(get("SherpaOnnxDecodeOfflineStream")?),
                    get_result: std::mem::transmute::<usize, GetResult>(get("SherpaOnnxGetOfflineStreamResult")?),
                    destroy_result: std::mem::transmute::<usize, Destroy>(get("SherpaOnnxDestroyOfflineRecognizerResult")?),
                })
            }
        }

        pub fn transcribe(&self, samples: &[f32]) -> Result<String, String> {
            unsafe {
                let stream = (self.create_stream)(self.recognizer);
                if stream.is_null() {
                    return Err("sherpa-onnx не создал поток".into());
                }
                (self.accept)(stream, 16000, samples.as_ptr(), samples.len() as i32);
                (self.decode)(self.recognizer, stream);
                let result = (self.get_result)(stream);
                let text = if result.is_null() {
                    String::new()
                } else {
                    // Первое поле SherpaOnnxOfflineRecognizerResult — const char *text.
                    let text = *(result as *const *const c_char);
                    let owned = if text.is_null() { String::new() } else { CStr::from_ptr(text).to_string_lossy().into_owned() };
                    (self.destroy_result)(result);
                    owned
                };
                (self.destroy_stream)(stream);
                Ok(text)
            }
        }
    }

    impl Drop for Recognizer {
        fn drop(&mut self) {
            unsafe { (self.destroy)(self.recognizer) }
        }
    }
}

// ---------------------------------------------------------------------------
// Настройки голоса и выбор движка
// ---------------------------------------------------------------------------

/// `Settings.extra["voice"]`. Ключ своего сервера — в secrets.json, как ключи провайдеров.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct VoiceSettings {
    /// local — этот компьютер, deloist — сайт (SpeechKit, платно), server — свой сервер.
    pub engine: String,
    pub model: String,
    /// auto или код языка; модель без выбора языка его не слушает.
    pub language: String,
    pub server_url: String,
    pub server_model: String,
}

impl Default for VoiceSettings {
    fn default() -> Self {
        VoiceSettings { engine: "local".into(), model: default_model().into(), language: "auto".into(), server_url: String::new(), server_model: "whisper-1".into() }
    }
}

const SERVER_KEY: &str = "voice-server";
pub const ENGINES: [&str; 3] = ["local", "deloist", "server"];

/// Платный голос через deloist.ru (SpeechKit) появится вместе с облачной версией. В 0.3.1 голос
/// только на компьютере: способа «Через Deloist» нет ни в настройках, ни в запросах, код ждёт флага.
pub const CLOUD_VOICE: bool = false;

/// Облачный путь включён: флаг выпуска или, в отладочной сборке, `CREW_TEST_CLOUD_VOICE=1` для проверок.
pub fn cloud_voice() -> bool {
    CLOUD_VOICE || (cfg!(debug_assertions) && std::env::var("CREW_TEST_CLOUD_VOICE").is_ok_and(|v| v == "1"))
}

fn engine_allowed(engine: &str) -> bool {
    ENGINES.contains(&engine) && (engine != "deloist" || cloud_voice())
}
pub const LANGUAGES: [&str; 8] = ["auto", "ru", "en", "de", "fr", "es", "it", "uk"];

pub fn settings(app: &App) -> VoiceSettings {
    let mut voice = app.store.settings().extra.get("voice").and_then(|v| serde_json::from_value::<VoiceSettings>(v.clone()).ok()).unwrap_or_default();
    if !engine_allowed(&voice.engine) {
        voice.engine = "local".into();
    }
    if model(&voice.model).is_none() {
        voice.model = default_model().into();
    }
    voice
}

/// Правка настроек из `PUT /api/voice/settings`. Возвращает ошибку для человека.
pub fn apply(mut voice: VoiceSettings, patch: &Value) -> Result<VoiceSettings, &'static str> {
    if let Some(engine) = patch.get("engine").and_then(Value::as_str) {
        if !engine_allowed(engine) {
            return Err("Такого способа распознавания нет");
        }
        voice.engine = engine.into();
    }
    if let Some(id) = patch.get("model").and_then(Value::as_str) {
        if model(id).is_none() {
            return Err("Такой модели нет");
        }
        voice.model = id.into();
    }
    if let Some(language) = patch.get("language").and_then(Value::as_str) {
        if !LANGUAGES.contains(&language) {
            return Err("Такого языка нет в списке");
        }
        voice.language = language.into();
    }
    if let Some(url) = patch.get("serverUrl").and_then(Value::as_str) {
        let url = url.trim().trim_end_matches('/');
        if !url.is_empty() && reqwest::Url::parse(url).map(|u| !matches!(u.scheme(), "http" | "https")).unwrap_or(true) {
            return Err("Адрес сервера должен начинаться с http:// или https://");
        }
        voice.server_url = url.into();
    }
    if let Some(name) = patch.get("serverModel").and_then(Value::as_str) {
        voice.server_model = if name.trim().is_empty() { "whisper-1".into() } else { name.trim().into() };
    }
    Ok(voice)
}

fn status_json(app: &App, voice: &VoiceSettings) -> Value {
    let data = app.store.data_dir();
    let job = job();
    let list: Vec<Value> = models()
        .iter()
        .map(|m| {
            json!({
                "id": m.id, "title": m.title, "note": m.note,
                "sizeMb": (m.pack.size as f64 / 1_048_576.0).round(),
                "installed": installed(data, &with_test_pins(m.pack.clone())),
                "languages": match m.langs { Langs::Russian => "ru", Langs::Auto => "auto" },
                "license": m.license, "source": m.source,
            })
        })
        .collect();
    json!({
        "engine": voice.engine, "model": voice.model, "language": voice.language,
        "serverUrl": voice.server_url, "serverModel": voice.server_model,
        "serverKey": !app.store.own_key(SERVER_KEY).is_empty(),
        "signedIn": app.store.account_token().is_some(),
        "cloud": cloud_voice(),
        "models": list,
        "runtimeMb": (runtime_pack().size as f64 / 1_048_576.0).round(),
        "job": job, "progress": job.percent(),
        "languages": LANGUAGES,
    })
}

fn emitter(app: &Arc<App>) -> Emit {
    let app = app.clone();
    Arc::new(move |event| app.turns.emit(event))
}

/// Распознать запись выбранным способом. Ошибка — код и текст для человека.
pub async fn transcribe(app: &App, clip: Clip) -> Result<String, (u16, String)> {
    let voice = settings(app);
    match voice.engine.as_str() {
        "deloist" if cloud_voice() => crate::voice::transcribe(app, clip).await.map_err(|(s, m)| (s, m.to_string())),
        "server" => transcribe_server(app, &voice, &clip).await,
        _ => transcribe_local(app, &voice, &clip).await,
    }
}

/// 16 кГц — родная частота моделей; другие частоты пересчитываем простым прореживанием/повтором.
fn to_16k(pcm: &[u8], rate: u32) -> Vec<u8> {
    if rate == 16000 {
        return pcm.to_vec();
    }
    let samples: Vec<i16> = pcm.chunks_exact(2).map(|b| i16::from_le_bytes([b[0], b[1]])).collect();
    let count = (samples.len() as u64 * 16000 / u64::from(rate)) as usize;
    let mut out = Vec::with_capacity(count * 2);
    for i in 0..count {
        let at = (i as u64 * u64::from(rate) / 16000) as usize;
        out.extend_from_slice(&samples[at.min(samples.len() - 1)].to_le_bytes());
    }
    out
}

async fn transcribe_local(app: &App, voice: &VoiceSettings, clip: &Clip) -> Result<String, (u16, String)> {
    let model = model(&voice.model).ok_or((409, "Выбери модель речи в настройках".to_string()))?;
    let data = app.store.data_dir();
    if !installed(data, &with_test_pins(runtime_pack())) || !installed(data, &with_test_pins(model.pack.clone())) {
        return Err((409, "Модель речи ещё не скачана. Нажми на микрофон ещё раз, она скачается".into()));
    }
    let pcm = to_16k(&clip.pcm, clip.rate);
    engine().transcribe(data, &model, &pcm).await.map_err(|e| (502, format!("Не получилось распознать речь на компьютере: {e}")))
}

/// WAV-заголовок для 16-битного моно: свой сервер ждёт файл, а не сырой PCM.
pub fn wav(pcm: &[u8], rate: u32) -> Vec<u8> {
    let mut out = Vec::with_capacity(44 + pcm.len());
    out.extend_from_slice(b"RIFF");
    out.extend_from_slice(&(36 + pcm.len() as u32).to_le_bytes());
    out.extend_from_slice(b"WAVEfmt ");
    out.extend_from_slice(&16u32.to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes());
    out.extend_from_slice(&rate.to_le_bytes());
    out.extend_from_slice(&(rate * 2).to_le_bytes());
    out.extend_from_slice(&2u16.to_le_bytes());
    out.extend_from_slice(&16u16.to_le_bytes());
    out.extend_from_slice(b"data");
    out.extend_from_slice(&(pcm.len() as u32).to_le_bytes());
    out.extend_from_slice(pcm);
    out
}

fn multipart(boundary: &str, fields: &[(&str, &str)], audio: &[u8]) -> Vec<u8> {
    let mut body = Vec::new();
    for (name, value) in fields {
        body.extend_from_slice(format!("--{boundary}\r\nContent-Disposition: form-data; name=\"{name}\"\r\n\r\n{value}\r\n").as_bytes());
    }
    body.extend_from_slice(format!("--{boundary}\r\nContent-Disposition: form-data; name=\"file\"; filename=\"speech.wav\"\r\nContent-Type: audio/wav\r\n\r\n").as_bytes());
    body.extend_from_slice(audio);
    body.extend_from_slice(format!("\r\n--{boundary}--\r\n").as_bytes());
    body
}

/// Свой сервер: OpenAI-совместимый `POST {url}/audio/transcriptions`.
async fn transcribe_server(app: &App, voice: &VoiceSettings, clip: &Clip) -> Result<String, (u16, String)> {
    if voice.server_url.is_empty() {
        return Err((409, "Укажи адрес своего сервера распознавания в настройках".into()));
    }
    let url = format!("{}/audio/transcriptions", voice.server_url);
    let boundary = format!("deloist{:016x}", rand::random::<u64>());
    let language = if voice.language == "auto" { String::new() } else { voice.language.clone() };
    let mut fields = vec![("model", voice.server_model.as_str()), ("response_format", "json")];
    if !language.is_empty() {
        fields.push(("language", language.as_str()));
    }
    let body = multipart(&boundary, &fields, &wav(&clip.pcm, clip.rate));
    let client = reqwest::Client::builder()
        .user_agent(concat!("deloist/", env!("CARGO_PKG_VERSION")))
        .connect_timeout(Duration::from_secs(10))
        .timeout(Duration::from_secs(60))
        .build()
        .map_err(|e| (502, e.to_string()))?;
    let mut request = client.post(&url).header("content-type", format!("multipart/form-data; boundary={boundary}")).body(body);
    let key = app.store.own_key(SERVER_KEY);
    if !key.is_empty() {
        request = request.bearer_auth(key);
    }
    let response = request.send().await.map_err(|_| (502, "Нет связи с сервером распознавания. Проверь адрес в настройках".to_string()))?;
    let status = response.status().as_u16();
    if status == 401 || status == 403 {
        return Err((401, "Сервер распознавания не принял ключ".into()));
    }
    if !(200..300).contains(&status) {
        return Err((502, format!("Сервер распознавания ответил HTTP {status}")));
    }
    let data: Value = response.json().await.map_err(|_| (502, "Сервер распознавания ответил не JSON".to_string()))?;
    let text = data["text"].as_str().ok_or((502, "В ответе сервера нет текста".to_string()))?;
    Ok(text.trim().to_string())
}

// ---------------------------------------------------------------------------
// Маршруты
// ---------------------------------------------------------------------------

async fn get_settings(State(app): State<Arc<App>>) -> Response {
    ok(status_json(&app, &settings(&app)))
}

async fn put_settings(State(app): State<Arc<App>>, Json(patch): Json<Value>) -> Response {
    let voice = match apply(settings(&app), &patch) {
        Ok(voice) => voice,
        Err(message) => return fail(400, message),
    };
    if let Some(key) = patch.get("serverKey").and_then(Value::as_str) {
        if let Err(e) = app.store.set_own_key(SERVER_KEY, key).await {
            return fail(500, format!("Не сохранил ключ: {e}"));
        }
    }
    let value = serde_json::to_value(&voice).unwrap_or_default();
    if let Err(e) = app.store.set_setting_extra("voice", value).await {
        return fail(500, format!("Не сохранил настройки: {e}"));
    }
    ok(status_json(&app, &voice))
}

/// Перед записью: страница спрашивает, можно ли слушать. Для этого компьютера здесь же
/// начинается скачивание и загрузка; страница следит за `job` и сама начинает запись.
async fn prepare_route(State(app): State<Arc<App>>, Json(body): Json<Value>) -> Response {
    let voice = settings(&app);
    if voice.engine != "local" {
        return ok(json!({ "ready": true, "engine": voice.engine }));
    }
    let model = model(&voice.model).unwrap_or_else(|| models().remove(0));
    let data = app.store.data_dir().to_path_buf();
    let wanted = format!("{}|{}", model.id, spec(&data, &model).language);
    // Модель уже в памяти: слушаем сразу.
    if engine().worker.lock().await.as_ref().is_some_and(|w| w.key == wanted) {
        return ok(json!({ "ready": true, "engine": "local" }));
    }
    // Только проверить, не начиная скачивание (настройки показывают «Скачать» сами).
    if body["dryRun"] == json!(true) {
        return ok(json!({ "ready": false, "engine": "local", "job": job() }));
    }
    let job = start_prepare(data, model, emitter(&app));
    ok(json!({ "ready": false, "engine": "local", "job": job, "progress": job.percent() }))
}

async fn job_route() -> Response {
    let job = job();
    ok(json!({ "job": job, "progress": job.percent(), "loaded": engine().loaded().await }))
}

async fn cancel_route() -> Response {
    ok(json!({ "cancelled": cancel() }))
}

async fn remove_route(State(app): State<Arc<App>>, UrlPath(id): UrlPath<String>) -> Response {
    match remove(app.store.data_dir(), &id).await {
        Ok(()) => ok(status_json(&app, &settings(&app))),
        Err(e) => fail(409, format!("Не удалось удалить модель: {e}")),
    }
}

/// Скачать модель из настроек, не дожидаясь нажатия на микрофон.
async fn download_route(State(app): State<Arc<App>>, UrlPath(id): UrlPath<String>) -> Response {
    let Some(model) = model(&id) else { return fail(404, "Такой модели нет") };
    let job = start_prepare(app.store.data_dir().to_path_buf(), model, emitter(&app));
    ok(json!({ "job": job, "progress": job.percent() }))
}

const STT_JS: &str = include_str!("../web/stt.js");

pub fn routes() -> Router<Arc<App>> {
    Router::new()
        .route("/stt.js", get(|| async { ([(axum::http::header::CONTENT_TYPE, "text/javascript; charset=utf-8"), (axum::http::header::CACHE_CONTROL, "no-cache")], STT_JS) }))
        .route("/api/voice/settings", get(get_settings).put(put_settings))
        .route("/api/voice/prepare", post(prepare_route))
        .route("/api/voice/local", get(job_route))
        .route("/api/voice/local/cancel", post(cancel_route))
        .route("/api/voice/local/{id}", delete(remove_route))
        .route("/api/voice/local/{id}/download", post(download_route))
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::AsyncReadExt;

    fn temp(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("crew-stt-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// Простейший HTTP-сервер с докачкой: понимает `Range: bytes=N-`. `cut` — после скольких
    /// байт оборвать первый ответ (имитация обрыва сети), `ranges` — умеет ли он докачку.
    async fn serve(bytes: Vec<u8>, cut: Option<usize>, ranges: bool) -> (String, Arc<Mutex<Vec<String>>>) {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let seen = Arc::new(Mutex::new(Vec::new()));
        let log = seen.clone();
        tokio::spawn(async move {
            let mut first = true;
            loop {
                let Ok((mut socket, _)) = listener.accept().await else { return };
                let mut buf = vec![0u8; 8192];
                let n = socket.read(&mut buf).await.unwrap_or(0);
                let head = String::from_utf8_lossy(&buf[..n]).to_string();
                let from = head.lines().find_map(|l| l.to_ascii_lowercase().strip_prefix("range: bytes=").map(|r| r.trim_end_matches('-').parse::<usize>().unwrap_or(0)));
                log.lock().unwrap().push(from.map(|f| format!("range {f}")).unwrap_or("full".into()));
                let (status, body) = match from {
                    Some(f) if ranges => ("206 Partial Content", bytes[f..].to_vec()),
                    _ => ("200 OK", bytes.clone()),
                };
                let send = if first { cut.map(|c| c.min(body.len())).unwrap_or(body.len()) } else { body.len() };
                first = false;
                let head = format!("HTTP/1.1 {status}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", body.len());
                let _ = socket.write_all(head.as_bytes()).await;
                let _ = socket.write_all(&body[..send]).await;
                let _ = socket.shutdown().await;
            }
        });
        (format!("http://127.0.0.1:{port}/a.tar.bz2"), seen)
    }

    /// tar.bz2 тем же встроенным tar: -j — bzip2.
    fn make_archive(dir: &Path, inner: &str, files: &[(&str, &[u8])]) -> Vec<u8> {
        let src = dir.join("src").join(inner);
        std::fs::create_dir_all(&src).unwrap();
        for (name, body) in files {
            std::fs::write(src.join(name), body).unwrap();
        }
        let out = dir.join("made.tar.bz2");
        let top = inner.split('/').next().unwrap();
        let status = std::process::Command::new(tar_exe()).arg("-c").arg("-j").arg("-f").arg(&out).arg("-C").arg(dir.join("src")).arg(top).status().unwrap();
        assert!(status.success());
        std::fs::read(out).unwrap()
    }

    fn pack_for(url: String, bytes: &[u8], inner: &str, files: &[&str]) -> Pack {
        Pack {
            dir: "stt-test-1".into(),
            archive: "test.tar.bz2".into(),
            size: bytes.len() as u64,
            sha256: to_hex(&Sha256::digest(bytes)),
            urls: vec![url],
            inner: inner.into(),
            files: files.iter().map(|f| f.to_string()).collect(),
        }
    }

    fn noop() -> impl Fn(u64) + Send + Sync {
        |_| {}
    }

    #[tokio::test]
    async fn installs_a_verified_archive_and_reports_progress() {
        let dir = temp("ok");
        let bytes = make_archive(&dir, "model-x/inner", &[("tokens.txt", b"a 0"), ("encoder.onnx", b"onnx")]);
        let (url, _) = serve(bytes.clone(), None, true).await;
        let pack = pack_for(url, &bytes, "model-x/inner", &["tokens.txt", "encoder.onnx"]);
        let data = dir.join("data");
        let seen = Arc::new(Mutex::new(Vec::new()));
        let log = seen.clone();
        install(&pack, &data, &CancellationToken::new(), &move |n| log.lock().unwrap().push(n)).await.unwrap();
        assert!(installed(&data, &pack));
        assert_eq!(std::fs::read(pack_dir(&data, &pack).join("tokens.txt")).unwrap(), b"a 0");
        assert_eq!(*seen.lock().unwrap().last().unwrap(), bytes.len() as u64);
        let engines: Vec<_> = std::fs::read_dir(engines_dir(&data)).unwrap().map(|e| e.unwrap().file_name().to_string_lossy().to_string()).collect();
        assert_eq!(engines, vec!["stt-test-1".to_string()], "архив и временная папка удалены");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[tokio::test]
    async fn wrong_checksum_installs_nothing_and_drops_the_part() {
        let dir = temp("sum");
        let bytes = make_archive(&dir, "m", &[("tokens.txt", b"a 0")]);
        let (url, _) = serve(bytes.clone(), None, true).await;
        let mut pack = pack_for(url, &bytes, "m", &["tokens.txt"]);
        pack.sha256 = "00".repeat(32);
        let data = dir.join("data");
        let error = install(&pack, &data, &CancellationToken::new(), &noop()).await.unwrap_err();
        assert!(error.contains("контрольная сумма"), "{error}");
        assert!(!installed(&data, &pack));
        assert!(!engines_dir(&data).join("test.tar.bz2.part").exists(), "битый архив не докачивается");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[tokio::test]
    async fn too_big_and_missing_files_are_refused() {
        let dir = temp("size");
        let bytes = make_archive(&dir, "m", &[("tokens.txt", b"a 0")]);
        let (url, _) = serve(bytes.clone(), None, true).await;
        let data = dir.join("data");
        let mut pack = pack_for(url.clone(), &bytes, "m", &["tokens.txt"]);
        pack.size -= 1;
        let error = install(&pack, &data, &CancellationToken::new(), &noop()).await.unwrap_err();
        assert!(error.contains("больше"), "{error}");
        let pack = pack_for(url, &bytes, "m", &["tokens.txt", "encoder.onnx"]);
        let error = install(&pack, &data, &CancellationToken::new(), &noop()).await.unwrap_err();
        assert!(error.contains("encoder.onnx"), "{error}");
        assert!(!installed(&data, &pack));
        let _ = std::fs::remove_dir_all(dir);
    }

    #[tokio::test]
    async fn a_broken_download_resumes_from_where_it_stopped() {
        let dir = temp("resume");
        let bytes: Vec<u8> = (0..200_000u32).map(|i| (i * 7 % 251) as u8).collect();
        let (url, seen) = serve(bytes.clone(), Some(50_000), true).await;
        let pack = pack_for(url.clone(), &bytes, "m", &[]);
        let part = dir.join("x.part");
        let error = download(&pack, &url, &part, &CancellationToken::new(), &noop()).await.unwrap_err();
        assert!(error.contains("не целиком") || !error.is_empty(), "{error}");
        let had = std::fs::metadata(&part).unwrap().len();
        assert!(had > 0 && had < bytes.len() as u64, "часть осталась: {had}");
        download(&pack, &url, &part, &CancellationToken::new(), &noop()).await.unwrap();
        assert_eq!(std::fs::read(&part).unwrap(), bytes);
        assert_eq!(seen.lock().unwrap()[1], format!("range {had}"), "вторая попытка докачивает");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[tokio::test]
    async fn a_server_without_ranges_starts_over() {
        let dir = temp("norange");
        let bytes: Vec<u8> = (0..100_000u32).map(|i| (i % 256) as u8).collect();
        let (url, _) = serve(bytes.clone(), None, false).await;
        let pack = pack_for(url.clone(), &bytes, "m", &[]);
        let part = dir.join("x.part");
        std::fs::write(&part, &bytes[..1000]).unwrap();
        download(&pack, &url, &part, &CancellationToken::new(), &noop()).await.unwrap();
        assert_eq!(std::fs::read(&part).unwrap(), bytes);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[tokio::test]
    async fn cancel_stops_the_download_and_keeps_the_part() {
        let dir = temp("cancel");
        let bytes: Vec<u8> = vec![7u8; 300_000];
        let (url, _) = serve(bytes.clone(), None, true).await;
        let pack = pack_for(url.clone(), &bytes, "m", &[]);
        let part = dir.join("x.part");
        let token = CancellationToken::new();
        token.cancel();
        let error = download(&pack, &url, &part, &token, &noop()).await.unwrap_err();
        assert_eq!(error, CANCELLED);
        // Отмена посреди потока: прогресс сам жмёт отмену после первых байт.
        let token = CancellationToken::new();
        let stop = token.clone();
        let error = download(&pack, &url, &part, &token, &move |n| {
            if n > 0 {
                stop.cancel()
            }
        })
        .await;
        match error {
            Err(e) => assert_eq!(e, CANCELLED),
            Ok(()) => assert_eq!(std::fs::read(&part).unwrap().len(), bytes.len(), "успело скачаться целиком одним куском"),
        }
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn release_pins_are_https_and_sha256() {
        for pack in std::iter::once(runtime_pack()).chain(models().into_iter().map(|m| m.pack)) {
            assert_eq!(pack.sha256.len(), 64, "{}", pack.dir);
            assert!(pack.urls.len() == 2 && pack.urls.iter().all(|u| u.starts_with("https://")), "{}", pack.dir);
            assert!(pack.urls[0].starts_with("https://api.github.com/"), "сначала API");
            assert!(pack.urls[1].ends_with(&pack.archive));
            assert!(pack.size > 1_000_000);
            assert!(pack.files.contains(&"tokens.txt".to_string()) || pack.dir.starts_with("sherpa-onnx"));
        }
        assert!(model(default_model()).is_some());
    }

    #[test]
    fn job_percent_never_overflows() {
        let mut job = Job::idle();
        assert_eq!(job.percent(), 0);
        job.total = 200;
        job.received = 50;
        assert_eq!(job.percent(), 25);
        job.received = 500;
        assert_eq!(job.percent(), 100);
    }

    #[test]
    fn settings_patch_is_checked() {
        let voice = VoiceSettings::default();
        assert_eq!(voice.engine, "local", "по умолчанию — бесплатно на компьютере");
        let got = apply(voice.clone(), &json!({ "engine": "server", "language": "ru", "serverUrl": "http://127.0.0.1:8000/v1/" })).unwrap();
        assert_eq!((got.engine.as_str(), got.language.as_str(), got.server_url.as_str()), ("server", "ru", "http://127.0.0.1:8000/v1"));
        assert_eq!(apply(voice.clone(), &json!({ "engine": "deloist" })).is_ok(), cloud_voice(), "облачный голос — только по флагу");
        assert!(apply(voice.clone(), &json!({ "engine": "cloud" })).is_err());
        assert!(apply(voice.clone(), &json!({ "model": "nope" })).is_err());
        assert!(apply(voice.clone(), &json!({ "language": "xx" })).is_err());
        assert!(apply(voice.clone(), &json!({ "serverUrl": "ftp://x" })).is_err());
        assert!(apply(voice, &json!({ "serverUrl": "" })).is_ok(), "пустой адрес стирает");
    }

    #[test]
    fn audio_for_the_model_is_16k() {
        let pcm: Vec<u8> = (0..4800i16).flat_map(|s| s.to_le_bytes()).collect();
        assert_eq!(to_16k(&pcm, 48000).len(), 1600 * 2);
        assert_eq!(to_16k(&pcm, 8000).len(), 9600 * 2);
        assert_eq!(to_16k(&pcm, 16000), pcm);
        let file = wav(&pcm, 16000);
        assert_eq!(&file[..4], b"RIFF");
        assert_eq!(file.len(), 44 + pcm.len());
        assert_eq!(u32::from_le_bytes(file[24..28].try_into().unwrap()), 16000);
    }

    #[test]
    fn multipart_body_has_the_file_and_fields() {
        let body = String::from_utf8_lossy(&multipart("b", &[("model", "whisper-1")], b"RIFFx")).to_string();
        assert!(body.contains("name=\"model\"\r\n\r\nwhisper-1\r\n"));
        assert!(body.contains("filename=\"speech.wav\""));
        assert!(body.ends_with("\r\n--b--\r\n"));
    }

    /// Настоящая модель на настоящей речи. Нужна скачанная модель:
    /// `DELOIST_STT_SMOKE=<папка data>` (в ней engines/sherpa-onnx-… и engines/stt-…).
    /// Без переменной проверка молча проходит.
    #[test]
    fn smoke_real_model_on_a_russian_phrase() {
        let Some(data) = std::env::var_os("DELOIST_STT_SMOKE").map(PathBuf::from) else { return };
        let wav = std::fs::read(Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/privet-bot.wav")).unwrap();
        let samples: Vec<f32> = wav[44..].chunks_exact(2).map(|b| f32::from(i16::from_le_bytes([b[0], b[1]])) / 32768.0).collect();
        for model in models().into_iter().filter(|m| installed(&data, &m.pack)) {
            let spec = spec(&data, &model);
            let rec = sherpa::Recognizer::load(&spec).unwrap();
            let text = rec.transcribe(&samples).unwrap().to_lowercase();
            assert!(text.contains("привет") && text.contains("бот"), "{}: {text}", model.id);
        }
    }
}
