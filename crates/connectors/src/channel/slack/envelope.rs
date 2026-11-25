use serde::Deserialize;
use serde_json::Value;
use serde_json::json;

use crate::channel::card::Card;
use crate::channel::card::WHOLE_ON_PC;
use crate::channel::card::cut;

const PIECE: usize = 3900;
const FIELD: usize = 1900;
const SECTION: usize = 2900;

/// What one Socket Mode frame asks of the connector.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Frame {
    /// Slack resends an envelope that is not acknowledged within a few seconds.
    pub(crate) ack: Option<String>,
    pub(crate) kind: Kind,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Kind {
    Hello,
    /// Slack is about to close this connection, so the connector opens a new one.
    Disconnect,
    Message(Arrived),
    /// Someone pressed a button of a card dotori posted.
    Pressed(Clicked),
    Nothing,
}

/// A button someone pressed, where its message is, and what the button carries.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Clicked {
    pub(crate) user: String,
    pub(crate) channel: String,
    pub(crate) ts: String,
    pub(crate) thread: Option<String>,
    pub(crate) value: String,
}

/// The owner's message to the assistant: a direct message, or a mention in a channel.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Arrived {
    pub(crate) id: String,
    pub(crate) place: String,
    /// The Slack conversation id, whose members can read the message.
    pub(crate) room: String,
    /// A direct message, which only the owner and the assistant read.
    pub(crate) direct: bool,
    pub(crate) user: String,
    pub(crate) text: String,
}

#[derive(Deserialize)]
struct Envelope {
    #[serde(rename = "type")]
    kind: String,
    envelope_id: Option<String>,
    payload: Option<Payload>,
}

#[derive(Deserialize)]
struct Payload {
    event: Option<Event>,
    #[serde(rename = "type")]
    kind: Option<String>,
    user: Option<Id>,
    channel: Option<Id>,
    container: Option<Container>,
    message: Option<Posted>,
    #[serde(default)]
    actions: Vec<Action>,
}

#[derive(Deserialize)]
struct Id {
    id: String,
}

#[derive(Deserialize)]
struct Container {
    channel_id: Option<String>,
    message_ts: Option<String>,
    thread_ts: Option<String>,
}

#[derive(Deserialize)]
struct Posted {
    ts: Option<String>,
    thread_ts: Option<String>,
}

#[derive(Deserialize)]
struct Action {
    value: Option<String>,
}

#[derive(Deserialize)]
struct Event {
    #[serde(rename = "type")]
    kind: String,
    subtype: Option<String>,
    channel: Option<String>,
    channel_type: Option<String>,
    user: Option<String>,
    bot_id: Option<String>,
    text: Option<String>,
    ts: Option<String>,
    thread_ts: Option<String>,
}

/// Reads one text frame. `bot` is the assistant's own Slack user id and `owner` is the owner's.
/// Everyone else's messages are acknowledged and dropped.
pub(crate) fn read(text: &str, bot: &str, owner: &str) -> Frame {
    let Ok(envelope) = serde_json::from_str::<Envelope>(text) else {
        return Frame {
            ack: None,
            kind: Kind::Nothing,
        };
    };
    let kind = match envelope.kind.as_str() {
        "hello" => Kind::Hello,
        "disconnect" => Kind::Disconnect,
        "events_api" => envelope
            .payload
            .and_then(|payload| payload.event)
            .and_then(|event| arrived(event, bot, owner))
            .map_or(Kind::Nothing, Kind::Message),
        "interactive" => envelope
            .payload
            .and_then(clicked)
            .map_or(Kind::Nothing, Kind::Pressed),
        _ => Kind::Nothing,
    };
    Frame {
        ack: envelope.envelope_id,
        kind,
    }
}

fn arrived(event: Event, bot: &str, owner: &str) -> Option<Arrived> {
    let direct = match (event.kind.as_str(), event.channel_type.as_deref()) {
        ("app_mention", _) => false,
        ("message", Some("im")) => true,
        _ => return None,
    };
    let plain_message = matches!(
        event.subtype.as_deref(),
        None | Some("file_share" | "thread_broadcast")
    );
    if !plain_message || event.bot_id.is_some() {
        return None;
    }
    let user = event.user?;
    if user != owner {
        return None;
    }
    let channel = event.channel?;
    let ts = event.ts?;
    let text = plain(&event.text?, bot);
    if text.is_empty() {
        return None;
    }
    let place = match (direct, event.thread_ts) {
        (true, None) => channel.clone(),
        (_, thread) => format!("{channel}/{}", thread.unwrap_or_else(|| ts.clone())),
    };
    Some(Arrived {
        id: format!("{channel}/{ts}"),
        place,
        room: channel,
        direct,
        user,
        text,
    })
}

