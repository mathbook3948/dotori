use std::error::Error;
use std::fmt;

use crate::id::ChannelId;
use crate::id::ExternalMessageId;
use crate::id::ExternalUserId;

/// Where inside a channel a message came from and a reply goes, such as a Slack thread.
/// The connector mints it. Core stores it and hands it back unread.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Place(String);

impl Place {
    pub fn new(raw: impl Into<String>) -> Self {
        Self(raw.into())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Address {
    pub channel: ChannelId,
    pub place: Place,
}

/// Non-empty message text.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Content(String);

impl Content {
    pub fn parse(text: impl Into<String>) -> Result<Self, EmptyContentError> {
        let text = text.into();
        if text.trim().is_empty() {
            return Err(EmptyContentError);
        }
        Ok(Self(text))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EmptyContentError;

impl fmt::Display for EmptyContentError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "message content is empty")
    }
}

impl Error for EmptyContentError {}

/// What a connector hands to its inbox: only the owner's messages addressed to the assistant.
/// Others' messages never arrive. Core trusts everything it receives.
/// It carries no channel id. The inbox stamps the channel.
#[derive(Clone, Debug)]
pub struct Incoming {
    pub id: ExternalMessageId,
    pub place: Place,
    pub sender_id: ExternalUserId,
    pub sender_name: String,
    pub content: Content,
    pub audience: Audience,
}

/// Who can read the room a message came from, as its channel saw it when the message arrived.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Audience {
    /// Only the owner and dotori, such as the app window or a direct message.
    Owner,
    /// The owner and other people. `some` names a few of the others and `total` counts everyone
    /// but dotori, the owner included.
    People { some: Vec<String>, total: usize },
    /// Anyone in a wider group can read it, such as everyone in a Slack workspace.
    Anyone(String),
}

/// Everything heard now is the owner's. `Other` survives only in records from before dotori
/// heard the owner alone.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Role {
    Owner,
    Other,
}

#[derive(Clone, Debug)]
pub struct Sender {
    pub id: ExternalUserId,
    pub name: String,
    pub role: Role,
}

/// An inbound message after core stamped its channel and resolved its sender.
#[derive(Clone, Debug)]
pub struct Message {
    pub id: ExternalMessageId,
    pub address: Address,
    pub sender: Sender,
    pub content: Content,
    /// `None` only in records from before rooms were described.
    pub audience: Option<Audience>,
}

#[cfg(test)]
#[path = "message.test.rs"]
mod tests;
