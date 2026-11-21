use std::error::Error;
use std::fmt;

use crate::BoxFuture;
use crate::conversation::Action;
use crate::conversation::Entry;
use crate::tool::ToolSpec;

#[derive(Clone, Debug)]
pub struct ModelRequest {
    pub conversation: Vec<Entry>,
    pub tools: Vec<ToolSpec>,
}

#[derive(Clone, Debug)]
pub struct ModelReply {
    pub actions: Vec<Action>,
    pub trace: Option<ModelTrace>,
}

/// What a connector needs to replay a reply exactly, such as the provider's raw content with
/// its thinking blocks. Core records it and hands it back unread.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ModelTrace {
    pub provider: String,
    pub data: String,
}

#[derive(Clone, Debug)]
pub struct ModelError {
    pub detail: String,
}

impl fmt::Display for ModelError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "model failed: {}", self.detail)
    }
}

impl Error for ModelError {}

pub trait Model: Send + Sync + 'static {
    /// Dropping the returned future cancels the request.
    fn complete(&self, request: ModelRequest) -> BoxFuture<'_, Result<ModelReply, ModelError>>;
}
