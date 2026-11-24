//! Discord through the Gateway, so no public address is needed. The assistant hears the owner's
//! direct messages to the bot and the owner's mentions of it in servers, and answers each where it
//! was said. Nobody else is heard.

use std::collections::HashMap;
use std::collections::HashSet;
use std::fmt;
use std::mem;
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
use dotori_core::message::Audience;
use dotori_core::message::Content;
use dotori_core::message::Incoming;
use dotori_core::message::Place;
use futures_util::SinkExt;
use futures_util::StreamExt;
use reqwest::Client;
use reqwest::Method;
use reqwest::StatusCode;
use reqwest::header::AUTHORIZATION;
use reqwest::header::CONTENT_TYPE;
use reqwest::header::RETRY_AFTER;
use serde_json::Value;
use serde_json::json;
use tokio::time;
use tokio::time::MissedTickBehavior;
use tokio_tungstenite::tungstenite::Message;

/// The channel id of Discord.
pub const DISCORD: &str = "discord";
/// Where the HTTP API lives.
pub const API_BASE: &str = "https://discord.com/api/v10";
/// The place in Discord that only the owner reads: the owner's direct messages with the bot.
pub const OWNER_DIRECT: &str = "@owner";
const INTENTS: u64 = (1 << 0) | (1 << 9) | (1 << 12) | (1 << 15);
const TIMEOUT: Duration = Duration::from_secs(30);
const CONNECT_WAIT: Duration = Duration::from_secs(30);
const FIRST_WAIT: Duration = Duration::from_secs(1);
const MAX_WAIT: Duration = Duration::from_secs(60);
const STEADY: Duration = Duration::from_secs(30);
const PIECE: usize = 2000;
const SERVER_WAIT: Duration = Duration::from_secs(5);
const SERVER_FRESH: Duration = Duration::from_secs(10 * 60);
const SERVERS_KEPT: usize = 256;
const SEND_TRIES: usize = 4;
const UNCOUNTED: &str = "this Discord server, whose members could not be counted";

/// What the Discord channel needs. The owner is the one Discord user id whose messages are
/// heard.
#[derive(Clone)]
pub struct DiscordConfig {
    pub token: String,
    pub owner: String,
    pub api_base: String,
}

impl DiscordConfig {
    pub fn new(token: impl Into<String>, owner: impl Into<String>) -> Self {
        Self {
            token: token.into(),
            owner: owner.into(),
            api_base: String::from(API_BASE),
        }
    }
}

impl fmt::Debug for DiscordConfig {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("DiscordConfig")
            .field("token", &"<hidden>")
            .field("owner", &self.owner)
            .finish_non_exhaustive()
    }
}

/// The Discord channel.
pub struct DiscordChannel {
    id: ChannelId,
    owner: String,
    token: String,
    base: String,
    client: Client,
    direct: Mutex<Option<String>>,
    servers: Mutex<HashMap<String, (Instant, Audience)>>,
}

impl fmt::Debug for DiscordChannel {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("DiscordChannel").finish_non_exhaustive()
    }
}

#[derive(Debug)]
enum Failed {
    Fatal(String),
    Passing(String, Option<Duration>),
}

enum Ended {
    Again,
    InboxClosed,
}

#[derive(Clone, Debug, Default)]
struct Session {
    id: String,
    url: String,
    seq: Option<u64>,
}

/// Who the bot is: its user id and, in each server, the role Discord made for it, which the
/// owner may mention in its place.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct Bot {
    pub(crate) id: String,
    pub(crate) roles: HashSet<String>,
}

/// One message the owner sent the bot.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Arrived {
    pub(crate) id: String,
    pub(crate) place: String,
    pub(crate) server: Option<String>,
    pub(crate) name: String,
    pub(crate) text: String,
}

impl DiscordChannel {
    pub fn new(config: DiscordConfig) -> Result<Self, ChannelError> {
        let client = Client::builder().build().map_err(|error| ChannelError {
            detail: error.to_string(),
        })?;
        Ok(Self {
            id: ChannelId::parse(DISCORD).map_err(|error| ChannelError {
                detail: error.to_string(),
            })?,
            owner: config.owner,
            token: config.token,
            base: config.api_base.trim_end_matches('/').to_owned(),
            client,
            direct: Mutex::new(None),
            servers: Mutex::new(HashMap::new()),
        })
    }

