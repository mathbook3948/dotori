use super::*;

#[test]
fn fused_lists_put_records_high_in_both_first() {
    assert_eq!(fuse(&[vec![1, 2, 3], vec![3, 4, 1]], 3), [1, 3, 2]);
    assert_eq!(fuse(&[vec![], vec![7, 8]], 1), [7]);
}

#[test]
fn the_nearest_records_come_first() {
    let index = Connection::open_in_memory().expect("opened");
    index
        .execute_batch(
            "CREATE TABLE vectors (seq INTEGER PRIMARY KEY, model TEXT NOT NULL, vector BLOB NOT NULL); \
             CREATE TABLE meta (key TEXT PRIMARY KEY, value TEXT NOT NULL); \
             CREATE TABLE vector_log (n INTEGER PRIMARY KEY AUTOINCREMENT, seq INTEGER NOT NULL);",
        )
        .expect("made");
    let put = |seq: i64, model: &str, vector: &[f32]| {
        index
            .execute(
                "INSERT INTO vectors (seq, model, vector) VALUES (?1, ?2, ?3)",
                params![seq, model, to_bytes(vector)],
            )
            .expect("put");
        index
            .execute("INSERT INTO vector_log (seq) VALUES (?1)", [seq])
            .expect("logged");
    };
    put(1, "a", &[1.0, 0.0]);
    put(2, "a", &[0.0, 1.0]);
    put(3, "b", &[1.0, 0.0]);
    let mut vectors = Vectors::default();
    vectors.load(&index, "a").expect("loaded");
    assert_eq!(vectors.nearest(&[0.6, 0.8], 5).expect("compared"), [2, 1]);
    assert!(vectors.nearest(&[1.0, 0.0, 0.0], 5).is_err());

    put(4, "a", &[0.8, 0.6]);
    vectors.load(&index, "a").expect("loaded again");
    assert_eq!(vectors.nearest(&[1.0, 0.0], 2).expect("compared"), [1, 4]);

    vectors.load(&index, "b").expect("another model");
    assert_eq!(vectors.nearest(&[0.0, 1.0], 5).expect("compared"), [3]);
}
