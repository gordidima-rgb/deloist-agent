//! Карточки разрешений. Лежат в `approvals.json` с момента показа и до решения человека: живое ожидание хода
//! (5 минут) их не снимает. Без ответа ход закрывается состоянием «ждёт человека», а карточка остаётся в «Нужен ты»
//! и после перезапуска. Позднее решение продолжает запуск из журнала и разрешает ровно тот вызов, о котором спросили.
//!
//! Сам вызов на диск не пишется (в аргументах бывают значения секретов): хранится только отпечаток инструмента и аргументов.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use tokio::sync::oneshot;

use super::{deliver_message, log_decision, Reply, Turns};
use crate::grants::Scope;
use crate::store::now;
use crate::App;

/// Через сколько без решения напоминаем один раз.
const REMIND_AFTER: Duration = Duration::from_secs(2 * 3600);

/// Открытая карточка: живая (`tx` есть, ход ждёт) или отложенная (хода уже нет, ответ придёт отдельным продолжением).
#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct Approval {
    pub bot_id: String,
    /// Что видит человек, ровно то, что показано в карточке.
    pub payload: Value,
    #[serde(skip)]
    pub tx: Option<oneshot::Sender<Reply>>,
    /// Запись журнала запусков, в которой спросили.
    pub run: Option<String>,
    /// Отпечаток вызова: инструмент и аргументы.
    pub key: String,
    pub conv: Option<String>,
    pub epoch: u64,
    pub scope: Option<Scope>,
    pub reminded: bool,
}

/// Разовое разрешение: один вызов, один раз, в одном запуске.
pub(super) struct Once {
    bot_id: String,
    run: Option<String>,
    key: String,
}

/// Отпечаток вызова. Слова бота (`purpose`) в него не входят: при повторе он может сказать иначе.
pub fn call_key(tool: &str, args: &Value) -> String {
    let mut args = args.clone();
    if let Some(map) = args.as_object_mut() {
        map.remove("purpose");
    }
    crate::store::hex(&Sha256::digest(format!("{tool}\0{args}").as_bytes()))
}

pub(super) fn load(path: &std::path::Path) -> HashMap<String, Approval> {
    std::fs::read_to_string(path).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default()
}

fn remind_after() -> Duration {
    if cfg!(debug_assertions) {
        if let Some(ms) = std::env::var("CREW_TEST_REMIND_AFTER_MS").ok().and_then(|v| v.parse().ok()) {
            return Duration::from_millis(ms);
        }
    }
    REMIND_AFTER
}

/// Как часто смотреть, не пора ли напомнить.
pub fn remind_tick() -> Duration {
    (remind_after() / 2).clamp(Duration::from_millis(50), Duration::from_secs(60))
}

impl Turns {
    fn save_approvals(&self, map: &HashMap<String, Approval>) {
        let Some(path) = &self.approvals_path else { return };
        let tmp = path.with_extension("json.tmp");
        let text = serde_json::to_string_pretty(map).unwrap_or_default();
        if std::fs::write(&tmp, text).and_then(|_| std::fs::rename(&tmp, path)).is_err() {
            eprintln!("[turn] не записал карточки разрешений");
        }
    }

    pub(super) fn add_approval(&self, id: String, item: Approval) {
        let mut map = self.approvals.lock().unwrap();
        map.insert(id, item);
        self.save_approvals(&map);
    }

    pub(super) fn take_approval(&self, id: &str) -> Option<Approval> {
        let mut map = self.approvals.lock().unwrap();
        let item = map.remove(id)?;
        self.save_approvals(&map);
        Some(item)
    }

    /// Живое ожидание кончилось: карточка остаётся, но ждать её решения больше некому. `false` — карточку уже решили.
    pub(super) fn detach_approval(&self, id: &str) -> bool {
        let mut map = self.approvals.lock().unwrap();
        let Some(item) = map.get_mut(id) else { return false };
        item.tx = None;
        true
    }

    /// Карточки, которые ждут запись журнала: для «Нужен ты» и для разбора после перезапуска.
    pub fn card_for_run(&self, run_id: &str) -> Option<(String, String)> {
        let map = self.approvals.lock().unwrap();
        map.iter().find(|(_, a)| a.run.as_deref() == Some(run_id)).map(|(id, a)| (id.clone(), a.payload["title"].as_str().unwrap_or_default().to_string()))
    }

    /// Ход упёрся в карточку без ответа: он должен закончиться, а не продолжать в обход.
    pub(super) fn set_waiting(&self, bot_id: &str, text: &str) {
        self.waiting.lock().unwrap().insert(bot_id.to_string(), text.to_string());
    }

    pub(super) fn waiting_text(&self, bot_id: &str) -> Option<String> {
        self.waiting.lock().unwrap().get(bot_id).cloned()
    }

    pub(super) fn take_waiting(&self, bot_id: &str) -> Option<String> {
        self.waiting.lock().unwrap().remove(bot_id)
    }

