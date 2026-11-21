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
