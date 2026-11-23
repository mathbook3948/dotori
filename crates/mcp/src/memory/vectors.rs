use rusqlite::Connection;
use rusqlite::OptionalExtension;
use rusqlite::TransactionBehavior;
use rusqlite::params;

use super::embedding::from_bytes;
use super::embedding::to_bytes;
use super::meta;

/// The longest text sent for one vector, in characters.
pub(crate) const LONGEST: usize = 2000;

/// The vectors of one model, held in memory so a search compares against all of them.
#[derive(Default)]
pub(crate) struct Vectors {
    model: String,
    merged: Option<String>,
    upto: i64,
    width: usize,
    seqs: Vec<i64>,
    values: Vec<f32>,
}

impl Vectors {
    /// Reads the vectors stored since the last load. Another model's vectors are never read.
    pub(crate) fn load(&mut self, index: &Connection, model: &str) -> rusqlite::Result<()> {
        let merged = meta(index, "vectors_merged")?;
        if self.model != model || self.merged != merged {
            *self = Self {
                model: model.to_owned(),
                merged,
                ..Self::default()
            };
        }
        let mut found = index.prepare(
            "SELECT seq, vector FROM vectors WHERE model = ?1 AND seq > ?2 ORDER BY seq",
        )?;
        let rows = found.query_map(params![model, self.upto], |row| {
            Ok((row.get::<_, i64>(0)?, row.get::<_, Vec<u8>>(1)?))
        })?;
        for row in rows {
            let (seq, bytes) = row?;
            self.upto = seq;
            if bytes.is_empty() || bytes.len() % 4 != 0 {
                continue;
            }
            let vector = from_bytes(&bytes);
            if self.width == 0 {
                self.width = vector.len();
            }
            if vector.len() != self.width {
                continue;
            }
            self.seqs.push(seq);
            self.values.extend(vector);
        }
        Ok(())
    }

    /// The records closest in meaning to `query`, closest first. A query of another length than
    /// the stored vectors cannot be compared.
    pub(crate) fn nearest(&self, query: &[f32], count: usize) -> Result<Vec<i64>, String> {
        if self.width == 0 {
            return Ok(Vec::new());
        }
        if query.len() != self.width {
            return Err(format!(
                "the query vector has {} numbers and the stored ones {}",
                query.len(),
                self.width
            ));
        }
        let mut scored: Vec<(f32, i64)> = self
            .values
            .chunks_exact(self.width)
            .zip(&self.seqs)
            .map(|(vector, seq)| {
                let score = vector.iter().zip(query).map(|(a, b)| a * b).sum::<f32>();
                (score, *seq)
            })
            .collect();
        let closer = |a: &(f32, i64), b: &(f32, i64)| b.0.total_cmp(&a.0).then(b.1.cmp(&a.1));
        if count < scored.len() {
            scored.select_nth_unstable_by(count, closer);
            scored.truncate(count);
        }
        scored.sort_by(closer);
        Ok(scored.into_iter().map(|(_, seq)| seq).collect())
    }
}

/// Texts of records waiting for vectors, by record number, and the last record looked at.
pub(crate) type Pending = (Vec<(i64, String)>, Option<i64>);

/// Texts of the next records without a vector from `model`, and the last record looked at.
/// Changing the model drops the other model's vectors and starts again from the first record.
pub(crate) fn pending(
    index: &mut Connection,
    model: &str,
    count: i64,
) -> rusqlite::Result<Pending> {
    let start = index.transaction_with_behavior(TransactionBehavior::Immediate)?;
    if meta(&start, "vector_model")?.as_deref() != Some(model) {
        start.execute(
            "INSERT OR REPLACE INTO meta (key, value) VALUES ('vector_model', ?1), ('vector_done', '0')",
            [model],
        )?;
        start.execute("DELETE FROM vectors WHERE model != ?1", [model])?;
    }
    let done: i64 = meta(&start, "vector_done")?
        .and_then(|done| done.parse().ok())
        .unwrap_or(0);
    start.commit()?;
    let rows: Vec<(i64, Option<String>, Option<String>, bool)> = index
        .prepare(
            "SELECT d.seq, d.sender, d.text, \
             EXISTS (SELECT 1 FROM vectors v WHERE v.seq = d.seq AND v.model = ?3) \
             FROM docs d WHERE d.seq > ?1 ORDER BY d.seq LIMIT ?2",
        )?
        .query_map(params![done, count, model], |row| {
            Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?))
        })?
        .collect::<rusqlite::Result<_>>()?;
    let last = rows.last().map(|row| row.0);
    let texts = rows
        .into_iter()
        .filter(|(_, _, _, held)| !held)
        .filter_map(|(seq, sender, text, _)| {
            let text = text.filter(|text| !text.trim().is_empty())?;
            let text = match sender {
                Some(sender) => format!("{sender}: {text}"),
                None => text,
            };
            Some((seq, text.chars().take(LONGEST).collect()))
        })
        .collect();
    Ok((texts, last))
}

