//! Подсказки по сайтам: короткие приёмы, которые экономят боту десятки кликов.
//!
//! Подсказка приходит вместе со снимком страницы, когда бот заходит на сайт, — так её получают
//! и боты на API, и агенты ACP через мост, без лишнего текста в каждом системном промпте.

use std::sync::LazyLock;

/// Хост сайта (и его поддомены) → подсказка.
const TIPS: &[(&str, &str)] = &[
    (
        "x.com",
        "X: ищи прямой ссылкой x.com/search?q=<запрос>&f=live (свежие) или &f=top (популярные), без кликов по полю поиска. \
         Операторы в q: from:имя, to:имя, since:2026-11-01, until:2026-11-13, lang:ru, min_faves:50, filter:links, \
         -filter:replies, \"точная фраза\", слово OR слово. В снимке у поста — строка со всем текстом и ссылка с датой \
         (…/status/<номер>) — это ссылка на пост. Видны только посты на экране: листай browser_mouse wheel dy=2500 и смотри \
         snapshot с search=<слово>. Лайк, репост, подписка, ответ и пост — только с разрешения человека; горячие клавиши \
         (l, t, r, n) не жми. Экран входа — browser_human.",
    ),
    (
        "twitter.com",
        "X: сайт переехал на x.com — открывай x.com/search?q=<запрос>&f=live.",
    ),
    (
        "web.whatsapp.com",
        "WhatsApp Web: чат с новым номером открывай ссылкой web.whatsapp.com/send?phone=<цифры с кодом страны, без + и пробелов>, \
         можно с &text=<текст> — текст встанет в поле, но не уйдёт; это быстрее поиска. Если номера нет в WhatsApp, \
         появится окно о недействительном номере: закрой его и переходи к следующему. Уже знакомый чат быстрее найти \
         в списке: snapshot с search=<имя>. Поле сообщения внизу чата; перевод строки в type станет новой строкой, \
         а отправка — Enter или кнопка «Отправить» — спрашивает человека. Непрочитанные чаты помечены числом. \
         Экран с QR-кодом — browser_human: входит человек.",
    ),
    (
        "web.telegram.org",
        "Telegram Web: открытый канал быстрее читать без браузера — telegram_posts. Чаты, группы и закрытые \
         каналы он не читает: их сразу открывай ссылкой web.telegram.org/k/#@имя, без пробного вызова. Поиск по чату — \
         лупа в шапке чата, введи запрос, затем один вызов browser_page telegram_results: он соберёт всю выдачу \
         (автор, время, текст), сам пролистает её, и открывать сообщения по одному не нужно. \
         Код на странице (browser_devtools eval) для этого не нужен: он спрашивает разрешение человека. \
         Вступить, подписаться и написать — только с разрешения человека. Экран с QR-кодом — browser_human.",
    ),
    (
        "youtube.com",
        "YouTube: ищи прямой ссылкой youtube.com/results?search_query=<запрос>; сортировка и длительность — кнопка «Фильтры». \
         В снимке у видео — ссылка с названием (…/watch?v=…) и строка «просмотры • когда». Описание видео раскрывает «…ещё»; \
         расшифровка — кнопка «Показать текст видео» в описании. Комментарии подгружаются при прокрутке вниз. \
         Подписка, лайк и комментарий — только с разрешения человека. Вход — browser_human.",
    ),
    (
        "reddit.com",
        "Reddit: обсуждения по теме быстрее найти поиском в интернете (site:reddit.com <запрос>) и открыть нужную ветку. \
         Поиск по сабреддиту — reddit.com/r/<имя>/search/?q=<запрос>&restrict_sr=1&sort=new. Экран «Prove your humanity» — \
         browser_human, сам не проходи. Голоса, комментарии и вступление в сообщество — только с разрешения человека.",
    ),
    (
        "discord.com",
        "Discord: читается только после входа — экран входа или QR-код отдай человеку (browser_human). Сервер выбирается слева, \
         каналы — в колонке рядом. Поиск по серверу — поле «Поиск» справа вверху, операторы from:имя, in:канал, has:link, \
         before:2026-11-01, after:. Сообщения грузятся при прокрутке вверх. Написать, отреагировать и войти на сервер — \
         только с разрешения человека.",
    ),
    (
        "instagram.com",
        "Instagram: без входа почти ничего не видно — экран входа отдай человеку (browser_human). Профиль — instagram.com/<имя>/, \
         пост — instagram.com/p/<код>/, хэштег — instagram.com/explore/tags/<тег>/. Подпись поста раскрывает «ещё». \
         Лайк, подписка, комментарий и сообщение — только с разрешения человека.",
    ),
    (
        "facebook.com",
        "Facebook: нужен вход — экран входа отдай человеку (browser_human). Поиск прямой ссылкой: facebook.com/search/posts?q=<запрос>, \
         …/search/groups?q=, …/search/pages?q=. Ленту листай колёсиком и снова смотри snapshot. Лайк, комментарий, заявка в друзья, \
         вступление в группу и сообщение — только с разрешения человека.",
    ),
    (
        "vk.com",
        "VK переехал на vk.ru: открывай vk.ru/<имя сообщества>. Многое видно только после входа — экран входа отдай человеку \
         (browser_human). Поиск — поле «Поиск» вверху, затем вкладки разделов. Лайк, репост, комментарий, подписка, \
         заявка в друзья и сообщение — только с разрешения человека.",
    ),
    (
        "vk.ru",
        "VK: многое видно только после входа — экран входа отдай человеку (browser_human). Сообщество — vk.ru/<имя>; поиск — \
         поле «Поиск» вверху, затем вкладки разделов. Лайк, репост, комментарий, подписка, заявка в друзья и сообщение — \
         только с разрешения человека.",
    ),
    (
        "rutube.ru",
        "Rutube: поиск — rutube.ru/search/?query=<запрос>. Из-за границы сайт бывает недоступен (отказ в подключении): \
         после одной неудачи не повторяй, скажи человеку и ищи видео поиском в интернете. Подписка, лайк и комментарий — \
         только с разрешения человека.",
    ),
    (
        "dzen.ru",
        "Дзен: поиск — dzen.ru/search?query=<запрос>; статьи открываются по ссылкам dzen.ru/a/…, читать можно без входа. \
         Подписка, лайк и комментарий — только с разрешения человека.",
    ),
    (
        "avito.ru",
        "Авито: поиск — avito.ru/<город латиницей>?q=<запрос> (например avito.ru/moskva?q=…), фильтры и сортировка — на странице выдачи. \
         Авито не пускает с зарубежных адресов, VPN и при частых запросах («Страница не загрузилась из-за проблем с IP»): \
         не обновляй страницу снова и снова — отдай проверку человеку (browser_human) или скажи, что сайт недоступен. \
         Написать продавцу, показать телефон и купить — только с разрешения человека.",
    ),
];

