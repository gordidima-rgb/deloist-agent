//! «Подключить свою ИИ»: провайдеры-агенты из реестра ACP и провайдеры по ключу API.
//!
//! Агента скачиваем в свою папку (`<данные>/agents/<id>`): пакет npm — через `npm install`,
//! готовую программу — архивом из реестра с проверкой sha256. После скачивания сразу пробный
//! запуск: агент сам находит вход, который уже есть на компьютере (например, вход Codex/ChatGPT),
//! и отдаёт свои модели. Нет входа — кнопка «Подключить», агент откроет браузер сам.
//!
//! Бот работает с провайдером через драйвер `acp:<id>`.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use serde_json::{json, Map, Value};
use sha2::{Digest, Sha256};
use tokio::io::AsyncWriteExt;
use tokio_util::sync::CancellationToken;

use crate::drivers::acp;
use crate::App;

mod antigravity;
mod antigravity_usage;

pub const ANTIGRAVITY: &str = antigravity::ID;

const REGISTRY_URL: &str = "https://cdn.agentclientprotocol.com/registry/v1/latest/registry.json";
const REGISTRY_TTL: Duration = Duration::from_secs(6 * 3600);
const PROVIDER_REGISTRY_REFRESH_EVERY: Duration = Duration::from_secs(60 * 60);
const PROVIDER_REGISTRY_FIRST: Duration = Duration::from_secs(5 * 60);

fn provider_refresh_period() -> Duration {
    if cfg!(debug_assertions) {
        if let Some(ms) = std::env::var("CREW_TEST_PROVIDER_REFRESH_MS").ok().and_then(|v| v.parse().ok()) {
            return Duration::from_millis(ms);
        }
    }
    PROVIDER_REGISTRY_REFRESH_EVERY
}

fn provider_refresh_first() -> Duration {
    if cfg!(debug_assertions) {
        if let Some(ms) = std::env::var("CREW_TEST_PROVIDER_REFRESH_MS").ok().and_then(|v| v.parse().ok()) {
            return Duration::from_millis(ms);
        }
    }
    PROVIDER_REGISTRY_FIRST
}
const INSTALL_TIMEOUT: Duration = Duration::from_secs(15 * 60);

/// Провайдеры на первом экране, в этом порядке, с привычными людям названиями.
/// Остальные агенты реестра находятся поиском.
const FEATURED: &[(&str, &str)] = &[
    ("codex-acp", "ChatGPT"),
    ("claude-acp", "Claude"),
    ("gemini", "Gemini CLI"),
    ("grok-build", "Grok"),
    ("github-copilot-cli", "GitHub Copilot"),
    ("cursor", "Cursor"),
    ("cline", "Cline"),
    ("qwen-code", "Qwen Code"),
    ("pi-acp", "Pi"),
    ("devin", "Devin"),
    ("opencode", "OpenCode"),
    ("goose", "Goose"),
];

/// Провайдер по адресу: OpenAI-совместимый API (или GigaChat), свой ключ и своя модель.
pub struct BuiltIn {
    pub id: &'static str,
    pub name: &'static str,
    pub about: &'static str,
    pub base_url: &'static str,
    /// `""` — Bearer, `api-key` — Yandex, `gigachat` — ключ авторизации Сбера, `none` — без ключа.
    pub auth: &'static str,
    pub key_label: &'static str,
    pub default_model: &'static str,
    pub models: &'static [(&'static str, &'static str)],
    /// Нужен каталог Yandex Cloud: модель зовётся `gpt://<каталог>/<модель>`.
    pub folder: bool,
    /// Сервер на этом компьютере: ищется сам, ключ не нужен.
    pub local: bool,
    /// У провайдера есть официальный бесплатный тариф.
    pub free: bool,
    /// Страница, где создаётся ключ API (или пустая строка).
    pub key_url: &'static str,
    /// Документированные адреса консолей провайдера, откуда можно сохранить ключ
    /// (origin'ы; могут совпадать с origin `key_url`).
    pub console_origins: &'static [&'static str],
}

pub const BUILT_IN: &[BuiltIn] = &[
    BuiltIn {
        id: "gigachat", name: "GigaChat", about: "Модели Сбера. Ключ авторизации — в личном кабинете developers.sber.ru",
        base_url: "https://gigachat.devices.sberbank.ru/api/v1", auth: "gigachat", key_label: "Ключ авторизации",
        default_model: "GigaChat-2", models: crate::drivers::gigachat::MODELS, folder: false, local: false,
        free: false, key_url: "https://developers.sber.ru/studio",
        console_origins: &[],
    },
    BuiltIn {
        id: "alice", name: "Алиса", about: "Alice AI LLM и YandexGPT через Yandex AI Studio. Нужны API-ключ и идентификатор каталога",
        base_url: "https://llm.api.cloud.yandex.net/v1", auth: "api-key", key_label: "API-ключ Yandex AI Studio",
        default_model: "aliceai-llm-flash/latest",
        models: &[
            ("aliceai-llm-flash/latest", "Alice AI LLM Flash"),
            ("aliceai-llm/latest", "Alice AI LLM"),
            ("yandexgpt/latest", "YandexGPT Pro"),
            ("yandexgpt-lite/latest", "YandexGPT Lite"),
        ],
        folder: true, local: false,
        free: false, key_url: "https://console.cloud.yandex.ru",
        console_origins: &[],
    },
    BuiltIn {
        id: "deepseek", name: "DeepSeek", about: "Модели DeepSeek по ключу с platform.deepseek.com",
        base_url: "https://api.deepseek.com/v1", auth: "", key_label: "Ключ API", default_model: "deepseek-chat",
        models: &[], folder: false, local: false,
        free: false, key_url: "https://platform.deepseek.com/api_keys",
        console_origins: &[],
    },
    BuiltIn {
        id: "openrouter", name: "OpenRouter", about: "Сотни моделей по одному ключу с openrouter.ai. Есть бесплатные модели (суффикс :free)",
        base_url: "https://openrouter.ai/api/v1", auth: "", key_label: "Ключ API", default_model: "openrouter/free",
        models: &[], folder: false, local: false,
        free: true, key_url: "https://openrouter.ai/keys",
        console_origins: &["https://openrouter.ai"],
    },
    BuiltIn {
        id: "nvidia", name: "NVIDIA NIM", about: "Модели NVIDIA NIM. Бесплатный тариф с лимитами в минуту и в день",
        base_url: "https://integrate.api.nvidia.com/v1", auth: "", key_label: "Ключ API", default_model: "",
        models: &[], folder: false, local: false,
        free: true, key_url: "https://build.nvidia.com",
        console_origins: &["https://build.nvidia.com"],
    },
    BuiltIn {
        id: "groq", name: "Groq", about: "Быстрые модели Groq. Бесплатный тариф с лимитами в минуту и в день",
        base_url: "https://api.groq.com/openai/v1", auth: "", key_label: "Ключ API", default_model: "",
        models: &[], folder: false, local: false,
        free: true, key_url: "https://console.groq.com/keys",
        console_origins: &["https://console.groq.com"],
    },
    BuiltIn {
        id: "cerebras", name: "Cerebras", about: "Быстрые модели Cerebras по ключу с cloud.cerebras.ai",
        base_url: "https://api.cerebras.ai/v1", auth: "", key_label: "Ключ API", default_model: "",
        models: &[], folder: false, local: false,
        free: false, key_url: "https://cloud.cerebras.ai",
        console_origins: &[],
    },
    BuiltIn {
        id: "zai", name: "Z.ai (GLM)", about: "Модели GLM от Z.ai. Модели Flash бесплатны, с лимитами",
        base_url: "https://api.z.ai/api/paas/v4", auth: "", key_label: "Ключ API", default_model: "glm-4.7-flash",
        models: &[], folder: false, local: false,
        free: true, key_url: "https://z.ai/manage-apikey/apikey-list",
        console_origins: &["https://z.ai"],
    },
    BuiltIn {
        id: "gemini", name: "Google AI Studio", about: "Модели Google Gemini. Бесплатный тариф с лимитами в минуту и в день",
        base_url: "https://generativelanguage.googleapis.com/v1beta/openai", auth: "", key_label: "Ключ API", default_model: "",
        models: &[], folder: false, local: false,
        free: true, key_url: "https://aistudio.google.com/apikey",
        console_origins: &["https://aistudio.google.com"],
    },
    BuiltIn {
        id: "ollama-cloud", name: "Ollama Cloud",
        about: "Большие открытые модели (DeepSeek, GLM, Kimi, gpt-oss) на серверах Ollama. Оплата по токенам или подписка; запросы не хранятся и не идут в обучение",
        base_url: "https://ollama.com/v1", auth: "", key_label: "Ключ API", default_model: "",
        models: &[], folder: false, local: false,
        free: false, key_url: "https://ollama.com/settings/keys",
        console_origins: &[],
    },
    BuiltIn {
        id: "ollama", name: "Ollama", about: "Модели на этом компьютере через Ollama",
        base_url: "http://127.0.0.1:11434/v1", auth: "none", key_label: "", default_model: "",
        models: &[], folder: false, local: true,
        free: false, key_url: "",
        console_origins: &[],
    },
    BuiltIn {
        id: "lmstudio", name: "LM Studio", about: "Модели на этом компьютере через LM Studio",
        base_url: "http://127.0.0.1:1234/v1", auth: "none", key_label: "", default_model: "",
        models: &[], folder: false, local: true,
        free: false, key_url: "",
        console_origins: &[],
    },
];

/// Встроенный провайдер по идентификатору.
pub fn built_in(id: &str) -> Option<&'static BuiltIn> {
    BUILT_IN.iter().find(|b| b.id == id)
}

/// Документированные адреса консолей провайдера, откуда разрешено сохранять ключ.
pub fn provider_console_origins(id: &str) -> &'static [&'static str] {
    built_in(id).map(|b| b.console_origins).unwrap_or(&[])
}

/// Origin адреса `key_url` провайдера.
pub fn key_url_origin(id: &str) -> Option<String> {
    let url = built_in(id)?.key_url;
    if url.is_empty() {
        return None;
    }
    let parsed = reqwest::Url::parse(url).ok()?;
    if !matches!(parsed.scheme(), "http" | "https") || parsed.host_str().is_none() {
        return None;
    }
    Some(parsed.origin().ascii_serialization())
}
/// Куда идёт бот на драйвере `openai:<id>`.
pub struct Route {
    pub base_url: String,
    pub api_key: String,
    pub auth: String,
    pub model: String,
}

/// Свой адрес: только http(s), без пробелов. Убираем хвост `/chat/completions`, если его вписали целиком.
fn check_url(url: &str) -> Result<String, String> {
    let url = url.trim().trim_end_matches('/');
    let url = url.strip_suffix("/chat/completions").unwrap_or(url);
    if !(url.starts_with("http://") || url.starts_with("https://")) || url.contains(char::is_whitespace) || url.len() > 300 {
        return Err("адрес нужен вида http://127.0.0.1:8000/v1".into());
    }
    Ok(url.to_string())
}

