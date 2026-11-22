use std::convert::Infallible;
use std::sync::Arc;
use std::time::Duration;

use http_body_util::BodyExt;
use http_body_util::Full;
use http_body_util::Limited;
use hyper::Method;
use hyper::Request;
use hyper::Response;
use hyper::StatusCode;
use hyper::body::Bytes;
use hyper::body::Incoming;
use hyper::header::AUTHORIZATION;
use hyper::header::CONTENT_TYPE;
use hyper::header::ORIGIN;
use hyper::server::conn::http1;
use hyper::service::service_fn;
use hyper_util::rt::TokioIo;
use hyper_util::rt::TokioTimer;
use serde_json::Value;
use serde_json::json;
use tokio::net::TcpListener;
use tokio::sync::Semaphore;
use tokio::time;

use crate::Tool;

/// The only path the server answers on.
pub const PATH: &str = "/mcp";
/// The protocol versions the server speaks, newest first.
pub const VERSIONS: [&str; 2] = ["2025-11-25", "2025-06-18"];
const PROTOCOL: &str = "mcp-protocol-version";
const MAX_BODY: usize = 4 * 1024 * 1024;
const MAX_CONNECTIONS: usize = 64;
const HEADER_WAIT: Duration = Duration::from_secs(10);
const BODY_WAIT: Duration = Duration::from_secs(30);
const ACCEPT_PAUSE: Duration = Duration::from_millis(50);
const PARSE_ERROR: i64 = -32700;
const INVALID_REQUEST: i64 = -32600;
const NO_METHOD: i64 = -32601;
const INVALID_PARAMS: i64 = -32602;

/// The tools and the token a client must send as a bearer token.
pub struct Server {
    token: String,
    tools: Vec<Box<dyn Tool>>,
}

impl std::fmt::Debug for Server {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Server").finish_non_exhaustive()
    }
}

impl Server {
    pub fn new(token: impl Into<String>, tools: Vec<Box<dyn Tool>>) -> Self {
        Self {
            token: token.into(),
            tools,
        }
    }

    /// Serves every connection on `listener` until the future is dropped.
    pub async fn serve(self, listener: TcpListener) {
        let server = Arc::new(self);
        let open = Arc::new(Semaphore::new(MAX_CONNECTIONS));
        loop {
            let Ok(permit) = Arc::clone(&open).acquire_owned().await else {
                return;
            };
            let stream = match listener.accept().await {
                Ok((stream, _)) => stream,
                Err(_) => {
                    time::sleep(ACCEPT_PAUSE).await;
                    continue;
                }
            };
            let server = Arc::clone(&server);
            tokio::spawn(async move {
                let _permit = permit;
                let service = service_fn(move |request| {
                    let server = Arc::clone(&server);
                    async move { Ok::<_, Infallible>(server.handle(request).await) }
                });
                let _ = http1::Builder::new()
                    .timer(TokioTimer::new())
                    .header_read_timeout(HEADER_WAIT)
                    .serve_connection(TokioIo::new(stream), service)
                    .await;
            });
        }
    }

    async fn handle(&self, request: Request<Incoming>) -> Response<Full<Bytes>> {
        if request.uri().path() != PATH {
            return plain(StatusCode::NOT_FOUND);
        }
        let bearer = format!("Bearer {}", self.token);
        if request
            .headers()
            .get(AUTHORIZATION)
            .map(|value| value.as_bytes())
            != Some(bearer.as_bytes())
        {
            return plain(StatusCode::UNAUTHORIZED);
        }
        if let Some(origin) = request.headers().get(ORIGIN)
            && !local_origin(origin.to_str().unwrap_or_default())
        {
            return plain(StatusCode::FORBIDDEN);
        }
        if request.method() != Method::POST {
            return plain(StatusCode::METHOD_NOT_ALLOWED);
        }
        if let Some(version) = request.headers().get(PROTOCOL)
            && !VERSIONS
                .iter()
                .any(|known| version.as_bytes() == known.as_bytes())
        {
            return plain(StatusCode::BAD_REQUEST);
        }
        let read = time::timeout(
            BODY_WAIT,
            Limited::new(request.into_body(), MAX_BODY).collect(),
        );
        let body = match read.await {
            Ok(Ok(body)) => body.to_bytes(),
            Ok(Err(_)) => return plain(StatusCode::PAYLOAD_TOO_LARGE),
            Err(_) => return plain(StatusCode::REQUEST_TIMEOUT),
        };
        match self.answer(&body).await {
            Some(reply) => Response::builder()
                .status(StatusCode::OK)
                .header(CONTENT_TYPE, "application/json")
                .body(Full::new(Bytes::from(reply.to_string())))
                .unwrap_or_else(|_| plain(StatusCode::INTERNAL_SERVER_ERROR)),
            None => plain(StatusCode::ACCEPTED),
        }
    }

