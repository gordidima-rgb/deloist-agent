//! Человеческий язык для карточек разрешений.
//!
//! Текст карточки собирает программа, а не модель: что трогается (файлы, папки, сайты), какого рода это
//! действие и насколько оно рискованно. Слово модели (`purpose`) показывается отдельной строкой «Бот объясняет»
//! и ни на что не влияет. Если команду понять нельзя, карточка так и говорит.

use std::path::{Path, PathBuf};

use serde_json::{json, Value};

use crate::shellplan::{self, OpKind};
use crate::store::Bot;
use crate::App;

/// Где лежит путь по отношению к боту.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Space {
    Own,
    Team(String),
    /// Проект команды (в том числе с программой внутри) или checkout режима разработки.
    Project(String),
    Outside,
    /// Путь неясен или закрыт: секреты, служебные файлы, подстановки, обходы.
    Closed,
}

/// Одна фраза на двух языках.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Phrase {
    pub ru: String,
    pub en: String,
}

macro_rules! ph {
    ($ru:expr, $en:expr) => {
        Phrase { ru: $ru.to_string(), en: $en.to_string() }
    };
}

/// Описание вызова для карточки.
#[derive(Clone, Debug, Default)]
pub struct Described {
    pub text: Phrase,
    /// `low`, `medium`, `high`: считает программа.
    pub risk: &'static str,
    /// `read`, `create`, `change`, `delete`, `run`, `send`, `install`, `release`, `unknown`, …
    pub class: &'static str,
    /// `own`, `team`, `project`, `outside`, `unknown`, `none`.
    pub place: &'static str,
    /// Слово самого бота. Только для показа, решений по нему нет.
    pub purpose: String,
    /// Технические подробности: аргументы как есть, секреты скрыты, длина ограничена.
    pub details: String,
}

impl Described {
    pub fn to_json(&self) -> Value {
        json!({
            "text": shellplan::plain_text(&self.text.ru), "textEn": shellplan::plain_text(&self.text.en), "risk": self.risk, "class": self.class, "place": self.place,
            "purpose": self.purpose,
        })
    }
}

const DETAILS_CAP: usize = 4000;
pub const PURPOSE_CAP: usize = 200;

fn rank(risk: &str) -> u8 {
    match risk {
        "high" => 2,
        "medium" => 1,
        _ => 0,
    }
}

fn worst(a: &'static str, b: &'static str) -> &'static str {
    if rank(b) > rank(a) {
        b
    } else {
        a
    }
}

/// Пояснение бота: одна строка простого текста, не длиннее `PURPOSE_CAP`. Разметку показывает интерфейс как текст.
pub fn purpose_of(args: &Value) -> String {
    let raw = args.get("purpose").and_then(Value::as_str).unwrap_or_default();
    let flat: String = raw.chars().map(|c| if c.is_control() { ' ' } else { c }).collect();
    let flat = flat.split_whitespace().collect::<Vec<_>>().join(" ");
    crate::sensitive::mask(&shellplan::plain_text(&flat).chars().take(PURPOSE_CAP).collect::<String>(), false)
}

/// Аргументы как есть для блока «технические детали»: без `purpose`, секреты скрыты, длина ограничена.
pub fn technical(args: &Value) -> String {
    let mut shown = args.clone();
    if let Some(map) = shown.as_object_mut() {
        map.remove("purpose");
    }
    let text = match &shown {
        Value::Object(map) if map.len() == 1 => match map.values().next() {
            Some(Value::String(s)) => s.clone(),
            _ => serde_json::to_string_pretty(&shown).unwrap_or_default(),
        },
        other => serde_json::to_string_pretty(other).unwrap_or_default(),
    };
    cap_masked(&text)
}

pub fn cap_masked(text: &str) -> String {
    // Невидимые знаки и переключатели направления в деталях показываются кодами, а не прячут текст.
    let masked = shellplan::reveal_odd(&crate::sensitive::mask(text, false));
    let count = masked.chars().count();
    if count <= DETAILS_CAP {
        return masked;
    }
    let head: String = masked.chars().take(DETAILS_CAP).collect();
    format!("{head}\n… обрезано ({count} символов)")
}

/// Параметры и якорь адресов в тексте заменяются многоточием: в них бывают токены. Хост и путь остаются.
pub fn scrub_urls(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(at) = rest.find("://") {
        let (head, tail) = rest.split_at(at + 3);
        out.push_str(head);
        let end = tail.find(|c: char| c.is_whitespace() || matches!(c, '"' | '\'' | '<' | '>' | ')' | '}' | ',')).unwrap_or(tail.len());
        let (url, after) = tail.split_at(end);
        match url.find(['?', '#']) {
            Some(cut) => {
                out.push_str(&url[..cut]);
                out.push_str("?…");
            }
            None => out.push_str(url),
        }
        rest = after;
    }
    out.push_str(rest);
    out
}

/// Адрес для людей: хост и путь, без параметров (в них бывают токены). Хост берётся после `@`: `x.com@evil.test` — это evil.test.
pub fn url_label(raw: &str) -> String {
    let raw = raw.trim();
    let rest = raw.split_once("://").map(|(_, r)| r).unwrap_or(raw);
    let authority = rest.split(['/', '?', '#']).next().unwrap_or_default();
    let host = authority.rsplit('@').next().unwrap_or_default().to_lowercase();
    let after = &rest[authority.len()..];
    let path = after.split(['?', '#']).next().unwrap_or_default();
    let path: String = path.chars().take(60).collect();
    let hidden = after.contains(['?', '#']);
    format!("{host}{path}{}", if hidden { " (параметры адреса скрыты)" } else { "" })
}


// ---------------------------------------------------------------------------
// Где лежит путь
// ---------------------------------------------------------------------------

fn canon(path: &Path) -> PathBuf {
    std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf())
}

