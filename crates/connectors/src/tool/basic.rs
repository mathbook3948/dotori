//! The basic tools dotori's own loop has besides the shared MCP tools: reading and editing files
//! and running a shell command. Coding agents bring their own.

use std::path::Path;
use std::path::PathBuf;
use std::process::Stdio;
use std::sync::Arc;
use std::time::Duration;

use dotori_core::BoxFuture;
use dotori_core::tool::ToolCall;
use dotori_core::tool::ToolError;
use dotori_core::tool::ToolSpec;
use dotori_core::tool::Toolbox;
use serde::Deserialize;
use serde_json::Value;
use serde_json::json;
use tokio::process::Command;
use tokio::time;

use crate::process::Tree;

/// The name of the tool that reads a text file.
pub const READ_FILE: &str = "read_file";
/// The name of the tool that writes a whole file.
pub const WRITE_FILE: &str = "write_file";
/// The name of the tool that replaces text in a file.
pub const EDIT_FILE: &str = "edit_file";
/// The name of the tool that runs a shell command.
pub const RUN_SHELL: &str = "run_shell";
const LINES: usize = 2000;
const READ_BYTES: u64 = 10 * 1024 * 1024;
const SHOWN: usize = 256 * 1024;
const OUTPUT: usize = 30 * 1024;

/// Files and a shell on this PC, as the owner's own user. Relative paths start in `folder`.
#[derive(Clone, Debug)]
pub struct BasicTools {
    pub folder: PathBuf,
    pub limit: Duration,
    /// Environment variables a command must not see, such as tokens only the main session's
    /// guarded tools use.
    pub hidden_env: Vec<String>,
}

/// The basic tools in front of `rest`, which gets every other call.
#[derive(Clone)]
pub struct WithBasic {
    pub basic: BasicTools,
    pub rest: Arc<dyn Toolbox>,
}

#[derive(Deserialize)]
struct ReadInput {
    path: String,
    #[serde(default)]
    offset: Option<usize>,
    #[serde(default)]
    limit: Option<usize>,
}

#[derive(Deserialize)]
struct WriteInput {
    path: String,
    content: String,
}

#[derive(Deserialize)]
struct EditInput {
    path: String,
    old: String,
    new: String,
    #[serde(default)]
    all: bool,
}

#[derive(Deserialize)]
struct ShellInput {
    command: String,
    #[serde(default)]
    folder: Option<String>,
    #[serde(default)]
    seconds: Option<u64>,
}

impl BasicTools {
    /// What the model is told about each basic tool.
    pub fn specs(&self) -> Vec<ToolSpec> {
        let spec = |name: &str, description: &str, schema: Value| ToolSpec {
            name: name.to_owned(),
            description: description.to_owned(),
            input_schema: schema.to_string(),
        };
        vec![
            spec(
                READ_FILE,
                "Read a text file on the owner's PC. Lines come numbered from 1. A relative path \
                 starts in dotori's work folder.",
                json!({
                    "type": "object",
                    "properties": {
                        "path": { "type": "string" },
                        "offset": { "type": "integer", "minimum": 1, "description": "The first line to read." },
                        "limit": { "type": "integer", "minimum": 1, "description": "How many lines to read." }
                    },
                    "required": ["path"],
                    "additionalProperties": false
                }),
            ),
            spec(
                WRITE_FILE,
                "Write a whole file on the owner's PC, making its folders. An existing file is \
                 replaced.",
                json!({
                    "type": "object",
                    "properties": {
                        "path": { "type": "string" },
                        "content": { "type": "string" }
                    },
                    "required": ["path", "content"],
                    "additionalProperties": false
                }),
            ),
            spec(
                EDIT_FILE,
                "Replace text in a file on the owner's PC. `old` must appear exactly once unless \
                 `all` is set. Read the file first so `old` matches it exactly.",
                json!({
                    "type": "object",
                    "properties": {
                        "path": { "type": "string" },
                        "old": { "type": "string" },
                        "new": { "type": "string" },
                        "all": { "type": "boolean", "description": "Replace every place `old` appears." }
                    },
                    "required": ["path", "old", "new"],
                    "additionalProperties": false
                }),
            ),
            spec(
                RUN_SHELL,
                "Run a shell command on the owner's PC as the owner and get its exit code and \
                 output. It runs in dotori's work folder unless `folder` says otherwise, gets no \
                 input, and is stopped with everything it started when its time runs out. Hand \
                 long work to a sub-session instead.",
                json!({
                    "type": "object",
                    "properties": {
                        "command": { "type": "string" },
                        "folder": { "type": "string" },
                        "seconds": { "type": "integer", "minimum": 1, "description": "How long it may run." }
                    },
                    "required": ["command"],
                    "additionalProperties": false
                }),
            ),
        ]
    }

