//! Список задач человека: срок, раздел, повтор, чек-лист, архив и поиск.
//! Порт планировщика (0.3.5): порядок `taskOrder.ts`, переносы `carryOver.ts`, поиск `search.ts`,
//! автозакрытие `autoClose.ts`, следующий экземпляр повтора `recurrence.py`, автоархив
//! `archive_sweeper.py`. Даты и повторы разбирает `dates_ru` (без модели). Дни — по часам компьютера.
//! Задача — это не ход бота (тот называется «task» в коде), поэтому маршрут `/api/todos`.

use std::cmp::Ordering;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering as AtomicOrd};
use std::sync::{Arc, Mutex};

use axum::body::Bytes;
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::{get, patch, post};
use axum::{Json, Router};
use chrono::{Local, NaiveDate, NaiveTime, TimeZone};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::dates_ru::{self, Recurrence};
use crate::store::uid;
use crate::App;

pub const MAX_ACTIVE: usize = 2000;
pub const MAX_TITLE: usize = 200;
pub const MAX_NOTES: usize = 2000;
pub const MAX_ITEMS: usize = 30;
pub const MAX_ITEM_TEXT: usize = 200;
const MIN_AUTO_CLOSE: i64 = 15;
const MAX_AUTO_CLOSE: i64 = 1440;
const MINUTE_MS: i64 = 60_000;

/// Раздел доски: «Главное», «Актуальное», «Когда-нибудь».
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Bucket {
    Key,
    #[default]
    Active,
    Someday,
}

