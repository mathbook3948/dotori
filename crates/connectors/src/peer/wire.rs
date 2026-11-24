//! One encrypted line between two PCs over TCP. The PC that dials knows the other's key. A PC
//! joining with an invite also proves it holds the invite's secret. Messages are JSON values,
//! cut into Noise frames.

use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use std::sync::atomic::AtomicUsize;
use std::sync::atomic::Ordering;
use std::time::Duration;

use serde::Deserialize;
use serde::Serialize;
use serde_json::Value;
use snow::HandshakeState;
use snow::TransportState;
use tokio::io::AsyncReadExt;
use tokio::io::AsyncWriteExt;
use tokio::net::TcpStream;
use tokio::net::tcp::OwnedReadHalf;
use tokio::net::tcp::OwnedWriteHalf;
use tokio::sync::Notify;
use tokio::sync::mpsc;
use tokio::time;

use crate::peer::Identity;
use crate::peer::KEY;
use crate::peer::SECRET;

pub(crate) const LINK: &str = "Noise_IK_25519_ChaChaPoly_BLAKE2s";
const JOIN: &str = "Noise_IKpsk1_25519_ChaChaPoly_BLAKE2s";
const PROLOGUE: &[u8] = b"dotori peers 1";
const FRAME: usize = 65535;
const TAG: usize = 16;
const PIECE: usize = FRAME - TAG - 1;
const MESSAGE_LONGEST: usize = 64 * 1024 * 1024;
pub(crate) const QUEUE_COUNT: usize = 4096;
pub(crate) const QUEUE_BYTES: usize = 2 * MESSAGE_LONGEST;
const PING_MESSAGE: &[u8] = br#"{"kind":"ping"}"#;
const HANDSHAKE_WAIT: Duration = Duration::from_secs(10);
const PING: Duration = Duration::from_secs(15);
const SILENCE: Duration = Duration::from_secs(45);
const LAST: u8 = 1;
const MORE: u8 = 0;

/// How a line starts: between PCs that already know each other, or with an invite.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Start {
    Link,
    Join([u8; SECRET]),
}

/// What each PC says about itself when a line opens.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Hello {
    pub device: String,
    #[serde(default)]
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub address: Option<String>,
}

/// An open line, before it starts carrying messages.
#[derive(Debug)]
pub struct Line {
    read: OwnedReadHalf,
    write: OwnedWriteHalf,
    transport: Box<TransportState>,
    pub key: [u8; KEY],
    pub hello: Hello,
}

/// Where messages wait to go out on a line. It holds at most `QUEUE_COUNT` messages and
/// `QUEUE_BYTES` bytes, the one being written included.
#[derive(Clone, Debug)]
pub struct Outbox {
    send: mpsc::Sender<Arc<[u8]>>,
    queue: Arc<Queue>,
}

/// The end of an `Outbox` that a line carries from.
#[derive(Debug)]
pub struct Outgoing {
    sent: mpsc::Receiver<Arc<[u8]>>,
    queue: Arc<Queue>,
}

#[derive(Debug, Default)]
struct Queue {
    bytes: AtomicUsize,
    hung_up: AtomicBool,
    hang_up: Notify,
}

/// A new outbox, and the end to give `Line::carry`.
pub fn outbox() -> (Outbox, Outgoing) {
    let (send, sent) = mpsc::channel(QUEUE_COUNT);
    let queue = Arc::new(Queue::default());
    (
        Outbox {
            send,
            queue: Arc::clone(&queue),
        },
        Outgoing { sent, queue },
    )
}

/// `message` as it goes on a line, made once for as many outboxes as it goes to.
pub fn encode(message: &Value) -> Result<Arc<[u8]>, String> {
    let bytes = serde_json::to_vec(message).map_err(|error| error.to_string())?;
    if bytes.len() > MESSAGE_LONGEST {
        return Err(String::from("a message too long to send"));
    }
    Ok(bytes.into())
}

