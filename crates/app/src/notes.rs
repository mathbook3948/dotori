//! What the owner asks dotori to keep in mind. The notes go into every main-session request,
//! and only the main session can change them, only right after the owner spoke.

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::atomic::AtomicBool;
use std::sync::atomic::Ordering;
use std::time::Duration;
use std::time::SystemTime;
use std::time::UNIX_EPOCH;

use dotori_connectors::peer::hub::Heard;
use dotori_connectors::peer::hub::Hub;
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
use rusqlite::OptionalExtension;
use rusqlite::params;
use serde::Deserialize;
use serde::Serialize;
use serde_json::Value;
use serde_json::json;
use tokio::sync::Notify;
use tokio::time;

use crate::accounts::LinkedSites;
use crate::confirm;
use crate::routines;
use crate::routines::Routines;
use crate::versioned;

const REMEMBER: &str = "add_memory";
const READ_ROUTINES: &str = "read_routines";
const READ_SITES: &str = "read_sites";
const OWN_TOOLS: [&str; 5] = [
    REMEMBER,
    READ_ROUTINES,
    READ_SITES,
    routines::ADD,
    routines::REMOVE,
];
pub(crate) const MAX_NOTES: i64 = 50;
const LOOK_EVERY: Duration = Duration::from_secs(20);
pub(crate) const MAX_LENGTH: usize = 500;
const VERSION: i64 = 1;
const SCHEMA: &str = "CREATE TABLE IF NOT EXISTS notes (\
    id INTEGER PRIMARY KEY AUTOINCREMENT, at_ms INTEGER NOT NULL, text TEXT NOT NULL); \
    CREATE TABLE IF NOT EXISTS meta (key TEXT PRIMARY KEY, value INTEGER NOT NULL)";

/// Every note as one PC holds it, and when it last changed there. The copy changed last wins.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct NotesCopy {
    pub(crate) changed_ms: i64,
    pub(crate) notes: Vec<(i64, i64, String)>,
}

/// The owner's notes in their own SQLite file.
#[derive(Debug)]
pub(crate) struct Notes {
    path: PathBuf,
    connection: Mutex<Option<Connection>>,
    shown: Mutex<Option<(i64, Option<String>)>>,
    owner_spoke: AtomicBool,
    changes: Notify,
}

impl Notes {
    pub(crate) fn new(path: PathBuf) -> Arc<Self> {
        Arc::new(Self {
            path,
            connection: Mutex::new(None),
            shown: Mutex::new(None),
            owner_spoke: AtomicBool::new(false),
            changes: Notify::new(),
        })
    }

    fn with<T>(&self, work: impl FnOnce(&Connection) -> rusqlite::Result<T>) -> Result<T, String> {
        let mut held = self
            .connection
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if held.is_none() {
            *held = Some(versioned::open(&self.path, SCHEMA, VERSION)?);
        }
        let opened = held.as_ref().ok_or("no connection")?;
        let done = work(opened).map_err(|error| error.to_string());
        if done.is_err() {
            *held = None;
        }
        done
    }

