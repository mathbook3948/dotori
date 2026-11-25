use std::time::Duration;

use reqwest::Client;
use reqwest::RequestBuilder;
use reqwest::StatusCode;
use reqwest::header::CONTENT_TYPE;
use reqwest::header::RETRY_AFTER;
use serde::Deserialize;
use serde_json::Value;
use serde_json::json;

const TIMEOUT: Duration = Duration::from_secs(30);
const ATTEMPTS: u32 = 3;
const MEMBERS_PAGE: usize = 200;
const FIRST_WAIT: Duration = Duration::from_secs(1);
const MAX_WAIT: Duration = Duration::from_secs(60);
const FATAL: [&str; 7] = [
    "invalid_auth",
    "not_authed",
    "account_inactive",
    "token_revoked",
    "token_expired",
    "missing_scope",
    "not_allowed_token_type",
];

/// Why a Slack call failed. A fatal failure will not pass by trying again.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum ApiError {
    Fatal(String),
    Passing(String),
}

impl ApiError {
    pub(crate) fn detail(self) -> String {
        match self {
            Self::Fatal(detail) | Self::Passing(detail) => detail,
        }
    }
}

/// The Slack Web API, with the app token for Socket Mode and the bot token for everything else.
#[derive(Clone)]
pub(crate) struct Api {
    client: Client,
    base: String,
    app_token: String,
    bot_token: String,
    attempts: u32,
}

/// What a Slack conversation's info says about who can read it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Room {
    pub(crate) public: bool,
    pub(crate) members: Option<usize>,
}

#[derive(Deserialize)]
struct Reply {
    ok: bool,
    error: Option<String>,
    #[serde(flatten)]
    rest: Value,
}

impl Api {
    pub(crate) fn new(base: String, app_token: String, bot_token: String) -> Result<Self, String> {
        let client = Client::builder()
            .timeout(TIMEOUT)
            .build()
            .map_err(|error| error.to_string())?;
        Ok(Self {
            client,
            base: base.trim_end_matches('/').to_owned(),
            app_token,
            bot_token,
            attempts: ATTEMPTS,
        })
    }

    /// The same API, trying each call once.
    pub(crate) fn once(&self) -> Self {
        Self {
            attempts: 1,
            ..self.clone()
        }
    }

    /// The assistant's own user id, so its own messages are not read as someone else's.
    pub(crate) async fn bot_user(&self) -> Result<String, ApiError> {
        let reply = self
            .call("auth.test", || {
                self.post("auth.test").bearer_auth(&self.bot_token)
            })
            .await?;
        text_at(&reply, "/user_id", "auth.test")
    }

    /// A fresh Socket Mode address. Each one can be used once.
    pub(crate) async fn socket_url(&self) -> Result<String, ApiError> {
        let reply = self
            .call("apps.connections.open", || {
                self.post("apps.connections.open")
                    .bearer_auth(&self.app_token)
            })
            .await?;
        text_at(&reply, "/url", "apps.connections.open")
    }

    /// The name people see for `user`.
    pub(crate) async fn user_name(&self, user: &str) -> Result<String, ApiError> {
        let url = format!("{}/users.info?user={user}", self.base);
        let reply = self
            .call("users.info", || {
                self.client.get(&url).bearer_auth(&self.bot_token)
            })
            .await?;
        [
            "/user/profile/display_name",
            "/user/real_name",
            "/user/name",
        ]
        .iter()
        .find_map(|pointer| {
            reply
                .pointer(pointer)
                .and_then(Value::as_str)
                .filter(|name| !name.is_empty())
        })
        .map(str::to_owned)
        .ok_or_else(|| ApiError::Passing(String::from("users.info gave no name")))
    }

    /// Whether anyone in the workspace can read `channel`, and how many members it has.
    pub(crate) async fn room(&self, channel: &str) -> Result<Room, ApiError> {
        let url = format!(
            "{}/conversations.info?channel={channel}&include_num_members=true",
            self.base
        );
        let reply = self
            .call("conversations.info", || {
                self.client.get(&url).bearer_auth(&self.bot_token)
            })
            .await?;
        let flag = |name: &str| {
            reply
                .pointer(&format!("/channel/{name}"))
                .and_then(Value::as_bool)
                .unwrap_or(false)
        };
        Ok(Room {
            public: flag("is_channel") && !flag("is_private"),
            members: reply
                .pointer("/channel/num_members")
                .and_then(Value::as_u64)
                .and_then(|count| usize::try_from(count).ok()),
        })
    }

    /// The first page of `channel`'s member ids.
    pub(crate) async fn members(&self, channel: &str) -> Result<Vec<String>, ApiError> {
        let url = format!(
            "{}/conversations.members?channel={channel}&limit={MEMBERS_PAGE}",
            self.base
        );
        let reply = self
            .call("conversations.members", || {
                self.client.get(&url).bearer_auth(&self.bot_token)
            })
            .await?;
        Ok(reply
            .pointer("/members")
            .and_then(Value::as_array)
            .map(|members| {
                members
                    .iter()
                    .filter_map(Value::as_str)
                    .map(str::to_owned)
                    .collect()
            })
            .unwrap_or_default())
    }

