//! The MCP server process. The daemon starts it as a child, reads its address from the first
//! line it prints, and stops it by closing its input.

use std::env;
use std::fmt;
use std::process::Stdio;
use std::sync::Arc;
use std::time::Duration;

use dotori_connectors::sub_session::cli::McpAddress;
use dotori_connectors::sub_session::cli::McpServer;
use dotori_connectors::tool::mcp::McpTools;
use dotori_core::BoxFuture;
use dotori_core::tool::ToolCall;
use dotori_core::tool::ToolError;
use dotori_core::tool::ToolSpec;
use dotori_core::tool::Toolbox;
use dotori_mcp::Tool;
use dotori_mcp::browser::Browser;
use dotori_mcp::browser::BrowserConfig;
use dotori_mcp::browser::Logins;
use dotori_mcp::fetch::WebFetch;
use dotori_mcp::memory::Memory;
use dotori_mcp::memory::MemoryConfig;
use dotori_mcp::relay::relay;
use dotori_mcp::screen::Screen;
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
use tokio::sync::Mutex;
use tokio::time;

use crate::accounts::VaultAccounts;
use crate::door;
use crate::door::Door;
use crate::home::Home;
use crate::settings::Settings;
use crate::vault;
use crate::vault::Vault;

const START_WAIT: Duration = Duration::from_secs(30);

/// Where the MCP server listens, as its first line of output: the tools every session and coding
/// agent shares, and on a port and token of their own, which only the daemon reads here, the
/// browser tools for the main session alone.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct Listening {
    pub(crate) port: u16,
    pub(crate) token: String,
    pub(crate) owner_port: u16,
    pub(crate) owner_token: String,
}

/// `dotori mcp`: serves the shared tools until its input closes.
/// Whether this PC holds `job` now, as the daemon wrote. Without the file, this PC alone does
/// everything.
pub(crate) fn holds_job(jobs: &std::path::Path, job: &str) -> bool {
    std::fs::read_to_string(jobs)
        .ok()
        .and_then(|text| serde_json::from_str::<serde_json::Value>(&text).ok())
        .and_then(|jobs| jobs.get(job).and_then(serde_json::Value::as_bool))
        .unwrap_or(true)
}

pub(crate) async fn serve(home: Home) -> Result<(), String> {
    crate::daemon::keep_to_itself();
    let settings = Settings::load(&home.settings())?;
    let vault = Arc::new(Vault::new(&home.root));
    let chosen = settings.browser.clone().unwrap_or_default();
    let (screen, asks) = Screen::new();
    let browser = Browser::with_screen(
        BrowserConfig {
            executable: chosen.path,
            downloads: home.chrome(),
            profile: home.browser(),
            hidden_env: vault::secret_names(&settings, &vault),
            logins: Some(Logins {
                out: home.logins_out(),
                into: home.logins_in(),
            }),
            accounts: Some(Arc::new(VaultAccounts {
                vault: Arc::clone(&vault),
            })),
        },
        Arc::clone(&screen),
        asks,
    );
    {
        let jobs = home.jobs();
        tokio::spawn(
            Arc::clone(&browser)
                .share_logins(move || holds_job(&jobs, "browser") && holds_job(&jobs, "linked")),
        );
    }
    let shown = TcpListener::bind(("127.0.0.1", 0))
        .await
        .map_err(|error| error.to_string())?;
    let screen_door = Door {
        port: shown
            .local_addr()
            .map_err(|error| error.to_string())?
            .port(),
        token: door::token()?,
    };
    screen_door.write(&home.screen())?;
    tokio::spawn(screen.serve(shown, screen_door.token));
    let mut tools: Vec<Box<dyn Tool>> = vec![Box::new(WebFetch::new()?)];
    tools.extend(browser.tools());
    let embedding = settings
        .embedding(vault::lookup(&vault))
        .unwrap_or_else(|error| {
            eprintln!("dotori: memory search matches words only: {error}");
            None
        });
    let memory = Memory::new(MemoryConfig {
        record: home.record(),
        index: home.search_index(),
        dictionaries: home.dictionaries(),
        embedding,
        jobs: Some(home.jobs()),
    });
    tools.extend(memory.tools());
    let (relays, wrong) = settings.relays(
        vault::lookup(&vault),
        false,
        &vault::secret_names(&settings, &vault),
    );
    let (relayed, failed) = relay(relays).await;
    for problem in wrong.iter().chain(&failed) {
        eprintln!("dotori: an MCP server is left out: {problem}");
    }
    tools.extend(relayed);
    let token = door::token()?;
    let listener = TcpListener::bind(("127.0.0.1", 0))
        .await
        .map_err(|error| error.to_string())?;
    let port = listener
        .local_addr()
        .map_err(|error| error.to_string())?
        .port();
    let owner_token = door::token()?;
    let owner_listener = TcpListener::bind(("127.0.0.1", 0))
        .await
        .map_err(|error| error.to_string())?;
    let owner_port = owner_listener
        .local_addr()
        .map_err(|error| error.to_string())?
        .port();
    let line = serde_json::to_string(&Listening {
        port,
        token: token.clone(),
        owner_port,
        owner_token: owner_token.clone(),
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
        () = Server::new(owner_token, browser.owner_tools()).serve(owner_listener) => {}
        _ = input.read_to_end(&mut sink) => {}
    }
    Ok(())
}

/// The shared tools of a `dotori mcp` child. A child that died is started again before the next
/// call, so a crash costs one restart rather than every later call.
pub(crate) struct SharedTools {
    specs: Vec<ToolSpec>,
    owner_specs: Vec<ToolSpec>,
    running: Mutex<Running>,
}

struct Running {
    child: Child,
    tools: Arc<McpTools>,
    owner: Arc<McpTools>,
    address: McpAddress,
}

/// The browser tools of the `dotori mcp` child that only the main session gets, reached on the
/// child's second port with a token no coding agent is given.
pub(crate) struct OwnerTools {
    shared: Arc<SharedTools>,
}

impl Toolbox for OwnerTools {
    fn specs(&self) -> Vec<ToolSpec> {
        self.shared.owner_specs.clone()
    }

    fn run(&self, call: ToolCall) -> BoxFuture<'_, Result<String, ToolError>> {
        Box::pin(async move {
            let owner = self
                .shared
                .current()
                .await
                .map_err(|detail| ToolError { detail })?
                .2;
            owner.run(call).await
        })
    }
}

