use serde::Deserialize;

const PIECE: usize = 3900;

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
    Nothing,
}

/// The owner's message to the assistant: a direct message, or a mention in a channel.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Arrived {
    pub(crate) id: String,
    pub(crate) place: String,
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
        user,
        text,
    })
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
