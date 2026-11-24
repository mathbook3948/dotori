use std::path::Path;
use std::path::PathBuf;
use std::sync::Mutex;
use std::sync::atomic::AtomicUsize;
use std::sync::atomic::Ordering;

use dotori_core::llm::ModelError;
use dotori_core::llm::ModelReply;
use dotori_core::tool::ToolCall;
use dotori_core::tool::ToolCallId;
use dotori_core::tool::ToolSpec;
use serde_json::Value;
use serde_json::json;

use super::*;
use crate::sub_session::plain;
use crate::tool::basic::Private;
use crate::tool::basic::READ_FILE;
use crate::tool::basic::RUN_SHELL;
use crate::tool::basic::WRITE_FILE;

static NEXT: AtomicUsize = AtomicUsize::new(0);

struct Scripted {
    replies: Mutex<Vec<Vec<Action>>>,
    seen: Mutex<Vec<usize>>,
    heard: Mutex<Vec<Entry>>,
    tools: Mutex<Vec<ToolSpec>>,
}

impl Model for Scripted {
    fn complete(&self, request: ModelRequest) -> BoxFuture<'_, Result<ModelReply, ModelError>> {
        self.seen
            .lock()
            .expect("unpoisoned")
            .push(request.conversation.len());
        *self.heard.lock().expect("unpoisoned") = request.conversation;
        *self.tools.lock().expect("unpoisoned") = request.tools;
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

impl Scripted {
    fn returned(&self) -> Vec<Result<String, String>> {
        self.heard
            .lock()
            .expect("unpoisoned")
            .iter()
            .filter_map(|entry| match entry {
                Entry::ToolReturned { result, .. } => {
                    Some(result.clone().map_err(|error| error.detail))
                }
                _ => None,
            })
            .collect()
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

struct Dotori {
    root: PathBuf,
    work: PathBuf,
    project: PathBuf,
}

impl Dotori {
    fn new() -> Self {
        let n = NEXT.fetch_add(1, Ordering::SeqCst);
        let root = std::env::temp_dir().join(format!("dotori-llm-{}-{n}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let work = root.join("dotori").join("work");
        let project = work.join("project");
        std::fs::create_dir_all(&project).expect("made");
        std::fs::write(root.join("dotori").join("settings.toml"), "secret").expect("written");
        Self {
            root,
            work,
            project,
        }
    }

    fn basic(&self) -> BasicTools {
        BasicTools {
            folder: self.work.clone(),
            limit: Duration::from_secs(5),
            hidden_env: Vec::new(),
            private: Some(Private {
                folder: self.root.join("dotori"),
                open: self.work.clone(),
            }),
        }
    }
}

impl Drop for Dotori {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

fn runner_with(
    basic: BasicTools,
    mut replies: Vec<Vec<Action>>,
    max_turns: usize,
) -> (LlmRunner, Arc<Scripted>) {
    replies.reverse();
    let model = Arc::new(Scripted {
        replies: Mutex::new(replies),
        seen: Mutex::new(Vec::new()),
        heard: Mutex::new(Vec::new()),
        tools: Mutex::new(Vec::new()),
    });
    let runner = LlmRunner {
        model: Arc::clone(&model) as Arc<dyn Model>,
        basic,
        rest: Arc::new(Echo),
        max_turns,
        tool_timeout: Duration::from_secs(5),
    };
    (runner, model)
}

fn runner(replies: Vec<Vec<Action>>, max_turns: usize) -> (LlmRunner, Arc<Scripted>) {
    let basic = BasicTools {
        folder: std::env::temp_dir(),
        limit: Duration::from_secs(5),
        hidden_env: Vec::new(),
        private: None,
    };
    runner_with(basic, replies, max_turns)
}

fn spec() -> SubSessionSpec {
    SubSessionSpec {
        title: String::from("Look up"),
        brief: String::from("Find the answer."),
        folder: None,
    }
}

fn about(folder: &Path) -> SubSessionSpec {
    SubSessionSpec {
        folder: Some(folder.display().to_string()),
        ..spec()
    }
}

fn tool(id: &str) -> Action {
    use_tool(id, "echo", json!({}))
}

fn use_tool(id: &str, name: &str, input: Value) -> Action {
    Action::UseTool(ToolCall {
        id: ToolCallId::new(id),
        name: String::from(name),
        input: input.to_string(),
    })
}

fn where_am_i(id: &str) -> Action {
    let command = if cfg!(windows) { "cd" } else { "pwd" };
    use_tool(id, RUN_SHELL, json!({ "command": command }))
}

fn printed_folder(shell: &Result<String, String>) -> PathBuf {
    let output = shell.as_ref().expect("the shell ran");
    let line = output.lines().nth(2).expect("a printed folder");
    Path::new(line.trim()).canonicalize().expect("a folder")
}

fn say(thread: u32, text: &str) -> Action {
    Action::Say {
        thread: ThreadRef::new(thread),
        text: String::from(text),
        privately: false,
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

#[tokio::test]
async fn a_job_about_a_folder_tells_the_model_through_its_tools_where_it_works() {
    let dotori = Dotori::new();
    let (runner, model) = runner_with(dotori.basic(), vec![vec![say(0, "done")]], 5);

    runner.run(about(&dotori.project)).await;

    let project = dotori.project.canonicalize().expect("exists");
    let tools = model.tools.lock().expect("unpoisoned").clone();
    for name in [READ_FILE, RUN_SHELL] {
        let spec = tools
            .iter()
            .find(|spec| spec.name == name)
            .expect("offered");
        assert!(
            spec.description
                .contains(&plain(project.clone()).display().to_string()),
            "{}",
            spec.description
        );
    }
    assert!(tools.iter().any(|spec| spec.name == "echo"));
}

#[tokio::test]
async fn a_job_about_a_folder_reads_relative_paths_and_runs_its_shell_there() {
    let dotori = Dotori::new();
    std::fs::write(dotori.project.join("notes.txt"), "in the project").expect("written");
    std::fs::write(dotori.work.join("notes.txt"), "in the work folder").expect("written");
    let (runner, model) = runner_with(
        dotori.basic(),
        vec![
            vec![
                use_tool("read", READ_FILE, json!({ "path": "notes.txt" })),
                where_am_i("shell"),
            ],
            vec![say(0, "done")],
        ],
        5,
    );

    let outcome = runner.run(about(&dotori.project)).await;

    assert!(
        matches!(&outcome, SubSessionOutcome::Finished(text) if text == "done"),
        "{outcome:?}"
    );
    let returned = model.returned();
    assert_eq!(returned[0], Ok(String::from("1\tin the project\n")));
    assert_eq!(
        printed_folder(&returned[1]),
        dotori.project.canonicalize().expect("exists")
    );
}

#[tokio::test]
async fn a_job_without_a_folder_works_in_the_work_folder() {
    let dotori = Dotori::new();
    std::fs::write(dotori.work.join("notes.txt"), "in the work folder").expect("written");
    let (runner, model) = runner_with(
        dotori.basic(),
        vec![
            vec![
                use_tool("read", READ_FILE, json!({ "path": "notes.txt" })),
                where_am_i("shell"),
            ],
            vec![say(0, "done")],
        ],
        5,
    );

    let outcome = runner.run(spec()).await;

    assert!(
        matches!(&outcome, SubSessionOutcome::Finished(text) if text == "done"),
        "{outcome:?}"
    );
    let returned = model.returned();
    assert_eq!(returned[0], Ok(String::from("1\tin the work folder\n")));
    assert_eq!(
        printed_folder(&returned[1]),
        dotori.work.canonicalize().expect("exists")
    );
}

#[tokio::test]
async fn a_job_about_a_folder_still_reaches_the_work_folder_but_not_the_rest_of_dotori() {
    let dotori = Dotori::new();
    let kept = dotori.work.join("kept.txt");
    let settings = dotori.root.join("dotori").join("settings.toml");
    let (runner, model) = runner_with(
        dotori.basic(),
        vec![
            vec![
                use_tool(
                    "write",
                    WRITE_FILE,
                    json!({ "path": kept.display().to_string(), "content": "kept" }),
                ),
                use_tool(
                    "read",
                    READ_FILE,
                    json!({ "path": settings.display().to_string() }),
                ),
            ],
            vec![say(0, "done")],
        ],
        5,
    );

    runner.run(about(&dotori.project)).await;

    let returned = model.returned();
    assert!(returned[0].is_ok(), "{:?}", returned[0]);
    assert_eq!(std::fs::read_to_string(&kept).ok().as_deref(), Some("kept"));
    assert!(
        returned[1]
            .as_ref()
            .is_err_and(|error| error.contains("is dotori's own")),
        "{:?}",
        returned[1]
    );
}

#[tokio::test]
async fn a_refused_folder_fails_the_sub_session_before_the_model_is_asked() {
    let dotori = Dotori::new();
    let home = std::env::var_os(if cfg!(windows) { "USERPROFILE" } else { "HOME" })
        .map(PathBuf::from)
        .expect("a home folder");

    for refused in [dotori.root.join("dotori"), home] {
        let (runner, model) = runner_with(dotori.basic(), vec![vec![say(0, "done")]], 5);

        let outcome = runner.run(about(&refused)).await;

        assert!(
            matches!(&outcome, SubSessionOutcome::Failed(text) if text.contains("not a folder a sub-session may work in")),
            "{refused:?} gave {outcome:?}"
        );
        assert!(model.seen.lock().expect("unpoisoned").is_empty());
    }
}
