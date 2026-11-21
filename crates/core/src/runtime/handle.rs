use std::error::Error;
use std::fmt;
use std::panic;

use tokio::sync::mpsc;
use tokio::task::JoinHandle;

use crate::agent::AgentState;
use crate::agent::Event;
use crate::record::RecordError;
use crate::runtime::AgentConfig;
use crate::runtime::ConfigError;
use crate::runtime::Ports;
use crate::runtime::caught::caught;
use crate::runtime::config::validate;
use crate::runtime::shell::Shell;

/// What the app holds while the assistant runs.
#[derive(Debug)]
pub struct Agent {
    tx: mpsc::Sender<Event>,
    done: JoinHandle<()>,
}

impl Agent {
    /// Validates the wiring, resumes the conversation from the raw record, then starts the
    /// agent loop and every channel on the current tokio runtime.
    pub async fn spawn(config: AgentConfig, ports: Ports) -> Result<Self, SpawnError> {
        let roster = validate(&config, &ports).map_err(SpawnError::Config)?;
        let records = caught(|| ports.records.load())
            .await
            .unwrap_or_else(|detail| Err(RecordError { detail }))
            .map_err(SpawnError::Load)?;
        let loaded = records.len();
        let conversation = records.into_iter().map(|record| record.entry).collect();
        let state = AgentState::resume(roster, config.seen_capacity, conversation);
        let (tx, rx) = mpsc::channel(config.queue_capacity.get());
        let shell = Shell::start(rx, tx.clone(), ports, config.tool_timeout);
        shell.record(&state.conversation[loaded..]);
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

#[derive(Clone, Debug)]
pub enum SpawnError {
    Config(ConfigError),
    Load(RecordError),
}

impl fmt::Display for SpawnError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Config(error) => write!(f, "{error}"),
            Self::Load(error) => write!(f, "{error}"),
        }
    }
}

impl Error for SpawnError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Config(error) => Some(error),
            Self::Load(error) => Some(error),
        }
    }
}

#[cfg(test)]
#[path = "handle.test.rs"]
mod tests;