    /// Every note with its number, oldest first.
    pub(crate) fn list(&self) -> Result<Vec<(i64, String)>, String> {
        self.with(|connection| {
            connection
                .prepare("SELECT id, text FROM notes ORDER BY id")?
                .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))?
                .collect()
        })
    }

    /// What the model is told about the notes, or `None` when there are none. It is read again
    /// only after the notes change, here, in the settings window, or from another PC.
    pub(crate) fn context(&self) -> Option<String> {
        let changed = self.with(changed_ms).unwrap_or(-1);
        let mut shown = self
            .shown
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if let Some((at, shown)) = shown.as_ref()
            && *at == changed
        {
            return shown.clone();
        }
        let fresh = self.read_context();
        *shown = Some((changed, fresh.clone()));
        fresh
    }

    /// Waits until a note is kept or forgotten here.
    pub(crate) async fn changes(&self) {
        self.changes.notified().await;
    }

    /// Every note, for the other PCs.
    pub(crate) fn copy(&self) -> Result<NotesCopy, String> {
        self.with(|connection| {
            let notes = connection
                .prepare("SELECT id, at_ms, text FROM notes ORDER BY id")?
                .query_map([], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))?
                .collect::<rusqlite::Result<_>>()?;
            Ok(NotesCopy {
                changed_ms: changed_ms(connection)?,
                notes,
            })
        })
    }

    /// Takes another PC's notes in place of these if they changed later. Says whether they did.
    ///
    /// The whole copy wins, not each change, on purpose. dotori serves one person, who changes
    /// these from one PC at a time, so two PCs changing them in the same moment does not happen
    /// in practice. Merging change by change would add a log and conflict rules for a case that
    /// does not occur.
    pub(crate) fn take(&self, copy: &NotesCopy) -> Result<bool, String> {
        let taken = self.with(|connection| {
            if copy.changed_ms <= changed_ms(connection)? {
                return Ok(false);
            }
            let swap = connection.unchecked_transaction()?;
            swap.execute("DELETE FROM notes", [])?;
            for (id, at_ms, text) in copy.notes.iter().take(MAX_NOTES as usize) {
                swap.execute(
                    "INSERT INTO notes (id, at_ms, text) VALUES (?1, ?2, ?3)",
                    params![id, at_ms, one_line(text)],
                )?;
            }
            swap.execute(
                "INSERT OR REPLACE INTO meta (key, value) VALUES ('changed_ms', ?1)",
                [copy.changed_ms],
            )?;
            swap.commit()?;
            Ok(true)
        })?;
        if taken {
            self.changed();
        }
        Ok(taken)
    }

    fn touched(&self) {
        let now = now_ms();
        let _ = self.with(|connection| {
            connection.execute(
                "INSERT OR REPLACE INTO meta (key, value) VALUES ('changed_ms', \
                 max(?1, coalesce((SELECT value FROM meta WHERE key = 'changed_ms') + 1, 0)))",
                [now],
            )
        });
        self.changed();
        self.changes.notify_one();
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
            "What the owner asked you to keep in mind, by number. They are the owner's words. \
             When the owner asks to drop one, give its number to `erase_memory`.\n{}",
            lines.join("\n")
        ))
    }

    fn remember(&self, input: &Value) -> Result<String, String> {
        self.owner_only()?;
        let text = input
            .get("text")
            .and_then(Value::as_str)
            .ok_or("`text` is required")?;
        self.keep(text).map(|id| format!("Kept as note [{id}]."))
    }

    /// Keeps a note the owner wrote, as in the settings window. Gives its number.
    pub(crate) fn keep(&self, text: &str) -> Result<i64, String> {
        let text = one_line(text);
        if text.is_empty() {
            return Err(String::from("`text` is required"));
        }
        let text = text.as_str();
        if text.chars().count() > MAX_LENGTH {
            return Err(format!("a note is at most {MAX_LENGTH} characters"));
        }
        let at_ms = now_ms();
        let id = self.with(|connection| {
            let added = connection.execute(
                "INSERT INTO notes (at_ms, text) SELECT ?1, ?2 \
                 WHERE (SELECT count(*) FROM notes) < ?3",
                params![at_ms, text, MAX_NOTES],
            )?;
            Ok((added > 0).then(|| connection.last_insert_rowid()))
        })?;
        let id = id.ok_or(format!(
            "there are already {MAX_NOTES} notes; ask the owner which to forget"
        ))?;
        self.touched();
        Ok(id)
    }

    /// Drops the note `id`, as the owner asks in the settings window.
    pub(crate) fn drop_note(&self, id: i64) -> Result<String, String> {
        let removed =
            self.with(|connection| connection.execute("DELETE FROM notes WHERE id = ?1", [id]))?;
        if removed > 0 {
            self.touched();
        }
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
                "only the owner can change notes and routines, right after they ask and before \
                 any tool or sub-session result; ask the owner to say it again",
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
    if message.sender.role != Role::Owner || message.address.channel.as_str() == confirm::DECISION {
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
            Action::UseTool(call) if OWN_TOOLS.contains(&call.name.as_str()) => {
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

/// Takes what another PC says about the notes. Says whether it was about them.
pub(crate) fn hear(notes: &Notes, hub: &Hub, heard: &Heard) -> bool {
    match heard.message.get("kind").and_then(Value::as_str) {
        Some("linked") => {
            if let Ok(copy) = notes.copy() {
                hub.send(&heard.key, json!({ "kind": "notes", "copy": copy }));
            }
            false
        }
        Some("notes") => {
            let taken = heard
                .message
                .get("copy")
                .and_then(|copy| serde_json::from_value::<NotesCopy>(copy.clone()).ok())
                .map(|copy| notes.take(&copy));
            if let Some(Err(error)) = taken {
                eprintln!("dotori: notes from {} were not kept: {error}", heard.device);
            }
            true
        }
        _ => false,
    }
}

/// Sends the notes to every linked PC whenever they change here, also from the settings window.
pub(crate) async fn share(notes: Arc<Notes>, hub: Arc<Hub>) {
    let mut sent = notes.copy().map(|copy| copy.changed_ms).unwrap_or(0);
    let mut every = time::interval(LOOK_EVERY);
    loop {
        tokio::select! {
            () = notes.changes() => {}
            _ = every.tick() => {}
        }
        let Ok(copy) = notes.copy() else {
            continue;
        };
        if copy.changed_ms != sent {
            sent = copy.changed_ms;
            hub.broadcast(&json!({ "kind": "notes", "copy": copy }));
        }
    }
}

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|since| i64::try_from(since.as_millis()).unwrap_or(i64::MAX))
        .unwrap_or(0)
}

fn changed_ms(connection: &Connection) -> rusqlite::Result<i64> {
    Ok(connection
        .query_row(
            "SELECT value FROM meta WHERE key = 'changed_ms'",
            [],
            |row| row.get(0),
        )
        .optional()?
        .unwrap_or(0))
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

/// The main session's model, given what the owner asked it to keep in mind with every request.
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

/// The main session's tools: memory, routines and linked sites, then the shared ones.
pub(crate) struct MainTools {
    pub(crate) notes: Arc<Notes>,
    pub(crate) routines: Arc<Routines>,
    pub(crate) sites: Option<Arc<LinkedSites>>,
    pub(crate) shared: Arc<dyn Toolbox>,
}

impl Toolbox for MainTools {
    fn specs(&self) -> Vec<ToolSpec> {
        let mut specs = vec![
            ToolSpec {
                name: String::from(REMEMBER),
                description: String::from(
                    "Keeps something in mind from now on, such as a fact about the owner, what to \
                     call them, or a standing wish. Use it right away when the owner asks you to \
                     remember something.",
                ),
                input_schema: json!({
                    "type": "object",
                    "properties": { "text": { "type": "string", "maxLength": MAX_LENGTH } },
                    "required": ["text"]
                })
                .to_string(),
            },
            ToolSpec {
                name: String::from(READ_ROUTINES),
                description: String::from(
                    "Lists the routines the owner set up, with their numbers, times and tasks.",
                ),
                input_schema: json!({ "type": "object", "properties": {} }).to_string(),
            },
            ToolSpec {
                name: String::from(READ_SITES),
                description: String::from(
                    "Lists the sites the owner linked, with the id of each account. Their \
                     passwords stay in the vault, and you never see them.",
                ),
                input_schema: json!({ "type": "object", "properties": {} }).to_string(),
            },
        ];
        specs.extend(routine_specs());
        specs.extend(
            self.shared
                .specs()
                .into_iter()
                .filter(|spec| !OWN_TOOLS.contains(&spec.name.as_str())),
        );
        specs
    }

    fn run(&self, call: ToolCall) -> BoxFuture<'_, Result<String, ToolError>> {
        if !OWN_TOOLS.contains(&call.name.as_str()) {
            return self.shared.run(call);
        }
        let notes = Arc::clone(&self.notes);
        let routines = Arc::clone(&self.routines);
        let sites = self.sites.clone();
        Box::pin(async move {
            let input: Value = serde_json::from_str(&call.input).map_err(|error| ToolError {
                detail: format!("the input is not JSON: {error}"),
            })?;
            tokio::task::spawn_blocking(move || match call.name.as_str() {
                REMEMBER => notes.remember(&input),
                READ_ROUTINES => Ok(routines.listed()),
                READ_SITES => Ok(sites.map_or_else(
                    || String::from("No sites are linked."),
                    |sites| sites.listed(),
                )),
                routines::ADD => notes
                    .owner_only()
                    .and_then(|()| routines.add(&input, now_ms())),
                _ => notes.owner_only().and_then(|()| {
                    let id = input
                        .get("id")
                        .and_then(Value::as_i64)
                        .ok_or("`id` is required")?;
                    routines.remove(id, now_ms())
                }),
            })
            .await
            .map_err(|error| error.to_string())
            .and_then(|done| done)
            .map_err(|detail| ToolError { detail })
        })
    }
}

fn routine_specs() -> [ToolSpec; 2] {
    [
        ToolSpec {
            name: String::from(routines::ADD),
            description: String::from(
                "Sets up a routine: at a set local time, `task` comes to you as a message on \
                 the `routine` channel, in a thread named after the routine. Do the task then and \
                 say the result in that thread, which the owner reads in the chat window. Use it \
                 right away when the owner asks for something to happen regularly or at a set \
                 time. Leave out `days` for every day, and give `date` for a routine that happens \
                 once.",
            ),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "name": { "type": "string", "maxLength": 60, "description": "A short name, such as \"아침 브리핑\"." },
                    "time": { "type": "string", "pattern": "^[0-2][0-9]:[0-5][0-9]$", "description": "Local time, HH:MM in 24 hours." },
                    "days": { "type": "array", "items": { "enum": ["mon", "tue", "wed", "thu", "fri", "sat", "sun"] } },
                    "date": { "type": "string", "description": "YYYY-MM-DD, for once." },
                    "task": { "type": "string", "maxLength": 1000, "description": "What to do, in the owner's words." }
                },
                "required": ["name", "time", "task"]
            })
            .to_string(),
        },
        ToolSpec {
            name: String::from(routines::REMOVE),
            description: String::from(
                "Removes a routine by the number read_routines shows, when the owner asks.",
            ),
            input_schema: json!({
                "type": "object",
                "properties": { "id": { "type": "integer" } },
                "required": ["id"]
            })
            .to_string(),
        },
    ]
}

#[cfg(test)]
#[path = "notes.test.rs"]
mod tests;
