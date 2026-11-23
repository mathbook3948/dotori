use std::sync::Mutex;

use dotori_core::llm::ModelError;
use dotori_core::llm::ModelReply;
use dotori_core::tool::ToolCall;
use dotori_core::tool::ToolCallId;
use dotori_core::tool::ToolSpec;

use super::*;

struct Scripted {
    replies: Mutex<Vec<Vec<Action>>>,
    seen: Mutex<Vec<usize>>,
}

impl Model for Scripted {
    fn complete(&self, request: ModelRequest) -> BoxFuture<'_, Result<ModelReply, ModelError>> {
        self.seen
            .lock()
            .expect("unpoisoned")
            .push(request.conversation.len());
        let next = self.replies.lock().expect("unpoisoned").pop();
        Box::pin(async move {
            next.map(|actions| ModelReply {
                actions,
                trace: None,
            })
            .ok_or_else(|| ModelError {
                detail: String::from("no more replies"),
            })
        })
    }
}

struct Echo;

impl Toolbox for Echo {
    fn specs(&self) -> Vec<ToolSpec> {
        vec![ToolSpec {
            name: String::from("echo"),
            description: String::from("Echoes."),
            input_schema: String::from(r#"{"type":"object"}"#),
        }]
    }

    fn run(&self, call: ToolCall) -> BoxFuture<'_, Result<String, ToolError>> {
        Box::pin(async move { Ok(format!("echo {}", call.input)) })
    }
}

fn runner(mut replies: Vec<Vec<Action>>, max_turns: usize) -> (LlmRunner, Arc<Scripted>) {
    replies.reverse();
    let model = Arc::new(Scripted {
        replies: Mutex::new(replies),
        seen: Mutex::new(Vec::new()),
    });
    let runner = LlmRunner {
        model: Arc::clone(&model) as Arc<dyn Model>,
        toolbox: Arc::new(Echo),
        max_turns,
        tool_timeout: Duration::from_secs(5),
    };
    (runner, model)
}

fn spec() -> SubSessionSpec {
    SubSessionSpec {
        title: String::from("Look up"),
        brief: String::from("Find the answer."),
    }
}

fn tool(id: &str) -> Action {
    Action::UseTool(ToolCall {
        id: ToolCallId::new(id),
        name: String::from("echo"),
        input: String::from("{}"),
    })
}

fn say(thread: u32, text: &str) -> Action {
    Action::Say {
        thread: ThreadRef::new(thread),
        text: String::from(text),
    }
}

#[tokio::test]
async fn tools_run_until_the_model_answers_on_the_brief_thread() {
    let (runner, model) = runner(vec![vec![tool("a")], vec![say(0, "42")]], 5);
    let outcome = runner.run(spec()).await;
    assert!(matches!(outcome, SubSessionOutcome::Finished(text) if text == "42"));
    assert_eq!(*model.seen.lock().expect("unpoisoned"), [1, 3]);
}

#[tokio::test]
async fn an_answer_on_another_thread_or_a_delegation_is_not_a_result() {
    let delegate = Action::Delegate(spec());
    let (runner, _) = runner(vec![vec![say(3, "hi"), delegate]], 5);
    let outcome = runner.run(spec()).await;
    assert!(
        matches!(outcome, SubSessionOutcome::Failed(text) if text.contains("without a result"))
    );
}

#[tokio::test]
async fn a_sub_session_stops_after_its_turns_run_out() {
    let (runner, _) = runner(vec![vec![tool("a")], vec![tool("b")]], 2);
    let outcome = runner.run(spec()).await;
    assert!(matches!(outcome, SubSessionOutcome::Failed(text) if text.contains("2 model turns")));
}

#[tokio::test]
async fn a_model_failure_fails_the_sub_session() {
    let (runner, _) = runner(Vec::new(), 2);
    let outcome = runner.run(spec()).await;
    assert!(matches!(outcome, SubSessionOutcome::Failed(text) if text.contains("no more replies")));
}

#[tokio::test]
async fn an_answer_given_alongside_tools_is_not_the_final_result() {
    let (runner, _) = runner(
        vec![vec![say(0, "partial"), tool("a")], vec![say(3, "bye")]],
        5,
    );
    let outcome = runner.run(spec()).await;
    assert!(
        matches!(outcome, SubSessionOutcome::Failed(text) if text.contains("without a result"))
    );
}
