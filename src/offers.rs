//! Карточки «Подключу специалиста»: лид предлагает специалиста, а решает человек.
//! Нерешённые карточки сохраняются на диск и переживают перезапуск.

use std::collections::{HashMap, VecDeque};
use std::path::Path;
use std::path::PathBuf;
use std::sync::Mutex;

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

/// Сколько предложений помнит один бот: старые вытесняются новыми.
pub const MAX_OFFERS_PER_BOT: usize = 20;

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Offer {
    pub id: String,
    pub host_bot_id: String,
    pub specialist_id: String,
    pub reason: String,
    pub task: String,
    pub at: i64,
    /// Предложение собрать команду (`propose_team`): ровно то, что создадим по «Создать».
    /// Хранится здесь, а не перечитывается у агента: человек видит и подтверждает именно этот состав.
    #[serde(default)]
    pub team: Option<crate::team_proposal::TeamPlan>,
    /// Предложение поправить участника или завести ему рутину (`propose_bot_change`, `propose_routine`):
    /// применяется ровно то, что сохранено здесь.
    #[serde(default)]
    pub change: Option<crate::team_changes::Change>,
}

impl Offer {
    /// Так предложение видит окно: к записи добавляется имя специалиста.
    pub fn event_json(&self, specialist_name: &str) -> Value {
        json!({
            "id": self.id,
            "hostBotId": self.host_bot_id,
            "specialistId": self.specialist_id,
            "specialistName": specialist_name,
            "reason": self.reason,
            "task": self.task,
            "at": self.at,
        })
    }
}

/// Предложения по боту-хозяину, от старых к новым.
#[derive(Default)]
pub struct Offers {
    inner: Mutex<HashMap<String, VecDeque<Offer>>>,
    /// Решения по карточкам команды: инструмент `propose_team` ждёт здесь ответа человека.
    decisions: Mutex<HashMap<String, tokio::sync::watch::Sender<Option<String>>>>,
    /// Оркестратор собрал команду по «Создать»: его итог в конце этого хода уходит и в чат команды.
    team_summaries: Mutex<HashMap<String, String>>,
    path: Option<PathBuf>,
}

impl Offers {
    pub fn open(data_dir: &Path) -> Self {
        let path = data_dir.join("offers.json");
        let inner: HashMap<String, VecDeque<Offer>> = std::fs::read_to_string(&path).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default();
        Self {
            inner: Mutex::new(inner),
            decisions: Mutex::new(HashMap::new()),
            team_summaries: Mutex::new(HashMap::new()),
            path: Some(path),
        }
    }

    fn save(&self) {
        let Some(path) = &self.path else { return; };
        let inner = self.inner.lock().unwrap();
        match serde_json::to_string(&*inner) {
            Ok(text) => {
                let tmp = path.with_extension("json.tmp");
                if std::fs::write(&tmp, text).and_then(|_| std::fs::rename(&tmp, path)).is_err() {
                    eprintln!("[offers] не смог записать offers.json");
                }
            }
            Err(e) => eprintln!("[offers] не смог сериализовать offers.json: {e}"),
        }
    }

    pub fn add(&self, offer: Offer) {
        let mut evicted = Vec::new();
        {
            let mut inner = self.inner.lock().unwrap();
            let queue = inner.entry(offer.host_bot_id.clone()).or_default();
            queue.retain(|item| item.id != offer.id);
            queue.push_back(offer);
            while queue.len() > MAX_OFFERS_PER_BOT {
                evicted.extend(queue.pop_front());
            }
        }
        // Вытесненную карточку команды уже не принять: инструмент не должен висеть до таймаута.
        for old in evicted.iter().filter(|o| o.team.is_some() || o.change.is_some()) {
            self.decide(&old.id, "Предложение снято: появились новые, эта карточка больше не действует.".into());
        }
        self.save();
    }

    /// Уже висящая карточка команды этого бота с тем же составом: повторный вызов не плодит вторую.
    pub fn find_team(&self, host_bot_id: &str, plan: &crate::team_proposal::TeamPlan) -> Option<String> {
        let key = serde_json::to_string(plan).ok()?;
        let inner = self.inner.lock().unwrap();
        inner.get(host_bot_id)?.iter().find(|o| o.team.as_ref().and_then(|t| serde_json::to_string(t).ok()).as_deref() == Some(key.as_str())).map(|o| o.id.clone())
    }

