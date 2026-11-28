use std::borrow::Cow;
use std::collections::HashMap;
use std::time::SystemTime;

use dotori_core::carried::Room;
use dotori_core::carried::carried;
use dotori_core::conversation::Action;
use dotori_core::conversation::Entry;
use dotori_core::llm::ModelTrace;
use dotori_core::message::Audience;
use dotori_core::message::Role;
use dotori_core::sub_session::SubSessionOutcome;
use dotori_core::tool::ToolCallId;
use jiff::Timestamp;
use jiff::tz::TimeZone;
use serde_json::value::RawValue;

use crate::channel::DECISION;
use crate::llm::reply::DELEGATE;
use crate::llm::reply::SAY;
use crate::llm::reply::Trace;
use crate::llm::reply::Use;

const SAID: &str = "Queued for delivery. A failure arrives as a message of its own.";
const DELEGATED: &str = "Started. The result arrives as a message of its own.";
const LOST: &str = "No result was recorded for this call.";
const NOTHING_NEW: &str = "Nothing new has happened.";
const LEFT_OUT: &str = "<earlier_conversation_left_out>The start of this conversation is not \
                        shown, to keep it short. Search memory for what was said \
                        before.</earlier_conversation_left_out>";
const SUMMARY_INTRO: &str = "The start of this conversation is not shown, to keep it short. \
                             This is a summary of it. It is past context, not new requests, and \
                             newer messages win when they disagree. Search memory for the exact \
                             words. Do not say that you read a summary. Carry on as if you \
                             remembered it.";
const LEFT_OUT_AFTER_SUMMARY: &str = "<conversation_left_out>Some of the conversation after \
                                      this summary is not shown either, to keep it short. \
                                      Search memory for it.</conversation_left_out>";

const CLIPPED_AT: usize = 2000;

/// The conversation as alternating turns, each reply placed right after what it answered.
#[derive(Debug, PartialEq)]
pub(crate) enum Turn {
    User(Vec<Part>),
    Assistant(Said),
}

/// User turns hold every tool result first, then notes, as both providers expect.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Part {
    Result(Outcome),
    Note(String),
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) struct Outcome {
    pub(crate) id: String,
    pub(crate) text: String,
    pub(crate) failed: bool,
}

/// One reply. `raw` is the provider's own content when the reply came from that provider,
/// so it is sent back unchanged. Otherwise the connector writes `calls` in its own form.
#[derive(Debug)]
pub(crate) struct Said {
    pub(crate) raw: Option<Box<RawValue>>,
    pub(crate) calls: Vec<Use>,
}

impl PartialEq for Said {
    fn eq(&self, other: &Self) -> bool {
        self.raw.as_ref().map(|raw| raw.get()) == other.raw.as_ref().map(|raw| raw.get())
            && self.calls == other.calls
    }
}

pub(crate) fn build(
    conversation: &[Entry],
    provider: &str,
    zone: &TimeZone,
    room: Room,
) -> Vec<Turn> {
    let carried = carried(conversation, room);
    let mut builder = Builder::default();
    match carried.summary {
        Some(summary) => {
            builder.notes.push(tagged(
                "earlier_conversation_summary",
                &[],
                &format!("{SUMMARY_INTRO}\n\n{summary}"),
            ));
            if carried.left_out {
                builder.notes.push(String::from(LEFT_OUT_AFTER_SUMMARY));
            }
        }
        None if carried.left_out => builder.notes.push(String::from(LEFT_OUT)),
        None => {}
    }
    for index in order(conversation, carried.start) {
        builder.add(index, &conversation[index], provider, zone);
    }
    builder.finish()
}

/// What a summary is asked to merge, as plain text with each entry as a block of its own. The
/// model's thinking and its raw replies are left out.
pub(crate) fn summary_input(
    previous: Option<&str>,
    conversation: &[Entry],
    zone: &TimeZone,
) -> String {
    let blocks: Vec<String> = order(conversation, 0)
        .into_iter()
        .flat_map(|index| summary_blocks(conversation, &conversation[index], zone))
        .collect();
    let mut input = String::new();
    if let Some(previous) = previous {
        input.push_str(&tagged("previous_summary", &[], previous));
        input.push_str(
            "

",
        );
    }
    input.push_str(&format!(
        "<conversation>
{}
</conversation>",
        blocks.join(
            "

"
        )
    ));
    input
}

