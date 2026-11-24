use std::future;
use std::num::NonZeroUsize;
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::atomic::AtomicUsize;
use std::sync::atomic::Ordering;
use std::time::Duration;
use std::time::SystemTime;

use tokio::runtime;
use tokio::sync::mpsc;
use tokio::sync::oneshot;
use tokio::time;

use super::*;
use crate::BoxFuture;
use crate::channel::Channel;
use crate::conversation::Action;
use crate::conversation::Entry;
use crate::conversation::ThreadRef;
use crate::id::ExternalMessageId;
use crate::id::ExternalUserId;
use crate::llm::ModelError;
use crate::llm::ModelReply;
use crate::llm::ModelRequest;
use crate::message::Address;
use crate::message::Audience;
use crate::message::Content;
use crate::message::Message;
use crate::message::Place;
use crate::message::Role;
use crate::message::Sender;
use crate::record::Record;
use crate::runtime::fakes::FakeChannel;
use crate::runtime::fakes::FakeChannelProbe;
use crate::runtime::fakes::FnModel;
use crate::runtime::fakes::FnSubSessions;
use crate::runtime::fakes::FnToolbox;
use crate::runtime::fakes::MemoryStore;
use crate::runtime::fakes::channel_id;
use crate::runtime::fakes::incoming;
use crate::runtime::fakes::next;
use crate::runtime::fakes::ports;
use crate::sub_session::SubSessionOutcome;
use crate::sub_session::SubSessionSpec;
use crate::tool::ToolCall;
use crate::tool::ToolCallId;
use crate::tool::ToolError;

type ModelFn =
    Box<dyn Fn(ModelRequest) -> BoxFuture<'static, Result<ModelReply, ModelError>> + Send + Sync>;

fn block_on(test: impl Future<Output = ()>) {
    runtime::Builder::new_current_thread()
        .enable_time()
        .build()
        .expect("runtime")
        .block_on(test);
}

fn config(tool_timeout: Duration) -> AgentConfig {
    AgentConfig {
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
            privately: false,
        }],
        trace: None,
    }
}

fn silence() -> ModelReply {
    ModelReply {
        actions: Vec::new(),
        trace: None,
    }
}

fn tool_call() -> Action {
    Action::UseTool(ToolCall {
        id: ToolCallId::new("t1"),
        name: String::from("read_file"),
        input: String::from("{}"),
    })
}

fn tool_result(request: &ModelRequest) -> Option<Result<String, ToolError>> {
    request.conversation.iter().find_map(|entry| match entry {
        Entry::ToolReturned { result, .. } => Some(result.clone()),
        _ => None,
    })
}

fn sub_session_outcome(request: &ModelRequest) -> Option<SubSessionOutcome> {
    request.conversation.iter().find_map(|entry| match entry {
        Entry::SubSessionReturned { outcome, .. } => Some(outcome.clone()),
        _ => None,
    })
}

fn model(answer: impl Fn(ModelRequest) -> ModelReply + Send + Sync + 'static) -> FnModel {
    FnModel(Box::new(move |request| {
        let reply = answer(request);
        Box::pin(async move { Ok(reply) })
    }))
}

fn echo_model() -> FnModel {
    model(|request| say(last_heard(&request), "hello"))
}

fn tool_then_report(seen: mpsc::UnboundedSender<Result<String, ToolError>>) -> FnModel {
    model(move |request| match tool_result(&request) {
        Some(result) => {
            let _ = seen.send(result);
            silence()
        }
        None => ModelReply {
            actions: vec![tool_call()],
            trace: None,
        },
    })
}

fn owner_says(probe: &FakeChannelProbe, id: &str, text: &str) {
    probe
        .say
        .send(incoming(id, "main", "me", text))
        .expect("channel alive");
}

