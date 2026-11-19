//! «Gemini» одной кнопкой: Antigravity CLI от Google (нативная программа, говорит по ACP).
//!
//! Не из реестра ACP, а встроенный агент `antigravity`. Версия и ссылка на архив — из релизов GitHub
//! `google-antigravity/antigravity-cli`; контрольная сумма берётся из ответа GitHub (`digest`), без неё
//! не ставим. Лицензия Google не разрешает раздавать программу, поэтому она скачивается на компьютер человека.
//!
//! Аккаунтов Google до трёх. У каждого своя папка `accounts/<id>/home` (`--gemini-dir`) и свой процесс.
//! Бот привязан к аккаунту драйвером `acp:antigravity@<id>`; без `@` — первый аккаунт.
//! Ключ API (aistudio.google.com/apikey) лежит только в зашифрованном хранилище секретов и уходит
//! программе в окружении `GEMINI_API_KEY`; такой вход — отдельная запись `key`.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use serde_json::{json, Map, Value};
use tokio_util::sync::CancellationToken;

use super::{agents_dir, http, platform, quote, Providers, REGISTRY_TTL};
use crate::drivers::acp;
use crate::App;

pub const ID: &str = "antigravity";
pub const MAX_ACCOUNTS: usize = 3;
/// Архив больше этого не качаем: настоящий весит около 85 МБ.
pub const SIZE_CAP: u64 = 300 * 1024 * 1024;
const RELEASES_PATH: &str = "/repos/google-antigravity/antigravity-cli/releases/latest";
const RELEASES_BASE: &str = "https://api.github.com";
/// Архивы релиза лежат только здесь.
pub const DOWNLOAD_PREFIX: &str = "https://github.com/google-antigravity/antigravity-cli/releases/download/";
const SECRET_SCOPE: &str = "_providers";
const SECRET_NAME: &str = "antigravity-gemini-api-key";
const KEY_ACCOUNT: &str = "key";
const ABOUT: &str = "Google: Gemini, Claude и GPT по подписке Google";

/// Подмена адреса релизов для тестов работает только в отладочной сборке.
fn releases_override() -> Option<String> {
    if cfg!(debug_assertions) {
        let url = std::env::var("CREW_TEST_ANTIGRAVITY_RELEASES").ok()?;
        let url = url.trim().trim_end_matches('/').to_string();
        return (!url.is_empty()).then_some(url);
    }
    None
}

pub fn has_release_override() -> bool {
    releases_override().is_some()
}

/// `antigravity@a2` → `Some("a2")`; `antigravity` → `None`; чужой идентификатор → `Err`.
pub fn split(id: &str) -> Option<Option<&str>> {
    if id == ID {
        return Some(None);
    }
    id.strip_prefix("antigravity@").map(Some)
}

fn asset_name() -> Option<&'static str> {
    match platform() {
        "windows-x86_64" => Some("agy_cli_windows_x64.zip"),
        "windows-aarch64" => Some("agy_cli_windows_arm64.zip"),
        _ => None,
    }
}

fn safe_version(version: &str) -> bool {
    !version.is_empty() && version.len() <= 40 && version.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'.' || b == b'-' || b == b'_') && !version.starts_with('.')
}

/// `sha256:<hex>` → `<hex>`; всё остальное — пусто (и тогда ставить нельзя).
fn digest_hex(digest: &str) -> String {
    match digest.strip_prefix("sha256:") {
        Some(hex) if hex.len() == 64 && hex.bytes().all(|b| b.is_ascii_hexdigit()) => hex.to_ascii_lowercase(),
        _ => String::new(),
    }
}

/// Ответ GitHub в короткий вид: версия и ресурсы.
fn normalize(raw: &Value) -> Result<Value, String> {
    let tag = raw["tag_name"].as_str().unwrap_or_default().trim_start_matches('v');
    if !safe_version(tag) {
        return Err("в ответе GitHub нет версии".into());
    }
    let assets: Vec<Value> = raw["assets"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|a| {
            json!({
                "name": a["name"], "url": a["browser_download_url"], "size": a["size"],
                "sha256": digest_hex(a["digest"].as_str().unwrap_or_default()),
            })
        })
        .collect();
    Ok(json!({ "version": tag, "assets": assets }))
}

