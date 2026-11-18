//! Ответ из уведомления Windows. Бот спросил, а окно программы не на переднем плане: тост показывает вопрос
//! с кнопками, и человек отвечает, не открывая чат.
//!
//! - вопрос `ask_user` с вариантами: кнопка на вариант (Windows даёт не больше 5 кнопок: при большем числе
//!   первые четыре и «Открыть»);
//! - вопрос без вариантов и формы: поле ввода и «Ответить»;
//! - форма: только «Открыть» (формы заполняют в программе);
//! - карточка разрешения: «Разрешить один раз» и «Отказать»; опасную, секретную и про отправку, удаление,
//!   оплату, установку, выпуск, или когда действие за человека, — только «Открыть» и «Отказать».
//!   «Всегда» и «в этом разговоре» из тоста не предлагаются.
//!
//! Защита: нового адреса HTTP нет. В аргументах кнопки только вид карточки, её номер и номер варианта;
//! текст варианта и что разрешено решает сервер по сохранённой карточке (`decide`), а не аргументы.
//! Чужой, устаревший или подделанный номер игнорируется. Ответы идут тем же путём, что в окне
//! (`turn::decide_approval`, `turn::answer_question`).
//!
//! Тосты с кнопками только при своём идентификаторе приложения (`aumid::in_use`): иначе нажатие ушло бы к PowerShell.
//! В отладочной сборке `CREW_TEST_TOAST_SINK=<файл>` пишет по строке JSON на показ и на снятие вместо Windows
//! и считает окно не на переднем плане.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock};

use serde_json::{json, Value};

use crate::turn::{Reply, Turns};
use crate::App;

/// Группа тостов программы: вместе с номером карточки (тегом) нужна, чтобы снять тост.
pub const GROUP: &str = "deloist";
/// Больше пяти кнопок Windows не показывает.
const MAX_ACTIONS: usize = 5;
const TITLE_CHARS: usize = 60;
const LINE_CHARS: usize = 160;
const BUTTON_CHARS: usize = 28;
const MAX_REPLY_CHARS: usize = 4_000;
/// Подпись поля ввода и его `id` в разметке.
const INPUT_ID: &str = "reply";
/// Сколько карточек помним, пока не придёт закрытие.
const MAX_HELD: usize = 100;

// ---------------------------------------------------------------------------
// Подписи
// ---------------------------------------------------------------------------

struct Labels {
    open: &'static str,
    reply: &'static str,
    placeholder: &'static str,
    allow_once: &'static str,
    deny: &'static str,
    waiting: &'static str,
}

impl Labels {
    fn new(english: bool) -> Self {
        if english {
            Labels { open: "Open", reply: "Reply", placeholder: "Your answer", allow_once: "Allow once", deny: "Deny", waiting: "is waiting for your decision" }
        } else {
            Labels { open: "Открыть", reply: "Ответить", placeholder: "Ваш ответ", allow_once: "Разрешить один раз", deny: "Отказать", waiting: "ждёт твоего решения" }
        }
    }
}

// ---------------------------------------------------------------------------
// Что показать
// ---------------------------------------------------------------------------

