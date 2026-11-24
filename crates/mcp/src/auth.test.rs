use serde_json::json;

use super::*;

fn asked(logins: &mut Challenges<()>, request: &str, origin: &str) -> u64 {
    logins.ask(
        (),
        json!(request),
        Some(String::from("s")),
        origin.to_owned(),
        json!("realm"),
    )
}

#[test]
fn each_login_waits_under_its_own_number_oldest_shown_first() {
    let mut logins = Challenges::default();
    let first = asked(&mut logins, "r1", "https://a.example");
    let second = asked(&mut logins, "r2", "https://b.example");

    assert_ne!(first, second);
    assert_eq!(logins.shown()["id"], first);
    assert_eq!(logins.shown()["origin"], "https://a.example");

    let taken = logins.take(first).expect("waiting");
    assert_eq!(taken.request, json!("r1"));
    assert_eq!(logins.shown()["id"], second);
    assert!(logins.take(first).is_none());
    assert!(logins.take(second).is_some());
    assert!(logins.shown().is_null());
}

#[test]
fn a_number_never_comes_back_so_an_old_answer_misses_a_new_login() {
    let mut logins = Challenges::default();
    let first = asked(&mut logins, "r1", "https://a.example");
    logins.take(first);
    let next = asked(&mut logins, "r1", "https://a.example");

    assert_ne!(first, next);
    assert!(logins.take(first).is_none());
    assert!(logins.take(next).is_some());
}

#[test]
fn the_same_request_asking_after_an_answer_is_marked_again() {
    let mut logins = Challenges::default();
    let first = asked(&mut logins, "r1", "https://a.example");
    assert_eq!(logins.shown()["again"], false);
    logins.take(first);

    asked(&mut logins, "r1", "https://a.example");
    assert_eq!(logins.shown()["again"], true);
    logins.clear();
    assert!(logins.shown().is_null());
}

#[test]
fn what_the_window_sees_names_no_request_or_session() {
    let mut logins = Challenges::default();
    asked(&mut logins, "r1", "https://a.example");

    let shown = logins.shown();
    assert!(shown.get("request").is_none());
    assert!(shown.get("session").is_none());
}