pub fn home_of(app: &App, aid: &str) -> PathBuf {
    agents_dir(app).join(ID).join("accounts").join(aid).join("home")
}

fn has_token(home: &Path) -> bool {
    let Ok(text) = std::fs::read_to_string(home.join("antigravity-acp").join("antigravity-oauth-token")) else { return false };
    serde_json::from_str::<Value>(&text).is_ok_and(|v| v["token"].as_object().is_some_and(|t| !t.is_empty()))
}

fn temp_root(app: &App) -> PathBuf {
    agents_dir(app).join(ID).join("tmp")
}

/// Файл входа, который пишет сама программа. Читаем только почту из него, наружу и в журнал он не идёт.
fn read_email(home: &Path) -> Option<String> {
    let text = std::fs::read_to_string(home.join("antigravity-acp").join("antigravity-oauth-token")).ok()?;
    let value: Value = serde_json::from_str(&text).ok()?;
    value["id_token_email"].as_str().filter(|e| e.contains('@') && e.len() < 200).map(str::to_string)
}

/// Что добавить к запуску агента: окружение, свои временные папки, способ входа.
#[derive(Default)]
pub struct Extras {
    pub env: Vec<(String, String)>,
    pub hidden: Vec<String>,
    pub temp_root: Option<PathBuf>,
    pub auth: String,
}

impl Providers {
    // -----------------------------------------------------------------------
    // Релиз
    // -----------------------------------------------------------------------

    async fn release(&self, app: &App, force: bool) -> Result<Value, String> {
        if !force {
            if let Some((at, data)) = self.release.lock().unwrap().clone() {
                if at.elapsed() < REGISTRY_TTL {
                    return Ok(data);
                }
            }
        }
        // Тесты со своим реестром без подмены релизов в сеть не ходят.
        if cfg!(debug_assertions) && std::env::var_os("CREW_TEST_ACP_REGISTRY").is_some() && releases_override().is_none() {
            return Err("нет связи с GitHub".into());
        }
        let cache = app.store.data_dir().join("antigravity-release.json");
        let recently_failed = !force && self.release_failed.lock().unwrap().is_some_and(|at| at.elapsed() < Duration::from_secs(60));
        let fetched = if recently_failed {
            Err("нет связи с GitHub".to_string())
        } else {
            async {
                let url = format!("{}{RELEASES_PATH}", releases_override().unwrap_or_else(|| RELEASES_BASE.to_string()));
                let response = http()?
                    .get(url)
                    .header("Accept", "application/vnd.github+json")
                    .timeout(Duration::from_secs(10))
                    .send()
                    .await
                    .map_err(|e| format!("нет связи с GitHub: {e}"))?;
                if !response.status().is_success() {
                    return Err(format!("GitHub ответил HTTP {}", response.status().as_u16()));
                }
                let raw = response.json::<Value>().await.map_err(|e| e.to_string())?;
                normalize(&raw)
            }
            .await
        };
        match fetched {
            Ok(data) => {
                let _ = std::fs::write(&cache, data.to_string());
                *self.release.lock().unwrap() = Some((Instant::now(), data.clone()));
                *self.release_failed.lock().unwrap() = None;
                Ok(data)
            }
            Err(why) => {
                *self.release_failed.lock().unwrap() = Some(Instant::now());
                let text = std::fs::read_to_string(&cache).map_err(|_| why.clone())?;
                serde_json::from_str(&text).map_err(|_| why)
            }
        }
    }

    /// Описание агента в виде записи реестра: дальше работают общие `view`, `install` и `download`.
    pub(super) async fn antigravity_agent(&self, app: &App, force: bool) -> Value {
        let mut agent = json!({ "id": ID, "name": "Gemini", "description": ABOUT, "version": "", "distribution": {},
            "website": "https://github.com/google-antigravity/antigravity-cli" });
        match self.release(app, force).await {
            Ok(release) => {
                agent["version"] = release["version"].clone();
                let asset = release["assets"].as_array().and_then(|list| list.iter().find(|a| Some(a["name"].as_str().unwrap_or_default()) == asset_name()));
                if let (Some(asset), Some(platform)) = (asset, Some(platform())) {
                    agent["distribution"] = json!({ "binary": { platform: {
                        "archive": asset["url"], "cmd": "antigravity.exe", "sha256": asset["sha256"],
                        "size": asset["size"], "strict": true,
                    } } });
                }
            }
            Err(why) => agent["releaseError"] = json!(why),
        }
        agent
    }

