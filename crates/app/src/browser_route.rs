//! Sends the main session's browser tool calls to the PC that holds the browser, so the owner's
//! logins live in one browser. Other tools run here.

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::OnceLock;
use std::sync::atomic::AtomicU64;
use std::sync::atomic::Ordering;
use std::time::Duration;

use dotori_connectors::peer::hub::Heard;
use dotori_core::BoxFuture;
use dotori_core::tool::ToolCall;
use dotori_core::tool::ToolCallId;
use dotori_core::tool::ToolError;
use dotori_core::tool::ToolSpec;
use dotori_core::tool::Toolbox;
use dotori_mcp::browser::CARD_WAIT;
use dotori_mcp::browser::WAITS_FOR_OWNER;
use serde_json::Value;
use serde_json::json;
use tokio::sync::oneshot;
use tokio::time;

use crate::main_session::Linked;

pub(crate) const BROWSER_JOB: &str = "browser";
const ANSWER_WAIT: Duration = Duration::from_secs(150);

type Waiting = HashMap<u64, (String, oneshot::Sender<Result<String, String>>)>;

enum Where<'a> {
    Here,
    Elsewhere(&'a Linked, String),
    Nowhere,
}

/// The shared tools as the main session sees them, with browser calls sent where the browser is.
pub(crate) struct BrowserRoute {
    local: Arc<dyn Toolbox>,
    linked: OnceLock<Linked>,
    waiting: Mutex<Waiting>,
    next: AtomicU64,
    running: Mutex<HashMap<(String, u64), tokio::task::AbortHandle>>,
}

struct Abandoned<'a> {
    linked: &'a Linked,
    pc: &'a str,
    id: u64,
    answered: bool,
}

impl Drop for Abandoned<'_> {
    fn drop(&mut self) {
        if !self.answered {
            self.linked
                .hub
                .send(self.pc, json!({ "kind": "tool_cancel", "id": self.id }));
        }
    }
}

impl BrowserRoute {
    pub(crate) fn new(local: Arc<dyn Toolbox>) -> Arc<Self> {
        Arc::new(Self {
            local,
            linked: OnceLock::new(),
            waiting: Mutex::new(HashMap::new()),
            next: AtomicU64::new(1),
            running: Mutex::new(HashMap::new()),
        })
    }

    /// Starts sending browser calls to other PCs once this PC is linked with them.
    pub(crate) fn link(&self, linked: Linked) {
        let _ = self.linked.set(linked);
    }

