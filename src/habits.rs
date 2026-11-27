//! Привычки: повторяющиеся маленькие дела с отметками по дням.
//! Расписание — маска дней недели: бит 0 = понедельник … бит 6 = воскресенье, 127 = каждый день.
//! Отметка — пара (привычка, день): повторная отметка того же дня ничего не добавляет.
//! Серия считает только расписанные дни: пропуск нерасписанного дня её не рвёт.
//! Порт `habits.py` из планировщика (0.3.5). Дни — по часам компьютера человека.

use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use axum::body::Bytes;
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::{get, patch, post};
use axum::{Json, Router};
use chrono::{Datelike, Local, NaiveDate, NaiveTime};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::store::uid;
use crate::App;

pub const MAX_HABITS: usize = 50;
const MAX_TITLE: usize = 80;
const MAX_CAPTION: usize = 200;
const EVERY_DAY: u8 = 127;
const WEEK: usize = 7;
/// Десять лет подряд — с запасом для любой серии.
const STREAK_LIMIT: usize = 3660;

/// Иконки из набора приложения. Годится и один эмодзи (короткая строка не из ASCII).
pub const ICON_NAMES: &[&str] = &[
    "drop", "sun", "moon", "book", "pen", "leaf", "coffee", "heart", "brain", "check", "star", "home", "bed", "apple", "dumbbell", "run", "music", "phone",
];

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Habit {
    pub id: String,
    pub title: String,
    pub caption: String,
    pub icon: String,
    pub days_mask: u8,
    pub created_at: i64,
    pub archived: bool,
    /// Время напоминания «ЧЧ:ММ»; нет — напоминаний не шлём.
    pub remind_at: Option<String>,
    /// Каналы напоминания, как у напоминаний: windows или telegram. Пусто в старом файле — windows.
    pub remind_channels: Vec<String>,
    /// День ГГГГ-ММ-ДД, когда напоминание уже ушло: в один день — одно напоминание.
    pub last_reminded: Option<String>,
}

/// Напоминание, которое пора отправить: текст и каналы. Отправляет планировщик (`remind_tick`).
#[derive(Debug, Clone, PartialEq)]
pub struct DueReminder {
    pub title: String,
    pub body: String,
    pub channels: Vec<String>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Check {
    pub habit_id: String,
    /// ГГГГ-ММ-ДД.
    pub date: String,
}

#[derive(Default, Serialize, Deserialize)]
#[serde(default)]
struct Saved {
    habits: Vec<Habit>,
    checks: Vec<Check>,
}

#[derive(Debug)]
pub enum HabitError {
    Invalid(String),
    Limit,
    NotFound,
    Future,
    Broken,
}

impl HabitError {
    fn status(&self) -> StatusCode {
        match self {
            Self::Invalid(_) | Self::Future => StatusCode::UNPROCESSABLE_ENTITY,
            Self::Limit => StatusCode::CONFLICT,
            Self::NotFound => StatusCode::NOT_FOUND,
            Self::Broken => StatusCode::CONFLICT,
        }
    }

    fn code(&self) -> &'static str {
        match self {
            Self::Invalid(_) => "invalid",
            Self::Limit => "habit_limit",
            Self::NotFound => "not_found",
            Self::Future => "future_day",
            Self::Broken => "storage_broken",
        }
    }

    pub fn message(&self) -> String {
        match self {
            Self::Invalid(message) => message.clone(),
            Self::Limit => format!("уже {MAX_HABITS} привычек, новые не создаются"),
            Self::NotFound => "привычка не найдена".into(),
            Self::Future => "нельзя отметить будущий день".into(),
            Self::Broken => "файл привычек повреждён: привычки временно не меняются (копия лежит рядом, habits.json.bak)".into(),
        }
    }
}

/// Хранилище привычек: `habits.json` в папке данных. Не читается — запись выключена, файл не затирается.
pub struct Habits {
    path: PathBuf,
    inner: Mutex<Saved>,
    broken: AtomicBool,
}

impl Habits {
    pub fn open(data_dir: PathBuf) -> Self {
        let path = data_dir.join("habits.json");
        let mut broken = false;
        let saved = match std::fs::read_to_string(&path) {
            Ok(text) => serde_json::from_str(&text).unwrap_or_else(|e| {
                eprintln!("[habits] habits.json не читается ({e}); копия habits.json.bak, привычки не сохраняются");
                broken = true;
                Saved::default()
            }),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Saved::default(),
            Err(e) => {
                eprintln!("[habits] habits.json не читается ({e}); привычки не сохраняются");
                broken = true;
                Saved::default()
            }
        };
        if broken {
            let _ = std::fs::copy(&path, path.with_extension("json.bak"));
        }
        Self { path, inner: Mutex::new(saved), broken: AtomicBool::new(broken) }
    }

    fn writable(&self) -> Result<(), HabitError> {
        if self.broken.load(Ordering::Relaxed) {
            Err(HabitError::Broken)
        } else {
            Ok(())
        }
    }

    fn save(&self, saved: &Saved) {
        if self.broken.load(Ordering::Relaxed) {
            return;
        }
        let Ok(text) = serde_json::to_string_pretty(saved) else { return };
        let tmp = self.path.with_extension("json.tmp");
        if std::fs::write(&tmp, text).is_ok() {
            let _ = std::fs::rename(&tmp, &self.path);
        }
    }

