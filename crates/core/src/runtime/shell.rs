use std::collections::HashMap;
use std::mem;
use std::sync::Arc;
use std::time::Duration;
use std::time::SystemTime;

use tokio::sync::mpsc;
use tokio::sync::oneshot;
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
use crate::conversation::Entry;
use crate::conversation::ThreadRef;
use crate::id::ChannelId;
use crate::llm::Model;
use crate::llm::ModelError;
use crate::llm::ModelRequest;
use crate::message::Address;
use crate::message::Place;
use crate::record::Record;
use crate::record::RecordStore;
use crate::runtime::Ports;
use crate::runtime::caught::caught;
use crate::runtime::inbox::ChannelInbox;
use crate::sub_session::SubSessionOutcome;
use crate::sub_session::SubSessionRunner;
use crate::tool::ToolError;
use crate::tool::Toolbox;
use crate::tool::asked_from;

const RETRY_FIRST: Duration = Duration::from_millis(100);
const RETRY_MAX: Duration = Duration::from_secs(30);

/// The only impure part of the agent. Receives events, runs `step`, and executes the effects.
/// Slow work is spawned into `tasks` and returns as an event, so the loop never waits on a port.
pub(crate) struct Shell {
    rx: mpsc::Receiver<Event>,
    tx: mpsc::Sender<Event>,
    shows: mpsc::UnboundedSender<Traffic>,
    app: Arc<dyn AppWindow>,
    working: bool,
    records: mpsc::UnboundedSender<Record>,
    outboxes: HashMap<ChannelId, mpsc::UnboundedSender<Delivery>>,
    model: Arc<dyn Model>,
    toolbox: Arc<dyn Toolbox>,
    sub_sessions: Arc<dyn SubSessionRunner>,
    acks: Acks,
    tool_timeout: Duration,
    tasks: JoinSet<()>,
    senders: JoinSet<()>,
    model_calls: HashMap<CallId, AbortHandle>,
}

