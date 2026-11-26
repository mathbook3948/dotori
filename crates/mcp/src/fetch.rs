use std::net::IpAddr;
use std::net::Ipv4Addr;
use std::net::SocketAddr;
use std::time::Duration;

use reqwest::Client;
use reqwest::Url;
use reqwest::dns::Addrs;
use reqwest::dns::Name;
use reqwest::dns::Resolve;
use reqwest::dns::Resolving;
use reqwest::header::CONTENT_TYPE;
use reqwest::redirect;
use serde_json::Value;
use serde_json::json;

use crate::Tool;
use crate::ToolFuture;

const TIMEOUT: Duration = Duration::from_secs(30);
const MAX_DOWNLOAD: usize = 5 * 1024 * 1024;
const MAX_REDIRECTS: usize = 5;
const DEFAULT_LENGTH: usize = 20_000;
const MAX_LENGTH: usize = 200_000;
const AGENT: &str = concat!("dotori/", env!("CARGO_PKG_VERSION"));

/// Reads a web page as Markdown. It reaches only public addresses, so a page cannot steer it
/// into this machine or the local network.
#[derive(Debug)]
pub struct WebFetch {
    client: Client,
}

impl WebFetch {
    pub fn new() -> Result<Self, String> {
        let mut builder = Client::builder().timeout(TIMEOUT).user_agent(AGENT);
        if !proxied() {
            builder = builder.dns_resolver(PublicOnly);
        }
        let client = builder
            .redirect(redirect::Policy::custom(|attempt| {
                if attempt.previous().len() >= MAX_REDIRECTS {
                    attempt.error("too many redirects")
                } else if let Err(error) = allowed(attempt.url()) {
                    attempt.error(error)
                } else {
                    attempt.follow()
                }
            }))
            .build()
            .map_err(|error| error.to_string())?;
        Ok(Self { client })
    }

    async fn fetch(&self, arguments: Value) -> Result<String, String> {
        let url = arguments
            .get("url")
            .and_then(Value::as_str)
            .ok_or("`url` is required")?;
        let url = Url::parse(url).map_err(|error| format!("not a URL: {error}"))?;
        allowed(&url)?;
        let start = number(&arguments, "start_index").unwrap_or(0);
        let length = number(&arguments, "max_length")
            .unwrap_or(DEFAULT_LENGTH)
            .clamp(1, MAX_LENGTH);
        let raw = arguments
            .get("raw")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        let mut response = self
            .client
            .get(url)
            .send()
            .await
            .map_err(|error| format!("could not fetch: {}", chain(&error)))?;
        let status = response.status();
        let kind = response
            .headers()
            .get(CONTENT_TYPE)
            .and_then(|value| value.to_str().ok())
            .unwrap_or_default()
            .to_ascii_lowercase();
        let mut body = Vec::new();
        while let Some(chunk) = response
            .chunk()
            .await
            .map_err(|error| format!("could not read: {}", chain(&error)))?
        {
            if body.len() + chunk.len() > MAX_DOWNLOAD {
                return Err(format!("the page is larger than {MAX_DOWNLOAD} bytes"));
            }
            body.extend_from_slice(&chunk);
        }
        let text = tokio::task::spawn_blocking(move || read(&kind, &body, raw))
            .await
            .map_err(|error| error.to_string())??;
        let page = window(&text, start, length);
        if status.is_success() {
            Ok(page)
        } else {
            Err(format!("the server answered {status}\n\n{page}"))
        }
    }
}

impl Tool for WebFetch {
    fn name(&self) -> &str {
        "fetch_url"
    }

    fn description(&self) -> &str {
        "Fetches a public web page that needs no login and returns it as Markdown (or as raw \
         text with `raw`). It is quicker than the browser. Long pages come in parts: call again \
         with the `start_index` the reply names."
    }

    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "url": { "type": "string", "description": "An http or https URL." },
                "max_length": { "type": "integer", "minimum": 1, "description": "Characters to return. Default 20000." },
                "start_index": { "type": "integer", "minimum": 0, "description": "Character to start from. Default 0." },
                "raw": { "type": "boolean", "description": "Return the page as fetched, without converting HTML." }
            },
            "required": ["url"]
        })
    }

    fn call(&self, arguments: Value) -> ToolFuture<'_> {
        Box::pin(self.fetch(arguments))
    }
}

/// Only http and https to a host that is not written as a private address.
pub(crate) fn allowed(url: &Url) -> Result<(), String> {
    if !matches!(url.scheme(), "http" | "https") {
        return Err(format!("`{}` URLs are not fetched", url.scheme()));
    }
    let host = url.host_str().ok_or("the URL has no host")?;
    let bare = host.trim_start_matches('[').trim_end_matches(']');
    if let Ok(address) = bare.parse::<IpAddr>()
        && !public(address)
    {
        return Err(format!("{host} is not a public address"));
    }
    let name = host.trim_end_matches('.').to_ascii_lowercase();
    if name == "localhost" || name.ends_with(".localhost") {
        return Err(format!("{host} is this machine"));
    }
    Ok(())
}

