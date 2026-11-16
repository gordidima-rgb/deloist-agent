//! «Научи бота»: человек делает задачу в экране бота, программа записывает шаги, служебная модель
//! превращает запись в навык. Пароли и данные карт не записываются вовсе, разовые значения
//! (даты, номера заказов) заменяются заготовками ещё до модели.

use std::sync::Arc;

use regex::Regex;
use serde::Serialize;
use serde_json::{json, Value};

use crate::browser::Session;
use crate::store::{now, Bot};
use crate::App;

/// Описание элемента так же, как его видит снимок `browser_tools`: роль, подпись, текст рядом
/// и устойчивый селектор. Поля type, auto и name — для проверки пароля и карты.
const RECORD_FN: &str = r#"function(){
let e=this;
const inter=n=>n&&n.matches&&n.matches('a,button,input,select,textarea,summary,label,[role],[onclick],[contenteditable=""],[contenteditable=true]');
let c=e;while(c&&c!==document.body&&!inter(c))c=c.parentElement;if(c&&c!==document.body)e=c;
const tag=(e.tagName||'').toLowerCase(),type=(e.type||'').toLowerCase();
const t=s=>(s||'').replace(/\s+/g,' ').trim().slice(0,120);
let role=e.getAttribute('role')||({a:'link',button:'button',select:'combobox',textarea:'textbox',summary:'button'})[tag]||'';
if(tag==='input')role=({checkbox:'checkbox',radio:'radio',submit:'button',button:'button',search:'searchbox',range:'slider',number:'spinbutton'})[type]||'textbox';
const lab=e.labels&&e.labels[0]?e.labels[0].innerText:'';
const field=tag==='input'||tag==='textarea'||tag==='select';
const label=t(e.getAttribute('aria-label')||lab||(field?e.placeholder:e.innerText)||e.title||e.alt||(type==='submit'||type==='button'?e.value:'')||'');
let near='';for(let p=e.parentElement,i=0;p&&i<3&&!near;p=p.parentElement,i++){const x=t(p.innerText);if(x&&x!==label)near=x}
let sel='';const attr=['data-testid','data-test','data-qa'].find(a=>e.hasAttribute(a));
if(attr)sel='['+attr+'="'+e.getAttribute(attr)+'"]';
else if(e.id&&!/\d{3,}|[0-9a-f]{8,}|^[:_]|^r\d|:/i.test(e.id))sel='#'+CSS.escape(e.id);
else if(e.getAttribute('name'))sel=tag+'[name="'+e.getAttribute('name')+'"]';
else if(e.getAttribute('aria-label'))sel=tag+'[aria-label="'+e.getAttribute('aria-label')+'"]';
else{const parts=[];for(let n=e;n&&n.nodeType===1&&n!==document.body&&parts.length<4;n=n.parentElement){let i=1;for(let s=n.previousElementSibling;s;s=s.previousElementSibling)if(s.tagName===n.tagName)i++;parts.unshift(n.tagName.toLowerCase()+':nth-of-type('+i+')')}sel=parts.join(' > ')}
return{tag,type,auto:(e.autocomplete||'').toLowerCase(),name:[e.getAttribute('name'),e.id,e.placeholder,e.getAttribute('aria-label'),lab,e.title].filter(Boolean).join(' '),role,label,near,selector:sel}}"#;

#[derive(Clone, Debug, Default, PartialEq, Serialize)]
pub struct Target {
    pub role: String,
    pub label: String,
    pub near: String,
    pub selector: String,
    /// «пароль» или «данные карты»: значение такого поля не записывается.
    #[serde(skip_serializing_if = "String::is_empty")]
    pub secret: String,
}

impl Target {
    pub fn from_info(info: &Value) -> Option<Self> {
        if !info.is_object() {
            return None;
        }
        let field = |key: &str| info.get(key).and_then(Value::as_str).unwrap_or_default().to_string();
        Some(Self {
            role: field("role"),
            label: field("label"),
            near: field("near"),
            selector: field("selector"),
            secret: record_secret(info).to_string(),
        })
    }

