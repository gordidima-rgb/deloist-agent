//! Поиск по словам для памяти: BM25 по основам слов (стеммер Snowball, русский и английский).
//!
//! Модель здесь не нужна: поиск одинаков для любого ИИ под капотом. Документ — несколько полей
//! с весами (заголовок важнее текста), запрос — обычная фраза.

use std::collections::HashMap;

use rust_stemmers::{Algorithm, Stemmer};

const K1: f64 = 1.2;
const B: f64 = 0.75;

/// Служебные слова: совпадение по «по» или «the» ничего не говорит о смысле.
const STOP: &[&str] = &[
    "по", "на", "за", "из", "от", "до", "для", "при", "про", "над", "под", "без", "не", "ни", "но", "да", "же", "ли",
    "бы", "то", "что", "как", "это", "так", "или", "мне", "мы", "он", "она", "они", "его", "ее", "их", "все", "уже",
    "the", "and", "of", "to", "in", "on", "for", "is", "are", "at", "by", "or", "an", "it", "be", "with", "from",
];

fn is_cyrillic(c: char) -> bool {
    matches!(c, '\u{0400}'..='\u{04FF}')
}

/// Основы слов: нижний регистр, «ё» как «е», слова от двух знаков.
pub fn tokens(text: &str) -> Vec<String> {
    let ru = Stemmer::create(Algorithm::Russian);
    let en = Stemmer::create(Algorithm::English);
    text.to_lowercase()
        .replace('ё', "е")
        .split(|c: char| !c.is_alphanumeric())
        .filter(|w| w.chars().count() >= 2 && !STOP.contains(w))
        .map(|w| if w.chars().any(is_cyrillic) { ru.stem(w).to_string() } else { en.stem(w).to_string() })
        .collect()
}

/// Ранжировать документы по запросу. Документ — поля `(текст, вес)`. Ответ — номера документов
/// с ненулевой оценкой, лучшие первыми; при равной оценке — в исходном порядке.
pub fn rank(docs: &[Vec<(&str, f64)>], query: &str) -> Vec<(usize, f64)> {
    let mut wanted = tokens(query);
    wanted.sort();
    wanted.dedup();
    if wanted.is_empty() || docs.is_empty() {
        return Vec::new();
    }
    let counted: Vec<(HashMap<String, f64>, f64)> = docs
        .iter()
        .map(|fields| {
            let mut tf: HashMap<String, f64> = HashMap::new();
            let mut len = 0.0;
            for (text, weight) in fields {
                for token in tokens(text) {
                    *tf.entry(token).or_default() += weight;
                    len += weight;
                }
            }
            (tf, len)
        })
        .collect();
    let n = docs.len() as f64;
    let average = (counted.iter().map(|(_, len)| len).sum::<f64>() / n).max(1.0);
    let idf: HashMap<&String, f64> = wanted
        .iter()
        .map(|term| {
            let with = counted.iter().filter(|(tf, _)| tf.contains_key(term)).count() as f64;
            (term, (1.0 + (n - with + 0.5) / (with + 0.5)).ln())
        })
        .collect();
    let mut out: Vec<(usize, f64)> = counted
        .iter()
        .enumerate()
        .map(|(i, (tf, len))| {
            let score = wanted
                .iter()
                .filter_map(|term| tf.get(term).map(|f| idf[term] * f * (K1 + 1.0) / (f + K1 * (1.0 - B + B * len / average))))
                .sum::<f64>();
            (i, score)
        })
        .filter(|(_, score)| *score > 0.0)
        .collect();
    out.sort_by(|a, b| b.1.total_cmp(&a.1).then(a.0.cmp(&b.0)));
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn word_forms_meet_at_one_stem() {
        assert_eq!(tokens("Отчётов"), tokens("отчет"));
        assert_eq!(tokens("кабинеты"), tokens("Кабинет"));
        assert_eq!(tokens("reports"), tokens("report"));
        assert_eq!(tokens("WB и Ozon"), vec!["wb", "ozon"], "слова из одной буквы не берём");
    }

    #[test]
    fn titles_outweigh_bodies_and_rare_words_outweigh_common() {
        let docs = vec![
            vec![("Кофе по утрам", 2.0), ("", 1.0)],
            vec![("Сводка продаж", 2.0), ("в отчётах Ozon по понедельникам", 1.0)],
            vec![("Отчёты Ozon", 2.0), ("каждый понедельник", 1.0)],
            vec![("Нет совпадений", 2.0), ("", 1.0)],
        ];
        let hits = rank(&docs, "отчёт по Ozon");
        assert_eq!(hits.iter().map(|h| h.0).collect::<Vec<_>>(), vec![2, 1], "{hits:?}");
        assert!(rank(&docs, "чай").is_empty());
        assert!(rank(&docs, "  ").is_empty());
    }
}
