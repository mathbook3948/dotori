//! The conversation windows show when dotori starts again, read back from the raw record: what
//! the owner said, what dotori answered, the traffic of the other channels, and the cards of the
//! calls that waited for the owner with what came of them, each at the time it was recorded.

use std::collections::HashMap;
use std::time::UNIX_EPOCH;

use dotori_connectors::channel::DECISION;
use dotori_connectors::channel::app::APP;
use dotori_connectors::channel::app::ToWindow;
use dotori_connectors::channel::slack::OWNER_DIRECT;
use dotori_core::conversation::Action;
use dotori_core::conversation::Entry;
use dotori_core::conversation::ThreadRef;
use dotori_core::message::Address;
use dotori_core::record::Record;
use dotori_core::tool::ToolCall;
use dotori_core::tool::ToolCallId;

use crate::confirm::DECLINED;
use crate::confirm::WAITING;
use crate::confirm::told_about;

/// How many of the last lines windows are shown after a start.
pub(crate) const SHOWN: usize = 200;

/// The last `SHOWN` lines a window would have shown for `records`.
pub(crate) fn lines(records: &[Record]) -> Vec<ToWindow> {
    let mut threads: HashMap<ThreadRef, Address> = HashMap::new();
    let mut lines: Vec<Option<ToWindow>> = Vec::new();
    let mut said: Vec<(ThreadRef, &str, usize)> = Vec::new();
    let mut called: HashMap<&ToolCallId, &ToolCall> = HashMap::new();
    let mut asked: Vec<(&ToolCallId, usize)> = Vec::new();
    for record in records {
        let at = millis(record);
        match &record.entry {
            Entry::Heard { thread, message } if message.address.channel.as_str() == DECISION => {
                threads
                    .entry(*thread)
                    .or_insert_with(|| message.address.clone());
                let text = message.content.as_str();
                let place = message.address.place.as_str();
                let card = told_about(&message.id).and_then(|(card, call)| {
                    let n = match call {
                        Some(call) => asked.iter().rposition(|(id, _)| **id == call)?,
                        None => asked.iter().position(|(_, line)| {
                            matches!(&lines[*line], Some(ToWindow::Ask { tool, .. }) if tool == place)
                        })?,
                    };
                    Some((card, asked.remove(n).1))
                });
                let Some((card, line)) = card else {
                    lines.push(Some(ToWindow::Traffic {
                        channel: String::from(DECISION),
                        place: place.to_owned(),
                        who: message.sender.name.clone(),
                        text: text.to_owned(),
                        at,
                    }));
                    continue;
                };
                if let Some(ToWindow::Ask { id, .. }) = &mut lines[line] {
                    *id = card;
                }
                lines.push(Some(ToWindow::Decided {
                    id: card,
                    ok: text != DECLINED,
                }));
                lines.push(Some(ToWindow::Ran {
                    id: card,
                    text: text.to_owned(),
                }));
            }
            Entry::ToolReturned {
                call,
                result: Ok(returned),
            } if returned == WAITING => {
                let Some(asking) = called.get(call) else {
                    continue;
                };
                asked.push((call, lines.len()));
                lines.push(Some(ToWindow::Ask {
                    id: 0,
                    tool: asking.name.clone(),
                    input: asking.input.clone(),
                    at,
                }));
            }
            Entry::Heard { thread, message } => {
                threads
                    .entry(*thread)
                    .or_insert_with(|| message.address.clone());
                let text = message.content.as_str().to_owned();
                if message.address.channel.as_str() == APP {
                    lines.push(Some(ToWindow::Owner { text, at }));
                } else {
                    lines.push(Some(ToWindow::Traffic {
                        channel: message.address.channel.as_str().to_owned(),
                        place: message.address.place.as_str().to_owned(),
                        who: message.sender.name.clone(),
                        text,
                        at,
                    }));
                }
            }
            Entry::Acted { actions, .. } => {
                for action in actions {
                    let Action::Say {
                        thread,
                        text,
                        privately,
                    } = action
                    else {
                        if let Action::UseTool(call) = action {
                            called.insert(&call.id, call);
                        }
                        continue;
                    };
                    let Some(address) = threads.get(thread) else {
                        continue;
                    };
                    said.push((*thread, text.as_str(), lines.len()));
                    if address.channel.as_str() == APP {
                        lines.push(Some(ToWindow::Reply {
                            text: text.clone(),
                            at,
                        }));
                    } else {
                        lines.push(Some(ToWindow::Traffic {
                            channel: address.channel.as_str().to_owned(),
                            place: if *privately {
                                String::from(OWNER_DIRECT)
                            } else {
                                address.place.as_str().to_owned()
                            },
                            who: String::from("dotori"),
                            text: text.clone(),
                            at,
                        }));
                    }
                }
            }
            Entry::DeliveryFailed { thread, text, .. } => {
                if let Some(n) = said
                    .iter()
                    .rposition(|(to, words, _)| to == thread && *words == text.as_str())
                {
                    let (_, _, line) = said.remove(n);
                    lines[line] = None;
                }
            }
            _ => {}
        }
    }
    for (_, line) in asked {
        let card = u64::try_from(line).unwrap_or(u64::MAX);
        if let Some(ToWindow::Ask { id, .. }) = &mut lines[line] {
            *id = card;
        }
        lines.push(Some(ToWindow::Lapsed { id: card }));
    }
    let mut lines: Vec<ToWindow> = lines.into_iter().flatten().collect();
    let skip = lines.len().saturating_sub(SHOWN);
    lines.split_off(skip)
}

fn millis(record: &Record) -> i64 {
    record
        .at
        .duration_since(UNIX_EPOCH)
        .map(|since| i64::try_from(since.as_millis()).unwrap_or(i64::MAX))
        .unwrap_or(0)
}

#[cfg(test)]
#[path = "history.test.rs"]
mod tests;
