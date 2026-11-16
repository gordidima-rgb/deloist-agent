//! Окно разговора бота по его модели: сколько знаков истории отдавать модели до сжатия.
//!
//! Раньше окно было одно на всех — 120 тысяч знаков (около 40 тысяч токенов). Для программирования
//! этого мало, а у современных моделей окно от 128 тысяч до миллиона токенов. Теперь окно известной
//! модели берётся из её настоящего размера, а человек выбирает у бота, какую долю окна занимать:
//! «Обычная» (как раньше, по умолчанию), «Большая» или «Максимум модели». Большее окно дороже
//! на каждом ходе, поэтому по умолчанию остаётся прежний размер.
//!
//! С 14.11.2026 вместо трёх режимов - ползунок «Сжимать при N% лимита» (поле бота `compactAtPercent`,
//! 20..95 с шагом 5), как `/autocompact` в Claude Code. У бота без этого поля работает прежний режим
//! `contextMode`, пока человек не тронет ползунок: сохранённое не теряется и ничего не меняется молча.
//!
//! Откуда числа окон (перепроверено по официальным страницам 14.11.2026, правок чисел не потребовалось, добавлен Gemini 4):
//! - Claude: таблица моделей Anthropic — 1M у Fable/Mythos 5.x, Opus 5.x и 4.6–4.8, Sonnet 5.x и 4.6, Haiku 5.x;
//!   200K у Haiku 4.5, Sonnet 4.5, Opus 4.5 и старше.
//! - Gemini: aistudio.google.com/docs/models (бывший ai.google.dev) — 1 048 576 входных токенов у gemini-2.5-pro и gemini-3.8-flash;
//!   у трёх моделей Gemini 4 страница называет 1M токенов одной цифрой, берём 1 000 000.
//! - DeepSeek: api-docs.deepseek.com/quick_start/pricing — 1M у `deepseek-flash` и `deepseek-v4-pro`.
//! - GPT-6.1 Terra (подписка Deloist): developers.openai.com/api/docs/models/gpt-6.1-terra — окно 1 050 000, вход до 922 000,
//!   выход до 128 000: запас под ответ (`reserve_tokens`) не меньше 130 000, чтобы история не перешла 922 000.
//! - GigaChat 2 Lite/Pro/Max: developers.sber.ru/docs/ru/gigachat/models/updates — 128 тыс.
//! - Yandex AI Studio: aistudio.yandex.ru/docs/en/ai-studio/concepts/generation/models — Alice AI LLM 131 072,
//!   Flash 65 536, YandexGPT Pro и Lite по 32 768.
//! - Ollama: docs.ollama.com/context-length — по умолчанию 4k токенов (24 ГиБ видеопамяти — 32k, 48 и больше — 256k),
//!   меняется `OLLAMA_CONTEXT_LENGTH` или `num_ctx` в Modelfile. Через `/v1` запрос окно не меняет.

use std::collections::HashMap;
use std::sync::Arc;

use axum::extract::{Query, State};
use axum::routing::get;
use axum::{Json, Router};
use serde_json::{json, Value};

use crate::store::Bot;
use crate::App;

/// Окно неизвестной модели: около 40 тысяч токенов, влезает почти везде.
pub const DEFAULT_TOKENS: usize = 40_000;
/// Окно «Обычная» для моделей с большим окном: как было всегда.
pub const BASE_CHARS: usize = 120_000;
/// Осторожное число знаков на токен для смеси русского и английского.
pub const CHARS_PER_TOKEN: usize = 3;
/// Сколько токенов Ollama берёт по умолчанию, если окно не задано ни в Modelfile, ни на сервере.
pub const OLLAMA_DEFAULT_TOKENS: usize = 4_096;

/// Сколько памяти разговора человек выбрал у бота.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Memory {
    /// Как раньше: около 40 тысяч токенов.
    #[default]
    Normal,
    /// Для программирования: половина окна модели.
    Large,
    /// Почти всё окно модели.
    Max,
}

impl Memory {
    pub const ALL: [Memory; 3] = [Memory::Normal, Memory::Large, Memory::Max];

    pub fn parse(text: &str) -> Self {
        match text {
            "large" => Memory::Large,
            "max" => Memory::Max,
            _ => Memory::Normal,
        }
    }

    pub fn key(self) -> &'static str {
        match self {
            Memory::Normal => "normal",
            Memory::Large => "large",
            Memory::Max => "max",
        }
    }

    /// Доля окна модели в процентах. Остальное — системный промпт, инструменты, память и ответ.
    fn share(self) -> usize {
        match self {
            Memory::Normal => 60,
            Memory::Large => 50,
            Memory::Max => 80,
        }
    }
}

/// Границы и шаг ползунка «Сжимать при N% лимита».
pub const MIN_PERCENT: u8 = 20;
pub const MAX_PERCENT: u8 = 95;
pub const STEP_PERCENT: u8 = 5;

