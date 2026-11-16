//! Картинки: инструмент `generate_image` и счётчик на день.
//!
//! Работает с любым сервисом, у которого есть OpenAI-совместимый `/images/generations`
//! (OpenAI, Grok, OpenRouter, Gemini через их OpenAI-совместимый адрес, свой шлюз).
//! Тело ошибки провайдера наружу не отдаём: в нём бывает ключ и кусок запроса.

use std::collections::HashMap;
use std::path::Path;
use std::sync::Mutex;
use std::time::Duration;

use serde_json::{json, Value};

use crate::store::{Bot, Settings};
use crate::App;

const TIMEOUT: Duration = Duration::from_secs(120);
/// Ответ провайдера целиком: картинка в base64 тяжелее готового файла примерно на треть.
const MAX_RESPONSE: usize = 20 * 1024 * 1024;
const MAX_IMAGE: usize = 10 * 1024 * 1024;

/// Сколько картинок нарисовано сегодня. Считаем по файлу, чтобы перезапуск не сбрасывал предел.
#[derive(Default)]
pub struct Images {
    state: Mutex<HashMap<String, i64>>,
}

impl Images {
    fn load(dir: &Path) -> HashMap<String, i64> {
        std::fs::read_to_string(dir.join("images.json"))
            .ok()
            .and_then(|text| serde_json::from_str(&text).ok())
            .unwrap_or_default()
    }

    fn today() -> String {
        chrono::Local::now().format("%Y-%m-%d").to_string()
    }

    fn save(dir: &Path, state: &HashMap<String, i64>) {
        let text = serde_json::to_string_pretty(state).unwrap_or_else(|_| "{}".into());
        let _ = std::fs::write(dir.join("images.json"), text);
    }

    /// Занять место до запроса: проверка и запись одним шагом, иначе параллельные боты
    /// вместе проскочат предел. `None` — на сегодня всё. `limit` 0 значит без предела.
    fn reserve(&self, dir: &Path, limit: i64) -> Option<i64> {
        let mut state = self.state.lock().unwrap();
        if state.is_empty() {
            *state = Self::load(dir);
        }
        let today = Self::today();
        let used = *state.get(&today).unwrap_or(&0);
        if limit > 0 && used >= limit {
            return None;
        }
        // Старые дни не нужны: файл не должен расти вечно.
        state.retain(|day, _| *day == today);
        state.insert(today, used + 1);
        Self::save(dir, &state);
        Some(used + 1)
    }

    /// Вернуть место: сервис отказал, а за отказ денег не берут.
    fn release(&self, dir: &Path) {
        let mut state = self.state.lock().unwrap();
        if let Some(used) = state.get_mut(&Self::today()) {
            *used = (*used - 1).max(0);
        }
        Self::save(dir, &state);
    }
}

/// Схема инструмента для модели.
pub fn tool_def() -> Value {
    json!({
        "type": "function",
        "function": {
            "name": "generate_image",
            "description": "Нарисовать картинку по описанию и сохранить её в воркспейс. Тратит деньги: одна картинка — один запрос к платному сервису.",
            "parameters": {
                "type": "object",
                "properties": {
                    "prompt": { "type": "string", "description": "Что нарисовать. Английский обычно даёт лучший результат." },
                    "size": { "type": "string", "description": "Размер, например 1024x1024. Пусто — как у сервиса по умолчанию." },
                    "file": { "type": "string", "description": "Имя файла, он ляжет в папку images. Пусто — придумаю сам." }
                },
                "required": ["prompt"]
            }
        }
    })
}

/// Какой ключ отдать сервису картинок. Основной — только тому же адресу, что у основной модели:
/// иначе он утечёт другому провайдеру (то же правило у запасной модели).
fn api_key(app: &App) -> String {
    let own = app.store.image_api_key();
    if !own.is_empty() {
        return own;
    }
    let settings = app.store.settings();
    if same_service(&settings.image_base_url, &settings.base_url) {
        app.store.api_key()
    } else {
        String::new()
    }
}

fn same_service(one: &str, other: &str) -> bool {
    match (reqwest::Url::parse(one.trim()), reqwest::Url::parse(other.trim())) {
        (Ok(one), Ok(other)) => one.origin() == other.origin(),
        _ => false,
    }
}

/// Настроен ли сервис картинок: без адреса, модели и ключа инструмент модели не показываем.
pub fn ready(app: &App) -> bool {
    let settings = app.store.settings();
    !settings.image_base_url.trim().is_empty() && !settings.image_model.trim().is_empty() && !api_key(app).is_empty()
}

