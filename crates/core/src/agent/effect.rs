use crate::agent::ids::CallId;
use crate::agent::ids::SubSessionId;
use crate::agent::ids::ToolRunId;
use crate::channel::Traffic;
use crate::conversation::Entry;
use crate::conversation::ThreadRef;
use crate::message::Address;
use crate::sub_session::SubSessionSpec;
use crate::tool::ToolCall;

/// The closed set of things the agent asks the world to do.
/// Anything with a result comes back as an `Event` carrying the same id.
pub(crate) enum Effect {
    Deliver {
        thread: ThreadRef,
        to: Address,
        text: String,
    },
    Show(Traffic),
    /// The shell adds the current tool specs to build the request.
    CallModel {
        call: CallId,
        conversation: Vec<Entry>,
    },
    CancelModel(CallId),
    RunTool {
        run: ToolRunId,
        call: ToolCall,
    },
    SpawnSubSession {
        sub_session: SubSessionId,
        spec: SubSessionSpec,
    },
    Stop,
}
