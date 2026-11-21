use crate::agent::ids::CallId;
use crate::agent::ids::SubTaskId;
use crate::agent::ids::ToolRunId;
use crate::channel::Traffic;
use crate::conversation::ThreadRef;
use crate::llm::ModelRequest;
use crate::message::Address;
use crate::sub_task::SubTaskSpec;
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
    CallModel {
        call: CallId,
        request: ModelRequest,
    },
    CancelModel(CallId),
    RunTool {
        run: ToolRunId,
        call: ToolCall,
    },
    SpawnSubTask {
        sub_task: SubTaskId,
        spec: SubTaskSpec,
    },
    Stop,
}