    async fn call(&self, method: Method, path: &str, body: Option<Value>) -> Result<Value, Failed> {
        let mut request = self
            .client
            .request(method, format!("{}{path}", self.base))
            .timeout(TIMEOUT)
            .header(AUTHORIZATION, format!("Bot {}", self.token));
        if let Some(body) = body {
            request = request
                .header(CONTENT_TYPE, "application/json")
                .body(body.to_string());
        }
        let response = request.send().await.map_err(|error| {
            Failed::Passing(format!("{path} unreachable: {}", error.without_url()), None)
        })?;
        let status = response.status();
        let waited = response
            .headers()
            .get(RETRY_AFTER)
            .and_then(|after| after.to_str().ok())
            .and_then(|after| after.trim().parse::<f64>().ok())
            .filter(|seconds| seconds.is_finite() && *seconds >= 0.0)
            .map(Duration::from_secs_f64);
        let text = response.text().await.map_err(|error| {
            Failed::Passing(
                format!("{path} answered oddly: {}", error.without_url()),
                None,
            )
        })?;
        let reply: Value = if text.trim().is_empty() {
            Value::Null
        } else {
            match serde_json::from_str(&text) {
                Ok(reply) => reply,
                Err(_) if !status.is_success() => Value::Null,
                Err(error) => {
                    return Err(Failed::Passing(
                        format!("{path} answered oddly: {error}"),
                        None,
                    ));
                }
            }
        };
        if status.is_success() {
            return Ok(reply);
        }
        let detail = format!(
            "{path} failed ({status}): {}",
            reply
                .get("message")
                .and_then(Value::as_str)
                .unwrap_or("no reason given")
        );
        if status == StatusCode::UNAUTHORIZED {
            return Err(Failed::Fatal(detail));
        }
        let after = reply
            .get("retry_after")
            .and_then(Value::as_f64)
            .filter(|seconds| seconds.is_finite() && *seconds >= 0.0)
            .map(Duration::from_secs_f64)
            .or(waited);
        Err(Failed::Passing(detail, after))
    }

    async fn listen(&self, inbox: Arc<dyn Inbox>) -> Result<(), ChannelError> {
        let mut wait = FIRST_WAIT;
        let mut session: Option<Session> = None;
        let mut bot = Bot::default();
        loop {
            let started = Instant::now();
            match self.session(&mut session, &mut bot, inbox.as_ref()).await {
                Ok(Ended::InboxClosed) => return Ok(()),
                Err(Failed::Fatal(detail)) => return Err(ChannelError { detail }),
                Ok(Ended::Again) => {
                    if started.elapsed() >= STEADY {
                        wait = FIRST_WAIT;
                    }
                    wait = pause(wait, None).await;
                }
                Err(Failed::Passing(detail, after)) => {
                    if wait == FIRST_WAIT {
                        eprintln!("dotori: Discord: {detail}");
                    }
                    if started.elapsed() >= STEADY {
                        wait = FIRST_WAIT;
                    }
                    wait = pause(wait, after).await;
                }
            }
        }
    }

