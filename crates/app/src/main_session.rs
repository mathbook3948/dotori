//! Which PC runs the main session, and how the other PCs pass the owner's words to it. Windows
//! on every PC show the same conversation: the PC running the main session sends what its
//! windows show to the others.

use std::collections::VecDeque;
use std::future;
use std::sync::Arc;
use std::sync::Mutex;
use std::time::Duration;
use std::time::SystemTime;
use std::time::UNIX_EPOCH;

use dotori_connectors::channel::app::AppChannel;
use dotori_connectors::peer::hub::Heard;
use dotori_connectors::peer::hub::Hub;
use dotori_connectors::peer::lease::Keeper;
use dotori_core::BoxFuture;
use dotori_core::channel::AppWindow;
use dotori_core::channel::Channel;
use dotori_core::channel::ChannelError;
use dotori_core::channel::Inbox;
use dotori_core::channel::InboxClosedError;
use dotori_core::channel::Traffic;
use dotori_core::id::ChannelId;
use dotori_core::message::Incoming;
use dotori_core::message::Place;
use dotori_core::tool::ToolCall;
use dotori_core::tool::ToolError;
use dotori_core::tool::ToolSpec;
use dotori_core::tool::Toolbox;
use serde_json::Value;
use serde_json::json;
use tokio::sync::broadcast::error::RecvError;

pub(crate) const MAIN_JOB: &str = "main";
const WAITING: usize = 100;

type Said = (String, Option<String>, Option<SystemTime>);

/// The other PCs and who among them runs which job. Without them this PC runs everything.
#[derive(Clone, Debug)]
pub(crate) struct Linked {
    pub(crate) hub: Arc<Hub>,
    pub(crate) keeper: Arc<Keeper>,
    pub(crate) own: String,
}

impl Linked {
    pub(crate) fn runs_main(&self) -> bool {
        self.keeper.holds(MAIN_JOB)
    }

    pub(crate) fn main_pc(&self) -> Option<String> {
        self.keeper
            .watch(MAIN_JOB)
            .and_then(|holder| holder.borrow().clone())
    }
}

enum Target {
    Agent(Arc<dyn Inbox>),
    Elsewhere,
}

/// Where the owner's words from this PC's windows go: to the agent here while this PC runs the
/// main session, or else to the PC that does. Words with nowhere to go yet wait.
pub(crate) struct Route {
    target: Mutex<Target>,
    linked: Option<Linked>,
    waiting: Mutex<VecDeque<Said>>,
}

impl Route {
    pub(crate) fn new(linked: Option<Linked>) -> Arc<Self> {
        Arc::new(Self {
            target: Mutex::new(Target::Elsewhere),
            linked,
            waiting: Mutex::new(VecDeque::new()),
        })
    }

    fn to_agent(&self, inbox: Arc<dyn Inbox>) {
        *self.target.lock().expect("unpoisoned") = Target::Agent(inbox);
    }

    /// Hands the words that waited to the agent, once it runs here.
    pub(crate) async fn hand_waiting(&self, app: &AppChannel) {
        let inbox = match &*self.target.lock().expect("unpoisoned") {
            Target::Agent(inbox) => Arc::clone(inbox),
            Target::Elsewhere => return,
        };
        let own = self.linked.as_ref().map(|linked| linked.own.clone());
        let waiting: Vec<Said> = self.waiting.lock().expect("unpoisoned").drain(..).collect();
        for (text, origin, at) in waiting {
            let incoming = match origin {
                Some(origin) if Some(&origin) != own.as_ref() => {
                    app.heard_elsewhere(text, origin, at)
                }
                _ => app.incoming(text, at),
            };
            if let Some(incoming) = incoming {
                let _ = inbox.push(incoming).await;
            }
        }
    }

    pub(crate) fn runs_agent(&self) -> bool {
        matches!(&*self.target.lock().expect("unpoisoned"), Target::Agent(_))
    }

    pub(crate) fn elsewhere(&self) {
        *self.target.lock().expect("unpoisoned") = Target::Elsewhere;
    }

