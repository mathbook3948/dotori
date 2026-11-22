use std::env;
use std::path::PathBuf;

/// Overrides the folder that holds every dotori file, such as for a second instance or tests.
pub(crate) const HOME_VARIABLE: &str = "DOTORI_HOME";

/// The folder that holds the settings, the raw record, and how windows reach the daemon.
#[derive(Clone, Debug)]
pub(crate) struct Home {
    pub(crate) root: PathBuf,
}

impl Home {
    pub(crate) fn find() -> Result<Self, String> {
        if let Some(root) = env::var_os(HOME_VARIABLE) {
            return Ok(Self { root: root.into() });
        }
        platform_root()
            .map(|base| Self {
                root: base.join("dotori"),
            })
            .ok_or_else(|| format!("cannot find a home folder; set {HOME_VARIABLE}"))
    }

    pub(crate) fn settings(&self) -> PathBuf {
        self.root.join("settings.toml")
    }

    pub(crate) fn record(&self) -> PathBuf {
        self.root.join("record.jsonl")
    }

    pub(crate) fn lock(&self) -> PathBuf {
        self.root.join("daemon.lock")
    }

    /// The browser profile, which keeps the owner's logins.
    pub(crate) fn browser(&self) -> PathBuf {
        self.root.join("browser")
    }

    pub(crate) fn door(&self) -> PathBuf {
        self.root.join("door.json")
    }
}

/// Whether a screen is there to show windows on. A Linux machine with no display has none.
pub(crate) fn desktop() -> bool {
    let set = |name: &str| env::var_os(name).is_some_and(|value| !value.is_empty());
    !cfg!(target_os = "linux") || set("DISPLAY") || set("WAYLAND_DISPLAY")
}

#[cfg(target_os = "windows")]
fn platform_root() -> Option<PathBuf> {
    env::var_os("APPDATA").map(PathBuf::from)
}

#[cfg(target_os = "macos")]
fn platform_root() -> Option<PathBuf> {
    env::var_os("HOME").map(|home| {
        PathBuf::from(home)
            .join("Library")
            .join("Application Support")
    })
}

#[cfg(not(any(target_os = "windows", target_os = "macos")))]
fn platform_root() -> Option<PathBuf> {
    env::var_os("XDG_DATA_HOME").map(PathBuf::from).or_else(|| {
        env::var_os("HOME").map(|home| PathBuf::from(home).join(".local").join("share"))
    })
}
