use std::future;
use std::num::NonZeroUsize;
use std::sync::Arc;
use std::sync::atomic::AtomicUsize;
use std::sync::atomic::Ordering;
use std::time::Duration;

use tokio::runtime;
use tokio::sync::mpsc;
use tokio::sync::oneshot;
use tokio::time;

use super::*;
use crate::channel::Channel;
use crate::conversation::Action;
use crate::conversation::Entry;
use crate::conversation::ThreadRef;
use crate::id::ChannelUser;
use crate::llm::ModelReply;
use crate::llm::ModelRequest;
use crate::message::Place;
use crate::runtime::fakes::FakeChannel;
use crate::runtime::fakes::FnModel;
use crate::runtime::fakes::FnSubTasks;
use crate::runtime::fakes::FnToolbox;
use crate::runtime::fakes::channel_id;
use crate::runtime::fakes::incoming;
use crate::runtime::fakes::next;
use crate::runtime::fakes::ports;
use crate::sub_task::SubTaskSpec;
use crate::tool::ToolCall;
use crate::tool::ToolCallId;
use crate::tool::ToolError;

fn block_on(test: impl Future<Output = ()>) {
    runtime::Builder::new_current_thread()
        .enable_time()
        .build()
        .expect("runtime")
        .block_on(test);
}

fn config(tool_timeout: Duration) -> AgentConfig {
    AgentConfig {
        owners: Vec::<ChannelUser>::new(),
        queue_capacity: NonZeroUsize::new(8).expect("non-zero"),
        seen_capacity: NonZeroUsize::new(8).expect("non-zero"),
        tool_timeout,
    }
}

fn last_heard(request: &ModelRequest) -> ThreadRef {
    request
        .conversation
        .iter()
        .rev()
        .find_map(|entry| match entry {
            Entry::Heard { thread, .. } => Some(*thread),
            _ => None,
        })
        .expect("a heard message")
}

fn say(thread: ThreadRef, text: &str) -> ModelReply {
    ModelReply {
        actions: vec![Action::Say {
            thread,
            text: text.to_owned(),
        }],
    }
}

fn tool_result(request: &ModelRequest) -> Option<Result<String, ToolError>> {
    request.conversation.iter().find_map(|entry| match entry {
        Entry::ToolReturned { result, .. } => Some(result.clone()),
        _ => None,
    })
}

fn use_tool() -> ModelReply {
    ModelReply {
        actions: vec![Action::UseTool(ToolCall {
            id: ToolCallId::new("t1"),
            name: String::from("read_file"),
            input: String::from("{}"),
        })],
    }
}

fn echo_model() -> FnModel {
    FnModel(Box::new(|request| {
        let thread = last_heard(&request);
        Box::pin(async move { Ok(say(thread, "hello")) })
    }))
}

fn tool_then_report(seen: mpsc::UnboundedSender<Result<String, ToolError>>) -> FnModel {
    FnModel(Box::new(move |request| {
        let reply = match tool_result(&request) {
            Some(result) => {
                let _ = seen.send(result);
                ModelReply {
                    actions: Vec::new(),
                }
            }
            None => use_tool(),
        };
        Box::pin(async move { Ok(reply) })
    }))
}

#[test]
fn owner_message_on_the_app_gets_a_reply() {
    block_on(async {
        let (app, mut probe) = FakeChannel::new("app");
        let mut ports = ports(app, Vec::new());
        ports.model = Arc::new(echo_model());
        let agent = Agent::spawn(config(Duration::from_secs(1)), ports).expect("valid");

        probe
            .say
            .send(incoming("m1", "main", "me", "hi"))
            .expect("channel alive");

        assert_eq!(
            next(&mut probe.delivered).await,
            (Place::new("main"), String::from("hello"))
        );
        agent.shutdown().await;
    });
}

