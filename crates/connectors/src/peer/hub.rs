//! Keeps this PC linked to every approved PC it can reach. It listens for the others, dials those
//! whose address it knows, keeps one line per PC, and shares the book of approved PCs. Messages
//! other than its own reach the caller as `Heard`.

use std::collections::HashMap;
use std::collections::HashSet;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::atomic::AtomicBool;
use std::sync::atomic::AtomicU64;
use std::sync::atomic::Ordering;
use std::time::Duration;

use serde::Deserialize;
use serde::Serialize;
use serde_json::Value;
use serde_json::json;
use tokio::net::TcpListener;
use tokio::net::TcpStream;
use tokio::sync::Semaphore;
use tokio::sync::mpsc;
use tokio::time;

use crate::peer::Book;
use crate::peer::Code;
use crate::peer::Identity;
use crate::peer::Invite;
use crate::peer::KEY;
use crate::peer::Peer;
use crate::peer::hex;
use crate::peer::now;
use crate::peer::unhex;
use crate::peer::wire;
use crate::peer::wire::Hello;
use crate::peer::wire::Line;
use crate::peer::wire::Outbox;
use crate::peer::wire::Start;
use crate::peer::write_privately;

const DIAL_TICK: Duration = Duration::from_millis(500);
const REDIAL_FIRST: Duration = Duration::from_secs(2);
const REDIAL_LONGEST: Duration = Duration::from_secs(60);
const STEADY: Duration = Duration::from_secs(60);
const BOOK_EVERY: u32 = 10;
const DIAL_WAIT: Duration = Duration::from_secs(5);
const BOOK: &str = "book";
const GREETING: usize = 16;

/// Where this PC keeps what the hub needs, and how the others reach it.
#[derive(Clone, Debug)]
pub struct HubConfig {
    pub identity: Identity,
    pub device: String,
    pub name: String,
    pub address: Option<String>,
    pub listen: Option<SocketAddr>,
    pub book: PathBuf,
    pub invite: PathBuf,
    pub seen: PathBuf,
}

/// A message from another PC.
#[derive(Clone, Debug, PartialEq)]
pub struct Heard {
    pub key: String,
    pub device: String,
    pub message: Value,
}

/// Which PCs are linked now and when each was last linked or lost, for the settings window.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Seen {
    #[serde(default)]
    pub peers: HashMap<String, SeenPeer>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SeenPeer {
    pub linked: bool,
    pub at: u64,
}

#[derive(Debug)]
struct Linked {
    id: u64,
    dialed_by: [u8; KEY],
    device: String,
    outbox: Outbox,
}

impl Drop for Linked {
    fn drop(&mut self) {
        self.outbox.hang_up();
    }
}

#[derive(Clone, Copy, Debug)]
struct Redial {
    wait: Duration,
    at: time::Instant,
}

impl Redial {
    fn after(previous: Option<Self>, now: time::Instant, random: u64) -> Self {
        let wait = previous.map_or(REDIAL_FIRST, |previous| {
            previous.wait.saturating_mul(2).min(REDIAL_LONGEST)
        });
        Self {
            wait,
            at: now + spread(wait, random),
        }
    }
}

fn spread(wait: Duration, random: u64) -> Duration {
    let half = wait / 2;
    let room = u64::try_from(half.as_nanos()).unwrap_or(u64::MAX);
    half + Duration::from_nanos(random % room.saturating_add(1))
}

/// The lines to the other PCs.
#[derive(Debug)]
pub struct Hub {
    config: HubConfig,
    own: String,
    lines: Mutex<HashMap<String, Linked>>,
    dialing: Mutex<HashSet<String>>,
    redials: Mutex<HashMap<(String, String), Redial>>,
    seen: Mutex<Seen>,
    heard: mpsc::UnboundedSender<Heard>,
    next: AtomicU64,
    listening: Option<SocketAddr>,
    kept: Mutex<Book>,
    shared: Mutex<Option<Book>>,
    joining: AtomicBool,
}

