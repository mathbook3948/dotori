use std::sync::Mutex;
use std::sync::atomic::AtomicUsize;
use std::sync::atomic::Ordering;
use std::time::Duration;
use std::time::UNIX_EPOCH;

use dotori_core::conversation::Entry;
use dotori_core::conversation::ThreadRef;
use dotori_core::id::ChannelId;
use dotori_core::id::ExternalMessageId;
use dotori_core::id::ExternalUserId;
use dotori_core::message::Address;
use dotori_core::message::Content;
use dotori_core::message::Message;
use dotori_core::message::Place;
use dotori_core::message::Role;
use dotori_core::message::Sender;
use dotori_core::record::Record;
use dotori_core::record::RecordStore;
use dotori_core::tool::ToolCallId;

use super::*;

static NEXT: AtomicUsize = AtomicUsize::new(0);

fn folder() -> std::path::PathBuf {
    let n = NEXT.fetch_add(1, Ordering::SeqCst);
    let folder = std::env::temp_dir().join(format!("dotori-forget-{}-{n}", std::process::id()));
    let _ = std::fs::remove_dir_all(&folder);
    folder
}

fn said(text: &str, ms: u64) -> Record {
    Record {
        at: UNIX_EPOCH + Duration::from_millis(ms),
        entry: Entry::Heard {
            thread: ThreadRef::new(1),
            message: Message {
                id: ExternalMessageId::new(format!("m{ms}")),
                address: Address {
                    channel: ChannelId::parse("app").expect("valid"),
                    place: Place::new("main"),
                },
                sender: Sender {
                    id: ExternalUserId::new("owner"),
                    name: String::from("유노"),
                    role: Role::Owner,
                },
                content: Content::parse(text.to_owned()).expect("text"),
                audience: None,
            },
        },
    }
}

async fn store(folder: &std::path::Path) -> (SqliteStore, Vec<String>) {
    let store = SqliteStore::new(folder.join("record"), "here");
    store
        .append(vec![
            said("내 카드 번호는 1234야", 1_767_225_600_000),
            said("내일 회의 3시", 1_767_225_600_001),
        ])
        .await
        .expect("appended");
    let ids = store
        .after("here", "", 10, 1 << 20)
        .await
        .expect("listed")
        .into_iter()
        .map(|copied| copied.id)
        .collect();
    (store, ids)
}

#[derive(Default)]
struct Kept(Mutex<Vec<ToolCall>>);

impl Toolbox for Kept {
    fn specs(&self) -> Vec<ToolSpec> {
        Vec::new()
    }

    fn run(&self, call: ToolCall) -> BoxFuture<'_, Result<String, ToolError>> {
        self.0.lock().expect("unpoisoned").push(call);
        Box::pin(async { Ok(String::from("waiting")) })
    }
}

fn call(name: &str, input: Value) -> ToolCall {
    ToolCall {
        id: ToolCallId::new("c1"),
        name: name.to_owned(),
        input: input.to_string(),
    }
}

#[tokio::test]
async fn the_card_shows_what_would_be_forgotten_and_an_unknown_number_never_reaches_it() {
    let folder = folder();
    let (store, ids) = store(&folder).await;
    let kept = Arc::new(Kept::default());
    let previewed = Previewed {
        inner: Arc::clone(&kept) as Arc<dyn Toolbox>,
        store,
    };

    previewed
        .run(call(FORGET, json!({ "records": [ids[0]] })))
        .await
        .expect("asked");
    let refused = previewed
        .run(call(
            FORGET,
            json!({ "records": ["019a0000-0000-7000-8000-000000000000"] }),
        ))
        .await;
    previewed
        .run(call("webfetch", json!({ "url": "https://example.com" })))
        .await
        .expect("passed");

    assert!(
        refused
            .expect_err("unknown")
            .detail
            .contains("memory_search")
    );
    let calls = kept.0.lock().expect("unpoisoned");
    assert_eq!(calls.len(), 2);
    let shown: Value = serde_json::from_str(&calls[0].input).expect("json");
    assert_eq!(shown["records"], json!([ids[0]]));
    let forgetting = shown["지울 기록"].as_str().expect("text");
    assert!(
        forgetting.contains("유노: 내 카드 번호는 1234야"),
        "{forgetting}"
    );
    assert!(!forgetting.contains("회의"));
    assert_eq!(calls[1].name, "webfetch");
    let _ = std::fs::remove_dir_all(folder);
}

