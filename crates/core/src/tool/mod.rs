use std::error::Error;
use std::fmt;
use std::sync::Arc;

use crate::BoxFuture;

/// The id the model provider gave one tool call, so its result can be paired with it.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct ToolCallId(Arc<str>);

impl ToolCallId {
    pub fn new(raw: impl Into<Arc<str>>) -> Self {
        Self(raw.into())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

#[derive(Clone, Debug)]
pub struct ToolCall {
    pub id: ToolCallId,
    pub name: String,
    pub input: String,
}

#[derive(Clone, Debug)]
pub struct ToolSpec {
    pub name: String,
    pub description: String,
}

#[derive(Clone, Debug)]
pub struct ToolError {
    pub detail: String,
}

impl fmt::Display for ToolError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "tool failed: {}", self.detail)
    }
}

impl Error for ToolError {}

pub trait Toolbox: Send + Sync + 'static {
    fn specs(&self) -> Vec<ToolSpec>;

    /// Core never cancels a running tool, except when it exceeds the configured timeout.
    fn run(&self, call: ToolCall) -> BoxFuture<'_, Result<String, ToolError>>;
}
