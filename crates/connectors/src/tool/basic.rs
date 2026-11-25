//! The basic tools dotori's own loop has besides the shared MCP tools: reading and editing files
//! and running a shell command. Coding agents bring their own.

use std::fs::File;
use std::io::Read;
use std::io::Write;
use std::path::Component;
use std::path::Path;
use std::path::PathBuf;
use std::process::Stdio;
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::atomic::AtomicU64;
use std::sync::atomic::Ordering;
use std::time::Duration;

use dotori_core::BoxFuture;
use dotori_core::tool::ToolCall;
use dotori_core::tool::ToolError;
use dotori_core::tool::ToolSpec;
use dotori_core::tool::Toolbox;
use serde::Deserialize;
use serde_json::Value;
use serde_json::json;
use tokio::io::AsyncRead;
use tokio::io::AsyncReadExt;
use tokio::process::Command;
use tokio::task::JoinHandle;
use tokio::time;

use crate::process::Tree;
use crate::sandbox::Sandbox;

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
const LINE: usize = 4000;
const DRAIN: Duration = Duration::from_secs(1);
const HOPS: u8 = 40;

static STAGED: AtomicU64 = AtomicU64::new(0);

/// Files and a shell on this PC, as the owner's own user. Relative paths start in `folder`.
#[derive(Clone, Debug)]
pub struct BasicTools {
    pub folder: PathBuf,
    pub limit: Duration,
    /// Environment variables a command must not see, such as tokens only the main session's
    /// guarded tools use.
    pub hidden_env: Vec<String>,
    /// A folder whose files these tools leave alone, such as dotori's own.
    pub private: Option<Private>,
    /// The sandbox commands run in when it works on this PC, which hides the private folder.
    pub sandbox: Option<Arc<Sandbox>>,
}

/// A folder the basic tools leave alone, apart from `open` inside it.
#[derive(Clone, Debug)]
pub struct Private {
    pub folder: PathBuf,
    pub open: PathBuf,
}

impl Private {
    /// Whether the work folder is a plain folder, not a link, whose real place is inside the
    /// private folder. It is made first when it is not there yet.
    pub fn plain_work(&self) -> bool {
        if std::fs::symlink_metadata(&self.open)
            .is_err_and(|error| error.kind() == std::io::ErrorKind::NotFound)
        {
            let _ = std::fs::create_dir_all(&self.folder);
            let _ = std::fs::create_dir(&self.open);
        }
        let plain =
            std::fs::symlink_metadata(&self.open).is_ok_and(|meta| meta.file_type().is_dir());
        let (Ok(open), Ok(folder)) = (self.open.canonicalize(), self.folder.canonicalize()) else {
            return false;
        };
        plain && open != folder && open.starts_with(&folder)
    }
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
        let folder = if self
            .private
            .as_ref()
            .is_some_and(|private| private.open == self.folder)
        {
            String::from("dotori's work folder")
        } else {
            self.folder.display().to_string()
        };
        let spec = |name: &str, description: &str, schema: Value| ToolSpec {
            name: name.to_owned(),
            description: description.to_owned(),
            input_schema: schema.to_string(),
        };
        vec![
            spec(
                READ_FILE,
                &format!(
                    "Read a text file on the owner's PC. Lines come numbered from 1. A relative \
                     path starts in {folder}."
                ),
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
                &format!(
                    "Write a whole file on the owner's PC, making its folders. An existing file \
                     is replaced. A relative path starts in {folder}."
                ),
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
                &format!(
                    "Replace text in a file on the owner's PC. `old` must appear exactly once \
                     unless `all` is set. Read the file first so `old` matches it exactly. A \
                     relative path starts in {folder}."
                ),
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
                &format!(
                    "Run a shell command on the owner's PC as the owner and get its exit code and \
                     output. It runs in {folder} unless `folder` says otherwise, gets no input, \
                     and is stopped with everything it started when its time runs out. Hand long \
                     work to a sub-session instead."
                ),
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
            READ_FILE | WRITE_FILE | EDIT_FILE => {
                let (tools, call) = (self.clone(), call.clone());
                tokio::task::spawn_blocking(move || tools.file_call(&call))
                    .await
                    .map_err(|error| error.to_string())?
            }
            RUN_SHELL => self.shell(parse(&call.input)?).await,
            other => Err(format!("{other} is not a basic tool")),
        }
    }

