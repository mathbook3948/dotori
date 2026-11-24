use std::sync::atomic::AtomicUsize;
use std::sync::atomic::Ordering;

use super::*;

static NEXT: AtomicUsize = AtomicUsize::new(0);

fn config() -> MemoryConfig {
    let n = NEXT.fetch_add(1, Ordering::SeqCst);
    let folder = std::env::temp_dir().join(format!("dotori-memory-{}-{n}", std::process::id()));
    let _ = std::fs::remove_dir_all(&folder);
    std::fs::create_dir_all(&folder).expect("made");
    MemoryConfig {
        record: folder.join("record"),
        index: folder.join("search.sqlite3"),
        dictionaries: folder.join("dictionaries"),
        embedding: None,
        jobs: None,
    }
}

fn record(config: &MemoryConfig, rows: &[(&str, Option<&str>, Option<&str>)]) {
    std::fs::create_dir_all(&config.record).expect("made");
    let record = Connection::open(config.record.join("2023.sqlite3")).expect("opened");
    record
        .execute_batch(
            "CREATE TABLE IF NOT EXISTS records (id TEXT PRIMARY KEY, device TEXT NOT NULL, \
             at_ms INTEGER NOT NULL, kind TEXT NOT NULL, channel TEXT, place TEXT, sender_id TEXT, \
             sender_name TEXT, role TEXT, text TEXT, line TEXT NOT NULL) WITHOUT ROWID; \
             CREATE TABLE IF NOT EXISTS arrived (seq INTEGER PRIMARY KEY AUTOINCREMENT, id TEXT NOT NULL UNIQUE)",
        )
        .expect("made");
    for (kind, sender, text) in rows {
        let held: i64 = record
            .query_row("SELECT count(*) FROM records", [], |row| row.get(0))
            .expect("counted");
        record
            .execute(
                "INSERT INTO records (id, device, at_ms, kind, channel, place, sender_name, role, text, line) \
                 VALUES (?1, 'pc', 1700000000123, ?2, 'slack', 'C1', ?3, ?4, ?5, '{}')",
                params![format!("r{:04}", held + 1), kind, sender, sender.map(|_| "other"), text],
            )
            .expect("inserted");
        record
            .execute(
                "INSERT INTO arrived (id) VALUES (?1)",
                [format!("r{:04}", held + 1)],
            )
            .expect("arrived");
    }
}

fn memory(config: MemoryConfig) -> Arc<Memory> {
    Arc::new(Memory {
        config,
        embedder: None,
        meaning_ready: AtomicBool::new(false),
        state: Mutex::new(State::default()),
    })
}

#[test]
fn a_search_finds_records_by_their_words_and_names_them() {
    let config = config();
    record(
        &config,
        &[
            ("heard", Some("민지"), Some("내일 회의는 3시에 해요")),
            ("tool_returned", None, None),
            ("heard", Some("유노"), Some("점심 메뉴 추천해줘")),
        ],
    );
    let memory = memory(config.clone());

    let found = memory.search("회의 언제", 10, None).expect("searched");
    assert_eq!(
        found,
        "[record r0001] 2023-11-14 22:13 UTC in slack C1, 민지 (other): 내일 회의는 3시에 해요"
    );
    assert_eq!(memory.search("민지가", 10, None).expect("by sender"), found);
    assert_eq!(
        memory.search("저녁", 10, None).expect("searched"),
        "Nothing in memory matches."
    );

    record(&config, &[("acted", None, Some("회의 장소는 2층입니다"))]);
    let later = memory.search("회의", 10, None).expect("caught up");
    assert!(
        later.contains("[record r0004]") && later.contains("dotori: 회의 장소"),
        "{later}"
    );
    let _ = std::fs::remove_dir_all(config.index.parent().expect("a folder"));
}

#[test]
fn nothing_recorded_yet_is_not_an_error() {
    let memory = memory(config());
    assert_eq!(
        memory.search("아무거나", 10, None).expect("searched"),
        "Nothing is recorded yet."
    );
}