impl Bucket {
    pub(crate) fn parse(raw: &str) -> Result<Self, TodoError> {
        match raw.trim() {
            "key" => Ok(Self::Key),
            "active" => Ok(Self::Active),
            "someday" => Ok(Self::Someday),
            _ => Err(TodoError::Invalid("раздел: key, active или someday".into())),
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Item {
    pub id: String,
    pub text: String,
    pub done: bool,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Todo {
    pub id: String,
    pub title: String,
    pub bucket: Bucket,
    /// ГГГГ-ММ-ДД.
    pub due_date: Option<String>,
    /// ЧЧ:ММ.
    pub due_time: Option<String>,
    pub notes: String,
    pub recurrence: Option<Recurrence>,
    pub items: Vec<Item>,
    /// Исполнитель — только бот (решение владельца 13.11.2026).
    pub assignee: Option<String>,
    pub auto_close_after_min: Option<u32>,
    /// Адрес места и телефон, которые разбор фразы вынес из названия (0.3.5).
    pub address: Option<String>,
    pub phone: Option<String>,
    pub done: bool,
    pub done_at: Option<i64>,
    pub archived: bool,
    pub created_at: i64,
}

#[derive(Default, Serialize, Deserialize)]
#[serde(default)]
struct Saved {
    todos: Vec<Todo>,
}

#[derive(Debug)]
pub enum TodoError {
    Invalid(String),
    Limit,
    NotFound,
    Broken,
}

impl TodoError {
    fn status(&self) -> StatusCode {
        match self {
            Self::Invalid(_) => StatusCode::UNPROCESSABLE_ENTITY,
            Self::Limit | Self::Broken => StatusCode::CONFLICT,
            Self::NotFound => StatusCode::NOT_FOUND,
        }
    }

    fn code(&self) -> &'static str {
        match self {
            Self::Invalid(_) => "invalid",
            Self::Limit => "todo_limit",
            Self::NotFound => "not_found",
            Self::Broken => "storage_broken",
        }
    }

    pub fn message(&self) -> String {
        match self {
            Self::Invalid(message) => message.clone(),
            Self::Limit => format!("уже {MAX_ACTIVE} задач, новые не создаются: вынеси старые в архив"),
            Self::NotFound => "задача не найдена".into(),
            Self::Broken => "файл задач повреждён: задачи временно не меняются (копия лежит рядом, todos.json.bak)".into(),
        }
    }
}

/// Хранилище задач: `todos.json` в папке данных. Не читается — запись выключена, файл не затирается.
pub struct Todos {
    path: PathBuf,
    inner: Mutex<Saved>,
    broken: AtomicBool,
}

impl Todos {
    pub fn open(data_dir: PathBuf) -> Self {
        let path = data_dir.join("todos.json");
        let mut broken = false;
        let saved = match std::fs::read_to_string(&path) {
            Ok(text) => serde_json::from_str(&text).unwrap_or_else(|e| {
                eprintln!("[todos] todos.json не читается ({e}); копия todos.json.bak, задачи не сохраняются");
                broken = true;
                Saved::default()
            }),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Saved::default(),
            Err(e) => {
                eprintln!("[todos] todos.json не читается ({e}); задачи не сохраняются");
                broken = true;
                Saved::default()
            }
        };
        if broken {
            let _ = std::fs::copy(&path, path.with_extension("json.bak"));
        }
        Self { path, inner: Mutex::new(saved), broken: AtomicBool::new(broken) }
    }

    fn writable(&self) -> Result<(), TodoError> {
        if self.broken.load(AtomicOrd::Relaxed) {
            Err(TodoError::Broken)
        } else {
            Ok(())
        }
    }

    fn save(&self, saved: &Saved) {
        if self.broken.load(AtomicOrd::Relaxed) {
            return;
        }
        let Ok(text) = serde_json::to_string_pretty(saved) else { return };
        let tmp = self.path.with_extension("json.tmp");
        if std::fs::write(&tmp, text).is_ok() {
            let _ = std::fs::rename(&tmp, &self.path);
        }
    }

    /// Незаархивированные задачи в порядке доски. Заодно уносит в архив выполненные до сегодня.
    pub fn list(&self, today: NaiveDate) -> Vec<Todo> {
        self.sweep(today);
        let saved = self.inner.lock().unwrap();
        let mut items: Vec<Todo> = saved.todos.iter().filter(|t| !t.archived).cloned().collect();
        items.sort_by(by_due);
        items
    }

    /// Архив: новые сверху.
    pub fn archived(&self) -> Vec<Todo> {
        let saved = self.inner.lock().unwrap();
        let mut items: Vec<Todo> = saved.todos.iter().filter(|t| t.archived).cloned().collect();
        items.sort_by(by_recent);
        items
    }

    /// Новая задача. Предел считается под тем же замком, что и вставка.
    pub fn add(&self, mut todo: Todo) -> Result<Todo, TodoError> {
        self.writable()?;
        let mut saved = self.inner.lock().unwrap();
        if saved.todos.iter().filter(|t| !t.archived).count() >= MAX_ACTIVE {
            return Err(TodoError::Limit);
        }
        todo.id = uid("todo");
        todo.created_at = crate::store::now();
        todo.done = false;
        todo.done_at = None;
        todo.archived = false;
        saved.todos.push(todo.clone());
        self.save(&saved);
        Ok(todo)
    }

    /// Изменение: при ошибке задача не меняется ни в памяти, ни на диске.
    pub fn update(&self, id: &str, change: impl FnOnce(&mut Todo) -> Result<(), TodoError>) -> Result<Todo, TodoError> {
        self.writable()?;
        let mut saved = self.inner.lock().unwrap();
        let index = find_open(&saved, id).ok_or(TodoError::NotFound)?;
        let mut copy = saved.todos[index].clone();
        change(&mut copy)?;
        saved.todos[index] = copy.clone();
        self.save(&saved);
        Ok(copy)
    }

    /// Удаление — это архив: задача уходит с доски, её можно вернуть.
    pub fn archive(&self, id: &str) -> Result<(), TodoError> {
        self.writable()?;
        let mut saved = self.inner.lock().unwrap();
        let index = find_open(&saved, id).ok_or(TodoError::NotFound)?;
        saved.todos[index].archived = true;
        self.save(&saved);
        Ok(())
    }

    /// Из архива обратно. Выполненная возвращается открытой: иначе её сразу унёс бы автоархив.
    pub fn restore(&self, id: &str) -> Result<Todo, TodoError> {
        self.writable()?;
        let mut saved = self.inner.lock().unwrap();
        let index = saved.todos.iter().position(|t| t.id == id && t.archived).ok_or(TodoError::NotFound)?;
        if saved.todos.iter().filter(|t| !t.archived).count() >= MAX_ACTIVE {
            return Err(TodoError::Limit);
        }
        let todo = &mut saved.todos[index];
        todo.archived = false;
        todo.done = false;
        todo.done_at = None;
        let restored = todo.clone();
        self.save(&saved);
        Ok(restored)
    }

    /// Отметить выполненной. Повтор создаёт следующий экземпляр; повторная отметка ничего не меняет.
    pub fn complete(&self, id: &str, today: NaiveDate, now: i64) -> Result<(Todo, Option<Todo>), TodoError> {
        self.writable()?;
        let mut saved = self.inner.lock().unwrap();
        let index = find_open(&saved, id).ok_or(TodoError::NotFound)?;
        let next = complete_in(&mut saved, index, today, now);
        let todo = saved.todos[index].clone();
        self.save(&saved);
        Ok((todo, next))
    }

    /// Снять отметку «выполнено».
    pub fn reopen(&self, id: &str) -> Result<Todo, TodoError> {
        self.writable()?;
        let mut saved = self.inner.lock().unwrap();
        let index = find_open(&saved, id).ok_or(TodoError::NotFound)?;
        let todo = &mut saved.todos[index];
        if todo.done {
            todo.done = false;
            todo.done_at = None;
        }
        let reopened = todo.clone();
        self.save(&saved);
        Ok(reopened)
    }

    /// Перенос просроченных неповторяющихся на сегодня. Возвращает, сколько перенесено.
    pub fn carry_over(&self, today: NaiveDate) -> Result<usize, TodoError> {
        self.writable()?;
        let mut saved = self.inner.lock().unwrap();
        let iso = today_iso(today);
        let mut count = 0;
        for todo in saved.todos.iter_mut().filter(|t| carry_over_candidate(t, &iso)) {
            todo.due_date = Some(iso.clone());
            count += 1;
        }
        if count > 0 {
            self.save(&saved);
        }
        Ok(count)
    }

    /// Автоархив: выполненные до сегодняшней полуночи. Возвращает, менялось ли что-то.
    pub fn sweep(&self, today: NaiveDate) -> bool {
        if self.writable().is_err() {
            return false;
        }
        let mut saved = self.inner.lock().unwrap();
        let changed = sweep_in(&mut saved, today);
        if changed {
            self.save(&saved);
        }
        changed
    }

    /// Автозакрытие: повторяющиеся задачи, у которых истёк срок и окно автозакрытия. Возвращает число закрытых.
    pub fn auto_close(&self, today: NaiveDate, now: i64) -> usize {
        if self.writable().is_err() {
            return 0;
        }
        let mut saved = self.inner.lock().unwrap();
        let due: Vec<usize> = saved
            .todos
            .iter()
            .enumerate()
            .filter(|(_, t)| !t.done && !t.archived && t.recurrence.is_some() && closes_at(t).is_some_and(|at| at <= now))
            .map(|(index, _)| index)
            .collect();
        for index in &due {
            complete_in(&mut saved, *index, today, now);
        }
        if !due.is_empty() {
            self.save(&saved);
        }
        due.len()
    }

    /// Номер или название (без учёта регистра) открытой задачи — для бота. Одинаковые названия не угадываются.
    pub fn resolve(&self, key: &str) -> Result<Todo, TodoError> {
        let key = key.trim();
        let saved = self.inner.lock().unwrap();
        if let Some(index) = find_open(&saved, key) {
            return Ok(saved.todos[index].clone());
        }
        let wanted = key.to_lowercase();
        let found: Vec<&Todo> = saved.todos.iter().filter(|t| !t.archived && !t.done && !wanted.is_empty() && t.title.to_lowercase() == wanted).collect();
        match found.as_slice() {
            [one] => Ok((*one).clone()),
            [] => Err(TodoError::NotFound),
            _ => Err(TodoError::Invalid("таких задач несколько — назови номер из list_todos".into())),
        }
    }
}

fn find_open(saved: &Saved, id: &str) -> Option<usize> {
    saved.todos.iter().position(|t| t.id == id && !t.archived)
}

fn today() -> NaiveDate {
    Local::now().date_naive()
}

fn today_iso(day: NaiveDate) -> String {
    day.format("%Y-%m-%d").to_string()
}

fn local_day(ms: i64) -> Option<NaiveDate> {
    Local.timestamp_millis_opt(ms).single().map(|at| at.date_naive())
}

fn sweep_in(saved: &mut Saved, today: NaiveDate) -> bool {
    let mut changed = false;
    for todo in saved.todos.iter_mut().filter(|t| t.done && !t.archived) {
        if todo.done_at.and_then(local_day).is_some_and(|day| day < today) {
            todo.archived = true;
            changed = true;
        }
    }
    changed
}

/// Порядок доски (`taskOrder.ts`): срок — дата, затем время; задача с датой без времени идёт после
/// задач того же дня со временем; без срока — в конце. При равенстве — новые сверху.
pub fn by_due(a: &Todo, b: &Todo) -> Ordering {
    match (&a.due_date, &b.due_date) {
        (Some(day_a), Some(day_b)) => {
            let day = day_a.cmp(day_b);
            if day != Ordering::Equal {
                return day;
            }
            match (&a.due_time, &b.due_time) {
                (Some(time_a), Some(time_b)) => {
                    let time = time_a.cmp(time_b);
                    if time != Ordering::Equal {
                        return time;
                    }
                }
                (Some(_), None) => return Ordering::Less,
                (None, Some(_)) => return Ordering::Greater,
                (None, None) => {}
            }
            by_recent(a, b)
        }
        (Some(_), None) => Ordering::Less,
        (None, Some(_)) => Ordering::Greater,
        (None, None) => by_recent(a, b),
    }
}

/// Новые сверху: created_at по убыванию, затем id.
pub fn by_recent(a: &Todo, b: &Todo) -> Ordering {
    b.created_at.cmp(&a.created_at).then_with(|| b.id.cmp(&a.id))
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Filter {
    All,
    Today,
    Overdue,
    NoDue,
}

impl Filter {
    pub fn parse(raw: &str) -> Result<Self, TodoError> {
        match raw.trim() {
            "" | "all" => Ok(Self::All),
            "today" => Ok(Self::Today),
            "overdue" => Ok(Self::Overdue),
            "nodue" => Ok(Self::NoDue),
            _ => Err(TodoError::Invalid("фильтр: all, today, overdue или nodue".into())),
        }
    }
}

/// Временной фильтр (`timeFilter.ts`). Даты — ГГГГ-ММ-ДД: строковое сравнение хронологическое.
pub fn matches_filter(todo: &Todo, filter: Filter, today: &str) -> bool {
    match filter {
        Filter::All => true,
        Filter::NoDue => todo.due_date.is_none(),
        Filter::Today => todo.due_date.as_deref() == Some(today),
        Filter::Overdue => todo.due_date.as_deref().is_some_and(|day| day < today),
    }
}

/// Кандидат на перенос (`carryOver.ts`): открытая, не повторяющаяся, просроченная.
pub fn carry_over_candidate(todo: &Todo, today: &str) -> bool {
    !todo.done && !todo.archived && todo.recurrence.is_none() && matches_filter(todo, Filter::Overdue, today)
}

/// Русское склонение слова «задача»: 1 задача, 2–4 задачи, 5–20 задач (с учётом 11–14).
pub fn plural_tasks(n: usize) -> &'static str {
    let mod100 = n % 100;
    if (11..=14).contains(&mod100) {
        return "задач";
    }
    match n % 10 {
        1 => "задача",
        2..=4 => "задачи",
        _ => "задач",
    }
}

fn digits(text: &str) -> String {
    text.chars().filter(|c| c.is_ascii_digit()).collect()
}

/// Поиск (`search.ts`): подстрока без учёта регистра по названию и заметке; номер телефона — по
/// последним цифрам (от 4 цифр в запросе), в любом формате записи. Пустой запрос ничего не находит.
pub fn matches_query(todo: &Todo, query: &str) -> bool {
    let needle = query.trim().to_lowercase();
    if needle.is_empty() {
        return false;
    }
    if todo.title.to_lowercase().contains(&needle) || todo.notes.to_lowercase().contains(&needle) {
        return true;
    }
    let wanted = digits(&needle);
    if wanted.chars().count() < 4 {
        return false;
    }
    let total = wanted.chars().count();
    let by_phone: String = wanted.chars().skip(total.saturating_sub(10)).collect();
    digits(&todo.title).contains(&by_phone)
        || digits(&todo.notes).contains(&by_phone)
        || digits(todo.phone.as_deref().unwrap_or_default()).contains(&by_phone)
}

fn deadline_ms(due_date: &str, due_time: &str) -> Option<i64> {
    let day = NaiveDate::parse_from_str(due_date, "%Y-%m-%d").ok()?;
    let time = NaiveTime::parse_from_str(&due_time.chars().take(5).collect::<String>(), "%H:%M").ok()?;
    Local.from_local_datetime(&day.and_time(time)).earliest().map(|at| at.timestamp_millis())
}

/// Конец окна автозакрытия: срок плюс минуты. Срок — локальное время компьютера.
pub fn closes_at_ms(due_date: Option<&str>, due_time: Option<&str>, after_min: Option<u32>) -> Option<i64> {
    let deadline = deadline_ms(due_date?, due_time?)?;
    let minutes = i64::from(after_min.filter(|m| *m > 0)?);
    Some(deadline + minutes * MINUTE_MS)
}

fn closes_at(todo: &Todo) -> Option<i64> {
    closes_at_ms(todo.due_date.as_deref(), todo.due_time.as_deref(), todo.auto_close_after_min)
}

/// Состояние окна автозакрытия (`autoClose.ts`): окно идёт, когда срок наступил, а закрытия ещё нет.
pub struct AutoClose {
    pub active: bool,
    /// «2:47» — часы:минуты до закрытия; только когда окно идёт.
    pub countdown: Option<String>,
}

pub fn auto_close_state(due_date: Option<&str>, due_time: Option<&str>, after_min: Option<u32>, now_ms: i64) -> AutoClose {
    let inactive = AutoClose { active: false, countdown: None };
    let (Some(day), Some(time)) = (due_date, due_time) else { return inactive };
    let (Some(deadline), Some(closes)) = (deadline_ms(day, time), closes_at_ms(due_date, due_time, after_min)) else { return inactive };
    if now_ms < deadline {
        return inactive;
    }
    let remaining = ((closes - now_ms) as f64 / MINUTE_MS as f64).round().max(0.0) as i64;
    AutoClose { active: true, countdown: Some(format!("{}:{:02}", remaining / 60, remaining % 60)) }
}

fn next_instance_day(todo: &Todo, today: NaiveDate) -> Option<NaiveDate> {
    let rule = todo.recurrence.as_ref()?;
    let base = todo.due_date.as_deref().and_then(|day| NaiveDate::parse_from_str(day, "%Y-%m-%d").ok()).unwrap_or(today);
    Some(dates_ru::next_occurrence_not_before(rule, base, today))
}

/// Отмечает задачу выполненной. Для повтора добавляет следующий экземпляр, если его ещё нет.
/// Пропущенные сроки не догоняются: следующий — первый срок, который не в прошлом.
fn complete_in(saved: &mut Saved, index: usize, today: NaiveDate, now: i64) -> Option<Todo> {
    if saved.todos[index].done {
        return None;
    }
    {
        let todo = &mut saved.todos[index];
        todo.done = true;
        todo.done_at = Some(now);
    }
    let template = saved.todos[index].clone();
    let next_key = today_iso(next_instance_day(&template, today)?);
    let exists = saved.todos.iter().any(|t| {
        !t.done && !t.archived && t.title == template.title && t.recurrence == template.recurrence && t.due_date.as_deref() == Some(next_key.as_str())
    });
    if exists {
        return None;
    }
    let items = template.items.iter().map(|item| Item { done: false, ..item.clone() }).collect();
    let next = Todo {
        id: uid("todo"),
        due_date: Some(next_key),
        done: false,
        done_at: None,
        archived: false,
        created_at: now,
        items,
        ..template
    };
    saved.todos.push(next.clone());
    Some(next)
}

/// Тик планировщика: автоархив и автозакрытие. Возвращает, менялось ли что-то.
pub fn tick(app: &App) {
    let day = today();
    let now = crate::store::now();
    let swept = app.todos.sweep(day);
    let closed = app.todos.auto_close(day, now);
    if swept || closed > 0 {
        changed(app);
    }
}

fn parse_date(raw: &str) -> Result<String, TodoError> {
    NaiveDate::parse_from_str(raw.trim(), "%Y-%m-%d")
        .map(today_iso)
        .map_err(|_| TodoError::Invalid("дата в формате ГГГГ-ММ-ДД".into()))
}

fn parse_time(raw: &str) -> Result<String, TodoError> {
    let text = raw.trim();
    NaiveTime::parse_from_str(text, "%H:%M")
        .or_else(|_| NaiveTime::parse_from_str(text, "%H:%M:%S"))
        .map(|time| time.format("%H:%M").to_string())
        .map_err(|_| TodoError::Invalid("время в формате ЧЧ:ММ".into()))
}

fn clean_title(raw: &str) -> Result<String, TodoError> {
    let title = raw.trim();
    if title.is_empty() {
        return Err(TodoError::Invalid("нужно название задачи".into()));
    }
    if title.chars().count() > MAX_TITLE {
        return Err(TodoError::Invalid(format!("название до {MAX_TITLE} символов")));
    }
    Ok(title.to_string())
}

fn clean_notes(raw: &str) -> Result<String, TodoError> {
    let notes = raw.trim();
    if notes.chars().count() > MAX_NOTES {
        return Err(TodoError::Invalid(format!("заметка до {MAX_NOTES} символов")));
    }
    Ok(notes.to_string())
}

fn items_field(value: &Value) -> Result<Vec<Item>, TodoError> {
    if value.is_null() {
        return Ok(Vec::new());
    }
    let list = value.as_array().ok_or_else(|| TodoError::Invalid("чек-лист — список пунктов".into()))?;
    if list.len() > MAX_ITEMS {
        return Err(TodoError::Invalid(format!("пунктов в чек-листе до {MAX_ITEMS}")));
    }
    list.iter()
        .map(|raw| {
            let text = raw.get("text").and_then(Value::as_str).unwrap_or_default().trim().to_string();
            if text.is_empty() {
                return Err(TodoError::Invalid("пункт чек-листа без текста".into()));
            }
            if text.chars().count() > MAX_ITEM_TEXT {
                return Err(TodoError::Invalid(format!("пункт чек-листа до {MAX_ITEM_TEXT} символов")));
            }
            Ok(Item { id: uid("item"), text, done: raw.get("done").and_then(Value::as_bool).unwrap_or(false) })
        })
        .collect()
}

fn json_body(bytes: &Bytes) -> Result<Value, TodoError> {
    if bytes.is_empty() {
        return Ok(json!({}));
    }
    serde_json::from_slice(bytes).map_err(|_| TodoError::Invalid("тело запроса — JSON-объект".into()))
}

fn text_field(body: &Value, key: &str) -> Result<Option<String>, TodoError> {
    match body.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(text)) => Ok(Some(text.clone())),
        Some(_) => Err(TodoError::Invalid(format!("поле {key} должно быть строкой"))),
    }
}

fn number_field(body: &Value, key: &str) -> Result<Option<i64>, TodoError> {
    match body.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(value) => value.as_i64().map(Some).ok_or_else(|| TodoError::Invalid(format!("поле {key} должно быть целым числом"))),
    }
}