    fn same(&self, other: &Target) -> bool {
        self.selector == other.selector && self.label == other.label
    }
}

/// Значение какого поля не записываем вовсе. Шире, чем отказ бота печатать (`sensitive_kind`):
/// лишний «секрет» в записи безвреден, а бот должен уметь вводить «код товара» и «промокод».
pub fn record_secret(info: &Value) -> &'static str {
    if let Some(kind) = crate::browser_tools::sensitive_kind(info) {
        return kind;
    }
    let field = |key: &str| info.get(key).and_then(Value::as_str).unwrap_or_default();
    let words = format!("{} {} {}", field("name"), field("label"), field("auto"));
    if Regex::new(r"(?i)cvv|cvc|csc|security|карт").unwrap().is_match(&words) {
        return "данные карты";
    }
    if field("auto") == "one-time-code" || Regex::new(r"(?i)код|code|otp|\bsms\b|смс|\bpin\b|\bпин").unwrap().is_match(&words) {
        return "код";
    }
    // Внутрь чужого фрейма не заглянуть: что там вводят, не знаем — значит, не записываем.
    if field("tag") == "iframe" {
        return "данные во фрейме";
    }
    ""
}

#[derive(Clone, Debug, Default, Serialize)]
pub struct RecStep {
    /// open | click | type | press | secret
    pub kind: String,
    /// Адрес страницы, на которой сделан шаг; для open — куда перешли.
    pub url: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub target: Option<Target>,
    /// Набранный текст или клавиша. У secret всегда пусто.
    pub value: String,
    /// Адрес, который открылся после шага.
    #[serde(skip_serializing_if = "String::is_empty")]
    pub after: String,
    /// Пояснение человека к шагу.
    #[serde(skip_serializing_if = "String::is_empty")]
    pub note: String,
    /// Уменьшенный кадр, JPEG в base64.
    #[serde(skip_serializing_if = "String::is_empty")]
    pub thumb: String,
    pub at: i64,
}

#[derive(Default)]
pub struct Recording {
    pub steps: Vec<RecStep>,
    /// «Стоп» нажат: ввод больше не пишется, но запись живёт до сохранения или «Не сохранять».
    pub stopped: bool,
}

impl Recording {
    fn push(&mut self, kind: &str, url: &str, target: Option<Target>, value: &str, thumb: String) {
        self.steps.push(RecStep { kind: kind.into(), url: url.into(), target, value: value.into(), thumb, at: now(), ..RecStep::default() });
    }

    /// Человек открыл адрес из адресной строки панели.
    pub fn open(&mut self, url: &str, thumb: String) {
        // Параметры адреса бывают одноразовыми ключами входа (?code=, #access_token=): их не храним.
        let bare = url.split(['?', '#']).next().unwrap_or(url);
        self.push("open", bare, None, "", thumb);
    }

    pub fn click(&mut self, url: &str, target: Option<Target>, thumb: String) {
        self.push("click", url, target, "", thumb);
    }

    /// Нужен ли новый кадр: печать в то же поле продолжает прошлый шаг.
    pub fn continues_typing(&self, target: &Option<Target>) -> bool {
        match (self.steps.last(), target) {
            (Some(last), Some(target)) if last.kind == "type" || last.kind == "secret" => last.target.as_ref().is_some_and(|t| t.same(target)),
            _ => false,
        }
    }

    /// Набранные символы. Подряд в одно поле — один шаг. В поле пароля или карты значение не хранится.
    pub fn typed(&mut self, url: &str, target: Option<Target>, chars: &str, thumb: String) {
        if self.continues_typing(&target) {
            let last = self.steps.last_mut().unwrap();
            if last.kind == "type" {
                last.value.push_str(chars);
            }
            return;
        }
        // Поле не распознали — значение не храним: это мог быть пароль.
        let secret = target.as_ref().is_none_or(|t| !t.secret.is_empty());
        if secret {
            self.push("secret", url, target, "", thumb);
        } else {
            self.push("type", url, target, chars, thumb);
        }
    }