    fn browser_pc(&self, name: &str) -> Where<'_> {
        if !dotori_mcp::browser::TOOLS.contains(&name) {
            return Where::Here;
        }
        let Some(linked) = self.linked.get() else {
            return Where::Here;
        };
        match linked
            .keeper
            .watch(BROWSER_JOB)
            .and_then(|holder| holder.borrow().clone())
        {
            Some(holder) if holder == linked.own => Where::Here,
            Some(holder) => Where::Elsewhere(linked, holder),
            None => Where::Nowhere,
        }
    }

    async fn elsewhere(
        &self,
        linked: &Linked,
        pc: &str,
        call: ToolCall,
    ) -> Result<String, ToolError> {
        let wait = if WAITS_FOR_OWNER.contains(&call.name.as_str()) {
            CARD_WAIT + ANSWER_WAIT
        } else {
            ANSWER_WAIT
        };
        let id = self.next.fetch_add(1, Ordering::Relaxed);
        let (tx, rx) = oneshot::channel();
        self.waiting
            .lock()
            .expect("unpoisoned")
            .insert(id, (pc.to_owned(), tx));
        let sent = linked.hub.send(
            pc,
            json!({ "kind": "tool_call", "id": id, "name": call.name, "input": call.input }),
        );
        let failed = |detail: String| ToolError { detail };
        if !sent {
            self.waiting.lock().expect("unpoisoned").remove(&id);
            return Err(failed(format!(
                "the browser is on {}, which cannot be reached now",
                linked.hub.name_of(pc)
            )));
        }
        let mut abandoned = Abandoned {
            linked,
            pc,
            id,
            answered: false,
        };
        let moved = async {
            let Some(mut holder) = linked.keeper.watch(BROWSER_JOB) else {
                return std::future::pending().await;
            };
            let _ = holder
                .wait_for(|holder| holder.as_deref() != Some(pc))
                .await;
        };
        let answer = tokio::select! {
            answer = time::timeout(wait, rx) => match answer {
                Ok(Ok(result)) => {
                    abandoned.answered = true;
                    result.map_err(failed)
                }
                Ok(Err(_)) => Err(failed(String::from("the browser's PC stopped answering"))),
                Err(_) => Err(failed(String::from("the browser's PC did not answer in time"))),
            },
            () = moved => Err(failed(String::from(
                "the browser moved to another PC before it answered; check the page and try again",
            ))),
        };
        self.waiting.lock().expect("unpoisoned").remove(&id);
        answer
    }

    /// Takes a browser call or its answer from another PC. Says whether it was one.
    pub(crate) fn hear(self: &Arc<Self>, heard: &Heard) -> bool {
        match heard.message.get("kind").and_then(Value::as_str) {
            Some("tool_call") => {
                let Some(id) = heard.message.get("id").and_then(Value::as_u64) else {
                    return true;
                };
                let route = Arc::clone(self);
                let call = heard.clone();
                let task = tokio::spawn(async move {
                    let key = (call.key.clone(), id);
                    route.answer(call).await;
                    route.running.lock().expect("unpoisoned").remove(&key);
                });
                if !task.is_finished() {
                    self.running
                        .lock()
                        .expect("unpoisoned")
                        .insert((heard.key.clone(), id), task.abort_handle());
                }
                true
            }
            Some("tool_cancel") => {
                if let Some(id) = heard.message.get("id").and_then(Value::as_u64)
                    && let Some(task) = self
                        .running
                        .lock()
                        .expect("unpoisoned")
                        .remove(&(heard.key.clone(), id))
                {
                    task.abort();
                }
                true
            }
            Some("tool_result") => {
                let Some(id) = heard.message.get("id").and_then(Value::as_u64) else {
                    return true;
                };
                let result = match heard.message.get("error").and_then(Value::as_str) {
                    Some(error) => Err(error.to_owned()),
                    None => Ok(heard
                        .message
                        .get("result")
                        .and_then(Value::as_str)
                        .unwrap_or_default()
                        .to_owned()),
                };
                let mut waiting = self.waiting.lock().expect("unpoisoned");
                if waiting.get(&id).is_some_and(|(pc, _)| *pc == heard.key)
                    && let Some((_, waiter)) = waiting.remove(&id)
                {
                    let _ = waiter.send(result);
                }
                true
            }
            _ => false,
        }
    }

    async fn answer(&self, heard: Heard) {
        let Some(linked) = self.linked.get() else {
            return;
        };
        let message = &heard.message;
        let id = message.get("id").and_then(Value::as_u64).unwrap_or(0);
        let name = message
            .get("name")
            .and_then(Value::as_str)
            .unwrap_or_default();
        let result = if !dotori_mcp::browser::TOOLS.contains(&name) {
            Err(String::from("only browser tools are run for another PC"))
        } else if linked.main_pc().as_deref() != Some(heard.key.as_str()) {
            Err(String::from(
                "only the PC that runs the main session may use the browser",
            ))
        } else if !linked.keeper.holds(BROWSER_JOB) {
            Err(String::from(
                "this PC no longer holds the browser; try again",
            ))
        } else {
            let call = ToolCall {
                id: ToolCallId::new(format!("peer-{id}")),
                name: name.to_owned(),
                input: message
                    .get("input")
                    .and_then(Value::as_str)
                    .unwrap_or("{}")
                    .to_owned(),
            };
            self.local.run(call).await.map_err(|error| error.detail)
        };
        let reply = match result {
            Ok(result) => json!({ "kind": "tool_result", "id": id, "result": result }),
            Err(error) => json!({ "kind": "tool_result", "id": id, "error": error }),
        };
        linked.hub.send(&heard.key, reply);
    }
}

impl Toolbox for BrowserRoute {
    fn specs(&self) -> Vec<ToolSpec> {
        self.local.specs()
    }

    fn run(&self, call: ToolCall) -> BoxFuture<'_, Result<String, ToolError>> {
        Box::pin(async move {
            match self.browser_pc(&call.name) {
                Where::Here => self.local.run(call).await,
                Where::Elsewhere(linked, pc) => self.elsewhere(linked, &pc, call).await,
                Where::Nowhere => Err(ToolError {
                    detail: String::from(
                        "no PC holds the browser right now; try again in a few seconds",
                    ),
                }),
            }
        })
    }
}

#[cfg(test)]
#[path = "browser_route.test.rs"]
mod tests;
