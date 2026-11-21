use std::error::Error;
use std::fmt;
use std::sync::Arc;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ChannelIdError {
    Empty,
    Invalid(char),
}

impl fmt::Display for ChannelIdError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Empty => write!(f, "channel id is empty"),
            Self::Invalid(c) => write!(f, "channel id has invalid character {c:?}"),
        }
    }
}

impl Error for ChannelIdError {}

/// Stable name of one connected channel, such as `app` or `slack`.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct ChannelId(Arc<str>);

impl ChannelId {
    /// Accepts lowercase ascii letters, digits, and `-`.
    pub fn parse(raw: &str) -> Result<Self, ChannelIdError> {
        if raw.is_empty() {
            return Err(ChannelIdError::Empty);
        }
        match raw
            .chars()
            .find(|c| !(c.is_ascii_lowercase() || c.is_ascii_digit() || *c == '-'))
        {
            Some(c) => Err(ChannelIdError::Invalid(c)),
            None => Ok(Self(raw.into())),
        }
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// A user id native to one channel, such as a Slack user id.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct ExternalUserId(Arc<str>);

impl ExternalUserId {
    pub fn new(raw: impl Into<Arc<str>>) -> Self {
        Self(raw.into())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// A message id native to one channel. Used only to drop redelivered messages.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct ExternalMessageId(Arc<str>);

impl ExternalMessageId {
    pub fn new(raw: impl Into<Arc<str>>) -> Self {
        Self(raw.into())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// One user on one channel.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct ChannelUser {
    pub channel: ChannelId,
    pub user: ExternalUserId,
}

#[cfg(test)]
#[path = "id.test.rs"]
mod tests;
