//! The chat channels the main session talks through, one slot for each kind, so a channel the
//! owner turns on, off or changes in the settings swaps in under a running session. The session
//! keeps the reply it is writing and the messages that wait, as nothing about it starts again.

use std::sync::Arc;
use std::sync::Mutex;

use dotori_connectors::channel::card::Card;
use dotori_connectors::channel::card::Cards;
use dotori_connectors::channel::card::Posted;
use dotori_connectors::channel::card::Press;
use dotori_connectors::channel::discord::DISCORD;
use dotori_connectors::channel::slack::SLACK;
use dotori_connectors::channel::telegram::TELEGRAM;
use dotori_core::BoxFuture;
use dotori_core::channel::Channel;
use dotori_core::channel::ChannelError;
use dotori_core::channel::Inbox;
use dotori_core::id::ChannelId;
use dotori_core::message::Place;
use tokio::sync::mpsc;
use tokio::sync::watch;

/// The slots, which hold whichever chat channels the settings turn on now.
pub(crate) struct ChannelSlots {
    slots: Vec<Arc<Slot>>,
}

impl ChannelSlots {
    pub(crate) fn new() -> Result<Self, String> {
        let slots = [SLACK, TELEGRAM, DISCORD]
            .into_iter()
            .map(|kind| Slot::new(kind).map(Arc::new))
            .collect::<Result<_, _>>()?;
        Ok(Self { slots })
    }

    /// Puts `chats` in the slots of the kinds in `changed`, emptying those with no channel in
    /// `chats`. The other slots keep the channel they run.
    pub(crate) fn fill(&self, chats: &[Arc<dyn Cards>], changed: &[&str]) {
        for slot in &self.slots {
            if !changed.contains(&slot.id.as_str()) {
                continue;
            }
            let chat = chats.iter().find(|chat| chat.id() == &slot.id).cloned();
            slot.put(chat);
        }
    }

    /// The slots as the channels the session and the room cards use.
    pub(crate) fn all(&self) -> Vec<Arc<dyn Cards>> {
        self.slots
            .iter()
            .map(|slot| Arc::clone(slot) as Arc<dyn Cards>)
            .collect()
    }
}

struct Slot {
    id: ChannelId,
    held: watch::Sender<Option<Arc<dyn Cards>>>,
    presses: Mutex<Option<mpsc::UnboundedSender<Press>>>,
}

impl Slot {
    fn new(kind: &str) -> Result<Self, String> {
        let id = ChannelId::parse(kind).map_err(|error| error.to_string())?;
        let (held, _) = watch::channel(None);
        Ok(Self {
            id,
            held,
            presses: Mutex::new(None),
        })
    }

    fn put(&self, chat: Option<Arc<dyn Cards>>) {
        if let Some(chat) = &chat {
            self.pass_presses(chat);
        }
        self.held.send_replace(chat);
    }

    fn pass_presses(&self, chat: &Arc<dyn Cards>) {
        let Some(to) = self.presses.lock().expect("unpoisoned").clone() else {
            return;
        };
        let mut from = chat.presses();
        tokio::spawn(async move {
            while let Some(press) = from.recv().await {
                if to.send(press).is_err() {
                    return;
                }
            }
        });
    }

    fn held(&self) -> Result<Arc<dyn Cards>, ChannelError> {
        self.held.borrow().clone().ok_or_else(|| ChannelError {
            detail: format!("{} is off in the settings", self.id.as_str()),
        })
    }
}

impl Channel for Slot {
    fn id(&self) -> &ChannelId {
        &self.id
    }

    fn run(&self, inbox: Arc<dyn Inbox>) -> BoxFuture<'_, Result<(), ChannelError>> {
        Box::pin(async move {
            let mut held = self.held.subscribe();
            loop {
                let chat = held.borrow_and_update().clone();
                match chat {
                    None => {}
                    Some(chat) => {
                        tokio::select! {
                            ended = chat.run(Arc::clone(&inbox)) => {
                                if let Err(error) = ended {
                                    eprintln!("dotori: {} stopped: {error}", self.id.as_str());
                                }
                            }
                            _ = held.changed() => continue,
                        }
                    }
                }
                if held.changed().await.is_err() {
                    return Ok(());
                }
            }
        })
    }

    fn deliver(&self, place: Place, text: String) -> BoxFuture<'_, Result<(), ChannelError>> {
        Box::pin(async move { self.held()?.deliver(place, text).await })
    }

    fn private_place(&self) -> Place {
        self.held
            .borrow()
            .as_ref()
            .map_or_else(|| Place::new(""), |chat| chat.private_place())
    }
}

impl Cards for Slot {
    fn post(&self, place: Place, card: Card) -> BoxFuture<'_, Result<Posted, ChannelError>> {
        Box::pin(async move { self.held()?.post(place, card).await })
    }

    fn settle(
        &self,
        posted: Posted,
        card: Option<Card>,
        said: String,
    ) -> BoxFuture<'_, Result<(), ChannelError>> {
        Box::pin(async move { self.held()?.settle(posted, card, said).await })
    }

    fn presses(&self) -> mpsc::UnboundedReceiver<Press> {
        let (to, from) = mpsc::unbounded_channel();
        *self.presses.lock().expect("unpoisoned") = Some(to);
        if let Some(chat) = self.held.borrow().clone() {
            self.pass_presses(&chat);
        }
        from
    }

    fn direct_place(&self) -> Option<Place> {
        self.held
            .borrow()
            .as_ref()
            .and_then(|chat| chat.direct_place())
    }
}

#[cfg(test)]
#[path = "channel_slots.test.rs"]
mod tests;
