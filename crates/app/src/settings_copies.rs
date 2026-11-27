//! Keeps the settings that mean the same on every PC the same on all of them: the model, Slack,
//! Telegram, Discord, memory search, and the assistant's name and icon. What belongs to one
//! machine, such as how other PCs reach it, its browser, its coding agent and its MCP servers,
//! stays with it. Each of the parts goes by its own time, and the one changed last wins. The daemon
//! follows the settings file, so a part taken from another PC applies as a change made here does.

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
use crate::settings::AssistantSettings;
use crate::settings::DiscordSettings;
use crate::settings::MemorySettings;
use crate::settings::ModelSettings;
use crate::settings::Settings;
use crate::settings::SlackSettings;
use crate::settings::TelegramSettings;

const LOOK_EVERY: Duration = Duration::from_secs(5);
const FUTURE_TAKEN: i64 = 24 * 60 * 60 * 1000;
const NEVER_STAMPED: i64 = 1;

/// The settings every PC shares.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct Shared {
    #[serde(default)]
    pub(crate) model: ModelSettings,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) slack: Option<SlackSettings>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) memory: Option<MemorySettings>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) telegram: Option<TelegramSettings>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) discord: Option<DiscordSettings>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) assistant: Option<AssistantSettings>,
}

/// When each shared part last changed, in milliseconds since the Unix epoch.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct Times {
    #[serde(default)]
    pub(crate) model: i64,
    #[serde(default)]
    pub(crate) slack: i64,
    #[serde(default)]
    pub(crate) memory: i64,
    #[serde(default)]
    pub(crate) telegram: i64,
    #[serde(default)]
    pub(crate) discord: i64,
    #[serde(default)]
    pub(crate) assistant: i64,
}

