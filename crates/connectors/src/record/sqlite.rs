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
use dotori_core::record::Marked;
use dotori_core::record::Record;
use dotori_core::record::RecordError;
use dotori_core::record::RecordStore;
use dotori_core::sub_session::SubSessionOutcome;
use rusqlite::Connection;
use rusqlite::OpenFlags;
use rusqlite::OptionalExtension;
use rusqlite::params;
use serde::Deserialize;
use serde::Serialize;
use uuid::Uuid;

use crate::record::form::Line;

const VERSION: i64 = 5;
const BEFORE_ARRIVALS: i64 = 2;
const LEGACY_VERSION: i64 = 1;
const SETTLE_TRIES: usize = 20;
const SETTLE_WAIT: std::time::Duration = std::time::Duration::from_millis(100);
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
CREATE TABLE IF NOT EXISTS arrived (
    seq INTEGER PRIMARY KEY AUTOINCREMENT,
    id TEXT NOT NULL UNIQUE
);
CREATE TRIGGER IF NOT EXISTS records_never_change BEFORE UPDATE ON records
BEGIN SELECT RAISE(ABORT, 'the raw record never changes'); END;
CREATE TABLE IF NOT EXISTS forgotten (
    n INTEGER PRIMARY KEY AUTOINCREMENT,
    id TEXT NOT NULL UNIQUE,
    device TEXT NOT NULL,
    held INTEGER NOT NULL DEFAULT 0
);
DROP TRIGGER IF EXISTS records_never_shrink;
CREATE TRIGGER IF NOT EXISTS records_shrink_only_when_forgotten BEFORE DELETE ON records
WHEN NOT EXISTS (SELECT 1 FROM forgotten WHERE id = OLD.id)
BEGIN SELECT RAISE(ABORT, 'the raw record loses only what the owner asked to forget'); END;
CREATE TRIGGER IF NOT EXISTS forgotten_never_changes BEFORE UPDATE ON forgotten
WHEN NEW.n != OLD.n OR NEW.id != OLD.id OR NEW.device != OLD.device OR NEW.held < OLD.held
BEGIN SELECT RAISE(ABORT, 'what was forgotten stays forgotten'); END;
CREATE TRIGGER IF NOT EXISTS forgotten_never_shrinks BEFORE DELETE ON forgotten
BEGIN SELECT RAISE(ABORT, 'what was forgotten stays forgotten'); END;
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

/// A record the owner asked to forget, by its number and the PC that made it. Only these numbers
/// stay, so a copy that arrives later is refused and the other PCs can forget it too.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Forgotten {
    pub id: String,
    pub device: String,
}

/// What `forget` removed. `entries` are the ones that could be read back, for whoever still
/// holds them in memory.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Forgot {
    pub list: Vec<Forgotten>,
    pub entries: Vec<Entry>,
}

/// A record as the owner would recognize it, to show before it is forgotten.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Described {
    pub id: String,
    pub at_ms: i64,
    pub kind: String,
    pub sender: Option<String>,
    pub text: Option<String>,
}

