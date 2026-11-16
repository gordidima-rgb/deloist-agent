//! «Начать бесплатно»: вход в OpenRouter одним кликом вместо ручного копирования ключа.
//!
//! Схема из документации OpenRouter (https://openrouter.ai/docs/guides/overview/auth/oauth):
//! браузер открывает `/auth?callback_url=…&code_challenge=…&code_challenge_method=S256&state=…`,
//! после входа OpenRouter возвращает человека на `callback_url` с `code` и тем же `state`,
//! затем `POST /api/v1/auth/keys` с `{code, code_verifier, code_challenge_method}` отдаёт `{key}`.
//! Ключ сразу уходит в хранилище секретов и не попадает ни в ответ API, ни в события, ни в журнал.

use std::sync::Arc;
use std::time::Duration;

use serde_json::{json, Value};
use tokio::io::AsyncReadExt;
use tokio::net::TcpListener;

use crate::providers::Providers;
use crate::App;

const ID: &str = "openrouter";
/// Бесплатный роутер OpenRouter: сам выбирает бесплатную модель с нужными возможностями (в том числе вызовом инструментов).
/// https://openrouter.ai/docs/guides/routing/routers/free-models-router
pub const FREE_MODEL: &str = "openrouter/free";
const WAIT: Duration = Duration::from_secs(300);

/// Адрес OpenRouter. В отладочной сборке тест подменяет его своим сервером.
fn base() -> String {
    if cfg!(debug_assertions) {
        if let Ok(url) = std::env::var("CREW_TEST_OPENROUTER_URL") {
            if !url.trim().is_empty() {
                return url.trim().trim_end_matches('/').to_string();
            }
        }
    }
    "https://openrouter.ai".to_string()
}

fn overridden() -> bool {
    cfg!(debug_assertions) && std::env::var("CREW_TEST_OPENROUTER_URL").is_ok_and(|u| !u.trim().is_empty())
}

impl Providers {
    pub fn login_active(&self) -> bool {
        self.login.lock().unwrap().is_some()
    }

    pub fn cancel_login(&self) -> bool {
        match self.login.lock().unwrap().take() {
            Some(handle) => {
                handle.abort();
                true
            }
            None => false,
        }
    }

    /// Открыть вход в браузере и ждать возврата. Ответ — только «начато»; результат приходит событием `provider`.
    pub async fn start_openrouter_login(&self, app: &Arc<App>) -> Result<Value, String> {
        if crate::cloud::active() {
            return Err("в облаке вход через браузер недоступен: впишите ключ вручную".into());
        }
        self.cancel_login();
        let listener = TcpListener::bind(("127.0.0.1", 0)).await.map_err(|e| format!("не открылся порт для входа: {e}"))?;
        let port = listener.local_addr().map_err(|e| e.to_string())?.port();
        let verifier = crate::oauth::new_verifier();
        let state = crate::guard::random_token();
        let callback = format!("http://localhost:{port}/cb");
        let mut url = reqwest::Url::parse(&format!("{}/auth", base())).map_err(|e| e.to_string())?;
        url.query_pairs_mut()
            .append_pair("callback_url", &callback)
            .append_pair("code_challenge", &crate::oauth::challenge(&verifier))
            .append_pair("code_challenge_method", "S256")
            .append_pair("state", &state);
        let app2 = app.clone();
        let me = app.providers.clone();
        let task = tokio::spawn(async move {
            let result = tokio::time::timeout(WAIT, wait_for_code(&app2, listener, &state)).await;
            let outcome = match result {
                Err(_) => Err("вход не завершён за 5 минут".to_string()),
                Ok(Err(e)) => Err(e),
                Ok(Ok(code)) => exchange(&code, &verifier).await,
            };
            let outcome = match outcome {
                Ok(key) => me.finish_login(&app2, &key).await,
                Err(e) => Err(e),
            };
            if let Err(error) = outcome {
                me.update_entry(&app2, ID, |e| {
                    e.insert("connected".into(), json!(false));
                    e.insert("error".into(), json!(error));
                });
            }
            me.login.lock().unwrap().take();
            me.emit(&app2, ID).await;
        });
        *self.login.lock().unwrap() = Some(task.abort_handle());
        let mut out = json!({ "started": true });
        if overridden() {
            // Тест открывает ссылку сам; настоящий браузер в тесте не запускаем.
            out["authUrl"] = json!(url.as_str());
        } else {
            crate::desktop::open_in_browser(url.as_str());
        }
        self.emit(app, ID).await;
        Ok(out)
    }

