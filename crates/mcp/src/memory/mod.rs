//! The memory search tool: keyword search over the raw record, with an index that can always be
//! made again from it.

mod analyzer;
mod dictionary;

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::Mutex;

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

const BATCH: i64 = 1000;
const DEFAULT_LIMIT: u64 = 10;
const MAX_LIMIT: u64 = 50;
const SHOWN: usize = 400;
const SCHEMA: &str = "
CREATE TABLE IF NOT EXISTS meta (key TEXT PRIMARY KEY, value TEXT NOT NULL);
CREATE VIRTUAL TABLE IF NOT EXISTS terms USING fts5(words, tokenize = 'unicode61');
";

/// Where memory reads the raw record, keeps its index, and keeps dictionaries.
#[derive(Clone, Debug)]
pub struct MemoryConfig {
    pub record: PathBuf,
    pub index: PathBuf,
    pub dictionaries: PathBuf,
}

/// Memory shared by its tools. The index catches up with the raw record before every search.
pub struct Memory {
    config: MemoryConfig,
    state: Mutex<State>,
}

struct State {
    index: Option<Connection>,
    analyzer: Analyzer,
    dictionary_failed: bool,
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
    /// split by a simpler rule.
    pub fn new(config: MemoryConfig) -> Arc<Self> {
        let memory = Arc::new(Self {
            config,
            state: Mutex::new(State {
                index: None,
                analyzer: Analyzer::Plain,
                dictionary_failed: false,
            }),
        });
        let folder = memory.config.dictionaries.clone();
        if tokio::runtime::Handle::try_current().is_ok() {
            tokio::spawn(async move {
                if let Err(error) = dictionary::fetch(&folder).await {
                    eprintln!("dotori: memory search uses a simpler word split: {error}");
                }
            });
        }
        memory
    }

    /// The tools that use this memory.
    pub fn tools(self: &Arc<Self>) -> Vec<Box<dyn Tool>> {
        vec![Box::new(MemorySearch {
            memory: Arc::clone(self),
        })]
    }

    fn search(&self, query: &str, limit: u64) -> Result<String, String> {
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
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
        if state.index.is_none() {
            state.index = Some(open_index(&self.config.index)?);
        }
        let State {
            index, analyzer, ..
        } = &mut *state;
        let index = index.as_mut().ok_or("no index")?;
        let record = match open_record(&self.config.record)? {
            Some(record) => record,
            None => return Ok(String::from("Nothing is recorded yet.")),
        };
        match catch_up(index, &record, analyzer) {
            Ok(()) => {}
            Err(error) if error.to_string().contains("no such table") => {
                return Ok(String::from("Nothing is recorded yet."));
            }
            Err(error) => return Err(format!("the search index could not catch up: {error}")),
        }
        let terms = analyzer.terms(query);
        if terms.is_empty() {
            return Err(String::from("the query has no words to search for"));
        }
        let matched = terms
            .iter()
            .map(|term| format!("\"{}\"", term.replace('"', "\"\"")))
            .collect::<Vec<_>>()
            .join(" OR ");
        let seqs: Vec<i64> = index
            .prepare("SELECT rowid FROM terms WHERE words MATCH ?1 ORDER BY rank LIMIT ?2")
            .and_then(|mut found| {
                found
                    .query_map(
                        params![matched, i64::try_from(limit).unwrap_or(MAX_LIMIT as i64)],
                        |row| row.get(0),
                    )?
                    .collect()
            })
            .map_err(|error| error.to_string())?;
        if seqs.is_empty() {
            return Ok(String::from("Nothing in memory matches."));
        }
        let mut shown = Vec::new();
        for seq in seqs {
            let row = record
                .query_row(
                    "SELECT at_ms, kind, channel, place, sender_name, role, text FROM records WHERE seq = ?1",
                    [seq],
                    |row| {
                        Ok(Hit {
                            seq,
                            at_ms: row.get(0)?,
                            kind: row.get(1)?,
                            channel: row.get(2)?,
                            place: row.get(3)?,
                            sender: row.get(4)?,
                            role: row.get(5)?,
                            text: row.get(6)?,
                        })
                    },
                )
                .optional()
                .map_err(|error| error.to_string())?;
            if let Some(hit) = row {
                shown.push(hit.line());
            }
        }
        Ok(shown.join("\n"))
    }
}

struct Hit {
    seq: i64,
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
            self.seq,
            when(self.at_ms)
        )
    }
}

fn open_index(path: &PathBuf) -> Result<Connection, String> {
    if let Some(folder) = path.parent() {
        std::fs::create_dir_all(folder).map_err(|error| error.to_string())?;
    }
    let index = Connection::open(path).map_err(|error| error.to_string())?;
    index
        .execute_batch(SCHEMA)
        .map_err(|error| error.to_string())?;
    Ok(index)
}

fn open_record(path: &PathBuf) -> Result<Option<Connection>, String> {
    if !path.is_file() {
        return Ok(None);
    }
    Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY)
        .map(Some)
        .map_err(|error| format!("cannot read the record: {error}"))
}

fn meta(index: &Connection, key: &str) -> rusqlite::Result<Option<String>> {
    index
        .query_row("SELECT value FROM meta WHERE key = ?1", [key], |row| {
            row.get(0)
        })
        .optional()
}

fn catch_up(
    index: &mut Connection,
    record: &Connection,
    analyzer: &Analyzer,
) -> rusqlite::Result<()> {
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
        let rows: Vec<(i64, Option<String>, Option<String>)> = record
            .prepare(
                "SELECT seq, sender_name, text FROM records WHERE seq > ?1 ORDER BY seq LIMIT ?2",
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
        "Searches everything dotori has heard and said for these words, best matches first. \
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
            tokio::task::spawn_blocking(move || memory.search(&query, limit))
                .await
                .map_err(|error| error.to_string())?
        })
    }
}

#[cfg(test)]
#[path = "mod.test.rs"]
mod tests;