    /// Активные привычки с их состоянием на `today`, в порядке создания.
    pub fn views(&self, today: NaiveDate) -> Vec<Value> {
        let saved = self.inner.lock().unwrap();
        let mut active: Vec<&Habit> = saved.habits.iter().filter(|h| !h.archived).collect();
        active.sort_by(|a, b| a.created_at.cmp(&b.created_at).then_with(|| a.id.cmp(&b.id)));
        active.into_iter().map(|h| view(h, &days_done(&saved, &h.id), today)).collect()
    }

    pub fn view_one(&self, id: &str, today: NaiveDate) -> Result<Value, HabitError> {
        let saved = self.inner.lock().unwrap();
        let habit = find_active(&saved, id).ok_or(HabitError::NotFound)?;
        Ok(view(habit, &days_done(&saved, &habit.id), today))
    }

    pub fn add(&self, mut habit: Habit) -> Result<Habit, HabitError> {
        self.writable()?;
        let mut saved = self.inner.lock().unwrap();
        // Предел проверяется под тем же замком, что и вставка: два одновременных запроса не обойдут его.
        if saved.habits.iter().filter(|h| !h.archived).count() >= MAX_HABITS {
            return Err(HabitError::Limit);
        }
        habit.id = uid("habit");
        habit.created_at = crate::store::now();
        habit.archived = false;
        saved.habits.push(habit.clone());
        self.save(&saved);
        Ok(habit)
    }

    pub fn update(&self, id: &str, change: impl FnOnce(&mut Habit) -> Result<(), HabitError>) -> Result<Habit, HabitError> {
        self.writable()?;
        let mut saved = self.inner.lock().unwrap();
        let habit = saved.habits.iter_mut().find(|h| h.id == id && !h.archived).ok_or(HabitError::NotFound)?;
        change(habit)?;
        let updated = habit.clone();
        self.save(&saved);
        Ok(updated)
    }

    /// Удаление — это архив: отметки остаются, привычка уходит из списка.
    pub fn archive(&self, id: &str) -> Result<(), HabitError> {
        self.update(id, |habit| {
            habit.archived = true;
            Ok(())
        })
        .map(|_| ())
    }

    /// Отметить (`done`) или снять отметку дня. Повтор ничего не меняет; будущий день нельзя.
    pub fn set_day(&self, id: &str, day: NaiveDate, done: bool, today: NaiveDate) -> Result<(), HabitError> {
        if day > today {
            return Err(HabitError::Future);
        }
        self.writable()?;
        let mut saved = self.inner.lock().unwrap();
        if find_active(&saved, id).is_none() {
            return Err(HabitError::NotFound);
        }
        let key = day.to_string();
        let position = saved.checks.iter().position(|c| c.habit_id == id && c.date == key);
        match (done, position) {
            (true, None) => saved.checks.push(Check { habit_id: id.to_string(), date: key }),
            (false, Some(index)) => {
                saved.checks.remove(index);
            }
            _ => return Ok(()),
        }
        self.save(&saved);
        Ok(())
    }

    /// Напоминания, которые пора отправить сейчас. Каждое отмечается «сегодня напомнено» и сохраняется
    /// до отправки, поэтому следующий тик его не повторит. Сломанный файл не пишем и ничего не шлём.
    pub fn take_due_reminders(&self, today: NaiveDate, now: NaiveTime) -> Vec<DueReminder> {
        if self.writable().is_err() {
            return Vec::new();
        }
        let mut saved = self.inner.lock().unwrap();
        let due: Vec<usize> = saved
            .habits
            .iter()
            .enumerate()
            .filter(|(_, habit)| reminder_due(habit, &days_done(&saved, &habit.id), today, now))
            .map(|(index, _)| index)
            .collect();
        let mark = today.to_string();
        let mut out = Vec::new();
        for index in due {
            let id = saved.habits[index].id.clone();
            let days = streak(saved.habits[index].days_mask, &days_done(&saved, &id), today);
            let habit = &mut saved.habits[index];
            habit.last_reminded = Some(mark.clone());
            out.push(DueReminder {
                title: habit.title.clone(),
                body: reminder_text(&habit.title, days),
                channels: crate::reminders::or_windows(habit.remind_channels.clone()),
            });
        }
        if !out.is_empty() {
            self.save(&saved);
        }
        out
    }

    /// Номер или название (без учёта регистра) — для бота. Одинаковые названия не угадываются.
    pub fn resolve(&self, key: &str) -> Result<Habit, HabitError> {
        let key = key.trim();
        let saved = self.inner.lock().unwrap();
        if let Some(habit) = find_active(&saved, key) {
            return Ok(habit.clone());
        }
        let wanted = key.to_lowercase();
        let found: Vec<&Habit> = saved.habits.iter().filter(|h| !h.archived && !wanted.is_empty() && h.title.to_lowercase() == wanted).collect();
        match found.as_slice() {
            [one] => Ok((*one).clone()),
            [] => Err(HabitError::NotFound),
            _ => Err(HabitError::Invalid("таких привычек несколько — назови номер из list_habits".into())),
        }
    }
}

fn find_active<'a>(saved: &'a Saved, id: &str) -> Option<&'a Habit> {
    saved.habits.iter().find(|h| h.id == id && !h.archived)
}

fn days_done(saved: &Saved, habit_id: &str) -> HashSet<NaiveDate> {
    saved
        .checks
        .iter()
        .filter(|c| c.habit_id == habit_id)
        .filter_map(|c| NaiveDate::parse_from_str(&c.date, "%Y-%m-%d").ok())
        .collect()
}

