use super::*;

fn address() -> McpAddress {
    McpAddress {
        url: String::from("http://127.0.0.1:4000/mcp"),
        token: String::from("secret"),
    }
}

#[test]
fn claude_runs_in_auto_mode_with_the_server_and_no_token_on_the_command_line() {
    let arguments = arguments(CodingAgent::ClaudeCode, Some(&address()), Path::new("last"));
    let at = |flag: &str| arguments.iter().position(|argument| argument == flag);
    let mode = at("--permission-mode").expect("a mode");
    assert_eq!(arguments[mode + 1], "auto");
    let config = at("--mcp-config").expect("the server");
    let config: Value = serde_json::from_str(&arguments[config + 1]).expect("json");
    assert_eq!(
        config["mcpServers"]["dotori"]["url"],
        "http://127.0.0.1:4000/mcp"
    );
    assert_eq!(
        config["mcpServers"]["dotori"]["headers"]["Authorization"],
        "Bearer ${DOTORI_MCP_TOKEN}"
    );
    assert!(!arguments.iter().any(|argument| argument.contains("secret")));
}

#[test]
fn codex_writes_its_last_message_and_reads_the_brief_from_input() {
    let arguments = arguments(CodingAgent::Codex, Some(&address()), Path::new("last"));
    assert_eq!(arguments[..2], ["exec", "--approve-for-me"]);
    assert!(arguments.contains(&String::from(
        "mcp_servers.dotori.url=\"http://127.0.0.1:4000/mcp\""
    )));
    assert!(arguments.contains(&String::from(
        "mcp_servers.dotori.bearer_token_env_var=\"DOTORI_MCP_TOKEN\""
    )));
    assert_eq!(arguments.last().map(String::as_str), Some("-"));
    assert!(!arguments.iter().any(|argument| argument.contains("secret")));
}

#[test]
fn no_server_means_no_server_arguments() {
    let claude = arguments(CodingAgent::ClaudeCode, None, Path::new("last"));
    assert!(!claude.contains(&String::from("--mcp-config")));
    let codex = arguments(CodingAgent::Codex, None, Path::new("last"));
    assert!(!codex.contains(&String::from("-c")));
}

#[test]
fn claude_output_becomes_an_outcome() {
    let finished = claude_result(r#"{"type":"result","is_error":false,"result":" done "}"#);
    assert!(matches!(finished, SubSessionOutcome::Finished(text) if text == "done"));
    let failed = claude_result(r#"{"type":"result","is_error":true,"result":"out of credit"}"#);
    assert!(matches!(failed, SubSessionOutcome::Failed(text) if text.contains("out of credit")));
    let garbled = claude_result("not json");
    assert!(matches!(garbled, SubSessionOutcome::Failed(text) if text.contains("not json")));
}

#[test]
fn a_long_error_keeps_its_end() {
    let long = format!("{}끝", "가".repeat(3000));
    let shown = tail(&long);
    assert!(shown.ends_with('끝') && shown.len() <= SHOWN_ERROR);
}

#[cfg(unix)]
#[tokio::test]
async fn a_run_returns_the_agent_answer_and_its_failures() {
    use std::os::unix::fs::PermissionsExt;
    let folder = std::env::temp_dir().join(format!("dotori-cli-test-{}", std::process::id()));
    std::fs::create_dir_all(&folder).expect("made");
    let script = folder.join("fake-claude");
    std::fs::write(
        &script,
        "#!/bin/sh\nread first\ncase \"$first\" in\n\
         '# fail') echo broken >&2; exit 3;;\n\
         *) printf '{\"is_error\":false,\"result\":\"%s from %s\"}' \"$first\" \"$DOTORI_MCP_TOKEN\";;\nesac\n",
    )
    .expect("written");
    std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).expect("runnable");

    struct Fixed;
    impl McpServer for Fixed {
        fn address(&self) -> BoxFuture<'_, Result<McpAddress, String>> {
            Box::pin(async { Ok(address()) })
        }
    }
    let runner = CliRunner {
        agent: CodingAgent::ClaudeCode,
        program: script,
        folder: folder.join("work"),
        mcp: Some(Arc::new(Fixed)),
        limit: Duration::from_secs(10),
        hidden_env: Vec::new(),
    };
    let spec = |title: &str| SubSessionSpec {
        title: String::from(title),
        brief: String::from("Do it."),
        folder: None,
    };
    let done = runner.run(spec("hi")).await;
    assert!(
        matches!(&done, SubSessionOutcome::Finished(text) if text == "# hi from secret"),
        "{done:?}"
    );
    let failed = runner.run(spec("fail")).await;
    assert!(
        matches!(&failed, SubSessionOutcome::Failed(text) if text.contains("broken")),
        "{failed:?}"
    );
    let _ = std::fs::remove_dir_all(&folder);
}

