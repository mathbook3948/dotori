use tokio::io::AsyncBufReadExt;
use tokio::io::AsyncWriteExt;
use tokio::io::BufReader;

use super::*;

const WAIT: Duration = Duration::from_secs(5);

async fn served() -> (Arc<Screen>, mpsc::UnboundedReceiver<Ask>, u16) {
    let (screen, asks) = Screen::new();
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bound");
    let port = listener.local_addr().expect("address").port();
    tokio::spawn(Arc::clone(&screen).serve(listener, String::from("secret")));
    (screen, asks, port)
}

async fn next_line(lines: &mut BufReader<tokio::net::tcp::OwnedReadHalf>) -> Value {
    let mut line = String::new();
    time::timeout(WAIT, lines.read_line(&mut line))
        .await
        .expect("in time")
        .expect("read");
    serde_json::from_str(line.trim()).expect("json")
}

#[tokio::test]
async fn a_window_with_the_token_sees_the_state_and_frames_and_is_heard() {
    let (screen, mut asks, port) = served().await;
    screen.update(json!({ "url": "https://example.com/", "asked": "log in to the bank" }));
    let stream = TcpStream::connect(("127.0.0.1", port))
        .await
        .expect("connected");
    let (read, mut write) = stream.into_split();
    let mut lines = BufReader::new(read);
    write
        .write_all(b"{\"token\":\"secret\"}\n")
        .await
        .expect("sent");

    let state = next_line(&mut lines).await;
    assert_eq!(state["kind"], "state");
    assert_eq!(state["asked"], "log in to the bank");
    assert_eq!(
        time::timeout(WAIT, asks.recv()).await.expect("in time"),
        Some(Ask::Watch(true))
    );
    assert!(screen.watched());

    screen.frame("AAAA", 1280.0, 800.0);
    let frame = next_line(&mut lines).await;
    assert_eq!(frame["kind"], "frame");
    assert_eq!(frame["jpeg"], "AAAA");
    screen.pointer(10.0, 20.0);
    assert_eq!(next_line(&mut lines).await["kind"], "pointer");

    write
        .write_all(b"{\"kind\":\"take\"}\n{\"kind\":\"text\",\"text\":\"\xed\x95\x9c\"}\n")
        .await
        .expect("sent");
    assert_eq!(
        time::timeout(WAIT, asks.recv()).await.expect("in time"),
        Some(Ask::Take)
    );
    assert_eq!(
        time::timeout(WAIT, asks.recv()).await.expect("in time"),
        Some(Ask::Input(json!({ "kind": "text", "text": "한" })))
    );

    drop(write);
    drop(lines);
    assert_eq!(
        time::timeout(WAIT, asks.recv()).await.expect("in time"),
        Some(Ask::Watch(false))
    );
    assert!(!screen.watched());
}

#[tokio::test]
async fn a_window_without_the_token_gets_nothing() {
    let (_screen, mut asks, port) = served().await;
    let stream = TcpStream::connect(("127.0.0.1", port))
        .await
        .expect("connected");
    let (read, mut write) = stream.into_split();
    write
        .write_all(b"{\"token\":\"guess\"}\n")
        .await
        .expect("sent");
    let mut line = String::new();
    let read = time::timeout(WAIT, BufReader::new(read).read_line(&mut line))
        .await
        .expect("in time")
        .expect("read");
    assert_eq!(read, 0, "{line}");
    assert!(asks.try_recv().is_err());
}

#[test]
fn only_known_asks_are_taken() {
    assert_eq!(ask_of(r#"{"kind":"give"}"#), Some(Ask::Give));
    assert!(matches!(
        ask_of(r#"{"kind":"auth","username":"owner","password":"secret"}"#),
        Some(Ask::Input(_))
    ));
    assert_eq!(ask_of(r#"{"kind":"watch"}"#), None);
    assert_eq!(ask_of("not json"), None);
}

#[tokio::test]
async fn a_quiet_listener_sees_each_state_but_no_frames_and_is_not_a_window() {
    let (screen, mut asks, port) = served().await;
    let stream = TcpStream::connect(("127.0.0.1", port))
        .await
        .expect("connected");
    let (read, mut write) = stream.into_split();
    let mut lines = BufReader::new(read);
    write
        .write_all(b"{\"token\":\"secret\",\"quiet\":true}\n")
        .await
        .expect("sent");

    assert_eq!(next_line(&mut lines).await["mode"], "idle");
    screen.frame("AAAA", 1280.0, 800.0);
    screen.update(json!({ "card": { "id": 7, "kind": "log_in" } }));
    let state = next_line(&mut lines).await;
    assert_eq!(state["kind"], "state");
    assert_eq!(state["card"]["id"], 7);
    assert!(!screen.watched());

    write
        .write_all(
            b"{\"kind\":\"fill\",\"id\":7,\"press\":3,\"values\":[]}\n{\"kind\":\"take\"}\n{\"kind\":\"fill\",\"id\":7,\"allow\":true,\"account\":1,\"later\":true}\n",
        )
        .await
        .expect("sent");
    assert_eq!(
        time::timeout(WAIT, asks.recv()).await.expect("in time"),
        Some(Ask::Input(json!({
            "kind": "fill",
            "id": 7,
            "allow": true,
            "trust": false,
            "account": 1,
        })))
    );
    assert!(asks.try_recv().is_err());
}
