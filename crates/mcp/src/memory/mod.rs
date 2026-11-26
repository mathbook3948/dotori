//! The memory search tool: keyword search over the raw record, joined with search by meaning when
//! an embedding API is set, from an index that can always be made again from the record.

mod analyzer;
mod dictionary;
mod embedding;
mod eval;
mod vectors;

use std::path::Path;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::Weak;
use std::sync::atomic::AtomicBool;
use std::sync::atomic::Ordering;
use std::time::Duration;

use rusqlite::Connection;
use rusqlite::OpenFlags;
use rusqlite::OptionalExtension;
use rusqlite::TransactionBehavior;
use rusqlite::params;
use serde_json::Value;
use serde_json::json;

use crate::Tool;
use crate::ToolFuture;
use analyzer::Analyzer;
use embedding::Embedder;
pub use embedding::EmbeddingConfig;
pub use eval::Case;
pub use eval::HITS;
pub use eval::Method;
pub use eval::Report;
pub use eval::Score;
use vectors::Vectors;

const BATCH: i64 = 1000;
const DEFAULT_LIMIT: u64 = 10;
const MAX_LIMIT: u64 = 50;
const SHOWN: usize = 400;
const CANDIDATES: u64 = 50;
const EMBED_BATCH: i64 = 64;
const NO_WORDS: &str = "the query has no words to search for";
const QUERY_WAIT: Duration = Duration::from_secs(15);
const BATCH_PAUSE: Duration = Duration::from_secs(1);
const FILL_PAUSE: Duration = Duration::from_secs(30);
const FAILED_PAUSE: Duration = Duration::from_secs(300);
const VERSION: i64 = 3;
const OLD_FORMAT: &str = "2";
const BUSY: Duration = Duration::from_secs(5);
const SCHEMA: &str = "
CREATE TABLE IF NOT EXISTS meta (key TEXT PRIMARY KEY, value TEXT NOT NULL);
CREATE TABLE IF NOT EXISTS docs (
    seq INTEGER PRIMARY KEY,
    record TEXT NOT NULL UNIQUE,
    at_ms INTEGER NOT NULL,
    kind TEXT NOT NULL,
    channel TEXT,
    place TEXT,
    sender_name TEXT,
    role TEXT,
    text TEXT
);
CREATE VIRTUAL TABLE IF NOT EXISTS terms USING fts5(words, tokenize = 'unicode61');
CREATE TABLE IF NOT EXISTS vectors (seq INTEGER PRIMARY KEY, model TEXT NOT NULL, vector BLOB NOT NULL);
CREATE TABLE IF NOT EXISTS vector_log (n INTEGER PRIMARY KEY AUTOINCREMENT, seq INTEGER NOT NULL);
CREATE TABLE IF NOT EXISTS copied_vectors (
    record TEXT NOT NULL,
    model TEXT NOT NULL,
    vector BLOB NOT NULL,
    PRIMARY KEY (record, model)
);
CREATE TABLE IF NOT EXISTS dropped (record TEXT PRIMARY KEY) WITHOUT ROWID;
";
const DROP: &str = "
DROP TABLE IF EXISTS meta;
DROP TABLE IF EXISTS docs;
DROP TABLE IF EXISTS terms;
DROP TABLE IF EXISTS vectors;
DROP TABLE IF EXISTS vector_log;
DROP TABLE IF EXISTS copied_vectors;
DROP TABLE IF EXISTS forgotten;
DROP TABLE IF EXISTS dropped;
";

/// Where memory reads the raw record, keeps its index, and keeps dictionaries. The record is a
/// folder with one SQLite file per year.
#[derive(Clone, Debug)]
pub struct MemoryConfig {
    pub record: PathBuf,
    pub index: PathBuf,
    pub dictionaries: PathBuf,
    pub embedding: Option<EmbeddingConfig>,
    /// A file that says whether this PC makes vectors, when the owner's PCs share that job.
    /// Without it, this PC makes them.
    pub jobs: Option<PathBuf>,
}

/// Memory shared by its tools. The index catches up with the raw record before every search.
pub struct Memory {
    config: MemoryConfig,
    embedder: Option<Arc<Embedder>>,
    meaning_ready: AtomicBool,
    state: Mutex<State>,
}

#[derive(Default)]
struct State {
    index: Option<Connection>,
    analyzer: Analyzer,
    dictionary_failed: bool,
    vectors: Vectors,
}