fn server_id(url: &str) -> String {
    let short: String = url
        .trim_start_matches("http://")
        .trim_start_matches("https://")
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c.to_ascii_lowercase() } else { '-' })
        .collect();
    format!("server-{}", short.trim_matches('-')).chars().take(60).collect()
}

/// Модели локального сервера, если он отвечает. У Ollama свой список (`/api/tags`), у остальных — OpenAI `/models`.
async fn local_models(client: &reqwest::Client, kind: &str, base_url: &str) -> Option<Vec<Value>> {
    let (url, ollama) = if kind == "ollama" {
        (format!("{}/api/tags", base_url.trim_end_matches("/v1")), true)
    } else {
        (format!("{}/models", base_url.trim_end_matches('/')), false)
    };
    let response = client.get(url).send().await.ok()?;
    if !response.status().is_success() {
        return None;
    }
    let body: Value = response.json().await.ok()?;
    let list = if ollama { body["models"].as_array().cloned()? } else { body["data"].as_array().cloned()? };
    let running = if ollama { ollama_running(client, base_url).await } else { HashMap::new() };
    let mut models: Vec<Value> = Vec::new();
    for m in &list {
        let Some(id) = m["name"].as_str().or(m["id"].as_str()) else { continue };
        let mut entry = json!({ "id": id, "name": id });
        // Окно, как его отдаёт сервер: у Ollama — то, что он реально загрузит, у остальных — поле `/models`.
        let context = if ollama && models.len() < 40 {
            ollama_context(client, base_url, id, running.get(id).copied()).await
        } else {
            crate::context::from_model_json(m)
        };
        if let Some(context) = context {
            entry["context"] = json!(context);
        }
        models.push(entry);
    }
    Some(models)
}

/// Загруженные в Ollama модели и окно каждой (`context_length` в `/api/ps`, если сервер его отдаёт).
async fn ollama_running(client: &reqwest::Client, base_url: &str) -> HashMap<String, usize> {
    let url = format!("{}/api/ps", base_url.trim_end_matches("/v1"));
    let Ok(response) = client.get(url).send().await else { return HashMap::new() };
    let Ok(body) = response.json::<Value>().await else { return HashMap::new() };
    body["models"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|m| Some((m["name"].as_str()?.to_string(), m["context_length"].as_u64()? as usize)))
        .collect()
}

/// Окно модели Ollama по `/api/show`: `num_ctx` из Modelfile или 4096 по умолчанию, не больше возможностей модели.
async fn ollama_context(client: &reqwest::Client, base_url: &str, model: &str, running: Option<usize>) -> Option<usize> {
    let url = format!("{}/api/show", base_url.trim_end_matches("/v1"));
    let response = client.post(url).json(&json!({ "model": model })).send().await.ok()?;
    if !response.status().is_success() {
        return None;
    }
    let show: Value = response.json().await.ok()?;
    Some(crate::context::ollama_window(&show, running))
}

#[derive(Clone)]
struct Job {
    /// installing, checking, connecting
    phase: &'static str,
    percent: Option<u8>,
    cancel: CancellationToken,
}

const PROBE_TIMEOUT_WARNING: &str = "Не удалось проверить подключение вовремя. Показано последнее известное состояние.";

fn preserve_probe_timeout(entry: &mut serde_json::Map<String, Value>, timed_out: bool, credentials_present: bool) -> bool {
    if timed_out && credentials_present && entry.get("connected") == Some(&json!(true)) {
        entry.insert("error".into(), json!(PROBE_TIMEOUT_WARNING));
        true
    } else {
        false
    }
}

#[derive(Default)]
pub struct Providers {
    jobs: Mutex<HashMap<String, Job>>,
    registry: Mutex<Option<(Instant, Value)>>,
    saved: Mutex<Option<Map<String, Value>>>,
    /// Когда последний раз искали локальные серверы моделей.
    scanned: Mutex<Option<Instant>>,
    /// Последний релиз Antigravity CLI с GitHub и время неудачной попытки его получить.
    release: Mutex<Option<(Instant, Value)>>,
    release_failed: Mutex<Option<Instant>>,
    /// Идущий вход в OpenRouter через браузер («Начать бесплатно»).
    pub(crate) login: Mutex<Option<(u64, tokio::task::AbortHandle)>>,
}

fn saved_path(app: &App) -> PathBuf {
    app.store.data_dir().join("providers.json")
}

fn agents_dir(app: &App) -> PathBuf {
    app.store.data_dir().join("agents")
}

/// Реестр можно подменить файлом — только в отладочной сборке, для тестов.
fn registry_override() -> Option<PathBuf> {
    if cfg!(debug_assertions) {
        std::env::var_os("CREW_TEST_ACP_REGISTRY").map(PathBuf::from)
    } else {
        None
    }
}

fn platform() -> &'static str {
    match (std::env::consts::OS, std::env::consts::ARCH) {
        ("windows", "aarch64") => "windows-aarch64",
        ("windows", _) => "windows-x86_64",
        ("macos", "aarch64") => "darwin-aarch64",
        ("macos", _) => "darwin-x86_64",
        (_, "aarch64") => "linux-aarch64",
        _ => "linux-x86_64",
    }
}

fn http() -> Result<reqwest::Client, String> {
    reqwest::Client::builder()
        .connect_timeout(Duration::from_secs(15))
        .user_agent(concat!("Deloist/", env!("CARGO_PKG_VERSION")))
        .build()
        .map_err(|e| e.to_string())
}

/// Усилие рассуждения, которое драйвер может передать провайдеру.
pub const EFFORTS: [&str; 3] = ["low", "medium", "high"];

/// Уровень модели, который бот просит у провайдера.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ModelLevel {
    Strong,
    Standard,
    Fast,
}

impl ModelLevel {
    pub fn parse(text: &str) -> Option<Self> {
        match text.trim().to_lowercase().as_str() {
            "strong" => Some(Self::Strong),
            "standard" => Some(Self::Standard),
            "fast" => Some(Self::Fast),
            _ => None,
        }
    }

    pub fn key(self) -> &'static str {
        match self {
            Self::Strong => "strong",
            Self::Standard => "standard",
            Self::Fast => "fast",
        }
    }

    /// Ступень выше: fast → standard → strong. Сильнее strong нет.
    pub fn up(self) -> Self {
        match self {
            Self::Fast => Self::Standard,
            Self::Standard | Self::Strong => Self::Strong,
        }
    }

    /// Ступень ниже: strong → standard → fast. Ниже fast нет.
    pub fn down(self) -> Option<Self> {
        match self {
            Self::Strong => Some(Self::Standard),
            Self::Standard => Some(Self::Fast),
            Self::Fast => None,
        }
    }
}

/// Таблица уровней: для каждого провайдера/драйвера — id модели на strong / standard / fast.
/// «проверено 12.11.2026». Пустая строка — модель провайдера по умолчанию (локальные серверы):
/// там все три уровня дают выбранную человеком модель.
pub const MODEL_LEVELS: &[(&str, [&str; 3])] = &[
    ("anthropic", ["claude-opus-5-5", "claude-sonnet-5-5", "claude-haiku-4-5"]),
    ("gemini", ["gemini-2.5-flash", "gemini-2.5-flash", "gemini-2.5-flash"]),
    // У OpenAI в коде есть только флагман и mini: среднюю модель уточнить.
    ("api", ["gpt-6.1-terra", "gpt-4o-mini", "gpt-4o-mini"]),
    ("deloist", ["gpt-6.1-terra", "deepseek-flash", "deepseek-flash"]),
    ("deepseek", ["deepseek-v4-pro", "deepseek-v4-flash", "deepseek-v4-flash"]),
    ("gigachat", ["GigaChat-2-Max", "GigaChat-2-Pro", "GigaChat-2"]),
    ("alice", ["aliceai-llm/latest", "aliceai-llm/latest", "aliceai-llm-flash/latest"]),
    // Бесплатный старт: без явного выбора человека платные модели не берём (`openrouter/auto` может уйти на платные).
    ("openrouter", ["openrouter/free", "openrouter/free", "openrouter/free"]),
    // TODO уточнить: у NVIDIA, Groq и Cerebras модель выбирают на экране провайдеров — берём её.
    ("nvidia", ["", "", ""]),
    ("groq", ["", "", ""]),
    ("cerebras", ["", "", ""]),
    // TODO уточнить: у Z.ai в коде только Flash, остальные модели появятся, когда провайдер их назовёт.
    ("zai", ["glm-4.7-flash", "glm-4.7-flash", "glm-4.7-flash"]),
    // Локальный сервер: все три уровня — выбранная человеком модель.
    ("ollama", ["", "", ""]),
    ("ollama-cloud", ["", "", ""]),
    ("lmstudio", ["", "", ""]),
];

fn level_index(level: ModelLevel) -> usize {
    match level {
        ModelLevel::Strong => 0,
        ModelLevel::Standard => 1,
        ModelLevel::Fast => 2,
    }
}

/// Модель провайдера для указанного уровня.
pub fn level_model(provider: &str, level: ModelLevel) -> Option<&'static str> {
    MODEL_LEVELS.iter().find(|(p, _)| *p == provider).map(|(_, models)| models[level_index(level)])
}

/// Ключ таблицы для драйвера: `openai:deepseek` → `deepseek`.
pub fn level_provider(driver: &str) -> &str {
    driver.strip_prefix("openai:").unwrap_or(driver)
}

/// Модель уровня для драйвера бота.
pub fn level_model_for_driver(driver: &str, level: ModelLevel) -> Option<&'static str> {
    level_model(level_provider(driver), level)
}

/// Все три модели провайдера разом (strong, standard, fast).
pub fn level_models(provider: &str) -> Option<[&'static str; 3]> {
    MODEL_LEVELS.iter().find(|(p, _)| *p == provider).map(|(_, models)| *models)
}

/// Таблица для интерфейса: провайдер → `{ strong, standard, fast }`.
pub fn levels_json() -> Value {
    let mut out = Map::new();
    for (provider, models) in MODEL_LEVELS {
        out.insert(
            (*provider).to_string(),
            json!({ "strong": models[0], "standard": models[1], "fast": models[2] }),
        );
    }
    Value::Object(out)
}

/// Остаток лимита подписки, при котором пора предложить ступень ниже (использовано ≥ 90%).
pub const LIMIT_HINT_LEFT: i64 = 10;

/// Подсказка при почти исчерпанном лимите подписки: на ступень ниже.
/// `None`, если лимит ещё не тесен или ниже fast уже некуда.
pub fn level_hint(left: i64, level: Option<ModelLevel>) -> Option<ModelLevel> {
    if left > LIMIT_HINT_LEFT {
        return None;
    }
    level.unwrap_or(ModelLevel::Standard).down()
}

