use tokio::net::TcpListener;

use super::*;

async fn pair() -> (TcpStream, TcpStream) {
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bound");
    let address = listener.local_addr().expect("address");
    let (dialed, answered) = tokio::join!(TcpStream::connect(address), listener.accept());
    (dialed.expect("dialed"), answered.expect("answered").0)
}

fn hello(device: &str) -> Hello {
    Hello {
        device: device.to_owned(),
        name: device.to_uppercase(),
        address: None,
    }
}

#[tokio::test]
async fn approved_pcs_link_and_carry_messages_longer_than_a_frame() {
    let a = Identity::generate().expect("a");
    let b = Identity::generate().expect("b");
    let ha = hello("a");
    let hb = hello("b");
    let (dialed, answered) = pair().await;
    let b_key = b.public;
    let a_key = a.public;

    let (dial, answer) = tokio::join!(
        dial(dialed, &a, &b_key, Start::Link, &ha),
        answer(answered, &b, |key| *key == a_key, || None, &hb),
    );
    let dial = dial.expect("dialed");
    let (answer, start) = answer.expect("answered");

    assert_eq!(start, Start::Link);
    assert_eq!(dial.hello.device, "b");
    assert_eq!(answer.hello.device, "a");
    assert_eq!(answer.key, a_key);
    let (a_send, a_sent) = outbox();
    let (a_heard, _a_hearing) = mpsc::unbounded_channel();
    let (_b_send, b_sent) = outbox();
    let (b_heard, mut b_hearing) = mpsc::unbounded_channel();
    tokio::spawn(dial.carry(a_sent, a_heard));
    tokio::spawn(answer.carry(b_sent, b_heard));
    let long = "도토리".repeat(60_000);
    assert!(
        a_send.put(encode(&serde_json::json!({ "kind": "note", "text": long })).expect("encoded"))
    );
    let heard = b_hearing.recv().await.expect("heard");
    assert_eq!(heard["text"].as_str().map(str::len), Some(long.len()));
}

#[tokio::test]
async fn a_pc_the_owner_has_not_approved_is_refused() {
    let a = Identity::generate().expect("a");
    let b = Identity::generate().expect("b");
    let ha = hello("a");
    let hb = hello("b");
    let (dialed, answered) = pair().await;
    let b_key = b.public;

    let (_, answer) = tokio::join!(
        dial(dialed, &a, &b_key, Start::Link, &ha),
        answer(answered, &b, |_| false, || None, &hb),
    );

    assert!(answer.expect_err("refused").contains("not approved"));
}

#[tokio::test]
async fn a_dialer_that_has_the_wrong_key_for_the_other_pc_fails() {
    let a = Identity::generate().expect("a");
    let b = Identity::generate().expect("b");
    let ha = hello("a");
    let hb = hello("b");
    let wrong = Identity::generate().expect("wrong").public;
    let (dialed, answered) = pair().await;

    let (dial, answer) = tokio::join!(
        dial(dialed, &a, &wrong, Start::Link, &ha),
        answer(answered, &b, |_| true, || None, &hb),
    );

    assert!(dial.is_err());
    assert!(answer.is_err());
}

#[tokio::test]
async fn joining_needs_the_invites_secret() {
    let a = Identity::generate().expect("a");
    let b = Identity::generate().expect("b");
    let ha = hello("a");
    let hb = hello("b");
    let b_key = b.public;
    let secret = [3u8; SECRET];

    let (dialed, answered) = pair().await;
    let (dial_ok, answer_ok) = tokio::join!(
        dial(dialed, &a, &b_key, Start::Join(secret), &ha),
        answer(answered, &b, |_| false, || Some(secret), &hb),
    );
    assert!(dial_ok.is_ok());
    assert_eq!(answer_ok.expect("joined").1, Start::Join(secret));

    let (dialed, answered) = pair().await;
    let (_, answer_wrong) = tokio::join!(
        dial(dialed, &a, &b_key, Start::Join([4u8; SECRET]), &ha),
        answer(answered, &b, |_| false, || Some(secret), &hb),
    );
    assert!(answer_wrong.expect_err("refused").contains("handshake"));

    let (dialed, answered) = pair().await;
    let (_, answer_none) = tokio::join!(
        dial(dialed, &a, &b_key, Start::Join(secret), &ha),
        answer(answered, &b, |_| false, || None, &hb),
    );
    assert!(answer_none.expect_err("no invite").contains("no invite"));
}

#[test]
fn an_outbox_nobody_empties_takes_no_more_than_its_bytes_and_then_refuses_everything() {
    let (outbox, _outgoing) = outbox();
    let mebibyte: Arc<[u8]> = vec![b'1'; 1024 * 1024].into();
    let fits = QUEUE_BYTES / mebibyte.len();

    for _ in 0..fits {
        assert!(outbox.put(Arc::clone(&mebibyte)));
    }

    assert!(!outbox.put(Arc::clone(&mebibyte)));
    assert!(!outbox.put(Arc::from(&b"1"[..])));
}

#[test]
fn an_outbox_nobody_empties_takes_no_more_than_its_count() {
    let (outbox, _outgoing) = outbox();
    let small: Arc<[u8]> = Arc::from(&b"1"[..]);

    for _ in 0..QUEUE_COUNT {
        assert!(outbox.put(Arc::clone(&small)));
    }

    assert!(!outbox.put(small));
}

#[test]
fn a_message_longer_than_a_line_carries_is_not_encoded() {
    let long = "1".repeat(MESSAGE_LONGEST);

    assert!(encode(&serde_json::json!(long)).is_err());
    assert!(encode(&serde_json::json!({ "kind": "note" })).is_ok());
}
