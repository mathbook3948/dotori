#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) struct CallId(u64);

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) struct SummaryId(u64);

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) struct ToolRunId(u64);

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub(crate) struct SubSessionId(u64);

/// The only source of internal ids. Lives in agent state, so ids follow queue order.
#[derive(Debug, Default)]
pub(crate) struct Ids {
    next: u64,
}

impl Ids {
    pub(crate) fn next_call(&mut self) -> CallId {
        CallId(self.bump())
    }

    pub(crate) fn next_summary(&mut self) -> SummaryId {
        SummaryId(self.bump())
    }

    pub(crate) fn next_tool_run(&mut self) -> ToolRunId {
        ToolRunId(self.bump())
    }

    pub(crate) fn next_sub_session(&mut self) -> SubSessionId {
        SubSessionId(self.bump())
    }

    fn bump(&mut self) -> u64 {
        self.next += 1;
        self.next
    }
}