/// Привести число с ползунка к допустимому: в границы и к шагу.
pub fn clamp_percent(raw: i64) -> u8 {
    let step = STEP_PERCENT as i64;
    let snapped = raw.clamp(-1_000, 1_000).saturating_add(step / 2).div_euclid(step) * step;
    snapped.clamp(MIN_PERCENT as i64, MAX_PERCENT as i64) as u8
}

/// Порог бота в процентах, если человек его задавал. Мусор в поле (ручная правка state.json) - как «не задан».
pub fn percent_of(bot: &Bot) -> Option<u8> {
    let raw = bot.extra.get("compactAtPercent")?.as_i64()?;
    Some(clamp_percent(raw))
}

/// Запас окна под системный промпт, инструменты и ответ: десятая часть, но не меньше 8 тысяч токенов
/// (у маленьких окон - не больше четверти). У окон от полумиллиона - не меньше 130 тысяч: ответ таких
/// моделей бывает до 128 тысяч, и вход плюс ответ не должны переходить окно.
pub fn reserve_tokens(window: usize) -> usize {
    let floor = if window >= 500_000 { 130_000 } else { 8_000.min(window / 4) };
    (window / 10).max(floor)
}

/// Сколько токенов истории можно держать в окне, не мешая системному промпту и ответу.
pub fn usable_tokens(window: usize) -> usize {
    window.saturating_sub(reserve_tokens(window))
}

/// Токены, при которых история сожмётся при пороге `percent`. Окно неизвестно - считаем от 40 тысяч токенов.
/// Выше `usable_tokens` порог не поднимается, сколько бы ни показывал ползунок.
pub fn threshold_tokens(window: Option<usize>, percent: u8) -> usize {
    let window = window.unwrap_or(DEFAULT_TOKENS);
    let share = (window as u128 * percent.clamp(MIN_PERCENT, MAX_PERCENT) as u128 / 100) as usize;
    share.min(usable_tokens(window))
}

/// Тот же порог в знаках.
pub fn threshold_chars(window: Option<usize>, percent: u8) -> usize {
    threshold_tokens(window, percent).saturating_mul(CHARS_PER_TOKEN)
}

/// Порог бота в процентах для показа: заданный, а у бота со старым режимом - равный ему.
pub fn effective_percent(bot: &Bot, window: Option<usize>) -> u8 {
    percent_of(bot).unwrap_or_else(|| {
        let tokens = approx_tokens(budget_chars(window, mode_of(bot)));
        let of_window = tokens * 100 / window.unwrap_or(DEFAULT_TOKENS).max(1);
        clamp_percent(of_window as i64)
    })
}

/// Режим бота. Нет поля или непонятное значение — «Обычная»: у старых ботов поля нет.
pub fn mode_of(bot: &Bot) -> Memory {
    Memory::parse(bot.extra.get("contextMode").and_then(Value::as_str).unwrap_or_default())
}

/// Сколько знаков истории отдать модели до сжатия.
///
/// Окно неизвестно — прежние 120 тысяч знаков в любом режиме: расти не от чего.
/// «Обычная» — не больше прежних 120 тысяч и не больше 60% окна: у модели с маленьким окном
/// (например, Ollama с 4k) иначе промпт не поместился бы и сервер молча обрезал бы его начало.
/// «Большая» и «Максимум» — своя доля окна, но не меньше «Обычной».
pub fn budget_chars(window: Option<usize>, mode: Memory) -> usize {
    let Some(window) = window else { return BASE_CHARS };
    let by_share = |percent: usize| window * percent * CHARS_PER_TOKEN / 100;
    let normal = BASE_CHARS.min(by_share(Memory::Normal.share()));
    match mode {
        Memory::Normal => normal,
        _ => normal.max(by_share(mode.share())),
    }
}

/// Токены по знакам, с тем же осторожным отношением.
pub fn approx_tokens(chars: usize) -> usize {
    chars / CHARS_PER_TOKEN
}

/// Окно модели из ответа `/models`: у каждого провайдера своё имя поля.
pub fn from_model_json(model: &Value) -> Option<usize> {
    [
        "context_length",
        "context_window",
        "max_context_length",
        "input_token_limit",
        "inputTokenLimit",
    ]
    .iter()
    .filter_map(|key| model.get(*key).and_then(Value::as_u64))
    .find(|n| *n >= 1_000)
    .map(|n| n as usize)
}

