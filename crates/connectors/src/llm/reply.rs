use dotori_core::conversation::Action;
use dotori_core::conversation::ThreadRef;
use dotori_core::sub_session::SubSessionSpec;
use dotori_core::tool::ToolCall;
use dotori_core::tool::ToolCallId;
use dotori_core::tool::ToolSpec;
use serde::Deserialize;
use serde::Serialize;
use serde_json::value::RawValue;

pub(crate) const SAY: &str = "say";
pub(crate) const DELEGATE: &str = "delegate";

/// One tool call as the provider wrote it. `input` is JSON text.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Use {
    pub(crate) id: String,
    pub(crate) name: String,
    pub(crate) input: String,
}

/// A call the connector turned down before core saw it, such as `say` without a thread.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct Rejected {
    pub(crate) id: String,
    pub(crate) detail: String,
}

/// What a connector stores in `ModelTrace::data`. `ids[k]` is the provider's id for action `k`.
#[derive(Debug, Serialize, Deserialize)]
pub(crate) struct Trace {
    pub(crate) raw: Box<RawValue>,
    pub(crate) ids: Vec<String>,
    pub(crate) rejected: Vec<Rejected>,
}

/// The reply split into what core runs and what was turned down, in the provider's order.
#[derive(Debug, Default)]
pub(crate) struct Decided {
    pub(crate) actions: Vec<Action>,
    pub(crate) ids: Vec<String>,
    pub(crate) rejected: Vec<Rejected>,
}

#[derive(Deserialize)]
struct SayInput {
    thread: u32,
    text: String,
    #[serde(default)]
    privately: bool,
}

#[derive(Deserialize)]
struct DelegateInput {
    title: String,
    brief: String,
    #[serde(default)]
    folder: Option<String>,
}

pub(crate) fn decide(uses: Vec<Use>) -> Decided {
    let mut decided = Decided::default();
    for call in uses {
        match action_of(&call) {
            Ok(action) => {
                decided.ids.push(call.id);
                decided.actions.push(action);
            }
            Err(detail) => decided.rejected.push(Rejected {
                id: call.id,
                detail,
            }),
        }
    }
    decided
}

fn action_of(call: &Use) -> Result<Action, String> {
    match call.name.as_str() {
        SAY => {
            let input: SayInput = parse(&call.input)?;
            if input.text.trim().is_empty() {
                return Err(String::from("`text` is empty"));
            }
            Ok(Action::Say {
                thread: ThreadRef::new(input.thread),
                text: input.text,
                privately: input.privately,
            })
        }
        DELEGATE => {
            let input: DelegateInput = parse(&call.input)?;
            if input.brief.trim().is_empty() {
                return Err(String::from("`brief` is empty"));
            }
            Ok(Action::Delegate(SubSessionSpec {
                title: input.title,
                brief: input.brief,
                folder: input
                    .folder
                    .map(|folder| folder.trim().to_owned())
                    .filter(|folder| !folder.is_empty()),
            }))
        }
        _ => Ok(Action::UseTool(ToolCall {
            id: ToolCallId::new(call.id.as_str()),
            name: call.name.clone(),
            input: call.input.clone(),
        })),
    }
}

fn parse<'a, T: Deserialize<'a>>(input: &'a str) -> Result<T, String> {
    serde_json::from_str(input).map_err(|error| format!("invalid input: {error}"))
}

/// The system prompt with what the app added after it.
pub(crate) fn system(base: &str, context: Option<&str>) -> String {
    match context.filter(|context| !context.trim().is_empty()) {
        Some(context) => format!("{base}\n\n{context}"),
        None => base.to_owned(),
    }
}

/// The tools the model sees: core's built-in `say` and `delegate`, then the toolbox's.
/// A toolbox tool that reuses a built-in name is left out.
pub(crate) fn tools(toolbox: &[ToolSpec]) -> Vec<ToolSpec> {
    let mut tools = vec![
        ToolSpec {
            name: String::from(SAY),
            description: String::from(
                "Send a message to a thread. This is the only way anyone sees what you write. \
                 When others can read the thread, keep the owner's private matters out of it and \
                 send them with `privately`, which goes to a room on the same channel that only \
                 the owner reads.",
            ),
            input_schema: String::from(
                r#"{"type":"object","properties":{"thread":{"type":"integer","description":"The thread number from the message you answer."},"text":{"type":"string","description":"What to send."},"privately":{"type":"boolean","description":"Send it to the owner alone on the thread's channel instead of to the thread."}},"required":["thread","text"],"additionalProperties":false}"#,
            ),
        },
        ToolSpec {
            name: String::from(DELEGATE),
            description: String::from(
                "Hand a large job to a sub-session that works on its own. The brief is all it knows.",
            ),
            input_schema: String::from(
                r#"{"type":"object","properties":{"title":{"type":"string","description":"A few words naming the job."},"brief":{"type":"string","description":"Everything the sub-session needs to know to do the job."},"folder":{"type":"string","description":"An absolute folder on the owner's PC the job is about, such as a project to change. A coding agent starts there."}},"required":["title","brief"],"additionalProperties":false}"#,
            ),
        },
    ];
    tools.extend(
        toolbox
            .iter()
            .filter(|spec| spec.name != SAY && spec.name != DELEGATE)
            .cloned(),
    );
    tools
}

#[cfg(test)]
#[path = "reply.test.rs"]
mod tests;