fn clicked(payload: Payload) -> Option<Clicked> {
    if payload.kind.as_deref() != Some("block_actions") {
        return None;
    }
    let container = payload.container;
    let channel = payload
        .channel
        .map(|channel| channel.id)
        .or_else(|| container.as_ref()?.channel_id.clone())?;
    let ts = container
        .as_ref()
        .and_then(|container| container.message_ts.clone())
        .or_else(|| payload.message.as_ref()?.ts.clone())?;
    let thread = payload
        .message
        .and_then(|message| message.thread_ts)
        .or_else(|| container?.thread_ts)
        .filter(|thread| *thread != ts);
    Some(Clicked {
        user: payload.user?.id,
        channel,
        ts,
        thread,
        value: payload.actions.into_iter().next()?.value?,
    })
}

/// The blocks of `card`: its title, rows and notes, then its buttons, or once it is settled,
/// what came of it in their place.
pub(crate) fn blocks(card: Option<&Card>, said: Option<&str>) -> Value {
    let mut blocks = Vec::new();
    if let Some(card) = card {
        blocks.push(json!({
            "type": "section",
            "text": { "type": "mrkdwn", "text": format!("*{}*", escaped(&cut(&card.title, SECTION))) },
        }));
        for (label, value) in &card.rows {
            blocks.push(json!({
                "type": "section",
                "fields": [
                    { "type": "mrkdwn", "text": format!("*{}*", escaped(&cut(label, FIELD))) },
                    { "type": "plain_text", "text": cut(value, FIELD), "emoji": false },
                ],
            }));
        }
        let notes: Vec<Value> = card
            .cut
            .then_some(WHOLE_ON_PC)
            .into_iter()
            .chain(card.notes.iter().map(String::as_str))
            .map(|note| json!({ "type": "plain_text", "text": cut(note, SECTION), "emoji": false }))
            .collect();
        if !notes.is_empty() {
            blocks.push(json!({ "type": "context", "elements": notes }));
        }
        if said.is_none() && !card.buttons.is_empty() {
            let buttons: Vec<Value> = card
                .labels()
                .map(|(n, button, label)| {
                    let mut element = json!({
                        "type": "button",
                        "action_id": format!("dotori-{n}"),
                        "text": { "type": "plain_text", "text": label, "emoji": false },
                        "value": card.value(n),
                    });
                    if button.primary {
                        element["style"] = json!("primary");
                    }
                    element
                })
                .collect();
            blocks.push(json!({ "type": "actions", "elements": buttons }));
        }
    }
    if let Some(said) = said {
        blocks.push(json!({
            "type": "section",
            "text": { "type": "plain_text", "text": cut(said, SECTION), "emoji": false },
        }));
    }
    Value::Array(blocks)
}

fn plain(text: &str, bot: &str) -> String {
    let mut out = String::new();
    let mut rest = text;
    while let Some(open) = rest.find('<') {
        out.push_str(&rest[..open]);
        let Some(close) = rest[open..].find('>') else {
            break;
        };
        out.push_str(&markup(&rest[open + 1..open + close], bot));
        rest = &rest[open + close + 1..];
    }
    out.push_str(rest);
    out.replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&amp;", "&")
        .trim()
        .to_owned()
}

fn markup(inner: &str, bot: &str) -> String {
    let (target, label) = match inner.split_once('|') {
        Some((target, label)) => (target, Some(label)),
        None => (inner, None),
    };
    if target.strip_prefix('@') == Some(bot) {
        return String::new();
    }
    match (target.chars().next(), label) {
        (Some('#'), Some(label)) => format!("#{label}"),
        (Some('@'), Some(label)) => format!("@{label}"),
        (Some('!'), Some(label)) => label.to_owned(),
        (Some('!'), None) => format!("@{}", &target[1..]),
        (Some('#' | '@'), None) => target.to_owned(),
        (_, Some(label)) => format!("{label} ({target})"),
        (_, None) => target.to_owned(),
    }
}

/// Text as Slack shows it literally, so a reply cannot ping a channel or mention anyone.
pub(crate) fn escaped(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

/// Splits a long reply into messages Slack shows whole, at line breaks where it can.
pub(crate) fn pieces(text: &str) -> Vec<&str> {
    let mut pieces = Vec::new();
    let mut rest = text;
    while rest.chars().count() > PIECE {
        let limit = rest
            .char_indices()
            .nth(PIECE)
            .map_or(rest.len(), |(index, _)| index);
        let cut = rest[..limit]
            .rfind('\n')
            .filter(|&cut| cut > 0)
            .unwrap_or(limit);
        pieces.push(&rest[..cut]);
        rest = rest[cut..].trim_start_matches('\n');
    }
    if !rest.is_empty() || pieces.is_empty() {
        pieces.push(rest);
    }
    pieces
}

/// Splits a place back into the channel and the thread a reply goes to.
pub(crate) fn target(place: &str) -> (&str, Option<&str>) {
    match place.split_once('/') {
        Some((channel, thread)) => (channel, Some(thread)),
        None => (place, None),
    }
}

#[cfg(test)]
#[path = "envelope.test.rs"]
mod tests;