    /// Sends the words that waited to the PC running the main session, if one does now.
    pub(crate) fn flush(&self) {
        let Some(linked) = &self.linked else {
            return;
        };
        let Some(main) = linked.main_pc().filter(|main| *main != linked.own) else {
            return;
        };
        let mut waiting = self.waiting.lock().expect("unpoisoned");
        while let Some((text, origin, at)) = waiting.front() {
            let origin = origin.as_deref().unwrap_or(&linked.own);
            let at = at.and_then(|at| at.duration_since(UNIX_EPOCH).ok());
            let at = at.map(|at| u64::try_from(at.as_millis()).unwrap_or(u64::MAX));
            if !linked.hub.send(
                &main,
                json!({ "kind": "owner_said", "text": text, "origin": origin, "at": at }),
            ) {
                return;
            }
            waiting.pop_front();
        }
    }

    /// Passes on words the owner said on the PC `origin` that reached this PC when it no longer
    /// ran the main session, or did not run the agent yet.
    pub(crate) fn forward(&self, text: String, origin: String, at: Option<SystemTime>) {
        self.wait(text, Some(origin), at);
        self.flush();
    }

    fn wait(&self, text: String, origin: Option<String>, at: Option<SystemTime>) {
        let mut waiting = self.waiting.lock().expect("unpoisoned");
        if waiting.len() == WAITING {
            waiting.pop_front();
        }
        waiting.push_back((text, origin, at));
    }
}

impl Inbox for Route {
    fn push(&self, incoming: Incoming) -> BoxFuture<'_, Result<(), InboxClosedError>> {
        let agent = match &*self.target.lock().expect("unpoisoned") {
            Target::Agent(inbox) => Some(Arc::clone(inbox)),
            Target::Elsewhere => None,
        };
        Box::pin(async move {
            if let Some(inbox) = agent {
                return inbox.push(incoming).await;
            }
            self.wait(incoming.content.as_str().to_owned(), None, incoming.at);
            self.flush();
            Ok(())
        })
    }
}

/// The app channel as the agent sees it. The windows themselves stay with the daemon, so they
/// keep working while the main session moves between PCs.
pub(crate) struct AgentApp {
    pub(crate) app: Arc<AppChannel>,
    pub(crate) route: Arc<Route>,
    pub(crate) linked: Option<Linked>,
}

impl Channel for AgentApp {
    fn id(&self) -> &ChannelId {
        self.app.id()
    }

    fn run(&self, inbox: Arc<dyn Inbox>) -> BoxFuture<'_, Result<(), ChannelError>> {
        self.route.to_agent(inbox);
        Box::pin(future::pending())
    }

    fn deliver(&self, place: Place, text: String) -> BoxFuture<'_, Result<(), ChannelError>> {
        if let Err(error) = still_main(self.linked.as_ref()) {
            return Box::pin(async move { Err(error) });
        }
        self.app.deliver(place, text)
    }

    fn private_place(&self) -> Place {
        self.app.private_place()
    }
}

impl AppWindow for AgentApp {
    fn show(&self, traffic: Traffic) -> BoxFuture<'_, Result<(), ChannelError>> {
        self.app.show(traffic)
    }

    fn working(&self, busy: bool) {
        self.app.working(busy);
    }
}

/// A channel that sends only while this PC still runs the main session.
pub(crate) struct Fenced {
    pub(crate) channel: Arc<dyn Channel>,
    pub(crate) linked: Option<Linked>,
}

impl Channel for Fenced {
    fn id(&self) -> &ChannelId {
        self.channel.id()
    }

    fn run(&self, inbox: Arc<dyn Inbox>) -> BoxFuture<'_, Result<(), ChannelError>> {
        self.channel.run(inbox)
    }

    fn deliver(&self, place: Place, text: String) -> BoxFuture<'_, Result<(), ChannelError>> {
        if let Err(error) = still_main(self.linked.as_ref()) {
            return Box::pin(async move { Err(error) });
        }
        self.channel.deliver(place, text)
    }

    fn private_place(&self) -> Place {
        self.channel.private_place()
    }
}

/// The main session's tools, which run only while this PC still runs the main session, so a PC
/// that lost it never acts for the owner.
pub(crate) struct FencedTools {
    pub(crate) tools: Arc<dyn Toolbox>,
    pub(crate) linked: Option<Linked>,
}

