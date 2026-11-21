use std::future;
use std::sync::Arc;
use std::sync::Mutex;
use std::time::Duration;

use tokio::sync::mpsc;
use tokio::time;

use crate::BoxFuture;
use crate::channel::AppWindow;
use crate::channel::Channel;
use crate::channel::ChannelError;
use crate::channel::Inbox;
use crate::channel::Traffic;
use crate::id::ChannelId;
use crate::id::ExternalMessageId;
use crate::id::ExternalUserId;
use crate::llm::Model;
use crate::llm::ModelError;
use crate::llm::ModelReply;
use crate::llm::ModelRequest;
use crate::message::Content;
use crate::message::Incoming;
use crate::message::Place;
use crate::runtime::Ports;
use crate::sub_task::SubTaskOutcome;
use crate::sub_task::SubTaskRunner;
use crate::sub_task::SubTaskSpec;
use crate::tool::ToolCall;
use crate::tool::ToolError;
use crate::tool::ToolSpec;
use crate::tool::Toolbox;

const WAIT: Duration = Duration::from_secs(5);

pub(crate) fn channel_id(raw: &str) -> ChannelId {
    ChannelId::parse(raw).expect("valid channel id")
}

pub(crate) fn incoming(id: &str, place: &str, sender: &str, text: &str) -> Incoming {
    Incoming {
        id: ExternalMessageId::new(id),
        place: Place::new(place),
        sender_id: ExternalUserId::new(sender),
        sender_name: sender.to_owned(),
        content: Content::parse(text).expect("non-empty"),
    }
}

pub(crate) async fn next<T>(rx: &mut mpsc::UnboundedReceiver<T>) -> T {
    time::timeout(WAIT, rx.recv())
        .await
        .expect("arrived in time")
        .expect("sender alive")
}

/// A channel whose inbound messages the test sends and whose deliveries the test reads.
pub(crate) struct FakeChannel {
    id: ChannelId,
    inbound: Mutex<Option<mpsc::UnboundedReceiver<Incoming>>>,
    delivered: mpsc::UnboundedSender<(Place, String)>,
    shown: mpsc::UnboundedSender<Traffic>,
}

pub(crate) struct FakeChannelProbe {
    pub(crate) say: mpsc::UnboundedSender<Incoming>,
    pub(crate) delivered: mpsc::UnboundedReceiver<(Place, String)>,
    pub(crate) shown: mpsc::UnboundedReceiver<Traffic>,
}

impl FakeChannel {
    pub(crate) fn new(id: &str) -> (Arc<Self>, FakeChannelProbe) {
        let (say, inbound) = mpsc::unbounded_channel();
        let (delivered_tx, delivered) = mpsc::unbounded_channel();
        let (shown_tx, shown) = mpsc::unbounded_channel();
        let channel = Arc::new(Self {
            id: channel_id(id),
            inbound: Mutex::new(Some(inbound)),
            delivered: delivered_tx,
            shown: shown_tx,
        });
        let probe = FakeChannelProbe {
            say,
            delivered,
            shown,
        };
        (channel, probe)
    }
}

impl Channel for FakeChannel {
    fn id(&self) -> &ChannelId {
        &self.id
    }

    fn run(&self, inbox: Arc<dyn Inbox>) -> BoxFuture<'_, Result<(), ChannelError>> {
        let inbound = self.inbound.lock().expect("unpoisoned").take();
        Box::pin(async move {
            let Some(mut inbound) = inbound else {
                return future::pending().await;
            };
            while let Some(message) = inbound.recv().await {
                if inbox.push(message).await.is_err() {
                    return Ok(());
                }
            }
            future::pending().await
        })
    }

    fn deliver(&self, place: Place, text: String) -> BoxFuture<'_, Result<(), ChannelError>> {
        let _ = self.delivered.send((place, text));
        Box::pin(async { Ok(()) })
    }
}

impl AppWindow for FakeChannel {
    fn show(&self, traffic: Traffic) -> BoxFuture<'_, Result<(), ChannelError>> {
        let _ = self.shown.send(traffic);
        Box::pin(async { Ok(()) })
    }
}

type Reply<T> = BoxFuture<'static, T>;

/// A model that answers each request with whatever the test's function returns.
pub(crate) struct FnModel(
    pub(crate) Box<dyn Fn(ModelRequest) -> Reply<Result<ModelReply, ModelError>> + Send + Sync>,
);

impl Model for FnModel {
    fn complete(&self, request: ModelRequest) -> BoxFuture<'_, Result<ModelReply, ModelError>> {
        (self.0)(request)
    }
}

pub(crate) struct FnToolbox(
    pub(crate) Box<dyn Fn(ToolCall) -> Reply<Result<String, ToolError>> + Send + Sync>,
);

impl Toolbox for FnToolbox {
    fn specs(&self) -> Vec<ToolSpec> {
        vec![ToolSpec {
            name: String::from("read_file"),
            description: String::from("Reads a file."),
        }]
    }

    fn run(&self, call: ToolCall) -> BoxFuture<'_, Result<String, ToolError>> {
        (self.0)(call)
    }
}

pub(crate) struct FnSubTasks(
    pub(crate) Box<dyn Fn(SubTaskSpec) -> Reply<SubTaskOutcome> + Send + Sync>,
);

impl SubTaskRunner for FnSubTasks {
    fn run(&self, spec: SubTaskSpec) -> BoxFuture<'_, SubTaskOutcome> {
        (self.0)(spec)
    }
}

pub(crate) fn idle_model() -> Arc<dyn Model> {
    Arc::new(FnModel(Box::new(|_| Box::pin(future::pending()))))
}

pub(crate) fn idle_toolbox() -> Arc<dyn Toolbox> {
    Arc::new(FnToolbox(Box::new(|_| Box::pin(future::pending()))))
}

pub(crate) fn idle_sub_tasks() -> Arc<dyn SubTaskRunner> {
    Arc::new(FnSubTasks(Box::new(|_| Box::pin(future::pending()))))
}

pub(crate) fn ports(app: Arc<FakeChannel>, channels: Vec<Arc<dyn Channel>>) -> Ports {
    Ports {
        app,
        channels,
        model: idle_model(),
        toolbox: idle_toolbox(),
        sub_tasks: idle_sub_tasks(),
    }
}