    pub fn backspace(&mut self, target: &Option<Target>) {
        if self.continues_typing(target) {
            if let Some(last) = self.steps.last_mut().filter(|s| s.kind == "type") {
                last.value.pop();
            }
        }
    }

    /// Enter, Tab, Escape и сочетания.
    pub fn press(&mut self, url: &str, target: Option<Target>, keys: &str, thumb: String) {
        self.push("press", url, target, keys, thumb);
    }

    pub fn note(&mut self, text: &str) {
        let text = text.trim();
        if text.is_empty() {
            return;
        }
        match self.steps.last_mut() {
            Some(last) => {
                if !last.note.is_empty() {
                    last.note.push(' ');
                }
                last.note.push_str(text);
            }
            None => self.steps.push(RecStep { kind: "note".into(), note: text.into(), at: now(), ..RecStep::default() }),
        }
    }

    /// После шага открылся другой адрес: это и есть «что проверить».
    pub fn page_is(&mut self, url: &str) {
        let Some(last) = self.steps.last_mut() else { return };
        let before = if last.after.is_empty() { &last.url } else { &last.after };
        if !url.is_empty() && url != before && last.kind != "open" {
            last.after = url.to_string();
        }
    }
}

// ---------------------------------------------------------------------------
// Описание элемента
// ---------------------------------------------------------------------------

async fn describe(session: &Session, object: Option<&str>) -> Option<Target> {
    let result = session
        .call("Runtime.callFunctionOn", json!({ "objectId": object?, "functionDeclaration": RECORD_FN, "returnByValue": true }))
        .await
        .ok()?;
    Target::from_info(&result["result"]["value"])
}

/// Элемент под точкой, в том числе внутри теневого DOM.
pub async fn describe_point(session: &Session, x: f64, y: f64) -> Option<Target> {
    let expression = format!("(()=>{{let e=document.elementFromPoint({x},{y});while(e&&e.shadowRoot){{const n=e.shadowRoot.elementFromPoint({x},{y});if(!n||n===e)break;e=n}}return e}})()");
    let found = session.call("Runtime.evaluate", json!({ "expression": expression })).await.ok()?;
    describe(session, found["result"]["objectId"].as_str()).await
}

/// Поле в фокусе, в том числе внутри теневого DOM.
pub async fn describe_focused(session: &Session) -> Option<Target> {
    let expression = "(()=>{let e=document.activeElement;while(e&&e.shadowRoot&&e.shadowRoot.activeElement)e=e.shadowRoot.activeElement;return e})()";
    let found = session.call("Runtime.evaluate", json!({ "expression": expression })).await.ok()?;
    describe(session, found["result"]["objectId"].as_str()).await
}

// ---------------------------------------------------------------------------
// Разовые значения
// ---------------------------------------------------------------------------

