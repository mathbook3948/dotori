//! Telegram through the Bot API with long polling, so no public address is needed. The assistant
//! hears the owner's private messages to the bot and the owner's mentions of it in groups, and
//! answers each in its chat. Nobody else is heard.

use std::fmt;
use std::mem;
use std::sync::Arc;
use std::sync::Mutex;
use std::time::Duration;
use std::time::Instant;
use std::time::SystemTime;
use std::time::UNIX_EPOCH;

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
use reqwest::Client;
use reqwest::StatusCode;
use reqwest::header::CONTENT_TYPE;
use serde_json::Value;
use serde_json::json;
use tokio::sync::mpsc;
use tokio::time;

use crate::channel::card;
use crate::channel::card::Card;
use crate::channel::card::Cards;
use crate::channel::card::Posted;
use crate::channel::card::Press;
use crate::channel::card::Refusals;

/// The channel id of Telegram.
pub const TELEGRAM: &str = "telegram";
/// Where the Bot API lives.
pub const API_BASE: &str = "https://api.telegram.org";
/// The place in Telegram that only the owner reads: the owner's private chat with the bot.
pub const OWNER_DIRECT: &str = "@owner";
const POLL_SECONDS: u64 = 50;
const POLL_MARGIN: Duration = Duration::from_secs(15);
const TIMEOUT: Duration = Duration::from_secs(30);
const FIRST_WAIT: Duration = Duration::from_secs(1);
const MAX_WAIT: Duration = Duration::from_secs(60);
const STEADY: Duration = Duration::from_secs(30);
const PIECE: usize = 4000;
const CONFLICT_WAIT: Duration = Duration::from_secs(30);
const LATE_SECONDS: i64 = 60;
const BUTTONS: usize = 25;
const UNCOUNTED: &str = "this Telegram group, whose members could not be counted";

/// What the Telegram channel needs. The owner is the one Telegram user id whose messages are
/// heard.
#[derive(Clone)]
pub struct TelegramConfig {
    pub token: String,
    pub owner: i64,
    pub api_base: String,
}

impl TelegramConfig {
    pub fn new(token: impl Into<String>, owner: i64) -> Self {
        Self {
            token: token.into(),
            owner,
            api_base: String::from(API_BASE),
        }
    }
}

impl fmt::Debug for TelegramConfig {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("TelegramConfig")
            .field("token", &"<hidden>")
            .field("owner", &self.owner)
            .finish_non_exhaustive()
    }
}

/// The Telegram channel.
pub struct TelegramChannel {
    id: ChannelId,
    owner: i64,
    token: String,
    base: String,
    client: Client,
    owner_name: Mutex<Option<String>>,
    presses: Mutex<Option<mpsc::UnboundedSender<Press>>>,
    refusals: Refusals,
}

impl fmt::Debug for TelegramChannel {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("TelegramChannel").finish_non_exhaustive()
    }
}

#[derive(Debug)]
enum Failed {
    Fatal(String),
    Passing(String, Option<Duration>),
    Conflict(String),
}

/// Who the bot is, as Telegram says.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct Bot {
    pub(crate) id: i64,
    pub(crate) username: String,
}

/// A button someone pressed under one of the bot's messages.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Clicked {
    pub(crate) id: String,
    pub(crate) user: i64,
    pub(crate) name: String,
    pub(crate) chat: i64,
    pub(crate) message: i64,
    pub(crate) value: String,
}

/// One message the owner sent the bot.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Arrived {
    pub(crate) id: String,
    pub(crate) chat: i64,
    pub(crate) place: String,
    pub(crate) private: bool,
    pub(crate) name: String,
    pub(crate) text: String,
}

impl TelegramChannel {
    pub fn new(config: TelegramConfig) -> Result<Self, ChannelError> {
        let client = Client::builder().build().map_err(|error| ChannelError {
            detail: error.to_string(),
        })?;
        Ok(Self {
            id: ChannelId::parse(TELEGRAM).map_err(|error| ChannelError {
                detail: error.to_string(),
            })?,
            owner: config.owner,
            token: config.token,
            base: config.api_base.trim_end_matches('/').to_owned(),
            client,
            owner_name: Mutex::new(None),
            presses: Mutex::new(None),
            refusals: Refusals::default(),
        })
    }

