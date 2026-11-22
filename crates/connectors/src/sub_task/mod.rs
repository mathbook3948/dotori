//! Sub-task runners. Each one takes a brief and returns one outcome.

pub mod cli;
pub mod llm;

/// What a sub-task is told about itself, on top of its brief.
pub const SUB_TASK_SYSTEM: &str = "\
You are a sub-task of dotori, a personal assistant that lives on its owner's computer.
The brief you get is all you know, and nobody will answer questions. Do the work with your tools.
When you are done, give your whole result in one final answer. The main conversation reads only that.";