impl Hub {
    /// Starts listening, if `listen` is set, and dialing the approved PCs.
    pub async fn start(
        config: HubConfig,
    ) -> Result<(Arc<Self>, mpsc::UnboundedReceiver<Heard>), String> {
        let listener = match config.listen {
            Some(address) => Some(
                TcpListener::bind(address)
                    .await
                    .map_err(|error| format!("cannot listen on {address}: {error}"))?,
            ),
            None => None,
        };
        let listening = match &listener {
            Some(listener) => Some(listener.local_addr().map_err(|error| error.to_string())?),
            None => None,
        };
        let mut seen: Seen = std::fs::read_to_string(&config.seen)
            .ok()
            .and_then(|text| serde_json::from_str(&text).ok())
            .unwrap_or_default();
        for peer in seen.peers.values_mut() {
            peer.linked = false;
        }
        let (heard, hearing) = mpsc::unbounded_channel();
        let hub = Arc::new(Self {
            own: hex(&config.identity.public),
            config,
            lines: Mutex::new(HashMap::new()),
            dialing: Mutex::new(HashSet::new()),
            redials: Mutex::new(HashMap::new()),
            seen: Mutex::new(seen),
            heard,
            next: AtomicU64::new(1),
            listening,
            kept: Mutex::new(Book::default()),
            shared: Mutex::new(None),
            joining: AtomicBool::new(false),
        });
        let _ = hub.write_seen();
        if let Some(listener) = listener {
            tokio::spawn(Arc::clone(&hub).listen(listener));
        }
        tokio::spawn(Arc::clone(&hub).dial_all());
        Ok((hub, hearing))
    }

    /// Queues `message` for the PC with `key` without waiting, and says whether it was queued. A
    /// PC too far behind to take it is unlinked, so it links again and gets whole copies.
    pub fn send(&self, key: &str, message: Value) -> bool {
        let Some(message) = encoded(&message) else {
            return false;
        };
        let mut lines = self.lines.lock().expect("unpoisoned");
        let Some(line) = lines.get(key) else {
            return false;
        };
        if line.outbox.put(message) {
            return true;
        }
        lines.remove(key);
        false
    }

    /// Queues `message` for every linked PC without waiting. A PC too far behind to take it is
    /// unlinked, as with `send`.
    pub fn broadcast(&self, message: &Value) {
        let Some(message) = encoded(message) else {
            return;
        };
        self.lines
            .lock()
            .expect("unpoisoned")
            .retain(|_, line| line.outbox.put(Arc::clone(&message)));
    }

    /// What the owner calls the PC with `key`, this one included.
    pub fn name_of(&self, key: &str) -> String {
        if key == self.own {
            return self.config.name.clone();
        }
        self.book()
            .find(key)
            .map(|peer| {
                if peer.name.trim().is_empty() {
                    format!("PC {}", peer.device.get(..6).unwrap_or(&peer.device))
                } else {
                    peer.name.clone()
                }
            })
            .unwrap_or_else(|| format!("PC {}", key.get(..6).unwrap_or(key)))
    }

    /// This PC's key, as the others know it.
    pub fn own(&self) -> &str {
        &self.own
    }

    /// The keys of every PC the owner approved, linked now or not.
    pub fn approved(&self) -> Vec<String> {
        self.book()
            .peers
            .into_iter()
            .map(|peer| peer.key)
            .filter(|key| *key != self.own)
            .collect()
    }

    /// Where this PC listens for the others, if it does.
    pub fn listening(&self) -> Option<SocketAddr> {
        self.listening
    }

    /// The keys and names of the PCs linked now.
    pub fn linked(&self) -> Vec<(String, String)> {
        self.lines
            .lock()
            .expect("unpoisoned")
            .iter()
            .map(|(key, line)| (key.clone(), line.device.clone()))
            .collect()
    }