#[test]
fn other_channel_traffic_and_its_reply_are_shown_on_the_app() {
    block_on(async {
        let (app, mut app_probe) = FakeChannel::new("app");
        let (slack, mut slack_probe) = FakeChannel::new("slack");
        let slack: Arc<dyn Channel> = slack;
        let mut ports = ports(app, vec![slack]);
        ports.model = Arc::new(echo_model());
        let agent = Agent::spawn(config(Duration::from_secs(1)), ports).expect("valid");

        slack_probe
            .say
            .send(incoming("m1", "general", "guest", "lunch?"))
            .expect("channel alive");

        assert_eq!(
            next(&mut slack_probe.delivered).await,
            (Place::new("general"), String::from("hello"))
        );
        let heard = next(&mut app_probe.shown).await;
        assert_eq!(heard.address.channel, channel_id("slack"));
        assert_eq!(heard.text, "lunch?");
        let replied = next(&mut app_probe.shown).await;
        assert_eq!(replied.address.place, Place::new("general"));
        assert_eq!(replied.text, "hello");
        agent.shutdown().await;
    });
}

#[test]
fn panicking_tool_is_reported_to_the_model_as_an_error() {
    block_on(async {
        let (app, probe) = FakeChannel::new("app");
        let (seen_tx, mut seen) = mpsc::unbounded_channel();
        let mut ports = ports(app, Vec::new());
        ports.model = Arc::new(tool_then_report(seen_tx));
        ports.toolbox = Arc::new(FnToolbox(Box::new(|_| {
            Box::pin(async { panic!("disk on fire") })
        })));
        let agent = Agent::spawn(config(Duration::from_secs(1)), ports).expect("valid");

        probe
            .say
            .send(incoming("m1", "main", "me", "read it"))
            .expect("channel alive");

        let error = next(&mut seen).await.expect_err("tool failed");
        assert!(error.detail.contains("disk on fire"), "{}", error.detail);
        agent.shutdown().await;
    });
}

#[test]
fn slow_tool_times_out() {
    block_on(async {
        let (app, probe) = FakeChannel::new("app");
        let (seen_tx, mut seen) = mpsc::unbounded_channel();
        let mut ports = ports(app, Vec::new());
        ports.model = Arc::new(tool_then_report(seen_tx));
        ports.toolbox = Arc::new(FnToolbox(Box::new(|_| Box::pin(future::pending()))));
        let agent = Agent::spawn(config(Duration::from_millis(50)), ports).expect("valid");

        probe
            .say
            .send(incoming("m1", "main", "me", "read it"))
            .expect("channel alive");

        let error = next(&mut seen).await.expect_err("tool timed out");
        assert!(error.detail.contains("timed out"), "{}", error.detail);
        agent.shutdown().await;
    });
}

#[test]
fn panicking_model_does_not_stop_the_agent() {
    block_on(async {
        let (app, mut probe) = FakeChannel::new("app");
        let calls = Arc::new(AtomicUsize::new(0));
        let mut ports = ports(app, Vec::new());
        ports.model = Arc::new(FnModel(Box::new(move |request| {
            let first = calls.fetch_add(1, Ordering::SeqCst) == 0;
            let thread = last_heard(&request);
            Box::pin(async move {
                assert!(!first, "model crashed");
                Ok(say(thread, "back"))
            })
        })));
        let agent = Agent::spawn(config(Duration::from_secs(1)), ports).expect("valid");

        probe
            .say
            .send(incoming("m1", "main", "me", "hi"))
            .expect("channel alive");
        time::sleep(Duration::from_millis(50)).await;
        probe
            .say
            .send(incoming("m2", "main", "me", "hello?"))
            .expect("channel alive");

        assert_eq!(next(&mut probe.delivered).await.1, "back");
        agent.shutdown().await;
    });
}

