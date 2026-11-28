use std::collections::HashMap;
use std::collections::HashSet;
use std::error::Error;
use std::fmt;
use std::iter;
use std::num::NonZeroUsize;
use std::time::Duration;

use crate::channel::Channel;
use crate::id::ChannelId;
use crate::message::Place;
use crate::runtime::Ports;

#[derive(Clone, Debug)]
pub struct AgentConfig {
    pub queue_capacity: NonZeroUsize,
    pub seen_capacity: NonZeroUsize,
    pub tool_timeout: Duration,
    /// How long the agent waits for more owner messages before it calls the model.
    pub gather_for: Duration,
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

/// What the agent needs to know about its channels.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Wiring {
    pub(crate) app: ChannelId,
    pub(crate) private: HashMap<ChannelId, Place>,
}

/// The single check between untrusted wiring and the agent.
/// Channel ids are unique, the app window's id included.
pub(crate) fn validate(ports: &Ports) -> Result<Wiring, ConfigError> {
    let mut connected = HashSet::new();
    let mut private = HashMap::new();
    let app: &dyn Channel = ports.app.as_ref();
    let channels = iter::once(app).chain(ports.channels.iter().map(|channel| channel.as_ref()));
    for channel in channels {
        if !connected.insert(channel.id()) {
            return Err(ConfigError::DuplicateChannel(channel.id().clone()));
        }
        private.insert(channel.id().clone(), channel.private_place());
    }
    Ok(Wiring {
        app: ports.app.id().clone(),
        private,
    })
}

#[cfg(test)]
#[path = "config.test.rs"]
mod tests;
