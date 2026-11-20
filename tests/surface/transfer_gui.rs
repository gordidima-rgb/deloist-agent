//! Легкая проверка проводки интерфейса переноса без сборки фронтенда.
//! Веб-страница встраивается в exe как есть, поэтому эти тексты — часть контракта.
const APP: &str = include_str!("../../web/app.js");
const HTML: &str = include_str!("../../web/index.html");
const I18N: &str = include_str!("../../web/i18n.js");

#[test]
fn transfer_controls_are_wired_from_the_window_to_the_backend() {
    for needle in [
        "id=\"transfer-bot\"",
        "id=\"transfer-to-cloud\"",
        "id=\"transfer-from-cloud\"",
        "id=\"transfer-report\"",
    ] {
        assert!(HTML.contains(needle), "в index.html нет {needle}");
    }
    for needle in [
        "async function transferBot(",
        "async function setCloudOnly(",
        "function showTransferReport(",
        "if (subId === 'team') fillTransferBots();",
        "if (pick === 'to-cloud') return transferBot(bot, 'to-cloud');",
        "if (pick === 'from-cloud') return transferBot(bot, 'from-cloud');",
        "if (pick === 'cloud-only') return setCloudOnly(bot, !bot.cloudOnly);",
        "$('#transfer-to-cloud').addEventListener('click'",
    ] {
        assert!(APP.contains(needle), "в app.js нет {needle}");
    }
    for key in [
        "'Перенести в облако'",
        "'Вернуть из облака'",
        "'Только в облаке'",
        "'Перенесено: ботов {bots}, навыков {skills}, команд {rooms}, рутин {routines}.'",
        "'Не смог перенести: {error}'",
    ] {
        assert!(I18N.contains(key), "в словаре EN нет {key}");
    }
}