/// Где лежит путь из команды или аргумента. Решает программа: сравнивает настоящие пути (ссылки и `..` раскрыты),
/// закрывает секреты, папку программы и файлы автозапуска. Всё сомнительное — `Closed`.
pub fn space_of(app: &App, bot_id: &str, raw: &str) -> Space {
    if shellplan::suspicious_path(raw) {
        return Space::Closed;
    }
    let store = &app.store;
    let workspace = store.workspace(bot_id);
    let target = if crate::reach::is_absolute_like(raw) {
        match crate::tools::write_target(app, bot_id, raw) {
            Ok((real, _)) => real,
            Err(_) => return Space::Closed,
        }
    } else {
        match store.safe_path(bot_id, raw) {
            Ok(path) => path,
            Err(_) => return Space::Closed,
        }
    };
    let Some(real) = crate::reach::real_path(&target) else { return Space::Closed };
    if crate::reach::secret_reason(&real, store.data_dir(), &workspace).is_some() {
        return Space::Closed;
    }
    if real.starts_with(canon(&workspace)) {
        return Space::Own;
    }
    if !crate::cloud::active() {
        for home in crate::rooms::homes_for_turn(app, bot_id) {
            if real.starts_with(canon(&home.path)) {
                return if home.program { Space::Project(home.team) } else { Space::Team(home.team) };
            }
        }
        for project in crate::rooms::projects_for_turn(app, bot_id) {
            if real.starts_with(canon(&project)) {
                return Space::Project(String::new());
            }
        }
    }
    Space::Outside
}

fn space_risk(space: &Space) -> &'static str {
    match space {
        Space::Own | Space::Team(_) => "low",
        Space::Project(_) => "medium",
        Space::Outside | Space::Closed => "high",
    }
}

fn space_tag(space: &Space) -> &'static str {
    match space {
        Space::Own => "own",
        Space::Team(_) => "team",
        Space::Project(_) => "project",
        Space::Outside => "outside",
        Space::Closed => "unknown",
    }
}

fn space_phrase(space: &Space) -> Phrase {
    match space {
        Space::Own => ph!("в своей папке", "in its own folder"),
        Space::Team(name) => ph!(format!("в папке команды «{name}»"), format!("in the folder of team “{name}”")),
        Space::Project(name) if name.is_empty() => ph!("в папке проекта", "in the project folder"),
        Space::Project(name) => ph!(format!("в папке проекта команды «{name}»"), format!("in the project folder of team “{name}”")),
        Space::Outside => ph!("вне своих папок", "outside its folders"),
        Space::Closed => ph!("в закрытом или неясном месте", "in a protected or unclear place"),
    }
}

fn file_name(raw: &str) -> String {
    let trimmed = raw.trim_end_matches(['\\', '/']);
    trimmed.rsplit(['\\', '/']).next().unwrap_or(trimmed).chars().take(80).collect()
}

fn parent_label(raw: &str) -> String {
    let trimmed = raw.trim_end_matches(['\\', '/']);
    match trimmed.rsplit_once(['\\', '/']) {
        Some((parent, _)) => parent.rsplit(['\\', '/']).next().unwrap_or(parent).chars().take(60).collect(),
        None => String::new(),
    }
}

// ---------------------------------------------------------------------------
// Строгая проверка «своя папка» для решения без карточки
// ---------------------------------------------------------------------------

/// Скрипт PowerShell делает только то, что доказано разбором: читает и пишет файлы в своих папках (своя папка бота
/// и папки его команд). Любое сомнение — `false`, и тогда карточка, как раньше.
/// Всё, что меняет файл (запись, дописывание, копия, перенос, папка, удаление), дополнительно не должно задевать
/// файлы автозапуска и `.git`, настройки агентов и сборки, программу и файлы, которые подхватит сама оболочка.
pub fn own_space_script(app: &App, bot_id: &str, script: &str) -> bool {
    if crate::cloud::active() {
        return false;
    }
    let Some(plan) = shellplan::ps_plan(script) else { return false };
    // Папка, созданная этим же скриптом, на диске ещё не папка: `New-Item -ItemType Directory .claude; Move-Item s.json .claude`
    // положил бы файл внутрь. Создание папки вместе с копией или переносом — карточка.
    let makes_dir = plan.ops.iter().any(|op| op.kind == OpKind::MakeDir);
    let moves = plan.ops.iter().any(|op| matches!(op.kind, OpKind::Copy | OpKind::Move));
    if makes_dir && moves {
        return false;
    }
    for op in &plan.ops {
        let mutating = op.kind != OpKind::Read;
        // У копии и переноса вторым идёт назначение: к нему добавляем и имя файла-источника (назначение может оказаться папкой).
        let mut targets: Vec<String> = op.paths.clone();
        if matches!(op.kind, OpKind::Copy | OpKind::Move) && op.paths.len() == 2 {
            let name = op.paths[0].trim_end_matches(['\\', '/']).rsplit(['\\', '/']).next().unwrap_or_default();
            let dest = op.paths[1].trim_end_matches(['\\', '/']);
            if op.paths[1].ends_with(['\\', '/']) || name.is_empty() {
                return false;
            }
            targets.push(format!("{dest}/{name}"));
        }
        for path in &targets {
            let space = space_of(app, bot_id, path);
            if !matches!(space, Space::Own | Space::Team(_) | Space::Project(_)) {
                return false;
            }
            let Ok((real, flagged)) = resolve_target(app, bot_id, path) else { return false };
            if op.kind == OpKind::Read && !crate::reach::safe_read_links(&real) { return false; }
            if mutating {
                // Признак `outside` у write_target — это и «вне папок», и «файл автозапуска»: любой — карточка.
                if flagged || crate::reach::planted_executable(&real) {
                    return false;
                }
                if real.components().any(|c| c.as_os_str().eq_ignore_ascii_case(".git")) {
                    return false;
                }
                let guard = if space == Space::Own { crate::reach::auto_exec_reason(&real) } else { crate::reach::project_guard_reason(&real) };
                if guard.is_some() || crate::reach::auto_exec_reason(&real).is_some() {
                    return false;
                }
            }
        }
        for path in &op.paths {
            let Ok(real) = path_on_disk(app, bot_id, path) else { return false };
            let meta = std::fs::symlink_metadata(&real).ok();
            match op.kind {
                // Запись, дописывание, копия и перенос: цель — файл или ещё не существует. Назначение-папка — карточка.
                OpKind::Write | OpKind::Append => {
                    if meta.is_some_and(|m| !m.is_file()) {
                        return false;
                    }
                }
                OpKind::Copy | OpKind::Move => {
                    if path == &op.paths[1] && meta.as_ref().is_some_and(|m| !m.is_file()) {
                        return false;
                    }
                }
                // Удаление — только существующий обычный файл: не папка и не ссылка.
                OpKind::Delete => {
                    if !meta.is_some_and(|m| m.is_file()) {
                        return false;
                    }
                }
                OpKind::MakeDir | OpKind::Read => {}
            }
        }
        // У переноса исходник тоже исчезает: он должен быть обычным файлом.
        if op.kind == OpKind::Move {
            let Ok(real) = path_on_disk(app, bot_id, &op.paths[0]) else { return false };
            if !std::fs::symlink_metadata(&real).is_ok_and(|m| m.is_file()) {
                return false;
            }
        }
    }
    true
}