/// Даты, длинные номера и почта — разовые: в навыке вместо них заготовки. Адреса сайтов не портятся:
/// в адресе разовыми бывают только путь и параметры, сайт и порт остаются как есть.
pub fn generalize(text: &str) -> String {
    let urls = Regex::new(r#"https?://[^\s«»"'<>]+"#).unwrap();
    let mut out = String::new();
    let mut last = 0;
    for found in urls.find_iter(text) {
        out.push_str(&generalize_plain(&text[last..found.start()]));
        out.push_str(&generalize_url(found.as_str()));
        last = found.end();
    }
    out.push_str(&generalize_plain(&text[last..]));
    out
}

/// Адрес для модели и навыка: сайт и порт как есть, разовое в пути заменено, якорь отброшен,
/// у параметров остаются только имена: `?code=<…>`. В них бывают ключи входа и подписи.
fn generalize_url(url: &str) -> String {
    let url = url.split('#').next().unwrap_or(url);
    let (base, query) = url.split_once('?').unwrap_or((url, ""));
    let start = base.find("://").map_or(0, |i| i + 3);
    let cut = start + base[start..].find('/').unwrap_or(base.len() - start);
    let mut out = format!("{}{}", &base[..cut], generalize_plain(&base[cut..]));
    let keys: Vec<String> = query
        .split('&')
        .filter(|pair| !pair.is_empty())
        .map(|pair| format!("{}=<…>", generalize_plain(pair.split('=').next().unwrap_or_default())))
        .collect();
    if !keys.is_empty() {
        out.push('?');
        out.push_str(&keys.join("&"));
    }
    out
}

/// Сумма Луна: так проверяются номера карт.
fn luhn(digits: &[u32]) -> bool {
    let sum: u32 = digits.iter().rev().enumerate().map(|(i, &d)| if i % 2 == 1 { let x = d * 2; if x > 9 { x - 9 } else { x } } else { d }).sum();
    digits.len() >= 12 && sum % 10 == 0
}

fn generalize_plain(text: &str) -> String {
    // Цифры с пробелами и дефисами (номер карты, телефон) — сначала: иначе их порежут правила ниже.
    let runs = Regex::new(r"\d[\d \-]{11,}\d").unwrap();
    let text = runs
        .replace_all(text, |found: &regex::Captures| {
            let digits: Vec<u32> = found[0].chars().filter_map(|c| c.to_digit(10)).collect();
            if luhn(&digits) { "<номер карты>".to_string() } else if digits.len() >= 10 { "<номер>".to_string() } else { found[0].to_string() }
        })
        .into_owned();
    let rules = [
        (r"\b\d{4}-\d{2}-\d{2}(?:[T ]\d{2}:\d{2}(?::\d{2})?)?\b", "<дата>"),
        (r"\b\d{1,2}[./]\d{1,2}[./]\d{2,4}\b", "<дата>"),
        (r"[\w.+-]+@[\w-]+(?:\.[\w-]+)+", "<почта>"),
        (r"\d{5,}", "<номер>"),
    ];
    let mut out = text;
    for (pattern, place) in rules {
        out = Regex::new(pattern).unwrap().replace_all(&out, place).into_owned();
    }
    out
}

/// Запись для модели: без картинок, без значений секретов, с заготовками вместо разовых значений.
pub fn for_model(steps: &[RecStep]) -> Value {
    let list: Vec<Value> = steps
        .iter()
        .enumerate()
        .map(|(i, step)| {
            let mut item = json!({ "n": i + 1, "kind": step.kind, "url": generalize(&step.url) });
            if let Some(target) = &step.target {
                item["target"] = json!({
                    "role": target.role, "label": generalize(&target.label), "near": generalize(&target.near), "selector": generalize(&target.selector),
                });
            }
            match step.kind.as_str() {
                "secret" => {
                    let what = step.target.as_ref().map_or("текст в нераспознанное поле", |t| t.secret.as_str());
                    let what = if what.is_empty() { "текст в нераспознанное поле" } else { what };
                    item["value"] = json!(format!("человек ввёл {what}, значение не записано"));
                }
                "type" | "press" => item["value"] = json!(generalize(&step.value)),
                _ => {}
            }
            if !step.after.is_empty() {
                item["after"] = json!(generalize(&step.after));
            }
            if !step.note.is_empty() {
                item["note"] = json!(generalize(&step.note));
            }
            item
        })
        .collect();
    Value::Array(list)
}

// ---------------------------------------------------------------------------
// Черновик навыка
// ---------------------------------------------------------------------------

#[derive(Clone, Debug, Serialize)]
pub struct Draft {
    pub name: String,
    pub description: String,
    pub body: String,
}

const DRAFT_SYSTEM: &str = "Ты превращаешь запись действий человека в браузере в навык для бота. \
Бот работает инструментами browser_nav (open, back, tabs), browser_page (snapshot, wait), browser_mouse (click по номеру элемента ref из снимка), \
browser_keyboard (type в поле ref, press Enter) и browser_human (передать окно человеку). Элементы ищутся в снимке по роли и подписи, номера ref меняются от снимка к снимку. \
Ответь одним JSON-объектом без пояснений: {\"name\": \"короткое название до 60 знаков\", \"description\": \"когда применять, одной строкой\", \
\"steps\": [{\"do\": \"что сделать, указаниями для инструментов бота\", \"check\": \"что проверить после шага\"}], \"human\": [\"где нужен человек\"]}. \
Правила: пароли, коды и данные карт бот не вводит — такие шаги делает человек через browser_human. \
Не вставляй разовые значения из записи (номера заказов, даты, почту, суммы): пиши, откуда их брать, или оставь заготовку вида <номер заказа>. \
Пиши по-русски, коротко.";

fn str_of<'a>(value: &'a Value, key: &str) -> &'a str {
    value.get(key).and_then(Value::as_str).unwrap_or_default().trim()
}

