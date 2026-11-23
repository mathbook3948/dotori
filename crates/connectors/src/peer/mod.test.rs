use super::*;

fn peer(key: &str, since: u64) -> Peer {
    Peer {
        device: format!("pc-{key}"),
        name: String::new(),
        key: key.to_owned(),
        address: None,
        since,
    }
}

#[test]
fn a_key_pair_is_made_once_and_kept() {
    let folder = std::env::temp_dir().join(format!("dotori-identity-{}", std::process::id()));
    let path = folder.join("peer.key");
    let _ = fs::remove_dir_all(&folder);

    let made = Identity::load_or_make(&path).expect("made");
    let kept = Identity::load_or_make(&path).expect("kept");

    assert_eq!(made.public, kept.public);
    assert_eq!(made.private, kept.private);
    assert_ne!(made.public, made.private);
    assert!(!format!("{made:?}").contains(&hex(&made.private)));
    let _ = fs::remove_dir_all(&folder);
}

#[test]
fn the_later_of_an_approval_and_a_removal_wins() {
    let mut book = Book::default();
    assert!(book.approve(peer("a", 10)));
    assert!(!book.approve(peer("a", 10)));
    assert!(book.remove("a", 20));
    assert!(book.find("a").is_none());
    assert!(!book.approve(peer("a", 15)));
    assert!(book.approve(peer("a", 30)));
    assert!(book.removed.is_empty());
    assert!(!book.remove("a", 25));
    assert!(book.find("a").is_some());
}

#[test]
fn merging_books_spreads_approvals_and_removals_but_not_ones_own_key() {
    let mut here = Book::default();
    here.approve(peer("b", 1));
    let mut there = Book::default();
    there.approve(peer("me", 1));
    there.approve(peer("c", 2));
    there.remove("b", 5);

    assert!(here.merge(&there, "me", 100));
    assert!(!here.merge(&there, "me", 100));

    let keys: Vec<&str> = here.peers.iter().map(|peer| peer.key.as_str()).collect();
    assert_eq!(keys, ["c"]);
    assert_eq!(here.removed[0].key, "b");
}

#[test]
fn a_moved_pc_keeps_its_last_known_address_when_it_names_none() {
    let mut book = Book::default();
    book.approve(Peer {
        address: Some(String::from("10.0.0.1:1")),
        ..peer("a", 1)
    });
    let hello = |address: Option<&str>| wire::Hello {
        device: String::from("pc-a"),
        name: String::new(),
        address: address.map(str::to_owned),
    };
    assert!(!book.moved("a", &hello(None)));
    assert!(book.moved("a", &hello(Some("10.0.0.2:1"))));
    assert_eq!(
        book.find("a").and_then(|peer| peer.address.as_deref()),
        Some("10.0.0.2:1")
    );
}

#[test]
fn a_code_reads_back_with_and_without_an_address() {
    let code = Code {
        key: [7; KEY],
        secret: [9; SECRET],
        address: Some(String::from("laptop.tail.ts.net:47820")),
    };
    let written = code.write();
    assert!(
        written
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-' || byte == b'_')
    );
    assert_eq!(Code::read(&format!(" {written}\n")), Some(code.clone()));
    let bare = Code {
        address: None,
        ..code
    };
    assert_eq!(Code::read(&bare.write()), Some(bare));
    assert_eq!(Code::read("short"), None);
    assert_eq!(Code::read("not base64!"), None);
}

#[test]
fn an_invite_is_good_only_until_its_time() {
    let folder = std::env::temp_dir().join(format!("dotori-invite-{}", std::process::id()));
    let path = folder.join("invite.json");
    let (_, secret) = Invite::make(&path, 100).expect("made");

    assert_eq!(Invite::open(&path, 99), Some(secret));
    assert_eq!(Invite::open(&path, 100), None);
    let _ = fs::remove_dir_all(&folder);
}

#[test]
fn hex_reads_back() {
    assert_eq!(unhex(&hex(&[0, 1, 254, 255])), Some(vec![0, 1, 254, 255]));
    assert_eq!(unhex("abc"), None);
    assert_eq!(unhex("zz"), None);
}

#[test]
fn a_time_from_the_future_is_taken_as_now_so_a_removal_still_works() {
    let mut here = Book::default();
    let mut there = Book::default();
    there.approve(peer("x", u64::MAX));

    here.merge(&there, "me", 100);

    assert_eq!(here.find("x").map(|peer| peer.since), Some(100));
    assert!(here.remove("x", 101));
}

#[test]
fn an_older_or_equal_copy_does_not_change_a_known_pc() {
    let mut book = Book::default();
    book.approve(Peer {
        address: Some(String::from("new:1")),
        ..peer("a", 5)
    });
    assert!(!book.approve(Peer {
        address: Some(String::from("old:1")),
        ..peer("a", 5)
    }));
    assert_eq!(
        book.find("a").and_then(|peer| peer.address.as_deref()),
        Some("new:1")
    );
}
