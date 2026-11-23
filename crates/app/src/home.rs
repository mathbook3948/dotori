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

    /// The raw record, one SQLite file per year.
    pub(crate) fn record(&self) -> PathBuf {
        self.root.join("record")
    }

    /// Earlier forms of the raw record, newest first: one SQLite file, and before that JSON Lines.
    /// Either is copied over once and then renamed.
    pub(crate) fn old_records(&self) -> [PathBuf; 2] {
        [
            self.root.join("record.sqlite3"),
            self.root.join("record.jsonl"),
        ]
    }

    /// This PC's name in the records it makes, chosen once.
    pub(crate) fn device(&self) -> PathBuf {
        self.root.join("device")
    }

    pub(crate) fn lock(&self) -> PathBuf {
        self.root.join("daemon.lock")
    }

    /// The notes the owner asked dotori to keep in mind.
    pub(crate) fn notes(&self) -> PathBuf {
        self.root.join("notes.sqlite3")
    }

    /// The keyword index over the raw record. It can always be made again from the record.
    pub(crate) fn search_index(&self) -> PathBuf {
        self.root.join("search.sqlite3")
    }

    /// Dictionaries for splitting words, fetched on first use.
    pub(crate) fn dictionaries(&self) -> PathBuf {
        self.root.join("dictionaries")
    }

    /// dotori's own Chrome for Testing, downloaded on first use.
    pub(crate) fn chrome(&self) -> PathBuf {
        self.root.join("chrome")
    }

    /// The browser profile, which keeps the owner's logins.
    pub(crate) fn browser(&self) -> PathBuf {
        self.root.join("browser")
    }

    /// The folder coding agents work in.
    pub(crate) fn work(&self) -> PathBuf {
        self.root.join("work")
    }

    /// How a chat window reaches the browser's screen. The MCP server writes it when it starts.
    pub(crate) fn screen(&self) -> PathBuf {
        self.root.join("screen.json")
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
