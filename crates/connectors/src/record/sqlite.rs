use std::path::PathBuf;
use std::sync::Arc;
use std::sync::Mutex;

use dotori_core::BoxFuture;
use dotori_core::conversation::Action;
use dotori_core::conversation::Entry;
use dotori_core::message::Role;
use dotori_core::record::Record;
use dotori_core::record::RecordError;
use dotori_core::record::RecordStore;
use dotori_core::sub_session::SubSessionOutcome;
use rusqlite::Connection;
use rusqlite::params;

use crate::record::form::Line;

const VERSION: i64 = 1;
const SCHEMA: &str = "
CREATE TABLE IF NOT EXISTS records (
    seq INTEGER PRIMARY KEY AUTOINCREMENT,
    at_ms INTEGER NOT NULL,
    kind TEXT NOT NULL,
    channel TEXT,
    place TEXT,
    sender_id TEXT,
    sender_name TEXT,
    role TEXT,
    text TEXT,
    line TEXT NOT NULL
);
CREATE TRIGGER IF NOT EXISTS records_never_change BEFORE UPDATE ON records
BEGIN SELECT RAISE(ABORT, 'the raw record never changes'); END;
CREATE TRIGGER IF NOT EXISTS records_never_shrink BEFORE DELETE ON records
BEGIN SELECT RAISE(ABORT, 'the raw record never shrinks'); END;
";

/// The raw record in a SQLite file. Each record keeps its full form, plus its time, channel,
/// place, sender, role, and text in their own columns for search. Triggers refuse edits.
#[derive(Clone, Debug)]
pub struct SqliteStore {
    path: Arc<PathBuf>,
    connection: Arc<Mutex<Option<Connection>>>,
}

impl SqliteStore {
    /// The file and its folder are created on first use.
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self {
            path: Arc::new(path.into()),
            connection: Arc::new(Mutex::new(None)),
        }
    }

    fn with<T: Send + 'static>(
        &self,
        work: impl FnOnce(&mut Connection) -> Result<T, RecordError> + Send + 'static,
    ) -> BoxFuture<'static, Result<T, RecordError>> {
        let path = Arc::clone(&self.path);
        let connection = Arc::clone(&self.connection);
        Box::pin(async move {
            tokio::task::spawn_blocking(move || {
                let mut held = connection
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner());
                if held.is_none() {
                    *held = Some(open(&path)?);
                }
                let opened = held.as_mut().ok_or_else(|| fail("no connection"))?;
                let done = work(opened);
                if done.is_err() {
                    *held = None;
                }
                done
            })
            .await
            .unwrap_or_else(|error| Err(fail(format!("record task failed: {error}"))))
        })
    }
}

fn open(path: &PathBuf) -> Result<Connection, RecordError> {
    if let Some(folder) = path.parent() {
        std::fs::create_dir_all(folder).map_err(fail)?;
    }
    let connection = Connection::open(path).map_err(fail)?;
    connection
        .pragma_update(None, "journal_mode", "WAL")
        .map_err(fail)?;
    connection
        .pragma_update(None, "synchronous", "FULL")
        .map_err(fail)?;
    let version: i64 = connection
        .pragma_query_value(None, "user_version", |row| row.get(0))
        .map_err(fail)?;
    if version > VERSION {
        return Err(fail(format!(
            "{} is from a newer dotori (format {version})",
            path.display()
        )));
    }
    connection.execute_batch(SCHEMA).map_err(fail)?;
    connection
        .pragma_update(None, "user_version", VERSION)
        .map_err(fail)?;
    Ok(connection)
}

impl SqliteStore {
    /// Whether nothing has been recorded yet.
    pub async fn is_empty(&self) -> Result<bool, RecordError> {
        self.with(|connection| {
            connection
                .query_row("SELECT NOT EXISTS(SELECT 1 FROM records)", [], |row| {
                    row.get(0)
                })
                .map_err(fail)
        })
        .await
    }
}