#[derive(Clone, Debug, PartialEq)]
pub struct Button {
    pub label: String,
    /// Аргументы нажатия: `<вид>|<номер карточки>|<действие>`.
    pub args: String,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Plan {
    /// `question` или `approval`.
    pub kind: &'static str,
    pub id: String,
    pub bot_id: String,
    /// Строки текста тоста: бот, потом вопрос или действие.
    pub lines: Vec<String>,
    /// Подпись поля ввода, если оно есть.
    pub input: Option<String>,
    pub buttons: Vec<Button>,
    /// Нажатие на сам тост.
    pub open_args: String,
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum Act {
    Open,
    Once,
    Deny,
    Reply,
    Choice(usize),
}

fn kind_code(kind: &str) -> &'static str {
    if kind == "question" { "q" } else { "a" }
}

fn args_for(kind: &str, id: &str, act: &str) -> String {
    format!("{}|{id}|{act}", kind_code(kind))
}

/// Разбор аргументов нажатия. Всё лишнее или не по форме отбрасывается.
fn parse_args(args: &str) -> Option<(&'static str, &str, Act)> {
    let mut parts = args.split('|');
    let kind = match parts.next()? {
        "q" => "question",
        "a" => "approval",
        _ => return None,
    };
    let id = parts.next()?;
    let act = parts.next()?;
    if parts.next().is_some() || id.is_empty() || id.len() > 64 || !id.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-') {
        return None;
    }
    let act = match act {
        "open" => Act::Open,
        "once" => Act::Once,
        "deny" => Act::Deny,
        "reply" => Act::Reply,
        other => {
            let digits = other.strip_prefix('c')?;
            if digits.is_empty() || digits.len() > 3 || !digits.chars().all(|c| c.is_ascii_digit()) {
                return None;
            }
            Act::Choice(digits.parse().ok()?)
        }
    };
    Some((kind, id, act))
}

/// Текст для тоста: без управляющих символов и переводов строк, не длиннее предела.
fn clean(text: &str, limit: usize) -> String {
    let flat: String = text.split_whitespace().collect::<Vec<_>>().join(" ");
    let flat: String = flat.chars().filter(|c| !c.is_control()).collect();
    if flat.chars().count() <= limit {
        flat
    } else {
        let mut cut: String = flat.chars().take(limit.saturating_sub(1)).collect();
        cut.push('…');
        cut
    }
}

/// Можно ли разрешить карточку одним нажатием в тосте. Опасное, секретное, про отправку, оплату,
/// удаление, установку, выпуск и передачу человеку — только в окне, где видно все подробности.
fn quick_allow(payload: &Value) -> bool {
    if payload["danger"].as_bool().unwrap_or(false) || payload["sensitive"].as_bool().unwrap_or(false) {
        return false;
    }
    if matches!(payload["tool"].as_str(), Some("browser_human")) {
        return false;
    }
    let human = &payload["human"];
    if human.is_null() {
        return true;
    }
    if human["risk"].as_str() != Some("low") && human["risk"].as_str() != Some("medium") {
        return false;
    }
    !matches!(human["class"].as_str(), Some("send" | "delete" | "spend" | "release" | "install" | "handoff" | "unknown"))
}

/// Тост по событию шины: `question` или `approval`. Остальное тостов не даёт.
fn plan(event: &Value, bot_name: &str, labels: &Labels) -> Option<Plan> {
    let id = event["id"].as_str()?.to_string();
    let bot_id = event["botId"].as_str()?.to_string();
    let who = clean(bot_name, TITLE_CHARS);
    let button = |label: &str, act: &str, kind: &str| Button { label: clean(label, BUTTON_CHARS), args: args_for(kind, &id, act) };
    match event["type"].as_str()? {
        "question" => {
            let question = clean(event["question"].as_str()?, LINE_CHARS);
            let choices: Vec<&str> = event["choices"].as_array().map(|c| c.iter().filter_map(Value::as_str).collect()).unwrap_or_default();
            let has_form = event["form"].as_array().is_some_and(|f| !f.is_empty());
            let (mut buttons, mut input) = (Vec::new(), None);
            if has_form {
                buttons.push(button(labels.open, "open", "question"));
            } else if !choices.is_empty() {
                let shown = if choices.len() <= MAX_ACTIONS { choices.len() } else { MAX_ACTIONS - 1 };
                for (i, choice) in choices.iter().take(shown).enumerate() {
                    buttons.push(button(choice, &format!("c{i}"), "question"));
                }
                if choices.len() > MAX_ACTIONS {
                    buttons.push(button(labels.open, "open", "question"));
                }
            } else {
                input = Some(labels.placeholder.to_string());
                buttons.push(button(labels.reply, "reply", "question"));
                buttons.push(button(labels.open, "open", "question"));
            }
            Some(Plan { kind: "question", open_args: args_for("question", &id, "open"), id, bot_id, lines: vec![who, question], input, buttons })
        }
        "approval" => {
            let mut lines = vec![who];
            let title = clean(event["title"].as_str().unwrap_or_default(), LINE_CHARS);
            if !title.is_empty() {
                lines.push(title);
            }
            let detail = event["human"]["text"].as_str().filter(|t| !t.is_empty()).or_else(|| event["detail"].as_str()).unwrap_or_default();
            let detail = clean(detail, LINE_CHARS);
            if !detail.is_empty() && lines.len() < 3 {
                lines.push(detail);
            }
            let mut buttons = Vec::new();
            buttons.push(if quick_allow(event) { button(labels.allow_once, "once", "approval") } else { button(labels.open, "open", "approval") });
            buttons.push(button(labels.deny, "deny", "approval"));
            Some(Plan { kind: "approval", open_args: args_for("approval", &id, "open"), id, bot_id, lines, input: None, buttons })
        }
        // Напоминание о карточке, которая давно ждёт решения: подробности и «разрешить» — в окне.
        "approval_reminder" => {
            let title = clean(event["title"].as_str().unwrap_or_default(), LINE_CHARS);
            let lines = vec![format!("{who} {}: {title}", labels.waiting)];
            let buttons = vec![button(labels.open, "open", "approval"), button(labels.deny, "deny", "approval")];
            Some(Plan { kind: "approval", open_args: args_for("approval", &id, "open"), id, bot_id, lines, input: None, buttons })
        }
        _ => None,
    }
}

