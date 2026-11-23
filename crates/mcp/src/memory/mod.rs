//! The memory search tool: keyword search over the raw record, joined with search by meaning when
//! an embedding API is set, from an index that can always be made again from the record.

mod analyzer;
mod dictionary;
mod embedding;
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
use vectors::Vectors;

const BATCH: i64 = 1000;
const DEFAULT_LIMIT: u64 = 10;
const MAX_LIMIT: u64 = 50;
const SHOWN: usize = 400;
const CANDIDATES: u64 = 50;
const EMBED_BATCH: i64 = 64;
const QUERY_WAIT: Duration = Duration::from_secs(15);
const BATCH_PAUSE: Duration = Duration::from_secs(1);
const FILL_PAUSE: Duration = Duration::from_secs(30);
const FAILED_PAUSE: Duration = Duration::from_secs(300);
const FORMAT: &str = "2";
const SCHEMA: &str = "
CREATE TABLE IF NOT EXISTS meta (key TEXT PRIMARY KEY, value TEXT NOT NULL);
CREATE TABLE IF NOT EXISTS docs (
    seq INTEGER PRIMARY KEY,
    record TEXT NOT NULL UNIQUE,
    at_ms INTEGER NOT NULL,
    kind TEXT NOT NULL,
    channel TEXT,
    place TEXT,
    sender TEXT,
    role TEXT,
    text TEXT
);
CREATE VIRTUAL TABLE IF NOT EXISTS terms USING fts5(words, tokenize = 'unicode61');
CREATE TABLE IF NOT EXISTS vectors (seq INTEGER PRIMARY KEY, model TEXT NOT NULL, vector BLOB NOT NULL);
";
const DROP: &str = "
DROP TABLE IF EXISTS meta;
DROP TABLE IF EXISTS docs;
DROP TABLE IF EXISTS terms;
DROP TABLE IF EXISTS vectors;
";

/// Where memory reads the raw record, keeps its index, and keeps dictionaries. The record is a
/// folder with one SQLite file per year.
#[derive(Clone, Debug)]
pub struct MemoryConfig {
    pub record: PathBuf,
    pub index: PathBuf,
    pub dictionaries: PathBuf,
    pub embedding: Option<EmbeddingConfig>,
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
            copy_records(index, &memory.config.record).map_err(|error| error.to_string())?;
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

    /// The tools that use this memory.
    pub fn tools(self: &Arc<Self>) -> Vec<Box<dyn Tool>> {
        vec![Box::new(MemorySearch {
            memory: Arc::clone(self),
        })]
    }

    fn search(
        &self,
        query: &str,
        limit: u64,
        meaning: Option<Result<Vec<f32>, String>>,
    ) -> Result<String, String> {
        let mut state = self.lock();
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
        self.opened(&mut state)?;
        let State {
            index,
            analyzer,
            vectors,
            ..
        } = &mut *state;
        let index = index.as_mut().ok_or("no index")?;
        catch_up(index, &self.config.record, analyzer)
            .map_err(|error| format!("the search index could not catch up: {error}"))?;
        let indexed: i64 = index
            .query_row("SELECT count(*) FROM docs", [], |row| row.get(0))
            .map_err(|error| error.to_string())?;
        if indexed == 0 {
            return Ok(String::from("Nothing is recorded yet."));
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
            return Err(String::from("the query has no words to search for"));
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
                    "SELECT record, at_ms, kind, channel, place, sender, role, text FROM docs WHERE seq = ?1",
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

fn open_index(path: &PathBuf) -> Result<Connection, String> {
    if let Some(folder) = path.parent() {
        std::fs::create_dir_all(folder).map_err(|error| error.to_string())?;
    }
    let index = Connection::open(path).map_err(|error| error.to_string())?;
    let fail = |error: rusqlite::Error| error.to_string();
    index.execute_batch(SCHEMA).map_err(fail)?;
    if meta(&index, "format").map_err(fail)?.as_deref() != Some(FORMAT) {
        index.execute_batch(DROP).map_err(fail)?;
        index.execute_batch(SCHEMA).map_err(fail)?;
        index
            .execute(
                "INSERT INTO meta (key, value) VALUES ('format', ?1)",
                [FORMAT],
            )
            .map_err(fail)?;
    }
    Ok(index)
}

fn record_years(folder: &Path) -> rusqlite::Result<Vec<Connection>> {
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
        .map(|path| Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY))
        .collect()
}

fn meta(index: &Connection, key: &str) -> rusqlite::Result<Option<String>> {
    index
        .query_row("SELECT value FROM meta WHERE key = ?1", [key], |row| {
            row.get(0)
        })
        .optional()
}

fn copy_records(index: &mut Connection, folder: &Path) -> rusqlite::Result<()> {
    for year in record_years(folder)? {
        loop {
            let batch = index.transaction_with_behavior(TransactionBehavior::Immediate)?;
            let copied = meta(&batch, "copied")?.unwrap_or_default();
            let rows: Vec<Doc> = match year.prepare(
                "SELECT id, at_ms, kind, channel, place, sender_name, role, text FROM records \
                 WHERE id > ?1 ORDER BY id LIMIT ?2",
            ) {
                Ok(mut found) => found
                    .query_map(params![copied, BATCH], |row| {
                        Ok(Doc {
                            record: row.get(0)?,
                            at_ms: row.get(1)?,
                            kind: row.get(2)?,
                            channel: row.get(3)?,
                            place: row.get(4)?,
                            sender: row.get(5)?,
                            role: row.get(6)?,
                            text: row.get(7)?,
                        })
                    })?
                    .collect::<rusqlite::Result<_>>()?,
                Err(error) if error.to_string().contains("no such table") => Vec::new(),
                Err(error) => return Err(error),
            };
            let Some(last) = rows.last().map(|doc| doc.record.clone()) else {
                break;
            };
            for doc in &rows {
                batch.execute(
                    "INSERT OR IGNORE INTO docs (record, at_ms, kind, channel, place, sender, role, text) \
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
                "INSERT OR REPLACE INTO meta (key, value) VALUES ('copied', ?1)",
                [last],
            )?;
            batch.commit()?;
        }
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
    copy_records(index, folder)?;
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
            .prepare("SELECT seq, sender, text FROM docs WHERE seq > ?1 ORDER BY seq LIMIT ?2")?
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

/// `memory_search`: finds earlier messages and answers by their words.
#[derive(Debug)]
pub struct MemorySearch {
    memory: Arc<Memory>,
}

impl Tool for MemorySearch {
    fn name(&self) -> &str {
        "memory_search"
    }

    fn description(&self) -> &str {
        "Searches everything dotori has heard and said for these words, and for their meaning \
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
