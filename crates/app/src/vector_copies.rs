//! Copies search vectors from the PC that makes them to the others, so the embedding API is
//! paid once. Each PC asks the maker for the vectors made since the last ones it got, by the
//! order the maker made them in, and keeps them until their records reach its own index.

use std::path::Path;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::Mutex;
use std::time::Duration;
use std::time::Instant;

use dotori_connectors::peer::base64url;
use dotori_connectors::peer::hub::Heard;
use dotori_connectors::peer::unbase64url;
use rusqlite::Connection;
use rusqlite::OptionalExtension;
use rusqlite::params;
use serde::Deserialize;
use serde::Serialize;
use serde_json::Value;
use serde_json::json;
use tokio::time;

use crate::main_session::Linked;

pub(crate) const VECTORS_JOB: &str = "vectors";
const BATCH: i64 = 200;
const ASK_EVERY: Duration = Duration::from_secs(30);
const ANSWER_WAIT: Duration = Duration::from_secs(60);
const BUSY: Duration = Duration::from_secs(5);
const TABLES: &str = "
CREATE TABLE IF NOT EXISTS meta (key TEXT PRIMARY KEY, value TEXT NOT NULL);
CREATE TABLE IF NOT EXISTS copied_vectors (
    record TEXT NOT NULL,
    model TEXT NOT NULL,
    vector BLOB NOT NULL,
    PRIMARY KEY (record, model)
);
CREATE TABLE IF NOT EXISTS forgotten (record TEXT PRIMARY KEY) WITHOUT ROWID;
";

/// One vector as it travels: the record it is for, the model that made it, and its bytes.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct Sent {
    pub(crate) record: String,
    pub(crate) model: String,
    pub(crate) vector: String,
}

/// The vectors made here after the `after`th one in the index named `asked`, the place of the
/// last one given, and this index's name. An index made again since starts from the first.
pub(crate) fn made_after(
    index: &Path,
    asked: &str,
    after: i64,
) -> Result<(Vec<Sent>, i64, String), String> {
    let fail = |error: rusqlite::Error| error.to_string();
    let connection = Connection::open(index).map_err(fail)?;
    let name: String = match connection
        .query_row("SELECT value FROM meta WHERE key = 'index_id'", [], |row| {
            row.get(0)
        })
        .optional()
    {
        Ok(name) => name.unwrap_or_default(),
        Err(error) if error.to_string().contains("no such table") => {
            return Ok((Vec::new(), 0, String::new()));
        }
        Err(error) => return Err(fail(error)),
    };
    let after = if asked == name { after } else { 0 };
    let found = connection.prepare(
        "SELECT l.n, d.record, v.model, v.vector FROM vector_log l \
         JOIN vectors v ON v.seq = l.seq JOIN docs d ON d.seq = l.seq \
         WHERE l.n > ?1 ORDER BY l.n LIMIT ?2",
    );
    let mut found = match found {
        Ok(found) => found,
        Err(error) if error.to_string().contains("no such table") => {
            return Ok((Vec::new(), after, name));
        }
        Err(error) => return Err(fail(error)),
    };
    let rows: Vec<(i64, String, String, Vec<u8>)> = found
        .query_map(params![after, BATCH], |row| {
            Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?))
        })
        .map_err(fail)?
        .collect::<rusqlite::Result<_>>()
        .map_err(fail)?;
    let last = rows.last().map_or(after, |row| row.0);
    let sent = rows
        .into_iter()
        .map(|(_, record, model, vector)| Sent {
            record,
            model,
            vector: base64url(&vector),
        })
        .collect();
    Ok((sent, last, name))
}

/// Keeps vectors from the PC `from` until their records are indexed here, and how far this PC
/// got with that PC's index `name`.
pub(crate) fn keep(
    index: &Path,
    from: &str,
    sent: &[Sent],
    name: &str,
    upto: i64,
) -> Result<(), String> {
    let fail = |error: rusqlite::Error| error.to_string();
    let mut connection = Connection::open(index).map_err(fail)?;
    connection.busy_timeout(BUSY).map_err(fail)?;
    connection.execute_batch(TABLES).map_err(fail)?;
    let batch = connection.transaction().map_err(fail)?;
    for one in sent {
        let forgotten: bool = batch
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM forgotten WHERE record = ?1)",
                [&one.record],
                |row| row.get(0),
            )
            .map_err(fail)?;
        if forgotten {
            continue;
        }
        let vector = unbase64url(&one.vector)
            .filter(|bytes| bytes.len() % 4 == 0)
            .ok_or_else(|| format!("a vector for {} does not read", one.record))?;
        batch
            .execute(
                "INSERT OR REPLACE INTO copied_vectors (record, model, vector) VALUES (?1, ?2, ?3)",
                params![one.record, one.model, vector],
            )
            .map_err(fail)?;
    }
    batch
        .execute(
            "INSERT OR REPLACE INTO meta (key, value) VALUES (?1, ?2)",
            params![mark_key(from), format!("{upto} {name}")],
        )
        .map_err(fail)?;
    batch.commit().map_err(fail)
}

/// How far this PC got with the vectors of the PC `from`, and the name of that PC's index then.
pub(crate) fn mark(index: &Path, from: &str) -> (i64, String) {
    let Ok(connection) = Connection::open(index) else {
        return (0, String::new());
    };
    let kept = connection
        .query_row(
            "SELECT value FROM meta WHERE key = ?1",
            [mark_key(from)],
            |row| row.get::<_, String>(0),
        )
        .optional()
        .ok()
        .flatten()
        .unwrap_or_default();
    let (upto, name) = kept.split_once(' ').unwrap_or((kept.as_str(), ""));
    (upto.parse().unwrap_or(0), name.to_owned())
}

