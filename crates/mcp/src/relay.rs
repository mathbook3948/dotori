//! MCP servers made elsewhere, relayed through dotori's own server so every session sees one
//! server. A server is reached over Streamable HTTP or started as a command that speaks over its
//! standard input and output. Its tools appear as `<server>__<tool>`.

use std::collections::HashMap;
use std::process::Stdio;
use std::sync::Arc;
use std::sync::atomic::AtomicU64;
use std::sync::atomic::Ordering;
use std::time::Duration;

use reqwest::header::ACCEPT;
use reqwest::header::CONTENT_TYPE;
use serde_json::Value;
use serde_json::json;
use tokio::io::AsyncBufReadExt;
use tokio::io::AsyncReadExt;
use tokio::io::AsyncWriteExt;
use tokio::io::BufReader;
use tokio::process::Child;
use tokio::process::ChildStdin;
use tokio::process::Command;
use tokio::sync::Mutex;
use tokio::sync::oneshot;
use tokio::time;

use crate::Tool;
use crate::ToolFuture;

const VERSION: &str = "2025-11-25";
const SESSION: &str = "mcp-session-id";
const PROTOCOL: &str = "mcp-protocol-version";
const CONNECT_WAIT: Duration = Duration::from_secs(8);
const CALL_WAIT: Duration = Duration::from_secs(120);
const LINE: u64 = 16 * 1024 * 1024;
const PAGES: usize = 20;
const NAME_LONGEST: usize = 64;
const UNSENT_SO_SAFE_TO_RESEND: &str = "not delivered";

/// How to reach one MCP server made elsewhere.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Reach {
    Http { url: String, token: Option<String> },
    Command { program: String, args: Vec<String> },
}

/// One server to relay. Without `tools`, every tool it lists is relayed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RelayConfig {
    pub name: String,
    pub reach: Reach,
    pub tools: Option<Vec<String>>,
}

/// Connects to every server in `configs` at once and returns their tools. A server that cannot
/// be reached in time is left out and named in the second list.
pub async fn relay(configs: Vec<RelayConfig>) -> (Vec<Box<dyn Tool>>, Vec<String>) {
    let tried = futures_util::future::join_all(configs.into_iter().map(|config| async move {
        let name = config.name.clone();
        if relayed_name(&name, "x").is_none() {
            return Err(format!(
                "{name}: a server's name uses only letters, digits, _ and -"
            ));
        }
        match time::timeout(CONNECT_WAIT, Remote::connect(config)).await {
            Ok(Ok(remote)) => Ok(remote),
            Ok(Err(error)) => Err(format!("{name}: {error}")),
            Err(_) => Err(format!("{name}: did not answer in time")),
        }
    }))
    .await;
    let mut tools: Vec<Box<dyn Tool>> = Vec::new();
    let mut failed = Vec::new();
    for remote in tried {
        match remote {
            Ok(remote) => {
                let remote = Arc::new(remote);
                for spec in &remote.specs {
                    if tools.iter().any(|tool| tool.name() == spec.0) {
                        failed.push(format!("{}: another server already has it", spec.0));
                        continue;
                    }
                    tools.push(Box::new(Relayed {
                        remote: Arc::clone(&remote),
                        name: spec.0.clone(),
                        remote_name: spec.1.clone(),
                        description: spec.2.clone(),
                        schema: spec.3.clone(),
                    }));
                }
            }
            Err(error) => failed.push(error),
        }
    }
    (tools, failed)
}

/// The name a relayed tool gets: the server's name and the tool's, joined, in the characters
/// MCP allows and no longer than 64.
pub(crate) fn relayed_name(server: &str, tool: &str) -> Option<String> {
    let name = format!("{server}__{tool}");
    let fits = !server.is_empty()
        && !tool.is_empty()
        && name.len() <= NAME_LONGEST
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-');
    fits.then_some(name)
}

struct Remote {
    config: RelayConfig,
    link: Mutex<Option<Link>>,
    next: AtomicU64,
    specs: Vec<(String, String, String, Value)>,
}

