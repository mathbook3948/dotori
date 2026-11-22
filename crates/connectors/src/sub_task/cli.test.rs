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
    assert!(matches!(finished, SubTaskOutcome::Finished(text) if text == "done"));
    let failed = claude_result(r#"{"type":"result","is_error":true,"result":"out of credit"}"#);
    assert!(matches!(failed, SubTaskOutcome::Failed(text) if text.contains("out of credit")));
    let garbled = claude_result("not json");
    assert!(matches!(garbled, SubTaskOutcome::Failed(text) if text.contains("not json")));
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
        fn address(&self) -> BoxFuture<'_, Option<McpAddress>> {
            Box::pin(async { Some(address()) })
        }
    }
    let runner = CliRunner {
        agent: CodingAgent::ClaudeCode,
        program: script,
        folder: folder.join("work"),
        mcp: Some(Arc::new(Fixed)),
        limit: Duration::from_secs(10),
    };
    let spec = |title: &str| SubTaskSpec {
        title: String::from(title),
        brief: String::from("Do it."),
    };
    let done = runner.run(spec("hi")).await;
    assert!(
        matches!(&done, SubTaskOutcome::Finished(text) if text == "# hi from secret"),
        "{done:?}"
    );
    let failed = runner.run(spec("fail")).await;
    assert!(
        matches!(&failed, SubTaskOutcome::Failed(text) if text.contains("broken")),
        "{failed:?}"
    );
    let _ = std::fs::remove_dir_all(&folder);
}
