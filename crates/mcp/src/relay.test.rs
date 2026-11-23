use tokio::io::AsyncReadExt;
use tokio::net::TcpListener;

use super::*;

const FAKE: &str = r#"
import json, sys
sys.stdin.reconfigure(encoding="utf-8")
sys.stdout.reconfigure(encoding="utf-8")
for line in sys.stdin:
    message = json.loads(line)
    if "id" not in message:
        continue
    method = message["method"]
    if method == "initialize":
        result = {"protocolVersion": "2025-11-25", "capabilities": {"tools": {}}, "serverInfo": {"name": "fake"}}
    elif method == "tools/list" and "cursor" not in message.get("params", {}):
        result = {"tools": [{"name": "echo", "description": "Says it back.", "inputSchema": {"type": "object"}}], "nextCursor": "2"}
    elif method == "tools/list":
        result = {"tools": [{"name": "send_mail", "description": "Sends mail.", "inputSchema": {"type": "object"}}, {"name": "bad name!"}]}
    elif method == "tools/call" and message["params"]["name"] == "quit":
        sys.exit(0)
    elif method == "tools/call" and message["params"]["name"] == "echo":
        result = {"content": [{"type": "text", "text": "echo " + json.dumps(message["params"]["arguments"])}]}
    else:
        result = {"content": [{"type": "text", "text": "refused"}], "isError": True}
    sys.stdout.write(json.dumps({"jsonrpc": "2.0", "id": message["id"], "result": result}) + "\n")
    sys.stdout.flush()
"#;

fn python() -> Option<String> {
    ["python3", "python"]
        .into_iter()
        .map(str::to_owned)
        .find(|name| {
            std::process::Command::new(name)
                .arg("--version")
                .output()
                .is_ok_and(|output| output.status.success())
        })
}

#[tokio::test]
async fn a_command_servers_tools_are_relayed_under_its_name() {
    let Some(python) = python() else {
        return;
    };
    let config = RelayConfig {
        name: String::from("mail"),
        reach: Reach::Command {
            program: python,
            args: vec![String::from("-c"), String::from(FAKE)],
        },
        tools: None,
    };

    let (tools, failed) = relay(vec![config.clone()]).await;

    assert!(failed.is_empty(), "{failed:?}");
    let names: Vec<&str> = tools.iter().map(|tool| tool.name()).collect();
    assert_eq!(names, ["mail__echo", "mail__send_mail"]);
    assert!(tools[0].description().contains("from mail"));
    let said = tools[0]
        .call(json!({ "word": "도토리" }))
        .await
        .expect("called");
    assert_eq!(said, "echo {\"word\": \"\\ub3c4\\ud1a0\\ub9ac\"}");
    let refused = tools[1].call(json!({})).await.expect_err("an error result");
    assert_eq!(refused, "refused");

    let (only, _) = relay(vec![RelayConfig {
        tools: Some(vec![String::from("echo")]),
        ..config
    }])
    .await;
    assert_eq!(only.len(), 1);
}

#[tokio::test]
async fn a_call_the_server_died_on_is_not_sent_again_but_the_next_one_restarts_it() {
    let Some(python) = python() else {
        return;
    };
    let config = RelayConfig {
        name: String::from("mail"),
        reach: Reach::Command {
            program: python,
            args: vec![String::from("-c"), String::from(FAKE)],
        },
        tools: None,
    };
    let remote = Remote::connect(config).await.expect("connected");

    let died = remote.call("quit", json!({})).await.expect_err("died");
    let again = remote.call("echo", json!({})).await.expect("restarted");

    assert_eq!(died, "the server stopped");
    assert_eq!(again, "echo {}");
}

#[tokio::test]
async fn two_servers_with_one_name_relay_the_first_ones_tools() {
    let Some(python) = python() else {
        return;
    };
    let config = RelayConfig {
        name: String::from("mail"),
        reach: Reach::Command {
            program: python,
            args: vec![String::from("-c"), String::from(FAKE)],
        },
        tools: None,
    };

    let (tools, failed) = relay(vec![config.clone(), config]).await;

    assert_eq!(tools.len(), 2);
    assert_eq!(failed.len(), 2, "{failed:?}");
}

#[tokio::test]
async fn a_server_that_cannot_start_is_named_and_left_out() {
    let (tools, failed) = relay(vec![RelayConfig {
        name: String::from("ghost"),
        reach: Reach::Command {
            program: String::from("/nonexistent/dotori-mcp-ghost"),
            args: Vec::new(),
        },
        tools: None,
    }])
    .await;

    assert!(tools.is_empty());
    assert!(failed[0].starts_with("ghost: cannot start"), "{failed:?}");
}