enum Link {
    Http {
        client: reqwest::Client,
        session: Option<String>,
        version: String,
    },
    Command {
        _child: Box<Child>,
        input: ChildStdin,
        waiting: Waiting,
    },
}

type Waiting = Arc<std::sync::Mutex<Option<HashMap<u64, oneshot::Sender<Value>>>>>;

impl Remote {
    async fn connect(config: RelayConfig) -> Result<Self, String> {
        let mut remote = Self {
            config,
            link: Mutex::new(None),
            next: AtomicU64::new(1),
            specs: Vec::new(),
        };
        remote.open().await?;
        let mut listed = Vec::new();
        let mut cursor: Option<String> = None;
        for _ in 0..PAGES {
            let params = match &cursor {
                Some(cursor) => json!({ "cursor": cursor }),
                None => json!({}),
            };
            let page = remote.request("tools/list", params).await?;
            listed.extend(
                page.get("tools")
                    .and_then(Value::as_array)
                    .cloned()
                    .unwrap_or_default(),
            );
            cursor = page
                .get("nextCursor")
                .and_then(Value::as_str)
                .map(str::to_owned);
            if cursor.is_none() {
                break;
            }
        }
        for tool in listed {
            let Some(remote_name) = tool.get("name").and_then(Value::as_str) else {
                continue;
            };
            if let Some(chosen) = &remote.config.tools
                && !chosen.iter().any(|name| name == remote_name)
            {
                continue;
            }
            let Some(name) = relayed_name(&remote.config.name, remote_name) else {
                continue;
            };
            let description = tool
                .get("description")
                .and_then(Value::as_str)
                .unwrap_or_default();
            let schema = tool
                .get("inputSchema")
                .filter(|schema| schema.is_object())
                .cloned()
                .unwrap_or_else(|| json!({ "type": "object" }));
            remote.specs.push((
                name,
                remote_name.to_owned(),
                format!(
                    "{description} (from {}, relayed by dotori)",
                    remote.config.name
                ),
                schema,
            ));
        }
        Ok(remote)
    }

    async fn open(&self) -> Result<(), String> {
        let link = match &self.config.reach {
            Reach::Http { .. } => Link::Http {
                client: reqwest::Client::builder()
                    .timeout(CALL_WAIT)
                    .build()
                    .map_err(|error| error.to_string())?,
                session: None,
                version: String::from(VERSION),
            },
            Reach::Command { program, args } => {
                let mut child = Command::new(program)
                    .args(args)
                    .stdin(Stdio::piped())
                    .stdout(Stdio::piped())
                    .stderr(Stdio::null())
                    .kill_on_drop(true)
                    .spawn()
                    .map_err(|error| format!("cannot start {program}: {error}"))?;
                let input = child.stdin.take().ok_or("the server has no input")?;
                let output = child.stdout.take().ok_or("the server has no output")?;
                let waiting: Waiting = Arc::new(std::sync::Mutex::new(Some(HashMap::new())));
                let answers = Arc::clone(&waiting);
                tokio::spawn(async move {
                    let mut lines = BufReader::new(output);
                    loop {
                        let mut bytes = Vec::new();
                        match (&mut lines).take(LINE).read_until(b'\n', &mut bytes).await {
                            Ok(0) | Err(_) => {
                                answers.lock().expect("unpoisoned").take();
                                return;
                            }
                            Ok(_) => {}
                        }
                        let Ok(message) = serde_json::from_slice::<Value>(&bytes) else {
                            continue;
                        };
                        let Some(id) = message.get("id").and_then(Value::as_u64) else {
                            continue;
                        };
                        if message.get("method").is_some() {
                            continue;
                        }
                        if let Some(waiter) = answers
                            .lock()
                            .expect("unpoisoned")
                            .as_mut()
                            .and_then(|waiting| waiting.remove(&id))
                        {
                            let _ = waiter.send(message);
                        }
                    }
                });
                Link::Command {
                    _child: Box::new(child),
                    input,
                    waiting,
                }
            }
        };
        *self.link.lock().await = Some(link);
        let reply = self
            .request(
                "initialize",
                json!({
                    "protocolVersion": VERSION,
                    "capabilities": {},
                    "clientInfo": { "name": "dotori", "version": env!("CARGO_PKG_VERSION") },
                }),
            )
            .await?;
        if let Some(Link::Http { version, .. }) = self.link.lock().await.as_mut()
            && let Some(agreed) = reply.get("protocolVersion").and_then(Value::as_str)
        {
            *version = agreed.to_owned();
        }
        self.notify("notifications/initialized").await
    }

