use super::*;

#[test]
fn an_answer_becomes_unit_vectors_in_the_order_sent() {
    let answer = json!({ "data": [
        { "index": 1, "embedding": [0.0, 2.0] },
        { "index": 0, "embedding": [3.0, 4.0] }
    ]});
    assert_eq!(
        vectors(&answer, 2).expect("read"),
        [vec![0.6, 0.8], vec![0.0, 1.0]]
    );
}

#[test]
fn a_short_or_broken_answer_is_refused() {
    let short = json!({ "data": [{ "index": 0, "embedding": [1.0] }] });
    assert!(vectors(&short, 2).is_err());
    let zero = json!({ "data": [{ "index": 0, "embedding": [0.0, 0.0] }] });
    assert!(vectors(&zero, 1).is_err());
    let uneven = json!({ "data": [
        { "index": 0, "embedding": [1.0] },
        { "index": 1, "embedding": [1.0, 1.0] }
    ]});
    assert!(vectors(&uneven, 2).is_err());
    assert!(vectors(&json!({ "error": "no" }), 1).is_err());
    let beyond = json!({ "data": [{ "index": 5, "embedding": [1.0] }] });
    assert!(vectors(&beyond, 1).is_err());
}

#[test]
fn a_vector_survives_its_stored_bytes() {
    let vector = vec![0.25, -1.5, 3.0];
    assert_eq!(from_bytes(&to_bytes(&vector)), vector);
}

#[test]
fn the_name_changes_with_the_model_and_its_size() {
    let config = |dimensions| EmbeddingConfig {
        base_url: String::from("https://api.example.com/v1/"),
        model: String::from("small"),
        api_key: None,
        dimensions,
    };
    let plain = Embedder::new(Embedding::Api(config(None)))
        .expect("made")
        .name();
    let sized = Embedder::new(Embedding::Api(config(Some(256))))
        .expect("made")
        .name();
    assert_eq!(plain, "https://api.example.com/v1 small");
    assert_eq!(sized, "https://api.example.com/v1 small 256");
}