    /// Уже висящая карточка изменения (бот, рутина) этого бота с тем же содержанием.
    pub fn find_change(&self, host_bot_id: &str, change: &crate::team_changes::Change) -> Option<String> {
        let key = change.dedupe_key();
        let inner = self.inner.lock().unwrap();
        inner.get(host_bot_id)?.iter().find(|o| o.change.as_ref().map(|c| c.dedupe_key()).as_deref() == Some(key.as_str())).map(|o| o.id.clone())
    }

    /// Нерешённые карточки этого бота в том же виде, в каком окно получает их событиями: окно, открытое позже
    /// (или перезагруженное), рисует их тем же кодом. Решённые отсюда уходят сами (`take`).
    pub fn pending_events(&self, host_bot_id: &str, specialist_name: impl Fn(&str) -> String) -> Vec<Value> {
        let inner = self.inner.lock().unwrap();
        let Some(queue) = inner.get(host_bot_id) else { return Vec::new() };
        queue
            .iter()
            .map(|offer| {
                if let Some(plan) = &offer.team {
                    json!({ "type": "team_proposal", "botId": host_bot_id, "proposal": plan.event_json(offer) })
                } else if let Some(change) = &offer.change {
                    json!({ "type": "team_proposal", "botId": host_bot_id, "proposal": change.event_json(offer) })
                } else {
                    json!({ "type": "specialist_offer", "botId": host_bot_id, "offer": offer.event_json(&specialist_name(&offer.specialist_id)) })
                }
            })
            .collect()
    }

    /// Начать ждать решения по карточке.
    pub fn open_decision(&self, offer_id: &str) -> tokio::sync::watch::Receiver<Option<String>> {
        let mut map = self.decisions.lock().unwrap();
        map.entry(offer_id.to_string()).or_insert_with(|| tokio::sync::watch::channel(None).0).subscribe()
    }

    /// Сообщить ждущему инструменту итог (создано или отказ) и закрыть ожидание.
    pub fn decide(&self, offer_id: &str, outcome: String) {
        if let Some(sender) = self.decisions.lock().unwrap().remove(offer_id) {
            let _ = sender.send(Some(outcome));
        }
        self.save();
    }

    /// Забрать предложение у его хозяина. Чужой id или чужой бот — `None`.
    pub fn take(&self, host_bot_id: &str, offer_id: &str) -> Option<Offer> {
        let mut inner = self.inner.lock().unwrap();
        let queue = inner.get_mut(host_bot_id)?;
        let found = queue.iter().position(|item| item.id == offer_id)?;
        let offer = queue.remove(found)?;
        if queue.is_empty() {
            inner.remove(host_bot_id);
        }
        drop(inner);
        self.save();
        Some(offer)
    }

    /// Итог хода этого бота продублировать в чат команды `room_id`.
    pub fn note_team_summary(&self, bot_id: &str, room_id: &str) {
        self.team_summaries.lock().unwrap().insert(bot_id.to_string(), room_id.to_string());
    }

    /// Команда, куда отдать итог хода, если в этом ходе её собрали; запись одноразовая.
    pub fn take_team_summary(&self, bot_id: &str) -> Option<String> {
        self.team_summaries.lock().unwrap().remove(bot_id)
    }
}

#[cfg(test)]
mod tests {
    use super::{Offer, Offers, MAX_OFFERS_PER_BOT};

    fn offer(host: &str, id: &str) -> Offer {
        Offer {
            id: id.into(),
            host_bot_id: host.into(),
            specialist_id: "spec".into(),
            reason: "нужен".into(),
            task: "почини".into(),
            at: 1,
            team: None,
            change: None,
        }
    }

    #[test]
    fn the_oldest_offer_is_evicted_and_only_the_host_can_take_it() {
        let offers = Offers::default();
        for i in 0..=MAX_OFFERS_PER_BOT {
            offers.add(offer("lead", &format!("offer_{i}")));
        }
        assert!(offers.take("lead", "offer_0").is_none(), "старое предложение вытеснено");
        assert!(offers.take("other", &format!("offer_{MAX_OFFERS_PER_BOT}")).is_none(), "чужой бот не забирает");
        let taken = offers.take("lead", &format!("offer_{MAX_OFFERS_PER_BOT}")).expect("новое предложение на месте");
        assert_eq!(taken.specialist_id, "spec");
        assert!(offers.take("lead", &format!("offer_{MAX_OFFERS_PER_BOT}")).is_none(), "второй раз уже нет");
    }

