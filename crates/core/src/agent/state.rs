use std::collections::HashMap;
use std::collections::HashSet;
use std::collections::VecDeque;
use std::num::NonZeroUsize;

use crate::agent::ids::CallId;
use crate::agent::ids::GatherId;
use crate::agent::ids::Ids;
use crate::agent::ids::SubSessionId;
use crate::agent::ids::SummaryId;
use crate::agent::ids::ToolRunId;
use crate::agent::threads::Threads;
use crate::carried::Room;
use crate::conversation::Action;
use crate::conversation::Entry;
use crate::conversation::MessageRef;
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
    pub(crate) summarizing: Summarizing,
    /// The room of the model that answers next, set before each event.
    pub(crate) room: Room,
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
            summarizing: Summarizing::Idle,
            room: Room::DEFAULT,
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

impl AgentState {
    pub(crate) fn working(&self) -> bool {
        matches!(
            self.phase,
            Phase::Gathering { .. } | Phase::Thinking { .. } | Phase::UsingTools { .. }
        )
    }
}

/// Whether the conversation is being summarized in the background. A restart starts `Idle`.
pub(crate) enum Summarizing {
    Idle,
    /// `kept` is the message the summary will name.
    Running {
        summary: SummaryId,
        kept: MessageRef,
    },
    /// The last try failed. Another waits for a step more than `weight`.
    Failed {
        weight: usize,
    },
}

pub(crate) enum Phase {
    Idle,
    /// Owner messages are gathered before a model call. Another one starts the wait over.
    Gathering {
        gather: GatherId,
    },
    Thinking {
        call: CallId,
        after: AfterReply,
        answered: usize,
    },
    /// An owner message waits here too. Tools finish, or time out, before the next call.
    UsingTools {
        waiting: HashMap<ToolRunId, ToolCallId>,
        call_again: bool,
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
    /// An owner message. The call in flight is cancelled, and the model is called after a wait
    /// for more.
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