#[test]
fn shutdown_returns_while_a_model_call_and_a_sub_task_hang() {
    block_on(async {
        let (app, probe) = FakeChannel::new("app");
        let calls = Arc::new(AtomicUsize::new(0));
        let (started_tx, mut started) = mpsc::unbounded_channel();
        let mut ports = ports(app, Vec::new());
        ports.model = Arc::new(FnModel(Box::new(move |_| {
            if calls.fetch_add(1, Ordering::SeqCst) > 0 {
                return Box::pin(future::pending());
            }
            Box::pin(async {
                Ok(ModelReply {
                    actions: vec![Action::Delegate(SubTaskSpec {
                        title: String::from("research"),
                        brief: String::from("look into it"),
                    })],
                })
            })
        })));
        ports.sub_tasks = Arc::new(FnSubTasks(Box::new(move |_| {
            let _ = started_tx.send(());
            Box::pin(future::pending())
        })));
        let agent = Agent::spawn(config(Duration::from_secs(1)), ports).expect("valid");

        probe
            .say
            .send(incoming("m1", "main", "me", "research this"))
            .expect("channel alive");
        next(&mut started).await;
        probe
            .say
            .send(incoming("m2", "main", "me", "and that"))
            .expect("channel alive");
        time::sleep(Duration::from_millis(50)).await;

        time::timeout(Duration::from_secs(5), agent.shutdown())
            .await
            .expect("shutdown finished");
    });
}

#[test]
fn shutdown_waits_for_a_running_tool() {
    block_on(async {
        let (app, probe) = FakeChannel::new("app");
        let (release_tx, release) = oneshot::channel::<()>();
        let release = Arc::new(std::sync::Mutex::new(Some(release)));
        let (finished_tx, mut finished) = mpsc::unbounded_channel();
        let (seen_tx, _seen) = mpsc::unbounded_channel();
        let mut ports = ports(app, Vec::new());
        ports.model = Arc::new(tool_then_report(seen_tx));
        ports.toolbox = Arc::new(FnToolbox(Box::new(move |_| {
            let release = release.lock().expect("unpoisoned").take();
            let finished = finished_tx.clone();
            Box::pin(async move {
                if let Some(release) = release {
                    let _ = release.await;
                }
                let _ = finished.send(());
                Ok(String::from("done"))
            })
        })));
        let agent = Agent::spawn(config(Duration::from_secs(5)), ports).expect("valid");

        probe
            .say
            .send(incoming("m1", "main", "me", "read it"))
            .expect("channel alive");
        time::sleep(Duration::from_millis(50)).await;
        let shutdown = tokio::spawn(agent.shutdown());
        time::sleep(Duration::from_millis(50)).await;
        assert!(!shutdown.is_finished(), "shutdown waits for the tool");

        release_tx.send(()).expect("tool waiting");

        next(&mut finished).await;
        time::timeout(Duration::from_secs(5), shutdown)
            .await
            .expect("shutdown finished")
            .expect("shutdown did not panic");
    });
}

struct DropSignal(Option<oneshot::Sender<()>>);

impl Drop for DropSignal {
    fn drop(&mut self) {
        if let Some(signal) = self.0.take() {
            let _ = signal.send(());
        }
    }
}

#[test]
fn interrupted_model_call_is_dropped() {
    block_on(async {
        let (app, probe) = FakeChannel::new("app");
        let (dropped_tx, dropped) = oneshot::channel();
        let dropped_tx = Arc::new(std::sync::Mutex::new(Some(dropped_tx)));
        let mut ports = ports(app, Vec::new());
        ports.model = Arc::new(FnModel(Box::new(move |_| {
            let guard = DropSignal(dropped_tx.lock().expect("unpoisoned").take());
            Box::pin(async move {
                let _guard = guard;
                future::pending().await
            })
        })));
        let agent = Agent::spawn(config(Duration::from_secs(1)), ports).expect("valid");

        probe
            .say
            .send(incoming("m1", "main", "me", "find my flight"))
            .expect("channel alive");
        time::sleep(Duration::from_millis(50)).await;
        probe
            .say
            .send(incoming("m2", "main", "me", "never mind"))
            .expect("channel alive");

        time::timeout(Duration::from_secs(5), dropped)
            .await
            .expect("cancelled in time")
            .expect("first call was dropped");
        agent.shutdown().await;
    });
}