    async fn call(&self, method: &str, body: Value, wait: Duration) -> Result<Value, Failed> {
        let response = self
            .client
            .post(format!("{}/bot{}/{method}", self.base, self.token))
            .timeout(wait)
            .header(CONTENT_TYPE, "application/json")
            .body(body.to_string())
            .send()
            .await
            .map_err(|error| {
                Failed::Passing(
                    format!("{method} unreachable: {}", error.without_url()),
                    None,
                )
            })?;
        let status = response.status();
        let text = response.text().await.map_err(|error| {
            Failed::Passing(
                format!("{method} answered oddly: {}", error.without_url()),
                None,
            )
        })?;
        let reply: Value = serde_json::from_str(&text)
            .map_err(|error| Failed::Passing(format!("{method} answered oddly: {error}"), None))?;
        if reply.get("ok").and_then(Value::as_bool) == Some(true) {
            return Ok(reply.get("result").cloned().unwrap_or(Value::Null));
        }
        let detail = format!(
            "{method} failed: {}",
            reply
                .get("description")
                .and_then(Value::as_str)
                .unwrap_or("no reason given")
        );
        let after = reply
            .pointer("/parameters/retry_after")
            .and_then(Value::as_u64)
            .map(Duration::from_secs);
        if matches!(status, StatusCode::UNAUTHORIZED | StatusCode::NOT_FOUND) {
            return Err(Failed::Fatal(detail));
        }
        if status == StatusCode::CONFLICT {
            return Err(Failed::Conflict(detail));
        }
        Err(Failed::Passing(detail, after))
    }

    async fn listen(&self, inbox: Arc<dyn Inbox>) -> Result<(), ChannelError> {
        let mut wait = FIRST_WAIT;
        let since = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |since| {
                i64::try_from(since.as_secs()).unwrap_or(i64::MAX)
            })
            .saturating_sub(LATE_SECONDS);
        let bot = loop {
            match self.call("getMe", json!({}), TIMEOUT).await {
                Ok(me) => {
                    break Bot {
                        id: me.get("id").and_then(Value::as_i64).unwrap_or_default(),
                        username: me
                            .get("username")
                            .and_then(Value::as_str)
                            .unwrap_or_default()
                            .to_owned(),
                    };
                }
                Err(Failed::Fatal(detail)) => return Err(ChannelError { detail }),
                Err(Failed::Passing(_, after)) => wait = pause(wait, after).await,
                Err(Failed::Conflict(_)) => wait = pause(wait, Some(CONFLICT_WAIT)).await,
            }
        };
        let mut offset: i64 = 0;
        wait = FIRST_WAIT;
        loop {
            let started = Instant::now();
            let asked = json!({
                "offset": offset,
                "timeout": POLL_SECONDS,
                "allowed_updates": ["message", "callback_query"],
            });
            let updates = match self
                .call(
                    "getUpdates",
                    asked,
                    Duration::from_secs(POLL_SECONDS) + POLL_MARGIN,
                )
                .await
            {
                Ok(updates) => updates,
                Err(Failed::Fatal(detail)) => return Err(ChannelError { detail }),
                Err(Failed::Conflict(detail)) => {
                    eprintln!(
                        "dotori: Telegram is read elsewhere too, as by another dotori or a webhook: {detail}"
                    );
                    time::sleep(CONFLICT_WAIT).await;
                    continue;
                }
                Err(Failed::Passing(detail, after)) => {
                    if wait == FIRST_WAIT {
                        eprintln!("dotori: Telegram: {detail}");
                    }
                    if started.elapsed() >= STEADY {
                        wait = FIRST_WAIT;
                    }
                    wait = pause(wait, after).await;
                    continue;
                }
            };
            wait = FIRST_WAIT;
            for update in updates.as_array().into_iter().flatten() {
                if let Some(id) = update.get("update_id").and_then(Value::as_i64) {
                    offset = offset.max(id + 1);
                }
                if let Some(query) = update.get("callback_query") {
                    self.pressed(query).await;
                    continue;
                }
                let Some(arrived) = read(update, self.owner, &bot, since) else {
                    continue;
                };
                let Ok(content) = Content::parse(arrived.text) else {
                    continue;
                };
                *self.owner_name.lock().expect("unpoisoned") = Some(arrived.name.clone());
                let audience = if arrived.private {
                    Audience::Owner
                } else {
                    self.audience(arrived.chat).await
                };
                let incoming = Incoming {
                    id: ExternalMessageId::new(arrived.id),
                    place: Place::new(arrived.place),
                    sender_id: ExternalUserId::new(self.owner.to_string()),
                    sender_name: arrived.name,
                    content,
                    audience,
                    at: None,
                };
                if inbox.push(incoming).await.is_err() {
                    return Ok(());
                }
            }
        }
    }

    async fn pressed(&self, query: &Value) {
        let Some(id) = query.get("id").and_then(Value::as_str) else {
            return;
        };
        let user = query.pointer("/from/id").and_then(Value::as_i64);
        let owner = user == Some(self.owner);
        let message = format!(
            "{}/{}",
            query.pointer("/message/chat/id").unwrap_or(&Value::Null),
            query.pointer("/message/message_id").unwrap_or(&Value::Null)
        );
        let mut answer = json!({ "callback_query_id": id });
        if !owner
            && let Some(user) = user
            && self.refusals.first(&message, &user.to_string())
        {
            let name = self
                .owner_name
                .lock()
                .expect("unpoisoned")
                .clone()
                .unwrap_or_else(|| self.owner.to_string());
            answer["text"] = json!(card::only_owner(&name));
        }
        if let Err(Failed::Fatal(detail) | Failed::Passing(detail, _) | Failed::Conflict(detail)) =
            self.call("answerCallbackQuery", answer, TIMEOUT).await
        {
            eprintln!("dotori: Telegram: {detail}");
        }
        if !owner {
            return;
        }
        let Some(clicked) = clicked(query) else {
            return;
        };
        *self.owner_name.lock().expect("unpoisoned") = Some(clicked.name.clone());
        let Some((token, button)) = card::pressed(&clicked.value) else {
            return;
        };
        if let Some(presses) = self.presses.lock().expect("unpoisoned").as_ref() {
            let _ = presses.send(Press {
                token,
                button,
                posted: Posted {
                    room: clicked.chat.to_string(),
                    message: clicked.message.to_string(),
                },
            });
        }
    }

    async fn audience(&self, chat: i64) -> Audience {
        match self
            .call("getChatMemberCount", json!({ "chat_id": chat }), TIMEOUT)
            .await
            .ok()
            .and_then(|count| count.as_u64())
        {
            Some(count) => {
                let total = usize::try_from(count.saturating_sub(1)).unwrap_or(usize::MAX);
                if total <= 1 {
                    Audience::Owner
                } else {
                    Audience::People {
                        some: Vec::new(),
                        total,
                    }
                }
            }
            None => Audience::Anyone(String::from(UNCOUNTED)),
        }
    }
}