#[cfg(unix)]
#[tokio::test]
async fn a_job_about_a_folder_starts_the_agent_there_and_a_missing_one_is_refused() {
    use std::os::unix::fs::PermissionsExt;
    let folder = std::env::temp_dir().join(format!("dotori-cli-folder-{}", std::process::id()));
    let project = folder.join("project");
    std::fs::create_dir_all(&project).expect("made");
    let script = folder.join("fake-claude");
    std::fs::write(
        &script,
        "#!/bin/sh\nprintf '{\"is_error\":false,\"result\":\"%s\"}' \"$(pwd)\"\n",
    )
    .expect("written");
    std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).expect("runnable");
    let runner = CliRunner {
        agent: CodingAgent::ClaudeCode,
        program: script,
        folder: folder.join(".local").join("dotori").join("work"),
        mcp: None,
        limit: Duration::from_secs(10),
        hidden_env: Vec::new(),
    };
    let spec = |at: Option<&Path>| SubSessionSpec {
        title: String::from("t"),
        brief: String::from("b"),
        folder: at.map(|at| at.display().to_string()),
    };

    let there = runner.run(spec(Some(&project))).await;
    let here = runner.run(spec(None)).await;
    let missing = runner.run(spec(Some(&folder.join("nowhere")))).await;
    let relative = runner.run(spec(Some(Path::new("project")))).await;
    std::fs::create_dir_all(folder.join(".secret").join("keys")).expect("made");
    std::fs::create_dir_all(folder.join(".local").join("dotori").join("record")).expect("made");
    std::fs::create_dir_all(
        folder
            .join(".local")
            .join("dotori")
            .join("work")
            .join("scratch"),
    )
    .expect("made");
    let hidden = runner
        .run(spec(Some(&folder.join(".secret").join("keys"))))
        .await;
    let dotori = runner
        .run(spec(Some(
            &folder.join(".local").join("dotori").join("record"),
        )))
        .await;
    let scratch = runner
        .run(spec(Some(
            &folder
                .join(".local")
                .join("dotori")
                .join("work")
                .join("scratch"),
        )))
        .await;
    let root = runner.run(spec(Some(Path::new("/")))).await;

    let canonical = |path: &Path| path.canonicalize().expect("exists").display().to_string();
    assert!(
        matches!(&there, SubSessionOutcome::Finished(text) if *text == canonical(&project)),
        "{there:?}"
    );
    assert!(
        matches!(&here, SubSessionOutcome::Finished(text) if *text == canonical(&folder.join(".local").join("dotori").join("work"))),
        "{here:?}"
    );
    for refused in [missing, relative, hidden, dotori, root] {
        assert!(
            matches!(&refused, SubSessionOutcome::Failed(text) if text.contains("not a folder")),
            "{refused:?}"
        );
    }
    assert!(
        matches!(&scratch, SubSessionOutcome::Finished(text) if text.ends_with("scratch")),
        "{scratch:?}"
    );
    let _ = std::fs::remove_dir_all(&folder);
}

#[cfg(target_os = "linux")]
#[tokio::test]
async fn a_run_past_its_limit_ends_with_everything_it_started() {
    use std::os::unix::fs::PermissionsExt;
    let folder = std::env::temp_dir().join(format!("dotori-cli-limit-{}", std::process::id()));
    std::fs::create_dir_all(&folder).expect("made");
    let script = folder.join("slow");
    let marker = folder.join("child.pid");
    std::fs::write(
        &script,
        format!(
            "#!/bin/sh\nsleep 30 &\necho $! > {}\nwait\n",
            marker.display()
        ),
    )
    .expect("written");
    std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).expect("runnable");
    let runner = CliRunner {
        agent: CodingAgent::ClaudeCode,
        program: script,
        folder: folder.join("work"),
        mcp: None,
        limit: Duration::from_secs(1),
        hidden_env: Vec::new(),
    };
    let outcome = runner
        .run(SubSessionSpec {
            title: String::from("slow"),
            brief: String::from("Wait."),
            folder: None,
        })
        .await;
    assert!(
        matches!(&outcome, SubSessionOutcome::Failed(text) if text.contains("did not finish")),
        "{outcome:?}"
    );
    let child = std::fs::read_to_string(&marker).expect("the child started");
    std::thread::sleep(Duration::from_millis(200));
    let state = std::fs::read_to_string(format!("/proc/{}/stat", child.trim())).unwrap_or_default();
    let running = state
        .rsplit_once(')')
        .is_some_and(|(_, rest)| !rest.trim_start().starts_with(['Z', 'X']));
    assert!(!running, "the child of the agent was left running: {state}");
    let _ = std::fs::remove_dir_all(&folder);
}

#[tokio::test]
async fn a_server_that_is_down_fails_the_run_before_it_starts() {
    struct Down;
    impl McpServer for Down {
        fn address(&self) -> BoxFuture<'_, Result<McpAddress, String>> {
            Box::pin(async { Err(String::from("it crashed")) })
        }
    }
    let runner = CliRunner {
        agent: CodingAgent::Codex,
        program: PathBuf::from("no-such-program"),
        folder: std::env::temp_dir().join("dotori-cli-down"),
        mcp: Some(Arc::new(Down)),
        limit: Duration::from_secs(1),
        hidden_env: Vec::new(),
    };
    let outcome = runner
        .run(SubSessionSpec {
            title: String::from("t"),
            brief: String::from("b"),
            folder: None,
        })
        .await;
    assert!(
        matches!(&outcome, SubSessionOutcome::Failed(text) if text.contains("it crashed")),
        "{outcome:?}"
    );
}

#[test]
fn a_verbatim_windows_path_starts_the_agent_as_a_plain_one() {
    assert_eq!(
        plain(PathBuf::from(r"\\?\C:\Users\me\project")),
        PathBuf::from(r"C:\Users\me\project")
    );
    assert_eq!(
        plain(PathBuf::from(r"\\?\UNC\server\share")),
        PathBuf::from(r"\\?\UNC\server\share")
    );
    assert_eq!(
        plain(PathBuf::from("/home/me/project")),
        PathBuf::from("/home/me/project")
    );
}
