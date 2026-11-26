//! How far the owner got through first run, kept in the dotori home so it outlives a restart.
//! The daemon waits until first run is done, and the onboarding window opens again at the step
//! the owner had not finished.

use std::fs;
use std::fs::OpenOptions;
use std::io::ErrorKind;
use std::path::Path;

use crate::replace::replace;

/// A screen of the onboarding window, in the order the owner walks them.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Step {
    Welcome,
    Key,
    Look,
    Elsewhere,
}

impl Step {
    pub(crate) const ALL: [Step; 4] = [Step::Welcome, Step::Key, Step::Look, Step::Elsewhere];

    pub(crate) fn position(self) -> usize {
        Self::ALL
            .iter()
            .position(|step| *step == self)
            .expect("every step is in the table")
    }

    pub(crate) fn next(self) -> Option<Step> {
        Self::ALL.get(self.position() + 1).copied()
    }

    pub(crate) fn previous(self) -> Option<Step> {
        self.position()
            .checked_sub(1)
            .map(|position| Self::ALL[position])
    }

    fn code(self) -> &'static str {
        match self {
            Step::Welcome => "welcome",
            Step::Key => "key",
            Step::Look => "look",
            Step::Elsewhere => "elsewhere",
        }
    }
}

/// How far first run is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum FirstRun {
    /// Nothing is written, as on an install from before first run was kept.
    Unbegun,
    /// The owner reached this step and has not finished it.
    At(Step),
    Done,
}

const DONE: &str = "done";

/// What `path` keeps. A file that cannot be read or makes no sense starts over at the greeting.
pub(crate) fn read(path: &Path) -> FirstRun {
    match fs::read_to_string(path) {
        Ok(text) => {
            let text = text.trim();
            if text == DONE {
                return FirstRun::Done;
            }
            FirstRun::At(
                Step::ALL
                    .into_iter()
                    .find(|step| step.code() == text)
                    .unwrap_or(Step::Welcome),
            )
        }
        Err(error) if error.kind() == ErrorKind::NotFound => FirstRun::Unbegun,
        Err(_) => FirstRun::At(Step::Welcome),
    }
}

/// Keeps `state` at `path`. Nothing marks an unbegun first run, so it writes nothing for one.
pub(crate) fn write(path: &Path, state: FirstRun) -> Result<(), String> {
    let text = match state {
        FirstRun::Unbegun => return Ok(()),
        FirstRun::At(step) => step.code(),
        FirstRun::Done => DONE,
    };
    replace(path, format!("{text}\n").as_bytes(), OpenOptions::new())
}

#[cfg(test)]
#[path = "first_run.test.rs"]
mod tests;