    fn resolve(&self, path: &str) -> Result<PathBuf, String> {
        if path.trim().is_empty() {
            return Err(String::from("the path is empty"));
        }
        let path = Path::new(path);
        Ok(if path.is_absolute() {
            path.to_path_buf()
        } else {
            self.folder.join(path)
        })
    }

    fn file_call(&self, call: &ToolCall) -> Result<String, String> {
        match call.name.as_str() {
            READ_FILE => self.read(parse(&call.input)?),
            WRITE_FILE => self.write(parse(&call.input)?),
            _ => self.edit(parse(&call.input)?),
        }
    }

    fn file(&self, path: &str) -> Result<(PathBuf, PathBuf), String> {
        let path = self.resolve(path)?;
        if let Some(private) = &self.private
            && !private.plain_work()
        {
            return Err(String::from(
                "dotori's work folder is not a plain folder inside dotori's folder, so these \
                 tools wait until it is one again",
            ));
        }
        let real = followed(&path, &mut HOPS.clone());
        self.allowed(&real, &path)?;
        Ok((path, real))
    }

    fn allowed(&self, real: &Path, shown: &Path) -> Result<(), String> {
        let Some(private) = &self.private else {
            return Ok(());
        };
        let open = followed(&private.open, &mut HOPS.clone());
        let folder = followed(&private.folder, &mut HOPS.clone());
        let inside = real.starts_with(&folder) || same_within(real, &folder);
        let work = real.starts_with(&open) || same_within(real, &open);
        if (inside && !work) || elsewhere_spelled(real) {
            return Err(format!(
                "{} is dotori's own; these tools work only in its work folder there",
                shown.display()
            ));
        }
        Ok(())
    }

    fn held(&self, opened: &File, real: &Path, shown: &Path) -> Result<(), String> {
        if self.private.is_none() {
            return Ok(());
        }
        let at =
            opened_at(opened, real).map_err(|error| format!("{}: {error}", shown.display()))?;
        self.allowed(&at, shown)
    }

    fn read(&self, input: ReadInput) -> Result<String, String> {
        let (path, real) = self.file(&input.path)?;
        let shown = path.display();
        let fail = |error: std::io::Error| format!("{shown}: {error}");
        if !std::fs::metadata(&real).map_err(fail)?.is_file() {
            return Err(format!("{shown} is not a file"));
        }
        let file = opening(&real).map_err(fail)?;
        self.held(&file, &real, &path)?;
        if !file.metadata().map_err(fail)?.is_file() {
            return Err(format!("{shown} is not a file"));
        }
        let mut bytes = Vec::new();
        file.take(READ_BYTES + 1)
            .read_to_end(&mut bytes)
            .map_err(fail)?;
        if bytes.len() as u64 > READ_BYTES {
            return Err(format!(
                "{shown} is too large to read whole; read parts of it with run_shell"
            ));
        }
        if bytes.contains(&0) {
            return Err(format!("{shown} is not a text file"));
        }
        let text = String::from_utf8_lossy(&bytes);
        let start = input.offset.unwrap_or(1).max(1);
        let count = input.limit.unwrap_or(LINES).clamp(1, LINES);
        let total = text.lines().count();
        let mut out = String::new();
        let mut last = start - 1;
        for (n, line) in text.lines().enumerate().skip(start - 1).take(count) {
            let mut kept: String = line.chars().take(LINE).collect();
            if kept.len() < line.len() {
                kept.push_str(" …(line cut short)");
            }
            let row = format!("{}\t{kept}\n", n + 1);
            if !out.is_empty() && out.len() + row.len() > SHOWN {
                break;
            }
            out.push_str(&row);
            last = n + 1;
        }
        if out.is_empty() {
            return Ok(format!("{shown} has {total} lines; none from line {start}"));
        }
        if last < total {
            out.push_str(&format!(
                "({total} lines in all; read on from line {})\n",
                last + 1
            ));
        }
        Ok(out)
    }