#[test]
fn a_query_of_only_symbols_is_refused() {
    let config = config();
    record(&config, &[("heard", Some("민지"), Some("안녕"))]);
    assert!(memory(config).search("?!", 10, None).is_err());
}

#[test]
fn a_change_of_analyzer_rebuilds_the_index() {
    let config = config();
    record(&config, &[("heard", Some("민지"), Some("회의"))]);
    let mut index = open_index(&config.index).expect("index");
    catch_up(&mut index, &config.record, &Analyzer::Plain).expect("caught up");
    index
        .execute("UPDATE meta SET value = 'other' WHERE key = 'analyzer'", [])
        .expect("changed");
    catch_up(&mut index, &config.record, &Analyzer::Plain).expect("rebuilt");
    let count: i64 = index
        .query_row("SELECT count(*) FROM terms", [], |row| row.get(0))
        .expect("counted");
    assert_eq!(count, 1);
}

#[test]
fn times_are_shown_in_utc() {
    assert_eq!(when(0), "1970-01-01 00:00 UTC");
    assert_eq!(when(951_825_600_000), "2000-02-29 12:00 UTC");
    assert_eq!(when(1_767_225_599_000), "2025-12-31 23:59 UTC");
}

#[test]
#[ignore = "needs the Korean dictionary in DOTORI_KO_DIC"]
fn the_korean_dictionary_finds_inflected_words() {
    let path = PathBuf::from(std::env::var("DOTORI_KO_DIC").expect("set"));
    let korean = Analyzer::korean(&path).expect("loaded");
    let terms = korean.terms("내일 회의는 민지네 사무실에서 했었다");
    assert!(terms.contains(&String::from("회의")), "{terms:?}");
    assert!(terms.contains(&String::from("사무실")), "{terms:?}");
    assert!(!terms.contains(&String::from("하")), "{terms:?}");
    assert!(
        !terms.iter().any(|term| term == "는" || term == "에서"),
        "{terms:?}"
    );
}

#[test]
fn a_record_file_without_its_table_yet_is_empty() {
    let config = config();
    std::fs::create_dir_all(&config.record).expect("made");
    Connection::open(config.record.join("2026.sqlite3")).expect("made");
    assert_eq!(
        memory(config).search("회의", 10, None).expect("searched"),
        "Nothing is recorded yet."
    );
}

async fn embedding_api(failing: bool) -> String {
    let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
        .await
        .expect("bound");
    let address = listener.local_addr().expect("an address");
    tokio::spawn(async move {
        loop {
            let Ok((mut stream, _)) = listener.accept().await else {
                return;
            };
            tokio::spawn(async move {
                use tokio::io::AsyncReadExt;
                use tokio::io::AsyncWriteExt;
                let mut read = Vec::new();
                let mut buffer = [0u8; 4096];
                let body = loop {
                    let got = stream.read(&mut buffer).await.expect("read");
                    read.extend_from_slice(&buffer[..got]);
                    let text = String::from_utf8_lossy(&read).to_string();
                    if let Some((head, body)) = text.split_once("\r\n\r\n") {
                        let length: usize = head
                            .lines()
                            .find_map(|line| {
                                line.to_ascii_lowercase()
                                    .strip_prefix("content-length:")
                                    .map(|value| value.trim().parse().expect("a length"))
                            })
                            .unwrap_or(0);
                        if body.len() >= length {
                            break body.to_owned();
                        }
                    }
                };
                let refused = body.contains("거절");
                let answer = if failing || refused {
                    String::from("{\"error\":\"no\"}")
                } else {
                    let asked: Value = serde_json::from_str(&body).expect("json");
                    let data: Vec<Value> = asked["input"]
                        .as_array()
                        .expect("inputs")
                        .iter()
                        .enumerate()
                        .map(|(index, text)| {
                            let text = text.as_str().expect("text");
                            let has = |words: &[&str]| {
                                if words.iter().any(|word| text.contains(word)) {
                                    1.0
                                } else {
                                    0.0
                                }
                            };
                            json!({ "index": index, "embedding": [
                                has(&["미팅", "회의", "meeting"]),
                                has(&["점심", "lunch"]),
                                0.1
                            ]})
                        })
                        .collect();
                    json!({ "data": data }).to_string()
                };
                let status = if failing {
                    "500 Internal Server Error"
                } else if refused {
                    "400 Bad Request"
                } else {
                    "200 OK"
                };
                let response = format!(
                    "HTTP/1.1 {status}\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{answer}",
                    answer.len()
                );
                stream.write_all(response.as_bytes()).await.expect("wrote");
            });
        }
    });
    format!("http://{address}/v1")
}

