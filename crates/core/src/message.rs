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
        todo!()
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

/// What a connector hands to its inbox: only messages addressed to the assistant
/// from places the owner trusts. Core trusts everything it receives.
/// It carries no channel id and no role. The inbox stamps the channel and core decides the role.
#[derive(Clone, Debug)]
pub struct Incoming {
    pub id: ExternalMessageId,
    pub place: Place,
    pub sender_id: ExternalUserId,
    pub sender_name: String,
    pub content: Content,
}

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
#[non_exhaustive]
pub struct Message {
    pub address: Address,
    pub sender: Sender,
    pub content: Content,
}