/// Исполнитель — только бот из списка команды.
fn assignee_field(app: &App, id: &str) -> Result<String, TodoError> {
    if app.store.bot(id).is_some() {
        Ok(id.to_string())
    } else {
        Err(TodoError::Invalid("исполнитель — только бот из списка команды".into()))
    }
}

/// Поля из тела запроса. Ключа, которого нет в теле, не касается; null очищает поле.
fn apply(app: &App, todo: &mut Todo, body: &Value) -> Result<(), TodoError> {
    if let Some(title) = text_field(body, "title")? {
        todo.title = clean_title(&title)?;
    }
    if let Some(bucket) = text_field(body, "bucket")? {
        todo.bucket = Bucket::parse(&bucket)?;
    }
    if body.get("due_date").is_some() {
        todo.due_date = text_field(body, "due_date")?.map(|raw| parse_date(&raw)).transpose()?;
    }
    if body.get("due_time").is_some() {
        todo.due_time = text_field(body, "due_time")?.map(|raw| parse_time(&raw)).transpose()?;
    }
    if let Some(notes) = text_field(body, "notes")? {
        todo.notes = clean_notes(&notes)?;
    }
    if let Some(value) = body.get("recurrence") {
        todo.recurrence = if value.is_null() {
            None
        } else {
            Some(serde_json::from_value::<Recurrence>(value.clone()).map_err(|_| TodoError::Invalid("повтор: freq daily, weekly, monthly или yearly".into()))?)
        };
    }
    if let Some(value) = body.get("items") {
        todo.items = items_field(value)?;
    }
    if body.get("assignee").is_some() {
        todo.assignee = text_field(body, "assignee")?.map(|id| assignee_field(app, &id)).transpose()?;
    }
    if body.get("auto_close_after_min").is_some() {
        todo.auto_close_after_min = match number_field(body, "auto_close_after_min")? {
            None => None,
            Some(minutes) if (MIN_AUTO_CLOSE..=MAX_AUTO_CLOSE).contains(&minutes) => Some(minutes as u32),
            Some(_) => return Err(TodoError::Invalid(format!("автозакрытие от {MIN_AUTO_CLOSE} до {MAX_AUTO_CLOSE} минут"))),
        };
    }
    if todo.auto_close_after_min.is_some() && todo.recurrence.is_none() {
        return Err(TodoError::Invalid("автозакрытие бывает только у повторяющейся задачи".into()));
    }
    Ok(())
}

