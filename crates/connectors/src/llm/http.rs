use std::error::Error;
use std::time::Duration;

use dotori_core::llm::ModelError;
use reqwest::Client;
use reqwest::RequestBuilder;
use reqwest::StatusCode;
use reqwest::header::RETRY_AFTER;
use serde::Deserialize;

/// How long one request may take. Replies are not streamed, so a long reply waits this long.
pub(crate) const REQUEST_TIMEOUT: Duration = Duration::from_secs(600);

/// How often a request that failed for a passing reason is tried, and how long to wait between.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Retry {
    pub(crate) attempts: u32,
    pub(crate) first_wait: Duration,
    pub(crate) max_wait: Duration,
}

const BODY_SHOWN: usize = 300;

pub(crate) const RETRY: Retry = Retry {
    attempts: 3,
    first_wait: Duration::from_secs(1),
    max_wait: Duration::from_secs(20),
};

#[derive(Deserialize)]
struct ErrorBody {
    error: ErrorDetail,
}

#[derive(Deserialize)]
struct ErrorDetail {
    #[serde(rename = "type")]
    kind: Option<String>,
    message: Option<String>,
}

pub(crate) fn client() -> Result<Client, String> {
    Client::builder()
        .timeout(REQUEST_TIMEOUT)
        .build()
        .map_err(|error| chain(&error))
}

/// Sends the request, retrying overload and server errors, and returns the body of a success.
pub(crate) async fn send(
    provider: &str,
    retry: Retry,
    request: impl Fn() -> RequestBuilder,
) -> Result<String, ModelError> {
    let mut wait = retry.first_wait;
    let mut attempt = 1;
    loop {
        let failure = match request().send().await {
            Ok(response) => {
                let status = response.status();
                let retry_after = response
                    .headers()
                    .get(RETRY_AFTER)
                    .and_then(|value| value.to_str().ok())
                    .and_then(|value| value.parse::<u64>().ok())
                    .map(Duration::from_secs);
                let body = response.text().await.map_err(|error| ModelError {
                    detail: format!("{provider} reply unreadable: {}", chain(&error)),
                })?;
                if status.is_success() {
                    return Ok(body);
                }
                let detail = status_detail(provider, status, &body);
                if !passing(status) {
                    return Err(ModelError { detail });
                }
                (detail, retry_after)
            }
            Err(error) if error.is_connect() || error.is_timeout() => {
                (format!("{provider} unreachable: {}", chain(&error)), None)
            }
            Err(error) => {
                return Err(ModelError {
                    detail: format!("{provider} request failed: {}", chain(&error)),
                });
            }
        };
        if attempt >= retry.attempts {
            return Err(ModelError {
                detail: format!("{} after {attempt} attempts", failure.0),
            });
        }
        tokio::time::sleep(failure.1.unwrap_or(wait).min(retry.max_wait)).await;
        wait = (wait * 2).min(retry.max_wait);
        attempt += 1;
    }
}

fn passing(status: StatusCode) -> bool {
    matches!(status.as_u16(), 408 | 409 | 429 | 500..=599)
}

fn status_detail(provider: &str, status: StatusCode, body: &str) -> String {
    match serde_json::from_str::<ErrorBody>(body) {
        Ok(ErrorBody { error }) => format!(
            "{provider} answered {status}: {} {}",
            error.kind.unwrap_or_default(),
            error.message.unwrap_or_default()
        )
        .trim_end()
        .to_owned(),
        Err(_) => {
            let body: String = body.trim().chars().take(BODY_SHOWN).collect();
            format!("{provider} answered {status}: {body}")
        }
    }
}

fn chain(error: &dyn Error) -> String {
    let mut text = error.to_string();
    let mut source = error.source();
    while let Some(cause) = source {
        text.push_str(&format!(": {cause}"));
        source = cause.source();
    }
    text
}

#[cfg(test)]
#[path = "http.test.rs"]
mod tests;
