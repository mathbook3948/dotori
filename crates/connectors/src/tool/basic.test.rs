use std::sync::atomic::AtomicUsize;
use std::sync::atomic::Ordering;

use dotori_core::tool::ToolCallId;

use super::*;

static NEXT: AtomicUsize = AtomicUsize::new(0);

struct Nothing;

impl Toolbox for Nothing {
    fn specs(&self) -> Vec<ToolSpec> {
        vec![ToolSpec {
            name: String::from("fetch_url"),
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
            private: Some(Private {
                folder: folder.clone(),
                open: folder.join("work"),
            }),
            sandbox: None,
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

#[cfg(windows)]
#[tokio::test]
async fn what_cmd_writes_in_the_code_page_of_windows_reads_as_text() {
    let (tools, folder) = tools();

    let ran = run(
        &tools,
        RUN_SHELL,
        json!({ "command": "dir C:\no-such-dotori-folder" }),
    )
    .await
    .expect("ran");

    assert!(
        ran.contains("no-such-dotori-folder") || ran.contains("C:\\"),
        "{ran}"
    );
    assert!(!ran.contains('\u{FFFD}'), "nothing is left unread: {ran}");
    let _ = std::fs::remove_dir_all(folder);
}

#[tokio::test]
async fn a_command_does_not_inherit_the_hidden_variables() {
    let (mut tools, folder) = tools();
    assert!(
        std::env::var_os("CARGO_PKG_NAME").is_some(),
        "cargo sets it"
    );
    tools.basic.hidden_env = vec![String::from("CARGO_PKG_NAME")];
    let line = if cfg!(windows) {
        "if defined CARGO_PKG_NAME (echo seen) else (echo hidden)"
    } else {
        "echo ${CARGO_PKG_NAME:-hidden}"
    };

    let ran = run(&tools, RUN_SHELL, json!({ "command": line }))
        .await
        .expect("ran");

    assert!(ran.contains("hidden"), "{ran}");
    let _ = std::fs::remove_dir_all(folder);
}

#[tokio::test]
async fn other_tools_pass_through_and_the_basic_ones_come_first() {
    let (tools, folder) = tools();

    let names: Vec<String> = tools.specs().into_iter().map(|spec| spec.name).collect();
    assert_eq!(
        names,
        [READ_FILE, WRITE_FILE, EDIT_FILE, RUN_SHELL, "fetch_url"]
    );
    assert_eq!(
        run(&tools, "fetch_url", json!({})).await.expect("ran"),
        "rest ran fetch_url"
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

#[cfg(unix)]
#[tokio::test]
async fn an_edit_through_a_link_changes_the_file_it_points_to() {
    let (tools, folder) = tools();
    std::fs::create_dir_all(folder.join("work")).expect("made");
    std::fs::write(folder.join("work").join("real.txt"), "a\n").expect("written");
    std::os::unix::fs::symlink(
        folder.join("work").join("real.txt"),
        folder.join("work").join("link.txt"),
    )
    .expect("linked");

    run(
        &tools,
        EDIT_FILE,
        json!({ "path": "link.txt", "old": "a", "new": "b" }),
    )
    .await
    .expect("edited");

    assert!(
        std::fs::symlink_metadata(folder.join("work").join("link.txt"))
            .expect("there")
            .file_type()
            .is_symlink()
    );
    assert_eq!(
        std::fs::read_to_string(folder.join("work").join("real.txt")).expect("read"),
        "b\n"
    );
    let _ = std::fs::remove_dir_all(folder);
}

#[cfg(target_os = "linux")]
#[tokio::test]
async fn output_already_written_is_kept_when_something_left_running_holds_the_pipe() {
    let (tools, folder) = tools();

    let ran = run(
        &tools,
        RUN_SHELL,
        json!({ "command": "echo built; sleep 20 &" }),
    )
    .await
    .expect("ran");

    assert!(ran.contains("built"), "{ran}");
    assert!(ran.contains("left running"), "{ran}");
    let _ = std::fs::remove_dir_all(folder);
}

#[cfg(unix)]
#[tokio::test]
async fn a_private_file_stays_private_while_it_is_replaced() {
    use std::os::unix::fs::PermissionsExt;
    let (tools, folder) = tools();
    std::fs::create_dir_all(folder.join("work")).expect("made");
    let secret = folder.join("work").join(".env");
    std::fs::write(&secret, "KEY=1\n").expect("written");
    std::fs::set_permissions(&secret, std::fs::Permissions::from_mode(0o600)).expect("private");

    run(
        &tools,
        EDIT_FILE,
        json!({ "path": ".env", "old": "1", "new": "2" }),
    )
    .await
    .expect("edited");

    let mode = std::fs::metadata(&secret)
        .expect("there")
        .permissions()
        .mode()
        & 0o777;
    assert_eq!(mode, 0o600);
    assert_eq!(std::fs::read_to_string(&secret).expect("read"), "KEY=2\n");
    let _ = std::fs::remove_dir_all(folder);
}

#[cfg(unix)]
#[tokio::test]
async fn a_link_in_the_work_folder_does_not_lead_into_the_rest_of_dotori_s_folder() {
    let (tools, folder) = tools();
    let work = folder.join("work");
    std::fs::create_dir_all(work.join("notes")).expect("made");
    std::fs::write(folder.join("vault.sqlite3"), "sealed").expect("written");
    std::os::unix::fs::symlink(&folder, work.join("up")).expect("linked");
    std::os::unix::fs::symlink(folder.join("later.txt"), work.join("dangling")).expect("linked");
    std::os::unix::fs::symlink(work.join("notes"), work.join("side")).expect("linked");

    for (name, input) in [
        (READ_FILE, json!({ "path": "up/vault.sqlite3" })),
        (READ_FILE, json!({ "path": "missing/../up/vault.sqlite3" })),
        (
            WRITE_FILE,
            json!({ "path": "missing/../up/new.txt", "content": "x" }),
        ),
        (WRITE_FILE, json!({ "path": "dangling", "content": "x" })),
        (
            EDIT_FILE,
            json!({ "path": "up/vault.sqlite3", "old": "sealed", "new": "open" }),
        ),
    ] {
        let refused = run(&tools, name, input.clone()).await;
        let refused = refused.expect_err(&input.to_string());
        assert!(refused.contains("dotori's own"), "{input}: {refused}");
    }
    assert!(!folder.join("new.txt").exists());
    assert!(!folder.join("later.txt").exists());
    assert_eq!(
        std::fs::read_to_string(folder.join("vault.sqlite3")).expect("read"),
        "sealed"
    );
    run(
        &tools,
        WRITE_FILE,
        json!({ "path": "side/kept.md", "content": "ok" }),
    )
    .await
    .expect("a link that stays in the work folder");
    assert_eq!(
        std::fs::read_to_string(work.join("notes").join("kept.md")).expect("read"),
        "ok"
    );
    let _ = std::fs::remove_dir_all(folder);
}

#[cfg(unix)]
#[tokio::test]
async fn no_file_is_touched_once_the_work_folder_is_a_link() {
    let (tools, folder) = tools();
    std::fs::create_dir_all(&folder).expect("made");
    std::fs::write(folder.join("vault.sqlite3"), "sealed").expect("written");
    std::os::unix::fs::symlink(&folder, folder.join("work")).expect("linked");

    for (name, input) in [
        (READ_FILE, json!({ "path": "vault.sqlite3" })),
        (WRITE_FILE, json!({ "path": "planted.txt", "content": "x" })),
    ] {
        let refused = run(&tools, name, input).await.expect_err("refused");
        assert!(refused.contains("not a plain folder"), "{refused}");
    }
    assert!(!folder.join("planted.txt").exists());
    let _ = std::fs::remove_dir_all(folder);
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
#[test]
fn a_file_is_checked_where_its_handle_really_is() {
    let (tools, folder) = tools();
    std::fs::create_dir_all(folder.join("work")).expect("made");
    std::fs::write(folder.join("vault.sqlite3"), "sealed").expect("written");
    std::fs::write(folder.join("work").join("note.md"), "ok").expect("written");
    let claimed = folder.join("work").join("note.md");

    let swapped = File::open(folder.join("vault.sqlite3")).expect("opened");
    let refused = tools
        .basic
        .held(&swapped, &claimed, &claimed)
        .expect_err("the handle is dotori's own");
    let kept = File::open(&claimed).expect("opened");

    assert!(refused.contains("dotori's own"), "{refused}");
    tools
        .basic
        .held(&kept, &claimed, &claimed)
        .expect("the work folder");
    let _ = std::fs::remove_dir_all(folder);
}