/// A record as it travels between PCs: its number, the PC that made it, and its full form.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Copied {
    pub id: String,
    pub device: String,
    pub line: String,
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

    /// The highest record number each PC made, among the records here.
    pub async fn marks(&self) -> Result<BTreeMap<String, String>, RecordError> {
        self.with(|book, folder, _| {
            let mut marks = BTreeMap::<String, String>::new();
            for year in years(folder)? {
                let connection = book.year(folder, year)?;
                let mut query = connection
                    .prepare(
                        "SELECT device, max(id) FROM \
                         (SELECT device, id FROM records \
                          UNION ALL SELECT device, id FROM forgotten WHERE held = 1) \
                         GROUP BY device",
                    )
                    .map_err(fail)?;
                let rows = query
                    .query_map([], |row| {
                        Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
                    })
                    .map_err(fail)?;
                for row in rows {
                    let (device, highest) = row.map_err(fail)?;
                    let mark = marks.entry(device).or_default();
                    if highest > *mark {
                        *mark = highest;
                    }
                }
            }
            Ok(marks)
        })
        .await
    }

    /// The records `device` made with numbers above `after`, in order, at most `count` of them
    /// and about `bytes` long.
    pub async fn after(
        &self,
        device: &str,
        after: &str,
        count: usize,
        bytes: usize,
    ) -> Result<Vec<Copied>, RecordError> {
        let device = device.to_owned();
        let after = after.to_owned();
        self.with(move |book, folder, _| {
            let mut found = Vec::new();
            for year in years(folder)? {
                let connection = book.year(folder, year)?;
                let mut query = connection
                    .prepare(
                        "SELECT id, line FROM records WHERE device = ?1 AND id > ?2 \
                         ORDER BY id LIMIT ?3",
                    )
                    .map_err(fail)?;
                let rows = query
                    .query_map(params![device, after, count as i64], |row| {
                        Ok(Copied {
                            id: row.get(0)?,
                            device: device.clone(),
                            line: row.get(1)?,
                        })
                    })
                    .map_err(fail)?;
                for row in rows {
                    found.push(row.map_err(fail)?);
                }
            }
            found.sort_by(|a, b| a.id.cmp(&b.id));
            let mut size = 0;
            let kept = found
                .iter()
                .take(count)
                .take_while(|copied| {
                    let fits = size == 0 || size + copied.line.len() <= bytes;
                    size += copied.line.len();
                    fits
                })
                .count();
            found.truncate(kept);
            Ok(found)
        })
        .await
    }

    /// Stores records another PC made, keeping their numbers. Records already here are passed
    /// over. Says how many were new.
    pub async fn copy_in(&self, copies: Vec<Copied>) -> Result<usize, RecordError> {
        self.with(move |book, folder, _| {
            let mut parsed = Vec::with_capacity(copies.len());
            for copied in &copies {
                let number = Uuid::parse_str(&copied.id)
                    .map_err(|error| fail(format!("record {}: {error}", copied.id)))?;
                if number.hyphenated().to_string() != copied.id {
                    return Err(fail(format!(
                        "record {} is not written as usual",
                        copied.id
                    )));
                }
                let line: Line = serde_json::from_str(&copied.line)
                    .map_err(|error| fail(format!("record {}: {error}", copied.id)))?;
                let at_ms = line.at_ms();
                let record = line
                    .into_record()
                    .map_err(|detail| fail(format!("record {}: {detail}", copied.id)))?;
                parsed.push((copied, at_ms, record));
            }
            let mut added = 0;
            let mut by_year = BTreeMap::<i64, Vec<_>>::new();
            let held = years(folder)?;
            let mut refused = Vec::new();
            'rows: for (copied, at_ms, record) in parsed {
                for year in &held {
                    let stone: Option<(String, bool)> = book
                        .year(folder, *year)?
                        .query_row(
                            "SELECT device, held FROM forgotten WHERE id = ?1",
                            [&copied.id],
                            |row| Ok((row.get(0)?, row.get(1)?)),
                        )
                        .optional()
                        .map_err(fail)?;
                    if let Some((device, held)) = stone
                        && device == copied.device
                    {
                        if !held {
                            refused.push((*year, copied.id.clone()));
                        }
                        continue 'rows;
                    }
                }
                by_year
                    .entry(year_of(record.at))
                    .or_default()
                    .push((copied, at_ms, record));
            }
            for (year, rows) in by_year {
                let connection = book.year(folder, year)?;
                let batch = connection.transaction().map_err(fail)?;
                for (copied, at_ms, record) in rows {
                    if insert_and_note_arrival(
                        &batch,
                        &copied.id,
                        &copied.device,
                        at_ms,
                        &record.entry,
                        &copied.line,
                    )? {
                        added += 1;
                    }
                }
                batch.commit().map_err(fail)?;
            }
            for (year, id) in refused {
                book.year(folder, year)?
                    .execute("UPDATE forgotten SET held = 1 WHERE id = ?1", [&id])
                    .map_err(fail)?;
            }
            Ok(added)
        })
        .await
    }

    /// Forgets the records numbered `ids` at the owner's request, together with `carriers`, the
    /// records the owner was shown that carry their numbers, such as the search result that found
    /// them. Each is removed from the raw record and its number kept as forgotten. Fails,
    /// forgetting nothing, if any of `ids` is not here; a carrier already gone is passed over.
    /// Every summary made after the earliest of them goes too, since it may hold what they said.
    pub async fn forget(
        &self,
        ids: Vec<String>,
        carriers: Vec<String>,
    ) -> Result<Forgot, RecordError> {
        self.with(move |book, folder, _| {
            let mut found = Vec::new();
            for id in &ids {
                let (year, device) =
                    held(book, folder, id)?.ok_or_else(|| fail(format!("no record {id}")))?;
                found.push((
                    year,
                    Forgotten {
                        id: id.clone(),
                        device,
                    },
                ));
            }
            for id in carriers {
                if found.iter().any(|(_, known)| known.id == id) {
                    continue;
                }
                if let Some((year, device)) = held(book, folder, &id)? {
                    found.push((year, Forgotten { id, device }));
                }
            }
            let earliest = found
                .iter()
                .min_by(|(_, a), (_, b)| a.id.cmp(&b.id))
                .map(|(year, forgotten)| (*year, forgotten.id.clone()));
            if let Some((year, id)) = earliest {
                for (year, summary, device) in summaries_after(book, folder, year, &id)? {
                    if !found.iter().any(|(_, known)| known.id == summary) {
                        found.push((
                            year,
                            Forgotten {
                                id: summary,
                                device,
                            },
                        ));
                    }
                }
            }
            let mut entries = Vec::new();
            for (year, forgotten) in &found {
                let connection = book.year(folder, *year)?;
                let line: String = connection
                    .query_row(
                        "SELECT line FROM records WHERE id = ?1",
                        [&forgotten.id],
                        |row| row.get(0),
                    )
                    .map_err(fail)?;
                if let Ok(record) = parse(&line) {
                    entries.push(record.entry);
                }
                forget_in(connection, forgotten, true)?;
            }
            settle(book, folder)?;
            Ok(Forgot {
                list: found.into_iter().map(|(_, forgotten)| forgotten).collect(),
                entries,
            })
        })
        .await
    }

    /// Takes in what another PC forgot. A record here is removed, unless the PC named as its maker
    /// is not the one that made it; one not here yet is refused when it arrives. Says how many
    /// were new.
    pub async fn take_forgotten(&self, list: Vec<Forgotten>) -> Result<usize, RecordError> {
        self.with(move |book, folder, _| {
            let mut taken = 0;
            let mut ids = Vec::new();
            for forgotten in &list {
                let Ok(number) = Uuid::parse_str(&forgotten.id) else {
                    continue;
                };
                if number.hyphenated().to_string() != forgotten.id {
                    continue;
                }
                let (year, here) = match held(book, folder, &forgotten.id)? {
                    Some((_, device)) if device != forgotten.device => continue,
                    Some((year, _)) => (year, true),
                    None => {
                        if tombstoned(book, folder, &forgotten.id)?.is_some() {
                            continue;
                        }
                        let Some((seconds, _)) =
                            number.get_timestamp().map(|stamp| stamp.to_unix())
                        else {
                            continue;
                        };
                        (
                            year_of(UNIX_EPOCH + std::time::Duration::from_secs(seconds)),
                            false,
                        )
                    }
                };
                if forget_in(book.year(folder, year)?, forgotten, here)? || here {
                    taken += 1;
                }
                ids.push(forgotten.id.clone());
            }
            for (year, id, device) in carrying(book, folder, &ids)? {
                if forget_in(book.year(folder, year)?, &Forgotten { id, device }, true)? {
                    taken += 1;
                }
            }
            if taken > 0 {
                settle(book, folder)?;
            }
            Ok(taken)
        })
        .await
    }

    /// What the records that carry one of `ids` hold, such as the search result that found them.
    /// These go when `ids` are forgotten.
    pub async fn carrying(&self, ids: Vec<String>) -> Result<Vec<Described>, RecordError> {
        self.with(move |book, folder, _| {
            let mut found = Vec::new();
            for (year, id, _) in carrying(book, folder, &ids)? {
                found.push(
                    book.year(folder, year)?
                        .query_row(
                            "SELECT at_ms, kind, sender_name, text FROM records WHERE id = ?1",
                            [&id],
                            |row| {
                                Ok(Described {
                                    id: id.clone(),
                                    at_ms: row.get(0)?,
                                    kind: row.get(1)?,
                                    sender: row.get(2)?,
                                    text: row.get(3)?,
                                })
                            },
                        )
                        .map_err(fail)?,
                );
            }
            Ok(found)
        })
        .await
    }

    /// What each record numbered `ids` holds, as the owner would recognize it: when it was made,
    /// who spoke and what was said, in the order asked. Fails on a number not here.
    pub async fn described(&self, ids: Vec<String>) -> Result<Vec<Described>, RecordError> {
        self.with(move |book, folder, _| {
            let mut found = Vec::with_capacity(ids.len());
            'ids: for id in &ids {
                for year in years(folder)? {
                    let row = book
                        .year(folder, year)?
                        .query_row(
                            "SELECT at_ms, kind, sender_name, text FROM records WHERE id = ?1",
                            [id],
                            |row| {
                                Ok(Described {
                                    id: id.clone(),
                                    at_ms: row.get(0)?,
                                    kind: row.get(1)?,
                                    sender: row.get(2)?,
                                    text: row.get(3)?,
                                })
                            },
                        )
                        .optional()
                        .map_err(fail)?;
                    if let Some(row) = row {
                        found.push(row);
                        continue 'ids;
                    }
                }
                return Err(fail(format!("no record {id}")));
            }
            Ok(found)
        })
        .await
    }

    /// Every record forgotten here.
    pub async fn forgotten(&self) -> Result<Vec<Forgotten>, RecordError> {
        self.with(|book, folder, _| {
            let mut all = Vec::new();
            for year in years(folder)? {
                let connection = book.year(folder, year)?;
                let mut query = connection
                    .prepare("SELECT id, device FROM forgotten ORDER BY n")
                    .map_err(fail)?;
                let rows = query
                    .query_map([], |row| {
                        Ok(Forgotten {
                            id: row.get(0)?,
                            device: row.get(1)?,
                        })
                    })
                    .map_err(fail)?;
                for row in rows {
                    all.push(row.map_err(fail)?);
                }
            }
            Ok(all)
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
                .query_row(
                    "SELECT max(id) FROM \
                     (SELECT id FROM records UNION ALL SELECT id FROM forgotten WHERE held = 1)",
                    [],
                    |row| row.get(0),
                )
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
    connection
        .pragma_update(None, "secure_delete", "ON")
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
    if version != 0 && version < BEFORE_ARRIVALS {
        return Err(fail(format!(
            "{} is in an older format ({version})",
            path.display()
        )));
    }
    let upgrade = connection.unchecked_transaction().map_err(fail)?;
    upgrade.execute_batch(SCHEMA).map_err(fail)?;
    if version == BEFORE_ARRIVALS {
        upgrade
            .execute(
                "INSERT OR IGNORE INTO arrived (id) SELECT id FROM records ORDER BY id",
                [],
            )
            .map_err(fail)?;
    }
    upgrade
        .pragma_update(None, "user_version", VERSION)
        .map_err(fail)?;
    upgrade.commit().map_err(fail)?;
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
        Box::pin(async move {
            Ok(self
                .load_marked()
                .await?
                .into_iter()
                .map(|marked| marked.record)
                .collect())
        })
    }

    fn load_marked(&self) -> BoxFuture<'_, Result<Vec<Marked>, RecordError>> {
        self.with(|book, folder, own| {
            let mut found = Vec::new();
            for year in years(folder)? {
                let connection = book.year(folder, year)?;
                let mut query = connection
                    .prepare("SELECT id, device, line FROM records")
                    .map_err(fail)?;
                let rows = query
                    .query_map([], |row| {
                        Ok((
                            row.get::<_, String>(0)?,
                            row.get::<_, String>(1)?,
                            row.get::<_, String>(2)?,
                        ))
                    })
                    .map_err(fail)?;
                for row in rows {
                    let (id, device, line) = row.map_err(fail)?;
                    let number = Uuid::parse_str(&id)
                        .map_err(|error| fail(format!("record {id}: {error}")))?;
                    let record =
                        parse(&line).map_err(|detail| fail(format!("record {id}: {detail}")))?;
                    found.push((
                        number,
                        Marked {
                            record,
                            here: device == own,
                        },
                    ));
                }
            }
            found.sort_by_key(|(number, _)| *number);
            if let Some((last, _)) = found.last() {
                book.last = book.last.max(Some(*last));
            }
            Ok(found.into_iter().map(|(_, marked)| marked).collect())
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
            let ids: Vec<Uuid> = lines
                .iter()
                .map(|line| book.next_id(line.at_ms()))
                .collect();
            let connection = book.year(folder, year)?;
            let batch = connection.transaction().map_err(fail)?;
            for ((record, line), id) in records.iter().zip(&lines).zip(&ids) {
                let text = serde_json::to_string(line).map_err(fail)?;
                insert_and_note_arrival(
                    &batch,
                    &id.to_string(),
                    device,
                    line.at_ms(),
                    &record.entry,
                    &text,
                )?;
            }
            batch.commit().map_err(fail)
        })
    }
}

