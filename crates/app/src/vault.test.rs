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
    Vault::without_keychain(folder)
}

fn copy(name: &str, value: Option<&str>, changed_ms: i64) -> SecretCopy {
    SecretCopy {
        name: name.to_owned(),
        value: value.map(str::to_owned),
        changed_ms,
    }
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
fn a_key_that_does_not_open_the_vault_is_never_used() {
    let folder = folder();
    vault(&folder).set("A", "one", 10).expect("kept");
    std::fs::write(folder.join("vault.key"), [7_u8; KEY_BYTES]).expect("another key");
    let vault = vault(&folder);

    let read = vault.get("A");
    let kept = vault.set("B", "two", 20);

    assert!(read.expect_err("refused").contains("another key"));
    assert!(kept.expect_err("refused").contains("another key"));
    assert_eq!(vault.names().expect("names"), vec!["A"]);
    let _ = std::fs::remove_dir_all(&folder);
}

#[test]
fn a_lost_key_lets_the_owner_give_keys_again_and_the_other_pcs_refill_the_rest() {
    let here = folder();
    let there = folder();
    let (here_vault, there_vault) = (vault(&here), vault(&there));
    here_vault.set("A", "one", 10).expect("kept");
    assert!(
        there_vault
            .take(&here_vault.copy().expect("copy"), 20)
            .expect("took")
    );
    std::fs::remove_file(here.join("vault.key")).expect("lost the key");
    let here_vault = vault(&here);

    assert!(
        here_vault
            .get("A")
            .expect_err("no key")
            .contains("give the keys again")
    );
    here_vault.set("B", "two", 30).expect("kept with a new key");

    assert_eq!(here_vault.get("A").expect("read"), None);
    assert!(
        !here_vault
            .copy()
            .expect("copy")
            .iter()
            .any(|copy| copy.name == "A"),
        "what could not be opened is not sent as removed"
    );
    assert!(
        here_vault
            .take(&there_vault.copy().expect("copy"), 40)
            .expect("took")
    );
    assert_eq!(here_vault.get("A").expect("read").as_deref(), Some("one"));
    assert_eq!(here_vault.get("B").expect("read").as_deref(), Some("two"));
    let _ = std::fs::remove_dir_all(&here);
    let _ = std::fs::remove_dir_all(&there);
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
            .take(&here_vault.copy().expect("copy"), 20)
            .expect("took")
    );
    assert_eq!(there_vault.get("A").expect("read").as_deref(), Some("one"));

    here_vault.remove("A", 20).expect("removed");
    assert!(
        there_vault
            .take(&here_vault.copy().expect("copy"), 30)
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
        .take(
            &[
                copy("A", Some("older"), 20),
                copy("B", Some("newer"), 20),
                copy("B", Some("newest"), 25),
                copy("B", Some("older again"), 22),
            ],
            40,
        )
        .expect("took");

    assert!(took);
    assert_eq!(vault.get("A").expect("read").as_deref(), Some("mine"));
    assert_eq!(vault.get("B").expect("read").as_deref(), Some("newest"));
    assert!(!vault.take(&vault.copy().expect("copy"), 50).expect("took"));
    let _ = std::fs::remove_dir_all(&folder);
}

#[test]
fn a_key_dated_far_ahead_is_left_out_and_the_rest_is_taken() {
    let folder = folder();
    let vault = vault(&folder);
    let now = 1_000_000_000;

    let took = vault
        .take(
            &[
                copy("AHEAD", Some("x"), now + FUTURE_TAKEN + 1),
                copy("FINE", Some("y"), now - 5),
            ],
            now,
        )
        .expect("took");

    assert!(took);
    assert_eq!(vault.names().expect("names"), vec!["FINE"]);
    let _ = std::fs::remove_dir_all(&folder);
}

#[test]
fn a_change_is_noticed_even_behind_a_key_dated_ahead() {
    let folder = folder();
    let vault = vault(&folder);
    vault
        .take(&[copy("AHEAD", Some("x"), 5_000)], 1_000)
        .expect("took");
    let before = vault.changes().expect("changes");

    vault.set("HERE", "y", 1_000).expect("kept");

    assert_ne!(vault.changes().expect("changes"), before);
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

#[test]
fn a_message_from_another_pc_never_empties_a_vault_that_lost_its_key() {
    let folder = folder();
    vault(&folder).set("A", "one", 10).expect("kept");
    std::fs::remove_file(folder.join("vault.key")).expect("lost the key");
    let vault = vault(&folder);

    let took = vault.take(&[copy("B", Some("two"), 20)], 30);

    assert!(took.expect_err("no key").contains("give the keys again"));
    assert_eq!(vault.names().expect("names"), vec!["A"]);
    assert!(!folder.join("vault.key").exists());
    let _ = std::fs::remove_dir_all(&folder);
}

#[test]
fn a_full_vault_takes_no_new_name_but_still_removes() {
    let folder = folder();
    let vault = vault(&folder);
    for n in 0..MAX_SECRETS {
        vault.set(&format!("K{n}"), "v", 10).expect("kept");
    }

    assert!(vault.set("ONE_MORE", "v", 20).is_err());
    vault.remove("K0", 30).expect("removed");
    vault.set("ONE_MORE", "v", 40).expect("room again");
    let _ = std::fs::remove_dir_all(&folder);
}