/// Задача из фразы: название, срок и повтор — кодом, без модели.
fn phrase_todo(text: &str, today: NaiveDate, now: NaiveTime) -> Result<Todo, TodoError> {
    let parsed = dates_ru::parse(text.trim(), today, now);
    Ok(Todo {
        title: clean_title(&parsed.title)?,
        due_date: parsed.due_date.map(today_iso),
        due_time: parsed.due_time.map(|time| time.format("%H:%M").to_string()),
        recurrence: parsed.recurrence,
        ..Todo::default()
    })
}

/// Новая задача из тела запроса: либо `text` (фраза), либо поля. Поля из тела поверх фразы.
fn build_new(app: &App, body: &Value) -> Result<Todo, TodoError> {
    let now = Local::now();
    let mut todo = match text_field(body, "text")? {
        Some(text) if !text.trim().is_empty() => phrase_todo(&text, now.date_naive(), now.time())?,
        _ => Todo::default(),
    };
    apply(app, &mut todo, body)?;
    if todo.title.is_empty() {
        return Err(TodoError::Invalid("нужно название задачи или текст".into()));
    }
    Ok(todo)
}

pub fn view(todo: &Todo, now: i64) -> Value {
    let state = auto_close_state(todo.due_date.as_deref(), todo.due_time.as_deref(), todo.auto_close_after_min, now);
    json!({
        "id": todo.id,
        "title": todo.title,
        "bucket": todo.bucket,
        "due_date": todo.due_date,
        "due_time": todo.due_time,
        "notes": todo.notes,
        "recurrence": todo.recurrence,
        "items": todo.items,
        "assignee": todo.assignee,
        "auto_close_after_min": todo.auto_close_after_min,
        "address": todo.address,
        "phone": todo.phone,
        "auto_close": { "active": state.active, "countdown": state.countdown },
        "done": todo.done,
        "done_at": todo.done_at,
        "archived": todo.archived,
        "created_at": todo.created_at,
    })
}