    fn hello(&self) -> Hello {
        Hello {
            device: self.config.device.clone(),
            name: self.config.name.clone(),
            address: self.config.address.clone(),
        }
    }

    fn book(&self) -> Book {
        let mut kept = self.kept.lock().expect("unpoisoned");
        match Book::load(&self.config.book) {
            Ok(book) => {
                kept.clone_from(&book);
                book
            }
            Err(error) => {
                eprintln!("dotori: {error}");
                kept.clone()
            }
        }
    }

    fn edit_book(&self, edit: impl FnOnce(&mut Book) -> bool) -> Option<Book> {
        let mut kept = self.kept.lock().expect("unpoisoned");
        let mut book = match Book::load(&self.config.book) {
            Ok(book) => book,
            Err(error) => {
                eprintln!("dotori: the book of PCs is left alone: {error}");
                return None;
            }
        };
        if !edit(&mut book) {
            kept.clone_from(&book);
            return None;
        }
        if let Err(error) = book.save(&self.config.book) {
            eprintln!("dotori: {error}");
            return None;
        }
        kept.clone_from(&book);
        Some(book)
    }

    fn share_changed(&self, book: &Book, except: Option<&str>) {
        {
            let mut shared = self.shared.lock().expect("unpoisoned");
            if shared.as_ref() == Some(book) {
                return;
            }
            *shared = Some(book.clone());
        }
        let Some(message) = encoded(&json!({ "kind": BOOK, "book": book })) else {
            return;
        };
        self.lines.lock().expect("unpoisoned").retain(|key, line| {
            Some(key.as_str()) == except || line.outbox.put(Arc::clone(&message))
        });
    }

    async fn listen(self: Arc<Self>, listener: TcpListener) {
        let greeting = Arc::new(Semaphore::new(GREETING));
        loop {
            let Ok((stream, _)) = listener.accept().await else {
                time::sleep(Duration::from_millis(50)).await;
                continue;
            };
            let Ok(turn) = Arc::clone(&greeting).try_acquire_owned() else {
                drop(stream);
                continue;
            };
            let hub = Arc::clone(&self);
            tokio::spawn(async move {
                let book = hub.book();
                let took = AtomicBool::new(false);
                let invite = || {
                    if hub.joining.swap(true, Ordering::SeqCst) {
                        return None;
                    }
                    took.store(true, Ordering::SeqCst);
                    Invite::open(&hub.config.invite, now())
                };
                let answered = wire::answer(
                    stream,
                    &hub.config.identity,
                    |key| book.find(&hex(key)).is_some(),
                    invite,
                    &hub.hello(),
                )
                .await;
                drop(turn);
                if let Ok((line, Start::Join(_))) = &answered {
                    hub.welcome(line);
                }
                if took.load(Ordering::SeqCst) {
                    hub.joining.store(false, Ordering::SeqCst);
                }
                match answered {
                    Ok((_, Start::Join(_))) => {}
                    Ok((line, Start::Link)) => {
                        let dialer = line.key;
                        hub.take(line, dialer).await;
                    }
                    Err(_) => {}
                }
            });
        }
    }

    fn welcome(&self, line: &Line) {
        let _ = std::fs::remove_file(&self.config.invite);
        let peer = Peer {
            device: line.hello.device.clone(),
            name: line.hello.name.clone(),
            key: hex(&line.key),
            address: line.hello.address.clone(),
            since: now(),
        };
        if let Some(book) = self.edit_book(|book| book.approve(peer)) {
            self.share_changed(&book, None);
        }
    }

