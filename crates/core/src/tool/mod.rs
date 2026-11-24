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

/// The verbs a tool of dotori's own starts with. A tool is named as a verb, followed by what it
/// acts on when the verb alone is unclear, such as `add_routine`, `read_page` or `say`.
pub const TOOL_VERBS: [&str; 12] = [
    "add", "remove", "read", "write", "edit", "run", "search", "erase", "fetch", "ask", "say",
    "delegate",
];

/// Whether `name` follows that rule: lowercase words joined by `_`, the first a verb of
/// [`TOOL_VERBS`]. A relayed MCP server's tool keeps its own name after `<server>__`.
pub fn well_named(name: &str) -> bool {
    if name.contains("__") {
        return true;
    }
    let first = name.split('_').next().unwrap_or_default();
    TOOL_VERBS.contains(&first)
        && name
            .split('_')
            .all(|word| !word.is_empty() && word.bytes().all(|byte| byte.is_ascii_lowercase()))
}

#[derive(Clone, Debug)]
pub struct ToolSpec {
    pub name: String,
    pub description: String,
    /// JSON Schema of the input object, as JSON text.
    pub input_schema: String,
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

#[cfg(test)]
#[path = "mod.test.rs"]
mod tests;