async fn pause(wait: Duration, after: Option<Duration>) -> Duration {
    time::sleep(after.unwrap_or(wait).min(MAX_WAIT)).await;
    (wait * 2).min(MAX_WAIT)
}

/// The owner's message in `update`, if it is one the assistant should hear: any text in the
/// private chat, and in a group only text that mentions the bot or answers it. Forwarded text,
/// text sent through a bot, and messages from before `since` (Unix seconds) are not heard.
pub(crate) fn read(update: &Value, owner: i64, bot: &Bot, since: i64) -> Option<Arrived> {
    let message = update.get("message")?;
    let from = message.get("from")?;
    if from.get("id").and_then(Value::as_i64)? != owner
        || from.get("is_bot").and_then(Value::as_bool) == Some(true)
        || message.get("forward_origin").is_some()
        || message.get("forward_from").is_some()
        || message.get("via_bot").is_some()
        || message.get("date").and_then(Value::as_i64).unwrap_or(0) < since
    {
        return None;
    }
    let text = message.get("text").and_then(Value::as_str)?;
    let chat = message.get("chat")?;
    let chat_id = chat.get("id").and_then(Value::as_i64)?;
    let message_id = message.get("message_id").and_then(Value::as_i64)?;
    let kind = chat.get("type").and_then(Value::as_str)?;
    let name = [
        from.get("first_name").and_then(Value::as_str),
        from.get("last_name").and_then(Value::as_str),
    ]
    .into_iter()
    .flatten()
    .collect::<Vec<_>>()
    .join(" ");
    let name = if name.is_empty() {
        owner.to_string()
    } else {
        name
    };
    let id = format!("{chat_id}:{message_id}");
    match kind {
        "private" => Some(Arrived {
            id,
            chat: chat_id,
            place: chat_id.to_string(),
            private: true,
            name,
            text: text.to_owned(),
        }),
        "group" | "supergroup" => {
            let (mentioned, rest) = without_mention(text, &bot.username);
            let answered = bot.id != 0
                && message
                    .pointer("/reply_to_message/from/id")
                    .and_then(Value::as_i64)
                    == Some(bot.id);
            if !mentioned && !answered {
                return None;
            }
            Some(Arrived {
                id,
                chat: chat_id,
                place: format!("{chat_id}/{message_id}"),
                private: false,
                name,
                text: rest,
            })
        }
        _ => None,
    }
}