    /// The reply to one JSON-RPC message, or `None` for a notification or a response.
    pub async fn answer(&self, body: &[u8]) -> Option<Value> {
        let Ok(message) = serde_json::from_slice::<Value>(body) else {
            return Some(failure(Value::Null, PARSE_ERROR, "not JSON"));
        };
        let Some(method) = message.get("method").and_then(Value::as_str) else {
            return message
                .get("id")
                .is_none()
                .then(|| failure(Value::Null, INVALID_REQUEST, "not a request"));
        };
        let id = message.get("id")?.clone();
        let params = message.get("params").cloned().unwrap_or(Value::Null);
        let result = match method {
            "initialize" => Ok(initialize(&params)),
            "ping" => Ok(json!({})),
            "tools/list" => Ok(json!({ "tools": self.list() })),
            "tools/call" => self.call(&params).await,
            _ => Err((NO_METHOD, format!("no method `{method}`"))),
        };
        Some(match result {
            Ok(result) => json!({ "jsonrpc": "2.0", "id": id, "result": result }),
            Err((code, detail)) => failure(id, code, &detail),
        })
    }

    fn list(&self) -> Vec<Value> {
        self.tools
            .iter()
            .map(|tool| {
                json!({
                    "name": tool.name(),
                    "description": tool.description(),
                    "inputSchema": tool.input_schema(),
                })
            })
            .collect()
    }

    async fn call(&self, params: &Value) -> Result<Value, (i64, String)> {
        let name = params
            .get("name")
            .and_then(Value::as_str)
            .ok_or((INVALID_PARAMS, String::from("a tool call needs a name")))?;
        let tool = self
            .tools
            .iter()
            .find(|tool| tool.name() == name)
            .ok_or_else(|| (INVALID_PARAMS, format!("no tool named `{name}`")))?;
        let arguments = params
            .get("arguments")
            .cloned()
            .unwrap_or_else(|| json!({}));
        let (text, failed) = match tool.call(arguments).await {
            Ok(text) => (text, false),
            Err(text) => (text, true),
        };
        Ok(json!({ "content": [{ "type": "text", "text": text }], "isError": failed }))
    }
}

fn initialize(params: &Value) -> Value {
    let asked = params.get("protocolVersion").and_then(Value::as_str);
    let version = asked
        .filter(|asked| VERSIONS.contains(asked))
        .unwrap_or(VERSIONS[0]);
    json!({
        "protocolVersion": version,
        "capabilities": { "tools": {} },
        "serverInfo": { "name": "dotori", "version": env!("CARGO_PKG_VERSION") },
    })
}

fn failure(id: Value, code: i64, message: &str) -> Value {
    json!({ "jsonrpc": "2.0", "id": id, "error": { "code": code, "message": message } })
}

fn plain(status: StatusCode) -> Response<Full<Bytes>> {
    let mut response = Response::new(Full::new(Bytes::new()));
    *response.status_mut() = status;
    response
}

fn local_origin(origin: &str) -> bool {
    let authority = origin
        .split_once("://")
        .map_or(origin, |(_, rest)| rest)
        .split('/')
        .next()
        .unwrap_or_default();
    ["localhost", "127.0.0.1", "[::1]"].iter().any(|host| {
        authority == *host
            || authority
                .strip_prefix(host)
                .is_some_and(|port| port.starts_with(':'))
    })
}

#[cfg(test)]
#[path = "server.test.rs"]
mod tests;
