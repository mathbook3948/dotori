//! Erasing memory at the owner's request: what the owner asked dotori to keep in mind, by its
//! number, and records, which the model finds with memory search and names by number. The owner
//! sees what they hold on a card and allows it; then they leave the raw record, the search index
//! and its vectors, the kept notes, and the owner's other PCs.

use std::sync::Arc;
use std::sync::Mutex;
use std::sync::OnceLock;

use dotori_connectors::peer::copies::Copies;
use dotori_connectors::record::sqlite::Described;
use dotori_connectors::record::sqlite::SqliteStore;
use dotori_core::BoxFuture;
use dotori_core::runtime::Forgets;
use dotori_core::tool::ToolCall;
use dotori_core::tool::ToolError;
use dotori_core::tool::ToolSpec;
use dotori_core::tool::Toolbox;
use dotori_mcp::Tool;
use dotori_mcp::ToolFuture;
use dotori_mcp::memory::Memory;
use dotori_mcp::memory::MemoryConfig;
use jiff::Timestamp;
use jiff::tz::TimeZone;
use serde_json::Value;
use serde_json::json;

use crate::notes::Notes;

/// The name of the tool that erases memory.
pub(crate) const FORGET: &str = "erase_memory";
/// The fields a card shows, as the keys the card turns into the owner's words.
pub(crate) const NOTES_SHOWN: &str = "notes_shown";
pub(crate) const RECORDS_SHOWN: &str = "records_shown";
pub(crate) const CARRIERS_SHOWN: &str = "carriers_shown";
const MOST: usize = 50;
const SHOWN: usize = 300;

/// Erases memory for good.
pub(crate) struct Forget {
    pub(crate) store: SqliteStore,
    pub(crate) memory: MemoryConfig,
    pub(crate) copies: Arc<OnceLock<Arc<Copies>>>,
    pub(crate) notes: Arc<Notes>,
    /// The agent running the main session here, if any, which drops what was erased.
    pub(crate) agent: Arc<Mutex<Option<Forgets>>>,
}

impl Tool for Forget {
    fn name(&self) -> &str {
        FORGET
    }

    fn description(&self) -> &str {
        "Erases memory for good, only when the owner asked you to forget or delete something. \
         Give `notes` by the numbers of what you keep in mind, and `records` by the numbers \
         search_memory shows. Records leave the record, memory search and the owner's other PCs, \
         and so does every record that carries their numbers, such as a search result. Your own \
         replies that repeated what they said carry no number: search for those too and give \
         their numbers. The owner sees what goes and decides in the chat window."
    }

    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "notes": {
                    "type": "array",
                    "items": { "type": "integer" },
                    "description": "The numbers of what you keep in mind."
                },
                "records": {
                    "type": "array",
                    "items": { "type": "string" },
                    "description": "The record numbers search_memory shows, such as 019a…"
                }
            }
        })
    }

    fn call(&self, arguments: Value) -> ToolFuture<'_> {
        Box::pin(async move {
            let (ids, notes) = asked(&arguments)?;
            let mut said = Vec::new();
            if !notes.is_empty() {
                let kept = Arc::clone(&self.notes);
                let erased = tokio::task::spawn_blocking(move || {
                    notes
                        .into_iter()
                        .map(|id| kept.drop_note(id).map(|_| ()))
                        .collect::<Result<Vec<()>, String>>()
                })
                .await
                .map_err(|error| error.to_string())??;
                said.push(format!(
                    "Erased {} note(s) from what you keep in mind.",
                    erased.len()
                ));
            }
            if ids.is_empty() {
                return Ok(said.join(" "));
            }
            let carriers: Vec<String> = arguments
                .get("carriers")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .filter_map(Value::as_str)
                .map(str::to_owned)
                .collect();
            let forgotten = self
                .store
                .forget(ids, carriers)
                .await
                .map_err(|error| error.to_string())?;
            let running = self.agent.lock().expect("unpoisoned").clone();
            if let Some(agent) = running {
                agent.forget(forgotten.entries).await;
            }
            let memory = self.memory.clone();
            let dropped = tokio::task::spawn_blocking(move || Memory::drop_forgotten(&memory))
                .await
                .map_err(|error| error.to_string())
                .and_then(|dropped| dropped);
            if let Some(copies) = self.copies.get() {
                copies.forget(&forgotten.list);
            }
            said.push(format!(
                "Erased {} record(s). They are gone from the record and from memory search, and \
                 the owner's other PCs forget them when they are linked. Do not repeat what they \
                 held.",
                forgotten.list.len()
            ));
            if let Err(error) = dropped {
                said.push(format!(
                    "The search index lets go of them on the next search: {error}"
                ));
            }
            Ok(said.join(" "))
        })
    }
}