/// The button press a `callback_query` holds, if it is a whole one.
pub(crate) fn clicked(query: &Value) -> Option<Clicked> {
    let from = query.get("from")?;
    let user = from.get("id").and_then(Value::as_i64)?;
    let name = [
        from.get("first_name").and_then(Value::as_str),
        from.get("last_name").and_then(Value::as_str),
    ]
    .into_iter()
    .flatten()
    .collect::<Vec<_>>()
    .join(" ");
    Some(Clicked {
        id: query.get("id").and_then(Value::as_str)?.to_owned(),
        user,
        name: if name.is_empty() {
            user.to_string()
        } else {
            name
        },
        chat: query.pointer("/message/chat/id").and_then(Value::as_i64)?,
        message: query
            .pointer("/message/message_id")
            .and_then(Value::as_i64)?,
        value: query.get("data").and_then(Value::as_str)?.to_owned(),
    })
}

/// The buttons of `card` under its message: side by side when there are two at most, else one
/// to a row.
pub(crate) fn keyboard(card: &Card) -> Value {
    let buttons: Vec<Value> = card
        .labels(BUTTONS)
        .into_iter()
        .map(|(n, _, label)| json!({ "text": label, "callback_data": card.value(n) }))
        .collect();
    let rows: Vec<Vec<Value>> = if buttons.len() <= 2 {
        vec![buttons]
    } else {
        buttons.into_iter().map(|button| vec![button]).collect()
    };
    json!({ "inline_keyboard": rows })
}

fn without_mention(text: &str, bot: &str) -> (bool, String) {
    if bot.is_empty() {
        return (false, text.to_owned());
    }
    let wanted = format!("@{}", bot.to_ascii_lowercase());
    let lower = text.to_ascii_lowercase();
    let mut kept = String::new();
    let mut found = false;
    let mut at = 0;
    while let Some(offset) = lower[at..].find(&wanted) {
        let start = at + offset;
        let end = start + wanted.len();
        let whole = !lower[end..]
            .chars()
            .next()
            .is_some_and(|next| next.is_ascii_alphanumeric() || next == '_');
        kept.push_str(&text[at..start]);
        if whole {
            found = true;
            kept.push(' ');
        } else {
            kept.push_str(&text[start..end]);
        }
        at = end;
    }
    kept.push_str(&text[at..]);
    (found, kept.split_whitespace().collect::<Vec<_>>().join(" "))
}

/// The chat a place means, and the message to answer there, if any.
pub(crate) fn target(place: &str, owner: i64) -> Option<(i64, Option<i64>)> {
    if place == OWNER_DIRECT {
        return Some((owner, None));
    }
    match place.split_once('/') {
        Some((chat, message)) => Some((chat.parse().ok()?, Some(message.parse().ok()?))),
        None => Some((place.parse().ok()?, None)),
    }
}

/// Splits `text` into messages Telegram takes, counted as Telegram counts, at line breaks where
/// it can. Blank text gives no message.
pub(crate) fn pieces(text: &str) -> Vec<String> {
    let units = |text: &str| text.encode_utf16().count();
    let mut pieces = Vec::new();
    let mut current = String::new();
    for line in text.split_inclusive('\n') {
        if units(&current) + units(line) > PIECE && !current.is_empty() {
            pieces.push(mem::take(&mut current));
        }
        let mut rest = line;
        while units(rest) > PIECE {
            let mut counted = 0;
            let cut = rest
                .char_indices()
                .find(|(_, c)| {
                    counted += c.len_utf16();
                    counted > PIECE
                })
                .map_or(rest.len(), |(at, _)| at);
            pieces.push(rest[..cut].to_owned());
            rest = &rest[cut..];
        }
        current.push_str(rest);
    }
    if !current.trim().is_empty() {
        pieces.push(current);
    }
    pieces.retain(|piece| !piece.trim().is_empty());
    pieces
}

