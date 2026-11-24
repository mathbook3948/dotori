use std::collections::HashMap;
use std::collections::HashSet;
use std::collections::VecDeque;
use std::num::NonZeroUsize;

use crate::agent::ids::CallId;
use crate::agent::ids::Ids;
use crate::agent::ids::SubSessionId;
use crate::agent::ids::ToolRunId;
use crate::agent::threads::Threads;
use crate::conversation::Action;
use crate::conversation::Entry;
use crate::id::ChannelId;
use crate::id::ExternalMessageId;
use crate::runtime::Wiring;
use crate::sub_session::SubSessionOutcome;
use crate::sub_session::SubSessionSpec;
use crate::tool::ToolCallId;
use crate::tool::ToolError;

/// Everything the agent knows. Owned by the single consumer of the queue, so it needs no lock.
pub(crate) struct AgentState {
    pub(crate) conversation: Vec<Entry>,
    pub(crate) phase: Phase,
    pub(crate) wiring: Wiring,
    pub(crate) threads: Threads,
    pub(crate) sub_sessions: HashMap<SubSessionId, SubSessionSpec>,
    pub(crate) seen: Seen,
    pub(crate) ids: Ids,
    pub(crate) handing_off: bool,
    pub(crate) unrecorded: usize,
}

impl AgentState {
    /// Resumes a conversation loaded from the raw record. Threads are interned again in
    /// arrival order, so every recorded `ThreadRef` points at the same place, and recent
    /// message ids are seen again. Work this PC started that was running when it stopped never
    /// returns, so it gets a result that says so, appended after the loaded entries. Each entry
    /// comes with whether this PC made it; work another PC started, and results another PC
    /// recorded, are left to that PC. Sub-sessions in `running` still run here and are not
    /// closed.
    pub(crate) fn resume(
        wiring: Wiring,
        seen_capacity: NonZeroUsize,
        marked: Vec<(Entry, bool)>,
        running: Vec<SubSessionSpec>,
    ) -> Self {
        let mut state = Self::new(wiring, seen_capacity);
        let mut tools = Vec::new();
        let mut delegated = Vec::new();
        for (entry, mine) in &marked {
            let mine = *mine;
            match entry {
                Entry::Heard { message, .. } => {
                    state.threads.intern(message.address.clone());
                    state.seen.first_time(MessageKey {
                        channel: message.address.channel.clone(),
                        message: message.id.clone(),
                    });
                }
                Entry::Acted { actions, .. } if mine => {
                    for action in actions {
                        match action {
                            Action::UseTool(call) => tools.push(call.id.clone()),
                            Action::Delegate(spec) => delegated.push(spec.clone()),
                            Action::Say { .. } => {}
                        }
                    }
                }
                Entry::ToolReturned { call, .. } if mine => {
                    tools.retain(|running| running != call);
                }
                Entry::SubSessionReturned { spec, .. } if mine => {
                    if let Some(index) = delegated.iter().position(|running| running == spec) {
                        delegated.remove(index);
                    }
                }
                _ => {}
            }
        }
        for spec in running {
            if let Some(index) = delegated.iter().position(|open| *open == spec) {
                delegated.remove(index);
            }
        }
        state.conversation = marked.into_iter().map(|(entry, _)| entry).collect();
        let stopped = String::from("stopped by a restart");
        for call in tools {
            state.conversation.push(Entry::ToolReturned {
                call,
                result: Err(ToolError {
                    detail: stopped.clone(),
                }),
            });
        }
        for spec in delegated {
            state.conversation.push(Entry::SubSessionReturned {
                spec,
                outcome: SubSessionOutcome::Failed(stopped.clone()),
            });
        }
        state
    }

    pub(crate) fn new(wiring: Wiring, seen_capacity: NonZeroUsize) -> Self {
        Self {
            conversation: Vec::new(),
            phase: Phase::Idle,
            wiring,
            threads: Threads::default(),
            sub_sessions: HashMap::new(),
            seen: Seen::new(seen_capacity),
            ids: Ids::default(),
            handing_off: false,
            unrecorded: 0,
        }
    }
}

pub(crate) enum Phase {
    Idle,
    Thinking {
        call: CallId,
        after: AfterReply,
        answered: usize,
    },
    /// An owner message waits here too. Tools finish, or time out, before the next call.
    UsingTools {
        waiting: HashMap<ToolRunId, ToolCallId>,
    },
    /// Shutdown waits for running tools, so their results are recorded.
    Stopping {
        waiting: HashMap<ToolRunId, ToolCallId>,
    },
}

/// What happens when the reply in flight arrives. Its actions always run.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum AfterReply {
    /// Nothing new arrived. The turn ends unless tools were requested.
    Finish,
    /// Someone else spoke during the call. The model is called again with the new input.
    CallAgain,
}

/// How a new input meets a model call in flight.
pub(crate) enum Arrival {
    /// An owner message. The call in flight is cancelled and the model is called again.
    Interrupts,
    /// Anything else. The reply in flight runs, then the model is called again.
    Joins,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub(crate) struct MessageKey {
    pub(crate) channel: ChannelId,
    pub(crate) message: ExternalMessageId,
}

/// Bounded set of recent message ids, so a redelivered Slack message is heard once.
pub(crate) struct Seen {
    order: VecDeque<MessageKey>,
    set: HashSet<MessageKey>,
    capacity: NonZeroUsize,
}

impl Seen {
    pub(crate) fn new(capacity: NonZeroUsize) -> Self {
        Self {
            order: VecDeque::with_capacity(capacity.get()),
            set: HashSet::with_capacity(capacity.get()),
            capacity,
        }
    }

    pub(crate) fn first_time(&mut self, key: MessageKey) -> bool {
        if !self.set.insert(key.clone()) {
            return false;
        }
        self.order.push_back(key);
        if self.order.len() > self.capacity.get()
            && let Some(oldest) = self.order.pop_front()
        {
            self.set.remove(&oldest);
        }
        true
    }
}
