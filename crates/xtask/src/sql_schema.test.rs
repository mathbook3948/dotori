use super::*;

fn file(path: &str, source: &str) -> (String, String) {
    (path.to_owned(), source.to_owned())
}

#[test]
fn tables_and_their_columns_are_read_from_rust_strings() {
    let source = "const VERSION: i64 = 1;\nconst SCHEMA: &str = \"CREATE TABLE IF NOT EXISTS notes (\\\n    id INTEGER PRIMARY KEY AUTOINCREMENT, at_ms INTEGER NOT NULL, text TEXT NOT NULL); \\\n    CREATE VIRTUAL TABLE IF NOT EXISTS terms USING fts5(words, tokenize = 'unicode61');\";";

    let tables = tables_in(source);

    assert_eq!(tables.len(), 2);
    assert_eq!(tables[0].name, "notes");
    assert_eq!(
        tables[0]
            .columns
            .iter()
            .map(|column| (column.name.as_str(), column.kind.as_str(), column.primary))
            .collect::<Vec<_>>(),
        [
            ("id", "INTEGER", true),
            ("at_ms", "INTEGER", false),
            ("text", "TEXT", false)
        ]
    );
    assert_eq!(tables[1].name, "terms");
    assert_eq!(tables[1].columns.len(), 1);
}

#[test]
fn a_schema_that_keeps_the_rules_passes() {
    let source = "const VERSION: i64 = 2;\nconst SCHEMA: &str = \"\nCREATE TABLE IF NOT EXISTS meta (key TEXT PRIMARY KEY, value TEXT NOT NULL);\nCREATE TABLE IF NOT EXISTS copied (\n    record TEXT NOT NULL,\n    seq INTEGER NOT NULL,\n    made_ms INTEGER NOT NULL,\n    PRIMARY KEY (record, seq)\n);\n\";";

    assert_eq!(
        violations(&[file("crates/a/src/a.rs", source)]),
        Vec::<String>::new()
    );
}

#[test]
fn each_broken_rule_is_named() {
    let source = "const SCHEMA: &str = \"\nCREATE TABLE IF NOT EXISTS Notes (\n    createdAt INTEGER,\n    sent_at INTEGER,\n    seen_ms TEXT,\n    sender TEXT,\n    record INTEGER,\n    owner INTEGER REFERENCES people(id)\n);\nCREATE TABLE IF NOT EXISTS meta (name TEXT PRIMARY KEY, value TEXT);\n\";";

    let found = violations(&[file("crates/a/src/a.rs", source)]);

    assert_eq!(
        found,
        [
            "crates/a/src/a.rs: makes tables but has no `const VERSION: i64` for PRAGMA user_version",
            "crates/a/src/a.rs: table Notes: name is not lowercase snake_case",
            "crates/a/src/a.rs: table Notes: uses a foreign key; refer to records by their id",
            "crates/a/src/a.rs: table Notes: column createdAt is not lowercase snake_case",
            "crates/a/src/a.rs: table Notes: column sent_at holds a time; name it with `_ms` and keep Unix milliseconds",
            "crates/a/src/a.rs: table Notes: column seen_ms holds milliseconds and must be INTEGER",
            "crates/a/src/a.rs: table Notes: column sender is ambiguous; use sender_id or sender_name",
            "crates/a/src/a.rs: table Notes: column record must be TEXT",
            "crates/a/src/a.rs: table meta: must be (key TEXT PRIMARY KEY, value ...)",
        ]
    );
}

#[test]
fn a_table_made_in_two_files_is_reported_but_meta_is_not() {
    let source = "const VERSION: i64 = 1;\nconst SCHEMA: &str = \"CREATE TABLE IF NOT EXISTS meta (key TEXT PRIMARY KEY, value INTEGER NOT NULL); CREATE TABLE IF NOT EXISTS dropped (record TEXT PRIMARY KEY) WITHOUT ROWID;\";";

    let found = violations(&[
        file("crates/a/src/a.rs", source),
        file("crates/b/src/b.rs", source),
    ]);

    assert_eq!(
        found,
        ["table dropped is made in more than one file: crates/a/src/a.rs, crates/b/src/b.rs"]
    );
}

#[test]
fn lowercase_split_and_temporary_tables_are_read_and_comments_are_not() {
    let source = "/// CREATE TABLE is how a file starts.\n// create table notes (a TEXT)\nconst SCHEMA: &str = \"create temp table\nIF NOT EXISTS\nlater (kind TEXT CHECK (kind = 'a'), seen_at INTEGER)\";";

    let tables = tables_in(source);

    assert_eq!(tables.len(), 1);
    assert_eq!(tables[0].name, "later");
    assert_eq!(
        tables[0]
            .columns
            .iter()
            .map(|column| column.name.as_str())
            .collect::<Vec<_>>(),
        ["kind", "seen_at"]
    );
}
