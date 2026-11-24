//! Processes dotori starts and must be able to end together with everything they started.

use std::process::Stdio;

/// The process with this id and every process it started, ended when dropped unless the id was
/// taken out because the process finished on its own.
pub(crate) struct Tree(pub(crate) Option<u32>);

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
