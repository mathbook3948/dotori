//! What the running daemon takes from `settings.toml` and the vault while it runs, so a change
//! in the settings window, or one another PC sent, applies without starting dotori again. The
//! model changes for the next reply, and a changed chat channel swaps into its slot under the
//! running main session. The language and the name are taken before the model is built, so the
//! model's instructions speak the language just chosen.

use std::collections::hash_map::DefaultHasher;
use std::hash::Hash;
use std::hash::Hasher;
use std::sync::Arc;
use std::sync::RwLock;
use std::time::Duration;

use dotori_connectors::channel::discord::DISCORD;
use dotori_connectors::channel::slack::SLACK;
use dotori_connectors::channel::telegram::TELEGRAM;
use dotori_core::BoxFuture;
use dotori_core::llm::CheckError;
use dotori_core::llm::Model;
use dotori_core::llm::ModelError;
use dotori_core::llm::ModelReply;
use dotori_core::llm::ModelRequest;
use tokio::time;

use crate::channel_slots::ChannelSlots;
use crate::home::Home;
use crate::language;
use crate::settings::DISCORD_KEY;
use crate::settings::SLACK_APP_KEY;
use crate::settings::SLACK_BOT_KEY;
use crate::settings::Settings;
use crate::settings::TELEGRAM_KEY;
use crate::vault;
use crate::vault::Vault;
use crate::window::identity;
use crate::window::identity::Identity;
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

struct Unbuilt(String);

impl Model for Unbuilt {
    fn complete(&self, _: ModelRequest) -> BoxFuture<'_, Result<ModelReply, ModelError>> {
        let detail = self.0.clone();
        Box::pin(async move { Err(ModelError { detail }) })
    }
}

/// What the daemon follows.
pub(crate) struct Followed {
    pub(crate) home: Home,
    pub(crate) vault: Arc<Vault>,
    pub(crate) main: Arc<LiveModel>,
    pub(crate) sub: Option<Arc<LiveModel>>,
    pub(crate) slots: Arc<ChannelSlots>,
}

/// Applies `settings.toml` and the vault at once, then looks at them every second and applies
/// what changed, until the daemon stops. A key or token the vault received counts, such as one
/// another PC sent.
pub(crate) async fn follow(followed: Followed) {
    let path = followed.home.settings();
    let mut seen = None;
    let mut prints: Vec<(&str, u64)> = Vec::new();
    loop {
        let now = {
            let path = path.clone();
            let vault = Arc::clone(&followed.vault);
            tokio::task::spawn_blocking(move || (stamp(&path), vault.changes().ok()))
                .await
                .ok()
        };
        if now.is_some() && now != seen {
            seen = now;
            take_settings(&followed.home);
            match Settings::load(&path) {
                Ok(settings) => {
                    apply(&followed, &settings);
                    prints = fill(&followed, &settings, &prints);
                }
                Err(error) => eprintln!(
                    "dotori: the changed settings do not read, so the last ones stay: {error}"
                ),
            }
        }
        time::sleep(LOOK_EVERY).await;
    }
}

/// Takes the language and the assistant's name from the settings, for the windows' words, the
/// cards and the tray of this process.
pub(crate) fn take_settings(home: &Home) {
    language::choose(language::saved(&home.settings()));
    identity::choose(Identity::read(&home.settings(), &home.icons()));
}

fn apply(followed: &Followed, settings: &Settings) {
    let built = |made: Result<Arc<dyn Model>, crate::settings::ModelError>| {
        made.unwrap_or_else(|error| {
            eprintln!("dotori: the settings do not make a model: {error}");
            Arc::new(Unbuilt(error.to_string())) as Arc<dyn Model>
        })
    };
    followed
        .main
        .swap(built(settings.model(vault::lookup(&followed.vault))));
    if let Some(sub) = &followed.sub {
        sub.swap(built(
            settings.sub_session_model(vault::lookup(&followed.vault)),
        ));
    }
}

fn fill<'a>(
    followed: &Followed,
    settings: &Settings,
    before: &[(&'a str, u64)],
) -> Vec<(&'a str, u64)> {
    let now = channels_of(settings, &followed.vault);
    let changed: Vec<&str> = now
        .iter()
        .filter(|print| !before.contains(print))
        .map(|(kind, _)| *kind)
        .collect();
    if !changed.is_empty() {
        let (chats, off) = settings.channels(vault::lookup(&followed.vault));
        for problem in off {
            eprintln!("dotori: {problem}");
        }
        followed.slots.fill(&chats, &changed);
    }
    now
}

fn channels_of(settings: &Settings, vault: &Vault) -> Vec<(&'static str, u64)> {
    let lookup = vault::lookup(vault);
    let print = |shown: String, keys: Vec<&str>| {
        let mut hasher = DefaultHasher::new();
        shown.hash(&mut hasher);
        for key in keys {
            lookup(key).hash(&mut hasher);
        }
        hasher.finish()
    };
    let slack = settings.slack.as_ref().map_or_else(Vec::new, |slack| {
        vec![
            slack.app_token_env.as_deref().unwrap_or(SLACK_APP_KEY),
            slack.bot_token_env.as_deref().unwrap_or(SLACK_BOT_KEY),
        ]
    });
    let telegram = settings
        .telegram
        .as_ref()
        .map_or_else(Vec::new, |telegram| {
            vec![telegram.token_env.as_deref().unwrap_or(TELEGRAM_KEY)]
        });
    let discord = settings.discord.as_ref().map_or_else(Vec::new, |discord| {
        vec![discord.token_env.as_deref().unwrap_or(DISCORD_KEY)]
    });
    vec![
        (SLACK, print(format!("{:?}", settings.slack), slack)),
        (
            TELEGRAM,
            print(format!("{:?}", settings.telegram), telegram),
        ),
        (DISCORD, print(format!("{:?}", settings.discord), discord)),
    ]
}

#[cfg(test)]
#[path = "live.test.rs"]
mod tests;
