//! Что нельзя хранить в памяти как есть: секреты (пароли и коды, номера карт, ключи и токены)
//! и чужие контакты (почты, телефоны, адреса). Здесь они только находятся по тексту; что с ними
//! делать — не писать, спросить человека или замаскировать в эпизоде — решает память.

use std::ops::Range;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Password,
    Card,
    Key,
    Email,
    Phone,
    Address,
}

impl Kind {
    pub fn is_secret(self) -> bool {
        matches!(self, Kind::Password | Kind::Card | Kind::Key)
    }

    /// Что показать вместо скрытого: «[скрыто: пароль]».
    pub fn label(self) -> &'static str {
        match self {
            Kind::Password => "пароль или код",
            Kind::Card => "номер карты",
            Kind::Key => "ключ или токен",
            Kind::Email => "почта",
            Kind::Phone => "телефон",
            Kind::Address => "адрес",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Span {
    /// Байты исходного текста.
    pub range: Range<usize>,
    pub kind: Kind,
}

/// Слова с их местом в тексте. Разделители — пробелы, запятые и точки с запятой.
fn words(text: &str) -> Vec<(Range<usize>, &str)> {
    let mut out = Vec::new();
    let mut start: Option<usize> = None;
    for (i, c) in text.char_indices() {
        let gap = c.is_whitespace() || c == ',' || c == ';';
        match (gap, start) {
            (true, Some(s)) => {
                out.push((s..i, &text[s..i]));
                start = None;
            }
            (false, None) => start = Some(i),
            _ => {}
        }
    }
    if let Some(s) = start {
        out.push((s..text.len(), &text[s..]));
    }
    out
}

const LABELS: [&str; 16] = ["пароль", "password", "passwd", "pass", "пин", "pin", "код", "cvv", "cvc", "otp", "token", "токен", "secret", "секрет", "ключ", "apikey"];
/// Слова между меткой и значением: «пароль от кабинета — Qwerty», «password is hunter2».
const SEPARATORS: [&str; 8] = [":", "—", "–", "-", "=", "is", "это", "был"];

fn is_label(word: &str) -> bool {
    let head = word.split([':', '=']).next().unwrap_or_default().to_lowercase();
    let bare: String = head.chars().filter(|c| c.is_alphanumeric()).collect();
    LABELS.iter().any(|l| bare.strip_prefix(l).is_some_and(|rest| rest.chars().count() <= 2 && !rest.chars().any(char::is_alphabetic)))
}

/// Значение секрета: в нём есть латиница или цифры («пароль простой» — не секрет).
fn looks_like_value(word: &str) -> bool {
    word.chars().any(|c| c.is_ascii_alphanumeric())
}

fn passwords(text: &str, out: &mut Vec<Span>) {
    let list = words(text);
    for (i, (range, word)) in list.iter().enumerate() {
        if !is_label(word) {
            continue;
        }
        // «пароль:qwerty», «pin=0000»: значение в том же слове.
        if let Some((_, value)) = word.split_once([':', '=']) {
            if looks_like_value(value) {
                out.push(Span { range: range.clone(), kind: Kind::Password });
                continue;
            }
        }
        // Значение в ближних словах: с цифрой — сразу, без цифры — только после разделителя.
        let mut separated = word.ends_with(':') || word.ends_with('=');
        for (next_range, next) in list.iter().skip(i + 1).take(5) {
            let bare = next.trim_matches(|c: char| ":=—–-".contains(c));
            if bare.is_empty() || SEPARATORS.contains(&next.to_lowercase().as_str()) {
                separated = true;
                continue;
            }
            if looks_like_value(bare) && (separated || bare.chars().any(|c| c.is_ascii_digit())) {
                out.push(Span { range: next_range.clone(), kind: Kind::Password });
                break;
            }
            if next.ends_with(':') {
                separated = true;
            }
        }
    }
}

fn luhn(digits: &[u32]) -> bool {
    let sum: u32 = digits
        .iter()
        .rev()
        .enumerate()
        .map(|(i, &d)| if i % 2 == 1 { if d * 2 > 9 { d * 2 - 9 } else { d * 2 } } else { d })
        .sum();
    sum % 10 == 0
}

/// Цифры подряд, между которыми только `allowed`: место в тексте и сами цифры.
fn digit_runs(text: &str, allowed: &str) -> Vec<(Range<usize>, Vec<u32>)> {
    let mut out = Vec::new();
    let mut digits: Vec<u32> = Vec::new();
    let mut start = 0;
    let mut end = 0;
    for (i, c) in text.char_indices() {
        if let Some(d) = c.to_digit(10).filter(|_| c.is_ascii_digit()) {
            if digits.is_empty() {
                start = i;
            }
            digits.push(d);
            end = i + 1;
        } else if !allowed.contains(c) && !digits.is_empty() {
            out.push((start..end, std::mem::take(&mut digits)));
        }
    }
    if !digits.is_empty() {
        out.push((start..end, digits));
    }
    out
}

fn cards(text: &str, out: &mut Vec<Span>) {
    for (range, digits) in digit_runs(text, " -") {
        if (13..=19).contains(&digits.len()) && luhn(&digits) {
            out.push(Span { range, kind: Kind::Card });
        }
    }
}

/// Слова, рядом с которыми строка из hex или base64 — почти наверняка ключ.
const KEY_WORDS: [&str; 14] =
    ["key", "api", "token", "client", "secret", "ключ", "токен", "apikey", "api-key", "api_key", "bearer", "client-id", "client_id", "clientid"];
/// Части имени переменной, у которой значение — ключ: `OZON_SELLER_API_KEY=…`.
const KEY_NAME_PARTS: [&str; 6] = ["KEY", "TOKEN", "SECRET", "PASSWORD", "PASSWD", "API"];

fn is_uuid(word: &str) -> bool {
    let parts: Vec<&str> = word.split('-').collect();
    parts.len() == 5
        && parts.iter().zip([8, 4, 4, 4, 12]).all(|(p, n)| p.len() == n && p.chars().all(|c| c.is_ascii_hexdigit()))
}

fn is_hex(word: &str, min: usize) -> bool {
    word.len() >= min && word.chars().all(|c| c.is_ascii_hexdigit()) && word.chars().any(|c| c.is_ascii_digit())
}

fn is_base64(word: &str, min: usize) -> bool {
    word.len() >= min
        && word.chars().all(|c| c.is_ascii_alphanumeric() || "+/=_-".contains(c))
        && word.chars().any(|c| c.is_ascii_digit())
        && word.chars().any(|c| c.is_ascii_alphabetic())
}

fn is_key_word(word: &str) -> bool {
    let bare = word.trim_matches(|c: char| !c.is_alphanumeric()).to_lowercase();
    KEY_WORDS.contains(&bare.as_str())
}

fn keys(text: &str, out: &mut Vec<Span>) {
    let list = words(text);
    for (i, (range, raw)) in list.iter().enumerate() {
        let word = raw.trim_matches(|c: char| "\"'`()[]{}<>:,.".contains(c));
        let start = range.start + raw.find(word).unwrap_or(0);
        let range = start..start + word.len();
        let known = ["sk-", "ghp_", "gho_", "xoxb-", "xoxp-", "AKIA", "eyJ"].iter().any(|p| word.starts_with(p) && word.len() >= 20);
        let mixed = word.len() >= 32 && is_base64(&word.replace('.', ""), 32) && !word.contains("://");
        // Рядом со словом «key», «api», «client», «токен»: hex от 16, base64 от 20 знаков.
        let near = list[i.saturating_sub(3)..i].iter().any(|(_, w)| is_key_word(w) || w.split([':', '=']).any(is_key_word));
        let short = near && (is_hex(word, 16) || is_base64(word, 20));
        if known || mixed || is_uuid(word) || short {
            out.push(Span { range, kind: Kind::Key });
            continue;
        }
        // `OZON_SELLER_API_KEY=abc123…`: скрываем значение, имя оставляем.
        if let Some((name, value)) = word.split_once('=') {
            let upper = name.to_uppercase();
            let value = value.trim_matches(|c: char| c == '"' || c == '\'');
            if KEY_NAME_PARTS.iter().any(|p| upper.contains(p)) && value.chars().count() >= 6 && !value.starts_with("{{secret:") {
                let at = range.start + name.len() + 1;
                out.push(Span { range: at..range.end, kind: Kind::Key });
            }
        }
    }
}

fn emails(text: &str, out: &mut Vec<Span>) {
    for (range, word) in words(text) {
        let w = word.trim_matches(|c: char| "\"'()<>.:".contains(c));
        if w.split_once('@').is_some_and(|(user, host)| !user.is_empty() && host.contains('.') && !host.starts_with('.') && !host.ends_with('.')) {
            out.push(Span { range, kind: Kind::Email });
        }
    }
}

fn phones(text: &str, out: &mut Vec<Span>) {
    for (mut range, digits) in digit_runs(text, " -()+.") {
        if digits.len() >= 10 {
            // Плюс перед номером — часть номера.
            if text[..range.start].ends_with('+') {
                range.start -= 1;
            }
            out.push(Span { range, kind: Kind::Phone });
        }
    }
}

fn addresses(text: &str, out: &mut Vec<Span>) {
    const MARKS: [&str; 10] = ["ул.", "улица", "проспект", "пр-т", "переулок", "шоссе", "кв.", "д.", "street", "avenue"];
    let list = words(text);
    for (i, (range, word)) in list.iter().enumerate() {
        let lower = word.to_lowercase();
        if !MARKS.iter().any(|m| lower == *m || (m.ends_with('.') && lower.starts_with(m))) {
            continue;
        }
        // Адрес — метка и до трёх слов за ней, если среди них есть номер дома.
        let tail: Vec<&(Range<usize>, &str)> = list.iter().skip(i + 1).take(3).collect();
        if let Some(last) = tail.iter().rposition(|(_, w)| w.chars().any(|c| c.is_ascii_digit())) {
            out.push(Span { range: range.start..tail[last].0.end, kind: Kind::Address });
        }
    }
}

/// Всё найденное, без пересечений: секреты важнее контактов.
pub fn spans(text: &str) -> Vec<Span> {
    let mut found = Vec::new();
    passwords(text, &mut found);
    cards(text, &mut found);
    keys(text, &mut found);
    emails(text, &mut found);
    phones(text, &mut found);
    addresses(text, &mut found);
    let mut out: Vec<Span> = Vec::new();
    for span in found {
        if !out.iter().any(|s| s.range.start < span.range.end && span.range.start < s.range.end) {
            out.push(span);
        }
    }
    out.sort_by_key(|s| s.range.start);
    out
}

/// Есть ли секрет: что нашлось.
pub fn secret_in(text: &str) -> Option<&'static str> {
    spans(text).into_iter().find(|s| s.kind.is_secret()).map(|s| s.kind.label())
}

/// Есть ли контакт: почта, телефон или адрес.
pub fn contact_in(text: &str) -> bool {
    spans(text).iter().any(|s| !s.kind.is_secret())
}

/// Скрыть секреты (всегда) и контакты (если `contacts`): «[скрыто: телефон]».
pub fn mask(text: &str, contacts: bool) -> String {
    let mut out = text.to_string();
    for span in spans(text).into_iter().rev() {
        if span.kind.is_secret() || contacts {
            out.replace_range(span.range, &format!("[скрыто: {}]", span.kind.label()));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn secrets_are_recognised_and_plain_talk_is_not() {
        for text in [
            "Пароль от кабинета Ozon: Qwerty123",
            "пароль: qwerty",
            "пароль от кабинета — Qwerty",
            "password is hunter2",
            "код 4821 из смс",
            "pin=0000",
            "Карта 4276 3800 1234 5679",
            "ключ sk-proj-abcdefghijklmnopqrstuvwx",
            "токен ghp_0123456789abcdefghijABCDEFGHIJ",
            "Bearer 9f8e7d6c5b4a39281706f5e4d3c2b1a0ffeeddcc",
        ] {
            assert!(secret_in(text).is_some(), "должен быть секрет: {text}");
        }
        for text in [
            "Пароль от кабинета лежит у Маши",
            "Отчёт Ozon по понедельникам к 10:00",
            "Ключевой клиент — Ромашка",
            "Код ОКВЭД уточнить у бухгалтера",
            "Телефон склада +7 913 123-45-67",
            "Сайт https://example.com/very/long/path/to/some/page/1234567890",
        ] {
            assert!(secret_in(text).is_none(), "не секрет: {text} → {:?}", secret_in(text));
        }
    }

    #[test]
    fn api_key_shapes_are_masked() {
        for (text, value) in [
            ("Client ID 131578\n923ce8f1-952f-4060-9ce7-b6c07e787674", "923ce8f1-952f-4060-9ce7-b6c07e787674"),
            ("api key: 9f8e7d6c5b4a3921", "9f8e7d6c5b4a3921"),
            ("Client-Id: 77 token Zm9vYmFyYmF6cXV4MTIzNDU2", "Zm9vYmFyYmF6cXV4MTIzNDU2"),
            ("OZON_SELLER_API_KEY=abc123def", "abc123def"),
            ("export WB_TOKEN=\"x9y8z7w6v5\"", "x9y8z7w6v5"),
            ("ключ: a1b2c3d4e5f6a7b8c9d0", "a1b2c3d4e5f6a7b8c9d0"),
        ] {
            let masked = mask(text, false);
            assert!(!masked.contains(value), "{text} → {masked}");
            assert!(masked.contains("[скрыто: "), "{text} → {masked}");
        }
        assert_eq!(mask("OZON_SELLER_API_KEY={{secret:OZON_SELLER_API_KEY}}", false), "OZON_SELLER_API_KEY={{secret:OZON_SELLER_API_KEY}}");
        for plain in ["коммит 9f8e7d6c5b4a3921 в main", "заказ 0f0e0d0c1111 отгружен", "DEBUG=true", "API работает, ответ 200"] {
            assert_eq!(mask(plain, false), plain, "не ключ: {plain}");
        }
    }

    #[test]
    fn contacts_are_found_and_masked_in_place() {
        assert!(contact_in("Склад: Новосибирск, ул. Ленина 5") && contact_in("звонить +7 (913) 123-45-67") && contact_in("masha@example.com"));
        assert!(!contact_in("отчёт к 10:00 по понедельникам"));
        let text = "Оля: +7 913 123-45-67, olya@firm.ru; пароль от кабинета — Qwerty; склад ул. Ленина 5, кв. 3";
        let masked = mask(text, true);
        assert_eq!(masked, "Оля: [скрыто: телефон], [скрыто: почта]; пароль от кабинета — [скрыто: пароль или код]; склад [скрыто: адрес], [скрыто: адрес]");
        assert!(mask(text, false).contains("+7 913 123-45-67") && !mask(text, false).contains("Qwerty"), "секреты скрыты всегда, контакты — по просьбе");
    }
}