impl std::fmt::Debug for Memory {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Memory")
            .field("config", &self.config)
            .finish_non_exhaustive()
    }
}

impl Memory {
    /// Starts fetching the Korean dictionary in the background. Until it arrives, words are
    /// split by a simpler rule. With an embedding API, records get vectors in the background too.
    pub fn new(config: MemoryConfig) -> Arc<Self> {
        let embedder =
            config
                .embedding
                .clone()
                .and_then(|embedding| match Embedder::new(embedding) {
                    Ok(embedder) => Some(Arc::new(embedder)),
                    Err(error) => {
                        eprintln!("dotori: memory search matches words only: {error}");
                        None
                    }
                });
        let memory = Arc::new(Self {
            config,
            embedder,
            meaning_ready: AtomicBool::new(false),
            state: Mutex::new(State::default()),
        });
        let folder = memory.config.dictionaries.clone();
        if tokio::runtime::Handle::try_current().is_ok() {
            tokio::spawn(async move {
                if let Err(error) = dictionary::fetch(&folder).await {
                    eprintln!("dotori: memory search uses a simpler word split: {error}");
                }
            });
            if memory.embedder.is_some() {
                tokio::spawn(fill(Arc::downgrade(&memory)));
            }
        }
        memory
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, State> {
        self.state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    fn opened(&self, state: &mut State) -> Result<(), String> {
        if state.index.is_none() {
            state.index = Some(open_index(&self.config.index)?);
        }
        Ok(())
    }

    async fn fill_once(self: &Arc<Self>) -> Result<bool, String> {
        let Some(embedder) = self.embedder.clone() else {
            return Ok(false);
        };
        let model = embedder.name();
        let memory = Arc::clone(self);
        let wanted = model.clone();
        let (texts, last) = tokio::task::spawn_blocking(move || {
            let mut state = memory.lock();
            memory.opened(&mut state)?;
            let index = state.index.as_mut().ok_or("no index")?;
            if vectors::any(index, &wanted).map_err(|error| error.to_string())? {
                memory.meaning_ready.store(true, Ordering::SeqCst);
            }
            drop_forgotten(index, &memory.config.record).map_err(|error| error.to_string())?;
            copy_records(index, &memory.config.record).map_err(|error| error.to_string())?;
            if vectors::merge_copied(index).map_err(|error| error.to_string())? {
                memory.meaning_ready.store(true, Ordering::SeqCst);
            }
            if !makes_vectors(memory.config.jobs.as_deref()) {
                return Ok((Vec::new(), None));
            }
            vectors::pending(index, &wanted, EMBED_BATCH).map_err(|error| error.to_string())
        })
        .await
        .map_err(|error| error.to_string())??;
        let Some(last) = last else {
            return Ok(false);
        };
        let made = made(&embedder, texts).await?;
        let memory = Arc::clone(self);
        tokio::task::spawn_blocking(move || {
            let mut state = memory.lock();
            memory.opened(&mut state)?;
            let index = state.index.as_mut().ok_or("no index")?;
            vectors::store(index, &model, &made, last)?;
            if !made.is_empty() {
                memory.meaning_ready.store(true, Ordering::SeqCst);
            }
            Ok::<(), String>(())
        })
        .await
        .map_err(|error| error.to_string())??;
        Ok(true)
    }

    /// Removes from the search index and its vectors every record the raw record has forgotten
    /// since the last time, without waiting for the next search.
    pub fn drop_forgotten(config: &MemoryConfig) -> Result<(), String> {
        let mut index = open_index(&config.index)?;
        drop_forgotten(&mut index, &config.record).map_err(|error| error.to_string())
    }

    /// The tools that use this memory.
    pub fn tools(self: &Arc<Self>) -> Vec<Box<dyn Tool>> {
        vec![Box::new(MemorySearch {
            memory: Arc::clone(self),
        })]
    }

    fn ranked(
        &self,
        state: &mut State,
        query: &str,
        limit: u64,
        meaning: Option<Result<Vec<f32>, String>>,
    ) -> Result<Option<Ranked>, String> {
        if matches!(state.analyzer, Analyzer::Plain)
            && !state.dictionary_failed
            && dictionary::ready(&self.config.dictionaries)
        {
            match Analyzer::korean(&dictionary::path(&self.config.dictionaries)) {
                Ok(korean) => state.analyzer = korean,
                Err(error) => {
                    state.dictionary_failed = true;
                    eprintln!("dotori: memory search keeps a simpler word split: {error}");
                }
            }
        }
        self.opened(state)?;
        let State {
            index,
            analyzer,
            vectors,
            ..
        } = state;
        let index = index.as_mut().ok_or("no index")?;
        catch_up(index, &self.config.record, analyzer)
            .map_err(|error| format!("the search index could not catch up: {error}"))?;
        let indexed: i64 = index
            .query_row("SELECT count(*) FROM docs", [], |row| row.get(0))
            .map_err(|error| error.to_string())?;
        if indexed == 0 {
            return Ok(None);
        }
        let terms = analyzer.terms(query);
        let mut note = None;
        let by_meaning = match (meaning, &self.embedder) {
            (Some(Ok(wanted)), Some(embedder)) => {
                match vectors
                    .load(index, &embedder.name())
                    .map_err(|error| error.to_string())
                    .and_then(|()| vectors.nearest(&wanted, CANDIDATES as usize))
                {
                    Ok(found) => Some(found),
                    Err(error) => {
                        note = Some(format!(
                            "(Only words were matched: search by meaning failed. {error})"
                        ));
                        None
                    }
                }
            }
            (Some(Err(error)), _) => {
                note = Some(format!(
                    "(Only words were matched: search by meaning failed. {error})"
                ));
                None
            }
            _ => None,
        };
        if terms.is_empty() && by_meaning.is_none() {
            return Err(String::from(NO_WORDS));
        }
        let by_words: Vec<i64> = if terms.is_empty() {
            Vec::new()
        } else {
            let matched = terms
                .iter()
                .map(|term| format!("\"{}\"", term.replace('"', "\"\"")))
                .collect::<Vec<_>>()
                .join(" OR ");
            let wanted = if by_meaning.is_some() {
                CANDIDATES
            } else {
                limit
            };
            index
                .prepare("SELECT rowid FROM terms WHERE words MATCH ?1 ORDER BY rank LIMIT ?2")
                .and_then(|mut found| {
                    found
                        .query_map(
                            params![matched, i64::try_from(wanted).unwrap_or(MAX_LIMIT as i64)],
                            |row| row.get(0),
                        )?
                        .collect()
                })
                .map_err(|error| error.to_string())?
        };
        Ok(Some(Ranked {
            by_words,
            by_meaning,
            note,
        }))
    }

    fn search(
        &self,
        query: &str,
        limit: u64,
        meaning: Option<Result<Vec<f32>, String>>,
    ) -> Result<String, String> {
        let mut state = self.lock();
        let Some(Ranked {
            by_words,
            by_meaning,
            note,
        }) = self.ranked(&mut state, query, limit, meaning)?
        else {
            return Ok(String::from("Nothing is recorded yet."));
        };
        let index = state.index.as_mut().ok_or("no index")?;
        let count = usize::try_from(limit).unwrap_or(MAX_LIMIT as usize);
        let seqs = match by_meaning {
            Some(by_meaning) => vectors::fuse(&[by_words, by_meaning], count),
            None => by_words,
        };
        if seqs.is_empty() {
            let mut nothing = String::from("Nothing in memory matches.");
            if let Some(note) = note {
                nothing = format!("{nothing}\n{note}");
            }
            return Ok(nothing);
        }
        let mut shown = Vec::new();
        for seq in seqs {
            let row = index
                .query_row(
                    "SELECT record, at_ms, kind, channel, place, sender_name, role, text FROM docs WHERE seq = ?1",
                    [seq],
                    |row| {
                        Ok(Hit {
                            record: row.get(0)?,
                            at_ms: row.get(1)?,
                            kind: row.get(2)?,
                            channel: row.get(3)?,
                            place: row.get(4)?,
                            sender: row.get(5)?,
                            role: row.get(6)?,
                            text: row.get(7)?,
                        })
                    },
                )
                .optional()
                .map_err(|error| error.to_string())?;
            if let Some(hit) = row {
                shown.push(hit.line());
            }
        }
        shown.extend(note);
        Ok(shown.join("\n"))
    }
}

async fn made(
    embedder: &Embedder,
    texts: Vec<(i64, String)>,
) -> Result<Vec<(i64, Vec<f32>)>, String> {
    if texts.is_empty() {
        return Ok(Vec::new());
    }
    let wording: Vec<String> = texts.iter().map(|(_, text)| text.clone()).collect();
    match embedder.embed(&wording).await {
        Ok(vectors) => Ok(texts.iter().map(|(seq, _)| *seq).zip(vectors).collect()),
        Err(refused) if refused.by_text => {
            let mut made = Vec::new();
            for (seq, text) in texts {
                match embedder.embed(std::slice::from_ref(&text)).await {
                    Ok(mut vectors) => made.extend(vectors.pop().map(|vector| (seq, vector))),
                    Err(refused) if refused.by_text => {
                        eprintln!("dotori: record {seq} gets no vector: {}", refused.detail);
                    }
                    Err(refused) => return Err(refused.detail),
                }
            }
            Ok(made)
        }
        Err(refused) => Err(refused.detail),
    }
}

async fn fill(memory: Weak<Memory>) {
    let mut reported = None;
    loop {
        let Some(strong) = memory.upgrade() else {
            return;
        };
        let pause = match strong.fill_once().await {
            Ok(true) => BATCH_PAUSE,
            Ok(false) => FILL_PAUSE,
            Err(error) => {
                if reported.as_ref() != Some(&error) {
                    eprintln!("dotori: records wait for their vectors: {error}");
                    reported = Some(error);
                }
                FAILED_PAUSE
            }
        };
        drop(strong);
        tokio::time::sleep(pause).await;
    }
}

struct Ranked {
    by_words: Vec<i64>,
    by_meaning: Option<Vec<i64>>,
    note: Option<String>,
}

struct Hit {
    record: String,
    at_ms: i64,
    kind: String,
    channel: Option<String>,
    place: Option<String>,
    sender: Option<String>,
    role: Option<String>,
    text: Option<String>,
}

impl Hit {
    fn line(&self) -> String {
        let who = match (&self.sender, &self.role) {
            (Some(sender), Some(role)) => format!("{sender} ({role})"),
            _ if self.kind == "acted" => String::from("dotori"),
            _ => self.kind.clone(),
        };
        let place = match (&self.channel, &self.place) {
            (Some(channel), Some(place)) => format!(" in {channel} {place}"),
            (Some(channel), None) => format!(" in {channel}"),
            _ => String::new(),
        };
        let text = self.text.as_deref().unwrap_or_default().replace('\n', " ");
        let mut cut: String = text.chars().take(SHOWN).collect();
        if cut.len() < text.len() {
            cut.push('…');
        }
        format!(
            "[record {}] {}{place}, {who}: {cut}",
            self.record,
            when(self.at_ms)
        )
    }
}

/// Opens the search index at `path`, made again from nothing when it is in another format. Every
/// table of the index is made here, also for the vectors other PCs send.
pub fn open_index(path: &Path) -> Result<Connection, String> {
    if let Some(folder) = path.parent() {
        std::fs::create_dir_all(folder).map_err(|error| error.to_string())?;
    }
    let index = Connection::open(path).map_err(|error| error.to_string())?;
    let fail = |error: rusqlite::Error| error.to_string();
    index.busy_timeout(BUSY).map_err(fail)?;
    index
        .pragma_update(None, "secure_delete", "ON")
        .map_err(fail)?;
    if current(&index).map_err(fail)? {
        index.execute_batch(SCHEMA).map_err(fail)?;
    } else {
        let upgrade = rusqlite::Transaction::new_unchecked(&index, TransactionBehavior::Immediate)
            .map_err(fail)?;
        if !current(&upgrade).map_err(fail)? {
            match old_format(&upgrade).map_err(fail)?.as_deref() {
                Some(OLD_FORMAT) => from_old_format(&upgrade).map_err(fail)?,
                _ => upgrade.execute_batch(DROP).map_err(fail)?,
            }
            upgrade.execute_batch(SCHEMA).map_err(fail)?;
            upgrade
                .pragma_update(None, "user_version", VERSION)
                .map_err(fail)?;
        }
        upgrade.commit().map_err(fail)?;
    }
    index
        .execute(
            "INSERT INTO terms (terms, rank) VALUES ('secure-delete', 1)",
            [],
        )
        .map_err(fail)?;
    index
        .execute(
            "INSERT INTO vector_log (seq) SELECT seq FROM vectors \
             WHERE NOT EXISTS (SELECT 1 FROM vector_log) ORDER BY seq",
            [],
        )
        .map_err(fail)?;
    let made = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |since| since.as_nanos());
    index
        .execute(
            "INSERT OR IGNORE INTO meta (key, value) VALUES ('index_id', ?1)",
            [format!("{made}-{}", std::process::id())],
        )
        .map_err(fail)?;
    Ok(index)
}

fn makes_vectors(jobs: Option<&Path>) -> bool {
    let Some(jobs) = jobs else {
        return true;
    };
    match std::fs::read_to_string(jobs) {
        Ok(text) => serde_json::from_str::<Value>(&text)
            .ok()
            .and_then(|jobs| jobs.get("vectors").and_then(Value::as_bool))
            .unwrap_or(true),
        Err(_) => true,
    }
}

fn record_years(folder: &Path) -> rusqlite::Result<Vec<(String, Connection)>> {
    let Ok(entries) = std::fs::read_dir(folder) else {
        return Ok(Vec::new());
    };
    let mut years: Vec<PathBuf> = entries
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .and_then(|name| name.strip_suffix(".sqlite3"))
                .is_some_and(|stem| stem.len() == 4 && stem.bytes().all(|b| b.is_ascii_digit()))
        })
        .collect();
    years.sort();
    years
        .iter()
        .map(|path| {
            let year = path
                .file_stem()
                .and_then(|stem| stem.to_str())
                .unwrap_or_default()
                .to_owned();
            Ok((
                year,
                Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY)?,
            ))
        })
        .collect()
}