/// Окно локальной модели Ollama в токенах: то, что сервер действительно загрузит.
/// `show` — ответ `/api/show`, `running` — `context_length` из `/api/ps`, если модель уже загружена.
///
/// Берём заданное в Modelfile `num_ctx` (или уже загруженное), а если его нет — 4096 по умолчанию Ollama,
/// и не больше, чем модель умеет сама (`<архитектура>.context_length`).
pub fn ollama_window(show: &Value, running: Option<usize>) -> usize {
    let architecture_max = show["model_info"]
        .as_object()
        .into_iter()
        .flatten()
        .filter(|(key, _)| key.ends_with(".context_length"))
        .filter_map(|(_, value)| value.as_u64())
        .max()
        .map(|n| n as usize);
    let configured = show["parameters"].as_str().and_then(|text| {
        text.lines().find_map(|line| {
            let mut words = line.split_whitespace();
            (words.next() == Some("num_ctx")).then(|| words.next()?.parse::<usize>().ok()).flatten()
        })
    });
    let effective = running.filter(|n| *n > 0).or(configured).unwrap_or(OLLAMA_DEFAULT_TOKENS);
    architecture_max.map_or(effective, |max| effective.min(max))
}

/// Известное окно модели по её имени, в токенах. Для неизвестных — `None`, и работает прежнее окно.
pub fn known_tokens(model: &str) -> Option<usize> {
    let lowered = model.trim().to_lowercase();
    // Yandex зовёт модель `gpt://каталог/модель/latest`: узнаём её по имени внутри адреса.
    if lowered.contains("aliceai-llm-flash") {
        return Some(65_536);
    }
    if lowered.contains("aliceai-llm") {
        return Some(131_072);
    }
    if lowered.contains("yandexgpt") {
        return Some(32_768);
    }
    // Префикс агрегатора (`anthropic/claude-…`) не мешает узнать модель.
    let id = lowered.rsplit('/').next().unwrap_or_default();

    const MILLION: usize = 1_000_000;
    match id {
        "deepseek-flash" | "deepseek-v4-flash" | "deepseek-v4-pro" => return Some(MILLION),
        // Старые имена API DeepSeek. На странице моделей их уже нет; берём последнее опубликованное значение.
        "deepseek-chat" | "deepseek-reasoner" => return Some(128_000),
        "gpt-6.1-terra" => return Some(1_050_000),
        // xAI, docs.x.ai/developers/models (16.11.2026): `maxPromptLength` 500 000 у 4.5/4.6/4.7, 256 000 у grok-build-0.1.
        // У `grok-4.7-build-fast` (Grok Build) на странице записи нет — окно не выдумываем.
        "grok-4.5" | "grok-4.6" | "grok-4.7" => return Some(500_000),
        "grok-build-0.1" => return Some(256_000),
        _ => {}
    }
    if id.starts_with("gigachat-2") {
        return Some(128_000);
    }
    if let Some(rest) = id.strip_prefix("claude-") {
        const BIG: [&str; 9] = [
            "fable-5", "mythos-5", "opus-5", "sonnet-5", "haiku-5", "opus-4-6", "opus-4-7", "opus-4-8", "sonnet-4-6",
        ];
        const SMALL: [&str; 9] = [
            "haiku-4-5", "sonnet-4-5", "opus-4-5", "opus-4-1", "opus-4-0", "sonnet-4-0", "opus-4-2", "sonnet-4-2", "3",
        ];
        if BIG.iter().any(|p| rest.starts_with(p)) {
            return Some(MILLION);
        }
        if SMALL.iter().any(|p| rest.starts_with(p)) {
            return Some(200_000);
        }
        return None;
    }
    if id.starts_with("gemini-4") {
        return Some(MILLION);
    }
    if id.starts_with("gemini-2") || id.starts_with("gemini-3") {
        // У картиночных, голосовых и поисковых моделей окно другое.
        if ["image", "tts", "live", "audio", "embedding", "robotics", "computer-use"].iter().any(|w| id.contains(w)) {
            return None;
        }
        return Some(1_048_576);
    }
    None
}

/// Окно моделей, которые чаще всего стоят за ACP-агентами (Claude, ChatGPT), по id модели или её названию.
/// Агент, который сам назвал размер окна (`usage_update`), главнее. Для моделей с документированным окном
/// число берётся из `known_tokens` — одна таблица на всю программу, своих чисел здесь нет (13.11.2026:
/// отдельная цифра 400 000 для gpt-6.1-terra разошлась с документацией, где 1 050 000).
pub fn acp_window_table(model: &str) -> Option<usize> {
    let m = model.to_lowercase();
    // Claude Code по умолчанию держит окно 200 тыс.; вариант модели с суффиксом [1m] — миллион.
    if ["haiku", "sonnet", "opus"].iter().any(|w| m.contains(w)) {
        return Some(if m.contains("[1m]") || m.ends_with("-1m") { 1_000_000 } else { 200_000 });
    }
    if let Some(tokens) = known_tokens(&m) {
        return Some(tokens);
    }
    // GPT-5.x в API — 400 тыс. (developers.openai.com); новые GPT без записи в known_tokens сюда не попадают.
    if m.contains("gpt-5") {
        return Some(400_000);
    }
    None
}

