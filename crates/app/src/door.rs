use std::fs;
use std::fs::OpenOptions;
use std::io::Write;
use std::path::Path;

use serde::Deserialize;
use serde::Serialize;

/// How a window reaches the running daemon. The daemon writes it on start and removes it on stop.
/// Only the user can read it, because the token lets a window speak as the owner.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct Door {
    pub(crate) port: u16,
    pub(crate) token: String,
}

impl Door {
    pub(crate) fn write(&self, path: &Path) -> Result<(), String> {
        let fail = |error: std::io::Error| format!("{}: {error}", path.display());
        if let Some(folder) = path.parent() {
            fs::create_dir_all(folder).map_err(fail)?;
        }
        let text = serde_json::to_string(self).map_err(|error| error.to_string())?;
        let draft = path.with_extension("json.new");
        let mut options = OpenOptions::new();
        options.write(true).create(true).truncate(true);
        only_the_user(&mut options);
        let mut file = options.open(&draft).map_err(fail)?;
        file.write_all(text.as_bytes()).map_err(fail)?;
        file.sync_all().map_err(fail)?;
        fs::rename(&draft, path).map_err(fail)
    }

    pub(crate) fn read(path: &Path) -> Result<Self, String> {
        let text = fs::read_to_string(path)
            .map_err(|error| format!("dotori is not running ({}: {error})", path.display()))?;
        serde_json::from_str(&text).map_err(|error| format!("{}: {error}", path.display()))
    }
}

#[cfg(unix)]
fn only_the_user(options: &mut OpenOptions) {
    use std::os::unix::fs::OpenOptionsExt;
    options.mode(0o600);
}

#[cfg(not(unix))]
fn only_the_user(_options: &mut OpenOptions) {}

/// 128 bits from the operating system's random source, as hex.
pub(crate) fn token() -> Result<String, String> {
    let mut bytes = [0u8; 16];
    getrandom::fill(&mut bytes).map_err(|error| format!("no random source: {error}"))?;
    Ok(bytes.iter().map(|byte| format!("{byte:02x}")).collect())
}

#[cfg(test)]
#[path = "door.test.rs"]
mod tests;
