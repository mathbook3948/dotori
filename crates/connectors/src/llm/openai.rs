use std::fmt;

use dotori_core::BoxFuture;
use dotori_core::llm::CheckError;
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

const PROVIDER: &str = "openai-compatible";

/// Settings for any server that speaks the OpenAI chat completions API.
/// The user always picks the address and the model.
#[derive(Clone)]
pub struct OpenAiConfig {
    /// Up to and including the version path, such as `https://api.openai.com/v1`.
    pub base_url: String,
    pub model: String,
    /// Sent as a bearer token. Local servers often need none.
    pub api_key: Option<String>,
    /// `None` leaves the reply length to the server.
    pub max_tokens: Option<u32>,
    pub system: String,
    /// Whether the model may hand work to a sub-session with `delegate`. A sub-session's own
    /// model may not.
    pub delegates: bool,
}

impl OpenAiConfig {
    pub fn new(base_url: impl Into<String>, model: impl Into<String>) -> Self {
        Self {
            base_url: base_url.into(),
            model: model.into(),
            api_key: None,
            max_tokens: None,
            system: String::from(DEFAULT_SYSTEM),
            delegates: true,
        }
    }
}

impl fmt::Debug for OpenAiConfig {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("OpenAiConfig")
            .field("base_url", &self.base_url)
            .field("model", &self.model)
            .field("api_key", &self.api_key.as_ref().map(|_| "<hidden>"))
            .field("max_tokens", &self.max_tokens)
            .finish_non_exhaustive()
    }
}

/// The main session's model on an OpenAI-compatible server.
#[derive(Debug)]
pub struct OpenAi {
    config: OpenAiConfig,
    client: Client,
    retry: Retry,
}

impl OpenAi {
    pub fn new(config: OpenAiConfig) -> Result<Self, ConnectorError> {
        let client = http::client().map_err(|detail| ConnectorError { detail })?;
        Ok(Self {
            config,
            client,
            retry: http::RETRY,
        })
    }

    async fn call(&self, request: ModelRequest) -> Result<ModelReply, ModelError> {
        let body = body(&self.config, &request)?;
        let url = format!(
            "{}/chat/completions",
            self.config.base_url.trim_end_matches('/')
        );
        let text = http::send(PROVIDER, self.retry, || {
            let builder = self
                .client
                .post(&url)
                .header("content-type", "application/json")
                .body(body.clone());
            match &self.config.api_key {
                Some(key) => builder.bearer_auth(key),
                None => builder,
            }
        })
        .await?;
        parse(&text)
    }
}

impl Model for OpenAi {
    fn complete(&self, request: ModelRequest) -> BoxFuture<'_, Result<ModelReply, ModelError>> {
        Box::pin(self.call(request))
    }

    fn check(&self) -> BoxFuture<'_, Result<(), CheckError>> {
        let url = format!("{}/models", self.config.base_url.trim_end_matches('/'));
        let request = self.client.get(url);
        let request = match &self.config.api_key {
            Some(key) => request.bearer_auth(key),
            None => request,
        };
        Box::pin(http::check(request))
    }
}

fn body(config: &OpenAiConfig, request: &ModelRequest) -> Result<String, ModelError> {
    let mut tools = Vec::new();
    for spec in reply::tools(&request.tools, config.delegates) {
        let schema = RawValue::from_string(spec.input_schema).map_err(|error| ModelError {
            detail: format!("tool `{}` has an invalid input schema: {error}", spec.name),
        })?;
        tools.push(json!({
            "type": "function",
            "function": {
                "name": spec.name,
                "description": spec.description,
                "parameters": schema,
            },
        }));
    }
    let mut messages = vec![
        json!({ "role": "system", "content": reply::system(&config.system, request.context.as_deref()) }),
    ];
    for turn in transcript::build(&request.conversation, PROVIDER) {
        messages.extend(turn_messages(turn));
    }
    let mut body = json!({
        "model": config.model,
        "messages": messages,
        "tools": tools,
    });
    if let Some(max_tokens) = config.max_tokens {
        body["max_tokens"] = json!(max_tokens);
    }
    Ok(body.to_string())
}