/// Сбой запроса. За ответ 2xx сервис деньги уже взял, даже если картинка оказалась битой.
struct Failure {
    charged: bool,
    message: String,
}

impl Failure {
    fn free(message: impl Into<String>) -> Self {
        Self { charged: false, message: message.into() }
    }

    fn charged(message: impl Into<String>) -> Self {
        Self { charged: true, message: message.into() }
    }
}

pub async fn generate(app: &App, bot: &Bot, args: &Value) -> Result<String, String> {
    let settings = app.store.settings();
    let prompt = args["prompt"].as_str().unwrap_or_default().trim();
    if prompt.is_empty() {
        return Err("пустое описание".into());
    }
    if !ready(app) {
        return Err("сервис картинок не настроен: Настройки → Картинки".into());
    }
    let name = target_name(args["file"].as_str().unwrap_or_default(), prompt);
    let path = app.store.safe_path(&bot.id, &name)?;

    let data_dir = app.store.data_dir();
    let daily = settings.image_daily.max(0);
    let Some(count) = app.images.reserve(data_dir, daily) else {
        return Err(format!("на сегодня предел картинок исчерпан ({daily})"));
    };

    let bytes = match request_image(&settings, &api_key(app), prompt, args["size"].as_str()).await {
        Ok(bytes) => bytes,
        Err(failure) => {
            if !failure.charged {
                app.images.release(data_dir);
            }
            return Err(failure.message);
        }
    };

    if let Some(parent) = path.parent() {
        tokio::fs::create_dir_all(parent).await.map_err(|e| e.to_string())?;
    }
    tokio::fs::write(&path, &bytes).await.map_err(|e| e.to_string())?;

    Ok(format!(
        "Картинка сохранена: {name} ({} КБ). Сегодня нарисовано {count}{}.",
        bytes.len() / 1024,
        if daily > 0 { format!(" из {daily}") } else { String::new() }
    ))
}

async fn request_image(settings: &Settings, key: &str, prompt: &str, size: Option<&str>) -> Result<Vec<u8>, Failure> {
    let base = settings.image_base_url.trim().trim_end_matches('/');
    let url = if base.ends_with("/images/generations") { base.to_string() } else { format!("{base}/images/generations") };
    let model = settings.image_model.trim();
    let mut body = json!({ "model": model, "prompt": prompt, "n": 1 });
    // Модели gpt-image всегда отдают base64 и отвечают 400 на этот параметр.
    if !model.to_lowercase().contains("gpt-image") {
        body["response_format"] = json!("b64_json");
    }
    if let Some(size) = size.map(str::trim).filter(|s| !s.is_empty()) {
        body["size"] = json!(size);
    }

    let client = reqwest::Client::builder().timeout(TIMEOUT).build().map_err(|e| Failure::free(e.to_string()))?;
    let response = client
        .post(&url)
        .bearer_auth(key)
        .json(&body)
        .send()
        .await
        .map_err(|e| Failure::free(format!("сервис картинок не ответил: {e}")))?;

    let status = response.status();
    if !status.is_success() {
        // Тело не читаем и не показываем: в нём бывает ключ и кусок запроса.
        return Err(Failure::free(format!("сервис картинок ответил HTTP {} (модель {model})", status.as_u16())));
    }
    let raw = read_limited(response, MAX_RESPONSE).await.map_err(Failure::charged)?;
    let value: Value = serde_json::from_slice(&raw).map_err(|_| Failure::charged("сервис ответил не JSON"))?;

    let first = &value["data"][0];
    let bytes = if let Some(encoded) = first["b64_json"].as_str().filter(|s| !s.is_empty()) {
        decode_base64(encoded).ok_or_else(|| Failure::charged("картинка пришла битой"))?
    } else if let Some(link) = first["url"].as_str().filter(|s| !s.is_empty()) {
        download(&client, link).await.map_err(Failure::charged)?
    } else {
        return Err(Failure::charged("в ответе нет картинки"));
    };
    if bytes.len() > MAX_IMAGE {
        return Err(Failure::charged("картинка больше 10 МБ"));
    }
    Ok(bytes)
}

/// Некоторые сервисы отдают ссылку. Ключ по ней не передаём: адрес выбран не нами.
async fn download(client: &reqwest::Client, link: &str) -> Result<Vec<u8>, String> {
    let url = reqwest::Url::parse(link).map_err(|_| "сервис вернул плохую ссылку".to_string())?;
    if !matches!(url.scheme(), "http" | "https") {
        return Err("сервис вернул плохую ссылку".into());
    }
    let response = client.get(url).send().await.map_err(|e| format!("картинка не скачалась: {}", e.without_url()))?;
    if !response.status().is_success() {
        return Err(format!("картинка не скачалась: HTTP {}", response.status().as_u16()));
    }
    read_limited(response, MAX_IMAGE).await
}

