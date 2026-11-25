//! The sandbox the shell commands of dotori's own model run in. It hides dotori's folder, apart
//! from its work folder, and leaves the rest of the PC and the network as they are. macOS has it
//! built in; Linux uses bubblewrap once it runs for the owner's user; Windows has none.

use std::path::Path;
use std::path::PathBuf;
use std::process::Stdio;
use std::sync::OnceLock;
use std::time::Duration;

use tokio::process::Command;
use tokio::time;

use crate::tool::basic::Private;

/// Where `dotori sandbox install` puts a copy of bubblewrap that the system lets make user
/// namespaces, for a Linux that keeps them from other programs.
pub const HELPER: &str = "/usr/local/libexec/dotori/bwrap";

const SEATBELT: &str = "/usr/bin/sandbox-exec";
const BUBBLEWRAP: [&str; 3] = [HELPER, "/usr/bin/bwrap", "/bin/bwrap"];
const PROBE: Duration = Duration::from_secs(10);

/// What confines a command on this PC.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Confinement {
    /// macOS's Seatbelt, through `sandbox-exec` and a profile dotori writes.
    Seatbelt,
    /// bubblewrap, by the program at this path.
    Bubblewrap(PathBuf),
}

/// Why a command the sandbox should hold is refused instead of run.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Blocked {
    /// The work folder is not a plain folder inside dotori's folder, so opening it again could
    /// open more than it.
    WorkNotPlain,
    /// The sandbox this PC has did not run.
    Failed,
}

impl std::fmt::Display for Blocked {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::WorkNotPlain => {
                "dotori's work folder is not a plain folder inside dotori's folder, so no command \
                 runs until it is one again"
            }
            Self::Failed => {
                "the sandbox commands run in did not start on this PC, so no command runs until it does"
            }
        })
    }
}

/// The sandbox for one dotori folder. It looks for a confinement that works each time it is
/// asked until one does, and keeps that one.
#[derive(Debug)]
pub struct Sandbox {
    private: Private,
    found: OnceLock<Confinement>,
}

impl Sandbox {
    /// A sandbox that hides `private.folder` and opens `private.open` inside it again.
    pub fn new(private: &Private) -> Self {
        Self {
            private: private.clone(),
            found: OnceLock::new(),
        }
    }

    /// The confinement a command runs in now, `None` when this PC has none and commands run as
    /// they are, or why a command must not run.
    pub async fn ready(&self) -> Result<Option<&Confinement>, Blocked> {
        let candidates = candidates();
        if candidates.is_empty() {
            return Ok(None);
        }
        if !self.private.plain_work() {
            return Err(Blocked::WorkNotPlain);
        }
        if let Some(found) = self.found.get() {
            return Ok(Some(found));
        }
        for candidate in candidates {
            if self.works(&candidate).await {
                return Ok(Some(self.found.get_or_init(|| candidate)));
            }
        }
        if cfg!(target_os = "macos") {
            Err(Blocked::Failed)
        } else {
            Ok(None)
        }
    }

    /// `line` for `sh` to run in `folder`, inside `confinement`.
    pub fn command(&self, confinement: &Confinement, line: &str, folder: &Path) -> Command {
        let (hidden, open) = (real(&self.private.folder), real(&self.private.open));
        let mut command = match confinement {
            Confinement::Seatbelt => {
                let above: Vec<&Path> = hidden
                    .ancestors()
                    .skip(1)
                    .filter(|folder| folder.parent().is_some())
                    .collect();
                let mut command = Command::new(SEATBELT);
                command
                    .arg("-D")
                    .arg(param("HIDDEN", &hidden))
                    .arg("-D")
                    .arg(param("OPEN", &open));
                for (n, folder) in above.iter().enumerate() {
                    command.arg("-D").arg(param(&format!("ABOVE{n}"), folder));
                }
                command.arg("-p").arg(profile(above.len()));
                command
            }
            Confinement::Bubblewrap(program) => {
                let mut command = Command::new(program);
                command
                    .args(["--die-with-parent", "--new-session", "--unshare-pid"])
                    .args(["--dev-bind", "/", "/", "--proc", "/proc", "--tmpfs"])
                    .arg(&hidden)
                    .arg("--bind")
                    .arg(&open)
                    .arg(&open)
                    .arg("--chdir")
                    .arg(folder)
                    .arg("--");
                command
            }
        };
        command
            .arg("/bin/sh")
            .arg("-c")
            .arg(line)
            .current_dir(folder);
        command
    }

    async fn works(&self, candidate: &Confinement) -> bool {
        let mut probe = self.command(candidate, "exit 0", &self.private.open);
        probe
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .kill_on_drop(true);
        matches!(
            time::timeout(PROBE, probe.status()).await,
            Ok(Ok(status)) if status.success()
        )
    }
}

fn profile(above: usize) -> String {
    let mut profile = String::from(
        "(version 1)\n\
         (allow default)\n\
         (deny file* (subpath (param \"HIDDEN\")))\n\
         (allow file-read-metadata (literal (param \"HIDDEN\")))\n\
         (allow file* (subpath (param \"OPEN\")))\n\
         (deny file-write-unlink file-write-create (literal (param \"OPEN\")))\n",
    );
    for n in 0..above {
        profile.push_str(&format!(
            "(deny file-write-unlink file-write-create (literal (param \"ABOVE{n}\")))\n"
        ));
    }
    profile
}

fn candidates() -> Vec<Confinement> {
    if cfg!(target_os = "macos") {
        return Path::new(SEATBELT)
            .is_file()
            .then_some(Confinement::Seatbelt)
            .into_iter()
            .collect();
    }
    if !cfg!(target_os = "linux") {
        return Vec::new();
    }
    BUBBLEWRAP
        .iter()
        .map(PathBuf::from)
        .filter(|program| root_owned(program))
        .map(Confinement::Bubblewrap)
        .collect()
}

/// Whether `program` is a file that only root can change, in folders only root can change.
#[cfg(unix)]
pub fn root_owned(program: &Path) -> bool {
    use std::os::unix::fs::MetadataExt;
    let Ok(real) = program.canonicalize() else {
        return false;
    };
    let closed = |path: &Path| {
        std::fs::metadata(path).is_ok_and(|meta| meta.uid() == 0 && meta.mode() & 0o022 == 0)
    };
    std::fs::metadata(&real).is_ok_and(|meta| meta.is_file()) && real.ancestors().all(closed)
}

/// Whether `program` is a file that only root can change, in folders only root can change.
#[cfg(not(unix))]
pub fn root_owned(_program: &Path) -> bool {
    false
}

fn real(path: &Path) -> PathBuf {
    path.canonicalize().unwrap_or_else(|_| path.to_path_buf())
}

fn param(name: &str, path: &Path) -> String {
    format!("{name}={}", path.display())
}

#[cfg(test)]
#[path = "sandbox.test.rs"]
mod tests;
