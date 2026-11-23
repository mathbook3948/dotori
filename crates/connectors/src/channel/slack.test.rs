use std::time::Duration;

use dotori_core::channel::InboxClosedError;
use futures_util::SinkExt;
use futures_util::StreamExt;
use tokio::net::TcpListener;
use tokio::sync::mpsc;
use tokio::time;

use super::*;
use crate::llm::mock;

const WAIT: Duration = Duration::from_secs(5);

struct Collect(mpsc::UnboundedSender<Incoming>);

impl Inbox for Collect {
    fn push(&self, incoming: Incoming) -> BoxFuture<'_, Result<(), InboxClosedError>> {
        let _ = self.0.send(incoming);
        Box::pin(async { Ok(()) })
    }
}

async fn socket(frames: Vec<String>) -> (String, mpsc::UnboundedReceiver<String>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bound");
    let address = listener.local_addr().expect("has address");
    let (heard_tx, heard) = mpsc::unbounded_channel();
    tokio::spawn(async move {
        let (stream, _) = listener.accept().await.expect("accepted");
        let mut socket = tokio_tungstenite::accept_async(stream)
            .await
            .expect("handshake");
        for frame in frames {
            socket.send(Message::text(frame)).await.expect("sent");
        }
        while let Some(Ok(frame)) = socket.next().await {
            if let Message::Text(text) = frame {
                let _ = heard_tx.send(text.as_str().to_owned());
            }
        }
    });
    (format!("ws://{address}"), heard)
}

fn leaked(text: String) -> &'static str {
    Box::leak(text.into_boxed_str())
}

#[tokio::test]
async fn the_owners_mention_reaches_the_inbox_with_their_name_and_is_acknowledged() {
    let stranger = r#"{"type":"events_api","envelope_id":"e0","payload":{"event":{"type":"app_mention","channel":"C1","user":"U2","text":"<@UBOT> 나도","ts":"10.4"}}}"#;
    let mention = r#"{"type":"events_api","envelope_id":"e1","payload":{"event":{"type":"app_mention","channel":"C1","user":"U1","text":"<@UBOT> 안녕","ts":"10.5"}}}"#;
    let (socket_url, mut heard) = socket(vec![
        String::from(r#"{"type":"hello"}"#),
        String::from(stranger),
        String::from(mention),
    ])
    .await;
    let (base, mut requests) = mock::serve(vec![
        (200, r#"{"ok":true,"user_id":"UBOT"}"#),
        (200, leaked(format!(r#"{{"ok":true,"url":"{socket_url}"}}"#))),
        (
            200,
            r#"{"ok":true,"user":{"name":"yuno","real_name":"Yuno","profile":{"display_name":""}}}"#,
        ),
    ])
    .await;
    let mut config = SlackConfig::new("xapp-1", "xoxb-1", "U1");
    config.api_base = base;
    let channel = Arc::new(SlackChannel::new(config).expect("built"));
    let (tx, mut rx) = mpsc::unbounded_channel();
    let running = Arc::clone(&channel);
    tokio::spawn(async move {
        let _ = running.run(Arc::new(Collect(tx))).await;
    });

    let incoming = time::timeout(WAIT, rx.recv())
        .await
        .expect("in time")
        .expect("pushed");
    assert_eq!(incoming.id.as_str(), "C1/10.5");
    assert_eq!(incoming.place.as_str(), "C1/10.5");
    assert_eq!(incoming.sender_id.as_str(), "U1");
    assert_eq!(incoming.sender_name, "Yuno");
    assert_eq!(incoming.content.as_str(), "안녕");
    for envelope in ["e0", "e1"] {
        let ack = time::timeout(WAIT, heard.recv()).await.expect("in time");
        assert_eq!(
            ack.as_deref(),
            Some(format!(r#"{{"envelope_id":"{envelope}"}}"#).as_str())
        );
    }

    let auth = requests.recv().await.expect("auth.test");
    assert!(auth.head.starts_with("POST /auth.test"), "{}", auth.head);
    assert!(
        auth.head
            .to_lowercase()
            .contains("authorization: bearer xoxb-1")
    );
    let open = requests.recv().await.expect("apps.connections.open");
    assert!(open.head.starts_with("POST /apps.connections.open"));
    assert!(
        open.head
            .to_lowercase()
            .contains("authorization: bearer xapp-1")
    );
    let info = requests.recv().await.expect("users.info");
    assert!(
        info.head.starts_with("GET /users.info?user=U1"),
        "{}",
        info.head
    );
}

#[tokio::test]
async fn a_reply_goes_to_the_thread_of_its_place() {
    let (base, mut requests) = mock::serve(vec![(200, r#"{"ok":true}"#)]).await;
    let mut config = SlackConfig::new("xapp-1", "xoxb-1", "U1");
    config.api_base = base;
    let channel = SlackChannel::new(config).expect("built");
    channel
        .deliver(Place::new("C1/10.5"), String::from("네"))
        .await
        .expect("delivered");
    let post = requests.recv().await.expect("chat.postMessage");
    assert!(post.head.starts_with("POST /chat.postMessage"));
    let body: serde_json::Value = serde_json::from_str(&post.body).expect("json");
    assert_eq!(
        body,
        serde_json::json!({ "channel": "C1", "thread_ts": "10.5", "text": "네" })
    );
}

#[tokio::test]
async fn a_refused_reply_is_a_delivery_error() {
    let (base, _requests) =
        mock::serve(vec![(200, r#"{"ok":false,"error":"channel_not_found"}"#)]).await;
    let mut config = SlackConfig::new("xapp-1", "xoxb-1", "U1");
    config.api_base = base;
    let channel = SlackChannel::new(config).expect("built");
    let error = channel
        .deliver(Place::new("D1"), String::from("hi"))
        .await
        .expect_err("refused");
    assert_eq!(error.detail, "chat.postMessage failed: channel_not_found");
}

#[tokio::test]
async fn a_revoked_token_ends_the_channel() {
    let (base, _requests) =
        mock::serve(vec![(200, r#"{"ok":false,"error":"invalid_auth"}"#)]).await;
    let mut config = SlackConfig::new("xapp-1", "xoxb-1", "U1");
    config.api_base = base;
    let channel = SlackChannel::new(config).expect("built");
    let (tx, _rx) = mpsc::unbounded_channel();
    let ended = time::timeout(WAIT, channel.run(Arc::new(Collect(tx))))
        .await
        .expect("in time");
    assert_eq!(
        ended.expect_err("fatal").detail,
        "auth.test failed: invalid_auth"
    );
}

#[test]
fn the_config_hides_its_tokens() {
    let shown = format!("{:?}", SlackConfig::new("xapp-secret", "xoxb-secret", "U1"));
    assert!(!shown.contains("secret"), "{shown}");
}