fn has_table(index: &Connection, name: &str) -> rusqlite::Result<bool> {
    index
        .query_row(
            "SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = ?1",
            [name],
            |_| Ok(()),
        )
        .optional()
        .map(|found| found.is_some())
}

fn current(index: &Connection) -> rusqlite::Result<bool> {
    let version: i64 = index.pragma_query_value(None, "user_version", |row| row.get(0))?;
    Ok(version == VERSION && old_format(index)?.is_none())
}

fn old_format(index: &Connection) -> rusqlite::Result<Option<String>> {
    if !has_table(index, "meta")? {
        return Ok(None);
    }
    meta(index, "format")
}

fn from_old_format(index: &Connection) -> rusqlite::Result<()> {
    match (has_table(index, "forgotten")?, has_table(index, "dropped")?) {
        (true, true) => index.execute_batch(
            "INSERT OR IGNORE INTO dropped (record) SELECT record FROM forgotten; \
             DROP TABLE forgotten;",
        )?,
        (true, false) => index.execute_batch("ALTER TABLE forgotten RENAME TO dropped;")?,
        (false, _) => {}
    }
    index.execute_batch(
        "ALTER TABLE docs RENAME COLUMN sender TO sender_name; \
         DELETE FROM meta WHERE key = 'format';",
    )
}

