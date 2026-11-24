//! Sub-sessions that outlive the main session on this PC. Each runs as a task of the daemon, not
//! of the agent, so when the main session moves to another PC the work goes on, and its result
//! goes to whichever PC runs the main session then, this one included.

use std::sync::Arc;
use std::sync::Mutex;
use std::sync::OnceLock;
use std::time::Duration;

use dotori_core::BoxFuture;
use dotori_core::runtime::Returns;
use dotori_core::sub_session::SubSessionOutcome;
use dotori_core::sub_session::SubSessionRunner;
use dotori_core::sub_session::SubSessionSpec;
use serde_json::Value;
use serde_json::json;
use tokio::sync::oneshot;
use tokio::task::AbortHandle;
use tokio::time;

use crate::main_session::Linked;

/// The kind of message that carries a sub-session's result to the PC running the main session.
pub(crate) const RETURNED: &str = "sub_session_returned";
const RETRY: Duration = Duration::from_secs(1);

/// Runs sub-sessions for the agent here and passes on the results the agent is no longer there to
/// take.
#[derive(Default)]
pub(crate) struct Handoff {
    running: Mutex<Vec<(u64, SubSessionSpec, AbortHandle)>>,
    next: Mutex<u64>,
    current: Mutex<Option<Returns>>,
    linked: OnceLock<Linked>,
}

impl Handoff {
    /// Lets results go to the other PCs once they are linked.
    pub(crate) fn link(&self, linked: Linked) {
        let _ = self.linked.set(linked);
    }

    /// Sets the agent that takes results now, or none while this PC does not run the main session.
    pub(crate) fn serve(&self, returns: Option<Returns>) {
        *self.current.lock().expect("unpoisoned") = returns;
    }

    /// The sub-sessions still running here, for an agent starting here to leave open.
    pub(crate) fn running(&self) -> Vec<SubSessionSpec> {
        self.running
            .lock()
            .expect("unpoisoned")
            .iter()
            .map(|(_, spec, _)| spec.clone())
            .collect()
    }

    /// Ends every sub-session still running, as when dotori stops on this PC.
    pub(crate) fn stop_all(&self) {
        for (_, _, task) in self.running.lock().expect("unpoisoned").drain(..) {
            task.abort();
        }
    }

    /// Gives what came of `spec` to the agent that runs the main session, here or on another PC,
    /// waiting until there is one.
    pub(crate) async fn pass_on(&self, spec: SubSessionSpec, outcome: SubSessionOutcome) {
        loop {
            let here = self.current.lock().expect("unpoisoned").clone();
            if let Some(returns) = here
                && returns.deliver(spec.clone(), outcome.clone()).await
            {
                return;
            }
            if let Some(linked) = self.linked.get()
                && !linked.runs_main()
                && let Some(main) = linked.main_pc()
                && main != linked.own
                && linked.hub.send(&main, message(&spec, &outcome))
            {
                return;
            }
            time::sleep(RETRY).await;
        }
    }

    fn start(
        self: &Arc<Self>,
        spec: SubSessionSpec,
        inner: Arc<dyn SubSessionRunner>,
    ) -> oneshot::Receiver<SubSessionOutcome> {
        let (tx, rx) = oneshot::channel();
        let id = {
            let mut next = self.next.lock().expect("unpoisoned");
            *next += 1;
            *next
        };
        let handoff = Arc::clone(self);
        let running = spec.clone();
        let mut held = self.running.lock().expect("unpoisoned");
        let task = tokio::spawn(async move {
            let outcome = inner.run(spec.clone()).await;
            handoff
                .running
                .lock()
                .expect("unpoisoned")
                .retain(|(known, _, _)| *known != id);
            if let Err(outcome) = tx.send(outcome) {
                handoff.pass_on(spec, outcome).await;
            }
        });
        held.push((id, running, task.abort_handle()));
        rx
    }
}

/// The sub-session runner the agent sees: it starts the real one through `Handoff`.
pub(crate) struct HandedOff {
    pub(crate) handoff: Arc<Handoff>,
    pub(crate) inner: Arc<dyn SubSessionRunner>,
}

impl SubSessionRunner for HandedOff {
    fn run(&self, spec: SubSessionSpec) -> BoxFuture<'_, SubSessionOutcome> {
        let result = self.handoff.start(spec, Arc::clone(&self.inner));
        Box::pin(async move {
            result.await.unwrap_or_else(|_| {
                SubSessionOutcome::Failed(String::from("the sub-session ended without a result"))
            })
        })
    }
}

/// The message that carries a result to another PC.
pub(crate) fn message(spec: &SubSessionSpec, outcome: &SubSessionOutcome) -> Value {
    let (finished, text) = match outcome {
        SubSessionOutcome::Finished(text) => (true, text),
        SubSessionOutcome::Failed(text) => (false, text),
    };
    json!({
        "kind": RETURNED,
        "title": spec.title,
        "brief": spec.brief,
        "folder": spec.folder,
        "finished": finished,
        "text": text,
    })
}

/// The result a message from another PC carries, if it is one.
pub(crate) fn heard(message: &Value) -> Option<(SubSessionSpec, SubSessionOutcome)> {
    if message.get("kind").and_then(Value::as_str) != Some(RETURNED) {
        return None;
    }
    let text = |name: &str| message.get(name).and_then(Value::as_str).map(str::to_owned);
    let spec = SubSessionSpec {
        title: text("title")?,
        brief: text("brief")?,
        folder: text("folder"),
    };
    let said = text("text")?;
    let outcome = if message.get("finished").and_then(Value::as_bool)? {
        SubSessionOutcome::Finished(said)
    } else {
        SubSessionOutcome::Failed(said)
    };
    Some((spec, outcome))
}

#[cfg(test)]
#[path = "handoff.test.rs"]
mod tests;
