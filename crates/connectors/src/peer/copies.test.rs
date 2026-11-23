use std::path::PathBuf;
use std::time::Duration;
use std::time::SystemTime;

use dotori_core::conversation::Entry;
use tokio::sync::mpsc;
use tokio::time;

use super::*;
use crate::peer::Code;
use crate::peer::Identity;
use crate::peer::Invite;
use crate::peer::hub::HubConfig;
use crate::peer::hub::join;
use crate::peer::now;
use crate::peer::wire::Hello;

struct Pc {
    folder: PathBuf,
    config: HubConfig,
    store: SqliteStore,
}

fn pc(name: &str, listen: bool) -> Pc {
    let folder = std::env::temp_dir().join(format!("dotori-copies-{name}-{}", std::process::id()));
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
    let store = SqliteStore::new(folder.join("record"), name);
    Pc {
        folder,
        config,
        store,
    }
}

fn failed(detail: &str) -> Record {
    Record {
        at: SystemTime::now(),
        entry: Entry::ModelFailed {
            detail: detail.to_owned(),
        },
    }
}

fn hear_all(copies: Arc<Copies>, mut heard: mpsc::UnboundedReceiver<Heard>) {
    tokio::spawn(async move {
        while let Some(heard) = heard.recv().await {
            copies.hear(&heard).await;
        }
    });
}

async fn details(store: &SqliteStore) -> Vec<String> {
    store
        .load()
        .await
        .expect("loaded")
        .into_iter()
        .map(|record| match record.entry {
            Entry::ModelFailed { detail } => detail,
            other => format!("{other:?}"),
        })
        .collect()
}

async fn until_holds(store: &SqliteStore, expected: &[&str]) {
    for _ in 0..200 {
        if details(store).await == expected {
            return;
        }
        time::sleep(Duration::from_millis(50)).await;
    }
    panic!("holds {:?}, not {expected:?}", details(store).await);
}

#[tokio::test]
async fn linked_pcs_catch_up_and_then_copy_each_new_record() {
    let a = pc("a", true);
    let b = pc("b", false);
    a.store.append(vec![failed("a1")]).await.expect("a1");
    time::sleep(Duration::from_millis(5)).await;
    b.store.append(vec![failed("b1")]).await.expect("b1");
    let (a_hub, a_heard) = Hub::start(a.config.clone()).await.expect("a started");
    let a_copies = Copies::start(Arc::clone(&a_hub), a.store.clone());
    hear_all(Arc::clone(&a_copies), a_heard);
    let (_, secret) = Invite::make(&a.config.invite, now() + 600).expect("invite");
    let code = Code {
        key: a.config.identity.public,
        secret,
        address: None,
    };
    let hello = Hello {
        device: String::from("b"),
        name: String::from("b"),
        address: None,
    };
    let address = a_hub.listening().expect("listening").to_string();
    join(&b.config.identity, &hello, &code, &address, &b.config.book)
        .await
        .expect("joined");
    let (b_hub, b_heard) = Hub::start(b.config.clone()).await.expect("b started");
    let b_copies = Copies::start(Arc::clone(&b_hub), b.store.clone());
    hear_all(Arc::clone(&b_copies), b_heard);

    until_holds(&a.store, &["a1", "b1"]).await;
    until_holds(&b.store, &["a1", "b1"]).await;

    let announced = Announced {
        store: a.store.clone(),
        copies: Arc::clone(&a_copies),
    };
    announced.append(vec![failed("a2")]).await.expect("a2");
    until_holds(&b.store, &["a1", "b1", "a2"]).await;
    let a_marks = a.store.marks().await.expect("marks");
    assert_eq!(b.store.marks().await.expect("marks"), a_marks);

    for folder in [a.folder, b.folder] {
        let _ = std::fs::remove_dir_all(folder);
    }
}