async fn http_server(
    token: &'static str,
) -> (String, tokio::sync::mpsc::UnboundedReceiver<String>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bound");
    let address = listener.local_addr().expect("address");
    let (seen_tx, seen) = tokio::sync::mpsc::unbounded_channel();
    tokio::spawn(async move {
        loop {
            let Ok((mut stream, _)) = listener.accept().await else {
                return;
            };
            let mut read = Vec::new();
            let mut buffer = [0u8; 8192];
            let (head, body) = loop {
                let n = stream.read(&mut buffer).await.expect("read");
                read.extend_from_slice(&buffer[..n]);
                let text = String::from_utf8_lossy(&read).into_owned();
                if let Some((head, body)) = text.split_once("\r\n\r\n") {
                    let length = head
                        .lines()
                        .find_map(|line| {
                            line.to_ascii_lowercase()
                                .strip_prefix("content-length:")
                                .map(|n| n.trim().parse::<usize>().unwrap_or(0))
                        })
                        .unwrap_or(0);
                    if body.len() >= length {
                        break (head.to_owned(), body.to_owned());
                    }
                }
            };
            let _ = seen_tx.send(head.clone());
            let message: Value = serde_json::from_str(&body).expect("json");
            let authorized = head
                .to_ascii_lowercase()
                .contains(&format!("authorization: bearer {token}"));
            let reply = match (authorized, message.get("id"), message.get("method").and_then(Value::as_str)) {
                (false, _, _) => (401, String::new(), "application/json"),
                (true, None, _) => (202, String::new(), "application/json"),
                (true, Some(id), Some("initialize")) => (
                    200,
                    format!("event: message\ndata: {}\n\n", json!({ "jsonrpc": "2.0", "id": id, "result": { "protocolVersion": "2025-06-18" } })),
                    "text/event-stream",
                ),
                (true, Some(id), Some("tools/list")) => (
                    200,
                    json!({ "jsonrpc": "2.0", "id": id, "result": { "tools": [{ "name": "find", "inputSchema": { "type": "object" } }] } }).to_string(),
                    "application/json",
                ),
                (true, Some(id), _) => (
                    200,
                    json!({ "jsonrpc": "2.0", "id": id, "result": { "content": [{ "type": "text", "text": "found it" }] } }).to_string(),
                    "application/json",
                ),
            };
            let response = format!(
                "HTTP/1.1 {} OK\r\ncontent-type: {}\r\nmcp-session-id: s1\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{}",
                reply.0,
                reply.2,
                reply.1.len(),
                reply.1
            );
            let _ = tokio::io::AsyncWriteExt::write_all(&mut stream, response.as_bytes()).await;
        }
    });
    (format!("http://{address}/mcp"), seen)
}

#[tokio::test]
async fn an_http_servers_tools_are_relayed_with_its_token_and_session() {
    let (url, mut seen) = http_server("sesame").await;

    let (tools, failed) = relay(vec![RelayConfig {
        name: String::from("search"),
        reach: Reach::Http {
            url: url.clone(),
            token: Some(String::from("sesame")),
        },
        tools: None,
    }])
    .await;

    assert!(failed.is_empty(), "{failed:?}");
    assert_eq!(tools[0].name(), "search__find");
    assert_eq!(tools[0].call(json!({})).await.expect("called"), "found it");
    let mut heads = Vec::new();
    while let Ok(head) = seen.try_recv() {
        heads.push(head.to_ascii_lowercase());
    }
    assert!(heads.last().expect("a call").contains("mcp-session-id: s1"));
    assert!(
        heads
            .last()
            .expect("a call")
            .contains("mcp-protocol-version: 2025-06-18")
    );

    let (none, failed) = relay(vec![RelayConfig {
        name: String::from("search"),
        reach: Reach::Http {
            url,
            token: Some(String::from("wrong")),
        },
        tools: None,
    }])
    .await;
    assert!(none.is_empty());
    assert!(failed[0].contains("401"), "{failed:?}");
}

#[test]
fn relayed_names_stay_within_what_mcp_allows() {
    assert_eq!(
        relayed_name("gh", "search_repos").as_deref(),
        Some("gh__search_repos")
    );
    assert_eq!(relayed_name("gh", "bad name!"), None);
    assert_eq!(relayed_name("", "x"), None);
    assert_eq!(relayed_name("a", &"x".repeat(70)), None);
}

#[tokio::test]
async fn a_server_named_outside_what_tool_names_allow_is_refused_at_once() {
    let (tools, failed) = relay(vec![RelayConfig {
        name: String::from("my notes"),
        reach: Reach::Command {
            program: String::from("/nonexistent/dotori-mcp-ghost"),
            args: Vec::new(),
        },
        tools: None,
    }])
    .await;

    assert!(tools.is_empty());
    assert!(failed[0].contains("only letters"), "{failed:?}");
}
