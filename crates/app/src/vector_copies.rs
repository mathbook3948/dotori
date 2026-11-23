//! Copies search vectors from the PC that makes them to the others, so the embedding API is
//! paid once. Each PC asks the maker for the vectors made since the last ones it got, by the
//! order the maker made them in, and keeps them until their records reach its own index.

use std::path::Path;
use std::path::PathBuf;
use std::time::Duration;

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
const TABLES: &str = "
CREATE TABLE IF NOT EXISTS meta (key TEXT PRIMARY KEY, value TEXT NOT NULL);
CREATE TABLE IF NOT EXISTS copied_vectors (
    record TEXT NOT NULL,
    model TEXT NOT NULL,
    vector BLOB NOT NULL,
    PRIMARY KEY (record, model)
);
";

/// One vector as it travels: the record it is for, the model that made it, and its bytes.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct Sent {
    pub(crate) record: String,
    pub(crate) model: String,
    pub(crate) vector: String,
}

/// The vectors made here after the `after`th one, and the place of the last one given.
pub(crate) fn made_after(index: &Path, after: i64) -> Result<(Vec<Sent>, i64), String> {
    let fail = |error: rusqlite::Error| error.to_string();
    let connection = Connection::open(index).map_err(fail)?;
    let found = connection.prepare(
        "SELECT l.n, d.record, v.model, v.vector FROM vector_log l \
         JOIN vectors v ON v.seq = l.seq JOIN docs d ON d.seq = l.seq \
         WHERE l.n > ?1 ORDER BY l.n LIMIT ?2",
    );
    let mut found = match found {
        Ok(found) => found,
        Err(error) if error.to_string().contains("no such table") => {
            return Ok((Vec::new(), after));
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
    Ok((sent, last))
}

/// Keeps vectors from the PC `from` until their records are indexed here, and how far this PC
/// got with that PC's vectors.
pub(crate) fn keep(index: &Path, from: &str, sent: &[Sent], upto: i64) -> Result<(), String> {
    let fail = |error: rusqlite::Error| error.to_string();
    let mut connection = Connection::open(index).map_err(fail)?;
    connection.execute_batch(TABLES).map_err(fail)?;
    let batch = connection.transaction().map_err(fail)?;
    for one in sent {
        let Some(vector) = unbase64url(&one.vector).filter(|bytes| bytes.len() % 4 == 0) else {
            continue;
        };
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
            params![mark_key(from), upto.to_string()],
        )
        .map_err(fail)?;
    batch.commit().map_err(fail)
}

/// How far this PC got with the vectors of the PC `from`.
pub(crate) fn mark(index: &Path, from: &str) -> i64 {
    let Ok(connection) = Connection::open(index) else {
        return 0;
    };
    connection
        .query_row(
            "SELECT value FROM meta WHERE key = ?1",
            [mark_key(from)],
            |row| row.get::<_, String>(0),
        )
        .optional()
        .ok()
        .flatten()
        .and_then(|mark| mark.parse().ok())
        .unwrap_or(0)
}

fn mark_key(from: &str) -> String {
    format!("vectors_from:{from}")
}

/// Asks the PC that makes vectors for the ones made since the last this PC got.
pub(crate) fn ask(linked: &Linked, index: &Path) {
    let Some(maker) = linked
        .keeper
        .watch(VECTORS_JOB)
        .and_then(|holder| holder.borrow().clone())
        .filter(|maker| *maker != linked.own)
    else {
        return;
    };
    let upto = mark(index, &maker);
    linked
        .hub
        .send(&maker, json!({ "kind": "vectors_since", "upto": upto }));
}

/// Asks again every so often, in case the maker made more.
pub(crate) async fn keep_asking(linked: Linked, index: PathBuf) {
    let mut every = time::interval(ASK_EVERY);
    loop {
        every.tick().await;
        ask(&linked, &index);
    }
}

/// Takes a message about vectors from another PC. Says whether it was one.
pub(crate) fn hear(linked: &Linked, index: &Path, heard: &Heard) -> bool {
    match heard.message.get("kind").and_then(Value::as_str) {
        Some("vectors_since") => {
            if !linked.keeper.holds(VECTORS_JOB) {
                return true;
            }
            let after = heard
                .message
                .get("upto")
                .and_then(Value::as_i64)
                .unwrap_or(0);
            match made_after(index, after) {
                Ok((sent, upto)) => {
                    linked.hub.send(
                        &heard.key,
                        json!({ "kind": "vectors", "rows": sent, "upto": upto }),
                    );
                }
                Err(error) => eprintln!("dotori: vectors could not be read to share: {error}"),
            }
            true
        }
        Some("vectors") => {
            let sent: Vec<Sent> = heard
                .message
                .get("rows")
                .and_then(|rows| serde_json::from_value(rows.clone()).ok())
                .unwrap_or_default();
            let Some(upto) = heard.message.get("upto").and_then(Value::as_i64) else {
                return true;
            };
            match keep(index, &heard.key, &sent, upto) {
                Ok(()) if !sent.is_empty() => ask(linked, index),
                Ok(()) => {}
                Err(error) => eprintln!(
                    "dotori: vectors from {} were not kept: {error}",
                    heard.device
                ),
            }
            true
        }
        _ => false,
    }
}

/// Tells the MCP server here whether this PC makes vectors now.
pub(crate) fn write_jobs(jobs: &Path, makes_vectors: bool) {
    let text = json!({ "vectors": makes_vectors }).to_string();
    if let Err(error) = std::fs::write(jobs, text) {
        eprintln!("dotori: {}: {error}", jobs.display());
    }
}

#[cfg(test)]
#[path = "vector_copies.test.rs"]
mod tests;
