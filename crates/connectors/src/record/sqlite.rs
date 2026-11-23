use std::collections::BTreeMap;
use std::path::Path;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::Mutex;
use std::time::SystemTime;
use std::time::UNIX_EPOCH;

use dotori_core::BoxFuture;
use dotori_core::conversation::Action;
use dotori_core::conversation::Entry;
use dotori_core::message::Role;
use dotori_core::record::Record;
use dotori_core::record::RecordError;
use dotori_core::record::RecordStore;
use dotori_core::sub_session::SubSessionOutcome;
use rusqlite::Connection;
use rusqlite::OpenFlags;
use rusqlite::params;
use uuid::Uuid;

use crate::record::form::Line;

const VERSION: i64 = 2;
const LEGACY_VERSION: i64 = 1;
const SCHEMA: &str = "
CREATE TABLE IF NOT EXISTS records (
    id TEXT PRIMARY KEY,
    device TEXT NOT NULL,
    at_ms INTEGER NOT NULL,
    kind TEXT NOT NULL,
    channel TEXT,
    place TEXT,
    sender_id TEXT,
    sender_name TEXT,
    role TEXT,
    text TEXT,
    line TEXT NOT NULL
) WITHOUT ROWID;
CREATE TRIGGER IF NOT EXISTS records_never_change BEFORE UPDATE ON records
BEGIN SELECT RAISE(ABORT, 'the raw record never changes'); END;
CREATE TRIGGER IF NOT EXISTS records_never_shrink BEFORE DELETE ON records
BEGIN SELECT RAISE(ABORT, 'the raw record never shrinks'); END;
";

/// The raw record in SQLite, one file per year in one folder, such as `2026.sqlite3`.
/// Each record has a UUID v7 number that only grows on this PC, the PC that made it, its full
/// form, and its time, channel, place, sender, role, and text in their own columns for search.
/// Triggers refuse edits.
#[derive(Clone, Debug)]
pub struct SqliteStore {
    folder: Arc<PathBuf>,
    device: Arc<str>,
    book: Arc<Mutex<Book>>,
}

#[derive(Debug, Default)]
struct Book {
    years: BTreeMap<i64, Connection>,
    last: Option<Uuid>,
}

impl SqliteStore {
    /// `device` names this PC in every record it makes. The folder is created on first use.
    pub fn new(folder: impl Into<PathBuf>, device: impl Into<Arc<str>>) -> Self {
        Self {
            folder: Arc::new(folder.into()),
            device: device.into(),
            book: Arc::new(Mutex::new(Book::default())),
        }
    }

    fn with<T: Send + 'static>(
        &self,
        work: impl FnOnce(&mut Book, &Path, &str) -> Result<T, RecordError> + Send + 'static,
    ) -> BoxFuture<'static, Result<T, RecordError>> {
        let folder = Arc::clone(&self.folder);
        let device = Arc::clone(&self.device);
        let book = Arc::clone(&self.book);
        Box::pin(async move {
            tokio::task::spawn_blocking(move || {
                let mut held = book.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
                let done = work(&mut held, &folder, &device);
                if done.is_err() {
                    held.years.clear();
                }
                done
            })
            .await
            .unwrap_or_else(|error| Err(fail(format!("record task failed: {error}"))))
        })
    }

    /// Whether no year file holds a record yet. The files are only read, so the folder can still
    /// be replaced afterwards.
    pub async fn is_empty(&self) -> Result<bool, RecordError> {
        self.with(|_, folder, _| {
            for year in years(folder)? {
                let path = folder.join(format!("{year:04}.sqlite3"));
                let connection =
                    Connection::open_with_flags(&path, OpenFlags::SQLITE_OPEN_READ_ONLY)
                        .map_err(fail)?;
                let held =
                    connection.query_row("SELECT EXISTS(SELECT 1 FROM records)", [], |row| {
                        row.get::<_, bool>(0)
                    });
                match held {
                    Ok(true) => return Ok(false),
                    Ok(false) => {}
                    Err(error) if error.to_string().contains("no such table") => {}
                    Err(error) => return Err(fail(error)),
                }
            }
            Ok(true)
        })
        .await
    }

    /// Stores records from another store, each in the file of its own year.
    pub async fn import(&self, records: Vec<Record>) -> Result<(), RecordError> {
        let mut rest = records.as_slice();
        while let Some(first) = rest.first() {
            let year = year_of(first.at);
            let same = rest
                .iter()
                .take_while(|record| year_of(record.at) == year)
                .count();
            self.append(rest[..same].to_vec()).await?;
            rest = &rest[same..];
        }
        Ok(())
    }
}

impl Book {
    fn year(&mut self, folder: &Path, year: i64) -> Result<&mut Connection, RecordError> {
        match self.years.entry(year) {
            std::collections::btree_map::Entry::Occupied(held) => Ok(held.into_mut()),
            std::collections::btree_map::Entry::Vacant(free) => {
                Ok(free.insert(open(&folder.join(format!("{year:04}.sqlite3")))?))
            }
        }
    }

    fn seed(&mut self, folder: &Path) -> Result<(), RecordError> {
        for year in years(folder)? {
            let highest: Option<String> = self
                .year(folder, year)?
                .query_row("SELECT max(id) FROM records", [], |row| row.get(0))
                .map_err(fail)?;
            if let Some(highest) = highest {
                let highest = Uuid::parse_str(&highest).map_err(fail)?;
                self.last = self.last.max(Some(highest));
            }
        }
        Ok(())
    }