/// Keeps vectors made by `model` and moves past `last`. Refused when another process changed the
/// model meanwhile, or when the vectors are not as long as the ones already kept.
pub(crate) fn store(
    index: &mut Connection,
    model: &str,
    made: &[(i64, Vec<f32>)],
    last: i64,
) -> Result<(), String> {
    let fail = |error: rusqlite::Error| error.to_string();
    let batch = index
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(fail)?;
    if meta(&batch, "vector_model").map_err(fail)?.as_deref() != Some(model) {
        return Err(String::from(
            "another dotori is making vectors with another model",
        ));
    }
    let kept: Option<i64> = batch
        .query_row(
            "SELECT length(vector) FROM vectors WHERE model = ?1 LIMIT 1",
            [model],
            |row| row.get(0),
        )
        .optional()
        .map_err(fail)?;
    let width = kept.or_else(|| {
        made.first()
            .and_then(|(_, vector)| i64::try_from(vector.len() * 4).ok())
    });
    if made
        .iter()
        .any(|(_, vector)| i64::try_from(vector.len() * 4).ok() != width)
    {
        return Err(String::from(
            "the embedding API answered vectors of another length than before",
        ));
    }
    for (seq, vector) in made {
        batch
            .execute(
                "INSERT OR REPLACE INTO vectors (seq, model, vector) VALUES (?1, ?2, ?3)",
                params![seq, model, to_bytes(vector)],
            )
            .map_err(fail)?;
        batch
            .execute("INSERT INTO vector_log (seq) VALUES (?1)", [seq])
            .map_err(fail)?;
    }
    batch
        .execute(
            "INSERT OR REPLACE INTO meta (key, value) VALUES ('vector_done', ?1)",
            [last.to_string()],
        )
        .map_err(fail)?;
    batch.commit().map_err(fail)
}

/// Moves vectors another PC made into place once their records are in the index here. Says
/// whether any moved, and if so the vectors held in memory are read again.
pub(crate) fn merge_copied(index: &mut Connection) -> rusqlite::Result<bool> {
    let merge = index.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let ready: Vec<(i64, String, String, Vec<u8>)> = merge
        .prepare(
            "SELECT d.seq, c.record, c.model, c.vector FROM copied_vectors c \
             JOIN docs d ON d.record = c.record",
        )?
        .query_map([], |row| {
            Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?))
        })?
        .collect::<rusqlite::Result<_>>()?;
    for (seq, record, model, vector) in &ready {
        let added = merge.execute(
            "INSERT OR IGNORE INTO vectors (seq, model, vector) VALUES (?1, ?2, ?3)",
            params![seq, model, vector],
        )?;
        if added > 0 {
            merge.execute("INSERT INTO vector_log (seq) VALUES (?1)", [seq])?;
        }
        merge.execute(
            "DELETE FROM copied_vectors WHERE record = ?1 AND model = ?2",
            params![record, model],
        )?;
    }
    if !ready.is_empty() {
        let merged: i64 = meta(&merge, "vectors_merged")?
            .and_then(|merged| merged.parse().ok())
            .unwrap_or(0);
        merge.execute(
            "INSERT OR REPLACE INTO meta (key, value) VALUES ('vectors_merged', ?1)",
            [(merged + 1).to_string()],
        )?;
    }
    merge.commit()?;
    Ok(!ready.is_empty())
}

/// Whether any vector from `model` is kept, so a search is worth embedding its query for.
pub(crate) fn any(index: &Connection, model: &str) -> rusqlite::Result<bool> {
    index
        .query_row(
            "SELECT 1 FROM vectors WHERE model = ?1 LIMIT 1",
            [model],
            |_| Ok(()),
        )
        .optional()
        .map(|found| found.is_some())
}

/// Joins ranked lists so a record high in either list comes first, by reciprocal rank fusion.
pub(crate) fn fuse(lists: &[Vec<i64>], count: usize) -> Vec<i64> {
    let mut scores: Vec<(i64, f64)> = Vec::new();
    for list in lists {
        for (rank, seq) in list.iter().enumerate() {
            let score = 1.0 / (60.0 + rank as f64);
            match scores.iter_mut().find(|(known, _)| known == seq) {
                Some((_, total)) => *total += score,
                None => scores.push((*seq, score)),
            }
        }
    }
    scores.sort_by(|a, b| b.1.total_cmp(&a.1));
    scores.into_iter().take(count).map(|(seq, _)| seq).collect()
}

#[cfg(test)]
#[path = "vectors.test.rs"]
mod tests;
