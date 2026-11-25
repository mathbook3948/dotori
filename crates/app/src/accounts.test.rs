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

fn coupang() -> Account {
    Account {
        site: String::from("쿠팡"),
        origin: String::from("https://www.coupang.com"),
        id_label: Some(String::from("아이디(이메일)")),
        id: Some(String::from("minji.example@mail.com")),
        password_label: String::from("비밀번호"),
        password: String::from("hunter22!"),
        trusted: false,
    }
}

#[test]
fn an_account_is_found_only_by_the_origin_it_was_kept_for() {
    let folder = folder();
    let accounts = accounts(&folder);

    accounts.keep(&coupang()).expect("kept");

    assert_eq!(
        accounts.find("https://www.coupang.com").expect("read"),
        Some(coupang())
    );
    assert_eq!(accounts.find("https://coupang.com").expect("read"), None);
    assert_eq!(accounts.find("http://www.coupang.com").expect("read"), None);
    let file = std::fs::read(folder.join("vault.sqlite3")).expect("the vault file");
    let file = String::from_utf8_lossy(&file);
    assert!(!file.contains("hunter22") && !file.contains("minji.example"));
}

#[test]
fn the_settings_list_names_each_site_and_its_id_and_the_model_only_the_site() {
    let folder = folder();
    let accounts = accounts(&folder);
    accounts.keep(&coupang()).expect("kept");
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

    assert_eq!(
        listed,
        vec![
            Site {
                site: String::from("네이버"),
                origin: String::from("https://nid.naver.com"),
                id: Some(String::from("minji_k")),
            },
            Site {
                site: String::from("쿠팡"),
                origin: String::from("https://www.coupang.com"),
                id: Some(String::from("minji.example@mail.com")),
            },
        ]
    );
    assert!(told.contains("- 쿠팡 (https://www.coupang.com)"), "{told}");
    assert!(told.contains("- 네이버 (https://nid.naver.com)"), "{told}");
    for secret in ["minji", "hunter22", "sk-ant"] {
        assert!(!told.contains(secret), "{told}");
    }
    assert!(is_account("site:https://www.coupang.com"));
    assert!(!is_account("ANTHROPIC_API_KEY"));
}

#[test]
fn an_unlinked_site_leaves_every_linked_pc() {
    let here = accounts(&folder());
    let there = accounts(&folder());
    here.keep(&coupang()).expect("kept");
    there
        .vault
        .take(&here.vault.copy().expect("copied"), now_ms())
        .expect("taken");
    assert_eq!(
        there.find("https://www.coupang.com").expect("read"),
        Some(coupang())
    );
    let told = LinkedSites::new(Arc::clone(&there.vault));
    assert!(told.context().is_some());

    std::thread::sleep(std::time::Duration::from_millis(5));
    forget(&here.vault, "https://www.coupang.com").expect("unlinked");
    there
        .vault
        .take(&here.vault.copy().expect("copied"), now_ms())
        .expect("taken");

    assert_eq!(there.find("https://www.coupang.com").expect("read"), None);
    assert_eq!(sites(&there.vault).expect("listed"), Vec::new());
    assert_eq!(told.context(), None);
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
