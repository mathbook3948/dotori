use std::collections::HashMap;
use std::mem;
use std::time::SystemTime;

use crate::agent::effect::Effect;
use crate::agent::event::Event;
use crate::agent::ids::CallId;
use crate::agent::ids::GatherId;
use crate::agent::ids::SubSessionId;
use crate::agent::ids::SummaryId;
use crate::agent::ids::ToolRunId;
use crate::agent::state::AfterReply;
use crate::agent::state::AgentState;
use crate::agent::state::Arrival;
use crate::agent::state::MessageKey;
use crate::agent::state::Phase;
use crate::agent::state::Summarizing;
use crate::carried::plan;
use crate::channel::ChannelError;
use crate::channel::Traffic;
use crate::conversation::Action;
use crate::conversation::Entry;
use crate::conversation::ThreadRef;
use crate::id::ChannelId;
use crate::llm::ModelError;
use crate::llm::ModelReply;
use crate::llm::SummaryRequest;
use crate::message::Address;
use crate::message::Incoming;
use crate::message::Message;
use crate::message::Place;
use crate::message::Role;
use crate::message::Sender;
use crate::sub_session::SubSessionOutcome;
use crate::sub_session::SubSessionSpec;
use crate::tool::Origin;
use crate::tool::ToolCallId;
use crate::tool::ToolError;

/// The whole agent behavior as a pure transition. No I/O, no clock, no tasks.
/// A stale or duplicate result matches no id and is ignored.
pub(crate) fn step(state: &mut AgentState, event: Event) -> Vec<Effect> {
    match event {
        Event::Received {
            channel,
            incoming,
            at,
        } => on_received(state, channel, incoming, at),
        Event::ModelDone { call, result } => on_model_done(state, call, result),
        Event::Gathered(gather) => on_gathered(state, gather),
        Event::SummaryDone { summary, result } => {
            on_summary_done(state, summary, result);
            Vec::new()
        }
        Event::ToolDone {
            run,
            result,
            asked_owner,
        } => on_tool_done(state, run, result, asked_owner),
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
        Event::SubSessionReturned {
            spec,
            outcome,
            record,
            ack,
        } => on_returned(state, spec, outcome, record, ack),
        Event::Forgotten { entries } => {
            on_forgotten(state, &entries);
            Vec::new()
        }
        Event::Shutdown { handing_off } => {
            state.handing_off |= handing_off;
            on_shutdown(state)
        }
    }
}

fn on_received(
    state: &mut AgentState,
    channel: ChannelId,
    incoming: Incoming,
    at: SystemTime,
) -> Vec<Effect> {
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
            at: Some(at),
        },
    });
    effects.extend(absorb(state, Arrival::Interrupts));
    effects
}

fn on_forgotten(state: &mut AgentState, entries: &[Entry]) {
    let mut first: Option<usize> = None;
    for forgotten in entries {
        let Some(index) = state
            .conversation
            .iter()
            .position(|entry| entry.is_same(forgotten))
        else {
            continue;
        };
        remove(state, index);
        first = Some(first.map_or(index, |first| first.min(index)));
    }
    let Some(first) = first else {
        return;
    };
    state.summarizing = Summarizing::Idle;
    for index in (first..state.conversation.len()).rev() {
        if matches!(state.conversation[index], Entry::Summarized { .. }) {
            remove(state, index);
        }
    }
}

fn remove(state: &mut AgentState, index: usize) {
    state.conversation.remove(index);
    for entry in &mut state.conversation[index..] {
        if let Entry::Acted { answered, .. } = entry
            && *answered > index
        {
            *answered -= 1;
        }
    }
    if let Phase::Thinking { answered, .. } = &mut state.phase
        && *answered > index
    {
        *answered -= 1;
    }
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
            return summarize_if_due(state).into_iter().collect();
        }
    };
    state.conversation.push(Entry::Acted {
        actions: reply.actions.clone(),
        answered,
        trace: reply.trace,
    });
    let (mut effects, waiting) = run_actions(state, reply.actions, answered);
    if !waiting.is_empty() {
        state.phase = Phase::UsingTools {
            waiting,
            call_again: false,
        };
    } else if after == AfterReply::CallAgain {
        effects.push(call_model(state));
    } else {
        state.phase = Phase::Idle;
        effects.extend(summarize_if_due(state));
    }
    effects
}

