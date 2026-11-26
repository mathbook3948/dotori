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
use tokio::sync::mpsc;
use tokio::sync::watch;
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
const WINDOWS: usize = 32;
const ACCEPT_PAUSE: Duration = Duration::from_millis(50);

/// The first line a window sends. Without the right token the daemon hangs up.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Hello {
    pub token: String,
}

/// What a window sends after its hello, one JSON object per line.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum FromWindow {
    Say {
        text: String,
    },
    /// The owner allows (`ok`) or declines a call dotori asked about.
    Decide {
        id: u64,
        ok: bool,
    },
}

/// What the daemon sends to every window, one JSON object per line.
/// A window that connects first gets the recent backlog, then everything live.
/// A window that falls too far behind is hung up on, so it reconnects and reads the backlog.
/// The lines of the conversation carry `at`, when the first PC showed them, in milliseconds since
/// the Unix epoch, or 0 from a dotori that did not say.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ToWindow {
    /// What the owner wrote in some window.
    Owner {
        text: String,
        #[serde(default)]
        at: i64,
    },
    /// What dotori answered in the app.
    Reply {
        text: String,
        #[serde(default)]
        at: i64,
    },
    /// A message or reply on another channel, such as Slack.
    Traffic {
        channel: String,
        place: String,
        who: String,
        text: String,
        #[serde(default)]
        at: i64,
    },
    /// dotori wants to call a tool that acts toward other people, and waits for the owner.
    /// `sandboxed` says the command it asks to run would run in the sandbox.
    Ask {
        id: u64,
        tool: String,
        input: String,
        #[serde(default)]
        at: i64,
        #[serde(default)]
        sandboxed: bool,
    },
    /// The owner allowed or declined what dotori asked about.
    Decided { id: u64, ok: bool },
    /// What dotori asked about no longer waits, so the owner's answer came too late.
    Lapsed { id: u64 },
    /// Which of the owner's PCs holds the browser now. Sent only while PCs are linked, and never
    /// kept in the conversation.
    BrowserAt { pc: String, here: bool },
    /// Everything before this came from the backlog, and what follows happens now. Sent once to
    /// each window, so it can let new lines move in without moving the history.
    Live,
    /// Whether dotori is at work now, thinking or running tools. Sent to a window when it
    /// connects and whenever it changes, and never kept in the conversation.
    Working { busy: bool },
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
    live: broadcast::Sender<Published>,
    next_id: AtomicU64,
    browser: watch::Sender<Option<String>>,
    working: watch::Sender<bool>,
    decisions: Mutex<Option<mpsc::UnboundedSender<(u64, bool)>>>,
}

/// One line every window shows, and the other PC whose window it came from, if it did.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Published {
    pub line: String,
    pub origin: Option<String>,
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
                browser: watch::channel(None).0,
                working: watch::channel(false).0,
                decisions: Mutex::new(None),
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

impl AppChannel {
    /// Every line windows are shown from now on, for showing them on the other PCs too.
    pub fn published(&self) -> broadcast::Receiver<Published> {
        self.hub.live.subscribe()
    }

    /// Whether dotori is at work, as windows are told it, from now on. The main session's PC
    /// passes this to the other PCs.
    pub fn working_changes(&self) -> watch::Receiver<bool> {
        self.hub.working.subscribe()
    }

    /// Says that another PC answers now, so whatever the last one was working on is no longer
    /// shown as going on.
    pub fn answerer_changed(&self) {
        self.hub.working.send_replace(false);
    }

    /// Tells every window which PC holds the browser now.
    pub fn browser_at(&self, pc: String, here: bool) {
        if let Ok(line) = serde_json::to_string(&ToWindow::BrowserAt { pc, here }) {
            self.hub.browser.send_replace(Some(line));
        }
    }

    /// Shows a line another PC's windows were shown. Anything that is not such a line is
    /// passed over.
    pub fn show_line(&self, line: &str) {
        if serde_json::from_str::<ToWindow>(line).is_ok_and(|shown| {
            !matches!(
                shown,
                ToWindow::BrowserAt { .. } | ToWindow::Live | ToWindow::Working { .. }
            )
        }) {
            self.hub.publish_line(line.to_owned(), None);
        }
    }

    /// Shows every window a call dotori wants the owner to allow first, and whether a command it
    /// runs would be sandboxed.
    pub fn ask(&self, id: u64, tool: String, input: String, sandboxed: bool) {
        self.hub.publish(&ToWindow::Ask {
            id,
            tool,
            input,
            at: now_ms(),
            sandboxed,
        });
    }

    /// Shows every window what the owner decided about the call `id`.
    pub fn decided(&self, id: u64, ok: bool) {
        self.hub.publish(&ToWindow::Decided { id, ok });
    }

    /// Shows every window that the call `id` no longer waits for the owner.
    pub fn lapsed(&self, id: u64) {
        self.hub.publish(&ToWindow::Lapsed { id });
    }

    /// Takes what the owner decided about the call `id` somewhere other than a window here, such
    /// as a chat room's card, as if they decided it in a window.
    pub fn decide(&self, id: u64, ok: bool) {
        self.hub.decide(id, ok);
    }

    /// What the owner decides in the windows here, from now on. Only the last caller hears.
    pub fn decisions(&self) -> mpsc::UnboundedReceiver<(u64, bool)> {
        let (decisions, heard) = mpsc::unbounded_channel();
        *self
            .hub
            .decisions
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(decisions);
        heard
    }