/// Настоящий путь и признак `outside` от `tools::write_target` (вне папок или файл автозапуска внутри проекта).
fn resolve_target(app: &App, bot_id: &str, raw: &str) -> Result<(PathBuf, bool), ()> {
    let (target, flagged) = crate::tools::write_target(app, bot_id, raw).map_err(|_| ())?;
    Ok((crate::reach::real_path(&target).ok_or(())?, flagged))
}

fn path_on_disk(app: &App, bot_id: &str, raw: &str) -> Result<PathBuf, ()> {
    resolve_target(app, bot_id, raw).map(|(real, _)| real)
}

/// Почему запись по этому пути нельзя принять молча: файл запускается сам, это запускаемый файл или путь записан так,
/// что прячет настоящее имя. Считает программа; слова бота сюда не попадают.
fn guard_note(app: &App, bot_id: &str, raw: &str) -> Option<Phrase> {
    let real = path_on_disk(app, bot_id, raw).ok();
    let given = Path::new(raw);
    let cleaned: Vec<&Path> = [real.as_deref(), Some(given)].into_iter().flatten().collect();
    let odd_spelling = raw.rsplit(['\\', '/']).any(|part| part.contains(':') && !(part.len() == 2 && part.ends_with(':')))
        || raw.split(['\\', '/']).any(|part| part.find('~').is_some_and(|at| part[at + 1..].chars().next().is_some_and(|d| d.is_ascii_digit())))
        || raw.split(['\\', '/']).any(|part| part.len() > 1 && (part.ends_with('.') || part.ends_with(' ')) && part != "..");
    if odd_spelling || shellplan::suspicious_path(raw) {
        return Some(ph!(
            "Путь записан необычно (поток файла «:», короткое имя «~1», точка или пробел на конце): так прячут настоящее имя файла. Подтверждай, только если ты сам об этом просил",
            "The path is written oddly (a file stream “:”, a short name “~1”, a trailing dot or space): that hides the real file name. Confirm only if you asked for this yourself"
        ));
    }
    for path in &cleaned {
        let reason = crate::reach::project_guard_reason(path).or_else(|| crate::reach::auto_exec_reason(path));
        if let Some(reason) = reason {
            return Some(ph!(
                format!("Этот файл запускается сам или задаёт, что запускать ({reason}). Подтверждай, только если ты это просил"),
                "This file runs by itself or decides what runs (git hooks, agent settings, build or startup files). Confirm only if you asked for this".to_string()
            ));
        }
        if path.components().any(|c| c.as_os_str().eq_ignore_ascii_case(".git")) {
            return Some(ph!(
                "Это внутренности репозитория git: хуки и настройки запускают программы сами. Подтверждай, только если ты это просил",
                "This is the inside of a git repository: hooks and settings run programs by themselves. Confirm only if you asked for this"
            ));
        }
        if crate::reach::planted_executable(path) {
            return Some(ph!(
                "Это запускаемый файл (программа или сценарий). Подтверждай, только если ты это просил",
                "This is a runnable file (a program or a script). Confirm only if you asked for this"
            ));
        }
    }
    None
}

fn with_note(text: &mut Phrase, risk: &mut &'static str, note: Option<Phrase>) {
    if let Some(note) = note {
        text.ru = format!("{}. {}", text.ru.trim_end_matches('.'), note.ru);
        text.en = format!("{}. {}", text.en.trim_end_matches('.'), note.en);
        *risk = "high";
    }
}

// ---------------------------------------------------------------------------
// Описание вызова
// ---------------------------------------------------------------------------

fn arg<'a>(args: &'a Value, key: &str) -> &'a str {
    args.get(key).and_then(Value::as_str).unwrap_or_default()
}

fn who(bot: &Bot) -> String {
    bot.name.clone()
}

