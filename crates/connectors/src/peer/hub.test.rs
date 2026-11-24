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
async fn greetings_past_the_limit_are_hung_up_on_until_one_finishes() {
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
