//! Клиент Chrome DevTools Protocol: запросы с ответами и поток событий по одному WebSocket.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use futures_util::{SinkExt, StreamExt};
use serde_json::{json, Value};
use tokio::sync::{mpsc, oneshot};
use tokio_tungstenite::tungstenite::Message;
use tokio_util::sync::CancellationToken;

const CALL_TIMEOUT: Duration = Duration::from_secs(20);

type Pending = Arc<Mutex<HashMap<u64, oneshot::Sender<Result<Value, String>>>>>;

/// Ждущий ответа запрос: его запись убирается вместе с ним.
struct Waiting<'a> {
    pending: &'a Pending,
    id: u64,
}

impl Drop for Waiting<'_> {
    fn drop(&mut self) {
        if let Ok(mut pending) = self.pending.lock() {
            pending.remove(&self.id);
        }
    }
}

pub struct Cdp {
    out: mpsc::UnboundedSender<String>,
    pending: Pending,
    next: AtomicU64,
    closed: CancellationToken,
}

impl Cdp {
    /// Подключение и приёмник событий. События идут строго в порядке прихода.
    pub async fn connect(url: &str) -> Result<(Arc<Self>, mpsc::UnboundedReceiver<Value>), String> {
        let (socket, _) = tokio_tungstenite::connect_async(url).await.map_err(|e| format!("браузер не отвечает: {e}"))?;
        let (mut sink, mut stream) = socket.split();
        let (out, mut out_rx) = mpsc::unbounded_channel::<String>();
        let (in_tx, in_rx) = mpsc::unbounded_channel::<String>();

        tokio::spawn(async move {
            while let Some(text) = out_rx.recv().await {
                if sink.send(Message::text(text)).await.is_err() {
                    break;
                }
            }
        });
        tokio::spawn(async move {
            while let Some(Ok(message)) = stream.next().await {
                let Ok(text) = message.to_text() else { continue };
                if in_tx.send(text.to_string()).is_err() {
                    break;
                }
            }
        });
        Ok(Self::from_channels(out, in_rx))
    }

    /// Тот же клиент поверх любых двух каналов: исходящие сообщения CDP и входящие ответы и события.
    /// Так «мой Chrome» (`user_chrome.rs`) говорит с расширением через фильтр, а не через WebSocket.
    /// Клиент закрыт, когда закрыт входящий канал.
    pub fn from_channels(out: mpsc::UnboundedSender<String>, mut incoming: mpsc::UnboundedReceiver<String>) -> (Arc<Self>, mpsc::UnboundedReceiver<Value>) {
        let (events_tx, events_rx) = mpsc::unbounded_channel::<Value>();
        let pending: Pending = Arc::new(Mutex::new(HashMap::new()));
        let closed = CancellationToken::new();
        {
            let pending = pending.clone();
            let closed = closed.clone();
            tokio::spawn(async move {
                while let Some(text) = incoming.recv().await {
                    let Ok(value) = serde_json::from_str::<Value>(&text) else { continue };
                    let Some(id) = value.get("id").and_then(Value::as_u64) else {
                        let _ = events_tx.send(value);
                        continue;
                    };
                    let Some(reply) = pending.lock().unwrap().remove(&id) else { continue };
                    let answer = match value.get("error") {
                        Some(error) => Err(error["message"].as_str().unwrap_or("ошибка браузера").to_string()),
                        None => Ok(value.get("result").cloned().unwrap_or_else(|| json!({}))),
                    };
                    let _ = reply.send(answer);
                }
                closed.cancel();
                pending.lock().unwrap().clear();
            });
        }
        (Arc::new(Self { out, pending, next: AtomicU64::new(1), closed }), events_rx)
    }

    pub fn is_closed(&self) -> bool {
        self.closed.is_cancelled()
    }

    /// `session` — вкладка; без него запрос идёт самому браузеру.
    pub async fn call(&self, session: Option<&str>, method: &str, params: Value) -> Result<Value, String> {
        if self.is_closed() {
            return Err("браузер закрыт".into());
        }
        let id = self.next.fetch_add(1, Ordering::Relaxed);
        let mut message = json!({ "id": id, "method": method, "params": params });
        if let Some(session) = session {
            message["sessionId"] = json!(session);
        }
        let (tx, rx) = oneshot::channel();
        self.pending.lock().unwrap().insert(id, tx);
        // Запись уходит при любом выходе, и при отмене: трансляция экрана бросает зависшие запросы, и раньше
        // они копились в таблице до закрытия соединения (аудит 19.11.2026: 256 отменённых — 256 записей).
        let _waiting = Waiting { pending: &self.pending, id };
        if self.out.send(message.to_string()).is_err() {
            return Err("браузер закрыт".into());
        }
        match tokio::time::timeout(CALL_TIMEOUT, rx).await {
            Ok(Ok(result)) => result,
            Ok(Err(_)) => Err("браузер закрыт".into()),
            Err(_) => Err(format!("браузер не ответил на {method} за {} с: страница могла зависнуть или стоять на точке останова", CALL_TIMEOUT.as_secs())),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn cancelled_calls_leave_no_pending_entries() {
        let (out, _sent) = mpsc::unbounded_channel();
        let (_incoming_tx, incoming) = mpsc::unbounded_channel();
        let (cdp, _events) = Cdp::from_channels(out, incoming);
        for _ in 0..256 {
            // Ответа нет: запрос бросают раньше срока, как трансляция экрана при зависшей странице.
            let _ = tokio::time::timeout(Duration::from_millis(1), cdp.call(None, "Page.captureScreenshot", json!({}))).await;
        }
        assert_eq!(cdp.pending.lock().unwrap().len(), 0);
    }
}
