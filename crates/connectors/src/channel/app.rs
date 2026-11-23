use std::collections::VecDeque;
use std::fmt;
use std::io;
use std::net::SocketAddr;
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::atomic::AtomicU64;
use std::sync::atomic::Ordering;
use std::time::Duration;
use std::time::SystemTime;
use std::time::UNIX_EPOCH;

use dotori_core::BoxFuture;
use dotori_core::channel::AppWindow;
use dotori_core::channel::Channel;
use dotori_core::channel::ChannelError;
use dotori_core::channel::Inbox;
use dotori_core::channel::Traffic;
use dotori_core::id::ChannelId;
use dotori_core::id::ExternalMessageId;
use dotori_core::id::ExternalUserId;
use dotori_core::message::Audience;
use dotori_core::message::Content;
use dotori_core::message::Incoming;
use dotori_core::message::Place;
use serde::Deserialize;
use serde::Serialize;
use tokio::io::AsyncBufReadExt;
use tokio::io::AsyncReadExt;
use tokio::io::AsyncWriteExt;
use tokio::io::BufReader;
use tokio::net::TcpListener;
use tokio::net::TcpStream;
use tokio::net::tcp::OwnedReadHalf;
use tokio::sync::broadcast;
use tokio::sync::broadcast::error::RecvError;
use tokio::task::JoinSet;
use tokio::time;

/// The channel id of the app. Everyone who reaches it is the owner.
pub const APP: &str = "app";
/// The only place in the app. Every window shows the same conversation.
pub const MAIN: &str = "main";
const OWNER: &str = "owner";
const BACKLOG: usize = 500;
const LIVE: usize = 1024;
const MAX_LINE: u64 = 256 * 1024;
const HELLO_WAIT: Duration = Duration::from_secs(5);

/// The first line a window sends. Without the right token the daemon hangs up.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Hello {
    pub token: String,
}

/// What a window sends after its hello, one JSON object per line.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum FromWindow {
    Say { text: String },
}

/// What the daemon sends to every window, one JSON object per line.
/// A window that connects first gets the recent backlog, then everything live.
/// A window that falls too far behind is hung up on, so it reconnects and reads the backlog.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ToWindow {
    /// What the owner wrote in some window.
    Owner { text: String },
    /// What dotori answered in the app.
    Reply { text: String },
    /// A message or reply on another channel, such as Slack.
    Traffic {
        channel: String,
        place: String,
        who: String,
        text: String,
    },
}

/// The app channel on the daemon's side. Windows connect over local TCP and prove themselves
/// with a token the daemon writes where only the user can read it.
pub struct AppChannel {
    id: ChannelId,
    token: String,
    listener: Mutex<Option<std::net::TcpListener>>,
    hub: Arc<Hub>,
}

#[derive(Debug)]
struct Hub {
    backlog: Mutex<VecDeque<String>>,
    live: broadcast::Sender<String>,
    next_id: AtomicU64,
}

impl AppChannel {
    /// Listens on a free port of 127.0.0.1 and returns it, so the daemon can tell windows.
    pub fn bind(token: impl Into<String>) -> io::Result<(Self, SocketAddr)> {
        let listener = std::net::TcpListener::bind(("127.0.0.1", 0))?;
        listener.set_nonblocking(true)?;
        let address = listener.local_addr()?;
        let (live, _) = broadcast::channel(LIVE);
        let channel = Self {
            id: ChannelId::parse(APP).map_err(|error| io::Error::other(error.to_string()))?,
            token: token.into(),
            listener: Mutex::new(Some(listener)),
            hub: Arc::new(Hub {
                backlog: Mutex::new(VecDeque::new()),
                live,
                next_id: AtomicU64::new(0),
            }),
        };
        Ok((channel, address))
    }
}

impl fmt::Debug for AppChannel {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("AppChannel")
            .field("id", &self.id)
            .field("token", &"<hidden>")
            .finish_non_exhaustive()
    }
}

impl Hub {
    fn publish(&self, event: &ToWindow) {
        let Ok(line) = serde_json::to_string(event) else {
            return;
        };
        let mut backlog = self
            .backlog
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if backlog.len() == BACKLOG {
            backlog.pop_front();
        }
        backlog.push_back(line.clone());
        let _ = self.live.send(line);
    }

    fn join(&self) -> (Vec<String>, broadcast::Receiver<String>) {
        let backlog = self
            .backlog
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        (backlog.iter().cloned().collect(), self.live.subscribe())
    }