    fn next_id(&mut self, at_ms: u64) -> Uuid {
        let secs = at_ms / 1000;
        let nanos = u32::try_from(at_ms % 1000).unwrap_or(0) * 1_000_000;
        let fresh = Uuid::new_v7(uuid::Timestamp::from_unix(uuid::NoContext, secs, nanos));
        let id = match self.last {
            Some(last) if fresh <= last => Uuid::from_u128(last.as_u128().wrapping_add(1)),
            _ => fresh,
        };
        self.last = Some(id);
        id
    }
}

fn open(path: &Path) -> Result<Connection, RecordError> {
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
    if version != 0 && version < VERSION {
        return Err(fail(format!(
            "{} is in an older format ({version})",
            path.display()
        )));
    }
    connection.execute_batch(SCHEMA).map_err(fail)?;
    connection
        .pragma_update(None, "user_version", VERSION)
        .map_err(fail)?;
    Ok(connection)
}

fn years(folder: &Path) -> Result<Vec<i64>, RecordError> {
    let entries = match std::fs::read_dir(folder) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => return Err(fail(error)),
    };
    let mut years = Vec::new();
    for entry in entries {
        let name = entry.map_err(fail)?.file_name();
        let Some(year) = name
            .to_str()
            .and_then(|name| name.strip_suffix(".sqlite3"))
            .filter(|stem| stem.len() == 4 && stem.bytes().all(|byte| byte.is_ascii_digit()))
            .and_then(|stem| stem.parse().ok())
        else {
            continue;
        };
        years.push(year);
    }
    years.sort_unstable();
    Ok(years)
}

/// The UTC year `at` falls in.
pub fn year_of(at: SystemTime) -> i64 {
    let seconds = match at.duration_since(UNIX_EPOCH) {
        Ok(since) => i64::try_from(since.as_secs()).unwrap_or(i64::MAX),
        Err(before) => -i64::try_from(before.duration().as_secs()).unwrap_or(i64::MAX),
    };
    let shifted = seconds.div_euclid(86_400) + 719_468;
    let era = shifted.div_euclid(146_097);
    let day_of_era = shifted.rem_euclid(146_097);
    let year_of_era =
        (day_of_era - day_of_era / 1460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let month_index = (5 * day_of_year + 2) / 153;
    year_of_era + era * 400 + i64::from(month_index >= 10)
}

/// Reads a raw record from before it was split by year, in the order it was written. Closing it
/// folds its write-ahead log back in, so the file can then be moved aside on its own.
pub fn read_legacy(path: &Path) -> Result<Vec<Record>, RecordError> {
    let connection =
        Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_WRITE).map_err(fail)?;
    let version: i64 = connection
        .pragma_query_value(None, "user_version", |row| row.get(0))
        .map_err(fail)?;
    if version != LEGACY_VERSION {
        return Err(fail(format!(
            "{} is not a format {LEGACY_VERSION} record (format {version})",
            path.display()
        )));
    }
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
        records.push(parse(&line).map_err(|detail| fail(format!("record {seq}: {detail}")))?);
    }
    Ok(records)
}

fn parse(line: &str) -> Result<Record, String> {
    serde_json::from_str::<Line>(line)
        .map_err(|error| error.to_string())
        .and_then(Line::into_record)
}

impl RecordStore for SqliteStore {
    fn load(&self) -> BoxFuture<'_, Result<Vec<Record>, RecordError>> {
        self.with(|book, folder, _| {
            let mut found = Vec::new();
            for year in years(folder)? {
                let connection = book.year(folder, year)?;
                let mut query = connection
                    .prepare("SELECT id, line FROM records")
                    .map_err(fail)?;
                let rows = query
                    .query_map([], |row| {
                        Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
                    })
                    .map_err(fail)?;
                for row in rows {
                    let (id, line) = row.map_err(fail)?;
                    let number = Uuid::parse_str(&id)
                        .map_err(|error| fail(format!("record {id}: {error}")))?;
                    let record =
                        parse(&line).map_err(|detail| fail(format!("record {id}: {detail}")))?;
                    found.push((number, record));
                }
            }
            found.sort_by_key(|(number, _)| *number);
            if let Some((last, _)) = found.last() {
                book.last = book.last.max(Some(*last));
            }
            Ok(found.into_iter().map(|(_, record)| record).collect())
        })
    }

    fn append(&self, records: Vec<Record>) -> BoxFuture<'_, Result<(), RecordError>> {
        self.with(move |book, folder, device| {
            let Some(first) = records.first() else {
                return Ok(());
            };
            if book.last.is_none() {
                book.seed(folder)?;
            }
            let year = year_of(first.at);
            let lines: Vec<Line> = records.iter().map(Line::of).collect();
            let ids: Vec<Uuid> = lines.iter().map(|line| book.next_id(line.at_ms())).collect();
            let connection = book.year(folder, year)?;
            let batch = connection.transaction().map_err(fail)?;
            {
                let mut insert = batch
                    .prepare(
                        "INSERT INTO records \
                         (id, device, at_ms, kind, channel, place, sender_id, sender_name, role, text, line) \
                         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
                    )
                    .map_err(fail)?;
                for ((record, line), id) in records.iter().zip(&lines).zip(&ids) {
                    let at_ms = i64::try_from(line.at_ms()).unwrap_or(i64::MAX);
                    let text = serde_json::to_string(line).map_err(fail)?;
                    let columns = Columns::of(&record.entry);
                    insert
                        .execute(params![
                            id.to_string(),
                            device,
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