pub fn respond(result: Result<(StatusCode, Value), TodoError>) -> Response {
    match result {
        Ok((status, value)) => (status, Json(value)).into_response(),
        Err(error) => (error.status(), Json(json!({ "error": error.code(), "message": error.message() }))).into_response(),
    }
}

pub fn changed(app: &App) {
    app.turns.emit(json!({ "type": "todos_changed" }));
}

/// Список для окна и бота: фильтр по сроку и поиск по тексту.
fn visible(app: &App, filter: Filter, query: &str, day: NaiveDate) -> Vec<Todo> {
    let iso = today_iso(day);
    let needle = query.trim();
    app.todos
        .list(day)
        .into_iter()
        .filter(|todo| matches_filter(todo, filter, &iso))
        .filter(|todo| needle.is_empty() || matches_query(todo, needle))
        .collect()
}

#[derive(Deserialize)]
pub struct ListQuery {
    filter: Option<String>,
    q: Option<String>,
}

#[derive(Deserialize)]
pub struct ArchiveQuery {
    q: Option<String>,
}

type Shared = State<Arc<App>>;

type Reply = Result<(StatusCode, Value), TodoError>;

fn list(app: &App, query: &ListQuery) -> Reply {
    let filter = Filter::parse(query.filter.as_deref().unwrap_or("all"))?;
    let now = crate::store::now();
    let items: Vec<Value> = visible(app, filter, query.q.as_deref().unwrap_or_default(), today()).iter().map(|t| view(t, now)).collect();
    Ok((StatusCode::OK, Value::Array(items)))
}

async fn list_route(State(app): Shared, Query(query): Query<ListQuery>) -> Response {
    respond(list(&app, &query))
}

async fn archive_list_route(State(app): Shared, Query(query): Query<ArchiveQuery>) -> Response {
    let needle = query.q.unwrap_or_default();
    let now = crate::store::now();
    let items: Vec<Value> = app
        .todos
        .archived()
        .iter()
        .filter(|todo| needle.trim().is_empty() || matches_query(todo, &needle))
        .map(|todo| view(todo, now))
        .collect();
    (StatusCode::OK, Json(Value::Array(items))).into_response()
}

fn create(app: &App, bytes: &Bytes) -> Reply {
    let todo = build_new(app, &json_body(bytes)?)?;
    let made = app.todos.add(todo)?;
    changed(app);
    Ok((StatusCode::CREATED, view(&made, crate::store::now())))
}

async fn create_route(State(app): Shared, body: Bytes) -> Response {
    respond(create(&app, &body))
}

fn edit(app: &App, id: &str, bytes: &Bytes) -> Reply {
    let body = json_body(bytes)?;
    let updated = app.todos.update(id, |todo| apply(app, todo, &body))?;
    changed(app);
    Ok((StatusCode::OK, view(&updated, crate::store::now())))
}

async fn patch_route(State(app): Shared, Path(id): Path<String>, body: Bytes) -> Response {
    respond(edit(&app, &id, &body))
}

async fn archive_route(State(app): Shared, Path(id): Path<String>) -> Response {
    match app.todos.archive(&id) {
        Ok(()) => {
            changed(&app);
            StatusCode::NO_CONTENT.into_response()
        }
        Err(error) => respond(Err(error)),
    }
}

async fn restore_route(State(app): Shared, Path(id): Path<String>) -> Response {
    respond(match app.todos.restore(&id) {
        Ok(todo) => {
            changed(&app);
            Ok((StatusCode::OK, view(&todo, crate::store::now())))
        }
        Err(error) => Err(error),
    })
}

fn complete(app: &App, id: &str) -> Reply {
    let (todo, next) = app.todos.complete(id, today(), crate::store::now())?;
    changed(app);
    let now = crate::store::now();
    Ok((StatusCode::OK, json!({ "todo": view(&todo, now), "next": next.as_ref().map(|n| view(n, now)) })))
}