fn embedded(config: MemoryConfig, base_url: String) -> Arc<Memory> {
    let embedding = EmbeddingConfig {
        base_url,
        model: String::from("test"),
        api_key: Some(String::from("key")),
        dimensions: None,
    };
    Arc::new(Memory {
        config,
        embedder: Some(Arc::new(Embedder::new(embedding).expect("made"))),
        meaning_ready: AtomicBool::new(false),
        state: Mutex::new(State::default()),
    })
}

#[tokio::test]
async fn records_are_also_found_by_their_meaning() {
    let config = config();
    record(
        &config,
        &[
            ("heard", Some("민지"), Some("내일 미팅은 3시에 해요")),
            ("tool_returned", None, None),
            ("heard", Some("유노"), Some("점심 메뉴 추천해줘")),
        ],
    );
    let memory = embedded(config.clone(), embedding_api(false).await);
    while memory.fill_once().await.expect("filled") {}
    let search = &memory.tools()[0];

    let found = search
        .call(json!({ "query": "meeting", "limit": 1 }))
        .await
        .expect("searched");
    assert!(found.starts_with("[record r0001]"), "{found}");
    let both = search
        .call(json!({ "query": "점심 meeting" }))
        .await
        .expect("searched");
    assert_eq!(both.lines().count(), 2, "{both}");
    assert!(both.starts_with("[record r0003]"), "{both}");

    record(&config, &[("acted", None, Some("회의실은 2층입니다"))]);
    while memory.fill_once().await.expect("filled again") {}
    let later = search
        .call(json!({ "query": "meeting", "limit": 2 }))
        .await
        .expect("searched");
    assert!(
        later.contains("[record r0001]") && later.contains("[record r0004]"),
        "{later}"
    );
}

#[tokio::test]
async fn a_failing_embedding_api_leaves_word_search_working() {
    let config = config();
    record(
        &config,
        &[("heard", Some("민지"), Some("내일 회의는 3시에 해요"))],
    );
    let memory = embedded(config, embedding_api(true).await);
    let refused = memory.fill_once().await.expect_err("the API is down");
    assert!(refused.contains("500"), "{refused}");
    let words = memory.tools()[0]
        .call(json!({ "query": "회의" }))
        .await
        .expect("searched");
    assert!(
        !words.contains("Only words"),
        "no vectors yet, so the query is not sent: {words}"
    );

    memory.meaning_ready.store(true, Ordering::SeqCst);

    let found = memory.tools()[0]
        .call(json!({ "query": "회의" }))
        .await
        .expect("searched");
    assert!(found.starts_with("[record r0001]"), "{found}");
    assert!(found.contains("Only words were matched"), "{found}");
}

