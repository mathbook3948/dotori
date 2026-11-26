use std::sync::Mutex;

use dotori_core::channel::InboxClosedError;

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
    let greeting = Greeting::new();
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
    assert!(taken[0].content.as_str().contains("add_memory"));
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
