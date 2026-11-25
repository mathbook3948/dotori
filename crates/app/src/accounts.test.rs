use std::path::Path;
use std::path::PathBuf;
use std::sync::atomic::AtomicUsize;
use std::sync::atomic::Ordering;

use super::*;

static NEXT: AtomicUsize = AtomicUsize::new(0);

fn folder() -> PathBuf {
    let n = NEXT.fetch_add(1, Ordering::SeqCst);
    let folder = std::env::temp_dir().join(format!("dotori-accounts-{}-{n}", std::process::id()));
    let _ = std::fs::remove_dir_all(&folder);
    folder
}

fn accounts(folder: &Path) -> VaultAccounts {
    VaultAccounts {
        vault: Arc::new(Vault::without_keychain(folder)),
    }
}

fn plain(found: Vec<Account>) -> Vec<Account> {
    found
        .into_iter()
        .map(|account| Account {
            kept_ms: 0,
            ..account
        })
        .collect()
}

fn coupang() -> Account {
    Account {
        site: String::from("쿠팡"),
        origin: String::from("https://www.coupang.com"),
        id_label: Some(String::from("아이디(이메일)")),
        id: Some(String::from("minji.example@mail.com")),
        password_label: String::from("비밀번호"),
        password: String::from("hunter22!"),
        trusted: false,
        page: Some(String::from("/login")),
        kept_ms: 0,
    }
}

fn second() -> Account {
    Account {
        id: Some(String::from("family@mail.com")),
        password: String::from("other-pass"),
        ..coupang()
    }
}

#[test]
fn an_account_is_found_only_by_the_origin_it_was_kept_for() {
    let folder = folder();
    let accounts = accounts(&folder);

    accounts.keep(&coupang()).expect("kept");

    assert_eq!(
        plain(accounts.find("https://www.coupang.com").expect("read")),
        vec![coupang()]
    );
    assert!(
        accounts
            .find("https://coupang.com")
            .expect("read")
            .is_empty()
    );
    assert!(
        accounts
            .find("http://www.coupang.com")
            .expect("read")
            .is_empty()
    );
    let names = accounts.vault.names().expect("names");
    assert!(
        names.iter().all(|name| !name.contains("minji")),
        "{names:?}"
    );
    let file = std::fs::read(folder.join("vault.sqlite3")).expect("the vault file");
    let file = String::from_utf8_lossy(&file);
    assert!(!file.contains("hunter22") && !file.contains("minji.example"));
}

#[test]
fn a_second_id_adds_an_account_and_the_same_id_replaces_it() {
    let accounts = accounts(&folder());
    accounts.keep(&coupang()).expect("kept");

    accounts.keep(&second()).expect("kept");
    accounts
        .keep(&Account {
            password: String::from("changed!"),
            ..coupang()
        })
        .expect("kept");

    let mut kept = plain(accounts.find("https://www.coupang.com").expect("read"));
    kept.sort_by(|a, b| a.id.cmp(&b.id));
    assert_eq!(
        kept,
        vec![
            second(),
            Account {
                password: String::from("changed!"),
                ..coupang()
            }
        ]
    );
    assert_eq!(accounts.vault.names().expect("names").len(), 2);
}

#[test]
fn the_model_sees_each_site_and_its_ids_and_never_a_password() {
    let folder = folder();
    let accounts = accounts(&folder);
    accounts.keep(&coupang()).expect("kept");
    accounts.keep(&second()).expect("kept");
    accounts
        .keep(&Account {
            site: String::from("네이버"),
            origin: String::from("https://nid.naver.com"),
            id: Some(String::from("minji_k")),
            ..coupang()
        })
        .expect("kept");
    accounts
        .vault
        .set("ANTHROPIC_API_KEY", "sk-ant-1234", now_ms())
        .expect("kept");

    let listed = sites(&accounts.vault).expect("listed");
    let told = LinkedSites::new(Arc::clone(&accounts.vault))
        .context()
        .expect("told");

    let shown: Vec<(&str, Option<&str>)> = listed
        .iter()
        .map(|site| (site.site.as_str(), site.id.as_deref()))
        .collect();
    assert_eq!(
        shown,
        [
            ("네이버", Some("minji_k")),
            ("쿠팡", Some("family@mail.com")),
            ("쿠팡", Some("minji.example@mail.com"))
        ]
    );
    assert!(
        told.contains("- 쿠팡 (https://www.coupang.com): family@mail.com, minji.example@mail.com"),
        "{told}"
    );
    assert!(
        told.contains("- 네이버 (https://nid.naver.com): minji_k"),
        "{told}"
    );
    for secret in ["hunter22", "other-pass", "sk-ant"] {
        assert!(!told.contains(secret), "{told}");
    }
    assert!(listed.iter().all(|site| is_account(&site.name)));
    assert!(!is_account("ANTHROPIC_API_KEY"));
    assert!(forget(&accounts.vault, "ANTHROPIC_API_KEY").is_err());
}

