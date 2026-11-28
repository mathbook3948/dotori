use std::time::Duration;
use std::time::UNIX_EPOCH;

use dotori_core::conversation::Action;
use dotori_core::conversation::Entry;
use dotori_core::conversation::MessageRef;
use dotori_core::conversation::ThreadRef;
use dotori_core::id::ChannelId;
use dotori_core::id::ExternalMessageId;
use dotori_core::id::ExternalUserId;
use dotori_core::llm::ModelTrace;
use dotori_core::message::Address;
use dotori_core::message::Audience;
use dotori_core::message::Content;
use dotori_core::message::Message;
use dotori_core::message::Place;
use dotori_core::message::Role;
use dotori_core::message::Sender;
use dotori_core::record::Record;
use dotori_core::sub_session::SubSessionOutcome;
use dotori_core::sub_session::SubSessionSpec;
use dotori_core::tool::ToolCall;
use dotori_core::tool::ToolCallId;
use dotori_core::tool::ToolError;
use serde::Deserialize;
use serde::Serialize;

/// One record as a line of the file. Field names are the file format, so they never change.
#[derive(Serialize, Deserialize)]
pub(crate) struct Line {
    at_ms: u64,
    entry: EntryForm,
}

#[derive(Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
enum EntryForm {
    Heard {
        thread: u32,
        message: MessageForm,
    },
    Acted {
        actions: Vec<ActionForm>,
        answered: usize,
        trace: Option<TraceForm>,
    },
    ToolReturned {
        call: String,
        result: OutcomeForm,
    },
    #[serde(rename = "sub_task_returned")]
    SubSessionReturned {
        spec: SpecForm,
        result: OutcomeForm,
    },
    Refused {
        action: ActionForm,
    },
    DeliveryFailed {
        thread: u32,
        text: String,
        detail: String,
    },
    ModelFailed {
        detail: String,
    },
    ChannelEnded {
        channel: String,
        detail: Option<String>,
    },
    Summarized {
        channel: String,
        message: String,
        text: String,
    },
}

#[derive(Serialize, Deserialize)]
struct MessageForm {
    id: String,
    channel: String,
    place: String,
    sender_id: String,
    sender_name: String,
    owner: bool,
    content: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    audience: Option<AudienceForm>,
}

#[derive(Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
enum AudienceForm {
    Owner,
    People { some: Vec<String>, total: usize },
    Anyone { scope: String },
}

#[derive(Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
enum ActionForm {
    Say {
        thread: u32,
        text: String,
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        privately: bool,
    },
    UseTool {
        id: String,
        name: String,
        input: String,
    },
    Delegate(SpecForm),
}

#[derive(Serialize, Deserialize)]
struct SpecForm {
    title: String,
    brief: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    folder: Option<String>,
}

#[derive(Serialize, Deserialize)]
struct TraceForm {
    provider: String,
    data: String,
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum OutcomeForm {
    Ok(String),
    Failed(String),
}

impl Line {
    pub(crate) fn of(record: &Record) -> Self {
        let at_ms = record
            .at
            .duration_since(UNIX_EPOCH)
            .map(|since| u64::try_from(since.as_millis()).unwrap_or(u64::MAX))
            .unwrap_or(0);
        Self {
            at_ms,
            entry: entry_form(&record.entry),
        }
    }

    pub(crate) fn at_ms(&self) -> u64 {
        self.at_ms
    }