/// Заполненность контекста ACP-агента: `(занято токенов, окно, источник)`. `chars` — оценка по истории, если агент молчит.
pub fn acp_usage(bot_id: &str, model: &str, chars: usize) -> Option<(usize, usize, &'static str)> {
    if let Some((used, size)) = crate::drivers::acp::reported_usage(bot_id) {
        return Some((used as usize, size as usize, "agent"));
    }
    acp_window_table(model).map(|w| (approx_tokens(chars), w, "table"))
}

/// Окно в токенах для драйвера и модели, если оно известно.
/// Внешние агенты и программы (ACP, CLI) ведут свой контекст сами: для них окна нет.
pub fn window_of(app: &App, driver: &str, model: &str) -> Option<usize> {
    if driver == "acp" || driver == "cli" || driver == "demo" || driver.starts_with("acp:") {
        return None;
    }
    if let Some(provider) = driver.strip_prefix("openai:") {
        if let Some(tokens) = app.providers.model_context(app, provider, model) {
            return Some(tokens);
        }
        // Ollama, у которого окно ещё не прочитали: берём то, что он даёт сам по умолчанию.
        if provider == "ollama" {
            return Some(OLLAMA_DEFAULT_TOKENS);
        }
    }
    known_tokens(model)
}

/// Окно бота в токенах: по тому драйверу и той модели, на которых он сейчас отвечает.
pub fn window_for(app: &App, bot: &Bot) -> Option<usize> {
    let settings = app.store.settings();
    let choice = crate::turn::main_choice(app, bot, &settings);
    if choice.driver == "acp" || choice.driver.starts_with("acp:") {
        return crate::drivers::acp::reported_usage(&bot.id).map(|(_, size)| size as usize).or_else(|| acp_window_table(&choice.model));
    }
    window_of(app, &choice.driver, &choice.model)
}

/// Бюджет окна бота в знаках. В отладочной сборке `CREW_TEST_CONTEXT_CHARS` задаёт его для тестов.
pub fn budget_for(app: &App, bot: &Bot) -> usize {
    if let Some(chars) = test_override() {
        return chars;
    }
    bot_budget(window_for(app, bot), bot)
}

/// Бюджет по окну и полям бота: порог в процентах главнее старого режима.
pub fn bot_budget(window: Option<usize>, bot: &Bot) -> usize {
    match percent_of(bot) {
        Some(percent) => threshold_chars(window, percent),
        None => budget_chars(window, mode_of(bot)),
    }
}

fn test_override() -> Option<usize> {
    if cfg!(debug_assertions) {
        return std::env::var("CREW_TEST_CONTEXT_CHARS").ok().and_then(|v| v.parse().ok());
    }
    None
}

/// Окно и режим для сжатия: бюджет в знаках и можно ли не смотреть на счётчик записей.
///
/// В группе окно по самому маленькому из ботов: сводка у всех общая, и никому не должно не хватить места.
/// Счётчик записей (`history_limit`, около 90 записей) для «Большой» и «Максимума» не применяется:
/// он обрезал бы такое окно раньше размера. Размер окна там задаёт бюджет в знаках.
pub fn limits_for(app: &App, bots: &[Bot]) -> (usize, bool) {
    let budget = bots.iter().map(|bot| budget_for(app, bot)).min().unwrap_or(BASE_CHARS);
    let ignore_count = !bots.is_empty() && bots.iter().all(|bot| percent_of(bot).is_some() || mode_of(bot) != Memory::Normal);
    (budget, ignore_count)
}

/// Что показать в настройках бота: окно выбранной модели, порог и сколько токенов он даёт.
/// `percent` - порог для ползунка, `stored` - задан ли он человеком (иначе работает старый режим
/// и `currentTokens` - его размер), `usable` - потолок истории в токенах. Окно берётся только отсюда:
/// интерфейс своих чисел окон не держит.
pub fn describe(window: Option<usize>, bot: &Bot) -> Value {
    let stored = percent_of(bot);
    let percent = effective_percent(bot, window);
    let current = approx_tokens(bot_budget(window, bot));
    let shown_window = window.unwrap_or(DEFAULT_TOKENS);
    json!({
        "mode": mode_of(bot).key(),
        "percent": percent,
        "stored": stored.is_some(),
        "min": MIN_PERCENT,
        "max": MAX_PERCENT,
        "step": STEP_PERCENT,
        "window": window,
        "known": window.is_some(),
        "usable": usable_tokens(shown_window),
        "currentTokens": current,
    })
}

/// `GET /api/context?botId=&driver=&model=` — окно и порог сжатия для выбранной (ещё не сохранённой) модели.
async fn get_context(State(app): State<Arc<App>>, Query(query): Query<HashMap<String, String>>) -> Json<Value> {
    let mut bot = query.get("botId").and_then(|id| app.store.bot(id)).unwrap_or_default();
    if let Some(driver) = query.get("driver") {
        bot.driver = driver.clone();
    }
    if let Some(model) = query.get("model") {
        bot.model = model.clone();
    }
    Json(describe(window_for(&app, &bot), &bot))
}