#[tokio::test]
async fn another_model_makes_every_vector_again() {
    let config = config();
    record(
        &config,
        &[("heard", Some("민지"), Some("내일 미팅은 3시에 해요"))],
    );
    let base_url = embedding_api(false).await;
    let first = embedded(config.clone(), base_url.clone());
    while first.fill_once().await.expect("filled") {}
    assert!(!first.fill_once().await.expect("nothing left"));
    drop(first);

    let second = Arc::new(Memory {
        config,
        embedder: Some(Arc::new(
            Embedder::new(EmbeddingConfig {
                base_url,
                model: String::from("other"),
                api_key: None,
                dimensions: None,
            })
            .expect("made"),
        )),
        meaning_ready: AtomicBool::new(false),
        state: Mutex::new(State::default()),
    });
    assert!(second.fill_once().await.expect("starts over"));
    let found = second.tools()[0]
        .call(json!({ "query": "meeting" }))
        .await
        .expect("searched");
    assert!(found.starts_with("[record r0001]"), "{found}");
}

#[tokio::test]
async fn a_text_the_api_refuses_is_passed_over() {
    let config = config();
    record(
        &config,
        &[
            ("heard", Some("민지"), Some("내일 미팅은 3시에 해요")),
            ("heard", Some("민지"), Some("이건 거절될 글")),
            ("heard", Some("유노"), Some("점심 메뉴 추천해줘")),
        ],
    );
    let memory = embedded(config.clone(), embedding_api(false).await);
    while memory.fill_once().await.expect("filled") {}
    let index = open_index(&config.index).expect("index");
    let kept: Vec<i64> = index
        .prepare("SELECT seq FROM vectors ORDER BY seq")
        .expect("prepared")
        .query_map([], |row| row.get(0))
        .expect("read")
        .collect::<rusqlite::Result<_>>()
        .expect("read");
    assert_eq!(kept, [1, 3]);
}

#[test]
fn vectors_are_kept_only_for_the_current_model_and_length() {
    let config = config();
    record(&config, &[("heard", Some("민지"), Some("회의"))]);
    let mut index = open_index(&config.index).expect("index");
    copy_records(&mut index, &config.record).expect("copied");

    vectors::pending(&mut index, "a", 10).expect("pending");
    vectors::store(&mut index, "a", &[(1, vec![1.0, 0.0])], 1).expect("stored");
    let longer = vectors::store(&mut index, "a", &[(1, vec![1.0, 0.0, 0.0])], 1);
    assert!(longer.expect_err("another length").contains("length"));

    vectors::pending(&mut index, "b", 10).expect("another model");
    assert!(!vectors::any(&index, "a").expect("checked"));
    let late = vectors::store(&mut index, "a", &[(1, vec![1.0, 0.0])], 1);
    assert!(
        late.expect_err("the model changed")
            .contains("another model")
    );
}

#[test]
fn records_from_every_year_are_found_and_an_older_index_is_made_again() {
    let config = config();
    record(&config, &[("heard", Some("민지"), Some("작년 회의"))]);
    let newer = Connection::open(config.record.join("2024.sqlite3")).expect("opened");
    newer
        .execute_batch(
            "CREATE TABLE records (id TEXT PRIMARY KEY, device TEXT NOT NULL, at_ms INTEGER NOT NULL, \
             kind TEXT NOT NULL, channel TEXT, place TEXT, sender_id TEXT, sender_name TEXT, role TEXT, \
             text TEXT, line TEXT NOT NULL) WITHOUT ROWID; \
             CREATE TABLE arrived (seq INTEGER PRIMARY KEY AUTOINCREMENT, id TEXT NOT NULL UNIQUE); \
             INSERT INTO records (id, device, at_ms, kind, sender_name, role, text, line) \
             VALUES ('s0001', 'pc', 1704067200000, 'heard', '유노', 'owner', '올해 회의', '{}'); \
             INSERT INTO arrived (id) VALUES ('s0001');",
        )
        .expect("made");
    let old = Connection::open(&config.index).expect("opened");
    old.execute_batch(
        "CREATE TABLE meta (key TEXT PRIMARY KEY, value TEXT NOT NULL); \
         INSERT INTO meta VALUES ('done', '7'); \
         CREATE TABLE vectors (seq INTEGER PRIMARY KEY, model TEXT NOT NULL, vector BLOB NOT NULL); \
         INSERT INTO vectors VALUES (7, 'm', x'00000000');",
    )
    .expect("made");
    drop(old);

    let found = memory(config.clone())
        .search("회의", 10, None)
        .expect("searched");

    assert!(found.contains("[record r0001]"), "{found}");
    assert!(found.contains("[record s0001]"), "{found}");
    let index = open_index(&config.index).expect("index");
    let stale: i64 = index
        .query_row("SELECT count(*) FROM vectors", [], |row| row.get(0))
        .expect("counted");
    assert_eq!(stale, 0);
}

