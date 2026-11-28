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
    /// What the app adds to the model's instructions, such as the owner's profile. Core leaves
    /// it empty.
    pub context: Option<String>,
    /// The IANA name of the time zone message times are shown in, or `None` for the connector's own PC.
    pub time_zone: Option<String>,
}

/// What a model is asked to summarize: the summary so far and what came after it.
#[derive(Clone, Debug)]
pub struct SummaryRequest {
    /// The summary so far, which the new one merges.
    pub previous: Option<String>,
    /// What came after it, oldest first.
    pub conversation: Vec<Entry>,
    /// Same meaning as [`ModelRequest::time_zone`].
    pub time_zone: Option<String>,
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

/// Why a model could not be reached when it was checked.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CheckError {
    /// The provider answered and turned the key down.
    KeyRefused,
    /// Nothing answered at the model's address.
    NoAnswer,
}

pub trait Model: Send + Sync + 'static {
    /// Dropping the returned future cancels the request.
    fn complete(&self, request: ModelRequest) -> BoxFuture<'_, Result<ModelReply, ModelError>>;

    /// One summary of the request's conversation, merged with the previous one.
    /// Dropping the returned future cancels the request.
    fn summarize(&self, _: SummaryRequest) -> BoxFuture<'_, Result<String, ModelError>> {
        Box::pin(async {
            Err(ModelError {
                detail: String::from("this model cannot summarize"),
            })
        })
    }

    /// Whether the model answers with its address and key, asked in a way that costs nothing,
    /// such as listing the provider's models. A model with nothing to reach passes.
    fn check(&self) -> BoxFuture<'_, Result<(), CheckError>> {
        Box::pin(async { Ok(()) })
    }
}
