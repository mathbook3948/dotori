use crate::BoxFuture;

/// What the main conversation hands to a sub-task. The brief is the only context it gets.
#[derive(Clone, Debug)]
pub struct SubTaskSpec {
    pub title: String,
    pub brief: String,
}

/// The only thing that flows back to the main conversation.
#[derive(Clone, Debug)]
pub enum SubTaskOutcome {
    Finished(String),
    Failed(String),
}

pub trait SubTaskRunner: Send + Sync + 'static {
    /// Core supervises the returned future and reports exactly one outcome, `Failed` on panic.
    fn run(&self, spec: SubTaskSpec) -> BoxFuture<'_, SubTaskOutcome>;
}
