use std::collections::HashSet;
use std::error::Error;
use std::fmt;
use std::iter;
use std::num::NonZeroUsize;
use std::time::Duration;

use crate::id::ChannelId;
use crate::runtime::Ports;

#[derive(Clone, Debug)]
pub struct AgentConfig {
    pub queue_capacity: NonZeroUsize,
    pub seen_capacity: NonZeroUsize,
    pub tool_timeout: Duration,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ConfigError {
    DuplicateChannel(ChannelId),
}

impl fmt::Display for ConfigError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::DuplicateChannel(id) => write!(f, "channel id {} is used twice", id.as_str()),
        }
    }
}

impl Error for ConfigError {}

/// The single check between untrusted wiring and the agent.
/// Channel ids are unique, the app window's id included. Returns the app window's id.
pub(crate) fn validate(ports: &Ports) -> Result<ChannelId, ConfigError> {
    let mut connected = HashSet::new();
    let ids = iter::once(ports.app.id()).chain(ports.channels.iter().map(|channel| channel.id()));
    for id in ids {
        if !connected.insert(id) {
            return Err(ConfigError::DuplicateChannel(id.clone()));
        }
    }
    Ok(ports.app.id().clone())
}

#[cfg(test)]
#[path = "config.test.rs"]
mod tests;