impl Times {
    fn changed_here(self) -> Self {
        let changed = |time: i64| if time > NEVER_STAMPED { time } else { 0 };
        Self {
            model: changed(self.model),
            slack: changed(self.slack),
            memory: changed(self.memory),
            telegram: changed(self.telegram),
            discord: changed(self.discord),
            assistant: changed(self.assistant),
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
struct Stamped {
    times: Times,
    shared: Shared,
}

impl Shared {
    pub(crate) fn of(settings: &Settings) -> Self {
        Self {
            model: settings.model.clone(),
            slack: settings.slack.clone(),
            memory: settings.memory.clone(),
            telegram: settings.telegram.clone(),
            discord: settings.discord.clone(),
            assistant: settings.assistant.clone(),
        }
    }

    pub(crate) fn into(self, settings: Settings) -> Settings {
        Settings {
            model: self.model,
            slack: self.slack,
            memory: self.memory,
            telegram: self.telegram,
            discord: self.discord,
            assistant: self.assistant,
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
        let mut known = self.known.lock().expect("unpoisoned");
        self.current_held(&mut known)
    }

    fn current_held(&self, known: &mut Option<Stamped>) -> Option<Stamped> {
        let shared = Shared::of(&Settings::load(&self.settings).ok()?);
        let before = known.clone().or_else(|| read_stamp(&self.stamp));
        let now = now_ms();
        let stamped = match before {
            Some(before) => Stamped {
                times: Times {
                    model: next(before.times.model, before.shared.model != shared.model, now),
                    slack: next(before.times.slack, before.shared.slack != shared.slack, now),
                    memory: next(
                        before.times.memory,
                        before.shared.memory != shared.memory,
                        now,
                    ),
                    telegram: next(
                        before.times.telegram,
                        before.shared.telegram != shared.telegram,
                        now,
                    ),
                    discord: next(
                        before.times.discord,
                        before.shared.discord != shared.discord,
                        now,
                    ),
                    assistant: next(
                        before.times.assistant,
                        before.shared.assistant != shared.assistant,
                        now,
                    ),
                },
                shared,
            },
            None => Stamped {
                times: Times {
                    model: i64::from(shared.model != ModelSettings::default()),
                    slack: i64::from(shared.slack.is_some()),
                    memory: i64::from(shared.memory.is_some()),
                    telegram: i64::from(shared.telegram.is_some()),
                    discord: i64::from(shared.discord.is_some()),
                    assistant: i64::from(shared.assistant.is_some()),
                },
                shared,
            },
        };
        if known.as_ref() != Some(&stamped) {
            write_stamp(&self.stamp, &stamped);
            *known = Some(stamped.clone());
        }
        Some(stamped)
    }

    /// Takes the parts of another PC's settings that changed later than these. A PC `joining`
    /// the others before its first run is done also takes the parts it never changed here, whatever
    /// their times, but keeps what its owner set since. Says whether any changed.
    ///
    /// The whole copy wins, not each change, on purpose. dotori serves one person, who changes
    /// these from one PC at a time, so two PCs changing them in the same moment does not happen
    /// in practice. Merging change by change would add a log and conflict rules for a case that
    /// does not occur.
    pub(crate) fn take(&self, times: Times, shared: Shared, joining: bool) -> Result<bool, String> {
        let mut known = self.known.lock().expect("unpoisoned");
        let Some(here) = self.current_held(&mut known) else {
            return Err(String::from("the settings here cannot be read"));
        };
        let mine = if joining {
            here.times.changed_here()
        } else {
            here.times
        };
        let mut taken = here.clone();
        if times.model > mine.model && shared.model != here.shared.model {
            taken.times.model = times.model;
            taken.shared.model = shared.model;
        }
        if times.slack > mine.slack && shared.slack != here.shared.slack {
            taken.times.slack = times.slack;
            taken.shared.slack = shared.slack;
        }
        if times.memory > mine.memory && shared.memory != here.shared.memory {
            taken.times.memory = times.memory;
            taken.shared.memory = shared.memory;
        }
        if times.telegram > mine.telegram && shared.telegram != here.shared.telegram {
            taken.times.telegram = times.telegram;
            taken.shared.telegram = shared.telegram;
        }
        if times.discord > mine.discord && shared.discord != here.shared.discord {
            taken.times.discord = times.discord;
            taken.shared.discord = shared.discord;
        }
        if times.assistant > mine.assistant && shared.assistant != here.shared.assistant {
            taken.times.assistant = times.assistant;
            taken.shared.assistant = shared.assistant;
        }
        if taken == here {
            return Ok(false);
        }
        let settings = taken.shared.clone().into(Settings::load(&self.settings)?);
        settings.save_parts(
            &self.settings,
            &[
                "model",
                "slack",
                "memory",
                "telegram",
                "discord",
                "assistant",
            ],
        )?;
        write_stamp(&self.stamp, &taken);
        *known = Some(taken);
        Ok(true)
    }

    fn message(&self) -> Option<Value> {
        let stamped = self.current()?;
        Some(json!({
            "kind": "settings",
            "times": stamped.times,
            "shared": stamped.shared,
        }))
    }

    /// Sends this PC's settings to a PC that just linked.
    pub(crate) fn hear(&self, linked: &Linked, heard: &Heard) {
        if heard.message.get("kind").and_then(Value::as_str) == Some("linked")
            && let Some(message) = self.message()
        {
            linked.hub.send(&heard.key, message);
        }
    }

    /// Takes the settings another PC sent, if `heard` is them, as `take` does. Says whether it
    /// was.
    pub(crate) fn take_heard(&self, heard: &Heard, joining: bool) -> bool {
        if heard.message.get("kind").and_then(Value::as_str) != Some("settings") {
            return false;
        }
        let times = heard
            .message
            .get("times")
            .and_then(|times| serde_json::from_value::<Times>(times.clone()).ok());
        let shared = heard
            .message
            .get("shared")
            .and_then(|shared| serde_json::from_value::<Shared>(shared.clone()).ok());
        let (Some(times), Some(shared)) = (times, shared) else {
            eprintln!(
                "dotori: settings from {} do not read here; is it a newer dotori?",
                heard.device
            );
            return true;
        };
        let latest = now_ms().saturating_add(FUTURE_TAKEN);
        if [
            times.model,
            times.slack,
            times.memory,
            times.telegram,
            times.discord,
            times.assistant,
        ]
        .iter()
        .any(|time| *time > latest)
        {
            eprintln!(
                "dotori: settings from {} are dated too far ahead",
                heard.device
            );
            return true;
        }
        match self.take(times, shared, joining) {
            Ok(true) => eprintln!(
                "dotori: took newer settings from {}; they take effect when dotori starts again",
                heard.device
            ),
            Ok(false) => {}
            Err(error) => eprintln!(
                "dotori: settings from {} were not taken: {error}",
                heard.device
            ),
        }
        true
    }

    /// Looks at the settings every few seconds and sends them out when they changed here.
    pub(crate) async fn share(&self, linked: Linked) {
        let mut sent = self.current().map(|stamped| stamped.times);
        let mut every = time::interval(LOOK_EVERY);
        loop {
            every.tick().await;
            let Some(stamped) = self.current() else {
                continue;
            };
            if sent != Some(stamped.times)
                && let Some(message) = self.message()
            {
                linked.hub.broadcast(&message);
                sent = Some(stamped.times);
            }
        }
    }
}

fn next(before: i64, changed: bool, now: i64) -> i64 {
    if changed {
        now.max(before.saturating_add(1))
    } else {
        before
    }
}

fn read_stamp(path: &Path) -> Option<Stamped> {
    serde_json::from_str(&std::fs::read_to_string(path).ok()?).ok()
}

fn write_stamp(path: &Path, stamped: &Stamped) {
    let Ok(text) = serde_json::to_string(stamped) else {
        return;
    };
    let draft = path.with_extension("stamp.new");
    if let Err(error) = std::fs::write(&draft, text).and_then(|()| std::fs::rename(&draft, path)) {
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
