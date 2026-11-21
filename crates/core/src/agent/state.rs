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
use crate::conversation::Entry;
use crate::id::ChannelId;
use crate::id::ExternalMessageId;
use crate::sub_task::SubTaskSpec;
use crate::tool::ToolCallId;

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
    pub(crate) fn new(roster: Roster, seen_capacity: NonZeroUsize) -> Self {
        todo!()
    }
}

pub(crate) enum Phase {
    Idle,
    Thinking {
        call: CallId,
        after: AfterReply,
    },
    /// An owner message waits here too. Tools finish, or time out, before the next call.
    UsingTools {
        waiting: HashMap<ToolRunId, ToolCallId>,
    },
    /// Shutdown waits for running tools, so their results are recorded.
    Stopping {
        waiting: HashSet<ToolRunId>,
    },
}

/// What happens when the reply in flight arrives. Its actions always run.
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
    pub(crate) fn first_time(&mut self, key: MessageKey) -> bool {
        todo!()
    }
}
