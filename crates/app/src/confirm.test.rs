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

    let said = asking.decide(id, true).await.expect("waiting");
    assert!(said.contains("allowed the call to `mail__send`"), "{said}");
    assert!(said.ends_with("ran mail__send"), "{said}");
    assert_eq!(counting.0.load(Ordering::SeqCst), 1);
    assert_eq!(
        shown(&published.recv().await.expect("decided").line),
        ToWindow::Decided { id, ok: true }
    );
    assert_eq!(asking.decide(id, true).await, None);
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

    let said = asking.decide(id, false).await.expect("waiting");

    assert!(said.contains("declined"), "{said}");
    assert_eq!(counting.0.load(Ordering::SeqCst), 0);
    assert_eq!(asking.decide(id + 1, true).await, None);
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
