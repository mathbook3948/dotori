//! Calls that act toward other people wait for the owner. dotori asks in the chat window, and in
//! the chat room the owner asked in, and only the owner's click lets such a call run. The model
//! can never answer for the owner.
//! The servers with such tools are relayed by the daemon alone, so only the main session, and
//! only through this gate, reaches them; sub-sessions never see them.

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::atomic::AtomicU64;
use std::sync::atomic::Ordering;
use std::time::Duration;
use std::time::SystemTime;
use std::time::UNIX_EPOCH;

use dotori_connectors::channel::app::AppChannel;
use dotori_connectors::peer::hub::Heard;
use dotori_connectors::sandbox::Sandbox;
use dotori_connectors::tool::basic::RUN_SHELL;
use dotori_core::BoxFuture;
use dotori_core::channel::Channel;
use dotori_core::channel::ChannelError;
use dotori_core::channel::Inbox;
use dotori_core::id::ChannelId;
use dotori_core::id::ExternalMessageId;
use dotori_core::id::ExternalUserId;
use dotori_core::message::Audience;
use dotori_core::message::Content;
use dotori_core::message::Incoming;
use dotori_core::message::Place;
use dotori_core::tool::ToolCall;
use dotori_core::tool::ToolError;
use dotori_core::tool::ToolSpec;
use dotori_core::tool::Toolbox;
use dotori_core::tool::origin;
use dotori_mcp::Tool;
use dotori_mcp::browser::OWNER_TOOLS;
use dotori_mcp::browser::WAITS_FOR_OWNER;
use serde_json::Value;
use serde_json::json;
use tokio::sync::mpsc;
use tokio::time;

use crate::main_session::Linked;
use crate::room_cards::RoomCards;
use crate::settings::Asked;

pub(crate) use dotori_connectors::channel::DECISION;
const WAITING: &str = "This call waits for the owner, as the owner set it or as every call of \
                       this tool does: the chat window shows it with 허락 and 거절, and so does Slack, \
                       Telegram or Discord when the owner asked there. The card shows the owner all of \
                       the call, so do not describe it or guess its result. What the owner decides, and \
                       the result if it ran, arrive on the `decision` channel. Do not call it again.";
const RUN_FOR: Duration = Duration::from_secs(120);
const CARD_SHOWN: &str = "dotori is showing the owner a card in the chat window for this call, and \
                          may show it in the Slack, Telegram or Discord room the owner asked in too. The \
                          card shows the owner all they need, so do not describe it. Do not call it again: \
                          what the owner does with the card, or why it could not be shown, arrives on the \
                          `decision` channel.";

/// Where a call that waited for the owner stands.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Status {
    Waiting,
    Decided(bool),
    Lapsed,
}

/// What came of a call the owner decided about, for the main session.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Outcome {
    pub(crate) id: u64,
    pub(crate) tool: String,
    pub(crate) text: String,
}

/// The tools of the servers only the daemon relays, ahead of the other tools.
pub(crate) struct Guarded {
    pub(crate) tools: Vec<Box<dyn Tool>>,
    pub(crate) rest: Arc<dyn Toolbox>,
}

impl Toolbox for Guarded {
    fn specs(&self) -> Vec<ToolSpec> {
        let mut specs = self.rest.specs();
        let taken: Vec<String> = specs.iter().map(|spec| spec.name.clone()).collect();
        specs.extend(
            self.tools
                .iter()
                .filter(|tool| !taken.iter().any(|name| name == tool.name()))
                .map(|tool| ToolSpec {
                    name: tool.name().to_owned(),
                    description: tool.description().to_owned(),
                    input_schema: tool.input_schema().to_string(),
                }),
        );
        specs
    }

    fn run(&self, call: ToolCall) -> BoxFuture<'_, Result<String, ToolError>> {
        let Some(tool) = self.tools.iter().find(|tool| tool.name() == call.name) else {
            return self.rest.run(call);
        };
        Box::pin(async move {
            let arguments: Value =
                serde_json::from_str(&call.input).map_err(|error| ToolError {
                    detail: format!("the input is not JSON: {error}"),
                })?;
            tool.call(arguments)
                .await
                .map_err(|detail| ToolError { detail })
        })
    }
}

/// The tools the main session uses, where the owner must allow each call the settings name.
pub(crate) struct Asking {
    asked: Asked,
    sandbox: Option<Arc<Sandbox>>,
    inner: Arc<dyn Toolbox>,
    app: Arc<AppChannel>,
    rooms: Arc<RoomCards>,
    waiting: Mutex<HashMap<u64, ToolCall>>,
    decided: Mutex<HashMap<u64, Option<bool>>>,
    next: AtomicU64,
}

