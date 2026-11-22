use dotori_core::BoxFuture;
use dotori_core::sub_task::SubTaskOutcome;
use dotori_core::sub_task::SubTaskRunner;
use dotori_core::sub_task::SubTaskSpec;
use dotori_core::tool::ToolCall;
use dotori_core::tool::ToolError;
use dotori_core::tool::ToolSpec;
use dotori_core::tool::Toolbox;

/// The toolbox until the MCP server and the basic tools exist. Every call fails.
#[derive(Debug)]
pub(crate) struct NoTools;

impl Toolbox for NoTools {
    fn specs(&self) -> Vec<ToolSpec> {
        Vec::new()
    }

    fn run(&self, call: ToolCall) -> BoxFuture<'_, Result<String, ToolError>> {
        Box::pin(async move {
            Err(ToolError {
                detail: format!("there is no tool named `{}`", call.name),
            })
        })
    }
}

/// The runner until the sub-task runners exist. Every sub-task fails at once.
#[derive(Debug)]
pub(crate) struct NoSubTasks;

impl SubTaskRunner for NoSubTasks {
    fn run(&self, _spec: SubTaskSpec) -> BoxFuture<'_, SubTaskOutcome> {
        Box::pin(async { SubTaskOutcome::Failed(String::from("sub-tasks are not available yet")) })
    }
}