    #[tokio::test]
    async fn evicted_team_offer_wakes_the_waiter() {
        let offers = Offers::default();
        let plan = crate::team_proposal::TeamPlan {
            new_bots: vec![],
            catalog_bots: vec![],
            add_bots: vec![],
            room: None,
            lead: String::new(),
            access: vec![],
            access_key: String::new(),
            host_name: "Хозяин".into(),
            host_driver: "api".into(),
            host_model: "m".into(),
            host_extra: vec![],
            host_orchestrator: false,
        };
        let first = Offer { team: Some(plan), ..offer("lead", "offer_team") };
        let mut waiter = offers.open_decision("offer_team");
        offers.add(first);
        for i in 0..MAX_OFFERS_PER_BOT {
            offers.add(offer("lead", &format!("offer_{i}")));
        }
        let outcome = tokio::time::timeout(std::time::Duration::from_secs(2), waiter.wait_for(|v| v.is_some())).await.expect("ожидающий проснулся").unwrap().clone();
        assert!(outcome.unwrap().contains("снято"));
    }

    fn change_card(title: &str) -> crate::team_changes::Change {
        crate::team_changes::Change::Routine(crate::team_changes::RoutinePlan {
            host_name: "Хозяин".into(),
            target_id: "b".into(),
            target_name: "Б".into(),
            title: title.into(),
            prompt: "п".into(),
            input: serde_json::json!({ "at": "09:00" }),
            schedule: "каждый день в 09:00".into(),
            next: String::new(),
            free_mode: false,
            quiet: true,
            host_id: "h".into(),
        })
    }

    #[test]
    fn a_repeated_change_card_is_found_and_a_different_one_is_not() {
        let offers = Offers::default();
        offers.add(Offer { change: Some(change_card("Пост")), ..offer("lead", "offer_change") });
        assert_eq!(offers.find_change("lead", &change_card("Пост")).as_deref(), Some("offer_change"));
        assert!(offers.find_change("lead", &change_card("Другое")).is_none());
        assert!(offers.find_change("other", &change_card("Пост")).is_none(), "карточки чужого бота не видны");
    }

    #[tokio::test]
    async fn offers_survive_reopen_with_team_and_specialist() {
        let dir = std::env::temp_dir().join(format!("deloist-offers-reopen-{}", std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()));
        std::fs::create_dir_all(&dir).unwrap();
        let plan = crate::team_proposal::TeamPlan {
            new_bots: vec![],
            catalog_bots: vec![],
            add_bots: vec![],
            room: None,
            lead: String::new(),
            access: vec![],
            access_key: String::new(),
            host_name: "Хозяин".into(),
            host_driver: "api".into(),
            host_model: "m".into(),
            host_extra: vec![],
            host_orchestrator: false,
        };
        {
            let offers = Offers::open(&dir);
            offers.add(Offer { team: Some(plan.clone()), ..offer("lead", "offer_team") });
            offers.add(offer("lead", "offer_spec"));
        }
        let reopened = Offers::open(&dir);
        assert_eq!(reopened.find_team("lead", &plan).as_deref(), Some("offer_team"));
        let events = reopened.pending_events("lead", |_| "Spec".into());
        assert_eq!(events.len(), 2, "командная и специалистская карточки пережили перезапуск");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn evicted_change_offer_wakes_the_waiter() {
        let offers = Offers::default();
        let mut waiter = offers.open_decision("offer_change");
        offers.add(Offer { change: Some(change_card("Пост")), ..offer("lead", "offer_change") });
        for i in 0..MAX_OFFERS_PER_BOT {
            offers.add(offer("lead", &format!("offer_{i}")));
        }
        let outcome = tokio::time::timeout(std::time::Duration::from_secs(2), waiter.wait_for(|v| v.is_some())).await.expect("ожидающий проснулся").unwrap().clone();
        assert!(outcome.unwrap().contains("снято"));
    }
}