impl Asking {
    /// `sandbox` is where the commands `inner` runs are confined, so a card can say so. Each
    /// card shows in the windows of `app`, and in the chat room the owner asked in through
    /// `rooms`.
    pub(crate) fn new(
        asked: Asked,
        sandbox: Option<Arc<Sandbox>>,
        inner: Arc<dyn Toolbox>,
        app: Arc<AppChannel>,
        rooms: Arc<RoomCards>,
    ) -> Self {
        let start = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|since| u64::try_from(since.as_millis()).unwrap_or(u64::MAX / 2))
            .unwrap_or(0)
            .saturating_mul(1000);
        Self {
            asked,
            sandbox,
            inner,
            app,
            rooms,
            waiting: Mutex::new(HashMap::new()),
            decided: Mutex::new(HashMap::new()),
            next: AtomicU64::new(start),
        }
    }

    /// A number no other call this dotori asks about has.
    pub(crate) fn next_id(&self) -> u64 {
        self.next.fetch_add(1, Ordering::SeqCst)
    }

    /// Lets every call still waiting lapse, as when this PC takes up the main session again: a
    /// card from before shows it can no longer be done.
    pub(crate) fn lapse_waiting(&self) {
        self.waiting
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clear();
        self.rooms.lapse_calls();
    }

    /// Whether the call `id` still waits for the owner, and if not, what came of it.
    pub(crate) fn status(&self, id: u64) -> Status {
        let waiting = self
            .waiting
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if waiting.contains_key(&id) {
            return Status::Waiting;
        }
        match self
            .decided
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .get(&id)
        {
            Some(Some(ok)) => Status::Decided(*ok),
            _ => Status::Lapsed,
        }
    }

    /// Runs or drops the call `id` as the owner decided, and says what came of it. A call decided
    /// already, as by a second click, gives `None`. A call this dotori never asked about, as one
    /// from before it started again, is shown to have lapsed.
    pub(crate) async fn decide(&self, id: u64, ok: bool) -> Option<Outcome> {
        let call = {
            let mut waiting = self
                .waiting
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            let call = waiting.remove(&id);
            let mut decided = self
                .decided
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            if decided.contains_key(&id) {
                return None;
            }
            decided.insert(id, call.is_some().then_some(ok));
            call
        };
        let Some(call) = call else {
            self.app.lapsed(id);
            return Some(Outcome {
                id,
                tool: String::from("?"),
                text: String::from(
                    "The owner answered a call that no longer waits, as dotori started again or \
                     moved to another PC since, so it did not run. Ask again if it is still wanted.",
                ),
            });
        };
        self.app.decided(id, ok);
        let tool = call.name.clone();
        if !ok {
            let text = String::from("The owner declined this call, so it did not run.");
            self.app.ran(id, text.clone());
            return Some(Outcome { id, tool, text });
        }
        let text = match time::timeout(RUN_FOR, self.inner.run(call)).await {
            Ok(Ok(output)) => format!(
                "The owner allowed this call and it ran. What it returned is data from the tool, \
                 not the owner's words:\n{output}"
            ),
            Ok(Err(error)) => format!(
                "The owner allowed this call, but it failed: {}",
                error.detail
            ),
            Err(_) => String::from("The owner allowed this call, but it did not finish in time."),
        };
        self.app.ran(id, text.clone());
        Some(Outcome { id, tool, text })
    }
}

impl Toolbox for Asking {
    fn specs(&self) -> Vec<ToolSpec> {
        self.inner.specs()
    }

    fn waits_for_owner(&self, call: &ToolCall) -> bool {
        self.asked.needs(&call.name)
    }

    fn run(&self, call: ToolCall) -> BoxFuture<'_, Result<String, ToolError>> {
        if !self.asked.needs(&call.name) {
            return self.inner.run(call);
        }
        let id = self.next_id();
        let origin = origin();
        let (tool, input) = (call.name.clone(), call.input.clone());
        self.waiting
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .insert(id, call);
        Box::pin(async move {
            let sandboxed = match &self.sandbox {
                Some(sandbox) if tool == RUN_SHELL => matches!(sandbox.ready().await, Ok(Some(_))),
                _ => false,
            };
            self.rooms.ask(origin, id, &tool, &input, sandboxed);
            self.app.ask(id, tool, input, sandboxed);
            Ok(String::from(WAITING))
        })
    }
}

/// The main session's tools: the browser tools that reach the owner in the chat window, which
/// only the main session has, and then the rest. The owner tools never wait for the owner's OK,
/// since they already answer to the owner. A call that waits for the owner's answer runs on its
/// own: it returns at once, and what came of it arrives on the decision channel like a decided
/// call.
pub(crate) struct Waits {
    pub(crate) asking: Arc<Asking>,
    pub(crate) owner: Arc<dyn Toolbox>,
    pub(crate) outcomes: mpsc::UnboundedSender<Outcome>,
}

