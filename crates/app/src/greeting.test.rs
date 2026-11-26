use std::sync::Mutex;

use std::time::SystemTime;

use dotori_core::channel::InboxClosedError;
use dotori_core::conversation::Entry;
use dotori_core::record::Record;
use dotori_core::record::RecordError;
use dotori_core::tool::ToolCallId;

use super::*;

#[derive(Default)]
struct Taken(Mutex<Vec<Incoming>>);

impl Inbox for Taken {
    fn push(&self, incoming: Incoming) -> BoxFuture<'_, Result<(), InboxClosedError>> {
        self.0.lock().expect("unpoisoned").push(incoming);
        Box::pin(async { Ok(()) })
    }
}

#[tokio::test]
async fn the_note_goes_in_once_however_often_the_channel_runs() {
    let greeting = Greeting::new("별이", "Korean");
    let taken = Arc::new(Taken::default());

    for _ in 0..2 {
        let _ = tokio::time::timeout(
            std::time::Duration::from_millis(50),
            greeting.run(Arc::clone(&taken) as Arc<dyn Inbox>),
        )
        .await;
    }

    let taken = taken.0.lock().expect("unpoisoned");
    assert_eq!(taken.len(), 1);
    assert_eq!(taken[0].place.as_str(), GREETING);
    let note = taken[0].content.as_str();
    assert!(note.contains("add_memory"));
    assert!(note.contains("Greet them in Korean"), "{note}");
    assert!(note.contains("your name, 별이,"), "{note}");
    assert!(!note.contains("  "), "{note}");
}

struct Earlier(bool);

impl RecordStore for Earlier {
    fn load(&self) -> BoxFuture<'_, Result<Vec<Record>, RecordError>> {
        let records = if self.0 {
            vec![Record {
                at: SystemTime::now(),
                entry: Entry::ToolReturned {
                    call: ToolCallId::new("c"),
                    result: Ok(String::from("done")),
                },
            }]
        } else {
            Vec::new()
        };
        Box::pin(async move { Ok(records) })
    }

    fn append(&self, _records: Vec<Record>) -> BoxFuture<'_, Result<(), RecordError>> {
        Box::pin(async { Ok(()) })
    }
}

fn notes(kept: bool) -> Arc<Notes> {
    let folder =
        std::env::temp_dir().join(format!("dotori-greeting-{}-{kept}", std::process::id()));
    let _ = std::fs::remove_dir_all(&folder);
    let notes = Notes::new(folder.join("notes.sqlite3"));
    if kept {
        notes.keep("커피는 아이스").expect("kept");
    }
    notes
}

#[tokio::test]
async fn only_an_owner_with_no_history_no_notes_and_no_other_pc_is_greeted() {
    assert!(is_new(&Earlier(false), &notes(false), false).await);
    assert!(!is_new(&Earlier(true), &notes(false), false).await);
    assert!(!is_new(&Earlier(false), &notes(true), false).await);
    assert!(!is_new(&Earlier(false), &notes(false), true).await);
}

#[test]
fn the_window_shows_the_greeting_as_a_reply_and_never_the_note() {
    let traffic = |who: &str| ToWindow::Traffic {
        channel: String::from(GREETING),
        place: String::from(GREETING),
        who: who.to_owned(),
        text: String::from("안녕하세요"),
        at: 7,
    };

    assert_eq!(
        shown(traffic("dotori")),
        Some(ToWindow::Reply {
            text: String::from("안녕하세요"),
            at: 7,
        })
    );
    assert_eq!(shown(traffic(GREETING)), None);
    let slack = ToWindow::Traffic {
        channel: String::from("slack"),
        place: String::from("C1"),
        who: String::from("dotori"),
        text: String::from("hi"),
        at: 1,
    };
    assert_eq!(shown(slack.clone()), Some(slack));
}
