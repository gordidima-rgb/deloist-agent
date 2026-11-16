//! Политика облака: у ботов нет консоли. Только файлы в своей папке и интернет.
//! Действует при `--server` и `CREW_CLOUD=1`; на ПК `active()` всегда false и ничего не меняется.

/// Единственный признак облака для всей программы.
pub fn active() -> bool {
    crate::guard::cloud_enabled()
}

pub const NO_CONSOLE: &str = "в облаке нет консоли: только файлы в своей папке и интернет";
pub const NO_DRIVER: &str = "в облаке нет консоли: боты через командную строку и агентов недоступны, только API-провайдеры";
pub const NO_INSTALL: &str = "в облаке установка программ отключена";
pub const NO_STDIO: &str = "в облаке нет консоли: подключения через запуск команды недоступны, только по HTTP";
pub const NO_FEATURE: &str = "в облаке эта функция отключена";
pub const OUTSIDE_WORKSPACE: &str = "в облаке доступна только своя папка бота";

/// `Err(NO_CONSOLE)` в облаке.
pub fn deny(message: &str) -> Result<(), String> {
    if active() { Err(message.to_string()) } else { Ok(()) }
}
