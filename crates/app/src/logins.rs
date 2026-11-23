//! Carries the browser's logins from the PC that holds the browser to the others, over the
//! encrypted link, so the PC that takes the browser over next starts already logged in.

use std::path::Path;
use std::path::PathBuf;
use std::time::Duration;

use dotori_connectors::peer::hub::Heard;
use dotori_mcp::browser::LoginCopy;
use serde_json::Value;
use serde_json::json;
use tokio::time;

use crate::browser_route::BROWSER_JOB;
use crate::main_session::Linked;

const LOOK_EVERY: Duration = Duration::from_secs(30);

/// Where this PC keeps its own copy and the one it took from another PC.
#[derive(Clone, Debug)]
pub(crate) struct LoginFiles {
    pub(crate) out: PathBuf,
    pub(crate) into: PathBuf,
}

impl LoginFiles {
    fn ours(&self) -> Option<LoginCopy> {
        read(&self.out)
    }

    /// Keeps a copy another PC sent, if it is newer than any copy here. Says whether it was.
    pub(crate) fn keep(&self, copy: &LoginCopy) -> Result<bool, String> {
        let newest = [read(&self.out), read(&self.into)]
            .into_iter()
            .flatten()
            .map(|copy| copy.at)
            .max()
            .unwrap_or(0);
        if copy.at <= newest {
            return Ok(false);
        }
        write_privately(&self.into, copy)?;
        Ok(true)
    }
}

/// Sends this PC's copy to the others whenever it changes, while this PC holds the browser.
pub(crate) async fn share(linked: Linked, files: LoginFiles) {
    let mut sent = 0;
    let mut every = time::interval(LOOK_EVERY);
    loop {
        every.tick().await;
        if !linked.keeper.holds(BROWSER_JOB) {
            continue;
        }
        let Some(copy) = files.ours() else {
            continue;
        };
        if copy.at > sent {
            sent = copy.at;
            linked
                .hub
                .broadcast(&json!({ "kind": "logins", "copy": copy }));
        }
    }
}

/// Takes a message about logins from another PC. Says whether it was one.
pub(crate) fn hear(linked: &Linked, files: &LoginFiles, heard: &Heard) -> bool {
    match heard.message.get("kind").and_then(Value::as_str) {
        Some("linked") => {
            if linked.keeper.holds(BROWSER_JOB)
                && let Some(copy) = files.ours()
            {
                linked
                    .hub
                    .send(&heard.key, json!({ "kind": "logins", "copy": copy }));
            }
            false
        }
        Some("logins") => {
            if linked.keeper.holds(BROWSER_JOB) {
                return true;
            }
            let copy = heard
                .message
                .get("copy")
                .and_then(|copy| serde_json::from_value::<LoginCopy>(copy.clone()).ok());
            if let Some(Err(error)) = copy.map(|copy| files.keep(&copy)) {
                eprintln!(
                    "dotori: logins from {} were not kept: {error}",
                    heard.device
                );
            }
            true
        }
        _ => false,
    }
}

fn read(path: &Path) -> Option<LoginCopy> {
    serde_json::from_str(&std::fs::read_to_string(path).ok()?).ok()
}

fn write_privately(path: &Path, copy: &LoginCopy) -> Result<(), String> {
    use std::io::Write;
    let text = serde_json::to_vec(copy).map_err(|error| error.to_string())?;
    let draft = path.with_extension("json.new");
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options.open(&draft).map_err(|error| error.to_string())?;
    file.write_all(&text).map_err(|error| error.to_string())?;
    file.sync_all().map_err(|error| error.to_string())?;
    std::fs::rename(&draft, path).map_err(|error| error.to_string())
}

#[cfg(test)]
#[path = "logins.test.rs"]
mod tests;
