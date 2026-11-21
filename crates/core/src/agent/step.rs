use crate::agent::effect::Effect;
use crate::agent::event::Event;
use crate::agent::ids::CallId;
use crate::agent::ids::SubTaskId;
use crate::agent::ids::ToolRunId;
use crate::agent::state::AgentState;
use crate::agent::state::Arrival;
use crate::channel::ChannelError;
use crate::conversation::Action;
use crate::conversation::ThreadRef;
use crate::id::ChannelId;
use crate::llm::ModelError;
use crate::llm::ModelReply;
use crate::message::Incoming;
use crate::sub_task::SubTaskOutcome;
use crate::tool::ToolError;

/// The whole agent behavior as a pure transition. No I/O, no clock, no tasks.
/// A stale or duplicate result matches no id and is ignored.
pub(crate) fn step(state: &mut AgentState, event: Event) -> Vec<Effect> {
    match event {
        Event::Received { channel, incoming } => on_received(state, channel, incoming),
        Event::ModelDone { call, result } => on_model_done(state, call, result),
        Event::ToolDone { run, result } => on_tool_done(state, run, result),
        Event::SubTaskDone { sub_task, outcome } => on_sub_task_done(state, sub_task, outcome),
        Event::DeliveryDone {
            thread,
            text,
            result,
        } => on_delivery_done(state, thread, text, result),
        Event::ChannelEnded { channel, result } => on_channel_ended(state, channel, result),
        Event::Shutdown => on_shutdown(state),
    }
}

fn on_received(state: &mut AgentState, channel: ChannelId, incoming: Incoming) -> Vec<Effect> {
    todo!()
}

fn on_model_done(
    state: &mut AgentState,
    call: CallId,
    result: Result<ModelReply, ModelError>,
) -> Vec<Effect> {
    todo!()
}

fn on_tool_done(
    state: &mut AgentState,
    run: ToolRunId,
    result: Result<String, ToolError>,
) -> Vec<Effect> {
    todo!()
}

fn on_sub_task_done(
    state: &mut AgentState,
    sub_task: SubTaskId,
    outcome: SubTaskOutcome,
) -> Vec<Effect> {
    todo!()
}

fn on_delivery_done(
    state: &mut AgentState,
    thread: ThreadRef,
    text: String,
    result: Result<(), ChannelError>,
) -> Vec<Effect> {
    todo!()
}

fn on_channel_ended(
    state: &mut AgentState,
    channel: ChannelId,
    result: Result<(), ChannelError>,
) -> Vec<Effect> {
    todo!()
}

fn on_shutdown(state: &mut AgentState) -> Vec<Effect> {
    todo!()
}

fn absorb(state: &mut AgentState, arrival: Arrival) -> Vec<Effect> {
    todo!()
}

fn run_actions(state: &mut AgentState, actions: Vec<Action>) -> Vec<Effect> {
    todo!()
}

fn call_model(state: &mut AgentState) -> Effect {
    todo!()
}

#[cfg(test)]
#[path = "step.test.rs"]
mod tests;
