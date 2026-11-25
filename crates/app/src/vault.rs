//! Keeps the owner's API keys and tokens sealed in a file of their own, with the key that opens
//! them in the OS keychain, and carries them to the other linked PCs over the encrypted link.
//!
//! Each secret is sealed on its own with XChaCha20-Poly1305 under one 32-byte key, with its
//! name bound in so a sealed value cannot be moved under another name. The key lives in the
//! keychain (Keychain on macOS, Credential Manager on Windows, the Secret Service on Linux).
//! Where there is no keychain, such as a Linux machine without a desktop session, it lives in a
//! file next to the vault that only the owner can read.
//!
//! A sealed check value made with the key marks which key the vault belongs to. A process only
//! makes a key while it holds `vault.lock` and the vault has no check value, so two processes
//! never seal with two keys, and a key that does not open the check value is never used.

use std::fs::File;
use std::fs::OpenOptions;
use std::io::Write;
use std::path::Path;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::Mutex;
use std::time::Duration;
use std::time::SystemTime;
use std::time::UNIX_EPOCH;

use chacha20poly1305::KeyInit;
use chacha20poly1305::XChaCha20Poly1305;
use chacha20poly1305::XNonce;
use chacha20poly1305::aead::Aead;
use chacha20poly1305::aead::Payload;
use dotori_connectors::peer::hub::Heard;
use rusqlite::Connection;
use rusqlite::OptionalExtension;
use rusqlite::params;
use serde::Deserialize;
use serde::Serialize;
use serde_json::Value;
use serde_json::json;
use tokio::time;

use crate::main_session::Linked;
use crate::settings::Settings;
use crate::settings::environment;
use crate::settings::reserved;
use crate::versioned;

const VERSION: i64 = 1;
const SCHEMA: &str = "CREATE TABLE IF NOT EXISTS secrets (\
    name TEXT PRIMARY KEY, \
    sealed BLOB, \
    changed_ms INTEGER NOT NULL\
);\
CREATE TABLE IF NOT EXISTS checks (\
    name TEXT PRIMARY KEY, \
    sealed BLOB NOT NULL\
);\
CREATE TABLE IF NOT EXISTS meta (\
    key TEXT PRIMARY KEY, \
    value INTEGER NOT NULL\
);";
const CHECK: &str = "key";
const VAULT_ID: &str = "vault_id";
const KEY_PLACE: &str = "key_place";
const KEPT_IN_FILE: i64 = 1;
const KEPT_IN_KEYCHAIN: i64 = 2;
const ROOM: &str = "WHERE (SELECT COUNT(*) FROM secrets WHERE sealed IS NOT NULL) < ?4 \
    OR ?2 IS NULL OR EXISTS (SELECT 1 FROM secrets WHERE name = ?1)";
const UPSERT: &str = "ON CONFLICT (name) DO UPDATE SET sealed = excluded.sealed, \
    changed_ms = MAX(excluded.changed_ms, secrets.changed_ms + 1)";
const LOCK_WAIT: Duration = Duration::from_secs(10);
const CHECK_TEXT: &str = "dotori vault";
const KEYCHAIN_SERVICE: &str = "dotori";
const KEY_BYTES: usize = 32;
const NONCE_BYTES: usize = 24;
const MAX_NAME: usize = 128;
const MAX_VALUE: usize = 16 * 1024;
const MAX_SECRETS: i64 = 256;
const BUSY: Duration = Duration::from_secs(5);
const LOOK_EVERY: Duration = Duration::from_secs(5);
const FUTURE_TAKEN: i64 = 24 * 60 * 60 * 1000;

type Key = [u8; KEY_BYTES];

/// One secret as a PC sends it to the others: its value, or `None` once it was removed, and
/// when that happened there.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct SecretCopy {
    pub(crate) name: String,
    pub(crate) value: Option<String>,
    pub(crate) changed_ms: i64,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Make {
    Never,
    New,
    Recover,
}

enum Found {
    Key(Key),
    Gone,
    Unreadable(String),
}

/// The owner's secrets, sealed on disk.
#[derive(Debug)]
pub(crate) struct Vault {
    path: PathBuf,
    keychain: bool,
    key_file: PathBuf,
    lock_file: PathBuf,
    held: Mutex<Option<Connection>>,
    key: Mutex<Option<Key>>,
}