fn on_gathered(state: &mut AgentState, gather: GatherId) -> Vec<Effect> {
    match state.phase {
        Phase::Gathering { gather: current } if current == gather => vec![call_model(state)],
        _ => Vec::new(),
    }
}

fn summarize_if_due(state: &mut AgentState) -> Option<Effect> {
    let plan = plan(&state.conversation, state.room)?;
    match state.summarizing {
        Summarizing::Idle => {}
        Summarizing::Failed { weight } if plan.weight >= weight + state.room.step() => {}
        Summarizing::Running { .. } | Summarizing::Failed { .. } => return None,
    }
    let summary = state.ids.next_summary();
    state.summarizing = Summarizing::Running {
        summary,
        kept: plan.kept,
    };
    Some(Effect::Summarize {
        summary,
        request: SummaryRequest {
            previous: plan.previous,
            conversation: state.conversation[plan.covered].to_vec(),
            time_zone: None,
        },
    })
}

fn on_summary_done(state: &mut AgentState, summary: SummaryId, result: Result<String, ModelError>) {
    match &state.summarizing {
        Summarizing::Running {
            summary: current, ..
        } if *current == summary => {}
        _ => return,
    }
    let Summarizing::Running { kept, .. } = mem::replace(&mut state.summarizing, Summarizing::Idle)
    else {
        return;
    };
    match result {
        Ok(text) => state.conversation.push(Entry::Summarized { kept, text }),
        Err(_) => {
            if let Some(plan) = plan(&state.conversation, state.room) {
                state.summarizing = Summarizing::Failed {
                    weight: plan.weight,
                };
            }
        }
    }
}

