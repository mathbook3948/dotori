//! The conversation windows show when dotori starts again, read back from the raw record: what
//! the owner said, what dotori answered, and the traffic of the other channels, each at the time
//! it was recorded.

use std::collections::HashMap;
use std::time::UNIX_EPOCH;

use dotori_connectors::channel::app::APP;
use dotori_connectors::channel::app::ToWindow;
use dotori_connectors::channel::slack::OWNER_DIRECT;
use dotori_core::conversation::Action;
use dotori_core::conversation::Entry;
use dotori_core::conversation::ThreadRef;
use dotori_core::message::Address;
use dotori_core::record::Record;

/// How many of the last lines windows are shown after a start.
pub(crate) const SHOWN: usize = 200;

/// The last `SHOWN` lines a window would have shown for `records`.
pub(crate) fn lines(records: &[Record]) -> Vec<ToWindow> {
    let mut threads: HashMap<ThreadRef, Address> = HashMap::new();
    let mut lines: Vec<Option<ToWindow>> = Vec::new();
    let mut said: Vec<(ThreadRef, &str, usize)> = Vec::new();
    for record in records {
        let at = millis(record);
        match &record.entry {
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
