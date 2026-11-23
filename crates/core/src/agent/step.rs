use std::collections::HashMap;
use std::mem;

use crate::agent::effect::Effect;
use crate::agent::event::Event;
use crate::agent::ids::CallId;
use crate::agent::ids::SubSessionId;
use crate::agent::ids::ToolRunId;
use crate::agent::state::AfterReply;
use crate::agent::state::AgentState;
use crate::agent::state::Arrival;
use crate::agent::state::MessageKey;
use crate::agent::state::Phase;
use crate::channel::ChannelError;
use crate::channel::Traffic;
use crate::conversation::Action;
use crate::conversation::Entry;
use crate::conversation::ThreadRef;
use crate::id::ChannelId;
use crate::llm::ModelError;
use crate::llm::ModelReply;
use crate::message::Address;
use crate::message::Incoming;
use crate::message::Message;
use crate::message::Place;
use crate::message::Role;
use crate::message::Sender;
use crate::sub_session::SubSessionOutcome;
use crate::tool::ToolCallId;
use crate::tool::ToolError;

/// The whole agent behavior as a pure transition. No I/O, no clock, no tasks.
/// A stale or duplicate result matches no id and is ignored.
pub(crate) fn step(state: &mut AgentState, event: Event) -> Vec<Effect> {
    match event {
        Event::Received { channel, incoming } => on_received(state, channel, incoming),
        Event::ModelDone { call, result } => on_model_done(state, call, result),
        Event::ToolDone { run, result } => on_tool_done(state, run, result),
        Event::SubSessionDone {
            sub_session,
            outcome,
        } => on_sub_session_done(state, sub_session, outcome),
        Event::DeliveryDone {
            thread,
            place,
            text,
            result,
        } => on_delivery_done(state, thread, place, text, result),
        Event::ChannelEnded { channel, result } => on_channel_ended(state, channel, result),
        Event::Shutdown => on_shutdown(state),
    }
}

fn on_received(state: &mut AgentState, channel: ChannelId, incoming: Incoming) -> Vec<Effect> {
    if matches!(state.phase, Phase::Stopping { .. }) {
        return Vec::new();
    }
    let key = MessageKey {
        channel: channel.clone(),
        message: incoming.id.clone(),
    };
    if !state.seen.first_time(key) {
        return Vec::new();
    }
    let address = Address {
        channel,
        place: incoming.place,
    };
    let thread = state.threads.intern(address.clone());
    let mut effects = Vec::new();
    if address.channel != state.wiring.app {
        effects.push(Effect::Show(Traffic {
            address: address.clone(),
            who: incoming.sender_name.clone(),
            text: incoming.content.as_str().to_owned(),
        }));
    }
    state.conversation.push(Entry::Heard {
        thread,
        message: Message {
            id: incoming.id,
            address,
            sender: Sender {
                id: incoming.sender_id,
                name: incoming.sender_name,
                role: Role::Owner,
            },
            content: incoming.content,
            audience: Some(incoming.audience),
        },
    });
    effects.extend(absorb(state, Arrival::Interrupts));
    effects
}

fn on_model_done(
    state: &mut AgentState,
    call: CallId,
    result: Result<ModelReply, ModelError>,
) -> Vec<Effect> {
    let (after, answered) = match state.phase {
        Phase::Thinking {
            call: current,
            after,
            answered,
        } if current == call => (after, answered),
        _ => return Vec::new(),
    };
    let reply = match result {
        Ok(reply) => reply,
        Err(error) => {
            state.conversation.push(Entry::ModelFailed {
                detail: error.detail,
            });
            if after == AfterReply::CallAgain {
                return vec![call_model(state)];
            }
            state.phase = Phase::Idle;
            return Vec::new();
        }
    };
    state.conversation.push(Entry::Acted {
        actions: reply.actions.clone(),
        answered,
        trace: reply.trace,
    });
    let (mut effects, waiting) = run_actions(state, reply.actions);
    if !waiting.is_empty() {
        state.phase = Phase::UsingTools { waiting };
    } else if after == AfterReply::CallAgain {
        effects.push(call_model(state));
    } else {
        state.phase = Phase::Idle;
    }
    effects
}

fn on_tool_done(
    state: &mut AgentState,
    run: ToolRunId,
    result: Result<String, ToolError>,
) -> Vec<Effect> {
    let waiting = match &mut state.phase {
        Phase::UsingTools { waiting } | Phase::Stopping { waiting } => waiting,
        Phase::Idle | Phase::Thinking { .. } => return Vec::new(),
    };
    let Some(call) = waiting.remove(&run) else {
        return Vec::new();
    };
    let done = waiting.is_empty();
    state
        .conversation
        .push(Entry::ToolReturned { call, result });
    match (&state.phase, done) {
        (Phase::UsingTools { .. }, true) => vec![call_model(state)],
        (Phase::Stopping { .. }, true) => vec![Effect::Stop],
        _ => Vec::new(),
    }
}