    fn message_id(&self) -> ExternalMessageId {
        let millis = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|since| since.as_millis())
            .unwrap_or(0);
        let n = self.next_id.fetch_add(1, Ordering::Relaxed);
        ExternalMessageId::new(format!("{millis}-{n}"))
    }
}

impl Channel for AppChannel {
    fn id(&self) -> &ChannelId {
        &self.id
    }

    fn run(&self, inbox: Arc<dyn Inbox>) -> BoxFuture<'_, Result<(), ChannelError>> {
        let listener = self
            .listener
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .take();
        Box::pin(async move {
            let Some(listener) = listener else {
                return Err(ChannelError {
                    detail: String::from("the app channel is already running"),
                });
            };
            let listener = TcpListener::from_std(listener).map_err(failed)?;
            let mut windows = JoinSet::new();
            loop {
                let (stream, _) = listener.accept().await.map_err(failed)?;
                while windows.try_join_next().is_some() {}
                windows.spawn(serve(
                    stream,
                    self.token.clone(),
                    Arc::clone(&self.hub),
                    Arc::clone(&inbox),
                ));
            }
        })
    }

    fn private_place(&self) -> Place {
        Place::new(MAIN)
    }

    fn deliver(&self, _place: Place, text: String) -> BoxFuture<'_, Result<(), ChannelError>> {
        self.hub.publish(&ToWindow::Reply { text });
        Box::pin(async { Ok(()) })
    }
}

impl AppWindow for AppChannel {
    fn show(&self, traffic: Traffic) -> BoxFuture<'_, Result<(), ChannelError>> {
        self.hub.publish(&ToWindow::Traffic {
            channel: traffic.address.channel.as_str().to_owned(),
            place: traffic.address.place.as_str().to_owned(),
            who: traffic.who,
            text: traffic.text,
        });
        Box::pin(async { Ok(()) })
    }
}

async fn serve(stream: TcpStream, token: String, hub: Arc<Hub>, inbox: Arc<dyn Inbox>) {
    let (read, mut write) = stream.into_split();
    let mut reader = BufReader::new(read);
    let mut line = Vec::new();
    let greeted = time::timeout(HELLO_WAIT, next_line(&mut reader, &mut line)).await;
    let hello = match greeted {
        Ok(Ok(true)) => serde_json::from_slice::<Hello>(&line).ok(),
        _ => None,
    };
    if hello.is_none_or(|hello| hello.token != token) {
        return;
    }
    let (backlog, mut live) = hub.join();
    let sending = async move {
        for line in backlog {
            send(&mut write, &line).await?;
        }
        loop {
            match live.recv().await {
                Ok(line) => send(&mut write, &line).await?,
                Err(RecvError::Lagged(_) | RecvError::Closed) => return Ok::<(), io::Error>(()),
            }
        }
    };
    let receiving = async {
        while let Ok(true) = next_line(&mut reader, &mut line).await {
            let Ok(FromWindow::Say { text }) = serde_json::from_slice::<FromWindow>(&line) else {
                continue;
            };
            let Ok(content) = Content::parse(text.clone()) else {
                continue;
            };
            hub.publish(&ToWindow::Owner { text });
            let incoming = Incoming {
                id: hub.message_id(),
                place: Place::new(MAIN),
                sender_id: ExternalUserId::new(OWNER),
                sender_name: String::from(OWNER),
                content,
                audience: Audience::Owner,
            };
            if inbox.push(incoming).await.is_err() {
                return;
            }
        }
    };
    tokio::select! {
        _ = sending => {}
        () = receiving => {}
    }
}

async fn next_line(reader: &mut BufReader<OwnedReadHalf>, line: &mut Vec<u8>) -> io::Result<bool> {
    line.clear();
    let read = (&mut *reader)
        .take(MAX_LINE + 1)
        .read_until(b'\n', line)
        .await?;
    if read == 0 {
        return Ok(false);
    }
    if line.len() as u64 > MAX_LINE {
        return Err(io::Error::new(io::ErrorKind::InvalidData, "line too long"));
    }
    Ok(true)
}

async fn send(write: &mut tokio::net::tcp::OwnedWriteHalf, line: &str) -> io::Result<()> {
    write.write_all(line.as_bytes()).await?;
    write.write_all(b"\n").await
}

fn failed(error: io::Error) -> ChannelError {
    ChannelError {
        detail: error.to_string(),
    }
}

#[cfg(test)]
#[path = "app.test.rs"]
mod tests;
