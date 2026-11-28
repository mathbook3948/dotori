use std::ops::Range;

use crate::conversation::Action;
use crate::conversation::Entry;
use crate::conversation::MessageRef;
use crate::sub_session::SubSessionOutcome;

const PROMPT_AND_TOOLS_TOKENS: u32 = 30_000;
const LEAST_CHARS: usize = 20_000;

/// How much of the conversation a request carries, and when a summary starts, all counted in
/// characters and derived from the most a request carries.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Room {
    kept: usize,
}

impl Room {
    pub const DEFAULT: Room = Room { kept: 300_000 };

    /// The room a model's context window leaves after its reply and the reserve. One token is
    /// counted as one character, which is a cautious estimate: text takes fewer tokens than that
    /// in most languages.
    pub fn for_window(context_tokens: u32, output_tokens: u32) -> Room {
        let free = context_tokens
            .saturating_sub(output_tokens)
            .saturating_sub(PROMPT_AND_TOOLS_TOKENS);
        Room {
            kept: (free as usize).max(LEAST_CHARS),
        }
    }

    /// About how many characters of the conversation a request carries at most.
    pub fn kept(self) -> usize {
        self.kept
    }

    /// How far the start of what is carried moves at a time, so the start of the conversation
    /// stays the same between most requests and the provider can reuse its cache.
    pub fn step(self) -> usize {
        self.kept / 3
    }

    /// How much of the conversation after the latest summary's message starts a new summary.
    pub(crate) fn summarize_at(self) -> usize {
        self.kept * 5 / 6
    }

    fn tail(self) -> usize {
        self.kept / 3
    }

    fn most(self) -> usize {
        self.kept
    }
}

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
/// conversation over `room.kept()` is cut the same way with or without a summary.
pub fn carried(conversation: &[Entry], room: Room) -> Carried<'_> {
    let summary = latest_summary(conversation);
    let base = summary
        .as_ref()
        .and_then(|latest| anchor(conversation, latest))
        .unwrap_or(0);
    let start = start(&conversation[base..], room) + base;
    Carried {
        summary: summary.map(|latest| latest.text),
        start,
        left_out: start > base,
    }
}

struct Latest<'a> {
    index: usize,
    kept: &'a MessageRef,
    text: &'a str,
}

fn latest_summary(conversation: &[Entry]) -> Option<Latest<'_>> {
    conversation
        .iter()
        .enumerate()
        .rev()
        .find_map(|(index, entry)| match entry {
            Entry::Summarized { kept, text } => Some(Latest {
                index,
                kept,
                text: text.as_str(),
            }),
            _ => None,
        })
}

fn anchor(conversation: &[Entry], latest: &Latest<'_>) -> Option<usize> {
    conversation[..latest.index].iter().rposition(|entry| {
        matches!(entry, Entry::Heard { message, .. }
            if message.address.channel == latest.kept.channel && message.id == latest.kept.id)
    })
}

/// What the next summary takes in.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct Plan {
    /// The summary so far, which the new one merges.
    pub(crate) previous: Option<String>,
    /// The entries to summarize.
    pub(crate) covered: Range<usize>,
    /// The message the new summary names, which stays word for word.
    pub(crate) kept: MessageRef,
    /// How much of the conversation after the latest summary's message there is.
    pub(crate) weight: usize,
}

/// A summary is due when the conversation after the latest summary's message passes
/// `room.summarize_at()`. That message, or the summary itself when its message is gone, starts what is
/// counted. The new summary names the first message that leaves at most `room.tail()` after it, or the
/// last message when none does.
pub(crate) fn plan(conversation: &[Entry], room: Room) -> Option<Plan> {
    let latest = latest_summary(conversation);
    let base = match &latest {
        Some(latest) => anchor(conversation, latest).unwrap_or(latest.index),
        None => 0,
    };
    let total: usize = conversation[base..].iter().map(weight).sum();
    if total <= room.summarize_at() {
        return None;
    }
    let mut before = 0;
    let mut cut = None;
    let mut cut_before = 0;
    for (index, entry) in conversation.iter().enumerate().skip(base) {
        if index > base
            && let Entry::Heard { message, .. } = entry
        {
            cut = Some((index, message));
            cut_before = before;
            if total - before <= room.tail() {
                break;
            }
        }
        before += weight(entry);
    }
    let (cut, message) = cut?;
    let mut start = base;
    let mut covered = cut_before;
    while covered > room.most() && start < cut {
        covered -= weight(&conversation[start]);
        start += 1;
    }
    (start < cut).then(|| Plan {
        previous: latest.map(|latest| latest.text.to_owned()),
        covered: start..cut,
        kept: MessageRef {
            channel: message.address.channel.clone(),
            id: message.id.clone(),
        },
        weight: total,
    })
}

fn start(conversation: &[Entry], room: Room) -> usize {
    let total: usize = conversation.iter().map(weight).sum();
    if total <= room.kept {
        return 0;
    }
    let cut = (total - room.kept).div_ceil(room.step()) * room.step();
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
