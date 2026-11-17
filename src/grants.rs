//! Разрешения, которые помнят: «в этом разговоре» и «всегда здесь».
//!
//! Ключ разрешения — (бот, инструмент, место, вид). Его считает сервер по самому вызову,
//! клиент и модель его не присылают. Вид (`pay`, `send`, `delete`, `publish`, `secure`) входит
//! в ключ: правило для обычного действия не открывает оплату, а оплату на разговор или навсегда
//! можно разрешить только с карточки оплаты. Всё лежит в служебной папке, куда ботам нельзя.

use std::path::{Path, PathBuf};
use std::sync::{LazyLock, Mutex};

use regex::Regex;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::store::{now, uid};

/// Место «везде» для инструментов, у которых нет своего места.
pub const ANYWHERE: &str = "*";

/// Что именно бот хочет сделать: из этого строится ключ разрешения и строка журнала.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", default)]
pub struct Scope {
    pub tool: String,
    pub place: String,
    pub kind: String,
    /// Для журнала: `click`, `press`, `run`, `write`… и подпись (текст кнопки, команда, путь).
    pub action: String,
    pub label: String,
    /// Можно ли запомнить ответ. Нет — карточка только «один раз» или «запретить».
    pub grantable: bool,
    /// Можно ли запомнить навсегда. Нет — только «в этом разговоре» (удаление и пароли на сайтах).
    pub always: bool,
}

impl Scope {
    pub fn new(tool: &str, place: &str, kind: &str, action: &str, label: &str) -> Self {
        let label: String = label.chars().take(120).collect();
        let grantable = !place.trim().is_empty();
        Self {
            tool: tool.into(),
            place: place.into(),
            kind: kind.into(),
            action: action.into(),
            label,
            grantable,
            always: grantable,
        }
    }

    pub fn once_only(mut self) -> Self {
        self.grantable = false;
        self.always = false;
        self
    }