pub fn xml_attr_text(text: &str) -> String {
    crate::reminders::xml_escape(text)
}

/// Разметка тоста для Windows.
fn toast_xml(plan: &Plan) -> String {
    let mut xml = format!("<toast launch=\"{}\" activationType=\"foreground\"><visual><binding template=\"ToastGeneric\">", xml_attr_text(&plan.open_args));
    for line in &plan.lines {
        xml.push_str(&format!("<text>{}</text>", xml_attr_text(line)));
    }
    xml.push_str("</binding></visual><actions>");
    if let Some(placeholder) = &plan.input {
        xml.push_str(&format!("<input id=\"{INPUT_ID}\" type=\"text\" placeHolderContent=\"{}\"/>", xml_attr_text(placeholder)));
    }
    for button in &plan.buttons {
        let hint = if plan.input.is_some() && button.args.ends_with("|reply") { format!(" hint-inputId=\"{INPUT_ID}\"") } else { String::new() };
        xml.push_str(&format!(
            "<action content=\"{}\" arguments=\"{}\" activationType=\"foreground\"{hint}/>",
            xml_attr_text(&button.label),
            xml_attr_text(&button.args)
        ));
    }
    xml.push_str("</actions></toast>");
    xml
}

// ---------------------------------------------------------------------------
// Нажатие
// ---------------------------------------------------------------------------