    async fn session(
        &self,
        session: &mut Option<Session>,
        bot: &mut Bot,
        inbox: &dyn Inbox,
    ) -> Result<Ended, Failed> {
        let url = match session {
            Some(resumed) => resumed.url.clone(),
            None => self
                .call(Method::GET, "/gateway/bot", None)
                .await?
                .get("url")
                .and_then(Value::as_str)
                .ok_or_else(|| Failed::Passing(String::from("no gateway address"), None))?
                .to_owned(),
        };
        let address = format!("{}/?v=10&encoding=json", url.trim_end_matches('/'));
        let (mut socket, _) =
            time::timeout(CONNECT_WAIT, tokio_tungstenite::connect_async(address))
                .await
                .map_err(|_| Failed::Passing(String::from("gateway did not open in time"), None))?
                .map_err(|error| Failed::Passing(format!("gateway unreachable: {error}"), None))?;
        let gone = |error: tokio_tungstenite::tungstenite::Error| {
            Failed::Passing(format!("gateway failed: {error}"), None)
        };
        let hello = loop {
            let frame = time::timeout(CONNECT_WAIT, socket.next())
                .await
                .map_err(|_| Failed::Passing(String::from("gateway said no hello"), None))?
                .ok_or_else(|| Failed::Passing(String::from("gateway closed"), None))?
                .map_err(gone)?;
            if let Message::Text(text) = frame {
                let hello: Value = serde_json::from_str(text.as_str()).unwrap_or(Value::Null);
                if hello.get("op").and_then(Value::as_u64) == Some(10) {
                    break hello;
                }
            }
        };
        let every = hello
            .pointer("/d/heartbeat_interval")
            .and_then(Value::as_u64)
            .filter(|every| *every > 0)
            .map_or(Duration::from_secs(40), Duration::from_millis);
        let opening = match session {
            Some(resumed) => json!({ "op": 6, "d": {
                "token": self.token,
                "session_id": resumed.id,
                "seq": resumed.seq,
            } }),
            None => json!({ "op": 2, "d": {
                "token": self.token,
                "intents": INTENTS,
                "properties": { "os": std::env::consts::OS, "browser": "dotori", "device": "dotori" },
            } }),
        };
        socket
            .send(Message::text(opening.to_string()))
            .await
            .map_err(gone)?;
        let mut beat = time::interval_at(time::Instant::now() + every / 2, every);
        beat.set_missed_tick_behavior(MissedTickBehavior::Delay);
        let mut answered = true;
        loop {
            let frame = tokio::select! {
                _ = beat.tick() => {
                    if !answered {
                        return Ok(Ended::Again);
                    }
                    answered = false;
                    let seq = session.as_ref().and_then(|now| now.seq);
                    socket
                        .send(Message::text(json!({ "op": 1, "d": seq }).to_string()))
                        .await
                        .map_err(gone)?;
                    continue;
                }
                frame = socket.next() => frame,
            };
            let frame = match frame {
                Some(Ok(frame)) => frame,
                Some(Err(error)) => return Err(gone(error)),
                None => return Ok(Ended::Again),
            };
            let text = match frame {
                Message::Text(text) => text,
                Message::Close(close) => {
                    let code = close.map_or(0, |close| u16::from(close.code));
                    return closed(code, session);
                }
                _ => continue,
            };
            let event: Value = serde_json::from_str(text.as_str()).unwrap_or(Value::Null);
            if let Some(seq) = event.get("s").and_then(Value::as_u64)
                && let Some(now) = session.as_mut()
            {
                now.seq = Some(seq);
            }
            match event.get("op").and_then(Value::as_u64) {
                Some(11) => answered = true,
                Some(1) => {
                    let seq = session.as_ref().and_then(|now| now.seq);
                    socket
                        .send(Message::text(json!({ "op": 1, "d": seq }).to_string()))
                        .await
                        .map_err(gone)?;
                }
                Some(7) => return Ok(Ended::Again),
                Some(9) => {
                    if event.get("d").and_then(Value::as_bool) != Some(true) {
                        *session = None;
                    }
                    return Ok(Ended::Again);
                }
                Some(0) => {}
                _ => continue,
            }
            let data = event.get("d").unwrap_or(&Value::Null);
            match event.get("t").and_then(Value::as_str) {
                Some("READY") => {
                    bot.id = data
                        .pointer("/user/id")
                        .and_then(Value::as_str)
                        .unwrap_or_default()
                        .to_owned();
                    *session = Some(Session {
                        id: data
                            .get("session_id")
                            .and_then(Value::as_str)
                            .unwrap_or_default()
                            .to_owned(),
                        url: data
                            .get("resume_gateway_url")
                            .and_then(Value::as_str)
                            .unwrap_or(&url)
                            .to_owned(),
                        seq: event.get("s").and_then(Value::as_u64),
                    });
                }
                Some("GUILD_CREATE" | "GUILD_ROLE_CREATE" | "GUILD_ROLE_UPDATE") => {
                    let roles = data
                        .get("roles")
                        .and_then(Value::as_array)
                        .cloned()
                        .unwrap_or_default()
                        .into_iter()
                        .chain(data.get("role").cloned());
                    for role in roles {
                        if role.pointer("/tags/bot_id").and_then(Value::as_str) == Some(&bot.id)
                            && let Some(id) = role.get("id").and_then(Value::as_str)
                        {
                            bot.roles.insert(id.to_owned());
                        }
                    }
                }
                Some("MESSAGE_CREATE") => {
                    let Some(arrived) = read(data, &self.owner, bot) else {
                        continue;
                    };
                    let Ok(content) = Content::parse(arrived.text) else {
                        continue;
                    };
                    let audience = match &arrived.server {
                        Some(server) => self.audience(server).await,
                        None => Audience::Owner,
                    };
                    let incoming = Incoming {
                        id: ExternalMessageId::new(arrived.id),
                        place: Place::new(arrived.place),
                        sender_id: ExternalUserId::new(self.owner.clone()),
                        sender_name: arrived.name,
                        content,
                        audience,
                    };
                    if inbox.push(incoming).await.is_err() {
                        return Ok(Ended::InboxClosed);
                    }
                }
                _ => {}
            }
        }
    }

