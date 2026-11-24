use std::collections::HashSet;
use std::sync::Arc;

use rusqlite::Connection;
use serde::Deserialize;

use super::DEFAULT_LIMIT;
use super::Memory;
use super::NO_WORDS;
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
    /// Questions that could not be scored, each with why.
    pub left_out: Vec<(String, String)>,
    pub scores: Vec<Score>,
    /// Why ranking by meaning was not scored.
    pub no_meaning: Option<String>,
    /// When meaning was scored: how many records had a vector, out of the records with text.
    pub vectored: Option<(usize, usize)>,
}

/// How many hits a search shows.
pub const HITS: usize = DEFAULT_LIMIT as usize;

impl Memory {
    /// Ranks every question the way memory search does, by words, by meaning, and by both, and
    /// scores each way against the records that hold the expected words. Questions go to the
    /// embedding API only while search by meaning can still be scored.
    pub async fn evaluate(self: &Arc<Self>, cases: Vec<Case>) -> Result<Report, String> {
        let mut no_meaning = match &self.embedder {
            None => Some(String::from("no embedding API is set up")),
            Some(embedder) => {
                let (vectored, _) = coverage(self, embedder.name()).await?;
                (vectored == 0).then(|| String::from("no record has a vector yet"))
            }
        };
        let mut found = Vec::new();
        let mut left_out = Vec::new();
        for case in cases {
            let meaning = match &self.embedder {
                Some(embedder) if no_meaning.is_none() => {
                    let wanted = case.question.chars().take(vectors::LONGEST).collect();
                    Some(
                        match tokio::time::timeout(QUERY_WAIT, embedder.embed(&[wanted])).await {
                            Ok(Ok(mut made)) => made.pop().ok_or_else(|| String::from("no vector")),
                            Ok(Err(refused)) => Err(refused.detail),
                            Err(_) => Err(String::from("the embedding API took too long")),
                        },
                    )
                }
                _ => None,
            };
            if let Some(Err(error)) = &meaning {
                no_meaning = Some(format!("search by meaning failed: {error}"));
            }
            let asked_meaning = matches!(meaning, Some(Ok(_)));
            match rank(self, &case, meaning).await? {
                Outcome::Scored(ranked, expected) => {
                    if asked_meaning && ranked.by_meaning.is_none() && no_meaning.is_none() {
                        no_meaning = Some(
                            ranked
                                .note
                                .as_deref()
                                .map(|note| note.trim_matches(['(', ')']).to_owned())
                                .unwrap_or_else(|| String::from("search by meaning failed")),
                        );
                    }
                    found.push((ranked, expected));
                }
                Outcome::LeftOut(why) => left_out.push((case.question, why)),
            }
        }
        let vectored = match (&self.embedder, &no_meaning) {
            (Some(embedder), None) => Some(coverage(self, embedder.name()).await?),
            _ => None,
        };
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
            left_out,
            scores,
            no_meaning,
            vectored,
        })
    }
}

enum Outcome {
    Scored(Ranked, HashSet<i64>),
    LeftOut(String),
}

async fn rank(
    memory: &Arc<Memory>,
    case: &Case,
    meaning: Option<Result<Vec<f32>, String>>,
) -> Result<Outcome, String> {
    let memory = Arc::clone(memory);
    let case = case.clone();
    tokio::task::spawn_blocking(move || {
        let mut state = memory.lock();
        let ranked = match memory.ranked(&mut state, &case.question, DEFAULT_LIMIT, meaning) {
            Ok(Some(ranked)) => ranked,
            Ok(None) => return Err(String::from("nothing is recorded yet")),
            Err(error) if error == NO_WORDS => {
                return Ok(Outcome::LeftOut(String::from(
                    "it has no words to search for",
                )));
            }
            Err(error) => return Err(error),
        };
        let index = state.index.as_ref().ok_or("no index")?;
        let expected = holding(index, &case.expect)?;
        if expected.is_empty() {
            return Ok(Outcome::LeftOut(String::from(
                "no record holds the words it expects",
            )));
        }
        Ok(Outcome::Scored(ranked, expected))
    })
    .await
    .map_err(|error| error.to_string())?
}

async fn coverage(memory: &Arc<Memory>, model: String) -> Result<(usize, usize), String> {
    let memory = Arc::clone(memory);
    tokio::task::spawn_blocking(move || {
        let mut state = memory.lock();
        memory.opened(&mut state)?;
        let index = state.index.as_mut().ok_or("no index")?;
        vectors::merge_copied(index).map_err(|error| error.to_string())?;
        let index = &*index;
        let count = |sql: &str, model: Option<&str>| {
            match model {
                Some(model) => index.query_row(sql, [model], |row| row.get::<_, i64>(0)),
                None => index.query_row(sql, [], |row| row.get::<_, i64>(0)),
            }
            .map(|counted| usize::try_from(counted).unwrap_or(0))
            .map_err(|error| error.to_string())
        };
        Ok((
            count(
                "SELECT count(*) FROM vectors WHERE model = ?1",
                Some(&model),
            )?,
            count(
                "SELECT count(*) FROM docs WHERE trim(coalesce(text, '')) != ''",
                None,
            )?,
        ))
    })
    .await
    .map_err(|error| error.to_string())?
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