/// Принимает ли модель OpenAI-совместимый `reasoning_effort`.
/// Список нарочно узкий: GigaChat, Алиса, Gemini и локальные серверы отвечают 400 на незнакомое поле.
pub fn takes_effort(driver: &str, model: &str) -> bool {
    let openai_compatible = driver == "api" || driver == "deloist" || driver.starts_with("openai:");
    if !openai_compatible {
        return false;
    }
    if matches!(level_provider(driver), "gigachat" | "alice" | "gemini" | "ollama" | "ollama-cloud" | "lmstudio") {
        return false;
    }
    // Только модели OpenAI, у которых `reasoning_effort` документирован.
    // TODO уточнить: DeepSeek V4 и GLM принимают ли этот параметр.
    let model = model.trim().to_lowercase();
    ["o1", "o3", "o4", "gpt-5", "gpt-6"].iter().any(|prefix| model.starts_with(prefix))
}

/// Усилие рассуждения для API-провайдеров, чьи модели принимают `reasoning_effort`.
pub fn api_efforts() -> Vec<Value> {
    vec![json!({ "id": "low", "name": "Низкое" }), json!({ "id": "medium", "name": "Среднее" }), json!({ "id": "high", "name": "Высокое" })]
}

impl Providers {
    // -----------------------------------------------------------------------
    // Сохранённое состояние
    // -----------------------------------------------------------------------

    fn load(&self, app: &App) -> Map<String, Value> {
        let mut saved = self.saved.lock().unwrap();
        if saved.is_none() {
            let data = std::fs::read_to_string(saved_path(app)).ok().and_then(|t| serde_json::from_str::<Value>(&t).ok());
            *saved = Some(data.and_then(|d| d.as_object().cloned()).unwrap_or_default());
        }
        saved.clone().unwrap_or_default()
    }

    pub(crate) fn entry(&self, app: &App, id: &str) -> Value {
        self.load(app).get(id).cloned().unwrap_or_else(|| json!({}))
    }

    pub(crate) fn update_entry(&self, app: &App, id: &str, change: impl FnOnce(&mut Map<String, Value>)) {
        self.load(app);
        let snapshot = {
            let mut saved = self.saved.lock().unwrap();
            let all = saved.get_or_insert_with(Map::new);
            let entry = all.entry(id.to_string()).or_insert_with(|| json!({}));
            if !entry.is_object() {
                *entry = json!({});
            }
            change(entry.as_object_mut().unwrap());
            Value::Object(all.clone())
        };
        let path = saved_path(app);
        let tmp = path.with_extension("json.tmp");
        if std::fs::write(&tmp, serde_json::to_vec_pretty(&snapshot).unwrap_or_default()).is_ok() {
            let _ = std::fs::rename(&tmp, &path);
        }
    }

    /// Полная команда запуска скачанного агента для драйвера `acp:<id>`.
    pub fn command(&self, app: &App, id: &str) -> Option<String> {
        if let Some(account) = antigravity::split(id) {
            return self.ag_command(app, account);
        }
        self.entry(app, id)["command"].as_str().filter(|c| !c.is_empty()).map(str::to_string)
    }

    // -----------------------------------------------------------------------
    // Реестр
    // -----------------------------------------------------------------------

    async fn registry(&self, app: &App) -> Result<Value, String> {
        if let Some(path) = registry_override() {
            let text = std::fs::read_to_string(&path).map_err(|e| format!("реестр: {e}"))?;
            return serde_json::from_str(&text).map_err(|e| format!("реестр: {e}"));
        }
        if let Some((at, data)) = self.registry.lock().unwrap().clone() {
            if at.elapsed() < REGISTRY_TTL {
                return Ok(data);
            }
        }
        let cache = app.store.data_dir().join("acp-registry.json");
        let fetched = async {
            let response = http()?.get(REGISTRY_URL).timeout(Duration::from_secs(20)).send().await.map_err(|e| e.to_string())?;
            if !response.status().is_success() {
                return Err(format!("HTTP {}", response.status().as_u16()));
            }
            response.json::<Value>().await.map_err(|e| e.to_string())
        }
        .await;
        match fetched {
            Ok(data) if data["agents"].is_array() => {
                let _ = std::fs::write(&cache, data.to_string());
                *self.registry.lock().unwrap() = Some((Instant::now(), data.clone()));
                Ok(data)
            }
            other => {
                let why = other.err().unwrap_or_else(|| "неожиданный ответ".into());
                let text = std::fs::read_to_string(&cache).map_err(|_| format!("не смог получить список агентов: {why}"))?;
                serde_json::from_str(&text).map_err(|e| e.to_string())
            }
        }
    }

    async fn agent(&self, app: &App, id: &str) -> Result<Value, String> {
        if id == antigravity::ID {
            return Ok(self.antigravity_agent(app, false).await);
        }
        let registry = self.registry(app).await?;
        registry["agents"]
            .as_array()
            .and_then(|a| a.iter().find(|a| a["id"] == id))
            .cloned()
            .ok_or_else(|| format!("агента «{id}» нет в реестре"))
    }

    // -----------------------------------------------------------------------
    // Что показать
    // -----------------------------------------------------------------------

    fn used_by(app: &App, driver: &str) -> usize {
        let settings = app.store.settings();
        app.store
            .bots()
            .iter()
            .filter(|b| b.driver == driver || (b.driver.is_empty() && settings.driver == driver))
            .count()
    }

    fn view(&self, app: &App, agent: &Value, featured_name: Option<&str>) -> Value {
        let id = agent["id"].as_str().unwrap_or_default();
        let saved = self.entry(app, id);
        let distribution = &agent["distribution"];
        let kind = if distribution["binary"][platform()].is_object() {
            "binary"
        } else if distribution["npx"].is_object() {
            "npx"
        } else if distribution["uvx"].is_object() {
            "uvx"
        } else {
            "none"
        };
        let installed = saved["version"].as_str().filter(|_| saved["command"].as_str().is_some_and(|c| !c.is_empty()));
        let latest = agent["version"].as_str().unwrap_or_default();
        let job = self.jobs.lock().unwrap().get(id).cloned();
        let status = match (&job, installed) {
            (Some(job), _) => job.phase,
            (None, None) if saved["installError"].is_string() => "error",
            (None, None) => "not_installed",
            // Агент может открыть сессию и без входа (claude-agent-acp так и делает), а сам
            // сообщает `authStatus.kind = none` — «Not logged in». Это не подключение: ответы не пойдут.
            (None, Some(_)) if saved["connected"] == true && saved["account"]["kind"] != "none" => "connected",
            (None, Some(_)) if saved["error"].is_string() => "error",
            (None, Some(_)) => "not_connected",
        };
        let driver = format!("acp:{id}");
        json!({
            "id": id,
            "kind": "acp",
            "name": featured_name.or(agent["name"].as_str()).unwrap_or(id),
            "agentName": agent["name"],
            "description": agent["description"],
            "icon": agent["icon"],
            "website": agent["website"].as_str().or(agent["repository"].as_str()),
            "license": agent["license"],
            "featured": featured_name.is_some(),
            "distribution": kind,
            "supported": kind == "npx" || kind == "binary",
            "latest": latest,
            "installed": installed,
            "update": installed.is_some_and(|v| v != latest && !latest.is_empty()),
            "status": status,
            "progress": job.as_ref().map(|j| json!({ "phase": j.phase, "percent": j.percent })),
            "account": saved["account"],
            "limits": saved["limits"].as_array().cloned().unwrap_or_default(),
            "limitsAt": saved["limitsAt"],
            "models": saved["models"].as_array().cloned().unwrap_or_default(),
            "efforts": saved["efforts"].as_array().cloned().unwrap_or_default(),
            "effortDefault": saved["effortDefault"],
            "options": saved["options"].as_array().cloned().unwrap_or_default(),
            "hiddenModels": saved["hiddenModels"].as_array().cloned().unwrap_or_default(),
            "enabled": saved["enabled"] != false,
            "error": if saved["error"].is_string() { saved["error"].clone() } else { saved["installError"].clone() },
            "driver": driver,
            "usedBy": Self::used_by(app, &driver),
            "isDefault": app.store.settings().driver == driver,
        })
    }

    // -----------------------------------------------------------------------
    // Провайдеры по адресу: ключевые (DeepSeek, OpenRouter, Алиса, GigaChat),
    // локальные (Ollama, LM Studio, свои серверы) и добавленные человеком
    // -----------------------------------------------------------------------

