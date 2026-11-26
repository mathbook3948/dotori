//! The first words of an assistant with no conversation yet. The main session gets one note it
//! answers with a greeting, and the chat window shows that greeting as an ordinary reply and never
//! shows the note.

use std::future;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use std::sync::atomic::Ordering;

use dotori_connectors::channel::app::ToWindow;
use dotori_core::BoxFuture;
use dotori_core::channel::Channel;
use dotori_core::channel::ChannelError;
use dotori_core::channel::Inbox;
use dotori_core::id::ChannelId;
use dotori_core::id::ExternalMessageId;
use dotori_core::id::ExternalUserId;
use dotori_core::message::Audience;
use dotori_core::message::Content;
use dotori_core::message::Incoming;
use dotori_core::message::Place;

/// The channel the note comes in on, and the thread the greeting goes to.
pub(crate) const GREETING: &str = "greeting";

const NOTE: &str = "The owner just finished setting up and opened the chat window, and you have not talked yet. Greet them in this thread with two `say` calls. In the first, say hello, give your name as their personal assistant, and tell them to call on you whenever they are curious about something or short of hands, as in \"Hi, I'm Dotori, your personal assistant. Call on me any time you're curious or short of hands.\" In the second, ask what you should call them, as in \"First, what should I call you?\" When they answer, keep it with `add_memory`.";

/// Hands the main session the note once, and takes nothing back, since the greeting reaches the
/// window as the channel's traffic.
pub(crate) struct Greeting {
    id: ChannelId,
    sent: AtomicBool,
}

impl Greeting {
    pub(crate) fn new() -> Self {
        Self {
            id: ChannelId::parse(GREETING).expect("a valid channel id"),
            sent: AtomicBool::new(false),
        }
    }
}

impl Channel for Greeting {
    fn id(&self) -> &ChannelId {
        &self.id
    }

    fn run(&self, inbox: Arc<dyn Inbox>) -> BoxFuture<'_, Result<(), ChannelError>> {
        Box::pin(async move {
            if !self.sent.swap(true, Ordering::SeqCst) {
                let content = Content::parse(String::from(NOTE)).map_err(|error| ChannelError {
                    detail: error.to_string(),
                })?;
                let incoming = Incoming {
                    id: ExternalMessageId::new(String::from(GREETING)),
                    place: Place::new(GREETING),
                    sender_id: ExternalUserId::new(GREETING),
                    sender_name: String::from(GREETING),
                    content,
                    audience: Audience::Owner,
                };
                if inbox.push(incoming).await.is_err() {
                    return Ok(());
                }
            }
            future::pending().await
        })
    }

    fn deliver(&self, _place: Place, _text: String) -> BoxFuture<'_, Result<(), ChannelError>> {
        Box::pin(async { Ok(()) })
    }

    fn private_place(&self) -> Place {
        Place::new(GREETING)
    }
}

/// How the chat window shows `event`: the greeting as dotori's reply, the note not at all, and
/// everything else as it is.
pub(crate) fn shown(event: ToWindow) -> Option<ToWindow> {
    match event {
        ToWindow::Traffic {
            channel,
            who,
            text,
            at,
            ..
        } if channel == GREETING => (who == "dotori").then_some(ToWindow::Reply { text, at }),
        event => Some(event),
    }
}

#[cfg(test)]
#[path = "greeting.test.rs"]
mod tests;