    async fn finish_login(&self, app: &Arc<App>, key: &str) -> Result<(), String> {
        app.store.set_own_key(ID, key).await.map_err(|e| e.to_string())?;
        // Модель человека не трогаем; если не выбрана — бесплатный роутер, не платная модель.
        let has_model = self.get_entry_model(app, ID);
        self.update_entry(app, ID, |e| {
            e.insert("connected".into(), json!(true));
            e.insert("checkedAt".into(), json!(crate::store::now()));
            e.remove("error");
            if !has_model {
                e.insert("model".into(), json!(FREE_MODEL));
            }
        });
        Ok(())
    }

    fn get_entry_model(&self, app: &App, id: &str) -> bool {
        self.entry(app, id)["model"].as_str().is_some_and(|m| !m.trim().is_empty())
    }
}

/// Принять ровно один возврат с верным `state`. Чужие и неверные ответы отклоняем, вход не обрываем.
async fn wait_for_code(app: &Arc<App>, listener: TcpListener, state: &str) -> Result<String, String> {
    loop {
        let Ok((mut socket, _)) = listener.accept().await else { continue };
        let mut buf = vec![0u8; 8192];
        let mut len = 0;
        while len < buf.len() {
            match tokio::time::timeout(Duration::from_secs(5), socket.read(&mut buf[len..])).await {
                Ok(Ok(0)) | Err(_) | Ok(Err(_)) => break,
                Ok(Ok(n)) => {
                    len += n;
                    if buf[..len].windows(4).any(|w| w == b"\r\n\r\n") {
                        break;
                    }
                }
            }
        }
        let head = String::from_utf8_lossy(&buf[..len]).to_string();
        let target = head.lines().next().and_then(|l| l.strip_prefix("GET ")).and_then(|r| r.split(' ').next()).unwrap_or("");
        let en = app.store.settings().ui_lang == "en";
        let say = |ru: &'static str, english: &'static str| if en { english } else { ru };
        let parsed = reqwest::Url::parse(&format!("http://localhost{target}")).ok();
        let Some(parsed) = parsed.filter(|u| u.path() == "/cb") else {
            crate::oauth::respond(&mut socket, 404, say("Не тот адрес.", "Wrong address.")).await;
            continue;
        };
        let params: std::collections::HashMap<String, String> = parsed.query_pairs().map(|(k, v)| (k.into_owned(), v.into_owned())).collect();
        if !crate::oauth::same(params.get("state").map(String::as_str).unwrap_or(""), state) {
            crate::oauth::respond(&mut socket, 400, say("Ответ не от этого входа (state не совпал).", "This answer is not from this sign-in (state mismatch).")).await;
            continue;
        }
        if params.contains_key("error") {
            crate::oauth::respond(&mut socket, 400, say("Вход отменён. Вернитесь в приложение.", "Sign-in cancelled. Return to the app.")).await;
            return Err("вход отменён в браузере".into());
        }
        let Some(code) = params.get("code").filter(|c| !c.is_empty()) else {
            crate::oauth::respond(&mut socket, 400, say("В ответе нет кода.", "The answer has no code.")).await;
            continue;
        };
        crate::oauth::respond(&mut socket, 200, say("Готово. Можно вернуться в приложение.", "Done. You can return to the app.")).await;
        return Ok(code.clone());
    }
}

/// Обмен кода на ключ. В тексте ошибки — только статус, без тела ответа.
async fn exchange(code: &str, verifier: &str) -> Result<String, String> {
    let response = crate::oauth::http()
        .post(format!("{}/api/v1/auth/keys", base()))
        .json(&json!({ "code": code, "code_verifier": verifier, "code_challenge_method": "S256" }))
        .send()
        .await
        .map_err(|_| "OpenRouter не ответил: проверьте интернет и повторите".to_string())?;
    if !response.status().is_success() {
        return Err(format!("OpenRouter не выдал ключ (код {}): повторите вход", response.status().as_u16()));
    }
    let body: Value = response.json().await.map_err(|_| "OpenRouter ответил не так, как ожидалось".to_string())?;
    body["key"].as_str().map(str::trim).filter(|k| !k.is_empty()).map(str::to_string).ok_or_else(|| "OpenRouter не прислал ключ".to_string())
}