    async fn dial_all(self: Arc<Self>) {
        let mut every = time::interval(DIAL_TICK);
        let mut ticks = 0_u32;
        loop {
            every.tick().await;
            ticks = ticks.wrapping_add(1);
            let book = if ticks % BOOK_EVERY == 1 {
                let book = self.book();
                self.drop_unapproved(&book);
                self.share_changed(&book, None);
                book
            } else {
                self.kept.lock().expect("unpoisoned").clone()
            };
            self.redials
                .lock()
                .expect("unpoisoned")
                .retain(|(key, address), _| {
                    book.find(key)
                        .is_some_and(|peer| peer.address.as_deref() == Some(address.as_str()))
                });
            let now = time::Instant::now();
            for peer in &book.peers {
                let Some(address) = peer.address.clone() else {
                    continue;
                };
                if peer.key == self.own
                    || self
                        .lines
                        .lock()
                        .expect("unpoisoned")
                        .contains_key(&peer.key)
                    || self
                        .redials
                        .lock()
                        .expect("unpoisoned")
                        .get(&(peer.key.clone(), address.clone()))
                        .is_some_and(|redial| redial.at > now)
                    || !self
                        .dialing
                        .lock()
                        .expect("unpoisoned")
                        .insert(peer.key.clone())
                {
                    continue;
                }
                let hub = Arc::clone(&self);
                let key = peer.key.clone();
                tokio::spawn(async move {
                    if !hub.dial(&key, &address).await {
                        hub.wait_longer(&key, &address);
                    }
                    hub.dialing.lock().expect("unpoisoned").remove(&key);
                });
            }
        }
    }

    fn wait_longer(&self, key: &str, address: &str) {
        let mut redials = self.redials.lock().expect("unpoisoned");
        let place = (key.to_owned(), address.to_owned());
        let previous = redials.get(&place).copied();
        let redial = Redial::after(previous, time::Instant::now(), random_or_zero());
        redials.insert(place, redial);
    }

    async fn dial(self: &Arc<Self>, key: &str, address: &str) -> bool {
        let Some(their) = unhex(key).and_then(|bytes| <[u8; KEY]>::try_from(bytes).ok()) else {
            return false;
        };
        let Ok(Ok(stream)) = time::timeout(DIAL_WAIT, TcpStream::connect(address)).await else {
            return false;
        };
        let Ok(line) = wire::dial(
            stream,
            &self.config.identity,
            &their,
            Start::Link,
            &self.hello(),
        )
        .await
        else {
            return false;
        };
        let dialer = self.config.identity.public;
        Arc::clone(self).take(line, dialer).await;
        true
    }

    fn drop_unapproved(&self, book: &Book) {
        self.lines
            .lock()
            .expect("unpoisoned")
            .retain(|key, _| book.find(key).is_some());
    }

    async fn take(self: Arc<Self>, line: Line, dialed_by: [u8; KEY]) {
        let key = hex(&line.key);
        if key == self.own {
            return;
        }
        let id = self.next.fetch_add(1, Ordering::Relaxed);
        let (outbox, outgoing) = wire::outbox();
        let preferred = self.config.identity.public.min(line.key);
        {
            let mut lines = self.lines.lock().expect("unpoisoned");
            if let Some(held) = lines.get(&key)
                && held.dialed_by == preferred
                && dialed_by != preferred
            {
                return;
            }
            lines.insert(
                key.clone(),
                Linked {
                    id,
                    dialed_by,
                    device: line.hello.device.clone(),
                    outbox: outbox.clone(),
                },
            );
        }
        let began = time::Instant::now();
        let book = self
            .edit_book(|book| book.moved(&key, &line.hello))
            .unwrap_or_else(|| self.book());
        if let Some(message) = encoded(&json!({ "kind": BOOK, "book": book })) {
            outbox.put(message);
        }
        drop(outbox);
        self.mark(&key, true);
        let device = line.hello.device.clone();
        let _ = self.heard.send(Heard {
            key: key.clone(),
            device: device.clone(),
            message: json!({ "kind": "linked" }),
        });
        let (heard_tx, mut heard) = mpsc::unbounded_channel::<Value>();
        let carried = tokio::spawn(line.carry(outgoing, heard_tx));
        while let Some(message) = heard.recv().await {
            if message.get("kind").and_then(Value::as_str) == Some(BOOK) {
                self.hear_book(&key, &message);
                continue;
            }
            let _ = self.heard.send(Heard {
                key: key.clone(),
                device: device.clone(),
                message,
            });
        }
        let _ = carried.await;
        let still = {
            let mut lines = self.lines.lock().expect("unpoisoned");
            if lines.get(&key).is_some_and(|line| line.id == id) {
                lines.remove(&key);
                false
            } else {
                lines.contains_key(&key)
            }
        };
        if !still {
            self.mark(&key, false);
        }
        if began.elapsed() >= STEADY {
            self.redials
                .lock()
                .expect("unpoisoned")
                .retain(|(known, _), _| *known != key);
        } else {
            let address = self
                .kept
                .lock()
                .expect("unpoisoned")
                .find(&key)
                .and_then(|peer| peer.address.clone());
            if let Some(address) = address {
                self.wait_longer(&key, &address);
            }
        }
    }

