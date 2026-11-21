use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use tokio::sync::mpsc;
use tokio::task::AbortHandle;
use tokio::task::JoinSet;

use crate::agent::AgentState;
use crate::agent::CallId;
use crate::agent::Effect;
use crate::agent::Event;
use crate::agent::step;
use crate::channel::AppWindow;
use crate::conversation::ThreadRef;
use crate::id::ChannelId;
use crate::llm::Model;
use crate::message::Place;
use crate::sub_task::SubTaskRunner;
use crate::tool::Toolbox;

/// The only impure part of the agent. Receives events, runs `step`, and executes the effects.
/// Slow work is spawned into `tasks` and returns as an event, so the loop never waits on a port.
pub(crate) struct Shell {
    rx: mpsc::Receiver<Event>,
    tx: mpsc::Sender<Event>,
    app: Arc<dyn AppWindow>,
    outboxes: HashMap<ChannelId, mpsc::Sender<Delivery>>,
    model: Arc<dyn Model>,
    toolbox: Arc<dyn Toolbox>,
    sub_tasks: Arc<dyn SubTaskRunner>,
    tool_timeout: Duration,
    tasks: JoinSet<()>,
    model_calls: HashMap<CallId, AbortHandle>,
}

/// One reply waiting in a channel's outbox. Each channel drains its outbox in order.
pub(crate) struct Delivery {
    thread: ThreadRef,
    place: Place,
    text: String,
}

impl Shell {
    pub(crate) async fn run(self, state: AgentState) {
        todo!()
    }

    fn execute(&mut self, effect: Effect) {
        todo!()
    }
}