impl Toolbox for FencedTools {
    fn specs(&self) -> Vec<ToolSpec> {
        self.tools.specs()
    }

    fn waits_for_owner(&self, call: &ToolCall) -> bool {
        self.tools.waits_for_owner(call)
    }

    fn run(&self, call: ToolCall) -> BoxFuture<'_, Result<String, ToolError>> {
        if let Err(error) = still_main(self.linked.as_ref()) {
            return Box::pin(async move {
                Err(ToolError {
                    detail: error.detail,
                })
            });
        }
        self.tools.run(call)
    }
}

fn still_main(linked: Option<&Linked>) -> Result<(), ChannelError> {
    match linked {
        Some(linked) if !linked.runs_main() => Err(ChannelError {
            detail: String::from("another PC runs the assistant now"),
        }),
        _ => Ok(()),
    }
}

/// Takes what another PC says about the conversation: the owner's words for the main session
/// here, or what that PC's windows show for the windows here. Says whether it was one.
pub(crate) async fn hear(linked: &Linked, app: &AppChannel, route: &Route, heard: &Heard) -> bool {
    match heard.message.get("kind").and_then(Value::as_str) {
        Some("owner_said") => {
            let Some(text) = heard.message.get("text").and_then(Value::as_str) else {
                return true;
            };
            let origin = heard
                .message
                .get("origin")
                .and_then(Value::as_str)
                .unwrap_or(&heard.key)
                .to_owned();
            let at = heard
                .message
                .get("at")
                .and_then(Value::as_u64)
                .map(|ms| UNIX_EPOCH + Duration::from_millis(ms));
            if linked.runs_main() && route.runs_agent() {
                if let Some(incoming) = app.heard_elsewhere(text.to_owned(), origin, at) {
                    let _ = route.push(incoming).await;
                }
            } else {
                route.forward(text.to_owned(), origin, at);
            }
            true
        }
        Some("working") => {
            if !linked.runs_main()
                && let Some(busy) = heard.message.get("busy").and_then(Value::as_bool)
            {
                app.working(busy);
            }
            true
        }
        Some("window") => {
            let from_here =
                heard.message.get("origin").and_then(Value::as_str) == Some(linked.own.as_str());
            if !from_here
                && !linked.runs_main()
                && let Some(line) = heard.message.get("line").and_then(Value::as_str)
            {
                app.show_line(line);
            }
            true
        }
        _ => false,
    }
}

/// Sends what the windows here show to the other PCs while this PC runs the main session.
pub(crate) async fn mirror(linked: Linked, app: Arc<AppChannel>) {
    let mut published = app.published();
    let mut working = app.working_changes();
    loop {
        tokio::select! {
            got = published.recv() => match got {
                Ok(published) => {
                    if linked.runs_main() {
                        linked.hub.broadcast(&json!({
                            "kind": "window",
                            "line": published.line,
                            "origin": published.origin,
                        }));
                    }
                }
                Err(RecvError::Lagged(_)) => {}
                Err(RecvError::Closed) => return,
            },
            changed = working.changed() => {
                if changed.is_err() {
                    return;
                }
                let busy = *working.borrow_and_update();
                if linked.runs_main() {
                    linked.hub.broadcast(&json!({ "kind": "working", "busy": busy }));
                }
            }
        }
    }
}

/// Waits until this PC runs the main session. Alone, it always does.
pub(crate) async fn gained(linked: Option<&Linked>) {
    let Some(linked) = linked else {
        return;
    };
    let Some(mut holder) = linked.keeper.watch(MAIN_JOB) else {
        return future::pending().await;
    };
    let _ = holder
        .wait_for(|holder| holder.as_deref() == Some(linked.own.as_str()))
        .await;
}

/// Waits until another PC runs the main session instead. Alone, that never happens.
pub(crate) async fn lost(linked: Option<&Linked>) {
    let Some(linked) = linked else {
        return future::pending().await;
    };
    let Some(mut holder) = linked.keeper.watch(MAIN_JOB) else {
        return future::pending().await;
    };
    let _ = holder
        .wait_for(|holder| holder.as_deref() != Some(linked.own.as_str()))
        .await;
}

#[cfg(test)]
#[path = "main_session.test.rs"]
mod tests;