/// Описание вызова нашего инструмента.
pub fn call(app: &App, bot: &Bot, name: &str, args: &Value) -> Described {
    let mut d = Described { purpose: purpose_of(args), details: technical(args), place: "none", risk: "medium", class: "run", ..Described::default() };
    let me = who(bot);
    match name {
        "write_file" => {
            let raw = arg(args, "path");
            let space = space_of(app, &bot.id, raw);
            let exists = path_on_disk(app, &bot.id, raw).ok().is_some_and(|p| p.is_file());
            let chars = arg(args, "content").chars().count();
            let file = file_name(raw);
            d.class = if exists { "change" } else { "create" };
            d.place = space_tag(&space);
            d.risk = space_risk(&space);
            let place = space_phrase(&space);
            let (verb_ru, verb_en) = if exists { ("изменить", "change") } else { ("создать", "create") };
            let mut text = ph!(
                format!("{me} хочет {verb_ru} файл «{file}» {}: {chars} знаков", place.ru),
                format!("{me} wants to {verb_en} the file “{file}” {}: {chars} characters", place.en)
            );
            if space == Space::Outside {
                let parent = parent_label(raw);
                if !parent.is_empty() {
                    text.ru.push_str(&format!(" (папка «{parent}»)"));
                    text.en.push_str(&format!(" (folder “{parent}”)"));
                }
            }
            with_note(&mut text, &mut d.risk, guard_note(app, &bot.id, raw));
            d.text = text;
        }
        "read_file" | "list_files" => {
            // Файловый режим: файл этого компьютера вне папок бота. Карточка — одна строка
            // о файле и его папке; запасной текст — для пути, который карточка не открывала.
            let raw = arg(args, "path");
            let (folder, label) = crate::tools::file_mode_place(app, &bot.id, name, args)
                .unwrap_or_else(|| (String::new(), file_name(raw)));
            d.class = "read";
            d.place = "outside";
            d.risk = "medium";
            d.text = if name == "list_files" {
                ph!(
                    format!("{me} хочет посмотреть список файлов папки «{folder}» на этом компьютере"),
                    format!("{me} wants to list the files of the folder “{folder}” on this computer")
                )
            } else {
                ph!(
                    format!("{me} хочет прочитать файл с этого компьютера: «{label}» (папка «{folder}»)"),
                    format!("{me} wants to read the file “{label}” on this computer (folder “{folder}”)")
                )
            };
        }
        "run_command" => describe_command(app, bot, arg(args, "command"), &mut d),
        "web_fetch" => {
            let label = url_label(arg(args, "url"));
            d.class = "read";
            d.risk = "medium";
            d.text = ph!(format!("{me} хочет открыть страницу {label}"), format!("{me} wants to open the page {label}"));
        }
        "web_search" => {
            let query: String = arg(args, "query").chars().take(120).collect();
            d.class = "read";
            d.risk = "low";
            d.text = ph!(format!("{me} хочет поискать в интернете: «{query}»"), format!("{me} wants to search the web: “{query}”"));
        }
        "create_bot" => {
            let n = arg(args, "name");
            d.class = "create";
            d.risk = "high";
            d.text = ph!(format!("{me} хочет создать нового бота «{n}»"), format!("{me} wants to create a new bot “{n}”"));
        }
        "update_bot" => {
            let fields: Vec<&str> = args.as_object().map(|m| m.keys().map(String::as_str).filter(|k| !matches!(*k, "bot_id" | "purpose")).collect()).unwrap_or_default();
            d.class = "change";
            d.risk = "high";
            d.text = ph!(
                format!("{me} хочет изменить настройки бота (поля: {})", fields.join(", ")),
                format!("{me} wants to change a bot’s settings (fields: {})", fields.join(", "))
            );
        }
        "delete_todo" | "archive_habit" | "delete_goal" => {
            let label = crate::tools::label(name);
            d.class = "delete";
            d.risk = "high";
            d.text = ph!(format!("{me} просит разрешение: {label}"), format!("{me} asks permission: {label}"));
        }
        "create_workflow" | "edit_workflow" | "run_workflow" | "delete_workflow" => {
            let summary = crate::workflow_tools::summary(app, name, args);
            d.class = match name {
                "create_workflow" => "create",
                "edit_workflow" => "change",
                "run_workflow" => "run",
                _ => "delete",
            };
            d.risk = "high";
            d.text = ph!(format!("{me}: {summary}"), format!("{me}: {summary}"));
            d.details = crate::workflow_tools::full_summary(app, name, args);
        }
        "mail_send" | "mail_draft" => {
            let (ru, en, detail) = crate::mail::describe(app, bot, name, args);
            d.class = if name == "mail_send" { "send" } else { "create" };
            d.place = "outside";
            d.risk = if name == "mail_send" { "high" } else { "low" };
            d.text = ph!(format!("{me} хочет {ru}"), format!("{me} wants to {en}"));
            d.details = detail;
        }
        "generate_image" => {
            d.class = "spend";
            d.risk = "medium";
            d.text = ph!(format!("{me} хочет нарисовать картинку у платного сервиса"), format!("{me} wants to draw a picture with a paid service"));
        }
        "remember" => {
            d.class = "change";
            d.risk = "low";
            d.text = ph!(format!("{me} хочет запомнить это в своей памяти"), format!("{me} wants to save this to its memory"));
        }
        "save_skill" => {
            d.class = "create";
            d.risk = "medium";
            d.text = ph!(format!("{me} хочет сохранить навык «{}»", arg(args, "name")), format!("{me} wants to save the skill “{}”", arg(args, "name")));
        }
        _ if name.starts_with("mcp__") => {
            let (server, tool) = crate::mcp::split_name(name).unwrap_or_default();
            let summary = summarize_args(args);
            d.class = "tool";
            d.risk = "medium";
            d.text = ph!(
                format!("{me} хочет вызвать инструмент «{tool}» подключённого сервера «{server}»{summary}"),
                format!("{me} wants to call the tool “{tool}” of the connected server “{server}”{summary}")
            );
        }
        _ => {
            let label = crate::tools::label(name);
            d.text = ph!(format!("{me} просит разрешение: {label}"), format!("{me} asks permission: {label}"));
        }
    }
    d
}

/// Аргументы в двух словах: адреса — хост и путь (без параметров), длинное обрезано.
pub fn summarize_args(args: &Value) -> String {
    let Some(map) = args.as_object() else { return String::new() };
    let mut parts: Vec<String> = Vec::new();
    for (key, value) in map {
        if key == "purpose" {
            continue;
        }
        let shown = match value {
            Value::String(s) if s.contains("://") => url_label(s),
            Value::String(s) => crate::sensitive::mask(&s.chars().take(60).collect::<String>(), false),
            Value::Number(n) => n.to_string(),
            Value::Bool(b) => b.to_string(),
            _ => "…".into(),
        };
        parts.push(format!("{key}: {shown}"));
        if parts.len() >= 4 {
            break;
        }
    }
    if parts.is_empty() {
        String::new()
    } else {
        format!(" ({})", parts.join("; "))
    }
}

/// Слова про файл: ««a.md»» или «3 файла».
fn files_phrase(paths: &[String]) -> Phrase {
    let mut names: Vec<String> = Vec::new();
    for p in paths {
        let n = file_name(p);
        if !names.contains(&n) {
            names.push(n);
        }
    }
    match names.len() {
        0 => ph!("файлы", "files"),
        1 => ph!(format!("файл «{}»", names[0]), format!("the file “{}”", names[0])),
        2 | 3 => ph!(format!("файлы «{}»", names.join("», «")), format!("the files “{}”", names.join("”, “"))),
        n => ph!(format!("{n} файлов"), format!("{n} files")),
    }
}

fn describe_command(app: &App, bot: &Bot, command: &str, d: &mut Described) {
    let me = who(bot);
    d.class = "run";
    // Выпуск наружу.
    if let Some(label) = crate::reach::release_command(command) {
        d.class = "release";
        d.risk = "high";
        d.text = ph!(format!("{me} хочет отправить изменения наружу: {label}"), format!("{me} wants to send changes out: {label}"));
        return;
    }
    // Скрипт PowerShell: из командной строки cmd или (для агента ACP) уже готовый.
    let script = shellplan::script_from_cmdline(command).or_else(|| crate::reach::inner_shell_script(command));
    describe_script_or_text(app, bot, command, script.as_deref(), d);
}

