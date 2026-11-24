//! The runners that hand a sub-session to a coding agent's command line, connected to dotori's
//! MCP server.

use std::path::Path;
use std::path::PathBuf;
use std::process::Stdio;
use std::sync::Arc;
use std::sync::atomic::AtomicU64;
use std::sync::atomic::Ordering;
use std::time::Duration;

use dotori_core::BoxFuture;
use dotori_core::sub_session::SubSessionOutcome;
use dotori_core::sub_session::SubSessionRunner;
use dotori_core::sub_session::SubSessionSpec;
use serde_json::Value;
use serde_json::json;
use tokio::io::AsyncWriteExt;
use tokio::process::Command;
use tokio::time;

use super::SUB_SESSION_SYSTEM;

const TOKEN_VARIABLE: &str = "DOTORI_MCP_TOKEN";
const SHOWN_ERROR: usize = 2000;
const TEXT_FILE_BUSY: i32 = if cfg!(unix) { 26 } else { -1 };
const PRIVATE: [&str; 2] = ["AppData", "Library"];
const BUSY_TRIES: u32 = 20;
const BUSY_WAIT: std::time::Duration = std::time::Duration::from_millis(50);

static NEXT: AtomicU64 = AtomicU64::new(0);

/// Where dotori's MCP server listens now.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct McpAddress {
    pub url: String,
    pub token: String,
}

/// The MCP server a coding agent connects to. Its address can change when it restarts.
pub trait McpServer: Send + Sync + 'static {
    /// Fails when the server is down and could not start again.
    fn address(&self) -> BoxFuture<'_, Result<McpAddress, String>>;
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CodingAgent {
    ClaudeCode,
    Codex,
}

impl CodingAgent {
    /// The command each agent installs.
    pub fn program(self) -> &'static str {
        match self {
            Self::ClaudeCode => "claude",
            Self::Codex => "codex",
        }
    }

    /// The agent's command found through `PATH`, including the `.cmd` shims npm installs on
    /// Windows.
    pub fn find(self) -> Option<PathBuf> {
        let suffixes: &[&str] = if cfg!(windows) {
            &[".exe", ".cmd", ""]
        } else {
            &[""]
        };
        let paths = std::env::var_os("PATH")?;
        std::env::split_paths(&paths).find_map(|folder| {
            suffixes.iter().find_map(|suffix| {
                let candidate = folder.join(format!("{}{suffix}", self.program()));
                candidate.is_file().then_some(candidate)
            })
        })
    }
}

struct Tree(Option<u32>);

impl Drop for Tree {
    fn drop(&mut self) {
        let Some(id) = self.0 else {
            return;
        };
        let mut kill = if cfg!(windows) {
            let mut kill = std::process::Command::new("taskkill");
            kill.args(["/T", "/F", "/PID", &id.to_string()]);
            kill
        } else {
            let mut kill = std::process::Command::new("kill");
            kill.args(["-KILL", "--", &format!("-{id}")]);
            kill
        };
        let _ = kill
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
    }
}

struct Removed(PathBuf);

impl Drop for Removed {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

/// Runs each sub-session as one non-interactive run of a coding agent in `folder`.
pub struct CliRunner {
    pub agent: CodingAgent,
    pub program: PathBuf,
    pub folder: PathBuf,
    pub mcp: Option<Arc<dyn McpServer>>,
    pub limit: Duration,
    /// Environment variables the coding agent must not see, such as tokens of servers only the
    /// main session may reach.
    pub hidden_env: Vec<String>,
}

impl std::fmt::Debug for CliRunner {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CliRunner")
            .field("agent", &self.agent)
            .field("program", &self.program)
            .field("folder", &self.folder)
            .finish_non_exhaustive()
    }
}

impl CliRunner {
    async fn work(&self, spec: SubSessionSpec) -> SubSessionOutcome {
        match self.attempt(spec).await {
            Ok(outcome) => outcome,
            Err(error) => SubSessionOutcome::Failed(error),
        }
    }

