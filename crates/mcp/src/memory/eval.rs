use std::collections::HashSet;
use std::sync::Arc;

use rusqlite::Connection;
use serde::Deserialize;

use super::DEFAULT_LIMIT;
use super::Memory;
use super::QUERY_WAIT;
use super::Ranked;
use super::vectors;

/// A question from the owner's evaluation set, with words that only the records it should find
/// hold.
#[derive(Clone, Debug, Deserialize)]
pub struct Case {
    pub question: String,
    pub expect: Vec<String>,
}

/// A way of ranking records.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Method {
    Words,
    Meaning,
    Both,
}

/// How well one way of ranking found the records the owner expected.
#[derive(Clone, Debug, PartialEq)]
pub struct Score {
    pub method: Method,
    /// Questions whose first hit was one of the expected records.
    pub first: usize,
    /// Questions with an expected record among the hits a search shows.
    pub shown: usize,
    /// The mean of one over the rank of the first expected record, zero when none is shown.
    pub reciprocal: f64,
}

/// What evaluating a set of questions found.
#[derive(Clone, Debug, PartialEq)]
pub struct Report {
    pub scored: usize,
    /// Questions left out because no record holds the words they expect.
    pub unmatched: Vec<String>,
    pub scores: Vec<Score>,
    /// Why ranking by meaning was not scored.
    pub no_meaning: Option<String>,
}

/// How many hits a search shows.
pub const HITS: usize = DEFAULT_LIMIT as usize;

impl Memory {
    /// Ranks every question the way memory search does, by words, by meaning, and by both, and
    /// scores each way against the records that hold the expected words.
    pub async fn evaluate(self: &Arc<Self>, cases: Vec<Case>) -> Result<Report, String> {
        let mut found = Vec::new();
        let mut no_meaning = self
            .embedder
            .is_none()
            .then(|| String::from("no embedding API is set up"));
        let mut unmatched = Vec::new();
        for case in cases {
            let meaning = match &self.embedder {
                Some(embedder) => Some(
                    match tokio::time::timeout(
                        QUERY_WAIT,
                        embedder.embed(&[case.question.chars().take(vectors::LONGEST).collect()]),
                    )
                    .await
                    {
                        Ok(Ok(mut made)) => made.pop().ok_or_else(|| String::from("no vector")),
                        Ok(Err(refused)) => Err(refused.detail),
                        Err(_) => Err(String::from("the embedding API took too long")),
                    },
                ),
                None => None,
            };
            let memory = Arc::clone(self);
            let ranked = tokio::task::spawn_blocking(move || {
                let mut state = memory.lock();
                let ranked = memory
                    .ranked(&mut state, &case.question, DEFAULT_LIMIT, meaning)?
                    .ok_or_else(|| String::from("nothing is recorded yet"))?;
                let index = state.index.as_ref().ok_or("no index")?;
                let expected = holding(index, &case.expect)?;
                let vectored = match &memory.embedder {
                    Some(embedder) => {
                        vectors::any(index, &embedder.name()).map_err(|error| error.to_string())?
                    }
                    None => false,
                };
                Ok::<_, String>((case.question, ranked, expected, vectored))
            })
            .await
            .map_err(|error| error.to_string())??;
            let (question, ranked, expected, vectored) = ranked;
            if expected.is_empty() {
                unmatched.push(question);
                continue;
            }
            if no_meaning.is_none() {
                if !vectored {
                    no_meaning = Some(String::from("no record has a vector yet"));
                } else if ranked.by_meaning.is_none() {
                    no_meaning = Some(
                        ranked
                            .note
                            .clone()
                            .unwrap_or_else(|| String::from("search by meaning failed")),
                    );
                }
            }
            found.push((ranked, expected));
        }
        let mut methods = vec![Method::Words];
        if no_meaning.is_none() {
            methods.extend([Method::Meaning, Method::Both]);
        }
        let scores = methods
            .into_iter()
            .map(|method| score(method, &found))
            .collect();
        Ok(Report {
            scored: found.len(),
            unmatched,
            scores,
            no_meaning,
        })
    }
}

fn holding(index: &Connection, expect: &[String]) -> Result<HashSet<i64>, String> {
    let mut held = HashSet::new();
    let mut matching = index
        .prepare("SELECT seq FROM docs WHERE instr(text, ?1) > 0")
        .map_err(|error| error.to_string())?;
    for words in expect.iter().filter(|words| !words.trim().is_empty()) {
        let seqs = matching
            .query_map([words], |row| row.get::<_, i64>(0))
            .and_then(|rows| rows.collect::<Result<Vec<_>, _>>())
            .map_err(|error| error.to_string())?;
        held.extend(seqs);
    }
    Ok(held)
}

fn score(method: Method, found: &[(Ranked, HashSet<i64>)]) -> Score {
    let mut first = 0;
    let mut shown = 0;
    let mut reciprocal = 0.0;
    for (ranked, expected) in found {
        let meaning = ranked.by_meaning.clone().unwrap_or_default();
        let order = match method {
            Method::Words => ranked.by_words.iter().copied().take(HITS).collect(),
            Method::Meaning => meaning.into_iter().take(HITS).collect(),
            Method::Both => vectors::fuse(&[ranked.by_words.clone(), meaning], HITS),
        };
        if let Some(rank) = order.iter().position(|seq| expected.contains(seq)) {
            shown += 1;
            if rank == 0 {
                first += 1;
            }
            reciprocal += 1.0 / (rank + 1) as f64;
        }
    }
    Score {
        method,
        first,
        shown,
        reciprocal: if found.is_empty() {
            0.0
        } else {
            reciprocal / found.len() as f64
        },
    }
}

#[cfg(test)]
#[path = "eval.test.rs"]
mod tests;