/// Общая часть для `run_command` и запроса ACP.
pub fn describe_script_or_text(app: &App, bot: &Bot, whole: &str, script: Option<&str>, d: &mut Described) {
    let me = who(bot);
    if let Some(script) = script {
        if let Some(plan) = shellplan::ps_plan(script) {
            describe_plan(app, bot, &plan, d);
            return;
        }
    }
    // Простые команды cmd (`del`, `copy`, `echo > файл`…): разбор по глаголам.
    if script.is_none() {
        if let Some(plan) = cmd_plan(whole) {
            describe_plan(app, bot, &plan, d);
            return;
        }
    }
    // Не поняли: честно говорим, что знаем.
    let text = script.unwrap_or(whole);
    let scan = shellplan::scan(whole);
    let scan2 = shellplan::scan(text);
    let mut parts_ru: Vec<String> = Vec::new();
    let mut parts_en: Vec<String> = Vec::new();
    let risk = "high";
    let mut class = "unknown";
    if scan.encoded || scan2.encoded {
        parts_ru.push("запускает закодированную команду, прочитать её нельзя".into());
        parts_en.push("runs an encoded command that cannot be read".into());
    }
    if scan.eval || scan2.eval {
        parts_ru.push("выполняет текст как код".into());
        parts_en.push("executes text as code".into());
    }
    let installs: Vec<String> = scan.installs.iter().chain(scan2.installs.iter()).cloned().collect();
    if !installs.is_empty() {
        parts_ru.push(format!("устанавливает программы ({})", installs.join(", ")));
        parts_en.push(format!("installs software ({})", installs.join(", ")));
        class = "install";
    }
    let hosts: Vec<String> = scan.hosts.iter().chain(scan2.hosts.iter()).filter(|h| !h.is_empty()).cloned().collect();
    if !scan.hosts.is_empty() || !scan2.hosts.is_empty() {
        if hosts.is_empty() {
            parts_ru.push("обращается к сети".into());
            parts_en.push("talks to the network".into());
        } else {
            parts_ru.push(format!("обращается к сети (в тексте есть адреса: {}; настоящие могут быть другими)", hosts.join(", ")));
            parts_en.push(format!("talks to the network (addresses seen in the text: {}; the real ones may differ)", hosts.join(", ")));
        }
        class = if class == "unknown" { "send" } else { class };
    }
    let deletes: Vec<String> = scan.deletes.iter().chain(scan2.deletes.iter()).cloned().collect();
    if !deletes.is_empty() {
        // Имена из непонятной команды не показываем: среди них легко спрятать «безобидную» приманку. Пути не определены.
        parts_ru.push("удаляет файлы (пути не определены)".into());
        parts_en.push("deletes files (paths not determined)".into());
        class = if class == "unknown" { "delete" } else { class };
    }
    let writes: Vec<String> = scan.writes.iter().chain(scan2.writes.iter()).cloned().collect();
    if !writes.is_empty() {
        parts_ru.push("записывает файлы (пути не определены)".into());
        parts_en.push("writes files (paths not determined)".into());
        class = if class == "unknown" { "change" } else { class };
    }
    if !scan.runs.is_empty() || !scan2.runs.is_empty() {
        let mut tools: Vec<String> = Vec::new();
        for tool in scan.runs.iter().chain(scan2.runs.iter()) {
            if !tools.contains(tool) {
                tools.push(tool.clone());
            }
        }
        parts_ru.push(format!("запускает другие программы ({})", tools.join(", ")));
        parts_en.push(format!("starts other programs ({})", tools.join(", ")));
    }
    if parts_ru.is_empty() {
        d.text = ph!(
            format!("{me} хочет выполнить команду. Запускает программу, смысл не определён"),
            format!("{me} wants to run a command. It starts a program and its meaning cannot be determined")
        );
    } else {
        d.text = ph!(
            format!("{me} хочет выполнить команду, которая {}. Разбор неполный: проверь технические детали", parts_ru.join(", ")),
            format!("{me} wants to run a command that {}. The analysis is incomplete: check the technical details", parts_en.join(", "))
        );
    }
    // Непонятная команда всегда с высоким риском, что бы в ней ни нашли.
    d.risk = risk;
    d.class = class;
    d.place = "unknown";
}

/// Простые команды cmd: только знакомые глаголы и пути; любое незнакомое слово, подстановка или вложенная оболочка — `None`.
fn cmd_plan(command: &str) -> Option<shellplan::Plan> {
    if command.chars().any(|c| matches!(c, '%' | '^' | '`' | '$' | '(' | ')' | '*' | '?')) || shellplan::has_lookalikes(command) {
        return None;
    }
    let mut plan = shellplan::Plan::default();
    // Сегменты по `&`, `|` и переводу строки вне кавычек.
    let mut segments: Vec<String> = Vec::new();
    let mut current = String::new();
    let mut quote = false;
    for c in command.chars() {
        match c {
            '"' => {
                quote = !quote;
                current.push(c);
            }
            '&' | '|' | '\n' | '\r' if !quote => segments.push(std::mem::take(&mut current)),
            _ => current.push(c),
        }
    }
    segments.push(current);
    for segment in segments.iter().filter(|s| !s.trim().is_empty()) {
        // Слова с учётом кавычек; `>` и `>>` отделяются.
        let spaced = segment.replace(">>", " > ").replace('>', " > ");
        let mut words: Vec<String> = Vec::new();
        let mut word = String::new();
        let mut q = false;
        for c in spaced.chars() {
            match c {
                '"' => q = !q,
                c if c.is_whitespace() && !q => {
                    if !word.is_empty() {
                        words.push(std::mem::take(&mut word));
                    }
                }
                c => word.push(c),
            }
        }
        if !word.is_empty() {
            words.push(word);
        }
        let verb = words.first()?.to_lowercase();
        let verb = verb.trim_start_matches('@').to_string();
        let mut rest: Vec<String> = Vec::new();
        let mut redirects: Vec<String> = Vec::new();
        let mut it = words.iter().skip(1);
        while let Some(w) = it.next() {
            if w == ">" {
                redirects.push(it.next()?.clone());
            } else if !w.starts_with('/') && !w.starts_with('-') {
                rest.push(w.clone());
            }
        }
        let kind = match verb.as_str() {
            "del" | "erase" | "rd" | "rmdir" | "rm" => Some(OpKind::Delete),
            "copy" | "xcopy" => Some(OpKind::Copy),
            "move" | "ren" | "rename" => Some(OpKind::Move),
            "mkdir" | "md" => Some(OpKind::MakeDir),
            "type" | "dir" | "more" | "findstr" | "tree" => Some(OpKind::Read),
            "echo" | "cd" | "chdir" | "ver" | "where" | "whoami" | "hostname" => None,
            _ => return None,
        };
        if let Some(kind) = kind {
            if rest.is_empty() {
                return None;
            }
            plan.ops.push(shellplan::Op { kind, paths: if matches!(kind, OpKind::Copy | OpKind::Move) { rest.iter().take(2).cloned().collect() } else { rest.clone() } });
        }
        for target in redirects {
            if target.eq_ignore_ascii_case("nul") {
                continue;
            }
            plan.ops.push(shellplan::Op { kind: OpKind::Write, paths: vec![target] });
        }
    }
    Some(plan)
}

