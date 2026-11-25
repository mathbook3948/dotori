#![cfg(unix)]

use std::sync::Arc;
use std::time::Duration;

use dotori_core::BoxFuture;
use dotori_core::tool::ToolCall;
use dotori_core::tool::ToolCallId;
use dotori_core::tool::ToolError;
use dotori_core::tool::ToolSpec;
use dotori_core::tool::Toolbox;
use serde_json::json;

use super::*;
use crate::tool::basic::BasicTools;
use crate::tool::basic::RUN_SHELL;
use crate::tool::basic::WithBasic;

const SECRET: &str = "dotori-sandbox-probe-secret";

struct Nothing;

impl Toolbox for Nothing {
    fn specs(&self) -> Vec<ToolSpec> {
        Vec::new()
    }

    fn run(&self, _: ToolCall) -> BoxFuture<'_, Result<String, ToolError>> {
        Box::pin(async {
            Err(ToolError {
                detail: String::from("none"),
            })
        })
    }
}

struct Probe {
    base: PathBuf,
    dotori: PathBuf,
    work: PathBuf,
    elsewhere: PathBuf,
    tools: WithBasic,
}

impl Probe {
    fn new(name: &str) -> Self {
        let base =
            std::env::temp_dir().join(format!("dotori-sandbox-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        std::fs::create_dir_all(&base).expect("made");
        let base = base.canonicalize().expect("real");
        let dotori = base.join("dotori");
        let work = dotori.join("work");
        let elsewhere = base.join("elsewhere");
        for folder in [&work, &elsewhere, &dotori.join("browser")] {
            std::fs::create_dir_all(folder).expect("made");
        }
        std::fs::write(dotori.join("vault.sqlite3"), SECRET).expect("written");
        std::fs::write(dotori.join("browser").join("Cookies"), SECRET).expect("written");
        std::fs::write(elsewhere.join("notes.txt"), "elsewhere").expect("written");
        let private = Private {
            folder: dotori.clone(),
            open: work.clone(),
        };
        let tools = WithBasic {
            basic: BasicTools {
                folder: work.clone(),
                limit: Duration::from_secs(20),
                hidden_env: Vec::new(),
                sandbox: Some(Arc::new(Sandbox::new(&private))),
                private: Some(private),
            },
            rest: Arc::new(Nothing),
        };
        Self {
            base,
            dotori,
            work,
            elsewhere,
            tools,
        }
    }

    async fn ready(&self) -> bool {
        let sandbox = self.tools.basic.sandbox.as_ref().expect("a sandbox");
        if sandbox.ready().await.is_some() {
            return true;
        }
        if cfg!(target_os = "macos") {
            panic!("macOS always has its sandbox, but sandbox-exec did not run");
        }
        eprintln!(
            "skipped: bubblewrap does not run for this user here, so commands are not sandboxed \
             on this PC; install it or allow unprivileged user namespaces to run this test"
        );
        false
    }

    async fn shell(&self, line: &str) -> String {
        self.tools
            .run(ToolCall {
                id: ToolCallId::new("c"),
                name: String::from(RUN_SHELL),
                input: json!({ "command": line }).to_string(),
            })
            .await
            .unwrap_or_else(|error| error.detail)
    }
}

impl Drop for Probe {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.base);
    }
}

#[tokio::test]
async fn a_sandboxed_command_cannot_read_dotori_s_folder_but_works_everywhere_else() {
    let probe = Probe::new("escape");
    if !probe.ready().await {
        return;
    }
    let (dotori, work, elsewhere) = (
        probe.dotori.display(),
        probe.work.display(),
        probe.elsewhere.display(),
    );

    for line in [
        format!("cat {dotori}/vault.sqlite3"),
        format!("cat {dotori}/browser/Cookies"),
        format!("cat {work}/../vault.sqlite3"),
        format!("ln -s {dotori}/vault.sqlite3 {work}/link; cat {work}/link"),
        format!("grep -r {SECRET} {dotori}"),
    ] {
        let ran = probe.shell(&line).await;
        assert!(!ran.contains(SECRET), "{line} read a secret:\n{ran}");
    }
    let planted = probe
        .shell(&format!("echo planted > {dotori}/planted.txt"))
        .await;
    assert!(
        !probe.dotori.join("planted.txt").exists(),
        "a command wrote into dotori's folder:\n{planted}"
    );

    let wrote = probe
        .shell(&format!(
            "echo made > {work}/made.txt && cat {work}/made.txt && cat {elsewhere}/notes.txt && \
             echo also > {elsewhere}/also.txt && pwd"
        ))
        .await;
    assert!(wrote.starts_with("exit code: 0\n"), "{wrote}");
    assert!(
        wrote.contains("made\n") && wrote.contains("elsewhere"),
        "{wrote}"
    );
    assert!(wrote.contains(&format!("{work}\n")), "{wrote}");
    assert_eq!(
        std::fs::read_to_string(probe.work.join("made.txt")).expect("kept"),
        "made\n"
    );
    assert_eq!(
        std::fs::read_to_string(probe.elsewhere.join("also.txt")).expect("kept"),
        "also\n"
    );
    assert_eq!(
        std::fs::read_to_string(probe.dotori.join("vault.sqlite3")).expect("there"),
        SECRET
    );
}

#[cfg(target_os = "linux")]
#[tokio::test]
async fn a_sandboxed_command_sees_none_of_dotori_s_processes_and_ends_with_its_time() {
    let probe = Probe::new("processes");
    if !probe.ready().await {
        return;
    }
    let own = std::process::id();

    let seen = probe
        .shell(&format!(
            "cat /proc/{own}/environ /proc/{own}/cmdline; ls /proc | grep -c '^[0-9]'"
        ))
        .await;
    assert!(!seen.contains("dotori_connectors"), "{seen}");
    let count: usize = seen
        .lines()
        .rev()
        .find_map(|line| line.trim().parse().ok())
        .expect("a count");
    assert!(count < 10, "the sandbox sees {count} processes:\n{seen}");

    let slow = probe
        .tools
        .run(ToolCall {
            id: ToolCallId::new("c"),
            name: String::from(RUN_SHELL),
            input: json!({ "command": "sleep 3091 & sleep 3092; wait", "seconds": 1 }).to_string(),
        })
        .await;
    assert!(slow.is_err(), "{slow:?}");
    tokio::time::sleep(Duration::from_millis(500)).await;
    let left: Vec<String> = std::fs::read_dir("/proc")
        .expect("proc")
        .flatten()
        .filter_map(|entry| std::fs::read(entry.path().join("cmdline")).ok())
        .map(|line| String::from_utf8_lossy(&line).replace('\0', " "))
        .filter(|line| line.contains("sleep 309"))
        .collect();
    assert!(left.is_empty(), "left running: {left:?}");
}
