//! The tools of an MCP server over Streamable HTTP, as core's toolbox.

use std::fmt;
use std::sync::atomic::AtomicU64;
use std::sync::atomic::Ordering;
use std::time::Duration;

use dotori_core::BoxFuture;
use dotori_core::tool::ToolCall;
use dotori_core::tool::ToolError;
use dotori_core::tool::ToolSpec;
use dotori_core::tool::Toolbox;
use reqwest::Client;
use reqwest::header::ACCEPT;
use reqwest::header::CONTENT_TYPE;
use serde_json::Value;
use serde_json::json;

const VERSION: &str = "2025-11-25";
const SESSION: &str = "mcp-session-id";
const PROTOCOL: &str = "mcp-protocol-version";
const TIMEOUT: Duration = Duration::from_secs(300);

/// An MCP server's tools. The list is read once when connecting.
pub struct McpTools {
    client: Client,
    url: String,
    token: String,
    session: Option<String>,
    version: String,
    specs: Vec<ToolSpec>,
    next_id: AtomicU64,
}

impl fmt::Debug for McpTools {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("McpTools")
            .field("url", &self.url)
            .finish_non_exhaustive()
    }
}

impl McpTools {
    /// Starts a session with the server at `url` and reads its tools.
    pub async fn connect(url: impl Into<String>, token: impl Into<String>) -> Result<Self, String> {
        let client = Client::builder()
            .timeout(TIMEOUT)
            .no_proxy()
            .build()
            .map_err(|error| error.to_string())?;
        let mut tools = Self {
            client,
            url: url.into(),
            token: token.into(),
            session: None,
            version: String::from(VERSION),
            specs: Vec::new(),
            next_id: AtomicU64::new(1),
        };
        let (reply, session) = tools
            .request(
                "initialize",
                json!({
                    "protocolVersion": VERSION,
                    "capabilities": {},
                    "clientInfo": { "name": "dotori", "version": env!("CARGO_PKG_VERSION") },
                }),
            )
            .await?;
        tools.session = session;
        if let Some(version) = reply.get("protocolVersion").and_then(Value::as_str) {
            tools.version = version.to_owned();
        }
        tools.notify("notifications/initialized").await?;
        let (listed, _) = tools.request("tools/list", json!({})).await?;
        tools.specs = specs(&listed)?;
        Ok(tools)
    }

    async fn request(
        &self,
        method: &str,
        params: Value,
    ) -> Result<(Value, Option<String>), String> {
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let body = json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params });
        let response = self
            .post(body)
            .send()
            .await
            .map_err(|error| format!("MCP server unreachable: {error}"))?;
        let status = response.status();
        let session = response
            .headers()
            .get(SESSION)
            .and_then(|value| value.to_str().ok())
            .map(str::to_owned);
        let kind = response
            .headers()
            .get(CONTENT_TYPE)
            .and_then(|value| value.to_str().ok())
            .unwrap_or_default()
            .to_owned();
        let text = response
            .text()
            .await
            .map_err(|error| format!("MCP reply unreadable: {error}"))?;
        if !status.is_success() {
            return Err(format!("MCP server answered {status} to {method}"));
        }
        let message = if kind.starts_with("text/event-stream") {
            from_events(&text, id)
        } else {
            serde_json::from_str(&text).ok()
        }
        .ok_or_else(|| format!("MCP server gave no answer to {method}"))?;
        if let Some(error) = message.get("error") {
            let detail = error
                .get("message")
                .and_then(Value::as_str)
                .unwrap_or("an error");
            return Err(format!("MCP {method} failed: {detail}"));
        }
        let result = message.get("result").cloned().unwrap_or(Value::Null);
        Ok((result, session))
    }

    async fn notify(&self, method: &str) -> Result<(), String> {
        let response = self
            .post(json!({ "jsonrpc": "2.0", "method": method }))
            .send()
            .await
            .map_err(|error| format!("MCP server unreachable: {error}"))?;
        if response.status().is_success() {
            Ok(())
        } else {
            Err(format!(
                "MCP server answered {} to {method}",
                response.status()
            ))
        }
    }

    fn post(&self, body: Value) -> reqwest::RequestBuilder {
        let mut request = self
            .client
            .post(&self.url)
            .bearer_auth(&self.token)
            .header(ACCEPT, "application/json, text/event-stream")
            .header(CONTENT_TYPE, "application/json")
            .header(PROTOCOL, &self.version)
            .body(body.to_string());
        if let Some(session) = &self.session {
            request = request.header(SESSION, session);
        }
        request
    }

    async fn call(&self, call: ToolCall) -> Result<String, ToolError> {
        let arguments: Value = serde_json::from_str(&call.input).map_err(|error| ToolError {
            detail: format!("the input is not JSON: {error}"),
        })?;
        let (result, _) = self
            .request(
                "tools/call",
                json!({ "name": call.name, "arguments": arguments }),
            )
            .await
            .map_err(|detail| ToolError { detail })?;
        let text = result
            .get("content")
            .and_then(Value::as_array)
            .map(|content| {
                content
                    .iter()
                    .filter_map(|part| match part.get("type").and_then(Value::as_str) {
                        Some("text") => part.get("text").and_then(Value::as_str).map(str::to_owned),
                        Some(other) => Some(format!("[{other} content left out]")),
                        None => None,
                    })
                    .collect::<Vec<_>>()
                    .join("\n")
            })
            .unwrap_or_default();
        if result
            .get("isError")
            .and_then(Value::as_bool)
            .unwrap_or(false)
        {
            Err(ToolError { detail: text })
        } else {
            Ok(text)
        }
    }
}

impl Toolbox for McpTools {
    fn specs(&self) -> Vec<ToolSpec> {
        self.specs.clone()
    }

    fn run(&self, call: ToolCall) -> BoxFuture<'_, Result<String, ToolError>> {
        Box::pin(self.call(call))
    }
}

fn specs(listed: &Value) -> Result<Vec<ToolSpec>, String> {
    let tools = listed
        .get("tools")
        .and_then(Value::as_array)
        .ok_or("MCP tools/list gave no tools")?;
    Ok(tools
        .iter()
        .filter_map(|tool| {
            Some(ToolSpec {
                name: tool.get("name")?.as_str()?.to_owned(),
                description: tool
                    .get("description")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_owned(),
                input_schema: tool
                    .get("inputSchema")
                    .cloned()
                    .unwrap_or_else(|| json!({ "type": "object" }))
                    .to_string(),
            })
        })
        .collect())
}

/// The response with `id` among the events of a server that answers with a stream.
pub(crate) fn from_events(text: &str, id: u64) -> Option<Value> {
    let mut data = String::new();
    for line in text.lines().chain(std::iter::once("")) {
        if let Some(rest) = line.strip_prefix("data:") {
            if !data.is_empty() {
                data.push('\n');
            }
            data.push_str(rest.strip_prefix(' ').unwrap_or(rest));
        } else if line.is_empty() && !data.is_empty() {
            if let Ok(message) = serde_json::from_str::<Value>(&data)
                && message.get("id").and_then(Value::as_u64) == Some(id)
            {
                return Some(message);
            }
            data.clear();
        }
    }
    None
}

#[cfg(test)]
#[path = "mcp.test.rs"]
mod tests;