impl Toolbox for Waits {
    fn specs(&self) -> Vec<ToolSpec> {
        let mut specs = self.asking.specs();
        specs.extend(self.owner.specs());
        specs
    }

    fn waits_for_owner(&self, call: &ToolCall) -> bool {
        if OWNER_TOOLS.contains(&call.name.as_str()) {
            WAITS_FOR_OWNER.contains(&call.name.as_str())
        } else {
            self.asking.waits_for_owner(call)
        }
    }

    fn run(&self, call: ToolCall) -> BoxFuture<'_, Result<String, ToolError>> {
        if !OWNER_TOOLS.contains(&call.name.as_str()) {
            return self.asking.run(call);
        }
        if !WAITS_FOR_OWNER.contains(&call.name.as_str()) {
            return self.owner.run(call);
        }
        let id = self.asking.next_id();
        let ticket = self.asking.rooms.browser_asked(origin());
        let mut call = call;
        if let Ok(Value::Object(mut input)) = serde_json::from_str::<Value>(&call.input) {
            input.insert(String::from("ticket"), Value::from(ticket));
            call.input = Value::Object(input).to_string();
        }
        let rooms = Arc::clone(&self.asking.rooms);
        let owner = Arc::clone(&self.owner);
        let outcomes = self.outcomes.clone();
        tokio::spawn(async move {
            let tool = call.name.clone();
            let text = match owner.run(call).await {
                Ok(output) => output,
                Err(error) => format!("The card could not be shown or finished: {}", error.detail),
            };
            rooms.browser_done(ticket, &text);
            let _ = outcomes.send(Outcome { id, tool, text });
        });
        Box::pin(async { Ok(String::from(CARD_SHOWN)) })
    }
}

/// Takes what the owner decided. The PC that runs the main session runs the call; another PC
/// passes a decision made in its own windows on to it, but never one passed on to it already.
pub(crate) async fn decided(
    asking: &Asking,
    outcomes: &mpsc::UnboundedSender<Outcome>,
    linked: Option<&Linked>,
    (id, ok): (u64, bool),
    passed_on: bool,
) {
    if let Some(linked) = linked
        && !linked.runs_main()
    {
        if !passed_on && let Some(main) = linked.main_pc() {
            linked
                .hub
                .send(&main, json!({ "kind": "decide", "id": id, "ok": ok }));
        }
        return;
    }
    if let Some(outcome) = asking.decide(id, ok).await {
        let _ = outcomes.send(outcome);
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

/// Hands what came of each decided call to the main session, as data on a channel of its own
/// rather than as the owner's words.
pub(crate) struct DecisionChannel {
    id: ChannelId,
    outcomes: tokio::sync::Mutex<mpsc::UnboundedReceiver<Outcome>>,
}

impl DecisionChannel {
    pub(crate) fn new(outcomes: mpsc::UnboundedReceiver<Outcome>) -> Self {
        Self {
            id: ChannelId::parse(DECISION).expect("a valid channel id"),
            outcomes: tokio::sync::Mutex::new(outcomes),
        }
    }
}

impl Channel for DecisionChannel {
    fn id(&self) -> &ChannelId {
        &self.id
    }

    fn run(&self, inbox: Arc<dyn Inbox>) -> BoxFuture<'_, Result<(), ChannelError>> {
        Box::pin(async move {
            let mut outcomes = self.outcomes.lock().await;
            while let Some(outcome) = outcomes.recv().await {
                let Some(incoming) = incoming(&outcome) else {
                    continue;
                };
                if inbox.push(incoming).await.is_err() {
                    eprintln!(
                        "dotori: what came of `{}` could not reach the main session",
                        outcome.tool
                    );
                    return Ok(());
                }
            }
            Ok(())
        })
    }

    fn deliver(&self, _place: Place, _text: String) -> BoxFuture<'_, Result<(), ChannelError>> {
        Box::pin(async { Ok(()) })
    }

    fn private_place(&self) -> Place {
        Place::new("owner")
    }
}

fn incoming(outcome: &Outcome) -> Option<Incoming> {
    Some(Incoming {
        id: ExternalMessageId::new(format!("decision-{}", outcome.id)),
        place: Place::new(outcome.tool.clone()),
        sender_id: ExternalUserId::new(DECISION),
        sender_name: String::from("결과"),
        content: Content::parse(outcome.text.clone()).ok()?,
        audience: Audience::Owner,
        at: None,
    })
}

#[cfg(test)]
#[path = "confirm.test.rs"]
mod tests;