    async fn attempt(&self, spec: SubSessionSpec) -> Result<SubSessionOutcome, String> {
        let folder = match spec.folder.as_deref().map(PathBuf::from) {
            Some(asked) => match self.allowed(&asked) {
                Ok(folder) => folder,
                Err(refused) => return Ok(SubSessionOutcome::Failed(refused)),
            },
            None => {
                std::fs::create_dir_all(&self.folder)
                    .map_err(|error| format!("cannot make {}: {error}", self.folder.display()))?;
                self.folder.clone()
            }
        };
        let address = match &self.mcp {
            Some(mcp) => Some(
                mcp.address()
                    .await
                    .map_err(|error| format!("dotori's tools are down: {error}"))?,
            ),
            None => None,
        };
        let last = std::env::temp_dir().join(format!(
            "dotori-sub-session-{}-{}.txt",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&last)
            .map_err(|error| format!("cannot make {}: {error}", last.display()))?;
        let last = Removed(last);
        let mut command = Command::new(&self.program);
        command
            .args(arguments(self.agent, address.as_ref(), &last.0))
            .current_dir(&folder)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);
        #[cfg(unix)]
        command.process_group(0);
        for hidden in &self.hidden_env {
            command.env_remove(hidden);
        }
        if let Some(address) = &address {
            command.env(TOKEN_VARIABLE, &address.token);
        }
        let mut tries = 0;
        let mut child = loop {
            match command.spawn() {
                Ok(child) => break child,
                Err(error)
                    if error.raw_os_error() == Some(TEXT_FILE_BUSY) && tries < BUSY_TRIES =>
                {
                    tries += 1;
                    time::sleep(BUSY_WAIT).await;
                }
                Err(error) => {
                    return Err(format!("cannot start {}: {error}", self.program.display()));
                }
            }
        };
        let mut tree = Tree(child.id());
        let mut stdin = child.stdin.take().ok_or("the agent has no input")?;
        let prompt = prompt(self.agent, &spec);
        tokio::spawn(async move {
            let _ = stdin.write_all(prompt.as_bytes()).await;
        });
        let output = time::timeout(self.limit, child.wait_with_output())
            .await
            .map_err(|_| format!("the sub-session did not finish in {:?}", self.limit))?
            .map_err(|error| error.to_string())?;
        tree.0 = None;
        let written = std::fs::read_to_string(&last.0).ok();
        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            let stdout = String::from_utf8_lossy(&output.stdout);
            let shown = if stderr.trim().is_empty() {
                stdout
            } else {
                stderr
            };
            return Ok(SubSessionOutcome::Failed(format!(
                "{} ended with {}: {}",
                self.agent.program(),
                output.status,
                tail(shown.trim())
            )));
        }
        Ok(match self.agent {
            CodingAgent::ClaudeCode => claude_result(&String::from_utf8_lossy(&output.stdout)),
            CodingAgent::Codex => match written.filter(|text| !text.trim().is_empty()) {
                Some(text) => SubSessionOutcome::Finished(text.trim().to_owned()),
                None => SubSessionOutcome::Failed(String::from("codex gave no final answer")),
            },
        })
    }
}

impl CliRunner {
    fn allowed(&self, asked: &Path) -> Result<PathBuf, String> {
        let shown = asked.display();
        if !asked.is_absolute() {
            return Err(format!(
                "{shown} is not a folder on this PC: give the whole path"
            ));
        }
        let folder = asked
            .canonicalize()
            .ok()
            .filter(|folder| folder.is_dir())
            .ok_or_else(|| format!("{shown} is not a folder on this PC"))?;
        let own = self.folder.canonicalize().ok();
        if own.as_ref().is_some_and(|own| folder.starts_with(own)) {
            return Ok(plain(folder));
        }
        let hidden = folder.components().any(|part| match part {
            std::path::Component::Normal(name) => name.to_string_lossy().starts_with('.'),
            _ => false,
        });
        let private = ["HOME", "USERPROFILE"]
            .into_iter()
            .filter_map(std::env::var_os)
            .filter_map(|home| PathBuf::from(home).canonicalize().ok())
            .any(|home| {
                home.starts_with(&folder)
                    || PRIVATE
                        .iter()
                        .any(|name| folder.starts_with(home.join(name)))
            });
        let dotori = self
            .folder
            .parent()
            .and_then(|root| root.canonicalize().ok())
            .is_some_and(|root| folder.starts_with(&root));
        if hidden || private || dotori || folder.parent().is_none() {
            return Err(format!(
                "{shown} is not a folder a coding agent may work in: pick the project folder itself"
            ));
        }
        Ok(plain(folder))
    }
}