    pub fn conversation_only(mut self) -> Self {
        self.always = false;
        self
    }
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Rule {
    pub id: String,
    pub bot_id: String,
    pub tool: String,
    pub place: String,
    pub kind: String,
    pub at: i64,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Grant {
    pub id: String,
    pub bot_id: String,
    /// `bot:<id>` — личный чат, `room:<id>` — группа.
    pub conv: String,
    pub tool: String,
    pub place: String,
    pub kind: String,
    pub label: String,
    pub at: i64,
}

#[derive(Default, Serialize, Deserialize)]
#[serde(default)]
struct File {
    rules: Vec<Rule>,
    grants: Vec<Grant>,
}

/// Чем решено действие без карточки.
#[derive(Clone, Debug, PartialEq)]
pub enum Decided {
    Rule(String),
    Conversation(String),
}

pub struct Perms {
    path: Option<PathBuf>,
    state: Mutex<File>,
    /// Эпоха разговора: растёт при очистке чата. Карточка, открытая до очистки, не может
    /// выдать разрешение новому разговору.
    epochs: Mutex<std::collections::HashMap<String, u64>>,
}

impl Default for Perms {
    fn default() -> Self {
        Self { path: None, state: Mutex::new(File::default()), epochs: Mutex::default() }
    }
}

/// Пути сравниваем по частям и без регистра на Windows: `C:\data` не покрывает `C:\data-evil`.
fn path_key(place: &str) -> PathBuf {
    let text = if cfg!(windows) { place.replace('/', "\\").to_lowercase() } else { place.to_string() };
    PathBuf::from(text.trim_end_matches(['\\', '/']).to_string())
}

fn is_path_place(place: &str) -> bool {
    crate::reach::is_absolute_like(place)
}

/// Покрывает ли разрешённое место то, о котором спрашивают. Папка покрывает только саму
/// себя, без вложенных: разрешение на `C:\Users\me` не открывает автозагрузку в `AppData`.
pub fn place_covers(granted: &str, asked: &str) -> bool {
    if granted.is_empty() || asked.is_empty() {
        return false;
    }
    if is_path_place(granted) && is_path_place(asked) {
        let (g, a) = (path_key(granted), path_key(asked));
        // Только нормальные части: `..` в пути — повод спросить, а не совпадение.
        if a.components().any(|c| matches!(c, std::path::Component::ParentDir | std::path::Component::CurDir)) {
            return false;
        }
        return a == g;
    }
    granted == asked
}

/// Папки, куда запись разрешают только один раз: корень диска, папка профиля и `C:\Users`,
/// `AppData` (автозагрузка), системные папки, профили PowerShell, ключи.
pub fn protected_folder(folder: &Path) -> bool {
    let names: Vec<String> = folder
        .components()
        .filter(|c| matches!(c, std::path::Component::Normal(_)))
        .map(|c| c.as_os_str().to_string_lossy().to_lowercase())
        .collect();
    let Some(top) = names.first() else {
        return true;
    };
    if names.len() == 1 && top == "users" {
        return true;
    }
    if matches!(top.as_str(), "windows" | "program files" | "program files (x86)" | "programdata") {
        return true;
    }
    if names.iter().any(|n| matches!(n.as_str(), "appdata" | "powershell" | "windowspowershell" | ".ssh" | ".config" | "startup" | "start menu")) {
        return true;
    }
    let profile = std::env::var_os("USERPROFILE").or_else(|| std::env::var_os("HOME"));
    if let Some(profile) = profile {
        let profile = crate::reach::display(&std::fs::canonicalize(&profile).unwrap_or_else(|_| PathBuf::from(&profile)));
        if path_key(&profile) == path_key(&crate::reach::display(folder)) {
            return true;
        }
    }
    false
}

fn same(tool: &str, place: &str, kind: &str, scope: &Scope) -> bool {
    tool == scope.tool && kind == scope.kind && place_covers(place, &scope.place)
}

impl Perms {
    pub fn open(dir: &Path) -> Self {
        let path = dir.join("permissions.json");
        let state = std::fs::read_to_string(&path).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default();
        Self { path: Some(path), state: Mutex::new(state), epochs: Mutex::default() }
    }

    pub fn epoch(&self, conv: &str) -> u64 {
        self.epochs.lock().unwrap().get(conv).copied().unwrap_or(0)
    }

    fn save(&self, state: &File) {
        let Some(path) = &self.path else { return };
        let text = serde_json::to_string_pretty(state).unwrap_or_default();
        let tmp = path.with_extension("json.tmp");
        if std::fs::write(&tmp, text).and_then(|_| std::fs::rename(&tmp, path)).is_err() {
            eprintln!("[grants] не записал разрешения");
        }
    }

    /// Покрыт ли вызов. `conv` — разговор текущего хода; у рутин и передачи задачи его нет,
    /// им доступны только правила.
    pub fn find(&self, bot_id: &str, conv: Option<&str>, scope: &Scope) -> Option<Decided> {
        if !scope.grantable {
            return None;
        }
        let state = self.state.lock().unwrap();
        if let Some(rule) = state.rules.iter().find(|r| r.bot_id == bot_id && same(&r.tool, &r.place, &r.kind, scope)) {
            return Some(Decided::Rule(rule.id.clone()));
        }
        let conv = conv?;
        state
            .grants
            .iter()
            .find(|g| g.bot_id == bot_id && g.conv == conv && same(&g.tool, &g.place, &g.kind, scope))
            .map(|g| Decided::Conversation(g.id.clone()))
    }

    pub fn add_rule(&self, bot_id: &str, scope: &Scope) -> Option<Rule> {
        if !scope.grantable || !scope.always {
            return None;
        }
        let mut state = self.state.lock().unwrap();
        if let Some(existing) = state.rules.iter().find(|r| r.bot_id == bot_id && r.tool == scope.tool && r.place == scope.place && r.kind == scope.kind) {
            return Some(existing.clone());
        }
        let rule = Rule { id: uid("rule"), bot_id: bot_id.into(), tool: scope.tool.clone(), place: scope.place.clone(), kind: scope.kind.clone(), at: now() };
        state.rules.push(rule.clone());
        self.save(&state);
        Some(rule)
    }

    /// `epoch` — эпоха разговора на момент показа карточки: если чат с тех пор очищен, нет.
    pub fn add_grant(&self, bot_id: &str, conv: &str, epoch: u64, scope: &Scope) -> Option<Grant> {
        if !scope.grantable || conv.is_empty() || self.epoch(conv) != epoch {
            return None;
        }
        let mut state = self.state.lock().unwrap();
        if let Some(existing) = state.grants.iter().find(|g| {
            g.bot_id == bot_id && g.conv == conv && g.tool == scope.tool && g.place == scope.place && g.kind == scope.kind
        }) {
            return Some(existing.clone());
        }
        let grant = Grant {
            id: uid("grant"),
            bot_id: bot_id.into(),
            conv: conv.into(),
            tool: scope.tool.clone(),
            place: scope.place.clone(),
            kind: scope.kind.clone(),
            label: scope.label.clone(),
            at: now(),
        };
        state.grants.push(grant.clone());
        self.save(&state);
        Some(grant)
    }

    pub fn rules(&self) -> Vec<Rule> {
        self.state.lock().unwrap().rules.clone()
    }

    pub fn grants(&self, conv: Option<&str>) -> Vec<Grant> {
        let state = self.state.lock().unwrap();
        state.grants.iter().filter(|g| conv.is_none_or(|c| g.conv == c)).cloned().collect()
    }

    pub fn remove_rule(&self, id: &str) -> bool {
        let mut state = self.state.lock().unwrap();
        let before = state.rules.len();
        state.rules.retain(|r| r.id != id);
        let changed = state.rules.len() != before;
        if changed {
            self.save(&state);
        }
        changed
    }

    pub fn remove_grant(&self, id: &str) -> bool {
        let mut state = self.state.lock().unwrap();
        let before = state.grants.len();
        state.grants.retain(|g| g.id != id);
        let changed = state.grants.len() != before;
        if changed {
            self.save(&state);
        }
        changed
    }

    /// Чат очищен или группа удалена: разрешения этого разговора кончаются.
    pub fn end_conversation(&self, conv: &str) -> bool {
        *self.epochs.lock().unwrap().entry(conv.to_string()).or_insert(0) += 1;
        let mut state = self.state.lock().unwrap();
        let before = state.grants.len();
        state.grants.retain(|g| g.conv != conv);
        let changed = state.grants.len() != before;
        if changed {
            self.save(&state);
        }
        changed
    }

    /// Бот ушёл из группы: его разрешения в этой группе кончаются.
    pub fn forget_bot_in(&self, bot_id: &str, conv: &str) -> bool {
        let mut state = self.state.lock().unwrap();
        let before = state.grants.len();
        state.grants.retain(|g| !(g.bot_id == bot_id && g.conv == conv));
        let changed = state.grants.len() != before;
        if changed {
            self.save(&state);
        }
        changed
    }

    /// Бот удалён: его правила и разрешения тоже.
    pub fn forget_bot(&self, bot_id: &str) {
        let mut state = self.state.lock().unwrap();
        state.rules.retain(|r| r.bot_id != bot_id);
        state.grants.retain(|g| g.bot_id != bot_id);
        self.save(&state);
    }
}

// ---------------------------------------------------------------------------
// Виды действий
// ---------------------------------------------------------------------------

static SECURE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)парол|двухфактор|двухэтапн|безопасност|выйти (из|на) всех|password|passcode|two-factor|2fa|security|sign out (of )?all").unwrap()
});
static PAY: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)оплат|плат[её]ж|купи|покуп|заказ|оформ|перев[её]|перевод|закрыть сч|вывест|вывод средств|pay|buy|purchase|order|checkout|transfer|withdraw").unwrap()
});
static DELETE: LazyLock<Regex> = LazyLock::new(|| {
    // «удали|удалени|удаля», а не «удал»: «открытый удалённо» — не кнопка удаления (12.11.2026).
    Regex::new(r"(?i)удали|удалени|удаля|стереть|сотри|очист|отмен|отпис|delete|remove|erase|unsubscribe|cancel|clear").unwrap()
});
// Лайк, репост и подписка на X и в соцсетях — публичные действия от имени человека (13.11.2026: «Понравилось»,
// «Сделать репост» и «Читать @имя» на x.com проходили без вопроса).
static PUBLISH: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"(?i)опубликов|публикац|поделит|share|publish|post\b|репост|retweet|нравится|понравил|\blike\b|unlike|читать @|follow @|комментир|оставить комментарий|\bcomment\b|проголос|голосовать|upvote|downvote",
    )
    .unwrap()
});
// «Добавить в друзья» (VK, Facebook) и «Присоединиться» к серверу Discord — тоже от имени человека (13.11.2026).
static SEND: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)отправ|подтверд|подпис|вступ|\bjoin\b|\bsubscribe\b|\bfollow\b|send|confirm|submit|reply|invite|в друзья|add friend|присоедин|принять приглашение|accept invite|показать (телефон|номер)").unwrap()
});

