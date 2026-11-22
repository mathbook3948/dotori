//! dotori's MCP server: the tools every runner shares, such as webfetch and the browser, served
//! over HTTP.

pub mod browser;
mod cdp;
pub mod fetch;
pub mod server;

use std::future::Future;
use std::pin::Pin;

use serde_json::Value;

/// A future a tool returns.
pub type ToolFuture<'a> = Pin<Box<dyn Future<Output = Result<String, String>> + Send + 'a>>;

/// One tool the server offers. An `Err` is shown to the model as a failed call, not as a
/// protocol error.
pub trait Tool: Send + Sync + 'static {
    fn name(&self) -> &str;

    fn description(&self) -> &str;

    /// JSON Schema of the arguments object.
    fn input_schema(&self) -> Value;

    fn call(&self, arguments: Value) -> ToolFuture<'_>;
}
