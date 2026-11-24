use super::*;

const SCHEMA: &str =
    "CREATE TABLE IF NOT EXISTS notes (id INTEGER PRIMARY KEY, text TEXT NOT NULL)";

fn folder(name: &str) -> std::path::PathBuf {
    let folder =
        std::env::temp_dir().join(format!("dotori-versioned-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&folder);
    folder
}

fn version(path: &Path) -> i64 {
    Connection::open(path)
        .expect("opened")
        .pragma_query_value(None, "user_version", |row| row.get(0))
        .expect("read")
}

#[test]
fn a_new_file_gets_its_tables_and_format() {
    let path = folder("new").join("notes.sqlite3");

    let opened = open(&path, SCHEMA, 1).expect("opened");
    opened
        .execute("INSERT INTO notes (text) VALUES ('a')", [])
        .expect("inserted");

    assert_eq!(version(&path), 1);
}

#[test]
fn a_file_from_before_formats_were_marked_keeps_what_it_holds() {
    let path = folder("old").join("notes.sqlite3");
    std::fs::create_dir_all(path.parent().expect("a folder")).expect("made");
    let old = Connection::open(&path).expect("opened");
    old.execute_batch(SCHEMA).expect("made");
    old.execute("INSERT INTO notes (text) VALUES ('kept')", [])
        .expect("inserted");
    drop(old);

    let opened = open(&path, SCHEMA, 1).expect("opened");
    let kept: String = opened
        .query_row("SELECT text FROM notes", [], |row| row.get(0))
        .expect("read");

    assert_eq!(kept, "kept");
    assert_eq!(version(&path), 1);
}

#[test]
fn a_file_a_newer_dotori_wrote_is_refused() {
    let path = folder("newer").join("notes.sqlite3");
    open(&path, SCHEMA, 2).expect("opened");

    let refused = open(&path, SCHEMA, 1).expect_err("refused");

    assert!(refused.contains("newer dotori (format 2)"), "{refused}");
}
