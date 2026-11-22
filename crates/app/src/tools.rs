//! The MCP server process. The daemon starts it as a child, reads its address from the first
//! line it prints, and stops it by closing its input.

use std::env;
use std::process::Stdio;
use std::time::Duration;

use dotori_connectors::tool::mcp::McpTools;
use dotori_mcp::Tool;
use dotori_mcp::fetch::WebFetch;
use dotori_mcp::server::PATH;
use dotori_mcp::server::Server;
use serde::Deserialize;
use serde::Serialize;
use tokio::io::AsyncBufReadExt;
use tokio::io::AsyncReadExt;
use tokio::io::AsyncWriteExt;
use tokio::io::BufReader;
use tokio::net::TcpListener;
use tokio::process::Child;
use tokio::process::Command;
use tokio::time;

use crate::door;

const START_WAIT: Duration = Duration::from_secs(10);

/// Where the MCP server listens, as its first line of output.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct Listening {
    pub(crate) port: u16,
    pub(crate) token: String,
}

/// `dotori mcp`: serves the shared tools until its input closes.
pub(crate) async fn serve() -> Result<(), String> {
    let tools: Vec<Box<dyn Tool>> = vec![Box::new(WebFetch::new()?)];
    let token = door::token()?;
    let listener = TcpListener::bind(("127.0.0.1", 0))
        .await
        .map_err(|error| error.to_string())?;
    let port = listener
        .local_addr()
        .map_err(|error| error.to_string())?
        .port();
    let line = serde_json::to_string(&Listening {
        port,
        token: token.clone(),
    })
    .map_err(|error| error.to_string())?;
    let mut out = tokio::io::stdout();
    out.write_all(format!("{line}\n").as_bytes())
        .await
        .map_err(|error| error.to_string())?;
    out.flush().await.map_err(|error| error.to_string())?;
    let mut input = tokio::io::stdin();
    let mut sink = Vec::new();
    tokio::select! {
        () = Server::new(token, tools).serve(listener) => {}
        _ = input.read_to_end(&mut sink) => {}
    }
    Ok(())
}

/// Starts `dotori mcp` and connects to it. The server stops when the returned child is dropped.
pub(crate) async fn start() -> Result<(Child, McpTools), String> {
    let exe = env::current_exe().map_err(|error| error.to_string())?;
    let mut child = Command::new(exe)
        .arg("mcp")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .map_err(|error| format!("cannot start the MCP server: {error}"))?;
    let stdout = child.stdout.take().ok_or("the MCP server has no output")?;
    let first = time::timeout(START_WAIT, BufReader::new(stdout).lines().next_line())
        .await
        .map_err(|_| String::from("the MCP server did not start in time"))?
        .map_err(|error| error.to_string())?
        .ok_or("the MCP server ended at once")?;
    let listening: Listening = serde_json::from_str(&first)
        .map_err(|error| format!("the MCP server said `{first}`: {error}"))?;
    let url = format!("http://127.0.0.1:{}{PATH}", listening.port);
    let tools = McpTools::connect(url, listening.token).await?;
    Ok((child, tools))
}
