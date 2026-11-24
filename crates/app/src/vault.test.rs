use std::sync::atomic::AtomicUsize;
use std::sync::atomic::Ordering;

use super::*;

static NEXT: AtomicUsize = AtomicUsize::new(0);

fn folder() -> PathBuf {
    let n = NEXT.fetch_add(1, Ordering::SeqCst);
    let folder = std::env::temp_dir().join(format!("dotori-vault-{}-{n}", std::process::id()));
    let _ = std::fs::remove_dir_all(&folder);
    folder
}

fn vault(folder: &Path) -> Vault {
    Vault::with_key_file(folder.join("vault.sqlite3"), folder.join("vault.key"))
}

#[test]
fn a_kept_key_reads_back_and_is_not_in_the_file_as_written() {
    let folder = folder();
    let vault = vault(&folder);

    vault
        .set("ANTHROPIC_API_KEY", "sk-ant-secret-1234", 10)
        .expect("kept");

    assert_eq!(
        vault.get("ANTHROPIC_API_KEY").expect("read").as_deref(),
        Some("sk-ant-secret-1234")
    );
    assert_eq!(vault.names().expect("names"), vec!["ANTHROPIC_API_KEY"]);
    let file = std::fs::read(folder.join("vault.sqlite3")).expect("the file");
    assert!(
        !file.windows(18).any(|part| part == b"sk-ant-secret-1234"),
        "the value is sealed on disk"
    );
    assert_eq!(
        std::fs::read(folder.join("vault.key"))
            .expect("a key")
            .len(),
        KEY_BYTES
    );
    let _ = std::fs::remove_dir_all(&folder);
}

#[test]
fn another_dotori_on_the_same_files_reads_the_same_keys() {
    let folder = folder();
    vault(&folder)
        .set("SLACK_BOT_TOKEN", "xoxb-1", 10)
        .expect("kept");

    let again = vault(&folder);

    assert_eq!(
        again.get("SLACK_BOT_TOKEN").expect("read").as_deref(),
        Some("xoxb-1")
    );
    let _ = std::fs::remove_dir_all(&folder);
}

#[test]
fn a_sealed_value_does_not_open_under_another_name() {
    let folder = folder();
    let vault = vault(&folder);
    vault.set("A", "one", 10).expect("kept");
    vault
        .with(|connection| {
            connection.execute(
                "INSERT INTO secrets (name, sealed, changed_ms) \
                 SELECT 'B', sealed, changed_ms FROM secrets WHERE name = 'A'",
                [],
            )
        })
        .expect("moved");

    assert!(vault.get("B").is_err());
    let _ = std::fs::remove_dir_all(&folder);
}

#[test]
fn without_its_key_the_vault_says_so_and_makes_no_new_one() {
    let folder = folder();
    vault(&folder).set("A", "one", 10).expect("kept");
    std::fs::remove_file(folder.join("vault.key")).expect("lost the key");
    let vault = vault(&folder);

    let read = vault.get("A");
    let kept = vault.set("B", "two", 20);

    assert!(read.expect_err("no key").contains("gone"));
    assert!(kept.is_err(), "a new key would leave A sealed for good");
    assert!(!folder.join("vault.key").exists());
    let _ = std::fs::remove_dir_all(&folder);
}

#[test]
fn nothing_kept_reads_as_nothing_without_making_a_file() {
    let folder = folder();
    let vault = vault(&folder);

    assert_eq!(vault.get("A").expect("read"), None);
    assert_eq!(vault.names().expect("names"), Vec::<String>::new());
    assert!(!folder.exists());
}

#[test]
fn a_removed_key_goes_from_every_pc() {
    let here = folder();
    let there = folder();
    let (here_vault, there_vault) = (vault(&here), vault(&there));
    here_vault.set("A", "one", 10).expect("kept");
    assert!(
        there_vault
            .take(&here_vault.copy().expect("copy"))
            .expect("took")
    );
    assert_eq!(there_vault.get("A").expect("read").as_deref(), Some("one"));

    here_vault.remove("A", 20).expect("removed");
    assert!(
        there_vault
            .take(&here_vault.copy().expect("copy"))
            .expect("took")
    );

    assert_eq!(there_vault.get("A").expect("read"), None);
    assert_ne!(
        std::fs::read(here.join("vault.key")).expect("key"),
        std::fs::read(there.join("vault.key")).expect("key"),
        "each PC seals with its own key"
    );
    let _ = std::fs::remove_dir_all(&here);
    let _ = std::fs::remove_dir_all(&there);
}

#[test]
fn each_key_goes_by_its_own_time() {
    let folder = folder();
    let vault = vault(&folder);
    vault.set("A", "mine", 30).expect("kept");
    vault.set("B", "mine", 10).expect("kept");

    let took = vault
        .take(&[
            SecretCopy {
                name: String::from("A"),
                value: Some(String::from("older")),
                changed_ms: 20,
            },
            SecretCopy {
                name: String::from("B"),
                value: Some(String::from("newer")),
                changed_ms: 20,
            },
        ])
        .expect("took");

    assert!(took);
    assert_eq!(vault.get("A").expect("read").as_deref(), Some("mine"));
    assert_eq!(vault.get("B").expect("read").as_deref(), Some("newer"));
    assert!(!vault.take(&vault.copy().expect("copy")).expect("took"));
    let _ = std::fs::remove_dir_all(&folder);
}

#[test]
fn a_set_environment_variable_wins_over_the_vault() {
    let folder = folder();
    let vault = vault(&folder);
    let unset = format!("DOTORI_VAULT_TEST_{}", std::process::id());
    vault.set(&unset, "from the vault", 10).expect("kept");
    vault.set("PATH", "from the vault", 10).expect("kept");
    let read = lookup(&vault);

    assert_eq!(read(&unset).as_deref(), Some("from the vault"));
    assert_eq!(read("PATH"), std::env::var("PATH").ok());
    let _ = std::fs::remove_dir_all(&folder);
}
