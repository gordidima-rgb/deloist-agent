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
/// Ответ при попытке запустить на ПК бота, который живёт только в облаке.
pub const NO_CLOUD_BOT: &str = "бот работает только в облаке: на этом компьютере он не запускается";

/// `Err(NO_CONSOLE)` в облаке.
pub fn deny(message: &str) -> Result<(), String> {
    if active() { Err(message.to_string()) } else { Ok(()) }
}

/// Чистое правило: бот «только в облаке» не работает там, где облако не активно.
pub fn blocked(bot: &crate::store::Bot, cloud_active: bool) -> bool {
    bot.cloud_only && !cloud_active
}

/// Главная проверка «только в облаке». Вызывается в самом исполнении хода, а не только в окне:
/// бот с таким флагом не стартует ни из чата, ни из рутины, ни как делегат, ни как шаг процесса.
/// Флаг читается из сохранённого состояния каждый раз, поэтому перезапуск ничего не сбрасывает.
pub fn dispatch_error(bot: &crate::store::Bot) -> Option<String> {
    if blocked(bot, active()) {
        Some(NO_CLOUD_BOT.to_string())
    } else {
        None
    }
}

/// Та же проверка по id бота: в глубине хода объект `Bot` мог быть скопирован до изменения флага.
pub fn dispatch_error_by_id(app: &crate::App, bot_id: &str) -> Option<String> {
    let bot = app.store.bot(bot_id)?;
    dispatch_error(&bot)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cloud_only_bot_is_blocked_outside_the_cloud() {
        let mut bot = crate::store::Bot::default();
        assert!(!blocked(&bot, false), "обычный бот работает везде");
        bot.cloud_only = true;
        assert!(blocked(&bot, false), "на ПК облачный бот не запускается");
        assert!(!blocked(&bot, true), "в облаке тот же бот работает");
    }
}