fn meta(index: &Connection, key: &str) -> rusqlite::Result<Option<String>> {
    index
        .query_row("SELECT value FROM meta WHERE key = ?1", [key], |row| {
            row.get(0)
        })
        .optional()
}

fn copy_records(index: &mut Connection, folder: &Path) -> rusqlite::Result<()> {
    for (name, year) in record_years(folder)? {
        let key = format!("arrived:{name}");
        loop {
            let batch = index.transaction_with_behavior(TransactionBehavior::Immediate)?;
            let arrived: i64 = meta(&batch, &key)?
                .and_then(|seq| seq.parse().ok())
                .unwrap_or(0);
            let rows: Vec<(i64, Doc)> = match year.prepare(
                "SELECT a.seq, r.id, r.at_ms, r.kind, r.channel, r.place, r.sender_name, r.role, r.text \
                 FROM arrived a JOIN records r ON r.id = a.id \
                 WHERE a.seq > ?1 ORDER BY a.seq LIMIT ?2",
            ) {
                Ok(mut found) => found
                    .query_map(params![arrived, BATCH], |row| {
                        Ok((
                            row.get(0)?,
                            Doc {
                                record: row.get(1)?,
                                at_ms: row.get(2)?,
                                kind: row.get(3)?,
                                channel: row.get(4)?,
                                place: row.get(5)?,
                                sender: row.get(6)?,
                                role: row.get(7)?,
                                text: row.get(8)?,
                            },
                        ))
                    })?
                    .collect::<rusqlite::Result<_>>()?,
                Err(error) if error.to_string().contains("no such table") => Vec::new(),
                Err(error) => return Err(error),
            };
            let Some(last) = rows.last().map(|(seq, _)| seq.to_string()) else {
                break;
            };
            let rows: Vec<Doc> = rows.into_iter().map(|(_, doc)| doc).collect();
            for doc in &rows {
                batch.execute(
                    "INSERT OR IGNORE INTO docs (record, at_ms, kind, channel, place, sender_name, role, text) \
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
                    params![
                        doc.record,
                        doc.at_ms,
                        doc.kind,
                        doc.channel,
                        doc.place,
                        doc.sender,
                        doc.role,
                        doc.text
                    ],
                )?;
            }
            batch.execute(
                "INSERT OR REPLACE INTO meta (key, value) VALUES (?1, ?2)",
                [&key, &last],
            )?;
            batch.commit()?;
        }
    }
    Ok(())
}