    /// Ссылка входа Google, пока идёт вход: если браузер сам не открылся. Только для окна, в списке её нет.
    pub fn signin_link(&self) -> Result<Value, String> {
        if !self.jobs.lock().unwrap().contains_key(ID) {
            return Err("сейчас вход не идёт".into());
        }
        acp::signin_link().map(|link| json!({ "link": link })).ok_or_else(|| "ссылка пока не появилась".into())
    }

    /// «Проверить обновления»: свежий ответ GitHub в обход кэша.
    pub async fn refresh_release(&self, app: &App) -> Result<Value, String> {
        self.release(app, true).await.map_err(|e| format!("не смог проверить обновления: {e}"))?;
        self.get(app, ID).await
    }

    // -----------------------------------------------------------------------
    // Аккаунты
    // -----------------------------------------------------------------------

    /// Аккаунты по порядку добавления.
    pub(super) fn ag_accounts(&self, app: &App) -> Vec<(String, Value)> {
        let entry = self.entry(app, ID);
        let accounts = entry["accounts"].as_object().cloned().unwrap_or_default();
        entry["accountOrder"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
            .filter_map(|aid| accounts.get(aid).map(|a| (aid.to_string(), a.clone())))
            .collect()
    }

    /// Аккаунт, который имеет в виду драйвер: названный или первый.
    fn ag_resolve(&self, app: &App, wanted: Option<&str>) -> Option<(String, Value)> {
        let all = self.ag_accounts(app);
        match wanted {
            Some(aid) => all.into_iter().find(|(id, _)| id == aid),
            None => all.into_iter().next(),
        }
    }

    fn ag_set_account(&self, app: &App, aid: &str, change: impl FnOnce(&mut Map<String, Value>)) {
        self.update_entry(app, ID, |e| {
            let accounts = e.entry("accounts").or_insert_with(|| json!({}));
            if let Some(account) = accounts.as_object_mut().and_then(|m| m.get_mut(aid)).and_then(Value::as_object_mut) {
                change(account);
            }
        });
    }

    /// Итог по всем аккаунтам для общей карточки; выбор модели — первого аккаунта.
    fn ag_aggregate(&self, app: &App) {
        let accounts = self.ag_accounts(app);
        self.update_entry(app, ID, |e| {
            let connected = accounts.iter().any(|(_, a)| a["connected"] == true);
            e.insert("connected".into(), json!(connected));
            // Драйвер `acp:antigravity` идёт первым аккаунтом (`ag_resolve`): и выбор модели у него — только его модели.
            // Прежде здесь было объединение всех аккаунтов, и бот получал модель второго аккаунта, которой первый не знает:
            // «Unknown model ID: gemini-4-neon» (Маркетолог X, 19.11.2026). Модели других аккаунтов — в их пунктах «Gemini · …».
            let models: Vec<Value> = accounts.first().and_then(|(_, a)| a["models"].as_array().cloned()).unwrap_or_default();
            if models.is_empty() {
                e.remove("models");
            } else {
                e.insert("models".into(), json!(models));
            }
            match accounts.iter().find(|(_, a)| a["connected"] == true) {
                Some((_, a)) => {
                    e.insert("account".into(), a["account"].clone());
                    if let Some(error) = accounts.iter().filter(|(_, a)| a["connected"] == true).find_map(|(_, a)| a["error"].as_str()) {
                        e.insert("error".into(), json!(error));
                    } else {
                        e.remove("error");
                    }
                }
                None => {
                    e.remove("account");
                    if let Some(error) = accounts.iter().find_map(|(_, a)| a["error"].as_str().map(str::to_string)) {
                        e.insert("error".into(), json!(error));
                    }
                }
            }
        });
    }

    /// Команда запуска программы для аккаунта; `None` — не скачана или такого аккаунта нет.
    pub(super) fn ag_command(&self, app: &App, wanted: Option<&str>) -> Option<String> {
        let base = self.entry(app, ID)["command"].as_str().filter(|c| !c.is_empty())?.to_string();
        let (aid, _) = self.ag_resolve(app, wanted)?;
        Some(format!(
            "{base} acp {} {}",
            quote(&format!("--gemini-dir={}", plain(&home_of(app, &aid)))),
            "--app-data-dir=antigravity-acp"
        ))
    }

    fn ag_key(&self, app: &App) -> Option<String> {
        app.connectors.secrets.get(SECRET_SCOPE, SECRET_NAME).filter(|k| !k.is_empty())
    }

    /// Окружение, временные папки и способ входа для драйвера `acp:<id>`.
    pub fn launch_extras(&self, app: &App, id: &str) -> Extras {
        let Some(wanted) = split(id) else { return Extras::default() };
        let mut extras = Extras {
            hidden: vec!["GEMINI_API_KEY".into(), "GOOGLE_API_KEY".into()],
            temp_root: Some(temp_root(app)),
            // Без этой переменной программа прячет вход Google в общее хранилище Windows (его видит и
            // обычный Antigravity человека, и все аккаунты сразу). С ней — файл токена в папке аккаунта.
            env: vec![("WSL_DISTRO_NAME".into(), "Deloist".into())],
            ..Extras::default()
        };
        if let Some((_, account)) = self.ag_resolve(app, wanted) {
            if account["kind"] == "key" {
                if let Some(key) = self.ag_key(app) {
                    extras.env.push(("GEMINI_API_KEY".into(), key));
                    extras.auth = "api-key".into();
                }
            } else {
                extras.auth = "oauth".into();
            }
        }
        extras
    }

    /// Запуск программы для проверки или выхода: возвращает ответ `acp::probe`.
    async fn ag_probe(&self, app: &Arc<App>, aid: &str, login: bool, cancel: &CancellationToken) -> bool {
        let Some(command) = self.ag_command(app, Some(aid)) else { return false };
        let extras = self.launch_extras(app, &format!("{ID}@{aid}"));
        let key = if extras.auth == "api-key" { self.ag_key(app) } else { None };
        let mut hidden = vec!["OPENAI_API_KEY".to_string(), app.store.settings().api_key_env.clone()];
        hidden.extend(extras.hidden.iter().cloned());
        let temp = extras.temp_root.clone();
        let spec = acp::Spec {
            command: &command, env: &extras.env, hidden_env: &hidden, mode: "", model: "", effort: "", options: &[],
            temp_root: temp.as_deref(), auth: &extras.auth,
        };
        let dir = agents_dir(app).join("_probe").join(aid);
        let _ = std::fs::create_dir_all(&dir);
        // Ссылку входа берём только из того, что пишет сама программа (acp::set_signin_link). Чужие процессы не
        // просматриваем: в их командных строках бывают ссылки входа других программ.
        let result = acp::probe(&spec, &dir, login, cancel).await;
        let scrub = |text: String| match &key {
            Some(key) => text.replace(key.as_str(), "***"),
            None => text,
        };
        let email = read_email(&home_of(app, aid));
        // Вход Google — это файл с токеном: список моделей программа отдаёт и без входа.
        let signed_in = key.is_some() || has_token(&home_of(app, aid));
        let mut started = false;
        self.ag_set_account(app, aid, |a| {
            a.insert("checkedAt".into(), json!(crate::store::now()));
            match result {
                Ok(probe) if super::preserve_probe_timeout(a, probe.timed_out, signed_in && probe.account["kind"] != "none") => {}
                Ok(probe) => {
                    started = probe.error.is_none();
                    let connected = probe.connected && signed_in;
                    a.insert("connected".into(), json!(connected));
                    if connected {
                        a.insert("account".into(), if probe.account.is_null() { json!({}) } else { probe.account });
                        if let Some(email) = email {
                            a.insert("email".into(), json!(email));
                        }
                    } else {
                        a.remove("account");
                    }
                    if connected && !probe.models.is_empty() {
                        a.insert("models".into(), json!(probe.models));
                    }
                    match probe.error {
                        Some(error) if error != "остановлено" => a.insert("error".into(), json!(scrub(error))),
                        _ => a.remove("error"),
                    };
                }
                Err(error) => {
                    a.insert("connected".into(), json!(false));
                    a.insert("error".into(), json!(scrub(error)));
                }
            }
        });
        // Тот же аккаунт Google, добавленный дважды, не нужен.
        let me = self.ag_resolve(app, Some(aid));
        if let Some((_, account)) = me {
            let email = account["email"].as_str().unwrap_or_default();
            let twin = !email.is_empty() && self.ag_accounts(app).iter().any(|(other, a)| other != aid && a["email"] == account["email"]);
            if twin && account["connected"] == true {
                self.ag_forget_account(app, aid).await;
                self.update_entry(app, ID, |e| {
                    e.insert("error".into(), json!("Этот аккаунт Google уже добавлен"));
                });
                return started;
            }
        }
        self.ag_aggregate(app);
        started
    }

    /// Проверка всех аккаунтов (без входа).
    pub(super) async fn ag_probe_all(&self, app: &Arc<App>, cancel: &CancellationToken) -> bool {
        let mut started = false;
        for (aid, _) in self.ag_accounts(app) {
            started |= self.ag_probe(app, &aid, false, cancel).await;
        }
        started
    }

    fn ag_next_id(&self, app: &App) -> Option<String> {
        let used: Vec<String> = self.ag_accounts(app).into_iter().map(|(id, _)| id).collect();
        (1..=MAX_ACCOUNTS).map(|n| format!("a{n}")).find(|id| !used.contains(id))
    }

    fn ag_create_account(&self, app: &App, aid: &str, kind: &str) {
        self.update_entry(app, ID, |e| {
            let accounts = e.entry("accounts").or_insert_with(|| json!({}));
            if let Some(map) = accounts.as_object_mut() {
                map.insert(aid.to_string(), json!({ "kind": kind, "connected": false, "addedAt": crate::store::now() }));
            }
            let order = e.entry("accountOrder").or_insert_with(|| json!([]));
            if let Some(list) = order.as_array_mut() {
                if !list.iter().any(|v| v == aid) {
                    list.push(json!(aid));
                }
            }
        });
    }

    /// Убрать запись аккаунта и его папку; боты на нём возвращаются к первому аккаунту.
    async fn ag_forget_account(&self, app: &Arc<App>, aid: &str) {
        let home = home_of(app, aid);
        if let Some(root) = home.parent() {
            let _ = tokio::fs::remove_dir_all(root).await;
        }
        self.update_entry(app, ID, |e| {
            if let Some(map) = e.get_mut("accounts").and_then(Value::as_object_mut) {
                map.remove(aid);
            }
            if let Some(list) = e.get_mut("accountOrder").and_then(Value::as_array_mut) {
                list.retain(|v| v != aid);
            }
        });
        if aid == KEY_ACCOUNT {
            let _ = app.connectors.secrets.delete(SECRET_SCOPE, SECRET_NAME);
        }
        let driver = format!("acp:{ID}@{aid}");
        for bot in app.store.bots().iter().filter(|b| b.driver == driver) {
            let _ = app.store.update_bot(&bot.id, &json!({ "driver": format!("acp:{ID}") })).await;
        }
        if app.store.settings().driver == driver {
            let _ = app.store.update_settings(&json!({ "driver": format!("acp:{ID}"), "model": "" })).await;
        }
        self.ag_aggregate(app);
    }

    /// Выйти из аккаунта у самой программы (ACP `logout`), потом забыть его. Папка удаляется в любом случае.
    pub async fn remove_account(&self, app: &Arc<App>, aid: &str) -> Result<Value, String> {
        if self.ag_resolve(app, Some(aid)).is_none() {
            return Err("такого аккаунта нет".into());
        }
        if self.jobs.lock().unwrap().contains_key(ID) {
            return Err("Сейчас идёт вход или загрузка: дождись конца или нажми «Отмена»".into());
        }
        if let Some(command) = self.ag_command(app, Some(aid)) {
            let extras = self.launch_extras(app, &format!("{ID}@{aid}"));
            let mut hidden = vec!["OPENAI_API_KEY".to_string(), app.store.settings().api_key_env.clone()];
            hidden.extend(extras.hidden.iter().cloned());
            let temp = extras.temp_root.clone();
            let spec = acp::Spec {
                command: &command, env: &extras.env, hidden_env: &hidden, mode: "", model: "", effort: "", options: &[],
                temp_root: temp.as_deref(), auth: &extras.auth,
            };
            let dir = agents_dir(app).join("_probe").join(aid);
            let _ = std::fs::create_dir_all(&dir);
            let _ = acp::logout(&spec, &dir).await;
        }
        self.ag_forget_account(app, aid).await;
        self.update_entry(app, ID, |e| {
            e.remove("error");
        });
        let view = self.get(app, ID).await?;
        app.turns.emit(json!({ "type": "provider", "provider": view }));
        Ok(view)
    }

    /// «Отключить»: выйти из всех аккаунтов и забыть ключ. Программа остаётся на диске.
    pub async fn disconnect_all(&self, app: &Arc<App>) -> Result<Value, String> {
        for (aid, _) in self.ag_accounts(app) {
            self.remove_account(app, &aid).await?;
        }
        let _ = app.connectors.secrets.delete(SECRET_SCOPE, SECRET_NAME);
        self.update_entry(app, ID, |e| {
            for key in ["connected", "account", "models", "error", "accounts", "accountOrder"] {
                e.remove(key);
            }
        });
        self.get(app, ID).await
    }

    /// Новый вход: свободная папка, программа сама открывает страницу Google.
    pub async fn add_account(self: &Arc<Self>, app: &Arc<App>) -> Result<Value, String> {
        if self.entry(app, ID)["command"].as_str().is_none_or(str::is_empty) {
            return Err("сначала скачай Gemini".into());
        }
        let Some(aid) = self.ag_next_id(app) else {
            return Err(format!("Можно подключить не больше {MAX_ACCOUNTS} аккаунтов Gemini. Отключи ненужный и повтори"));
        };
        let Some(cancel) = self.begin_job(ID, "connecting", None) else {
            return Err("уже идёт".into());
        };
        self.ag_create_account(app, &aid, "google");
        let _ = std::fs::create_dir_all(home_of(app, &aid));
        self.update_entry(app, ID, |e| {
            e.remove("error");
        });
        let (me, app2) = (self.clone(), app.clone());
        tokio::spawn(async move {
            me.ag_login(&app2, &aid, &cancel).await;
            me.jobs.lock().unwrap().remove(ID);
            me.emit(&app2, ID).await;
        });
        self.get(app, ID).await
    }

    /// Вход в аккаунт. Не вышло (отмена, время вышло) — запись не оставляем, причину показываем на карточке.
    pub(super) async fn ag_login(&self, app: &Arc<App>, aid: &str, cancel: &CancellationToken) -> bool {
        self.ag_probe(app, aid, true, cancel).await;
        let Some((_, account)) = self.ag_resolve(app, Some(aid)) else { return false };
        if account["connected"] == true {
            return true;
        }
        let why = account["error"].as_str().map(str::to_string);
        self.ag_forget_account(app, aid).await;
        if cancel.is_cancelled() {
            return false;
        }
        self.update_entry(app, ID, |e| {
            e.insert("error".into(), json!(why.unwrap_or_else(|| "вход не завершён".into())));
        });
        false
    }

    /// «Подключить» на уже скачанной программе: войти заново в аккаунт без входа или добавить первый.
    pub async fn connect(self: &Arc<Self>, app: &Arc<App>) -> Result<Value, String> {
        match self.ag_accounts(app).iter().find(|(_, a)| a["connected"] != true && a["kind"] == "google") {
            Some((aid, _)) => {
                let Some(cancel) = self.begin_job(ID, "connecting", None) else {
                    return Err("уже идёт".into());
                };
                let (me, app2, aid) = (self.clone(), app.clone(), aid.clone());
                tokio::spawn(async move {
                    let _ = me.ag_probe(&app2, &aid, true, &cancel).await;
                    me.jobs.lock().unwrap().remove(ID);
                    me.emit(&app2, ID).await;
                });
                self.get(app, ID).await
            }
            None => self.add_account(app).await,
        }
    }

    // -----------------------------------------------------------------------
    // Ключ API
    // -----------------------------------------------------------------------

    /// Ключ — только в зашифрованное хранилище. Своя запись `key` входит по нему.
    pub async fn set_key(self: &Arc<Self>, app: &Arc<App>, key: &str) -> Result<Value, String> {
        let key = key.trim();
        if key.len() < 8 || key.len() > 200 || key.bytes().any(|b| b.is_ascii_whitespace() || b.is_ascii_control()) {
            return Err("ключ API не похож на ключ: вставь его целиком, без пробелов".into());
        }
        if self.entry(app, ID)["command"].as_str().is_none_or(str::is_empty) {
            return Err("сначала скачай Gemini".into());
        }
        let exists = self.ag_resolve(app, Some(KEY_ACCOUNT)).is_some();
        if !exists && self.ag_accounts(app).len() >= MAX_ACCOUNTS {
            return Err(format!("Можно подключить не больше {MAX_ACCOUNTS} аккаунтов Gemini. Отключи ненужный и повтори"));
        }
        let Some(cancel) = self.begin_job(ID, "checking", None) else {
            return Err("уже идёт".into());
        };
        if let Err(e) = app.connectors.secrets.set(SECRET_SCOPE, SECRET_NAME, key) {
            self.jobs.lock().unwrap().remove(ID);
            return Err(e);
        }
        if !exists {
            self.ag_create_account(app, KEY_ACCOUNT, "key");
        }
        let _ = std::fs::create_dir_all(home_of(app, KEY_ACCOUNT));
        let (me, app2) = (self.clone(), app.clone());
        tokio::spawn(async move {
            // Вход по ключу: окно Google не нужно, ключ идёт в окружении, а `authenticate` берёт способ «по ключу».
            me.ag_probe(&app2, KEY_ACCOUNT, true, &cancel).await;
            me.jobs.lock().unwrap().remove(ID);
            me.emit(&app2, ID).await;
        });
        self.get(app, ID).await
    }

    /// Ключ убран — запись по ключу отключается.
    pub async fn remove_key(&self, app: &Arc<App>) -> Result<Value, String> {
        let _ = app.connectors.secrets.delete(SECRET_SCOPE, SECRET_NAME);
        if self.ag_resolve(app, Some(KEY_ACCOUNT)).is_some() {
            self.remove_account(app, KEY_ACCOUNT).await?;
        }
        self.get(app, ID).await
    }

    // -----------------------------------------------------------------------
    // Установка
    // -----------------------------------------------------------------------

    /// Папка этой версии программы.
    pub(super) fn ag_version_dir(&self, app: &App, version: &str) -> Result<PathBuf, String> {
        if !safe_version(version) {
            return Err("плохая версия".into());
        }
        Ok(agents_dir(app).join(ID).join(version))
    }

    /// После скачивания: войти (первый раз) или проверить все аккаунты. Новая версия не запустилась —
    /// возвращаем прежнюю, а если запустилась — старые версии убираем.
    pub(super) async fn ag_after_install(&self, app: &Arc<App>, cancel: &CancellationToken) {
        let started = if self.ag_accounts(app).is_empty() {
            self.set_job(ID, "connecting", None);
            self.emit(app, ID).await;
            self.update_entry(app, ID, |e| {
                e.remove("error");
            });
            let aid = self.ag_next_id(app).unwrap_or_else(|| "a1".into());
            self.ag_create_account(app, &aid, "google");
            let _ = std::fs::create_dir_all(home_of(app, &aid));
            self.ag_login(app, &aid, cancel).await;
            // Программа запустилась, даже если человек не дошёл до конца входа.
            true
        } else {
            self.set_job(ID, "checking", None);
            self.emit(app, ID).await;
            self.ag_probe_all(app, cancel).await
        };
        let entry = self.entry(app, ID);
        let current = entry["version"].as_str().unwrap_or_default().to_string();
        let previous = entry["previous"].clone();
        if started {
            if let Ok(keep) = self.ag_version_dir(app, &current) {
                if let Some(root) = keep.parent() {
                    for dir in std::fs::read_dir(root).into_iter().flatten().flatten() {
                        let name = dir.file_name().to_string_lossy().to_string();
                        if dir.path() != keep && name != "accounts" && name != "tmp" {
                            let _ = std::fs::remove_dir_all(dir.path());
                        }
                    }
                }
            }
            self.update_entry(app, ID, |e| {
                e.remove("previous");
            });
        } else if previous["command"].is_string() {
            let why = self.entry(app, ID)["accounts"].as_object().and_then(|m| m.values().find_map(|a| a["error"].as_str().map(str::to_string)));
            if let Ok(bad) = self.ag_version_dir(app, &current) {
                let _ = std::fs::remove_dir_all(bad);
            }
            self.update_entry(app, ID, |e| {
                e.insert("version".into(), previous["version"].clone());
                e.insert("command".into(), previous["command"].clone());
                e.remove("previous");
                e.insert("installError".into(), json!(format!("Новая версия не запустилась, оставил прежнюю{}", why.map(|w| format!(": {w}")).unwrap_or_default())));
            });
            // Прежняя версия снова проверяет аккаунты: неудачный запуск новой не должен их «отключить».
            self.ag_probe_all(app, cancel).await;
        }
    }

    pub(super) fn ag_remember_previous(&self, app: &App, new_version: &str) {
        let old = self.entry(app, ID);
        let (Some(version), Some(command)) = (old["version"].as_str(), old["command"].as_str()) else { return };
        if version == new_version || command.is_empty() {
            return;
        }
        let previous = json!({ "version": version, "command": command });
        self.update_entry(app, ID, |e| {
            e.insert("previous".into(), previous);
        });
    }

    // -----------------------------------------------------------------------
    // Что показать
    // -----------------------------------------------------------------------

    fn ag_account_label(account: &Value, index: usize) -> String {
        if let Some(email) = account["email"].as_str().filter(|e| !e.is_empty()) {
            return email.to_string();
        }
        let a = &account["account"];
        if let Some(text) = a["account"]["email"].as_str().or(a["email"].as_str()).or(a["label"].as_str()).filter(|t| !t.is_empty()) {
            return text.to_string();
        }
        if account["kind"] == "key" {
            return "Ключ API".into();
        }
        format!("Аккаунт {}", index + 1)
    }

    /// Карточка «Gemini» и, если аккаунтов больше одного, по записи на каждый аккаунт для выбора модели бота.
    pub(super) fn ag_views(&self, app: &App, agent: &Value) -> Vec<Value> {
        let mut main = self.view(app, agent, Some("Gemini"));
        let accounts = self.ag_accounts(app);
        let key = self.ag_key(app);
        let busy = self.jobs.lock().unwrap().contains_key(ID);
        let rows: Vec<Value> = accounts
            .iter()
            .enumerate()
            .map(|(i, (aid, a))| {
                let status = if a["connected"] == true { "connected" } else if a["error"].is_string() { "error" } else { "not_connected" };
                json!({
                    "id": aid, "kind": a["kind"], "label": Self::ag_account_label(a, i), "email": a["email"],
                    "connected": a["connected"] == true, "status": status, "error": a["error"],
                    "driver": format!("acp:{ID}@{aid}"), "usedBy": Self::used_by(app, &format!("acp:{ID}@{aid}")) + if i == 0 { Self::used_by(app, &format!("acp:{ID}")) } else { 0 },
                })
            })
            .collect();
        main["builtin"] = json!(true);
        main["about"] = json!(ABOUT);
        main["accounts"] = json!(rows);
        main["maxAccounts"] = json!(MAX_ACCOUNTS);
        main["hasKey"] = json!(key.is_some());
        main["keyTail"] = json!(key.as_deref().map(|k| k.chars().rev().take(4).collect::<Vec<_>>().into_iter().rev().collect::<String>()));
        main["busy"] = json!(busy);
        main["signinLink"] = json!(busy && acp::signin_link().is_some());
        main["icon"] = Value::Null;
        main["releaseError"] = agent["releaseError"].clone();
        main["supported"] = json!(asset_name().is_some());
        main["usedBy"] = json!(Self::used_by(app, &format!("acp:{ID}")));
        let mut out = vec![main.clone()];
        if accounts.len() > 1 {
            for (i, (aid, a)) in accounts.iter().enumerate().skip(1) {
                let mut extra = main.clone();
                let driver = format!("acp:{ID}@{aid}");
                extra["id"] = json!(format!("{ID}@{aid}"));
                extra["name"] = json!(format!("Gemini · {}", Self::ag_account_label(a, i)));
                extra["driver"] = json!(driver);
                extra["pickerOnly"] = json!(true);
                extra["accounts"] = json!([]);
                extra["status"] = json!(if a["connected"] == true { "connected" } else { "not_connected" });
                extra["models"] = a["models"].clone();
                extra["usedBy"] = json!(Self::used_by(app, &driver));
                extra["isDefault"] = json!(app.store.settings().driver == driver);
                if !extra["models"].is_array() {
                    extra["models"] = json!([]);
                }
                out.push(extra);
            }
        }
        out
    }
}

fn plain(path: &Path) -> String {
    let text = path.display().to_string();
    text.strip_prefix(r"\\?\").map(str::to_string).unwrap_or(text)
}