fn held(book: &mut Book, folder: &Path, id: &str) -> Result<Option<(i64, String)>, RecordError> {
    for year in years(folder)? {
        let device: Option<String> = book
            .year(folder, year)?
            .query_row("SELECT device FROM records WHERE id = ?1", [id], |row| {
                row.get(0)
            })
            .optional()
            .map_err(fail)?;
        if let Some(device) = device {
            return Ok(Some((year, device)));
        }
    }
    Ok(None)
}

fn tombstoned(book: &mut Book, folder: &Path, id: &str) -> Result<Option<i64>, RecordError> {
    for year in years(folder)? {
        let here: bool = book
            .year(folder, year)?
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM forgotten WHERE id = ?1)",
                [id],
                |row| row.get(0),
            )
            .map_err(fail)?;
        if here {
            return Ok(Some(year));
        }
    }
    Ok(None)
}

fn carrying(
    book: &mut Book,
    folder: &Path,
    ids: &[String],
) -> Result<Vec<(i64, String, String)>, RecordError> {
    let mut found = Vec::new();
    for year in years(folder)? {
        let connection = book.year(folder, year)?;
        for id in ids {
            let mut query = connection
                .prepare(
                    "SELECT id, device FROM records WHERE id != ?1 \
                     AND kind IN ('tool_returned', 'sub_task_returned') AND instr(line, ?1) > 0",
                )
                .map_err(fail)?;
            let rows = query
                .query_map([id], |row| {
                    Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
                })
                .map_err(fail)?;
            for row in rows {
                let (carrier, device) = row.map_err(fail)?;
                if !ids.contains(&carrier) && !found.iter().any(|(_, known, _)| *known == carrier) {
                    found.push((year, carrier, device));
                }
            }
        }
    }
    Ok(found)
}

