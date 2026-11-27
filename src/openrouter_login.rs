//! «Начать бесплатно»: вход в OpenRouter одним кликом вместо ручного копирования ключа.
//!
//! Схема из документации OpenRouter (https://openrouter.ai/docs/guides/overview/auth/oauth):
//! браузер открывает `/auth?callback_url=…&code_challenge=…&code_challenge_method=S256&state=…`,
//! после входа OpenRouter возвращает человека на `callback_url` с `code` и тем же `state`,
//! затем `POST /api/v1/auth/keys` с `{code, code_verifier, code_challenge_method}` отдаёт `{key}`.
//! Ключ сразу уходит в хранилище секретов и не попадает ни в ответ API, ни в события, ни в журнал.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

use serde_json::{json, Value};
use tokio::io::AsyncReadExt;
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::mpsc;

use crate::providers::Providers;
use crate::App;

const ID: &str = "openrouter";
/// Бесплатный роутер OpenRouter: сам выбирает бесплатную модель с нужными возможностями (в том числе вызовом инструментов).
/// https://openrouter.ai/docs/guides/routing/routers/free-models-router
pub const FREE_MODEL: &str = "openrouter/free";
const WAIT: Duration = Duration::from_secs(300);
/// Голова запроса возврата должна прийти целиком за это время и уложиться в этот размер.
const HEAD_TIMEOUT: Duration = Duration::from_secs(5);
const HEAD_MAX: usize = 8192;
/// Столько соединений на возврат обслуживаем одновременно; остальные закрываем сразу (чужая программа на петле не забьёт задачи).
const MAX_CONNECTIONS: usize = 16;
/// Ответ обмена кода на ключ крошечный; больше этого не читаем.
const EXCHANGE_MAX: usize = 64 * 1024;
/// Номер входа: отменённый вход не может записать ключ и снять метку нового.
static LOGIN_GEN: AtomicU64 = AtomicU64::new(0);

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
            Some((_, handle)) => {
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
        let listener = TcpListener::bind(("127.0.0.1", 0)).await.map_err(|e| format!("не открылся порт для входа: {e}"))?;
        let port = listener.local_addr().map_err(|e| e.to_string())?.port();
        // OpenRouter допускает localhost на любом порту, а 127.0.0.1 в документации не назван:
        // адрес в ссылке `localhost`, поэтому слушаем и IPv6-петлю на том же порту (не вышло — не страшно).
        let listener6 = TcpListener::bind(("::1", port)).await.ok();
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
        let generation = LOGIN_GEN.fetch_add(1, Ordering::SeqCst) + 1;
        {
            // Старый вход отменяется, новый запускается и записывается одним шагом без await:
            // завершающий код задачи ждёт этого же замка, поэтому не успеет стереть чужую метку.
            let mut slot = self.login.lock().unwrap();
            if let Some((_, old)) = slot.take() {
                old.abort();
            }
            let task = tokio::spawn(async move {
                let result = tokio::time::timeout(WAIT, wait_for_code(app2.store.settings().ui_lang == "en", vec![Some(listener), listener6], &state, "/cb")).await;
                let outcome = match result {
                    Err(_) => Err("вход не завершён за 5 минут".to_string()),
                    Ok(Err(e)) => Err(e),
                    Ok(Ok(code)) => exchange(&code, &verifier).await,
                };
                let outcome = match outcome {
                    Ok(key) => me.finish_login(&app2, &key, generation).await,
                    Err(e) => Err(e),
                };
                if let Err(error) = outcome {
                    if me.is_current_login(generation) {
                        // Уже подключённый провайдер отключённым не помечаем: только ошибка.
                        let keep = !app2.store.own_key(ID).is_empty();
                        me.update_entry(&app2, ID, |e| {
                            if !keep {
                                e.insert("connected".into(), json!(false));
                            }
                            e.insert("error".into(), json!(error));
                        });
                    }
                }
                let cleared = {
                    let mut slot = me.login.lock().unwrap();
                    if slot.as_ref().is_some_and(|(g, _)| *g == generation) {
                        slot.take();
                        true
                    } else {
                        false
                    }
                };
                if cleared {
                    me.emit(&app2, ID).await;
                }
            });
            *slot = Some((generation, task.abort_handle()));
        }
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

    fn is_current_login(&self, generation: u64) -> bool {
        self.login.lock().unwrap().as_ref().is_some_and(|(g, _)| *g == generation)
    }

    async fn finish_login(&self, app: &Arc<App>, key: &str, generation: u64) -> Result<(), String> {
        if !self.is_current_login(generation) {
            return Err("вход отменён".into());
        }
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
/// Каждое соединение обслуживается своей задачей, медленный клиент остальным не мешает.
pub(crate) async fn wait_for_code(en: bool, listeners: Vec<Option<TcpListener>>, state: &str, path: &str) -> Result<String, String> {
    struct Stop(Vec<tokio::task::AbortHandle>);
    impl Drop for Stop {
        fn drop(&mut self) {
            for h in &self.0 {
                h.abort();
            }
        }
    }
    let (tx, mut rx) = mpsc::channel::<Result<String, String>>(8);
    let slots = Arc::new(tokio::sync::Semaphore::new(MAX_CONNECTIONS));
    let mut handles = Vec::new();
    for listener in listeners.into_iter().flatten() {
        let (tx, state, path, slots) = (tx.clone(), state.to_string(), path.to_string(), slots.clone());
        handles.push(
            tokio::spawn(async move {
                loop {
                    let Ok((socket, _)) = listener.accept().await else {
                        tokio::time::sleep(Duration::from_millis(50)).await;
                        continue;
                    };
                    let Ok(permit) = slots.clone().try_acquire_owned() else { continue };
                    let (tx, state, path) = (tx.clone(), state.clone(), path.clone());
                    tokio::spawn(async move {
                        serve_one(en, socket, &state, &path, tx).await;
                        drop(permit);
                    });
                }
            })
            .abort_handle(),
        );
    }
    drop(tx);
    let _stop = Stop(handles);
    rx.recv().await.unwrap_or_else(|| Err("не открылся порт для входа".into()))
}

/// Голова запроса: одна общая граница по времени и размеру.
async fn read_head(socket: &mut TcpStream) -> Option<String> {
    tokio::time::timeout(HEAD_TIMEOUT, async {
        let mut buf: Vec<u8> = Vec::with_capacity(1024);
        let mut chunk = [0u8; 1024];
        loop {
            let n = socket.read(&mut chunk).await.ok()?;
            if n == 0 {
                return None;
            }
            buf.extend_from_slice(&chunk[..n]);
            if buf.windows(4).any(|w| w == b"\r\n\r\n") {
                return Some(String::from_utf8_lossy(&buf).into_owned());
            }
            if buf.len() >= HEAD_MAX {
                return None;
            }
        }
    })
    .await
    .ok()
    .flatten()
}

async fn serve_one(en: bool, mut socket: TcpStream, state: &str, path: &str, tx: mpsc::Sender<Result<String, String>>) {
    let Some(head) = read_head(&mut socket).await else { return };
    let target = head.lines().next().and_then(|l| l.strip_prefix("GET ")).and_then(|r| r.split(' ').next()).unwrap_or("");
    let say = |ru: &'static str, english: &'static str| if en { english } else { ru };
    let parsed = reqwest::Url::parse(&format!("http://localhost{target}")).ok();
    let Some(parsed) = parsed.filter(|u| u.path() == path) else {
        crate::oauth::respond(&mut socket, 404, say("Не тот адрес.", "Wrong address.")).await;
        return;
    };
    let params: std::collections::HashMap<String, String> = parsed.query_pairs().map(|(k, v)| (k.into_owned(), v.into_owned())).collect();
    if !crate::oauth::same(params.get("state").map(String::as_str).unwrap_or(""), state) {
        crate::oauth::respond(&mut socket, 400, say("Ответ не от этого входа (state не совпал).", "This answer is not from this sign-in (state mismatch).")).await;
        return;
    }
    if params.contains_key("error") {
        crate::oauth::respond(&mut socket, 400, say("Вход отменён. Вернитесь в приложение.", "Sign-in cancelled. Return to the app.")).await;
        let _ = tx.send(Err("вход отменён в браузере".into())).await;
        return;
    }
    let Some(code) = params.get("code").filter(|c| !c.is_empty()) else {
        crate::oauth::respond(&mut socket, 400, say("В ответе нет кода.", "The answer has no code.")).await;
        return;
    };
    crate::oauth::respond(&mut socket, 200, say("Готово. Можно вернуться в приложение.", "Done. You can return to the app.")).await;
    let _ = tx.send(Ok(code.clone())).await;
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
    let odd = || "OpenRouter ответил не так, как ожидалось".to_string();
    if response.content_length().is_some_and(|n| n as usize > EXCHANGE_MAX) {
        return Err(odd());
    }
    let mut response = response;
    let mut bytes: Vec<u8> = Vec::new();
    while let Some(chunk) = response.chunk().await.map_err(|_| odd())? {
        if bytes.len() + chunk.len() > EXCHANGE_MAX {
            return Err(odd());
        }
        bytes.extend_from_slice(&chunk);
    }
    let body: Value = serde_json::from_slice(&bytes).map_err(|_| odd())?;
    body["key"].as_str().map(str::trim).filter(|k| !k.is_empty()).map(str::to_string).ok_or_else(|| "OpenRouter не прислал ключ".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::AsyncWriteExt;

    #[tokio::test]
    async fn connections_over_the_cap_are_dropped_and_the_real_answer_still_comes_through() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let waiting = tokio::spawn(wait_for_code(false, vec![Some(listener)], "st", "/cb"));
        // Молчащие соединения занимают все места.
        let mut idle = Vec::new();
        for _ in 0..MAX_CONNECTIONS {
            idle.push(TcpStream::connect(("127.0.0.1", port)).await.unwrap());
        }
        tokio::time::sleep(Duration::from_millis(200)).await;
        let mut extra = TcpStream::connect(("127.0.0.1", port)).await.unwrap();
        let _ = extra.write_all(b"GET /cb?code=c&state=st HTTP/1.1\r\n\r\n").await;
        let mut out = Vec::new();
        let read = tokio::time::timeout(Duration::from_secs(2), extra.read_to_end(&mut out)).await;
        assert!(read.is_ok() && out.is_empty(), "лишнее соединение закрыто без ответа");
        assert!(!waiting.is_finished());
        // Места освободились: настоящий возврат проходит.
        drop(idle);
        tokio::time::sleep(Duration::from_millis(200)).await;
        let mut ok = TcpStream::connect(("127.0.0.1", port)).await.unwrap();
        ok.write_all(b"GET /cb?code=the-code&state=st HTTP/1.1\r\n\r\n").await.unwrap();
        assert_eq!(waiting.await.unwrap().unwrap(), "the-code");
    }
}
