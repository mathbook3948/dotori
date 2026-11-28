use std::fmt;

use dotori_core::BoxFuture;
use dotori_core::carried::Room;
use dotori_core::llm::CheckError;
use dotori_core::llm::Model;
use dotori_core::llm::ModelError;
use dotori_core::llm::ModelReply;
use dotori_core::llm::ModelRequest;
use dotori_core::llm::ModelTrace;
use dotori_core::llm::SummaryRequest;
use reqwest::Client;
use serde::Deserialize;
use serde_json::Value;
use serde_json::json;
use serde_json::value::RawValue;

use crate::llm::ConnectorError;
use crate::llm::DEFAULT_SYSTEM;
use crate::llm::SUMMARY_SYSTEM;
use crate::llm::http;
use crate::llm::http::Retry;
use crate::llm::reply;
use crate::llm::reply::Trace;
use crate::llm::reply::Use;
use crate::llm::transcript;
use crate::llm::transcript::Part;
use crate::llm::transcript::Turn;

const PROVIDER: &str = "anthropic";
const VERSION: &str = "2026-10-21";
const BETAS: &str = "server-side-fallback-2026-07-01,thinking-binding-controls-2026-08-01";

/// The model a Claude API key thinks with when the settings name none.
pub const DEFAULT_MODEL: &str = "claude-sonnet-5-5";

/// Settings for the Claude API. `new` fills in everything but the key.
#[derive(Clone)]
pub struct AnthropicConfig {
    pub api_key: String,
    pub model: String,
    pub base_url: String,
    pub max_tokens: u32,
    /// The model's context window in tokens. `None` carries the default amount of conversation.
    pub context_window: Option<u32>,
    /// How hard the model thinks, such as `low` or `high`. `None` leaves the API default.
    pub effort: Option<String>,
    pub system: String,
    /// Whether the model may hand work to a sub-session with `delegate`. A sub-session's own
    /// model may not.
    pub delegates: bool,
}

impl AnthropicConfig {
    pub fn new(api_key: impl Into<String>) -> Self {
        Self {
            api_key: api_key.into(),
            model: String::from(DEFAULT_MODEL),
            base_url: String::from("https://api.anthropic.com"),
            max_tokens: 16000,
            context_window: None,
            effort: None,
            system: String::from(DEFAULT_SYSTEM),
            delegates: true,
        }
    }
}

impl AnthropicConfig {
    fn room(&self) -> Room {
        match self.context_window {
            Some(window) => Room::for_window(window, self.max_tokens),
            None => Room::DEFAULT,
        }
    }
}

impl fmt::Debug for AnthropicConfig {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("AnthropicConfig")
            .field("api_key", &"<hidden>")
            .field("model", &self.model)
            .field("base_url", &self.base_url)
            .field("max_tokens", &self.max_tokens)
            .field("context_window", &self.context_window)
            .field("effort", &self.effort)
            .finish_non_exhaustive()
    }
}

/// The main session's model on the Claude API.
/// A refused request is re-run server-side on Anthropic's recommended fallback model.
#[derive(Debug)]
pub struct Anthropic {
    config: AnthropicConfig,
    client: Client,
    retry: Retry,
}

impl Anthropic {
    pub fn new(config: AnthropicConfig) -> Result<Self, ConnectorError> {
        let client = http::client().map_err(|detail| ConnectorError { detail })?;
        Ok(Self {
            config,
            client,
            retry: http::RETRY,
        })
    }

    /// The models this key may use, as their ids and the names Anthropic shows, newest first.
    pub async fn models(&self) -> Result<Vec<(String, String)>, CheckError> {
        let url = format!(
            "{}/v1/models?limit=1000",
            self.config.base_url.trim_end_matches('/')
        );
        let request = self
            .client
            .get(url)
            .header("x-api-key", &self.config.api_key)
            .header("anthropic-version", VERSION);
        model_names(&http::fetch(request).await?).ok_or(CheckError::NoAnswer)
    }

    async fn call(&self, request: ModelRequest) -> Result<ModelReply, ModelError> {
        parse(&self.post(body(&self.config, &request)?).await?)
    }

    async fn summary(&self, request: SummaryRequest) -> Result<String, ModelError> {
        parse_summary(&self.post(summary_body(&self.config, &request)).await?)
    }

    async fn post(&self, body: String) -> Result<String, ModelError> {
        let url = format!("{}/v1/messages", self.config.base_url.trim_end_matches('/'));
        http::send(PROVIDER, self.retry, || {
            self.client
                .post(&url)
                .header("x-api-key", &self.config.api_key)
                .header("anthropic-version", VERSION)
                .header("anthropic-beta", BETAS)
                .header("content-type", "application/json")
                .body(body.clone())
        })
        .await
    }
}