#[test]
fn a_record_copied_in_later_with_a_smaller_number_is_still_found() {
    let config = config();
    record(&config, &[("heard", Some("민지"), Some("첫 회의"))]);
    let memory = memory(config.clone());
    assert!(
        memory
            .search("회의", 10, None)
            .expect("searched")
            .contains("r0001")
    );
    let year = Connection::open(config.record.join("2023.sqlite3")).expect("opened");
    year.execute_batch(
        "INSERT INTO records (id, device, at_ms, kind, channel, place, sender_name, role, text, line) \
         VALUES ('a0001', 'other-pc', 1700000000000, 'heard', 'slack', 'C1', '유노', 'owner', '다른 PC 회의', '{}'); \
         INSERT INTO arrived (id) VALUES ('a0001');",
    )
    .expect("copied in");

    let found = memory.search("회의", 10, None).expect("searched");

    assert!(found.contains("[record a0001]"), "{found}");
}

#[test]
fn a_vector_from_another_pc_waits_for_its_record_and_is_not_made_again() {
    let config = config();
    record(&config, &[("heard", Some("민지"), Some("회의"))]);
    let mut index = open_index(&config.index).expect("index");
    index
        .execute(
            "INSERT INTO copied_vectors (record, model, vector) VALUES \
             ('r0001', 'm', x'0000803f'), ('r0002', 'm', x'0000803f')",
            [],
        )
        .expect("copied");

    copy_records(&mut index, &config.record).expect("copied records");
    assert!(vectors::merge_copied(&mut index).expect("merged"));

    let held: Vec<i64> = index
        .prepare("SELECT seq FROM vectors")
        .expect("prepared")
        .query_map([], |row| row.get(0))
        .expect("queried")
        .collect::<rusqlite::Result<_>>()
        .expect("read");
    assert_eq!(held, [1]);
    let waiting: i64 = index
        .query_row("SELECT count(*) FROM copied_vectors", [], |row| row.get(0))
        .expect("counted");
    assert_eq!(waiting, 1, "r0002 waits for its record");
    let logged: i64 = index
        .query_row("SELECT count(*) FROM vector_log", [], |row| row.get(0))
        .expect("counted");
    assert_eq!(logged, 1);
    let (texts, last) = vectors::pending(&mut index, "m", 10).expect("pending");
    assert!(texts.is_empty(), "a record with a vector is not sent again");
    assert_eq!(last, Some(1));
}