/// Whether an address is on the public internet.
pub(crate) fn public(address: IpAddr) -> bool {
    match address {
        IpAddr::V4(v4) => {
            let [a, b, ..] = v4.octets();
            !(v4.is_private()
                || v4.is_loopback()
                || v4.is_link_local()
                || v4.is_unspecified()
                || v4.is_broadcast()
                || v4.is_documentation()
                || v4.is_multicast()
                || a == 0
                || (a == 100 && (64..128).contains(&b))
                || (a == 198 && (b == 18 || b == 19))
                || (a == 192 && b == 0 && v4.octets()[2] == 0)
                || a >= 240)
        }
        IpAddr::V6(v6) => {
            if let Some(v4) = v6.to_ipv4_mapped() {
                return public(IpAddr::V4(v4));
            }
            let segments = v6.segments();
            let first = segments[0];
            if first == 0x2002 {
                let [a, b] = segments[1].to_be_bytes();
                let [c, d] = segments[2].to_be_bytes();
                return public(IpAddr::V4(Ipv4Addr::new(a, b, c, d)));
            }
            !(v6.is_loopback()
                || v6.is_unspecified()
                || v6.is_multicast()
                || segments[..6].iter().all(|segment| *segment == 0)
                || (first == 0x64 && segments[1] == 0xff9b)
                || (first == 0x2001 && segments[1] == 0)
                || (first == 0x2001 && segments[1] == 0xdb8)
                || (first & 0xfe00) == 0xfc00
                || (first & 0xffc0) == 0xfe80
                || (first & 0xffc0) == 0xfec0)
        }
    }
}

/// The page as the model reads it: HTML as Markdown, other text as it is.
pub(crate) fn read(kind: &str, body: &[u8], raw: bool) -> Result<String, String> {
    let text = String::from_utf8_lossy(body);
    if !raw && (kind.contains("html") || (kind.is_empty() && looks_like_html(&text))) {
        htmd::HtmlToMarkdown::builder()
            .skip_tags(vec![
                "script", "style", "noscript", "template", "svg", "head",
            ])
            .build()
            .convert(&text)
            .map_err(|error| format!("could not read the page: {error}"))
    } else if raw || readable(kind) {
        Ok(text.into_owned())
    } else {
        Err(format!("the page is `{kind}`, not text"))
    }
}

/// `max_length` characters from `start`, with a note on where the rest begins.
pub(crate) fn window(text: &str, start: usize, length: usize) -> String {
    let total = text.chars().count();
    if start >= total {
        return format!("[nothing at start_index {start}; the page has {total} characters]");
    }
    let end = start.saturating_add(length).min(total);
    let part: String = text.chars().skip(start).take(end - start).collect();
    if end < total {
        format!(
            "{part}\n\n[the page goes on: {} more characters, call again with start_index {end}]",
            total - end
        )
    } else {
        part
    }
}

/// With a proxy the proxy looks names up, so only addresses written in the URL are checked here.
pub(crate) fn proxied() -> bool {
    [
        "HTTPS_PROXY",
        "https_proxy",
        "HTTP_PROXY",
        "http_proxy",
        "ALL_PROXY",
        "all_proxy",
    ]
    .iter()
    .any(|name| std::env::var_os(name).is_some_and(|value| !value.is_empty()))
}

fn number(arguments: &Value, key: &str) -> Option<usize> {
    arguments
        .get(key)
        .and_then(Value::as_u64)
        .and_then(|value| usize::try_from(value).ok())
}

fn readable(kind: &str) -> bool {
    kind.is_empty()
        || kind.starts_with("text/")
        || kind.contains("json")
        || kind.contains("xml")
        || kind.contains("javascript")
}

fn looks_like_html(text: &str) -> bool {
    let start: String = text
        .trim_start()
        .chars()
        .take(100)
        .collect::<String>()
        .to_ascii_lowercase();
    start.starts_with("<!doctype html") || start.starts_with("<html")
}

fn chain(error: &dyn std::error::Error) -> String {
    let mut text = error.to_string();
    let mut source = error.source();
    while let Some(cause) = source {
        text.push_str(": ");
        text.push_str(&cause.to_string());
        source = cause.source();
    }
    text
}

struct PublicOnly;

impl Resolve for PublicOnly {
    fn resolve(&self, name: Name) -> Resolving {
        Box::pin(async move {
            let host = name.as_str().to_owned();
            let found = tokio::net::lookup_host((host.as_str(), 0)).await?;
            let addresses: Vec<SocketAddr> = found.filter(|address| public(address.ip())).collect();
            if addresses.is_empty() {
                return Err(format!("{host} has no public address").into());
            }
            let addresses: Addrs = Box::new(addresses.into_iter());
            Ok(addresses)
        })
    }
}

#[cfg(test)]
#[path = "fetch.test.rs"]
mod tests;