async fn done_route(State(app): Shared, Path(id): Path<String>) -> Response {
    respond(complete(&app, &id))
}

async fn undone_route(State(app): Shared, Path(id): Path<String>) -> Response {
    respond(match app.todos.reopen(&id) {
        Ok(todo) => {
            changed(&app);
            Ok((StatusCode::OK, view(&todo, crate::store::now())))
        }
        Err(error) => Err(error),
    })
}

fn carry(app: &App) -> Reply {
    let count = app.todos.carry_over(today())?;
    if count > 0 {
        changed(app);
    }
    let text = if count == 0 { "Переносить нечего.".to_string() } else { format!("Перенёс {count} {} на сегодня.", plural_tasks(count)) };
    Ok((StatusCode::OK, json!({ "count": count, "text": text })))
}

async fn carry_over_route(State(app): Shared) -> Response {
    respond(carry(&app))
}

/// Маршруты задач. Менять их может только окно человека (см. `guard::todo_write`).
pub fn routes() -> Router<Arc<App>> {
    Router::new()
        .route("/api/todos", get(list_route).post(create_route))
        .route("/api/todos/archive", get(archive_list_route))
        .route("/api/todos/carry-over", post(carry_over_route))
        // Разбор фразы моделью: предпросмотр и сохранение. Оба вызова — расход денег, решает человек (guard::todo_write).
        .route("/api/todos/parse", post(crate::todo_parse::parse_route))
        .route("/api/todos/from-text", post(crate::todo_parse::from_text_route))
        .route("/api/todos/{id}", patch(patch_route).delete(archive_route))
        .route("/api/todos/{id}/done", post(done_route).delete(undone_route))
        .route("/api/todos/{id}/restore", post(restore_route))
}

/// Инструменты бота: список, добавление и отметка задач человека. Подтверждения не нужны:
/// человек сам попросил бота и задачи идут в его собственный список.
pub fn is_bot_tool(name: &str) -> bool {
    matches!(name, "list_todos" | "add_todo" | "complete_todo")
}

pub fn bot_tool_defs() -> Vec<Value> {
    vec![
        json!({
            "type": "function",
            "function": {
                "name": "list_todos",
                "description": "Показать задачи человека: номер, название, срок, выполнено ли. Фильтр: all, today, overdue (просроченные), nodue (без срока).",
                "parameters": {
                    "type": "object",
                    "properties": { "filter": { "type": "string", "enum": ["all", "today", "overdue", "nodue"] } },
                },
            },
        }),
        json!({
            "type": "function",
            "function": {
                "name": "add_todo",
                "description": "Добавить задачу в список человека. Проще всего передать text фразой, как он сказал (\"позвонить маме завтра в 10\"): срок и повтор распознаются кодом. Или задать title, due_date (ГГГГ-ММ-ДД), due_time (ЧЧ:ММ), bucket (key, active, someday). Подтверждение не нужно.",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "text": { "type": "string", "description": "Фраза задачи как сказал человек." },
                        "title": { "type": "string", "description": "Название, если фраза не нужна." },
                        "due_date": { "type": "string", "description": "Срок ГГГГ-ММ-ДД." },
                        "due_time": { "type": "string", "description": "Время ЧЧ:ММ." },
                        "bucket": { "type": "string", "enum": ["key", "active", "someday"] },
                    },
                },
            },
        }),
        json!({
            "type": "function",
            "function": {
                "name": "complete_todo",
                "description": "Отметить задачу человека выполненной. Повторяющаяся задача сама создаст следующий срок. Подтверждение не нужно.",
                "parameters": {
                    "type": "object",
                    "properties": { "id": { "type": "string", "description": "Номер задачи из list_todos или её название." } },
                    "required": ["id"],
                },
            },
        }),
    ]
}

fn bot_line(todo: &Todo) -> String {
    let due = match (&todo.due_date, &todo.due_time) {
        (Some(day), Some(time)) => format!(" · срок {day} {time}"),
        (Some(day), None) => format!(" · срок {day}"),
        _ => String::new(),
    };
    let state = if todo.done { " · выполнено" } else { "" };
    format!("{} · {}{due}{state}", todo.id, todo.title)
}

