use std::fs;
use std::fs::File;
use std::fs::OpenOptions;
use std::fs::TryLockError;
use std::path::Path;

/// Held for as long as a daemon runs on a home, so a second daemon cannot share its record.
#[derive(Debug)]
pub(crate) struct DaemonLock {
    _file: File,
}

impl DaemonLock {
    pub(crate) fn take(path: &Path) -> Result<Self, String> {
        let file = open(path)?;
        match file.try_lock() {
            Ok(()) => Ok(Self { _file: file }),
            Err(TryLockError::WouldBlock) => Err(String::from("dotori is already running")),
            Err(TryLockError::Error(error)) => Err(format!("{}: {error}", path.display())),
        }
    }

    /// Whether a daemon holds the lock right now. A window checks this before it trusts the door.
    pub(crate) fn held(path: &Path) -> Result<bool, String> {
        if !path.exists() {
            return Ok(false);
        }
        let file = open(path)?;
        match file.try_lock() {
            Ok(()) => Ok(false),
            Err(TryLockError::WouldBlock) => Ok(true),
            Err(TryLockError::Error(error)) => Err(format!("{}: {error}", path.display())),
        }
    }
}

fn open(path: &Path) -> Result<File, String> {
    if let Some(folder) = path.parent() {
        fs::create_dir_all(folder).map_err(|error| format!("{}: {error}", folder.display()))?;
    }
    OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(path)
        .map_err(|error| format!("{}: {error}", path.display()))
}

#[cfg(test)]
#[path = "lock.test.rs"]
mod tests;
