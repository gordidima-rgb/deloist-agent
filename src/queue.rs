//! Очередь сообщений бота: человек может писать, править и отправлять «сейчас», пока бот занят.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Mutex;

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tokio::sync::{broadcast, Mutex as AsyncMutex};

use crate::store::{now, uid};

pub const MAX_ITEMS: usize = 50;

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PauseReason {
    Manual,
    Restart,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct BotQueue {
    pub paused: bool,
    pub pause_reason: Option<PauseReason>,
    pub items: Vec<Value>,
}

#[derive(Debug)]
pub enum QueueError {
    Full,
    NotFound,
    Io(String),
}

pub struct Queues {
    path: PathBuf,
    inner: Mutex<HashMap<String, BotQueue>>,
    /// Сообщения «сейчас»: их вставит в текущий ход перед следующим запросом к модели.
    steers: Mutex<HashMap<String, Vec<Value>>>,
    /// Запись файла строго по очереди: старый снимок не должен затереть новый.
    gate: AsyncMutex<()>,
    events: broadcast::Sender<Value>,
}

impl Queues {
    pub async fn open(data_dir: PathBuf, events: broadcast::Sender<Value>) -> Self {
        let path = data_dir.join("queue.json");
        let mut loaded: HashMap<String, BotQueue> = match tokio::fs::read_to_string(&path).await {
            Ok(text) => serde_json::from_str(&text).unwrap_or_default(),
            Err(_) => HashMap::new(),
        };
        // После перезапуска очередь стоит: старые сообщения не должны поехать сами ночью.
        for queue in loaded.values_mut() {
            if !queue.paused || queue.pause_reason != Some(PauseReason::Manual) {
                queue.pause_reason = Some(PauseReason::Restart);
            }
            queue.paused = true;
        }
        let this = Self {
            path,
            inner: Mutex::new(loaded),
            steers: Mutex::new(HashMap::new()),
            gate: AsyncMutex::new(()),
            events,
        };
        let has_items = this.inner.lock().unwrap().values().any(|q| !q.items.is_empty());
        if has_items {
            let _ = this.save().await;
        }
        this
    }

    async fn save(&self) -> Result<(), QueueError> {
        let text = serde_json::to_string_pretty(&*self.inner.lock().unwrap())
            .map_err(|e| QueueError::Io(e.to_string()))?;
        let tmp = self.path.with_extension("json.tmp");
        tokio::fs::write(&tmp, text).await.map_err(|e| QueueError::Io(e.to_string()))?;
        tokio::fs::rename(&tmp, &self.path).await.map_err(|e| QueueError::Io(e.to_string()))
    }

    fn emit(&self, bot_id: &str, state: &BotQueue) {
        let _ = self.events.send(json!({
            "type": "queue_updated",
            "botId": bot_id,
            "paused": state.paused,
            "pauseReason": state.pause_reason,
            "items": state.items.clone(),
            "at": now(),
        }));
    }

    pub fn get(&self, bot_id: &str) -> BotQueue {
        self.inner.lock().unwrap().get(bot_id).cloned().unwrap_or_default()
    }

    pub fn count(&self, bot_id: &str) -> usize {
        self.inner.lock().unwrap().get(bot_id).map_or(0, |q| q.items.len())
    }

    pub fn is_paused(&self, bot_id: &str) -> bool {
        self.inner.lock().unwrap().get(bot_id).is_some_and(|q| q.paused)
    }

    pub fn front(&self, bot_id: &str) -> Option<Value> {
        self.inner.lock().unwrap().get(bot_id)?.items.first().cloned()
    }

    pub fn item(&self, bot_id: &str, qid: &str) -> Option<Value> {
        self.inner
            .lock()
            .unwrap()
            .get(bot_id)?
            .items
            .iter()
            .find(|item| item["id"].as_str() == Some(qid))
            .cloned()
    }

    pub async fn add(&self, bot_id: &str, text: &str) -> Result<Value, QueueError> {
        self.add_with(bot_id, text, Vec::new(), false, false, None).await
    }

    /// Сообщение с вложениями: файлы уже лежат в воркспейсе, в очереди хранится их описание.
    /// `window` — сообщение пришло из окна приложения: его ход станет разговором с человеком.
    /// `source` — откуда пришло сообщение (например, `telegram`): ход сохранит эту отметку.
    pub async fn add_with(
        &self,
        bot_id: &str,
        text: &str,
        attachments: Vec<Value>,
        window: bool,
        voice: bool,
        source: Option<&str>,
    ) -> Result<Value, QueueError> {
        let _gate = self.gate.lock().await;
        let (item, state) = {
            let mut map = self.inner.lock().unwrap();
            let queue = map.entry(bot_id.to_string()).or_default();
            if queue.items.len() >= MAX_ITEMS {
                return Err(QueueError::Full);
            }
            let mut item = json!({ "id": uid("q"), "text": text, "at": now() });
            if !attachments.is_empty() {
                item["attachments"] = Value::Array(attachments);
            }
            if window {
                item["window"] = json!(true);
            }
            if voice {
                item["voice"] = json!(true);
            }
            if let Some(source) = source.filter(|s| !s.is_empty()) {
                item["source"] = json!(source);
            }
            queue.items.push(item.clone());
            // Новое сообщение снимает паузу: человек снова ждёт ответа.
            queue.paused = false;
            queue.pause_reason = None;
            (item, queue.clone())
        };
        self.save().await?;
        self.emit(bot_id, &state);
        Ok(item)
    }

    pub async fn patch(&self, bot_id: &str, qid: &str, text: &str) -> Result<Value, QueueError> {
        let _gate = self.gate.lock().await;
        let (item, state) = {
            let mut map = self.inner.lock().unwrap();
            let Some(queue) = map.get_mut(bot_id) else {
                return Err(QueueError::NotFound);
            };
            let Some(item) = queue.items.iter_mut().find(|item| item["id"].as_str() == Some(qid)) else {
                return Err(QueueError::NotFound);
            };
            item["text"] = json!(text);
            (item.clone(), queue.clone())
        };
        self.save().await?;
        self.emit(bot_id, &state);
        Ok(item)
    }

    /// Переставить элемент перед `before` (или в конец, если `None`).
    pub async fn move_item(
        &self,
        bot_id: &str,
        qid: &str,
        before: Option<&str>,
    ) -> Result<BotQueue, QueueError> {
        let _gate = self.gate.lock().await;
        let state = {
            let mut map = self.inner.lock().unwrap();
            let Some(queue) = map.get_mut(bot_id) else {
                return Err(QueueError::NotFound);
            };
            let Some(index) = queue.items.iter().position(|item| item["id"].as_str() == Some(qid)) else {
                return Err(QueueError::NotFound);
            };
            // Оба идентификатора проверяем до перестановки: при ошибке список не меняем.
            if let Some(before) = before {
                if !queue.items.iter().any(|item| item["id"].as_str() == Some(before)) {
                    return Err(QueueError::NotFound);
                }
                if before == qid {
                    return Ok(queue.clone());
                }
            }
            let item = queue.items.remove(index);
            // После удаления позиция `before` сдвигается — ищем её заново.
            let target = before
                .and_then(|before| queue.items.iter().position(|item| item["id"].as_str() == Some(before)))
                .unwrap_or(queue.items.len());
            queue.items.insert(target, item);
            queue.clone()
        };
        self.save().await?;
        self.emit(bot_id, &state);
        Ok(state)
    }

    pub async fn remove_item(&self, bot_id: &str, qid: &str) -> Result<(), QueueError> {
        let _gate = self.gate.lock().await;
        let state = {
            let mut map = self.inner.lock().unwrap();
            let Some(queue) = map.get_mut(bot_id) else {
                return Err(QueueError::NotFound);
            };
            let Some(index) = queue.items.iter().position(|item| item["id"].as_str() == Some(qid)) else {
                return Err(QueueError::NotFound);
            };
            queue.items.remove(index);
            queue.clone()
        };
        self.save().await?;
        self.emit(bot_id, &state);
        Ok(())
    }

    /// Поставить сообщение в начало: «сейчас» не успело попасть в ход, но терять его нельзя.
    pub async fn prepend(&self, bot_id: &str, items: Vec<Value>) -> Result<(), QueueError> {
        if items.is_empty() {
            return Ok(());
        }
        let _gate = self.gate.lock().await;
        let state = {
            let mut map = self.inner.lock().unwrap();
            let queue = map.entry(bot_id.to_string()).or_default();
            let mut list = items;
            list.append(&mut queue.items);
            queue.items = list;
            queue.clone()
        };
        self.save().await?;
        self.emit(bot_id, &state);
        Ok(())
    }

    pub async fn pause(&self, bot_id: &str) -> Result<(), QueueError> {
        let _gate = self.gate.lock().await;
        let state = {
            let mut map = self.inner.lock().unwrap();
            let Some(queue) = map.get_mut(bot_id) else {
                return Ok(());
            };
            if queue.paused && queue.pause_reason == Some(PauseReason::Manual) {
                return Ok(());
            }
            queue.paused = true;
            queue.pause_reason = Some(PauseReason::Manual);
            queue.clone()
        };
        self.save().await?;
        self.emit(bot_id, &state);
        Ok(())
    }

    pub async fn resume(&self, bot_id: &str) -> Result<(), QueueError> {
        let _gate = self.gate.lock().await;
        let state = {
            let mut map = self.inner.lock().unwrap();
            let Some(queue) = map.get_mut(bot_id) else {
                return Ok(());
            };
            if !queue.paused {
                return Ok(());
            }
            queue.paused = false;
            queue.pause_reason = None;
            queue.clone()
        };
        self.save().await?;
        self.emit(bot_id, &state);
        Ok(())
    }

    /// Забрать элемент в текущий ход. Ход сам вставит его перед следующим запросом к модели.
    pub async fn steer(&self, bot_id: &str, qid: &str) -> Result<Value, QueueError> {
        let _gate = self.gate.lock().await;
        let (item, state) = {
            let mut map = self.inner.lock().unwrap();
            let Some(queue) = map.get_mut(bot_id) else {
                return Err(QueueError::NotFound);
            };
            let Some(index) = queue.items.iter().position(|item| item["id"].as_str() == Some(qid)) else {
                return Err(QueueError::NotFound);
            };
            let item = queue.items.remove(index);
            self.steers
                .lock()
                .unwrap()
                .entry(bot_id.to_string())
                .or_default()
                .push(item.clone());
            (item, queue.clone())
        };
        self.save().await?;
        self.emit(bot_id, &state);
        Ok(item)
    }

    pub fn take_steers(&self, bot_id: &str) -> Vec<Value> {
        self.steers.lock().unwrap().remove(bot_id).unwrap_or_default()
    }

    /// Бота удалили — его очередь больше не нужна.
    pub async fn clear_bot(&self, bot_id: &str) -> Result<(), QueueError> {
        let _gate = self.gate.lock().await;
        let was = self.inner.lock().unwrap().remove(bot_id).is_some();
        self.steers.lock().unwrap().remove(bot_id);
        if !was {
            return Ok(());
        }
        self.save().await?;
        self.emit(bot_id, &BotQueue::default());
        Ok(())
    }
}
