//! Рабочие процессы: граф шагов (холст), его проверка, хранение и расписание запусков.
//! Договор данных и движка — `notes/workflows-spec.md`. Обход графа — `workflow_walk`, запуски — `workflow_run`.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use chrono::{Datelike, Duration, Local, NaiveDate, NaiveDateTime, NaiveTime, TimeZone, Timelike};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use tokio_util::sync::CancellationToken;

use crate::store::{now, uid};

pub const MAX_NODES: usize = 200;
pub const MAX_EDGES: usize = 400;
pub const MAX_FIELDS: usize = 32;
pub const MAX_FIELD_CHARS: usize = 20_000;
pub const MAX_STROKES: usize = 500;
pub const MAX_POINTS: usize = 2000;
pub const MAX_GRAPH_BYTES: usize = 1024 * 1024;
pub const KEEP_RUNS: usize = 50;
pub const MIN_INTERVAL_MINUTES: f64 = 3.0;

pub const ANNOTATIONS: [&str; 3] = ["note", "text", "shape"];
pub const KINDS: [&str; 10] = ["trigger", "prompt", "bot", "approval", "output", "llm", "condition", "note", "text", "shape"];

pub fn is_annotation(kind: &str) -> bool {
    ANNOTATIONS.contains(&kind)
}

