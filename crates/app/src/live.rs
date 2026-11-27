//! What the running daemon takes from `settings.toml` and the vault while it runs, so a change
//! in the settings window, or one another PC sent, applies without starting dotori again. The
//! model changes for the next reply, and a change to the chat channels starts the main session
//! again with them. The name and the language are followed from the start, in the daemon.

use std::collections::hash_map::DefaultHasher;
use std::hash::Hash;
use std::hash::Hasher;
use std::sync::Arc;
use std::sync::RwLock;
use std::time::Duration;

use dotori_core::BoxFuture;
use dotori_core::llm::CheckError;
use dotori_core::llm::Model;
use dotori_core::llm::ModelError;
use dotori_core::llm::ModelReply;
use dotori_core::llm::ModelRequest;
use tokio::sync::Notify;
use tokio::time;

use crate::home::Home;
use crate::settings::DISCORD_KEY;
use crate::settings::SLACK_APP_KEY;
use crate::settings::SLACK_BOT_KEY;
use crate::settings::Settings;
use crate::settings::TELEGRAM_KEY;
use crate::vault;
use crate::vault::Vault;
use crate::window::stamp;

const LOOK_EVERY: Duration = Duration::from_secs(1);

/// A model that can be swapped while the session runs. A request already sent finishes on the
/// model it started with.
pub(crate) struct LiveModel {
    now: RwLock<Arc<dyn Model>>,
}

impl LiveModel {
    pub(crate) fn new(model: Arc<dyn Model>) -> Arc<Self> {
        Arc::new(Self {
            now: RwLock::new(model),
        })
    }

    fn now(&self) -> Arc<dyn Model> {
        Arc::clone(&self.now.read().expect("unpoisoned"))
    }

    fn swap(&self, model: Arc<dyn Model>) {
        *self.now.write().expect("unpoisoned") = model;
    }
}

impl Model for LiveModel {
    fn complete(&self, request: ModelRequest) -> BoxFuture<'_, Result<ModelReply, ModelError>> {
        let model = self.now();
        Box::pin(async move { model.complete(request).await })
    }

    fn check(&self) -> BoxFuture<'_, Result<(), CheckError>> {
        let model = self.now();
        Box::pin(async move { model.check().await })
    }
}

/// What the daemon follows, and where it tells the main loop that the channels changed.
pub(crate) struct Followed {
    pub(crate) home: Home,
    pub(crate) vault: Arc<Vault>,
    pub(crate) main: Arc<LiveModel>,
    pub(crate) sub: Option<Arc<LiveModel>>,
    pub(crate) channels_changed: Arc<Notify>,
}

/// Looks at `settings.toml` and the vault every second and applies what changed, until the
/// daemon stops. A key or token the vault received counts, such as one another PC sent.
pub(crate) async fn follow(followed: Followed, started_with: &Settings) {
    let path = followed.home.settings();
    let looked = || (stamp(&path), followed.vault.changes().ok());
    let mut seen = looked();
    let mut channels = channels_of(started_with, &followed.vault);
    loop {
        time::sleep(LOOK_EVERY).await;
        let now = looked();
        if now == seen {
            continue;
        }
        seen = now;
        let settings = match Settings::load(&path) {
            Ok(settings) => settings,
            Err(error) => {
                eprintln!(
                    "dotori: the changed settings do not read, so the last ones stay: {error}"
                );
                continue;
            }
        };
        apply(&followed, &settings);
        let changed = channels_of(&settings, &followed.vault);
        if changed != channels {
            channels = changed;
            followed.channels_changed.notify_one();
        }
    }
}

fn apply(followed: &Followed, settings: &Settings) {
    match settings.model(vault::lookup(&followed.vault)) {
        Ok(model) => followed.main.swap(model),
        Err(error) => eprintln!("dotori: the model stays as it was: {error}"),
    }
    if let Some(sub) = &followed.sub {
        match settings.sub_session_model(vault::lookup(&followed.vault)) {
            Ok(model) => sub.swap(model),
            Err(error) => eprintln!("dotori: the sub-session model stays as it was: {error}"),
        }
    }
}

fn channels_of(settings: &Settings, vault: &Vault) -> u64 {
    let lookup = vault::lookup(vault);
    let mut hasher = DefaultHasher::new();
    format!(
        "{:?}",
        (&settings.slack, &settings.telegram, &settings.discord)
    )
    .hash(&mut hasher);
    let mut names = Vec::new();
    if let Some(slack) = &settings.slack {
        names.push(slack.app_token_env.as_deref().unwrap_or(SLACK_APP_KEY));
        names.push(slack.bot_token_env.as_deref().unwrap_or(SLACK_BOT_KEY));
    }
    if let Some(telegram) = &settings.telegram {
        names.push(telegram.token_env.as_deref().unwrap_or(TELEGRAM_KEY));
    }
    if let Some(discord) = &settings.discord {
        names.push(discord.token_env.as_deref().unwrap_or(DISCORD_KEY));
    }
    for name in names {
        lookup(name).hash(&mut hasher);
    }
    hasher.finish()
}

#[cfg(test)]
#[path = "live.test.rs"]
mod tests;