fn host(url: &str) -> Option<String> {
    reqwest::Url::parse(url).ok()?.host_str().map(|h| h.trim_start_matches("www.").to_lowercase())
}

/// Сайты, где бот пишет людям: к их подсказкам добавляется правило про адресата.
const MESSENGER_HOSTS: &[&str] = &["web.whatsapp.com", "web.telegram.org", "discord.com", "vk.com", "vk.ru", "instagram.com", "facebook.com"];

/// Кому писать, когда человека назвали словом, а не именем («жене», «маме», «начальнику»).
/// Угаданный адресат уходит со сообщением насовсем, поэтому его сверяют один раз и запоминают.
const RECIPIENT_RULE: &str = "Адресат: если человека назвали словом («жена», «мама», «начальник»), а не именем, сначала recall: нет ли \
     уже подтверждённого контакта. Нет — найденный чат не угадывай: до отправки спроси человека (ask_user), \
     показав имя, ник или номер найденного чата и кого ты в нём видишь. После «да» запиши remember (kind reference, \
     entities — имя человека): «<кто> в <мессенджер>: имя, ник или номер, подтверждено <дата>» — в общую память, \
     чтобы все боты знали это точно. Если подтверждённый контакт в памяти есть — отправляй без вопроса. \
     Человек поправил — исправь ту же запись по её номеру.";

/// Подсказки с добавленным правилом для мессенджеров. Склеиваем один раз и держим до конца работы.
static FULL_TIPS: LazyLock<Vec<(&'static str, &'static str)>> = LazyLock::new(|| {
    TIPS.iter()
        .map(|(site, tip)| {
            if MESSENGER_HOSTS.contains(site) {
                let joined: &'static str = Box::leak(format!("{tip} {RECIPIENT_RULE}").into_boxed_str());
                (*site, joined)
            } else {
                (*site, *tip)
            }
        })
        .collect()
});

