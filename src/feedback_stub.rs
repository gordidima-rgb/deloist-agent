//! Public build: feedback goes to the Deloist server, which is not part of this repository.

use serde_json::Value;

pub async fn send(_input: &Value, _lang: &str) -> Result<(), String> {
    Err("Отправка обращений недоступна в этой сборке. Напиши в GitHub Issues".to_string())
}