    /// The direct message conversation between the assistant and `user`.
    pub(crate) async fn direct(&self, user: &str) -> Result<String, ApiError> {
        let body = json!({ "users": user }).to_string();
        let reply = self
            .call("conversations.open", || {
                self.post("conversations.open")
                    .bearer_auth(&self.bot_token)
                    .header(CONTENT_TYPE, "application/json; charset=utf-8")
                    .body(body.clone())
            })
            .await?;
        text_at(&reply, "/channel/id", "conversations.open")
    }

    pub(crate) async fn say(
        &self,
        channel: &str,
        thread: Option<&str>,
        text: &str,
    ) -> Result<(), ApiError> {
        let mut body = json!({ "channel": channel, "text": text });
        if let Some(thread) = thread {
            body["thread_ts"] = json!(thread);
        }
        self.send("chat.postMessage", &body).await.map(drop)
    }

    /// Posts `blocks`, with `text` for notifications, and says the conversation and the
    /// message's timestamp.
    pub(crate) async fn say_blocks(
        &self,
        channel: &str,
        thread: Option<&str>,
        text: &str,
        blocks: Value,
    ) -> Result<(String, String), ApiError> {
        let mut body = json!({ "channel": channel, "text": text, "blocks": blocks });
        if let Some(thread) = thread {
            body["thread_ts"] = json!(thread);
        }
        let reply = self.send("chat.postMessage", &body).await?;
        Ok((
            text_at(&reply, "/channel", "chat.postMessage")?,
            text_at(&reply, "/ts", "chat.postMessage")?,
        ))
    }

    /// Replaces the message at `ts` in `channel` with `text` and `blocks`.
    pub(crate) async fn update(
        &self,
        channel: &str,
        ts: &str,
        text: &str,
        blocks: Value,
    ) -> Result<(), ApiError> {
        let body = json!({ "channel": channel, "ts": ts, "text": text, "blocks": blocks });
        self.send("chat.update", &body).await.map(drop)
    }

    /// Shows `text` in `channel` to `user` alone.
    pub(crate) async fn whisper(
        &self,
        channel: &str,
        user: &str,
        thread: Option<&str>,
        text: &str,
    ) -> Result<(), ApiError> {
        let mut body = json!({ "channel": channel, "user": user, "text": text });
        if let Some(thread) = thread {
            body["thread_ts"] = json!(thread);
        }
        self.send("chat.postEphemeral", &body).await.map(drop)
    }

    async fn send(&self, method: &str, body: &Value) -> Result<Value, ApiError> {
        let body = body.to_string();
        self.call(method, || {
            self.post(method)
                .bearer_auth(&self.bot_token)
                .header(CONTENT_TYPE, "application/json; charset=utf-8")
                .body(body.clone())
        })
        .await
    }

    fn post(&self, method: &str) -> RequestBuilder {
        self.client.post(format!("{}/{method}", self.base))
    }

    async fn call(
        &self,
        method: &str,
        request: impl Fn() -> RequestBuilder,
    ) -> Result<Value, ApiError> {
        let mut wait = FIRST_WAIT;
        let mut attempt = 1;
        loop {
            let (detail, retry_after) = match request().send().await {
                Ok(response) => {
                    let status = response.status();
                    let retry_after = response
                        .headers()
                        .get(RETRY_AFTER)
                        .and_then(|value| value.to_str().ok())
                        .and_then(|value| value.parse::<u64>().ok())
                        .map(Duration::from_secs);
                    if status == StatusCode::TOO_MANY_REQUESTS || status.is_server_error() {
                        (format!("{method} answered {status}"), retry_after)
                    } else {
                        return read_reply(method, status, response.text().await);
                    }
                }
                Err(error) if error.is_connect() => {
                    (format!("{method} unreachable: {error}"), None)
                }
                Err(error) => return Err(ApiError::Passing(format!("{method} failed: {error}"))),
            };
            if attempt >= self.attempts {
                return Err(ApiError::Passing(detail));
            }
            tokio::time::sleep(retry_after.unwrap_or(wait).min(MAX_WAIT)).await;
            wait = (wait * 2).min(MAX_WAIT);
            attempt += 1;
        }
    }
}

fn read_reply(
    method: &str,
    status: StatusCode,
    body: Result<String, reqwest::Error>,
) -> Result<Value, ApiError> {
    let body =
        body.map_err(|error| ApiError::Passing(format!("{method} reply unreadable: {error}")))?;
    let reply = serde_json::from_str::<Reply>(&body)
        .map_err(|_| ApiError::Passing(format!("{method} answered {status} without a reply")))?;
    if reply.ok {
        return Ok(reply.rest);
    }
    let error = reply.error.unwrap_or_else(|| String::from("unknown_error"));
    let detail = format!("{method} failed: {error}");
    if FATAL.contains(&error.as_str()) {
        Err(ApiError::Fatal(detail))
    } else {
        Err(ApiError::Passing(detail))
    }
}

fn text_at(reply: &Value, pointer: &str, method: &str) -> Result<String, ApiError> {
    reply
        .pointer(pointer)
        .and_then(Value::as_str)
        .map(str::to_owned)
        .ok_or_else(|| ApiError::Passing(format!("{method} gave no {}", &pointer[1..])))
}