fn summaries_after(
    book: &mut Book,
    folder: &Path,
    year: i64,
    id: &str,
) -> Result<Vec<(i64, String, String)>, RecordError> {
    let mut found = Vec::new();
    for later in years(folder)?.into_iter().filter(|later| *later >= year) {
        let mut query = book
            .year(folder, later)?
            .prepare("SELECT id, device FROM records WHERE kind = 'summarized' AND id > ?1")
            .map_err(fail)?;
        let rows = query
            .query_map([id], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
            })
            .map_err(fail)?;
        for row in rows {
            let (summary, device) = row.map_err(fail)?;
            found.push((later, summary, device));
        }
    }
    Ok(found)
}

fn forget_in(
    connection: &mut Connection,
    forgotten: &Forgotten,
    held: bool,
) -> Result<bool, RecordError> {
    let batch = connection.transaction().map_err(fail)?;
    let new = batch
        .execute(
            "INSERT OR IGNORE INTO forgotten (id, device, held) VALUES (?1, ?2, ?3)",
            params![forgotten.id, forgotten.device, held],
        )
        .map_err(fail)?;
    if held {
        batch
            .execute(
                "UPDATE forgotten SET held = 1 WHERE id = ?1",
                [&forgotten.id],
            )
            .map_err(fail)?;
    }
    batch
        .execute("DELETE FROM records WHERE id = ?1", [&forgotten.id])
        .map_err(fail)?;
    batch
        .execute("DELETE FROM arrived WHERE id = ?1", [&forgotten.id])
        .map_err(fail)?;
    batch.commit().map_err(fail)?;
    Ok(new > 0)
}