fn plain(folder: PathBuf) -> PathBuf {
    let stripped = folder
        .to_string_lossy()
        .strip_prefix(r"\\?\")
        .filter(|rest| !rest.starts_with("UNC\\"))
        .map(PathBuf::from);
    stripped.unwrap_or(folder)
}

impl SubSessionRunner for CliRunner {
    fn run(&self, spec: SubSessionSpec) -> BoxFuture<'_, SubSessionOutcome> {
        Box::pin(self.work(spec))
    }
}

fn arguments(agent: CodingAgent, address: Option<&McpAddress>, last: &Path) -> Vec<String> {
    let mut arguments: Vec<String> = Vec::new();
    match agent {
        CodingAgent::ClaudeCode => {
            arguments.extend(
                [
                    "--print",
                    "--output-format",
                    "json",
                    "--permission-mode",
                    "auto",
                    "--disallowedTools",
                    "WebFetch",
                    "--append-system-prompt",
                    SUB_SESSION_SYSTEM,
                ]
                .map(String::from),
            );
            if let Some(address) = address {
                let config = json!({
                    "mcpServers": {
                        "dotori": {
                            "type": "http",
                            "url": address.url,
                            "headers": { "Authorization": format!("Bearer ${{{TOKEN_VARIABLE}}}") }
                        }
                    }
                });
                arguments.push(String::from("--mcp-config"));
                arguments.push(config.to_string());
            }
        }
        CodingAgent::Codex => {
            arguments.extend(
                [
                    "exec",
                    "--approve-for-me",
                    "--skip-git-repo-check",
                    "--output-last-message",
                ]
                .map(String::from),
            );
            arguments.push(last.display().to_string());
            if let Some(address) = address {
                arguments.push(String::from("-c"));
                arguments.push(format!("mcp_servers.dotori.url={}", json!(address.url)));
                arguments.push(String::from("-c"));
                arguments.push(format!(
                    "mcp_servers.dotori.bearer_token_env_var={}",
                    json!(TOKEN_VARIABLE)
                ));
            }
            arguments.push(String::from("-"));
        }
    }
    arguments
}

fn prompt(agent: CodingAgent, spec: &SubSessionSpec) -> String {
    match agent {
        CodingAgent::ClaudeCode => format!("# {}\n\n{}", spec.title, spec.brief),
        CodingAgent::Codex => format!("{SUB_SESSION_SYSTEM}\n\n# {}\n\n{}", spec.title, spec.brief),
    }
}

fn claude_result(stdout: &str) -> SubSessionOutcome {
    let Ok(answer) = serde_json::from_str::<Value>(stdout.trim()) else {
        return SubSessionOutcome::Failed(format!("claude said: {}", tail(stdout.trim())));
    };
    let text = answer
        .get("result")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .trim()
        .to_owned();
    if answer.get("is_error").and_then(Value::as_bool) == Some(true) {
        let detail = if text.is_empty() {
            "no detail"
        } else {
            tail(&text)
        };
        return SubSessionOutcome::Failed(format!("claude failed: {detail}"));
    }
    if text.is_empty() {
        return SubSessionOutcome::Failed(String::from("claude gave no final answer"));
    }
    SubSessionOutcome::Finished(text)
}

fn tail(text: &str) -> &str {
    let mut start = text.len().saturating_sub(SHOWN_ERROR);
    while !text.is_char_boundary(start) {
        start += 1;
    }
    &text[start..]
}

#[cfg(test)]
#[path = "cli.test.rs"]
mod tests;
