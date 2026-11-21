use std::error::Error;
use std::fmt;
use std::sync::Arc;

use crate::BoxFuture;
use crate::id::ChannelId;
use crate::message::Address;
use crate::message::Incoming;
use crate::message::Place;

/// One connection, such as Slack. It holds no conversation state.
/// It decides only whether a message is addressed to the assistant.
pub trait Channel: Send + Sync + 'static {
    fn id(&self) -> &ChannelId;

    /// Receives until the connection closes and pushes only messages addressed to the assistant.
    /// Core drops this future on shutdown.
    fn run(&self, inbox: Arc<dyn Inbox>) -> BoxFuture<'_, Result<(), ChannelError>>;

    /// Core calls this one delivery at a time per channel, so replies keep their order.
    fn deliver(&self, place: Place, text: String) -> BoxFuture<'_, Result<(), ChannelError>>;
}

/// The app chat window. Besides its own conversation it shows the traffic of every other channel.
pub trait AppWindow: Channel {
    fn show(&self, traffic: Traffic) -> BoxFuture<'_, Result<(), ChannelError>>;
}

#[derive(Clone, Debug)]
pub struct Traffic {
    pub address: Address,
    pub who: String,
    pub text: String,
}

/// Where a channel pushes its messages. Core binds each inbox to one channel,
/// so a channel cannot speak as another.
pub trait Inbox: Send + Sync {
    /// Waits while the queue is full, which throttles a flooding channel.
    fn push(&self, incoming: Incoming) -> BoxFuture<'_, Result<(), InboxClosedError>>;
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InboxClosedError;

impl fmt::Display for InboxClosedError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "agent inbox is closed")
    }
}

impl Error for InboxClosedError {}

#[derive(Clone, Debug)]
pub struct ChannelError {
    pub detail: String,
}

impl fmt::Display for ChannelError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "channel failed: {}", self.detail)
    }
}

impl Error for ChannelError {}