    fn handles(name: &str) -> bool {
        matches!(name, READ_FILE | WRITE_FILE | EDIT_FILE | RUN_SHELL)
    }

    async fn call(&self, call: &ToolCall) -> Result<String, String> {
        match call.name.as_str() {
            READ_FILE => self.read(parse(&call.input)?).await,
            WRITE_FILE => self.write(parse(&call.input)?).await,
            EDIT_FILE => self.edit(parse(&call.input)?).await,
            RUN_SHELL => self.shell(parse(&call.input)?).await,
            other => Err(format!("{other} is not a basic tool")),
        }
    }

    fn resolve(&self, path: &str) -> Result<PathBuf, String> {
        let path = path.trim();
        if path.is_empty() {
            return Err(String::from("the path is empty"));
        }
        let path = Path::new(path);
        Ok(if path.is_absolute() {
            path.to_path_buf()
        } else {
            self.folder.join(path)
        })
    }

    async fn read(&self, input: ReadInput) -> Result<String, String> {
        let path = self.resolve(&input.path)?;
        let size = tokio::fs::metadata(&path)
            .await
            .map_err(|error| format!("{}: {error}", path.display()))?
            .len();
        if size > READ_BYTES {
            return Err(format!(
                "{} is too large to read whole; read it with run_shell",
                path.display()
            ));
        }
        let bytes = tokio::fs::read(&path)
            .await
            .map_err(|error| format!("{}: {error}", path.display()))?;
        if bytes.contains(&0) {
            return Err(format!("{} is not a text file", path.display()));
        }
        let text = String::from_utf8_lossy(&bytes);
        let start = input.offset.unwrap_or(1).max(1);
        let count = input.limit.unwrap_or(LINES).clamp(1, LINES);
        let total = text.lines().count();
        let mut shown = String::new();
        for (n, line) in text.lines().enumerate().skip(start - 1).take(count) {
            let row = format!("{}\t{line}\n", n + 1);
            if shown.len() + row.len() > SHOWN {
                shown.push_str("(cut short; read on with `offset`)\n");
                break;
            }
            shown.push_str(&row);
        }
        if shown.is_empty() {
            return Ok(format!(
                "{} has {total} lines; none from line {start}",
                path.display()
            ));
        }
        let last = (start - 1 + count).min(total);
        if last < total {
            shown.push_str(&format!(
                "({total} lines in all; read on from line {})\n",
                last + 1
            ));
        }
        Ok(shown)
    }

    async fn write(&self, input: WriteInput) -> Result<String, String> {
        let path = self.resolve(&input.path)?;
        if let Some(parent) = path.parent() {
            tokio::fs::create_dir_all(parent)
                .await
                .map_err(|error| format!("{}: {error}", parent.display()))?;
        }
        tokio::fs::write(&path, input.content.as_bytes())
            .await
            .map_err(|error| format!("{}: {error}", path.display()))?;
        Ok(format!(
            "Wrote {} bytes to {}.",
            input.content.len(),
            path.display()
        ))
    }

