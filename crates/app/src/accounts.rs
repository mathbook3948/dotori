//! The accounts the owner linked, kept in the vault and sealed like every other secret, so they
//! go to the linked PCs and their removal does too. A site may have several accounts, one per id.
//! Each is kept under `site:<origin>#<random tag>`, so nothing about the id shows in the name.
//! The model may learn which sites are linked and how many accounts each has, never an id or a
//! password.

use std::collections::BTreeMap;
use std::sync::Arc;
use std::sync::Mutex;

use dotori_mcp::card::Account;
use dotori_mcp::card::Accounts;

use crate::vault::MAX_NAME;
use crate::vault::Vault;
use crate::vault::now_ms;

const PREFIX: &str = "site:";
const TAG_BYTES: usize = 8;

/// The vault as the browser keeps accounts in it.
#[derive(Debug)]
pub(crate) struct VaultAccounts {
    pub(crate) vault: Arc<Vault>,
}

impl Accounts for VaultAccounts {
    fn find(&self, origin: &str) -> Result<Vec<Account>, String> {
        Ok(kept_for(&self.vault, origin)?
            .into_iter()
            .map(|(_, account)| account)
            .collect())
    }

    fn keep(&self, account: &Account) -> Result<(), String> {
        let same: Vec<String> = every(&self.vault)?
            .into_iter()
            .filter(|(_, kept)| kept.origin == account.origin && kept.id == account.id)
            .map(|(name, _)| name)
            .collect();
        let name = match same.first() {
            Some(name) => name.clone(),
            None => new_name(&account.origin)?,
        };
        let account = Account {
            kept_ms: now_ms(),
            ..account.clone()
        };
        let text = serde_json::to_string(&account).map_err(|error| error.to_string())?;
        self.vault.set(&name, &text, now_ms())?;
        for older in same.iter().skip(1) {
            self.vault.remove(older, now_ms())?;
        }
        Ok(())
    }

    fn room(&self, origin: &str) -> Result<(), String> {
        if PREFIX.len() + origin.len() + 1 + TAG_BYTES * 2 > MAX_NAME {
            return Err(format!(
                "the address {origin} is too long for the vault to keep an account for, so nothing was shown to the owner"
            ));
        }
        if !self.vault.has_room()? {
            return Err(String::from(
                "the vault is full, so no account can be kept; ask the owner to unlink one in the settings window first",
            ));
        }
        Ok(())
    }
}

/// A linked account as the settings window shows it: its site, origin and id, and the name it is
/// kept under. The password never leaves the vault this way.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Site {
    pub(crate) name: String,
    pub(crate) site: String,
    pub(crate) origin: String,
    pub(crate) id: Option<String>,
}

/// Whether `name` is where the vault keeps an account rather than a key.
pub(crate) fn is_account(name: &str) -> bool {
    name.starts_with(PREFIX)
}

fn origin_of(name: &str) -> Option<&str> {
    let rest = name.strip_prefix(PREFIX)?;
    Some(rest.split_once('#').map_or(rest, |(origin, _)| origin))
}

fn new_name(origin: &str) -> Result<String, String> {
    let mut tag = [0_u8; TAG_BYTES];
    getrandom::fill(&mut tag).map_err(|error| format!("no random source: {error}"))?;
    let tag: String = tag.iter().map(|byte| format!("{byte:02x}")).collect();
    Ok(format!("{PREFIX}{origin}#{tag}"))
}

fn read(vault: &Vault, name: &str) -> Option<Account> {
    let text = vault.get(name).ok()??;
    let account: Account = serde_json::from_str(&text).ok()?;
    (origin_of(name) == Some(account.origin.as_str())).then_some(account)
}

fn every(vault: &Vault) -> Result<Vec<(String, Account)>, String> {
    Ok(vault
        .names()?
        .into_iter()
        .filter(|name| is_account(name))
        .filter_map(|name| {
            let account = read(vault, &name)?;
            Some((name, account))
        })
        .collect())
}

fn kept(vault: &Vault) -> Result<Vec<(String, Account)>, String> {
    let mut latest: Vec<(String, Account)> = Vec::new();
    for (name, account) in every(vault)? {
        match latest
            .iter_mut()
            .find(|(_, held)| held.origin == account.origin && held.id == account.id)
        {
            Some(held) if held.1.kept_ms < account.kept_ms => *held = (name, account),
            Some(_) => {}
            None => latest.push((name, account)),
        }
    }
    Ok(latest)
}

fn kept_for(vault: &Vault, origin: &str) -> Result<Vec<(String, Account)>, String> {
    Ok(kept(vault)?
        .into_iter()
        .filter(|(_, account)| account.origin == origin)
        .collect())
}

/// Every account the owner linked, by site. An account that does not open here is left out.
pub(crate) fn sites(vault: &Vault) -> Result<Vec<Site>, String> {
    let mut sites: Vec<Site> = kept(vault)?
        .into_iter()
        .map(|(name, account)| Site {
            name,
            site: account.site,
            origin: account.origin,
            id: account.id,
        })
        .collect();
    sites.sort_by(|a, b| (&a.site, &a.origin, &a.id).cmp(&(&b.site, &b.origin, &b.id)));
    Ok(sites)
}

/// Unlinks the account kept under `name`: it leaves the vault here and on the linked PCs.
pub(crate) fn forget(vault: &Vault, name: &str) -> Result<(), String> {
    if !is_account(name) {
        return Err(format!("{name} is not a linked account"));
    }
    vault.remove(name, now_ms())
}

/// What the main session is told about the linked sites, kept until the vault changes.
pub(crate) struct LinkedSites {
    vault: Arc<Vault>,
    shown: Mutex<Option<(i64, Option<String>)>>,
}

impl LinkedSites {
    pub(crate) fn new(vault: Arc<Vault>) -> Self {
        Self {
            vault,
            shown: Mutex::new(None),
        }
    }

    /// The sites the owner linked, by name and origin, with how many accounts each has, or `None`
    /// when there are none.
    pub(crate) fn context(&self) -> Option<String> {
        let changes = self.vault.changes().ok()?;
        let mut shown = self
            .shown
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if let Some((at, text)) = shown.as_ref()
            && *at == changes
        {
            return text.clone();
        }
        let text = told(&sites(&self.vault).unwrap_or_default());
        *shown = Some((changes, text.clone()));
        text
    }
}

fn told(sites: &[Site]) -> Option<String> {
    if sites.is_empty() {
        return None;
    }
    let mut counted: BTreeMap<(String, &str), usize> = BTreeMap::new();
    for site in sites {
        *counted
            .entry((one_line(&site.site), site.origin.as_str()))
            .or_default() += 1;
    }
    let lines: Vec<String> = counted
        .iter()
        .map(|((site, origin), count)| format!("- {site} ({origin}), 계정 {count}개"))
        .collect();
    Some(format!(
        "Sites the owner linked. dotori's vault keeps their accounts, and you never see an id or \
         a password. On a page of one of these origins, log in with `fill_login`; when a site has \
         several accounts, the owner picks one. To link another account, open the site's login \
         page and show the owner a card with `ask_owner_to_fill` and `keep`; the owner unlinks \
         accounts in the settings window.\n{}",
        lines.join("\n")
    ))
}

fn one_line(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

#[cfg(test)]
#[path = "accounts.test.rs"]
mod tests;