/// Текст навыка из ответа модели: нумерованные шаги, проверки и где нужен человек.
pub fn draft_from_reply(reply: &Value) -> Option<Draft> {
    let name = str_of(reply, "name");
    let steps = reply.get("steps").and_then(Value::as_array).filter(|s| !s.is_empty())?;
    if name.is_empty() {
        return None;
    }
    let mut body = String::from("Шаги:\n");
    for (i, step) in steps.iter().enumerate() {
        let action = match step {
            Value::String(text) => text.trim(),
            other => str_of(other, "do"),
        };
        if action.is_empty() {
            continue;
        }
        body.push_str(&format!("{}. {action}\n", i + 1));
        let check = str_of(step, "check");
        if !check.is_empty() {
            body.push_str(&format!("   Проверить: {check}\n"));
        }
    }
    let human: Vec<&str> = reply.get("human").and_then(Value::as_array).into_iter().flatten().filter_map(Value::as_str).map(str::trim).filter(|s| !s.is_empty()).collect();
    if !human.is_empty() {
        body.push_str("\nГде нужен человек:\n");
        for line in human {
            body.push_str(&format!("- {line}\n"));
        }
    }
    Some(Draft {
        name: generalize(name).chars().take(60).collect(),
        description: generalize(str_of(reply, "description")).chars().take(160).collect(),
        body: generalize(body.trim_end()),
    })
}

fn target_words(target: &Option<Target>) -> String {
    match target {
        Some(t) if !t.label.is_empty() => format!("{} «{}»", if t.role.is_empty() { "элемент" } else { &t.role }, t.label),
        Some(t) if !t.selector.is_empty() => format!("элемент {}", t.selector),
        _ => "элемент".into(),
    }
}

/// Черновик без модели: шаги записи как есть. Нужен, когда служебная модель недоступна.
pub fn plain_draft(steps: &[RecStep]) -> Draft {
    let mut lines = Vec::new();
    let mut human = Vec::new();
    for step in steps {
        let what = match step.kind.as_str() {
            "open" => format!("browser_nav open {}", step.url),
            "click" => format!("browser_mouse click: {}", target_words(&step.target)),
            "type" => format!("browser_keyboard type в {}: «{}»", target_words(&step.target), step.value),
            "press" => format!("browser_keyboard press {}", step.value),
            "secret" => {
                let what = step.target.as_ref().map_or("текст", |t| t.secret.as_str());
                human.push(format!("{what} в {}", target_words(&step.target)));
                format!("browser_human: человек вводит {what} в {}", target_words(&step.target))
            }
            _ => continue,
        };
        let mut line = format!("{}. {what}", lines.len() + 1);
        if !step.after.is_empty() {
            line.push_str(&format!("\n   Проверить: открылась {}", step.after));
        }
        if !step.note.is_empty() {
            line.push_str(&format!("\n   Пояснение: {}", step.note));
        }
        lines.push(line);
    }
    let mut body = format!("Шаги:\n{}", lines.join("\n"));
    if !human.is_empty() {
        body.push_str(&format!("\n\nГде нужен человек:\n- {}", human.join("\n- ")));
    }
    let host = steps.iter().find_map(|s| reqwest::Url::parse(&s.url).ok().and_then(|u| u.host_str().map(str::to_string))).unwrap_or_default();
    Draft {
        name: if host.is_empty() { "Записанный навык".into() } else { format!("Записанный навык: {host}").chars().take(60).collect() },
        description: "Повторить действия, которые показал человек.".into(),
        body: generalize(&body),
    }
}