fn drop_forgotten(index: &mut Connection, folder: &Path) -> rusqlite::Result<()> {
    let mut dropped_vectors = false;
    let mut dropped = false;
    for (name, year) in record_years(folder)? {
        let key = format!("forgotten:{name}");
        let seen: i64 = meta(index, &key)?
            .and_then(|seen| seen.parse().ok())
            .unwrap_or(0);
        let rows: Vec<(i64, String)> =
            match year.prepare("SELECT n, id FROM forgotten WHERE n > ?1 ORDER BY n") {
                Ok(mut found) => found
                    .query_map([seen], |row| Ok((row.get(0)?, row.get(1)?)))?
                    .collect::<rusqlite::Result<_>>()?,
                Err(error) if error.to_string().contains("no such table") => Vec::new(),
                Err(error) => return Err(error),
            };
        let Some(last) = rows.last().map(|(n, _)| n.to_string()) else {
            continue;
        };
        let batch = index.transaction_with_behavior(TransactionBehavior::Immediate)?;
        for (_, record) in &rows {
            batch.execute(
                "INSERT OR IGNORE INTO dropped (record) VALUES (?1)",
                [record],
            )?;
            let seq: Option<i64> = batch
                .query_row("SELECT seq FROM docs WHERE record = ?1", [record], |row| {
                    row.get(0)
                })
                .optional()?;
            if let Some(seq) = seq {
                dropped = true;
                batch.execute("DELETE FROM terms WHERE rowid = ?1", [seq])?;
                dropped_vectors |= batch.execute("DELETE FROM vectors WHERE seq = ?1", [seq])? > 0;
                batch.execute("DELETE FROM vector_log WHERE seq = ?1", [seq])?;
                batch.execute("DELETE FROM docs WHERE seq = ?1", [seq])?;
            }
        }
        batch.execute(
            "INSERT OR REPLACE INTO meta (key, value) VALUES (?1, ?2)",
            [&key, &last],
        )?;
        batch.commit()?;
    }
    index.execute(
        "DELETE FROM copied_vectors WHERE record IN (SELECT record FROM dropped)",
        [],
    )?;
    if dropped {
        index.query_row("PRAGMA wal_checkpoint(TRUNCATE)", [], |_| Ok(()))?;
    }
    if dropped_vectors {
        let merged: i64 = meta(index, "vectors_merged")?
            .and_then(|merged| merged.parse().ok())
            .unwrap_or(0);
        index.execute(
            "INSERT OR REPLACE INTO meta (key, value) VALUES ('vectors_merged', ?1)",
            [(merged + 1).to_string()],
        )?;
    }
    Ok(())
}

