use serde_json::Value;
use serde_json::json;
use tokio::net::TcpListener;

use super::*;
use crate::ToolFuture;

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

fn server() -> Server {
    Server::new("secret", vec![Box::new(Echo)])
}

async fn answer(message: Value) -> Option<Value> {
    server().answer(message.to_string().as_bytes()).await
}

#[tokio::test]
async fn initialize_agrees_on_a_version_it_speaks() {
    let reply = answer(json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-06-18"}}))
        .await
        .expect("a reply");
    assert_eq!(reply["id"], 1);
    assert_eq!(reply["result"]["protocolVersion"], "2025-06-18");
    assert!(reply["result"]["capabilities"]["tools"].is_object());
    let unknown = answer(json!({"jsonrpc":"2.0","id":2,"method":"initialize","params":{"protocolVersion":"1999-01-01"}}))
        .await
        .expect("a reply");
    assert_eq!(unknown["result"]["protocolVersion"], VERSIONS[0]);
}

#[tokio::test]
async fn tools_are_listed_and_called() {
    let list = answer(json!({"jsonrpc":"2.0","id":"a","method":"tools/list"}))
        .await
        .expect("a reply");
    assert_eq!(list["result"]["tools"][0]["name"], "echo");
    assert_eq!(list["result"]["tools"][0]["inputSchema"]["type"], "object");

    let called = answer(json!({"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"echo","arguments":{"text":"안녕"}}}))
        .await
        .expect("a reply");
    assert_eq!(
        called["result"],
        json!({ "content": [{ "type": "text", "text": "안녕" }], "isError": false })
    );

    let failed = answer(json!({"jsonrpc":"2.0","id":4,"method":"tools/call","params":{"name":"echo","arguments":{}}}))
        .await
        .expect("a reply");
    assert_eq!(failed["result"]["isError"], true);
    assert_eq!(failed["result"]["content"][0]["text"], "no text");
}

#[tokio::test]
async fn mistakes_are_protocol_errors_and_notifications_get_no_reply() {
    let unknown_tool =
        answer(json!({"jsonrpc":"2.0","id":5,"method":"tools/call","params":{"name":"nope"}}))
            .await
            .expect("a reply");
    assert_eq!(unknown_tool["error"]["code"], -32602);
    let unknown_method = answer(json!({"jsonrpc":"2.0","id":6,"method":"resources/list"}))
        .await
        .expect("a reply");
    assert_eq!(unknown_method["error"]["code"], -32601);
    assert!(
        answer(json!({"jsonrpc":"2.0","method":"notifications/initialized"}))
            .await
            .is_none()
    );
    let garbage = server().answer(b"{not json").await.expect("a reply");
    assert_eq!(garbage["error"]["code"], -32700);
}

#[tokio::test]
async fn http_needs_the_token_the_path_and_a_local_origin() {
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bound");
    let address = listener.local_addr().expect("address");
    tokio::spawn(server().serve(listener));
    let client = reqwest::Client::new();
    let url = format!("http://{address}{PATH}");
    let ping = json!({"jsonrpc":"2.0","id":1,"method":"ping"}).to_string();
    let send = |request: reqwest::RequestBuilder| {
        let ping = ping.clone();
        async move {
            request
                .body(ping)
                .send()
                .await
                .expect("sent")
                .status()
                .as_u16()
        }
    };

    assert_eq!(send(client.post(&url)).await, 401);
    assert_eq!(send(client.post(&url).bearer_auth("wrong")).await, 401);
    assert_eq!(
        send(
            client
                .post(format!("http://{address}/other"))
                .bearer_auth("secret")
        )
        .await,
        404
    );
    assert_eq!(send(client.get(&url).bearer_auth("secret")).await, 405);
    assert_eq!(
        send(
            client
                .post(&url)
                .bearer_auth("secret")
                .header("mcp-protocol-version", "1999-01-01")
        )
        .await,
        400
    );
    assert_eq!(
        send(
            client
                .post(&url)
                .bearer_auth("secret")
                .header("origin", "https://evil.example")
        )
        .await,
        403
    );
    assert_eq!(
        send(
            client
                .post(&url)
                .bearer_auth("secret")
                .header("origin", "http://localhost:3000")
        )
        .await,
        200
    );
    let reply = client
        .post(&url)
        .bearer_auth("secret")
        .body(ping.clone())
        .send()
        .await
        .expect("sent")
        .text()
        .await
        .expect("text");
    let reply: Value = serde_json::from_str(&reply).expect("json");
    assert_eq!(reply, json!({"jsonrpc":"2.0","id":1,"result":{}}));
    let notified = client
        .post(&url)
        .bearer_auth("secret")
        .body(json!({"jsonrpc":"2.0","method":"notifications/initialized"}).to_string())
        .send()
        .await
        .expect("sent");
    assert_eq!(notified.status().as_u16(), 202);
}
