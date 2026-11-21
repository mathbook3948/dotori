#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) struct CallId(u64);

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) struct ToolRunId(u64);

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) struct SubTaskId(u64);

/// The only source of internal ids. Lives in agent state, so ids follow queue order.
#[derive(Debug, Default)]
pub(crate) struct Ids {
    next: u64,
}

impl Ids {
    pub(crate) fn next_call(&mut self) -> CallId {
        todo!()
    }

    pub(crate) fn next_tool_run(&mut self) -> ToolRunId {
        todo!()
    }

    pub(crate) fn next_sub_task(&mut self) -> SubTaskId {
        todo!()
    }
}