/// Перевод текста, а не денег: «Показать перевод» стоит под каждым иностранным постом на X (13.11.2026).
static TRANSLATE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)^\s*(показать (перевод|оригинал)|перевести (пост|публикацию|текст|сообщение|страницу|твит)|show (translation|original)|translate( post| text)?)\s*$").unwrap()
});

/// Вид необратимого действия по подписи элемента или имени инструмента. `None` — обычное.
pub fn risky_kind(text: &str) -> Option<&'static str> {
    if TRANSLATE.is_match(text) {
        None
    } else if SECURE.is_match(text) {
        Some("secure")
    } else if PAY.is_match(text) {
        Some("pay")
    } else if DELETE.is_match(text) {
        Some("delete")
    } else if PUBLISH.is_match(text) {
        Some("publish")
    } else if SEND.is_match(text) {
        Some("send")
    } else {
        None
    }
}

/// Строка журнала словами: «Рен нажал „Экспорт“ на seller.ozon.ru — разрешено правилом „всегда здесь“».
pub fn log_text(bot_name: &str, scope: &Scope, how: &str) -> String {
    let verb = match scope.action.as_str() {
        "click" => format!("нажал „{}“", scope.label),
        "press" => format!("нажал клавиши {}", scope.label),
        "type" => "ввёл текст".to_string(),
        "drag" => format!("перетащил „{}“", scope.label),
        "dialog" => "подтвердил диалог".to_string(),
        "upload" => format!("загрузил файл {}", scope.label),
        "site" => "начал действовать".to_string(),
        "run" => format!("выполнил «{}»", scope.label),
        "write" => format!("записал {}", scope.label),
        _ => format!("вызвал {}", scope.tool),
    };
    let place = if scope.place == ANYWHERE || scope.place.is_empty() {
        String::new()
    } else if scope.action == "run" {
        String::new()
    } else if is_path_place(&scope.place) {
        format!(" в {}", scope.place)
    } else {
        format!(" на {}", scope.place)
    };
    let how = match how {
        "rule" => "разрешено правилом „всегда здесь“",
        "conversation" => "разрешено в этом разговоре",
        "once" => "человек разрешил один раз",
        "always" => "человек разрешил всегда здесь",
        "conversation_new" => "человек разрешил в этом разговоре",
        "deny" => "человек запретил",
        _ => "без ответа человека",
    };
    format!("{bot_name} {verb}{place} — {how}")
}

/// Запись журнала решений.
pub fn log_entry(bot_id: &str, bot_name: &str, scope: &Scope, how: &str, by_id: &str) -> Value {
    json!({
        "at": now(), "botId": bot_id, "bot": bot_name, "tool": scope.tool, "place": scope.place, "kind": scope.kind,
        "action": scope.action, "label": scope.label, "how": how, "by": by_id, "text": log_text(bot_name, scope, how),
    })
}

