use std::thread;
use std::time::Duration;

use dotori_connectors::channel::app::FromWindow;
use dotori_connectors::channel::app::Hello;
use dotori_connectors::channel::app::ToWindow;
use tokio::io::AsyncBufReadExt;
use tokio::io::AsyncWriteExt;
use tokio::io::BufReader;
use tokio::net::TcpStream;
use tokio::sync::mpsc;
use tokio::time;

use crate::door::Door;
use crate::home::Home;
use crate::lock::DaemonLock;

const RETRY: Duration = Duration::from_secs(2);
const SETTLE: Duration = Duration::from_millis(500);
const NOT_RUNNING: &str = "dotori is not running";
const STARTING: &str = "dotori is starting";

/// What a window learns from its link to the daemon, in order.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Update {
    /// The daemon kept the link open past the hello, so it took the token. The backlog follows,
    /// so a window drops what it showed before.
    Connected,
    Down(String),
    Event(ToWindow),
    /// dotori stopped after this window reached it, as when the owner quits from the tray, so
    /// the window closes too.
    Gone,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Daemon {
    Unseen,
    Started,
    Reached,
}

/// A window's link to the daemon. It reconnects by itself, so a daemon restart or a hang-up for
/// falling behind only shows as `Down` followed by `Connected`. A window does not send on
/// `say` after `Down` until the next `Connected`, since that would go out late and unseen.
#[derive(Debug)]
pub(crate) struct Link {
    pub(crate) updates: mpsc::UnboundedReceiver<Update>,
    pub(crate) say: mpsc::UnboundedSender<FromWindow>,
}

/// Starts the link on a thread of its own, so any UI loop can read `updates`.
pub(crate) fn open(home: Home) -> Link {
    let (updates_tx, updates) = mpsc::unbounded_channel();
    let (say, said) = mpsc::unbounded_channel();
    thread::spawn(move || {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build();
        match runtime {
            Ok(runtime) => runtime.block_on(keep(home, updates_tx, said)),
            Err(error) => {
                let _ = updates_tx.send(Update::Down(error.to_string()));
            }
        }
    });
    Link { updates, say }
}

async fn keep(
    home: Home,
    updates: mpsc::UnboundedSender<Update>,
    mut said: mpsc::UnboundedReceiver<FromWindow>,
) {
    let mut daemon = Daemon::Unseen;
    loop {
        let reason = match session(&home, &updates, &mut said, &mut daemon).await {
            Ok(Ended::WindowClosed) => return,
            Ok(Ended::HungUp) => String::from("dotori closed the connection"),
            Err(error) => error,
        };
        let reason = match (reason == NOT_RUNNING, daemon) {
            (false, _) => reason,
            (true, Daemon::Reached) => {
                let _ = updates.send(Update::Gone);
                return;
            }
            (true, Daemon::Unseen) => {
                daemon = Daemon::Started;
                crate::tray::start();
                String::from(STARTING)
            }
            (true, Daemon::Started) => String::from(STARTING),
        };
        if updates.send(Update::Down(reason)).is_err() {
            return;
        }
        time::sleep(RETRY).await;
    }
}

enum Ended {
    WindowClosed,
    HungUp,
}

async fn session(
    home: &Home,
    updates: &mpsc::UnboundedSender<Update>,
    said: &mut mpsc::UnboundedReceiver<FromWindow>,
    daemon: &mut Daemon,
) -> Result<Ended, String> {
    if !DaemonLock::held(&home.lock())? {
        return Err(String::from(NOT_RUNNING));
    }
    if !home.door().exists() {
        return Err(String::from(STARTING));
    }
    let door = Door::read(&home.door())?;
    let stream = TcpStream::connect(("127.0.0.1", door.port))
        .await
        .map_err(|error| format!("dotori is not reachable: {error}"))?;
    let (read, mut write) = stream.into_split();
    let hello =
        serde_json::to_string(&Hello { token: door.token }).map_err(|error| error.to_string())?;
    line(&mut write, &hello).await?;
    let mut events = BufReader::new(read).lines();
    let first = match time::timeout(SETTLE, events.next_line()).await {
        Err(_) => None,
        Ok(Ok(Some(text))) => Some(text),
        Ok(Ok(None)) => return Ok(Ended::HungUp),
        Ok(Err(error)) => return Err(format!("dotori is not reachable: {error}")),
    };
    *daemon = Daemon::Reached;
    if updates.send(Update::Connected).is_err() {
        return Ok(Ended::WindowClosed);
    }
    if let Some(text) = first
        && !forward(updates, &text)
    {
        return Ok(Ended::WindowClosed);
    }
    loop {
        tokio::select! {
            event = events.next_line() => match event {
                Ok(Some(text)) => {
                    if !forward(updates, &text) {
                        return Ok(Ended::WindowClosed);
                    }
                }
                Ok(None) => return Ok(Ended::HungUp),
                Err(error) => return Err(format!("dotori is not reachable: {error}")),
            },
            message = said.recv() => match message {
                Some(message) => {
                    let said = serde_json::to_string(&message).map_err(|error| error.to_string())?;
                    line(&mut write, &said).await?;
                }
                None => return Ok(Ended::WindowClosed),
            },
        }
    }
}

fn forward(updates: &mpsc::UnboundedSender<Update>, text: &str) -> bool {
    match serde_json::from_str::<ToWindow>(text) {
        Ok(event) => updates.send(Update::Event(event)).is_ok(),
        Err(_) => true,
    }
}

async fn line(write: &mut tokio::net::tcp::OwnedWriteHalf, text: &str) -> Result<(), String> {
    let sent = async {
        write.write_all(text.as_bytes()).await?;
        write.write_all(b"\n").await
    };
    sent.await
        .map_err(|error| format!("dotori is not reachable: {error}"))
}
