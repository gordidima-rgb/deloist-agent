//! Фраза → дела через модель человека (0.3.5, порт `task_parser.py` и `phones.py` старого планировщика).
//!
//! Модель предлагает разбиение: отдельные дела, чек-лист, адрес, телефон, исполнителя.
//! Даты, время и повтор считает код (`dates_ru`). Дату или час от модели задача получает, только если
//! код видит их в тексте фразы; повтор берётся только у кода, по явному маркеру. Фраза, которую код
//! разобрал целиком, модель не видит вовсе. Без модели (демо, ошибка, два неудачных ответа) фраза
//! всё равно становится одной задачей: задача не теряется.

use std::sync::Arc;

use axum::body::Bytes;
use axum::extract::State;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use chrono::{Local, NaiveDate, NaiveTime, Timelike};
use regex::Regex;
use serde::Serialize;
use serde_json::{Map, Value};

use crate::dates_ru::{self, Recurrence};
use crate::service;
use crate::store::{uid, Bot};
use crate::todos::{self, Bucket, Item, Todo, TodoError, MAX_ITEMS, MAX_ITEM_TEXT, MAX_NOTES, MAX_TITLE};
use crate::App;

const MAX_TASKS: usize = 20;
const LONG_INPUT_CHARS: usize = 300;
const LONG_INPUT_PARAGRAPHS: usize = 3;
const ADDRESS_MAX: usize = 300;
const PHONE_MAX: usize = 32;
const MIN_AUTO_CLOSE: i64 = 15;
const MAX_AUTO_CLOSE: i64 = 1440;
/// Две попытки: вторая получает текст ошибки первой.
const ATTEMPTS: usize = 2;
const TRIM: &[char] = &[' ', '\t', ',', ';', ':', '—', '–', '-'];

