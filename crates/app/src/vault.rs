//! Keeps the owner's API keys and tokens sealed in a file of their own, with the key that opens
//! them in the OS keychain, and carries them to the other linked PCs over the encrypted link.
//!
//! Each secret is sealed on its own with XChaCha20-Poly1305 under one 32-byte key, with its
//! name bound in so a sealed value cannot be moved under another name. The key lives in the
//! keychain (Keychain on macOS, Credential Manager on Windows, the Secret Service on Linux).
//! Where there is no keychain, such as a Linux machine without a desktop session, it lives in a
//! file next to the vault that only the owner can read.

use std::io::Write;
use std::path::Path;
use std::path::PathBuf;
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
use crate::settings::environment;
use crate::versioned;

const VERSION: i64 = 1;
const SCHEMA: &str = "CREATE TABLE IF NOT EXISTS secrets (\
    name TEXT PRIMARY KEY, \
    sealed BLOB, \
    changed_ms INTEGER NOT NULL\
);";
const KEYCHAIN_SERVICE: &str = "dotori";
const KEY_BYTES: usize = 32;
const NONCE_BYTES: usize = 24;
const MAX_NAME: usize = 128;
const MAX_VALUE: usize = 16 * 1024;
const MAX_SECRETS: usize = 256;
const LOOK_EVERY: Duration = Duration::from_secs(5);
const FUTURE_TAKEN: i64 = 24 * 60 * 60 * 1000;

/// One secret as a PC sends it to the others: its value, or `None` once it was removed, and
/// when that happened there.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct SecretCopy {
    pub(crate) name: String,
    pub(crate) value: Option<String>,
    pub(crate) changed_ms: i64,
}

/// The owner's secrets, sealed on disk.
#[derive(Debug)]
pub(crate) struct Vault {
    path: PathBuf,
    keychain: Option<String>,
    key_file: PathBuf,
    held: Mutex<Option<Connection>>,
    key: Mutex<Option<[u8; KEY_BYTES]>>,
}

impl Vault {
    /// The vault of the dotori home at `root`, opened with a key from the keychain, or from
    /// `vault.key` there when the keychain cannot keep one. Tests never touch the keychain.
    pub(crate) fn new(root: &Path) -> Self {
        Self::at(
            root.join("vault.sqlite3"),
            (!cfg!(test)).then(|| format!("vault {}", root.display())),
            root.join("vault.key"),
        )
    }

    /// A vault whose key is kept in `key` and never in the keychain, for tests.
    #[cfg(test)]
    pub(crate) fn with_key_file(path: PathBuf, key: PathBuf) -> Self {
        Self::at(path, None, key)
    }

    fn at(path: PathBuf, keychain: Option<String>, key_file: PathBuf) -> Self {
        Self {
            path,
            keychain,
            key_file,
            held: Mutex::new(None),
            key: Mutex::new(None),
        }
    }

