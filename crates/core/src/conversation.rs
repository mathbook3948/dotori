use crate::id::ChannelId;
use crate::message::Message;
use crate::sub_task::SubTaskOutcome;
use crate::sub_task::SubTaskSpec;
use crate::tool::ToolCall;
use crate::tool::ToolCallId;
use crate::tool::ToolError;

/// Short handle core gives each place it has heard from.
/// The model replies by handle and never writes connector ids.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct ThreadRef(u32);

impl ThreadRef {
    pub(crate) fn new(index: u32) -> Self {
        Self(index)
    }
}

/// One line of the main conversation, in arrival order. The model reads this.
#[derive(Clone, Debug)]
pub enum Entry {
    Heard {
        thread: ThreadRef,
        message: Message,
    },
    /// What the model decided in one reply, recorded before any of it runs.
    Acted(Vec<Action>),
    ToolReturned {
        call: ToolCallId,
        result: Result<String, ToolError>,
    },
    SubTaskReturned {
        spec: SubTaskSpec,
        outcome: SubTaskOutcome,
    },
    /// An action core could not carry out, such as a reply to a thread it never heard from.
    Refused(Action),
    DeliveryFailed {
        thread: ThreadRef,
        text: String,
        detail: String,
    },
    ModelFailed {
        detail: String,
    },
    ChannelEnded {
        channel: ChannelId,
        detail: Option<String>,
    },
}

#[derive(Clone, Debug)]
pub enum Action {
    Say { thread: ThreadRef, text: String },
    UseTool(ToolCall),
    Delegate(SubTaskSpec),
}