struct Doc {
    record: String,
    at_ms: i64,
    kind: String,
    channel: Option<String>,
    place: Option<String>,
    sender: Option<String>,
    role: Option<String>,
    text: Option<String>,
}

fn catch_up(index: &mut Connection, folder: &Path, analyzer: &Analyzer) -> rusqlite::Result<()> {
    drop_forgotten(index, folder)?;
    copy_records(index, folder)?;
    vectors::merge_copied(index)?;
    if meta(index, "analyzer")?.as_deref() != Some(analyzer.name()) {
        let rebuild = index.transaction_with_behavior(TransactionBehavior::Immediate)?;
        rebuild.execute("DELETE FROM terms", [])?;
        rebuild.execute(
            "INSERT OR REPLACE INTO meta (key, value) VALUES ('analyzer', ?1), ('done', '0')",
            [analyzer.name()],
        )?;
        rebuild.commit()?;
    }
    loop {
        let batch = index.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let done: i64 = meta(&batch, "done")?
            .and_then(|done| done.parse().ok())
            .unwrap_or(0);
        let rows: Vec<(i64, Option<String>, Option<String>)> = batch
            .prepare(
                "SELECT seq, sender_name, text FROM docs WHERE seq > ?1 ORDER BY seq LIMIT ?2",
            )?
            .query_map(params![done, BATCH], |row| {
                Ok((row.get(0)?, row.get(1)?, row.get(2)?))
            })?
            .collect::<rusqlite::Result<_>>()?;
        let Some(last) = rows.last().map(|row| row.0) else {
            return Ok(());
        };
        for (seq, sender, text) in &rows {
            let Some(text) = text else {
                continue;
            };
            let mut terms = analyzer.terms(text);
            if let Some(sender) = sender {
                terms.extend(analyzer.terms(sender));
            }
            batch.execute(
                "INSERT OR REPLACE INTO terms (rowid, words) VALUES (?1, ?2)",
                params![seq, terms.join(" ")],
            )?;
        }
        batch.execute(
            "INSERT OR REPLACE INTO meta (key, value) VALUES ('done', ?1)",
            [last.to_string()],
        )?;
        batch.commit()?;
    }
}