    fn with<T>(&self, work: impl FnOnce(&Connection) -> rusqlite::Result<T>) -> Result<T, String> {
        let mut held = self.held.lock().expect("unpoisoned");
        if held.is_none() {
            *held = Some(versioned::open(&self.path, SCHEMA, VERSION)?);
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
        let key = self.key(false)?;
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

    /// Keeps `value` under `name`, in place of what was there.
    pub(crate) fn set(&self, name: &str, value: &str, now_ms: i64) -> Result<(), String> {
        let name = checked_name(name)?;
        let value = value.trim();
        if value.is_empty() {
            return Err(String::from("an empty value is not kept"));
        }
        if value.len() > MAX_VALUE {
            return Err(format!("a value is at most {MAX_VALUE} bytes"));
        }
        let key = self.key(true)?;
        let sealed = seal(&key, name, value)?;
        self.write(name, Some(&sealed), now_ms)
    }

    /// Removes what is kept under `name`. The other PCs remove it too.
    pub(crate) fn remove(&self, name: &str, now_ms: i64) -> Result<(), String> {
        let name = checked_name(name)?;
        self.write(name, None, now_ms)
    }

    fn write(&self, name: &str, sealed: Option<&[u8]>, now_ms: i64) -> Result<(), String> {
        self.with(|connection| {
            let before: Option<i64> = connection
                .query_row(
                    "SELECT changed_ms FROM secrets WHERE name = ?1",
                    [name],
                    |row| row.get(0),
                )
                .optional()?;
            let changed_ms = before.map_or(now_ms, |before| now_ms.max(before.saturating_add(1)));
            connection.execute(
                "INSERT OR REPLACE INTO secrets (name, sealed, changed_ms) VALUES (?1, ?2, ?3)",
                params![name, sealed, changed_ms],
            )?;
            Ok(())
        })
    }

    /// When anything here last changed, in milliseconds since the Unix epoch.
    pub(crate) fn changed_ms(&self) -> Result<i64, String> {
        if !self.path.exists() {
            return Ok(0);
        }
        self.with(|connection| {
            connection.query_row(
                "SELECT COALESCE(MAX(changed_ms), 0) FROM secrets",
                [],
                |row| row.get(0),
            )
        })
    }

    /// Every secret opened, and every removal, for the other PCs.
    pub(crate) fn copy(&self) -> Result<Vec<SecretCopy>, String> {
        if !self.path.exists() {
            return Ok(Vec::new());
        }
        let rows: Vec<(String, Option<Vec<u8>>, i64)> = self.with(|connection| {
            let mut found =
                connection.prepare("SELECT name, sealed, changed_ms FROM secrets ORDER BY name")?;
            found
                .query_map([], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))?
                .collect()
        })?;
        if rows.iter().all(|(_, sealed, _)| sealed.is_none()) {
            return Ok(rows
                .into_iter()
                .map(|(name, _, changed_ms)| SecretCopy {
                    name,
                    value: None,
                    changed_ms,
                })
                .collect());
        }
        let key = self.key(false)?;
        rows.into_iter()
            .map(|(name, sealed, changed_ms)| {
                let value = sealed
                    .map(|sealed| open(&key, &name, &sealed))
                    .transpose()?;
                Ok(SecretCopy {
                    name,
                    value,
                    changed_ms,
                })
            })
            .collect()
    }

    /// Takes each secret another PC changed later than this one. Says whether any was taken.
    pub(crate) fn take(&self, copies: &[SecretCopy]) -> Result<bool, String> {
        let here: Vec<(String, i64)> = if self.path.exists() {
            self.with(|connection| {
                let mut found = connection.prepare("SELECT name, changed_ms FROM secrets")?;
                found
                    .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))?
                    .collect()
            })?
        } else {
            Vec::new()
        };
        let newer: Vec<&SecretCopy> = copies
            .iter()
            .take(MAX_SECRETS)
            .filter(|copy| checked_name(&copy.name).is_ok())
            .filter(|copy| {
                copy.value
                    .as_ref()
                    .is_none_or(|value| !value.trim().is_empty() && value.len() <= MAX_VALUE)
            })
            .filter(|copy| {
                here.iter()
                    .find(|(name, _)| *name == copy.name)
                    .is_none_or(|(_, changed_ms)| copy.changed_ms > *changed_ms)
            })
            .collect();
        if newer.is_empty() {
            return Ok(false);
        }
        let key = if newer.iter().any(|copy| copy.value.is_some()) {
            Some(self.key(true)?)
        } else {
            None
        };
        let sealed = newer
            .iter()
            .map(|copy| match (&copy.value, &key) {
                (Some(value), Some(key)) => seal(key, &copy.name, value.trim()).map(Some),
                _ => Ok(None),
            })
            .collect::<Result<Vec<Option<Vec<u8>>>, String>>()?;
        self.with(|connection| {
            let all = connection.unchecked_transaction()?;
            for (copy, sealed) in newer.iter().zip(&sealed) {
                all.execute(
                    "INSERT OR REPLACE INTO secrets (name, sealed, changed_ms) VALUES (?1, ?2, ?3)",
                    params![copy.name, sealed, copy.changed_ms],
                )?;
            }
            all.commit()
        })?;
        Ok(true)
    }

    fn sealed_any(&self) -> Result<bool, String> {
        if !self.path.exists() {
            return Ok(false);
        }
        self.with(|connection| {
            connection.query_row(
                "SELECT EXISTS (SELECT 1 FROM secrets WHERE sealed IS NOT NULL)",
                [],
                |row| row.get(0),
            )
        })
    }

    fn key(&self, create: bool) -> Result<[u8; KEY_BYTES], String> {
        let mut held = self.key.lock().expect("unpoisoned");
        if let Some(key) = *held {
            return Ok(key);
        }
        let found = match (read_key_file(&self.key_file), &self.keychain) {
            (Ok(None), Some(account)) => read_keychain(account),
            (kept, _) => kept,
        };
        let key = match found {
            Ok(Some(key)) => key,
            Ok(None) | Err(_) if create && !self.sealed_any()? => self.make_key()?,
            Ok(None) => {
                return Err(String::from(
                    "the key that opens the vault is gone, so the keys kept in it cannot be read; set them again in the settings window",
                ));
            }
            Err(error) => return Err(error),
        };
        *held = Some(key);
        Ok(key)
    }

    fn make_key(&self) -> Result<[u8; KEY_BYTES], String> {
        let mut key = [0_u8; KEY_BYTES];
        getrandom::fill(&mut key).map_err(|error| format!("no random source: {error}"))?;
        let in_keychain = self
            .keychain
            .as_deref()
            .is_some_and(|account| write_keychain(account, &key));
        if !in_keychain {
            write_key_file(&self.key_file, &key)?;
        }
        Ok(key)
    }
}

