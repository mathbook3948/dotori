//! The runner that works a sub-task in dotori's own loop, with the same tools as the main
//! conversation.

use std::sync::Arc;
use std::time::Duration;

use dotori_core::BoxFuture;
use dotori_core::conversation::Action;
use dotori_core::conversation::Entry;
use dotori_core::conversation::ThreadRef;
use dotori_core::id::ChannelId;
use dotori_core::id::ExternalMessageId;
use dotori_core::id::ExternalUserId;
use dotori_core::llm::Model;
use dotori_core::llm::ModelRequest;
use dotori_core::message::Address;
use dotori_core::message::Content;
use dotori_core::message::Message;
use dotori_core::message::Place;
use dotori_core::message::Role;
use dotori_core::message::Sender;
use dotori_core::sub_task::SubTaskOutcome;
use dotori_core::sub_task::SubTaskRunner;
use dotori_core::sub_task::SubTaskSpec;
use dotori_core::tool::ToolError;
use dotori_core::tool::Toolbox;
use tokio::time;

/// What the model of an LLM sub-task is told, to be appended to its system prompt.
pub const LLM_SUB_TASK_SYSTEM: &str = "\
Your brief arrives as a message on thread 0. Call `say` on thread 0 with your whole result when you are done.
Do not use `delegate`.";

/// Runs each sub-task as its own conversation with the model, until the model answers on the
/// brief's thread and calls no more tools.
pub struct LlmRunner {
    pub model: Arc<dyn Model>,
    pub toolbox: Arc<dyn Toolbox>,
    pub max_turns: usize,
    pub tool_timeout: Duration,
}

impl std::fmt::Debug for LlmRunner {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LlmRunner")
            .field("max_turns", &self.max_turns)
            .finish_non_exhaustive()
    }
}

impl LlmRunner {
    async fn work(&self, spec: SubTaskSpec) -> SubTaskOutcome {
        let content = match Content::parse(spec.brief.clone()) {
            Ok(content) => content,
            Err(error) => return SubTaskOutcome::Failed(error.to_string()),
        };
        let channel = match ChannelId::parse("sub-task") {
            Ok(channel) => channel,
            Err(error) => return SubTaskOutcome::Failed(error.to_string()),
        };
        let mut conversation = vec![Entry::Heard {
            thread: brief_thread(),
            message: Message {
                id: ExternalMessageId::new("brief"),
                address: Address {
                    channel,
                    place: Place::new(spec.title.clone()),
                },
                sender: Sender {
                    id: ExternalUserId::new("dotori"),
                    name: String::from("dotori"),
                    role: Role::Owner,
                },
                content,
            },
        }];
        let mut result = None;
        for _ in 0..self.max_turns {
            let request = ModelRequest {
                conversation: conversation.clone(),
                tools: self.toolbox.specs(),
            };
            let reply = match self.model.complete(request).await {
                Ok(reply) => reply,
                Err(error) => return SubTaskOutcome::Failed(error.to_string()),
            };
            let answered = conversation.len();
            let actions = reply.actions.clone();
            conversation.push(Entry::Acted {
                actions: reply.actions,
                answered,
                trace: reply.trace,
            });
            let mut used_tools = false;
            for action in actions {
                match action {
                    Action::Say { thread, text } if thread == brief_thread() => result = Some(text),
                    Action::UseTool(call) => {
                        used_tools = true;
                        let id = call.id.clone();
                        let outcome = time::timeout(self.tool_timeout, self.toolbox.run(call))
                            .await
                            .unwrap_or_else(|_| {
                                Err(ToolError {
                                    detail: String::from("the tool did not finish in time"),
                                })
                            });
                        conversation.push(Entry::ToolReturned {
                            call: id,
                            result: outcome,
                        });
                    }
                    refused => conversation.push(Entry::Refused(refused)),
                }
            }
            if !used_tools {
                return match result {
                    Some(text) => SubTaskOutcome::Finished(text),
                    None => {
                        SubTaskOutcome::Failed(String::from("the sub-task ended without a result"))
                    }
                };
            }
        }
        SubTaskOutcome::Failed(format!(
            "the sub-task did not finish in {} model turns",
            self.max_turns
        ))
    }
}

fn brief_thread() -> ThreadRef {
    ThreadRef::new(0)
}

impl SubTaskRunner for LlmRunner {
    fn run(&self, spec: SubTaskSpec) -> BoxFuture<'_, SubTaskOutcome> {
        Box::pin(self.work(spec))
    }
}

#[cfg(test)]
#[path = "llm.test.rs"]
mod tests;