pub fn today() -> NaiveDate {
    Local::now().date_naive()
}

/// Расписано ли на день: бит дня недели стоит в маске.
pub fn scheduled(mask: u8, day: NaiveDate) -> bool {
    (mask & (1u8 << day.weekday().num_days_from_monday())) != 0
}

/// Серия: подряд идущие расписанные дни с отметкой, заканчивается сегодня (или вчера, если сегодня ещё открыт).
/// Нерасписанные дни пропускаются и серию не рвут.
pub fn streak(mask: u8, done: &HashSet<NaiveDate>, today: NaiveDate) -> u32 {
    let mut count = 0;
    let mut day = today;
    if scheduled(mask, day) && !done.contains(&day) {
        match day.pred_opt() {
            Some(previous) => day = previous,
            None => return 0,
        }
    }
    for _ in 0..STREAK_LIMIT {
        if scheduled(mask, day) {
            if done.contains(&day) {
                count += 1;
            } else {
                break;
            }
        }
        match day.pred_opt() {
            Some(previous) => day = previous,
            None => break,
        }
    }
    count
}

/// Пора ли напомнить о привычке в `now` дня `today`: расписано, не отмечено, сегодня ещё не напоминали,
/// время пришло и прошло не больше 12 часов. Дольше — компьютер был выключен, день пропускаем.
pub fn reminder_due(habit: &Habit, done: &HashSet<NaiveDate>, today: NaiveDate, now: NaiveTime) -> bool {
    if habit.archived || !scheduled(habit.days_mask, today) || done.contains(&today) {
        return false;
    }
    if habit.last_reminded.as_deref() == Some(today.to_string().as_str()) {
        return false;
    }
    let Some(at) = habit.remind_at.as_deref().and_then(parse_clock) else { return false };
    if now < at {
        return false;
    }
    now.signed_duration_since(at).num_milliseconds() <= crate::reminders::OVERDUE_MS
}

fn parse_clock(raw: &str) -> Option<NaiveTime> {
    NaiveTime::parse_from_str(raw, "%H:%M").ok()
}

/// Текст напоминания: название и, если серия идёт, её длина.
pub fn reminder_text(title: &str, streak_days: u32) -> String {
    if streak_days > 0 { format!("{title}. Серия: {streak_days} дн.") } else { title.to_string() }
}

/// Семь дней, заканчивающихся `today`, по возрастанию.
fn last_week(today: NaiveDate) -> Vec<NaiveDate> {
    let mut days = vec![today];
    for _ in 1..WEEK {
        let last = *days.last().unwrap_or(&today);
        days.push(last.pred_opt().unwrap_or(last));
    }
    days.reverse();
    days
}

fn view(habit: &Habit, done: &HashSet<NaiveDate>, today: NaiveDate) -> Value {
    let week = last_week(today);
    let days: Vec<Value> = week
        .iter()
        .map(|day| json!({ "day": day.to_string(), "scheduled": scheduled(habit.days_mask, *day), "done": done.contains(day) }))
        .collect();
    let week_scheduled = week.iter().filter(|day| scheduled(habit.days_mask, **day)).count();
    let week_done = week.iter().filter(|day| scheduled(habit.days_mask, **day) && done.contains(day)).count();
    json!({
        "id": habit.id,
        "title": habit.title,
        "caption": habit.caption,
        "icon": habit.icon,
        "days_mask": habit.days_mask,
        "created_at": habit.created_at,
        "archived": habit.archived,
        "remind_at": habit.remind_at,
        "remind_channels": crate::reminders::or_windows(habit.remind_channels.clone()),
        "today_done": done.contains(&today),
        "today_scheduled": scheduled(habit.days_mask, today),
        "streak": streak(habit.days_mask, done, today),
        "week": days,
        "week_done": week_done,
        "week_scheduled": week_scheduled,
        "total_done": done.len(),
    })
}

struct Template {
    group: &'static str,
    title: &'static str,
    en: &'static str,
    icon: &'static str,
    days_mask: u8,
}

/// Группы галереи: id, название, английское название.
const GROUPS: &[(&str, &str, &str)] = &[
    ("suggested", "Предложенные", "Suggested"),
    ("routine", "Рутина", "Routine"),
    ("health", "Здоровье", "Health"),
    ("sport", "Спорт", "Sport"),
];

/// Маски: 127 — каждый день; 31 — будни; 42 — вт/чт/сб; 21 — пн/ср/пт.
const TEMPLATES: &[Template] = &[
    Template { group: "suggested", title: "Выпить стакан воды", en: "Drink a glass of water", icon: "drop", days_mask: 127 },
    Template { group: "suggested", title: "Прочитать 10 страниц", en: "Read 10 pages", icon: "book", days_mask: 127 },
    Template { group: "suggested", title: "Записать три дела на завтра", en: "Write three tasks for tomorrow", icon: "pen", days_mask: 127 },
    Template { group: "routine", title: "Заправить кровать", en: "Make the bed", icon: "bed", days_mask: 127 },
    Template { group: "routine", title: "Кофе без телефона", en: "Coffee without the phone", icon: "coffee", days_mask: 31 },
    Template { group: "routine", title: "Вечерний обзор дня", en: "Evening review of the day", icon: "moon", days_mask: 127 },
    Template { group: "routine", title: "Навести порядок на столе", en: "Tidy up the desk", icon: "home", days_mask: 31 },
    Template { group: "health", title: "Прогулка 15 минут", en: "Walk for 15 minutes", icon: "leaf", days_mask: 127 },
    Template { group: "health", title: "Овощи или фрукты за день", en: "Vegetables or fruit today", icon: "apple", days_mask: 127 },
    Template { group: "health", title: "Пять минут тишины", en: "Five minutes of quiet", icon: "heart", days_mask: 127 },
    Template { group: "sport", title: "Зарядка 10 минут", en: "10-minute workout", icon: "dumbbell", days_mask: 127 },
    Template { group: "sport", title: "Пробежка", en: "Run", icon: "run", days_mask: 42 },
    Template { group: "sport", title: "Зал", en: "Gym", icon: "dumbbell", days_mask: 21 },
];

