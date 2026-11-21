use std::panic;

use tokio::sync::mpsc;
use tokio::task::JoinHandle;

use crate::agent::AgentState;
use crate::agent::Event;
use crate::runtime::AgentConfig;
use crate::runtime::ConfigError;
use crate::runtime::Ports;
use crate::runtime::config::validate;
use crate::runtime::shell::Shell;

/// What the app holds while the assistant runs.
#[derive(Debug)]
pub struct Agent {
    tx: mpsc::Sender<Event>,
    done: JoinHandle<()>,
}

impl Agent {
    /// Validates the wiring, then starts the agent loop and every channel on the current tokio runtime.
    pub fn spawn(config: AgentConfig, ports: Ports) -> Result<Self, ConfigError> {
        let roster = validate(&config, &ports)?;
        let (tx, rx) = mpsc::channel(config.queue_capacity.get());
        let state = AgentState::new(roster, config.seen_capacity);
        let shell = Shell::start(rx, tx.clone(), ports, config.tool_timeout);
        let done = tokio::spawn(shell.run(state));
        Ok(Self { tx, done })
    }

    /// Stops taking input, waits for running tools, aborts sub-tasks and channels,
    /// gives queued replies up to the tool timeout to go out, then returns.
    /// A panic in the agent loop resumes here.
    pub async fn shutdown(mut self) {
        let _ = self.tx.send(Event::Shutdown).await;
        if let Err(error) = (&mut self.done).await
            && error.is_panic()
        {
            panic::resume_unwind(error.into_panic());
        }
    }
}

impl Drop for Agent {
    fn drop(&mut self) {
        self.done.abort();
    }
}

#[cfg(test)]
#[path = "handle.test.rs"]
mod tests;