    /// Puts lines said before dotori started ahead of everything windows are shown, without
    /// sending them to the other PCs.
    pub fn show_earlier(&self, lines: &[ToWindow]) {
        let mut backlog = self
            .hub
            .backlog
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let room = BACKLOG.saturating_sub(backlog.len());
        let earlier: Vec<String> = lines
            .iter()
            .filter_map(|line| serde_json::to_string(line).ok())
            .collect();
        for line in earlier.into_iter().rev().take(room) {
            backlog.push_front(line);
        }
    }

    /// Words the owner already saw in a window here, ready to hand to the agent. `at` is when
    /// the owner said them.
    pub fn incoming(&self, text: String, at: Option<SystemTime>) -> Option<Incoming> {
        Some(self.hub.incoming(Content::parse(text).ok()?, at))
    }

    /// What the owner said in a window on the PC `origin`, shown here as if said in a window
    /// here, and ready to hand to the agent.
    pub fn heard_elsewhere(
        &self,
        text: String,
        origin: String,
        at: Option<SystemTime>,
    ) -> Option<Incoming> {
        let content = Content::parse(text.clone()).ok()?;
        let Ok(line) = serde_json::to_string(&ToWindow::Owner { text, at: now_ms() }) else {
            return None;
        };
        self.hub.publish_line(line, Some(origin));
        Some(self.hub.incoming(content, at))
    }
}

impl Hub {
    fn decide(&self, id: u64, ok: bool) {
        if let Some(decisions) = self
            .decisions
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .as_ref()
        {
            let _ = decisions.send((id, ok));
        }
    }

    fn publish(&self, event: &ToWindow) {
        let Ok(line) = serde_json::to_string(event) else {
            return;
        };
        self.publish_line(line, None);
    }

    fn incoming(&self, content: Content, at: Option<SystemTime>) -> Incoming {
        Incoming {
            id: self.message_id(),
            place: Place::new(MAIN),
            sender_id: ExternalUserId::new(OWNER),
            sender_name: String::from(OWNER),
            content,
            audience: Audience::Owner,
            at,
        }
    }

    fn publish_line(&self, line: String, origin: Option<String>) {
        let mut backlog = self
            .backlog
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if backlog.len() == BACKLOG {
            backlog.pop_front();
        }
        backlog.push_back(line.clone());
        let _ = self.live.send(Published { line, origin });
    }

    fn join(&self) -> (Vec<String>, broadcast::Receiver<Published>) {
        let backlog = self
            .backlog
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        (backlog.iter().cloned().collect(), self.live.subscribe())
    }

    fn message_id(&self) -> ExternalMessageId {
        let millis = now_ms();
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
                let Ok((stream, _)) = listener.accept().await else {
                    time::sleep(ACCEPT_PAUSE).await;
                    continue;
                };
                while windows.try_join_next().is_some() {}
                if windows.len() >= WINDOWS {
                    drop(stream);
                    continue;
                }
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
        self.hub.publish(&ToWindow::Reply { text, at: now_ms() });
        Box::pin(async { Ok(()) })
    }
}

impl AppWindow for AppChannel {
    fn working(&self, busy: bool) {
        self.hub.working.send_replace(busy);
    }

    fn show(&self, traffic: Traffic) -> BoxFuture<'_, Result<(), ChannelError>> {
        self.hub.publish(&ToWindow::Traffic {
            channel: traffic.address.channel.as_str().to_owned(),
            place: traffic.address.place.as_str().to_owned(),
            who: traffic.who,
            text: traffic.text,
            at: now_ms(),
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
    let mut browser = hub.browser.subscribe();
    let mut working = hub.working.subscribe();
    let sending = async move {
        for line in backlog {
            send(&mut write, &line).await?;
        }
        let now = browser.borrow_and_update().clone();
        if let Some(line) = now {
            send(&mut write, &line).await?;
        }
        let busy = *working.borrow_and_update();
        if let Ok(line) = serde_json::to_string(&ToWindow::Working { busy }) {
            send(&mut write, &line).await?;
        }
        if let Ok(line) = serde_json::to_string(&ToWindow::Live) {
            send(&mut write, &line).await?;
        }
        loop {
            tokio::select! {
                published = live.recv() => match published {
                    Ok(published) => send(&mut write, &published.line).await?,
                    Err(RecvError::Lagged(_) | RecvError::Closed) => {
                        return Ok::<(), io::Error>(());
                    }
                },
                changed = browser.changed() => {
                    if changed.is_err() {
                        return Ok(());
                    }
                    let now = browser.borrow_and_update().clone();
                    if let Some(line) = now {
                        send(&mut write, &line).await?;
                    }
                }
                changed = working.changed() => {
                    if changed.is_err() {
                        return Ok(());
                    }
                    let busy = *working.borrow_and_update();
                    if let Ok(line) = serde_json::to_string(&ToWindow::Working { busy }) {
                        send(&mut write, &line).await?;
                    }
                }
            }
        }
    };
    let receiving = async {
        while let Ok(true) = next_line(&mut reader, &mut line).await {
            let text = match serde_json::from_slice::<FromWindow>(&line) {
                Ok(FromWindow::Say { text }) => text,
                Ok(FromWindow::Decide { id, ok }) => {
                    hub.decide(id, ok);
                    continue;
                }
                Err(_) => continue,
            };
            let Ok(content) = Content::parse(text.clone()) else {
                continue;
            };
            hub.publish(&ToWindow::Owner { text, at: now_ms() });
            if inbox
                .push(hub.incoming(content, Some(SystemTime::now())))
                .await
                .is_err()
            {
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

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|since| i64::try_from(since.as_millis()).unwrap_or(i64::MAX))
        .unwrap_or(0)
}

fn failed(error: io::Error) -> ChannelError {
    ChannelError {
        detail: error.to_string(),
    }
}

#[cfg(test)]
#[path = "app.test.rs"]
mod tests;