pub fn routes() -> Router<Arc<App>> {
    Router::new().route("/api/context", get(get_context))
}

#[cfg(test)]
mod tests {
    #[test]
    fn acp_window_table_knows_claude_and_chatgpt() {
        assert_eq!(super::acp_window_table("claude-sonnet-5"), Some(200_000));
        assert_eq!(super::acp_window_table("opus"), Some(200_000));
        assert_eq!(super::acp_window_table("gpt-6.1-terra"), Some(1_050_000));
        assert_eq!(super::acp_window_table("claude-opus-5-5[1m]"), Some(1_000_000));
        assert_eq!(super::acp_window_table("deepseek-flash"), Some(1_000_000));
        assert_eq!(super::acp_window_table("gpt-5.4"), Some(400_000));
        assert_eq!(super::acp_window_table("mystery"), None);
        assert_eq!(super::acp_window_table(""), None);
    }

    use super::*;

    #[test]
    fn known_models_have_their_documented_windows() {
        assert_eq!(known_tokens("deepseek-flash"), Some(1_000_000));
        assert_eq!(known_tokens("gpt-6.1-terra"), Some(1_050_000));
        assert_eq!(known_tokens("GigaChat-2-Max"), Some(128_000));
        assert_eq!(known_tokens("aliceai-llm-flash/latest"), Some(65_536));
        assert_eq!(known_tokens("aliceai-llm/latest"), Some(131_072));
        assert_eq!(known_tokens("gpt://b1g123/yandexgpt/latest"), Some(32_768));
        assert_eq!(known_tokens("grok-4.7"), Some(500_000));
        assert_eq!(known_tokens("grok-4.5"), Some(500_000));
        assert_eq!(known_tokens("grok-build-0.1"), Some(256_000));
        assert_eq!(known_tokens("grok-4.7-build-fast"), None);
        assert_eq!(known_tokens("claude-opus-5-5"), Some(1_000_000));
        assert_eq!(known_tokens("claude-sonnet-5-5"), Some(1_000_000));
        assert_eq!(known_tokens("claude-haiku-4-5"), Some(200_000));
        assert_eq!(known_tokens("anthropic/claude-sonnet-4-5-20250929"), Some(200_000));
        assert_eq!(known_tokens("gemini-2.5-flash"), Some(1_048_576));
        assert_eq!(known_tokens("gemini-2.5-flash-image"), None, "у картиночной модели окно другое");
    }

    #[test]
    fn an_unknown_model_keeps_todays_window() {
        assert_eq!(known_tokens("my-local-model:7b"), None);
        assert_eq!(known_tokens("claude-something-new"), None);
        assert_eq!(known_tokens(""), None);
        for mode in Memory::ALL {
            assert_eq!(budget_chars(None, mode), 120_000, "без известного окна расти не от чего: {mode:?}");
        }
        assert_eq!(approx_tokens(BASE_CHARS), DEFAULT_TOKENS);
    }

    #[test]
    fn the_budget_follows_the_model_window_and_the_mode() {
        // 128 тыс.: «Обычная» остаётся прежней, «Большая» — половина окна, «Максимум» — 80%.
        assert_eq!(budget_chars(Some(128_000), Memory::Normal), 120_000);
        assert_eq!(budget_chars(Some(128_000), Memory::Large), 192_000);
        assert_eq!(budget_chars(Some(128_000), Memory::Max), 307_200);
        // Миллион: «Обычная» по-прежнему дешёвая, режимы растут вместе с окном.
        assert_eq!(budget_chars(Some(1_000_000), Memory::Normal), 120_000);
        assert_eq!(budget_chars(Some(1_000_000), Memory::Large), 1_500_000);
        assert_eq!(budget_chars(Some(1_000_000), Memory::Max), 2_400_000);
        // Окно в 40 тыс. токенов: «Большая» не бывает меньше «Обычной».
        assert_eq!(budget_chars(Some(40_000), Memory::Normal), 72_000);
        assert_eq!(budget_chars(Some(40_000), Memory::Large), 72_000);
        assert!(budget_chars(Some(40_000), Memory::Max) > 72_000);
        // Маленькое окно не переполняем: 4096 токенов — это 12 288 знаков, берём 60% от них.
        assert_eq!(budget_chars(Some(4_096), Memory::Normal), 7_372);
    }

    #[test]
    fn the_memory_setting_defaults_to_normal() {
        let mut bot = Bot::default();
        assert_eq!(mode_of(&bot), Memory::Normal, "у бота из старого state.json поля нет");
        bot.extra.insert("contextMode".into(), json!("large"));
        assert_eq!(mode_of(&bot), Memory::Large);
        bot.extra.insert("contextMode".into(), json!("всё подряд"));
        assert_eq!(mode_of(&bot), Memory::Normal, "непонятное значение — обычная");
        assert_eq!(Memory::parse("max").key(), "max");
    }