pub fn templates_json() -> Value {
    let groups: Vec<Value> = GROUPS.iter().map(|(id, title, en)| json!({ "id": id, "title": title, "en": en })).collect();
    let items: Vec<Value> = TEMPLATES
        .iter()
        .map(|t| json!({ "group": t.group, "title": t.title, "en": t.en, "icon": t.icon, "days_mask": t.days_mask }))
        .collect();
    json!({ "groups": groups, "templates": items })
}

fn clean_title(raw: &str) -> Result<String, HabitError> {
    let title = raw.trim();
    if title.is_empty() {
        return Err(HabitError::Invalid("нужно название привычки".into()));
    }
    if title.chars().count() > MAX_TITLE {
        return Err(HabitError::Invalid(format!("название до {MAX_TITLE} символов")));
    }
    Ok(title.to_string())
}

fn clean_caption(raw: &str) -> Result<String, HabitError> {
    let caption = raw.trim();
    if caption.chars().count() > MAX_CAPTION {
        return Err(HabitError::Invalid(format!("подпись до {MAX_CAPTION} символов")));
    }
    Ok(caption.to_string())
}

fn clean_icon(raw: &str) -> Result<String, HabitError> {
    let icon = raw.trim();
    let emoji = !icon.is_empty() && !icon.is_ascii() && icon.chars().count() <= 8;
    if ICON_NAMES.contains(&icon) || emoji {
        Ok(icon.to_string())
    } else {
        Err(HabitError::Invalid("иконка: имя из списка или один эмодзи".into()))
    }
}

fn clean_mask(value: i64) -> Result<u8, HabitError> {
    if (1..=i64::from(EVERY_DAY)).contains(&value) {
        Ok(value as u8)
    } else {
        Err(HabitError::Invalid("дни недели: маска от 1 до 127 (бит 0 — понедельник)".into()))
    }
}

/// Время напоминания: пусто — без напоминания; иначе ЧЧ:ММ (секунды отбрасываем).
fn clean_remind_at(raw: &str) -> Result<Option<String>, HabitError> {
    let text = raw.trim();
    if text.is_empty() {
        return Ok(None);
    }
    NaiveTime::parse_from_str(text, "%H:%M")
        .or_else(|_| NaiveTime::parse_from_str(text, "%H:%M:%S"))
        .map(|time| Some(time.format("%H:%M").to_string()))
        .map_err(|_| HabitError::Invalid("напоминание: время ЧЧ:ММ".into()))
}

/// Каналы напоминания — те же, что у напоминаний: непустой список windows и/или telegram.
fn clean_remind_channels(list: &Value) -> Result<Vec<String>, HabitError> {
    let items = list.as_array().ok_or_else(|| HabitError::Invalid("каналы — список: windows или telegram".into()))?;
    let mut wanted: Vec<String> = Vec::new();
    for item in items {
        match item.as_str() {
            Some(channel) if crate::reminders::CHANNELS.contains(&channel) => {
                if !wanted.iter().any(|c| c == channel) {
                    wanted.push(channel.to_string());
                }
            }
            _ => return Err(HabitError::Invalid("каналы: windows или telegram".into())),
        }
    }
    if wanted.is_empty() {
        return Err(HabitError::Invalid("каналы: windows или telegram".into()));
    }
    Ok(wanted)
}

fn parse_day(raw: &str) -> Result<NaiveDate, HabitError> {
    NaiveDate::parse_from_str(raw.trim(), "%Y-%m-%d").map_err(|_| HabitError::Invalid("дата в формате ГГГГ-ММ-ДД".into()))
}

/// Пустой день — сегодня.
fn day_or_today(raw: Option<&str>) -> Result<NaiveDate, HabitError> {
    match raw.map(str::trim).filter(|s| !s.is_empty()) {
        Some(day) => parse_day(day),
        None => Ok(today()),
    }
}

fn text_field(body: &Value, key: &str) -> Result<Option<String>, HabitError> {
    match body.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(text)) => Ok(Some(text.clone())),
        Some(_) => Err(HabitError::Invalid(format!("поле {key} должно быть строкой"))),
    }
}

fn number_field(body: &Value, key: &str) -> Result<Option<i64>, HabitError> {
    match body.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(value) => value.as_i64().map(Some).ok_or_else(|| HabitError::Invalid(format!("поле {key} должно быть целым числом"))),
    }
}

fn json_body(bytes: &Bytes) -> Result<Value, HabitError> {
    if bytes.is_empty() {
        return Ok(json!({}));
    }
    serde_json::from_slice(bytes).map_err(|_| HabitError::Invalid("тело запроса — JSON-объект".into()))
}

fn reply(status: StatusCode, value: Value) -> Response {
    (status, Json(value)).into_response()
}

