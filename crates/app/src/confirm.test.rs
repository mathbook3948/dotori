use std::sync::atomic::AtomicUsize;

use dotori_connectors::channel::app::ToWindow;
use dotori_core::tool::ToolCallId;

use super::*;
use crate::settings::Settings;

struct Counting(AtomicUsize);

impl Toolbox for Counting {
    fn specs(&self) -> Vec<ToolSpec> {
        Vec::new()
    }

    fn run(&self, call: ToolCall) -> BoxFuture<'_, Result<String, ToolError>> {
        self.0.fetch_add(1, Ordering::SeqCst);
        Box::pin(async move { Ok(format!("ran {}", call.name)) })
    }
}

fn asking() -> (Asking, Arc<Counting>, Arc<AppChannel>) {
    let settings: Settings =
        toml::from_str("[[mcp]]\nname = \"mail\"\ncommand = \"mail-mcp\"\nconfirm = [\"send\"]")
            .expect("settings");
    let counting = Arc::new(Counting(AtomicUsize::new(0)));
    let (app, _) = AppChannel::bind("token").expect("bound");
    let app = Arc::new(app);
    let asking = Asking::new(
        settings.asked(),
        Arc::clone(&counting) as Arc<dyn Toolbox>,
        Arc::clone(&app),
    );
    (asking, counting, app)
}

fn call(name: &str) -> ToolCall {
    ToolCall {
        id: ToolCallId::new("c"),
        name: name.to_owned(),
        input: String::from(r#"{"to":"boss@example.com","text":"I quit"}"#),
    }
}

fn shown(line: &str) -> ToWindow {
    serde_json::from_str(line).expect("a window line")
}

#[tokio::test]
async fn a_call_the_owner_did_not_list_runs_at_once() {
    let (asking, counting, _) = asking();

    assert_eq!(
        asking.run(call("mail__read")).await.expect("ran"),
        "ran mail__read"
    );
    assert_eq!(counting.0.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn a_listed_call_waits_and_runs_only_once_the_owner_allows_it() {
    let (asking, counting, app) = asking();
    let mut published = app.published();

    let waited = asking.run(call("mail__send")).await.expect("waits");

    assert_eq!(waited, WAITING);
    assert_eq!(counting.0.load(Ordering::SeqCst), 0);
    let ToWindow::Ask {
        id, tool, input, ..
    } = shown(&published.recv().await.expect("asked").line)
    else {
        panic!("an ask first");
    };
    assert_eq!(tool, "mail__send");
    assert!(input.contains("I quit"));

    let outcome = asking.decide(id, true).await;
    assert_eq!(outcome.tool, "mail__send");
    assert!(
        outcome.text.contains("not the owner's words"),
        "{}",
        outcome.text
    );
    assert!(outcome.text.ends_with("ran mail__send"), "{}", outcome.text);
    assert_eq!(counting.0.load(Ordering::SeqCst), 1);
    assert_eq!(
        shown(&published.recv().await.expect("decided").line),
        ToWindow::Decided { id, ok: true }
    );

    let again = asking.decide(id, true).await;
    assert!(again.text.contains("no longer waits"), "{}", again.text);
    assert_eq!(
        shown(&published.recv().await.expect("lapsed").line),
        ToWindow::Lapsed { id }
    );
    assert_eq!(counting.0.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn a_declined_call_never_runs() {
    let (asking, counting, app) = asking();
    let mut published = app.published();
    asking.run(call("mail__send")).await.expect("waits");
    let ToWindow::Ask { id, .. } = shown(&published.recv().await.expect("asked").line) else {
        panic!("an ask first");
    };

    let outcome = asking.decide(id, false).await;

    assert!(outcome.text.contains("declined"), "{}", outcome.text);
    assert_eq!(counting.0.load(Ordering::SeqCst), 0);
}

#[test]
fn a_decision_passed_on_by_another_pc_reads_back() {
    let from = |message: Value| Heard {
        key: String::from("k"),
        device: String::from("d"),
        message,
    };

    assert_eq!(
        heard(&from(json!({ "kind": "decide", "id": 7, "ok": true }))),
        Some((7, true))
    );
    assert_eq!(heard(&from(json!({ "kind": "decide", "id": 7 }))), None);
    assert_eq!(heard(&from(json!({ "kind": "notes" }))), None);
}

struct Mail;

impl Tool for Mail {
    fn name(&self) -> &str {
        "mail__send"
    }

    fn description(&self) -> &str {
        "Sends mail."
    }

    fn input_schema(&self) -> Value {
        json!({ "type": "object" })
    }

    fn call(&self, arguments: Value) -> dotori_mcp::ToolFuture<'_> {
        Box::pin(async move { Ok(format!("sent {arguments}")) })
    }
}

#[tokio::test]
async fn guarded_tools_join_the_others_and_run_where_they_are() {
    let counting = Arc::new(Counting(AtomicUsize::new(0)));
    let guarded = Guarded {
        tools: vec![Box::new(Mail)],
        rest: Arc::clone(&counting) as Arc<dyn Toolbox>,
    };

    let names: Vec<String> = guarded.specs().into_iter().map(|spec| spec.name).collect();
    assert_eq!(names, ["mail__send"]);
    assert_eq!(
        guarded.run(call("mail__send")).await.expect("ran"),
        r#"sent {"to":"boss@example.com","text":"I quit"}"#
    );
    assert_eq!(counting.0.load(Ordering::SeqCst), 0);
    assert_eq!(
        guarded.run(call("webfetch")).await.expect("ran"),
        "ran webfetch"
    );
}

#[test]
fn what_came_of_a_call_arrives_as_data_not_as_the_owner() {
    let outcome = Outcome {
        id: 3,
        tool: String::from("mail__send"),
        text: String::from("The owner declined this call, so it did not run."),
    };

    let incoming = incoming(&outcome).expect("a message");

    assert_eq!(incoming.id.as_str(), "decision-3");
    assert_eq!(incoming.place.as_str(), "mail__send");
    assert_eq!(incoming.sender_id.as_str(), DECISION);
}