fn describe_plan(app: &App, bot: &Bot, plan: &shellplan::Plan, d: &mut Described) {
    let me = who(bot);
    let mut risk = "low";
    let mut place_tag = "none";
    let mut seen: Vec<Space> = Vec::new();
    let mut changed: Vec<String> = Vec::new();
    let mut created = false;
    let mut deleted: Vec<String> = Vec::new();
    let mut read: Vec<String> = Vec::new();
    for op in &plan.ops {
        for path in &op.paths {
            let space = space_of(app, &bot.id, path);
            risk = worst(risk, space_risk(&space));
            if place_tag == "none" || rank(space_risk(&space)) > rank(match place_tag { "outside" | "unknown" => "high", "project" => "medium", _ => "low" }) {
                place_tag = space_tag(&space);
            }
            if !seen.contains(&space) {
                seen.push(space);
            }
            match op.kind {
                OpKind::Read => read.push(path.clone()),
                OpKind::Delete => {
                    risk = worst(risk, "medium");
                    deleted.push(path.clone());
                }
                OpKind::Move => {
                    risk = worst(risk, "medium");
                    changed.push(path.clone());
                }
                OpKind::Write | OpKind::Append | OpKind::MakeDir | OpKind::Copy => {
                    let exists = path_on_disk(app, &bot.id, path).ok().is_some_and(|p| p.exists());
                    created |= !exists;
                    changed.push(path.clone());
                }
            }
        }
    }
    let place = match seen.len() {
        0 => Phrase::default(),
        1 => space_phrase(&seen[0]),
        _ => ph!("в нескольких местах", "in several places"),
    };
    let join = |a: &str, b: &Phrase| if b.ru.is_empty() { a.to_string() } else { format!("{a} {}", b.ru) };
    let _ = join;
    let mut ru: Vec<String> = Vec::new();
    let mut en: Vec<String> = Vec::new();
    if !changed.is_empty() {
        let f = files_phrase(&changed);
        let (vr, ve) = if created && !read.iter().any(|r| changed.contains(r)) { ("создать или изменить", "create or change") } else { ("изменить", "change") };
        let mut r = format!("{vr} {}", f.ru);
        let mut e = format!("{ve} {}", f.en);
        if plan.replaces > 0 {
            r.push_str(&format!(": заменить {} фрагм. текста", plan.replaces));
            e.push_str(&format!(": replace {} text fragment(s)", plan.replaces));
        }
        ru.push(r);
        en.push(e);
    }
    if !deleted.is_empty() {
        let f = files_phrase(&deleted);
        ru.push(format!("удалить {}", f.ru));
        en.push(format!("delete {}", f.en));
    }
    if !read.is_empty() && changed.is_empty() && deleted.is_empty() {
        let f = files_phrase(&read);
        ru.push(format!("прочитать {}", f.ru));
        en.push(format!("read {}", f.en));
    }
    if ru.is_empty() {
        ru.push("выполнить простую команду без доступа к файлам".into());
        en.push("run a simple command with no file access".into());
    }
    d.class = if !deleted.is_empty() {
        "delete"
    } else if !changed.is_empty() {
        if created { "create" } else { "change" }
    } else {
        "read"
    };
    let mut text = ph!(format!("{me} хочет {} {}", ru.join(" и "), place.ru), format!("{me} wants to {} {}", en.join(" and "), place.en));
    text.ru = text.ru.trim_end().to_string();
    text.en = text.en.trim_end().to_string();
    // Причины, по которым запись нельзя принять молча: для каждого изменяемого пути и для имени файла в папке-назначении.
    let mut risk = risk;
    let mut note: Option<Phrase> = None;
    for op in plan.ops.iter().filter(|op| op.kind != OpKind::Read) {
        let mut paths = op.paths.clone();
        if matches!(op.kind, OpKind::Copy | OpKind::Move) && op.paths.len() == 2 {
            let name = op.paths[0].trim_end_matches(['\\', '/']).rsplit(['\\', '/']).next().unwrap_or_default().to_string();
            paths.push(format!("{}/{name}", op.paths[1].trim_end_matches(['\\', '/'])));
            let dest_is_dir = op.paths[1].ends_with(['\\', '/']) || path_on_disk(app, &bot.id, &op.paths[1]).ok().is_some_and(|p| p.is_dir());
            if dest_is_dir && note.is_none() {
                note = Some(ph!(
                    "Назначение: папка, файл ляжет в неё под своим именем, и куда именно, по скрипту не проверить. Подтверждай, только если ты это просил",
                    "The destination is a folder, the file lands in it under its own name and the script cannot be checked for where. Confirm only if you asked for this"
                ));
            }
        }
        for path in paths {
            if note.is_none() {
                note = guard_note(app, &bot.id, &path);
            }
        }
    }
    if plan.ops.iter().any(|op| op.kind == OpKind::MakeDir) && plan.ops.iter().any(|op| matches!(op.kind, OpKind::Copy | OpKind::Move)) && note.is_none() {
        note = Some(ph!(
            "Скрипт создаёт папку и переносит в неё файл: куда он ляжет, по тексту не проверить. Подтверждай, только если ты это просил",
            "The script creates a folder and moves a file into it: where it lands cannot be checked from the text. Confirm only if you asked for this"
        ));
    }
    with_note(&mut text, &mut risk, note);
    d.text = text;
    d.risk = risk;
    d.place = place_tag;
}

// ---------------------------------------------------------------------------
// Запросы агента ACP
// ---------------------------------------------------------------------------

/// Инструмент нашего моста в названии вызова: слово сразу после имени моста, за любыми разделителями.
fn bridge_tool_in(title: &str) -> Option<String> {
    let (_, after) = title.split_once(crate::drivers::acp::bridge_name())?;
    let tool: String = after.trim_start_matches(|c: char| !c.is_ascii_alphanumeric()).chars().take_while(|c| c.is_ascii_alphanumeric() || *c == '_').collect();
    (!tool.is_empty()).then_some(tool)
}

