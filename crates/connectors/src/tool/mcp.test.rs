use dotori_core::tool::ToolCallId;
use dotori_mcp::Tool;
use dotori_mcp::ToolFuture;
use dotori_mcp::server::PATH;
use dotori_mcp::server::Server;
use tokio::net::TcpListener;

use super::*;

struct Echo;

impl Tool for Echo {
    fn name(&self) -> &str {
        "echo"
    }

    fn description(&self) -> &str {
        "Says the text back."
    }

    fn input_schema(&self) -> Value {
        json!({ "type": "object", "properties": { "text": { "type": "string" } } })
    }

    fn call(&self, arguments: Value) -> ToolFuture<'_> {
        Box::pin(async move {
            match arguments.get("text").and_then(Value::as_str) {
                Some(text) => Ok(text.to_owned()),
                None => Err(String::from("no text")),
            }
        })
    }
}

async fn start() -> String {
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bound");
    let address = listener.local_addr().expect("address");
    tokio::spawn(Server::new("secret", vec![Box::new(Echo)]).serve(listener));
    format!("http://{address}{PATH}")
}

fn call(input: &str) -> ToolCall {
    ToolCall {
        id: ToolCallId::new("t1"),
        name: String::from("echo"),
        input: input.to_owned(),
    }
}

#[tokio::test]
async fn the_servers_tools_become_specs_and_run() {
    let tools = McpTools::connect(start().await, "secret")
        .await
        .expect("connected");
    let specs = tools.specs();
    assert_eq!(specs.len(), 1);
    assert_eq!(specs[0].name, "echo");
    assert_eq!(specs[0].description, "Says the text back.");
    let schema: Value = serde_json::from_str(&specs[0].input_schema).expect("json");
    assert_eq!(schema["type"], "object");

    assert_eq!(
        tools.run(call(r#"{"text":"안녕"}"#)).await.expect("ran"),
        "안녕"
    );
    assert_eq!(
        tools.run(call("{}")).await.expect_err("failed").detail,
        "no text"
    );
    assert!(tools.run(call("not json")).await.is_err());
}

#[tokio::test]
async fn a_wrong_token_fails_to_connect() {
    let refused = McpTools::connect(start().await, "wrong")
        .await
        .expect_err("refused");
    assert!(refused.contains("401"), "{refused}");
}

#[test]
fn a_streamed_answer_is_found_by_its_id() {
    let stream = "event: message\ndata: {\"jsonrpc\":\"2.0\",\"method\":\"notifications/progress\"}\n\nevent: message\ndata: {\"jsonrpc\":\"2.0\",\"id\":7,\ndata: \"result\":{}}\n\n";
    assert_eq!(
        from_events(stream, 7),
        Some(json!({"jsonrpc":"2.0","id":7,"result":{}}))
    );
    let simple = "data: {\"jsonrpc\":\"2.0\",\"id\":7,\"result\":{\"ok\":true}}\n\n";
    assert_eq!(from_events(simple, 7).expect("found")["result"]["ok"], true);
    assert!(from_events(simple, 8).is_none());
}