/// Прочитать ответ, не веря его размеру на слово: лишнее не должно попасть в память.
async fn read_limited(mut response: reqwest::Response, limit: usize) -> Result<Vec<u8>, String> {
    let too_big = || "ответ сервиса слишком большой".to_string();
    if response.content_length().is_some_and(|length| length > limit as u64) {
        return Err(too_big());
    }
    let mut out = Vec::new();
    while let Some(chunk) = response.chunk().await.map_err(|e| e.without_url().to_string())? {
        out.extend_from_slice(&chunk);
        if out.len() > limit {
            return Err(too_big());
        }
    }
    Ok(out)
}

/// Куда сохранить. Всегда папка images: имя придумывает модель, и картинка не должна
/// затереть рабочий файл вроде `notes.md`.
fn target_name(asked: &str, prompt: &str) -> String {
    let base = asked.trim().rsplit(['/', '\\']).next().unwrap_or_default().trim();
    if base.is_empty() || base.chars().all(|c| c == '.') {
        return format!("images/{}-{}.png", chrono::Local::now().format("%Y%m%d-%H%M%S"), slug(prompt));
    }
    let lower = base.to_lowercase();
    let has_picture_extension = [".png", ".jpg", ".jpeg", ".webp"].iter().any(|ext| lower.ends_with(ext));
    if has_picture_extension {
        format!("images/{base}")
    } else {
        format!("images/{base}.png")
    }
}

fn slug(prompt: &str) -> String {
    let mut out = String::new();
    for ch in prompt.chars().take(40) {
        if ch.is_ascii_alphanumeric() {
            out.push(ch.to_ascii_lowercase());
        } else if ch.is_whitespace() && !out.ends_with('-') {
            out.push('-');
        }
    }
    let trimmed = out.trim_matches('-').to_string();
    if trimmed.is_empty() { "picture".into() } else { trimmed }
}

fn decode_base64(text: &str) -> Option<Vec<u8>> {
    const TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut lookup = [255u8; 256];
    for (index, byte) in TABLE.iter().enumerate() {
        lookup[*byte as usize] = index as u8;
    }
    let mut out = Vec::with_capacity(text.len() * 3 / 4);
    let mut buffer = 0u32;
    let mut bits = 0u32;
    for byte in text.bytes() {
        if byte == b'=' || byte == b'\n' || byte == b'\r' {
            continue;
        }
        let value = lookup[byte as usize];
        if value == 255 {
            return None;
        }
        buffer = (buffer << 6) | value as u32;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((buffer >> bits) as u8);
        }
    }
    Some(out)
}


#[cfg(test)]
mod tests {
    use super::{decode_base64, same_service, slug, target_name};

    #[test]
    fn base64_round_trips_known_text() {
        // "Delooist" в base64 без выравнивания по строкам.
        let text = b"Delooist!";
        let encoded = crate::browser_tools::base64_encode(text);
        assert_eq!(decode_base64(&encoded).unwrap(), text);
    }

    #[test]
    fn a_slug_is_short_and_safe() {
        let name = slug("Робот на закате, 4K!");
        assert!(name.len() <= 40);
        assert!(name.chars().all(|c| c.is_ascii_alphanumeric() || c == '-'));
        assert_eq!(slug("!!!"), "picture");
    }

    #[test]
    fn a_picture_stays_in_its_folder() {
        assert_eq!(target_name("cat.png", "кот"), "images/cat.png");
        assert_eq!(target_name("notes.md", "кот"), "images/notes.md.png");
        assert_eq!(target_name("../../secrets.json", "кот"), "images/secrets.json.png");
        assert_eq!(target_name("a\\b\\cat.WEBP", "кот"), "images/cat.WEBP");
        assert!(target_name("", "cat").starts_with("images/"));
        assert!(target_name("..", "cat").ends_with("-cat.png"));
    }

    #[test]
    fn a_service_is_its_scheme_host_and_port() {
        assert!(same_service("https://api.openai.com/v1", "https://API.openai.com/v2/"));
        assert!(!same_service("https://api.openai.com/v1", "https://evil.example/v1"));
        assert!(!same_service("http://api.openai.com/v1", "https://api.openai.com/v1"));
        assert!(!same_service("", "https://api.openai.com/v1"));
    }
}

