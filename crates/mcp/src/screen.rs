//! The browser's screen for the chat window: frames go out to every window that connected with
//! the token, and the owner's mouse and keys come back while they hold the browser. One JSON
//! object per line, like the app channel.

use std::sync::Arc;
use std::sync::Mutex;
use std::sync::atomic::AtomicUsize;
use std::sync::atomic::Ordering;
use std::time::Duration;

use serde_json::Value;
use serde_json::json;
use tokio::io::AsyncBufReadExt;
use tokio::io::AsyncReadExt;
use tokio::io::AsyncWriteExt;
use tokio::io::BufReader;
use tokio::net::TcpListener;
use tokio::net::TcpStream;
use tokio::sync::broadcast;
use tokio::sync::mpsc;
use tokio::sync::watch;
use tokio::time;

const HELLO_WAIT: Duration = Duration::from_secs(5);
const LINE: usize = 64 * 1024;
const KEPT: usize = 8;
const WINDOWS: usize = 8;

/// What the windows ask of the browser.
#[derive(Clone, Debug, PartialEq)]
pub enum Ask {
    Watch(bool),
    Take,
    Give,
    Input(Value),
}

/// The meeting point between the browser and the chat windows that show it.
pub struct Screen {
    out: broadcast::Sender<Arc<str>>,
    pub(crate) state: Mutex<Value>,
    states: watch::Sender<Arc<str>>,
    asks: mpsc::UnboundedSender<Ask>,
    watching: AtomicUsize,
}

impl std::fmt::Debug for Screen {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Screen").finish_non_exhaustive()
    }
}

impl Screen {
    /// The screen, and where the browser hears what windows ask of it.
    pub fn new() -> (Arc<Self>, mpsc::UnboundedReceiver<Ask>) {
        let (out, _) = broadcast::channel(KEPT);
        let (asks, heard) = mpsc::unbounded_channel();
        let state = json!({ "kind": "state", "mode": "idle" });
        let screen = Arc::new(Self {
            out,
            states: watch::Sender::new(Arc::from(state.to_string())),
            state: Mutex::new(state),
            asks,
            watching: AtomicUsize::new(0),
        });
        (screen, heard)
    }

    /// Whether any window shows the screen now.
    pub(crate) fn watched(&self) -> bool {
        self.watching.load(Ordering::SeqCst) > 0
    }

    pub(crate) fn frame(&self, jpeg: &str, width: f64, height: f64) {
        let line = json!({ "kind": "frame", "jpeg": jpeg, "width": width, "height": height });
        let _ = self.out.send(Arc::from(line.to_string()));
    }

    pub(crate) fn pointer(&self, x: f64, y: f64) {
        let line = json!({ "kind": "pointer", "x": x, "y": y });
        let _ = self.out.send(Arc::from(line.to_string()));
    }

    /// Changes some fields of the state every window sees, such as who holds the browser.
    pub(crate) fn update(&self, fields: Value) {
        let line = {
            let mut state = self.state.lock().expect("unpoisoned");
            if let (Some(state), Some(fields)) = (state.as_object_mut(), fields.as_object()) {
                for (key, value) in fields {
                    state.insert(key.clone(), value.clone());
                }
            }
            state.to_string()
        };
        let line: Arc<str> = Arc::from(line);
        self.states.send_replace(Arc::clone(&line));
        let _ = self.out.send(line);
    }

    /// Serves windows on `listener` until the process ends. A window proves itself with `token`.
    pub async fn serve(self: Arc<Self>, listener: TcpListener, token: String) {
        let token: Arc<str> = Arc::from(token);
        loop {
            let Ok((stream, _)) = listener.accept().await else {
                time::sleep(Duration::from_millis(50)).await;
                continue;
            };
            if self.watching.load(Ordering::SeqCst) >= WINDOWS {
                continue;
            }
            tokio::spawn(Arc::clone(&self).window(stream, Arc::clone(&token)));
        }
    }