/// Какой MCP-инструмент вызывает агент: `(сервер, инструмент)`. Из `rawInput` (`server`/`tool`) или из названия
/// вида `mcp.сервер.инструмент` / `Tool: сервер/инструмент`.
pub fn acp_mcp_tool(call: &Value) -> Option<(String, String)> {
    let raw = &call["rawInput"];
    let s = |v: &Value| v.as_str().map(str::trim).filter(|t| !t.is_empty()).map(str::to_string);
    if let (Some(server), Some(tool)) = (s(&raw["server"]).or_else(|| s(&raw["mcp_server"])), s(&raw["tool"]).or_else(|| s(&raw["toolName"]))) {
        return Some((server, tool));
    }
    let title = call["title"].as_str().unwrap_or_default().trim();
    // Наш мост узнаём по его секретному имени, где бы оно ни стояло и чем бы ни было отделено: Codex пишет
    // «mcp.<мост>.<инструмент>», Claude «mcp__<мост>__<инструмент>», Antigravity «Run mcp_<мост>_<инструмент>», а новый
    // агент напишет по-своему. Раньше каждый формат дописывали руками, и Gemini на каждом шаге ждал карточку
    // (Маркетолог X, 19.11.2026). Имени моста посторонний сервер не знает, так что подделать его нельзя.
    if let Some(tool) = bridge_tool_in(title) {
        return Some((crate::drivers::acp::bridge_name().to_string(), tool));
    }
    let (title, had_tool) = match title.strip_prefix("Tool:") {
        Some(t) => (t.trim(), true),
        None => (title, false),
    };
    let stripped = title.strip_prefix("mcp.").or_else(|| title.strip_prefix("mcp__")).or_else(|| title.strip_prefix("mcp:")).map(str::trim);
    let title = match stripped {
        Some(body) => body,
        None if had_tool => title,
        None => return None,
    };
    let token = title.split_whitespace().next()?;
    let (server, tool) = token.split_once(['.', '/', ':']).or_else(|| token.split_once("__"))?;
    let ok = |t: &str| !t.is_empty() && t.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-'));
    (ok(server) && ok(tool)).then(|| (server.to_string(), tool.to_string()))
}

/// Что за действие просит агент, когда вид ничего не говорит (`other`): инструмент нашего браузера, спрятанный в обёртке
/// агента (`execute` у OpenCode), либо название вызова. Только для подписи карточки: решения по нему не принимаются.
pub fn acp_what(call: &Value) -> Option<String> {
    let inner = bridge_tool_in(&call["rawInput"].to_string());
    let title = scrub_urls(call["title"].as_str().unwrap_or_default().trim());
    let title: String = title.chars().take(60).collect();
    match (inner, title.is_empty()) {
        (Some(tool), false) if tool != title => Some(format!("{title} → {tool}")),
        (Some(tool), _) => Some(tool),
        (None, false) => Some(title),
        (None, true) => None,
    }
}

/// Описание запроса агента ACP: по тому, что он реально прислал.
pub fn acp(app: &App, bot: &Bot, call: &Value, paths: &[PathBuf]) -> Described {
    let me = who(bot);
    let kind = call["kind"].as_str().unwrap_or("other");
    let raw = &call["rawInput"];
    let mut d = Described { risk: "high", class: "unknown", place: "unknown", ..Described::default() };
    // Всё, что агент прислал, как технические детали (скрытые секреты, ограничено).
    let mut details = String::new();
    if let Some(title) = call["title"].as_str().filter(|t| !t.is_empty()) {
        details.push_str(&format!("название: {title}\n"));
    }
    details.push_str(&format!("вид: {kind}\n"));
    for path in paths {
        details.push_str(&format!("файл: {}\n", path.display()));
    }
    let command = match &raw["command"] {
        Value::Array(parts) => parts.iter().filter_map(Value::as_str).map(str::to_string).collect::<Vec<_>>(),
        Value::String(s) => vec![s.clone()],
        _ => Vec::new(),
    };
    // Antigravity (Gemini) присылает команду только названием и исполняет её в PowerShell.
    let title_only = command.is_empty() && kind == "execute" && paths.is_empty() && raw["shell_command"].is_null()
        && acp_mcp_tool(call).is_none() && call["title"].as_str().is_some_and(|t| !t.trim().is_empty() && !t.contains("://"));
    let command = if title_only { vec![call["title"].as_str().unwrap_or_default().to_string()] } else { command };
    if !command.is_empty() {
        details.push_str(&format!("команда: {}\n", command.join(" ")));
    }
    let content = tool_text(&call["content"]);
    if !content.is_empty() {
        details.push_str(&format!("{content}\n"));
    }
    if raw.is_object() && command.is_empty() {
        details.push_str(&format!("параметры: {}\n", serde_json::to_string(raw).unwrap_or_default()));
    }
    let nothing = call["title"].as_str().unwrap_or_default().is_empty() && raw.is_null() && paths.is_empty() && content.is_empty();
    if nothing {
        details = format!("{}{}", details, "(агент подробностей не передал)");
        d.text = ph!(
            format!("{me}: агент просит выполнить действие, подробностей не передал. Повышенный риск: лучше отклонить"),
            format!("{me}: the agent asks to perform an action and gave no details. Elevated risk: better to deny")
        );
        d.details = cap_masked(&scrub_urls(&details));
        return d;
    }
    d.details = cap_masked(&scrub_urls(&details));
    if let Some((server, tool)) = acp_mcp_tool(call) {
        let args = if raw["arguments"].is_object() { &raw["arguments"] } else if raw["args"].is_object() { &raw["args"] } else { raw };
        let summary = summarize_args(args);
        d.class = "tool";
        d.risk = "medium";
        d.place = "none";
        d.text = ph!(
            format!("{me}: агент просит вызвать инструмент «{tool}» сервера «{server}»{summary}"),
            format!("{me}: the agent asks to call the tool “{tool}” of the server “{server}”{summary}")
        );
        return d;
    }
    match kind {
        "execute" if !command.is_empty() => {
            let script = shellplan::script_from_argv(&command).or_else(|| if command.len() == 1 { shellplan::script_from_cmdline(&command[0]) } else { None })
                .or_else(|| title_only.then(|| command[0].clone()));
            if let Some(label) = crate::reach::release_command(&command.join(" ")) {
                d.class = "release";
                d.text = ph!(format!("{me}: агент хочет отправить изменения наружу: {label}"), format!("{me}: the agent wants to send changes out: {label}"));
                return d;
            }
            describe_script_or_text(app, bot, &command.join(" "), script.as_deref(), &mut d);
            d.text.ru = d.text.ru.replacen(&format!("{me} хочет"), &format!("{me}: агент хочет"), 1);
            d.text.en = d.text.en.replacen(&format!("{me} wants to"), &format!("{me}: the agent wants to"), 1);
        }
        "edit" | "delete" | "move" | "read" if !paths.is_empty() => {
            let space = space_of(app, &bot.id, &paths[0].to_string_lossy());
            let (ru, en) = match kind {
                "edit" => ("изменить", "change"),
                "delete" => ("удалить", "delete"),
                "move" => ("переместить", "move"),
                _ => ("прочитать", "read"),
            };
            d.class = match kind {
                "edit" => "change",
                "delete" => "delete",
                "move" => "change",
                _ => "read",
            };
            d.place = space_tag(&space);
            d.risk = if kind == "read" { "low" } else { space_risk(&space) };
            let place = space_phrase(&space);
            let f = files_phrase(&paths.iter().map(|p| p.to_string_lossy().to_string()).collect::<Vec<_>>());
            d.text = ph!(format!("{me}: агент хочет {ru} {} {}", f.ru, place.ru), format!("{me}: the agent wants to {en} {} {}", f.en, place.en));
        }
        "fetch" | "search" => {
            let title_url = call["title"].as_str().and_then(|t| t.split_whitespace().find(|w| w.starts_with("http")));
            let target = raw["url"].as_str().or(title_url).map(url_label).or_else(|| raw["query"].as_str().map(|q| q.chars().take(80).collect()));
            d.class = "read";
            d.risk = "medium";
            d.place = "none";
            let what = target.unwrap_or_default();
            d.text = ph!(format!("{me}: агент хочет обратиться в интернет {what}"), format!("{me}: the agent wants to reach the internet {what}"));
        }
        _ => {
            let title = acp_what(call).unwrap_or_default();
            d.text = ph!(
                format!("{me}: агент просит действие «{title}», что именно оно делает, программа не определила"),
                format!("{me}: the agent asks for the action “{title}”; the program could not determine what exactly it does")
            );
        }
    }
    d
}