/// Что делать по нажатию. Решается только по сохранённой карточке.
#[derive(Debug, PartialEq)]
pub enum Decision {
    Open { bot_id: String },
    Resolve { id: String, reply: Reply },
    Answer { id: String, body: Value },
    Ignore(&'static str),
}

/// Нажатие в тосте: `args` — аргументы кнопки, `user_input` — текст из поля ввода.
pub fn decide(turns: &Turns, args: &str, user_input: Option<&str>) -> Decision {
    let Some((kind, id, act)) = parse_args(args) else { return Decision::Ignore("аргументы не по форме") };
    if kind == "question" {
        let Some(asked) = turns.question(id) else { return Decision::Ignore("вопрос уже неактивен") };
        let id = id.to_string();
        return match act {
            Act::Open => Decision::Open { bot_id: asked.bot_id },
            Act::Choice(i) if asked.form.is_empty() => match asked.choices.get(i) {
                Some(choice) => Decision::Answer { id, body: json!({ "answer": choice }) },
                None => Decision::Ignore("такого варианта нет"),
            },
            Act::Reply if asked.form.is_empty() && asked.choices.is_empty() => {
                let text: String = user_input.unwrap_or_default().trim().chars().take(MAX_REPLY_CHARS).collect();
                if text.is_empty() {
                    Decision::Ignore("пустой ответ")
                } else {
                    Decision::Answer { id, body: json!({ "answer": text }) }
                }
            }
            _ => Decision::Ignore("действие не подходит к вопросу"),
        };
    }
    let Some((bot_id, payload)) = turns.approval_info(id) else { return Decision::Ignore("подтверждение уже неактивно") };
    let id = id.to_string();
    match act {
        Act::Open => Decision::Open { bot_id },
        Act::Deny => Decision::Resolve { id, reply: Reply::Deny },
        Act::Once if quick_allow(&payload) => Decision::Resolve { id, reply: Reply::Once },
        _ => Decision::Ignore("действие не подходит к карточке"),
    }
}

/// Выполнить нажатие. Одна функция и для обработчика Windows, и для тестов.
pub async fn activate(app: &Arc<App>, args: &str, user_input: Option<&str>) -> Decision {
    let decision = decide(&app.turns, args, user_input);
    match &decision {
        Decision::Open { bot_id } => {
            // То же, что клик по питомцу: окно из трея, и чат этого бота.
            crate::desktop::activate_running_window();
            app.pets.request_open(bot_id);
            app.turns.emit(json!({ "type": "open_bot", "botId": bot_id }));
        }
        Decision::Resolve { id, reply } => {
            crate::turn::decide_approval(app, id, *reply).await;
        }
        Decision::Answer { id, body } => {
            if let Err(e) = crate::turn::answer_question(app, id, body).await {
                eprintln!("[toast] ответ из уведомления не принят: {e}");
            }
        }
        Decision::Ignore(_) => {}
    }
    decision
}

// ---------------------------------------------------------------------------
// Показ
// ---------------------------------------------------------------------------

#[cfg(windows)]
type Native = windows::UI::Notifications::ToastNotification;
#[cfg(not(windows))]
type Native = ();

/// Показанные тосты: объекты живут, пока нужны их обработчики нажатий.
fn held() -> &'static Mutex<HashMap<String, Option<Native>>> {
    static HELD: OnceLock<Mutex<HashMap<String, Option<Native>>>> = OnceLock::new();
    HELD.get_or_init(|| Mutex::new(HashMap::new()))
}

/// Файл для проверок: только отладочная сборка.
fn sink_path() -> Option<String> {
    if cfg!(debug_assertions) {
        std::env::var("CREW_TEST_TOAST_SINK").ok().filter(|p| !p.is_empty())
    } else {
        None
    }
}

fn write_sink(path: &str, line: Value) {
    use std::io::Write;
    if let Ok(mut file) = std::fs::OpenOptions::new().create(true).append(true).open(path) {
        let _ = writeln!(file, "{line}");
    }
}

/// Окно программы сейчас на переднем плане.
#[cfg(windows)]
fn main_window_foreground() -> bool {
    use windows_sys::Win32::UI::WindowsAndMessaging::{FindWindowW, GetForegroundWindow};
    let class: Vec<u16> = crate::desktop::WINDOW_CLASS.encode_utf16().chain(Some(0)).collect();
    unsafe {
        let window = FindWindowW(class.as_ptr(), std::ptr::null());
        !window.is_null() && GetForegroundWindow() == window
    }
}

#[cfg(not(windows))]
fn main_window_foreground() -> bool {
    false
}

/// Запуск: слушаем шину событий, пока работает программа.
pub fn start(app: Arc<App>, runtime: tokio::runtime::Handle) {
    let mut events = app.turns.subscribe();
    let handle = runtime.clone();
    // Карточка без решения дольше срока: одно напоминание.
    let sweep = app.clone();
    runtime.spawn(async move {
        loop {
            tokio::time::sleep(crate::turn::remind_tick()).await;
            for event in sweep.turns.due_reminders() {
                sweep.turns.emit(event);
            }
        }
    });
    runtime.spawn(async move {
        loop {
            let event = match events.recv().await {
                Ok(event) => event,
                Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
                Err(_) => break,
            };
            match event["type"].as_str() {
                Some("question" | "approval") => on_card(&app, &handle, &event).await,
                Some("approval_reminder") => {
                    // Прежний тост по этой карточке мог остаться: напоминание показывается заново.
                    if let Some(id) = event["id"].as_str() {
                        remove(id);
                    }
                    on_card(&app, &handle, &event).await;
                }
                Some("question_closed" | "approval_closed") => {
                    if let Some(id) = event["id"].as_str() {
                        remove(id);
                    }
                }
                _ => {}
            }
        }
    });
}

