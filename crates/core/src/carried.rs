use crate::conversation::Action;
use crate::conversation::Entry;
use crate::sub_session::SubSessionOutcome;

/// About how many characters of the conversation a request carries at most.
pub const KEPT: usize = 300_000;
/// How far the start of what is carried moves at a time, so the start of the conversation stays
/// the same between most requests and the provider can reuse its cache.
pub const STEP: usize = 100_000;
const SMALL: usize = 200;

/// What a request carries of a conversation: the latest summary, if any, and the entries from
/// `start` on.
#[derive(Debug, PartialEq, Eq)]
pub struct Carried<'a> {
    pub summary: Option<&'a str>,
    pub start: usize,
    /// Whether entries after the summary, or after the start of the conversation when there is
    /// none, are left out.
    pub left_out: bool,
}

/// The latest summary starts what is carried at the message it names. A summary whose message is
/// gone, or not copied to this PC yet, leaves the whole conversation to cut. Past that start, a
/// conversation over `KEPT` is cut the same way with or without a summary.
pub fn carried(conversation: &[Entry]) -> Carried<'_> {
    let summary = conversation
        .iter()
        .enumerate()
        .rev()
        .find_map(|(index, entry)| match entry {
            Entry::Summarized { kept, text } => Some((index, kept, text.as_str())),
            _ => None,
        });
    let base = summary
        .and_then(|(index, kept, _)| {
            conversation[..index].iter().rposition(|entry| {
                matches!(entry, Entry::Heard { message, .. }
                    if message.address.channel == kept.channel && message.id == kept.id)
            })
        })
        .unwrap_or(0);
    let start = start(&conversation[base..]) + base;
    Carried {
        summary: summary.map(|(_, _, text)| text),
        start,
        left_out: start > base,
    }
}

fn start(conversation: &[Entry]) -> usize {
    let total: usize = conversation.iter().map(weight).sum();
    if total <= KEPT {
        return 0;
    }
    let cut = (total - KEPT).div_ceil(STEP) * STEP;
    let mut before = 0;
    let mut last_heard = 0;
    for (index, entry) in conversation.iter().enumerate() {
        if matches!(entry, Entry::Heard { .. }) {
            if before >= cut {
                return index;
            }
            last_heard = index;
        }
        before += weight(entry);
    }
    last_heard
}

/// How much of a request an entry takes, in characters, with a small cost for the entry itself.
/// A summary has none, because it is not carried as an entry.
pub fn weight(entry: &Entry) -> usize {
    if matches!(entry, Entry::Summarized { .. }) {
        return 0;
    }
    SMALL
        + match entry {
            Entry::Heard { message, .. } => chars(message.content.as_str()),
            Entry::Acted { actions, trace, .. } => match trace {
                Some(trace) => chars(&trace.data),
                None => actions
                    .iter()
                    .map(|action| match action {
                        Action::Say { text, .. } => chars(text),
                        Action::UseTool(call) => chars(&call.input),
                        Action::Delegate(spec) => chars(&spec.brief),
                    })
                    .sum(),
            },
            Entry::ToolReturned { result, .. } => match result {
                Ok(output) => chars(output),
                Err(error) => chars(&error.detail),
            },
            Entry::SubSessionReturned { outcome, .. } => match outcome {
                SubSessionOutcome::Finished(text) | SubSessionOutcome::Failed(text) => chars(text),
            },
            Entry::DeliveryFailed { text, .. } => chars(text),
            _ => 0,
        }
}

fn chars(text: &str) -> usize {
    text.chars().count()
}

#[cfg(test)]
#[path = "carried.test.rs"]
mod tests;