impl RecordStore for SqliteStore {
    fn load(&self) -> BoxFuture<'_, Result<Vec<Record>, RecordError>> {
        self.with(|connection| {
            let mut query = connection
                .prepare("SELECT seq, line FROM records ORDER BY seq")
                .map_err(fail)?;
            let rows = query
                .query_map([], |row| {
                    Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?))
                })
                .map_err(fail)?;
            let mut records = Vec::new();
            for row in rows {
                let (seq, line) = row.map_err(fail)?;
                let record = serde_json::from_str::<Line>(&line)
                    .map_err(|error| error.to_string())
                    .and_then(Line::into_record)
                    .map_err(|detail| fail(format!("record {seq}: {detail}")))?;
                records.push(record);
            }
            Ok(records)
        })
    }

    fn append(&self, records: Vec<Record>) -> BoxFuture<'_, Result<(), RecordError>> {
        self.with(move |connection| {
            if records.is_empty() {
                return Ok(());
            }
            let batch = connection.transaction().map_err(fail)?;
            {
                let mut insert = batch
                    .prepare(
                        "INSERT INTO records \
                         (at_ms, kind, channel, place, sender_id, sender_name, role, text, line) \
                         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
                    )
                    .map_err(fail)?;
                for record in &records {
                    let line = Line::of(record);
                    let at_ms = i64::try_from(line.at_ms()).unwrap_or(i64::MAX);
                    let text = serde_json::to_string(&line).map_err(fail)?;
                    let columns = Columns::of(&record.entry);
                    insert
                        .execute(params![
                            at_ms,
                            columns.kind,
                            columns.channel,
                            columns.place,
                            columns.sender_id,
                            columns.sender_name,
                            columns.role,
                            columns.text,
                            text
                        ])
                        .map_err(fail)?;
                }
            }
            batch.commit().map_err(fail)
        })
    }
}

#[derive(Default)]
struct Columns {
    kind: &'static str,
    channel: Option<String>,
    place: Option<String>,
    sender_id: Option<String>,
    sender_name: Option<String>,
    role: Option<&'static str>,
    text: Option<String>,
}

impl Columns {
    fn of(entry: &Entry) -> Self {
        match entry {
            Entry::Heard { message, .. } => Self {
                kind: "heard",
                channel: Some(message.address.channel.as_str().to_owned()),
                place: Some(message.address.place.as_str().to_owned()),
                sender_id: Some(message.sender.id.as_str().to_owned()),
                sender_name: Some(message.sender.name.clone()),
                role: Some(match message.sender.role {
                    Role::Owner => "owner",
                    Role::Other => "other",
                }),
                text: Some(message.content.as_str().to_owned()),
            },
            Entry::Acted { actions, .. } => {
                let said: Vec<&str> = actions
                    .iter()
                    .filter_map(|action| match action {
                        Action::Say { text, .. } => Some(text.as_str()),
                        Action::UseTool(_) | Action::Delegate(_) => None,
                    })
                    .collect();
                Self {
                    kind: "acted",
                    role: Some("assistant"),
                    text: (!said.is_empty()).then(|| said.join("\n")),
                    ..Self::default()
                }
            }
            Entry::ToolReturned { .. } => Self {
                kind: "tool_returned",
                ..Self::default()
            },
            Entry::SubSessionReturned { spec, outcome } => Self {
                kind: "sub_task_returned",
                text: Some(match outcome {
                    SubSessionOutcome::Finished(text) | SubSessionOutcome::Failed(text) => {
                        format!("{}\n{text}", spec.title)
                    }
                }),
                ..Self::default()
            },
            Entry::Refused(_) => Self {
                kind: "refused",
                ..Self::default()
            },
            Entry::DeliveryFailed { .. } => Self {
                kind: "delivery_failed",
                ..Self::default()
            },
            Entry::ModelFailed { .. } => Self {
                kind: "model_failed",
                ..Self::default()
            },
            Entry::ChannelEnded { channel, .. } => Self {
                kind: "channel_ended",
                channel: Some(channel.as_str().to_owned()),
                ..Self::default()
            },
        }
    }
}

fn fail(error: impl std::fmt::Display) -> RecordError {
    RecordError {
        detail: error.to_string(),
    }
}

#[cfg(test)]
#[path = "sqlite.test.rs"]
mod tests;
