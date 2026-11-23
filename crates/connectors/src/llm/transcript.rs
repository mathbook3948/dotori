use std::collections::HashMap;

use dotori_core::conversation::Action;
use dotori_core::conversation::Entry;
use dotori_core::llm::ModelTrace;
use dotori_core::message::Role;
use dotori_core::sub_session::SubSessionOutcome;
use serde_json::value::RawValue;

use crate::llm::reply::DELEGATE;
use crate::llm::reply::SAY;
use crate::llm::reply::Trace;
use crate::llm::reply::Use;

const SAID: &str = "Queued for delivery. A failure arrives as a message of its own.";
const DELEGATED: &str = "Started. The result arrives as a message of its own.";
const LOST: &str = "No result was recorded for this call.";
const NOTHING_NEW: &str = "Nothing new has happened.";

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

pub(crate) fn build(conversation: &[Entry], provider: &str) -> Vec<Turn> {
    let mut builder = Builder::default();
    for index in order(conversation) {
        builder.add(index, &conversation[index], provider);
    }
    builder.finish()
}

fn order(conversation: &[Entry]) -> Vec<usize> {
    let mut replies: Vec<(usize, usize)> = conversation
        .iter()
        .enumerate()
        .filter_map(|(index, entry)| match entry {
            Entry::Acted { answered, .. } => Some(((*answered).min(index), index)),
            _ => None,
        })
        .collect();
    replies.sort();
    let mut replies = replies.into_iter().peekable();
    let mut order = Vec::with_capacity(conversation.len());
    for (index, entry) in conversation.iter().enumerate() {
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
    fn add(&mut self, index: usize, entry: &Entry, provider: &str) {
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
            other => self.notes.push(note(other)),
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
                Action::Say { thread, text } => {
                    done.push(Outcome {
                        id: id.clone(),
                        text: String::from(SAID),
                        failed: false,
                    });
                    (
                        String::from(SAY),
                        serde_json::json!({ "thread": thread.get(), "text": text }).to_string(),
                    )
                }
                Action::Delegate(spec) => {
                    done.push(Outcome {
                        id: id.clone(),
                        text: String::from(DELEGATED),
                        failed: false,
                    });
                    (
                        String::from(DELEGATE),
                        serde_json::json!({ "title": spec.title, "brief": spec.brief }).to_string(),
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

fn note(entry: &Entry) -> String {
    match entry {
        Entry::Heard { thread, message } => {
            let role = match message.sender.role {
                Role::Owner => "owner",
                Role::Other => "other",
            };
            tagged(
                "message",
                &[
                    ("thread", &thread.get().to_string()),
                    ("channel", message.address.channel.as_str()),
                    ("place", message.address.place.as_str()),
                    ("from", &message.sender.name),
                    ("role", role),
                ],
                message.content.as_str(),
            )
        }
        Entry::SubSessionReturned { spec, outcome } => {
            let (status, text) = match outcome {
                SubSessionOutcome::Finished(text) => ("finished", text),
                SubSessionOutcome::Failed(text) => ("failed", text),
            };
            tagged(
                "sub_session",
                &[("title", &spec.title), ("status", status)],
                text,
            )
        }
        Entry::Refused(Action::Say { thread, text }) => tagged(
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
        Entry::Acted { .. } | Entry::ToolReturned { .. } => String::new(),
    }
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
