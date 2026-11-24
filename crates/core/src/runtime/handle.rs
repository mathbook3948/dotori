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
use crate::sub_session::SubSessionOutcome;
use crate::sub_session::SubSessionSpec;

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
        Self::spawn_continuing(config, ports, Vec::new()).await
    }

    /// Like `spawn`, with `running` the sub-sessions this process still runs from before, which
    /// resume leaves open for their results to arrive through `returns`.
    pub async fn spawn_continuing(
        config: AgentConfig,
        ports: Ports,
        running: Vec<SubSessionSpec>,
    ) -> Result<Self, SpawnError> {
        let wiring = validate(&ports).map_err(SpawnError::Config)?;
        let records = caught(|| ports.records.load_marked())
            .await
            .unwrap_or_else(|detail| Err(RecordError { detail }))
            .map_err(SpawnError::Load)?;
        let loaded = records.len();
        let marked = records
            .into_iter()
            .map(|marked| (marked.record.entry, marked.here))
            .collect();
        let state = AgentState::resume(wiring, config.seen_capacity, marked, running);
        let (tx, rx) = mpsc::channel(config.queue_capacity.get());
        let shell = Shell::start(rx, tx.clone(), ports, config.tool_timeout);
        shell.record(&state.conversation[loaded..]);
        let done = tokio::spawn(shell.run(state));
        Ok(Self { tx, done })
    }

    /// Stops taking input, waits for running tools, aborts sub-sessions and channels,
    /// gives queued replies up to the tool timeout to go out, then returns.
    /// A panic in the agent loop resumes here.
    pub async fn shutdown(self) {
        self.stop(false).await;
    }

    /// Stops like `shutdown`, for when the main session moves to another PC: sub-sessions still
    /// running are not recorded as stopped, since whoever runs them passes their results on.
    pub async fn hand_off(self) {
        self.stop(true).await;
    }

    /// Where results of sub-sessions this agent did not start itself come in, such as one
    /// handed off by another PC.
    pub fn returns(&self) -> Returns {
        Returns {
            tx: self.tx.clone(),
        }
    }

    async fn stop(mut self, handing_off: bool) {
        let _ = self.tx.send(Event::Shutdown { handing_off }).await;
        if let Err(error) = (&mut self.done).await
            && error.is_panic()
        {
            panic::resume_unwind(error.into_panic());
        }
    }
}

/// Takes results of sub-sessions into a running agent, which records them and goes on.
#[derive(Clone, Debug)]
pub struct Returns {
    tx: mpsc::Sender<Event>,
}

impl Returns {
    /// Gives the agent what came of `spec`. Says whether the agent was still there to take it.
    pub async fn deliver(&self, spec: SubSessionSpec, outcome: SubSessionOutcome) -> bool {
        self.tx
            .send(Event::SubSessionReturned { spec, outcome })
            .await
            .is_ok()
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
