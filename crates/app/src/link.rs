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
const COULD_NOT_START: &str = "dotori could not start";
const START_RETRIES: u32 = 7;

/// What a window learns from its link to the daemon, in order.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Update {
    /// The daemon kept the link open past the hello, so it took the token. The backlog follows,
    /// so a window drops what it showed before.
    Connected,
    Down(String),
    Event(ToWindow),
    /// dotori stopped after this window saw it running, as when the owner quits from the tray, so
    /// the window closes too. dotori never restarts itself, so there is nothing to wait for.
    Gone,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Daemon {
    /// The lock has never been held in this window's sight.
    Unseen,
    /// This window started dotori and has found the lock free `free` times since.
    Started { free: u32 },
    /// The lock was held, so dotori was running or starting.
    Seen,
    /// The window connected.
    Reached,
}

#[derive(Debug, PartialEq, Eq)]
enum Step {
    Gone,
    Start,
    Starting,
    CouldNotStart,
    /// The lock is held, so the failure is the link's own and it shows as it is.
    Show,
}

fn step(daemon: Daemon, free: bool) -> (Step, Daemon) {
    match (free, daemon) {
        (false, _) => (Step::Show, daemon),
        (true, Daemon::Seen | Daemon::Reached) => (Step::Gone, daemon),
        (true, Daemon::Unseen) => (Step::Start, Daemon::Started { free: 0 }),
        (true, Daemon::Started { free }) if free + 1 >= START_RETRIES => {
            (Step::CouldNotStart, Daemon::Started { free: free + 1 })
        }
        (true, Daemon::Started { free }) => (Step::Starting, Daemon::Started { free: free + 1 }),
    }
}

/// A window's link to the daemon. It reconnects after a hang-up while dotori keeps running, as
/// when the window fell behind, so that only shows as `Down` followed by `Connected`. When dotori
/// itself stops the window closes instead. A window does not send on `say` after `Down` until
/// the next `Connected`, since that would go out late and unseen.
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
    let mut logged = String::new();
    loop {
        let reason = match session(&home, &updates, &mut said, &mut daemon).await {
            Ok(Ended::WindowClosed) => return,
            Ok(Ended::HungUp) => String::from("dotori closed the connection"),
            Err(error) => error,
        };
        let (next, after) = step(daemon, reason == NOT_RUNNING);
        daemon = after;
        let reason = match next {
            Step::Gone => {
                let _ = updates.send(Update::Gone);
                return;
            }
            Step::Start => {
                crate::tray::start();
                String::from(STARTING)
            }
            Step::Starting => String::from(STARTING),
            Step::CouldNotStart => String::from(COULD_NOT_START),
            Step::Show => reason,
        };
        if reason != logged {
            eprintln!("dotori: link: {reason}");
            logged.clone_from(&reason);
        }
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
    if *daemon != Daemon::Reached {
        *daemon = Daemon::Seen;
    }
    let door = Door::read(&home.door()).map_err(|error| {
        if home.door().exists() {
            error
        } else {
            String::from(STARTING)
        }
    })?;
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

#[cfg(test)]
#[path = "link.test.rs"]
mod tests;