fn settle(book: &mut Book, folder: &Path) -> Result<(), RecordError> {
    for year in years(folder)? {
        let connection = book.year(folder, year)?;
        let mut settled = false;
        for _ in 0..SETTLE_TRIES {
            let busy: i64 = connection
                .query_row("PRAGMA wal_checkpoint(TRUNCATE)", [], |row| row.get(0))
                .map_err(fail)?;
            if busy == 0 {
                settled = true;
                break;
            }
            std::thread::sleep(SETTLE_WAIT);
        }
        if !settled {
            eprintln!(
                "dotori: the {year} record is busy, so what was forgotten leaves its log only later"
            );
        }
    }
    Ok(())
}

fn insert_and_note_arrival(
    batch: &rusqlite::Transaction<'_>,
    id: &str,
    device: &str,
    at_ms: u64,
    entry: &Entry,
    line: &str,
) -> Result<bool, RecordError> {
    let columns = Columns::of(entry);
    let added = batch
        .execute(
            "INSERT OR IGNORE INTO records \
             (id, device, at_ms, kind, channel, place, sender_id, sender_name, role, text, line) \
             SELECT ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11 \
             WHERE NOT EXISTS (SELECT 1 FROM forgotten WHERE id = ?1 AND device = ?2)",
            params![
                id,
                device,
                i64::try_from(at_ms).unwrap_or(i64::MAX),
                columns.kind,
                columns.channel,
                columns.place,
                columns.sender_id,
                columns.sender_name,
                columns.role,
                columns.text,
                line
            ],
        )
        .map_err(fail)?;
    if added > 0 {
        batch
            .execute("INSERT INTO arrived (id) VALUES (?1)", [id])
            .map_err(fail)?;
    }
    Ok(added > 0)
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
            Entry::Summarized { .. } => Self {
                kind: "summarized",
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
