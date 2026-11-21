use std::collections::HashSet;
use std::error::Error;
use std::fmt;
use std::iter;
use std::num::NonZeroUsize;
use std::time::Duration;

use crate::agent::Roster;
use crate::id::ChannelId;
use crate::id::ChannelUser;
use crate::runtime::Ports;

#[derive(Clone, Debug)]
pub struct AgentConfig {
    pub owners: Vec<ChannelUser>,
    pub queue_capacity: NonZeroUsize,
    pub seen_capacity: NonZeroUsize,
    pub tool_timeout: Duration,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ConfigError {
    DuplicateChannel(ChannelId),
    UnknownOwnerChannel(ChannelId),
}

impl fmt::Display for ConfigError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::DuplicateChannel(id) => write!(f, "channel id {} is used twice", id.as_str()),
            Self::UnknownOwnerChannel(id) => {
                write!(f, "owner refers to unknown channel {}", id.as_str())
            }
        }
    }
}

impl Error for ConfigError {}

/// The single check between untrusted wiring and the agent.
/// Channel ids are unique, the app window's id included, and every owner names a connected channel.
pub(crate) fn validate(config: &AgentConfig, ports: &Ports) -> Result<Roster, ConfigError> {
    let mut connected = HashSet::new();
    let ids = iter::once(ports.app.id()).chain(ports.channels.iter().map(|channel| channel.id()));
    for id in ids {
        if !connected.insert(id) {
            return Err(ConfigError::DuplicateChannel(id.clone()));
        }
    }
    if let Some(owner) = config
        .owners
        .iter()
        .find(|owner| !connected.contains(&owner.channel))
    {
        return Err(ConfigError::UnknownOwnerChannel(owner.channel.clone()));
    }
    Ok(Roster::new(
        ports.app.id().clone(),
        config.owners.iter().cloned().collect(),
    ))
}

#[cfg(test)]
#[path = "config.test.rs"]
mod tests;
