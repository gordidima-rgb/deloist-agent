//! Док агентов у края экрана: есть только в Windows-сборке с окном. Здесь те же имена без действий.

use std::sync::Arc;

use crate::guard::Guard;

/// Клик по «+»: главное окно откроет создание бота. Ботов с таким id не бывает.
pub const NEW_BOT: &str = "+new";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Rect {
    pub left: i32,
    pub top: i32,
    pub right: i32,
    pub bottom: i32,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Screen {
    pub name: String,
    pub area: Rect,
    pub work: Rect,
    pub primary: bool,
}

#[derive(Default)]
pub struct Dock;

impl Dock {
    pub fn attach(&self, _origin: String, _guard: Arc<Guard>) {}
    pub fn pid(&self) -> Option<u32> {
        None
    }
    pub fn stop(&self) {}
}

pub fn screens() -> Vec<Screen> {
    Vec::new()
}

pub fn sync_with_store(_app: &crate::App) {}

pub fn run(_layer: &str, _edge: Option<String>, _display: Option<String>) -> Result<(), String> {
    Err("Док доступен только в Windows.".into())
}