    #[test]
    fn the_window_is_read_from_models_lists() {
        assert_eq!(from_model_json(&json!({ "id": "a", "context_length": 163840 })), Some(163_840));
        assert_eq!(from_model_json(&json!({ "context_window": 32768 })), Some(32_768));
        assert_eq!(from_model_json(&json!({ "max_context_length": 8192 })), Some(8_192));
        assert_eq!(from_model_json(&json!({ "input_token_limit": 1048576 })), Some(1_048_576));
        assert_eq!(from_model_json(&json!({ "id": "x" })), None);
        assert_eq!(from_model_json(&json!({ "context_length": 0 })), None);
    }

    #[test]
    fn ollama_is_capped_by_what_it_will_really_load() {
        let show = json!({
            "model_info": { "general.architecture": "llama", "llama.context_length": 131072 },
            "parameters": "stop \"<|eot|>\"\ntemperature 0.7"
        });
        assert_eq!(ollama_window(&show, None), 4_096, "без num_ctx Ollama берёт 4096, а не окно модели");
        let custom = json!({
            "model_info": { "llama.context_length": 131072 },
            "parameters": "num_ctx                        16384\ntemperature 0.7"
        });
        assert_eq!(ollama_window(&custom, None), 16_384, "num_ctx из Modelfile");
        assert_eq!(ollama_window(&custom, Some(32_768)), 32_768, "загруженное окно точнее");
        let small = json!({ "model_info": { "gemma.context_length": 2048 }, "parameters": "num_ctx 8192" });
        assert_eq!(ollama_window(&small, None), 2_048, "больше, чем умеет сама модель, не берём");
        assert_eq!(ollama_window(&json!({}), None), 4_096);
    }

    #[test]
    fn the_record_cap_is_skipped_only_when_every_bot_asks_for_more() {
        let normal = Bot::default();
        let mut large = Bot::default();
        large.extra.insert("contextMode".into(), json!("large"));
        let count_ignored = |bots: &[Bot]| !bots.is_empty() && bots.iter().all(|b| mode_of(b) != Memory::Normal);
        assert!(!count_ignored(&[normal.clone()]));
        assert!(count_ignored(&[large.clone()]));
        assert!(!count_ignored(&[large, normal]), "в группе с обычным ботом счётчик остаётся");
    }

    #[tokio::test]
    async fn the_setting_survives_a_restart_and_old_state_has_none() {
        let stamp = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos();
        let dir = std::env::temp_dir().join(format!("deloist-ctx-mode-{stamp}"));
        let store = crate::store::Store::open(dir.clone()).await.unwrap();
        let bot = store.add_bot(&json!({ "name": "Кодер" })).await.unwrap();
        assert_eq!(mode_of(&bot), Memory::Normal, "новый бот — «Обычная»");

        let updated = store.update_bot(&bot.id, &json!({ "contextMode": "large" })).await.unwrap().unwrap();
        assert_eq!(mode_of(&updated), Memory::Large);
        let junk = store.update_bot(&bot.id, &json!({ "contextMode": "огромная" })).await.unwrap().unwrap();
        assert_eq!(mode_of(&junk), Memory::Normal, "непонятное значение не запоминается как есть");
        store.update_bot(&bot.id, &json!({ "contextMode": "max" })).await.unwrap();
        // Другие правки профиля режим не трогают.
        let renamed = store.update_bot(&bot.id, &json!({ "name": "Кодер 2" })).await.unwrap().unwrap();
        assert_eq!(mode_of(&renamed), Memory::Max);
        drop(store);

        let reopened = crate::store::Store::open(dir.clone()).await.unwrap();
        assert_eq!(mode_of(&reopened.bot(&bot.id).unwrap()), Memory::Max, "режим лежит в state.json");

        // Старый state.json: у бота нет поля.
        let old: Bot = serde_json::from_value(json!({ "id": "bot_old", "name": "Старый" })).unwrap();
        assert_eq!(mode_of(&old), Memory::Normal);
        let _ = tokio::fs::remove_dir_all(dir).await;
    }

    fn bot_with(field: &str, value: Value) -> Bot {
        let mut bot = Bot::default();
        bot.extra.insert(field.into(), value);
        bot
    }

    #[test]
    fn percent_is_clamped_and_snapped_to_the_step() {
        assert_eq!(clamp_percent(60), 60);
        assert_eq!(clamp_percent(0), 20);
        assert_eq!(clamp_percent(-40), 20);
        assert_eq!(clamp_percent(100), 95);
        assert_eq!(clamp_percent(10_000), 95);
        assert_eq!(clamp_percent(i64::MAX), 95);
        assert_eq!(clamp_percent(i64::MIN), 20);
        assert_eq!(clamp_percent(62), 60);
        assert_eq!(clamp_percent(63), 65);
        assert_eq!(percent_of(&Bot::default()), None);
        assert_eq!(percent_of(&bot_with("compactAtPercent", json!(70))), Some(70));
        assert_eq!(percent_of(&bot_with("compactAtPercent", json!(3))), Some(20), "ручная правка state.json тоже в границах");
        assert_eq!(percent_of(&bot_with("compactAtPercent", json!("много"))), None, "мусор - как не задан");
    }

