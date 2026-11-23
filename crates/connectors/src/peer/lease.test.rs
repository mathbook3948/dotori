use super::*;

fn later(moment: Moment, by: Duration) -> Moment {
    Moment {
        steady: moment.steady + by,
        wall: moment.wall + by,
    }
}

fn slept(moment: Moment, by: Duration) -> Moment {
    Moment {
        steady: moment.steady,
        wall: moment.wall + by,
    }
}

#[test]
fn a_pc_alone_claims_at_once_and_holds_while_it_renews() {
    let start = Moment::now();
    let mut book = Book::new("a", start);

    assert_eq!(book.claim("main", start, true), Some(1));
    assert!(book.holds("main"));
    assert_eq!(book.claim("main", start, true), None);
    let soon = later(start, RENEW);
    book.renewed("main", soon);
    assert!(book.lapse(later(soon, RENEW)).is_empty());
    assert!(book.holds("main"));
}

#[test]
fn a_pc_waits_to_hear_from_the_others_before_claiming() {
    let start = Moment::now();
    let mut book = Book::new("a", start);

    assert_eq!(book.claim("main", later(start, RENEW), false), None);
    book.heard("main", "b", 3, later(start, RENEW));
    assert_eq!(book.claim("main", later(start, TTL), false), None);
    assert_eq!(book.holder("main", later(start, TTL)), Some("b"));
    let quiet = later(start, RENEW + TTL);
    assert_eq!(book.holder("main", quiet), None);
    assert_eq!(book.claim("main", quiet, false), Some(4));
}

#[test]
fn a_holder_that_slept_past_its_lease_lets_go() {
    let start = Moment::now();
    let mut book = Book::new("a", start);
    book.claim("main", start, true);

    let lapsed = book.lapse(slept(start, TTL + RENEW));

    assert_eq!(lapsed, ["main"]);
    assert!(!book.holds("main"));
    let woke = slept(start, TTL + RENEW);
    assert_eq!(
        book.claim("main", woke, false),
        None,
        "it waits to hear the others again"
    );
    assert_eq!(book.claim("main", woke, true), Some(2));
}

#[test]
fn between_two_holders_the_higher_term_and_then_the_smaller_key_wins() {
    let start = Moment::now();
    let mut b = Book::new("b", start);
    b.claim("main", start, true);

    b.heard("main", "a", 1, start);
    assert_eq!(b.holder("main", start), Some("a"));
    assert!(!b.holds("main"));

    let mut a = Book::new("a", start);
    a.claim("main", start, true);
    a.heard("main", "b", 1, start);
    assert!(a.holds("main"));
    a.heard("main", "c", 2, start);
    assert_eq!(a.holder("main", start), Some("c"));
}

#[test]
fn a_released_job_is_free_at_once_and_old_news_cannot_bring_it_back() {
    let start = Moment::now();
    let mut book = Book::new("b", start);
    book.heard("main", "a", 5, start);

    book.released("main", 5);
    book.heard("main", "a", 5, start);

    assert_eq!(book.holder("main", start), None);
    assert_eq!(book.claim("main", later(start, TTL), false), Some(6));
}

mod linked {
    use std::path::PathBuf;

    use tokio::sync::mpsc;

    use super::*;
    use crate::peer::Code;
    use crate::peer::Identity;
    use crate::peer::Invite;
    use crate::peer::hub::HubConfig;
    use crate::peer::hub::join;
    use crate::peer::now;
    use crate::peer::wire::Hello;

    fn config(name: &str, listen: bool) -> (PathBuf, HubConfig) {
        let folder =
            std::env::temp_dir().join(format!("dotori-lease-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&folder);
        std::fs::create_dir_all(&folder).expect("folder");
        let config = HubConfig {
            identity: Identity::load_or_make(&folder.join("peer.key")).expect("key"),
            device: name.to_owned(),
            name: name.to_owned(),
            address: None,
            listen: listen.then(|| "127.0.0.1:0".parse().expect("address")),
            book: folder.join("peers.json"),
            invite: folder.join("invite.json"),
            seen: folder.join("peers-seen.json"),
        };
        (folder, config)
    }

    fn hear_all(keeper: Arc<Keeper>, mut heard: mpsc::UnboundedReceiver<Heard>) {
        tokio::spawn(async move {
            while let Some(heard) = heard.recv().await {
                keeper.hear(&heard);
            }
        });
    }

    async fn until(what: &str, mut done: impl FnMut() -> bool) {
        for _ in 0..300 {
            if done() {
                return;
            }
            time::sleep(Duration::from_millis(50)).await;
        }
        panic!("{what} did not happen in time");
    }

    #[tokio::test]
    async fn one_of_two_linked_pcs_holds_the_main_job_and_the_other_takes_over_when_it_lets_go() {
        let (a_folder, a) = config("a", true);
        let (b_folder, b) = config("b", false);
        let (a_hub, a_heard) = Hub::start(a.clone()).await.expect("a started");
        let a_keeper = Keeper::start(Arc::clone(&a_hub), a_hub.own().to_owned(), &["main"]);
        hear_all(Arc::clone(&a_keeper), a_heard);
        until("a holds the job alone", || a_keeper.holds("main")).await;

        let (_, secret) = Invite::make(&a.invite, now() + 600).expect("invite");
        let code = Code {
            key: a.identity.public,
            secret,
            address: None,
        };
        let hello = Hello {
            device: String::from("b"),
            name: String::from("b"),
            address: None,
        };
        let address = a_hub.listening().expect("listening").to_string();
        join(&b.identity, &hello, &code, &address, &b.book)
            .await
            .expect("joined");
        let (b_hub, b_heard) = Hub::start(b.clone()).await.expect("b started");
        let b_keeper = Keeper::start(Arc::clone(&b_hub), b_hub.own().to_owned(), &["main"]);
        hear_all(Arc::clone(&b_keeper), b_heard);
        let mut b_sees = b_keeper.watch("main").expect("watched");
        let a_key = a_hub.own().to_owned();
        time::timeout(
            Duration::from_secs(15),
            b_sees.wait_for(|holder| holder.as_deref() == Some(a_key.as_str())),
        )
        .await
        .expect("b learns a holds it in time")
        .expect("watched");
        assert!(!b_keeper.holds("main"));

        a_keeper.release_all();
        until("b takes over", || b_keeper.holds("main")).await;

        for folder in [a_folder, b_folder] {
            let _ = std::fs::remove_dir_all(folder);
        }
    }
}

#[test]
fn a_huge_term_from_another_pc_cannot_overflow_a_claim() {
    let start = Moment::now();
    let mut book = Book::new("b", start);
    book.heard("main", "a", u64::MAX, start);
    book.released("main", u64::MAX);

    assert_eq!(book.claim("main", later(start, TTL), false), Some(u64::MAX));
}