    async fn notify(&self, method: &str) -> Result<(), String> {
        let message = json!({ "jsonrpc": "2.0", "method": method });
        let mut link = self.link.lock().await;
        match link.as_mut().ok_or("not connected")? {
            Link::Http {
                client,
                session,
                version,
            } => {
                let Reach::Http { url, token } = &self.config.reach else {
                    return Err(String::from("not an HTTP server"));
                };
                let mut request = client
                    .post(url)
                    .header(ACCEPT, "application/json, text/event-stream")
                    .header(CONTENT_TYPE, "application/json")
                    .body(message.to_string())
                    .header(PROTOCOL, version.as_str());
                if let Some(token) = token {
                    request = request.bearer_auth(token);
                }
                if let Some(session) = session {
                    request = request.header(SESSION, session.as_str());
                }
                request.send().await.map_err(|error| error.to_string())?;
                Ok(())
            }
            Link::Command { input, .. } => input
                .write_all(format!("{message}\n").as_bytes())
                .await
                .map_err(|error| error.to_string()),
        }
    }

    async fn request(&self, method: &str, params: Value) -> Result<Value, String> {
        let id = self.next.fetch_add(1, Ordering::Relaxed);
        let message = json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params });
        let answer = {
            let mut link = self.link.lock().await;
            match link.as_mut().ok_or("not connected")? {
                Link::Http {
                    client,
                    session,
                    version,
                } => {
                    let Reach::Http { url, token } = &self.config.reach else {
                        return Err(String::from("not an HTTP server"));
                    };
                    let mut request = client
                        .post(url)
                        .header(ACCEPT, "application/json, text/event-stream")
                        .header(PROTOCOL, version.as_str())
                        .header(CONTENT_TYPE, "application/json")
                        .body(message.to_string());
                    if let Some(token) = token {
                        request = request.bearer_auth(token);
                    }
                    let had_session = session.is_some();
                    if let Some(session) = session.as_ref() {
                        request = request.header(SESSION, session.as_str());
                    }
                    drop(link);
                    let response = request.send().await.map_err(|error| {
                        if error.is_connect() {
                            format!("{UNSENT_SO_SAFE_TO_RESEND}: {error}")
                        } else {
                            error.to_string()
                        }
                    })?;
                    let status = response.status();
                    if let Some(given) = response
                        .headers()
                        .get(SESSION)
                        .and_then(|value| value.to_str().ok())
                        && let Some(Link::Http { session, .. }) = self.link.lock().await.as_mut()
                    {
                        *session = Some(given.to_owned());
                    }
                    let kind = response
                        .headers()
                        .get(CONTENT_TYPE)
                        .and_then(|value| value.to_str().ok())
                        .unwrap_or_default()
                        .to_owned();
                    let text = response.text().await.map_err(|error| error.to_string())?;
                    if status == reqwest::StatusCode::NOT_FOUND && had_session {
                        return Err(format!(
                            "{UNSENT_SO_SAFE_TO_RESEND}: the server forgot this session"
                        ));
                    }
                    if !status.is_success() {
                        return Err(format!("answered {status} to {method}"));
                    }
                    if kind.starts_with("text/event-stream") {
                        from_events(&text, id)
                    } else {
                        serde_json::from_str(&text).ok()
                    }
                    .ok_or_else(|| format!("gave no answer to {method}"))?
                }
                Link::Command { input, waiting, .. } => {
                    let (tx, rx) = oneshot::channel();
                    match waiting.lock().expect("unpoisoned").as_mut() {
                        Some(waiting) => waiting.insert(id, tx),
                        None => {
                            return Err(format!("{UNSENT_SO_SAFE_TO_RESEND}: the server stopped"));
                        }
                    };
                    if let Err(error) = input.write_all(format!("{message}\n").as_bytes()).await {
                        if let Some(waiting) = waiting.lock().expect("unpoisoned").as_mut() {
                            waiting.remove(&id);
                        }
                        return Err(format!(
                            "{UNSENT_SO_SAFE_TO_RESEND}: the server stopped: {error}"
                        ));
                    }
                    let waiting = Arc::clone(waiting);
                    drop(link);
                    let answer = match time::timeout(CALL_WAIT, rx).await {
                        Ok(Ok(answer)) => answer,
                        Ok(Err(_)) => return Err(String::from("the server stopped")),
                        Err(_) => {
                            if let Some(waiting) = waiting.lock().expect("unpoisoned").as_mut() {
                                waiting.remove(&id);
                            }
                            return Err(format!("gave no answer to {method} in time"));
                        }
                    };
                    return answer_of(&answer, method);
                }
            }
        };
        answer_of(&answer, method)
    }

    async fn call(&self, name: &str, arguments: Value) -> Result<String, String> {
        let params = json!({ "name": name, "arguments": arguments });
        let first = self.request("tools/call", params.clone()).await;
        let result = match first {
            Err(error) if error.starts_with(UNSENT_SO_SAFE_TO_RESEND) => {
                self.open()
                    .await
                    .map_err(|again| format!("{error}; reconnecting failed: {again}"))?;
                self.request("tools/call", params).await?
            }
            other => other?,
        };
        let text = result
            .get("content")
            .and_then(Value::as_array)
            .map(|parts| {
                parts
                    .iter()
                    .map(|part| match part.get("type").and_then(Value::as_str) {
                        Some("text") => part
                            .get("text")
                            .and_then(Value::as_str)
                            .unwrap_or_default()
                            .to_owned(),
                        Some(other) => format!("[{other} content]"),
                        None => String::new(),
                    })
                    .collect::<Vec<_>>()
                    .join("\n")
            })
            .unwrap_or_default();
        if result.get("isError").and_then(Value::as_bool) == Some(true) {
            return Err(text);
        }
        Ok(text)
    }
}

