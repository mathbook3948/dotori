//! What a card that waits for the owner says, the same in the chat window and in a chat room.

use dotori_connectors::tool::basic::PURPOSE;

use crate::forget::CARRIERS_SHOWN;
use crate::forget::FORGET;
use crate::forget::NOTES_SHOWN;
use crate::forget::RECORDS_SHOWN;
use crate::words::words;

pub(crate) const SHOWN_VALUE: usize = 600;
const HIDDEN_FIELDS: [&str; 3] = ["records", "carriers", "notes"];
const SHELL_HIDDEN: [&str; 1] = ["seconds"];

/// Whether `tool` is one of dotori's own, whose card says what it does without naming the tool.
pub(crate) fn is_own(tool: &str) -> bool {
    matches!(tool, "run_shell" | "write_file" | "edit_file" | FORGET)
}

/// The label a field of a waiting call shows under, in the owner's words where dotori knows the
/// field.
pub(crate) fn field_label(key: &str) -> &str {
    let chat = &words().chat;
    match key {
        PURPOSE => chat.purpose_field,
        "command" => chat.command_field,
        "folder" => chat.folder_field,
        "path" => chat.path_field,
        "content" => chat.content_field,
        "old" => chat.old_field,
        "new" => chat.new_field,
        "all" => chat.all_field,
        NOTES_SHOWN => chat.notes_shown_field,
        RECORDS_SHOWN => chat.records_shown_field,
        CARRIERS_SHOWN => chat.carriers_shown_field,
        other => other,
    }
}

/// The fields of a waiting call as the card shows them, and whether any was shortened. An erase
/// shows what the notes and records say, not their numbers, and a command leaves out how long it
/// may run.
pub(crate) fn asked_fields(tool: &str, input: &str, whole: bool) -> (Vec<(String, String)>, bool) {
    let hidden: &[&str] = match tool {
        FORGET => &HIDDEN_FIELDS,
        "run_shell" => &SHELL_HIDDEN,
        _ => &[],
    };
    let fields: Vec<(String, String)> = match serde_json::from_str::<serde_json::Value>(input) {
        Ok(serde_json::Value::Object(fields)) if !fields.is_empty() => fields
            .into_iter()
            .filter(|(key, _)| !hidden.contains(&key.as_str()))
            .map(|(key, value)| {
                let shown = match value {
                    serde_json::Value::String(text) => text,
                    other => other.to_string(),
                };
                (key, shown)
            })
            .collect(),
        _ => vec![(String::from("input"), input.to_owned())],
    };
    let mut cut = false;
    let (purpose, rest): (Vec<_>, Vec<_>) = fields.into_iter().partition(|(key, _)| key == PURPOSE);
    let fields = purpose
        .into_iter()
        .chain(rest)
        .map(|(key, value)| {
            let (shown, shortened) = shown_value(&value, whole);
            cut |= shortened;
            (key, shown)
        })
        .collect();
    (fields, cut)
}

fn shown_value(value: &str, whole: bool) -> (String, bool) {
    let mut lines: Vec<&str> = Vec::new();
    for line in value.lines() {
        if line.trim().is_empty() && lines.last().is_some_and(|last| last.trim().is_empty()) {
            continue;
        }
        lines.push(line);
    }
    let joined = lines.join("\n");
    let total = joined.chars().count();
    if whole || total <= SHOWN_VALUE {
        return (joined, false);
    }
    let cut: String = joined.chars().take(SHOWN_VALUE).collect();
    (format!("{cut}\n{}", (words().chat.cut_note)(total)), true)
}

/// The title of a card for a call that waits, where `subject` names the assistant with its
/// particle, such as "Dotori가".
pub(crate) fn title(subject: &str) -> String {
    (words().chat.asking)(subject)
}

/// A warning said beside the buttons when allowing `tool` cannot be undone.
pub(crate) fn warning(tool: &str) -> Option<&'static str> {
    (tool == FORGET).then_some(words().chat.erase_hint)
}

/// What a card says once its call no longer waits.
pub(crate) fn lapsed() -> &'static str {
    words().chat.expired
}

/// What a card says under a command that would run in the sandbox.
pub(crate) fn sandboxed() -> String {
    (words().chat.sandboxed)()
}

/// The title of a card that asks to log in with a kept account.
pub(crate) fn log_in_title(host: &str, site: &str) -> String {
    (words().chat.log_in_with_kept)(host, site)
}

/// An account as a log-in card lists it.
pub(crate) fn log_in_account(host: &str, site: &str, id: &str) -> String {
    format!("{host} ({site}), {id}")
}

/// Where a log-in card's account goes, and that the model never sees its password.
pub(crate) fn log_in_note(subject: &str, host: &str, path: &str) -> String {
    (words().chat.log_in_note)(subject, host, path)
}

#[cfg(test)]
#[path = "asked.test.rs"]
mod tests;
