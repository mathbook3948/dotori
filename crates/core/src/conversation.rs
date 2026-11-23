use crate::id::ChannelId;
use crate::llm::ModelTrace;
use crate::message::Message;
use crate::sub_session::SubSessionOutcome;
use crate::sub_session::SubSessionSpec;
use crate::tool::ToolCall;
use crate::tool::ToolCallId;
use crate::tool::ToolError;

/// Short handle core gives each place it has heard from.
/// The model replies by handle and never writes connector ids.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct ThreadRef(u32);

impl ThreadRef {
    /// For connectors that read a handle back, such as from model output or the raw record.
    /// Core refuses a reply to a handle it never gave out.
    pub fn new(index: u32) -> Self {
        Self(index)
    }

    pub fn get(self) -> u32 {
        self.0
    }
}

/// One line of the main session, in arrival order. The model reads this.
#[derive(Clone, Debug)]
pub enum Entry {
    Heard {
        thread: ThreadRef,
        message: Message,
    },
    /// What the model decided in one reply, recorded before any of it runs.
    /// `answered` is how many entries the model had read, so a connector can place the reply
    /// before the entries that arrived while it was thinking.
    Acted {
        actions: Vec<Action>,
        answered: usize,
        trace: Option<ModelTrace>,
    },
    ToolReturned {
        call: ToolCallId,
        result: Result<String, ToolError>,
    },
    SubSessionReturned {
        spec: SubSessionSpec,
        outcome: SubSessionOutcome,
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
    /// `privately` sends the text to the room on the thread's channel that only the owner reads,
    /// instead of to the thread itself.
    Say {
        thread: ThreadRef,
        text: String,
        privately: bool,
    },
    UseTool(ToolCall),
    Delegate(SubSessionSpec),
}