impl Vault {
    /// The vault of the dotori home at `root`, opened with a key from the keychain, or from
    /// `vault.key` there when the keychain cannot keep one. Tests never touch the keychain.
    pub(crate) fn new(root: &Path) -> Self {
        Self::at(root, !cfg!(test))
    }

    /// A vault whose key is kept in `vault.key` in `root` and never in the keychain, for tests.
    #[cfg(test)]
    pub(crate) fn without_keychain(root: &Path) -> Self {
        Self::at(root, false)
    }

    fn at(root: &Path, keychain: bool) -> Self {
        Self {
            path: root.join("vault.sqlite3"),
            keychain,
            key_file: root.join("vault.key"),
            lock_file: root.join("vault.lock"),
            held: Mutex::new(None),
            key: Mutex::new(None),
        }
    }

    fn with<T>(&self, work: impl FnOnce(&Connection) -> rusqlite::Result<T>) -> Result<T, String> {
        let mut held = self.held.lock().expect("unpoisoned");
        if held.is_none() {
            let connection = versioned::open(&self.path, SCHEMA, VERSION)?;
            connection
                .busy_timeout(BUSY)
                .map_err(|error| format!("{}: {error}", self.path.display()))?;
            *held = Some(connection);
        }
        let connection = held.as_ref().expect("opened");
        work(connection).map_err(|error| format!("{}: {error}", self.path.display()))
    }

    /// The value kept under `name`, if there is one.
    pub(crate) fn get(&self, name: &str) -> Result<Option<String>, String> {
        if !self.path.exists() {
            return Ok(None);
        }
        let sealed: Option<Vec<u8>> = self.with(|connection| {
            connection
                .query_row(
                    "SELECT sealed FROM secrets WHERE name = ?1",
                    [name],
                    |row| row.get(0),
                )
                .optional()
                .map(Option::flatten)
        })?;
        let Some(sealed) = sealed else {
            return Ok(None);
        };
        let key = self.key(Make::Never)?;
        open(&key, name, &sealed).map(Some)
    }

    /// The names that hold a value, never the values.
    pub(crate) fn names(&self) -> Result<Vec<String>, String> {
        if !self.path.exists() {
            return Ok(Vec::new());
        }
        self.with(|connection| {
            let mut found = connection
                .prepare("SELECT name FROM secrets WHERE sealed IS NOT NULL ORDER BY name")?;
            found
                .query_map([], |row| row.get(0))?
                .collect::<rusqlite::Result<Vec<String>>>()
        })
    }

    /// Keeps `value` under `name`, in place of what was there. When the owner gives a key on a
    /// PC that lost the key of its vault, the values that no longer open are emptied here only,
    /// so the other PCs fill them back.
    pub(crate) fn set(&self, name: &str, value: &str, now_ms: i64) -> Result<(), String> {
        let name = checked_name(name)?;
        let value = value.trim();
        if value.is_empty() {
            return Err(String::from("an empty value is not kept"));
        }
        if value.len() > MAX_VALUE {
            return Err(format!("a value is at most {MAX_VALUE} bytes"));
        }
        let key = self.key(Make::Recover)?;
        let sealed = seal(&key, name, value)?;
        self.write(name, Some(&sealed), now_ms)
    }

    /// Removes what is kept under `name`. The other PCs remove it too.
    pub(crate) fn remove(&self, name: &str, now_ms: i64) -> Result<(), String> {
        let name = checked_name(name)?;
        self.write(name, None, now_ms)
    }

    fn write(&self, name: &str, sealed: Option<&[u8]>, now_ms: i64) -> Result<(), String> {
        let written = self.with(|connection| {
            immediate(connection, |connection| {
                let written = connection.execute(
                    &format!("INSERT INTO secrets (name, sealed, changed_ms) SELECT ?1, ?2, ?3 {ROOM} {UPSERT}"),
                    params![name, sealed, now_ms, MAX_SECRETS],
                )?;
                if written > 0 {
                    bump(connection)?;
                }
                Ok(written)
            })
        })?;
        if written == 0 {
            return Err(format!("the vault keeps at most {MAX_SECRETS} names"));
        }
        Ok(())
    }

    /// A number that changes whenever anything here changes.
    pub(crate) fn changes(&self) -> Result<i64, String> {
        if !self.path.exists() {
            return Ok(0);
        }
        self.with(|connection| {
            Ok(connection
                .query_row("SELECT value FROM meta WHERE key = 'changes'", [], |row| {
                    row.get(0)
                })
                .optional()?
                .unwrap_or(0))
        })
    }

