use std::path::Path;
use std::sync::atomic::AtomicUsize;
use std::sync::atomic::Ordering;
use std::time::Duration;
use std::time::UNIX_EPOCH;

use dotori_core::conversation::Action;
use dotori_core::conversation::Entry;
use dotori_core::conversation::ThreadRef;
use dotori_core::id::ChannelId;
use dotori_core::id::ExternalMessageId;
use dotori_core::id::ExternalUserId;
use dotori_core::llm::ModelTrace;
use dotori_core::message::Address;
use dotori_core::message::Content;
use dotori_core::message::Message;
use dotori_core::message::Place;
use dotori_core::message::Role;
use dotori_core::message::Sender;
use dotori_core::sub_task::SubTaskOutcome;
use dotori_core::sub_task::SubTaskSpec;
use dotori_core::tool::ToolCall;
use dotori_core::tool::ToolCallId;
use dotori_core::tool::ToolError;

use super::*;

static NEXT: AtomicUsize = AtomicUsize::new(0);

fn scratch() -> PathBuf {
    let n = NEXT.fetch_add(1, Ordering::SeqCst);
    let dir = std::env::temp_dir().join(format!("dotori-jsonl-{}-{n}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    dir.join("nested").join("record.jsonl")
}

fn record(entry: Entry) -> Record {
    Record {
        at: UNIX_EPOCH + Duration::from_millis(1_700_000_000_123),
        entry,
    }
}

fn spec() -> SubTaskSpec {
    SubTaskSpec {
        title: String::from("Build"),
        brief: String::from("Build \"it\"\nnow."),
    }
}

fn every_entry() -> Vec<Entry> {
    vec![
        Entry::Heard {
            thread: ThreadRef::new(4),
            message: Message {
                id: ExternalMessageId::new("m1"),
                address: Address {
                    channel: ChannelId::parse("slack").expect("valid"),
                    place: Place::new("C1/123.4"),
                },
                sender: Sender {
                    id: ExternalUserId::new("U1"),
                    name: String::from("유노"),
                    role: Role::Owner,
                },
                content: Content::parse("안녕\n둘째 줄").expect("non-empty"),
            },
        },
        Entry::Acted {
            actions: vec![
                Action::Say {
                    thread: ThreadRef::new(4),
                    text: String::from("hi"),
                },
                Action::UseTool(ToolCall {
                    id: ToolCallId::new("toolu_1"),
                    name: String::from("read_file"),
                    input: String::from(r#"{"path":"a"}"#),
                }),
                Action::Delegate(spec()),
            ],
            answered: 1,
            trace: Some(ModelTrace {
                provider: String::from("anthropic"),
                data: String::from(r#"{"raw":[]}"#),
            }),
        },
        Entry::ToolReturned {
            call: ToolCallId::new("toolu_1"),
            result: Ok(String::from("contents")),
        },
        Entry::ToolReturned {
            call: ToolCallId::new("toolu_2"),
            result: Err(ToolError {
                detail: String::from("denied"),
            }),
        },
        Entry::SubTaskReturned {
            spec: spec(),
            outcome: SubTaskOutcome::Finished(String::from("done")),
        },
        Entry::SubTaskReturned {
            spec: spec(),
            outcome: SubTaskOutcome::Failed(String::from("broke")),
        },
        Entry::Refused(Action::Say {
            thread: ThreadRef::new(9),
            text: String::from("lost"),
        }),
        Entry::DeliveryFailed {
            thread: ThreadRef::new(4),
            text: String::from("hi"),
            detail: String::from("offline"),
        },
        Entry::ModelFailed {
            detail: String::from("overloaded"),
        },
        Entry::ChannelEnded {
            channel: ChannelId::parse("slack").expect("valid"),
            detail: None,
        },
    ]
}

fn shown(records: &[Record]) -> Vec<String> {
    records
        .iter()
        .map(|record| format!("{:?} {:?}", record.at, record.entry))
        .collect()
}

#[tokio::test]
async fn every_entry_survives_a_round_trip() {
    let store = JsonlStore::new(scratch());
    let records: Vec<Record> = every_entry().into_iter().map(record).collect();

    store.append(records[..3].to_vec()).await.expect("appended");
    store.append(records[3..].to_vec()).await.expect("appended");

    let loaded = store.load().await.expect("loaded");
    assert_eq!(shown(&loaded), shown(&records));
}

#[tokio::test]
async fn a_missing_file_is_an_empty_record() {
    let store = JsonlStore::new(scratch());

    assert!(store.load().await.expect("loaded").is_empty());
}

fn torn(path: &Path) {
    let mut file = OpenOptions::new().append(true).open(path).expect("open");
    let cut = "[{\"at_ms\":1,\"entry\":{\"kind\":\"model_failed\",\"detail\":\"한";
    file.write_all(&cut.as_bytes()[..cut.len() - 1])
        .expect("write");
}

#[tokio::test]
async fn a_line_cut_short_by_a_crash_is_dropped_and_cut_away() {
    let path = scratch();
    let store = JsonlStore::new(path.clone());
    let first = record(Entry::ModelFailed {
        detail: String::from("one"),
    });
    store.append(vec![first.clone()]).await.expect("appended");
    torn(&path);

    assert_eq!(
        shown(&store.load().await.expect("loaded")),
        shown(std::slice::from_ref(&first))
    );

    let second = record(Entry::ModelFailed {
        detail: String::from("two"),
    });
    store.append(vec![second.clone()]).await.expect("appended");

    assert_eq!(
        shown(&store.load().await.expect("loaded")),
        shown(&[first, second])
    );
}

#[tokio::test]
async fn a_damaged_whole_line_fails_the_load_and_names_it() {
    let path = scratch();
    let store = JsonlStore::new(path.clone());
    store
        .append(vec![record(Entry::ModelFailed {
            detail: String::from("one"),
        })])
        .await
        .expect("appended");
    torn(&path);
    let mut file = OpenOptions::new().append(true).open(&path).expect("open");
    file.write_all(b"\n").expect("write");

    let error = store.load().await.expect_err("damaged");

    assert!(error.detail.starts_with("line 2 of "), "{}", error.detail);
}
