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
const OUTSIDE: &str = "dotori-sandbox-probe-outside";
const CHILD: &str = "dotori-sandbox-probe-child";

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
        match sandbox.ready().await {
            Ok(Some(_)) => return true,
            Err(blocked) => panic!(
                "the sandbox refused every command before any probe line ran: {blocked}. On macOS \
                 this means sandbox-exec rejected dotori's Seatbelt profile.\n{}",
                self.rejected().await
            ),
            Ok(None) => {}
        }
        if cfg!(target_os = "macos") {
            panic!("macOS always has sandbox-exec, but /usr/bin/sandbox-exec is not a file here");
        }
        if std::env::var_os("CI").is_some() {
            panic!(
                "CI is set, but no root-owned bubblewrap runs for this user here, so the escape \
                 probe cannot run; the CI job installs bubblewrap and lifts the AppArmor \
                 restriction on user namespaces before the tests"
            );
        }
        eprintln!(
            "skipped: bubblewrap does not run for this user here, so commands are not sandboxed \
             on this PC; install it or allow unprivileged user namespaces to run this test"
        );
        false
    }

    async fn rejected(&self) -> String {
        if !cfg!(target_os = "macos") {
            return String::new();
        }
        let sandbox = self.tools.basic.sandbox.as_ref().expect("a sandbox");
        let mut command = sandbox.command(&Confinement::Seatbelt, "exit 0", &self.work);
        let profile = command
            .as_std()
            .get_args()
            .skip_while(|arg| *arg != "-p")
            .nth(1)
            .map(|profile| profile.to_string_lossy().into_owned())
            .unwrap_or_default();
        let said = match command.output().await {
            Ok(output) => String::from_utf8_lossy(&output.stderr).into_owned(),
            Err(error) => error.to_string(),
        };
        let numbered: String = profile
            .lines()
            .enumerate()
            .map(|(n, rule)| format!("{:>3} {rule}\n", n + 1))
            .collect();
        format!("sandbox-exec said:\n{said}\nthe profile it was given:\n{numbered}")
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

    let base = probe.base.display();
    for (how, line) in [
        ("a plain read", format!("cat {dotori}/vault.sqlite3")),
        (
            "a read of the browser",
            format!("cat {dotori}/browser/Cookies"),
        ),
        (
            "`..` from the work folder",
            format!("cat {work}/../vault.sqlite3"),
        ),
        (
            "a symbolic link in the work folder",
            format!("ln -s {dotori}/vault.sqlite3 {work}/link; cat {work}/link"),
        ),
        (
            "a hard link in the work folder",
            format!("ln {dotori}/vault.sqlite3 {work}/hard; cat {work}/hard"),
        ),
        (
            "a clone into the work folder (cp -c)",
            format!("cp -c {dotori}/vault.sqlite3 {work}/clone; cat {work}/clone"),
        ),
        (
            "renaming a folder above dotori's",
            format!("mv {base} {base}-moved && cat {base}-moved/dotori/vault.sqlite3"),
        ),
        (
            "replacing the work folder with a link",
            format!("rm -rf {work} && ln -s {dotori} {work} && cat {work}/vault.sqlite3"),
        ),
        ("a search", format!("grep -r {SECRET} {dotori}")),
    ] {
        let ran = probe.shell(&line).await;
        let moved = PathBuf::from(format!("{base}-moved"));
        if moved.exists() {
            let _ = std::fs::rename(&moved, &probe.base);
        }
        assert!(
            !ran.contains(SECRET),
            "the sandbox let {how} read dotori's vault.\ncommand: {line}\n{ran}"
        );
    }
    assert!(
        std::fs::symlink_metadata(&probe.work).is_ok_and(|meta| meta.is_dir()),
        "a sandboxed command replaced the work folder"
    );
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

#[tokio::test]
async fn no_command_runs_once_the_work_folder_is_a_link() {
    let probe = Probe::new("swapped");
    if !probe.ready().await {
        return;
    }
    std::fs::remove_dir(&probe.work).expect("removed");
    std::os::unix::fs::symlink(&probe.dotori, &probe.work).expect("linked");

    let ran = probe
        .shell(&format!("cat {}/vault.sqlite3", probe.work.display()))
        .await;

    assert!(!ran.contains(SECRET), "{ran}");
    assert!(ran.contains("not a plain folder"), "{ran}");
}

#[tokio::test]
async fn a_sandboxed_command_reads_its_own_children_but_not_processes_outside_the_sandbox() {
    let probe = Probe::new("others");
    if !probe.ready().await {
        return;
    }
    let outside = Command::new("/bin/sh")
        .arg("-c")
        .arg(format!("read line; : {OUTSIDE}"))
        .env_clear()
        .env("DOTORI_PROBE_SECRET", OUTSIDE)
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .kill_on_drop(true)
        .spawn()
        .expect("started");
    let pid = outside.id().expect("running");
    let unsandboxed = std::process::Command::new("ps")
        .args(["-ww", "-o", "args=", "-p", &pid.to_string()])
        .output()
        .expect("ps ran");
    let unsandboxed = String::from_utf8_lossy(&unsandboxed.stdout);
    assert!(
        unsandboxed.contains(OUTSIDE),
        "ps outside the sandbox does not show the helper's arguments, so the probe below would \
         prove nothing:\n{unsandboxed}"
    );

    let mut line = format!("ps -ww -o args= -p {pid}; ps eww -p {pid}");
    if cfg!(target_os = "macos") {
        line.push_str(&format!("; ps -E -ww -p {pid}"));
    }
    let seen = probe.shell(&line).await;
    assert!(
        !seen.contains(OUTSIDE),
        "a sandboxed command read the arguments or environment of a process outside the sandbox. \
         On macOS the Seatbelt rules on process-info* and the kern.procargs sysctls in sandbox.rs \
         let it through.\ncommand: {line}\n{seen}"
    );

    let own = probe
        .shell(&format!(
            "/bin/sh -c 'sleep 5; : {CHILD}' >/dev/null 2>&1 & c=$!; sleep 1; \
             ps -ww -o args= -p $c; kill $c"
        ))
        .await;
    if cfg!(target_os = "macos") {
        assert!(
            own.contains("Operation not permitted") || own.contains(CHILD),
            "on macOS a sandboxed ps either is refused or reads its own child:\n{own}"
        );
        let plain = probe.shell("echo still-runs").await;
        assert!(plain.contains("still-runs"), "{plain}");
        return;
    }
    assert!(
        own.starts_with("exit code: 0\n") && own.contains(CHILD),
        "a sandboxed command cannot read its own child's arguments:\n{own}"
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
