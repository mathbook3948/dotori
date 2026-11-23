use super::*;

fn index(name: &str) -> PathBuf {
    let folder = std::env::temp_dir().join(format!("dotori-vectors-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&folder);
    std::fs::create_dir_all(&folder).expect("folder");
    folder.join("search.sqlite3")
}

fn maker(name: &str, index_id: &str) -> PathBuf {
    let path = index(name);
    Connection::open(&path)
        .expect("opened")
        .execute_batch(&format!(
            "CREATE TABLE meta (key TEXT PRIMARY KEY, value TEXT NOT NULL); \
             INSERT INTO meta VALUES ('index_id', '{index_id}'); \
             CREATE TABLE docs (seq INTEGER PRIMARY KEY, record TEXT NOT NULL UNIQUE); \
             CREATE TABLE vectors (seq INTEGER PRIMARY KEY, model TEXT NOT NULL, vector BLOB NOT NULL); \
             CREATE TABLE vector_log (n INTEGER PRIMARY KEY AUTOINCREMENT, seq INTEGER NOT NULL); \
             INSERT INTO docs VALUES (1, 'r1'), (2, 'r2'); \
             INSERT INTO vectors VALUES (2, 'm', x'0000803f'), (1, 'm', x'00000040'); \
             INSERT INTO vector_log (seq) VALUES (2), (1);"
        ))
        .expect("made");
    path
}

#[test]
fn vectors_go_in_the_order_they_were_made_and_wait_on_the_other_pc() {
    let maker = maker("maker", "first");

    let (sent, upto, name) = made_after(&maker, "", 0).expect("read");
    let records: Vec<&str> = sent.iter().map(|one| one.record.as_str()).collect();
    assert_eq!(records, ["r2", "r1"]);
    assert_eq!((upto, name.as_str()), (2, "first"));
    assert!(made_after(&maker, "first", 2).expect("read").0.is_empty());

    let other = index("other");
    assert_eq!(mark(&other, "maker-key"), (0, String::new()));
    keep(&other, "maker-key", &sent, &name, upto).expect("kept");
    assert_eq!(mark(&other, "maker-key"), (2, String::from("first")));
    let waiting: Vec<(String, Vec<u8>)> = Connection::open(&other)
        .expect("opened")
        .prepare("SELECT record, vector FROM copied_vectors ORDER BY record")
        .expect("prepared")
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
        .expect("queried")
        .collect::<rusqlite::Result<_>>()
        .expect("read");
    assert_eq!(
        waiting,
        [
            (String::from("r1"), vec![0, 0, 0, 0x40]),
            (String::from("r2"), vec![0, 0, 0x80, 0x3f]),
        ]
    );
}

#[test]
fn a_maker_whose_index_was_made_again_sends_from_the_first_vector() {
    let maker = maker("remade", "second");

    let (sent, upto, name) = made_after(&maker, "first", 50_000).expect("read");

    assert_eq!(sent.len(), 2);
    assert_eq!((upto, name.as_str()), (2, "second"));
}

#[test]
fn a_vector_that_does_not_read_keeps_the_mark_where_it_was() {
    let other = index("bad");
    let bad = Sent {
        record: String::from("r1"),
        model: String::from("m"),
        vector: String::from("!!"),
    };

    assert!(keep(&other, "maker-key", &[bad], "first", 9).is_err());
    assert_eq!(mark(&other, "maker-key").0, 0);
}

#[test]
fn an_index_not_made_yet_has_nothing_to_give() {
    let empty = index("empty");
    assert_eq!(
        made_after(&empty, "", 0).expect("read"),
        (Vec::new(), 0, String::new())
    );
}

#[test]
fn jobs_are_written_whole() {
    let folder = index("jobs");
    let jobs = folder.with_file_name("jobs.json");
    write_job(&jobs, "vectors", false);
    write_job(&jobs, "browser", true);
    let written: Value =
        serde_json::from_str(&std::fs::read_to_string(&jobs).expect("read")).expect("json");
    assert_eq!(written, json!({ "browser": true, "vectors": false }));
    assert!(!jobs.with_extension("json.new").exists());
}
