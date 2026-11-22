use std::fs;
use std::future;
use std::num::NonZeroUsize;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use dotori_connectors::channel::app::AppChannel;
use dotori_connectors::record::jsonl::JsonlStore;
use dotori_connectors::record::sqlite::SqliteStore;
use dotori_connectors::sub_task::cli::CliRunner;
use dotori_connectors::sub_task::cli::CodingAgent;
use dotori_connectors::sub_task::cli::McpServer;
use dotori_connectors::sub_task::llm::LlmRunner;
use dotori_core::record::RecordError;
use dotori_core::record::RecordStore;
use dotori_core::runtime::Agent;
use dotori_core::runtime::AgentConfig;
use dotori_core::runtime::Ports;
use dotori_core::sub_task::SubTaskRunner;
use dotori_core::tool::Toolbox;

use crate::door;
use crate::door::Door;
use crate::home::Home;
use crate::idle::NoTools;
use crate::lock::DaemonLock;
use crate::settings::RunnerChoice;
use crate::settings::Settings;
use crate::settings::environment;
use crate::tools;

const QUEUE: NonZeroUsize = NonZeroUsize::new(256).expect("non-zero");
const SEEN: NonZeroUsize = NonZeroUsize::new(4096).expect("non-zero");
const TOOL_TIMEOUT: Duration = Duration::from_secs(120);
const SUB_TASK_TURNS: usize = 50;
const SUB_TASK_LIMIT: Duration = Duration::from_secs(60 * 60);

/// Runs the agent until Ctrl-C or a terminate signal, with the app channel open for windows.
pub(crate) async fn run(home: Home) -> Result<(), String> {
    serve(home, || {}, future::pending()).await
}

/// Runs the agent like `run`. It calls `ready` once windows can connect, and also stops when
/// `quit` completes.
pub(crate) async fn serve(
    home: Home,
    ready: impl FnOnce(),
    quit: impl Future<Output = ()>,
) -> Result<(), String> {
    let _lock = DaemonLock::take(&home.lock())?;
    let settings = Settings::load(&home.settings())?;
    let model = settings.model(environment)?;
    let connected = settings.channels(environment).unwrap_or_else(|error| {
        eprintln!("dotori: Slack is off: {error}");
        Vec::new()
    });
    let (channels, owners): (Vec<_>, Vec<_>) = connected.into_iter().unzip();
    let shared = match tools::SharedTools::start().await {
        Ok(tools) => Some(Arc::new(tools)),
        Err(error) => {
            eprintln!("dotori: no shared tools: {error}");
            None
        }
    };
    let toolbox: Arc<dyn Toolbox> = match &shared {
        Some(tools) => Arc::clone(tools) as Arc<dyn Toolbox>,
        None => Arc::new(NoTools),
    };
    let sub_tasks = sub_task_runner(&home, &settings, &toolbox, shared)?;
    let records = open_record(&home).await?;
    let token = door::token()?;
    let (app, address) = AppChannel::bind(token.clone()).map_err(|error| error.to_string())?;
    let door = Door {
        port: address.port(),
        token,
    };
    let ports = Ports {
        app: Arc::new(app),
        channels,
        model,
        toolbox,
        sub_tasks,
        records: Arc::new(records),
    };
    let config = AgentConfig {
        owners: owners.into_iter().flatten().collect(),
        queue_capacity: QUEUE,
        seen_capacity: SEEN,
        tool_timeout: TOOL_TIMEOUT,
    };
    let agent = Agent::spawn(config, ports)
        .await
        .map_err(|error| error.to_string())?;
    door.write(&home.door())?;
    eprintln!("dotori is running. Open a window with `dotori chat`.");
    ready();
    let stopped = tokio::select! {
        stopped = stop_signal() => stopped,
        () = quit => Ok(()),
    };
    let _ = fs::remove_file(home.door());
    agent.shutdown().await;
    stopped
}

async fn open_record(home: &Home) -> Result<SqliteStore, String> {
    let store = SqliteStore::new(home.record());
    let old = home.old_record();
    if !old.exists() {
        return Ok(store);
    }
    let fail = |error: RecordError| error.to_string();
    if store.load().await.map_err(fail)?.is_empty() {
        let earlier = JsonlStore::new(old.clone()).load().await.map_err(fail)?;
        store.append(earlier).await.map_err(fail)?;
    }
    fs::rename(&old, old.with_extension("jsonl.moved")).map_err(|error| error.to_string())?;
    Ok(store)
}

fn sub_task_runner(
    home: &Home,
    settings: &Settings,
    toolbox: &Arc<dyn Toolbox>,
    shared: Option<Arc<tools::SharedTools>>,
) -> Result<Arc<dyn SubTaskRunner>, String> {
    let chosen = settings.sub_tasks.clone().unwrap_or_default();
    let agent = match chosen.runner.unwrap_or_default() {
        RunnerChoice::Llm => {
            return Ok(Arc::new(LlmRunner {
                model: settings.sub_task_model(environment)?,
                toolbox: Arc::clone(toolbox),
                max_turns: SUB_TASK_TURNS,
                tool_timeout: TOOL_TIMEOUT,
            }));
        }
        RunnerChoice::ClaudeCode => CodingAgent::ClaudeCode,
        RunnerChoice::Codex => CodingAgent::Codex,
    };
    Ok(Arc::new(CliRunner {
        agent,
        program: chosen
            .path
            .or_else(|| agent.find())
            .unwrap_or_else(|| PathBuf::from(agent.program())),
        folder: home.work(),
        mcp: shared.map(|shared| shared as Arc<dyn McpServer>),
        limit: SUB_TASK_LIMIT,
    }))
}

#[cfg(unix)]
async fn stop_signal() -> Result<(), String> {
    use tokio::signal::unix::SignalKind;
    use tokio::signal::unix::signal;
    let mut terminate = signal(SignalKind::terminate()).map_err(|error| error.to_string())?;
    tokio::select! {
        stopped = tokio::signal::ctrl_c() => stopped.map_err(|error| error.to_string()),
        _ = terminate.recv() => Ok(()),
    }
}

#[cfg(not(unix))]
async fn stop_signal() -> Result<(), String> {
    tokio::signal::ctrl_c()
        .await
        .map_err(|error| error.to_string())
}