#[cfg(test)]
mod tests {
    #[test]
    fn remote_approval_is_default_deny() {
        let ws = std::path::Path::new("C:/data/workspaces/bot1");
        let scope = |tool: &str, place: &str, kind: &str| Scope::new(tool, place, kind, "run", "x");
        let no = Value::Null;
        let cmd = |c: &str| json!({ "command": c });
        let ok = |tool: &str, args: &Value, s: Option<&Scope>| remote_ok(tool, args, s, false, false, ws);
        assert_eq!(ok("web_fetch", &no, None), Remote::Safe);
        assert_eq!(ok("browser_nav", &no, None), Remote::Safe);
        assert_eq!(ok("browser_site", &no, None), Remote::Actions);
        assert_eq!(ok("update_bot", &no, Some(&scope("update_bot", "*", ""))), Remote::No);
        assert_eq!(ok("", &no, None), Remote::No);
        let s = scope("run_command", "echo", "");
        // Только чтение — для любого устройства.
        for good in ["echo hi", "dir notes", "type notes/a.txt", "findstr /i abc notes/a.txt", "git status", "git log --oneline", "ls", "cat a.txt", "where git"] {
            assert_eq!(ok("run_command", &cmd(good), Some(&s)), Remote::Safe, "{good}");
        }
        // Другие программы — только с переключателем.
        for other in ["git commit -m x", "node app.js", "npm install", "git push"] {
            assert_eq!(ok("run_command", &cmd(other), Some(&s)), Remote::Actions, "{other}");
        }
        // Перенаправление, цепочки, подстановки, кавычки, выход наверх, чужие пути — никогда.
        for bad in [
            "echo hi > a.txt", "type a.txt | more", "echo a & echo b", "echo a; echo b", "echo a^b", "echo \"x\"", "type ../a.txt", "type ~/a.txt",
            "type C:\\Windows\\win.ini", "type C:/Windows/win.ini", "type /etc/passwd", "curl https://x.y", "echo $HOME", "echo %PATH%", "echo hi\nwhoami", "",
        ] {
            assert_eq!(ok("run_command", &cmd(bad), Some(&s)), Remote::No, "{bad:?}");
        }
        for kind in ["delete", "secure", "pay", "publish", "send", "login", "unknown"] {
            assert_eq!(ok("run_command", &cmd("echo hi"), Some(&scope("run_command", "echo", kind))), Remote::No, "{kind}");
        }
        assert_eq!(remote_ok("run_command", &cmd("echo hi"), Some(&s), true, false, ws), Remote::No);
        assert_eq!(remote_ok("run_command", &cmd("echo hi"), Some(&s), false, true, ws), Remote::No);
        assert_eq!(ok("run_command", &cmd("echo hi"), Some(&s.clone().once_only())), Remote::No, "нельзя запомнить — нельзя и с устройства");
        // mcp без объявленного readOnlyHint закрыт.
        assert_eq!(ok("mcp__files__read", &no, Some(&scope("mcp__files__read", "files", ""))), Remote::No);
    }

