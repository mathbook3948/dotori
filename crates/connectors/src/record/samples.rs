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
use dotori_core::record::Record;
use dotori_core::sub_task::SubTaskOutcome;
use dotori_core::sub_task::SubTaskSpec;
use dotori_core::tool::ToolCall;
use dotori_core::tool::ToolCallId;
use dotori_core::tool::ToolError;

pub(crate) fn record(entry: Entry) -> Record {
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

pub(crate) fn every_entry() -> Vec<Entry> {
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

pub(crate) fn shown(records: &[Record]) -> Vec<String> {
    records
        .iter()
        .map(|record| format!("{:?} {:?}", record.at, record.entry))
        .collect()
}