fn respond(result: Result<(StatusCode, Value), HabitError>) -> Response {
    match result {
        Ok((status, value)) => reply(status, value),
        Err(error) => reply(error.status(), json!({ "error": error.code(), "message": error.message() })),
    }
}

fn changed(app: &App) {
    app.turns.emit(json!({ "type": "habits_changed" }));
}

fn create(app: &App, body: &Value) -> Result<(StatusCode, Value), HabitError> {
    let title = clean_title(&text_field(body, "title")?.unwrap_or_default())?;
    let icon = clean_icon(&text_field(body, "icon")?.ok_or_else(|| HabitError::Invalid("нужна иконка".into()))?)?;
    let caption = clean_caption(&text_field(body, "caption")?.unwrap_or_default())?;
    let days_mask = clean_mask(number_field(body, "days_mask")?.unwrap_or(i64::from(EVERY_DAY)))?;
    let remind_at = clean_remind_at(&text_field(body, "remind_at")?.unwrap_or_default())?;
    let remind_channels = match body.get("remind_channels") {
        None | Some(Value::Null) => crate::reminders::or_windows(Vec::new()),
        Some(list) => clean_remind_channels(list)?,
    };
    let habit = app.habits.add(Habit { title, caption, icon, days_mask, remind_at, remind_channels, ..Habit::default() })?;
    changed(app);
    Ok((StatusCode::CREATED, app.habits.view_one(&habit.id, today())?))
}

fn edit(app: &App, id: &str, body: &Value) -> Result<(StatusCode, Value), HabitError> {
    let title = text_field(body, "title")?.map(|t| clean_title(&t)).transpose()?;
    let caption = text_field(body, "caption")?.map(|t| clean_caption(&t)).transpose()?;
    let icon = text_field(body, "icon")?.map(|t| clean_icon(&t)).transpose()?;
    let days_mask = number_field(body, "days_mask")?.map(clean_mask).transpose()?;
    // Пустая строка снимает напоминание; отсутствующее поле оставляет как есть.
    let remind_at = text_field(body, "remind_at")?.map(|t| clean_remind_at(&t)).transpose()?;
    let remind_channels = match body.get("remind_channels") {
        None | Some(Value::Null) => None,
        Some(list) => Some(clean_remind_channels(list)?),
    };
    app.habits.update(id, |habit| {
        if let Some(title) = title {
            habit.title = title;
        }
        if let Some(caption) = caption {
            habit.caption = caption;
        }
        if let Some(icon) = icon {
            habit.icon = icon;
        }
        if let Some(days_mask) = days_mask {
            habit.days_mask = days_mask;
        }
        if let Some(remind_at) = remind_at {
            // Новое время — новое напоминание сегодня, даже если старое уже отправили.
            if habit.remind_at != remind_at {
                habit.last_reminded = None;
            }
            habit.remind_at = remind_at;
        }
        if let Some(remind_channels) = remind_channels {
            habit.remind_channels = remind_channels;
        }
        Ok(())
    })?;
    changed(app);
    Ok((StatusCode::OK, app.habits.view_one(id, today())?))
}

fn archive(app: &App, id: &str) -> Result<(), HabitError> {
    app.habits.archive(id)?;
    changed(app);
    Ok(())
}

fn check(app: &App, id: &str, bytes: &Bytes) -> Result<(StatusCode, Value), HabitError> {
    let body = json_body(bytes)?;
    let day = day_or_today(text_field(&body, "date")?.as_deref())?;
    app.habits.set_day(id, day, true, today())?;
    changed(app);
    Ok((StatusCode::OK, app.habits.view_one(id, today())?))
}

fn uncheck(app: &App, id: &str, date: Option<&str>) -> Result<(StatusCode, Value), HabitError> {
    let day = day_or_today(date)?;
    app.habits.set_day(id, day, false, today())?;
    changed(app);
    Ok((StatusCode::OK, app.habits.view_one(id, today())?))
}

type Shared = State<Arc<App>>;

async fn list_route(State(app): Shared) -> Response {
    reply(StatusCode::OK, Value::Array(app.habits.views(today())))
}

async fn templates_route() -> Response {
    reply(StatusCode::OK, templates_json())
}

async fn create_route(State(app): Shared, body: Bytes) -> Response {
    respond(json_body(&body).and_then(|body| create(&app, &body)))
}

async fn patch_route(State(app): Shared, Path(id): Path<String>, body: Bytes) -> Response {
    respond(json_body(&body).and_then(|body| edit(&app, &id, &body)))
}

async fn archive_route(State(app): Shared, Path(id): Path<String>) -> Response {
    match archive(&app, &id) {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(error) => respond(Err(error)),
    }
}

async fn check_route(State(app): Shared, Path(id): Path<String>, body: Bytes) -> Response {
    respond(check(&app, &id, &body))
}

#[derive(Deserialize)]
pub struct DayQuery {
    date: Option<String>,
}

async fn uncheck_route(State(app): Shared, Path(id): Path<String>, Query(query): Query<DayQuery>) -> Response {
    respond(uncheck(&app, &id, query.date.as_deref()))
}

/// Тик планировщика: напоминания о привычках, которые пора показать. Модель не вызывается.
pub async fn remind_tick(app: &Arc<App>) {
    let clock = Local::now();
    for due in app.habits.take_due_reminders(clock.date_naive(), clock.time()) {
        for delivery in crate::reminders::send_text(app, "Напоминание", &due.body, &due.channels).await {
            if let Some(error) = delivery.error {
                eprintln!("[habits] напоминание «{}» по каналу {}: {error}", due.title, delivery.channel);
            }
        }
    }
}

