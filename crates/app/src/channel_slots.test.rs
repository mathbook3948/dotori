use std::future;
use std::time::Duration;

use dotori_core::channel::InboxClosedError;
use dotori_core::message::Incoming;
use tokio::time::timeout;

use super::*;

const WAIT: Duration = Duration::from_secs(5);

struct Chat {
    id: ChannelId,
    name: &'static str,
    said: mpsc::UnboundedSender<(&'static str, String)>,
    runs: mpsc::UnboundedSender<&'static str>,
    presses: Mutex<Option<mpsc::UnboundedSender<Press>>>,
}

impl Chat {
    fn new(
        name: &'static str,
        said: &mpsc::UnboundedSender<(&'static str, String)>,
        runs: &mpsc::UnboundedSender<&'static str>,
    ) -> Arc<Self> {
        Arc::new(Self {
            id: ChannelId::parse(SLACK).expect("an id"),
            name,
            said: said.clone(),
            runs: runs.clone(),
            presses: Mutex::new(None),
        })
    }

    fn press(&self, token: &str) {
        let presses = self.presses.lock().expect("unpoisoned");
        let presses = presses.as_ref().expect("someone listens");
        let _ = presses.send(Press {
            token: token.to_owned(),
            button: 0,
            posted: Posted {
                room: String::from("C1"),
                message: String::from("1"),
            },
        });
    }
}

impl Channel for Chat {
    fn id(&self) -> &ChannelId {
        &self.id
    }

    fn run(&self, _: Arc<dyn Inbox>) -> BoxFuture<'_, Result<(), ChannelError>> {
        let _ = self.runs.send(self.name);
        Box::pin(future::pending())
    }

    fn deliver(&self, _: Place, text: String) -> BoxFuture<'_, Result<(), ChannelError>> {
        let _ = self.said.send((self.name, text));
        Box::pin(async { Ok(()) })
    }

    fn private_place(&self) -> Place {
        Place::new("@owner")
    }
}

impl Cards for Chat {
    fn post(&self, _: Place, _: Card) -> BoxFuture<'_, Result<Posted, ChannelError>> {
        Box::pin(async {
            Err(ChannelError {
                detail: String::from("no cards here"),
            })
        })
    }

    fn settle(
        &self,
        _: Posted,
        _: Option<Card>,
        _: String,
    ) -> BoxFuture<'_, Result<(), ChannelError>> {
        Box::pin(async { Ok(()) })
    }

    fn presses(&self) -> mpsc::UnboundedReceiver<Press> {
        let (presses, heard) = mpsc::unbounded_channel();
        *self.presses.lock().expect("unpoisoned") = Some(presses);
        heard
    }

    fn direct_place(&self) -> Option<Place> {
        None
    }
}

struct Nowhere;

impl Inbox for Nowhere {
    fn push(&self, _: Incoming) -> BoxFuture<'_, Result<(), InboxClosedError>> {
        Box::pin(async { Ok(()) })
    }
}

async fn next<T>(seen: &mut mpsc::UnboundedReceiver<T>) -> T {
    timeout(WAIT, seen.recv())
        .await
        .expect("in time")
        .expect("open")
}

#[tokio::test]
async fn a_channel_swapped_in_runs_and_takes_the_next_reply_under_the_same_session() {
    let (said, mut heard) = mpsc::unbounded_channel();
    let (runs, mut ran) = mpsc::unbounded_channel();
    let slots = Arc::new(ChannelSlots::new().expect("slots"));
    let slack = slots
        .all()
        .into_iter()
        .find(|slot| slot.id().as_str() == SLACK)
        .expect("a slack slot");
    let running = Arc::clone(&slack);
    let session = tokio::spawn(async move { running.run(Arc::new(Nowhere)).await });

    assert!(
        slack
            .deliver(Place::new("C1"), String::from("hi"))
            .await
            .is_err()
    );

    let first = Chat::new("first", &said, &runs);
    slots.fill(&[Arc::clone(&first) as Arc<dyn Cards>], &[SLACK]);
    assert_eq!(next(&mut ran).await, "first");
    slack
        .deliver(Place::new("C1"), String::from("one"))
        .await
        .expect("delivered");
    assert_eq!(next(&mut heard).await, ("first", String::from("one")));

    let second = Chat::new("second", &said, &runs);
    slots.fill(&[Arc::clone(&second) as Arc<dyn Cards>], &[SLACK]);
    assert_eq!(next(&mut ran).await, "second");
    slack
        .deliver(Place::new("C1"), String::from("two"))
        .await
        .expect("delivered");
    assert_eq!(next(&mut heard).await, ("second", String::from("two")));
    assert!(!session.is_finished(), "the session never starts again");

    slots.fill(&[], &[SLACK]);
    assert!(
        slack
            .deliver(Place::new("C1"), String::from("three"))
            .await
            .is_err()
    );
    session.abort();
}

#[tokio::test]
async fn a_slot_whose_kind_did_not_change_keeps_its_channel() {
    let (said, _heard) = mpsc::unbounded_channel();
    let (runs, mut ran) = mpsc::unbounded_channel();
    let slots = Arc::new(ChannelSlots::new().expect("slots"));
    let slack = slots
        .all()
        .into_iter()
        .find(|slot| slot.id().as_str() == SLACK)
        .expect("a slack slot");
    let running = Arc::clone(&slack);
    let session = tokio::spawn(async move { running.run(Arc::new(Nowhere)).await });
    let first = Chat::new("first", &said, &runs);
    slots.fill(&[Arc::clone(&first) as Arc<dyn Cards>], &[SLACK]);
    assert_eq!(next(&mut ran).await, "first");

    let other = Chat::new("other", &said, &runs);
    slots.fill(&[other as Arc<dyn Cards>], &[TELEGRAM]);

    assert!(
        timeout(Duration::from_millis(200), ran.recv())
            .await
            .is_err(),
        "slack did not start again"
    );
    session.abort();
}

#[tokio::test]
async fn presses_reach_the_room_cards_from_whichever_channel_the_slot_holds() {
    let (said, _heard) = mpsc::unbounded_channel();
    let (runs, _ran) = mpsc::unbounded_channel();
    let slots = ChannelSlots::new().expect("slots");
    let slack = slots
        .all()
        .into_iter()
        .find(|slot| slot.id().as_str() == SLACK)
        .expect("a slack slot");
    let mut presses = slack.presses();

    let first = Chat::new("first", &said, &runs);
    slots.fill(&[Arc::clone(&first) as Arc<dyn Cards>], &[SLACK]);
    first.press("a");
    assert_eq!(next(&mut presses).await.token, "a");

    let second = Chat::new("second", &said, &runs);
    slots.fill(&[Arc::clone(&second) as Arc<dyn Cards>], &[SLACK]);
    second.press("b");
    assert_eq!(next(&mut presses).await.token, "b");
}