    /// Every secret opened, and every removal, for the other PCs. Values this PC cannot open
    /// are left out.
    pub(crate) fn copy(&self) -> Result<Vec<SecretCopy>, String> {
        if !self.path.exists() {
            return Ok(Vec::new());
        }
        let rows: Vec<(String, Option<Vec<u8>>, i64)> = self.with(|connection| {
            let mut found = connection.prepare(
                "SELECT name, sealed, changed_ms FROM secrets WHERE changed_ms > 0 ORDER BY name",
            )?;
            found
                .query_map([], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))?
                .collect()
        })?;
        let key = if rows.iter().any(|(_, sealed, _)| sealed.is_some()) {
            Some(self.key(Make::Never)?)
        } else {
            None
        };
        Ok(rows
            .into_iter()
            .filter_map(|(name, sealed, changed_ms)| {
                let value = match (sealed, &key) {
                    (Some(sealed), Some(key)) => Some(open(key, &name, &sealed).ok()?),
                    _ => None,
                };
                Some(SecretCopy {
                    name,
                    value,
                    changed_ms,
                })
            })
            .collect())
    }

    /// Takes each secret another PC changed later than this one. A secret dated more than a day
    /// ahead of this PC's clock is left out. Says whether any was taken.
    pub(crate) fn take(&self, copies: &[SecretCopy], now_ms: i64) -> Result<bool, String> {
        let latest = now_ms.saturating_add(FUTURE_TAKEN);
        let wanted: Vec<(&str, &SecretCopy)> = copies
            .iter()
            .filter(|copy| copy.changed_ms > 0 && copy.changed_ms <= latest)
            .filter(|copy| {
                copy.value
                    .as_ref()
                    .is_none_or(|value| !value.trim().is_empty() && value.len() <= MAX_VALUE)
            })
            .filter_map(|copy| Some((checked_name(&copy.name).ok()?, copy)))
            .take(usize::try_from(MAX_SECRETS).unwrap_or(usize::MAX))
            .collect();
        if wanted.is_empty() {
            return Ok(false);
        }
        let key = if wanted.iter().any(|(_, copy)| copy.value.is_some()) {
            Some(self.key(Make::New)?)
        } else {
            None
        };
        let sealed = wanted
            .iter()
            .map(|(name, copy)| match (&copy.value, &key) {
                (Some(value), Some(key)) => seal(key, name, value.trim()).map(Some),
                _ => Ok(None),
            })
            .collect::<Result<Vec<Option<Vec<u8>>>, String>>()?;
        self.with(|connection| {
            immediate(connection, |connection| {
                let mut taken = 0;
                for ((name, copy), sealed) in wanted.iter().zip(&sealed) {
                    taken += connection.execute(
                        &format!(
                            "INSERT INTO secrets (name, sealed, changed_ms) SELECT ?1, ?2, ?3 {ROOM} \
                             ON CONFLICT (name) DO UPDATE SET sealed = excluded.sealed, \
                             changed_ms = excluded.changed_ms \
                             WHERE excluded.changed_ms > secrets.changed_ms"
                        ),
                        params![name, sealed, copy.changed_ms, MAX_SECRETS],
                    )?;
                }
                if taken > 0 {
                    bump(connection)?;
                }
                Ok(taken > 0)
            })
        })
    }

    fn key(&self, make: Make) -> Result<Key, String> {
        let mut held = self.key.lock().expect("unpoisoned");
        if let Some(key) = *held {
            if self.confirmed(&key)? {
                return Ok(key);
            }
            *held = None;
        }
        let key = match self.find_key()? {
            Found::Key(key) if self.confirmed(&key)? => key,
            _ => {
                let lock = self.lock()?;
                let found = self.key_locked(make);
                drop(lock);
                found?
            }
        };
        *held = Some(key);
        Ok(key)
    }

    fn key_locked(&self, make: Make) -> Result<Key, String> {
        let check = self.check()?;
        match (self.find_key()?, &check, make) {
            (Found::Key(key), Some(_), _) if self.confirmed(&key)? => Ok(key),
            (Found::Key(_), Some(_), _) => Err(String::from(
                "the vault key here does not open this vault; it was made with another key",
            )),
            (Found::Key(key), None, _) => {
                self.mark(&key)?;
                Ok(key)
            }
            (Found::Unreadable(error), Some(_), _) => Err(error),
            (Found::Gone | Found::Unreadable(_), _, Make::Never)
            | (Found::Gone, Some(_), Make::New) => Err(String::from(
                "the key that opens the vault is gone; give the keys again in the settings window",
            )),
            (Found::Gone, Some(_), Make::Recover) => {
                self.forget_unreadable()?;
                self.make_key()
            }
            (Found::Gone | Found::Unreadable(_), None, Make::New | Make::Recover) => {
                self.make_key()
            }
        }
    }

    fn find_key(&self) -> Result<Found, String> {
        match read_key_file(&self.key_file) {
            Ok(Some(key)) => return Ok(Found::Key(key)),
            Ok(None) => {}
            Err(error) => return Ok(Found::Unreadable(error)),
        }
        let (Some(id), Some(KEPT_IN_KEYCHAIN)) = (self.meta(VAULT_ID)?, self.meta(KEY_PLACE)?)
        else {
            return Ok(Found::Gone);
        };
        if !self.keychain {
            return Ok(Found::Gone);
        }
        Ok(read_keychain(&account(id)))
    }

    fn meta(&self, key: &str) -> Result<Option<i64>, String> {
        self.with(|connection| {
            connection
                .query_row("SELECT value FROM meta WHERE key = ?1", [key], |row| {
                    row.get(0)
                })
                .optional()
        })
    }

    fn check(&self) -> Result<Option<Vec<u8>>, String> {
        self.with(|connection| {
            connection
                .query_row(
                    "SELECT sealed FROM checks WHERE name = ?1",
                    [CHECK],
                    |row| row.get(0),
                )
                .optional()
        })
    }

    fn confirmed(&self, key: &Key) -> Result<bool, String> {
        Ok(self
            .check()?
            .is_some_and(|sealed| open(key, CHECK, &sealed).is_ok_and(|text| text == CHECK_TEXT)))
    }

    fn make_key(&self) -> Result<Key, String> {
        let mut key = [0_u8; KEY_BYTES];
        getrandom::fill(&mut key).map_err(|error| format!("no random source: {error}"))?;
        let mut id = [0_u8; 8];
        getrandom::fill(&mut id).map_err(|error| format!("no random source: {error}"))?;
        let id = i64::from_le_bytes(id) & i64::MAX;
        let place = if self.keychain && write_keychain(&account(id), &key) {
            KEPT_IN_KEYCHAIN
        } else {
            write_key_file(&self.key_file, &key)?;
            KEPT_IN_FILE
        };
        self.with(|connection| {
            immediate(connection, |connection| {
                for (key, value) in [(VAULT_ID, id), (KEY_PLACE, place)] {
                    connection.execute(
                        "INSERT OR REPLACE INTO meta (key, value) VALUES (?1, ?2)",
                        params![key, value],
                    )?;
                }
                Ok(())
            })
        })?;
        self.mark(&key)?;
        Ok(key)
    }

    fn mark(&self, key: &Key) -> Result<(), String> {
        let sealed = seal(key, CHECK, CHECK_TEXT)?;
        self.with(|connection| {
            connection.execute(
                "INSERT OR IGNORE INTO checks (name, sealed) VALUES (?1, ?2)",
                params![CHECK, sealed],
            )?;
            Ok(())
        })
    }

    fn forget_unreadable(&self) -> Result<(), String> {
        self.with(|connection| {
            immediate(connection, |connection| {
                connection.execute(
                    "UPDATE secrets SET sealed = NULL, changed_ms = 0 WHERE sealed IS NOT NULL",
                    [],
                )?;
                connection.execute("DELETE FROM checks", [])?;
                bump(connection)
            })
        })
    }

    fn lock(&self) -> Result<File, String> {
        if let Some(folder) = self.lock_file.parent() {
            std::fs::create_dir_all(folder).map_err(|error| error.to_string())?;
        }
        let fail = |error: std::io::Error| format!("{}: {error}", self.lock_file.display());
        let file = OpenOptions::new()
            .create(true)
            .truncate(false)
            .write(true)
            .open(&self.lock_file)
            .map_err(fail)?;
        let waited = std::time::Instant::now();
        loop {
            match file.try_lock() {
                Ok(()) => return Ok(file),
                Err(std::fs::TryLockError::WouldBlock) if waited.elapsed() < LOCK_WAIT => {
                    std::thread::sleep(Duration::from_millis(50));
                }
                Err(std::fs::TryLockError::WouldBlock) => {
                    return Err(format!(
                        "{} stays locked by another dotori",
                        self.lock_file.display()
                    ));
                }
                Err(std::fs::TryLockError::Error(error)) => return Err(fail(error)),
            }
        }
    }
}