impl fmt::Debug for SharedTools {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SharedTools").finish_non_exhaustive()
    }
}

impl SharedTools {
    pub(crate) async fn start() -> Result<Self, String> {
        let running = launch().await?;
        Ok(Self {
            specs: running.tools.specs(),
            owner_specs: running.owner.specs(),
            running: Mutex::new(running),
        })
    }

    /// The tools only the main session gets.
    pub(crate) fn owner(self: &Arc<Self>) -> Arc<dyn Toolbox> {
        Arc::new(OwnerTools {
            shared: Arc::clone(self),
        })
    }

    async fn current(&self) -> Result<(Arc<McpTools>, McpAddress, Arc<McpTools>), String> {
        let mut running = self.running.lock().await;
        if !matches!(running.child.try_wait(), Ok(None)) {
            *running = launch().await.map_err(|error| {
                format!("the tool server stopped and did not start again: {error}")
            })?;
        }
        Ok((
            Arc::clone(&running.tools),
            running.address.clone(),
            Arc::clone(&running.owner),
        ))
    }

    async fn call(&self, call: ToolCall) -> Result<String, ToolError> {
        let (tools, _, _) = self
            .current()
            .await
            .map_err(|detail| ToolError { detail })?;
        tools.run(call).await
    }
}

impl McpServer for SharedTools {
    fn address(&self) -> BoxFuture<'_, Result<McpAddress, String>> {
        Box::pin(async { self.current().await.map(|(_, address, _)| address) })
    }
}

impl Toolbox for SharedTools {
    fn specs(&self) -> Vec<ToolSpec> {
        self.specs.clone()
    }

    fn run(&self, call: ToolCall) -> BoxFuture<'_, Result<String, ToolError>> {
        Box::pin(self.call(call))
    }
}

async fn launch() -> Result<Running, String> {
    if cfg!(test) {
        return Err(String::from("tests never start the MCP server"));
    }
    let exe = env::current_exe().map_err(|error| error.to_string())?;
    let mut child = Command::new(exe)
        .arg("mcp")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .map_err(|error| format!("cannot start the MCP server: {error}"))?;
    let stdout = child.stdout.take().ok_or("the MCP server has no output")?;
    let mut reader = BufReader::new(stdout);
    let mut first = String::new();
    let read = time::timeout(START_WAIT, reader.read_line(&mut first))
        .await
        .map_err(|_| String::from("the MCP server did not start in time"))?
        .map_err(|error| error.to_string())?;
    if read == 0 {
        return Err(String::from("the MCP server ended at once"));
    }
    tokio::spawn(async move {
        let _ = tokio::io::copy(&mut reader, &mut tokio::io::sink()).await;
    });
    let listening: Listening = serde_json::from_str(first.trim())
        .map_err(|error| format!("the MCP server said `{}`: {error}", first.trim()))?;
    let url = format!("http://127.0.0.1:{}{PATH}", listening.port);
    let address = McpAddress {
        url: url.clone(),
        token: listening.token.clone(),
    };
    let tools = McpTools::connect(url, listening.token).await?;
    let owner = McpTools::connect(
        format!("http://127.0.0.1:{}{PATH}", listening.owner_port),
        listening.owner_token,
    )
    .await?;
    Ok(Running {
        child,
        tools: Arc::new(tools),
        owner: Arc::new(owner),
        address,
    })
}
