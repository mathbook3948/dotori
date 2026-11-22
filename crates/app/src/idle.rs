use dotori_core::BoxFuture;
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
