use std::fs;
use std::num::NonZeroUsize;
use std::sync::Arc;
use std::time::Duration;

use dotori_connectors::channel::app::AppChannel;
use dotori_connectors::record::jsonl::JsonlStore;
use dotori_core::runtime::Agent;
use dotori_core::runtime::AgentConfig;
use dotori_core::runtime::Ports;

use crate::door::Door;
use crate::home::Home;
use crate::idle::NoSubTasks;
use crate::idle::NoTools;
use crate::settings::Settings;
use crate::settings::environment;

const QUEUE: NonZeroUsize = NonZeroUsize::new(256).expect("non-zero");
const SEEN: NonZeroUsize = NonZeroUsize::new(4096).expect("non-zero");
const TOOL_TIMEOUT: Duration = Duration::from_secs(120);

/// Runs the agent until Ctrl-C, with the app channel open for windows.
pub(crate) async fn run(home: Home) -> Result<(), String> {
    let settings = Settings::load(&home.settings())?;
    let model = settings.model(environment)?;
    let probe = Door::new(0);
    let (app, address) =
        AppChannel::bind(probe.token.clone()).map_err(|error| error.to_string())?;
    let door = Door {
        port: address.port(),
        token: probe.token,
    };
    let ports = Ports {
        app: Arc::new(app),
        channels: Vec::new(),
        model,
        toolbox: Arc::new(NoTools),
        sub_tasks: Arc::new(NoSubTasks),
        records: Arc::new(JsonlStore::new(home.record())),
    };
    let config = AgentConfig {
        owners: Vec::new(),
        queue_capacity: QUEUE,
        seen_capacity: SEEN,
        tool_timeout: TOOL_TIMEOUT,
    };
    let agent = Agent::spawn(config, ports)
        .await
        .map_err(|error| error.to_string())?;
    door.write(&home.door())?;
    eprintln!("dotori is running. Open a window with `dotori chat`.");
    let stopped = tokio::signal::ctrl_c().await;
    let _ = fs::remove_file(home.door());
    agent.shutdown().await;
    stopped.map_err(|error| error.to_string())
}
