use tokio::sync::mpsc;
use tokio::task::JoinHandle;

use crate::agent::Event;
use crate::runtime::AgentConfig;
use crate::runtime::ConfigError;
use crate::runtime::Ports;

/// What the app holds while the assistant runs.
#[derive(Debug)]
pub struct Agent {
    tx: mpsc::Sender<Event>,
    done: JoinHandle<()>,
}

impl Agent {
    /// Validates the wiring, then starts the agent loop and every channel on the current tokio runtime.
    pub fn spawn(config: AgentConfig, ports: Ports) -> Result<Self, ConfigError> {
        todo!()
    }

    /// Stops taking input, waits for running tools, aborts sub-tasks and channels, then returns.
    pub async fn shutdown(self) {
        todo!()
    }
}
