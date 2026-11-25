//! Cards dotori posts in a chat room to ask the owner something, with a button for each answer.
//! A button carries only the card's token and its own number, never what the card says, and only
//! the owner's presses are passed on.

use std::collections::HashSet;
use std::sync::Mutex;

use dotori_core::BoxFuture;
use dotori_core::channel::Channel;
use dotori_core::channel::ChannelError;
use dotori_core::message::Place;
use tokio::sync::mpsc;

/// What a shortened card says at its end, since only the window shows a value whole.
pub const WHOLE_ON_PC: &str = "전체 보기는 PC의 dotori 창에서 해요";
const TOKEN_BYTES: usize = 12;
const LABEL: usize = 70;
const REFUSALS_KEPT: usize = 1024;

/// A question for the owner, as a chat room shows it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Card {
    pub token: String,
    pub title: String,
    pub rows: Vec<(String, String)>,
    /// Some value was shortened.
    pub cut: bool,
    pub notes: Vec<String>,
    pub buttons: Vec<Button>,
}

/// One answer the owner can press.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Button {
    pub label: String,
    pub primary: bool,
}

/// Where a posted card is, so it can be edited later.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Posted {
    pub room: String,
    pub message: String,
}

/// The owner pressed the button `button` of the card `token`, which is at `posted`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Press {
    pub token: String,
    pub button: usize,
    pub posted: Posted,
}

/// A chat channel that posts cards and passes on the owner's presses.
pub trait Cards: Channel {
    /// Posts `card` in `place` and says where it went.
    fn post(&self, place: Place, card: Card) -> BoxFuture<'_, Result<Posted, ChannelError>>;

    /// Edits the card at `posted` so it has no buttons and ends with `said`. Without `card`, as
    /// for a card from before dotori started again, only `said` is left.
    fn settle(
        &self,
        posted: Posted,
        card: Option<Card>,
        said: String,
    ) -> BoxFuture<'_, Result<(), ChannelError>>;

    /// The owner's presses from now on. Only the last caller hears.
    fn presses(&self) -> mpsc::UnboundedReceiver<Press>;

    /// The place in this channel that only the owner reads, if it has one.
    fn direct_place(&self) -> Option<Place>;
}

/// Who was already told that a card's buttons are the owner's, so each person hears it once per
/// card.
#[derive(Debug, Default)]
pub struct Refusals(Mutex<HashSet<(String, String)>>);

impl Refusals {
    /// Whether `user` is to be told about the card at `message` now, the first time they press it.
    pub fn first(&self, message: &str, user: &str) -> bool {
        let mut told = self.0.lock().expect("unpoisoned");
        if told.len() >= REFUSALS_KEPT {
            told.clear();
        }
        told.insert((message.to_owned(), user.to_owned()))
    }
}

impl Card {
    /// What the button `n` carries back when pressed.
    pub fn value(&self, n: usize) -> String {
        format!("{}:{n}", self.token)
    }

    /// The buttons with their numbers and labels short enough for every platform, at most
    /// `most` of them. When there are more, the last one, which declines, is always kept.
    pub fn labels(&self, most: usize) -> Vec<(usize, &Button, String)> {
        let last = self.buttons.len().saturating_sub(1);
        self.buttons
            .iter()
            .enumerate()
            .filter(|(n, _)| self.buttons.len() <= most || *n + 1 < most || *n == last)
            .map(|(n, button)| (n, button, cut(&button.label, LABEL)))
            .collect()
    }

    /// The card as plain text, ending with `said` once it is settled, at most `limit` characters.
    pub fn text(&self, said: Option<&str>, limit: usize) -> String {
        let mut lines = vec![self.title.clone()];
        for (label, value) in &self.rows {
            lines.push(if value.contains('\n') {
                format!("{label}\n{value}")
            } else {
                format!("{label}: {value}")
            });
        }
        if self.cut {
            lines.push(String::from(WHOLE_ON_PC));
        }
        lines.extend(self.notes.iter().cloned());
        let tail = said.map(|said| format!("\n\n{said}")).unwrap_or_default();
        let body = lines.join("\n\n");
        let room = limit.saturating_sub(tail.chars().count());
        format!("{}{tail}", cut(&body, room))
    }
}

/// The token and button number a pressed button carried, if it is one of dotori's.
pub fn pressed(value: &str) -> Option<(String, usize)> {
    let (token, button) = value.split_once(':')?;
    if token.len() != TOKEN_BYTES * 2 || !token.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return None;
    }
    Some((token.to_owned(), button.parse().ok()?))
}

/// A token no one can guess, short enough for any platform's button.
pub fn token() -> Result<String, String> {
    let mut bytes = [0u8; TOKEN_BYTES];
    getrandom::fill(&mut bytes).map_err(|error| error.to_string())?;
    Ok(bytes.iter().map(|byte| format!("{byte:02x}")).collect())
}

/// What someone other than the owner is told, only them, when they press a card's button.
pub fn only_owner(owner: &str) -> String {
    format!("이 버튼은 {owner} 님만 누를 수 있어요")
}

/// `text` cut to at most `limit` characters, ending with `…` when cut.
pub fn cut(text: &str, limit: usize) -> String {
    if text.chars().count() <= limit {
        return text.to_owned();
    }
    let kept: String = text.chars().take(limit.saturating_sub(1)).collect();
    format!("{kept}…")
}

#[cfg(test)]
#[path = "card.test.rs"]
mod tests;