/// Служебный вызов модели бота. Модель недоступна или ответила не так — черновик из записи и причина.
pub async fn draft(app: &Arc<App>, bot: &Bot, steps: &[RecStep]) -> (Draft, Option<String>) {
    let task = format!("Запись действий человека, по шагам:\n{}", serde_json::to_string_pretty(&for_model(steps)).unwrap_or_default());
    match crate::service::complete(app, bot, DRAFT_SYSTEM, &task).await {
        Ok(reply) => match crate::service::json_object(&reply).as_ref().and_then(draft_from_reply) {
            Some(draft) => (draft, None),
            None => (plain_draft(steps), Some("модель ответила не по форме: вот черновик прямо из записи".into())),
        },
        Err(e) => (plain_draft(steps), Some(e)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn field(label: &str, kind: &str) -> Option<Target> {
        Target::from_info(&json!({ "role": "textbox", "label": label, "selector": format!("input[aria-label=\"{label}\"]"), "type": kind, "name": label }))
    }

    #[test]
    fn typing_into_one_field_is_one_step_and_backspace_edits_it() {
        let mut rec = Recording::default();
        rec.click("http://s/form", field("Поиск", "text"), String::new());
        for ch in ["к", "о", "т", "ы"] {
            rec.typed("http://s/form", field("Поиск", "text"), ch, String::new());
        }
        rec.backspace(&field("Поиск", "text"));
        rec.typed("http://s/form", field("Тема", "text"), "a", String::new());
        let kinds: Vec<&str> = rec.steps.iter().map(|s| s.kind.as_str()).collect();
        assert_eq!(kinds, ["click", "type", "type"]);
        assert_eq!(rec.steps[1].value, "кот");
        assert_eq!(rec.steps[2].value, "a");
    }

    #[test]
    fn password_and_card_values_are_never_kept() {
        let mut rec = Recording::default();
        for ch in ["h", "u", "n", "t", "e", "r"] {
            rec.typed("http://s/login", field("Пароль", "password"), ch, String::new());
        }
        let card = Target::from_info(&json!({ "role": "textbox", "label": "Карта", "selector": "#card", "auto": "cc-number" }));
        rec.typed("http://s/login", card, "4", String::new());
        assert_eq!(rec.steps.len(), 2);
        assert_eq!(rec.steps[0].kind, "secret");
        assert_eq!(rec.steps[0].value, "");
        assert_eq!(rec.steps[1].target.as_ref().unwrap().secret, "данные карты");
        let model = for_model(&rec.steps).to_string();
        assert!(!model.contains("hunter") && !model.contains("\"h\""), "{model}");
        assert!(model.contains("человек ввёл пароль"), "{model}");
    }

    #[test]
    fn typing_into_an_unknown_field_or_a_frame_is_not_kept() {
        let mut rec = Recording::default();
        rec.typed("http://s/", None, "x", String::new());
        let frame = Target::from_info(&json!({ "tag": "iframe", "label": "", "selector": "iframe" }));
        assert_eq!(frame.as_ref().unwrap().secret, "данные во фрейме");
        rec.typed("http://s/", frame, "y", String::new());
        assert!(rec.steps.iter().all(|s| s.kind == "secret" && s.value.is_empty()));
    }

    #[test]
    fn page_change_is_noted_on_the_step_that_caused_it() {
        let mut rec = Recording::default();
        rec.open("http://s/", String::new());
        rec.page_is("http://s/");
        rec.click("http://s/", field("Экспорт", "submit"), String::new());
        rec.page_is("http://s/");
        assert_eq!(rec.steps[1].after, "");
        rec.page_is("http://s/done");
        assert_eq!(rec.steps[1].after, "http://s/done");
        rec.note("так выгружается отчёт");
        assert_eq!(rec.steps[1].note, "так выгружается отчёт");
    }

    #[test]
    fn one_off_values_are_generalized() {
        assert_eq!(generalize("заказ 87654321 от 12.11.2026"), "заказ <номер> от <дата>");
        assert_eq!(generalize("с 2026-11-01 по 2026-11-30"), "с <дата> по <дата>");
        assert_eq!(generalize("пиши на shop@example.ru"), "пиши на <почта>");
        assert_eq!(generalize("Экспорт в Excel, 2 шт."), "Экспорт в Excel, 2 шт.");
        // Сайт и порт в адресе не разовые, номер заказа в пути — разовый.
        assert_eq!(
            generalize("открой http://127.0.0.1:47811/orders/87654321?d=12.11.2026 и жди"),
            "открой http://127.0.0.1:47811/orders/<номер>?d=<…> и жди"
        );
        assert_eq!(generalize("https://seller.ozon.ru:443/app"), "https://seller.ozon.ru:443/app");
    }

    #[test]
    fn url_query_values_and_fragments_never_leave_the_recording() {
        let text = generalize("вход https://id.example/cb?code=AbC123xyz&state=qwe#access_token=SECRET1 готов");
        assert_eq!(text, "вход https://id.example/cb?code=<…>&state=<…> готов");
        let mut rec = Recording::default();
        rec.open("https://shop.test/orders?session=s3cr3t#sig=zz", String::new());
        assert_eq!(rec.steps[0].url, "https://shop.test/orders");
        rec.click("https://shop.test/orders?token=t0k3n", field("Экспорт", "submit"), String::new());
        rec.page_is("https://shop.test/done?sig=abc#x");
        let model = for_model(&rec.steps).to_string();
        for secret in ["s3cr3t", "t0k3n", "abc", "#x", "sig=zz"] {
            assert!(!model.contains(secret), "{secret} в {model}");
        }
        let draft = plain_draft(&rec.steps);
        assert!(!draft.body.contains("t0k3n") && !draft.body.contains("sig=abc"), "{}", draft.body);
    }

    #[test]
    fn card_numbers_and_codes_are_not_kept() {
        // Номер карты с пробелами и дефисами проходит проверку Луна.
        assert_eq!(generalize("карта 4111 1111 1111 1111 ок"), "карта <номер карты> ок");
        assert_eq!(generalize("карта 5500-0000-0000-0004"), "карта <номер карты>");
        assert_eq!(generalize("тел. 8 999 123-45-67"), "тел. <номер>");
        // Подпись <label> и слова «код», «SMS», «CVC» делают поле секретным для записи.
        for info in [
            json!({ "name": "Код из SMS", "label": "" }),
            json!({ "name": "otp", "label": "" }),
            json!({ "name": "", "label": "CVC" }),
            json!({ "name": "pin", "label": "" }),
            json!({ "auto": "one-time-code" }),
        ] {
            assert!(!record_secret(&info).is_empty(), "{info}");
        }
        assert_eq!(record_secret(&json!({ "name": "Код из SMS" })), "код");
        assert_eq!(record_secret(&json!({ "label": "Security code" })), "данные карты");
        assert_eq!(record_secret(&json!({ "name": "Адрес доставки shipping" })), "");
    }

    #[test]
    fn reply_becomes_numbered_steps_with_checks() {
        let reply = json!({
            "name": "Выгрузка заказа", "description": "когда нужен отчёт",
            "steps": [{ "do": "Открой кабинет", "check": "видна кнопка Экспорт" }, { "do": "Введи номер 87654321" }],
            "human": ["вход по паролю"],
        });
        let draft = draft_from_reply(&reply).unwrap();
        assert_eq!(draft.name, "Выгрузка заказа");
        assert!(draft.body.starts_with("Шаги:\n1. Открой кабинет\n   Проверить: видна кнопка Экспорт\n2. Введи номер <номер>"), "{}", draft.body);
        assert!(draft.body.contains("Где нужен человек:\n- вход по паролю"), "{}", draft.body);
        assert!(draft_from_reply(&json!({ "name": "x", "steps": [] })).is_none());
    }

    #[test]
    fn plain_draft_sends_secrets_to_the_human() {
        let mut rec = Recording::default();
        rec.open("http://shop.test/login", String::new());
        rec.typed("http://shop.test/login", field("Пароль", "password"), "x", String::new());
        let draft = plain_draft(&rec.steps);
        assert!(draft.body.contains("browser_human"), "{}", draft.body);
        assert!(draft.name.contains("shop.test"), "{}", draft.name);
    }
}
