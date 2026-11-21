use std::pin::Pin;

pub mod channel;
pub mod conversation;
pub mod id;
pub mod llm;
pub mod message;
pub mod runtime;
pub mod sub_task;
pub mod tool;

mod agent;

/// A boxed future, so ports can be stored as trait objects.
pub type BoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;
