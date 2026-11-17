//! Сборка без окна (`--no-default-features`): остаётся режим `--server`.
//! Те же имена, что у `desktop.rs`, чтобы остальной код не знал о сборке.

pub fn show_error(message: &str) {
    eprintln!("{message}");
}

/// У сервера нет рабочего стола: открывать ссылку некому, человек откроет её сам.
pub(crate) fn open_in_browser(_url: &str) {}

pub fn activate_running_window() -> bool {
    false
}

/// Класс окна нужен только сборке с окном; здесь его нет, но имя общее.
#[cfg(windows)]
pub(crate) const WINDOW_CLASS: &str = "DeloistAgentWindow";
