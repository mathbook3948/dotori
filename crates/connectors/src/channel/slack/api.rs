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
pub(crate) struct Api {
    client: Client,
    base: String,
    app_token: String,
    bot_token: String,
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
        })
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
        let body = body.to_string();
        self.call("chat.postMessage", || {
            self.post("chat.postMessage")
                .bearer_auth(&self.bot_token)
                .header(CONTENT_TYPE, "application/json; charset=utf-8")
                .body(body.clone())
        })
        .await
        .map(drop)
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
            if attempt >= ATTEMPTS {
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
