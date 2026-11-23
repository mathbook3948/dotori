use super::*;

fn index(name: &str) -> PathBuf {
    let folder = std::env::temp_dir().join(format!("dotori-vectors-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&folder);
    std::fs::create_dir_all(&folder).expect("folder");
    folder.join("search.sqlite3")
}

#[test]
fn vectors_go_in_the_order_they_were_made_and_wait_on_the_other_pc() {
    let maker = index("maker");
    let connection = Connection::open(&maker).expect("opened");
    connection
        .execute_batch(
            "CREATE TABLE docs (seq INTEGER PRIMARY KEY, record TEXT NOT NULL UNIQUE); \
             CREATE TABLE vectors (seq INTEGER PRIMARY KEY, model TEXT NOT NULL, vector BLOB NOT NULL); \
             CREATE TABLE vector_log (n INTEGER PRIMARY KEY AUTOINCREMENT, seq INTEGER NOT NULL); \
             INSERT INTO docs VALUES (1, 'r1'), (2, 'r2'); \
             INSERT INTO vectors VALUES (2, 'm', x'0000803f'), (1, 'm', x'00000040'); \
             INSERT INTO vector_log (seq) VALUES (2), (1);",
        )
        .expect("made");

    let (sent, upto) = made_after(&maker, 0).expect("read");
    let records: Vec<&str> = sent.iter().map(|one| one.record.as_str()).collect();
    assert_eq!(records, ["r2", "r1"]);
    assert_eq!(upto, 2);
    assert!(made_after(&maker, 2).expect("read").0.is_empty());

    let other = index("other");
    assert_eq!(mark(&other, "maker-key"), 0);
    keep(&other, "maker-key", &sent, upto).expect("kept");
    assert_eq!(mark(&other, "maker-key"), 2);
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
fn an_index_not_made_yet_has_nothing_to_give() {
    let empty = index("empty");
    assert_eq!(made_after(&empty, 0).expect("read"), (Vec::new(), 0));
}