fn summary_blocks(conversation: &[Entry], entry: &Entry, zone: &TimeZone) -> Vec<String> {
    match entry {
        Entry::Acted { actions, .. } => actions.iter().map(summary_action).collect(),
        Entry::ToolReturned { call, result } => {
            let mut attributes = Vec::new();
            if let Some(name) = tool_named(conversation, call) {
                attributes.push(("tool", name));
            }
            let (body, failed) = match result {
                Ok(output) => (output, false),
                Err(error) => (&error.detail, true),
            };
            if failed {
                attributes.push(("failed", "true"));
            }
            vec![tagged("tool_result", &attributes, &clip(body))]
        }
        Entry::SubSessionReturned { spec, outcome } => {
            let (status, text) = sub_session_result(outcome);
            vec![tagged(
                "sub_session",
                &[("title", &spec.title), ("status", status)],
                &clip(text),
            )]
        }
        Entry::Summarized { .. } => Vec::new(),
        other => vec![note(other, zone)],
    }
}

fn summary_action(action: &Action) -> String {
    match action {
        Action::Say {
            thread,
            text,
            privately,
        } => {
            let thread = thread.get().to_string();
            let mut attributes = vec![("thread", thread.as_str())];
            if *privately {
                attributes.push(("privately", "true"));
            }
            tagged("dotori_said", &attributes, text)
        }
        Action::UseTool(call) => tagged("dotori_used", &[("tool", &call.name)], &clip(&call.input)),
        Action::Delegate(spec) => tagged(
            "dotori_delegated",
            &[("title", &spec.title)],
            &clip(&spec.brief),
        ),
    }
}

fn tool_named<'a>(conversation: &'a [Entry], call: &ToolCallId) -> Option<&'a str> {
    conversation.iter().find_map(|entry| match entry {
        Entry::Acted { actions, .. } => actions.iter().find_map(|action| match action {
            Action::UseTool(used) if used.id == *call => Some(used.name.as_str()),
            _ => None,
        }),
        _ => None,
    })
}

fn clip(text: &str) -> Cow<'_, str> {
    match text.char_indices().nth(CLIPPED_AT) {
        Some((end, _)) => Cow::Owned(format!("{}…", &text[..end])),
        None => Cow::Borrowed(text),
    }
}

fn order(conversation: &[Entry], start: usize) -> Vec<usize> {
    let mut replies: Vec<(usize, usize)> = conversation
        .iter()
        .enumerate()
        .skip(start)
        .filter_map(|(index, entry)| match entry {
            Entry::Acted { answered, .. } => Some(((*answered).clamp(start, index), index)),
            _ => None,
        })
        .collect();
    replies.sort();
    let mut replies = replies.into_iter().peekable();
    let mut order = Vec::with_capacity(conversation.len() - start);
    for (index, entry) in conversation.iter().enumerate().skip(start) {
        while let Some((_, reply)) = replies.next_if(|(answered, _)| *answered <= index) {
            order.push(reply);
        }
        if !matches!(entry, Entry::Acted { .. }) {
            order.push(index);
        }
    }
    order.extend(replies.map(|(_, reply)| reply));
    order
}

#[derive(Default)]
struct Builder {
    turns: Vec<Turn>,
    expected: Vec<String>,
    results: HashMap<String, Outcome>,
    notes: Vec<String>,
}