    async fn edit(&self, input: EditInput) -> Result<String, String> {
        let path = self.resolve(&input.path)?;
        if input.old.is_empty() {
            return Err(String::from(
                "`old` is empty; use write_file for a new file",
            ));
        }
        let text = tokio::fs::read_to_string(&path)
            .await
            .map_err(|error| format!("{}: {error}", path.display()))?;
        let found = text.matches(&input.old).count();
        if found == 0 {
            return Err(format!(
                "`old` is not in {}; read it again and copy the text exactly",
                path.display()
            ));
        }
        if found > 1 && !input.all {
            return Err(format!(
                "`old` appears {found} times in {}; give more of the text around it, or set `all`",
                path.display()
            ));
        }
        let changed = if input.all {
            text.replace(&input.old, &input.new)
        } else {
            text.replacen(&input.old, &input.new, 1)
        };
        tokio::fs::write(&path, changed.as_bytes())
            .await
            .map_err(|error| format!("{}: {error}", path.display()))?;
        Ok(format!(
            "Replaced {} place(s) in {}.",
            if input.all { found } else { 1 },
            path.display()
        ))
    }

    async fn shell(&self, input: ShellInput) -> Result<String, String> {
        if input.command.trim().is_empty() {
            return Err(String::from("the command is empty"));
        }
        let folder = match &input.folder {
            Some(folder) => self.resolve(folder)?,
            None => {
                tokio::fs::create_dir_all(&self.folder)
                    .await
                    .map_err(|error| format!("{}: {error}", self.folder.display()))?;
                self.folder.clone()
            }
        };
        let limit = input
            .seconds
            .map(Duration::from_secs)
            .unwrap_or(self.limit)
            .min(self.limit);
        let mut command = if cfg!(windows) {
            let mut command = Command::new("cmd");
            command.arg("/C").arg(&input.command);
            command
        } else {
            let mut command = Command::new("sh");
            command.arg("-c").arg(&input.command);
            command
        };
        command
            .current_dir(&folder)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);
        #[cfg(unix)]
        command.process_group(0);
        for hidden in &self.hidden_env {
            command.env_remove(hidden);
        }
        let child = command
            .spawn()
            .map_err(|error| format!("cannot run in {}: {error}", folder.display()))?;
        let mut tree = Tree(child.id());
        let output = match time::timeout(limit, child.wait_with_output()).await {
            Ok(output) => output.map_err(|error| error.to_string())?,
            Err(_) => {
                return Err(format!(
                    "the command did not finish in {} seconds and was stopped",
                    limit.as_secs()
                ));
            }
        };
        tree.0 = None;
        let code = output.status.code().map_or_else(
            || String::from("none (stopped by a signal)"),
            |code| code.to_string(),
        );
        Ok(format!(
            "exit code: {code}\nstdout:\n{}\nstderr:\n{}",
            tail(&String::from_utf8_lossy(&output.stdout)),
            tail(&String::from_utf8_lossy(&output.stderr))
        ))
    }
}

impl std::fmt::Debug for WithBasic {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("WithBasic")
            .field("basic", &self.basic)
            .finish_non_exhaustive()
    }
}

impl Toolbox for WithBasic {
    fn specs(&self) -> Vec<ToolSpec> {
        let mut specs = self.basic.specs();
        specs.extend(
            self.rest
                .specs()
                .into_iter()
                .filter(|spec| !BasicTools::handles(&spec.name)),
        );
        specs
    }

    fn run(&self, call: ToolCall) -> BoxFuture<'_, Result<String, ToolError>> {
        if !BasicTools::handles(&call.name) {
            return self.rest.run(call);
        }
        Box::pin(async move {
            self.basic
                .call(&call)
                .await
                .map_err(|detail| ToolError { detail })
        })
    }
}

fn parse<T: for<'de> Deserialize<'de>>(input: &str) -> Result<T, String> {
    serde_json::from_str(input).map_err(|error| format!("the input does not fit: {error}"))
}

fn tail(text: &str) -> String {
    if text.len() <= OUTPUT {
        return text.to_owned();
    }
    let mut start = text.len() - OUTPUT;
    while !text.is_char_boundary(start) {
        start += 1;
    }
    format!("(only the last part)\n{}", &text[start..])
}

#[cfg(test)]
#[path = "basic.test.rs"]
mod tests;
