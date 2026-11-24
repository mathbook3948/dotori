//! Sub-sessions that outlive the main session on this PC. Each runs as a task of the daemon, not
//! of the agent, so when the main session moves to another PC the work goes on, and its result
//! goes to whichever PC runs the main session then, this one included.

use std::sync::Arc;
use std::sync::Mutex;
use std::sync::OnceLock;
use std::sync::atomic::AtomicBool;
use std::sync::atomic::AtomicU64;
use std::sync::atomic::Ordering;
use std::time::Duration;
use std::time::SystemTime;

use dotori_core::BoxFuture;
use dotori_core::conversation::Entry;
use dotori_core::record::Record;
use dotori_core::record::RecordStore;
use dotori_core::runtime::Delivered;
use dotori_core::runtime::Returns;
use dotori_core::sub_session::SubSessionOutcome;
use dotori_core::sub_session::SubSessionRunner;
use dotori_core::sub_session::SubSessionSpec;
use serde_json::Value;
use serde_json::json;
use tokio::task::AbortHandle;
use tokio::time;

use crate::main_session::Linked;

/// The kind of message that carries a sub-session's result to the PC running the main session.
pub(crate) const RETURNED: &str = "sub_session_returned";
const RETRY: Duration = Duration::from_secs(1);
const FORWARD_TRIES: usize = 120;

/// Runs sub-sessions as tasks of the daemon and brings each result to the agent that runs the
/// main session: the one here, or, once this PC no longer runs it, the one on the PC that does,
/// after recording the result here as the PC that started it.
#[derive(Default)]
pub(crate) struct Handoff {
    running: Mutex<Vec<(u64, SubSessionSpec, AbortHandle)>>,
    next: AtomicU64,
    current: Mutex<Option<Returns>>,
    peers: OnceLock<(Linked, Arc<dyn RecordStore>)>,
    quitting: AtomicBool,
}

impl Handoff {
    /// Lets results go to the other PCs once they are linked, recording them in `records` first.
    pub(crate) fn link(&self, linked: Linked, records: Arc<dyn RecordStore>) {
        let _ = self.peers.set((linked, records));
    }

    /// Sets the agent that takes results now, or none while this PC does not run the main session.
    pub(crate) fn serve(&self, returns: Option<Returns>) {
        *self.current.lock().expect("unpoisoned") = returns;
    }

    /// The sub-sessions whose results have not reached an agent or the record yet, for an agent
    /// starting here to leave open.
    pub(crate) fn running(&self) -> Vec<SubSessionSpec> {
        self.running
            .lock()
            .expect("unpoisoned")
            .iter()
            .map(|(_, spec, _)| spec.clone())
            .collect()
    }

    /// Stops passing results on, as dotori stops on this PC: the agent records what is left as
    /// stopped. Call before the agent shuts down.
    pub(crate) fn quit(&self) {
        self.quitting.store(true, Ordering::SeqCst);
    }

    /// Ends every sub-session still running. Call after the agent shut down.
    pub(crate) fn stop_all(&self) {
        for (_, _, task) in self.running.lock().expect("unpoisoned").drain(..) {
            task.abort();
        }
    }

    fn done(&self, id: u64) {
        self.running
            .lock()
            .expect("unpoisoned")
            .retain(|(known, _, _)| *known != id);
    }

    fn current(&self) -> Option<Returns> {
        self.current.lock().expect("unpoisoned").clone()
    }

    async fn settle(&self, id: u64, spec: SubSessionSpec, outcome: SubSessionOutcome) {
        let mut recorded = false;
        loop {
            if self.quitting.load(Ordering::SeqCst) {
                return;
            }
            if let Some(returns) = self.current() {
                match returns
                    .deliver(spec.clone(), outcome.clone(), !recorded)
                    .await
                {
                    Delivered::Taken | Delivered::NotOpen => {
                        self.done(id);
                        return;
                    }
                    Delivered::Gone => {}
                }
            } else if let Some((linked, records)) = self.peers.get()
                && !linked.runs_main()
            {
                if !recorded {
                    let entry = Entry::SubSessionReturned {
                        spec: spec.clone(),
                        outcome: outcome.clone(),
                    };
                    let at = SystemTime::now();
                    if records.append(vec![Record { at, entry }]).await.is_ok() {
                        recorded = true;
                        self.done(id);
                    }
                }
                if recorded
                    && let Some(main) = linked.main_pc()
                    && main != linked.own
                    && linked.hub.send(&main, message(&spec, &outcome))
                {
                    return;
                }
            }
            time::sleep(RETRY).await;
        }
    }

    /// Shows a result another PC recorded to the agent running the main session, here or on the
    /// PC that runs it now, for a while.
    pub(crate) async fn show(&self, spec: SubSessionSpec, outcome: SubSessionOutcome) {
        for _ in 0..FORWARD_TRIES {
            if let Some(returns) = self.current() {
                match returns.deliver(spec.clone(), outcome.clone(), false).await {
                    Delivered::Taken | Delivered::NotOpen => return,
                    Delivered::Gone => {}
                }
            } else if let Some((linked, _)) = self.peers.get()
                && let Some(main) = linked.main_pc()
                && main != linked.own
                && linked.hub.send(&main, message(&spec, &outcome))
            {
                return;
            }
            time::sleep(RETRY).await;
        }
    }

    fn start(self: &Arc<Self>, spec: SubSessionSpec, inner: Arc<dyn SubSessionRunner>) {
        let id = self.next.fetch_add(1, Ordering::SeqCst);
        let handoff = Arc::clone(self);
        let running = spec.clone();
        let mut held = self.running.lock().expect("unpoisoned");
        let task = tokio::spawn(async move {
            let ran = {
                let spec = spec.clone();
                tokio::spawn(async move { inner.run(spec).await }).await
            };
            let outcome = ran.unwrap_or_else(|error| {
                SubSessionOutcome::Failed(format!("the sub-session broke down: {error}"))
            });
            handoff.settle(id, spec, outcome).await;
        });
        held.push((id, running, task.abort_handle()));
    }
}

/// The sub-session runner the agent sees. It starts the real one through `Handoff`, whose result
/// reaches the agent through `Returns`, so the future the agent waits on never ends by itself.
pub(crate) struct HandedOff {
    pub(crate) handoff: Arc<Handoff>,
    pub(crate) inner: Arc<dyn SubSessionRunner>,
}

impl SubSessionRunner for HandedOff {
    fn run(&self, spec: SubSessionSpec) -> BoxFuture<'_, SubSessionOutcome> {
        self.handoff.start(spec, Arc::clone(&self.inner));
        Box::pin(std::future::pending())
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