// ---------------------------------------------------------------------------
// Данные
// ---------------------------------------------------------------------------

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct Schedule {
    pub kind: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub time: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub minute: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub weekday: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub day: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub amount: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub unit: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub date: Option<String>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Node {
    pub id: String,
    pub kind: String,
    pub x: f64,
    pub y: f64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub w: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub h: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub locked: Option<bool>,
    pub fields: Map<String, Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub schedule: Option<Schedule>,
}

impl Node {
    pub fn text(&self, key: &str) -> String {
        match self.fields.get(key) {
            Some(Value::String(s)) => s.clone(),
            Some(Value::Number(n)) => n.to_string(),
            Some(Value::Bool(b)) => b.to_string(),
            _ => String::new(),
        }
    }
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Edge {
    pub id: String,
    pub from: String,
    pub to: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub branch: Option<String>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Stroke {
    pub id: String,
    pub brush: String,
    pub color: String,
    pub width: f64,
    pub points: Vec<[f64; 2]>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Graph {
    pub nodes: Vec<Node>,
    pub edges: Vec<Edge>,
    pub strokes: Vec<Stroke>,
}

impl Graph {
    pub fn node(&self, id: &str) -> Option<&Node> {
        self.nodes.iter().find(|n| n.id == id)
    }
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct LastRun {
    pub status: String,
    pub at: i64,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Workflow {
    pub id: String,
    pub title: String,
    pub revision: u64,
    pub graph: Graph,
    pub created_at: i64,
    pub updated_at: i64,
    pub last_run: Option<LastRun>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Step {
    pub id: String,
    pub run_id: String,
    pub node_id: String,
    pub attempt: u32,
    pub status: String,
    pub input: String,
    pub output: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub branch: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    pub started_at: i64,
    pub ended_at: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub thread_id: Option<String>,
    /// Порядок начала шага в запуске (по общему счётчику запуска): по нему обход видит, что свежее.
    pub seq: u64,
    /// Порядок конца шага; 0, пока шаг не закончен.
    pub end_seq: u64,
    /// Ответ взят из прошлого запуска: настройки узла и входы с тех пор не менялись.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub reused: bool,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Run {
    pub id: String,
    pub workflow_id: String,
    pub status: String,
    pub source: String,
    pub started_at: i64,
    pub ended_at: Option<i64>,
    pub revision: u64,
    pub graph: Graph,
    pub steps: Vec<Step>,
    /// Повтор запроса с тем же `request_id` возвращает этот же запуск.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub request_id: Option<String>,
    /// «Запустить отсюда».
    #[serde(skip_serializing_if = "Option::is_none")]
    pub from_node: Option<String>,
    /// Какой запускающий узел сработал (расписание).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub trigger_node: Option<String>,
    /// Общий счётчик порядка шагов.
    pub counter: u64,
    /// Запуск одного узла («выполнить шаг»): ниже по графу ничего не идёт.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub solo: bool,
    /// Почему запуск упал целиком («слишком много кругов»).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

impl Run {
    pub fn is_open(&self) -> bool {
        matches!(self.status.as_str(), "running" | "approval" | "stopping")
    }
}

// ---------------------------------------------------------------------------
// Проверка графа
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct GraphError {
    pub code: &'static str,
    pub message: String,
    pub node_id: Option<String>,
    pub edge_id: Option<String>,
}

fn err_node(code: &'static str, message: impl Into<String>, node: &str) -> GraphError {
    GraphError { code, message: message.into(), node_id: Some(node.to_string()), edge_id: None }
}

fn err_edge(code: &'static str, message: impl Into<String>, edge: &str) -> GraphError {
    GraphError { code, message: message.into(), node_id: None, edge_id: Some(edge.to_string()) }
}

fn err_plain(code: &'static str, message: impl Into<String>) -> GraphError {
    GraphError { code, message: message.into(), node_id: None, edge_id: None }
}

const TRIGGERS: [&str; 6] = ["manual", "schedule", "webhook", "telegram", "discord", "slack"];
const TARGETS: [&str; 6] = ["notify", "chat", "telegram", "discord", "slack", "file"];
const SCHEDULE_KINDS: [&str; 7] = ["hourly", "daily", "weekdays", "weekly", "monthly", "interval", "once"];

fn parse_time(text: &str) -> Option<NaiveTime> {
    crate::schedule::parse_time(text)
}

/// Интервал в минутах, если он задан верно.
fn interval_minutes(s: &Schedule) -> Option<f64> {
    let amount = s.amount?;
    let per = match s.unit.as_deref()? {
        "minutes" => 1.0,
        "hours" => 60.0,
        "days" => 1440.0,
        _ => return None,
    };
    (amount.is_finite() && amount > 0.0).then_some(amount * per)
}

pub fn validate_schedule(s: &Schedule) -> Result<(), String> {
    if !SCHEDULE_KINDS.contains(&s.kind.as_str()) {
        return Err("неизвестный вид расписания".into());
    }
    let time_ok = || s.time.as_deref().and_then(parse_time).is_some();
    match s.kind.as_str() {
        "hourly" => {
            if s.minute.is_some_and(|m| !(0..=59).contains(&m)) {
                return Err("минута от 0 до 59".into());
            }
        }
        "daily" | "weekdays" => {
            if !time_ok() {
                return Err("нужно время ЧЧ:ММ".into());
            }
        }
        "weekly" => {
            if !time_ok() {
                return Err("нужно время ЧЧ:ММ".into());
            }
            if !s.weekday.is_some_and(|d| (1..=7).contains(&d)) {
                return Err("день недели от 1 до 7".into());
            }
        }
        "monthly" => {
            if !time_ok() {
                return Err("нужно время ЧЧ:ММ".into());
            }
            if !s.day.is_some_and(|d| (1..=31).contains(&d)) {
                return Err("число месяца от 1 до 31".into());
            }
        }
        "interval" => match interval_minutes(s) {
            None => return Err("нужны количество и единица (минуты, часы, дни)".into()),
            Some(m) if m < MIN_INTERVAL_MINUTES => return Err("интервал не меньше 3 минут".into()),
            Some(_) => {}
        },
        "once" => {
            if !time_ok() {
                return Err("нужно время ЧЧ:ММ".into());
            }
            if s.date.as_deref().and_then(|d| NaiveDate::parse_from_str(d, "%Y-%m-%d").ok()).is_none() {
                return Err("нужна дата ГГГГ-ММ-ДД".into());
            }
        }
        _ => {}
    }
    Ok(())
}

pub fn validate_graph(graph: &Graph) -> Result<(), GraphError> {
    if graph.nodes.len() > MAX_NODES {
        return Err(err_plain("too_many_nodes", format!("узлов не больше {MAX_NODES}")));
    }
    if graph.edges.len() > MAX_EDGES {
        return Err(err_plain("too_many_edges", format!("связей не больше {MAX_EDGES}")));
    }
    if graph.strokes.len() > MAX_STROKES {
        return Err(err_plain("too_many_strokes", format!("штрихов не больше {MAX_STROKES}")));
    }
    let mut stroke_ids: HashSet<&str> = HashSet::new();
    for stroke in &graph.strokes {
        if stroke.points.len() > MAX_POINTS {
            return Err(err_plain("too_many_points", format!("точек в штрихе не больше {MAX_POINTS}")));
        }
        if !matches!(stroke.brush.as_str(), "pen" | "marker") {
            return Err(err_plain("bad_brush", "кисть pen или marker"));
        }
        if stroke.id.trim().is_empty() || !stroke_ids.insert(stroke.id.as_str()) {
            return Err(err_plain("bad_stroke_id", "у штриха нет id или он повторяется"));
        }
        if stroke.color.chars().count() > 64 || !stroke.width.is_finite() || stroke.width <= 0.0 || stroke.width > 200.0 {
            return Err(err_plain("bad_stroke", "цвет до 64 знаков, толщина от 0 до 200"));
        }
        if stroke.points.iter().any(|p| !p[0].is_finite() || !p[1].is_finite()) {
            return Err(err_plain("bad_stroke", "точки штриха должны быть числами"));
        }
    }
    let size = serde_json::to_vec(graph).map(|v| v.len()).unwrap_or(usize::MAX);
    if size > MAX_GRAPH_BYTES {
        return Err(err_plain("graph_too_big", "граф больше 1 МБ"));
    }
    let mut kinds: HashMap<&str, &str> = HashMap::new();
    for node in &graph.nodes {
        if node.id.trim().is_empty() {
            return Err(err_plain("empty_node_id", "у узла нет id"));
        }
        if kinds.insert(&node.id, &node.kind).is_some() {
            return Err(err_node("duplicate_node", "два узла с одним id", &node.id));
        }
        if !KINDS.contains(&node.kind.as_str()) {
            return Err(err_node("unknown_kind", format!("неизвестный вид узла «{}»", node.kind), &node.id));
        }
        if node.fields.len() > MAX_FIELDS {
            return Err(err_node("too_many_fields", format!("полей не больше {MAX_FIELDS}"), &node.id));
        }
        for (key, value) in &node.fields {
            match value {
                Value::String(s) if s.chars().count() > MAX_FIELD_CHARS => {
                    return Err(err_node("field_too_long", format!("поле «{key}» длиннее {MAX_FIELD_CHARS} символов"), &node.id));
                }
                Value::String(_) | Value::Number(_) | Value::Bool(_) => {}
                _ => return Err(err_node("bad_field", format!("поле «{key}» должно быть строкой, числом или да/нет"), &node.id)),
            }
        }
        if node.kind == "trigger" {
            let trigger = node.text("trigger");
            if !trigger.is_empty() && !TRIGGERS.contains(&trigger.as_str()) {
                return Err(err_node("bad_trigger", format!("неизвестный запуск «{trigger}»"), &node.id));
            }
        }
        if node.kind == "shape" {
            let shape = node.text("shape");
            if !shape.is_empty() && !matches!(shape.as_str(), "rect" | "circle" | "diamond" | "triangle" | "oval" | "star") {
                return Err(err_node("bad_shape", format!("неизвестная фигура «{shape}»"), &node.id));
            }
        }
        if is_annotation(&node.kind) {
            for key in ["color", "fill"] {
                if node.text(key).chars().count() > 64 {
                    return Err(err_node("bad_field", format!("поле «{key}» слишком длинное"), &node.id));
                }
            }
        }
        if node.kind == "output" {
            let target = node.text("target");
            if !target.is_empty() && !TARGETS.contains(&target.as_str()) {
                return Err(err_node("bad_target", format!("неизвестное место вывода «{target}»"), &node.id));
            }
        }
        if let Some(schedule) = &node.schedule {
            validate_schedule(schedule).map_err(|m| err_node("bad_schedule", m, &node.id))?;
        }
    }
    let mut ids: HashSet<&str> = HashSet::new();
    let mut pairs: HashSet<(&str, &str)> = HashSet::new();
    for edge in &graph.edges {
        if edge.id.trim().is_empty() {
            return Err(err_plain("empty_edge_id", "у связи нет id"));
        }
        if !ids.insert(&edge.id) {
            return Err(err_edge("duplicate_edge_id", "две связи с одним id", &edge.id));
        }
        let (Some(from), Some(to)) = (kinds.get(edge.from.as_str()), kinds.get(edge.to.as_str())) else {
            return Err(err_edge("edge_unknown_node", "связь ведёт к узлу, которого нет", &edge.id));
        };
        if edge.from == edge.to {
            return Err(err_edge("edge_self", "связь узла с самим собой", &edge.id));
        }
        if !pairs.insert((&edge.from, &edge.to)) {
            return Err(err_edge("edge_duplicate", "такая связь уже есть", &edge.id));
        }
        if is_annotation(from) || is_annotation(to) {
            return Err(err_edge("edge_annotation", "заметки и фигуры нельзя соединять", &edge.id));
        }
        if matches!(*to, "trigger" | "prompt") {
            return Err(err_edge("edge_into_source", "в запуск и текст ничего не входит", &edge.id));
        }
        if *from == "output" {
            return Err(err_edge("edge_from_output", "из вывода ничего не выходит", &edge.id));
        }
        let branch = edge.branch.as_deref();
        if *from == "condition" {
            if !matches!(branch, Some("yes" | "no")) {
                return Err(err_edge("edge_branch_missing", "связь от условия: нужна ветка yes или no", &edge.id));
            }
        } else if branch.is_some() {
            return Err(err_edge("edge_branch_forbidden", "ветка бывает только у связи от условия", &edge.id));
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Расписание
// ---------------------------------------------------------------------------

fn local(ms: i64) -> NaiveDateTime {
    Local.timestamp_millis_opt(ms).single().map_or_else(|| Local::now().naive_local(), |at| at.naive_local())
}

fn to_ms(at: NaiveDateTime) -> i64 {
    Local
        .from_local_datetime(&at)
        .earliest()
        .or_else(|| Local.from_local_datetime(&(at + Duration::hours(1))).earliest())
        .map_or_else(|| Local::now().timestamp_millis(), |t| t.timestamp_millis())
}

fn days_in_month(date: NaiveDate) -> u32 {
    let (y, m) = if date.month() == 12 { (date.year() + 1, 1) } else { (date.year(), date.month() + 1) };
    NaiveDate::from_ymd_opt(y, m, 1).and_then(|first| first.pred_opt()).map_or(28, |last| last.day())
}

/// Ближайший срок строго после `now_ms`. `None` — больше срока нет (разовый уже прошёл).
pub fn next_after(s: &Schedule, now_ms: i64) -> Option<i64> {
    next_after_with(s, now_ms, local, to_ms)
}

pub fn next_after_with(s: &Schedule, now_ms: i64, local_of: impl Fn(i64) -> NaiveDateTime, ms_of: impl Fn(NaiveDateTime) -> i64) -> Option<i64> {
    let now = local_of(now_ms);
    if s.kind == "interval" {
        return Some(now_ms + (interval_minutes(s)?.max(MIN_INTERVAL_MINUTES) * 60_000.0) as i64);
    }
    if s.kind == "hourly" {
        let minute = s.minute.unwrap_or(0).clamp(0, 59) as u32;
        let mut at = now.date().and_hms_opt(now.hour(), minute, 0)?;
        if at <= now {
            at += Duration::hours(1);
        }
        return Some(ms_of(at));
    }
    let time = s.time.as_deref().and_then(parse_time)?;
    if s.kind == "once" {
        let date = NaiveDate::parse_from_str(s.date.as_deref()?, "%Y-%m-%d").ok()?;
        let at = date.and_time(time);
        return (at > now).then(|| ms_of(at));
    }
    let mut date = now.date();
    for _ in 0..=400 {
        let day_ok = match s.kind.as_str() {
            "daily" => true,
            "weekdays" => date.weekday().number_from_monday() <= 5,
            "weekly" => i64::from(date.weekday().number_from_monday()) == s.weekday.unwrap_or(1),
            "monthly" => i64::from(date.day()) == s.day.unwrap_or(1).min(i64::from(days_in_month(date))),
            _ => return None,
        };
        let at = date.and_time(time);
        if day_ok && at > now {
            return Some(ms_of(at));
        }
        date = date.succ_opt()?;
    }
    None
}

/// Когда сработает запускающий узел: срок и подпись расписания (при смене подписи срок считается заново).
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Slot {
    pub sig: String,
    pub next: Option<i64>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Stored {
    #[serde(flatten)]
    pub wf: Workflow,
    pub slots: HashMap<String, Slot>,
}

fn schedule_nodes(graph: &Graph) -> impl Iterator<Item = (&Node, &Schedule)> {
    graph.nodes.iter().filter(|n| n.kind == "trigger" && n.text("trigger") == "schedule").filter_map(|n| n.schedule.as_ref().map(|s| (n, s)))
}

/// Привести сроки в порядок: новые и изменённые узлы получают срок, исчезнувшие забываются. `true` — что-то изменилось.
pub fn refresh_slots(stored: &mut Stored, now_ms: i64) -> bool {
    let mut changed = false;
    let mut keep: HashSet<String> = HashSet::new();
    for (node, schedule) in schedule_nodes(&stored.wf.graph) {
        keep.insert(node.id.clone());
        let sig = serde_json::to_string(schedule).unwrap_or_default();
        if stored.slots.get(&node.id).is_some_and(|slot| slot.sig == sig) {
            continue;
        }
        stored.slots.insert(node.id.clone(), Slot { sig, next: next_after(schedule, now_ms) });
        changed = true;
    }
    let before = stored.slots.len();
    stored.slots.retain(|id, _| keep.contains(id));
    changed || before != stored.slots.len()
}

/// Узлы, которым пора. Следующий срок сдвигается сразу, раньше самого запуска; пропущенные слоты схлопываются в один.
pub fn collect_due(stored: &mut Stored, now_ms: i64) -> (Vec<String>, bool) {
    let mut changed = refresh_slots(stored, now_ms);
    let mut due = Vec::new();
    let nodes: Vec<(String, Schedule)> = schedule_nodes(&stored.wf.graph).map(|(n, s)| (n.id.clone(), s.clone())).collect();
    for (id, schedule) in nodes {
        let Some(slot) = stored.slots.get_mut(&id) else { continue };
        if slot.next.is_some_and(|next| now_ms >= next) {
            slot.next = next_after(&schedule, now_ms);
            changed = true;
            due.push(id);
        }
    }
    (due, changed)
}

// ---------------------------------------------------------------------------
// Хранилище
// ---------------------------------------------------------------------------

/// Сохранённое состояние графа: `auto` (при сохранении, не чаще раза в 5 минут), `run`, `restore`.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Version {
    pub v: u64,
    pub kind: String,
    pub at: i64,
    pub revision: u64,
    pub graph: Graph,
}

pub const KEEP_VERSIONS: usize = 50;
pub const AUTO_VERSION_MS: i64 = 5 * 60_000;

pub struct Inner {
    dir: PathBuf,
    pub versions: HashMap<String, Vec<Version>>,
    pub items: Vec<Stored>,
    pub runs: HashMap<String, Vec<Run>>,
    /// Токены остановки идущих шагов (по id шага).
    pub tokens: HashMap<String, CancellationToken>,
}

pub struct Workflows {
    pub inner: Mutex<Inner>,
}

fn write_atomic(path: &Path, text: &str) {
    let tmp = path.with_extension("json.tmp");
    if std::fs::write(&tmp, text).and_then(|()| std::fs::rename(&tmp, path)).is_err() {
        eprintln!("[workflows] не смог записать {}", path.display());
    }
}

impl Inner {
    fn wf_path(&self, id: &str) -> PathBuf {
        self.dir.join(format!("{id}.json"))
    }

    fn runs_path(&self, id: &str) -> PathBuf {
        self.dir.join(format!("{id}.runs.json"))
    }

    pub fn save_wf(&self, id: &str) {
        let Some(item) = self.items.iter().find(|s| s.wf.id == id) else { return };
        let _ = std::fs::create_dir_all(&self.dir);
        if let Ok(text) = serde_json::to_string(item) {
            write_atomic(&self.wf_path(id), &text);
        }
    }

    fn versions_path(&self, id: &str) -> PathBuf {
        self.dir.join(format!("{id}.versions.json"))
    }

    pub fn save_versions(&self, id: &str) {
        let _ = std::fs::create_dir_all(&self.dir);
        let empty = Vec::new();
        if let Ok(text) = serde_json::to_string(self.versions.get(id).unwrap_or(&empty)) {
            write_atomic(&self.versions_path(id), &text);
        }
    }

    /// Положить версию. `auto` пишется, только если прошлая версия старше пяти минут; для `run` повтор той же ревизии не плодится.
    pub fn push_version(&mut self, id: &str, kind: &str, graph: &Graph, revision: u64) -> Option<u64> {
        let at = now();
        let list = self.versions.entry(id.to_string()).or_default();
        if let Some(last) = list.last() {
            if (kind == "auto" && at - last.at < AUTO_VERSION_MS) || (kind == "run" && last.kind == "run" && last.revision == revision) {
                return None;
            }
        }
        let v = list.last().map_or(1, |l| l.v + 1);
        list.push(Version { v, kind: kind.to_string(), at, revision, graph: graph.clone() });
        while list.len() > KEEP_VERSIONS {
            list.remove(0);
        }
        self.save_versions(id);
        Some(v)
    }

    pub fn save_runs(&self, id: &str) {
        let _ = std::fs::create_dir_all(&self.dir);
        let empty = Vec::new();
        let runs = self.runs.get(id).unwrap_or(&empty);
        if let Ok(text) = serde_json::to_string(runs) {
            write_atomic(&self.runs_path(id), &text);
        }
    }

    pub fn remove_files(&self, id: &str) {
        let _ = std::fs::remove_file(self.wf_path(id));
        let _ = std::fs::remove_file(self.runs_path(id));
        let _ = std::fs::remove_file(self.versions_path(id));
    }

    pub fn get(&self, id: &str) -> Option<&Stored> {
        self.items.iter().find(|s| s.wf.id == id)
    }

    pub fn get_mut(&mut self, id: &str) -> Option<&mut Stored> {
        self.items.iter_mut().find(|s| s.wf.id == id)
    }

    pub fn run_mut(&mut self, run_id: &str) -> Option<&mut Run> {
        self.runs.values_mut().flat_map(|v| v.iter_mut()).find(|r| r.id == run_id)
    }

    pub fn run(&self, run_id: &str) -> Option<&Run> {
        self.runs.values().flat_map(|v| v.iter()).find(|r| r.id == run_id)
    }

    /// Запуск, в котором лежит шаг.
    pub fn run_of_step(&self, step_id: &str) -> Option<String> {
        self.runs.values().flat_map(|v| v.iter()).find(|r| r.steps.iter().any(|s| s.id == step_id)).map(|r| r.id.clone())
    }

    pub fn has_open_run(&self, wf_id: &str) -> bool {
        self.runs.get(wf_id).is_some_and(|v| v.iter().any(Run::is_open))
    }
}

impl Workflows {
    pub fn open(data_dir: &Path) -> Self {
        let dir = data_dir.join("workflows");
        let mut items: Vec<Stored> = Vec::new();
        let mut runs: HashMap<String, Vec<Run>> = HashMap::new();
        let mut versions: HashMap<String, Vec<Version>> = HashMap::new();
        if let Ok(entries) = std::fs::read_dir(&dir) {
            for entry in entries.flatten() {
                let name = entry.file_name().to_string_lossy().into_owned();
                let Some(id) = name.strip_suffix(".json") else { continue };
                if id.ends_with(".runs") || id.ends_with(".versions") {
                    continue;
                }
                let Some(stored) = std::fs::read_to_string(entry.path()).ok().and_then(|t| serde_json::from_str::<Stored>(&t).ok()) else {
                    eprintln!("[workflows] не смог прочитать {name}");
                    continue;
                };
                let list: Vec<Run> = std::fs::read_to_string(dir.join(format!("{id}.runs.json"))).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default();
                runs.insert(stored.wf.id.clone(), list);
                let saved: Vec<Version> = std::fs::read_to_string(dir.join(format!("{id}.versions.json"))).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default();
                versions.insert(stored.wf.id.clone(), saved);
                items.push(stored);
            }
        }
        items.sort_by_key(|s| s.wf.created_at);
        Self { inner: Mutex::new(Inner { dir, versions, items, runs, tokens: HashMap::new() }) }
    }

    pub fn lock(&self) -> std::sync::MutexGuard<'_, Inner> {
        self.inner.lock().unwrap_or_else(|e| e.into_inner())
    }
}

pub fn new_workflow(title: &str, graph: Graph) -> Stored {
    let at = now();
    Stored { wf: Workflow { id: uid("wf"), title: title.trim().to_string(), revision: 1, graph, created_at: at, updated_at: at, last_run: None }, slots: HashMap::new() }
}

/// Строка списка: без графа.
pub fn summary(w: &Workflow) -> Value {
    let steps = w.graph.nodes.iter().filter(|n| !is_annotation(&n.kind)).count();
    serde_json::json!({ "id": w.id, "title": w.title, "steps": steps, "updated_at": w.updated_at, "last_run": w.last_run })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn node(id: &str, kind: &str) -> Node {
        Node { id: id.into(), kind: kind.into(), ..Node::default() }
    }

    fn edge(id: &str, from: &str, to: &str, branch: Option<&str>) -> Edge {
        Edge { id: id.into(), from: from.into(), to: to.into(), branch: branch.map(Into::into) }
    }

    fn graph(nodes: Vec<Node>, edges: Vec<Edge>) -> Graph {
        Graph { nodes, edges, strokes: vec![] }
    }

    fn code(g: &Graph) -> &'static str {
        validate_graph(g).unwrap_err().code
    }

    #[test]
    fn graph_rules_match_the_contract() {
        let ok = graph(vec![node("a", "prompt"), node("b", "bot"), node("c", "output")], vec![edge("e1", "a", "b", None), edge("e2", "b", "c", None)]);
        assert!(validate_graph(&ok).is_ok());
        let dup = graph(vec![node("a", "prompt"), node("b", "bot")], vec![edge("e1", "a", "b", None), edge("e2", "a", "b", None)]);
        assert_eq!(code(&dup), "edge_duplicate");
        let note = graph(vec![node("a", "prompt"), node("n", "note")], vec![edge("e1", "a", "n", None)]);
        assert_eq!(code(&note), "edge_annotation");
        let into_prompt = graph(vec![node("a", "bot"), node("p", "prompt")], vec![edge("e1", "a", "p", None)]);
        assert_eq!(code(&into_prompt), "edge_into_source");
        let from_output = graph(vec![node("o", "output"), node("b", "bot")], vec![edge("e1", "o", "b", None)]);
        assert_eq!(code(&from_output), "edge_from_output");
        let no_branch = graph(vec![node("c", "condition"), node("b", "bot")], vec![edge("e1", "c", "b", None)]);
        assert_eq!(code(&no_branch), "edge_branch_missing");
        let stray = graph(vec![node("a", "prompt"), node("b", "bot")], vec![edge("e1", "a", "b", Some("yes"))]);
        assert_eq!(code(&stray), "edge_branch_forbidden");
        let e = validate_graph(&stray).unwrap_err();
        assert_eq!(e.edge_id.as_deref(), Some("e1"));
        assert_eq!(code(&graph(vec![node("a", "wat")], vec![])), "unknown_kind");
        let mut long = node("a", "prompt");
        long.fields.insert("text".into(), Value::String("я".repeat(MAX_FIELD_CHARS + 1)));
        assert_eq!(code(&graph(vec![long], vec![])), "field_too_long");
    }

    #[test]
    fn schedule_rules() {
        let daily = Schedule { kind: "daily".into(), time: Some("09:00".into()), ..Schedule::default() };
        assert!(validate_schedule(&daily).is_ok());
        let fast = Schedule { kind: "interval".into(), amount: Some(2.0), unit: Some("minutes".into()), ..Schedule::default() };
        assert!(validate_schedule(&fast).is_err(), "интервал меньше трёх минут");
        let okay = Schedule { amount: Some(3.0), ..fast };
        assert!(validate_schedule(&okay).is_ok());
        let weekly = Schedule { kind: "weekly".into(), time: Some("09:00".into()), weekday: Some(8), ..Schedule::default() };
        assert!(validate_schedule(&weekly).is_err());
    }

    fn at(y: i32, m: u32, d: u32, h: u32, mi: u32) -> NaiveDateTime {
        NaiveDate::from_ymd_opt(y, m, d).unwrap().and_hms_opt(h, mi, 0).unwrap()
    }

    /// Часы без пояса: местное время равно «мс с начала эпохи», так срок можно сверить глазами.
    fn next(s: &Schedule, now: NaiveDateTime) -> Option<NaiveDateTime> {
        let epoch = NaiveDate::from_ymd_opt(1970, 1, 1).unwrap().and_hms_opt(0, 0, 0).unwrap();
        let now_ms = (now - epoch).num_milliseconds();
        next_after_with(s, now_ms, |ms| epoch + Duration::milliseconds(ms), |t| (t - epoch).num_milliseconds()).map(|ms| epoch + Duration::milliseconds(ms))
    }

    #[test]
    fn next_slots() {
        // 2026-11-16 — понедельник.
        let time = Some("09:00".to_string());
        let daily = Schedule { kind: "daily".into(), time: time.clone(), ..Schedule::default() };
        assert_eq!(next(&daily, at(2026, 11, 16, 8, 0)), Some(at(2026, 11, 16, 9, 0)));
        assert_eq!(next(&daily, at(2026, 11, 16, 9, 0)), Some(at(2026, 11, 17, 9, 0)));
        let weekdays = Schedule { kind: "weekdays".into(), time: time.clone(), ..Schedule::default() };
        assert_eq!(next(&weekdays, at(2026, 11, 20, 10, 0)), Some(at(2026, 11, 23, 9, 0)), "пятница вечером — понедельник");
        let weekly = Schedule { kind: "weekly".into(), time: time.clone(), weekday: Some(3), ..Schedule::default() };
        assert_eq!(next(&weekly, at(2026, 11, 16, 12, 0)), Some(at(2026, 11, 18, 9, 0)));
        let monthly = Schedule { kind: "monthly".into(), time: time.clone(), day: Some(31), ..Schedule::default() };
        assert_eq!(next(&monthly, at(2026, 11, 16, 12, 0)), Some(at(2026, 11, 30, 9, 0)), "в ноябре 31-го нет: последний день");
        let hourly = Schedule { kind: "hourly".into(), minute: Some(15), ..Schedule::default() };
        assert_eq!(next(&hourly, at(2026, 11, 16, 8, 20)), Some(at(2026, 11, 16, 9, 15)));
        let once = Schedule { kind: "once".into(), time, date: Some("2026-11-20".into()), ..Schedule::default() };
        assert_eq!(next(&once, at(2026, 11, 16, 8, 0)), Some(at(2026, 11, 20, 9, 0)));
        assert_eq!(next(&once, at(2026, 11, 21, 8, 0)), None);
    }

    #[test]
    fn due_slots_advance_first_and_collapse_missed() {
        let mut trigger = node("t", "trigger");
        trigger.fields.insert("trigger".into(), Value::String("schedule".into()));
        trigger.schedule = Some(Schedule { kind: "interval".into(), amount: Some(5.0), unit: Some("minutes".into()), ..Schedule::default() });
        let mut stored = new_workflow("Т", graph(vec![trigger], vec![]));
        let t0 = 1_000_000_000_000;
        let (due, changed) = collect_due(&mut stored, t0);
        assert!(due.is_empty() && changed, "новый узел получает срок и не срабатывает сразу");
        let first = stored.slots["t"].next.unwrap();
        assert_eq!(first, t0 + 5 * 60_000);
        // Компьютер спал сутки: пропущенных слотов много, запуск один, а новый срок — после «сейчас».
        let late = t0 + 24 * 3600_000;
        let (due, _) = collect_due(&mut stored, late);
        assert_eq!(due, vec!["t".to_string()]);
        assert!(stored.slots["t"].next.unwrap() > late);
        let (due, _) = collect_due(&mut stored, late + 1000);
        assert!(due.is_empty());
        // Смена расписания пересчитывает срок.
        stored.wf.graph.nodes[0].schedule.as_mut().unwrap().amount = Some(10.0);
        let (_, changed) = collect_due(&mut stored, late + 2000);
        assert!(changed);
        assert_eq!(stored.slots["t"].next, Some(late + 2000 + 10 * 60_000));
    }
}
