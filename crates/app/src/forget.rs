//! Forgetting records at the owner's request. The model finds them with memory search and names
//! them by number; the owner sees what they hold on a card and allows it; then they leave the raw
//! record, the search index and its vectors, and the owner's other PCs.

use std::sync::Arc;
use std::sync::OnceLock;

use dotori_connectors::peer::copies::Copies;
use dotori_connectors::record::sqlite::Described;
use dotori_connectors::record::sqlite::SqliteStore;
use dotori_core::BoxFuture;
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

/// The name of the tool that forgets records.
pub(crate) const FORGET: &str = "erase_records";
const MOST: usize = 50;
const SHOWN: usize = 300;

/// Forgets records for good.
pub(crate) struct Forget {
    pub(crate) store: SqliteStore,
    pub(crate) memory: MemoryConfig,
    pub(crate) copies: Arc<OnceLock<Arc<Copies>>>,
}

impl Tool for Forget {
    fn name(&self) -> &str {
        FORGET
    }

    fn description(&self) -> &str {
        "Forget records for good, only when the owner asked you to forget or delete something. \
         Find them with search_memory first and give the record numbers it shows. They leave the \
         record, memory search and the owner's other PCs, and so does every record that carries \
         their numbers, such as a search result. Your own replies that repeated what they said \
         carry no number: search for those too and give their numbers. The owner sees what goes \
         and decides in the chat window."
    }

    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "records": {
                    "type": "array",
                    "items": { "type": "string" },
                    "description": "The record numbers search_memory shows, such as 019a…"
                }
            },
            "required": ["records"]
        })
    }

    fn call(&self, arguments: Value) -> ToolFuture<'_> {
        Box::pin(async move {
            let ids = ids(&arguments)?;
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
            let memory = self.memory.clone();
            let dropped = tokio::task::spawn_blocking(move || Memory::drop_forgotten(&memory))
                .await
                .map_err(|error| error.to_string())
                .and_then(|dropped| dropped);
            if let Some(copies) = self.copies.get() {
                copies.forget(&forgotten);
            }
            let mut said = format!(
                "Forgot {} record(s). They are gone from the record and from memory search, and \
                 the owner's other PCs forget them when they are linked. Do not repeat what they \
                 held.",
                forgotten.len()
            );
            if let Err(error) = dropped {
                said.push_str(&format!(
                    " The search index lets go of them on the next search: {error}"
                ));
            }
            Ok(said)
        })
    }
}

/// Shows the owner what a forget call would remove before it waits for their OK, by adding the
/// records' contents to the call. A number that is not in the record fails at once.
pub(crate) struct Previewed {
    pub(crate) inner: Arc<dyn Toolbox>,
    pub(crate) store: SqliteStore,
}

impl Toolbox for Previewed {
    fn specs(&self) -> Vec<ToolSpec> {
        self.inner.specs()
    }

    fn run(&self, mut call: ToolCall) -> BoxFuture<'_, Result<String, ToolError>> {
        if call.name != FORGET {
            return self.inner.run(call);
        }
        Box::pin(async move {
            let failed = |detail: String| ToolError { detail };
            let arguments: Value = serde_json::from_str(&call.input)
                .map_err(|error| failed(format!("the input is not JSON: {error}")))?;
            let ids = ids(&arguments).map_err(failed)?;
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
            let carriers: Vec<&str> = carrying.iter().map(|record| record.id.as_str()).collect();
            let mut input = json!({
                "records": ids,
                "carriers": carriers,
                "지울 기록": shown.join("\n"),
            });
            if !carrying.is_empty() {
                let also: Vec<String> = carrying.iter().map(|record| line(record, &zone)).collect();
                input["함께 지울 기록 (검색 결과 등)"] = json!(also.join("\n"));
            }
            call.input = input.to_string();
            self.inner.run(call).await
        })
    }
}

fn ids(arguments: &Value) -> Result<Vec<String>, String> {
    let ids: Vec<String> = arguments
        .get("records")
        .and_then(Value::as_array)
        .ok_or("give `records`, the record numbers to forget")?
        .iter()
        .map(|id| id.as_str().map(|id| id.trim().to_owned()))
        .collect::<Option<_>>()
        .ok_or("each record number is a string")?;
    if ids.is_empty() {
        return Err(String::from("give at least one record number"));
    }
    if ids.len() > MOST {
        return Err(format!("forget at most {MOST} records at a time"));
    }
    Ok(ids)
}

/// One record as the forget card shows it.
pub(crate) fn line(record: &Described, zone: &TimeZone) -> String {
    let when = Timestamp::from_millisecond(record.at_ms)
        .map(|at| at.to_zoned_datetime(zone.clone()))
        .map(|at| at.strftime("%Y-%m-%d %H:%M").to_string())
        .unwrap_or_default();
    let who = match (&record.sender, record.kind.as_str()) {
        (Some(sender), _) => sender.clone(),
        (None, "acted") => String::from("dotori"),
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
