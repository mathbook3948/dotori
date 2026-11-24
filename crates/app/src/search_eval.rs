//! Scoring memory search against the owner's evaluation set, so the embedding model and the way
//! of searching are chosen from real use.

use std::fmt::Write;

use dotori_mcp::memory::Case;
use dotori_mcp::memory::HITS;
use dotori_mcp::memory::Memory;
use dotori_mcp::memory::MemoryConfig;
use dotori_mcp::memory::Method;
use dotori_mcp::memory::Report;

use crate::home::Home;
use crate::settings;
use crate::settings::Settings;

const USAGE: &str =
    "usage: dotori search-eval <file>, one {\"question\": ..., \"expect\": [...]} per line";

/// Reads the evaluation set at `file`, searches the raw record for each question by words, by
/// meaning and by both, and prints how often each found what the owner expected.
pub(crate) async fn run(home: Home, file: Option<String>) -> Result<(), String> {
    let file = file.ok_or(USAGE)?;
    let text = std::fs::read_to_string(&file).map_err(|error| format!("{file}: {error}"))?;
    let cases = parsed(&text)?;
    if cases.is_empty() {
        return Err(format!("{file} holds no questions. {USAGE}"));
    }
    let embedding = Settings::load(&home.settings())?.embedding(settings::environment)?;
    let memory = Memory::new(MemoryConfig {
        record: home.record(),
        index: home.search_index(),
        dictionaries: home.dictionaries(),
        embedding,
        jobs: Some(home.jobs()),
    });
    let report = memory.evaluate(cases).await?;
    print!("{}", shown(&report));
    Ok(())
}

fn parsed(text: &str) -> Result<Vec<Case>, String> {
    text.lines()
        .enumerate()
        .filter(|(_, line)| !line.trim().is_empty())
        .map(|(at, line)| {
            serde_json::from_str::<Case>(line)
                .map_err(|error| format!("line {}: {error}. {USAGE}", at + 1))
        })
        .collect()
}

fn shown(report: &Report) -> String {
    let mut out = String::new();
    let _ = writeln!(
        out,
        "Scored {} questions against the first {HITS} hits a search shows.",
        report.scored
    );
    if !report.left_out.is_empty() {
        let _ = writeln!(out, "Left out:");
        for (question, why) in &report.left_out {
            let _ = writeln!(out, "  {question} ({why})");
        }
    }
    if report.scored > 0 {
        let _ = writeln!(
            out,
            "{:<8} {:>6} {:>6} {:>6}",
            "by", "first", "shown", "MRR"
        );
        for score in &report.scores {
            let by = match score.method {
                Method::Words => "words",
                Method::Meaning => "meaning",
                Method::Both => "both",
            };
            let _ = writeln!(
                out,
                "{by:<8} {:>6} {:>6} {:>6.3}",
                score.first, score.shown, score.reciprocal
            );
        }
    }
    if let Some(why) = &report.no_meaning {
        let _ = writeln!(out, "Not scored by meaning: {why}.");
    }
    if let Some((vectored, records)) = report.vectored {
        let _ = writeln!(
            out,
            "Meaning was scored with vectors for {vectored} of {records} records with text."
        );
    }
    out
}

#[cfg(test)]
#[path = "search_eval.test.rs"]
mod tests;
