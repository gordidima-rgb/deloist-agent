//! Public build: the Deloist account and subscription are not part of this repository.
//! Same names as the real `account` module, no network: sign-in is never required, the account
//! calls answer "not available in this build", and the "Deloist" model provider has no models.
#![allow(dead_code)]

use serde_json::{json, Value};

const UNAVAILABLE: &str = "Аккаунт Deloist недоступен в этой сборке";

pub fn auth_url() -> Option<String> {
    None
}

pub fn required() -> bool {
    false
}

pub fn llm_url() -> Option<String> {
    None
}

pub fn site_url() -> Option<reqwest::Url> {
    None
}

pub const PLAN_DEFAULT_MODEL: &str = "";
pub const PLAN_MODELS: &[(&str, &str)] = &[];

pub fn plan_json() -> Value {
    json!({})
}

/// Ошибка аккаунта: код ответа для окна и текст для человека.
pub struct Failure {
    pub status: u16,
    pub message: String,
    pub code: Option<String>,
}

fn unavailable() -> Failure {
    Failure { status: 503, message: UNAVAILABLE.to_string(), code: None }
}

pub const CONSENT_NEEDED: &str = UNAVAILABLE;

#[derive(Default)]
pub struct Account;

impl Account {
    pub async fn status(&self, app: &crate::App) -> Value {
        json!({
            "required": false,
            "signedIn": false,
            "email": "",
            "name": "",
            "lang": app.store.settings().ui_lang,
            "onboarded": app.store.settings().team_onboarded,
            "consentPending": false,
            "siteUrl": "",
        })
    }
    pub async fn send_code(&self, _app: &crate::App, _input: &Value) -> Result<Value, Failure> {
        Err(unavailable())
    }
    pub async fn verify_code(&self, _app: &crate::App, _input: &Value) -> Result<Value, Failure> {
        Err(unavailable())
    }
    pub async fn start_browser(&self, _input: &Value) -> Result<Value, Failure> {
        Err(unavailable())
    }
    pub async fn poll_browser(&self, _app: &crate::App) -> Result<Value, Failure> {
        Err(unavailable())
    }
    pub fn cancel_browser(&self) {}
    pub async fn sign_out(&self, _app: &crate::App) -> Result<(), String> {
        Ok(())
    }
    pub fn consent_pending(&self) -> bool {
        false
    }
    pub fn require_consent(&self, _app: &crate::App) {}
    pub async fn consent(&self, _app: &crate::App) -> Result<Value, Failure> {
        Err(unavailable())
    }
    pub async fn accept_consent(&self, _app: &crate::App, _input: &Value) -> Result<Value, Failure> {
        Err(unavailable())
    }
    pub async fn details(&self, _app: &crate::App) -> Result<Value, Failure> {
        Err(unavailable())
    }
    pub async fn request_refund(&self, _app: &crate::App) -> Result<Value, Failure> {
        Err(unavailable())
    }
    pub async fn export(&self, _app: &crate::App) -> Result<Value, Failure> {
        Err(unavailable())
    }
    pub async fn delete(&self, _app: &crate::App, _input: &Value) -> Result<Value, Failure> {
        Err(unavailable())
    }
    pub fn note_wallet(&self, _data: &Value) {}
    pub fn wallet_today(&self) -> Option<crate::limit::WalletDay> {
        None
    }
    pub async fn wallet(&self, _app: &crate::App) -> Result<Value, Failure> {
        Err(unavailable())
    }
    pub async fn choose_plan(&self, _app: &crate::App, _input: &Value) -> Result<Value, Failure> {
        Err(unavailable())
    }
    pub async fn topup(&self, _app: &crate::App, _input: &Value) -> Result<Value, Failure> {
        Err(unavailable())
    }
}
