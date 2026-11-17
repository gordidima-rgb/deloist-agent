//! Public build: device relay (phone and second PC) is not part of this repository.
//! Same names as the real `relay` module, no network.
#![allow(dead_code)]

use std::sync::Arc;

use serde_json::{json, Value};

use crate::App;

type Failure = (u16, String);
/// (HTTP status, code, message).
pub type RemoteFailure = (u16, String, String);

const UNAVAILABLE: &str = "Связь с другими устройствами недоступна в этой сборке";

fn unavailable() -> RemoteFailure {
    (503, "no_link".to_string(), UNAVAILABLE.to_string())
}

#[derive(Default)]
pub struct Relay;

impl Relay {
    pub fn list(&self, _app: &App) -> Value {
        json!({ "devices": [], "pending": [], "connected": false, "pcName": "" })
    }

    pub fn revoke(&self, _app: &App, _id: &str) -> Result<(), Failure> {
        Err((503, UNAVAILABLE.to_string()))
    }

    pub fn set_actions(&self, _app: &App, _id: &str, _enabled: bool, _confirm: Option<&str>) -> Result<(), Failure> {
        Err((503, UNAVAILABLE.to_string()))
    }

    pub fn decide(&self, _app: &App, _req: &str, _allow: bool, _code: Option<&str>) -> Result<Value, Failure> {
        Err((503, UNAVAILABLE.to_string()))
    }

    pub fn remote_pcs(&self, _app: &App) -> Value {
        json!({ "connected": false, "pcs": [], "candidates": [], "pairing": null })
    }

    pub async fn remote_pair_start(&self, _app: &App, _pc: &str) -> Result<Value, RemoteFailure> {
        Err(unavailable())
    }

    pub fn remote_pair_ack(&self, _app: &App, _req: &str, _code: &str) -> Result<Value, RemoteFailure> {
        Err(unavailable())
    }

    pub fn remote_forget(&self, _app: &App, _pc: &str) -> Result<(), RemoteFailure> {
        Err(unavailable())
    }

    pub async fn remote_call(&self, _app: &App, _pc: &str, _op: &str, _args: Value) -> Result<Value, RemoteFailure> {
        Err(unavailable())
    }
}

pub fn start(_app: Arc<App>) {}