impl Model for Anthropic {
    fn complete(&self, request: ModelRequest) -> BoxFuture<'_, Result<ModelReply, ModelError>> {
        Box::pin(self.call(request))
    }

    fn summarize(&self, request: SummaryRequest) -> BoxFuture<'_, Result<String, ModelError>> {
        Box::pin(self.summary(request))
    }

    fn room(&self) -> Room {
        self.config.room()
    }

    fn check(&self) -> BoxFuture<'_, Result<(), CheckError>> {
        let url = format!(
            "{}/v1/models?limit=1",
            self.config.base_url.trim_end_matches('/')
        );
        let request = self
            .client
            .get(url)
            .header("x-api-key", &self.config.api_key)
            .header("anthropic-version", VERSION);
        Box::pin(http::check(request))
    }
}

fn body(config: &AnthropicConfig, request: &ModelRequest) -> Result<String, ModelError> {
    let mut tools = Vec::new();
    for spec in reply::tools(&request.tools, config.delegates) {
        let schema = RawValue::from_string(spec.input_schema).map_err(|error| ModelError {
            detail: format!("tool `{}` has an invalid input schema: {error}", spec.name),
        })?;
        tools.push(json!({
            "name": spec.name,
            "description": spec.description,
            "input_schema": schema,
        }));
    }
    let mut messages: Vec<Value> = transcript::build(
        &request.conversation,
        PROVIDER,
        &reply::zone(request.time_zone.as_deref()),
        config.room(),
    )
    .into_iter()
    .map(message)
    .collect();
    mark_for_cache(&mut messages);
    let mut body = json!({
        "model": config.model,
        "max_tokens": config.max_tokens,
        "system": [{
            "type": "text",
            "text": reply::system(&config.system, request.context.as_deref()),
            "cache_control": { "type": "ephemeral" },
        }],
        "messages": messages,
        "tools": tools,
        "tool_choice": { "type": "auto" },
        "thinking": {
            "type": "adaptive",
            "block_binding": { "prefix_mismatch_behavior": "drop_block" },
        },
        "fallbacks": "default",
    });
    if let Some(effort) = &config.effort {
        body["output_config"] = json!({ "effort": effort });
    }
    Ok(body.to_string())
}

fn summary_body(config: &AnthropicConfig, request: &SummaryRequest) -> String {
    let input = transcript::summary_input(
        request.previous.as_deref(),
        &request.conversation,
        &reply::zone(request.time_zone.as_deref()),
    );
    json!({
        "model": config.model,
        "max_tokens": config.max_tokens,
        "system": [{ "type": "text", "text": SUMMARY_SYSTEM }],
        "messages": [{ "role": "user", "content": input }],
        "fallbacks": "default",
    })
    .to_string()
}

fn mark_for_cache(messages: &mut [Value]) {
    for message in messages
        .iter_mut()
        .rev()
        .filter(|message| message["role"] == "user")
        .take(2)
    {
        if let Some(last) = message["content"]
            .as_array_mut()
            .and_then(|content| content.last_mut())
        {
            last["cache_control"] = json!({ "type": "ephemeral" });
        }
    }
}

fn message(turn: Turn) -> Value {
    match turn {
        Turn::User(parts) => {
            let content: Vec<Value> = parts
                .into_iter()
                .map(|part| match part {
                    Part::Result(outcome) => json!({
                        "type": "tool_result",
                        "tool_use_id": id_claude_accepts(&outcome.id),
                        "content": outcome.text,
                        "is_error": outcome.failed,
                    }),
                    Part::Note(text) => json!({ "type": "text", "text": text }),
                })
                .collect();
            json!({ "role": "user", "content": content })
        }
        Turn::Assistant(said) => match said.raw {
            Some(raw) => json!({ "role": "assistant", "content": raw }),
            None => {
                let content: Vec<Value> = said.calls.into_iter().map(tool_use).collect();
                json!({ "role": "assistant", "content": content })
            }
        },
    }
}

fn tool_use(call: Use) -> Value {
    let input = serde_json::from_str::<Value>(&call.input)
        .ok()
        .filter(Value::is_object)
        .unwrap_or_else(|| json!({}));
    json!({
        "type": "tool_use",
        "id": id_claude_accepts(&call.id),
        "name": call.name,
        "input": input,
    })
}

