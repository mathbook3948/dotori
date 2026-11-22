use std::borrow::Cow;
use std::path::Path;

use lindera::dictionary::load_dictionary;
use lindera::mode::Mode;
use lindera::segmenter::Segmenter;

const PARTICLES: &[&str] = &[
    "에서는",
    "에게서",
    "으로는",
    "한테서",
    "에서",
    "에게",
    "한테",
    "으로",
    "까지",
    "부터",
    "처럼",
    "보다",
    "이랑",
    "은",
    "는",
    "이",
    "가",
    "을",
    "를",
    "에",
    "의",
    "도",
    "로",
    "와",
    "과",
    "랑",
    "만",
    "께",
];

const LIGHT: &[&str] = &["하", "되", "이", "있"];

const KEPT: &[&str] = &["NN", "NP", "VV", "VA", "MAG", "XR", "SL", "SH", "SN"];

/// Turns text into the terms the index stores: the base forms of content words, so particles and
/// endings do not split one word into many.
#[derive(Default)]
pub(crate) enum Analyzer {
    Korean(Box<Segmenter>),
    #[default]
    Plain,
}

impl Analyzer {
    pub(crate) fn korean(dictionary: &Path) -> Result<Self, String> {
        let dictionary = load_dictionary(&dictionary.display().to_string())
            .map_err(|error| format!("cannot load the dictionary: {error}"))?;
        Ok(Self::Korean(Box::new(Segmenter::new(
            Mode::Normal,
            dictionary,
            None,
        ))))
    }

    /// Which analyzer made an index, so a different one rebuilds it.
    pub(crate) fn name(&self) -> &'static str {
        match self {
            Self::Korean(_) => "ko-dic",
            Self::Plain => "plain",
        }
    }

    pub(crate) fn terms(&self, text: &str) -> Vec<String> {
        match self {
            Self::Korean(segmenter) => match segmenter.segment(Cow::Borrowed(text)) {
                Ok(mut tokens) => tokens
                    .iter_mut()
                    .filter_map(|token| {
                        let surface = token.surface.to_string();
                        let details: Vec<String> = token
                            .details()
                            .iter()
                            .map(|part| part.to_string())
                            .collect();
                        lemma(&surface, &details)
                    })
                    .collect(),
                Err(_) => plain(text),
            },
            Self::Plain => plain(text),
        }
    }
}

fn lemma(surface: &str, details: &[String]) -> Option<String> {
    let tag = details.first()?.split('+').next()?;
    if !KEPT.iter().any(|kept| tag.starts_with(kept)) {
        return None;
    }
    let base = match (details.get(4), details.get(7)) {
        (Some(kind), Some(expression)) if kind == "Inflect" => {
            expression.split('/').next().unwrap_or(surface)
        }
        _ => details
            .get(3)
            .filter(|reading| *reading != "*" && !reading.is_empty())
            .map_or(surface, String::as_str),
    };
    let base = base.to_lowercase();
    if tag.starts_with('V') && LIGHT.contains(&base.as_str()) {
        return None;
    }
    (!base.trim().is_empty()).then_some(base)
}

fn plain(text: &str) -> Vec<String> {
    text.split(|c: char| !c.is_alphanumeric())
        .filter(|word| !word.is_empty())
        .map(|word| {
            let word = word.to_lowercase();
            PARTICLES
                .iter()
                .find_map(|particle| {
                    word.strip_suffix(particle).filter(|stem| {
                        stem.chars().count() >= 2 && (hangul(stem) || stem.is_ascii())
                    })
                })
                .map_or(word.clone(), str::to_owned)
        })
        .collect()
}

fn hangul(text: &str) -> bool {
    text.chars().all(|c| ('가'..='힣').contains(&c))
}

#[cfg(test)]
#[path = "analyzer.test.rs"]
mod tests;