fn on_sub_session_done(
    state: &mut AgentState,
    sub_session: SubSessionId,
    outcome: SubSessionOutcome,
) -> Vec<Effect> {
    let Some(spec) = state.sub_sessions.remove(&sub_session) else {
        return Vec::new();
    };
    state
        .conversation
        .push(Entry::SubSessionReturned { spec, outcome });
    absorb(state, Arrival::Joins)
}

fn on_delivery_done(
    state: &mut AgentState,
    thread: ThreadRef,
    place: Place,
    text: String,
    result: Result<(), ChannelError>,
) -> Vec<Effect> {
    match result {
        Ok(()) => match state.threads.resolve(thread) {
            Some(address) if address.channel != state.wiring.app => {
                vec![Effect::Show(Traffic {
                    address: Address {
                        channel: address.channel.clone(),
                        place,
                    },
                    who: String::from("dotori"),
                    text,
                })]
            }
            _ => Vec::new(),
        },
        Err(error) => {
            state.conversation.push(Entry::DeliveryFailed {
                thread,
                text,
                detail: error.detail,
            });
            Vec::new()
        }
    }
}

fn on_channel_ended(
    state: &mut AgentState,
    channel: ChannelId,
    result: Result<(), ChannelError>,
) -> Vec<Effect> {
    state.conversation.push(Entry::ChannelEnded {
        channel,
        detail: result.err().map(|error| error.detail),
    });
    Vec::new()
}

fn on_shutdown(state: &mut AgentState) -> Vec<Effect> {
    let mut effects = Vec::new();
    let waiting = match mem::replace(&mut state.phase, Phase::Idle) {
        Phase::Idle => HashMap::new(),
        Phase::Thinking { call, .. } => {
            effects.push(Effect::CancelModel(call));
            HashMap::new()
        }
        Phase::UsingTools { waiting } => waiting,
        stopping @ Phase::Stopping { .. } => {
            state.phase = stopping;
            return Vec::new();
        }
    };
    if waiting.is_empty() {
        effects.push(Effect::Stop);
    }
    state.phase = Phase::Stopping { waiting };
    effects
}

fn absorb(state: &mut AgentState, arrival: Arrival) -> Vec<Effect> {
    match (&mut state.phase, arrival) {
        (Phase::Idle, _) => vec![call_model(state)],
        (Phase::Thinking { call, .. }, Arrival::Interrupts) => {
            let cancelled = *call;
            vec![Effect::CancelModel(cancelled), call_model(state)]
        }
        (Phase::Thinking { after, .. }, Arrival::Joins) => {
            *after = AfterReply::CallAgain;
            Vec::new()
        }
        (Phase::UsingTools { .. } | Phase::Stopping { .. }, _) => Vec::new(),
    }
}

fn destination(state: &AgentState, thread: ThreadRef, privately: bool) -> Option<Address> {
    let address = state.threads.resolve(thread)?;
    if !privately {
        return Some(address.clone());
    }
    let place = state.wiring.private.get(&address.channel)?;
    Some(Address {
        channel: address.channel.clone(),
        place: place.clone(),
    })
}

fn run_actions(
    state: &mut AgentState,
    actions: Vec<Action>,
) -> (Vec<Effect>, HashMap<ToolRunId, ToolCallId>) {
    let mut effects = Vec::new();
    let mut waiting = HashMap::new();
    for action in actions {
        match action {
            Action::Say {
                thread,
                text,
                privately,
            } => match destination(state, thread, privately) {
                Some(to) => effects.push(Effect::Deliver { thread, to, text }),
                None => state.conversation.push(Entry::Refused(Action::Say {
                    thread,
                    text,
                    privately,
                })),
            },
            Action::UseTool(call) => {
                let run = state.ids.next_tool_run();
                waiting.insert(run, call.id.clone());
                effects.push(Effect::RunTool { run, call });
            }
            Action::Delegate(spec) => {
                let sub_session = state.ids.next_sub_session();
                state.sub_sessions.insert(sub_session, spec.clone());
                effects.push(Effect::SpawnSubSession { sub_session, spec });
            }
        }
    }
    (effects, waiting)
}

fn call_model(state: &mut AgentState) -> Effect {
    let call = state.ids.next_call();
    state.phase = Phase::Thinking {
        call,
        after: AfterReply::Finish,
        answered: state.conversation.len(),
    };
    Effect::CallModel {
        call,
        conversation: state.conversation.clone(),
    }
}

#[cfg(test)]
#[path = "step.test.rs"]
mod tests;