/// Маршруты привычек. Менять их может только окно человека (см. `guard::habit_write`).
pub fn routes() -> Router<Arc<App>> {
    Router::new()
        .route("/api/habits", get(list_route).post(create_route))
        .route("/api/habits/templates", get(templates_route))
        .route("/api/habits/{id}", patch(patch_route).delete(archive_route))
        .route("/api/habits/{id}/check", post(check_route).delete(uncheck_route))
}

/// Инструменты бота: список, создание, правка, отметка и архив привычек человека. Карточки нет, кроме архива
/// (`tools::is_high_risk`): он спрашивается всегда.
pub fn is_bot_tool(name: &str) -> bool {
    matches!(name, "list_habits" | "create_habit" | "update_habit" | "check_habit" | "archive_habit")
}

pub fn bot_tool_defs() -> Vec<Value> {
    let fields = || {
        json!({
            "title": { "type": "string", "description": "Название, до 80 символов." },
            "icon": { "type": "string", "description": "Имя значка (drop, sun, moon, book, pen, leaf, coffee, heart, brain, check, star, home, bed, apple, dumbbell, run, music, phone) или один эмодзи." },
            "caption": { "type": "string", "description": "Подпись, до 200 символов." },
            "days_mask": { "type": "integer", "description": "Дни недели битами: 1 — пн, 2 — вт, 4 — ср, 8 — чт, 16 — пт, 32 — сб, 64 — вс; сумма. 127 (по умолчанию) — каждый день, 31 — будни." },
            "remind_at": { "type": "string", "description": "Время напоминания ЧЧ:ММ; пустая строка снимает." },
            "remind_channels": { "type": "array", "items": { "type": "string" }, "description": "Каналы напоминания: windows, telegram." },
        })
    };
    let with_habit = |extra: Value| {
        let mut all = json!({ "habit": { "type": "string", "description": "Номер привычки из list_habits или её название." } });
        if let (Some(all), Some(extra)) = (all.as_object_mut(), extra.as_object()) {
            all.extend(extra.clone());
        }
        all
    };
    vec![
        json!({
            "type": "function",
            "function": {
                "name": "create_habit",
                "description": "Завести привычку человека. Нужны title и icon. Привычек не больше 50. Подтверждение не нужно.",
                "parameters": { "type": "object", "properties": fields(), "required": ["title", "icon"] },
            },
        }),
        json!({
            "type": "function",
            "function": {
                "name": "update_habit",
                "description": "Изменить привычку человека: передавай только то, что меняется. Подтверждение не нужно.",
                "parameters": { "type": "object", "properties": with_habit(fields()), "required": ["habit"] },
            },
        }),
        json!({
            "type": "function",
            "function": {
                "name": "archive_habit",
                "description": "Убрать привычку человека в архив (из списка она исчезает). Человек подтверждает каждое удаление.",
                "parameters": { "type": "object", "properties": with_habit(json!({})), "required": ["habit"] },
            },
        }),
        json!({
            "type": "function",
            "function": {
                "name": "list_habits",
                "description": "Показать привычки человека: номер, название, отметку за сегодня, серию и сколько сделано за неделю.",
                "parameters": { "type": "object", "properties": {} },
            },
        }),
        json!({
            "type": "function",
            "function": {
                "name": "check_habit",
                "description": "Отметить привычку человека сделанной за сегодня или за вчера. Только его собственные привычки; подтверждение не нужно.",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "habit": { "type": "string", "description": "Номер привычки из list_habits или её название." },
                        "date": { "type": "string", "description": "День ГГГГ-ММ-ДД: только сегодня или вчера. По умолчанию сегодня." },
                    },
                    "required": ["habit"],
                },
            },
        }),
    ]
}

fn bot_line(view: &Value) -> String {
    let today_state = if view["today_done"].as_bool() == Some(true) {
        "отмечено"
    } else if view["today_scheduled"].as_bool() == Some(true) {
        "не отмечено"
    } else {
        "сегодня не в расписании"
    };
    format!(
        "{} · {} · сегодня: {today_state} · серия: {} · за неделю: {} из {}",
        view["id"].as_str().unwrap_or_default(),
        view["title"].as_str().unwrap_or_default(),
        view["streak"],
        view["week_done"],
        view["week_scheduled"],
    )
}

