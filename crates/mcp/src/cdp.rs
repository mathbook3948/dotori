use std::collections::HashMap;
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::atomic::AtomicU64;
use std::sync::atomic::Ordering;
use std::time::Duration;

use futures_util::SinkExt;
use futures_util::StreamExt;
use futures_util::stream::SplitSink;
use serde_json::Value;
use serde_json::json;
use tokio::net::TcpStream;
use tokio::sync::oneshot;
use tokio::task::JoinHandle;
use tokio::time;
use tokio_tungstenite::MaybeTlsStream;
use tokio_tungstenite::WebSocketStream;
use tokio_tungstenite::tungstenite::Message;

const ANSWER_WAIT: Duration = Duration::from_secs(30);

type Socket = WebSocketStream<MaybeTlsStream<TcpStream>>;
type Waiting = Arc<Mutex<Option<HashMap<u64, oneshot::Sender<Result<Value, String>>>>>>;

/// A Chrome DevTools Protocol connection to the browser. Commands for a page carry its session.
pub(crate) struct Cdp {
    sink: tokio::sync::Mutex<SplitSink<Socket, Message>>,
    waiting: Waiting,
    next: AtomicU64,
    reader: JoinHandle<()>,
}

impl Cdp {
    pub(crate) async fn connect(url: &str) -> Result<Self, String> {
        let (socket, _) = tokio_tungstenite::connect_async(url)
            .await
            .map_err(|error| format!("cannot reach the browser: {error}"))?;
        let (sink, mut stream) = socket.split();
        let waiting: Waiting = Arc::new(Mutex::new(Some(HashMap::new())));
        let answers = Arc::clone(&waiting);
        let reader = tokio::spawn(async move {
            while let Some(Ok(frame)) = stream.next().await {
                let Message::Text(text) = frame else {
                    continue;
                };
                let Ok(message) = serde_json::from_str::<Value>(text.as_str()) else {
                    continue;
                };
                let Some(id) = message.get("id").and_then(Value::as_u64) else {
                    continue;
                };
                let Some(waiter) = answers
                    .lock()
                    .expect("unpoisoned")
                    .as_mut()
                    .and_then(|waiting| waiting.remove(&id))
                else {
                    continue;
                };
                let answer = match message.get("error") {
                    Some(error) => Err(error
                        .get("message")
                        .and_then(Value::as_str)
                        .unwrap_or("the browser refused")
                        .to_owned()),
                    None => Ok(message.get("result").cloned().unwrap_or(Value::Null)),
                };
                let _ = waiter.send(answer);
            }
            answers.lock().expect("unpoisoned").take();
        });
        Ok(Self {
            sink: tokio::sync::Mutex::new(sink),
            waiting,
            next: AtomicU64::new(1),
            reader,
        })
    }

    /// Whether the connection to the browser still stands.
    pub(crate) fn open(&self) -> bool {
        !self.reader.is_finished()
    }

    pub(crate) async fn send(
        &self,
        method: &str,
        params: Value,
        session: Option<&str>,
    ) -> Result<Value, String> {
        let id = self.next.fetch_add(1, Ordering::Relaxed);
        let mut message = json!({ "id": id, "method": method, "params": params });
        if let Some(session) = session {
            message["sessionId"] = json!(session);
        }
        let (tx, rx) = oneshot::channel();
        match self.waiting.lock().expect("unpoisoned").as_mut() {
            Some(waiting) => waiting.insert(id, tx),
            None => return Err(String::from("the browser connection closed")),
        };
        let sent = self
            .sink
            .lock()
            .await
            .send(Message::text(message.to_string()))
            .await;
        if let Err(error) = sent {
            self.forget(id);
            return Err(format!("the browser connection failed: {error}"));
        }
        match time::timeout(ANSWER_WAIT, rx).await {
            Ok(Ok(answer)) => answer.map_err(|error| format!("{method}: {error}")),
            Ok(Err(_)) => Err(String::from("the browser connection closed")),
            Err(_) => {
                self.forget(id);
                Err(format!("the browser did not answer {method} in time"))
            }
        }
    }
}

impl Cdp {
    fn forget(&self, id: u64) {
        if let Some(waiting) = self.waiting.lock().expect("unpoisoned").as_mut() {
            waiting.remove(&id);
        }
    }
}

impl Drop for Cdp {
    fn drop(&mut self) {
        self.reader.abort();
    }
}