#[tokio::test]
async fn forgetting_removes_the_record_and_its_place_in_search() {
    let folder = folder();
    let (store, ids) = store(&folder).await;
    let memory = MemoryConfig {
        record: folder.join("record"),
        index: folder.join("search.sqlite3"),
        dictionaries: folder.join("dictionaries"),
        embedding: None,
        jobs: None,
    };
    let search = Memory::new(memory.clone());
    let tools = search.tools();
    let before = tools[0]
        .call(json!({ "query": "카드 번호" }))
        .await
        .expect("searched");
    assert!(before.contains("1234"), "{before}");
    let forget = Forget {
        store: store.clone(),
        memory,
        copies: Arc::default(),
    };

    let said = forget
        .call(json!({ "records": [ids[0]] }))
        .await
        .expect("forgot");

    assert!(said.starts_with("Forgot 1 record"), "{said}");
    let left = store.load().await.expect("loaded");
    assert_eq!(left.len(), 1);
    let index = rusqlite::Connection::open(folder.join("search.sqlite3")).expect("opened");
    let texts: String = index
        .query_row("SELECT group_concat(text, ' ') FROM docs", [], |row| {
            row.get(0)
        })
        .expect("read");
    assert!(!texts.contains("1234"), "{texts}");
    assert!(forget.call(json!({ "records": [] })).await.is_err());
    assert!(forget.call(json!({ "records": [ids[0]] })).await.is_err());
    let _ = std::fs::remove_dir_all(folder);
}

fn result_carrying(id: &str, ms: u64) -> Record {
    Record {
        at: UNIX_EPOCH + Duration::from_millis(ms),
        entry: Entry::ToolReturned {
            call: ToolCallId::new(format!("search-{ms}")),
            result: Ok(format!("[record {id}] 내 카드 번호는 1234야")),
        },
    }
}

#[tokio::test]
async fn only_the_carriers_the_owner_saw_go_with_the_record() {
    let folder = folder();
    let (store, ids) = store(&folder).await;
    store
        .append(vec![result_carrying(&ids[0], 1_767_225_600_010)])
        .await
        .expect("appended");
    let kept = Arc::new(Kept::default());
    let previewed = Previewed {
        inner: Arc::clone(&kept) as Arc<dyn Toolbox>,
        store: store.clone(),
    };
    previewed
        .run(call(
            FORGET,
            json!({ "records": [ids[0]], "carriers": ["made-up"] }),
        ))
        .await
        .expect("asked");
    let shown: Value = {
        let calls = kept.0.lock().expect("unpoisoned");
        serde_json::from_str(&calls[0].input).expect("json")
    };
    assert_eq!(shown["carriers"].as_array().map(Vec::len), Some(1));
    assert!(
        shown["함께 지울 기록 (검색 결과 등)"]
            .as_str()
            .is_some_and(|also| also.contains("도구 결과")),
        "{shown}"
    );
    store
        .append(vec![result_carrying(&ids[0], 1_767_225_600_020)])
        .await
        .expect("appended later");
    let forget = Forget {
        store: store.clone(),
        memory: MemoryConfig {
            record: folder.join("record"),
            index: folder.join("search.sqlite3"),
            dictionaries: folder.join("dictionaries"),
            embedding: None,
            jobs: None,
        },
        copies: Arc::default(),
    };

    let said = forget.call(shown).await.expect("forgot");

    assert!(said.starts_with("Forgot 2 record"), "{said}");
    let left = store.load().await.expect("loaded");
    assert_eq!(left.len(), 2, "the other message and the later result stay");
    let _ = std::fs::remove_dir_all(folder);
}