#[test]
fn unlinking_one_of_two_accounts_leaves_the_other_on_every_linked_pc() {
    let here = accounts(&folder());
    let there = accounts(&folder());
    here.keep(&coupang()).expect("kept");
    here.keep(&second()).expect("kept");
    there
        .vault
        .take(&here.vault.copy().expect("copied"), now_ms())
        .expect("taken");
    assert_eq!(
        plain(there.find("https://www.coupang.com").expect("read")).len(),
        2
    );
    let told = LinkedSites::new(Arc::clone(&there.vault));
    assert!(
        told.context()
            .is_some_and(|told| told.contains("minji.example@mail.com"))
    );

    std::thread::sleep(std::time::Duration::from_millis(5));
    let first = sites(&here.vault)
        .expect("listed")
        .into_iter()
        .find(|site| site.id.as_deref() == Some("minji.example@mail.com"))
        .expect("listed");
    forget(&here.vault, &first.name).expect("unlinked");
    there
        .vault
        .take(&here.vault.copy().expect("copied"), now_ms())
        .expect("taken");

    assert_eq!(
        plain(there.find("https://www.coupang.com").expect("read")),
        vec![second()]
    );
    assert!(
        told.context()
            .is_some_and(|told| !told.contains("minji.example") && told.contains("family"))
    );
}

#[test]
fn unlinking_an_account_removes_every_copy_of_it() {
    let accounts = accounts(&folder());
    for (tag, kept_ms) in [("a", 10), ("b", 20)] {
        accounts
            .vault
            .set(
                &format!("site:https://www.coupang.com#{tag}"),
                &serde_json::to_string(&Account {
                    kept_ms,
                    ..coupang()
                })
                .expect("json"),
                now_ms(),
            )
            .expect("kept");
    }
    accounts.keep(&second()).expect("kept");
    let shown = sites(&accounts.vault).expect("listed");
    let chosen = shown
        .iter()
        .find(|site| site.id.as_deref() == Some("minji.example@mail.com"))
        .expect("listed");

    forget(&accounts.vault, &chosen.name).expect("unlinked");

    assert_eq!(
        plain(accounts.find("https://www.coupang.com").expect("read")),
        vec![second()]
    );
}

#[test]
fn a_full_vault_keeps_no_new_account() {
    let folder = folder();
    let accounts = accounts(&folder);
    for n in 0..256 {
        accounts
            .vault
            .set(&format!("KEY_{n}"), "value", now_ms())
            .expect("kept");
    }

    let refused = accounts.keep(&coupang()).expect_err("refused");

    assert!(refused.contains("at most"), "{refused}");
}

#[test]
fn an_account_is_never_read_as_a_key_or_token() {
    let accounts = accounts(&folder());
    accounts.keep(&coupang()).expect("kept");
    let name = sites(&accounts.vault).expect("listed")[0].name.clone();

    assert_eq!(crate::vault::lookup(&accounts.vault)(&name), None);
    assert_eq!(crate::settings::environment(&name), None);
}

#[test]
fn two_copies_of_one_account_read_as_the_later_one() {
    let accounts = accounts(&folder());
    let older = Account {
        password: String::from("older"),
        kept_ms: 10,
        ..coupang()
    };
    let newer = Account {
        password: String::from("newer"),
        kept_ms: 20,
        ..coupang()
    };
    for (tag, account) in [("a", &newer), ("b", &older)] {
        accounts
            .vault
            .set(
                &format!("site:https://www.coupang.com#{tag}"),
                &serde_json::to_string(account).expect("json"),
                now_ms(),
            )
            .expect("kept");
    }

    assert_eq!(
        accounts.find("https://www.coupang.com").expect("read"),
        vec![newer]
    );
    assert_eq!(sites(&accounts.vault).expect("listed").len(), 1);
}

#[test]
fn no_card_keeps_an_account_the_vault_has_no_room_or_name_for() {
    let accounts = accounts(&folder());
    let long = format!("https://{}.example", "a".repeat(120));

    assert!(accounts.room("https://www.coupang.com").is_ok());
    assert!(
        accounts
            .room(&long)
            .expect_err("too long")
            .contains("too long")
    );
    for n in 0..256 {
        accounts
            .vault
            .set(&format!("KEY_{n}"), "value", now_ms())
            .expect("kept");
    }
    assert!(
        accounts
            .room("https://www.coupang.com")
            .expect_err("full")
            .contains("full")
    );
}