/// Системный промпт. `{TODAY}`, `{NOW}` и `{BOTS}` подставляет `system_prompt`.
const SYSTEM: &str = r##"Ты — разборщик дел. Сегодня {TODAY}.{NOW} Преобразуй сообщение человека в JSON РОВНО такой формы:
{"tasks": [{"title": "<название задачи>", "importance": "key|active|someday", "due_date": "YYYY-MM-DD или null", "due_time": "HH:MM или null", "address": "<адрес места или null>", "phone": "<телефон или null>", "assignee": "<имя бота или null>", "items": ["<пункт>"], "recurrence": {"freq": "daily|weekly|monthly|yearly", "interval": 1, "weekdays": [0..6] или null, "monthday": 1..31 или null} ИЛИ null, "auto_close_after_min": <минуты 15..1440 или null>}]}
ЯЗЫК ОТВЕТА = ЯЗЫК СООБЩЕНИЯ: title, items, address заполняй на том языке, на котором написал человек. Не переводи сообщение и не «исправляй» его.
Несколько независимых дел — верни КАЖДОЕ отдельным элементом tasks (до 20). Ни одно смысловое слово не пропадает: каждое существительное остаётся в title, address или items. Выбрасывай только предлоги и служебные обороты.
Чек-лист (items) — только у составного дела с общеизвестным составом или порядком шагов (например «собрать чемодан в Сочи»), пункты конкретные. У учебных дел и дел «посмотреть/прочитать/разобраться» items = [], если человек сам не перечислил пункты через запятую или двоеточие. Разные поручения из одного сообщения не складывай в один чек-лист. items = [] у дел, которые делаются одним движением: «позвонить маме», «купить молоко».
importance: key — критично или на сегодня-завтра; active — обычное; someday — когда-нибудь.
due_date — только если день НАЗВАН («сегодня», «завтра», «во вторник», «15-го»); относительные даты считай от сегодняшней. Если день не назван, due_date = null. «Сегодня или завтра» — одна задача с ближайшей датой.
due_time — только при явном часе («в 10:00», «к 9 утра», «в 18:30», «в 7 вечера») или при интервале «через N минут/часов» (тогда время = текущее время + интервал). Число из «через N минут» — длительность, не время суток. Число, которое обозначает количество, сумму, вес или размер («10-20 адресов», «5 тысяч», «22 кг»), временем не является. Слово «утром», «вечером», «ночью» без цифры — due_time = null, и слово остаётся в title. Повтор часа не даёт. «С 9 до 17» — ДВЕ задачи: начало в 09:00 и конец в 17:00.
recurrence — только при явном маркере периодичности: «каждый день», «ежедневно», «каждую пятницу», «по будням», «раз в неделю», «каждый месяц N-го», «3 раза в неделю». Оборот про будущее без маркера («на следующие дни», «на неделе», «на днях») — recurrence = null. Иначе recurrence = null.
auto_close_after_min — только у повторяющейся бытовой рутины (обычно 60–240); у разовых поручений — null.
Дни недели по-казахски: дүйсенбі=0, сейсенбі=1, сәрсенбі=2, бейсенбі=3, жұма=4, сенбі=5, жексенбі=6. По-белорусски: панядзелак=0, аўторак=1, серада=2, чацвер=3, пятніца=4, субота=5, нядзеля=6.
title держи близко к словам человека: не перефразируй действие, убирай только адрес и служебные обороты. Название места (клиника, магазин) оставляй в title. Не превращай короткое дело в отглагольное существительное. Адрес в title не дублируй.
address — почтовый или уличный адрес, если он есть; только адрес, без названия заведения; номер дома с буквой («д71 а») сохраняй целиком. Нет адреса — null.
phone — телефон из сообщения, только цифры и «+», без слов «тел.». Номер из title убирай. Нет — null.
assignee — только бот из списка: {BOTS}. Исполнитель назначается ТОЛЬКО когда имя написано через «+» или «@» («+Оля», «@Петя»). Имя, просто упомянутое в тексте, исполнителем не делает: «позвонить Олегу» — Олег не исполнитель. Без «+» и «@» assignee = null, имя остаётся в title.
Ответь ТОЛЬКО JSON одной строкой, без пояснений, без markdown и без ``` ограждений."##;

/// Одна задача, как её увидит человек и как её сохранит `Todos`.
#[derive(Clone, Debug, Default, PartialEq, Serialize)]
pub struct Draft {
    pub title: String,
    pub bucket: Bucket,
    /// ГГГГ-ММ-ДД.
    pub due_date: Option<String>,
    /// ЧЧ:ММ.
    pub due_time: Option<String>,
    pub notes: String,
    pub recurrence: Option<Recurrence>,
    pub items: Vec<String>,
    /// Id бота-исполнителя.
    pub assignee: Option<String>,
    pub auto_close_after_min: Option<u32>,
    pub address: Option<String>,
    pub phone: Option<String>,
}

/// Подсказка программе, что спросить у человека перед сохранением (`ClarifyStrip`).
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Clarify {
    /// `day`: есть время, нет дня. `routine`: три и больше задач со временем без повтора.
    pub kind: &'static str,
}

/// Результат разбора: задачи, откуда они взялись и что спросить.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Parsed {
    pub tasks: Vec<Draft>,
    /// `code` — код справился без модели; `model` — модель разбила фразу; `fallback` — модели не было или она не ответила.
    pub source: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub clarify: Option<Clarify>,
}

fn with_clarify(tasks: Vec<Draft>, source: &'static str) -> Parsed {
    let clarify = clarify_for(&tasks);
    Parsed { tasks, source, clarify }
}

fn clarify_for(tasks: &[Draft]) -> Option<Clarify> {
    if tasks.iter().any(|task| task.due_time.is_some() && task.due_date.is_none()) {
        return Some(Clarify { kind: "day" });
    }
    let timed = tasks.iter().filter(|task| task.due_time.is_some() && task.recurrence.is_none()).count();
    (timed >= 3).then_some(Clarify { kind: "routine" })
}

/// Разбор фразы. Без модели или при сбое модели фраза всё равно становится задачами кодом.
pub async fn parse_with_model(app: &Arc<App>, text: &str, today: NaiveDate, now: NaiveTime) -> Result<Parsed, String> {
    let text = text.trim();
    if text.is_empty() {
        return Err("напиши, что сделать".into());
    }
    if dates_ru::parse(text, today, now).complete {
        return Ok(with_clarify(vec![code_draft(text, today, now)], "code"));
    }
    let Some(bot) = service::house_bot(app) else {
        return Ok(with_clarify(vec![code_draft(text, today, now)], "fallback"));
    };
    let bots = app.store.bots();
    let system = system_prompt(today, now, &bots);
    let mut task = text.to_string();
    for attempt in 0..ATTEMPTS {
        let reply = match service::complete(app, &bot, &system, &task).await {
            Ok(reply) => reply,
            // Модель недоступна или упёрлась в лимит: повторять бессмысленно.
            Err(_) => break,
        };
        match read_reply(&reply).and_then(|raw| finish(text, today, now, &raw, &bots)) {
            Ok(drafts) => return Ok(with_clarify(drafts, "model")),
            Err(error) if attempt + 1 < ATTEMPTS => {
                task = format!("{text}\n\nПредыдущий ответ не прошёл проверку: {error}. Верни ТОЛЬКО корректный JSON по схеме.");
            }
            Err(_) => {}
        }
    }
    Ok(with_clarify(vec![code_draft(text, today, now)], "fallback"))
}

fn system_prompt(today: NaiveDate, now: NaiveTime, bots: &[Bot]) -> String {
    let roster = if bots.is_empty() {
        "нет".to_string()
    } else {
        bots.iter().map(|bot| bot.name.as_str()).collect::<Vec<_>>().join(", ")
    };
    SYSTEM
        .replace("{TODAY}", &today.format("%Y-%m-%d").to_string())
        .replace("{NOW}", &format!(" Текущее время {}.", now.format("%H:%M")))
        .replace("{BOTS}", &roster)
}

/// Список задач из ответа модели: `{"tasks": [...]}`, а если модель отдала одну задачу без обёртки — она же.
fn read_reply(reply: &str) -> Result<Vec<Value>, String> {
    let value = service::json_object(reply).ok_or("ответ не JSON-объект")?;
    if let Some(list) = value.get("tasks").and_then(Value::as_array) {
        return Ok(list.clone());
    }
    if value.get("title").is_some() {
        return Ok(vec![value]);
    }
    Err("нет поля tasks".into())
}

/// Что код видит в тексте: дата, время и повтор по всей фразе и по её кускам.
struct Facts {
    phrase: dates_ru::Parsed,
    clauses: Vec<dates_ru::Parsed>,
    text: String,
}

impl Facts {
    fn date_known(&self, day: NaiveDate) -> bool {
        self.phrase.due_date == Some(day) || self.clauses.iter().any(|clause| clause.due_date == Some(day))
    }

    fn time_known(&self, time: NaiveTime) -> bool {
        self.phrase.due_time == Some(time)
            || self.clauses.iter().any(|clause| clause.due_time == Some(time))
            || hour_in_text(&self.text, time)
    }
}

fn facts(text: &str, today: NaiveDate, now: NaiveTime) -> Facts {
    let clause_re = Regex::new(r"[.;,\n!?]|\s+(?:и|или)\s+").unwrap();
    let clauses = clause_re.split(text).filter(|part| !part.trim().is_empty()).map(|part| dates_ru::parse(part, today, now)).collect();
    Facts { phrase: dates_ru::parse(text, today, now), clauses, text: text.to_string() }
}

/// Час из модели виден в тексте: цифрой (с учётом «7 вечера» = 19) или «H:MM».
/// Минуты «:MM» должны совпасть; голый час даёт только :00.
fn hour_in_text(text: &str, time: NaiveTime) -> bool {
    let (hour, minute) = (time.hour(), time.minute());
    let clock = Regex::new(r"(^|\D)(\d{1,2}):(\d{2})(\D|$)").unwrap();
    let clock_hit = clock.captures_iter(text).any(|caps| {
        let h: u32 = caps[2].parse().unwrap_or(99);
        let m: u32 = caps[3].parse().unwrap_or(99);
        m == minute && h % 12 == hour % 12
    });
    if clock_hit {
        return true;
    }
    let bare = Regex::new(r"(^|\D)(\d{1,2})(\D|$)").unwrap();
    minute == 0
        && bare.captures_iter(text).any(|caps| {
            let n: u32 = caps[2].parse().unwrap_or(99);
            n % 12 == hour % 12
        })
}

fn text_of(obj: &Map<String, Value>, key: &str) -> Option<String> {
    obj.get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty() && !s.eq_ignore_ascii_case("null"))
        .map(str::to_string)
}

fn clip(text: &str, max: usize) -> String {
    text.chars().take(max).collect()
}

fn bucket_of(raw: Option<String>) -> Bucket {
    match raw.as_deref() {
        Some("key") => Bucket::Key,
        Some("someday") => Bucket::Someday,
        _ => Bucket::Active,
    }
}

/// Разбор ответа модели: сверка с кодом, очистка полей, ограничения. Ошибка формы → повтор модели.
pub fn finish(text: &str, today: NaiveDate, now: NaiveTime, raw_tasks: &[Value], bots: &[Bot]) -> Result<Vec<Draft>, String> {
    let count = raw_tasks.len().min(MAX_TASKS);
    if count == 0 {
        return Err("пустой список задач".into());
    }
    let facts = facts(text, today, now);
    let single = count == 1;
    let phone_fallback = find_phone(text);
    let study_input = looks_like_study(text);
    let mut drafts = Vec::with_capacity(count);
    for raw in raw_tasks.iter().take(count) {
        let obj = raw.as_object().ok_or("задача — не объект")?;
        let mut title = clip(&text_of(obj, "title").ok_or("у задачи нет названия")?, MAX_TITLE);

        let phone = text_of(obj, "phone").and_then(|raw| normalize_phone(&raw)).or_else(|| phone_fallback.clone());
        if let Some(phone) = &phone {
            title = strip_phone(&title, phone);
        }

        let model_date = text_of(obj, "due_date").and_then(|raw| NaiveDate::parse_from_str(&raw, "%Y-%m-%d").ok());
        let due_date = model_date
            .filter(|day| facts.date_known(*day))
            .or(if single { facts.phrase.due_date } else { None });

        let model_time = text_of(obj, "due_time").and_then(|raw| NaiveTime::parse_from_str(&raw, "%H:%M").ok());
        let due_time = model_time
            .filter(|time| facts.time_known(*time))
            .or(if single { facts.phrase.due_time } else { None });

        // Повтор берёт код, по маркеру в тексте. Модельный повтор без маркера отбрасывается.
        let marked = obj.get("recurrence").is_some_and(|value| !value.is_null());
        let recurrence = facts.phrase.recurrence.clone().filter(|_| marked || single);

        let auto_close_after_min = if recurrence.is_some() {
            obj.get("auto_close_after_min")
                .and_then(Value::as_i64)
                .filter(|minutes| (MIN_AUTO_CLOSE..=MAX_AUTO_CLOSE).contains(minutes))
                .map(|minutes| minutes as u32)
        } else {
            None
        };

        let items = if !has_enumeration(text) && (study_input || looks_like_study(&title)) {
            Vec::new()
        } else {
            clean_items(obj.get("items"))
        };

        let assignee = text_of(obj, "assignee")
            .map(|name| name.trim_start_matches(['@', '+']).trim().to_string())
            .and_then(|name| bots.iter().find(|bot| bot.name.to_lowercase() == name.to_lowercase() && mentioned(text, &name)))
            .map(|bot| bot.id.clone());

        drafts.push(Draft {
            title,
            bucket: bucket_of(text_of(obj, "importance")),
            due_date: due_date.map(|day| day.format("%Y-%m-%d").to_string()),
            due_time: due_time.map(|time| time.format("%H:%M").to_string()),
            notes: String::new(),
            recurrence,
            items,
            assignee,
            auto_close_after_min,
            address: text_of(obj, "address").map(|address| clip(&address, ADDRESS_MAX)),
            phone: phone.map(|phone| clip(&phone, PHONE_MAX)),
        });
    }
    // Длинный ввод целиком в заметку первой задачи: без модели, точно.
    if is_long_input(text) {
        if let Some(first) = drafts.first_mut() {
            first.notes = clip(text.trim(), MAX_NOTES);
        }
    }
    Ok(drafts)
}

fn clean_items(value: Option<&Value>) -> Vec<String> {
    value
        .and_then(Value::as_array)
        .map(|list| {
            list.iter()
                .filter_map(Value::as_str)
                .map(str::trim)
                .filter(|item| !item.is_empty())
                .take(MAX_ITEMS)
                .map(|item| clip(item, MAX_ITEM_TEXT))
                .collect()
        })
        .unwrap_or_default()
}

fn mentioned(text: &str, name: &str) -> bool {
    Regex::new(&format!(r"(?i)[+@]\s*{}", regex::escape(name))).map(|re| re.is_match(text)).unwrap_or(false)
}

/// Задача из фразы без модели: название, срок и повтор кодом, раздел «Актуальное».
pub fn code_draft(text: &str, today: NaiveDate, now: NaiveTime) -> Draft {
    let phrase = dates_ru::parse(text, today, now);
    let phone = find_phone(text);
    let mut title = clip(&phrase.title, MAX_TITLE);
    if let Some(phone) = &phone {
        title = strip_phone(&title, phone);
    }
    Draft {
        title,
        due_date: phrase.due_date.map(|day| day.format("%Y-%m-%d").to_string()),
        due_time: phrase.due_time.map(|time| time.format("%H:%M").to_string()),
        notes: if is_long_input(text) { clip(text.trim(), MAX_NOTES) } else { String::new() },
        recurrence: phrase.recurrence.clone(),
        phone,
        ..Draft::default()
    }
}

/// «+Имя» или «@Имя» с полным именем бота делает его исполнителем, даже если модели не было: слово из названия уходит.
/// Только живой текст человека; исполнитель, которого уже выбрала модель, остаётся.
pub fn assign_mentioned(draft: &mut Draft, bots: &[Bot]) {
    if draft.assignee.is_some() {
        return;
    }
    for bot in bots {
        let Ok(re) = Regex::new(&format!(r"(?i)(^|\s)[+@]{}($|[\s,.;:!?])", regex::escape(&bot.name))) else { continue };
        if !re.is_match(&draft.title) {
            continue;
        }
        let rest = re.replace_all(&draft.title, " ").split_whitespace().collect::<Vec<_>>().join(" ");
        if rest.chars().count() >= 3 {
            draft.title = rest;
        }
        draft.assignee = Some(bot.id.clone());
        return;
    }
}

/// Задача из черновика: та же запись, что у ручной задачи.
pub fn to_todo(draft: Draft) -> Todo {
    Todo {
        title: draft.title,
        bucket: draft.bucket,
        due_date: draft.due_date,
        due_time: draft.due_time,
        notes: draft.notes,
        recurrence: draft.recurrence,
        items: draft.items.into_iter().map(|text| Item { id: uid("item"), text, done: false }).collect(),
        assignee: draft.assignee,
        auto_close_after_min: draft.auto_close_after_min,
        address: draft.address,
        phone: draft.phone,
        ..Todo::default()
    }
}

// --- Порт `phones.py`: телефон из свободного текста ---

fn candidate_re() -> Regex {
    Regex::new(r"(?:^|[^\d+])(\+?\d[\d\s\-.()]{7,20}\d)").unwrap()
}

fn digits_of(text: &str) -> String {
    text.chars().filter(char::is_ascii_digit).collect()
}

fn last_ten(digits: &str) -> String {
    let count = digits.chars().count();
    digits.chars().skip(count.saturating_sub(10)).collect()
}

/// Номер к виду хранения: «+79001234567». Не телефон (короче 5 цифр) — None.
pub fn normalize_phone(raw: &str) -> Option<String> {
    let raw = raw.trim();
    let d = digits_of(raw);
    if raw.is_empty() || d.len() < 5 {
        return None;
    }
    if d.len() == 11 && d.starts_with('8') {
        return Some(format!("+7{}", &d[1..]));
    }
    if d.len() == 11 && d.starts_with('7') {
        return Some(format!("+{d}"));
    }
    if d.len() == 10 {
        return Some(format!("+7{d}"));
    }
    if raw.starts_with('+') || d.len() >= 11 {
        return Some(format!("+{d}"));
    }
    Some(d)
}

/// Первый фрагмент из 10–15 цифр: даты (8 цифр) и время его не дают.
pub fn find_phone(text: &str) -> Option<String> {
    candidate_re().captures_iter(text).find_map(|caps| {
        let raw = &caps[1];
        let count = digits_of(raw).len();
        (10..=15).contains(&count).then(|| normalize_phone(raw)).flatten()
    })
}

/// Убрать номер (и подпись «тел.» перед ним) из названия. Пустое название не возвращаем.
pub fn strip_phone(title: &str, phone: &str) -> String {
    let target = last_ten(&digits_of(phone));
    let label = Regex::new(r"(?i)(?:по\s+)?(?:тел(?:ефон[ауеы]?)?|номер[уа]?|т)\.?\s*:?\s*$").unwrap();
    let spaces = Regex::new(r"\s{2,}").unwrap();
    for caps in candidate_re().captures_iter(title) {
        let found = caps.get(1).unwrap();
        if last_ten(&digits_of(found.as_str())) != target {
            continue;
        }
        let head = label.replace(&title[..found.start()], "").trim_end_matches(TRIM).to_string();
        let tail = title[found.end()..].trim_start_matches(TRIM);
        let joined = spaces.replace_all(format!("{head} {tail}").trim(), " ").to_string();
        let cleaned = joined.trim_matches(TRIM).to_string();
        return if cleaned.is_empty() { title.to_string() } else { cleaned };
    }
    title.to_string()
}

// --- Порт `_STUDY_RE` и правил чек-листа ---

fn looks_like_study(text: &str) -> bool {
    let study = Regex::new(
        r"(?i)(^|\s)(выуч|учить|подуч|изуч|повтор|разобр|посмотр|почит|прочит|лекци|урок|вебинар|курс|[уү]йрен|о[кқ]ы|вывуч|навуч|падрыхт|подготов\w*\s+к\s+(экзамен|зач[её]т|контрольн|тест))\w*",
    )
    .unwrap();
    study.is_match(text)
}

/// Человек сам перечислил пункты: запятые, точки с запятой, двоеточие, переносы.
fn has_enumeration(text: &str) -> bool {
    text.contains([',', ';', ':', '\n'])
}

fn is_long_input(text: &str) -> bool {
    let text = text.trim();
    if text.chars().count() >= LONG_INPUT_CHARS {
        return true;
    }
    let blank_line = Regex::new(r"\n\s*\n").unwrap();
    blank_line.split(text).filter(|part| !part.trim().is_empty()).count() >= LONG_INPUT_PARAGRAPHS
}

// --- HTTP ---

fn request_text(bytes: &Bytes) -> Result<String, Response> {
    let body: Value = serde_json::from_slice(bytes).unwrap_or(Value::Null);
    match body.get("text").and_then(Value::as_str).map(str::trim).filter(|text| !text.is_empty()) {
        Some(text) => Ok(text.to_string()),
        None => Err(todos::respond(Err(TodoError::Invalid("нужен текст фразы".into())))),
    }
}

/// `POST /api/todos/parse` — предпросмотр разбора, ничего не сохраняет.
pub async fn parse_route(State(app): State<Arc<App>>, bytes: Bytes) -> Response {
    let text = match request_text(&bytes) {
        Ok(text) => text,
        Err(response) => return response,
    };
    let now = Local::now();
    match parse_with_model(&app, &text, now.date_naive(), now.time()).await {
        Ok(parsed) => (StatusCode::OK, Json(parsed)).into_response(),
        Err(message) => todos::respond(Err(TodoError::Invalid(message))),
    }
}

/// `POST /api/todos/from-text` — разбирает фразу и сохраняет все задачи. Ответ — созданные задачи.
/// Необязательный `bucket`: раздел, который человек выбрал кнопкой в поле ввода. «Актуальное» — значение
/// по умолчанию, поэтому оно не перебивает догадку модели; «Главное» и «Когда-нибудь» — выбор человека, он важнее.
pub async fn from_text_route(State(app): State<Arc<App>>, bytes: Bytes) -> Response {
    let text = match request_text(&bytes) {
        Ok(text) => text,
        Err(response) => return response,
    };
    let body: Value = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
    let chosen = match body.get("bucket").and_then(Value::as_str) {
        None => None,
        Some(raw) => match todos::Bucket::parse(raw) {
            Ok(todos::Bucket::Active) => None,
            Ok(bucket) => Some(bucket),
            Err(error) => return todos::respond(Err(error)),
        },
    };
    let now = Local::now();
    let parsed = match parse_with_model(&app, &text, now.date_naive(), now.time()).await {
        Ok(parsed) => parsed,
        Err(message) => return todos::respond(Err(TodoError::Invalid(message))),
    };
    let bots = app.store.bots();
    let mut made = Vec::with_capacity(parsed.tasks.len());
    for mut draft in parsed.tasks {
        assign_mentioned(&mut draft, &bots);
        let mut todo = to_todo(draft);
        if let Some(bucket) = chosen {
            todo.bucket = bucket;
        }
        match app.todos.add(todo) {
            Ok(todo) => made.push(todos::view(&todo, crate::store::now())),
            Err(error) => return todos::respond(Err(error)),
        }
    }
    todos::changed(&app);
    todos::respond(Ok((StatusCode::CREATED, Value::Array(made))))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn today() -> NaiveDate {
        NaiveDate::from_ymd_opt(2026, 11, 13).unwrap()
    }

    fn nine() -> NaiveTime {
        NaiveTime::from_hms_opt(9, 0, 0).unwrap()
    }

    fn tomorrow_iso() -> String {
        "2026-11-14".to_string()
    }

    fn tasks(value: Value) -> Vec<Value> {
        value["tasks"].as_array().unwrap().clone()
    }

    fn bot(id: &str, name: &str) -> Bot {
        Bot { id: id.into(), name: name.into(), ..Bot::default() }
    }

    #[test]
    fn plus_name_makes_the_bot_the_assignee_without_a_model() {
        let bots = vec![bot("b1", "Бета")];
        let mut draft = Draft { title: "+Бета сделать отчёт".into(), ..Draft::default() };
        assign_mentioned(&mut draft, &bots);
        assert_eq!(draft.assignee.as_deref(), Some("b1"));
        assert_eq!(draft.title, "сделать отчёт");
        let mut plain = Draft { title: "позвонить Бета-тестеру".into(), ..Draft::default() };
        assign_mentioned(&mut plain, &bots);
        assert_eq!(plain.assignee, None, "без + и @ и внутри слова исполнителем не делает");
        let mut prefix = Draft { title: "сделать +Бетаигра".into(), ..Draft::default() };
        assign_mentioned(&mut prefix, &bots);
        assert_eq!(prefix.assignee, None, "только полное имя");
    }

    #[test]
    fn two_independent_tasks_are_split_and_both_get_the_confirmed_date() {
        let reply = json!({"tasks": [
            {"title": "Купить хлеб", "importance": "active", "due_date": "2026-11-14"},
            {"title": "Позвонить маме", "importance": "key", "due_date": "2026-11-14"},
        ]});
        let drafts = finish("купить хлеб завтра и позвонить маме", today(), nine(), &tasks(reply), &[]).unwrap();
        assert_eq!(drafts.len(), 2);
        assert_eq!(drafts[0].title, "Купить хлеб");
        assert_eq!(drafts[1].bucket, Bucket::Key);
        assert!(drafts.iter().all(|d| d.due_date.as_deref() == Some(tomorrow_iso().as_str())));
    }

    #[test]
    fn invented_checklist_is_stripped_for_study_tasks_but_kept_when_listed() {
        let reply = json!({"tasks": [{"title": "Выучить физику", "items": ["повторить кинематику", "проработать задачи"]}]});
        let drafts = finish("выучить физику", today(), nine(), &tasks(reply.clone()), &[]).unwrap();
        assert!(drafts[0].items.is_empty(), "придуманный чек-лист снят");
        let listed = finish("выучить физику: кинематика, динамика", today(), nine(), &tasks(reply), &[]).unwrap();
        assert_eq!(listed[0].items.len(), 2, "пункты, которые человек назвал сам, остаются");
    }

    #[test]
    fn evening_is_not_a_time_even_when_the_model_invents_one() {
        let reply = json!({"tasks": [{"title": "Полить цветы", "due_time": "20:00"}]});
        let drafts = finish("полить цветы вечером", today(), nine(), &tasks(reply), &[]).unwrap();
        assert_eq!(drafts[0].due_time, None);
        assert_eq!(drafts[0].title, "Полить цветы");
    }

    #[test]
    fn phone_is_taken_out_of_the_title_and_normalized() {
        let reply = json!({"tasks": [{"title": "Позвонить маме +7 900 123-45-67", "phone": null}]});
        let drafts = finish("позвонить маме +7 900 123-45-67", today(), nine(), &tasks(reply), &[]).unwrap();
        assert_eq!(drafts[0].phone.as_deref(), Some("+79001234567"));
        assert_eq!(drafts[0].title, "Позвонить маме");
    }

    #[test]
    fn model_date_that_the_phrase_does_not_contain_is_rejected() {
        let reply = json!({"tasks": [{"title": "Купить хлеб", "due_date": "2031-01-01"}]});
        let drafts = finish("купить хлеб", today(), nine(), &tasks(reply), &[]).unwrap();
        assert_eq!(drafts[0].due_date, None);
    }

    #[test]
    fn model_date_the_phrase_confirms_is_kept() {
        let reply = json!({"tasks": [{"title": "Купить хлеб", "due_date": "2026-11-14"}]});
        let drafts = finish("купить хлеб завтра", today(), nine(), &tasks(reply), &[]).unwrap();
        assert_eq!(drafts[0].due_date.as_deref(), Some("2026-11-14"));
    }

    #[test]
    fn interval_from_nine_to_seventeen_becomes_two_tasks_with_both_hours() {
        let reply = json!({"tasks": [
            {"title": "Начать работу", "due_time": "09:00"},
            {"title": "Закончить работу", "due_time": "17:00"},
        ]});
        let drafts = finish("работа с 9 до 17", today(), nine(), &tasks(reply), &[]).unwrap();
        assert_eq!(drafts[0].due_time.as_deref(), Some("09:00"));
        assert_eq!(drafts[1].due_time.as_deref(), Some("17:00"));
    }

    #[test]
    fn long_input_is_kept_whole_in_the_first_task_notes() {
        let long = format!("Первый абзац про поездку.\n\nВторой абзац.\n\nТретий абзац. {}", "текст ".repeat(10));
        let reply = json!({"tasks": [{"title": "Поездка"}, {"title": "Вещи"}]});
        let drafts = finish(&long, today(), nine(), &tasks(reply), &[]).unwrap();
        assert_eq!(drafts[0].notes, long.trim());
        assert!(drafts[1].notes.is_empty());
    }

    #[test]
    fn assignee_needs_a_plus_or_at_and_a_bot_name() {
        let bots = vec![bot("bot_petya", "Петя")];
        let reply = json!({"tasks": [{"title": "Купить хлеб", "assignee": "Петя"}]});
        let with_plus = finish("+Петя купить хлеб", today(), nine(), &tasks(reply.clone()), &bots).unwrap();
        assert_eq!(with_plus[0].assignee.as_deref(), Some("bot_petya"));
        let mentioned = finish("позвонить Пете купить хлеб", today(), nine(), &tasks(reply), &bots).unwrap();
        assert_eq!(mentioned[0].assignee, None, "имя без + и @ исполнителем не делает");
    }

    #[test]
    fn repeat_comes_from_the_code_marker_and_auto_close_needs_it() {
        let reply = json!({"tasks": [{"title": "Зарядка", "recurrence": {"freq": "daily"}, "auto_close_after_min": 60}]});
        let drafts = finish("каждый день зарядка", today(), nine(), &tasks(reply), &[]).unwrap();
        assert_eq!(drafts[0].recurrence.as_ref().map(|r| r.freq), Some(dates_ru::Freq::Daily));
        assert_eq!(drafts[0].auto_close_after_min, Some(60));
        let once = json!({"tasks": [{"title": "Зарядка", "recurrence": {"freq": "daily"}, "auto_close_after_min": 60}]});
        let no_marker = finish("зарядка", today(), nine(), &tasks(once), &[]).unwrap();
        assert_eq!(no_marker[0].recurrence, None, "модельный повтор без маркера отбрасывается");
        assert_eq!(no_marker[0].auto_close_after_min, None);
    }

    #[test]
    fn an_empty_or_nameless_reply_is_an_error_so_the_model_is_asked_again() {
        assert!(finish("купить хлеб", today(), nine(), &[], &[]).is_err());
        let nameless = json!({"tasks": [{"importance": "key"}]});
        assert!(finish("купить хлеб", today(), nine(), &tasks(nameless), &[]).is_err());
    }

    #[test]
    fn a_phrase_the_code_finishes_needs_no_model_and_reads_the_date_and_time() {
        let draft = code_draft("позвонить маме завтра в 10 часов", today(), nine());
        assert_eq!(draft.title.to_lowercase(), "позвонить маме");
        assert_eq!(draft.due_date.as_deref(), Some("2026-11-14"));
        assert_eq!(draft.due_time.as_deref(), Some("10:00"));
    }

    #[test]
    fn clarify_asks_for_a_day_or_a_routine() {
        let timed = |title: &str| Draft { title: title.into(), due_time: Some("10:00".into()), ..Draft::default() };
        let no_day = with_clarify(vec![timed("а")], "code");
        assert_eq!(no_day.clarify, Some(Clarify { kind: "day" }));
        let dated = Draft { due_date: Some(tomorrow_iso()), ..timed("б") };
        let two = with_clarify(vec![dated.clone(), dated.clone()], "model");
        assert_eq!(two.clarify, None, "два дела со временем и датой — без вопроса");
        let routine = with_clarify(vec![dated.clone(), dated.clone(), dated], "model");
        assert_eq!(routine.clarify, Some(Clarify { kind: "routine" }), "три дела со временем без повтора — спросить про рутину");
        let daily = Recurrence { freq: dates_ru::Freq::Daily, interval: 1, weekdays: vec![], monthday: None };
        let repeated = Draft { due_date: Some(tomorrow_iso()), recurrence: Some(daily), ..timed("г") };
        let with_repeat = with_clarify(vec![repeated.clone(), repeated.clone(), repeated], "model");
        assert_eq!(with_repeat.clarify, None, "повторяющиеся дела рутину не спрашивают");
    }

    #[test]
    fn phone_normalization_matches_the_old_rules() {
        assert_eq!(normalize_phone("8 900 123-45-67").as_deref(), Some("+79001234567"));
        assert_eq!(normalize_phone("123"), None);
        assert_eq!(find_phone("на 12.05.2026 в 10"), None, "даты и время телефоном не считаем");
    }
}