    fn write(&self, input: WriteInput) -> Result<String, String> {
        let (path, real) = self.file(&input.path)?;
        self.replace(&path, &real, input.content.as_bytes())?;
        Ok(format!(
            "Wrote {} bytes to {}.",
            input.content.len(),
            path.display()
        ))
    }

    fn edit(&self, input: EditInput) -> Result<String, String> {
        let (path, real) = self.file(&input.path)?;
        if input.old.is_empty() {
            return Err(String::from(
                "`old` is empty; use write_file for a new file",
            ));
        }
        let fail = |error: std::io::Error| format!("{}: {error}", path.display());
        let file = opening(&real).map_err(fail)?;
        self.held(&file, &real, &path)?;
        let mut bytes = Vec::new();
        file.take(READ_BYTES + 1)
            .read_to_end(&mut bytes)
            .map_err(fail)?;
        if bytes.len() as u64 > READ_BYTES {
            return Err(format!(
                "{} is too large to edit here; change it with run_shell",
                path.display()
            ));
        }
        let text = String::from_utf8(bytes).map_err(|_| {
            format!(
                "{} is not UTF-8 text, so it is not edited; change it with run_shell",
                path.display()
            )
        })?;
        let crlf = |text: &str| text.replace("\r\n", "\n").replace('\n', "\r\n");
        let (old, new) = if text.contains("\r\n") && !input.old.contains('\r') {
            (crlf(&input.old), crlf(&input.new))
        } else {
            (input.old.clone(), input.new.clone())
        };
        let found = text.matches(&old).count();
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
            text.replace(&old, &new)
        } else {
            text.replacen(&old, &new, 1)
        };
        self.replace(&path, &real, changed.as_bytes())?;
        Ok(format!(
            "Replaced {} place(s) in {}.",
            if input.all { found } else { 1 },
            path.display()
        ))
    }

    #[cfg(unix)]
    fn replace(&self, shown: &Path, real: &Path, bytes: &[u8]) -> Result<(), String> {
        use std::os::unix::fs::PermissionsExt;

        use rustix::fs::AtFlags;
        use rustix::fs::Mode;
        use rustix::fs::OFlags;
        use rustix::fs::RawMode;

        let fail = |error: std::io::Error| format!("{}: {error}", shown.display());
        let failed = |error: rustix::io::Errno| fail(error.into());
        let (Some(parent), Some(name)) = (real.parent(), real.file_name()) else {
            return Err(format!("{} is not a file", shown.display()));
        };
        std::fs::create_dir_all(parent).map_err(fail)?;
        let folder = File::open(parent).map_err(fail)?;
        self.held(&folder, parent, shown)?;
        let kept = std::fs::symlink_metadata(real)
            .ok()
            .filter(std::fs::Metadata::is_file)
            .map(|was| was.permissions().mode() & 0o7777);
        let staged = format!(
            ".{}.dotori-{}-{}",
            name.to_string_lossy(),
            std::process::id(),
            STAGED.fetch_add(1, Ordering::Relaxed)
        );
        let fd = rustix::fs::openat(
            &folder,
            staged.as_str(),
            OFlags::CREATE | OFlags::EXCL | OFlags::WRONLY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
            Mode::from_bits_truncate(kept.unwrap_or(0o666) as RawMode),
        )
        .map_err(failed)?;
        let written = (|| {
            if let Some(kept) = kept {
                rustix::fs::fchmod(&fd, Mode::from_bits_truncate(kept as RawMode))?;
            }
            let mut file = File::from(fd);
            file.write_all(bytes)?;
            file.sync_all()?;
            drop(file);
            rustix::fs::renameat(&folder, staged.as_str(), &folder, name)?;
            Ok::<(), std::io::Error>(())
        })();
        written.map_err(|error| {
            let _ = rustix::fs::unlinkat(&folder, staged.as_str(), AtFlags::empty());
            fail(error)
        })
    }

    #[cfg(not(unix))]
    fn replace(&self, shown: &Path, real: &Path, bytes: &[u8]) -> Result<(), String> {
        let fail = |error: std::io::Error| format!("{}: {error}", shown.display());
        let (Some(parent), Some(name)) = (real.parent(), real.file_name()) else {
            return Err(format!("{} is not a file", shown.display()));
        };
        std::fs::create_dir_all(parent).map_err(fail)?;
        let staged = parent.join(format!(
            ".{}.dotori-{}-{}",
            name.to_string_lossy(),
            std::process::id(),
            STAGED.fetch_add(1, Ordering::Relaxed)
        ));
        let written = (|| {
            let mut file = std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&staged)?;
            self.held(&file, &staged, shown)
                .map_err(std::io::Error::other)?;
            file.write_all(bytes)?;
            file.sync_all()?;
            drop(file);
            if let Ok(was) = std::fs::metadata(real) {
                std::fs::set_permissions(&staged, was.permissions())?;
            }
            std::fs::rename(&staged, real)
        })();
        written.map_err(|error| {
            let _ = std::fs::remove_file(&staged);
            fail(error)
        })
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
        let confinement = match &self.sandbox {
            Some(sandbox) => sandbox
                .ready()
                .await
                .map_err(|blocked| blocked.to_string())?
                .map(|found| (sandbox, found)),
            None => None,
        };
        let mut command = match confinement {
            Some((sandbox, found)) => sandbox.command(found, &input.command, &folder),
            None => shell_command(&input.command),
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
        let mut child = command
            .spawn()
            .map_err(|error| format!("cannot run in {}: {error}", folder.display()))?;
        let mut tree = Tree(child.id());
        let stdout = child.stdout.take().map(reading);
        let stderr = child.stderr.take().map(reading);
        let status = match time::timeout(limit, child.wait()).await {
            Ok(status) => status.map_err(|error| error.to_string())?,
            Err(_) => {
                drop(tree);
                return Err(format!(
                    "the command did not finish in {} seconds and was stopped. Its output so far:\n{}",
                    limit.as_secs(),
                    outputs(&drained(stdout).await, &drained(stderr).await)
                ));
            }
        };
        tree.0 = None;
        let code = status.code().map_or_else(
            || String::from("none (stopped by a signal)"),
            |code| code.to_string(),
        );
        Ok(format!(
            "exit code: {code}\n{}",
            outputs(&drained(stdout).await, &drained(stderr).await)
        ))
    }
}