impl Channel for TelegramChannel {
    fn id(&self) -> &ChannelId {
        &self.id
    }

    fn run(&self, inbox: Arc<dyn Inbox>) -> BoxFuture<'_, Result<(), ChannelError>> {
        Box::pin(self.listen(inbox))
    }

    fn private_place(&self) -> Place {
        Place::new(OWNER_DIRECT)
    }

    fn deliver(&self, place: Place, text: String) -> BoxFuture<'_, Result<(), ChannelError>> {
        Box::pin(async move {
            let (chat, answering) =
                target(place.as_str(), self.owner).ok_or_else(|| ChannelError {
                    detail: format!("{} is not a Telegram chat", place.as_str()),
                })?;
            for (n, piece) in pieces(&text).into_iter().enumerate() {
                let mut body = json!({
                    "chat_id": chat,
                    "text": piece,
                    "link_preview_options": { "is_disabled": true },
                });
                if n == 0
                    && let Some(message) = answering
                {
                    body["reply_parameters"] =
                        json!({ "message_id": message, "allow_sending_without_reply": true });
                }
                self.call("sendMessage", body, TIMEOUT)
                    .await
                    .map_err(failed)?;
            }
            Ok(())
        })
    }
}

impl Cards for TelegramChannel {
    fn post(&self, place: Place, card: Card) -> BoxFuture<'_, Result<Posted, ChannelError>> {
        Box::pin(async move {
            let (chat, answering) =
                target(place.as_str(), self.owner).ok_or_else(|| ChannelError {
                    detail: format!("{} is not a Telegram chat", place.as_str()),
                })?;
            let mut body = json!({
                "chat_id": chat,
                "text": card.text(None, PIECE),
                "link_preview_options": { "is_disabled": true },
            });
            if !card.buttons.is_empty() {
                body["reply_markup"] = keyboard(&card);
            }
            if let Some(message) = answering {
                body["reply_parameters"] =
                    json!({ "message_id": message, "allow_sending_without_reply": true });
            }
            let sent = self
                .call("sendMessage", body, TIMEOUT)
                .await
                .map_err(failed)?;
            let message = sent
                .get("message_id")
                .and_then(Value::as_i64)
                .ok_or_else(|| ChannelError {
                    detail: String::from("Telegram gave the card no message id"),
                })?;
            Ok(Posted {
                room: chat.to_string(),
                message: message.to_string(),
            })
        })
    }

    fn settle(
        &self,
        posted: Posted,
        card: Option<Card>,
        said: String,
    ) -> BoxFuture<'_, Result<(), ChannelError>> {
        Box::pin(async move {
            let text = match &card {
                Some(card) => card.text(Some(&said), PIECE),
                None => card::cut(&said, PIECE),
            };
            let body = json!({
                "chat_id": posted.room.parse::<i64>().map_err(|_| ChannelError {
                    detail: format!("{} is not a Telegram chat", posted.room),
                })?,
                "message_id": posted.message.parse::<i64>().map_err(|_| ChannelError {
                    detail: format!("{} is not a Telegram message", posted.message),
                })?,
                "text": text,
                "link_preview_options": { "is_disabled": true },
            });
            self.call("editMessageText", body, TIMEOUT)
                .await
                .map(drop)
                .map_err(failed)
        })
    }

    fn presses(&self) -> mpsc::UnboundedReceiver<Press> {
        let (presses, heard) = mpsc::unbounded_channel();
        *self.presses.lock().expect("unpoisoned") = Some(presses);
        heard
    }

    fn direct_place(&self) -> Option<Place> {
        Some(Place::new(OWNER_DIRECT))
    }
}

fn failed(failed: Failed) -> ChannelError {
    ChannelError {
        detail: match failed {
            Failed::Fatal(detail) | Failed::Passing(detail, _) | Failed::Conflict(detail) => detail,
        },
    }
}

#[cfg(test)]
#[path = "telegram.test.rs"]
mod tests;