fn when(at_ms: i64) -> String {
    let seconds = at_ms.div_euclid(1000);
    let days = seconds.div_euclid(86_400);
    let of_day = seconds.rem_euclid(86_400);
    let shifted = days + 719_468;
    let era = shifted.div_euclid(146_097);
    let day_of_era = shifted.rem_euclid(146_097);
    let year_of_era =
        (day_of_era - day_of_era / 1460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let month_index = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * month_index + 2) / 5 + 1;
    let month = if month_index < 10 {
        month_index + 3
    } else {
        month_index - 9
    };
    let year = year_of_era + era * 400 + i64::from(month <= 2);
    format!(
        "{year:04}-{month:02}-{day:02} {:02}:{:02} UTC",
        of_day / 3600,
        of_day % 3600 / 60
    )
}

/// `search_memory`: finds earlier messages and answers by their words.
#[derive(Debug)]
pub struct MemorySearch {
    memory: Arc<Memory>,
}

impl Tool for MemorySearch {
    fn name(&self) -> &str {
        "search_memory"
    }

    fn description(&self) -> &str {
        "Searches everything you have heard and said for these words, and for their meaning \
         when that is set up, best matches first. \
         Each hit names its record, time, place, and who said it. Hits can hold the owner's \
         private messages: pass them on only to the owner."
    }

    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "query": { "type": "string", "description": "Words to look for." },
                "limit": { "type": "integer", "minimum": 1, "maximum": MAX_LIMIT }
            },
            "required": ["query"]
        })
    }

    fn call(&self, arguments: Value) -> ToolFuture<'_> {
        let memory = Arc::clone(&self.memory);
        Box::pin(async move {
            let query = arguments
                .get("query")
                .and_then(Value::as_str)
                .ok_or("`query` is required")?
                .to_owned();
            let limit = match arguments.get("limit") {
                None => DEFAULT_LIMIT,
                Some(limit) => limit
                    .as_u64()
                    .filter(|limit| (1..=MAX_LIMIT).contains(limit))
                    .ok_or_else(|| format!("`limit` must be from 1 to {MAX_LIMIT}"))?,
            };
            let meaning = match &memory.embedder {
                Some(embedder) if memory.meaning_ready.load(Ordering::SeqCst) => Some(
                    match tokio::time::timeout(
                        QUERY_WAIT,
                        embedder.embed(&[query.chars().take(vectors::LONGEST).collect()]),
                    )
                    .await
                    {
                        Ok(Ok(mut vectors)) => {
                            vectors.pop().ok_or_else(|| String::from("no vector"))
                        }
                        Ok(Err(refused)) => Err(refused.detail),
                        Err(_) => Err(String::from("the embedding API took too long")),
                    },
                ),
                _ => None,
            };
            tokio::task::spawn_blocking(move || memory.search(&query, limit, meaning))
                .await
                .map_err(|error| error.to_string())?
        })
    }
}

#[cfg(test)]
#[path = "mod.test.rs"]
mod tests;
