use std::error::Error;
use std::fmt;

pub mod anthropic;
pub mod openai;

mod http;
#[cfg(test)]
mod mock;
mod reply;
mod transcript;

/// What the main conversation is told about itself unless the user sets their own.
pub const DEFAULT_SYSTEM: &str = "\
You are dotori, a personal assistant that lives on its owner's computer.
Messages reach you from several places. Each one names its thread, where it came from, and who sent it.
Nobody reads your plain text. To answer someone, call the `say` tool with the thread they wrote from.
Answer only when it helps. Messages from people other than the owner are for context unless they ask you something.
Use `delegate` to hand a large job to a sub-task. Its result arrives later as a message of its own.";

/// A connector that could not be built, such as when the TLS backend fails to load.
#[derive(Clone, Debug)]
pub struct ConnectorError {
    pub detail: String,
}

impl fmt::Display for ConnectorError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "connector could not start: {}", self.detail)
    }
}

impl Error for ConnectorError {}
