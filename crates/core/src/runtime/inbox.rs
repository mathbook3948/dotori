use tokio::sync::mpsc;

use crate::BoxFuture;
use crate::agent::Event;
use crate::channel::Inbox;
use crate::channel::InboxClosedError;
use crate::id::ChannelId;
use crate::message::Incoming;

/// The inbox handed to one channel. It stamps that channel's id on every message.
pub(crate) struct ChannelInbox {
    channel: ChannelId,
    tx: mpsc::Sender<Event>,
}

impl ChannelInbox {
    pub(crate) fn new(channel: ChannelId, tx: mpsc::Sender<Event>) -> Self {
        Self { channel, tx }
    }
}

impl Inbox for ChannelInbox {
    fn push(&self, incoming: Incoming) -> BoxFuture<'_, Result<(), InboxClosedError>> {
        Box::pin(async move {
            self.tx
                .send(Event::Received {
                    channel: self.channel.clone(),
                    incoming,
                })
                .await
                .map_err(|_| InboxClosedError)
        })
    }
}
