//! The accounts the owner linked, kept in the vault one per site origin and sealed like every
//! other secret, so they go to the linked PCs and their removal does too. The model may learn
//! which sites are linked, never an id or a password.

use std::sync::Arc;
use std::sync::Mutex;

use dotori_mcp::card::Account;
use dotori_mcp::card::Accounts;

use crate::vault::Vault;
use crate::vault::now_ms;

const PREFIX: &str = "site:";

/// The vault as the browser keeps accounts in it.
#[derive(Debug)]
pub(crate) struct VaultAccounts {
    pub(crate) vault: Arc<Vault>,
}

impl Accounts for VaultAccounts {
    fn find(&self, origin: &str) -> Result<Option<Account>, String> {
        account(&self.vault, origin)
    }

    fn keep(&self, account: &Account) -> Result<(), String> {
        let text = serde_json::to_string(account).map_err(|error| error.to_string())?;
        self.vault.set(&name(&account.origin), &text, now_ms())
    }
}

/// A linked site as the settings window shows it: its name, its origin and the id it logs in
/// with. The password never leaves the vault this way.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Site {
    pub(crate) site: String,
    pub(crate) origin: String,
    pub(crate) id: Option<String>,
}

/// Whether `name` is where the vault keeps an account rather than a key.
pub(crate) fn is_account(name: &str) -> bool {
    name.starts_with(PREFIX)
}

fn name(origin: &str) -> String {
    format!("{PREFIX}{origin}")
}

fn account(vault: &Vault, origin: &str) -> Result<Option<Account>, String> {
    let Some(text) = vault.get(&name(origin))? else {
        return Ok(None);
    };
    let account: Account = serde_json::from_str(&text)
        .map_err(|_| format!("the account kept for {origin} does not read"))?;
    Ok((account.origin == origin).then_some(account))
}

/// Every site the owner linked, by name. An account that does not open here is left out.
pub(crate) fn sites(vault: &Vault) -> Result<Vec<Site>, String> {
    let mut sites: Vec<Site> = vault
        .names()?
        .iter()
        .filter_map(|name| name.strip_prefix(PREFIX))
        .filter_map(|origin| {
            let account = account(vault, origin).ok()??;
            Some(Site {
                site: account.site,
                origin: account.origin,
                id: account.id,
            })
        })
        .collect();
    sites.sort_by(|a, b| a.site.cmp(&b.site).then_with(|| a.origin.cmp(&b.origin)));
    Ok(sites)
}

/// Unlinks the site at `origin`: its account leaves the vault here and on the linked PCs.
pub(crate) fn forget(vault: &Vault, origin: &str) -> Result<(), String> {
    vault.remove(&name(origin), now_ms())
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

    /// The sites the owner linked, by name and origin, or `None` when there are none.
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
    let lines: Vec<String> = sites
        .iter()
        .map(|site| format!("- {} ({})", one_line(&site.site), site.origin))
        .collect();
    Some(format!(
        "Sites the owner linked. dotori's vault keeps an account for each, and you never see \
         its id or password. On a page of one of these origins, log in with `fill_login`. To link \
         another site, open its login page and show the owner a card with `ask_owner_to_fill` \
         and `keep`; the owner unlinks sites in the settings window.\n{}",
        lines.join("\n")
    ))
}

fn one_line(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

#[cfg(test)]
#[path = "accounts.test.rs"]
mod tests;