    fn builtin(id: &str) -> Option<&'static BuiltIn> {
        BUILT_IN.iter().find(|b| b.id == id)
    }

    /// Провайдер по адресу: встроенный, найденный на компьютере или свой.
    fn is_address(&self, app: &App, id: &str) -> bool {
        Self::builtin(id).is_some() || self.entry(app, id)["custom"] == "openai" || self.entry(app, id)["server"] == true
    }

    /// Есть ли у человека провайдер `openai:<id>`.
    pub fn has_address(&self, app: &App, id: &str) -> bool {
        self.is_address(app, id)
    }

    /// Куда и с каким ключом идёт бот на драйвере `openai:<id>`.
    pub fn route(&self, app: &App, id: &str, wanted: &str) -> Route {
        let entry = self.entry(app, id);
        let built = Self::builtin(id);
        let base_url = entry["baseUrl"].as_str().filter(|u| !u.is_empty()).map(str::to_string)
            .or_else(|| built.map(|b| b.base_url.to_string()))
            .unwrap_or_default();
        let auth = built.map(|b| b.auth.to_string()).or_else(|| entry["auth"].as_str().map(str::to_string)).unwrap_or_default();
        let mut model = [wanted, entry["model"].as_str().unwrap_or_default(), built.map(|b| b.default_model).unwrap_or_default()]
            .into_iter()
            .find(|m| !m.trim().is_empty())
            .unwrap_or_default()
            .trim()
            .to_string();
        // Модели Yandex AI Studio зовутся по каталогу: gpt://<каталог>/<модель>.
        if built.is_some_and(|b| b.folder) && !model.starts_with("gpt://") && !model.is_empty() {
            let folder = entry["folder"].as_str().unwrap_or_default();
            let path = if model.contains('/') { model.clone() } else { format!("{model}/latest") };
            model = format!("gpt://{folder}/{path}");
        }
        Route { base_url, api_key: app.store.own_key(id), auth, model }
    }

    /// Минимальный остаток лимита подписки (в процентах), сохранённый для провайдера.
    /// Читает providers.json без запуска агента: ход не ждёт проверки `/status`.

    pub fn subscription_left(&self, app: &App, id: &str) -> Option<i64> {
        self.entry(app, id)["limits"].as_array()?.iter().filter_map(|l| l["left"].as_i64()).min()
    }

    /// Окно модели в токенах, которое провайдер назвал в списке моделей (для Ollama — загружаемое окно).
    pub fn model_context(&self, app: &App, id: &str, model: &str) -> Option<usize> {
        let entry = self.entry(app, id);
        entry["models"]
            .as_array()?
            .iter()
            .find(|m| m["id"].as_str() == Some(model))
            .and_then(|m| m["context"].as_u64())
            .map(|n| n as usize)
    }

    fn address_view(&self, app: &App, id: &str) -> Value {
        let entry = self.entry(app, id);
        let built = Self::builtin(id);
        let auth = built.map(|b| b.auth).or(entry["auth"].as_str()).unwrap_or("");
        let needs_key = auth != "none";
        let has_key = !app.store.own_key(id).is_empty();
        let job = self.jobs.lock().unwrap().get(id).cloned();
        let status = if let Some(job) = &job {
            job.phase
        } else if entry["connected"] == true && (has_key || !needs_key) {
            "connected"
        } else if entry["error"].is_string() {
            "error"
        } else {
            "not_connected"
        };
        let mut models: Vec<Value> = entry["models"].as_array().cloned().unwrap_or_default();
        if models.is_empty() {
            models = built.map(|b| b.models.iter().map(|(id, name)| json!({ "id": id, "name": name })).collect()).unwrap_or_default();
        }
        // Окно в токенах: что назвал провайдер, иначе то, что известно по имени модели.
        for model in models.iter_mut() {
            if model["context"].is_null() {
                if let Some(context) = model["id"].as_str().and_then(crate::context::known_tokens) {
                    model["context"] = json!(context);
                }
            }
        }
        let driver = format!("openai:{id}");
        let efforts = if models.iter().any(|m| m["id"].as_str().is_some_and(|m| takes_effort(&driver, m))) { api_efforts() } else { Vec::new() };
        let name = entry["name"].as_str().map(str::to_string).or_else(|| built.map(|b| b.name.to_string())).unwrap_or_else(|| id.to_string());
        json!({
            "id": id,
            "kind": "key",
            "signingIn": id == "openrouter" && self.login_active(),
            "name": name,
            "description": built.map(|b| b.about).or(entry["about"].as_str()),
            "featured": built.is_some_and(|b| !b.local),
            "local": built.is_some_and(|b| b.local) || entry["server"] == true,
            "custom": entry["custom"] == "openai",
            "baseUrl": self.route(app, id, "").base_url,
            "auth": auth,
            "needsKey": needs_key,
            "keyLabel": built.map(|b| b.key_label).unwrap_or("Ключ API"),
            "free": built.is_some_and(|b| b.free),
            "keyUrl": built.map(|b| b.key_url).unwrap_or(""),
            "needsFolder": built.is_some_and(|b| b.folder),
            "folder": entry["folder"],
            "hasKey": has_key,
            "model": entry["model"].as_str().unwrap_or_default(),
            "defaultModel": built.map(|b| b.default_model).unwrap_or_default(),
            "models": models,
            "efforts": efforts,
            "hiddenModels": entry["hiddenModels"].as_array().cloned().unwrap_or_default(),
            "enabled": entry["enabled"] != false,
            "found": entry["found"] == true,
            "status": status,
            "error": entry["error"],
            "driver": driver,
            "usedBy": Self::used_by(app, &driver),
            "isDefault": app.store.settings().driver == driver,
        })
    }

    /// Проверка адреса и ключа: список моделей. У Yandex списка может не быть — тогда пробный ответ на один токен.
    async fn verify_address(&self, app: &App, id: &str) -> Result<Vec<Value>, String> {
        let route = self.route(app, id, "");
        if route.auth == "gigachat" {
            return crate::drivers::gigachat::models(&route.api_key).await;
        }
        if route.auth != "none" && route.api_key.is_empty() {
            return Err("впиши ключ".into());
        }
        let client = http()?;
        let with_auth = |request: reqwest::RequestBuilder| match route.auth.as_str() {
            "none" => request,
            "api-key" => request.header("authorization", format!("Api-Key {}", route.api_key)),
            _ => request.bearer_auth(&route.api_key),
        };
        let url = format!("{}/models", route.base_url.trim_end_matches('/'));
        let response = with_auth(client.get(&url)).timeout(Duration::from_secs(15)).send().await.map_err(|e| format!("адрес не ответил: {}", e.without_url()))?;
        let status = response.status().as_u16();
        if status == 401 || status == 403 {
            return Err(format!("API {status}: ключ не подошёл"));
        }
        if response.status().is_success() {
            let body: Value = response.json().await.unwrap_or(Value::Null);
            let list = body["data"].as_array().or(body["models"].as_array()).cloned().unwrap_or_default();
            // Окно модели, если провайдер его называет (OpenRouter, LM Studio и другие).
            let models: Vec<Value> = list
                .iter()
                .filter_map(|m| Some((m["id"].as_str().or(m["name"].as_str()).or(m.as_str())?, crate::context::from_model_json(m))))
                .map(|(id, context)| match context {
                    Some(context) => json!({ "id": id, "name": id, "context": context }),
                    None => json!({ "id": id, "name": id }),
                })
                .collect();
            return Ok(models);
        }
        // Списка нет: проверяем ключ и модель коротким ответом.
        if route.model.is_empty() {
            return Err(format!("API {status}: адрес не отдал список моделей, выбери модель вручную"));
        }
        let body = json!({ "model": route.model, "max_tokens": 1, "messages": [{ "role": "user", "content": "ping" }] });
        let url = format!("{}/chat/completions", route.base_url.trim_end_matches('/'));
        let response = with_auth(client.post(&url)).json(&body).timeout(Duration::from_secs(30)).send().await.map_err(|e| format!("адрес не ответил: {}", e.without_url()))?;
        let status = response.status().as_u16();
        if !response.status().is_success() {
            let text: String = response.text().await.unwrap_or_default().chars().take(300).collect();
            return Err(format!("API {status}: {text}"));
        }
        Ok(Vec::new())
    }

    pub async fn check_address(&self, app: &App, id: &str) -> Result<Vec<Value>, String> {
        let result = self.verify_address(app, id).await;
        let checked = result.clone();
        self.update_entry(app, id, |e| {
            e.insert("checkedAt".into(), json!(crate::store::now()));
            match result {
                Ok(models) => {
                    e.insert("connected".into(), json!(true));
                    e.remove("error");
                    if !models.is_empty() {
                        e.insert("models".into(), json!(models));
                    }
                }
                Err(error) => {
                    e.insert("connected".into(), json!(false));
                    e.insert("error".into(), json!(error));
                }
            }
        });
        checked
    }

    async fn patch_address(&self, app: &Arc<App>, id: &str, body: &Value) -> Result<Value, String> {
        if let Some(key) = body.get("apiKey").and_then(Value::as_str) {
            app.store.set_own_key(id, key).await.map_err(|e| e.to_string())?;
        }
        let custom = self.entry(app, id)["custom"] == "openai";
        self.update_entry(app, id, |e| {
            for field in ["model", "folder"] {
                if let Some(value) = body.get(field).and_then(Value::as_str) {
                    e.insert(field.into(), json!(value.trim()));
                }
            }
            if custom {
                if let Some(value) = body.get("name").and_then(Value::as_str).filter(|v| !v.trim().is_empty()) {
                    e.insert("name".into(), json!(value.trim()));
                }
            }
            if let Some(enabled) = body.get("enabled").and_then(Value::as_bool) {
                e.insert("enabled".into(), json!(enabled));
            }
            if let Some(hidden) = body.get("hiddenModels").and_then(Value::as_array) {
                let list: Vec<Value> = hidden.iter().filter(|v| v.is_string()).cloned().collect();
                e.insert("hiddenModels".into(), json!(list));
            }
        });
        if custom || self.entry(app, id)["server"] == true {
            if let Some(url) = body.get("baseUrl").and_then(Value::as_str) {
                let url = check_url(url)?;
                self.update_entry(app, id, |e| {
                    e.insert("baseUrl".into(), json!(url));
                });
            }
        }
        let changed = ["apiKey", "folder", "baseUrl", "verify"].iter().any(|k| body.get(*k).is_some());
        if changed {
            let _ = self.check_address(app, id).await;
        }
        if body.get("default") == Some(&json!(true)) {
            app.store.update_settings(&json!({ "driver": format!("openai:{id}"), "model": "" })).await.map_err(|e| e.to_string())?;
        }
        let view = self.address_view(app, id);
        app.turns.emit(json!({ "type": "provider", "provider": view }));
        Ok(view)
    }

    /// «Добавить своё»: OpenAI-совместимый адрес (в том числе Ollama и LM Studio на другом адресе) или свой агент ACP.
    pub async fn create(&self, app: &Arc<App>, body: &Value) -> Result<Value, String> {
        let kind = body["kind"].as_str().unwrap_or_default();
        let name = body["name"].as_str().map(str::trim).filter(|n| !n.is_empty()).ok_or("назови провайдера")?.to_string();
        let id = format!("own-{}", &crate::store::uid("p").replace('_', "-")[2..].to_lowercase());
        let id: String = id.chars().filter(|c| c.is_ascii_alphanumeric() || *c == '-').take(40).collect();
        match kind {
            "openai" => {
                let url = check_url(body["baseUrl"].as_str().unwrap_or_default())?;
                let auth = if body["apiKey"].as_str().is_some_and(|k| !k.trim().is_empty()) { "" } else { "none" };
                self.update_entry(app, &id, |e| {
                    e.insert("custom".into(), json!("openai"));
                    e.insert("name".into(), json!(name));
                    e.insert("baseUrl".into(), json!(url));
                    e.insert("auth".into(), json!(auth));
                    if let Some(model) = body["model"].as_str() {
                        e.insert("model".into(), json!(model.trim()));
                    }
                });
                if let Some(key) = body["apiKey"].as_str().filter(|k| !k.trim().is_empty()) {
                    app.store.set_own_key(&id, key).await.map_err(|e| e.to_string())?;
                }
                let _ = self.check_address(app, &id).await;
                let view = self.address_view(app, &id);
                app.turns.emit(json!({ "type": "provider", "provider": view }));
                Ok(view)
            }
            "acp" => {
                crate::cloud::deny(crate::cloud::NO_DRIVER)?;
                let command = body["command"].as_str().map(str::trim).filter(|c| !c.is_empty()).ok_or("впиши команду запуска агента")?.to_string();
                self.update_entry(app, &id, |e| {
                    e.insert("custom".into(), json!("acp"));
                    e.insert("name".into(), json!(name));
                    e.insert("command".into(), json!(command));
                    e.insert("version".into(), json!("своя команда"));
                });
                Ok(self.get(app, &id).await?)
            }
            _ => Err("непонятный вид провайдера".into()),
        }
    }

    /// Ollama, LM Studio и свои адреса серверов моделей: что отвечает прямо сейчас.
    pub async fn scan(&self, app: &App) -> Value {
        *self.scanned.lock().unwrap() = Some(Instant::now());
        let client = match reqwest::Client::builder().no_proxy().timeout(Duration::from_millis(1500)).build() {
            Ok(client) => client,
            Err(_) => return json!({ "found": [] }),
        };
        let mut found: Vec<String> = Vec::new();
        for built in BUILT_IN.iter().filter(|b| b.local) {
            let models = local_models(&client, built.id, built.base_url).await;
            let alive = models.is_some();
            self.update_entry(app, built.id, |e| {
                e.insert("found".into(), json!(alive));
                if let Some(models) = &models {
                    e.insert("connected".into(), json!(true));
                    e.remove("error");
                    e.insert("models".into(), json!(models));
                } else if e.get("connected") == Some(&json!(true)) {
                    e.insert("connected".into(), json!(false));
                    e.insert("error".into(), json!("сервер не отвечает: он запущен?"));
                }
            });
            if alive {
                found.push(built.id.to_string());
            }
        }
        for url in app.store.settings().model_servers {
            let Ok(url) = check_url(&url) else { continue };
            let id = server_id(&url);
            let models = local_models(&client, "openai", &url).await;
            let alive = models.is_some();
            self.update_entry(app, &id, |e| {
                e.insert("server".into(), json!(true));
                e.insert("name".into(), json!(url.trim_start_matches("http://").trim_start_matches("https://").trim_end_matches("/v1")));
                e.insert("baseUrl".into(), json!(url));
                e.insert("auth".into(), json!("none"));
                e.insert("found".into(), json!(alive));
                e.insert("connected".into(), json!(alive));
                if let Some(models) = &models {
                    e.insert("models".into(), json!(models));
                    e.remove("error");
                } else {
                    e.insert("error".into(), json!("сервер не отвечает"));
                }
            });
            if alive {
                found.push(id);
            }
        }
        json!({ "found": found })
    }


    /// Первый экран: избранные агенты, провайдеры по ключу и скачанные из остального реестра.
    /// С `query` — поиск по всему реестру.
    pub async fn list(&self, app: &App, query: &str) -> Result<Value, String> {
        let registry = self.registry(app).await?;
        let agents = registry["agents"].as_array().cloned().unwrap_or_default();
        let stale = self.scanned.lock().unwrap().is_none_or(|at| at.elapsed() > Duration::from_secs(60));
        if stale {
            self.scan(app).await;
        }
        let saved = self.load(app);
        let query = query.trim().to_lowercase();
        let mut out: Vec<Value> = Vec::new();
        if query.is_empty() {
            for (index, (id, name)) in FEATURED.iter().enumerate() {
                if *id == "gemini" {
                    let own = self.antigravity_agent(app, false).await;
                    out.extend(self.ag_views(app, &own));
                }
                if let Some(agent) = agents.iter().find(|a| a["id"] == *id) {
                    out.push(self.view(app, agent, Some(name)));
                }
                // Российские провайдеры — сразу за ChatGPT и Claude.
                if index == 1 {
                    out.push(self.address_view(app, "gigachat"));
                    out.push(self.address_view(app, "alice"));
                }
            }
            for built in BUILT_IN.iter().filter(|b| b.id != "gigachat" && b.id != "alice") {
                // Локальный сервер показываем, когда он нашёлся или уже настроен.
                let entry = saved.get(built.id);
                if built.local && !entry.is_some_and(|e| e["found"] == true || e["connected"] == true) {
                    continue;
                }
                out.push(self.address_view(app, built.id));
            }
            for agent in &agents {
                let id = agent["id"].as_str().unwrap_or_default();
                let known = FEATURED.iter().any(|(f, _)| *f == id);
                if !known && saved.get(id).is_some_and(|s| s["command"].is_string()) {
                    out.push(self.view(app, agent, None));
                }
            }
            for (id, entry) in &saved {
                if entry["custom"] == "openai" || entry["server"] == true {
                    out.push(self.address_view(app, id));
                } else if entry["custom"] == "acp" {
                    out.push(self.custom_agent_view(app, id));
                }
            }
        } else {
            for agent in &agents {
                let id = agent["id"].as_str().unwrap_or_default();
                let featured = FEATURED.iter().find(|(f, _)| *f == id).map(|(_, n)| *n);
                let text = format!(
                    "{} {} {} {}",
                    id,
                    agent["name"].as_str().unwrap_or_default(),
                    agent["description"].as_str().unwrap_or_default(),
                    featured.unwrap_or_default()
                )
                .to_lowercase();
                if text.contains(&query) {
                    out.push(self.view(app, agent, featured));
                }
            }
        }
        if !query.is_empty() {
            if "gemini google antigravity гемини".contains(&query) || query.contains("gemini") {
                let own = self.antigravity_agent(app, false).await;
                out.extend(self.ag_views(app, &own));
            }
            for built in BUILT_IN {
                if format!("{} {} {}", built.id, built.name, built.about).to_lowercase().contains(&query) {
                    out.push(self.address_view(app, built.id));
                }
            }
        }
        // «Подписка Deloist» — не провайдер из реестра, но её модели тоже выбирают в общем выборе модели.
        Ok(json!({ "providers": out, "total": agents.len(), "plan": crate::account::plan_json() }))
    }

    /// Свой агент ACP: команда запуска вместо скачивания.
    fn custom_agent_view(&self, app: &App, id: &str) -> Value {
        let entry = self.entry(app, id);
        let agent = json!({ "id": id, "name": entry["name"], "version": "своя команда", "distribution": {} });
        let mut view = self.view(app, &agent, None);
        view["custom"] = json!(true);
        view["supported"] = json!(true);
        view["command"] = entry["command"].clone();
        view["update"] = json!(false);
        view
    }

    pub async fn get(&self, app: &App, id: &str) -> Result<Value, String> {
        if id == antigravity::ID {
            let own = self.antigravity_agent(app, false).await;
            return Ok(self.ag_views(app, &own).remove(0));
        }
        if self.is_address(app, id) {
            return Ok(self.address_view(app, id));
        }
        if self.entry(app, id)["custom"] == "acp" {
            return Ok(self.custom_agent_view(app, id));
        }
        let agent = self.agent(app, id).await?;
        let featured = FEATURED.iter().find(|(f, _)| *f == id).map(|(_, n)| *n);
        Ok(self.view(app, &agent, featured))
    }

    pub(crate) async fn emit(&self, app: &Arc<App>, id: &str) {
        if let Ok(view) = self.get(app, id).await {
            app.turns.emit(json!({ "type": "provider", "provider": view }));
        }
    }

    /// Обновить кэш реестра ACP и разослать события, чтобы флаг «обновить» стал виден без клика.
    pub async fn refresh_registry(&self, app: &Arc<App>) {
        *self.registry.lock().unwrap() = None;
        if let Ok(data) = self.registry(app).await {
            if let Some(agents) = data["agents"].as_array() {
                for agent in agents {
                    if let Some(id) = agent["id"].as_str() {
                        // Только установленные (есть сохранённая версия): у них может измениться флаг update.
                        if !self.entry(app, id)["version"].is_string() {
                            continue;
                        }
                        if let Ok(view) = self.get(app, id).await {
                            app.turns.emit(json!({ "type": "provider", "provider": view }));
                        }
                    }
                }
            }
        }
    }

    /// Начать работу над провайдером. Идёт другая — отказ, и её ход (этап, проценты) не меняется:
    /// обновление списка или повторное нажатие не должны сбивать вход или загрузку.
    fn begin_job(&self, id: &str, phase: &'static str, percent: Option<u8>) -> Option<CancellationToken> {
        if self.jobs.lock().unwrap().contains_key(id) {
            return None;
        }
        self.set_job(id, phase, percent)
    }

    fn set_job(&self, id: &str, phase: &'static str, percent: Option<u8>) -> Option<CancellationToken> {
        let mut jobs = self.jobs.lock().unwrap();
        if let Some(job) = jobs.get_mut(id) {
            job.phase = phase;
            job.percent = percent;
            return None;
        }
        let cancel = CancellationToken::new();
        jobs.insert(id.to_string(), Job { phase, percent, cancel: cancel.clone() });
        Some(cancel)
    }

    /// Значок агента из реестра. Окно не грузит картинки с чужих адресов, поэтому значок скачиваем
    /// сами (только с CDN реестра), храним в папке данных и отдаём со своего адреса.
    /// `currentColor` в SVG заменяется на цвет темы: в `<img>` он иначе всегда чёрный.
    pub async fn icon(&self, app: &App, id: &str, color: &str) -> Result<Vec<u8>, String> {
        let id = safe_id(id)?;
        let color = if color.len() == 6 && color.bytes().all(|b| b.is_ascii_hexdigit()) { color } else { "000000" };
        let dir = app.store.data_dir().join("agent-icons");
        let path = dir.join(format!("{id}.svg"));
        let svg = match std::fs::read(&path) {
            Ok(bytes) => bytes,
            Err(_) => {
                let agent = self.agent(app, id).await?;
                let url = agent["icon"].as_str().ok_or("у агента нет значка")?;
                if !url.starts_with("https://cdn.agentclientprotocol.com/") || !url.ends_with(".svg") {
                    return Err("значок не с CDN реестра".into());
                }
                let response = http()?.get(url).timeout(Duration::from_secs(15)).send().await.map_err(|e| e.to_string())?;
                if !response.status().is_success() {
                    return Err(format!("HTTP {}", response.status().as_u16()));
                }
                let bytes = response.bytes().await.map_err(|e| e.to_string())?.to_vec();
                if bytes.len() > 256 * 1024 || !String::from_utf8_lossy(&bytes).contains("<svg") {
                    return Err("значок не похож на SVG".into());
                }
                let _ = std::fs::create_dir_all(&dir);
                let _ = std::fs::write(&path, &bytes);
                bytes
            }
        };
        let text = String::from_utf8_lossy(&svg).replace("currentColor", &format!("#{color}"));
        Ok(text.into_bytes())
    }

    // -----------------------------------------------------------------------
    // Действия
    // -----------------------------------------------------------------------

    /// Скачать (или обновить) агента и сразу проверить вход. Работа идёт в фоне, ход виден событиями.
    pub async fn install(self: &Arc<Self>, app: &Arc<App>, id: &str) -> Result<Value, String> {
        crate::cloud::deny(crate::cloud::NO_INSTALL)?;
        let agent = self.agent(app, id).await?;
        // У архива проценты настоящие, у npm их нет: там бегущая полоса.
        let percent = agent["distribution"]["binary"][platform()].is_object().then_some(0);
        let Some(cancel) = self.begin_job(id, "installing", percent) else {
            return Err("уже идёт".into());
        };
        let (me, app2, id2) = (self.clone(), app.clone(), id.to_string());
        tokio::spawn(async move {
            let prev_saved = me.entry(&app2, &id2);
            let prev_version = prev_saved["version"].clone();
            let prev_command = prev_saved["command"].clone();
            let had_prev = prev_version.is_string() && prev_command.is_string();
            let result = tokio::select! {
                result = tokio::time::timeout(INSTALL_TIMEOUT, me.download(&app2, &id2, &agent)) => {
                    result.unwrap_or_else(|_| Err("скачивание не закончилось за 15 минут".into()))
                }
                _ = cancel.cancelled() => Err("отменено".into()),
            };
            match result {
                Ok((version, command)) => {
                    if id2 == antigravity::ID {
                        me.ag_remember_previous(&app2, &version);
                    }
                    me.update_entry(&app2, &id2, |e| {
                        e.insert("version".into(), json!(version));
                        e.insert("command".into(), json!(command));
                        e.remove("error");
                        e.remove("installError");
                    });
                    if id2 == antigravity::ID {
                        me.ag_after_install(&app2, &cancel).await;
                    } else {
                        me.set_job(&id2, "checking", None);
                        me.emit(&app2, &id2).await;
                        let probe_ok = me.run_probe(&app2, &id2, false, &cancel).await;
                        let dir = agents_dir(&app2).join(safe_id(&id2).unwrap_or(&id2));
                        let old = dir.parent().unwrap_or(dir.as_path()).join(format!("{}.old", dir.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default()));
                        if probe_ok {
                            let _ = tokio::fs::remove_dir_all(&old).await;
                        } else if old.exists() {
                            let _ = tokio::fs::remove_dir_all(&dir).await;
                            let _ = tokio::fs::rename(&old, &dir).await;
                            if had_prev {
                                me.update_entry(&app2, &id2, |e| {
                                    e.insert("version".into(), prev_version);
                                    e.insert("command".into(), prev_command);
                                });
                            } else {
                                me.update_entry(&app2, &id2, |e| {
                                    e.remove("version");
                                    e.remove("command");
                                });
                            }
                        }
                    }
                }
                Err(e) => {
                    if e != "отменено" {
                        me.update_entry(&app2, &id2, |entry| {
                            entry.insert("installError".into(), json!(e));
                        });
                    }
                }
            }
            me.jobs.lock().unwrap().remove(&id2);
            me.emit(&app2, &id2).await;
        });
        self.get(app, id).await
    }

    /// Проверка входа (`login=false`) или вход (`login=true`: агент откроет браузер).
    pub async fn check(self: &Arc<Self>, app: &Arc<App>, id: &str, login: bool) -> Result<Value, String> {
        crate::cloud::deny(crate::cloud::NO_DRIVER)?;
        if id == antigravity::ID && login {
            return self.connect(app).await;
        }
        if self.command(app, id).is_none() {
            return Err("сначала скачай агента".into());
        }
        let phase = if login { "connecting" } else { "checking" };
        let Some(cancel) = self.begin_job(id, phase, None) else {
            return Err("уже идёт".into());
        };
        let (me, app2, id2) = (self.clone(), app.clone(), id.to_string());
        tokio::spawn(async move {
            me.run_probe(&app2, &id2, login, &cancel).await;
            me.jobs.lock().unwrap().remove(&id2);
            me.emit(&app2, &id2).await;
        });
        self.get(app, id).await
    }

    pub fn cancel(&self, id: &str) -> bool {
        match self.jobs.lock().unwrap().get(id) {
            Some(job) => {
                job.cancel.cancel();
                true
            }
            None => false,
        }
    }

    async fn run_probe(&self, app: &Arc<App>, id: &str, login: bool, cancel: &CancellationToken) -> bool {
        if id == antigravity::ID {
            self.ag_probe_all(app, cancel).await;
            return true;
        }
        let Some(command) = self.command(app, id) else { return false };
        let settings = app.store.settings();
        let hidden = ["OPENAI_API_KEY".to_string(), settings.api_key_env.clone()];
        let spec = acp::Spec { command: &command, env: &[], hidden_env: &hidden, mode: "", model: "", effort: "", options: &[], temp_root: None, auth: "" };
        let dir = agents_dir(app).join("_probe");
        let _ = std::fs::create_dir_all(&dir);
        let result = acp::probe(&spec, &dir, login, cancel).await;
        let probe_ok = result.is_ok();
        self.update_entry(app, id, |e| {
            e.insert("checkedAt".into(), json!(crate::store::now()));
            match result {
                Ok(probe) if preserve_probe_timeout(e, probe.timed_out, probe.account["kind"] != "none") => {}
                Ok(probe) => {
                    e.insert("connected".into(), json!(probe.connected));
                    if !probe.account.is_null() {
                        e.insert("account".into(), probe.account);
                    } else if !probe.connected {
                        e.remove("account");
                    }
                    if !probe.models.is_empty() {
                        e.insert("models".into(), json!(probe.models));
                    }
                    if probe.connected {
                        // Агент без такой настройки стирает прежний список: он мог её потерять после обновления.
                        e.insert("efforts".into(), json!(probe.efforts));
                        e.insert("options".into(), json!(probe.options));
                        match probe.effort_default {
                            Some(d) => e.insert("effortDefault".into(), json!(d)),
                            None => e.remove("effortDefault"),
                        };
                    }
                    match probe.error {
                        Some(error) if error != "остановлено" => e.insert("error".into(), json!(error)),
                        _ => e.remove("error"),
                    };
                    e.insert("agentInfo".into(), probe.agent);
                    if !probe.limits.is_empty() {
                        e.insert("limits".into(), json!(probe.limits));
                    }
                    // Время пробы помечаем и без лимитов: агент, который их не сообщает, не опрашивается каждый раз.
                    if probe.connected {
                        e.insert("limitsAt".into(), json!(crate::store::now()));
                    }
                }
                Err(error) => {
                    e.insert("connected".into(), json!(false));
                    e.insert("error".into(), json!(error));
                }
            }
        });
        probe_ok
    }

    /// Видимость, «использовать» и «по умолчанию для новых ботов».
    pub async fn patch(&self, app: &Arc<App>, id: &str, body: &Value) -> Result<Value, String> {
        if self.is_address(app, id) {
            return self.patch_address(app, id, body).await;
        }
        if self.entry(app, id)["custom"] != "acp" {
            self.agent(app, id).await?;
        }
        self.update_entry(app, id, |e| {
            if let Some(enabled) = body.get("enabled").and_then(Value::as_bool) {
                e.insert("enabled".into(), json!(enabled));
            }
            if let Some(hidden) = body.get("hiddenModels").and_then(Value::as_array) {
                let list: Vec<Value> = hidden.iter().filter(|v| v.is_string()).cloned().collect();
                e.insert("hiddenModels".into(), json!(list));
            }
        });
        if body.get("default") == Some(&json!(true)) {
            if self.command(app, id).is_none() {
                return Err("сначала скачай и подключи агента".into());
            }
            app.store.update_settings(&json!({ "driver": format!("acp:{id}"), "model": "" })).await.map_err(|e| e.to_string())?;
        }
        let view = self.get(app, id).await?;
        app.turns.emit(json!({ "type": "provider", "provider": view }));
        Ok(view)
    }

    /// Удалить скачанного агента с диска.
    pub async fn remove(&self, app: &Arc<App>, id: &str) -> Result<Value, String> {
        let entry = self.entry(app, id);
        // Свой провайдер удаляется целиком, у встроенного забываем ключ и проверку.
        if entry["custom"].is_string() || entry["server"] == true || Self::builtin(id).is_some() {
            app.store.set_own_key(id, "").await.map_err(|e| e.to_string())?;
            let whole = entry["custom"].is_string() || entry["server"] == true;
            self.load(app);
            let snapshot = {
                let mut saved = self.saved.lock().unwrap();
                let all = saved.get_or_insert_with(Map::new);
                if whole {
                    all.remove(id);
                } else if let Some(e) = all.get_mut(id).and_then(Value::as_object_mut) {
                    for key in ["connected", "models", "efforts", "options", "effortDefault", "error", "folder", "model"] {
                        e.remove(key);
                    }
                }
                Value::Object(all.clone())
            };
            let _ = std::fs::write(saved_path(app), serde_json::to_vec_pretty(&snapshot).unwrap_or_default());
            return Ok(if whole { json!({ "id": id, "removed": true }) } else { self.address_view(app, id) });
        }
        if id == antigravity::ID {
            self.disconnect_all(app).await?;
        }
        let dir = agents_dir(app).join(safe_id(id)?);
        let _ = tokio::fs::remove_dir_all(&dir).await;
        self.update_entry(app, id, |e| {
            for key in ["version", "command", "previous", "connected", "account", "models", "efforts", "options", "effortDefault", "error", "installError"] {
                e.remove(key);
            }
        });
        self.get(app, id).await
    }

    /// Лимиты подписок для кружка в левом меню. Устаревшие (старше 10 минут) обновляются в фоне,
    /// но только у агентов, которыми пользуются боты: зря агентов не запускаем.
    pub async fn limits(self: &Arc<Self>, app: &Arc<App>) -> Value {
        let saved = self.load(app);
        let now = crate::store::now();
        let mut out = Vec::new();
        // Gemini (Antigravity): квота Google по каждому аккаунту с входом Google. Ключ API квоты не даёт.
        let accounts = self.ag_accounts(app);
        // Адреса значков берём из реестра (он на диске и в памяти, 6 часов): телефон рисует их через <img>.
        let registry = tokio::time::timeout(Duration::from_secs(3), self.registry(app)).await.ok().and_then(Result::ok);
        let icon_url = |id: &str| -> Value {
            registry.as_ref().and_then(|r| r["agents"].as_array()).and_then(|a| a.iter().find(|a| a["id"] == id)).map(|a| a["icon"].clone()).unwrap_or(Value::Null)
        };
        let several = accounts.iter().filter(|(_, a)| a["connected"] == true).count() > 1;
        for (index, (aid, account)) in accounts.iter().enumerate() {
            if account["connected"] != true {
                continue;
            }
            let (limits, status, at) = if aid == "key" { (Vec::new(), "unsupported", None) } else { antigravity_usage::limits(&antigravity::home_of(app, aid)).await };
            let id = if index == 0 { antigravity::ID.to_string() } else { format!("{}@{aid}", antigravity::ID) };
            let name = match account["email"].as_str().filter(|_| several) {
                Some(email) => format!("Gemini · {email}"),
                None => "Gemini".to_string(),
            };
            out.push(json!({ "id": id, "name": name, "limits": limits, "limitsStatus": status, "at": at, "icon": true, "iconUrl": icon_url(&id) }));
        }
        for (id, entry) in &saved {
            if id == antigravity::ID {
                continue;
            }
            if entry["connected"] != true || !entry["command"].is_string() {
                continue;
            }
            // Кружок есть у каждого подключённого агента, поэтому лимиты обновляем и у тех, кем боты сейчас
            // не пользуются: реже, раз в час. Агент без лимитов пробуем раз в полчаса: проба могла сорваться.
            let used = Self::used_by(app, &format!("acp:{id}")) > 0;
            let has_limits = entry["limits"].as_array().is_some_and(|l| !l.is_empty());
            let due = match (has_limits, used) {
                (true, true) => 10 * 60_000,
                (true, false) => 60 * 60_000,
                (false, _) => 30 * 60_000,
            };
            let stale = entry["limitsAt"].as_i64().is_none_or(|at| now - at > due);
            // A timed-out background probe should not be restarted on every menu refresh.
            let retry_pending = entry["error"] == PROBE_TIMEOUT_WARNING
                && entry["checkedAt"].as_i64().is_some_and(|at| now - at < 5 * 60_000);
            let busy = self.jobs.lock().unwrap().contains_key(id.as_str());
            if stale && !busy && !retry_pending {
                // `check` сам уходит в фон (`/usage` у Claude отвечает до полуминуты): меню не ждёт,
                // новые лимиты придут событием `provider`, и оно перерисуется само.
                let _ = self.check(app, id, false).await;
            }
            // Подключённый агент виден всегда: без лимитов у него пустые кружки и пояснение в подсказке.
            let limits = entry["limits"].as_array().cloned().unwrap_or_default();
            let name = FEATURED.iter().find(|(f, _)| *f == id.as_str()).map(|(_, n)| n.to_string())
                .or_else(|| entry["name"].as_str().map(str::to_string))
                .unwrap_or_else(|| id.clone());
            out.push(json!({ "id": id, "name": name, "limits": limits, "at": entry["limitsAt"], "icon": true, "iconUrl": icon_url(id) }));
        }
        json!({ "providers": out })
    }

    // -----------------------------------------------------------------------
    // Скачивание
    // -----------------------------------------------------------------------

    async fn download(&self, app: &Arc<App>, id: &str, agent: &Value) -> Result<(String, String), String> {
        let version = agent["version"].as_str().unwrap_or_default().to_string();
        let dir = if id == antigravity::ID {
            if let Some(why) = agent["releaseError"].as_str() {
                return Err(format!("не смог узнать последнюю версию Gemini: {why}"));
            }
            self.ag_version_dir(app, &version)?
        } else {
            agents_dir(app).join(safe_id(id)?)
        };
        let distribution = &agent["distribution"];
        if let Some(binary) = distribution["binary"].get(platform()).filter(|b| b.is_object()) {
            let command = self.download_binary(app, id, binary, &dir).await?;
            return Ok((version, command));
        }
        if let Some(npx) = distribution["npx"].as_object() {
            let command = self.install_npm(app, id, agent, npx, &dir).await?;
            return Ok((version, command));
        }
        if distribution["uvx"].is_object() {
            return Err("этот агент ставится через Python (uvx): Deloist так пока не умеет".into());
        }
        Err("у агента нет сборки для этого компьютера".into())
    }

    async fn download_binary(&self, app: &Arc<App>, id: &str, binary: &Value, dir: &Path) -> Result<String, String> {
        crate::cloud::deny(crate::cloud::NO_INSTALL)?;
        let url = binary["archive"].as_str().ok_or("в реестре нет адреса архива")?;
        // Строгий режим (Gemini): сумма обязательна, размер ограничен, адрес только https с GitHub.
        let strict = binary["strict"] == true;
        let expected = binary["sha256"].as_str().filter(|s| !s.is_empty());
        if strict && expected.is_none() {
            return Err("у релиза нет контрольной суммы: не ставлю".into());
        }
        let local = url.strip_prefix("file:///").filter(|_| cfg!(debug_assertions));
        if local.is_none() {
            if !url.starts_with("https://") {
                return Err("архив агента не по https: не скачиваю".into());
            }
            if strict && !antigravity::has_release_override() && !url.starts_with(antigravity::DOWNLOAD_PREFIX) {
                return Err("архив не с официальной страницы релизов: не скачиваю".into());
            }
        }
        let cap = if strict { antigravity::SIZE_CAP } else { u64::MAX };
        let parent = dir.parent().map(|p| p.to_path_buf()).unwrap_or_else(|| PathBuf::from("."));
        let staging = parent.join(format!("{}.staging", dir.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default()));
        let old = parent.join(format!("{}.old", dir.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default()));
        let _ = tokio::fs::remove_dir_all(&staging).await;
        tokio::fs::create_dir_all(&staging).await.map_err(|e| e.to_string())?;
        let name = url.rsplit('/').next().unwrap_or("agent.zip");
        let name = if name.ends_with(".zip") || name.ends_with(".tar.gz") || name.ends_with(".tgz") || name.ends_with(".tar.xz") { name } else { "agent.zip" };
        let archive = staging.join(format!("_download-{name}"));
        let fetched = self.fetch_archive(app, id, url, local, &archive, cap).await;
        let actual = match fetched {
            Ok(actual) => actual,
            Err(e) => {
                let _ = tokio::fs::remove_dir_all(&staging).await;
                return Err(e);
            }
        };
        if let Some(expected) = expected {
            if !actual.eq_ignore_ascii_case(expected) {
                let _ = tokio::fs::remove_dir_all(&staging).await;
                return Err("архив агента не совпал с контрольной суммой: не ставлю".into());
            }
        }
        self.set_job(id, "unpacking", None);
        self.emit(app, id).await;
        let unpacked = extract(&archive, &staging).await;
        let _ = tokio::fs::remove_file(&archive).await;
        if let Err(e) = unpacked {
            let _ = tokio::fs::remove_dir_all(&staging).await;
            return Err(e);
        }

        let cmd = binary["cmd"].as_str().ok_or("в реестре нет команды запуска")?;
        let relative = cmd.trim_start_matches("./").trim_start_matches(".\\").replace('/', "\\");
        let program = staging.join(&relative);
        if !program.starts_with(&staging) || relative.contains("..") {
            let _ = tokio::fs::remove_dir_all(&staging).await;
            return Err("команда агента указывает за пределы его папки".into());
        }
        if !program.exists() {
            let _ = tokio::fs::remove_dir_all(&staging).await;
            return Err(format!("в архиве нет {}", program.display()));
        }
        // Swap старой и новой папок.
        if dir.exists() {
            let _ = tokio::fs::remove_dir_all(&old).await;
            if let Err(_) = tokio::fs::rename(dir, &old).await {
                let _ = tokio::fs::remove_dir_all(&staging).await;
                return Err("агент сейчас запущен: обнови, когда он закончит".into());
            }
        }
        if let Err(e) = tokio::fs::rename(&staging, dir).await {
            if old.exists() {
                let _ = tokio::fs::rename(&old, dir).await;
            }
            let _ = tokio::fs::remove_dir_all(&staging).await;
            return Err(format!("не смог переложить файлы: {e}"));
        }
        Ok(command_line(&dir.join(&relative).display().to_string(), binary["args"].as_array()))
    }

    /// Архив в файл с подсчётом sha256 на лету; больше `cap` байт не берём.
    async fn fetch_archive(&self, app: &Arc<App>, id: &str, url: &str, local: Option<&str>, archive: &Path, cap: u64) -> Result<String, String> {
        let mut hasher = Sha256::new();
        let mut file = tokio::fs::File::create(archive).await.map_err(|e| e.to_string())?;
        if let Some(path) = local {
            let bytes = std::fs::read(path).map_err(|e| e.to_string())?;
            if bytes.len() as u64 > cap {
                return Err("архив слишком большой: не скачиваю".into());
            }
            hasher.update(&bytes);
            file.write_all(&bytes).await.map_err(|e| e.to_string())?;
            self.set_job(id, "installing", Some(99));
            self.emit(app, id).await;
        } else {
            let mut response = http()?.get(url).send().await.map_err(|e| format!("не скачался архив: {e}"))?;
            if !response.status().is_success() {
                return Err(format!("не скачался архив: HTTP {}", response.status().as_u16()));
            }
            let total = response.content_length();
            if total.is_some_and(|t| t > cap) {
                return Err("архив слишком большой: не скачиваю".into());
            }
            let mut got: u64 = 0;
            let mut last = 0u8;
            while let Some(chunk) = response.chunk().await.map_err(|e| format!("архив оборвался: {e}"))? {
                got += chunk.len() as u64;
                if got > cap {
                    return Err("архив слишком большой: не скачиваю".into());
                }
                hasher.update(&chunk);
                file.write_all(&chunk).await.map_err(|e| e.to_string())?;
                if let Some(total) = total.filter(|t| *t > 0) {
                    let percent = ((got * 100) / total).min(99) as u8;
                    if percent >= last + 3 {
                        last = percent;
                        self.set_job(id, "installing", Some(percent));
                        self.emit(app, id).await;
                    }
                }
            }
        }
        file.flush().await.map_err(|e| e.to_string())?;
        drop(file);
        self.set_job(id, "verifying", None);
        self.emit(app, id).await;
        Ok(hasher.finalize().iter().map(|b| format!("{b:02x}")).collect())
    }

    /// Поставить npm-агента: Node — системный 20+ или управляемый, npm запускаем напрямую
    /// (`node npm-cli.js`), без cmd.exe и без надежды на PATH. Агент тоже стартует с полным
    /// путём к `node.exe`, чтобы после установки не зависеть от PATH.
    async fn install_npm(&self, app: &Arc<App>, id: &str, agent: &Value, npx: &Map<String, Value>, dir: &Path) -> Result<String, String> {
        crate::cloud::deny(crate::cloud::NO_INSTALL)?;
        let spec = npx.get("package").and_then(Value::as_str).ok_or("в реестре нет пакета")?;
        // Имя с `-` в начале npm прочитал бы как свой флаг (`--registry=…`): такое из реестра не ставим.
        if spec.starts_with('-') || spec.chars().any(|c| c.is_whitespace() || "\"&|<>^%!;`$".contains(c)) {
            return Err("странное имя пакета в реестре: не ставлю".into());
        }
        let featured = FEATURED.iter().find(|(f, _)| *f == id).map(|(_, n)| *n);
        let on_stage = |stage: crate::runtime_node::Stage| {
            let phase = match stage {
                crate::runtime_node::Stage::Downloading => "node",
                crate::runtime_node::Stage::Verifying => "node_verify",
                crate::runtime_node::Stage::Unpacking => "unpacking",
            };
            self.set_job(id, phase, None);
            self.emit_live(app, agent, featured);
        };
        let paths = crate::runtime_node::ensure_node_with_progress(app, &on_stage)
            .await
            .map_err(|e| format!("для этого агента нужен Node.js: скачать его не удалось — {e}"))?;
        if !paths.npm_cli.is_file() {
            return Err("у Node.js нет npm: не могу поставить агента".into());
        }
        self.set_job(id, "npm", None);
        self.emit(app, id).await;
        tokio::fs::create_dir_all(dir).await.map_err(|e| e.to_string())?;
        let cache = app.store.data_dir().join("runtimes").join("npm-cache");
        tokio::fs::create_dir_all(&cache).await.map_err(|e| format!("не смог создать папку npm (диск): {e}"))?;
        let mut cmd = tokio::process::Command::new(&paths.node);
        cmd.arg(&paths.npm_cli)
            .arg("install")
            .arg("--no-audit")
            .arg("--no-fund")
            .arg("--loglevel=error")
            .arg("--prefix")
            .arg(dir)
            .arg(spec)
            .env("PATH", crate::runtime_node::path_with_node(&paths.node))
            .env("npm_config_cache", &cache)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .kill_on_drop(true);
        #[cfg(windows)]
        cmd.creation_flags(0x0800_0000);
        let output = cmd.output().await.map_err(|e| format!("npm не запустился: {e}"))?;
        if !output.status.success() {
            let why = String::from_utf8_lossy(&output.stderr);
            let why: String = why.trim().chars().rev().take(600).collect::<String>().chars().rev().collect();
            return Err(format!("npm не поставил пакет: {why}"));
        }
        let name = package_name(spec);
        let package_dir = dir.join("node_modules").join(name.replace('/', "\\"));
        let manifest: Value = std::fs::read_to_string(package_dir.join("package.json"))
            .ok()
            .and_then(|t| serde_json::from_str(&t).ok())
            .ok_or("npm поставил пакет без package.json")?;
        let bin = match &manifest["bin"] {
            Value::String(path) => path.clone(),
            Value::Object(map) => {
                let short = name.rsplit('/').next().unwrap_or(name);
                map.get(short).or_else(|| map.values().next()).and_then(Value::as_str).ok_or("у пакета нет программы запуска")?.to_string()
            }
            _ => return Err("у пакета нет программы запуска".into()),
        };
        let script = package_dir.join(bin.trim_start_matches("./").replace('/', "\\"));
        let mut parts = vec![quote(&paths.node.display().to_string()), quote(&script.display().to_string())];
        for arg in npx.get("args").and_then(Value::as_array).into_iter().flatten().filter_map(Value::as_str) {
            parts.push(quote(arg));
        }
        Ok(parts.join(" "))
    }

    /// Событие о ходе установки без ожидания реестра: агент уже под рукой.
    fn emit_live(&self, app: &Arc<App>, agent: &Value, featured: Option<&str>) {
        let view = self.view(app, agent, featured);
        app.turns.emit(json!({ "type": "provider", "provider": view }));
    }
}