    /// The record this line keeps. A message takes the record's time as the time it was heard,
    /// since a message is recorded as it arrives.
    pub(crate) fn into_record(self) -> Result<Record, String> {
        let at = UNIX_EPOCH + Duration::from_millis(self.at_ms);
        let mut entry = entry(self.entry)?;
        if let Entry::Heard { message, .. } = &mut entry {
            message.at = Some(at);
        }
        Ok(Record { at, entry })
    }
}

fn entry_form(entry: &Entry) -> EntryForm {
    match entry {
        Entry::Heard { thread, message } => EntryForm::Heard {
            thread: thread.get(),
            message: MessageForm {
                id: message.id.as_str().to_owned(),
                channel: message.address.channel.as_str().to_owned(),
                place: message.address.place.as_str().to_owned(),
                sender_id: message.sender.id.as_str().to_owned(),
                sender_name: message.sender.name.clone(),
                owner: message.sender.role == Role::Owner,
                content: message.content.as_str().to_owned(),
                audience: message.audience.as_ref().map(|audience| match audience {
                    Audience::Owner => AudienceForm::Owner,
                    Audience::People { some, total } => AudienceForm::People {
                        some: some.clone(),
                        total: *total,
                    },
                    Audience::Anyone(scope) => AudienceForm::Anyone {
                        scope: scope.clone(),
                    },
                }),
            },
        },
        Entry::Acted {
            actions,
            answered,
            trace,
        } => EntryForm::Acted {
            actions: actions.iter().map(action_form).collect(),
            answered: *answered,
            trace: trace.as_ref().map(|trace| TraceForm {
                provider: trace.provider.clone(),
                data: trace.data.clone(),
            }),
        },
        Entry::ToolReturned { call, result } => EntryForm::ToolReturned {
            call: call.as_str().to_owned(),
            result: match result {
                Ok(output) => OutcomeForm::Ok(output.clone()),
                Err(error) => OutcomeForm::Failed(error.detail.clone()),
            },
        },
        Entry::SubSessionReturned { spec, outcome } => EntryForm::SubSessionReturned {
            spec: spec_form(spec),
            result: match outcome {
                SubSessionOutcome::Finished(text) => OutcomeForm::Ok(text.clone()),
                SubSessionOutcome::Failed(text) => OutcomeForm::Failed(text.clone()),
            },
        },
        Entry::Refused(action) => EntryForm::Refused {
            action: action_form(action),
        },
        Entry::DeliveryFailed {
            thread,
            text,
            detail,
        } => EntryForm::DeliveryFailed {
            thread: thread.get(),
            text: text.clone(),
            detail: detail.clone(),
        },
        Entry::ModelFailed { detail } => EntryForm::ModelFailed {
            detail: detail.clone(),
        },
        Entry::ChannelEnded { channel, detail } => EntryForm::ChannelEnded {
            channel: channel.as_str().to_owned(),
            detail: detail.clone(),
        },
        Entry::Summarized { kept, text } => EntryForm::Summarized {
            channel: kept.channel.as_str().to_owned(),
            message: kept.id.as_str().to_owned(),
            text: text.clone(),
        },
    }
}

fn action_form(action: &Action) -> ActionForm {
    match action {
        Action::Say {
            thread,
            text,
            privately,
        } => ActionForm::Say {
            thread: thread.get(),
            text: text.clone(),
            privately: *privately,
        },
        Action::UseTool(call) => ActionForm::UseTool {
            id: call.id.as_str().to_owned(),
            name: call.name.clone(),
            input: call.input.clone(),
        },
        Action::Delegate(spec) => ActionForm::Delegate(spec_form(spec)),
    }
}

fn spec_form(spec: &SubSessionSpec) -> SpecForm {
    SpecForm {
        title: spec.title.clone(),
        brief: spec.brief.clone(),
        folder: spec.folder.clone(),
    }
}

fn entry(form: EntryForm) -> Result<Entry, String> {
    Ok(match form {
        EntryForm::Heard { thread, message } => Entry::Heard {
            thread: ThreadRef::new(thread),
            message: Message {
                id: ExternalMessageId::new(message.id),
                address: Address {
                    channel: channel(&message.channel)?,
                    place: Place::new(message.place),
                },
                sender: Sender {
                    id: ExternalUserId::new(message.sender_id),
                    name: message.sender_name,
                    role: if message.owner {
                        Role::Owner
                    } else {
                        Role::Other
                    },
                },
                content: Content::parse(message.content).map_err(|error| error.to_string())?,
                audience: message.audience.map(|audience| match audience {
                    AudienceForm::Owner => Audience::Owner,
                    AudienceForm::People { some, total } => Audience::People { some, total },
                    AudienceForm::Anyone { scope } => Audience::Anyone(scope),
                }),
                at: None,
            },
        },
        EntryForm::Acted {
            actions,
            answered,
            trace,
        } => Entry::Acted {
            actions: actions.into_iter().map(action).collect(),
            answered,
            trace: trace.map(|trace| ModelTrace {
                provider: trace.provider,
                data: trace.data,
            }),
        },
        EntryForm::ToolReturned { call, result } => Entry::ToolReturned {
            call: ToolCallId::new(call),
            result: match result {
                OutcomeForm::Ok(output) => Ok(output),
                OutcomeForm::Failed(detail) => Err(ToolError { detail }),
            },
        },
        EntryForm::SubSessionReturned { spec, result } => Entry::SubSessionReturned {
            spec: spec_of(spec),
            outcome: match result {
                OutcomeForm::Ok(text) => SubSessionOutcome::Finished(text),
                OutcomeForm::Failed(text) => SubSessionOutcome::Failed(text),
            },
        },
        EntryForm::Refused { action: form } => Entry::Refused(action(form)),
        EntryForm::DeliveryFailed {
            thread,
            text,
            detail,
        } => Entry::DeliveryFailed {
            thread: ThreadRef::new(thread),
            text,
            detail,
        },
        EntryForm::ModelFailed { detail } => Entry::ModelFailed { detail },
        EntryForm::ChannelEnded {
            channel: id,
            detail,
        } => Entry::ChannelEnded {
            channel: channel(&id)?,
            detail,
        },
        EntryForm::Summarized {
            channel: id,
            message,
            text,
        } => Entry::Summarized {
            kept: MessageRef {
                channel: channel(&id)?,
                id: ExternalMessageId::new(message),
            },
            text,
        },
    })
}

fn action(form: ActionForm) -> Action {
    match form {
        ActionForm::Say {
            thread,
            text,
            privately,
        } => Action::Say {
            thread: ThreadRef::new(thread),
            text,
            privately,
        },
        ActionForm::UseTool { id, name, input } => Action::UseTool(ToolCall {
            id: ToolCallId::new(id),
            name,
            input,
        }),
        ActionForm::Delegate(spec) => Action::Delegate(spec_of(spec)),
    }
}

fn spec_of(form: SpecForm) -> SubSessionSpec {
    SubSessionSpec {
        title: form.title,
        brief: form.brief,
        folder: form.folder,
    }
}

fn channel(raw: &str) -> Result<ChannelId, String> {
    ChannelId::parse(raw).map_err(|error| error.to_string())
}