fn on_tool_done(
    state: &mut AgentState,
    run: ToolRunId,
    result: Result<String, ToolError>,
    asked_owner: bool,
) -> Vec<Effect> {
    let waiting = match &mut state.phase {
        Phase::UsingTools {
            waiting,
            call_again,
        } => {
            *call_again |= !asked_owner;
            waiting
        }
        Phase::Stopping { waiting } => waiting,
        Phase::Idle | Phase::Gathering { .. } | Phase::Thinking { .. } => return Vec::new(),
    };
    let Some(call) = waiting.remove(&run) else {
        return Vec::new();
    };
    let done = waiting.is_empty();
    state
        .conversation
        .push(Entry::ToolReturned { call, result });
    match (&state.phase, done) {
        (
            Phase::UsingTools {
                call_again: true, ..
            },
            true,
        ) => vec![call_model(state)],
        (Phase::UsingTools { .. }, true) => {
            state.phase = Phase::Idle;
            Vec::new()
        }
        (Phase::Stopping { .. }, true) => vec![stop(state)],
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

fn on_returned(
    state: &mut AgentState,
    spec: SubSessionSpec,
    outcome: SubSessionOutcome,
    record: bool,
    ack: u64,
) -> Vec<Effect> {
    let started = state
        .sub_sessions
        .iter()
        .filter(|(_, running)| **running == spec)
        .map(|(id, _)| *id)
        .min();
    if started.is_none() && !still_open(&state.conversation, &spec) {
        if !record && unanswered(&state.conversation, &spec, &outcome) {
            let mut effects = vec![Effect::Acknowledge(ack, true)];
            effects.extend(absorb(state, Arrival::Joins));
            return effects;
        }
        return vec![Effect::Acknowledge(ack, false)];
    }
    if let Some(id) = started {
        state.sub_sessions.remove(&id);
    }
    state
        .conversation
        .push(Entry::SubSessionReturned { spec, outcome });
    if !record {
        state.unrecorded += 1;
    }
    let mut effects = vec![Effect::Acknowledge(ack, true)];
    effects.extend(absorb(state, Arrival::Joins));
    effects
}

fn unanswered(conversation: &[Entry], spec: &SubSessionSpec, outcome: &SubSessionOutcome) -> bool {
    conversation
        .iter()
        .rev()
        .take_while(|entry| !matches!(entry, Entry::Acted { .. }))
        .any(|entry| {
            matches!(entry, Entry::SubSessionReturned { spec: returned, outcome: came } if returned == spec && came == outcome)
        })
}

fn still_open(conversation: &[Entry], spec: &SubSessionSpec) -> bool {
    let mut open = 0usize;
    for entry in conversation {
        match entry {
            Entry::Acted { actions, .. } => {
                open += actions
                    .iter()
                    .filter(
                        |action| matches!(action, Action::Delegate(delegated) if delegated == spec),
                    )
                    .count();
            }
            Entry::SubSessionReturned { spec: returned, .. } if returned == spec => {
                open = open.saturating_sub(1);
            }
            _ => {}
        }
    }
    open > 0
}

fn stop(state: &mut AgentState) -> Effect {
    if state.handing_off {
        state.sub_sessions.clear();
        return Effect::Stop;
    }
    let mut stopped: Vec<(SubSessionId, SubSessionSpec)> = state.sub_sessions.drain().collect();
    stopped.sort_by_key(|(id, _)| *id);
    for (_, spec) in stopped {
        state.conversation.push(Entry::SubSessionReturned {
            spec,
            outcome: SubSessionOutcome::Failed(String::from(
                "stopped: the main session stopped running on this PC",
            )),
        });
    }
    Effect::Stop
}

fn on_shutdown(state: &mut AgentState) -> Vec<Effect> {
    let mut effects = Vec::new();
    let waiting = match mem::replace(&mut state.phase, Phase::Idle) {
        Phase::Idle | Phase::Gathering { .. } => HashMap::new(),
        Phase::Thinking { call, .. } => {
            effects.push(Effect::CancelModel(call));
            HashMap::new()
        }
        Phase::UsingTools { waiting, .. } => waiting,
        stopping @ Phase::Stopping { .. } => {
            state.phase = stopping;
            return Vec::new();
        }
    };
    if waiting.is_empty() {
        effects.push(stop(state));
    }
    state.phase = Phase::Stopping { waiting };
    effects
}

fn absorb(state: &mut AgentState, arrival: Arrival) -> Vec<Effect> {
    match (&mut state.phase, arrival) {
        (Phase::Idle, Arrival::Joins) => vec![call_model(state)],
        (Phase::Idle | Phase::Gathering { .. }, Arrival::Interrupts) => vec![gather(state)],
        (Phase::Gathering { .. }, Arrival::Joins) => Vec::new(),
        (Phase::Thinking { call, .. }, Arrival::Interrupts) => {
            let cancelled = *call;
            vec![Effect::CancelModel(cancelled), gather(state)]
        }
        (Phase::Thinking { after, .. }, Arrival::Joins) => {
            *after = AfterReply::CallAgain;
            Vec::new()
        }
        (Phase::UsingTools { call_again, .. }, _) => {
            *call_again = true;
            Vec::new()
        }
        (Phase::Stopping { .. }, _) => Vec::new(),
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
    answered: usize,
) -> (Vec<Effect>, HashMap<ToolRunId, ToolCallId>) {
    let origin = origin(&state.conversation[..answered]);
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
                effects.push(Effect::RunTool {
                    run,
                    call,
                    origin: origin.clone(),
                });
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

fn origin(seen: &[Entry]) -> Option<Origin> {
    for entry in seen.iter().rev() {
        match entry {
            Entry::Heard { message, .. } => {
                return Some(Origin {
                    address: message.address.clone(),
                    owner: message.sender.name.clone(),
                    audience: message.audience.clone(),
                });
            }
            Entry::SubSessionReturned { .. } => return None,
            _ => {}
        }
    }
    None
}

fn gather(state: &mut AgentState) -> Effect {
    let gather = state.ids.next_gather();
    state.phase = Phase::Gathering { gather };
    Effect::Gather(gather)
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
