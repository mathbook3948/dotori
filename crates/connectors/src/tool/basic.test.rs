use std::sync::atomic::AtomicUsize;
use std::sync::atomic::Ordering;

use dotori_core::tool::ToolCallId;

use super::*;

static NEXT: AtomicUsize = AtomicUsize::new(0);

struct Nothing;

impl Toolbox for Nothing {
    fn specs(&self) -> Vec<ToolSpec> {
        vec![ToolSpec {
            name: String::from("webfetch"),
            description: String::new(),
            input_schema: String::from("{}"),
        }]
    }

    fn run(&self, call: ToolCall) -> BoxFuture<'_, Result<String, ToolError>> {
        Box::pin(async move { Ok(format!("rest ran {}", call.name)) })
    }
}

fn tools() -> (WithBasic, PathBuf) {
    let n = NEXT.fetch_add(1, Ordering::SeqCst);
    let folder = std::env::temp_dir().join(format!("dotori-basic-{}-{n}", std::process::id()));
    let _ = std::fs::remove_dir_all(&folder);
    let tools = WithBasic {
        basic: BasicTools {
            folder: folder.join("work"),
            limit: Duration::from_secs(5),
            hidden_env: vec![String::from("DOTORI_TEST_SECRET")],
        },
        rest: Arc::new(Nothing),
    };
    (tools, folder)
}

async fn run(tools: &WithBasic, name: &str, input: Value) -> Result<String, String> {
    tools
        .run(ToolCall {
            id: ToolCallId::new("c"),
            name: name.to_owned(),
            input: input.to_string(),
        })
        .await
        .map_err(|error| error.detail)
}

#[tokio::test]
async fn a_file_is_written_read_with_numbers_and_edited_in_place() {
    let (tools, folder) = tools();

    let wrote = run(
        &tools,
        WRITE_FILE,
        json!({ "path": "notes/a.txt", "content": "하나\n둘\n셋\n둘\n" }),
    )
    .await;
    assert!(wrote.expect("wrote").contains("notes"));
    let read = run(&tools, READ_FILE, json!({ "path": "notes/a.txt" }))
        .await
        .expect("read");
    assert_eq!(read, "1\t하나\n2\t둘\n3\t셋\n4\t둘\n");
    let part = run(
        &tools,
        READ_FILE,
        json!({ "path": "notes/a.txt", "offset": 2, "limit": 1 }),
    )
    .await
    .expect("read");
    assert_eq!(part, "2\t둘\n(4 lines in all; read on from line 3)\n");

    let twice = run(
        &tools,
        EDIT_FILE,
        json!({ "path": "notes/a.txt", "old": "둘", "new": "2" }),
    )
    .await;
    assert!(twice.expect_err("ambiguous").contains("2 times"));
    let missing = run(
        &tools,
        EDIT_FILE,
        json!({ "path": "notes/a.txt", "old": "넷", "new": "4" }),
    )
    .await;
    assert!(missing.expect_err("missing").contains("not in"));
    run(
        &tools,
        EDIT_FILE,
        json!({ "path": "notes/a.txt", "old": "셋", "new": "3" }),
    )
    .await
    .expect("edited");
    run(
        &tools,
        EDIT_FILE,
        json!({ "path": "notes/a.txt", "old": "둘", "new": "2", "all": true }),
    )
    .await
    .expect("edited all");
    let absolute = folder.join("work").join("notes").join("a.txt");
    assert_eq!(
        std::fs::read_to_string(&absolute).expect("read"),
        "하나\n2\n3\n2\n"
    );
    let by_whole_path = run(
        &tools,
        READ_FILE,
        json!({ "path": absolute.display().to_string(), "limit": 1 }),
    )
    .await
    .expect("read");
    assert!(by_whole_path.starts_with("1\t하나\n"));

    std::fs::write(folder.join("binary"), [0u8, 1, 2]).expect("written");
    let binary = run(
        &tools,
        READ_FILE,
        json!({ "path": folder.join("binary").display().to_string() }),
    )
    .await;
    assert!(binary.expect_err("binary").contains("not a text file"));
    let _ = std::fs::remove_dir_all(folder);
}

#[cfg(target_os = "linux")]
#[tokio::test]
async fn a_command_gives_its_code_and_output_and_a_slow_one_is_stopped_with_its_children() {
    let (tools, folder) = tools();

    let ran = run(
        &tools,
        RUN_SHELL,
        json!({ "command": "echo out; echo err >&2; pwd; exit 3" }),
    )
    .await
    .expect("ran");
    assert!(ran.starts_with("exit code: 3\n"), "{ran}");
    assert!(ran.contains("out\n") && ran.contains("err\n"), "{ran}");
    assert!(ran.contains("/work"), "{ran}");

    let marker = folder.join("child.pid");
    let slow = run(
        &tools,
        RUN_SHELL,
        json!({ "command": format!("sleep 30 & echo $! > {}; wait", marker.display()), "seconds": 1 }),
    )
    .await;
    assert!(slow.expect_err("stopped").contains("did not finish"));
    let child = std::fs::read_to_string(&marker)
        .expect("pid")
        .trim()
        .to_owned();
    tokio::time::sleep(Duration::from_millis(300)).await;
    let state = std::fs::read_to_string(format!("/proc/{child}/stat")).unwrap_or_default();
    let running = state
        .rsplit_once(')')
        .is_some_and(|(_, rest)| !rest.trim_start().starts_with(['Z', 'X']));
    assert!(!running, "the command's child was left running: {state}");
    let _ = std::fs::remove_dir_all(folder);
}

#[tokio::test]
async fn other_tools_pass_through_and_the_basic_ones_come_first() {
    let (tools, folder) = tools();

    let names: Vec<String> = tools.specs().into_iter().map(|spec| spec.name).collect();
    assert_eq!(
        names,
        [READ_FILE, WRITE_FILE, EDIT_FILE, RUN_SHELL, "webfetch"]
    );
    assert_eq!(
        run(&tools, "webfetch", json!({})).await.expect("ran"),
        "rest ran webfetch"
    );
    assert!(run(&tools, READ_FILE, json!({ "path": "" })).await.is_err());
    assert!(
        run(&tools, RUN_SHELL, json!({ "command": "  " }))
            .await
            .is_err()
    );
    let _ = std::fs::remove_dir_all(folder);
}

#[test]
fn long_output_keeps_its_end_on_a_character_boundary() {
    let long = format!("{}끝", "가".repeat(OUTPUT));
    let shown = tail(&long);
    assert!(shown.ends_with('끝'));
    assert!(shown.len() <= OUTPUT + 40);
}