/// Выполнить инструмент бота. Ошибки — текстом для модели.
pub fn exec_bot_tool(app: &App, name: &str, args: &Value) -> Result<String, String> {
    let now = Local::now();
    match name {
        "list_todos" => {
            let filter = Filter::parse(crate::routine_tools::text(args, "filter")).map_err(|e| e.message())?;
            let items = visible(app, filter, "", now.date_naive());
            if items.is_empty() {
                return Ok("Задач нет.".into());
            }
            Ok(items.iter().map(bot_line).collect::<Vec<_>>().join("\n"))
        }
        "add_todo" => {
            let todo = build_new(app, args).map_err(|e| e.message())?;
            let made = app.todos.add(todo).map_err(|e| e.message())?;
            changed(app);
            Ok(format!("Добавил задачу «{}»{}. Номер: {}.", made.title, made.due_date.as_deref().map(|d| format!(", срок {d}")).unwrap_or_default(), made.id))
        }
        _ => {
            let key = crate::routine_tools::text(args, "id");
            if key.is_empty() {
                return Err("нужен номер или название задачи (list_todos)".into());
            }
            let todo = app.todos.resolve(key).map_err(|e| e.message())?;
            let (done, next) = app.todos.complete(&todo.id, now.date_naive(), crate::store::now()).map_err(|e| e.message())?;
            changed(app);
            let tail = match next.and_then(|n| n.due_date) {
                Some(day) => format!(" Следующий срок: {day}."),
                None => String::new(),
            };
            Ok(format!("Отметил «{}» выполненной.{tail}", done.title))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Datelike;

    fn day(text: &str) -> NaiveDate {
        NaiveDate::parse_from_str(text, "%Y-%m-%d").unwrap()
    }

    /// Папка внутри `target` этой копии: системный TEMP здесь недоступен для записи.
    fn fresh_dir(name: &str) -> PathBuf {
        let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("target").join("todos-tests").join(format!("{name}-{}", uid("t")));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn mk(id: &str, due: Option<&str>, time: Option<&str>, created: i64) -> Todo {
        Todo { id: id.into(), title: "task".into(), due_date: due.map(Into::into), due_time: time.map(Into::into), created_at: created, ..Todo::default() }
    }

    fn task(title: &str) -> Todo {
        Todo { title: title.into(), ..Todo::default() }
    }

    fn weekly(weekdays: Vec<u8>) -> Option<Recurrence> {
        Some(Recurrence { freq: dates_ru::Freq::Weekly, interval: 1, weekdays, monthday: None })
    }

    fn at(hh: u32, mm: u32) -> i64 {
        Local.with_ymd_and_hms(2026, 6, 19, hh, mm, 0).single().unwrap().timestamp_millis()
    }

    /// Полдень выбранного дня: момент отметки для тестов, где важна дата выполнения.
    fn at_ms(year: i32, month: u32, day: u32) -> i64 {
        Local.with_ymd_and_hms(year, month, day, 12, 0, 0).single().unwrap().timestamp_millis()
    }

    fn order(todos: &mut [Todo]) -> Vec<String> {
        todos.sort_by(by_due);
        todos.iter().map(|t| t.id.clone()).collect()
    }

    #[test]
    fn one_day_sorts_by_time_not_by_creation() {
        // Сценарий бага 13.09.2026: 10:00, 10:00, 13:00, 09:00 на «завтра».
        let d = Some("2026-09-14");
        let mut tasks = vec![mk("1", d, Some("10:00"), 4), mk("2", d, Some("10:00"), 3), mk("3", d, Some("13:00"), 2), mk("4", d, Some("09:00"), 1)];
        assert_eq!(order(&mut tasks), ["4", "1", "2", "3"]);
    }

    #[test]
    fn earlier_day_goes_first_even_with_later_time() {
        let mut tasks = vec![mk("1", Some("2026-09-15"), Some("08:00"), 1), mk("2", Some("2026-09-14"), Some("20:00"), 1)];
        assert_eq!(order(&mut tasks), ["2", "1"]);
    }

    #[test]
    fn no_time_goes_after_timed_tasks_of_the_same_day() {
        let mut tasks = vec![mk("1", Some("2026-09-14"), None, 1), mk("2", Some("2026-09-14"), Some("23:30"), 1)];
        assert_eq!(order(&mut tasks), ["2", "1"]);
    }

    #[test]
    fn no_due_goes_last_and_newest_first() {
        let mut tasks = vec![mk("1", Some("2026-12-31"), None, 1), mk("2", None, None, 2), mk("3", None, None, 3)];
        assert_eq!(order(&mut tasks), ["1", "3", "2"]);
    }

    #[test]
    fn same_due_is_newest_first_and_recent_is_id_desc() {
        let mut tasks = vec![mk("1", Some("2026-09-14"), Some("10:00"), 1), mk("2", Some("2026-09-14"), Some("10:00"), 2)];
        assert_eq!(order(&mut tasks), ["2", "1"]);
        let mut same = vec![mk("a", None, None, 5), mk("b", None, None, 5), mk("c", None, None, 1)];
        assert_eq!(order(&mut same), ["b", "a", "c"]);
    }

    #[test]
    fn carry_over_takes_open_overdue_non_repeating() {
        let today = "2026-07-14";
        let overdue = Todo { due_date: Some("2026-07-13".into()), ..task("a") };
        assert!(carry_over_candidate(&overdue, today));
        assert!(!carry_over_candidate(&Todo { done: true, ..overdue.clone() }, today), "выполненная");
        assert!(!carry_over_candidate(&Todo { recurrence: weekly(vec![1]), ..overdue.clone() }, today), "повторяющаяся");
        assert!(!carry_over_candidate(&Todo { due_date: Some("2026-07-14".into()), ..task("b") }, today), "сегодня");
        assert!(!carry_over_candidate(&Todo { due_date: Some("2026-07-20".into()), ..task("c") }, today), "будущая");
        assert!(!carry_over_candidate(&task("d"), today), "без срока");
    }

    #[test]
    fn plural_forms_are_russian() {
        assert_eq!(plural_tasks(1), "задача");
        assert_eq!(plural_tasks(2), "задачи");
        assert_eq!(plural_tasks(4), "задачи");
        assert_eq!(plural_tasks(5), "задач");
        assert_eq!(plural_tasks(11), "задач");
        assert_eq!(plural_tasks(12), "задач");
        assert_eq!(plural_tasks(21), "задача");
        assert_eq!(plural_tasks(22), "задачи");
        assert_eq!(plural_tasks(25), "задач");
    }

    #[test]
    fn search_by_title_notes_and_partial_match() {
        let doctor = task("Позвонить врачу");
        let bread = Todo { notes: "в пекарне у дома".into(), ..task("Купить хлеб") };
        assert!(matches_query(&doctor, "ВРАЧ"));
        assert!(matches_query(&bread, "пекарн"));
        assert!(!matches_query(&bread, "самолёт"));
        assert!(!matches_query(&doctor, "   "), "пустой запрос ничего не находит");
    }

    #[test]
    fn search_finds_phone_digits_in_any_format() {
        let phone = task("Позвонить +79001234567");
        assert!(matches_query(&phone, "9001234567"));
        assert!(matches_query(&phone, "8 900 123-45-67"));
        assert!(matches_query(&phone, "1234567"));
        assert!(!matches_query(&phone, "9998887766"));
        assert!(!matches_query(&task("Купить +79001234567"), "9 00"), "меньше четырёх цифр — не поиск номера");
    }

    #[test]
    fn auto_close_is_inactive_without_date_time_or_minutes() {
        let day = Some("2026-06-19");
        assert!(!auto_close_state(None, Some("10:00"), Some(180), at(10, 0)).active);
        assert!(!auto_close_state(day, None, Some(180), at(10, 0)).active);
        assert!(!auto_close_state(day, Some("10:00"), None, at(10, 0)).active);
    }

    #[test]
    fn auto_close_waits_for_the_deadline() {
        let state = auto_close_state(Some("2026-06-19"), Some("10:00"), Some(180), at(9, 59));
        assert!(!state.active);
        assert!(state.countdown.is_none());
    }

    #[test]
    fn auto_close_counts_down_to_the_window_end() {
        let at_deadline = auto_close_state(Some("2026-06-19"), Some("10:00"), Some(180), at(10, 0));
        assert!(at_deadline.active);
        assert_eq!(at_deadline.countdown.as_deref(), Some("3:00"));
        assert_eq!(auto_close_state(Some("2026-06-19"), Some("10:00"), Some(180), at(11, 13)).countdown.as_deref(), Some("1:47"));
        assert_eq!(auto_close_state(Some("2026-06-19"), Some("10:00"), Some(60), at(10, 51)).countdown.as_deref(), Some("0:09"));
    }

    #[test]
    fn auto_close_reads_seconds_in_time_and_never_goes_negative() {
        assert_eq!(auto_close_state(Some("2026-06-19"), Some("10:00:00"), Some(180), at(11, 13)).countdown.as_deref(), Some("1:47"));
        let late = auto_close_state(Some("2026-06-19"), Some("10:00"), Some(60), at(11, 5));
        assert!(late.active);
        assert_eq!(late.countdown.as_deref(), Some("0:00"));
    }

    #[test]
    fn repeat_creates_next_instance_once() {
        let store = Todos::open(fresh_dir("next"));
        let made = store.add(Todo { recurrence: weekly(vec![3]), due_date: Some("2026-09-09".into()), ..task("Вынести мусор") }).unwrap();
        let (done, next) = store.complete(&made.id, day("2026-09-09"), at_ms(2026, 9, 9)).unwrap();
        assert!(done.done);
        let next = next.expect("следующий экземпляр");
        assert_eq!(next.due_date.as_deref(), Some("2026-09-16"), "следующая среда");
        assert!(!next.done);
        let (_, again) = store.complete(&made.id, day("2026-09-09"), at_ms(2026, 9, 9)).unwrap();
        assert!(again.is_none(), "повторная отметка не создаёт второй экземпляр");
        let (_, twice) = store.complete(&next.id, day("2026-09-16"), at_ms(2026, 9, 16)).unwrap();
        assert!(twice.is_some(), "следующий шаг ряда создаётся от нового экземпляра");
        // 16-го сентября выполненный 9-го уже в архиве; видны выполненный 16-го и следующий 23-го.
        let shown = store.list(day("2026-09-16"));
        assert_eq!(shown.len(), 2, "без дублей: выполненная сегодня и следующая");
        assert_eq!(store.archived().len(), 1);
    }

    #[test]
    fn missed_instances_are_not_caught_up() {
        let rule = Some(Recurrence { freq: dates_ru::Freq::Weekly, interval: 1, weekdays: vec![3], monthday: None });
        let base = Todo { recurrence: rule, due_date: Some("2026-09-02".into()), ..task("Зал") };
        let day_of = next_instance_day(&base, day("2026-09-10")).unwrap();
        assert_eq!(day_of, day("2026-09-16"), "среды 2 и 9 сентября пропущены, не догоняются");
        assert_eq!(day_of.weekday().num_days_from_monday(), 2);
    }

    #[test]
    fn carry_over_moves_overdue_and_counts_them() {
        let store = Todos::open(fresh_dir("carry"));
        store.add(Todo { due_date: Some("2026-09-01".into()), ..task("старая")}).unwrap();
        store.add(Todo { due_date: Some("2026-09-02".into()), ..task("ещё старее")}).unwrap();
        store.add(Todo { due_date: Some("2026-09-01".into()), recurrence: weekly(vec![1]), ..task("повтор") }).unwrap();
        assert_eq!(store.carry_over(day("2026-09-10")).unwrap(), 2);
        assert_eq!(store.carry_over(day("2026-09-10")).unwrap(), 0, "второй раз переносить нечего");
        let moved = store.list(day("2026-09-10"));
        assert_eq!(moved.iter().filter(|t| t.due_date.as_deref() == Some("2026-09-10")).count(), 2);
    }

    #[test]
    fn done_before_today_is_archived_and_restore_reopens_it() {
        let store = Todos::open(fresh_dir("sweep"));
        let yesterday = Local::now().date_naive().pred_opt().unwrap();
        let made = store.add(task("вчерашняя")).unwrap();
        let long_ago = Local.from_local_datetime(&yesterday.and_hms_opt(12, 0, 0).unwrap()).earliest().unwrap().timestamp_millis();
        store.complete(&made.id, yesterday, long_ago).unwrap();
        let today = Local::now().date_naive();
        assert!(store.list(today).is_empty(), "после полуночи в архиве");
        assert_eq!(store.archived().len(), 1);
        let back = store.restore(&made.id).unwrap();
        assert!(!back.done && !back.archived, "вернулась открытой");
        assert_eq!(store.list(today).len(), 1);
    }

    #[test]
    fn archive_and_restore_round_trip() {
        let store = Todos::open(fresh_dir("archive"));
        let made = store.add(task("удалю")).unwrap();
        store.archive(&made.id).unwrap();
        assert!(store.list(day("2026-09-10")).is_empty());
        assert!(matches!(store.archive(&made.id), Err(TodoError::NotFound)));
        store.restore(&made.id).unwrap();
        assert_eq!(store.list(day("2026-09-10")).len(), 1);
    }

    #[test]
    fn the_limit_is_two_thousand_active_and_archive_frees_a_slot() {
        let store = Todos {
            path: fresh_dir("limit").join("todos.json"),
            inner: Mutex::new(Saved { todos: (0..MAX_ACTIVE).map(|n| Todo { id: format!("t{n}"), title: "x".into(), ..Todo::default() }).collect() }),
            broken: AtomicBool::new(false),
        };
        assert!(matches!(store.add(task("лишняя")), Err(TodoError::Limit)));
        store.archive("t0").unwrap();
        assert!(store.add(task("новая")).is_ok());
    }

    #[test]
    fn bot_resolves_by_title_but_not_an_ambiguous_one() {
        let store = Todos::open(fresh_dir("resolve"));
        store.add(task("Позвонить маме")).unwrap();
        store.add(task("позвонить маме")).unwrap();
        store.add(task("Купить хлеб")).unwrap();
        assert_eq!(store.resolve("купить ХЛЕБ").unwrap().title, "Купить хлеб");
        assert!(matches!(store.resolve("Позвонить маме"), Err(TodoError::Invalid(_))));
        assert!(matches!(store.resolve("нет такой"), Err(TodoError::NotFound)));
    }

    #[test]
    fn phrase_sets_title_date_and_time() {
        // «в 10 часов»: голое «в 10» dates_ru временем не считает (правило «ложное время хуже отсутствующего»).
        let parsed = phrase_todo("позвонить маме завтра в 10 часов", day("2026-09-10"), NaiveTime::from_hms_opt(8, 0, 0).unwrap()).unwrap();
        assert_eq!(parsed.title.to_lowercase(), "позвонить маме");
        assert_eq!(parsed.due_date.as_deref(), Some("2026-09-11"));
        assert_eq!(parsed.due_time.as_deref(), Some("10:00"));
    }

}