    fn grant_once(&self, bot_id: &str, run: Option<String>, key: &str) {
        self.once.lock().unwrap().push(Once { bot_id: bot_id.to_string(), run, key: key.to_string() });
    }

    /// Разовое разрешение на этот вызов в этом запуске: использованное исчезает.
    pub(super) fn take_once(&self, bot_id: &str, run: Option<&str>, key: &str) -> bool {
        let mut list = self.once.lock().unwrap();
        let Some(at) = list.iter().position(|g| g.bot_id == bot_id && g.key == key && (g.run.is_none() || g.run.as_deref() == run)) else { return false };
        list.remove(at);
        true
    }

    /// Бот удалён: его карточки без хода больше никто не ждёт.
    pub(super) fn forget_approvals(&self, bot_id: &str) {
        let mut map = self.approvals.lock().unwrap();
        map.retain(|_, a| a.bot_id != bot_id || a.tx.is_some());
        self.save_approvals(&map);
        self.once.lock().unwrap().retain(|g| g.bot_id != bot_id);
    }

    /// Карточки без решения дольше срока: по одному напоминанию на карточку.
    pub fn due_reminders(&self) -> Vec<Value> {
        let after = remind_after().as_millis() as i64;
        let mut map = self.approvals.lock().unwrap();
        let mut out = Vec::new();
        for (id, item) in map.iter_mut() {
            if item.tx.is_none() && !item.reminded && now() - item.payload["at"].as_i64().unwrap_or(0) >= after {
                item.reminded = true;
                out.push(json!({ "type": "approval_reminder", "botId": item.bot_id, "id": id, "title": item.payload["title"] }));
            }
        }
        if !out.is_empty() {
            self.save_approvals(&map);
        }
        out
    }
}

/// Что запомнить из ответа человека (разрешение разговора или правило) и записать в журнал решений.
pub(super) async fn settle_reply(app: &Arc<App>, bot_id: &str, conv: Option<&str>, epoch: u64, scope: Option<&Scope>, reply: Reply) {
    let Some(scope) = scope else { return };
    // Запомнить можно только то, что можно запомнить; иначе «всегда» значит «один раз».
    let (how, by) = match reply {
        Reply::Deny => ("deny", String::new()),
        Reply::Once => ("once", String::new()),
        Reply::Conversation => match conv.and_then(|c| app.turns.perms.add_grant(bot_id, c, epoch, scope)) {
            Some(grant) => ("conversation_new", grant.id),
            None => ("once", String::new()),
        },
        // Где «всегда» нельзя (удаление и пароли на сайте), «всегда» значит «в этом разговоре».
        Reply::Always if !scope.always => match conv.and_then(|c| app.turns.perms.add_grant(bot_id, c, epoch, scope)) {
            Some(grant) => ("conversation_new", grant.id),
            None => ("once", String::new()),
        },
        Reply::Always => match app.turns.perms.add_rule(bot_id, scope) {
            Some(rule) => ("always", rule.id),
            None => ("once", String::new()),
        },
    };
    if !by.is_empty() {
        app.turns.emit(json!({ "type": "permissions_changed", "botId": bot_id }));
    }
    log_decision(app, bot_id, scope, how, &by).await;
}

/// Ответ человека на карточку. Ждущему ходу ответ уходит сразу; если ждать некому, он продолжает запуск
/// и (если разрешил) один раз пропускает ровно тот вызов. `false` — карточки уже нет.
pub async fn decide_approval(app: &Arc<App>, id: &str, reply: Reply) -> bool {
    let Some(item) = app.turns.take_approval(id) else { return false };
    let status = if reply == Reply::Deny { "denied" } else { "allowed" };
    app.turns.emit(json!({ "type": "approval_closed", "botId": item.bot_id, "id": id, "status": status }));
    if let Some(tx) = item.tx {
        let _ = tx.send(reply);
        return true;
    }
    settle_reply(app, &item.bot_id, item.conv.as_deref(), item.epoch, item.scope.as_ref(), reply).await;
    let title = item.payload["title"].as_str().unwrap_or_default();
    let text = if reply == Reply::Deny {
        format!("Человек запретил: {title}. Продолжай без этого действия.")
    } else {
        app.turns.grant_once(&item.bot_id, item.run.clone(), &item.key);
        format!("Человек разрешил: {title}. Выполни это действие и продолжай задание.")
    };
    if let Some(run) = &item.run {
        if crate::runs::resume(app, run, &text).await.is_ok() {
            return true;
        }
    }
    // Запуска уже нет или бот занят другим: решение приходит сообщением, разрешение действует для любого запуска этого бота.
    if reply != Reply::Deny {
        app.turns.take_once(&item.bot_id, item.run.as_deref(), &item.key);
        app.turns.grant_once(&item.bot_id, None, &item.key);
    }
    let conv = item.payload["conv"].as_str().map(str::to_string).unwrap_or_else(|| format!("bot:{}", item.bot_id));
    if let Err(e) = deliver_message(app, &item.bot_id, &conv, &text).await {
        eprintln!("[turn] решение по карточке не доставлено: {e}");
    }
    true
}