/// Who waits to hear whether a result handed in from outside was taken, by the number its event
/// carries.
pub(crate) type Acks = Arc<std::sync::Mutex<HashMap<u64, oneshot::Sender<bool>>>>;

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
        acks: Acks,
    ) -> Self {
        let mut tasks = JoinSet::new();
        let mut senders = JoinSet::new();
        let shows = spawn_show_worker(&mut senders, Arc::clone(&ports.app));
        let window = Arc::clone(&ports.app);
        let records = spawn_record_writer(&mut senders, ports.records);
        let app: Arc<dyn Channel> = ports.app;
        let mut outboxes = HashMap::new();
        for channel in std::iter::once(app).chain(ports.channels) {
            spawn_channel(&mut tasks, Arc::clone(&channel), tx.clone());
            let outbox = spawn_outbox(&mut senders, Arc::clone(&channel), tx.clone());
            outboxes.insert(channel.id().clone(), outbox);
        }
        Self {
            rx,
            tx,
            shows,
            app: window,
            working: false,
            records,
            outboxes,
            model: ports.model,
            toolbox: ports.toolbox,
            sub_sessions: ports.sub_sessions,
            acks,
            tool_timeout,
            tasks,
            senders,
            model_calls: HashMap::new(),
        }
    }

    pub(crate) async fn run(mut self, mut state: AgentState) {
        'events: while let Some(event) = self.rx.recv().await {
            if let Event::ModelDone { call, .. } = &event {
                self.model_calls.remove(call);
            }
            let recorded = state.conversation.len();
            let effects = step(&mut state, event);
            let end = state
                .conversation
                .len()
                .saturating_sub(mem::take(&mut state.unrecorded))
                .max(recorded);
            self.record(&state.conversation[recorded..end]);
            self.tell_working(state.working());
            for effect in effects {
                if matches!(effect, Effect::Stop) {
                    break 'events;
                }
                self.execute(effect);
            }
            while self.tasks.try_join_next().is_some() {}
        }
        self.stop().await;
    }

    fn tell_working(&mut self, busy: bool) {
        if self.working != busy {
            self.working = busy;
            self.app.working(busy);
        }
    }

    async fn stop(mut self) {
        self.tell_working(false);
        self.rx.close();
        self.acks
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clear();
        drop(self.outboxes);
        drop(self.shows);
        drop(self.records);
        let _ = time::timeout(self.tool_timeout, async {
            while self.senders.join_next().await.is_some() {}
        })
        .await;
        self.senders.shutdown().await;
        self.tasks.shutdown().await;
    }

    pub(crate) fn record(&self, entries: &[Entry]) {
        let at = SystemTime::now();
        for entry in entries {
            let _ = self.records.send(Record {
                at,
                entry: entry.clone(),
            });
        }
    }

    fn execute(&mut self, effect: Effect) {
        match effect {
            Effect::Deliver { thread, to, text } => self.deliver(thread, to, text),
            Effect::Show(traffic) => {
                let _ = self.shows.send(traffic);
            }
            Effect::CallModel { call, conversation } => {
                let model = Arc::clone(&self.model);
                let toolbox = Arc::clone(&self.toolbox);
                let tx = self.tx.clone();
                let handle = self.tasks.spawn(async move {
                    let result = caught(|| {
                        model.complete(ModelRequest {
                            conversation,
                            tools: toolbox.specs(),
                            context: None,
                        })
                    })
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
            Effect::RunTool { run, call, origin } => {
                let toolbox = Arc::clone(&self.toolbox);
                let tx = self.tx.clone();
                let timeout = self.tool_timeout;
                self.tasks.spawn(asked_from(origin, async move {
                    let result = match time::timeout(timeout, caught(|| toolbox.run(call))).await {
                        Ok(Ok(result)) => result,
                        Ok(Err(detail)) => Err(ToolError { detail }),
                        Err(_) => Err(ToolError {
                            detail: format!("timed out after {timeout:?}"),
                        }),
                    };
                    let _ = tx.send(Event::ToolDone { run, result }).await;
                }));
            }
            Effect::SpawnSubSession { sub_session, spec } => {
                let runner = Arc::clone(&self.sub_sessions);
                let tx = self.tx.clone();
                self.tasks.spawn(async move {
                    let outcome = caught(|| runner.run(spec))
                        .await
                        .unwrap_or_else(SubSessionOutcome::Failed);
                    let _ = tx
                        .send(Event::SubSessionDone {
                            sub_session,
                            outcome,
                        })
                        .await;
                });
            }
            Effect::Acknowledge(ack, taken) => {
                let waiting = self
                    .acks
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner())
                    .remove(&ack);
                if let Some(waiting) = waiting {
                    let _ = waiting.send(taken);
                }
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
        let result = caught(|| channel.run(inbox))
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
            let result = caught(|| channel.deliver(delivery.place.clone(), delivery.text.clone()))
                .await
                .unwrap_or_else(|detail| Err(ChannelError { detail }));
            let event = Event::DeliveryDone {
                thread: delivery.thread,
                place: delivery.place,
                text: delivery.text,
                result,
            };
            let _ = tx.send(event).await;
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
            let _ = caught(|| app.show(item)).await;
        }
    });
    shows
}

fn spawn_record_writer(
    senders: &mut JoinSet<()>,
    store: Arc<dyn RecordStore>,
) -> mpsc::UnboundedSender<Record> {
    let (records, mut incoming) = mpsc::unbounded_channel::<Record>();
    senders.spawn(async move {
        let mut pending = Vec::new();
        let mut backoff = RETRY_FIRST;
        loop {
            if pending.is_empty() {
                match incoming.recv().await {
                    Some(record) => pending.push(record),
                    None => return,
                }
            }
            while let Ok(record) = incoming.try_recv() {
                pending.push(record);
            }
            let batch = pending.clone();
            if let Ok(Ok(())) = caught(|| store.append(batch)).await {
                pending.clear();
                backoff = RETRY_FIRST;
            } else {
                time::sleep(backoff).await;
                backoff = (backoff * 2).min(RETRY_MAX);
            }
        }
    });
    records
}