    #[test]
    fn the_threshold_in_tokens_follows_the_window_and_never_eats_the_reserve() {
        // 200K: 60% = 120 000; запас 20 000 - потолок 180 000.
        assert_eq!(threshold_tokens(Some(200_000), 60), 120_000);
        assert_eq!(threshold_tokens(Some(200_000), 95), 180_000, "95% не оставило бы места ответу");
        assert_eq!(threshold_chars(Some(200_000), 60), 360_000);
        // Миллион токенов - окно большинства современных моделей: 60% = 600 000, запас 130 000.
        assert_eq!(threshold_tokens(Some(1_000_000), 60), 600_000);
        assert_eq!(threshold_chars(Some(1_000_000), 60), 1_800_000);
        assert_eq!(threshold_tokens(Some(1_000_000), 50), 500_000);
        assert_eq!(threshold_tokens(Some(1_000_000), 95), 870_000, "95% съело бы запас, потолок 870 000");
        assert_eq!(threshold_tokens(Some(1_050_000), 60), 630_000);
        assert_eq!(threshold_tokens(Some(1_050_000), 95), 920_000, "у gpt-6.1-terra вход не больше 922 000");
        assert_eq!(known_tokens("gemini-4-carbon"), Some(1_000_000));
        // Неизвестное окно: считаем от 40 000, а не от миллиона.
        assert_eq!(threshold_tokens(None, 60), 24_000);
        assert_eq!(threshold_tokens(None, 95), 32_000);
        // Ollama с 4096: запас четверть окна.
        assert_eq!(reserve_tokens(4_096), 1_024);
        assert_eq!(threshold_tokens(Some(4_096), 20), 819);
        assert_eq!(threshold_tokens(Some(4_096), 95), 3_072);
        for window in [4_096, 32_768, 40_000, 128_000, 200_000, 1_048_576, 10_000_000, usize::MAX / 8] {
            for percent in (MIN_PERCENT..=MAX_PERCENT).step_by(STEP_PERCENT as usize) {
                let t = threshold_tokens(Some(window), percent);
                assert!(t <= window - reserve_tokens(window), "{window} {percent}");
                assert!(t > 0);
                assert!(threshold_chars(Some(window), percent) >= t, "знаков не меньше токенов, без переполнения");
            }
        }
        assert_eq!(threshold_tokens(Some(200_000), 5), 40_000, "ниже границы - граница");
        // Даже на краю usize ничего не паникует.
        let _ = threshold_chars(Some(usize::MAX), 95);
    }

    #[test]
    fn an_old_mode_keeps_its_budget_until_the_slider_is_touched() {
        let normal = Bot::default();
        let large = bot_with("contextMode", json!("large"));
        let max = bot_with("contextMode", json!("max"));
        for window in [None, Some(128_000), Some(200_000), Some(1_000_000), Some(4_096)] {
            assert_eq!(bot_budget(window, &normal), budget_chars(window, Memory::Normal));
            assert_eq!(bot_budget(window, &large), budget_chars(window, Memory::Large));
            assert_eq!(bot_budget(window, &max), budget_chars(window, Memory::Max));
        }
        // Эквивалентные проценты для показа.
        assert_eq!(effective_percent(&large, Some(200_000)), 50);
        assert_eq!(effective_percent(&max, Some(200_000)), 80);
        assert_eq!(effective_percent(&large, Some(1_000_000)), 50);
        assert_eq!(effective_percent(&normal, Some(200_000)), 20, "40 000 из 200 000");
        assert_eq!(effective_percent(&normal, Some(1_000_000)), 20, "ниже границы ползунка - граница");
        assert_eq!(effective_percent(&normal, Some(128_000)), 30);
        assert_eq!(effective_percent(&normal, None), 95, "120 000 знаков - всё окно в 40 000, но не выше границы");
        // Заданный порог главнее старого режима, поле contextMode остаётся читаемым.
        let mut both = bot_with("contextMode", json!("max"));
        both.extra.insert("compactAtPercent".into(), json!(40));
        assert_eq!(bot_budget(Some(200_000), &both), 240_000);
        assert_eq!(mode_of(&both), Memory::Max);
    }

