//! The owner's PCs, linked into one assistant. Each PC has a lasting key, and only PCs the owner
//! approved by a one-time invite can link. Everything between PCs is encrypted with Noise.

pub mod copies;
pub mod hub;
pub mod lease;
pub mod wire;

use std::fs;
use std::fs::OpenOptions;
use std::io::ErrorKind;
use std::io::Write;
use std::path::Path;
use std::time::SystemTime;
use std::time::UNIX_EPOCH;

use serde::Deserialize;
use serde::Serialize;

/// The length of a PC's public and private keys.
pub const KEY: usize = 32;
/// The length of an invite's secret.
pub const SECRET: usize = 16;
/// The port a PC listens on for the others unless its settings say otherwise.
pub const PORT: u16 = 47820;

/// A PC's lasting key pair. Its public half is how the other PCs know it.
#[derive(Clone)]
pub struct Identity {
    private: [u8; KEY],
    pub public: [u8; KEY],
}

impl std::fmt::Debug for Identity {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Identity")
            .field("public", &hex(&self.public))
            .finish_non_exhaustive()
    }
}

impl Identity {
    /// The key pair kept in `path`, made the first time. Only the user can read the file.
    pub fn load_or_make(path: &Path) -> Result<Self, String> {
        let fail = |error: &dyn std::fmt::Display| format!("{}: {error}", path.display());
        match fs::read_to_string(path) {
            Ok(text) => {
                let (private, public) = text
                    .trim()
                    .split_once(':')
                    .ok_or_else(|| fail(&"does not hold a key pair"))?;
                let read = |half: &str| {
                    unhex(half)
                        .and_then(|bytes| <[u8; KEY]>::try_from(bytes).ok())
                        .ok_or_else(|| fail(&"does not hold a key pair"))
                };
                return Ok(Self {
                    private: read(private)?,
                    public: read(public)?,
                });
            }
            Err(error) if error.kind() == ErrorKind::NotFound => {}
            Err(error) => return Err(fail(&error)),
        }
        let made = Self::generate()?;
        let text = format!("{}:{}", hex(&made.private), hex(&made.public));
        write_privately(path, text.as_bytes())?;
        Ok(made)
    }

    /// A new key pair from the operating system's random source.
    pub fn generate() -> Result<Self, String> {
        let pair = snow::Builder::new(wire::LINK.parse().map_err(|error| format!("{error}"))?)
            .generate_keypair()
            .map_err(|error| format!("cannot make a key: {error}"))?;
        let take =
            |bytes: Vec<u8>| <[u8; KEY]>::try_from(bytes).map_err(|_| "a key of the wrong length");
        Ok(Self {
            private: take(pair.private)?,
            public: take(pair.public)?,
        })
    }

    pub(crate) fn private(&self) -> &[u8; KEY] {
        &self.private
    }
}

/// Which PCs the owner approved, and which they took away. Every PC keeps a copy and the copies
/// are merged, so a later approval or removal wins.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Book {
    #[serde(default)]
    pub peers: Vec<Peer>,
    #[serde(default)]
    pub removed: Vec<Removed>,
}

/// An approved PC.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Peer {
    pub device: String,
    #[serde(default)]
    pub name: String,
    pub key: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub address: Option<String>,
    pub since: u64,
}

/// A PC the owner took away, and when.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Removed {
    pub key: String,
    pub at: u64,
}

impl Book {
    /// The book kept in `path`, or an empty one if there is none yet.
    pub fn load(path: &Path) -> Result<Self, String> {
        match fs::read_to_string(path) {
            Ok(text) => {
                serde_json::from_str(&text).map_err(|error| format!("{}: {error}", path.display()))
            }
            Err(error) if error.kind() == ErrorKind::NotFound => Ok(Self::default()),
            Err(error) => Err(format!("{}: {error}", path.display())),
        }
    }

    pub fn save(&self, path: &Path) -> Result<(), String> {
        let text = serde_json::to_string_pretty(self).map_err(|error| error.to_string())?;
        write_privately(path, text.as_bytes())
    }

    pub fn find(&self, key: &str) -> Option<&Peer> {
        self.peers.iter().find(|peer| peer.key == key)
    }

    /// Approves `peer`, unless it was removed later than it was approved. Says whether the book
    /// changed.
    pub fn approve(&mut self, peer: Peer) -> bool {
        if let Some(removed) = self
            .removed
            .iter()
            .position(|removed| removed.key == peer.key)
        {
            if self.removed[removed].at >= peer.since {
                return false;
            }
            self.removed.remove(removed);
        }
        match self.peers.iter_mut().find(|known| known.key == peer.key) {
            Some(known) if *known == peer => false,
            Some(known) if peer.since > known.since => {
                let address = peer.address.clone().or_else(|| known.address.clone());
                *known = Peer { address, ..peer };
                true
            }
            Some(known) if known.address.is_none() && peer.address.is_some() => {
                known.address = peer.address;
                true
            }
            Some(_) => false,
            None => {
                self.peers.push(peer);
                true
            }
        }
    }

    /// Takes a PC away at `at`, unless it was approved again later.
    pub fn remove(&mut self, key: &str, at: u64) -> bool {
        if self.find(key).is_some_and(|peer| peer.since > at) {
            return false;
        }
        let before = self.peers.len();
        self.peers.retain(|peer| peer.key != key);
        match self.removed.iter_mut().find(|removed| removed.key == key) {
            Some(removed) if removed.at >= at => before != self.peers.len(),
            Some(removed) => {
                removed.at = at;
                true
            }
            None => {
                self.removed.push(Removed {
                    key: key.to_owned(),
                    at,
                });
                true
            }
        }
    }