fn checked_name(name: &str) -> Result<&str, String> {
    let name = name.trim();
    if name.is_empty() || name.len() > MAX_NAME || name.chars().any(char::is_control) {
        return Err(format!("`{name}` is not a name a key is kept under"));
    }
    Ok(name)
}

fn seal(key: &[u8; KEY_BYTES], name: &str, value: &str) -> Result<Vec<u8>, String> {
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

fn open(key: &[u8; KEY_BYTES], name: &str, sealed: &[u8]) -> Result<String, String> {
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

fn read_key_file(file: &Path) -> Result<Option<[u8; KEY_BYTES]>, String> {
    match std::fs::read(file) {
        Ok(bytes) => <[u8; KEY_BYTES]>::try_from(bytes.as_slice())
            .map(Some)
            .map_err(|_| format!("{} is not a vault key", file.display())),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(format!("{}: {error}", file.display())),
    }
}

fn write_key_file(file: &Path, key: &[u8; KEY_BYTES]) -> Result<(), String> {
    if let Some(folder) = file.parent() {
        std::fs::create_dir_all(folder).map_err(|error| error.to_string())?;
    }
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    std::os::unix::fs::OpenOptionsExt::mode(&mut options, 0o600);
    let fail = |error: std::io::Error| format!("{}: {error}", file.display());
    let mut out = options.open(file).map_err(fail)?;
    out.write_all(key)
        .and_then(|()| out.sync_all())
        .map_err(fail)
}

fn read_keychain(account: &str) -> Result<Option<[u8; KEY_BYTES]>, String> {
    let Ok(entry) = keyring::Entry::new(KEYCHAIN_SERVICE, account) else {
        return Ok(None);
    };
    match entry.get_secret() {
        Ok(bytes) => <[u8; KEY_BYTES]>::try_from(bytes.as_slice())
            .map(Some)
            .map_err(|_| String::from("the vault key in the keychain is not a vault key")),
        Err(keyring::Error::NoEntry) => Ok(None),
        Err(error) => Err(format!("the keychain did not give the vault key: {error}")),
    }
}

fn write_keychain(account: &str, key: &[u8; KEY_BYTES]) -> bool {
    let Ok(entry) = keyring::Entry::new(KEYCHAIN_SERVICE, account) else {
        return false;
    };
    entry.set_secret(key).is_ok() && entry.get_secret().is_ok_and(|kept| kept == key)
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

/// Takes a message about the vault from another PC. Says whether it was one.
pub(crate) fn hear(vault: &Vault, linked: &Linked, heard: &Heard) -> bool {
    match heard.message.get("kind").and_then(Value::as_str) {
        Some("linked") => {
            match vault.copy() {
                Ok(copies) if !copies.is_empty() => {
                    linked.hub.send(&heard.key, message(&copies));
                }
                Ok(_) => {}
                Err(error) => eprintln!("dotori: the vault was not shared: {error}"),
            }
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
            let latest = now_ms().saturating_add(FUTURE_TAKEN);
            if copies.iter().any(|copy| copy.changed_ms > latest) {
                eprintln!(
                    "dotori: the vault from {} is dated too far ahead",
                    heard.device
                );
                return true;
            }
            match vault.take(&copies) {
                Ok(true) => eprintln!(
                    "dotori: took newer keys from {}; they take effect when dotori starts again",
                    heard.device
                ),
                Ok(false) => {}
                Err(error) => eprintln!(
                    "dotori: the keys from {} were not taken: {error}",
                    heard.device
                ),
            }
            true
        }
        _ => false,
    }
}

fn message(copies: &[SecretCopy]) -> Value {
    json!({ "kind": "vault", "secrets": copies })
}

/// Looks at the vault every few seconds and sends it to the other PCs when it changed here.
pub(crate) async fn share(vault: &Vault, linked: Linked) {
    let mut sent = vault.changed_ms().ok();
    let mut every = time::interval(LOOK_EVERY);
    loop {
        every.tick().await;
        let Ok(changed) = vault.changed_ms() else {
            continue;
        };
        if sent == Some(changed) {
            continue;
        }
        match vault.copy() {
            Ok(copies) => {
                linked.hub.broadcast(&message(&copies));
                sent = Some(changed);
            }
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