/// Фоновая проверка реестра ACP раз в час: обновляет кэшированные версии, не устанавливая агентов.
pub fn start(app: Arc<App>) {
    let first = provider_refresh_first();
    let period = provider_refresh_period();
    tokio::spawn(async move {
        tokio::time::sleep(first).await;
        loop {
            app.providers.refresh_registry(&app).await;
            tokio::time::sleep(period).await;
        }
    });
}

fn safe_id(id: &str) -> Result<&str, String> {
    if !id.is_empty() && id.len() <= 80 && id.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_' || b == b'.') && !id.starts_with('.') {
        Ok(id)
    } else {
        Err("плохой идентификатор агента".into())
    }
}

fn quote(part: &str) -> String {
    if part.is_empty() || part.chars().any(|c| c.is_whitespace() || "\"&|<>^()%!".contains(c)) {
        format!("\"{}\"", part.replace('"', "\\\""))
    } else {
        part.to_string()
    }
}

fn command_line(program: &str, args: Option<&Vec<Value>>) -> String {
    let mut parts = vec![quote(program)];
    for arg in args.into_iter().flatten().filter_map(Value::as_str) {
        parts.push(quote(arg));
    }
    parts.join(" ")
}

async fn extract(archive: &Path, dir: &Path) -> Result<(), String> {
    crate::cloud::deny(crate::cloud::NO_INSTALL)?;
    // В Windows 10+ есть bsdtar: он раскрывает и zip, и tar.gz.
    let mut cmd = tokio::process::Command::new("tar");
    cmd.arg("-xf").arg(archive).arg("-C").arg(dir).stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::piped());
    #[cfg(windows)]
    cmd.creation_flags(0x0800_0000);
    let output = cmd.output().await.map_err(|e| format!("не смог распаковать архив: {e}"))?;
    if !output.status.success() {
        let why = String::from_utf8_lossy(&output.stderr);
        return Err(format!("не смог распаковать архив: {}", why.trim()));
    }
    Ok(())
}