fn followed(path: &Path, hops: &mut u8) -> PathBuf {
    let mut real = PathBuf::new();
    for part in path.components() {
        match part {
            Component::ParentDir => {
                real.pop();
            }
            Component::CurDir => {}
            Component::Prefix(_) | Component::RootDir => real.push(part),
            Component::Normal(_) => {
                real.push(part);
                if let Ok(resolved) = real.canonicalize() {
                    real = resolved;
                } else if *hops > 0
                    && let Ok(target) = std::fs::read_link(&real)
                {
                    *hops -= 1;
                    real.pop();
                    real = followed(&real.join(target), hops);
                }
            }
        }
    }
    real
}

#[cfg(unix)]
fn same_within(path: &Path, folder: &Path) -> bool {
    use std::os::unix::fs::MetadataExt;
    let Ok(folder) = std::fs::metadata(folder) else {
        return false;
    };
    path.ancestors()
        .filter_map(|known| std::fs::metadata(known).ok())
        .any(|known| known.dev() == folder.dev() && known.ino() == folder.ino())
}

#[cfg(not(unix))]
fn same_within(_path: &Path, _folder: &Path) -> bool {
    false
}

fn elsewhere_spelled(path: &Path) -> bool {
    let text = path.to_string_lossy();
    cfg!(windows)
        && (text.starts_with(r"\\?\UNC\")
            || (text.starts_with(r"\\") && !text.starts_with(r"\\?\")))
}

/// A command's two outputs, each headed with its length in characters so a reader can tell them
/// apart whatever they hold.
pub fn outputs(stdout: &str, stderr: &str) -> String {
    format!(
        "stdout ({} characters):\n{stdout}\nstderr ({} characters):\n{stderr}",
        stdout.chars().count(),
        stderr.chars().count()
    )
}

#[cfg(windows)]
fn shell_command(line: &str) -> Command {
    let mut command = Command::new("cmd");
    command.raw_arg(format!("/S /C \"{line}\""));
    command
}

#[cfg(not(windows))]
fn shell_command(line: &str) -> Command {
    let mut command = Command::new("sh");
    command.arg("-c").arg(line);
    command
}

type Kept = Arc<Mutex<(Vec<u8>, bool)>>;

fn reading(from: impl AsyncRead + Unpin + Send + 'static) -> (Kept, JoinHandle<()>) {
    let kept: Kept = Arc::default();
    let filling = Arc::clone(&kept);
    let task = tokio::spawn(async move {
        let mut from = from;
        let mut buffer = [0u8; 8192];
        while let Ok(read) = from.read(&mut buffer).await {
            if read == 0 {
                break;
            }
            let mut kept = filling
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            kept.0.extend_from_slice(&buffer[..read]);
            if kept.0.len() > OUTPUT * 2 {
                let extra = kept.0.len() - OUTPUT;
                kept.0.drain(..extra);
                kept.1 = true;
            }
        }
    });
    (kept, task)
}

async fn drained(reading: Option<(Kept, JoinHandle<()>)>) -> String {
    let Some((kept, mut task)) = reading else {
        return String::new();
    };
    let ended = time::timeout(DRAIN, &mut task).await.is_ok();
    let (bytes, cut) = kept
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .clone();
    let text = String::from_utf8_lossy(&bytes);
    let mut shown = tail(&text);
    if cut && !shown.starts_with("(only the last part)") {
        shown = format!("(only the last part)\n{shown}");
    }
    if !ended {
        shown.push_str("\n(something the command left running still writes here)");
    }
    shown
}

#[cfg(unix)]
fn opening(path: &Path) -> std::io::Result<File> {
    use std::os::unix::fs::OpenOptionsExt;
    std::fs::OpenOptions::new()
        .read(true)
        .custom_flags(rustix::fs::OFlags::NONBLOCK.bits() as i32)
        .open(path)
}

#[cfg(not(unix))]
fn opening(path: &Path) -> std::io::Result<File> {
    File::open(path)
}

#[cfg(target_os = "linux")]
fn opened_at(opened: &File, _real: &Path) -> std::io::Result<PathBuf> {
    use std::os::fd::AsRawFd;
    std::fs::read_link(format!("/proc/self/fd/{}", opened.as_raw_fd()))
}

#[cfg(target_os = "macos")]
fn opened_at(opened: &File, _real: &Path) -> std::io::Result<PathBuf> {
    use std::os::unix::ffi::OsStringExt;
    let path = rustix::fs::getpath(opened)?;
    Ok(PathBuf::from(std::ffi::OsString::from_vec(
        path.into_bytes(),
    )))
}

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
fn opened_at(_opened: &File, real: &Path) -> std::io::Result<PathBuf> {
    real.canonicalize()
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