    #[test]
    fn describe_shows_threshold_window_and_ceiling() {
        let info = describe(Some(200_000), &bot_with("compactAtPercent", json!(60)));
        assert_eq!(info["percent"], 60);
        assert_eq!(info["stored"], true);
        assert_eq!(info["known"], true);
        assert_eq!(info["window"], 200_000);
        assert_eq!(info["usable"], 180_000);
        assert_eq!(info["currentTokens"], 120_000);
        let million = describe(Some(1_000_000), &bot_with("compactAtPercent", json!(60)));
        assert_eq!(million["currentTokens"], 600_000);
        let old = describe(Some(128_000), &bot_with("contextMode", json!("large")));
        assert_eq!(old["stored"], false);
        assert_eq!(old["percent"], 50);
        assert_eq!(old["currentTokens"], 64_000);
        let unknown = describe(None, &Bot::default());
        assert_eq!(unknown["known"], false);
        assert_eq!(unknown["window"], Value::Null);
        assert_eq!(unknown["usable"], 32_000);
        assert_eq!(unknown["currentTokens"], 40_000);
    }

    // Сжатие настоящее: по порогу в процентах история длиннее порога сворачивается, короче - нет.
    #[tokio::test]
    async fn compaction_starts_exactly_when_history_reaches_the_chosen_percent() {
        use crate::store::Thread;
        let stamp = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos();
        let dir = std::env::temp_dir().join(format!("deloist-ctx-percent-{stamp}"));
        let store = crate::store::Store::open(dir.clone()).await.unwrap();
        // Окно 4096 токенов: порог 30% - 1228 токенов (3684 знака), порог 90% - 3072 токена (9216 знаков).
        for i in 0..4 {
            store.append_message("bot", json!({ "role": "user", "text": format!("q{i}") })).await.unwrap();
            store.append_message("bot", json!({ "role": "assistant", "text": "x".repeat(1_000) })).await.unwrap();
        }
        let chars = store.window_size(Thread::Bot("bot")).await;
        assert!((4_000..4_100).contains(&chars), "в окне {chars} знаков");
        let at = |percent: u8| threshold_chars(Some(4_096), percent);
        assert!(store.plan_compaction(Thread::Bot("bot"), 0, 0, false, at(30)).await.is_some(), "4000 знаков больше порога 30%");
        assert!(store.plan_compaction(Thread::Bot("bot"), 0, 0, false, at(90)).await.is_none(), "и меньше порога 90%");
        // Тот же путь, что у бота: поле бота -> бюджет -> план.
        let low = bot_with("compactAtPercent", json!(30));
        let high = bot_with("compactAtPercent", json!(90));
        assert!(store.plan_compaction(Thread::Bot("bot"), 0, 0, false, bot_budget(Some(4_096), &low)).await.is_some());
        assert!(store.plan_compaction(Thread::Bot("bot"), 0, 0, false, bot_budget(Some(4_096), &high)).await.is_none());
        drop(store);
        let _ = tokio::fs::remove_dir_all(dir).await;
    }

    #[tokio::test]
    async fn the_percent_is_validated_on_patch_and_survives_a_restart() {
        let stamp = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos();
        let dir = std::env::temp_dir().join(format!("deloist-ctx-patch-{stamp}"));
        let store = crate::store::Store::open(dir.clone()).await.unwrap();
        let bot = store.add_bot(&json!({ "name": "Кодер" })).await.unwrap();
        assert_eq!(percent_of(&bot), None, "новый бот - без порога, прежнее поведение");
        let set = |v: Value| {
            let store = &store;
            let id = bot.id.clone();
            async move { store.update_bot(&id, &json!({ "compactAtPercent": v })).await.unwrap().unwrap() }
        };
        assert_eq!(percent_of(&set(json!(65)).await), Some(65));
        assert_eq!(percent_of(&set(json!(5)).await), Some(20), "ниже границы - граница");
        assert_eq!(percent_of(&set(json!(500)).await), Some(95), "выше - граница");
        assert_eq!(percent_of(&set(json!(62)).await), Some(60), "к шагу");
        assert_eq!(percent_of(&set(json!(70.0)).await), Some(70), "целое число с точкой");
        for junk in [json!("высоко"), json!([1]), json!({ "a": 1 }), json!(true), json!(70.5)] {
            let after = set(junk.clone()).await;
            assert_eq!(percent_of(&after), Some(70), "мусор {junk} не меняет сохранённое");
        }
        let renamed = store.update_bot(&bot.id, &json!({ "name": "Кодер 2" })).await.unwrap().unwrap();
        assert_eq!(percent_of(&renamed), Some(70), "другие правки порог не трогают");
        // Старый режим остаётся на месте и после записи порога.
        store.update_bot(&bot.id, &json!({ "contextMode": "large" })).await.unwrap();
        assert_eq!(mode_of(&store.bot(&bot.id).unwrap()), Memory::Large);
        assert_eq!(percent_of(&store.bot(&bot.id).unwrap()), Some(70));
        assert_eq!(percent_of(&set(Value::Null).await), None, "null убирает порог: снова старый режим");
        set(json!(45)).await;
        drop(store);
        let reopened = crate::store::Store::open(dir.clone()).await.unwrap();
        assert_eq!(percent_of(&reopened.bot(&bot.id).unwrap()), Some(45), "порог лежит в state.json");
        let _ = tokio::fs::remove_dir_all(dir).await;
    }
}
