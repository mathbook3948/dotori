use std::path::Path;
use std::sync::atomic::AtomicUsize;
use std::sync::atomic::Ordering;

use dotori_core::conversation::Entry;

use super::*;
use crate::record::samples::every_entry;
use crate::record::samples::record;
use crate::record::samples::shown;

static NEXT: AtomicUsize = AtomicUsize::new(0);

fn scratch() -> PathBuf {
    let n = NEXT.fetch_add(1, Ordering::SeqCst);
    let dir = std::env::temp_dir().join(format!("dotori-jsonl-{}-{n}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    dir.join("nested").join("record.jsonl")
}

#[tokio::test]
async fn every_entry_survives_a_round_trip() {
    let store = JsonlStore::new(scratch());
    let records: Vec<Record> = every_entry().into_iter().map(record).collect();

    store.append(records[..3].to_vec()).await.expect("appended");
    store.append(records[3..].to_vec()).await.expect("appended");

    let loaded = store.load().await.expect("loaded");
    assert_eq!(shown(&loaded), shown(&records));
}

#[tokio::test]
async fn a_missing_file_is_an_empty_record() {
    let store = JsonlStore::new(scratch());

    assert!(store.load().await.expect("loaded").is_empty());
}

fn torn(path: &Path) {
    let mut file = OpenOptions::new().append(true).open(path).expect("open");
    let cut = "[{\"at_ms\":1,\"entry\":{\"kind\":\"model_failed\",\"detail\":\"한";
    file.write_all(&cut.as_bytes()[..cut.len() - 1])
        .expect("write");
}

#[tokio::test]
async fn a_line_cut_short_by_a_crash_is_dropped_and_cut_away() {
    let path = scratch();
    let store = JsonlStore::new(path.clone());
    let first = record(Entry::ModelFailed {
        detail: String::from("one"),
    });
    store.append(vec![first.clone()]).await.expect("appended");
    torn(&path);

    assert_eq!(
        shown(&store.load().await.expect("loaded")),
        shown(std::slice::from_ref(&first))
    );

    let second = record(Entry::ModelFailed {
        detail: String::from("two"),
    });
    store.append(vec![second.clone()]).await.expect("appended");

    assert_eq!(
        shown(&store.load().await.expect("loaded")),
        shown(&[first, second])
    );
}

#[tokio::test]
async fn a_damaged_whole_line_fails_the_load_and_names_it() {
    let path = scratch();
    let store = JsonlStore::new(path.clone());
    store
        .append(vec![record(Entry::ModelFailed {
            detail: String::from("one"),
        })])
        .await
        .expect("appended");
    torn(&path);
    let mut file = OpenOptions::new().append(true).open(&path).expect("open");
    file.write_all(b"\n").expect("write");

    let error = store.load().await.expect_err("damaged");

    assert!(error.detail.starts_with("line 2 of "), "{}", error.detail);
}

#[tokio::test]
async fn a_sub_session_result_keeps_the_kind_it_was_stored_under() {
    let path = scratch();
    std::fs::create_dir_all(path.parent().expect("has a folder")).expect("created");
    std::fs::write(
        &path,
        "[{\"at_ms\":1,\"entry\":{\"kind\":\"sub_task_returned\",\"spec\":{\"title\":\"t\",\"brief\":\"b\"},\"result\":{\"ok\":\"done\"}}}]\n",
    )
    .expect("written");
    let store = JsonlStore::new(&path);

    let loaded = store.load().await.expect("loaded");
    assert!(matches!(
        loaded.as_slice(),
        [Record {
            entry: Entry::SubSessionReturned { .. },
            ..
        }]
    ));
    store.append(loaded).await.expect("appended");
    let written = std::fs::read_to_string(&path).expect("read");
    assert_eq!(written.matches("\"kind\":\"sub_task_returned\"").count(), 2);
}