    async fn audience(&self, server: &str) -> Audience {
        let cached = self
            .servers
            .lock()
            .expect("unpoisoned")
            .get(server)
            .filter(|(at, _)| at.elapsed() < SERVER_FRESH)
            .map(|(_, audience)| audience.clone());
        if let Some(audience) = cached {
            return audience;
        }
        let path = format!("/guilds/{server}?with_counts=true");
        let counted = time::timeout(SERVER_WAIT, self.call(Method::GET, &path, None))
            .await
            .ok()
            .and_then(Result::ok)
            .and_then(|found| {
                found
                    .get("approximate_member_count")
                    .and_then(Value::as_u64)
            });
        let Some(count) = counted else {
            return Audience::Anyone(String::from(UNCOUNTED));
        };
        let audience = Audience::Anyone(format!(
            "this Discord server, which has about {count} members"
        ));
        let mut servers = self.servers.lock().expect("unpoisoned");
        if servers.len() >= SERVERS_KEPT {
            servers.clear();
        }
        servers.insert(server.to_owned(), (Instant::now(), audience.clone()));
        audience
    }

    async fn direct_channel(&self) -> Result<String, Failed> {
        if let Some(known) = self.direct.lock().expect("unpoisoned").clone() {
            return Ok(known);
        }
        let opened = self
            .call(
                Method::POST,
                "/users/@me/channels",
                Some(json!({ "recipient_id": self.owner })),
            )
            .await?;
        let id = opened
            .get("id")
            .and_then(Value::as_str)
            .ok_or_else(|| Failed::Passing(String::from("no direct message channel"), None))?
            .to_owned();
        *self.direct.lock().expect("unpoisoned") = Some(id.clone());
        Ok(id)
    }

    async fn send(&self, channel: &str, body: Value) -> Result<(), Failed> {
        let path = format!("/channels/{channel}/messages");
        let mut tries = 1;
        loop {
            match self.call(Method::POST, &path, Some(body.clone())).await {
                Err(Failed::Passing(_, Some(after))) if after <= MAX_WAIT && tries < SEND_TRIES => {
                    tries += 1;
                    time::sleep(after).await;
                }
                done => return done.map(drop),
            }
        }
    }
}

fn closed(code: u16, session: &mut Option<Session>) -> Result<Ended, Failed> {
    match code {
        4004 => Err(Failed::Fatal(String::from(
            "Discord did not accept the bot token",
        ))),
        4014 => Err(Failed::Fatal(String::from(
            "turn on Message Content Intent for the bot in the Discord Developer Portal",
        ))),
        4010..=4013 => Err(Failed::Fatal(format!(
            "Discord refused the connection ({code})"
        ))),
        4007 | 4009 => {
            *session = None;
            Ok(Ended::Again)
        }
        _ => Ok(Ended::Again),
    }
}

async fn pause(wait: Duration, after: Option<Duration>) -> Duration {
    time::sleep(after.unwrap_or(wait).min(MAX_WAIT)).await;
    (wait * 2).min(MAX_WAIT)
}

/// The owner's message in a `MESSAGE_CREATE`, if it is one the assistant should hear: any text
/// in a direct message, and in a server only text that mentions the bot, or its role, or answers
/// it. Forwarded text, webhooks and other bots are not heard.
pub(crate) fn read(message: &Value, owner: &str, bot: &Bot) -> Option<Arrived> {
    let author = message.get("author")?;
    if author.get("id").and_then(Value::as_str)? != owner
        || author.get("bot").and_then(Value::as_bool) == Some(true)
        || message
            .get("webhook_id")
            .is_some_and(|hook| !hook.is_null())
        || message.get("message_snapshots").is_some()
        || !matches!(message.get("type").and_then(Value::as_u64), Some(0 | 19))
    {
        return None;
    }
    let text = message.get("content").and_then(Value::as_str)?;
    let channel = message.get("channel_id").and_then(Value::as_str)?;
    let id = message.get("id").and_then(Value::as_str)?;
    let name = [
        message.pointer("/member/nick"),
        author.get("global_name"),
        author.get("username"),
    ]
    .into_iter()
    .flatten()
    .filter_map(Value::as_str)
    .find(|name| !name.is_empty())
    .unwrap_or(owner)
    .to_owned();
    let server = message
        .get("guild_id")
        .and_then(Value::as_str)
        .map(str::to_owned);
    let Some(server) = server else {
        return Some(Arrived {
            id: format!("{channel}:{id}"),
            place: channel.to_owned(),
            server: None,
            name,
            text: text.to_owned(),
        });
    };
    if bot.id.is_empty() {
        return None;
    }
    let listed = |field: &str, wanted: &dyn Fn(&Value) -> bool| {
        message
            .get(field)
            .and_then(Value::as_array)
            .is_some_and(|listed| listed.iter().any(wanted))
    };
    let named = listed("mentions", &|user| {
        user.get("id").and_then(Value::as_str) == Some(bot.id.as_str())
    }) || listed("mention_roles", &|role| {
        role.as_str().is_some_and(|role| bot.roles.contains(role))
    });
    let (written, rest) = without_mention(text, bot);
    let mentioned = named || written;
    let answered = message
        .pointer("/referenced_message/author/id")
        .and_then(Value::as_str)
        == Some(bot.id.as_str());
    if !mentioned && !answered {
        return None;
    }
    Some(Arrived {
        id: format!("{channel}:{id}"),
        place: format!("{channel}/{id}"),
        server: Some(server),
        name,
        text: rest,
    })
}