impl Outbox {
    /// Queues `message` without waiting. Says false when the line has ended or the message does
    /// not fit, and a message that does not fit hangs the line up.
    pub fn put(&self, message: Arc<[u8]>) -> bool {
        if self.queue.hung_up.load(Ordering::SeqCst) {
            return false;
        }
        let length = message.len();
        let before = self.queue.bytes.fetch_add(length, Ordering::SeqCst);
        if before.saturating_add(length) <= QUEUE_BYTES && self.send.try_send(message).is_ok() {
            return true;
        }
        self.queue.bytes.fetch_sub(length, Ordering::SeqCst);
        self.hang_up();
        false
    }

    /// Ends the line this outbox feeds at once, dropping what is still queued.
    pub fn hang_up(&self) {
        self.queue.hung_up.store(true, Ordering::SeqCst);
        self.queue.hang_up.notify_one();
    }
}

struct Aborted(tokio::task::JoinHandle<()>);

impl Drop for Aborted {
    fn drop(&mut self) {
        self.0.abort();
    }
}

/// Opens a line to the PC with `key` on `stream`.
pub async fn dial(
    stream: TcpStream,
    identity: &Identity,
    key: &[u8; KEY],
    start: Start,
    hello: &Hello,
) -> Result<Line, String> {
    time::timeout(
        HANDSHAKE_WAIT,
        dial_now(stream, identity, key, start, hello),
    )
    .await
    .map_err(|_| String::from("the other PC did not answer in time"))?
}

async fn dial_now(
    stream: TcpStream,
    identity: &Identity,
    key: &[u8; KEY],
    start: Start,
    hello: &Hello,
) -> Result<Line, String> {
    let (mut read, mut write) = stream.into_split();
    let psk = start_psk(start);
    let mut handshake = builder(start, identity, psk.as_ref())?
        .remote_public_key(key)
        .map_err(noise)?
        .build_initiator()
        .map_err(noise)?;
    write
        .write_all(&[match start {
            Start::Link => b'L',
            Start::Join(_) => b'J',
        }])
        .await
        .map_err(io)?;
    let said = serde_json::to_vec(hello).map_err(|error| error.to_string())?;
    send_handshake(&mut handshake, &mut write, &said).await?;
    let heard = read_handshake(&mut handshake, &mut read).await?;
    finish(read, write, handshake, &heard)
}

/// Answers a line another PC opened on `stream`. `approved` says whether a key may link, and
/// `invite` gives the secret of the invite this PC handed out, if one is good now.
pub async fn answer(
    stream: TcpStream,
    identity: &Identity,
    approved: impl Fn(&[u8; KEY]) -> bool,
    invite: impl FnOnce() -> Option<[u8; SECRET]>,
    hello: &Hello,
) -> Result<(Line, Start), String> {
    time::timeout(
        HANDSHAKE_WAIT,
        answer_now(stream, identity, approved, invite, hello),
    )
    .await
    .map_err(|_| String::from("the other PC did not finish in time"))?
}

async fn answer_now(
    stream: TcpStream,
    identity: &Identity,
    approved: impl Fn(&[u8; KEY]) -> bool,
    invite: impl FnOnce() -> Option<[u8; SECRET]>,
    hello: &Hello,
) -> Result<(Line, Start), String> {
    let (mut read, mut write) = stream.into_split();
    let mut kind = [0u8];
    read.read_exact(&mut kind).await.map_err(io)?;
    let start = match kind[0] {
        b'L' => Start::Link,
        b'J' => Start::Join(invite().ok_or("no invite is open")?),
        _ => return Err(String::from("not a dotori PC")),
    };
    let psk = start_psk(start);
    let mut handshake = builder(start, identity, psk.as_ref())?
        .build_responder()
        .map_err(noise)?;
    let heard = read_handshake(&mut handshake, &mut read).await?;
    let key = remote(&handshake)?;
    if start == Start::Link && !approved(&key) {
        return Err(String::from("a PC the owner has not approved"));
    }
    let said = serde_json::to_vec(hello).map_err(|error| error.to_string())?;
    send_handshake(&mut handshake, &mut write, &said).await?;
    Ok((finish(read, write, handshake, &heard)?, start))
}