async fn on_card(app: &Arc<App>, handle: &tokio::runtime::Handle, event: &Value) {
    let sink = sink_path();
    if sink.is_none() && main_window_foreground() {
        return;
    }
    let Some(id) = event["id"].as_str() else { return };
    if held().lock().unwrap().contains_key(id) {
        return;
    }
    let bot_name = event["botId"].as_str().and_then(|b| app.store.bot(b)).map(|b| b.name).unwrap_or_default();
    let labels = Labels::new(app.store.settings().ui_lang == "en");
    let Some(plan) = plan(event, &bot_name, &labels) else { return };
    if let Some(path) = sink {
        held().lock().unwrap().insert(plan.id.clone(), None);
        let labels: Vec<&str> = plan.buttons.iter().map(|b| b.label.as_str()).collect();
        write_sink(
            &path,
            json!({
                "event": "show", "kind": plan.kind, "id": plan.id, "botId": plan.bot_id, "title": plan.lines[0],
                "body": plan.lines[1..].join("\n"), "buttons": labels, "input": plan.input.is_some(), "args": plan.buttons.iter().map(|b| b.args.clone()).collect::<Vec<_>>(),
            }),
        );
        return;
    }
    let (app, handle) = (app.clone(), handle.clone());
    let _ = tokio::task::spawn_blocking(move || present(&app, &handle, &plan)).await;
}

/// Снять тост: карточка закрыта (ответили в окне, в тосте, время вышло).
fn remove(id: &str) {
    let Some(native) = held().lock().unwrap().remove(id) else { return };
    if let Some(path) = sink_path() {
        write_sink(&path, json!({ "event": "remove", "id": id }));
        return;
    }
    #[cfg(windows)]
    if let Some(toast) = native {
        hide(id, &toast);
    }
    #[cfg(not(windows))]
    let _ = native;
}

#[cfg(windows)]
fn hide(id: &str, toast: &Native) {
    use windows::core::HSTRING;
    use windows::UI::Notifications::ToastNotificationManager;
    let app_id = HSTRING::from(crate::aumid::AUMID);
    if let Ok(notifier) = ToastNotificationManager::CreateToastNotifierWithId(&app_id) {
        let _ = notifier.Hide(toast);
    }
    if let Ok(history) = ToastNotificationManager::History() {
        let _ = history.RemoveGroupedTagWithId(&HSTRING::from(id), &HSTRING::from(GROUP), &app_id);
    }
}

#[cfg(not(windows))]
fn present(_app: &Arc<App>, _handle: &tokio::runtime::Handle, _plan: &Plan) {}

#[cfg(windows)]
fn present(app: &Arc<App>, handle: &tokio::runtime::Handle, plan: &Plan) {
    // Без своего идентификатора нажатие ушло бы PowerShell, а не программе.
    if !crate::aumid::in_use() {
        return;
    }
    if let Err(e) = show_interactive(app, handle, plan) {
        eprintln!("[toast] не показал: {e}");
    }
}