fn immediate<T>(
    connection: &Connection,
    work: impl FnOnce(&Connection) -> rusqlite::Result<T>,
) -> rusqlite::Result<T> {
    connection.execute_batch("BEGIN IMMEDIATE")?;
    match work(connection) {
        Ok(done) => {
            connection.execute_batch("COMMIT")?;
            Ok(done)
        }
        Err(error) => {
            let _ = connection.execute_batch("ROLLBACK");
            Err(error)
        }
    }
}

fn account(id: i64) -> String {
    format!("vault {id}")
}

fn bump(connection: &Connection) -> rusqlite::Result<()> {
    connection.execute(
        "INSERT INTO meta (key, value) VALUES ('changes', 1) \
         ON CONFLICT (key) DO UPDATE SET value = meta.value + 1",
        [],
    )?;
    Ok(())
}

fn checked_name(name: &str) -> Result<&str, String> {
    let name = name.trim();
    if name.is_empty() || name.len() > MAX_NAME || name.chars().any(char::is_control) {
        return Err(format!("`{name}` is not a name a key is kept under"));
    }
    Ok(name)
}

fn seal(key: &Key, name: &str, value: &str) -> Result<Vec<u8>, String> {
    let mut nonce = [0_u8; NONCE_BYTES];
    getrandom::fill(&mut nonce).map_err(|error| format!("no random source: {error}"))?;
    let cipher = XChaCha20Poly1305::new(key.into());
    let sealed = cipher
        .encrypt(
            XNonce::from_slice(&nonce),
            Payload {
                msg: value.as_bytes(),
                aad: name.as_bytes(),
            },
        )
        .map_err(|_| String::from("the value could not be sealed"))?;
    Ok([nonce.as_slice(), &sealed].concat())
}