/// Подсказка для адреса: точный хост или его поддомен (`mobile.x.com` → x.com).
pub fn for_url(url: &str) -> Option<&'static str> {
    let host = host(url)?;
    FULL_TIPS.iter().find(|(site, _)| host == *site || host.ends_with(&format!(".{site}"))).map(|(_, tip)| *tip)
}

/// Подсказку даём при переходе на другой сайт, а не на каждой странице того же.
pub fn on_arrival(from: &str, to: &str) -> Option<&'static str> {
    let tip = for_url(to)?;
    if for_url(from) == Some(tip) {
        None
    } else {
        Some(tip)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tips_match_the_site_and_its_subdomains_only() {
        assert!(for_url("https://x.com/search?q=godot").unwrap().contains("f=live"));
        assert!(for_url("https://mobile.x.com/home").is_some());
        assert!(for_url("https://www.x.com/").is_some());
        assert!(for_url("https://notx.com/").is_none(), "чужой сайт с похожим концом имени");
        assert!(for_url("https://web.whatsapp.com/").unwrap().contains("send?phone="));
        let telegram = for_url("https://web.telegram.org/k/").unwrap();
        assert!(telegram.contains("telegram_posts"));
        assert!(telegram.contains("закрытые"), "бот не должен пробовать telegram_posts на чате");
        assert!(telegram.contains("telegram_results") && telegram.contains("eval"), "выдачу читают снимком, а не кодом с карточкой");
        assert!(for_url("about:blank").is_none());
        for (url, word) in [
            ("https://www.youtube.com/watch?v=x", "search_query"),
            ("https://m.youtube.com/", "search_query"),
            ("https://www.reddit.com/r/godot/", "site:reddit.com"),
            ("https://discord.com/channels/@me", "from:"),
            ("https://www.instagram.com/godotengine/", "explore/tags"),
            ("https://www.facebook.com/godotengine", "search/posts"),
            ("https://vk.com/godot", "vk.ru"),
            ("https://vk.ru/godot", "vk.ru/<имя>"),
            ("https://rutube.ru/search/?query=a", "rutube.ru/search"),
            ("https://dzen.ru/", "dzen.ru/search"),
            ("https://www.avito.ru/moskva?q=a", "проблем с IP"),
        ] {
            assert!(for_url(url).unwrap_or_default().contains(word), "{url}");
        }
    }

    #[test]
    fn a_tip_comes_once_per_site_visit() {
        assert!(on_arrival("about:blank", "https://x.com/home").is_some());
        assert!(on_arrival("https://x.com/home", "https://x.com/search?q=a").is_none());
        assert!(on_arrival("https://x.com/home", "https://web.whatsapp.com/").is_some());
        assert!(on_arrival("https://x.com/home", "https://example.com/").is_none());
    }

    #[test]
    fn messengers_ask_who_the_recipient_is_and_remember_the_answer() {
        for url in [
            "https://web.telegram.org/k/",
            "https://web.whatsapp.com/",
            "https://discord.com/channels/@me",
            "https://vk.ru/im",
            "https://www.instagram.com/direct/inbox/",
            "https://www.facebook.com/messages/",
        ] {
            let tip = for_url(url).unwrap_or_else(|| panic!("{url}: нет подсказки"));
            assert!(tip.contains("ask_user") && tip.contains("remember") && tip.contains("recall"), "{url}: правило про адресата");
            assert!(tip.chars().count() <= 1700, "{url}: подсказка длинная");
        }
        for url in ["https://x.com/home", "https://www.youtube.com/", "https://www.avito.ru/"] {
            assert!(!for_url(url).unwrap().contains("Адресат:"), "{url}: правило только для мессенджеров");
        }
    }

    #[test]
    fn tips_never_tell_the_bot_to_act_without_the_human() {
        for (site, tip) in TIPS {
            assert!(tip.chars().count() <= 900, "{site}: подсказка длинная");
            if tip.contains("Enter") || tip.contains("Лайк") || tip.contains("Вступить") {
                assert!(tip.contains("человек"), "{site}: про разрешение человека");
            }
        }
    }
}