#[cfg(windows)]
fn show_interactive(app: &Arc<App>, handle: &tokio::runtime::Handle, plan: &Plan) -> Result<(), String> {
    use windows::core::{Interface, HSTRING};
    use windows::Data::Xml::Dom::XmlDocument;
    use windows::Foundation::{IReference, TypedEventHandler};
    use windows::UI::Notifications::{ToastActivatedEventArgs, ToastNotification, ToastNotificationManager};
    use windows::Win32::System::WinRT::{RoInitialize, RO_INIT_MULTITHREADED};

    fn err(e: windows::core::Error) -> String {
        e.to_string()
    }
    let _ = unsafe { RoInitialize(RO_INIT_MULTITHREADED) };
    let xml = XmlDocument::new().map_err(err)?;
    xml.LoadXml(&HSTRING::from(toast_xml(plan))).map_err(err)?;
    let toast = ToastNotification::CreateToastNotification(&xml).map_err(err)?;
    toast.SetTag(&HSTRING::from(plan.id.as_str())).map_err(err)?;
    toast.SetGroup(&HSTRING::from(GROUP)).map_err(err)?;
    let (app, handle) = (app.clone(), handle.clone());
    toast
        .Activated(&TypedEventHandler::new(move |_sender, args: windows::core::Ref<'_, windows::core::IInspectable>| {
            let Some(args) = args.as_ref().and_then(|a| a.cast::<ToastActivatedEventArgs>().ok()) else { return Ok(()) };
            let arguments = args.Arguments().map(|a| a.to_string()).unwrap_or_default();
            let input = args
                .UserInput()
                .ok()
                .and_then(|values| values.Lookup(&HSTRING::from(INPUT_ID)).ok())
                .and_then(|value| value.cast::<IReference<HSTRING>>().ok())
                .and_then(|value| value.Value().ok())
                .map(|text| text.to_string());
            let app = app.clone();
            handle.spawn(async move {
                activate(&app, &arguments, input.as_deref()).await;
            });
            Ok(())
        }))
        .map_err(err)?;
    let notifier = ToastNotificationManager::CreateToastNotifierWithId(&HSTRING::from(crate::aumid::AUMID)).map_err(err)?;
    {
        let mut map = held().lock().unwrap();
        if map.len() >= MAX_HELD {
            map.clear();
        }
        map.insert(plan.id.clone(), Some(toast.clone()));
    }
    notifier.Show(&toast).map_err(err)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ru() -> Labels {
        Labels::new(false)
    }

    fn question(choices: &[&str], form: bool) -> Value {
        json!({
            "type": "question", "kind": "question", "id": "ap_q1", "botId": "b1", "conv": "bot:b1", "question": "Какой склад?",
            "choices": choices, "form": if form { json!([{ "name": "n", "label": "Имя", "kind": "text", "options": [] }]) } else { json!([]) },
        })
    }

    fn approval(extra: Value) -> Value {
        let mut event = json!({ "type": "approval", "id": "ap_a1", "botId": "b1", "tool": "write_file", "title": "Записать файл", "detail": "a.txt", "danger": false, "sensitive": false });
        if let (Some(e), Some(x)) = (event.as_object_mut(), extra.as_object()) {
            e.extend(x.clone());
        }
        event
    }

    fn labels(plan: &Plan) -> Vec<&str> {
        plan.buttons.iter().map(|b| b.label.as_str()).collect()
    }

    fn turns() -> Turns {
        let stamp = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos();
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("target").join("tmp").join(format!("toast-{stamp}"));
        std::fs::create_dir_all(&dir).unwrap();
        Turns::open(&dir)
    }

    fn store(turns: &Turns, choices: &[&str], form: bool) {
        let mut asked = crate::turn::Asked { id: "ap_q1".into(), bot_id: "b1".into(), conv: "bot:b1".into(), question: "Какой склад?".into(), at: 1, ..Default::default() };
        asked.choices = choices.iter().map(|c| c.to_string()).collect();
        if form {
            asked.form = vec![crate::turn::FormField { name: "n".into(), label: "Имя".into(), kind: "text".into(), ..Default::default() }];
        }
        turns.store_question_for_test(asked);
    }

    #[test]
    fn choices_become_buttons_up_to_five() {
        let plan = plan(&question(&["Москва", "Казань", "Тверь"], false), "Лена", &ru()).unwrap();
        assert_eq!(labels(&plan), ["Москва", "Казань", "Тверь"]);
        assert_eq!(plan.buttons[1].args, "q|ap_q1|c1");
        assert!(plan.input.is_none());
        let five = plan_of(&["a", "b", "c", "d", "e"]);
        assert_eq!(labels(&five), ["a", "b", "c", "d", "e"], "ровно пять — все пять");
    }

    fn plan_of(choices: &[&str]) -> Plan {
        plan(&question(choices, false), "Лена", &ru()).unwrap()
    }

    #[test]
    fn more_than_five_choices_show_four_and_open() {
        let plan = plan_of(&["a", "b", "c", "d", "e", "f", "g"]);
        assert_eq!(labels(&plan), ["a", "b", "c", "d", "Открыть"]);
        assert_eq!(plan.buttons[4].args, "q|ap_q1|open");
    }

    #[test]
    fn a_free_text_question_gets_an_input_reply_and_open() {
        let plan = plan_of(&[]);
        assert_eq!(plan.input.as_deref(), Some("Ваш ответ"));
        assert_eq!(labels(&plan), ["Ответить", "Открыть"]);
        let xml = toast_xml(&plan);
        assert!(xml.contains("<input id=\"reply\" type=\"text\""), "{xml}");
        assert!(xml.contains("hint-inputId=\"reply\""), "{xml}");
        assert!(xml.contains("launch=\"q|ap_q1|open\" activationType=\"foreground\""), "{xml}");
    }

    #[test]
    fn a_form_question_only_opens() {
        let plan = plan(&question(&[], true), "Лена", &ru()).unwrap();
        assert_eq!(labels(&plan), ["Открыть"]);
        assert!(plan.input.is_none());
    }

    #[test]
    fn a_normal_approval_can_be_allowed_once_or_denied() {
        let plan = plan(&approval(json!({})), "Лена", &ru()).unwrap();
        assert_eq!(labels(&plan), ["Разрешить один раз", "Отказать"]);
        assert_eq!(plan.buttons[0].args, "a|ap_a1|once");
        let low = approval(json!({ "human": { "risk": "low", "class": "read", "text": "Прочитать файл" } }));
        assert_eq!(labels(&super::plan(&low, "Лена", &ru()).unwrap()), ["Разрешить один раз", "Отказать"]);
    }

    #[test]
    fn dangerous_sensitive_and_send_cards_never_offer_allow() {
        let cases = [
            json!({ "danger": true }),
            json!({ "sensitive": true }),
            json!({ "tool": "browser_human" }),
            json!({ "human": { "risk": "high", "class": "run" } }),
            json!({ "human": { "risk": "medium", "class": "send" } }),
            json!({ "human": { "risk": "medium", "class": "delete" } }),
            json!({ "human": { "risk": "medium", "class": "spend" } }),
            json!({ "human": { "risk": "medium", "class": "handoff" } }),
        ];
        for extra in cases {
            let plan = plan(&approval(extra.clone()), "Лена", &ru()).unwrap();
            assert_eq!(labels(&plan), ["Открыть", "Отказать"], "{extra}");
            assert!(!toast_xml(&plan).contains("once"), "{extra}");
        }
    }

    #[test]
    fn text_is_escaped_and_limited() {
        let evil = question(&["\"><action arguments=\"x\"/>", "it's <b>"], false);
        let mut evil = evil;
        evil["question"] = json!(format!("<script>&'\"{}", "я".repeat(500)));
        let plan = plan(&evil, "<Бот>", &ru()).unwrap();
        let xml = toast_xml(&plan);
        assert!(!xml.contains("<script>") && !xml.contains("<b>") && !xml.contains("<Бот>"), "{xml}");
        assert!(xml.contains("&lt;script&gt;&amp;&apos;&quot;"), "{xml}");
        assert_eq!(xml.matches("<action ").count(), 2, "чужая разметка не добавила кнопок: {xml}");
        assert!(plan.lines[1].chars().count() <= LINE_CHARS && plan.lines[1].ends_with('…'));
    }

    #[test]
    fn english_labels_follow_the_language_setting() {
        let plan = plan(&approval(json!({})), "Lena", &Labels::new(true)).unwrap();
        assert_eq!(labels(&plan), ["Allow once", "Deny"]);
    }

    #[test]
    fn forged_arguments_are_ignored() {
        let turns = turns();
        store(&turns, &["Москва"], false);
        for bad in ["", "x", "q|ap_q1", "q|ap_q1|c0|extra", "z|ap_q1|c0", "q|ap_q1|c-1", "q|ap_q1|c", "q|ap_q1|c9999", "q|../x|c0", "q|ap q1|c0", "q||c0", "q|ap_q1|once", "q|ap_q1|C0"] {
            assert!(matches!(decide(&turns, bad, None), Decision::Ignore(_)), "{bad}");
        }
        assert!(matches!(decide(&turns, "q|ap_nope|c0", None), Decision::Ignore(_)), "неизвестный номер");
        assert!(matches!(decide(&turns, "a|ap_q1|deny", None), Decision::Ignore(_)), "вопрос — не карточка");
    }

    #[test]
    fn a_choice_index_is_resolved_from_the_stored_question() {
        let turns = turns();
        store(&turns, &["Москва", "Казань"], false);
        assert_eq!(decide(&turns, "q|ap_q1|c1", None), Decision::Answer { id: "ap_q1".into(), body: json!({ "answer": "Казань" }) });
        assert!(matches!(decide(&turns, "q|ap_q1|c2", None), Decision::Ignore(_)), "индекс за пределами");
        assert!(matches!(decide(&turns, "q|ap_q1|reply", Some("своё")), Decision::Ignore(_)), "у вопроса с вариантами нет поля ввода");
        assert_eq!(decide(&turns, "q|ap_q1|open", None), Decision::Open { bot_id: "b1".into() });
    }

    #[test]
    fn a_text_reply_needs_text_and_no_choices() {
        let turns = turns();
        store(&turns, &[], false);
        assert_eq!(decide(&turns, "q|ap_q1|reply", Some("  склад 2 ")), Decision::Answer { id: "ap_q1".into(), body: json!({ "answer": "склад 2" }) });
        assert!(matches!(decide(&turns, "q|ap_q1|reply", Some("   ")), Decision::Ignore(_)));
        assert!(matches!(decide(&turns, "q|ap_q1|reply", None), Decision::Ignore(_)));
        assert!(matches!(decide(&turns, "q|ap_q1|c0", None), Decision::Ignore(_)));
    }

    #[test]
    fn a_form_question_cannot_be_answered_from_a_toast() {
        let turns = turns();
        store(&turns, &[], true);
        assert!(matches!(decide(&turns, "q|ap_q1|reply", Some("x")), Decision::Ignore(_)));
        assert_eq!(decide(&turns, "q|ap_q1|open", None), Decision::Open { bot_id: "b1".into() });
    }

    #[test]
    fn approval_buttons_are_checked_against_the_stored_card() {
        let turns = turns();
        let _a = turns.open_approval_for_test("ap_a1", "b1", approval(json!({})));
        let _d = turns.open_approval_for_test("ap_d1", "b1", approval(json!({ "danger": true })));
        assert_eq!(decide(&turns, "a|ap_a1|once", None), Decision::Resolve { id: "ap_a1".into(), reply: Reply::Once });
        assert_eq!(decide(&turns, "a|ap_a1|deny", None), Decision::Resolve { id: "ap_a1".into(), reply: Reply::Deny });
        assert!(matches!(decide(&turns, "a|ap_d1|once", None), Decision::Ignore(_)), "подделанное «разрешить» на опасной карточке");
        assert_eq!(decide(&turns, "a|ap_d1|deny", None), Decision::Resolve { id: "ap_d1".into(), reply: Reply::Deny });
        assert_eq!(decide(&turns, "a|ap_d1|open", None), Decision::Open { bot_id: "b1".into() });
        assert!(matches!(decide(&turns, "a|ap_a1|always", None), Decision::Ignore(_)), "«всегда» из тоста нет");
        assert!(matches!(decide(&turns, "a|ap_a1|c0", None), Decision::Ignore(_)));
        assert!(matches!(decide(&turns, "a|ap_gone|once", None), Decision::Ignore(_)), "устаревшая карточка");
    }
}
