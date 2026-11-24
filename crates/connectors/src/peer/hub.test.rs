use std::path::Path;

use super::*;

struct Pc {
    folder: PathBuf,
    config: HubConfig,
}

fn pc(test: &str, name: &str, listen: bool) -> Pc {
    let folder =
        std::env::temp_dir().join(format!("dotori-hub-{test}-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&folder);
    std::fs::create_dir_all(&folder).expect("folder");
    let config = HubConfig {
        identity: Identity::load_or_make(&folder.join("peer.key")).expect("key"),
        device: name.to_owned(),
        name: format!("PC {name}"),
        address: None,
        listen: listen.then(|| "127.0.0.1:0".parse().expect("address")),
        book: folder.join("peers.json"),
        invite: folder.join("invite.json"),
        seen: folder.join("peers-seen.json"),
    };
    Pc { folder, config }
}

fn hello(pc: &Pc) -> Hello {
    Hello {
        device: pc.config.device.clone(),
        name: pc.config.name.clone(),
        address: pc.config.address.clone(),
    }
}

async fn until(what: &str, mut done: impl FnMut() -> bool) {
    for _ in 0..200 {
        if done() {
            return;
        }
        time::sleep(Duration::from_millis(50)).await;
    }
    panic!("{what} did not happen in time");
}

fn approved(book: &Path, key: &[u8; KEY]) -> bool {
    Book::load(book).expect("book").find(&hex(key)).is_some()
}

#[tokio::test]
async fn a_pc_joins_with_a_code_and_then_the_two_stay_linked() {
    let a = pc("join", "a", true);
    let b = pc("join", "b", false);
    let (a_hub, _a_heard) = Hub::start(a.config.clone()).await.expect("a started");
    let address = a_hub.listening().expect("listening").to_string();
    let (_, secret) = Invite::make(&a.config.invite, now() + 600).expect("invite");
    let code = Code {
        key: a.config.identity.public,
        secret,
        address: None,
    };

    let joined = join(
        &b.config.identity,
        &hello(&b),
        &code,
        &address,
        &b.config.book,
    )
    .await
    .expect("joined");

    assert_eq!(joined.device, "a");
    until("a approves b", || {
        approved(&a.config.book, &b.config.identity.public)
    })
    .await;
    assert!(!a.config.invite.exists(), "an invite is used once");
    let used = join(
        &b.config.identity,
        &hello(&b),
        &code,
        &address,
        &b.config.book,
    )
    .await;
    assert!(used.is_err());

    let (b_hub, mut b_heard) = Hub::start(b.config.clone()).await.expect("b started");
    until("the two link", || {
        a_hub.linked().len() == 1 && b_hub.linked().len() == 1
    })
    .await;
    assert!(a_hub.send(&hex(&b.config.identity.public), json!({ "kind": "hi" })));
    let linked = b_heard.recv().await.expect("linked");
    assert_eq!(linked.message["kind"], "linked");
    let heard = b_heard.recv().await.expect("heard");
    assert_eq!(heard.device, "a");
    assert_eq!(heard.message["kind"], "hi");
    let seen: Seen = serde_json::from_str(&std::fs::read_to_string(&b.config.seen).expect("seen"))
        .expect("read");
    assert!(seen.peers[&hex(&a.config.identity.public)].linked);

    for folder in [a.folder, b.folder] {
        let _ = std::fs::remove_dir_all(folder);
    }
}

#[tokio::test]
async fn a_pc_learns_of_another_through_a_third_and_a_removal_unlinks_it() {
    let a = pc("gossip", "a", true);
    let b = pc("gossip", "b", false);
    let c = pc("gossip", "c", false);
    let (a_hub, _a_heard) = Hub::start(a.config.clone()).await.expect("a started");
    let address = a_hub.listening().expect("listening").to_string();
    for joiner in [&b, &c] {
        let (_, secret) = Invite::make(&a.config.invite, now() + 600).expect("invite");
        let code = Code {
            key: a.config.identity.public,
            secret,
            address: None,
        };
        join(
            &joiner.config.identity,
            &hello(joiner),
            &code,
            &address,
            &joiner.config.book,
        )
        .await
        .expect("joined");
        until("a approves the joiner", || {
            approved(&a.config.book, &joiner.config.identity.public)
        })
        .await;
    }

    let (b_hub, _b_heard) = Hub::start(b.config.clone()).await.expect("b started");
    until("b learns of c", || {
        approved(&b.config.book, &c.config.identity.public)
    })
    .await;

    let mut book = Book::load(&a.config.book).expect("book");
    book.remove(&hex(&b.config.identity.public), now() + 1);
    book.save(&a.config.book).expect("saved");
    until("a drops b", || a_hub.linked().is_empty()).await;
    drop(b_hub);

    for folder in [a.folder, b.folder, c.folder] {
        let _ = std::fs::remove_dir_all(folder);
    }
}

#[tokio::test]
async fn greetings_past_the_limit_are_hung_up_on_until_they_end() {
    let a = pc("crowd", "a", true);
    let b = pc("crowd", "b", false);
    let (a_hub, _a_heard) = Hub::start(a.config.clone()).await.expect("a started");
    let address = a_hub.listening().expect("listening").to_string();
    let (_, secret) = Invite::make(&a.config.invite, now() + 600).expect("invite");
    let code = Code {
        key: a.config.identity.public,
        secret,
        address: None,
    };
    let mut silent = Vec::new();
    for _ in 0..GREETING {
        silent.push(TcpStream::connect(&address).await.expect("connected"));
    }
    time::sleep(Duration::from_millis(200)).await;

    let crowded = join(
        &b.config.identity,
        &hello(&b),
        &code,
        &address,
        &b.config.book,
    )
    .await;
    assert!(crowded.is_err());

    drop(silent);
    time::sleep(Duration::from_millis(200)).await;
    let joined = join(
        &b.config.identity,
        &hello(&b),
        &code,
        &address,
        &b.config.book,
    )
    .await
    .expect("joined");
    assert_eq!(joined.device, "a");

    for folder in [a.folder, b.folder] {
        let _ = std::fs::remove_dir_all(folder);
    }
}

fn approve(book: &Path, other: &Pc, address: Option<String>) {
    let mut kept = Book::load(book).expect("book");
    kept.approve(Peer {
        device: other.config.device.clone(),
        name: other.config.name.clone(),
        key: hex(&other.config.identity.public),
        address,
        since: now(),
    });
    kept.save(book).expect("saved");
}

async fn joined(a: &Pc, b: &Pc, address: &str) {
    let (_, secret) = Invite::make(&a.config.invite, now() + 600).expect("invite");
    let code = Code {
        key: a.config.identity.public,
        secret,
        address: None,
    };
    join(
        &b.config.identity,
        &hello(b),
        &code,
        address,
        &b.config.book,
    )
    .await
    .expect("joined");
    until("a approves b", || {
        approved(&a.config.book, &b.config.identity.public)
    })
    .await;
}

fn filler() -> Value {
    json!({ "kind": "filler", "text": "도".repeat(400) })
}

#[tokio::test]
async fn a_pc_that_stops_reading_is_unlinked_and_sending_to_it_says_false() {
    let a = pc("stuck", "a", true);
    let b = pc("stuck", "b", false);
    let (a_hub, _a_heard) = Hub::start(a.config.clone()).await.expect("a started");
    let address = a_hub.listening().expect("listening").to_string();
    joined(&a, &b, &address).await;
    let stream = TcpStream::connect(&address).await.expect("connected");
    let _unread = wire::dial(
        stream,
        &b.config.identity,
        &a.config.identity.public,
        Start::Link,
        &hello(&b),
    )
    .await
    .expect("dialed");
    until("a links b", || a_hub.linked().len() == 1).await;
    let b_key = hex(&b.config.identity.public);

    let mut queued = 0;
    while queued < 100_000 && a_hub.send(&b_key, filler()) {
        queued += 1;
        tokio::task::yield_now().await;
    }

    assert!(
        queued > wire::QUEUE_COUNT,
        "the socket and the queue both filled"
    );
    assert!(
        queued < 100_000,
        "a PC that stops reading is not queued for forever"
    );
    assert!(a_hub.linked().is_empty());
    assert!(!a_hub.send(&b_key, filler()));
    until("a marks b unlinked", || {
        std::fs::read_to_string(&a.config.seen)
            .ok()
            .and_then(|text| serde_json::from_str::<Seen>(&text).ok())
            .is_some_and(|seen| seen.peers.get(&b_key).is_some_and(|peer| !peer.linked))
    })
    .await;

    for folder in [a.folder, b.folder] {
        let _ = std::fs::remove_dir_all(folder);
    }
}

#[tokio::test]
async fn a_pc_that_keeps_reading_stays_linked_through_more_messages_than_a_queue_holds() {
    let a = pc("steady", "a", true);
    let b = pc("steady", "b", false);
    let (a_hub, _a_heard) = Hub::start(a.config.clone()).await.expect("a started");
    let address = a_hub.listening().expect("listening").to_string();
    joined(&a, &b, &address).await;
    let (b_hub, mut b_heard) = Hub::start(b.config.clone()).await.expect("b started");
    until("the two link", || {
        a_hub.linked().len() == 1 && b_hub.linked().len() == 1
    })
    .await;
    let b_key = hex(&b.config.identity.public);
    let total = 3 * wire::QUEUE_COUNT;

    for _ in 0..total {
        assert!(a_hub.send(&b_key, filler()));
        tokio::task::yield_now().await;
    }

    let mut heard = 0;
    while heard < total {
        let message = time::timeout(Duration::from_secs(10), b_heard.recv())
            .await
            .expect("heard in time")
            .expect("heard");
        if message.message["kind"] == "filler" {
            heard += 1;
        }
    }
    assert_eq!(a_hub.linked().len(), 1);

    for folder in [a.folder, b.folder] {
        let _ = std::fs::remove_dir_all(folder);
    }
}

#[test]
fn redials_wait_twice_as_long_after_each_failure_up_to_a_minute() {
    let now = time::Instant::now();
    for random in [0, 12_345_678_901, u64::MAX] {
        let mut previous = None;
        for longest in [2, 4, 8, 16, 32, 60, 60] {
            let redial = Redial::after(previous, now, random);
            let waited = redial.at - now;
            let longest = Duration::from_secs(longest);
            assert!(
                waited >= longest / 2 && waited <= longest,
                "{waited:?} for {longest:?}"
            );
            previous = Some(redial);
        }
    }
}

#[test]
fn redials_are_spread_between_half_and_all_of_the_wait() {
    let wait = Duration::from_secs(8);

    let spreads: HashSet<Duration> = [0, 1, 1_000_000_007, u64::MAX / 3, u64::MAX]
        .into_iter()
        .map(|random| spread(wait, random))
        .collect();

    assert_eq!(spread(wait, 0), wait / 2);
    assert!(spreads.len() > 1);
    assert!(
        spreads
            .iter()
            .all(|waited| *waited >= wait / 2 && *waited <= wait)
    );
}

#[tokio::test]
async fn a_pc_that_was_unreachable_is_linked_soon_after_it_starts_listening() {
    let a = pc("late", "a", false);
    let b = pc("late", "b", false);
    let port = std::net::TcpListener::bind("127.0.0.1:0")
        .expect("bound")
        .local_addr()
        .expect("address")
        .port();
    let a_config = HubConfig {
        listen: Some(SocketAddr::from(([127, 0, 0, 1], port))),
        ..a.config.clone()
    };
    approve(&a.config.book, &b, None);
    approve(&b.config.book, &a, Some(format!("127.0.0.1:{port}")));
    let (b_hub, _b_heard) = Hub::start(b.config.clone()).await.expect("b started");
    time::sleep(Duration::from_millis(300)).await;
    assert!(b_hub.linked().is_empty());

    let (a_hub, _a_heard) = Hub::start(a_config).await.expect("a started");

    until("b dials a again", || {
        a_hub.linked().len() == 1 && b_hub.linked().len() == 1
    })
    .await;

    for folder in [a.folder, b.folder] {
        let _ = std::fs::remove_dir_all(folder);
    }
}