fn start_psk(start: Start) -> Option<[u8; 32]> {
    match start {
        Start::Link => None,
        Start::Join(secret) => {
            let mut psk = [0u8; 32];
            psk[..SECRET].copy_from_slice(&secret);
            Some(psk)
        }
    }
}

fn builder<'a>(
    start: Start,
    identity: &'a Identity,
    psk: Option<&'a [u8; 32]>,
) -> Result<snow::Builder<'a>, String> {
    let pattern = match start {
        Start::Link => LINK,
        Start::Join(_) => JOIN,
    };
    let mut builder = snow::Builder::new(pattern.parse().map_err(noise)?)
        .prologue(PROLOGUE)
        .map_err(noise)?
        .local_private_key(identity.private())
        .map_err(noise)?;
    if let Some(psk) = psk {
        builder = builder.psk(1, psk).map_err(noise)?;
    }
    Ok(builder)
}

fn remote(handshake: &HandshakeState) -> Result<[u8; KEY], String> {
    handshake
        .get_remote_static()
        .and_then(|key| <[u8; KEY]>::try_from(key).ok())
        .ok_or_else(|| String::from("the other PC gave no key"))
}

fn finish(
    read: OwnedReadHalf,
    write: OwnedWriteHalf,
    handshake: HandshakeState,
    heard: &[u8],
) -> Result<Line, String> {
    let key = remote(&handshake)?;
    let hello: Hello = serde_json::from_slice(heard).map_err(|_| "the other PC sent no hello")?;
    Ok(Line {
        read,
        write,
        transport: Box::new(handshake.into_transport_mode().map_err(noise)?),
        key,
        hello,
    })
}

async fn send_handshake(
    handshake: &mut HandshakeState,
    write: &mut OwnedWriteHalf,
    payload: &[u8],
) -> Result<(), String> {
    let mut frame = vec![0u8; FRAME];
    let length = handshake
        .write_message(payload, &mut frame)
        .map_err(noise)?;
    write_frame(write, &frame[..length]).await
}

async fn read_handshake(
    handshake: &mut HandshakeState,
    read: &mut OwnedReadHalf,
) -> Result<Vec<u8>, String> {
    let frame = read_frame(read).await?;
    let mut payload = vec![0u8; FRAME];
    let length = handshake
        .read_message(&frame, &mut payload)
        .map_err(|_| String::from("the other PC failed the handshake"))?;
    payload.truncate(length);
    Ok(payload)
}

async fn write_frame(write: &mut OwnedWriteHalf, frame: &[u8]) -> Result<(), String> {
    let length = u16::try_from(frame.len()).map_err(|_| "a frame too long")?;
    write.write_all(&length.to_be_bytes()).await.map_err(io)?;
    write.write_all(frame).await.map_err(io)
}

async fn read_frame(read: &mut OwnedReadHalf) -> Result<Vec<u8>, String> {
    let mut length = [0u8; 2];
    read.read_exact(&mut length).await.map_err(io)?;
    let mut frame = vec![0u8; usize::from(u16::from_be_bytes(length))];
    read.read_exact(&mut frame).await.map_err(io)?;
    Ok(frame)
}

impl Line {
    /// Carries messages until either side stops: what comes out of `outgoing` goes to the other
    /// PC, and what the other PC sends goes into `heard`. A quiet line is pinged, and a line silent
    /// for too long is closed. A line whose outbox overflowed closes at once, even mid-message.
    pub async fn carry(
        self,
        outgoing: Outgoing,
        heard: mpsc::UnboundedSender<Value>,
    ) -> Result<(), String> {
        let Outgoing { mut sent, queue } = outgoing;
        tokio::select! {
            ended = self.carry_until_stopped(&mut sent, &queue, heard) => ended,
            () = queue.hang_up.notified() => Err(String::from("the other PC fell too far behind")),
        }
    }

