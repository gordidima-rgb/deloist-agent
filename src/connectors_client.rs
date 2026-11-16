//! Клиент модуля коннекторов (ветка `feat/connectors-base`). Каталог зовёт его, когда человек жмёт «Подключить».
//!
//! Пока модуля нет в этой сборке, клиент ходит по HTTP туда, куда указывает `CREW_CONNECTORS_URL`
//! (только debug-сборка и только loopback: в тестах там заглушка). Без адреса — честное «ещё нет».
//! Когда ветка коннекторов вольётся, вызовы `Connectors::*` заменяются прямыми вызовами её модуля,
//! а форма запросов ниже — то, что каталог от неё ждёт:
//!
//! | Шаг | Запрос | Ответ |
//! |---|---|---|
//! | установить | `POST /api/connectors/install {catalogId, manifest}` | `{id, card}` — карточка согласия |
//! | согласие | `POST /api/connectors/{id}/confirm` | `{status}` |
//! | ключ | `POST /api/connectors/{id}/secret {name, value}` | `{status}` |
//! | OAuth | `POST /api/connectors/{id}/oauth/start` | `{url}` |
//! | проверка | `POST /api/connectors/{id}/check` | `{tools: [...]}` |
//! | удалить | `DELETE /api/connectors/{id}` | `{ok}` |

use std::time::Duration;

use serde_json::{json, Value};

#[derive(Debug)]
pub struct ConnectorError {
    /// HTTP-статус ответа; 0 — не дошли (нет модуля, нет связи).
    pub status: u16,
    pub message: String,
    pub data: Value,
}

impl ConnectorError {
    fn unavailable() -> Self {
        Self {
            status: 503,
            message: "подключение приложений появится вместе с модулем коннекторов".into(),
            data: Value::Null,
        }
    }
}

pub struct Installed {
    pub id: String,
    pub card: Value,
}

pub struct Connectors {
    base: Option<String>,
}

/// Адрес заглушки или модуля: только эта машина, ключи наружу не уходят.
fn loopback(url: &str) -> bool {
    ["http://127.0.0.1:", "http://localhost:", "http://[::1]:"].iter().any(|p| url.starts_with(p))
}

impl Connectors {
    pub fn from_env() -> Self {
        let base = crate::update::test_env("CREW_CONNECTORS_URL").map(|u| u.trim_end_matches('/').to_string()).filter(|u| loopback(u));
        Self { base }
    }

    async fn call(&self, method: reqwest::Method, path: &str, body: Option<Value>) -> Result<Value, ConnectorError> {
        let Some(base) = &self.base else {
            return Err(ConnectorError::unavailable());
        };
        let client = reqwest::Client::builder().no_proxy().timeout(Duration::from_secs(30)).build().map_err(|e| ConnectorError {
            status: 0,
            message: e.to_string(),
            data: Value::Null,
        })?;
        let mut request = client.request(method, format!("{base}{path}"));
        if let Some(body) = body {
            request = request.json(&body);
        }
        let response = request.send().await.map_err(|_| ConnectorError {
            status: 502,
            message: "модуль коннекторов не ответил".into(),
            data: Value::Null,
        })?;
        let status = response.status().as_u16();
        let data: Value = response.json().await.unwrap_or(Value::Null);
        if (200..300).contains(&status) {
            return Ok(data);
        }
        // 404 на самом маршруте install значит, что модуля нет, а не что нет подключения.
        let message = data.get("error").and_then(Value::as_str).unwrap_or("модуль коннекторов отказал").to_string();
        Err(ConnectorError { status, message, data })
    }

    pub async fn install(&self, catalog_id: &str, manifest: &Value) -> Result<Installed, ConnectorError> {
        let data = self
            .call(reqwest::Method::POST, "/api/connectors/install", Some(json!({ "catalogId": catalog_id, "manifest": manifest })))
            .await?;
        let id = data.get("id").and_then(Value::as_str).unwrap_or_default().to_string();
        if id.is_empty() {
            return Err(ConnectorError { status: 502, message: "модуль коннекторов не вернул id".into(), data });
        }
        Ok(Installed { id, card: data.get("card").cloned().unwrap_or(Value::Null) })
    }

    pub async fn confirm(&self, id: &str) -> Result<Value, ConnectorError> {
        self.call(reqwest::Method::POST, &format!("/api/connectors/{}/confirm", seg(id)), None).await
    }

    pub async fn secret(&self, id: &str, name: &str, value: &str) -> Result<Value, ConnectorError> {
        self.call(reqwest::Method::POST, &format!("/api/connectors/{}/secret", seg(id)), Some(json!({ "name": name, "value": value })))
            .await
    }

    /// `login` — client id и секрет клиента, если у сервера авторизации нет регистрации клиентов.
    pub async fn oauth_start(&self, id: &str, login: Value) -> Result<String, ConnectorError> {
        let data = self.call(reqwest::Method::POST, &format!("/api/connectors/{}/oauth/start", seg(id)), Some(login)).await?;
        let url = data.get("url").and_then(Value::as_str).unwrap_or_default().to_string();
        if !url.starts_with("https://") && !url.starts_with("http://127.0.0.1") {
            return Err(ConnectorError { status: 502, message: "модуль коннекторов вернул странный адрес входа".into(), data });
        }
        Ok(url)
    }

    pub async fn check(&self, id: &str) -> Result<Value, ConnectorError> {
        let data = self.call(reqwest::Method::POST, &format!("/api/connectors/{}/check", seg(id)), None).await?;
        Ok(data.get("tools").cloned().unwrap_or(json!([])))
    }

    pub async fn remove(&self, id: &str) -> Result<(), ConnectorError> {
        match self.call(reqwest::Method::DELETE, &format!("/api/connectors/{}", seg(id)), None).await {
            // Уже нет — удалять нечего.
            Err(e) if e.status == 404 => Ok(()),
            other => other.map(|_| ()),
        }
    }
}

/// id подключения идёт в путь: только безопасные символы.
fn seg(id: &str) -> String {
    id.chars().filter(|c| c.is_ascii_alphanumeric() || *c == '_' || *c == '-' || *c == '.').collect()
}
