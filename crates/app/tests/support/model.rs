//! A fake OpenAI-compatible model that plays one fixed conversation, for the end-to-end test and
//! the demo an agent runs to look at the windows.

use std::net::SocketAddr;

use serde_json::Value;
use serde_json::json;
use tokio::io::AsyncReadExt;
use tokio::io::AsyncWriteExt;
use tokio::net::TcpListener;
use tokio::net::TcpStream;

/// What the fake model asks for, and the command it asks to run.
#[derive(Clone, Debug)]
pub struct Script {
    pub command: String,
    pub think: std::time::Duration,
}

/// Set to a file path to keep each last message and answer there, one JSON object per line.
pub const LOG: &str = "DOTORI_FAKE_MODEL_LOG";
/// The day the reminder is set for. Far ahead, so it never comes due while the script runs.
pub const REMINDER_DATE: &str = "2099-12-31";
/// The reply after the reminder is set.
pub const REMINDED: &str = "알겠어요. 내일 아침 8시에 치과 예약을 알려 드릴게요.";
/// The reply after the week is searched.
pub const LISTED: &str = "이번 주 일정이에요.\n\n- **화 10:00** 디자인 리뷰\n- **수 13:00** 민지 님과 점심\n- **목 10:30** 치과, 아침 8시에 알려 드려요";
/// The reply after the command ran.
pub const RAN: &str = "실행했어요.";
/// The reply after the owner declined the command.
pub const SKIPPED: &str = "알겠어요, 실행하지 않을게요.";
/// The reply to anything else.
pub const OTHER: &str = "알겠어요. 바로 확인해 볼게요.";

/// Serves the script on a free local port until the runtime ends.
pub async fn start(script: Script) -> SocketAddr {
    let listener = TcpListener::bind(("127.0.0.1", 0)).await.expect("bound");
    let address = listener.local_addr().expect("an address");
    tokio::spawn(async move {
        while let Ok((stream, _)) = listener.accept().await {
            let script = script.clone();
            tokio::spawn(async move {
                let _ = serve(stream, &script).await;
            });
        }
    });
    address
}

async fn serve(mut stream: TcpStream, script: &Script) -> std::io::Result<()> {
    let mut read = Vec::new();
    let mut buffer = [0u8; 8192];
    let body = loop {
        let got = stream.read(&mut buffer).await?;
        if got == 0 {
            return Ok(());
        }
        read.extend_from_slice(&buffer[..got]);
        if let Some(end) = read.windows(4).position(|window| window == b"\r\n\r\n") {
            let head = String::from_utf8_lossy(&read[..end]).to_lowercase();
            let length: usize = head
                .lines()
                .find_map(|line| line.strip_prefix("content-length:"))
                .and_then(|value| value.trim().parse().ok())
                .unwrap_or(0);
            if read.len() >= end + 4 + length {
                break read[end + 4..end + 4 + length].to_vec();
            }
        }
    };
    let asked: Value = serde_json::from_slice(&body).unwrap_or(Value::Null);
    tokio::time::sleep(script.think).await;
    let message = answer(&asked, script);
    if let Some(log) = std::env::var_os(LOG) {
        let line = json!({ "last": asked["messages"].as_array().and_then(|all| all.last()), "answer": message });
        let _ = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(log)
            .and_then(|mut file| {
                std::io::Write::write_all(&mut file, format!("{line}\n").as_bytes())
            });
    }
    let out = json!({
        "id": "fake",
        "object": "chat.completion",
        "choices": [{ "index": 0, "message": message, "finish_reason": "stop" }],
        "usage": { "prompt_tokens": 1, "completion_tokens": 1 }
    })
    .to_string();
    let response = format!(
        "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{out}",
        out.len()
    );
    stream.write_all(response.as_bytes()).await
}

/// The next message of the script, given the conversation so far.
pub fn answer(asked: &Value, script: &Script) -> Value {
    let messages = asked["messages"].as_array().cloned().unwrap_or_default();
    let owner = messages
        .iter()
        .flat_map(|message| heard(&text_of(message)))
        .filter(|said| said.channel == "app")
        .map(|said| said.thread)
        .next_back();
    let Some(last) = messages.last() else {
        return done();
    };
    let to_owner = |text: &str| match owner {
        Some(thread) => call("say", json!({ "thread": thread, "text": text })),
        None => done(),
    };
    match last["role"].as_str() {
        Some("tool") => {
            let called = messages
                .iter()
                .rev()
                .find_map(|message| message["tool_calls"].as_array()?.last().cloned())
                .and_then(|call| call["function"]["name"].as_str().map(str::to_owned));
            match called.as_deref() {
                Some("routine_add") => to_owner(REMINDED),
                Some("memory_search") => to_owner(LISTED),
                _ => done(),
            }
        }
        Some("user") => {
            let text = text_of(last);
            if text.contains("<decided_call") {
                if text.contains("declined") {
                    return to_owner(SKIPPED);
                }
                return to_owner(RAN);
            }
            let Some(said) = heard(&text).pop() else {
                return done();
            };
            if said.text.contains("치과") {
                return call(
                    "routine_add",
                    json!({
                        "name": "치과 알림",
                        "task": "오전 10시 30분 치과 예약을 알려 주기",
                        "time": "08:00",
                        "date": REMINDER_DATE
                    }),
                );
            }
            if said.text.contains("일정") {
                return call("memory_search", json!({ "query": "이번 주 일정" }));
            }
            if said.text.contains("실행") || said.text.contains("옮겨") {
                return call("run_shell", json!({ "command": script.command }));
            }
            call("say", json!({ "thread": said.thread, "text": OTHER }))
        }
        _ => done(),
    }
}

fn done() -> Value {
    json!({ "role": "assistant", "content": null })
}

fn call(name: &str, arguments: Value) -> Value {
    json!({
        "role": "assistant",
        "content": null,
        "tool_calls": [{
            "id": format!("call_{name}"),
            "type": "function",
            "function": { "name": name, "arguments": arguments.to_string() }
        }]
    })
}

fn text_of(message: &Value) -> String {
    match &message["content"] {
        Value::String(text) => text.clone(),
        Value::Array(parts) => parts
            .iter()
            .filter_map(|part| part["text"].as_str())
            .collect::<Vec<_>>()
            .join(" "),
        _ => String::new(),
    }
}

struct Heard {
    thread: u64,
    channel: String,
    text: String,
}

fn heard(text: &str) -> Vec<Heard> {
    let mut found = Vec::new();
    let mut rest = text;
    while let Some(at) = rest.find("<message ") {
        rest = &rest[at + "<message ".len()..];
        let Some(open) = rest.find(">\n") else {
            break;
        };
        let tag = &rest[..open];
        let attribute = |name: &str| {
            let key = format!("{name}=\"");
            let start = tag.find(&key)? + key.len();
            let end = tag[start..].find('"')? + start;
            Some(tag[start..end].to_owned())
        };
        let body = &rest[open + 2..];
        let said = body.split("\n</message>").next().unwrap_or(body).to_owned();
        if let Some(thread) = attribute("thread").and_then(|thread| thread.parse().ok()) {
            found.push(Heard {
                thread,
                channel: attribute("channel").unwrap_or_default(),
                text: said,
            });
        }
    }
    found
}
