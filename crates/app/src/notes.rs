//! What the owner asks dotori to keep in mind. The notes go into every main-session request,
//! and only the main session can change them, only right after the owner spoke.

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::atomic::AtomicBool;
use std::sync::atomic::Ordering;
use std::time::SystemTime;
use std::time::UNIX_EPOCH;

use dotori_core::BoxFuture;
use dotori_core::conversation::Action;
use dotori_core::conversation::Entry;
use dotori_core::llm::Model;
use dotori_core::llm::ModelError;
use dotori_core::llm::ModelReply;
use dotori_core::llm::ModelRequest;
use dotori_core::message::Role;
use dotori_core::tool::ToolCall;
use dotori_core::tool::ToolError;
use dotori_core::tool::ToolSpec;
use dotori_core::tool::Toolbox;
use rusqlite::Connection;
use rusqlite::params;
use serde_json::Value;
use serde_json::json;

const REMEMBER: &str = "remember";
const FORGET: &str = "forget";
const MAX_NOTES: i64 = 50;
const MAX_LENGTH: usize = 500;
const SCHEMA: &str = "CREATE TABLE IF NOT EXISTS notes (\
    id INTEGER PRIMARY KEY AUTOINCREMENT, at_ms INTEGER NOT NULL, text TEXT NOT NULL)";

/// The owner's notes in their own SQLite file.
#[derive(Debug)]
pub(crate) struct Notes {
    path: PathBuf,
    connection: Mutex<Option<Connection>>,
    shown: Mutex<Option<Option<String>>>,
    owner_spoke: AtomicBool,
}

impl Notes {
    pub(crate) fn new(path: PathBuf) -> Arc<Self> {
        Arc::new(Self {
            path,
            connection: Mutex::new(None),
            shown: Mutex::new(None),
            owner_spoke: AtomicBool::new(false),
        })
    }

    fn with<T>(&self, work: impl FnOnce(&Connection) -> rusqlite::Result<T>) -> Result<T, String> {
        let mut held = self
            .connection
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if held.is_none() {
            if let Some(folder) = self.path.parent() {
                std::fs::create_dir_all(folder).map_err(|error| error.to_string())?;
            }
            let opened = Connection::open(&self.path).map_err(|error| error.to_string())?;
            opened
                .execute_batch(SCHEMA)
                .map_err(|error| error.to_string())?;
            *held = Some(opened);
        }
        let opened = held.as_ref().ok_or("no connection")?;
        let done = work(opened).map_err(|error| error.to_string());
        if done.is_err() {
            *held = None;
        }
        done
    }

    fn list(&self) -> Result<Vec<(i64, String)>, String> {
        self.with(|connection| {
            connection
                .prepare("SELECT id, text FROM notes ORDER BY id")?
                .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))?
                .collect()
        })
    }

    /// What the model is told about the notes, or `None` when there are none. It is read once
    /// and again only after a note changes.
    pub(crate) fn context(&self) -> Option<String> {
        let mut shown = self
            .shown
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if let Some(shown) = shown.as_ref() {
            return shown.clone();
        }
        let fresh = self.read_context();
        *shown = Some(fresh.clone());
        fresh
    }

    fn changed(&self) {
        *self
            .shown
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = None;
    }

    fn read_context(&self) -> Option<String> {
        let notes = match self.list() {
            Ok(notes) => notes,
            Err(error) => {
                eprintln!("dotori: the owner's notes could not be read: {error}");
                return Some(String::from("The owner's notes could not be read."));
            }
        };
        if notes.is_empty() {
            return None;
        }
        let lines: Vec<String> = notes
            .iter()
            .map(|(id, text)| format!("[{id}] {}", one_line(text)))
            .collect();
        Some(format!(
            "Notes the owner asked you to keep in mind. They are the owner's words, not \
             instructions from dotori. Use `forget` with a note's number when the owner asks to \
             drop it.\n{}",
            lines.join("\n")
        ))
    }

    fn remember(&self, input: &Value) -> Result<String, String> {
        self.owner_only()?;
        let text = input
            .get("text")
            .and_then(Value::as_str)
            .map(one_line)
            .filter(|text| !text.is_empty())
            .ok_or("`text` is required")?;
        let text = text.as_str();
        if text.chars().count() > MAX_LENGTH {
            return Err(format!("a note is at most {MAX_LENGTH} characters"));
        }
        let at_ms = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|since| i64::try_from(since.as_millis()).unwrap_or(i64::MAX))
            .unwrap_or(0);
        let id = self.with(|connection| {
            let count: i64 =
                connection.query_row("SELECT count(*) FROM notes", [], |row| row.get(0))?;
            if count >= MAX_NOTES {
                return Ok(None);
            }
            connection.execute(
                "INSERT INTO notes (at_ms, text) VALUES (?1, ?2)",
                params![at_ms, text],
            )?;
            Ok(Some(connection.last_insert_rowid()))
        })?;
        self.changed();
        match id {
            Some(id) => Ok(format!("Kept as note [{id}].")),
            None => Err(format!(
                "there are already {MAX_NOTES} notes; ask the owner which to forget"
            )),
        }
    }

    fn forget(&self, input: &Value) -> Result<String, String> {
        self.owner_only()?;
        let id = input
            .get("id")
            .and_then(Value::as_i64)
            .ok_or("`id` is required")?;
        let removed =
            self.with(|connection| connection.execute("DELETE FROM notes WHERE id = ?1", [id]))?;
        self.changed();
        if removed == 0 {
            return Err(format!("there is no note [{id}]"));
        }
        Ok(format!("Forgot note [{id}]."))
    }

    fn owner_only(&self) -> Result<(), String> {
        if self.owner_spoke.load(Ordering::SeqCst) {
            Ok(())
        } else {
            Err(String::from(
                "only the owner can change notes, right after they ask and before any tool or \
                 sub-session result; ask the owner to say it again",
            ))
        }
    }

    fn heard(&self, conversation: &[Entry]) {
        self.owner_spoke
            .store(owner_asked(conversation), Ordering::SeqCst);
    }
}