#[test]
fn only_the_pc_holding_the_job_makes_vectors() {
    let folder = std::env::temp_dir().join(format!("dotori-jobs-{}", std::process::id()));
    std::fs::create_dir_all(&folder).expect("folder");
    let jobs = folder.join("jobs.json");

    assert!(makes_vectors(None));
    assert!(
        makes_vectors(Some(&jobs)),
        "no file yet means this PC alone"
    );
    std::fs::write(&jobs, r#"{"vectors":false}"#).expect("written");
    assert!(!makes_vectors(Some(&jobs)));
    std::fs::write(&jobs, r#"{"vectors":true}"#).expect("written");
    assert!(makes_vectors(Some(&jobs)));
}

#[test]
fn a_copy_made_with_the_makers_newer_model_takes_the_place_of_an_older_vector() {
    let config = config();
    record(&config, &[("heard", Some("민지"), Some("회의"))]);
    let mut index = open_index(&config.index).expect("index");
    copy_records(&mut index, &config.record).expect("copied records");
    index
        .execute_batch(
            "INSERT INTO vectors (seq, model, vector) VALUES (1, 'old', x'0000803f'); \
             INSERT INTO vector_log (seq) VALUES (1); \
             INSERT INTO copied_vectors (record, model, vector) VALUES ('r0001', 'new', x'00000040');",
        )
        .expect("held");

    assert!(vectors::merge_copied(&mut index).expect("merged"));

    let mut loaded = vectors::Vectors::default();
    loaded.load(&index, "new").expect("loaded");
    assert_eq!(loaded.nearest(&[1.0], 5).expect("compared"), [1]);
    let mut old = vectors::Vectors::default();
    old.load(&index, "old").expect("loaded");
    assert!(old.nearest(&[1.0], 5).expect("compared").is_empty());
}

fn forget_in_record(config: &MemoryConfig, id: &str) {
    let record = Connection::open(config.record.join("2023.sqlite3")).expect("opened");
    record
        .execute_batch(
            "CREATE TABLE IF NOT EXISTS forgotten (n INTEGER PRIMARY KEY AUTOINCREMENT, \
             id TEXT NOT NULL UNIQUE, device TEXT NOT NULL)",
        )
        .expect("made");
    record
        .execute("INSERT INTO forgotten (id, device) VALUES (?1, 'pc')", [id])
        .expect("forgotten");
    record
        .execute("DELETE FROM records WHERE id = ?1", [id])
        .expect("removed");
    record
        .execute("DELETE FROM arrived WHERE id = ?1", [id])
        .expect("removed");
}

#[test]
fn a_forgotten_record_leaves_the_search_index() {
    let config = config();
    record(
        &config,
        &[
            ("heard", Some("민지"), Some("내 카드 번호는 1234야")),
            ("heard", Some("민지"), Some("카드 정리 끝났어")),
        ],
    );
    let memory = memory(config.clone());
    assert_eq!(
        memory
            .search("카드", 10, None)
            .expect("searched")
            .lines()
            .count(),
        2
    );

    forget_in_record(&config, "r0001");

    let found = memory.search("카드", 10, None).expect("searched");
    assert_eq!(found.lines().count(), 1, "{found}");
    assert!(found.starts_with("[record r0002]"), "{found}");
    assert_eq!(
        memory.search("1234", 10, None).expect("searched"),
        "Nothing in memory matches."
    );
    let index = Connection::open(&config.index).expect("opened");
    let kept: String = index
        .query_row("SELECT group_concat(text, ' ') FROM docs", [], |row| {
            row.get(0)
        })
        .expect("read");
    assert!(!kept.contains("1234"), "{kept}");
}

#[tokio::test]
async fn a_forgotten_record_loses_its_vector_too() {
    let config = config();
    record(
        &config,
        &[
            ("heard", Some("민지"), Some("내일 미팅은 3시에 해요")),
            ("heard", Some("유노"), Some("점심 메뉴 추천해줘")),
        ],
    );
    let memory = embedded(config.clone(), embedding_api(false).await);
    while memory.fill_once().await.expect("filled") {}
    let search = &memory.tools()[0];
    let before = search
        .call(json!({ "query": "meeting", "limit": 1 }))
        .await
        .expect("searched");
    assert!(before.starts_with("[record r0001]"), "{before}");

    forget_in_record(&config, "r0001");
    let after = search
        .call(json!({ "query": "meeting", "limit": 2 }))
        .await
        .expect("searched");

    assert!(!after.contains("r0001"), "{after}");
    let index = Connection::open(&config.index).expect("opened");
    let vectors: i64 = index
        .query_row("SELECT count(*) FROM vectors", [], |row| row.get(0))
        .expect("counted");
    assert_eq!(vectors, 1);
}