    fn hear_book(&self, from: &str, message: &Value) {
        let Some(other) = message
            .get(BOOK)
            .and_then(|book| serde_json::from_value::<Book>(book.clone()).ok())
        else {
            return;
        };
        let Some(book) = self.edit_book(|book| book.merge(&other, &self.own, now())) else {
            return;
        };
        self.drop_unapproved(&book);
        self.share_changed(&book, Some(from));
    }

    fn mark(&self, key: &str, linked: bool) {
        self.seen
            .lock()
            .expect("unpoisoned")
            .peers
            .insert(key.to_owned(), SeenPeer { linked, at: now() });
        let _ = self.write_seen();
    }

    fn write_seen(&self) -> Result<(), String> {
        let text = serde_json::to_string(&*self.seen.lock().expect("unpoisoned"))
            .map_err(|error| error.to_string())?;
        write_privately(&self.config.seen, text.as_bytes())
    }
}

fn encoded(message: &Value) -> Option<Arc<[u8]>> {
    wire::encode(message)
        .inspect_err(|error| eprintln!("dotori: not sent to the other PCs: {error}"))
        .ok()
}

fn random_or_zero() -> u64 {
    getrandom::u64().unwrap_or(0)
}

/// Why joining another PC failed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum JoinError {
    /// The other PC was not reached or refused the code.
    There(String),
    /// This PC could not prepare or keep the join.
    Here(String),
}

impl std::fmt::Display for JoinError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let (Self::There(error) | Self::Here(error)) = self;
        formatter.write_str(error)
    }
}

/// Joins the PC that handed out `code`, reached at `address`, and approves it in `book`. The
/// inviting PC approves this one in turn.
pub async fn join(
    identity: &Identity,
    hello: &Hello,
    code: &Code,
    address: &str,
    book: &std::path::Path,
) -> Result<Peer, JoinError> {
    let stream = time::timeout(DIAL_WAIT, TcpStream::connect(address))
        .await
        .map_err(|_| JoinError::There(format!("{address} did not answer in time")))?
        .map_err(|error| JoinError::There(format!("cannot reach {address}: {error}")))?;
    let line = wire::dial(stream, identity, &code.key, Start::Join(code.secret), hello)
        .await
        .map_err(|error| {
            JoinError::There(if error.contains("broke") {
                String::from("the other PC refused the code; make a new one there")
            } else {
                error
            })
        })?;
    let peer = Peer {
        device: line.hello.device.clone(),
        name: line.hello.name.clone(),
        key: hex(&line.key),
        address: Some(address.to_owned()),
        since: now(),
    };
    let mut kept = Book::load(book).map_err(JoinError::Here)?;
    kept.approve(peer.clone());
    kept.save(book).map_err(JoinError::Here)?;
    Ok(peer)
}

#[cfg(test)]
#[path = "hub.test.rs"]
mod tests;