    #[test]
    fn remote_write_rejects_executables_escapes_and_links() {
        // Не `%TEMP%`: из папки с меткой Low он недоступен на запись, поэтому рядом со сборкой.
        let dir = std::env::current_dir().unwrap().join("target").join(format!("deloist-remote-write-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let ws = dir.join("ws");
        std::fs::create_dir_all(ws.join("sub")).unwrap();
        std::fs::create_dir_all(dir.join("outside")).unwrap();
        let w = |raw: &str| write_allowed(raw, &ws);
        assert!(w("notes.txt") && w("sub/a.md") && w("new/dir/a.json"));
        for ext in EXECUTABLE_EXT {
            assert!(!w(&format!("x.{ext}")), "{ext}");
            assert!(!w(&format!("x.{}", ext.to_uppercase())), "{ext} в верхнем регистре");
        }
        assert!(!w("a.txt.bat") && !w("a.bat.") && !w("a.bat ") && !w("a.txt:evil.bat") && !w("a.txt:stream"));
        assert!(!w("../outside/a.txt") && !w(""));
        assert!(!w(&dir.join("outside").join("a.txt").display().to_string()), "вне рабочей папки");
        assert!(w(&ws.join("sub").join("ok.txt").display().to_string()));
        // Ссылка внутри рабочей папки на папку снаружи (junction на Windows, symlink в других системах).
        let link = ws.join("link");
        #[cfg(windows)]
        let made = std::process::Command::new("cmd").args(["/C", "mklink", "/J"]).arg(&link).arg(dir.join("outside")).output().map(|o| o.status.success()).unwrap_or(false);
        #[cfg(unix)]
        let made = std::os::unix::fs::symlink(dir.join("outside"), &link).is_ok();
        if made {
            assert!(!w("link/a.txt"), "ссылка уводит наружу");
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    use super::*;

    fn scope(tool: &str, place: &str, kind: &str) -> Scope {
        Scope::new(tool, place, kind, "click", "x")
    }

    #[test]
    fn a_rule_covers_only_its_bot_tool_place_and_kind() {
        let perms = Perms::default();
        perms.add_rule("b1", &scope("browser_site", "seller.ozon.ru", "")).unwrap();
        assert!(perms.find("b1", None, &scope("browser_site", "seller.ozon.ru", "")).is_some());
        assert!(perms.find("b2", None, &scope("browser_site", "seller.ozon.ru", "")).is_none(), "чужой бот");
        assert!(perms.find("b1", None, &scope("browser_risky", "seller.ozon.ru", "")).is_none(), "другой инструмент");
        assert!(perms.find("b1", None, &scope("browser_site", "ozon.ru", "")).is_none(), "другое место");
        assert!(perms.find("b1", None, &scope("browser_site", "seller.ozon.ru.evil.com", "")).is_none());
        assert!(perms.find("b1", None, &scope("browser_site", "seller.ozon.ru", "pay")).is_none(), "оплату обычное правило не открывает");
    }

    #[test]
    fn a_risky_kind_is_opened_only_by_a_card_of_that_kind() {
        let perms = Perms::default();
        perms.add_rule("b1", &scope("browser_risky", "shop.ru", "delete")).unwrap();
        assert!(perms.find("b1", None, &scope("browser_risky", "shop.ru", "delete")).is_some());
        assert!(perms.find("b1", None, &scope("browser_risky", "shop.ru", "pay")).is_none());
        assert!(perms.find("b1", None, &scope("browser_risky", "shop.ru", "send")).is_none());
    }

    #[test]
    fn a_folder_covers_only_itself() {
        let root = if cfg!(windows) { r"C:\data" } else { "/data" };
        let same_case = if cfg!(windows) { r"c:\DATA\" } else { "/data/" };
        let inside = if cfg!(windows) { r"C:\Data\reports" } else { "/data/reports" };
        let sibling = if cfg!(windows) { r"C:\data-evil" } else { "/data-evil" };
        let climb = if cfg!(windows) { r"C:\data\..\secret" } else { "/data/../secret" };
        assert!(place_covers(root, root));
        assert!(place_covers(root, same_case));
        assert!(!place_covers(root, inside), "вложенные папки — уже другое место");
        assert!(!place_covers(root, sibling));
        assert!(!place_covers(root, climb));
        assert!(!place_covers(inside, root), "разрешение на подпапку не открывает родителя");
        assert!(!place_covers("", root));
    }

    #[test]
    fn a_conversation_grant_stays_in_its_conversation_and_ends() {
        let perms = Perms::default();
        let s = scope("write_file", ANYWHERE, "");
        let g = perms.add_grant("b1", "bot:b1", 0, &s).unwrap();
        assert!(perms.find("b1", Some("bot:b1"), &s).is_some());
        assert!(perms.find("b1", Some("room:r1"), &s).is_none(), "другой разговор");
        assert!(perms.find("b1", None, &s).is_none(), "рутина без человека");
        assert!(perms.find("b2", Some("bot:b1"), &s).is_none(), "чужой бот");
        assert!(perms.remove_grant(&g.id));
        assert!(perms.find("b1", Some("bot:b1"), &s).is_none());
        perms.add_grant("b1", "bot:b1", 0, &s).unwrap();
        assert!(perms.end_conversation("bot:b1"));
        assert!(perms.find("b1", Some("bot:b1"), &s).is_none(), "очистка чата");
        // Карточка, показанная до очистки (эпоха 0), новому разговору разрешение не выдаёт.
        assert!(perms.add_grant("b1", "bot:b1", 0, &s).is_none(), "старая карточка после очистки");
        assert!(perms.add_grant("b1", "bot:b1", perms.epoch("bot:b1"), &s).is_some());
    }

    #[test]
    fn a_bot_leaving_a_room_loses_its_grants_there() {
        let perms = Perms::default();
        let s = scope("write_file", ANYWHERE, "");
        perms.add_grant("b1", "room:r1", 0, &s).unwrap();
        perms.add_grant("b2", "room:r1", 0, &s).unwrap();
        assert!(perms.forget_bot_in("b1", "room:r1"));
        assert!(perms.find("b1", Some("room:r1"), &s).is_none());
        assert!(perms.find("b2", Some("room:r1"), &s).is_some(), "чужие разрешения в группе остаются");
    }

    #[test]
    fn nothing_is_remembered_for_a_once_only_scope() {
        let perms = Perms::default();
        let s = scope("browser_eval", "x.ru", "").once_only();
        assert!(perms.add_rule("b1", &s).is_none());
        assert!(perms.add_grant("b1", "bot:b1", 0, &s).is_none());
        assert!(perms.find("b1", Some("bot:b1"), &s).is_none());
        // Удаление на сайте — только на разговор.
        let d = scope("browser_risky", "https://shop.ru", "delete").conversation_only();
        assert!(perms.add_rule("b1", &d).is_none());
        assert!(perms.add_grant("b1", "bot:b1", 0, &d).is_some());
    }

    #[test]
    fn system_and_profile_folders_are_once_only() {
        if !cfg!(windows) {
            return;
        }
        for folder in [
            r"C:\",
            r"C:\Users",
            r"C:\Windows\System32",
            r"C:\Program Files\App",
            r"C:\Users\me\AppData\Roaming\Microsoft\Windows\Start Menu\Programs\Startup",
            r"C:\Users\me\Documents\PowerShell",
            r"C:\Users\me\Documents\WindowsPowerShell",
            r"C:\Users\me\.ssh",
        ] {
            assert!(protected_folder(Path::new(folder)), "{folder}");
        }
        if let Some(profile) = std::env::var_os("USERPROFILE") {
            assert!(protected_folder(Path::new(&profile)), "папка профиля");
        }
        assert!(!protected_folder(Path::new(r"D:\work\reports")));
    }

    #[test]
    fn kinds_come_from_words() {
        assert_eq!(risky_kind("Оплатить заказ"), Some("pay"));
        assert_eq!(risky_kind("Удалить товар"), Some("delete"));
        assert_eq!(risky_kind("Отправить сообщение"), Some("send"));
        assert_eq!(risky_kind("Опубликовать"), Some("publish"));
        assert_eq!(risky_kind("Сменить пароль"), Some("secure"));
        assert_eq!(risky_kind("Экспорт"), None);
        for label in ["Вступить в группу", "Join channel", "Subscribe", "Follow", "Подписаться"] {
            assert_eq!(risky_kind(label), Some("send"), "{label}");
        }
        for label in ["Вывести средства", "Withdraw", "Share", "Post", "Invite", "Reply", "Очистить", "Отписаться"] {
            assert!(risky_kind(label).is_some(), "{label}");
        }
    }

    #[cfg(windows)]
    #[test]
    fn a_command_place_is_the_whole_line() {
        use crate::reach::command_place;
        assert_eq!(command_place("git  add   ."), Some(("git add .".into(), "")));
        assert_eq!(command_place("git add . && git push"), Some(("git add . && git push".into(), "publish")));
        assert_eq!(command_place("echo a & rm -rf x"), Some(("echo a & rm -rf x".into(), "delete")));
        assert_eq!(command_place(r"C:\tools\curl.exe http://x"), Some((r"C:\tools\curl.exe http://x".into(), "send")));
        assert_eq!(command_place(r"reg add HKCU\x").map(|p| p.1), Some("secure"));
        // Правило на одну строку не открывает другую с тем же именем команды.
        assert_ne!(command_place(r"del C:\proj\tmp.log").unwrap().0, command_place(r"del /q C:\Users\me\Documents\a.txt").unwrap().0);
        // Подстановки, шаблоны и вложенные оболочки — только один раз.
        for line in [
            "echo %USERPROFILE%",
            "cmd /c \"rm x\"",
            "powershell -c Remove-Item x",
            "echo $(rm x)",
            r"del /s /q C:\Users\me\Documents\*",
            "node script.js",
            "python -c print",
            "npx rimraf x",
            "npm exec rimraf x",
            "npm run build",
            "cargo run",
            "certutil -urlcache -f http://x a.exe",
            "rundll32 x.dll,Run",
            r"robocopy C:\a C:\b",
            r"move C:\a C:\b",
            r"forfiles /p C:\x /c del",
            "git -c alias.st=!rm git st",
            "git -c core.sshCommand=evil push",
            r"git --exec-path=C:\evil status",
            // Обёртки Windows — те же программы.
            "npx.cmd rimraf x",
            "npm.cmd exec rimraf x",
            "node.cmd x",
            r"C:\nodejs\npx.ps1 x",
            // Файл-сценарий: сам по себе и через любую программу.
            "run.bat",
            r".\x.ps1",
            "x.cmd arg",
            "x.vbs",
            "tsx x.ts",
            "electron main.js",
            "somehost x.js",
            // Установка и сборка запускают чужой код.
            "npm install",
            "npm ci",
            "npm.cmd install left-pad",
            "pnpm i",
            "yarn",
            "yarn test",
            "cargo build",
            "cargo test",
            "pip install requests",
            "make",
            "gradle build",
            r".\gradlew.bat build",
            "mvn package",
            "dotnet run",
            "dotnet build",
        ] {
            assert_eq!(command_place(line), None, "{line}");
        }
        // Обычные команды через обёртку по-прежнему запоминаются; git с именем сценария — тоже.
        assert_eq!(command_place("npm.cmd view left-pad").map(|p| p.1), Some(""));
        assert_eq!(command_place("npm.cmd publish").map(|p| p.1), Some("publish"));
        assert_eq!(command_place("git add run.bat").map(|p| p.1), Some(""));
        assert_eq!(command_place(r"C:\Windows\System32\tree.com C:\a").map(|p| p.1), Some(""));
        // Ключи git со значением не мешают узнать подкоманду.
        assert_eq!(command_place(r"git --git-dir C:\r\.git --work-tree C:\r push").map(|p| p.1), Some("publish"));
        assert_eq!(command_place(r"git -C C:\r reset --hard").map(|p| p.1), Some("delete"));
        assert_eq!(command_place(r"git -C C:\r branch -D old").map(|p| p.1), Some("delete"));
        assert_eq!(command_place(r"git -C C:\r push --force").map(|p| p.1), Some("delete"));
    }

    #[test]
    fn the_log_reads_as_words() {
        let s = Scope::new("browser_site", "seller.ozon.ru", "", "click", "Экспорт");
        assert_eq!(log_text("Рен", &s, "rule"), "Рен нажал „Экспорт“ на seller.ozon.ru — разрешено правилом „всегда здесь“");
    }
}

// ---------------------------------------------------------------------------
// Одобрение с другого устройства (телефон, второй ПК)
// ---------------------------------------------------------------------------

/// Кому можно одобрить карточку с другого устройства. Считается один раз при создании карточки.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Remote {
    /// Только на самом ПК.
    #[default]
    No,
    /// Любому доверенному устройству.
    Safe,
    /// Только устройству с включённым переключателем «Разрешать действия в рабочей папке».
    Actions,
}

impl Remote {
    pub fn as_str(self) -> &'static str {
        match self {
            Remote::No => "no",
            Remote::Safe => "safe",
            Remote::Actions => "actions",
        }
    }
}

/// Расширения, запись которых не уходит на устройство никогда: запускаются сами или открывают запуск.
const EXECUTABLE_EXT: &[&str] = &[
    "bat", "cmd", "ps1", "psm1", "vbs", "vbe", "js", "jse", "wsf", "wsh", "py", "pyw", "exe", "com", "scr", "lnk", "dll", "msi", "msp", "jar", "sh", "bash", "reg", "hta", "cpl", "url",
];
/// Только чтение: эти программы с устройства можно запускать без переключателя.
const READ_ONLY_PROGRAMS: &[&str] = &["dir", "type", "findstr", "where", "ls", "cat", "echo"];
const GIT_READ_ONLY: &[&str] = &["status", "log", "diff", "show"];

/// Символы и слова, которыми команда выходит из «простой»: перенаправление, цепочки, подстановки, выход наверх.
fn command_tokens_ok(command: &str) -> bool {
    !command.trim().is_empty()
        && !command.chars().any(|c| matches!(c, '>' | '<' | '|' | '&' | ';' | '^' | '"' | '~' | '\\' | '`' | '$' | '%' | '(' | ')' | '{' | '}' | '*' | '?' | '!') || c.is_control())
        && !command.contains("..")
}

fn read_only_command(command: &str) -> bool {
    let mut words = command.split_whitespace();
    let first = words.next().unwrap_or_default().to_lowercase();
    if first == "git" {
        return words.next().is_some_and(|sub| GIT_READ_ONLY.contains(&sub.to_lowercase().as_str()));
    }
    READ_ONLY_PROGRAMS.contains(&first.as_str())
}

/// Простая читающая команда без перенаправлений и цепочек: ничего не меняет (для журнала запусков).
pub(crate) fn is_plain_read_only(command: &str) -> bool {
    command_tokens_ok(command) && read_only_command(command)
}

/// Настоящий путь без ссылок: самая длинная существующая часть канонизируется, остальное достраивается как есть.
fn canonical(path: &std::path::Path) -> Option<std::path::PathBuf> {
    let mut rest = Vec::new();
    let mut current = path.to_path_buf();
    loop {
        if let Ok(real) = std::fs::canonicalize(&current) {
            let mut out = real;
            for part in rest.iter().rev() {
                out.push(part);
            }
            return Some(out);
        }
        rest.push(current.file_name()?.to_os_string());
        current = current.parent()?.to_path_buf();
    }
}

fn executable_name(name: &str) -> bool {
    let name = name.trim_end_matches(['.', ' ']).to_lowercase();
    name.contains(':') || name.rsplit_once('.').is_some_and(|(_, ext)| EXECUTABLE_EXT.contains(&ext.trim()))
}

/// Запись `raw` с устройства: путь настоящий и внутри настоящей рабочей папки, имя не исполняемого вида.
fn write_allowed(raw: &str, workspace: &std::path::Path) -> bool {
    if raw.trim().is_empty() || raw.contains("..") {
        return false;
    }
    let path = std::path::Path::new(raw);
    let target = if path.is_absolute() { path.to_path_buf() } else { workspace.join(path) };
    let Some(name) = target.file_name().map(|n| n.to_string_lossy().into_owned()) else { return false };
    if executable_name(&name) {
        return false;
    }
    let (Some(real), Some(root)) = (canonical(&target), canonical(workspace)) else { return false };
    inside_workspace(&crate::reach::display(&real), &root)
}

/// Что можно одобрить с другого устройства. По умолчанию ничего: открыто только названное здесь.
/// Ответы `always` и `conversation` и переключатель устройства проверяются отдельно (`relay_ops`).
pub fn remote_ok(tool: &str, args: &Value, scope: Option<&Scope>, sensitive: bool, danger: bool, workspace: &std::path::Path) -> Remote {
    if sensitive || danger {
        return Remote::No;
    }
    // Только обычные действия: удаление, пароли, платежи, выпуск, отправка людям и незнакомый вид остаются на ПК.
    if scope.is_some_and(|s| !s.kind.is_empty()) {
        return Remote::No;
    }
    let grantable = scope.is_some_and(|s| s.grantable);
    match tool {
        "web_fetch" | "web_search" | "remember" | "share_file" | "browser_nav" | "browser_page" => Remote::Safe,
        "browser_site" | "browser_mouse" | "browser_keyboard" => Remote::Actions,
        "write_file" if grantable && write_allowed(arg(args, "path"), workspace) => Remote::Safe,
        "run_command" if grantable && command_tokens_ok(arg(args, "command")) && command_inside(arg(args, "command"), workspace) => {
            if read_only_command(arg(args, "command")) {
                Remote::Safe
            } else {
                Remote::Actions
            }
        }
        _ if tool.starts_with("mcp__") && grantable && crate::mcp::is_read_only(tool) => Remote::Safe,
        _ => Remote::No,
    }
}

fn norm_path(text: &str) -> String {
    text.trim_matches(|c| c == '"' || c == '\'').replace('\\', "/").to_lowercase().trim_end_matches('/').to_string()
}

/// `path` (строка) лежит внутри `workspace`. Обе стороны берутся как есть, без обращения к диску.
fn inside_workspace(path: &str, workspace: &std::path::Path) -> bool {
    let (path, root) = (norm_path(path), norm_path(&crate::reach::display(workspace)));
    !root.is_empty() && !path.split('/').any(|p| p == "..") && (path == root || path.starts_with(&format!("{root}/")))
}

/// Все похожие на путь слова команды лежат в рабочей папке бота: абсолютные пути внутри неё, относительные без `..`.
fn command_inside(command: &str, workspace: &std::path::Path) -> bool {
    if command.trim().is_empty() {
        return false;
    }
    command.split_whitespace().all(|word| {
        let word = word.trim_matches(|c| c == '"' || c == '\'');
        // Ключи программ в стиле Windows: `/s`, `/i`.
        if word.starts_with('/') && word.len() <= 3 && word[1..].chars().all(|c| c.is_ascii_alphanumeric()) {
            return true;
        }
        if word.split(['/', '\\']).any(|p| p == "..") || word.starts_with('~') {
            return false;
        }
        let drive = word.len() >= 2 && word.as_bytes()[1] == b':';
        let absolute = word.starts_with('/') || word.starts_with('\\') || drive;
        if absolute {
            return inside_workspace(word, workspace);
        }
        // `https://…` и подобное: двоеточие вне диска — это не локальный путь.
        !word.contains(':')
    })
}

// ---------------------------------------------------------------------------
// Область вызова инструмента
// ---------------------------------------------------------------------------

fn arg<'a>(args: &'a Value, key: &str) -> &'a str {
    args.get(key).and_then(Value::as_str).unwrap_or_default()
}

/// Что разрешает карточка обычного инструмента. Браузер считает свою область сам.
pub fn scope_for(app: &crate::App, bot: &crate::store::Bot, name: &str, args: &Value) -> Option<Scope> {
    // Файл с ключами: свой вид `secret`, только «один раз» или «в этом разговоре».
    if let Some(touch) = crate::keys::call_touch(name, args) {
        return Some(touch.scope(name));
    }
    let scope = match name {
        "run_command" => {
            let command = arg(args, "command");
            // Выпуск наружу — каждый раз отдельное решение: ни «в разговоре», ни «всегда».
            if crate::reach::release_command(command).is_some() {
                return Some(Scope::new(name, "", "publish", "run", command).once_only());
            }
            match crate::reach::command_place(command) {
                Some((place, kind)) => Scope::new(name, &place, kind, "run", &place),
                None => Scope::new(name, "", "", "run", command).once_only(),
            }
        }
        "write_file" => {
            let raw = arg(args, "path");
            match crate::tools::write_target(app, &bot.id, raw) {
                Ok((path, _)) => {
                    let folder = path.parent().map(crate::reach::display).unwrap_or_default();
                    let file = path.file_name().map(|f| f.to_string_lossy().into_owned()).unwrap_or_default();
                    let scope = Scope::new(name, &folder, "", "write", &file);
                    if path.parent().is_none_or(protected_folder) {
                        scope.once_only()
                    } else {
                        scope
                    }
                }
                Err(_) => Scope::new(name, "", "", "write", raw).once_only(),
            }
        }
        "generate_image" => Scope::new(name, ANYWHERE, "pay", "call", ""),
        _ if name.starts_with("mcp__") => {
            let (server, tool) = crate::mcp::split_name(name)?;
            let kind = risky_kind(&tool.replace('_', " ")).unwrap_or("");
            Scope::new(name, &server, kind, "call", &tool)
        }
        // Боты правят и создают ботов, передают задачи: запоминать такое нельзя, иначе бот A
        // перепишет бота B вместе с его правилами.
        "create_bot" | "update_bot" | "delegate" => Scope::new(name, ANYWHERE, "", "call", "").once_only(),
        _ => Scope::new(name, ANYWHERE, "", "call", ""),
    };
    // Удаление рутины — каждый раз отдельное решение. Рутины и цели из хода без человека
    // (рутина, ночная смена, самопроверка) тоже спрашиваются каждый раз: такой ход человек
    // не запускал, и правило «всегда» не должно молча менять его расписание.
    let unattended = app.turns.is_unattended(&bot.id) || matches!(crate::goals::current_run(&bot.id), Some(crate::goals::Run::CheckIn));
    let schedule_tool = crate::routine_tools::is_routine_tool(name) || crate::goal_tools::is_goal_tool(name);
    let scope = if crate::tools::never_remember(name) || (schedule_tool && unattended) { scope.once_only() } else { scope };
    Some(scope)
}