    /// Takes in what another PC's book says, leaving out this PC's own key. Says whether this
    /// book changed.
    /// Times from the future are taken as now, so no PC can make an approval that later
    /// removals cannot undo.
    pub fn merge(&mut self, other: &Book, own: &str, now: u64) -> bool {
        let mut changed = false;
        for removed in &other.removed {
            changed |= self.remove(&removed.key, removed.at.min(now));
        }
        for peer in &other.peers {
            if peer.key != own {
                changed |= self.approve(Peer {
                    since: peer.since.min(now),
                    ..peer.clone()
                });
            }
        }
        changed
    }

    /// Records where a linked PC says it can be reached now.
    pub fn moved(&mut self, key: &str, hello: &wire::Hello) -> bool {
        let Some(peer) = self.peers.iter_mut().find(|peer| peer.key == key) else {
            return false;
        };
        let address = hello.address.clone().or_else(|| peer.address.clone());
        let changed =
            peer.device != hello.device || peer.name != hello.name || peer.address != address;
        if changed {
            peer.device = hello.device.clone();
            peer.name = hello.name.clone();
            peer.address = address;
            peer.since = peer.since.max(now());
        }
        changed
    }
}

/// A one-time invite a PC hands out so that another PC can join it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Invite {
    pub secret: String,
    pub until: u64,
}

impl Invite {
    /// A new invite, good until `until`, kept in `path` for the daemon to accept.
    pub fn make(path: &Path, until: u64) -> Result<(Self, [u8; SECRET]), String> {
        let mut secret = [0u8; SECRET];
        getrandom::fill(&mut secret).map_err(|error| format!("no random source: {error}"))?;
        let invite = Self {
            secret: hex(&secret),
            until,
        };
        let text = serde_json::to_string(&invite).map_err(|error| error.to_string())?;
        write_privately(path, text.as_bytes())?;
        Ok((invite, secret))
    }

    /// The secret of the invite in `path`, if it is still good at `now`.
    pub fn open(path: &Path, now: u64) -> Option<[u8; SECRET]> {
        let text = fs::read_to_string(path).ok()?;
        let invite: Self = serde_json::from_str(&text).ok()?;
        (invite.until > now)
            .then(|| unhex(&invite.secret))
            .flatten()
            .and_then(|bytes| <[u8; SECRET]>::try_from(bytes).ok())
    }
}

/// What the owner copies from one PC to another: the inviting PC's key, the invite's secret, and
/// where the inviting PC can be reached, if it knows.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Code {
    pub key: [u8; KEY],
    pub secret: [u8; SECRET],
    pub address: Option<String>,
}

impl Code {
    pub fn write(&self) -> String {
        let mut bytes = Vec::with_capacity(KEY + SECRET);
        bytes.extend_from_slice(&self.key);
        bytes.extend_from_slice(&self.secret);
        if let Some(address) = &self.address {
            bytes.extend_from_slice(address.as_bytes());
        }
        base64url(&bytes)
    }

    pub fn read(text: &str) -> Option<Self> {
        let bytes = unbase64url(text.trim())?;
        if bytes.len() < KEY + SECRET {
            return None;
        }
        let address = String::from_utf8(bytes[KEY + SECRET..].to_vec()).ok()?;
        Some(Self {
            key: bytes[..KEY].try_into().ok()?,
            secret: bytes[KEY..KEY + SECRET].try_into().ok()?,
            address: (!address.is_empty()).then_some(address),
        })
    }
}

/// Seconds since the Unix epoch.
pub fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |since| since.as_secs())
}

pub fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

pub fn unhex(text: &str) -> Option<Vec<u8>> {
    if !text.len().is_multiple_of(2) {
        return None;
    }
    (0..text.len())
        .step_by(2)
        .map(|at| u8::from_str_radix(text.get(at..at + 2)?, 16).ok())
        .collect()
}

const URL_SAFE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";

fn base64url(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let joined = chunk.iter().enumerate().fold(0u32, |joined, (at, byte)| {
            joined | u32::from(*byte) << (16 - 8 * at)
        });
        for at in 0..=chunk.len() {
            out.push(char::from(
                URL_SAFE[(joined >> (18 - 6 * at)) as usize & 63],
            ));
        }
    }
    out
}

fn unbase64url(text: &str) -> Option<Vec<u8>> {
    let mut out = Vec::with_capacity(text.len() * 3 / 4);
    let mut bits = 0u32;
    let mut held = 0;
    for byte in text.bytes() {
        let value = URL_SAFE.iter().position(|known| *known == byte)?;
        bits = (bits << 6) | value as u32;
        held += 6;
        if held >= 8 {
            held -= 8;
            out.push((bits >> held) as u8);
            bits &= (1 << held) - 1;
        }
    }
    Some(out)
}

pub(crate) fn write_privately(path: &Path, bytes: &[u8]) -> Result<(), String> {
    let fail = |error: std::io::Error| format!("{}: {error}", path.display());
    if let Some(folder) = path.parent() {
        fs::create_dir_all(folder).map_err(fail)?;
    }
    static DRAFTS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let mut draft = path.as_os_str().to_owned();
    draft.push(format!(
        ".{}-{}.new",
        std::process::id(),
        DRAFTS.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
    ));
    let mut options = OpenOptions::new();
    options.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options.open(&draft).map_err(fail)?;
    file.write_all(bytes).map_err(fail)?;
    file.sync_all().map_err(fail)?;
    fs::rename(&draft, path).map_err(fail)
}

#[cfg(test)]
#[path = "mod.test.rs"]
mod tests;
