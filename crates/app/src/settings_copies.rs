//! Keeps the settings that mean the same on every PC the same on all of them: the model, Slack
//! and memory search. What belongs to one machine, such as how other PCs reach it, its browser,
//! its coding agent and its MCP servers, stays with it. The settings changed last win, and
//! like any setting they take effect when dotori starts again.

use std::path::Path;
use std::path::PathBuf;
use std::sync::Mutex;
use std::time::Duration;
use std::time::SystemTime;
use std::time::UNIX_EPOCH;

use dotori_connectors::peer::hub::Heard;
use serde::Deserialize;
use serde::Serialize;
use serde_json::Value;
use serde_json::json;
use tokio::time;

use crate::main_session::Linked;
use crate::settings::MemorySettings;
use crate::settings::ModelSettings;
use crate::settings::Settings;
use crate::settings::SlackSettings;

const LOOK_EVERY: Duration = Duration::from_secs(5);

/// The settings every PC shares, and when they last changed.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct Shared {
    #[serde(default)]
    pub(crate) model: ModelSettings,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) slack: Option<SlackSettings>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) memory: Option<MemorySettings>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
struct Stamped {
    changed_ms: i64,
    shared: Shared,
}

impl Shared {
    pub(crate) fn of(settings: &Settings) -> Self {
        Self {
            model: settings.model.clone(),
            slack: settings.slack.clone(),
            memory: settings.memory.clone(),
        }
    }

    pub(crate) fn into(self, settings: Settings) -> Settings {
        Settings {
            model: self.model,
            slack: self.slack,
            memory: self.memory,
            ..settings
        }
    }
}

/// Shares this PC's settings and takes in newer ones from the others.
#[derive(Debug)]
pub(crate) struct SettingsCopies {
    settings: PathBuf,
    stamp: PathBuf,
    known: Mutex<Option<Stamped>>,
}

impl SettingsCopies {
    pub(crate) fn new(settings: PathBuf, stamp: PathBuf) -> Self {
        Self {
            settings,
            stamp,
            known: Mutex::new(None),
        }
    }

    fn current(&self) -> Option<Stamped> {
        let shared = Shared::of(&Settings::load(&self.settings).ok()?);
        let mut known = self.known.lock().expect("unpoisoned");
        let stamped = match known.as_ref() {
            Some(known) if known.shared == shared => known.clone(),
            Some(_) => Stamped {
                changed_ms: now_ms(),
                shared,
            },
            None => match read_stamp(&self.stamp) {
                Some(kept) if kept.shared == shared => kept,
                Some(kept) => Stamped {
                    changed_ms: now_ms().max(kept.changed_ms + 1),
                    shared,
                },
                None => Stamped {
                    changed_ms: 0,
                    shared,
                },
            },
        };
        if known.as_ref() != Some(&stamped) {
            write_stamp(&self.stamp, &stamped);
            *known = Some(stamped.clone());
        }
        Some(stamped)
    }

    /// Takes settings another PC sent, if they changed later than these. Says whether they did.
    pub(crate) fn take(&self, changed_ms: i64, shared: Shared) -> Result<bool, String> {
        let Some(here) = self.current() else {
            return Err(String::from("the settings here cannot be read"));
        };
        if changed_ms <= here.changed_ms || shared == here.shared {
            return Ok(false);
        }
        let settings = shared.clone().into(Settings::load(&self.settings)?);
        settings.save(&self.settings)?;
        let stamped = Stamped { changed_ms, shared };
        write_stamp(&self.stamp, &stamped);
        *self.known.lock().expect("unpoisoned") = Some(stamped);
        Ok(true)
    }

    fn message(&self) -> Option<Value> {
        let stamped = self.current()?;
        Some(json!({
            "kind": "settings",
            "changed_ms": stamped.changed_ms,
            "shared": stamped.shared,
        }))
    }

    /// Takes a message about settings from another PC. Says whether it was one.
    pub(crate) fn hear(&self, linked: &Linked, heard: &Heard) -> bool {
        match heard.message.get("kind").and_then(Value::as_str) {
            Some("linked") => {
                if let Some(message) = self.message() {
                    linked.hub.send(&heard.key, message);
                }
                false
            }
            Some("settings") => {
                let changed_ms = heard
                    .message
                    .get("changed_ms")
                    .and_then(Value::as_i64)
                    .unwrap_or(0)
                    .min(now_ms());
                let shared = heard
                    .message
                    .get("shared")
                    .and_then(|shared| serde_json::from_value::<Shared>(shared.clone()).ok());
                match shared.map(|shared| self.take(changed_ms, shared)) {
                    Some(Ok(true)) => eprintln!(
                        "dotori: took newer settings from {}; they take effect when dotori starts again",
                        heard.device
                    ),
                    Some(Err(error)) => eprintln!(
                        "dotori: settings from {} were not taken: {error}",
                        heard.device
                    ),
                    _ => {}
                }
                true
            }
            _ => false,
        }
    }

    /// Looks at the settings every few seconds and sends them out when they changed here.
    pub(crate) async fn share(&self, linked: Linked) {
        let mut sent = self.current().map(|stamped| stamped.changed_ms);
        let mut every = time::interval(LOOK_EVERY);
        loop {
            every.tick().await;
            let Some(stamped) = self.current() else {
                continue;
            };
            if sent != Some(stamped.changed_ms)
                && let Some(message) = self.message()
            {
                linked.hub.broadcast(&message);
                sent = Some(stamped.changed_ms);
            }
        }
    }
}

fn read_stamp(path: &Path) -> Option<Stamped> {
    serde_json::from_str(&std::fs::read_to_string(path).ok()?).ok()
}

fn write_stamp(path: &Path, stamped: &Stamped) {
    if let Ok(text) = serde_json::to_string(stamped)
        && let Err(error) = std::fs::write(path, text)
    {
        eprintln!("dotori: {}: {error}", path.display());
    }
}

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|since| i64::try_from(since.as_millis()).unwrap_or(i64::MAX))
        .unwrap_or(0)
}

#[cfg(test)]
#[path = "settings_copies.test.rs"]
mod tests;