fn open(key: &Key, name: &str, sealed: &[u8]) -> Result<String, String> {
    if sealed.len() < NONCE_BYTES {
        return Err(format!("the value kept under {name} is cut short"));
    }
    let (nonce, body) = sealed.split_at(NONCE_BYTES);
    let cipher = XChaCha20Poly1305::new(key.into());
    let opened = cipher
        .decrypt(
            XNonce::from_slice(nonce),
            Payload {
                msg: body,
                aad: name.as_bytes(),
            },
        )
        .map_err(|_| format!("the value kept under {name} does not open with this vault's key"))?;
    String::from_utf8(opened).map_err(|_| format!("the value kept under {name} is not text"))
}

fn read_key_file(file: &Path) -> Result<Option<Key>, String> {
    match std::fs::read(file) {
        Ok(bytes) => Key::try_from(bytes.as_slice())
            .map(Some)
            .map_err(|_| format!("{} is not a vault key", file.display())),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(format!("{}: {error}", file.display())),
    }
}

fn write_key_file(file: &Path, key: &Key) -> Result<(), String> {
    if let Some(folder) = file.parent() {
        std::fs::create_dir_all(folder).map_err(|error| error.to_string())?;
    }
    let draft = file.with_extension("key.new");
    let fail = |error: std::io::Error| format!("{}: {error}", file.display());
    let _ = std::fs::remove_file(&draft);
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    std::os::unix::fs::OpenOptionsExt::mode(&mut options, 0o600);
    let mut out = options.open(&draft).map_err(fail)?;
    out.write_all(key)
        .and_then(|()| out.sync_all())
        .map_err(fail)?;
    drop(out);
    match std::fs::hard_link(&draft, file) {
        Ok(()) => {
            let _ = std::fs::remove_file(&draft);
            Ok(())
        }
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
            let _ = std::fs::remove_file(&draft);
            Err(fail(error))
        }
        Err(_) => std::fs::rename(&draft, file).map_err(fail),
    }
}

fn read_keychain(account: &str) -> Found {
    let entry = match keyring::Entry::new(KEYCHAIN_SERVICE, account) {
        Ok(entry) => entry,
        Err(error) => return Found::Unreadable(format!("the keychain cannot be used: {error}")),
    };
    match entry.get_secret() {
        Ok(bytes) => Key::try_from(bytes.as_slice()).map_or_else(
            |_| {
                Found::Unreadable(String::from(
                    "the vault key in the keychain is not a vault key",
                ))
            },
            Found::Key,
        ),
        Err(keyring::Error::NoEntry) => Found::Gone,
        Err(error) => {
            Found::Unreadable(format!("the keychain did not give the vault key: {error}"))
        }
    }
}