/// Выполнить инструмент бота. Ошибки — текстом для модели.
pub fn exec_bot_tool(app: &App, name: &str, args: &Value) -> Result<String, String> {
    let now = today();
    if name == "list_habits" {
        let views = app.habits.views(now);
        if views.is_empty() {
            return Ok("Привычек нет.".into());
        }
        return Ok(views.iter().map(bot_line).collect::<Vec<_>>().join("\n"));
    }
    if name == "create_habit" {
        let (_, view) = create(app, args).map_err(|e| e.message())?;
        return Ok(format!("Завёл привычку «{}». Номер: {}.", view["title"].as_str().unwrap_or_default(), view["id"].as_str().unwrap_or_default()));
    }
    let key = crate::workflow_tools::text(args, "habit");
    if key.is_empty() {
        return Err("нужен номер или название привычки (list_habits)".into());
    }
    let habit = app.habits.resolve(key).map_err(|e| e.message())?;
    match name {
        "update_habit" => {
            edit(app, &habit.id, args).map_err(|e| e.message())?;
            return Ok(format!("Изменил привычку «{}».", habit.title));
        }
        "archive_habit" => {
            archive(app, &habit.id).map_err(|e| e.message())?;
            return Ok(format!("Убрал привычку «{}» в архив.", habit.title));
        }
        _ => {}
    }
    let raw = crate::workflow_tools::text(args, "date");
    let day = day_or_today(Some(raw)).map_err(|e| e.message())?;
    if day != now && Some(day) != now.pred_opt() {
        return Err("отмечать привычку можно только за сегодня или вчера".into());
    }
    app.habits.set_day(&habit.id, day, true, now).map_err(|e| e.message())?;
    changed(app);
    let streak = app.habits.view_one(&habit.id, now).map_err(|e| e.message())?["streak"].clone();
    Ok(format!("Отметил «{}» за {day}. Серия: {streak}.", habit.title))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn day(text: &str) -> NaiveDate {
        NaiveDate::parse_from_str(text, "%Y-%m-%d").unwrap()
    }

    /// Папка внутри `target` этой копии: системный TEMP здесь недоступен для записи.
    fn fresh_dir(name: &str) -> PathBuf {
        let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("target").join("habits-tests").join(format!("{name}-{}", uid("t")));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn habit(title: &str, mask: u8) -> Habit {
        Habit { title: title.into(), icon: "drop".into(), days_mask: mask, ..Habit::default() }
    }

    #[test]
    fn streak_skips_unscheduled_days() {
        // Пн–пт (бит 0–4 = 31); сегодня — понедельник 2026-09-07.
        let today = day("2026-09-07");
        let done: HashSet<NaiveDate> = [day("2026-09-07"), day("2026-09-04"), day("2026-09-03")].into_iter().collect();
        assert_eq!(streak(31, &done, today), 3, "пн, (сб и вс пропущены), пт, чт");
        let mut open = done.clone();
        open.remove(&today);
        assert_eq!(streak(31, &open, today), 2, "сегодня открыт: считаем с пятницы");
        assert_eq!(streak(127, &done, today), 1, "каждый день: воскресенье без отметки рвёт серию");
        assert_eq!(streak(127, &HashSet::new(), today), 0);
    }

    #[test]
    fn monday_is_bit_zero_and_sunday_is_bit_six() {
        assert!(scheduled(1, day("2026-09-07")));
        assert!(!scheduled(1, day("2026-09-08")));
        assert!(scheduled(64, day("2026-09-06")));
        assert!(scheduled(EVERY_DAY, day("2026-09-06")));
    }

    #[test]
    fn validation_refuses_bad_values() {
        assert!(clean_title("  ").is_err());
        assert!(clean_title(&"x".repeat(81)).is_err());
        assert!(clean_title(&"x".repeat(80)).is_ok());
        assert!(clean_icon("drop").is_ok());
        assert!(clean_icon("sword").is_err());
        assert!(clean_icon("💧").is_ok());
        assert!(clean_mask(0).is_err());
        assert!(clean_mask(128).is_err());
        assert!(clean_mask(200).is_err());
        assert_eq!(clean_mask(127).unwrap(), 127);
    }

    #[test]
    fn one_check_per_day_and_future_days_are_refused() {
        let store = Habits::open(fresh_dir("checks"));
        let made = store.add(habit("Вода", EVERY_DAY)).unwrap();
        let today = day("2026-09-06");
        store.set_day(&made.id, today, true, today).unwrap();
        store.set_day(&made.id, today, true, today).unwrap();
        let view = store.view_one(&made.id, today).unwrap();
        assert_eq!(view["total_done"], 1);
        assert_eq!(view["today_done"], true);
        assert_eq!(view["streak"], 1);
        assert!(matches!(store.set_day(&made.id, day("2026-09-07"), true, today), Err(HabitError::Future)));
        store.set_day(&made.id, today, false, today).unwrap();
        assert_eq!(store.view_one(&made.id, today).unwrap()["streak"], 0);
    }

    #[test]
    fn the_limit_is_fifty_active_habits_and_archive_frees_a_slot() {
        let store = Habits::open(fresh_dir("limit"));
        let mut ids = Vec::new();
        for n in 0..MAX_HABITS {
            ids.push(store.add(habit(&format!("привычка {n}"), EVERY_DAY)).unwrap().id);
        }
        assert!(matches!(store.add(habit("лишняя", EVERY_DAY)), Err(HabitError::Limit)));
        store.archive(&ids[0]).unwrap();
        assert!(store.add(habit("новая", EVERY_DAY)).is_ok());
    }

    #[test]
    fn bot_resolves_by_title_but_not_an_ambiguous_one() {
        let store = Habits::open(fresh_dir("resolve"));
        store.add(habit("Бег", EVERY_DAY)).unwrap();
        store.add(habit("бег", 21)).unwrap();
        store.add(habit("Вода", EVERY_DAY)).unwrap();
        assert_eq!(store.resolve("вода").unwrap().title, "Вода");
        assert!(matches!(store.resolve("Бег"), Err(HabitError::Invalid(_))));
        assert!(matches!(store.resolve("нет такой"), Err(HabitError::NotFound)));
    }

    #[test]
    fn templates_have_english_and_valid_icons_and_masks() {
        for template in TEMPLATES {
            assert!(ICON_NAMES.contains(&template.icon), "{}", template.title);
            assert!(template.days_mask >= 1 && template.days_mask <= EVERY_DAY, "{}", template.title);
            assert!(!template.en.is_empty(), "{}", template.title);
            assert!(GROUPS.iter().any(|(id, _, _)| *id == template.group));
        }
    }

    fn clock(text: &str) -> NaiveTime {
        NaiveTime::parse_from_str(text, "%H:%M").unwrap()
    }

    /// Привычка с напоминанием в 9:00; понедельник 2 ноября 2026 — расписан, вторник — нет.
    fn reminded(mask: u8) -> Habit {
        Habit { remind_at: Some("09:00".into()), remind_channels: vec!["windows".into()], ..habit("Вода", mask) }
    }

    #[test]
    fn due_on_a_scheduled_day_after_the_time_only() {
        let monday = day("2026-11-02");
        let none = HashSet::new();
        assert!(reminder_due(&reminded(1), &none, monday, clock("09:00")), "в назначенное время");
        assert!(reminder_due(&reminded(1), &none, monday, clock("11:30")), "позже назначенного тоже");
        assert!(!reminder_due(&reminded(1), &none, monday, clock("08:59")), "раньше времени — рано");
        assert!(!reminder_due(&reminded(1), &none, day("2026-11-03"), clock("11:30")), "вторник не расписан");
        let no_time = Habit { remind_at: None, ..reminded(1) };
        assert!(!reminder_due(&no_time, &none, monday, clock("11:30")), "без времени напоминаний нет");
        let archived = Habit { archived: true, ..reminded(1) };
        assert!(!reminder_due(&archived, &none, monday, clock("11:30")), "архивная не напоминает");
    }

    #[test]
    fn done_or_already_reminded_today_is_not_due_again() {
        let monday = day("2026-11-02");
        let done: HashSet<NaiveDate> = [monday].into_iter().collect();
        assert!(!reminder_due(&reminded(1), &done, monday, clock("11:30")), "уже отмечена сегодня");
        let sent = Habit { last_reminded: Some("2026-11-02".into()), ..reminded(1) };
        assert!(!reminder_due(&sent, &HashSet::new(), monday, clock("11:30")), "сегодня уже напомнили");
        let yesterday = Habit { last_reminded: Some("2026-11-01".into()), ..reminded(1) };
        assert!(reminder_due(&yesterday, &HashSet::new(), monday, clock("11:30")), "вчерашнее напоминание не в счёт");
    }

    #[test]
    fn a_reminder_more_than_twelve_hours_late_is_skipped() {
        let monday = day("2026-11-02");
        let none = HashSet::new();
        assert!(reminder_due(&reminded(1), &none, monday, clock("21:00")), "ровно 12 часов — ещё можно");
        assert!(!reminder_due(&reminded(1), &none, monday, clock("21:01")), "дольше 12 часов — день пропущен");
        assert!(!reminder_due(&reminded(1), &none, monday, clock("23:30")), "компьютер был выключен весь вечер");
    }

    #[test]
    fn reminder_text_names_the_streak_only_when_there_is_one() {
        assert_eq!(reminder_text("Вода", 0), "Вода");
        assert_eq!(reminder_text("Вода", 3), "Вода. Серия: 3 дн.");
    }

    #[test]
    fn due_reminders_are_taken_once_and_survive_a_restart() {
        let dir = fresh_dir("remind-once");
        let store = Habits::open(dir.clone());
        store.add(Habit { remind_at: Some("09:00".into()), remind_channels: vec!["windows".into()], ..habit("Вода", 127) }).unwrap();
        let monday = day("2026-11-02");
        let first = store.take_due_reminders(monday, clock("10:00"));
        assert_eq!(first, vec![DueReminder { title: "Вода".into(), body: "Вода".into(), channels: vec!["windows".into()] }]);
        assert!(store.take_due_reminders(monday, clock("10:05")).is_empty(), "второй тик не повторяет");
        let reopened = Habits::open(dir);
        assert!(reopened.take_due_reminders(monday, clock("10:10")).is_empty(), "отметка переживает перезапуск");
    }

    #[test]
    fn an_old_file_without_reminder_fields_loads_with_none() {
        let dir = fresh_dir("remind-old");
        let old = r#"{"habits":[{"id":"habit_1","title":"Вода","caption":"","icon":"drop","days_mask":127,"created_at":1,"archived":false}],"checks":[]}"#;
        std::fs::write(dir.join("habits.json"), old).unwrap();
        let store = Habits::open(dir);
        let views = store.views(day("2026-11-02"));
        assert_eq!(views[0]["remind_at"], Value::Null);
        assert_eq!(views[0]["remind_channels"], json!(["windows"]));
        assert!(store.take_due_reminders(day("2026-11-02"), clock("12:00")).is_empty());
    }

    #[test]
    fn reminder_channels_are_checked_like_reminders() {
        assert_eq!(clean_remind_channels(&json!(["telegram", "windows", "telegram"])).unwrap(), vec!["telegram".to_string(), "windows".to_string()]);
        assert!(clean_remind_channels(&json!(["sms"])).is_err());
        assert!(clean_remind_channels(&json!([])).is_err());
        assert!(clean_remind_channels(&json!("windows")).is_err());
        assert_eq!(clean_remind_at("25:00").unwrap_err().message(), "напоминание: время ЧЧ:ММ");
        assert_eq!(clean_remind_at("  ").unwrap(), None);
        assert_eq!(clean_remind_at("07:30:15").unwrap(), Some("07:30".into()));
    }
}
