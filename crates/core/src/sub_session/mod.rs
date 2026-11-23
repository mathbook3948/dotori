use crate::BoxFuture;

/// What the main session hands to a sub-session. The brief is the only context it gets.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SubSessionSpec {
    pub title: String,
    pub brief: String,
}

/// The only thing that flows back to the main session.
#[derive(Clone, Debug)]
pub enum SubSessionOutcome {
    Finished(String),
    Failed(String),
}

pub trait SubSessionRunner: Send + Sync + 'static {
    /// Core supervises the returned future and reports exactly one outcome, `Failed` on panic.
    fn run(&self, spec: SubSessionSpec) -> BoxFuture<'_, SubSessionOutcome>;
}