fn write_keychain(account: &str, key: &Key) -> bool {
    let Ok(entry) = keyring::Entry::new(KEYCHAIN_SERVICE, account) else {
        return false;
    };
    entry.set_secret(key).is_ok() && entry.get_secret().is_ok_and(|kept| kept == key)
}

/// Every name a secret goes by here: those the settings read and those the vault keeps. Programs
/// dotori starts for the model, and MCP servers, do not inherit variables of these names.
pub(crate) fn secret_names(settings: &Settings, vault: &Vault) -> Vec<String> {
    let mut names = settings.secret_names();
    match vault.names() {
        Ok(kept) => names.extend(kept.into_iter().filter(|name| !reserved(name))),
        Err(error) => eprintln!("dotori: the vault's names cannot be read: {error}"),
    }
    names.sort();
    names.dedup();
    names
}

/// Reads a key or token by the name it goes by: from the environment variable of that name when
/// one is set, which wins, or else from the vault.
pub(crate) fn lookup(vault: &Vault) -> impl Fn(&str) -> Option<String> + '_ {
    move |name| {
        environment(name).or_else(|| match vault.get(name) {
            Ok(value) => value,
            Err(error) => {
                eprintln!("dotori: {name} could not be read from the vault: {error}");
                None
            }
        })
    }
}

/// Takes a message about the vault from another PC. Says whether it was one. The work that may
/// wait on the keychain runs off the message loop.
pub(crate) fn hear(vault: &Arc<Vault>, linked: &Linked, heard: &Heard) -> bool {
    match heard.message.get("kind").and_then(Value::as_str) {
        Some("linked") => {
            let vault = Arc::clone(vault);
            let hub = linked.hub.clone();
            let to = heard.key.clone();
            tokio::task::spawn_blocking(move || match vault.copy() {
                Ok(copies) if !copies.is_empty() => {
                    hub.send(&to, message(&copies));
                }
                Ok(_) => {}
                Err(error) => eprintln!("dotori: the vault was not shared: {error}"),
            });
            false
        }
        Some("vault") => {
            let Some(copies) = heard.message.get("secrets").and_then(|secrets| {
                serde_json::from_value::<Vec<SecretCopy>>(secrets.clone()).ok()
            }) else {
                eprintln!(
                    "dotori: the vault from {} does not read here; is it a newer dotori?",
                    heard.device
                );
                return true;
            };
            let vault = Arc::clone(vault);
            let device = heard.device.clone();
            tokio::task::spawn_blocking(move || match vault.take(&copies, now_ms()) {
                Ok(true) => eprintln!(
                    "dotori: took newer keys from {device}; they take effect when dotori starts again"
                ),
                Ok(false) => {}
                Err(error) => eprintln!("dotori: the keys from {device} were not taken: {error}"),
            });
            true
        }
        _ => false,
    }
}

fn message(copies: &[SecretCopy]) -> Value {
    json!({ "kind": "vault", "secrets": copies })
}

/// Looks at the vault every few seconds and sends it to the other PCs when it changed here.
pub(crate) async fn share(vault: Arc<Vault>, linked: Linked) {
    let mut sent = vault.changes().ok();
    let mut every = time::interval(LOOK_EVERY);
    loop {
        every.tick().await;
        let Ok(changes) = vault.changes() else {
            continue;
        };
        if sent == Some(changes) {
            continue;
        }
        let copied = {
            let vault = Arc::clone(&vault);
            tokio::task::spawn_blocking(move || vault.copy()).await
        };
        match copied {
            Ok(Ok(copies)) => {
                linked.hub.broadcast(&message(&copies));
                sent = Some(changes);
            }
            Ok(Err(error)) => eprintln!("dotori: the vault was not shared: {error}"),
            Err(error) => eprintln!("dotori: the vault was not shared: {error}"),
        }
    }
}

/// The time now, in milliseconds since the Unix epoch.
pub(crate) fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|since| i64::try_from(since.as_millis()).unwrap_or(i64::MAX))
        .unwrap_or(0)
}

#[cfg(test)]
#[path = "vault.test.rs"]
mod tests;