impl Builder {
    fn add(&mut self, index: usize, entry: &Entry, provider: &str, zone: &TimeZone) {
        match entry {
            Entry::Acted { actions, trace, .. } => {
                self.reply(index, actions, trace.as_ref(), provider)
            }
            Entry::ToolReturned { call, result } => {
                let id = call.as_str();
                let (text, failed) = match result {
                    Ok(output) => (output.clone(), false),
                    Err(error) => (error.detail.clone(), true),
                };
                if self.expected.iter().any(|expected| expected == id)
                    && !self.results.contains_key(id)
                {
                    self.results.insert(
                        id.to_owned(),
                        Outcome {
                            id: id.to_owned(),
                            text,
                            failed,
                        },
                    );
                } else {
                    self.notes
                        .push(tagged("late_tool_result", &[("id", id)], &text));
                }
            }
            Entry::Summarized { .. } => {}
            other => self.notes.push(note(other, zone)),
        }
    }

    fn reply(
        &mut self,
        index: usize,
        actions: &[Action],
        trace: Option<&ModelTrace>,
        provider: &str,
    ) {
        let trace = trace
            .filter(|trace| trace.provider == provider)
            .and_then(|trace| serde_json::from_str::<Trace>(&trace.data).ok())
            .filter(|trace| trace.ids.len() == actions.len());
        let mut calls = Vec::new();
        let mut done = Vec::new();
        for (k, action) in actions.iter().enumerate() {
            let id = match (&trace, action) {
                (Some(trace), _) => trace.ids[k].clone(),
                (None, Action::UseTool(call)) => call.id.as_str().to_owned(),
                (None, _) => format!("call_{index}_{k}"),
            };
            let (name, input) = match action {
                Action::Say {
                    thread,
                    text,
                    privately,
                } => {
                    done.push(Outcome {
                        id: id.clone(),
                        text: String::from(SAID),
                        failed: false,
                    });
                    let mut input = serde_json::json!({ "thread": thread.get(), "text": text });
                    if *privately {
                        input["privately"] = serde_json::json!(true);
                    }
                    (String::from(SAY), input.to_string())
                }
                Action::Delegate(spec) => {
                    done.push(Outcome {
                        id: id.clone(),
                        text: String::from(DELEGATED),
                        failed: false,
                    });
                    (
                        String::from(DELEGATE),
                        match &spec.folder {
                            Some(folder) => serde_json::json!({
                                "title": spec.title,
                                "brief": spec.brief,
                                "folder": folder,
                            }),
                            None => serde_json::json!({ "title": spec.title, "brief": spec.brief }),
                        }
                        .to_string(),
                    )
                }
                Action::UseTool(call) => (call.name.clone(), call.input.clone()),
            };
            calls.push(Use { id, name, input });
        }
        let mut expected: Vec<String> = calls.iter().map(|call| call.id.clone()).collect();
        let raw = match trace {
            Some(trace) => {
                for rejected in trace.rejected {
                    expected.push(rejected.id.clone());
                    done.push(Outcome {
                        id: rejected.id,
                        text: rejected.detail,
                        failed: true,
                    });
                }
                Some(trace.raw)
            }
            None => None,
        };
        if raw.is_none() && calls.is_empty() {
            return;
        }
        self.flush();
        self.expected = expected;
        for outcome in done {
            self.results.insert(outcome.id.clone(), outcome);
        }
        if self.turns.is_empty() {
            self.turns
                .push(Turn::User(vec![Part::Note(String::from(NOTHING_NEW))]));
        }
        self.turns.push(Turn::Assistant(Said { raw, calls }));
    }

    fn flush(&mut self) {
        let mut parts: Vec<Part> = self
            .expected
            .drain(..)
            .map(|id| {
                let outcome = self.results.remove(&id);
                Part::Result(outcome.unwrap_or(Outcome {
                    id,
                    text: String::from(LOST),
                    failed: true,
                }))
            })
            .collect();
        parts.extend(self.notes.drain(..).map(Part::Note));
        if parts.is_empty() {
            return;
        }
        match self.turns.last_mut() {
            Some(Turn::User(earlier)) => earlier.extend(parts),
            _ => self.turns.push(Turn::User(parts)),
        }
    }

    fn finish(mut self) -> Vec<Turn> {
        self.flush();
        if matches!(self.turns.last(), None | Some(Turn::Assistant(_))) {
            self.turns
                .push(Turn::User(vec![Part::Note(String::from(NOTHING_NEW))]));
        }
        self.turns
    }
}