#[test]
fn owner_message_on_the_app_gets_a_reply() {
    block_on(async {
        let (app, mut probe) = FakeChannel::new("app");
        let mut ports = ports(app, Vec::new());
        ports.model = Arc::new(echo_model());
        let agent = Agent::spawn(config(Duration::from_secs(1)), ports)
            .await
            .expect("valid");

        owner_says(&probe, "m1", "hi");

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
        let agent = Agent::spawn(config(Duration::from_secs(1)), ports)
            .await
            .expect("valid");

        slack_probe
            .say
            .send(incoming("m1", "general", "me", "lunch?"))
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

fn panicking_tool_is_reported(toolbox: FnToolbox) {
    block_on(async {
        let (app, probe) = FakeChannel::new("app");
        let (seen_tx, mut seen) = mpsc::unbounded_channel();
        let mut ports = ports(app, Vec::new());
        ports.model = Arc::new(tool_then_report(seen_tx));
        ports.toolbox = Arc::new(toolbox);
        let agent = Agent::spawn(config(Duration::from_secs(1)), ports)
            .await
            .expect("valid");

        owner_says(&probe, "m1", "read it");

        let error = next(&mut seen).await.expect_err("tool failed");
        assert!(error.detail.contains("disk on fire"), "{}", error.detail);
        agent.shutdown().await;
    });
}

#[test]
fn tool_panicking_while_running_is_reported_to_the_model() {
    panicking_tool_is_reported(FnToolbox(Box::new(|_| {
        Box::pin(async { panic!("disk on fire") })
    })));
}

#[test]
fn tool_panicking_before_it_starts_is_reported_to_the_model() {
    panicking_tool_is_reported(FnToolbox(Box::new(|_| panic!("disk on fire"))));
}

#[test]
fn slow_tool_times_out() {
    block_on(async {
        let (app, probe) = FakeChannel::new("app");
        let (seen_tx, mut seen) = mpsc::unbounded_channel();
        let mut ports = ports(app, Vec::new());
        ports.model = Arc::new(tool_then_report(seen_tx));
        ports.toolbox = Arc::new(FnToolbox(Box::new(|_| Box::pin(future::pending()))));
        let agent = Agent::spawn(config(Duration::from_millis(50)), ports)
            .await
            .expect("valid");

        owner_says(&probe, "m1", "read it");

        let error = next(&mut seen).await.expect_err("tool timed out");
        assert!(error.detail.contains("timed out"), "{}", error.detail);
        agent.shutdown().await;
    });
}

fn agent_recovers_from_a_panicking_model(
    crash: fn() -> BoxFuture<'static, Result<ModelReply, ModelError>>,
) {
    block_on(async {
        let (app, probe) = FakeChannel::new("app");
        let (slack, mut slack_probe) = FakeChannel::new("slack");
        let slack: Arc<dyn Channel> = slack;
        let (failed_tx, mut failed) = mpsc::unbounded_channel();
        let calls = Arc::new(AtomicUsize::new(0));
        let mut ports = ports(app, vec![slack]);
        let store = Arc::new(MemoryStore::default());
        ports.records = Arc::clone(&store) as _;
        let answer: ModelFn = Box::new(move |request| {
            if calls.fetch_add(1, Ordering::SeqCst) == 0 {
                return crash();
            }
            let failure = request.conversation.iter().find_map(|entry| match entry {
                Entry::ModelFailed { detail } => Some(detail.clone()),
                _ => None,
            });
            let _ = failed_tx.send(failure);
            let thread = last_heard(&request);
            Box::pin(async move { Ok(say(thread, "back")) })
        });
        ports.model = Arc::new(FnModel(answer));
        let agent = Agent::spawn(config(Duration::from_secs(1)), ports)
            .await
            .expect("valid");

        owner_says(&probe, "m1", "hi");
        time::timeout(Duration::from_secs(5), async {
            while !stored(&store)
                .iter()
                .any(|entry| matches!(entry, Entry::ModelFailed { .. }))
            {
                time::sleep(Duration::from_millis(5)).await;
            }
        })
        .await
        .expect("the crash was recorded");
        slack_probe
            .say
            .send(incoming("m2", "general", "me", "hello?"))
            .expect("channel alive");

        assert_eq!(next(&mut slack_probe.delivered).await.1, "back");
        let failure = next(&mut failed).await.expect("model failure recorded");
        assert!(failure.contains("model crashed"), "{failure}");
        agent.shutdown().await;
    });
}

#[test]
fn model_panicking_while_running_does_not_stop_the_agent() {
    agent_recovers_from_a_panicking_model(|| Box::pin(async { panic!("model crashed") }));
}

#[test]
fn model_panicking_before_it_starts_does_not_stop_the_agent() {
    agent_recovers_from_a_panicking_model(|| panic!("model crashed"));
}

#[test]
fn panicking_sub_session_is_reported_as_failed() {
    block_on(async {
        let (app, probe) = FakeChannel::new("app");
        let (outcome_tx, mut outcome) = mpsc::unbounded_channel();
        let mut ports = ports(app, Vec::new());
        ports.model = Arc::new(model(move |request| match sub_session_outcome(&request) {
            Some(result) => {
                let _ = outcome_tx.send(result);
                silence()
            }
            None => ModelReply {
                actions: vec![Action::Delegate(SubSessionSpec {
                    title: String::from("research"),
                    brief: String::from("look into it"),
                    folder: None,
                })],
                trace: None,
            },
        }));
        ports.sub_sessions = Arc::new(FnSubSessions(Box::new(|_| panic!("runner crashed"))));
        let agent = Agent::spawn(config(Duration::from_secs(1)), ports)
            .await
            .expect("valid");

        owner_says(&probe, "m1", "research this");

        match next(&mut outcome).await {
            SubSessionOutcome::Failed(detail) => {
                assert!(detail.contains("runner crashed"), "{detail}")
            }
            SubSessionOutcome::Finished(text) => panic!("expected a failure, got {text}"),
        }
        agent.shutdown().await;
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
        let (started_tx, mut started) = mpsc::unbounded_channel();
        let (dropped_tx, dropped) = oneshot::channel();
        let dropped_tx = Arc::new(Mutex::new(Some(dropped_tx)));
        let mut ports = ports(app, Vec::new());
        let answer: ModelFn = Box::new(move |_| {
            let guard = DropSignal(dropped_tx.lock().expect("unpoisoned").take());
            let started = started_tx.clone();
            Box::pin(async move {
                let _guard = guard;
                let _ = started.send(());
                future::pending().await
            })
        });
        ports.model = Arc::new(FnModel(answer));
        let agent = Agent::spawn(config(Duration::from_secs(1)), ports)
            .await
            .expect("valid");

        owner_says(&probe, "m1", "find my flight");
        next(&mut started).await;
        owner_says(&probe, "m2", "never mind");

        time::timeout(Duration::from_secs(5), dropped)
            .await
            .expect("cancelled in time")
            .expect("first call was dropped");
        agent.shutdown().await;
    });
}

#[test]
fn shutdown_returns_while_a_model_call_and_a_sub_session_hang() {
    block_on(async {
        let (app, probe) = FakeChannel::new("app");
        let (started_tx, mut started) = mpsc::unbounded_channel();
        let calls = Arc::new(AtomicUsize::new(0));
        let mut ports = ports(app, Vec::new());
        let model_started = started_tx.clone();
        let answer: ModelFn = Box::new(move |_| {
            if calls.fetch_add(1, Ordering::SeqCst) > 0 {
                let _ = model_started.send("model");
                return Box::pin(future::pending());
            }
            Box::pin(async {
                Ok(ModelReply {
                    actions: vec![Action::Delegate(SubSessionSpec {
                        title: String::from("research"),
                        brief: String::from("look into it"),
                        folder: None,
                    })],
                    trace: None,
                })
            })
        });
        ports.model = Arc::new(FnModel(answer));
        ports.sub_sessions = Arc::new(FnSubSessions(Box::new(move |_| {
            let _ = started_tx.send("sub-session");
            Box::pin(future::pending())
        })));
        let agent = Agent::spawn(config(Duration::from_secs(1)), ports)
            .await
            .expect("valid");

        owner_says(&probe, "m1", "research this");
        assert_eq!(next(&mut started).await, "sub-session");
        owner_says(&probe, "m2", "and that");
        assert_eq!(next(&mut started).await, "model");

        time::timeout(Duration::from_secs(5), agent.shutdown())
            .await
            .expect("shutdown finished");
    });
}

#[test]
fn shutdown_waits_for_a_running_tool() {
    block_on(async {
        let (app, probe) = FakeChannel::new("app");
        let (started_tx, mut started) = mpsc::unbounded_channel();
        let (release_tx, release) = oneshot::channel::<()>();
        let release = Arc::new(Mutex::new(Some(release)));
        let (finished_tx, mut finished) = mpsc::unbounded_channel();
        let (seen_tx, _seen) = mpsc::unbounded_channel();
        let mut ports = ports(app, Vec::new());
        ports.model = Arc::new(tool_then_report(seen_tx));
        ports.toolbox = Arc::new(FnToolbox(Box::new(move |_| {
            let release = release.lock().expect("unpoisoned").take();
            let started = started_tx.clone();
            let finished = finished_tx.clone();
            Box::pin(async move {
                let _ = started.send(());
                if let Some(release) = release {
                    let _ = release.await;
                }
                let _ = finished.send(());
                Ok(String::from("done"))
            })
        })));
        let agent = Agent::spawn(config(Duration::from_secs(5)), ports)
            .await
            .expect("valid");

        owner_says(&probe, "m1", "read it");
        next(&mut started).await;
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

#[test]
fn queued_reply_is_delivered_before_shutdown_returns() {
    block_on(async {
        let (app, mut probe) = FakeChannel::slow("app", Duration::from_millis(100));
        let (started_tx, mut started) = mpsc::unbounded_channel();
        let mut ports = ports(app, Vec::new());
        ports.model = Arc::new(model(|request| match tool_result(&request) {
            Some(_) => silence(),
            None => ModelReply {
                actions: vec![
                    Action::Say {
                        thread: last_heard(&request),
                        text: String::from("on it"),
                        privately: false,
                    },
                    tool_call(),
                ],
                trace: None,
            },
        }));
        ports.toolbox = Arc::new(FnToolbox(Box::new(move |_| {
            let started = started_tx.clone();
            Box::pin(async move {
                let _ = started.send(());
                Ok(String::from("done"))
            })
        })));
        let agent = Agent::spawn(config(Duration::from_secs(5)), ports)
            .await
            .expect("valid");

        owner_says(&probe, "m1", "read it");
        next(&mut started).await;
        agent.shutdown().await;

        assert_eq!(
            probe.delivered.try_recv().ok(),
            Some((Place::new("main"), String::from("on it")))
        );
    });
}

#[test]
fn hung_delivery_does_not_hang_shutdown() {
    block_on(async {
        let (app, probe) = FakeChannel::slow("app", Duration::from_secs(3600));
        let (asked_tx, mut asked) = mpsc::unbounded_channel();
        let mut ports = ports(app, Vec::new());
        ports.model = Arc::new(model(move |request| {
            let _ = asked_tx.send(());
            say(last_heard(&request), "on it")
        }));
        let agent = Agent::spawn(config(Duration::from_millis(100)), ports)
            .await
            .expect("valid");

        owner_says(&probe, "m1", "hi");
        next(&mut asked).await;
        time::sleep(Duration::from_millis(20)).await;

        time::timeout(Duration::from_secs(5), agent.shutdown())
            .await
            .expect("shutdown finished");
    });
}

fn heard_texts(entries: &[Entry]) -> Vec<String> {
    entries
        .iter()
        .filter_map(|entry| match entry {
            Entry::Heard { message, .. } => Some(message.content.as_str().to_owned()),
            _ => None,
        })
        .collect()
}

fn stored(store: &MemoryStore) -> Vec<Entry> {
    store
        .records
        .lock()
        .expect("unpoisoned")
        .iter()
        .map(|record| record.entry.clone())
        .collect()
}

#[test]
fn conversation_resumes_after_a_restart() {
    block_on(async {
        let store = Arc::new(MemoryStore::default());

        let (app, mut probe) = FakeChannel::new("app");
        let mut first = ports(app, Vec::new());
        first.model = Arc::new(echo_model());
        first.records = Arc::clone(&store) as _;
        let agent = Agent::spawn(config(Duration::from_secs(1)), first)
            .await
            .expect("valid");
        owner_says(&probe, "m1", "remember the blue door");
        next(&mut probe.delivered).await;
        agent.shutdown().await;

        let (app, mut probe) = FakeChannel::new("app");
        let (asked_tx, mut asked) = mpsc::unbounded_channel();
        let mut second = ports(app, Vec::new());
        second.model = Arc::new(model(move |request| {
            let thread = match request.conversation.first() {
                Some(Entry::Heard { thread, .. }) => *thread,
                _ => panic!("the old message comes first"),
            };
            let _ = asked_tx.send(request.conversation);
            say(thread, "still here")
        }));
        second.records = Arc::clone(&store) as _;
        let agent = Agent::spawn(config(Duration::from_secs(1)), second)
            .await
            .expect("valid");
        probe
            .say
            .send(incoming("m2", "side", "me", "which door?"))
            .expect("channel alive");

        let conversation = next(&mut asked).await;
        assert_eq!(
            heard_texts(&conversation),
            vec!["remember the blue door", "which door?"]
        );
        assert!(
            conversation
                .iter()
                .any(|entry| matches!(entry, Entry::Acted { .. })),
            "the earlier reply resumes too"
        );
        assert_eq!(
            next(&mut probe.delivered).await,
            (Place::new("main"), String::from("still here"))
        );
        agent.shutdown().await;
    });
}

#[test]
fn records_refused_by_the_store_are_appended_once_it_recovers() {
    block_on(async {
        let store = Arc::new(MemoryStore::default());
        store.failing_appends.store(1, Ordering::SeqCst);
        let (app, mut probe) = FakeChannel::new("app");
        let mut ports = ports(app, Vec::new());
        ports.model = Arc::new(echo_model());
        ports.records = Arc::clone(&store) as _;
        let agent = Agent::spawn(config(Duration::from_secs(1)), ports)
            .await
            .expect("valid");

        owner_says(&probe, "m1", "first");
        next(&mut probe.delivered).await;
        owner_says(&probe, "m2", "second");
        next(&mut probe.delivered).await;
        agent.shutdown().await;

        let entries = stored(&store);
        assert_eq!(heard_texts(&entries), vec!["first", "second"]);
        let kinds: Vec<&str> = entries
            .iter()
            .map(|entry| match entry {
                Entry::Heard { .. } => "heard",
                Entry::Acted { .. } => "acted",
                _ => "other",
            })
            .collect();
        assert_eq!(kinds, vec!["heard", "acted", "heard", "acted"]);
    });
}

#[test]
fn spawn_fails_when_the_record_cannot_be_loaded() {
    block_on(async {
        let (app, _probe) = FakeChannel::new("app");
        let mut ports = ports(app, Vec::new());
        ports.records = Arc::new(MemoryStore {
            failing_load: true,
            ..MemoryStore::default()
        });

        let result = Agent::spawn(config(Duration::from_secs(1)), ports).await;

        assert!(matches!(result, Err(SpawnError::Load(_))));
    });
}

#[test]
fn refused_record_is_retried_while_the_agent_is_idle() {
    block_on(async {
        let store = Arc::new(MemoryStore::default());
        store.failing_appends.store(2, Ordering::SeqCst);
        let (app, mut probe) = FakeChannel::new("app");
        let mut ports = ports(app, Vec::new());
        ports.model = Arc::new(echo_model());
        ports.records = Arc::clone(&store) as _;
        let agent = Agent::spawn(config(Duration::from_secs(1)), ports)
            .await
            .expect("valid");

        owner_says(&probe, "m1", "only message");
        next(&mut probe.delivered).await;

        time::timeout(Duration::from_secs(5), async {
            while heard_texts(&stored(&store)).is_empty() {
                time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .expect("record stored without another message");
        agent.shutdown().await;
    });
}

#[test]
fn message_redelivered_after_a_restart_is_heard_once() {
    block_on(async {
        let store = Arc::new(MemoryStore::default());
        let (app, mut probe) = FakeChannel::new("app");
        let mut first = ports(app, Vec::new());
        first.model = Arc::new(echo_model());
        first.records = Arc::clone(&store) as _;
        let agent = Agent::spawn(config(Duration::from_secs(1)), first)
            .await
            .expect("valid");
        owner_says(&probe, "m1", "hi");
        next(&mut probe.delivered).await;
        agent.shutdown().await;

        let (app, probe) = FakeChannel::new("app");
        let (asked_tx, mut asked) = mpsc::unbounded_channel();
        let mut second = ports(app, Vec::new());
        second.model = Arc::new(model(move |request| {
            let _ = asked_tx.send(heard_texts(&request.conversation));
            silence()
        }));
        second.records = Arc::clone(&store) as _;
        let agent = Agent::spawn(config(Duration::from_secs(1)), second)
            .await
            .expect("valid");
        owner_says(&probe, "m1", "hi");
        owner_says(&probe, "m2", "still there?");

        assert_eq!(next(&mut asked).await, vec!["hi", "still there?"]);
        agent.shutdown().await;
    });
}

fn record(entry: Entry) -> Record {
    Record {
        at: SystemTime::now(),
        entry,
    }
}

#[test]
fn work_cut_off_by_a_restart_is_reported_as_stopped() {
    block_on(async {
        let research = SubSessionSpec {
            title: String::from("research"),
            brief: String::from("look into it"),
            folder: None,
        };
        let store = Arc::new(MemoryStore::default());
        store.records.lock().expect("unpoisoned").extend([
            record(Entry::Heard {
                thread: ThreadRef::new(0),
                message: Message {
                    id: ExternalMessageId::new("m1"),
                    address: Address {
                        channel: channel_id("app"),
                        place: Place::new("main"),
                    },
                    sender: Sender {
                        id: ExternalUserId::new("me"),
                        name: String::from("me"),
                        role: Role::Owner,
                    },
                    content: Content::parse("read it and research").expect("non-empty"),
                    audience: Some(Audience::Owner),
                },
            }),
            record(Entry::Acted {
                actions: vec![tool_call(), Action::Delegate(research.clone())],
                answered: 1,
                trace: None,
            }),
        ]);
        let (app, probe) = FakeChannel::new("app");
        let (asked_tx, mut asked) = mpsc::unbounded_channel();
        let mut ports = ports(app, Vec::new());
        ports.model = Arc::new(model(move |request| {
            let _ = asked_tx.send(request);
            silence()
        }));
        ports.records = Arc::clone(&store) as _;
        let agent = Agent::spawn(config(Duration::from_secs(1)), ports)
            .await
            .expect("valid");

        owner_says(&probe, "m2", "where were we?");

        let request = next(&mut asked).await;
        let error = tool_result(&request)
            .expect("tool result")
            .expect_err("stopped");
        assert!(error.detail.contains("restart"), "{}", error.detail);
        match sub_session_outcome(&request) {
            Some(SubSessionOutcome::Failed(detail)) => {
                assert!(detail.contains("restart"), "{detail}")
            }
            other => panic!("expected a stopped sub-session, got {other:?}"),
        }
        agent.shutdown().await;
        assert!(
            stored(&store)
                .iter()
                .any(|entry| matches!(entry, Entry::ToolReturned { .. })),
            "the stopped result is recorded"
        );
    });
}

#[test]
fn work_another_pc_started_is_left_for_that_pc_to_close() {
    block_on(async {
        let research = SubSessionSpec {
            title: String::from("research"),
            brief: String::from("look into it"),
            folder: None,
        };
        let store = Arc::new(MemoryStore {
            elsewhere: vec![0],
            ..MemoryStore::default()
        });
        store
            .records
            .lock()
            .expect("unpoisoned")
            .push(record(Entry::Acted {
                actions: vec![tool_call(), Action::Delegate(research)],
                answered: 0,
                trace: None,
            }));
        let (app, probe) = FakeChannel::new("app");
        let (asked_tx, mut asked) = mpsc::unbounded_channel();
        let mut ports = ports(app, Vec::new());
        ports.model = Arc::new(model(move |request| {
            let _ = asked_tx.send(request);
            silence()
        }));
        ports.records = Arc::clone(&store) as _;
        let agent = Agent::spawn(config(Duration::from_secs(1)), ports)
            .await
            .expect("valid");

        owner_says(&probe, "m1", "where were we?");

        let request = next(&mut asked).await;
        assert!(tool_result(&request).is_none());
        assert!(sub_session_outcome(&request).is_none());
        agent.shutdown().await;
        assert!(
            !stored(&store).iter().any(|entry| matches!(
                entry,
                Entry::ToolReturned { .. } | Entry::SubSessionReturned { .. }
            )),
            "nothing is closed for the other PC"
        );
    });
}

#[test]
fn a_sub_session_still_running_when_the_main_session_stops_is_recorded_as_stopped() {
    block_on(async {
        let store = Arc::new(MemoryStore::default());
        let (app, probe) = FakeChannel::new("app");
        let mut ports = ports(app, Vec::new());
        let asked = Arc::new(AtomicUsize::new(0));
        let counting = Arc::clone(&asked);
        ports.model = Arc::new(model(move |_| {
            if counting.fetch_add(1, Ordering::SeqCst) == 0 {
                ModelReply {
                    actions: vec![Action::Delegate(SubSessionSpec {
                        title: String::from("long"),
                        brief: String::from("take your time"),
                        folder: None,
                    })],
                    trace: None,
                }
            } else {
                silence()
            }
        }));
        let (started_tx, mut started) = mpsc::unbounded_channel();
        ports.sub_sessions = Arc::new(FnSubSessions(Box::new(move |_| {
            let _ = started_tx.send(());
            Box::pin(std::future::pending())
        })));
        ports.records = Arc::clone(&store) as _;
        let agent = Agent::spawn(config(Duration::from_secs(1)), ports)
            .await
            .expect("valid");

        owner_says(&probe, "m1", "start the long job");
        time::timeout(Duration::from_secs(5), started.recv())
            .await
            .expect("started in time");
        agent.shutdown().await;

        let stopped = stored(&store).into_iter().find_map(|entry| match entry {
            Entry::SubSessionReturned {
                outcome: SubSessionOutcome::Failed(detail),
                ..
            } => Some(detail),
            _ => None,
        });
        assert!(
            stopped.is_some_and(|detail| detail.contains("stopped running on this PC")),
            "the PC that started it says it stopped"
        );
    });
}

#[test]
fn a_result_another_pc_recorded_does_not_close_work_started_here() {
    block_on(async {
        let research = SubSessionSpec {
            title: String::from("research"),
            brief: String::from("look into it"),
            folder: None,
        };
        let store = Arc::new(MemoryStore {
            elsewhere: vec![1],
            ..MemoryStore::default()
        });
        store.records.lock().expect("unpoisoned").extend([
            record(Entry::Acted {
                actions: vec![Action::Delegate(research.clone())],
                answered: 0,
                trace: None,
            }),
            record(Entry::SubSessionReturned {
                spec: research,
                outcome: SubSessionOutcome::Finished(String::from("theirs")),
            }),
        ]);
        let (app, probe) = FakeChannel::new("app");
        let (asked_tx, mut asked) = mpsc::unbounded_channel();
        let mut ports = ports(app, Vec::new());
        ports.model = Arc::new(model(move |request| {
            let _ = asked_tx.send(request);
            silence()
        }));
        ports.records = Arc::clone(&store) as _;
        let agent = Agent::spawn(config(Duration::from_secs(1)), ports)
            .await
            .expect("valid");

        owner_says(&probe, "m1", "where were we?");
        let request = next(&mut asked).await;
        agent.shutdown().await;

        let stopped = request
            .conversation
            .iter()
            .filter(|entry| {
                matches!(
                    entry,
                    Entry::SubSessionReturned {
                        outcome: SubSessionOutcome::Failed(_),
                        ..
                    }
                )
            })
            .count();
        assert_eq!(stopped, 1, "this PC still closes its own delegation");
    });
}
