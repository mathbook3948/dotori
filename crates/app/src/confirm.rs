//! Calls that act toward other people wait for the owner. dotori asks in the chat window, and
//! only the owner's click there lets such a call run. The model can never answer for the owner.

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::atomic::AtomicU64;
use std::sync::atomic::Ordering;
use std::time::SystemTime;
use std::time::UNIX_EPOCH;

use dotori_connectors::channel::app::AppChannel;
use dotori_connectors::peer::hub::Heard;
use dotori_core::BoxFuture;
use dotori_core::channel::Inbox;
use dotori_core::tool::ToolCall;
use dotori_core::tool::ToolError;
use dotori_core::tool::ToolSpec;
use dotori_core::tool::Toolbox;
use serde_json::Value;
use serde_json::json;

use crate::main_session::Linked;
use crate::settings::Asked;

const WAITING: &str = "This call acts toward other people, so it waits for the owner: the chat \
                       window shows it with 허락 and 거절. What the owner decides, and the result \
                       if it ran, arrive as a message from the owner. Do not call it again.";

/// The tools the main session uses, where the owner must allow each call the settings name.
pub(crate) struct Asking {
    asked: Asked,
    inner: Arc<dyn Toolbox>,
    app: Arc<AppChannel>,
    waiting: Mutex<HashMap<u64, ToolCall>>,
    next: AtomicU64,
}

impl Asking {
    pub(crate) fn new(asked: Asked, inner: Arc<dyn Toolbox>, app: Arc<AppChannel>) -> Self {
        let start = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|since| u64::try_from(since.as_millis()).unwrap_or(u64::MAX / 2))
            .unwrap_or(0)
            .saturating_mul(1000);
        Self {
            asked,
            inner,
            app,
            waiting: Mutex::new(HashMap::new()),
            next: AtomicU64::new(start),
        }
    }

    /// Runs or drops the call `id` as the owner decided, and says what to tell the main session.
    /// Gives `None` for a call that does not wait here, such as one decided already.
    pub(crate) async fn decide(&self, id: u64, ok: bool) -> Option<String> {
        let call = self
            .waiting
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .remove(&id)?;
        self.app.decided(id, ok);
        let name = call.name.clone();
        if !ok {
            return Some(format!(
                "The owner declined the call to `{name}`, so it did not run."
            ));
        }
        Some(match self.inner.run(call).await {
            Ok(output) => format!("The owner allowed the call to `{name}`. It returned:\n{output}"),
            Err(error) => format!(
                "The owner allowed the call to `{name}`, but it failed: {}",
                error.detail
            ),
        })
    }
}

impl Toolbox for Asking {
    fn specs(&self) -> Vec<ToolSpec> {
        self.inner.specs()
    }

    fn run(&self, call: ToolCall) -> BoxFuture<'_, Result<String, ToolError>> {
        if !self.asked.needs(&call.name) {
            return self.inner.run(call);
        }
        let id = self.next.fetch_add(1, Ordering::SeqCst);
        self.app.ask(id, call.name.clone(), call.input.clone());
        self.waiting
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .insert(id, call);
        Box::pin(async { Ok(String::from(WAITING)) })
    }
}

/// Takes what the owner decided in a window here or on another PC. The PC that runs the main
/// session runs the call; any other PC passes the decision on to it.
pub(crate) async fn decided(
    asking: &Asking,
    app: &AppChannel,
    route: &dyn Inbox,
    linked: Option<&Linked>,
    id: u64,
    ok: bool,
) {
    if let Some(linked) = linked
        && !linked.runs_main()
    {
        if let Some(main) = linked.main_pc() {
            linked
                .hub
                .send(&main, json!({ "kind": "decide", "id": id, "ok": ok }));
        }
        return;
    }
    let Some(said) = asking.decide(id, ok).await else {
        return;
    };
    if let Some(incoming) = app.incoming(said) {
        let _ = route.push(incoming).await;
    }
}

/// A decision another PC passed on, if `heard` is one.
pub(crate) fn heard(heard: &Heard) -> Option<(u64, bool)> {
    if heard.message.get("kind").and_then(Value::as_str) != Some("decide") {
        return None;
    }
    Some((
        heard.message.get("id").and_then(Value::as_u64)?,
        heard.message.get("ok").and_then(Value::as_bool)?,
    ))
}

#[cfg(test)]
#[path = "confirm.test.rs"]
mod tests;