fn mark_key(from: &str) -> String {
    format!("vectors_from:{from}")
}

/// Asks the PC that makes vectors for the ones it made since the last this PC got, one question
/// at a time.
#[derive(Debug)]
pub(crate) struct VectorCopies {
    linked: Linked,
    index: PathBuf,
    asked: Mutex<Option<Instant>>,
}

impl VectorCopies {
    pub(crate) fn new(linked: Linked, index: PathBuf) -> Arc<Self> {
        Arc::new(Self {
            linked,
            index,
            asked: Mutex::new(None),
        })
    }

    async fn ask(self: &Arc<Self>) {
        let Some(maker) = self
            .linked
            .keeper
            .watch(VECTORS_JOB)
            .and_then(|holder| holder.borrow().clone())
            .filter(|maker| *maker != self.linked.own)
        else {
            return;
        };
        {
            let mut asked = self.asked.lock().expect("unpoisoned");
            if asked.is_some_and(|at| at.elapsed() < ANSWER_WAIT) {
                return;
            }
            *asked = Some(Instant::now());
        }
        let copies = Arc::clone(self);
        let from = maker.clone();
        let Ok((upto, name)) =
            tokio::task::spawn_blocking(move || mark(&copies.index, &from)).await
        else {
            return;
        };
        self.linked.hub.send(
            &maker,
            json!({ "kind": "vectors_since", "upto": upto, "index": name }),
        );
    }

    /// Asks again every so often, in case the maker made more.
    pub(crate) async fn keep_asking(self: Arc<Self>) {
        let mut every = time::interval(ASK_EVERY);
        loop {
            every.tick().await;
            self.ask().await;
        }
    }

    /// Takes a message about vectors from another PC. Says whether it was one. The work with
    /// the index happens apart, so other messages do not wait for it.
    pub(crate) fn hear(self: &Arc<Self>, heard: &Heard) -> bool {
        let kind = heard.message.get("kind").and_then(Value::as_str);
        if !matches!(kind, Some("vectors_since" | "vectors")) {
            return false;
        }
        let copies = Arc::clone(self);
        let heard = heard.clone();
        tokio::spawn(async move { copies.answer(heard).await });
        true
    }

    async fn answer(self: Arc<Self>, heard: Heard) {
        match heard.message.get("kind").and_then(Value::as_str) {
            Some("vectors_since") => {
                if !self.linked.keeper.holds(VECTORS_JOB) {
                    return;
                }
                let after = heard
                    .message
                    .get("upto")
                    .and_then(Value::as_i64)
                    .unwrap_or(0);
                let asked = heard
                    .message
                    .get("index")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_owned();
                let index = self.index.clone();
                let made =
                    tokio::task::spawn_blocking(move || made_after(&index, &asked, after)).await;
                match made {
                    Ok(Ok((sent, upto, name))) => {
                        self.linked.hub.send(
                            &heard.key,
                            json!({ "kind": "vectors", "rows": sent, "upto": upto, "index": name }),
                        );
                    }
                    Ok(Err(error)) => {
                        eprintln!("dotori: vectors could not be read to share: {error}")
                    }
                    Err(_) => {}
                }
            }
            Some("vectors") => {
                *self.asked.lock().expect("unpoisoned") = None;
                let sent: Option<Vec<Sent>> = heard
                    .message
                    .get("rows")
                    .and_then(|rows| serde_json::from_value(rows.clone()).ok());
                let upto = heard.message.get("upto").and_then(Value::as_i64);
                let name = heard
                    .message
                    .get("index")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_owned();
                let (Some(sent), Some(upto)) = (sent, upto) else {
                    eprintln!("dotori: vectors from {} do not read", heard.device);
                    return;
                };
                let more = !sent.is_empty();
                let index = self.index.clone();
                let from = heard.key.clone();
                let kept =
                    tokio::task::spawn_blocking(move || keep(&index, &from, &sent, &name, upto))
                        .await;
                match kept {
                    Ok(Ok(())) if more => self.ask().await,
                    Ok(Ok(())) => {}
                    Ok(Err(error)) => {
                        eprintln!(
                            "dotori: vectors from {} were not kept: {error}",
                            heard.device
                        );
                    }
                    Err(_) => {}
                }
            }
            _ => {}
        }
    }

    /// Asks the new maker at once when the job moves.
    pub(crate) async fn on_maker_change(self: Arc<Self>) {
        let Some(mut holder) = self.linked.keeper.watch(VECTORS_JOB) else {
            return;
        };
        while holder.changed().await.is_ok() {
            *self.asked.lock().expect("unpoisoned") = None;
            self.ask().await;
        }
    }
}

/// Tells the MCP server here whether this PC holds `job` now.
pub(crate) fn write_job(jobs: &Path, job: &str, holds: bool) {
    static WRITING: Mutex<()> = Mutex::new(());
    let _writing = WRITING.lock().expect("unpoisoned");
    let mut all = std::fs::read_to_string(jobs)
        .ok()
        .and_then(|text| serde_json::from_str::<serde_json::Map<String, Value>>(&text).ok())
        .unwrap_or_default();
    all.insert(job.to_owned(), Value::Bool(holds));
    let text = Value::Object(all).to_string();
    let draft = jobs.with_extension("json.new");
    let written = std::fs::write(&draft, text).and_then(|()| std::fs::rename(&draft, jobs));
    if let Err(error) = written {
        eprintln!("dotori: {}: {error}", jobs.display());
    }
}

#[cfg(test)]
#[path = "vector_copies.test.rs"]
mod tests;
