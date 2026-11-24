use crate::agent::ids::CallId;
use crate::agent::ids::SubSessionId;
use crate::agent::ids::ToolRunId;
use crate::channel::ChannelError;
use crate::conversation::ThreadRef;
use crate::id::ChannelId;
use crate::llm::ModelError;
use crate::llm::ModelReply;
use crate::message::Incoming;
use crate::message::Place;
use crate::sub_session::SubSessionOutcome;
use crate::sub_session::SubSessionSpec;
use crate::tool::ToolError;

/// The single inbound queue item. Crate-private, so outside code reaches the queue only
/// through an `Inbox` and cannot forge a model reply or a sub-session outcome.
/// The runtime turns a panic in any spawned task into an `Err`, so every effect gets exactly one result.
pub(crate) enum Event {
    Received {
        channel: ChannelId,
        incoming: Incoming,
    },
    ModelDone {
        call: CallId,
        result: Result<ModelReply, ModelError>,
    },
    ToolDone {
        run: ToolRunId,
        result: Result<String, ToolError>,
    },
    SubSessionDone {
        sub_session: SubSessionId,
        outcome: SubSessionOutcome,
    },
    DeliveryDone {
        thread: ThreadRef,
        place: Place,
        text: String,
        result: Result<(), ChannelError>,
    },
    ChannelEnded {
        channel: ChannelId,
        result: Result<(), ChannelError>,
    },
    SubSessionReturned {
        spec: SubSessionSpec,
        outcome: SubSessionOutcome,
        record: bool,
        ack: u64,
    },
    Shutdown {
        handing_off: bool,
    },
}
