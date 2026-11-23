use super::*;

struct Echo;

impl Toolbox for Echo {
    fn specs(&self) -> Vec<ToolSpec> {
        Vec::new()
    }

    fn run(&self, call: ToolCall) -> BoxFuture<'_, Result<String, ToolError>> {
        Box::pin(async move { Ok(format!("here {}", call.name)) })
    }
}

fn call(name: &str) -> ToolCall {
    ToolCall {
        id: ToolCallId::new("1"),
        name: name.to_owned(),
        input: String::from("{}"),
    }
}

#[tokio::test]
async fn alone_every_tool_runs_here() {
    let route = BrowserRoute::new(Arc::new(Echo));

    assert_eq!(
        route.run(call("browser_run")).await.expect("ran"),
        "here browser_run"
    );
    assert_eq!(
        route.run(call("webfetch")).await.expect("ran"),
        "here webfetch"
    );
}

#[tokio::test]
async fn an_answer_from_the_browsers_pc_reaches_the_call_waiting_for_it() {
    let route = BrowserRoute::new(Arc::new(Echo));
    let (tx, rx) = oneshot::channel();
    route.waiting.lock().expect("unpoisoned").insert(7, tx);
    let heard = |message: Value| Heard {
        key: String::from("b"),
        device: String::from("b"),
        message,
    };

    assert!(route.hear(&heard(
        json!({ "kind": "tool_result", "id": 7, "error": "no page" })
    )));
    assert!(!route.hear(&heard(json!({ "kind": "marks" }))));

    assert_eq!(rx.await.expect("answered"), Err(String::from("no page")));
}
