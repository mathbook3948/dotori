//! The sandbox the shell commands of dotori's own model run in. It hides dotori's folder, apart
//! from its work folder, and leaves the rest of the PC and the network as they are. macOS has it
//! built in; Linux uses bubblewrap once it runs for the owner's user; Windows has none.

use std::path::Path;
use std::path::PathBuf;
use std::process::Stdio;
use std::sync::OnceLock;

use tokio::process::Command;

use crate::tool::basic::Private;

/// Where `dotori sandbox install` puts a copy of bubblewrap that the system lets make user
/// namespaces, for a Linux that keeps them from other programs.
pub const HELPER: &str = "/usr/local/libexec/dotori/bwrap";

const SEATBELT: &str = "/usr/bin/sandbox-exec";
const PROFILE: &str = r#"(version 1)
(allow default)
(deny file-read* file-write* (subpath (param "HIDDEN")))
(allow file-read-metadata (literal (param "HIDDEN")))
(allow file-read* file-write* (subpath (param "OPEN")))
"#;

/// What confines a command on this PC.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Confinement {
    /// macOS's Seatbelt, through `sandbox-exec` and a profile dotori writes.
    Seatbelt,
    /// bubblewrap, by the program at this path.
    Bubblewrap(PathBuf),
}

/// The sandbox for one dotori folder. It looks for a confinement that works each time it is
/// asked until one does, and keeps that one.
#[derive(Debug)]
pub struct Sandbox {
    hidden: PathBuf,
    open: PathBuf,
    found: OnceLock<Confinement>,
}

impl Sandbox {
    /// A sandbox that hides `private.folder` and opens `private.open` inside it again.
    pub fn new(private: &Private) -> Self {
        Self {
            hidden: private.folder.clone(),
            open: private.open.clone(),
            found: OnceLock::new(),
        }
    }

    /// The confinement a command runs in now, if one works on this PC.
    pub async fn ready(&self) -> Option<&Confinement> {
        if let Some(found) = self.found.get() {
            return Some(found);
        }
        for candidate in candidates() {
            if self.works(&candidate).await {
                return Some(self.found.get_or_init(|| candidate));
            }
        }
        None
    }

    /// `line` for `sh` to run in `folder`, inside `confinement`.
    pub fn command(&self, confinement: &Confinement, line: &str, folder: &Path) -> Command {
        let (hidden, open) = (real(&self.hidden), real(&self.open));
        let mut command = match confinement {
            Confinement::Seatbelt => {
                let mut command = Command::new(SEATBELT);
                command
                    .arg("-D")
                    .arg(param("HIDDEN", &hidden))
                    .arg("-D")
                    .arg(param("OPEN", &open))
                    .arg("-p")
                    .arg(PROFILE);
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
        if std::fs::create_dir_all(&self.open).is_err() {
            return false;
        }
        self.command(candidate, "exit 0", &self.open)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .kill_on_drop(true)
            .status()
            .await
            .is_ok_and(|status| status.success())
    }
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
    let on_path = std::env::var_os("PATH")
        .into_iter()
        .flat_map(|path| std::env::split_paths(&path).collect::<Vec<_>>())
        .map(|folder| folder.join("bwrap"));
    std::iter::once(PathBuf::from(HELPER))
        .chain(on_path)
        .filter(|program| program.is_file())
        .map(Confinement::Bubblewrap)
        .collect()
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
