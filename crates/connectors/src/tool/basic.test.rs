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
            private: Some(folder.clone()),
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

    std::fs::write(folder.join("work").join("binary"), [0u8, 1, 2]).expect("written");
    let binary = run(
        &tools,
        READ_FILE,
        json!({ "path": folder.join("work").join("binary").display().to_string() }),
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

#[tokio::test]
async fn dotori_s_own_files_stay_out_of_reach_but_its_work_folder_does_not() {
    let (tools, folder) = tools();
    std::fs::create_dir_all(&folder).expect("made");
    std::fs::write(folder.join("door.json"), "{\"token\":\"secret\"}").expect("written");

    let door = run(
        &tools,
        READ_FILE,
        json!({ "path": folder.join("door.json").display().to_string() }),
    )
    .await;
    assert!(door.expect_err("refused").contains("dotori's own"));
    let sneaky = run(&tools, READ_FILE, json!({ "path": "../door.json" })).await;
    let sneaky = sneaky.expect_err("refused");
    assert!(sneaky.contains("dotori's own"), "{sneaky}");
    let overwrite = run(
        &tools,
        WRITE_FILE,
        json!({ "path": "../door.json", "content": "x" }),
    )
    .await;
    assert!(overwrite.is_err());
    run(
        &tools,
        WRITE_FILE,
        json!({ "path": "draft.md", "content": "ok" }),
    )
    .await
    .expect("work folder");
    let _ = std::fs::remove_dir_all(folder);
}

#[tokio::test]
async fn windows_line_ends_are_kept_when_an_edit_spans_lines() {
    let (tools, folder) = tools();
    run(
        &tools,
        WRITE_FILE,
        json!({ "path": "crlf.txt", "content": "one\r\ntwo\r\nthree\r\n" }),
    )
    .await
    .expect("wrote");

    run(
        &tools,
        EDIT_FILE,
        json!({ "path": "crlf.txt", "old": "one\ntwo", "new": "1\n2" }),
    )
    .await
    .expect("edited");

    let text = std::fs::read_to_string(folder.join("work").join("crlf.txt")).expect("read");
    assert_eq!(text, "1\r\n2\r\nthree\r\n");
    let _ = std::fs::remove_dir_all(folder);
}

#[tokio::test]
async fn a_long_read_says_where_it_really_stopped() {
    let (tools, folder) = tools();
    let line = "x".repeat(200);
    let content: String = (0..2000).map(|_| format!("{line}\n")).collect();
    run(
        &tools,
        WRITE_FILE,
        json!({ "path": "big.txt", "content": content }),
    )
    .await
    .expect("wrote");

    let read = run(&tools, READ_FILE, json!({ "path": "big.txt" }))
        .await
        .expect("read");

    let shown = read.lines().filter(|row| row.contains('\t')).count();
    assert!(shown < 2000);
    assert!(
        read.ends_with(&format!(
            "(2000 lines in all; read on from line {})\n",
            shown + 1
        )),
        "{}",
        &read[read.len() - 80..]
    );
    let _ = std::fs::remove_dir_all(folder);
}

#[cfg(target_os = "linux")]
#[tokio::test]
async fn a_device_is_not_read_and_a_command_left_running_does_not_hold_the_answer() {
    let (tools, folder) = tools();

    let device = run(&tools, READ_FILE, json!({ "path": "/dev/zero" })).await;
    assert!(device.expect_err("refused").contains("not a file"));

    let started = std::time::Instant::now();
    let left = run(
        &tools,
        RUN_SHELL,
        json!({ "command": "sleep 20 & echo started" }),
    )
    .await
    .expect("ran");
    assert!(
        started.elapsed() < Duration::from_secs(5),
        "{:?}",
        started.elapsed()
    );
    assert!(left.starts_with("exit code: 0"), "{left}");

    let loud = run(
        &tools,
        RUN_SHELL,
        json!({ "command": "yes | head -c 5000000" }),
    )
    .await
    .expect("ran");
    assert!(loud.len() < OUTPUT * 2 + 200, "{}", loud.len());
    assert!(loud.contains("(only the last part)"));
    let _ = std::fs::remove_dir_all(folder);
}