fn without_mention(text: &str, bot: &Bot) -> (bool, String) {
    let mut found = false;
    let mut kept = text.to_owned();
    let tokens = [format!("<@{}>", bot.id), format!("<@!{}>", bot.id)]
        .into_iter()
        .chain(bot.roles.iter().map(|role| format!("<@&{role}>")));
    for token in tokens {
        if kept.contains(&token) {
            found = true;
            kept = kept.replace(&token, " ");
        }
    }
    (found, kept.split_whitespace().collect::<Vec<_>>().join(" "))
}

/// The channel a place means, and the message to answer there, if any. `None` for the channel
/// means the owner's direct messages.
pub(crate) fn target(place: &str) -> Option<(Option<String>, Option<String>)> {
    if place == OWNER_DIRECT {
        return Some((None, None));
    }
    let snowflake = |text: &str| !text.is_empty() && text.bytes().all(|byte| byte.is_ascii_digit());
    match place.split_once('/') {
        Some((channel, message)) if snowflake(channel) && snowflake(message) => {
            Some((Some(channel.to_owned()), Some(message.to_owned())))
        }
        None if snowflake(place) => Some((Some(place.to_owned()), None)),
        _ => None,
    }
}

/// Splits `text` into messages Discord takes, at line breaks where it can. Blank text gives no
/// message.
pub(crate) fn pieces(text: &str) -> Vec<String> {
    let count = |text: &str| text.chars().count();
    let mut pieces = Vec::new();
    let mut current = String::new();
    for line in text.split_inclusive('\n') {
        if count(&current) + count(line) > PIECE && !current.is_empty() {
            pieces.push(mem::take(&mut current));
        }
        let mut rest = line;
        while count(rest) > PIECE {
            let cut = rest
                .char_indices()
                .nth(PIECE)
                .map_or(rest.len(), |(at, _)| at);
            pieces.push(rest[..cut].to_owned());
            rest = &rest[cut..];
        }
        current.push_str(rest);
    }
    pieces.push(current);
    pieces.retain(|piece| !piece.trim().is_empty());
    pieces
}

impl Channel for DiscordChannel {
    fn id(&self) -> &ChannelId {
        &self.id
    }

    fn run(&self, inbox: Arc<dyn Inbox>) -> BoxFuture<'_, Result<(), ChannelError>> {
        Box::pin(self.listen(inbox))
    }

    fn private_place(&self) -> Place {
        Place::new(OWNER_DIRECT)
    }

    fn deliver(&self, place: Place, text: String) -> BoxFuture<'_, Result<(), ChannelError>> {
        Box::pin(async move {
            let failed = |failed: Failed| ChannelError {
                detail: match failed {
                    Failed::Fatal(detail) | Failed::Passing(detail, _) => detail,
                },
            };
            let (channel, answering) = target(place.as_str()).ok_or_else(|| ChannelError {
                detail: format!("{} is not a Discord channel", place.as_str()),
            })?;
            let channel = match channel {
                Some(channel) => channel,
                None => self.direct_channel().await.map_err(failed)?,
            };
            for (n, piece) in pieces(&text).into_iter().enumerate() {
                let mut body = json!({
                    "content": piece,
                    "allowed_mentions": { "parse": [] },
                    "flags": 1 << 2,
                });
                if n == 0
                    && let Some(message) = &answering
                {
                    body["message_reference"] =
                        json!({ "message_id": message, "fail_if_not_exists": false });
                }
                self.send(&channel, body).await.map_err(failed)?;
            }
            Ok(())
        })
    }
}

#[cfg(test)]
#[path = "discord.test.rs"]
mod tests;