/// Shows the owner what an erase call would remove before it waits for their OK, by adding what
/// the notes and records say to the call. A number that is not there fails at once.
pub(crate) struct Previewed {
    pub(crate) inner: Arc<dyn Toolbox>,
    pub(crate) store: SqliteStore,
    pub(crate) notes: Arc<Notes>,
}

impl Toolbox for Previewed {
    fn specs(&self) -> Vec<ToolSpec> {
        self.inner.specs()
    }

    fn waits_for_owner(&self, call: &ToolCall) -> bool {
        self.inner.waits_for_owner(call)
    }

    fn run(&self, mut call: ToolCall) -> BoxFuture<'_, Result<String, ToolError>> {
        if call.name != FORGET {
            return self.inner.run(call);
        }
        Box::pin(async move {
            let failed = |detail: String| ToolError { detail };
            let arguments: Value = serde_json::from_str(&call.input)
                .map_err(|error| failed(format!("the input is not JSON: {error}")))?;
            let (ids, notes) = asked(&arguments).map_err(failed)?;
            let kept = self.notes.list().map_err(failed)?;
            let noted: Vec<String> = notes
                .iter()
                .map(|id| {
                    kept.iter()
                        .find(|(kept, _)| kept == id)
                        .map(|(id, text)| format!("[{id}] {text}"))
                        .ok_or_else(|| failed(format!("there is no note [{id}]")))
                })
                .collect::<Result<_, _>>()?;
            let mut input = json!({ "records": ids, "notes": notes });
            if !noted.is_empty() {
                input[NOTES_SHOWN] = json!(noted.join("\n"));
            }
            if !ids.is_empty() {
                let found = self.store.described(ids.clone()).await.map_err(|error| {
                    failed(format!(
                        "{error}; find the record numbers with search_memory first"
                    ))
                })?;
                let carrying = self.store.carrying(ids.clone()).await.map_err(|error| {
                    failed(format!(
                        "the records that carry them cannot be read: {error}"
                    ))
                })?;
                let zone = TimeZone::system();
                let shown: Vec<String> = found.iter().map(|record| line(record, &zone)).collect();
                let carriers: Vec<&str> =
                    carrying.iter().map(|record| record.id.as_str()).collect();
                input["carriers"] = json!(carriers);
                input[RECORDS_SHOWN] = json!(shown.join("\n"));
                if !carrying.is_empty() {
                    let also: Vec<String> =
                        carrying.iter().map(|record| line(record, &zone)).collect();
                    input[CARRIERS_SHOWN] = json!(also.join("\n"));
                }
            }
            call.input = input.to_string();
            self.inner.run(call).await
        })
    }
}

fn asked(arguments: &Value) -> Result<(Vec<String>, Vec<i64>), String> {
    let listed = |key: &str| {
        arguments
            .get(key)
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default()
    };
    let ids: Vec<String> = listed("records")
        .iter()
        .map(|id| id.as_str().map(|id| id.trim().to_owned()))
        .collect::<Option<_>>()
        .ok_or("each record number is a string")?;
    let notes: Vec<i64> = listed("notes")
        .iter()
        .map(Value::as_i64)
        .collect::<Option<_>>()
        .ok_or("each note number is a whole number")?;
    if ids.is_empty() && notes.is_empty() {
        return Err(String::from(
            "give `notes` or `records`, the numbers of what to erase",
        ));
    }
    if ids.len() + notes.len() > MOST {
        return Err(format!("erase at most {MOST} at a time"));
    }
    Ok((ids, notes))
}

/// One record as the forget card shows it.
pub(crate) fn line(record: &Described, zone: &TimeZone) -> String {
    let when = Timestamp::from_millisecond(record.at_ms)
        .map(|at| at.to_zoned_datetime(zone.clone()))
        .map(|at| at.strftime("%Y-%m-%d %H:%M").to_string())
        .unwrap_or_default();
    let who = match (&record.sender, record.kind.as_str()) {
        (Some(sender), _) => sender.clone(),
        (None, "acted") => String::from("비서"),
        (None, "tool_returned") => String::from("도구 결과"),
        (None, "sub_task_returned") => String::from("서브 세션 결과"),
        (None, kind) => kind.to_owned(),
    };
    let text = record
        .text
        .as_deref()
        .unwrap_or("(내용은 원본 기록에만 있어요)")
        .replace('\n', " ");
    let mut cut: String = text.chars().take(SHOWN).collect();
    if cut.len() < text.len() {
        cut.push('…');
    }
    format!("{when} {who}: {cut}")
}

#[cfg(test)]
#[path = "forget.test.rs"]
mod tests;