#[derive(Deserialize)]
struct Echo {
    content: Option<Value>,
    tool_calls: Option<Value>,
}

fn turn_messages(turn: Turn) -> Vec<Value> {
    match turn {
        Turn::User(parts) => {
            let mut messages = Vec::new();
            let mut notes = Vec::new();
            for part in parts {
                match part {
                    Part::Result(outcome) => {
                        let content = if outcome.failed {
                            format!("Error: {}", outcome.text)
                        } else {
                            outcome.text
                        };
                        messages.push(json!({
                            "role": "tool",
                            "tool_call_id": outcome.id,
                            "content": content,
                        }));
                    }
                    Part::Note(text) => notes.push(text),
                }
            }
            if !notes.is_empty() {
                messages.push(json!({ "role": "user", "content": notes.join("\n\n") }));
            }
            messages
        }
        Turn::Assistant(said) => {
            let echo = said
                .raw
                .and_then(|raw| serde_json::from_str::<Echo>(raw.get()).ok());
            let mut message = json!({ "role": "assistant", "content": Value::Null });
            match echo {
                Some(echo) => {
                    let tool_calls = echo
                        .tool_calls
                        .filter(|calls| !calls.as_array().is_some_and(Vec::is_empty));
                    match (echo.content, tool_calls) {
                        (Some(content), Some(tool_calls)) => {
                            message["content"] = content;
                            message["tool_calls"] = tool_calls;
                        }
                        (Some(content), None) => message["content"] = content,
                        (None, Some(tool_calls)) => message["tool_calls"] = tool_calls,
                        (None, None) => message["content"] = json!(""),
                    }
                }
                None if !said.calls.is_empty() => {
                    let calls: Vec<Value> = said
                        .calls
                        .into_iter()
                        .map(|call| {
                            json!({
                                "id": call.id,
                                "type": "function",
                                "function": { "name": call.name, "arguments": call.input },
                            })
                        })
                        .collect();
                    message["tool_calls"] = json!(calls);
                }
                None => {}
            }
            vec![message]
        }
    }
}

#[derive(Deserialize)]
struct Response {
    choices: Vec<Choice>,
}

#[derive(Deserialize)]
struct Choice {
    message: Box<RawValue>,
    finish_reason: Option<String>,
}

#[derive(Deserialize)]
struct Message {
    tool_calls: Option<Vec<Call>>,
}

#[derive(Deserialize)]
struct Call {
    id: String,
    function: Function,
}

#[derive(Deserialize)]
struct Function {
    name: String,
    arguments: Option<Value>,
}

fn parse(text: &str) -> Result<ModelReply, ModelError> {
    let unreadable = |error: serde_json::Error| ModelError {
        detail: format!("{PROVIDER} reply unreadable: {error}"),
    };
    let response: Response = serde_json::from_str(text).map_err(unreadable)?;
    let Some(choice) = response.choices.into_iter().next() else {
        return Err(ModelError {
            detail: format!("{PROVIDER} reply has no choices"),
        });
    };
    match choice.finish_reason.as_deref() {
        Some("length") => {
            return Err(ModelError {
                detail: format!("{PROVIDER} reply was cut off at the token limit"),
            });
        }
        Some("content_filter") => {
            return Err(ModelError {
                detail: format!("{PROVIDER} declined the request"),
            });
        }
        _ => {}
    }
    let message: Message = serde_json::from_str(choice.message.get()).map_err(unreadable)?;
    let uses: Vec<Use> = message
        .tool_calls
        .unwrap_or_default()
        .into_iter()
        .map(|call| Use {
            id: call.id,
            name: call.function.name,
            input: match call.function.arguments {
                Some(Value::String(text)) if !text.trim().is_empty() => text,
                Some(Value::String(_)) | Some(Value::Null) | None => String::from("{}"),
                Some(other) => other.to_string(),
            },
        })
        .collect();
    let decided = reply::decide(uses);
    let trace = Trace {
        raw: choice.message,
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
#[path = "openai.test.rs"]
mod tests;