fn id_claude_accepts(id: &str) -> String {
    id.chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' {
                c
            } else {
                '_'
            }
        })
        .collect()
}

#[derive(Deserialize)]
struct Response<'a> {
    #[serde(borrow)]
    content: &'a RawValue,
    stop_reason: Option<String>,
    stop_details: Option<StopDetails>,
}

#[derive(Deserialize)]
struct StopDetails {
    category: Option<String>,
}

#[derive(Deserialize)]
struct Block {
    #[serde(rename = "type")]
    kind: String,
    id: Option<String>,
    name: Option<String>,
    input: Option<Box<RawValue>>,
    text: Option<String>,
}

#[derive(Deserialize)]
struct Listed {
    data: Vec<ListedModel>,
}

#[derive(Deserialize)]
struct ListedModel {
    id: String,
    #[serde(default)]
    display_name: Option<String>,
}

fn model_names(text: &str) -> Option<Vec<(String, String)>> {
    let listed: Listed = serde_json::from_str(text).ok()?;
    Some(
        listed
            .data
            .into_iter()
            .filter(|model| !model.id.trim().is_empty())
            .map(|model| {
                let name = model
                    .display_name
                    .filter(|name| !name.trim().is_empty())
                    .unwrap_or_else(|| model.id.clone());
                (model.id, name)
            })
            .collect(),
    )
}

fn read(text: &str) -> Result<Response<'_>, ModelError> {
    let response: Response = serde_json::from_str(text).map_err(|error| ModelError {
        detail: format!("{PROVIDER} reply unreadable: {error}"),
    })?;
    match response.stop_reason.as_deref() {
        Some("refusal") => {
            let category = response
                .stop_details
                .and_then(|details| details.category)
                .unwrap_or_else(|| String::from("unspecified"));
            return Err(ModelError {
                detail: format!("{PROVIDER} declined the request ({category})"),
            });
        }
        Some("max_tokens") => {
            return Err(ModelError {
                detail: format!("{PROVIDER} reply was cut off at the token limit"),
            });
        }
        Some("model_context_window_exceeded") => {
            return Err(ModelError {
                detail: format!("{PROVIDER} conversation no longer fits the context window"),
            });
        }
        _ => {}
    }
    Ok(response)
}

fn parse_summary(text: &str) -> Result<String, ModelError> {
    let blocks = blocks(&read(text)?)?;
    let summary: String = blocks
        .iter()
        .filter(|block| block.kind == "text")
        .filter_map(|block| block.text.as_deref())
        .collect();
    let summary = summary.trim();
    if summary.is_empty() {
        return Err(ModelError {
            detail: format!("{PROVIDER} wrote an empty summary"),
        });
    }
    Ok(summary.to_owned())
}

fn blocks(response: &Response) -> Result<Vec<Block>, ModelError> {
    serde_json::from_str(response.content.get()).map_err(|error| ModelError {
        detail: format!("{PROVIDER} reply content unreadable: {error}"),
    })
}

fn parse(text: &str) -> Result<ModelReply, ModelError> {
    let response = read(text)?;
    let blocks = blocks(&response)?;
    let blocks_empty = blocks.is_empty();
    let blank_text = blocks.iter().any(|block| {
        block.kind == "text"
            && block
                .text
                .as_deref()
                .is_some_and(|text| text.trim().is_empty())
    });
    let mut uses = Vec::new();
    for block in blocks {
        if block.kind != "tool_use" {
            continue;
        }
        let (Some(id), Some(name)) = (block.id, block.name) else {
            return Err(ModelError {
                detail: format!("{PROVIDER} tool use without an id or name"),
            });
        };
        let input = block
            .input
            .map(|input| input.get().to_owned())
            .unwrap_or_else(|| String::from("{}"));
        uses.push(Use { id, name, input });
    }
    let decided = reply::decide(uses);
    if blocks_empty || blank_text {
        return Ok(ModelReply {
            actions: decided.actions,
            trace: None,
        });
    }
    let trace = Trace {
        raw: response.content.to_owned(),
        ids: decided.ids,
        rejected: decided.rejected,
    };
    let data = serde_json::to_string(&trace).map_err(|error| ModelError {
        detail: format!("{PROVIDER} reply could not be kept: {error}"),
    })?;
    Ok(ModelReply {
        actions: decided.actions,
        trace: Some(ModelTrace {
            provider: String::from(PROVIDER),
            data,
        }),
    })
}

#[cfg(test)]
#[path = "anthropic.test.rs"]
mod tests;
