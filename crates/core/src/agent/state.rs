use std::collections::HashMap;
use std::collections::HashSet;
use std::collections::VecDeque;
use std::num::NonZeroUsize;

use crate::agent::ids::CallId;
use crate::agent::ids::Ids;
use crate::agent::ids::SubTaskId;
use crate::agent::ids::ToolRunId;
use crate::agent::roster::Roster;
use crate::agent::threads::Threads;
use crate::conversation::Action;
use crate::conversation::Entry;
use crate::id::ChannelId;
use crate::id::ExternalMessageId;
use crate::sub_task::SubTaskOutcome;
use crate::sub_task::SubTaskSpec;
use crate::tool::ToolCallId;
use crate::tool::ToolError;

/// Everything the agent knows. Owned by the single consumer of the queue, so it needs no lock.
pub(crate) struct AgentState {
    pub(crate) conversation: Vec<Entry>,
    pub(crate) phase: Phase,
    pub(crate) roster: Roster,
    pub(crate) threads: Threads,
    pub(crate) sub_tasks: HashMap<SubTaskId, SubTaskSpec>,
    pub(crate) seen: Seen,
    pub(crate) ids: Ids,
}

impl AgentState {
    /// Resumes a conversation loaded from the raw record. Threads are interned again in
    /// arrival order, so every recorded `ThreadRef` points at the same place, and recent
    /// message ids are seen again. Work that was running when the process stopped never
    /// returns, so it gets a result that says so, appended after the loaded entries.
    pub(crate) fn resume(
        roster: Roster,
        seen_capacity: NonZeroUsize,
        conversation: Vec<Entry>,
    ) -> Self {
        let mut state = Self::new(roster, seen_capacity);
        let mut tools = Vec::new();
        let mut delegated = Vec::new();
        for entry in &conversation {
            match entry {
                Entry::Heard { message, .. } => {
                    state.threads.intern(message.address.clone());
                    state.seen.first_time(MessageKey {
                        channel: message.address.channel.clone(),
                        message: message.id.clone(),
                    });
                }
                Entry::Acted { actions, .. } => {
                    for action in actions {
                        match action {
                            Action::UseTool(call) => tools.push(call.id.clone()),
                            Action::Delegate(spec) => delegated.push(spec.clone()),
                            Action::Say { .. } => {}
                        }
                    }
                }
                Entry::ToolReturned { call, .. } => tools.retain(|running| running != call),
                Entry::SubTaskReturned { spec, .. } => {
                    if let Some(index) = delegated.iter().position(|running| running == spec) {
                        delegated.remove(index);
                    }
                }
                _ => {}
            }
        }
        state.conversation = conversation;
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
            state.conversation.push(Entry::SubTaskReturned {
                spec,
                outcome: SubTaskOutcome::Failed(stopped.clone()),
            });
        }
        state
    }

    pub(crate) fn new(roster: Roster, seen_capacity: NonZeroUsize) -> Self {
        Self {
            conversation: Vec::new(),
            phase: Phase::Idle,
            roster,
            threads: Threads::default(),
            sub_tasks: HashMap::new(),
            seen: Seen::new(seen_capacity),
            ids: Ids::default(),
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
