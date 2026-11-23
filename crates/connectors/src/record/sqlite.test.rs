use std::sync::atomic::AtomicUsize;
use std::sync::atomic::Ordering;

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
    dir.join("nested").join("record.sqlite3")
}

#[tokio::test]
async fn every_entry_survives_a_round_trip_and_a_reopen() {
    let path = scratch();
    let store = SqliteStore::new(path.clone());
    let records: Vec<Record> = every_entry().into_iter().map(record).collect();

    store.append(records[..3].to_vec()).await.expect("appended");
    store.append(records[3..].to_vec()).await.expect("appended");
    store.append(Vec::new()).await.expect("nothing appended");

    assert_eq!(shown(&store.load().await.expect("loaded")), shown(&records));
    let reopened = SqliteStore::new(path);
    assert_eq!(
        shown(&reopened.load().await.expect("loaded")),
        shown(&records)
    );
}

#[tokio::test]
async fn a_missing_file_is_an_empty_record() {
    let store = SqliteStore::new(scratch());

    assert!(store.load().await.expect("loaded").is_empty());
}

#[tokio::test]
async fn what_was_heard_is_kept_in_columns_for_search() {
    let path = scratch();
    let store = SqliteStore::new(path.clone());
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
        },
    };
    store.append(vec![record(heard)]).await.expect("appended");

    let connection = Connection::open(&path).expect("opened");
    let row: (i64, String, String, String, String, String, String) = connection
        .query_row(
            "SELECT at_ms, kind, channel, place, sender_name, role, text FROM records",
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
                ))
            },
        )
        .expect("one row");
    assert_eq!(
        row,
        (
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
async fn the_record_refuses_edits_and_removals() {
    let path = scratch();
    let store = SqliteStore::new(path.clone());
    store
        .append(vec![record(Entry::ModelFailed {
            detail: String::from("one"),
        })])
        .await
        .expect("appended");

    let connection = Connection::open(&path).expect("opened");
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
    let path = scratch();
    let store = SqliteStore::new(path.clone());
    assert!(store.is_empty().await.expect("checked"));
    store
        .append(vec![record(Entry::ModelFailed {
            detail: String::from("one"),
        })])
        .await
        .expect("appended");
    assert!(!store.is_empty().await.expect("checked"));
    Connection::open(&path)
        .expect("opened")
        .pragma_update(None, "user_version", 99)
        .expect("set");

    let error = SqliteStore::new(path).load().await.expect_err("newer");
    assert!(error.detail.contains("newer dotori"), "{}", error.detail);
}
