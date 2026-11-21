use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use tokio::sync::mpsc;
use tokio::task::AbortHandle;
use tokio::task::JoinSet;
use tokio::time;

use crate::agent::AgentState;
use crate::agent::CallId;
use crate::agent::Effect;
use crate::agent::Event;
use crate::agent::step;
use crate::channel::AppWindow;
use crate::channel::Channel;
use crate::channel::ChannelError;
use crate::channel::Traffic;
use crate::conversation::ThreadRef;
use crate::id::ChannelId;
use crate::llm::Model;
use crate::llm::ModelError;
use crate::llm::ModelRequest;
use crate::message::Address;
use crate::message::Place;
use crate::runtime::Ports;
use crate::runtime::caught::caught;
use crate::runtime::inbox::ChannelInbox;
use crate::sub_task::SubTaskOutcome;
use crate::sub_task::SubTaskRunner;
use crate::tool::ToolError;
use crate::tool::Toolbox;

/// The only impure part of the agent. Receives events, runs `step`, and executes the effects.
/// Slow work is spawned into `tasks` and returns as an event, so the loop never waits on a port.
pub(crate) struct Shell {
    rx: mpsc::Receiver<Event>,
    tx: mpsc::Sender<Event>,
    shows: mpsc::UnboundedSender<Traffic>,
    outboxes: HashMap<ChannelId, mpsc::UnboundedSender<Delivery>>,
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
    /// Starts every channel with its own inbox and outbox. Must run inside a tokio runtime.
    pub(crate) fn start(
        rx: mpsc::Receiver<Event>,
        tx: mpsc::Sender<Event>,
        ports: Ports,
        tool_timeout: Duration,
    ) -> Self {
        let mut tasks = JoinSet::new();
        let shows = spawn_show_worker(&mut tasks, Arc::clone(&ports.app));
        let app: Arc<dyn Channel> = ports.app;
        let mut outboxes = HashMap::new();
        for channel in std::iter::once(app).chain(ports.channels) {
            spawn_channel(&mut tasks, Arc::clone(&channel), tx.clone());
            let outbox = spawn_outbox(&mut tasks, Arc::clone(&channel), tx.clone());
            outboxes.insert(channel.id().clone(), outbox);
        }
        Self {
            rx,
            tx,
            shows,
            outboxes,
            model: ports.model,
            toolbox: ports.toolbox,
            sub_tasks: ports.sub_tasks,
            tool_timeout,
            tasks,
            model_calls: HashMap::new(),
        }
    }

    pub(crate) async fn run(mut self, mut state: AgentState) {
        'events: while let Some(event) = self.rx.recv().await {
            if let Event::ModelDone { call, .. } = &event {
                self.model_calls.remove(call);
            }
            for effect in step(&mut state, event) {
                if matches!(effect, Effect::Stop) {
                    break 'events;
                }
                self.execute(effect);
            }
            while self.tasks.try_join_next().is_some() {}
        }
        self.tasks.shutdown().await;
    }

    fn execute(&mut self, effect: Effect) {
        match effect {
            Effect::Deliver { thread, to, text } => self.deliver(thread, to, text),
            Effect::Show(traffic) => {
                let _ = self.shows.send(traffic);
            }
            Effect::CallModel { call, conversation } => {
                let request = ModelRequest {
                    conversation,
                    tools: self.toolbox.specs(),
                };
                let model = Arc::clone(&self.model);
                let tx = self.tx.clone();
                let handle = self.tasks.spawn(async move {
                    let result = caught(model.complete(request))
                        .await
                        .unwrap_or_else(|detail| Err(ModelError { detail }));
                    let _ = tx.send(Event::ModelDone { call, result }).await;
                });
                self.model_calls.insert(call, handle);
            }
            Effect::CancelModel(call) => {
                if let Some(handle) = self.model_calls.remove(&call) {
                    handle.abort();
                }
            }
            Effect::RunTool { run, call } => {
                let toolbox = Arc::clone(&self.toolbox);
                let tx = self.tx.clone();
                let timeout = self.tool_timeout;
                self.tasks.spawn(async move {
                    let result = match time::timeout(timeout, caught(toolbox.run(call))).await {
                        Ok(Ok(result)) => result,
                        Ok(Err(detail)) => Err(ToolError { detail }),
                        Err(_) => Err(ToolError {
                            detail: format!("timed out after {timeout:?}"),
                        }),
                    };
                    let _ = tx.send(Event::ToolDone { run, result }).await;
                });
            }
            Effect::SpawnSubTask { sub_task, spec } => {
                let runner = Arc::clone(&self.sub_tasks);
                let tx = self.tx.clone();
                self.tasks.spawn(async move {
                    let outcome = caught(runner.run(spec))
                        .await
                        .unwrap_or_else(SubTaskOutcome::Failed);
                    let _ = tx.send(Event::SubTaskDone { sub_task, outcome }).await;
                });
            }
            Effect::Stop => {}
        }
    }

    fn deliver(&mut self, thread: ThreadRef, to: Address, text: String) {
        let delivery = Delivery {
            thread,
            place: to.place,
            text,
        };
        if let Some(outbox) = self.outboxes.get(&to.channel) {
            let _ = outbox.send(delivery);
        }
    }
}

fn spawn_channel(tasks: &mut JoinSet<()>, channel: Arc<dyn Channel>, tx: mpsc::Sender<Event>) {
    tasks.spawn(async move {
        let id = channel.id().clone();
        let inbox = Arc::new(ChannelInbox::new(id.clone(), tx.clone()));
        let result = caught(channel.run(inbox))
            .await
            .unwrap_or_else(|detail| Err(ChannelError { detail }));
        let _ = tx
            .send(Event::ChannelEnded {
                channel: id,
                result,
            })
            .await;
    });
}

fn spawn_outbox(
    tasks: &mut JoinSet<()>,
    channel: Arc<dyn Channel>,
    tx: mpsc::Sender<Event>,
) -> mpsc::UnboundedSender<Delivery> {
    let (outbox, mut deliveries) = mpsc::unbounded_channel::<Delivery>();
    tasks.spawn(async move {
        while let Some(delivery) = deliveries.recv().await {
            let result = caught(channel.deliver(delivery.place, delivery.text.clone()))
                .await
                .unwrap_or_else(|detail| Err(ChannelError { detail }));
            let event = Event::DeliveryDone {
                thread: delivery.thread,
                text: delivery.text,
                result,
            };
            if tx.send(event).await.is_err() {
                return;
            }
        }
    });
    outbox
}

fn spawn_show_worker(
    tasks: &mut JoinSet<()>,
    app: Arc<dyn AppWindow>,
) -> mpsc::UnboundedSender<Traffic> {
    let (shows, mut traffic) = mpsc::unbounded_channel::<Traffic>();
    tasks.spawn(async move {
        while let Some(item) = traffic.recv().await {
            let _ = caught(app.show(item)).await;
        }
    });
    shows
}
