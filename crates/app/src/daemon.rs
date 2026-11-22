use std::fs;
use std::future;
use std::num::NonZeroUsize;
use std::sync::Arc;
use std::time::Duration;

use dotori_connectors::channel::app::AppChannel;
use dotori_connectors::record::jsonl::JsonlStore;
use dotori_core::runtime::Agent;
use dotori_core::runtime::AgentConfig;
use dotori_core::runtime::Ports;
use dotori_core::tool::Toolbox;

use crate::door;
use crate::door::Door;
use crate::home::Home;
use crate::idle::NoSubTasks;
use crate::idle::NoTools;
use crate::lock::DaemonLock;
use crate::settings::Settings;
use crate::settings::environment;
use crate::tools;

const QUEUE: NonZeroUsize = NonZeroUsize::new(256).expect("non-zero");
const SEEN: NonZeroUsize = NonZeroUsize::new(4096).expect("non-zero");
const TOOL_TIMEOUT: Duration = Duration::from_secs(120);

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
    let (_mcp, toolbox): (_, Arc<dyn Toolbox>) = match tools::start().await {
        Ok((child, tools)) => (Some(child), Arc::new(tools)),
        Err(error) => {
            eprintln!("dotori: no shared tools: {error}");
            (None, Arc::new(NoTools))
        }
    };
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
        sub_tasks: Arc::new(NoSubTasks),
        records: Arc::new(JsonlStore::new(home.record())),
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
