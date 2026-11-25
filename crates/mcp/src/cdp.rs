use std::collections::HashMap;
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::atomic::AtomicU64;
use std::sync::atomic::Ordering;
use std::time::Duration;

use futures_util::SinkExt;
use futures_util::Stream;
use futures_util::StreamExt;
use serde_json::Value;
use serde_json::json;
use tokio::io::AsyncBufReadExt;
use tokio::io::AsyncRead;
use tokio::io::AsyncReadExt;
use tokio::io::AsyncWrite;
use tokio::io::AsyncWriteExt;
use tokio::io::BufReader;
use tokio::sync::mpsc;
use tokio::sync::oneshot;
use tokio::task::JoinHandle;
use tokio::time;
use tokio_tungstenite::tungstenite::Message;

const ANSWER_WAIT: Duration = Duration::from_secs(30);
const MESSAGE: u64 = 256 * 1024 * 1024;

type Waiting = Arc<Mutex<Option<HashMap<u64, oneshot::Sender<Result<Value, String>>>>>>;

/// A Chrome DevTools Protocol connection to the browser. Commands for a page carry its session.
/// Events the browser sends on its own wait in one queue, none dropped, for whoever takes it.
pub(crate) struct Cdp {
    outgoing: mpsc::UnboundedSender<String>,
    waiting: Waiting,
    next: AtomicU64,
    events: Mutex<Option<mpsc::UnboundedReceiver<Value>>>,
    reader: JoinHandle<()>,
    writer: JoinHandle<()>,
}

impl Cdp {
    /// Connects to a browser that listens for DevTools on a local WebSocket.
    pub(crate) async fn connect(url: &str) -> Result<Self, String> {
        let (socket, _) = tokio_tungstenite::connect_async(url)
            .await
            .map_err(|error| format!("cannot reach the browser: {error}"))?;
        let (mut sink, stream) = socket.split();
        let incoming = stream
            .take_while(|frame| std::future::ready(frame.is_ok()))
            .filter_map(|frame| {
                std::future::ready(match frame {
                    Ok(Message::Text(text)) => Some(text.as_str().to_owned()),
                    _ => None,
                })
            });
        Ok(Self::over(incoming, move |mut outgoing| async move {
            while let Some(text) = outgoing.recv().await {
                if sink.send(Message::text(text)).await.is_err() {
                    return;
                }
            }
        }))
    }

    /// Talks to a browser started with `--remote-debugging-pipe` over the two ends of its pipe:
    /// what it writes and what it reads. Each message ends with a NUL byte.
    pub(crate) fn pipe(
        from: impl AsyncRead + Unpin + Send + 'static,
        mut to: impl AsyncWrite + Unpin + Send + 'static,
    ) -> Self {
        let incoming = futures_util::stream::unfold(BufReader::new(from), |mut from| async move {
            let mut message = Vec::new();
            match (&mut from).take(MESSAGE).read_until(0, &mut message).await {
                Ok(0) | Err(_) => None,
                Ok(_) if message.last() != Some(&0) => None,
                Ok(_) => {
                    message.pop();
                    Some((String::from_utf8_lossy(&message).into_owned(), from))
                }
            }
        });
        Self::over(incoming, move |mut outgoing| async move {
            while let Some(text) = outgoing.recv().await {
                let mut message = text.into_bytes();
                message.push(0);
                if to.write_all(&message).await.is_err() || to.flush().await.is_err() {
                    return;
                }
            }
        })
    }

    fn over<W>(
        incoming: impl Stream<Item = String> + Send + 'static,
        write: impl FnOnce(mpsc::UnboundedReceiver<String>) -> W,
    ) -> Self
    where
        W: Future<Output = ()> + Send + 'static,
    {
        let waiting: Waiting = Arc::new(Mutex::new(Some(HashMap::new())));
        let answers = Arc::clone(&waiting);
        let (told, events) = mpsc::unbounded_channel();
        let reader = tokio::spawn(async move {
            let mut incoming = std::pin::pin!(incoming);
            while let Some(text) = incoming.next().await {
                let Ok(message) = serde_json::from_str::<Value>(&text) else {
                    continue;
                };
                let Some(id) = message.get("id").and_then(Value::as_u64) else {
                    if message.get("method").is_some() {
                        let _ = told.send(message);
                    }
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
        let (outgoing, sent) = mpsc::unbounded_channel();
        let unanswered = Arc::clone(&waiting);
        let writing = write(sent);
        let writer = tokio::spawn(async move {
            writing.await;
            unanswered.lock().expect("unpoisoned").take();
        });
        Self {
            outgoing,
            waiting,
            next: AtomicU64::new(1),
            events: Mutex::new(Some(events)),
            reader,
            writer,
        }
    }

    /// The events the browser sends, such as a dialog opening or a request waiting to go out.
    /// Only the first caller gets them.
    pub(crate) fn events(&self) -> Option<mpsc::UnboundedReceiver<Value>> {
        self.events.lock().expect("unpoisoned").take()
    }

    /// Whether the connection to the browser still stands.
    pub(crate) fn open(&self) -> bool {
        !self.reader.is_finished() && !self.writer.is_finished()
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
        if self.outgoing.send(message.to_string()).is_err() {
            self.forget(id);
            return Err(String::from("the browser connection closed"));
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
        self.writer.abort();
    }
}

#[cfg(test)]
#[path = "cdp.test.rs"]
mod tests;