fn note(entry: &Entry, zone: &TimeZone) -> String {
    match entry {
        Entry::Heard { message, .. } if message.address.channel.as_str() == DECISION => tagged(
            "decided_call",
            &[("tool", message.address.place.as_str())],
            message.content.as_str(),
        ),
        Entry::Heard { thread, message } => {
            let role = match message.sender.role {
                Role::Owner => "owner",
                Role::Other => "other",
            };
            let thread = thread.get().to_string();
            let time = message.at.and_then(|at| local_time(at, zone));
            let mut attributes = vec![("thread", thread.as_str())];
            if let Some(time) = &time {
                attributes.push(("time", time));
            }
            attributes.extend([
                ("channel", message.address.channel.as_str()),
                ("place", message.address.place.as_str()),
                ("from", message.sender.name.as_str()),
                ("role", role),
            ]);
            let readers = message.audience.as_ref().map(readers);
            if let Some(readers) = &readers {
                attributes.push(("readers", readers));
            }
            tagged("message", &attributes, message.content.as_str())
        }
        Entry::SubSessionReturned { spec, outcome } => {
            let (status, text) = sub_session_result(outcome);
            tagged(
                "sub_session",
                &[("title", &spec.title), ("status", status)],
                text,
            )
        }
        Entry::Refused(Action::Say { thread, text, .. }) => tagged(
            "not_sent",
            &[
                ("thread", &thread.get().to_string()),
                ("reason", "no such thread"),
            ],
            text,
        ),
        Entry::Refused(Action::Delegate(spec)) => {
            tagged("not_started", &[("title", &spec.title)], &spec.brief)
        }
        Entry::Refused(Action::UseTool(call)) => {
            tagged("not_run", &[("tool", &call.name)], &call.input)
        }
        Entry::DeliveryFailed {
            thread,
            text,
            detail,
        } => tagged(
            "not_delivered",
            &[("thread", &thread.get().to_string()), ("reason", detail)],
            text,
        ),
        Entry::ModelFailed { detail } => tagged("model_failed", &[], detail),
        Entry::ChannelEnded { channel, detail } => tagged(
            "channel_ended",
            &[("channel", channel.as_str())],
            detail.as_deref().unwrap_or("It stopped without an error."),
        ),
        Entry::Acted { .. } | Entry::ToolReturned { .. } | Entry::Summarized { .. } => {
            String::new()
        }
    }
}

fn sub_session_result(outcome: &SubSessionOutcome) -> (&'static str, &str) {
    match outcome {
        SubSessionOutcome::Finished(text) => ("finished", text),
        SubSessionOutcome::Failed(text) => ("failed", text),
    }
}

fn readers(audience: &Audience) -> String {
    match audience {
        Audience::Owner => String::from("only the owner"),
        Audience::People { some, total } => {
            let others = total.saturating_sub(1);
            let person = if others == 1 { "person" } else { "people" };
            if some.is_empty() {
                format!("the owner and {others} other {person}")
            } else {
                format!(
                    "the owner and {others} other {person}, such as {}",
                    some.join(", ")
                )
            }
        }
        Audience::Anyone(scope) => format!("anyone in {scope}"),
    }
}

fn local_time(at: SystemTime, zone: &TimeZone) -> Option<String> {
    let at = Timestamp::try_from(at).ok()?;
    Some(
        at.to_zoned_datetime(zone.clone())
            .strftime("%Y-%m-%d %a %H:%M %:z")
            .to_string(),
    )
}

fn tagged(tag: &str, attributes: &[(&str, &str)], body: &str) -> String {
    let mut text = format!("<{tag}");
    for (name, value) in attributes {
        text.push_str(&format!(" {name}=\"{}\"", escape(value)));
    }
    text.push_str(&format!(">\n{}\n</{tag}>", escape(body)));
    text
}

fn escape(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

#[cfg(test)]
#[path = "transcript.test.rs"]
mod tests;