fn tool_text(content: &Value) -> String {
    let Some(items) = content.as_array() else { return String::new() };
    let mut parts: Vec<String> = Vec::new();
    for item in items {
        match item["type"].as_str() {
            Some("content") => {
                if let Some(text) = item["content"]["text"].as_str() {
                    parts.push(text.chars().take(800).collect());
                }
            }
            Some("diff") => parts.push(format!("правка файла {}", item["path"].as_str().unwrap_or_default())),
            _ => {}
        }
    }
    parts.join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls_are_shown_without_parameters_and_the_real_host_wins() {
        assert_eq!(url_label("https://x.com/home?token=abc#frag"), "x.com/home (параметры адреса скрыты)");
        assert_eq!(url_label("https://x.com@evil.test/a"), "evil.test/a");
        assert_eq!(url_label("https://X.COM.evil.test/"), "x.com.evil.test/");
        assert!(!url_label("https://a.b/c?token=secret").contains("secret"));
    }

    #[test]
    fn query_strings_and_fragments_are_cut_out_of_any_text() {
        assert_eq!(scrub_urls("open https://a.b/c?token=1&x=2 now"), "open https://a.b/c?… now");
        assert_eq!(scrub_urls("{\"url\":\"https://a.b/p#frag\"}"), "{\"url\":\"https://a.b/p?…\"}");
        assert_eq!(scrub_urls("no links"), "no links");
    }

    #[test]
    fn the_bots_purpose_is_one_short_plain_line_and_never_a_risk_source() {
        let long = "а".repeat(500);
        let p = purpose_of(&json!({ "purpose": format!("<b>x</b>\n\n  {long}") }));
        assert!(p.chars().count() <= PURPOSE_CAP + 20 && !p.contains('\n'));
        assert!(p.starts_with("<b>x</b>"), "разметка остаётся текстом, экранирует интерфейс");
        assert_eq!(purpose_of(&json!({})), "");
        // Секрет в пояснении скрывается так же, как в деталях.
        assert!(!purpose_of(&json!({ "purpose": "ключ sk-abcdefghijklmnopqrstuvwxyz0123456789" })).contains("abcdefghijklmnop"));
        assert!(!technical(&json!({ "command": "x", "purpose": "объяснение" })).contains("объяснение"));
    }

    #[test]
    fn acp_tool_names_are_read_from_the_raw_input_or_the_title() {
        let a = acp_mcp_tool(&json!({ "title": "mcp.deloist_browser.browser_nav" }));
        assert_eq!(a, Some(("deloist_browser".into(), "browser_nav".into())));
        let b = acp_mcp_tool(&json!({ "title": "Tool: deloist_browser/browser_mouse" }));
        assert_eq!(b, Some(("deloist_browser".into(), "browser_mouse".into())));
        let c = acp_mcp_tool(&json!({ "rawInput": { "server": "evil", "tool": "browser_nav", "arguments": {} } }));
        assert_eq!(c, Some(("evil".into(), "browser_nav".into())));
        assert_eq!(acp_mcp_tool(&json!({ "title": "Run shell command" })), None);
        // Название без разделителя сервера — не наш инструмент.
        assert_eq!(acp_mcp_tool(&json!({ "title": "mcp.browser_nav" })), None);
    }

    #[test]
    fn a_wrapped_browser_call_is_named_on_the_card() {
        let bridge = crate::drivers::acp::bridge_name();
        let wrapped = json!({ "kind": "other", "title": "execute", "rawInput": { "code": format!("await {bridge}_browser_nav({{url:'https://x.com'}})") } });
        assert_eq!(acp_what(&wrapped).as_deref(), Some("execute → browser_nav"));
        assert_eq!(acp_what(&json!({ "title": "ToolSearch" })).as_deref(), Some("ToolSearch"));
        assert_eq!(acp_what(&json!({ "rawInput": {} })), None);
    }

    #[test]
    fn our_bridge_is_recognised_in_any_agents_naming_and_a_lookalike_is_not() {
        let bridge = crate::drivers::acp::bridge_name();
        // Codex, Claude, Antigravity и выдуманный новый формат.
        for title in [format!("mcp.{bridge}.browser_nav"), format!("mcp__{bridge}__browser_nav"), format!("Run mcp_{bridge}_browser_nav"), format!("Call {bridge} -> browser_nav(url)")] {
            assert_eq!(acp_mcp_tool(&json!({ "title": title })), Some((bridge.to_string(), "browser_nav".into())), "{title}");
        }
        // Сервер с похожим, но не нашим именем, и пустой инструмент — не мост.
        assert_eq!(acp_mcp_tool(&json!({ "title": "Run mcp_deloist_browser_0000000000000000_browser_nav" })), None);
        assert_eq!(acp_mcp_tool(&json!({ "title": format!("Run mcp_{bridge}_") })), None);
    }
}