    async fn carry_until_stopped(
        self,
        sent: &mut mpsc::Receiver<Arc<[u8]>>,
        queue: &Queue,
        heard: mpsc::UnboundedSender<Value>,
    ) -> Result<(), String> {
        let Self {
            mut read,
            mut write,
            mut transport,
            ..
        } = self;
        let (frames_tx, mut frames) = mpsc::channel::<Vec<u8>>(16);
        let reader = Aborted(tokio::spawn(async move {
            while let Ok(frame) = read_frame(&mut read).await {
                if frames_tx.send(frame).await.is_err() {
                    return;
                }
            }
        }));
        let mut ping = time::interval(PING);
        let mut last_heard = time::Instant::now();
        let mut pieces = Vec::new();
        let mut plain = vec![0u8; FRAME];
        let ended = loop {
            tokio::select! {
                frame = frames.recv() => {
                    let Some(frame) = frame else { break Ok(()) };
                    last_heard = time::Instant::now();
                    let Ok(length) = transport.read_message(&frame, &mut plain) else {
                        break Err(String::from("a frame that does not decrypt"));
                    };
                    let Some((flag, piece)) = plain[..length].split_first() else {
                        break Err(String::from("an empty frame"));
                    };
                    if pieces.len() + piece.len() > MESSAGE_LONGEST {
                        break Err(String::from("a message too long"));
                    }
                    pieces.extend_from_slice(piece);
                    if *flag == LAST {
                        let message = std::mem::take(&mut pieces);
                        let Ok(message) = serde_json::from_slice::<Value>(&message) else {
                            break Err(String::from("a message that is not JSON"));
                        };
                        if message.get("kind").and_then(Value::as_str) != Some("ping")
                            && heard.send(message).is_err()
                        {
                            break Ok(());
                        }
                    }
                }
                out = sent.recv() => {
                    let Some(out) = out else { break Ok(()) };
                    let written = sent_in_time(&mut transport, &mut write, &out).await;
                    queue.bytes.fetch_sub(out.len(), Ordering::SeqCst);
                    if let Err(error) = written {
                        break Err(error);
                    }
                }
                _ = ping.tick() => {
                    if last_heard.elapsed() > SILENCE {
                        break Err(String::from("the other PC went quiet"));
                    }
                    if let Err(error) = sent_in_time(&mut transport, &mut write, PING_MESSAGE).await {
                        break Err(error);
                    }
                }
            }
        };
        drop(reader);
        ended
    }
}

async fn sent_in_time(
    transport: &mut TransportState,
    write: &mut OwnedWriteHalf,
    message: &[u8],
) -> Result<(), String> {
    time::timeout(SILENCE, send_message(transport, write, message))
        .await
        .map_err(|_| String::from("the other PC stopped taking what was sent"))?
}

async fn send_message(
    transport: &mut TransportState,
    write: &mut OwnedWriteHalf,
    message: &[u8],
) -> Result<(), String> {
    let mut frame = vec![0u8; FRAME];
    let mut piece = Vec::with_capacity(PIECE + 1);
    let mut chunks = message.chunks(PIECE).peekable();
    if chunks.peek().is_none() {
        return Err(String::from("an empty message"));
    }
    while let Some(chunk) = chunks.next() {
        piece.clear();
        piece.push(if chunks.peek().is_some() { MORE } else { LAST });
        piece.extend_from_slice(chunk);
        let length = transport.write_message(&piece, &mut frame).map_err(noise)?;
        write_frame(write, &frame[..length]).await?;
    }
    Ok(())
}

fn noise(error: impl std::fmt::Display) -> String {
    format!("encryption failed: {error}")
}

fn io(error: std::io::Error) -> String {
    format!("the line broke: {error}")
}

#[cfg(test)]
#[path = "wire.test.rs"]
mod tests;
