//! Slack over Socket Mode, so no public address is needed. The assistant hears direct messages
//! and mentions, and answers a mention in its thread.

mod api;
mod envelope;

use std::collections::HashMap;
use std::fmt;
use std::sync::Arc;
use std::sync::Mutex;
use std::time::Duration;
use std::time::Instant;

use dotori_core::BoxFuture;
use dotori_core::channel::Channel;
use dotori_core::channel::ChannelError;
use dotori_core::channel::Inbox;
use dotori_core::id::ChannelId;
use dotori_core::id::ExternalMessageId;
use dotori_core::id::ExternalUserId;
use dotori_core::message::Content;
use dotori_core::message::Incoming;
use dotori_core::message::Place;
use futures_util::SinkExt;
use futures_util::StreamExt;
use tokio::time;
use tokio_tungstenite::tungstenite::Message;

use crate::channel::slack::api::Api;
use crate::channel::slack::api::ApiError;
use crate::channel::slack::envelope::Kind;

/// The channel id of Slack.
pub const SLACK: &str = "slack";
/// Where the Web API lives.
pub const API_BASE: &str = "https://slack.com/api";
const FIRST_WAIT: Duration = Duration::from_secs(1);
const MAX_WAIT: Duration = Duration::from_secs(60);
const STEADY: Duration = Duration::from_secs(30);
const CONNECT_WAIT: Duration = Duration::from_secs(30);
const QUIET: Duration = Duration::from_secs(75);
const NAME_WAIT: Duration = Duration::from_secs(2);
const NAMES_KEPT: usize = 4096;

/// What the Slack channel needs. The app token opens Socket Mode and the bot token speaks.
#[derive(Clone)]
pub struct SlackConfig {
    pub app_token: String,
    pub bot_token: String,
    pub api_base: String,
}

impl SlackConfig {
    pub fn new(app_token: impl Into<String>, bot_token: impl Into<String>) -> Self {
        Self {
            app_token: app_token.into(),
            bot_token: bot_token.into(),
            api_base: String::from(API_BASE),
        }
    }
}

impl fmt::Debug for SlackConfig {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SlackConfig")
            .field("api_base", &self.api_base)
            .finish_non_exhaustive()
    }
}

/// The Slack channel. It reconnects by itself and ends only when a token stops working.
pub struct SlackChannel {
    id: ChannelId,
    api: Api,
    names: Mutex<HashMap<String, String>>,
}

impl fmt::Debug for SlackChannel {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SlackChannel").finish_non_exhaustive()
    }
}

enum Ended {
    InboxClosed,
    Again,
}

impl SlackChannel {
    pub fn new(config: SlackConfig) -> Result<Self, ChannelError> {
        let api = Api::new(config.api_base, config.app_token, config.bot_token)
            .map_err(|detail| ChannelError { detail })?;
        Ok(Self {
            id: ChannelId::parse(SLACK).map_err(|error| ChannelError {
                detail: error.to_string(),
            })?,
            api,
            names: Mutex::new(HashMap::new()),
        })
    }

    async fn listen(&self, inbox: Arc<dyn Inbox>) -> Result<(), ChannelError> {
        let mut wait = FIRST_WAIT;
        let bot = loop {
            match self.api.bot_user().await {
                Ok(bot) => break bot,
                Err(ApiError::Fatal(detail)) => return Err(ChannelError { detail }),
                Err(ApiError::Passing(_)) => wait = pause(wait).await,
            }
        };
        wait = FIRST_WAIT;
        loop {
            let started = Instant::now();
            match self.session(&bot, inbox.as_ref()).await {
                Ok(Ended::InboxClosed) => return Ok(()),
                Err(ApiError::Fatal(detail)) => return Err(ChannelError { detail }),
                Ok(Ended::Again) | Err(ApiError::Passing(_)) => {
                    if started.elapsed() >= STEADY {
                        wait = FIRST_WAIT;
                    }
                    wait = pause(wait).await;
                }
            }
        }
    }

    async fn session(&self, bot: &str, inbox: &dyn Inbox) -> Result<Ended, ApiError> {
        let url = self.api.socket_url().await?;
        let (mut socket, _) =
            time::timeout(CONNECT_WAIT, tokio_tungstenite::connect_async(url.as_str()))
                .await
                .map_err(|_| ApiError::Passing(String::from("socket did not open in time")))?
                .map_err(|error| ApiError::Passing(format!("socket unreachable: {error}")))?;
        loop {
            let frame = time::timeout(QUIET, socket.next())
                .await
                .map_err(|_| ApiError::Passing(String::from("socket went quiet")))?
                .ok_or_else(|| ApiError::Passing(String::from("socket closed")))?
                .map_err(|error| ApiError::Passing(format!("socket failed: {error}")))?;
            let Message::Text(text) = frame else {
                continue;
            };
            let frame = envelope::read(text.as_str(), bot);
            if let Some(ack) = frame.ack {
                let ack = serde_json::json!({ "envelope_id": ack }).to_string();
                socket
                    .send(Message::text(ack))
                    .await
                    .map_err(|error| ApiError::Passing(format!("socket failed: {error}")))?;
            }
            match frame.kind {
                Kind::Hello | Kind::Nothing => {}
                Kind::Disconnect => return Ok(Ended::Again),
                Kind::Message(arrived) => {
                    let Ok(content) = Content::parse(arrived.text) else {
                        continue;
                    };
                    let incoming = Incoming {
                        id: ExternalMessageId::new(arrived.id),
                        place: Place::new(arrived.place),
                        sender_name: self.name(&arrived.user).await,
                        sender_id: ExternalUserId::new(arrived.user),
                        content,
                    };
                    if inbox.push(incoming).await.is_err() {
                        return Ok(Ended::InboxClosed);
                    }
                }
            }
        }
    }

    async fn name(&self, user: &str) -> String {
        if let Some(name) = self.names.lock().expect("unpoisoned").get(user) {
            return name.clone();
        }
        let name = match time::timeout(NAME_WAIT, self.api.user_name(user)).await {
            Ok(Ok(name)) => name,
            _ => user.to_owned(),
        };
        let mut names = self.names.lock().expect("unpoisoned");
        if names.len() >= NAMES_KEPT {
            names.clear();
        }
        names.insert(user.to_owned(), name.clone());
        name
    }
}

async fn pause(wait: Duration) -> Duration {
    time::sleep(wait).await;
    (wait * 2).min(MAX_WAIT)
}

impl Channel for SlackChannel {
    fn id(&self) -> &ChannelId {
        &self.id
    }

    fn run(&self, inbox: Arc<dyn Inbox>) -> BoxFuture<'_, Result<(), ChannelError>> {
        Box::pin(self.listen(inbox))
    }

    fn deliver(&self, place: Place, text: String) -> BoxFuture<'_, Result<(), ChannelError>> {
        Box::pin(async move {
            let (channel, thread) = envelope::target(place.as_str());
            for piece in envelope::pieces(&text) {
                self.api
                    .say(channel, thread, &envelope::escaped(piece))
                    .await
                    .map_err(|error| ChannelError {
                        detail: error.detail(),
                    })?;
            }
            Ok(())
        })
    }
}

#[cfg(test)]
#[path = "slack.test.rs"]
mod tests;