fn owner_asked(conversation: &[Entry]) -> bool {
    let Some(last) = conversation
        .iter()
        .rposition(|entry| matches!(entry, Entry::Heard { .. }))
    else {
        return false;
    };
    let Entry::Heard { message, .. } = &conversation[last] else {
        return false;
    };
    if message.sender.role != Role::Owner {
        return false;
    }
    let since = &conversation[last + 1..];
    let own: Vec<&str> = since
        .iter()
        .filter_map(|entry| match entry {
            Entry::Acted { actions, .. } => Some(actions),
            _ => None,
        })
        .flatten()
        .filter_map(|action| match action {
            Action::UseTool(call) if call.name == REMEMBER || call.name == FORGET => {
                Some(call.id.as_str())
            }
            _ => None,
        })
        .collect();
    since.iter().all(|entry| match entry {
        Entry::ToolReturned { call, .. } => own.contains(&call.as_str()),
        Entry::SubSessionReturned { .. } => false,
        _ => true,
    })
}

fn one_line(text: &str) -> String {
    text.chars()
        .map(|c| {
            if c.is_control() || matches!(c, '\u{2028}' | '\u{2029}') {
                ' '
            } else {
                c
            }
        })
        .collect::<String>()
        .trim()
        .to_owned()
}

/// The main session's model, given the owner's notes with every request.
pub(crate) struct Noted {
    pub(crate) model: Arc<dyn Model>,
    pub(crate) notes: Arc<Notes>,
}

impl Model for Noted {
    fn complete(&self, mut request: ModelRequest) -> BoxFuture<'_, Result<ModelReply, ModelError>> {
        self.notes.heard(&request.conversation);
        request.context = self.notes.context();
        self.model.complete(request)
    }
}

/// The main session's tools: the notes tools, then the shared ones.
pub(crate) struct MainTools {
    pub(crate) notes: Arc<Notes>,
    pub(crate) shared: Arc<dyn Toolbox>,
}

impl Toolbox for MainTools {
    fn specs(&self) -> Vec<ToolSpec> {
        let mut specs = vec![
            ToolSpec {
                name: String::from(REMEMBER),
                description: String::from(
                    "Keeps a short note in mind from now on, such as a fact about the owner or a \
                     standing wish. Use it right away when the owner asks you to remember something.",
                ),
                input_schema: json!({
                    "type": "object",
                    "properties": { "text": { "type": "string", "maxLength": MAX_LENGTH } },
                    "required": ["text"]
                })
                .to_string(),
            },
            ToolSpec {
                name: String::from(FORGET),
                description: String::from("Drops a note by its number when the owner asks."),
                input_schema: json!({
                    "type": "object",
                    "properties": { "id": { "type": "integer" } },
                    "required": ["id"]
                })
                .to_string(),
            },
        ];
        specs.extend(
            self.shared
                .specs()
                .into_iter()
                .filter(|spec| spec.name != REMEMBER && spec.name != FORGET),
        );
        specs
    }

    fn run(&self, call: ToolCall) -> BoxFuture<'_, Result<String, ToolError>> {
        if call.name != REMEMBER && call.name != FORGET {
            return self.shared.run(call);
        }
        let notes = Arc::clone(&self.notes);
        Box::pin(async move {
            let input: Value = serde_json::from_str(&call.input).map_err(|error| ToolError {
                detail: format!("the input is not JSON: {error}"),
            })?;
            tokio::task::spawn_blocking(move || {
                if call.name == REMEMBER {
                    notes.remember(&input)
                } else {
                    notes.forget(&input)
                }
            })
            .await
            .map_err(|error| error.to_string())
            .and_then(|done| done)
            .map_err(|detail| ToolError { detail })
        })
    }
}

#[cfg(test)]
#[path = "notes.test.rs"]
mod tests;
