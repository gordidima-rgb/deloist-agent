//! Питомцы на столе: окна есть только в Windows-сборке с окном. Здесь те же имена без действий.

use std::sync::Arc;

use crate::guard::Guard;

pub const MAX_PETS: usize = 8;

#[derive(Default)]
pub struct Pets;

impl Pets {
    pub fn attach(&self, _origin: String, _guard: Arc<Guard>) {}
    pub fn request_open(&self, _bot_id: &str) {}
    pub fn take_open(&self) -> Option<String> {
        None
    }
    pub fn stop(&self) {}
}

pub fn sync_with_store(_app: &crate::App) {}

pub fn run(_mode: &str, _bot: Option<String>, _at: Option<String>, _slot: Option<String>) -> Result<(), String> {
    Err("Питомцы доступны только в Windows.".into())
}
