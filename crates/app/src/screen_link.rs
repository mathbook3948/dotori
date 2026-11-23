//! The chat window's line to the browser's screen in `dotori mcp`. It decodes frames on its own
//! thread, so the window only swaps pictures, and it reconnects when the server restarts.

use std::thread;
use std::time::Duration;

use serde_json::Value;
use tokio::io::AsyncBufReadExt;
use tokio::io::AsyncReadExt;
use tokio::io::AsyncWriteExt;
use tokio::io::BufReader;
use tokio::net::TcpStream;
use tokio::sync::mpsc;
use tokio::time;

use crate::door::Door;
use crate::home::Home;

const RETRY: Duration = Duration::from_secs(1);
const CONNECT_WAIT: Duration = Duration::from_secs(5);
const LINE: u64 = 8 * 1024 * 1024;

/// One frame, already turned into the BGRA pixels GPUI draws.
pub(crate) struct Picture {
    pub(crate) width: u32,
    pub(crate) height: u32,
    pub(crate) bgra: Vec<u8>,
    pub(crate) page_width: f64,
    pub(crate) page_height: f64,
}

pub(crate) enum Seen {
    Down,
    State(Value),
    Frame(Picture),
    Pointer { x: f64, y: f64 },
}

pub(crate) struct ScreenLink {
    pub(crate) seen: mpsc::UnboundedReceiver<Seen>,
    pub(crate) send: mpsc::UnboundedSender<Value>,
}

/// Starts the line on a thread of its own.
pub(crate) fn open(home: Home) -> ScreenLink {
    let (seen_tx, seen) = mpsc::unbounded_channel();
    let (send, mut sent) = mpsc::unbounded_channel::<Value>();
    thread::spawn(move || {
        let Ok(runtime) = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
        else {
            return;
        };
        runtime.block_on(async move {
            loop {
                let ended = run(&home, &seen_tx, &mut sent).await;
                if seen_tx.send(Seen::Down).is_err() || ended.is_none() {
                    return;
                }
                time::sleep(RETRY).await;
            }
        });
    });
    ScreenLink { seen, send }
}

async fn run(
    home: &Home,
    seen: &mpsc::UnboundedSender<Seen>,
    sent: &mut mpsc::UnboundedReceiver<Value>,
) -> Option<()> {
    let Ok(door) = Door::read(&home.screen()) else {
        return Some(());
    };
    let Ok(Ok(stream)) =
        time::timeout(CONNECT_WAIT, TcpStream::connect(("127.0.0.1", door.port))).await
    else {
        return Some(());
    };
    let (read, mut write) = stream.into_split();
    let hello = serde_json::json!({ "token": door.token }).to_string();
    if write
        .write_all(format!("{hello}\n").as_bytes())
        .await
        .is_err()
    {
        return Some(());
    }
    let (lines_tx, mut lines) = mpsc::channel::<String>(4);
    let reader = tokio::spawn(async move {
        let mut read = BufReader::new(read);
        loop {
            let mut bytes = Vec::new();
            match (&mut read).take(LINE).read_until(b'\n', &mut bytes).await {
                Ok(0) | Err(_) => return,
                Ok(_) => {
                    if lines_tx
                        .send(String::from_utf8_lossy(&bytes).into_owned())
                        .await
                        .is_err()
                    {
                        return;
                    }
                }
            }
        }
    });
    let ended = loop {
        tokio::select! {
            line = lines.recv() => {
                let Some(line) = line else { break Some(()) };
                if let Some(item) = understood(line.trim())
                    && seen.send(item).is_err()
                {
                    break None;
                }
            }
            out = sent.recv() => {
                let Some(out) = out else { break None };
                if write.write_all(format!("{out}\n").as_bytes()).await.is_err() {
                    break Some(());
                }
            }
        }
    };
    reader.abort();
    ended
}

/// What one line from the screen means for the window. A frame is decoded here.
pub(crate) fn understood(line: &str) -> Option<Seen> {
    let value: Value = serde_json::from_str(line).ok()?;
    match value.get("kind").and_then(Value::as_str)? {
        "state" => Some(Seen::State(value)),
        "pointer" => Some(Seen::Pointer {
            x: value.get("x").and_then(Value::as_f64)?,
            y: value.get("y").and_then(Value::as_f64)?,
        }),
        "frame" => {
            let jpeg = unbase64(value.get("jpeg").and_then(Value::as_str)?)?;
            let decoded =
                image::load_from_memory_with_format(&jpeg, image::ImageFormat::Jpeg).ok()?;
            let mut rgba = decoded.into_rgba8();
            let (width, height) = rgba.dimensions();
            for pixel in rgba.as_chunks_mut::<4>().0 {
                pixel.swap(0, 2);
            }
            let size = |name: &str, fallback: u32| {
                value
                    .get(name)
                    .and_then(Value::as_f64)
                    .filter(|size| *size > 0.0)
                    .unwrap_or(f64::from(fallback))
            };
            Some(Seen::Frame(Picture {
                width,
                height,
                bgra: rgba.into_raw(),
                page_width: size("width", width),
                page_height: size("height", height),
            }))
        }
        _ => None,
    }
}

pub(crate) fn unbase64(text: &str) -> Option<Vec<u8>> {
    let mut out = Vec::with_capacity(text.len() * 3 / 4);
    let mut bits = 0u32;
    let mut held = 0;
    for byte in text.bytes() {
        let value = match byte {
            b'A'..=b'Z' => byte - b'A',
            b'a'..=b'z' => byte - b'a' + 26,
            b'0'..=b'9' => byte - b'0' + 52,
            b'+' => 62,
            b'/' => 63,
            b'=' | b'\n' | b'\r' => continue,
            _ => return None,
        };
        bits = (bits << 6) | u32::from(value);
        held += 6;
        if held >= 8 {
            held -= 8;
            out.push((bits >> held) as u8);
            bits &= (1 << held) - 1;
        }
    }
    Some(out)
}

#[cfg(test)]
#[path = "screen_link.test.rs"]
mod tests;
