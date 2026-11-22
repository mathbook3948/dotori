use std::fmt;

use dotori_core::BoxFuture;
use dotori_core::llm::Model;
use dotori_core::llm::ModelError;
use dotori_core::llm::ModelReply;
use dotori_core::llm::ModelRequest;
use dotori_core::llm::ModelTrace;
use reqwest::Client;
use serde::Deserialize;
use serde_json::Value;
use serde_json::json;
use serde_json::value::RawValue;

use crate::llm::ConnectorError;
use crate::llm::DEFAULT_SYSTEM;
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

/// Settings for the Claude API. `new` fills in everything but the key.
#[derive(Clone)]
pub struct AnthropicConfig {
    pub api_key: String,
    pub model: String,
    pub base_url: String,
    pub max_tokens: u32,
    /// How hard the model thinks, such as `low` or `high`. `None` leaves the API default.
    pub effort: Option<String>,
    pub system: String,
}

impl AnthropicConfig {
    pub fn new(api_key: impl Into<String>) -> Self {
        Self {
            api_key: api_key.into(),
            model: String::from("claude-sonnet-5-5"),
            base_url: String::from("https://api.anthropic.com"),
            max_tokens: 16000,
            effort: None,
            system: String::from(DEFAULT_SYSTEM),
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
            .field("effort", &self.effort)
            .finish_non_exhaustive()
    }
}

/// The main conversation's model on the Claude API.
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

    async fn call(&self, request: ModelRequest) -> Result<ModelReply, ModelError> {
        let body = body(&self.config, &request)?;
        let url = format!("{}/v1/messages", self.config.base_url.trim_end_matches('/'));
        let text = http::send(PROVIDER, self.retry, || {
            self.client
                .post(&url)
                .header("x-api-key", &self.config.api_key)
                .header("anthropic-version", VERSION)
                .header("anthropic-beta", BETAS)
                .header("content-type", "application/json")
                .body(body.clone())
        })
        .await?;
        parse(&text)
    }
}

impl Model for Anthropic {
    fn complete(&self, request: ModelRequest) -> BoxFuture<'_, Result<ModelReply, ModelError>> {
        Box::pin(self.call(request))
    }
}

fn body(config: &AnthropicConfig, request: &ModelRequest) -> Result<String, ModelError> {
    let mut tools = Vec::new();
    for spec in reply::tools(&request.tools) {
        let schema = RawValue::from_string(spec.input_schema).map_err(|error| ModelError {
            detail: format!("tool `{}` has an invalid input schema: {error}", spec.name),
        })?;
        tools.push(json!({
            "name": spec.name,
            "description": spec.description,
            "input_schema": schema,
        }));
    }
    let messages: Vec<Value> = transcript::build(&request.conversation, PROVIDER)
        .into_iter()
        .map(message)
        .collect();
    let mut body = json!({
        "model": config.model,
        "max_tokens": config.max_tokens,
        "system": reply::system(&config.system, request.context.as_deref()),
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

fn parse(text: &str) -> Result<ModelReply, ModelError> {
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
    let blocks: Vec<Block> =
        serde_json::from_str(response.content.get()).map_err(|error| ModelError {
            detail: format!("{PROVIDER} reply content unreadable: {error}"),
        })?;
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