fn answer_of(message: &Value, method: &str) -> Result<Value, String> {
    if let Some(error) = message.get("error") {
        let detail = error
            .get("message")
            .and_then(Value::as_str)
            .unwrap_or("an error");
        return Err(format!("{method} failed: {detail}"));
    }
    Ok(message.get("result").cloned().unwrap_or(Value::Null))
}

/// The message with `id` among server-sent events.
pub(crate) fn from_events(text: &str, id: u64) -> Option<Value> {
    text.split("\n\n")
        .filter_map(|event| {
            let data: Vec<&str> = event
                .lines()
                .filter_map(|line| line.strip_prefix("data:"))
                .map(str::trim_start)
                .collect();
            (!data.is_empty()).then(|| data.join("\n"))
        })
        .filter_map(|data| serde_json::from_str::<Value>(&data).ok())
        .find(|message| message.get("id").and_then(Value::as_u64) == Some(id))
}

struct Relayed {
    remote: Arc<Remote>,
    name: String,
    remote_name: String,
    description: String,
    schema: Value,
}

impl Tool for Relayed {
    fn name(&self) -> &str {
        &self.name
    }

    fn description(&self) -> &str {
        &self.description
    }

    fn input_schema(&self) -> Value {
        self.schema.clone()
    }

    fn call(&self, arguments: Value) -> ToolFuture<'_> {
        Box::pin(self.remote.call(&self.remote_name, arguments))
    }
}

#[cfg(test)]
#[path = "relay.test.rs"]
mod tests;
