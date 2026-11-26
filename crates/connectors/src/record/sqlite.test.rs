use std::sync::atomic::AtomicUsize;
use std::sync::atomic::Ordering;
use std::time::Duration;

use dotori_core::conversation::ThreadRef;
use dotori_core::id::ChannelId;
use dotori_core::id::ExternalMessageId;
use dotori_core::id::ExternalUserId;
use dotori_core::message::Address;
use dotori_core::message::Content;
use dotori_core::message::Message;
use dotori_core::message::Place;
use dotori_core::message::Sender;

use super::*;
use crate::record::samples::every_entry;
use crate::record::samples::record;
use crate::record::samples::shown;

static NEXT: AtomicUsize = AtomicUsize::new(0);

fn scratch() -> PathBuf {
    let n = NEXT.fetch_add(1, Ordering::SeqCst);
    let dir = std::env::temp_dir().join(format!("dotori-sqlite-{}-{n}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    dir.join("nested").join("record")
}

fn failed_at(ms: u64, detail: &str) -> Record {
    Record {
        at: UNIX_EPOCH + Duration::from_millis(ms),
        entry: Entry::ModelFailed {
            detail: detail.to_owned(),
        },
    }
}

fn details(records: &[Record]) -> Vec<String> {
    records
        .iter()
        .map(|record| match &record.entry {
            Entry::ModelFailed { detail } => detail.clone(),
            other => format!("{other:?}"),
        })
        .collect()
}

fn ids(folder: &Path, year: i64) -> Vec<Uuid> {
    let connection = Connection::open(folder.join(format!("{year}.sqlite3"))).expect("opened");
    let mut query = connection
        .prepare("SELECT id FROM records ORDER BY id")
        .expect("prepared");
    query
        .query_map([], |row| row.get::<_, String>(0))
        .expect("queried")
        .map(|id| Uuid::parse_str(&id.expect("read")).expect("uuid"))
        .collect()
}

const DEC_31_2025: u64 = 1_767_225_599_000;
const JAN_1_2026: u64 = 1_767_225_600_000;

#[tokio::test]
async fn every_entry_survives_a_round_trip_and_a_reopen() {
    let folder = scratch();
    let store = SqliteStore::new(folder.clone(), "pc-1");
    let records: Vec<Record> = every_entry().into_iter().map(record).collect();

    store.append(records[..3].to_vec()).await.expect("appended");
    store.append(records[3..].to_vec()).await.expect("appended");
    store.append(Vec::new()).await.expect("nothing appended");

    assert_eq!(shown(&store.load().await.expect("loaded")), shown(&records));
    let reopened = SqliteStore::new(folder, "pc-1");
    assert_eq!(
        shown(&reopened.load().await.expect("loaded")),
        shown(&records)
    );
}

#[tokio::test]
async fn a_missing_folder_is_an_empty_record() {
    let store = SqliteStore::new(scratch(), "pc-1");

    assert!(store.is_empty().await.expect("checked"));
    assert!(store.load().await.expect("loaded").is_empty());
}

#[tokio::test]
async fn a_record_keeps_its_number_its_pc_and_columns_for_search() {
    let folder = scratch();
    let store = SqliteStore::new(folder.clone(), "pc-1");
    let heard = Entry::Heard {
        thread: ThreadRef::new(1),
        message: Message {
            id: ExternalMessageId::new("m"),
            address: Address {
                channel: ChannelId::parse("slack").expect("valid"),
                place: Place::new("C1"),
            },
            sender: Sender {
                id: ExternalUserId::new("U2"),
                name: String::from("민지"),
                role: Role::Other,
            },
            content: Content::parse("내일 회의는 3시").expect("non-empty"),
            audience: None,
            at: None,
        },
    };
    store.append(vec![record(heard)]).await.expect("appended");

    let connection = Connection::open(folder.join("2023.sqlite3")).expect("opened");
    let row: (
        String,
        String,
        i64,
        String,
        String,
        String,
        String,
        String,
        String,
    ) = connection
        .query_row(
            "SELECT id, device, at_ms, kind, channel, place, sender_name, role, text FROM records",
            [],
            |row| {
                Ok((
                    row.get(0)?,
                    row.get(1)?,
                    row.get(2)?,
                    row.get(3)?,
                    row.get(4)?,
                    row.get(5)?,
                    row.get(6)?,
                    row.get(7)?,
                    row.get(8)?,
                ))
            },
        )
        .expect("one row");
    let id = Uuid::parse_str(&row.0).expect("uuid");
    assert_eq!(id.get_version_num(), 7);
    let (secs, nanos) = id.get_timestamp().expect("timed").to_unix();
    assert_eq!(
        secs * 1000 + u64::from(nanos) / 1_000_000,
        1_700_000_000_123
    );
    assert_eq!(
        (row.1, row.2, row.3, row.4, row.5, row.6, row.7, row.8),
        (
            String::from("pc-1"),
            1_700_000_000_123,
            String::from("heard"),
            String::from("slack"),
            String::from("C1"),
            String::from("민지"),
            String::from("other"),
            String::from("내일 회의는 3시"),
        )
    );
}

#[tokio::test]
async fn records_go_to_the_file_of_their_year_and_a_batch_stays_whole() {
    let folder = scratch();
    let store = SqliteStore::new(folder.clone(), "pc-1");

    store
        .append(vec![
            failed_at(DEC_31_2025, "last of 2025"),
            failed_at(JAN_1_2026, "straddling"),
        ])
        .await
        .expect("appended");
    store
        .append(vec![failed_at(JAN_1_2026 + 1, "first of 2026")])
        .await
        .expect("appended");

    assert_eq!(ids(&folder, 2025).len(), 2);
    assert_eq!(ids(&folder, 2026).len(), 1);
    let loaded = SqliteStore::new(folder, "pc-1")
        .load()
        .await
        .expect("loaded");
    assert_eq!(
        details(&loaded),
        vec!["last of 2025", "straddling", "first of 2026"]
    );
}

#[tokio::test]
async fn numbers_keep_growing_when_the_clock_goes_back_and_after_a_reopen() {
    let folder = scratch();
    let store = SqliteStore::new(folder.clone(), "pc-1");
    store
        .append(vec![failed_at(JAN_1_2026 + 5000, "later")])
        .await
        .expect("appended");
    store
        .append(vec![failed_at(JAN_1_2026, "clock went back")])
        .await
        .expect("appended");
    SqliteStore::new(folder.clone(), "pc-1")
        .append(vec![failed_at(JAN_1_2026, "after a restart")])
        .await
        .expect("appended");

    let loaded = SqliteStore::new(folder.clone(), "pc-1")
        .load()
        .await
        .expect("loaded");
    assert_eq!(
        details(&loaded),
        vec!["later", "clock went back", "after a restart"]
    );
    assert_eq!(ids(&folder, 2026).len(), 3);
}

#[tokio::test]
async fn imported_records_are_split_by_their_own_year() {
    let folder = scratch();
    let store = SqliteStore::new(folder.clone(), "pc-1");

    store
        .import(vec![
            failed_at(DEC_31_2025, "old"),
            failed_at(JAN_1_2026, "new"),
        ])
        .await
        .expect("imported");

    assert_eq!(ids(&folder, 2025).len(), 1);
    assert_eq!(ids(&folder, 2026).len(), 1);
}

#[tokio::test]
async fn the_record_refuses_edits_and_removals() {
    let folder = scratch();
    let store = SqliteStore::new(folder.clone(), "pc-1");
    store
        .append(vec![failed_at(JAN_1_2026, "one")])
        .await
        .expect("appended");

    let connection = Connection::open(folder.join("2026.sqlite3")).expect("opened");
    assert!(
        connection
            .execute("UPDATE records SET text = 'x'", [])
            .is_err()
    );
    assert!(connection.execute("DELETE FROM records", []).is_err());
    assert_eq!(store.load().await.expect("loaded").len(), 1);
}

#[tokio::test]
async fn a_file_from_a_newer_dotori_is_refused() {
    let folder = scratch();
    let store = SqliteStore::new(folder.clone(), "pc-1");
    store
        .append(vec![failed_at(JAN_1_2026, "one")])
        .await
        .expect("appended");
    assert!(!store.is_empty().await.expect("checked"));
    Connection::open(folder.join("2026.sqlite3"))
        .expect("opened")
        .pragma_update(None, "user_version", 99)
        .expect("set");

    let error = SqliteStore::new(folder, "pc-1")
        .load()
        .await
        .expect_err("newer");
    assert!(error.detail.contains("newer dotori"), "{}", error.detail);
}

#[test]
fn a_record_from_before_the_split_is_read_in_its_order() {
    let path = scratch().with_extension("sqlite3");
    std::fs::create_dir_all(path.parent().expect("has a folder")).expect("made");
    let connection = Connection::open(&path).expect("opened");
    connection
        .execute_batch(
            "CREATE TABLE records (seq INTEGER PRIMARY KEY AUTOINCREMENT, at_ms INTEGER NOT NULL, \
             kind TEXT NOT NULL, channel TEXT, place TEXT, sender_id TEXT, sender_name TEXT, \
             role TEXT, text TEXT, line TEXT NOT NULL); PRAGMA user_version = 1;",
        )
        .expect("made");
    for (at_ms, detail) in [(2, "second written"), (1, "first written")] {
        let line = r#"{"at_ms":AT,"entry":{"kind":"model_failed","detail":"DETAIL"}}"#
            .replace("AT", &at_ms.to_string())
            .replace("DETAIL", detail);
        connection
            .execute(
                "INSERT INTO records (at_ms, kind, line) VALUES (?1, 'model_failed', ?2)",
                params![at_ms, line],
            )
            .expect("inserted");
    }

    let records = read_legacy(&path).expect("read");

    assert_eq!(details(&records), vec!["second written", "first written"]);
    let error = read_legacy(&scratch().with_extension("missing")).expect_err("no file");
    assert!(!error.detail.is_empty());
}

#[test]
fn the_year_of_a_moment_is_its_utc_year() {
    let at = |ms: u64| UNIX_EPOCH + Duration::from_millis(ms);
    assert_eq!(year_of(at(0)), 1970);
    assert_eq!(year_of(at(DEC_31_2025)), 2025);
    assert_eq!(year_of(at(JAN_1_2026)), 2026);
    assert_eq!(year_of(at(951_782_400_000)), 2000);
    assert_eq!(year_of(UNIX_EPOCH - Duration::from_secs(1)), 1969);
}

#[tokio::test]
async fn a_year_file_without_records_still_counts_as_empty() {
    let folder = scratch();
    std::fs::create_dir_all(&folder).expect("made");
    Connection::open(folder.join("2026.sqlite3"))
        .expect("made")
        .execute_batch(SCHEMA)
        .expect("schema");
    let store = SqliteStore::new(folder, "pc-1");

    assert!(store.is_empty().await.expect("checked"));
    store
        .append(vec![failed_at(JAN_1_2026, "one")])
        .await
        .expect("appended");
    assert!(!store.is_empty().await.expect("checked"));
}

#[tokio::test]
async fn records_copied_from_another_pc_keep_their_numbers_and_count_once() {
    let here_folder = scratch();
    let there_folder = scratch();
    let here = SqliteStore::new(here_folder.clone(), "here");
    let there = SqliteStore::new(there_folder, "there");
    there
        .append(vec![
            failed_at(DEC_31_2025, "a"),
            failed_at(DEC_31_2025 + 1, "b"),
        ])
        .await
        .expect("appended there");
    here.append(vec![failed_at(DEC_31_2025 + 5, "c")])
        .await
        .expect("appended here");

    let marks = there.marks().await.expect("marks");
    let sent = there.after("there", "", 100, 1 << 20).await.expect("after");
    assert_eq!(sent.len(), 2);
    assert_eq!(here.copy_in(sent.clone()).await.expect("copied"), 2);
    assert_eq!(here.copy_in(sent.clone()).await.expect("copied again"), 0);

    let loaded = here.load().await.expect("loaded");
    assert_eq!(details(&loaded), ["a", "b", "c"]);
    let here_marks = here.marks().await.expect("marks");
    assert_eq!(here_marks["there"], marks["there"]);
    assert!(here_marks.contains_key("here"));
    assert!(
        there
            .after("there", &marks["there"], 100, 1 << 20)
            .await
            .expect("after")
            .is_empty()
    );
    let one = there.after("there", "", 100, 1).await.expect("after");
    assert_eq!(
        one.len(),
        1,
        "the first record goes even when it is longer than asked"
    );
    let arrived: Vec<String> = {
        let connection = Connection::open(here_folder.join("2025.sqlite3")).expect("opened");
        let mut query = connection
            .prepare("SELECT id FROM arrived ORDER BY seq")
            .expect("prepared");
        query
            .query_map([], |row| row.get(0))
            .expect("queried")
            .map(|id| id.expect("read"))
            .collect()
    };
    assert_eq!(arrived.len(), 3);
    assert_eq!(arrived[1..], [sent[0].id.clone(), sent[1].id.clone()]);
}

#[tokio::test]
async fn a_damaged_copy_is_refused_whole() {
    let store = SqliteStore::new(scratch(), "here");
    let good = Copied {
        id: Uuid::now_v7().to_string(),
        device: String::from("there"),
        line: String::from("{}"),
    };
    assert!(store.copy_in(vec![good]).await.is_err());
    assert!(store.is_empty().await.expect("checked"));
}

#[tokio::test]
async fn a_format_2_file_gains_arrivals_in_number_order() {
    let folder = scratch();
    let store = SqliteStore::new(folder.clone(), "here");
    store
        .append(vec![
            failed_at(DEC_31_2025, "a"),
            failed_at(DEC_31_2025 + 1, "b"),
        ])
        .await
        .expect("appended");
    drop(store);
    let path = folder.join("2025.sqlite3");
    {
        let connection = Connection::open(&path).expect("opened");
        connection
            .execute_batch("DROP TABLE arrived; PRAGMA user_version = 2;")
            .expect("made older");
    }

    let reopened = SqliteStore::new(folder.clone(), "here");
    reopened.load().await.expect("loaded");

    let connection = Connection::open(&path).expect("opened");
    let arrived: i64 = connection
        .query_row("SELECT count(*) FROM arrived", [], |row| row.get(0))
        .expect("counted");
    let version: i64 = connection
        .pragma_query_value(None, "user_version", |row| row.get(0))
        .expect("version");
    let triggers: Vec<String> = connection
        .prepare("SELECT name FROM sqlite_master WHERE type = 'trigger' ORDER BY name")
        .expect("prepared")
        .query_map([], |row| row.get(0))
        .expect("queried")
        .map(|name| name.expect("read"))
        .collect();
    assert_eq!(arrived, 2);
    assert_eq!(version, VERSION);
    assert_eq!(ids(&folder, 2025).len(), 2);
    assert!(
        !triggers.contains(&String::from("records_never_shrink")),
        "{triggers:?}"
    );
    assert!(triggers.contains(&String::from("records_shrink_only_when_forgotten")));
}

#[tokio::test]
async fn a_forgotten_record_leaves_only_its_number_and_never_comes_back() {
    let here_folder = scratch();
    let there_folder = scratch();
    let here = SqliteStore::new(here_folder.clone(), "here");
    let there = SqliteStore::new(there_folder, "there");
    there
        .append(vec![
            failed_at(JAN_1_2026, "keep"),
            failed_at(JAN_1_2026 + 1, "card number 1234"),
        ])
        .await
        .expect("appended there");
    let sent = there.after("there", "", 100, 1 << 20).await.expect("after");
    here.copy_in(sent.clone()).await.expect("copied");
    let secret = sent[1].id.clone();

    let forgotten = here
        .forget(vec![secret.clone()], Vec::new())
        .await
        .expect("forgot");
    assert_eq!(
        forgotten,
        [Forgotten {
            id: secret.clone(),
            device: String::from("there")
        }]
    );
    assert_eq!(details(&here.load().await.expect("loaded")), ["keep"]);
    assert_eq!(here.copy_in(sent.clone()).await.expect("copied again"), 0);
    assert_eq!(details(&here.load().await.expect("loaded")), ["keep"]);
    assert_eq!(
        here.marks().await.expect("marks")["there"],
        secret,
        "a forgotten number still counts as had, so it is not sent again"
    );
    let connection = Connection::open(here_folder.join("2026.sqlite3")).expect("opened");
    assert!(
        connection.execute("DELETE FROM records", []).is_err(),
        "what was not forgotten still cannot be removed"
    );
    assert!(connection.execute("DELETE FROM forgotten", []).is_err());
    let lines: String = connection
        .query_row("SELECT group_concat(line) FROM records", [], |row| {
            row.get(0)
        })
        .expect("read");
    assert!(!lines.contains("1234"));

    assert!(
        here.forget(vec![String::from("not-a-record")], Vec::new())
            .await
            .is_err()
    );
    assert_eq!(
        here.forget(vec![secret.clone()], Vec::new())
            .await
            .map(|f| f.len())
            .ok(),
        None
    );
}

#[tokio::test]
async fn what_another_pc_forgot_is_taken_whether_or_not_the_record_came_yet() {
    let there_folder = scratch();
    let early = SqliteStore::new(scratch(), "early");
    let late = SqliteStore::new(scratch(), "late");
    let there = SqliteStore::new(there_folder, "there");
    there
        .append(vec![failed_at(JAN_1_2026, "secret")])
        .await
        .expect("appended");
    let sent = there.after("there", "", 100, 1 << 20).await.expect("after");
    early.copy_in(sent.clone()).await.expect("copied");
    let forgotten = there
        .forget(vec![sent[0].id.clone()], Vec::new())
        .await
        .expect("forgot");

    assert_eq!(
        early
            .take_forgotten(forgotten.clone())
            .await
            .expect("taken"),
        1
    );
    assert_eq!(
        early
            .take_forgotten(forgotten.clone())
            .await
            .expect("again"),
        0
    );
    assert!(early.load().await.expect("loaded").is_empty());

    assert_eq!(
        late.take_forgotten(forgotten.clone()).await.expect("taken"),
        1
    );
    assert_eq!(late.copy_in(sent).await.expect("copied"), 0);
    assert!(late.load().await.expect("loaded").is_empty());
    assert_eq!(late.forgotten().await.expect("listed"), forgotten);
}

#[tokio::test]
async fn a_forget_for_a_record_not_here_yet_never_moves_how_far_this_pc_holds() {
    let there = SqliteStore::new(scratch(), "there");
    let late = SqliteStore::new(scratch(), "late");
    there
        .append(vec![
            failed_at(JAN_1_2026, "first"),
            failed_at(JAN_1_2026 + 1, "second"),
        ])
        .await
        .expect("appended");
    let sent = there.after("there", "", 100, 1 << 20).await.expect("after");
    let forgotten = there
        .forget(vec![sent[1].id.clone()], Vec::new())
        .await
        .expect("forgot");

    late.take_forgotten(forgotten).await.expect("taken");
    assert!(
        !late.marks().await.expect("marks").contains_key("there"),
        "a record never held does not count as held"
    );
    late.copy_in(sent.clone()).await.expect("copied");

    assert_eq!(details(&late.load().await.expect("loaded")), ["first"]);
    assert_eq!(
        late.marks().await.expect("marks")["there"],
        sent[1].id,
        "once its copy was refused it counts"
    );
}

#[tokio::test]
async fn a_copy_is_refused_wherever_its_forget_was_filed() {
    let there = SqliteStore::new(scratch(), "there");
    let late = SqliteStore::new(scratch(), "late");
    there
        .append(vec![failed_at(DEC_31_2025, "end of year")])
        .await
        .expect("appended");
    let mut sent = there.after("there", "", 100, 1 << 20).await.expect("after");
    let next_year = Uuid::new_v7(uuid::Timestamp::from_unix(
        uuid::NoContext,
        JAN_1_2026 / 1000 + 5,
        0,
    ))
    .to_string();
    sent[0].id.clone_from(&next_year);

    late.take_forgotten(vec![Forgotten {
        id: next_year,
        device: String::from("there"),
    }])
    .await
    .expect("taken");

    assert_eq!(late.copy_in(sent).await.expect("copied"), 0);
    assert!(late.load().await.expect("loaded").is_empty());
}

#[tokio::test]
async fn what_carries_a_forgotten_number_goes_with_it_and_a_wrong_maker_is_refused() {
    let folder = scratch();
    let store = SqliteStore::new(folder.clone(), "here");
    store
        .append(vec![failed_at(JAN_1_2026, "card 1234")])
        .await
        .expect("appended");
    let id = store.after("here", "", 10, 1 << 20).await.expect("after")[0]
        .id
        .clone();
    store
        .append(vec![
            Record {
                at: UNIX_EPOCH + Duration::from_millis(JAN_1_2026 + 1),
                entry: Entry::ToolReturned {
                    call: dotori_core::tool::ToolCallId::new("search"),
                    result: Ok(format!("[record {id}] card 1234")),
                },
            },
            failed_at(JAN_1_2026 + 2, &format!("I said {id} once")),
        ])
        .await
        .expect("appended");

    let carrying = store.carrying(vec![id.clone()]).await.expect("found");
    assert_eq!(
        carrying.len(),
        1,
        "only results carry a number, not what was said"
    );
    let forgotten = store
        .forget(vec![id.clone()], vec![carrying[0].id.clone()])
        .await
        .expect("forgot");

    assert_eq!(forgotten.len(), 2);
    assert_eq!(
        details(&store.load().await.expect("loaded")),
        [format!("I said {id} once")]
    );

    let other = SqliteStore::new(scratch(), "other");
    other
        .append(vec![failed_at(JAN_1_2026, "mine")])
        .await
        .expect("appended");
    let mine = other.after("other", "", 10, 1 << 20).await.expect("after")[0]
        .id
        .clone();
    let forged = other
        .take_forgotten(vec![Forgotten {
            id: mine,
            device: String::from("someone else"),
        }])
        .await
        .expect("taken");
    assert_eq!(forged, 0);
    assert_eq!(details(&other.load().await.expect("loaded")), ["mine"]);
}

#[tokio::test]
async fn loaded_records_say_which_this_pc_made() {
    let here = SqliteStore::new(scratch(), "here");
    let there = SqliteStore::new(scratch(), "there");
    there
        .append(vec![failed_at(JAN_1_2026, "theirs")])
        .await
        .expect("appended");
    here.copy_in(there.after("there", "", 10, 1 << 20).await.expect("after"))
        .await
        .expect("copied");
    here.append(vec![failed_at(JAN_1_2026 + 1, "mine")])
        .await
        .expect("appended");

    let marked: Vec<bool> = here
        .load_marked()
        .await
        .expect("loaded")
        .into_iter()
        .map(|marked| marked.here)
        .collect();

    assert_eq!(marked, [false, true]);
}