/// Пакет без версии: `@scope/name@1.2.3` → `@scope/name`.
fn package_name(spec: &str) -> &str {
    let start = usize::from(spec.starts_with('@'));
    match spec[start..].find('@') {
        Some(i) => &spec[..start + i],
        None => spec,
    }
}


#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn timeout_preserves_only_a_previously_connected_account_with_credentials() {
        let saved = json!({"connected": true, "models": [{"id": "working"}], "account": {"kind": "account"}});
        let mut entry = saved.as_object().unwrap().clone();
        assert!(preserve_probe_timeout(&mut entry, true, true));
        assert_eq!(entry["connected"], true);
        assert_eq!(entry["models"], saved["models"]);
        assert_eq!(entry["account"], saved["account"]);
        assert_eq!(entry["error"], PROBE_TIMEOUT_WARNING);
        assert!(!preserve_probe_timeout(&mut entry, false, true));
        assert!(!preserve_probe_timeout(&mut entry, true, false));
        entry.insert("connected".into(), json!(false));
        assert!(!preserve_probe_timeout(&mut entry, true, true));
    }

    #[test]
    fn package_names_lose_the_version() {
        assert_eq!(package_name("@agentclientprotocol/codex-acp@2.6.0"), "@agentclientprotocol/codex-acp");
        assert_eq!(package_name("cline@3.0.74"), "cline");
        assert_eq!(package_name("pi-acp"), "pi-acp");
    }

    #[test]
    fn ids_cannot_escape_the_agents_folder() {
        assert!(safe_id("codex-acp").is_ok());
        assert!(safe_id("../x").is_err());
        assert!(safe_id("a\\b").is_err());
        assert!(safe_id("").is_err());
    }

    #[test]
    fn command_lines_quote_paths_with_spaces() {
        let args = vec![json!("acp")];
        assert_eq!(command_line("C:\\Program Files\\a.exe", Some(&args)), "\"C:\\Program Files\\a.exe\" acp");
    }

    #[test]
    fn free_providers_have_https_base_and_key_url() {
        for b in BUILT_IN.iter().filter(|b| b.free) {
            assert!(b.base_url.starts_with("https://"), "base_url не https: {}", b.id);
            assert!(!b.key_url.is_empty(), "пустой key_url: {}", b.id);
        }
    }

    #[test]
    fn built_in_ids_are_unique() {
        let mut ids: Vec<&str> = BUILT_IN.iter().map(|b| b.id).collect();
        let total = ids.len();
        let unique: std::collections::BTreeSet<&str> = ids.drain(..).collect();
        assert_eq!(total, unique.len());
    }

    #[test]
    fn new_free_provider_ids_present() {
        for id in ["nvidia", "groq", "gemini", "openrouter", "zai"] {
            assert!(BUILT_IN.iter().any(|b| b.id == id), "нет провайдера {id}");
        }
    }

    #[test]
    fn every_builtin_provider_has_a_level_row() {
        for built in BUILT_IN {
            assert!(level_models(built.id).is_some(), "нет уровней для провайдера {}", built.id);
        }
        for driver in ["anthropic", "gemini", "api", "deloist"] {
            assert!(level_models(driver).is_some(), "нет уровней для драйвера {driver}");
        }
    }

    #[test]
    fn levels_pick_the_documented_models() {
        assert_eq!(level_model("anthropic", ModelLevel::Strong), Some("claude-opus-5-5"));
        assert_eq!(level_model("anthropic", ModelLevel::Standard), Some("claude-sonnet-5-5"));
        assert_eq!(level_model("anthropic", ModelLevel::Fast), Some("claude-haiku-4-5"));
        assert_eq!(level_model("deepseek", ModelLevel::Strong), Some("deepseek-v4-pro"));
        assert_eq!(level_model("deepseek", ModelLevel::Fast), Some("deepseek-v4-flash"));
        assert_eq!(level_model("gigachat", ModelLevel::Strong), Some("GigaChat-2-Max"));
        assert_eq!(level_model("alice", ModelLevel::Fast), Some("aliceai-llm-flash/latest"));
        assert_eq!(level_model("openrouter", ModelLevel::Standard), Some("openrouter/free"));
        assert_eq!(level_model("zai", ModelLevel::Fast), Some("glm-4.7-flash"));
        // Локальный сервер: все три уровня — выбранная модель, пустая строка значит «по умолчанию».
        assert_eq!(level_models("ollama"), Some(["", "", ""]));
        assert_eq!(level_models("lmstudio"), Some(["", "", ""]));
    }

    #[test]
    fn level_lookup_understands_a_provider_driver() {
        assert_eq!(level_model_for_driver("openai:deepseek", ModelLevel::Strong), Some("deepseek-v4-pro"));
        assert_eq!(level_model_for_driver("openai:gigachat", ModelLevel::Fast), Some("GigaChat-2"));
        assert_eq!(level_model_for_driver("anthropic", ModelLevel::Fast), Some("claude-haiku-4-5"));
        assert_eq!(level_model_for_driver("acp:codex-acp", ModelLevel::Fast), None);
    }

    #[test]
    fn a_level_moves_one_step_at_a_time() {
        assert_eq!(ModelLevel::Fast.up(), ModelLevel::Standard);
        assert_eq!(ModelLevel::Standard.up(), ModelLevel::Strong);
        assert_eq!(ModelLevel::Strong.up(), ModelLevel::Strong, "сильнее strong нет");
        assert_eq!(ModelLevel::Strong.down(), Some(ModelLevel::Standard));
        assert_eq!(ModelLevel::Standard.down(), Some(ModelLevel::Fast));
        assert_eq!(ModelLevel::Fast.down(), None, "ниже fast нет");
    }

    #[test]
    fn a_nearly_exhausted_limit_suggests_the_next_lower_level() {
        assert_eq!(level_hint(50, Some(ModelLevel::Strong)), None, "половина лимита — не повод");
        assert_eq!(level_hint(LIMIT_HINT_LEFT, Some(ModelLevel::Strong)), Some(ModelLevel::Standard));
        assert_eq!(level_hint(3, Some(ModelLevel::Standard)), Some(ModelLevel::Fast));
        assert_eq!(level_hint(3, Some(ModelLevel::Fast)), None);
        assert_eq!(level_hint(3, None), Some(ModelLevel::Fast), "без уровня предлагаем самый быстрый");
    }

    #[test]
    fn effort_goes_only_to_models_that_accept_it() {
        assert!(takes_effort("api", "gpt-6.1-terra"));
        assert!(takes_effort("deloist", "gpt-6.1-terra"));
        assert!(!takes_effort("openai:deepseek", "deepseek-v4-pro"), "DeepSeek V4 пока без allowlist");
        assert!(!takes_effort("api", "gpt-4o-mini"));
        assert!(!takes_effort("openai:gigachat", "GigaChat-2-Max"));
        assert!(!takes_effort("openai:ollama", "gpt-6.1-terra"));
        assert!(!takes_effort("anthropic", "claude-opus-5-5"));
        assert!(!takes_effort("gemini", "gemini-2.5-flash"));
    }

    #[test]
    fn the_ui_gets_levels_by_provider() {
        let levels = levels_json();
        assert_eq!(levels["anthropic"]["strong"], "claude-opus-5-5");
        assert_eq!(levels["ollama"]["fast"], "");
    }
}