    async fn window(self: Arc<Self>, stream: TcpStream, token: Arc<str>) {
        let (read, mut write) = stream.into_split();
        let mut lines = BufReader::new(read);
        let mut first = String::new();
        let greeted = time::timeout(HELLO_WAIT, read_line(&mut lines, &mut first)).await;
        let hello: Option<Value> = serde_json::from_str(first.trim()).ok();
        let proved = matches!(greeted, Ok(Ok(true)))
            && hello
                .as_ref()
                .and_then(|hello| hello.get("token"))
                .and_then(Value::as_str)
                == Some(&*token);
        if !proved {
            return;
        }
        if hello.as_ref().and_then(|hello| hello.get("quiet")) == Some(&Value::Bool(true)) {
            self.listen(lines, write).await;
            return;
        }
        let mut out = self.out.subscribe();
        let state = self.state.lock().expect("unpoisoned").to_string();
        if write
            .write_all(format!("{state}\n").as_bytes())
            .await
            .is_err()
        {
            return;
        }
        if self.watching.fetch_add(1, Ordering::SeqCst) == 0 {
            let _ = self.asks.send(Ask::Watch(true));
        }
        let asks = self.asks.clone();
        let mut heard = tokio::spawn(async move {
            let mut line = String::new();
            while let Ok(true) = read_line(&mut lines, &mut line).await {
                if let Some(ask) = ask_of(line.trim()) {
                    let _ = asks.send(ask);
                }
            }
        });
        loop {
            tokio::select! {
                sent = out.recv() => match sent {
                    Ok(sent) => {
                        if write.write_all(sent.as_bytes()).await.is_err() || write.write_all(b"\n").await.is_err() {
                            break;
                        }
                    }
                    Err(broadcast::error::RecvError::Lagged(_)) => {}
                    Err(broadcast::error::RecvError::Closed) => break,
                },
                _ = &mut heard => break,
            }
        }
        heard.abort();
        if self.watching.fetch_sub(1, Ordering::SeqCst) == 1 {
            let _ = self.asks.send(Ask::Watch(false));
        }
    }

    async fn listen(
        self: Arc<Self>,
        mut lines: BufReader<tokio::net::tcp::OwnedReadHalf>,
        mut write: tokio::net::tcp::OwnedWriteHalf,
    ) {
        let mut states = self.states.subscribe();
        let asks = self.asks.clone();
        let mut heard = tokio::spawn(async move {
            let mut line = String::new();
            while let Ok(true) = read_line(&mut lines, &mut line).await {
                if let Some(allowed) = allowed(line.trim()) {
                    let _ = asks.send(Ask::Input(allowed));
                }
            }
        });
        loop {
            let state = states.borrow_and_update().clone();
            if write.write_all(state.as_bytes()).await.is_err()
                || write.write_all(b"\n").await.is_err()
            {
                break;
            }
            tokio::select! {
                changed = states.changed() => if changed.is_err() {
                    break;
                },
                _ = &mut heard => break,
            }
        }
        heard.abort();
    }
}

async fn read_line(
    lines: &mut BufReader<tokio::net::tcp::OwnedReadHalf>,
    line: &mut String,
) -> std::io::Result<bool> {
    line.clear();
    let mut bytes = Vec::new();
    let read = (&mut *lines)
        .take(LINE as u64)
        .read_until(b'\n', &mut bytes)
        .await?;
    if read == 0 || (read == LINE && !bytes.ends_with(b"\n")) {
        return Ok(false);
    }
    *line = String::from_utf8_lossy(&bytes).into_owned();
    Ok(true)
}

/// The answer to a log-in card in a listener's line, if it is one: whether to log in, with which
/// account, and whether to stop asking. A listener never types into a card.
pub(crate) fn allowed(line: &str) -> Option<Value> {
    let value: Value = serde_json::from_str(line).ok()?;
    if value.get("kind").and_then(Value::as_str) != Some("fill") {
        return None;
    }
    Some(json!({
        "kind": "fill",
        "id": value.get("id").and_then(Value::as_u64)?,
        "allow": value.get("allow").and_then(Value::as_bool)?,
        "trust": value.get("trust").and_then(Value::as_bool).unwrap_or(false),
        "account": value.get("account").and_then(Value::as_u64).unwrap_or(0),
    }))
}

/// What a window's line asks for, if it is something the browser does.
pub(crate) fn ask_of(line: &str) -> Option<Ask> {
    let value: Value = serde_json::from_str(line).ok()?;
    match value.get("kind").and_then(Value::as_str)? {
        "take" => Some(Ask::Take),
        "give" => Some(Ask::Give),
        "mouse" | "wheel" | "key" | "text" | "compose" | "pick" | "auth" | "fill" => {
            Some(Ask::Input(value))
        }
        _ => None,
    }
}

#[cfg(test)]
#[path = "screen.test.rs"]
mod tests;
