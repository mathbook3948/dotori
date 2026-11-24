use std::time::Duration;

use dotori_core::channel::InboxClosedError;
use futures_util::SinkExt;
use futures_util::StreamExt;
use tokio::net::TcpListener;
use tokio::sync::mpsc;
use tokio::time;
use tokio_tungstenite::tungstenite::protocol::CloseFrame;

use super::*;
use crate::llm::mock;

const WAIT: Duration = Duration::from_secs(5);
const OWNER: &str = "111";
const BOT: &str = "999";

struct Collect(mpsc::UnboundedSender<Incoming>);

impl Inbox for Collect {
    fn push(&self, incoming: Incoming) -> BoxFuture<'_, Result<(), InboxClosedError>> {
        let _ = self.0.send(incoming);
        Box::pin(async { Ok(()) })
    }
}

fn message(author: &str, guild: Option<&str>, content: &str) -> Value {
    let mut message = json!({
        "id": "500",
        "channel_id": "300",
        "type": 0,
        "content": content,
        "author": { "id": author, "username": "yuno", "global_name": "Yuno" },
        "mentions": [],
    });
    if let Some(guild) = guild {
        message["guild_id"] = json!(guild);
    }
    message
}

async fn gateway(
    frames: Vec<Value>,
    close: Option<u16>,
) -> (String, mpsc::UnboundedReceiver<Value>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bound");
    let address = listener.local_addr().expect("has address");
    let (heard_tx, heard) = mpsc::unbounded_channel();
    tokio::spawn(async move {
        let (stream, _) = listener.accept().await.expect("accepted");
        let mut socket = tokio_tungstenite::accept_async(stream)
            .await
            .expect("handshake");
        socket
            .send(Message::text(
                json!({ "op": 10, "d": { "heartbeat_interval": 200 } }).to_string(),
            ))
            .await
            .expect("hello");
        let Some(Ok(Message::Text(opening))) = socket.next().await else {
            return;
        };
        let _ = heard_tx.send(serde_json::from_str(opening.as_str()).expect("json"));
        for frame in frames {
            socket
                .send(Message::text(frame.to_string()))
                .await
                .expect("sent");
        }
        if let Some(code) = close {
            let _ = socket
                .send(Message::Close(Some(CloseFrame {
                    code: code.into(),
                    reason: "".into(),
                })))
                .await;
            return;
        }
        while let Some(Ok(frame)) = socket.next().await {
            if let Message::Text(text) = frame {
                let heard: Value = serde_json::from_str(text.as_str()).expect("json");
                if heard["op"] == 1 {
                    let _ = socket
                        .send(Message::text(json!({ "op": 11 }).to_string()))
                        .await;
                }
                let _ = heard_tx.send(heard);
            }
        }
    });
    (format!("ws://{address}"), heard)
}

fn leaked(text: String) -> &'static str {
    Box::leak(text.into_boxed_str())
}

fn ready() -> Value {
    json!({ "op": 0, "s": 1, "t": "READY", "d": {
        "session_id": "s1",
        "resume_gateway_url": "ws://127.0.0.1:1",
        "user": { "id": BOT, "username": "dotori" },
    } })
}

#[test]
fn the_owners_direct_messages_are_heard_and_nobody_elses() {
    let heard = read(&message(OWNER, None, "안녕"), OWNER, BOT).expect("heard");
    assert_eq!(heard.place, "300");
    assert_eq!(heard.id, "300:500");
    assert_eq!(heard.server, None);
    assert_eq!(heard.name, "Yuno");
    assert_eq!(heard.text, "안녕");

    assert_eq!(read(&message("222", None, "안녕"), OWNER, BOT), None);
    let mut bot = message(OWNER, None, "안녕");
    bot["author"]["bot"] = json!(true);
    assert_eq!(read(&bot, OWNER, BOT), None);
    let mut hooked = message(OWNER, None, "안녕");
    hooked["webhook_id"] = json!("77");
    assert_eq!(read(&hooked, OWNER, BOT), None);
    let mut forwarded = message(OWNER, None, "");
    forwarded["message_snapshots"] = json!([{ "message": { "content": "다른 사람 말" } }]);
    assert_eq!(read(&forwarded, OWNER, BOT), None);
    let mut joined = message(OWNER, None, "");
    joined["type"] = json!(7);
    assert_eq!(read(&joined, OWNER, BOT), None);
}

#[test]
fn in_a_server_only_a_mention_or_an_answer_to_the_bot_is_heard() {
    assert_eq!(
        read(&message(OWNER, Some("1"), "그냥 말"), OWNER, BOT),
        None
    );

    let mentioned = read(
        &message(OWNER, Some("1"), "<@999> 내일 일정 <@!999>"),
        OWNER,
        BOT,
    )
    .expect("heard");
    assert_eq!(mentioned.text, "내일 일정");
    assert_eq!(mentioned.place, "300/500");
    assert_eq!(mentioned.server.as_deref(), Some("1"));

    let mut answer = message(OWNER, Some("1"), "고마워");
    answer["type"] = json!(19);
    answer["referenced_message"] = json!({ "author": { "id": BOT } });
    assert_eq!(read(&answer, OWNER, BOT).expect("heard").text, "고마워");

    let mut nick = message(OWNER, Some("1"), "<@999> 안녕");
    nick["member"] = json!({ "nick": "도토리 주인" });
    assert_eq!(read(&nick, OWNER, BOT).expect("heard").name, "도토리 주인");

    assert_eq!(
        read(&message(OWNER, Some("1"), "<@9990> 안녕"), OWNER, BOT),
        None
    );
    assert_eq!(
        read(&message(OWNER, Some("1"), "<@999> 안녕"), OWNER, ""),
        None
    );
}

#[test]
fn a_place_names_its_channel_and_the_message_to_answer() {
    assert_eq!(target(OWNER_DIRECT), Some((None, None)));
    assert_eq!(target("300"), Some((Some(String::from("300")), None)));
    assert_eq!(
        target("300/500"),
        Some((Some(String::from("300")), Some(String::from("500"))))
    );
    assert_eq!(target("../users"), None);
    assert_eq!(target("300/x"), None);
}

#[test]
fn long_replies_break_at_lines_and_blank_text_sends_nothing() {
    let line = format!("{}\n", "가".repeat(1500));
    let parts = pieces(&line.repeat(3));
    assert_eq!(parts.len(), 3);
    assert!(parts.iter().all(|part| part.chars().count() <= PIECE));
    let one = "나".repeat(4500);
    let cut = pieces(&one);
    assert_eq!(cut.len(), 3);
    assert_eq!(cut.concat(), one);
    assert!(pieces(" \n\n ").is_empty());
}

#[test]
fn the_config_hides_its_token() {
    let shown = format!("{:?}", DiscordConfig::new("secret-token", OWNER));
    assert!(!shown.contains("secret-token"));
}

#[tokio::test]
async fn the_owners_mention_reaches_the_inbox_and_heartbeats_keep_the_gateway() {
    let stranger = json!({ "op": 0, "s": 2, "t": "MESSAGE_CREATE",
        "d": message("222", Some("1"), "<@999> 나도") });
    let mention = json!({ "op": 0, "s": 3, "t": "MESSAGE_CREATE",
        "d": message(OWNER, Some("1"), "<@999> 안녕") });
    let (socket, mut heard) = gateway(vec![ready(), stranger, mention], None).await;
    let (base, mut requests) = mock::serve(vec![
        (200, leaked(format!(r#"{{"url":"{socket}"}}"#))),
        (200, r#"{"id":"1","approximate_member_count":40}"#),
    ])
    .await;
    let mut config = DiscordConfig::new("bot-token", OWNER);
    config.api_base = base;
    let channel = Arc::new(DiscordChannel::new(config).expect("built"));
    let (tx, mut rx) = mpsc::unbounded_channel();
    let running = Arc::clone(&channel);
    tokio::spawn(async move {
        let _ = running.run(Arc::new(Collect(tx))).await;
    });

    let incoming = time::timeout(WAIT, rx.recv())
        .await
        .expect("in time")
        .expect("pushed");
    assert_eq!(incoming.place.as_str(), "300/500");
    assert_eq!(incoming.sender_id.as_str(), OWNER);
    assert_eq!(incoming.sender_name, "Yuno");
    assert_eq!(incoming.content.as_str(), "안녕");
    assert_eq!(
        incoming.audience,
        Audience::Anyone(String::from("this Discord server and its 39 members"))
    );

    let identify = time::timeout(WAIT, heard.recv())
        .await
        .expect("in time")
        .expect("identify");
    assert_eq!(identify["op"], 2);
    assert_eq!(identify["d"]["token"], "bot-token");
    assert_eq!(identify["d"]["intents"], INTENTS);
    let beat = time::timeout(WAIT, heard.recv())
        .await
        .expect("in time")
        .expect("heartbeat");
    assert_eq!(beat, json!({ "op": 1, "d": 3 }));
    let again = time::timeout(WAIT, heard.recv())
        .await
        .expect("in time")
        .expect("heartbeat");
    assert_eq!(again["op"], 1);

    let gateway = requests.recv().await.expect("gateway");
    assert!(
        gateway.head.starts_with("GET /gateway/bot"),
        "{}",
        gateway.head
    );
    assert!(
        gateway
            .head
            .to_lowercase()
            .contains("authorization: bot bot-token")
    );
    let server = requests.recv().await.expect("guild");
    assert!(
        server.head.starts_with("GET /guilds/1?with_counts=true"),
        "{}",
        server.head
    );
}

#[tokio::test]
async fn a_reply_answers_the_mention_without_pinging_anyone() {
    let (base, mut requests) =
        mock::serve(vec![(200, r#"{"id":"600"}"#), (200, r#"{"id":"601"}"#)]).await;
    let mut config = DiscordConfig::new("bot-token", OWNER);
    config.api_base = base;
    let channel = DiscordChannel::new(config).expect("built");

    let text = format!("{}\n{}", "가".repeat(1500), "나".repeat(1500));
    channel
        .deliver(Place::new("300/500"), text)
        .await
        .expect("delivered");

    let first = requests.recv().await.expect("first");
    assert!(
        first.head.starts_with("POST /channels/300/messages"),
        "{}",
        first.head
    );
    let body: Value = serde_json::from_str(&first.body).expect("json");
    assert_eq!(body["message_reference"]["message_id"], "500");
    assert_eq!(body["allowed_mentions"], json!({ "parse": [] }));
    let second = requests.recv().await.expect("second");
    let body: Value = serde_json::from_str(&second.body).expect("json");
    assert!(body.get("message_reference").is_none());
}

#[tokio::test]
async fn a_private_answer_opens_the_owners_direct_messages_once() {
    let (base, mut requests) = mock::serve(vec![
        (200, r#"{"id":"777","type":1}"#),
        (200, r#"{"id":"1"}"#),
        (200, r#"{"id":"2"}"#),
    ])
    .await;
    let mut config = DiscordConfig::new("bot-token", OWNER);
    config.api_base = base;
    let channel = DiscordChannel::new(config).expect("built");

    for text in ["하나", "둘"] {
        channel
            .deliver(channel.private_place(), String::from(text))
            .await
            .expect("delivered");
    }

    let opened = requests.recv().await.expect("open");
    assert!(
        opened.head.starts_with("POST /users/@me/channels"),
        "{}",
        opened.head
    );
    assert!(opened.body.contains(r#""recipient_id":"111""#));
    for _ in 0..2 {
        let sent = requests.recv().await.expect("sent");
        assert!(
            sent.head.starts_with("POST /channels/777/messages"),
            "{}",
            sent.head
        );
    }
}

#[tokio::test]
async fn a_token_discord_refuses_ends_the_channel() {
    let (base, _requests) =
        mock::serve(vec![(401, r#"{"message":"401: Unauthorized","code":0}"#)]).await;
    let mut config = DiscordConfig::new("bad", OWNER);
    config.api_base = base;
    let channel = DiscordChannel::new(config).expect("built");
    let (tx, _rx) = mpsc::unbounded_channel();

    let ended = time::timeout(WAIT, channel.run(Arc::new(Collect(tx))))
        .await
        .expect("in time");
    assert!(ended.expect_err("ended").detail.contains("Unauthorized"));
}

#[tokio::test]
async fn a_missing_message_content_intent_ends_the_channel_and_says_what_to_turn_on() {
    let (socket, _heard) = gateway(Vec::new(), Some(4014)).await;
    let (base, _requests) =
        mock::serve(vec![(200, leaked(format!(r#"{{"url":"{socket}"}}"#)))]).await;
    let mut config = DiscordConfig::new("bot-token", OWNER);
    config.api_base = base;
    let channel = DiscordChannel::new(config).expect("built");
    let (tx, _rx) = mpsc::unbounded_channel();

    let ended = time::timeout(WAIT, channel.run(Arc::new(Collect(tx))))
        .await
        .expect("in time");
    assert!(
        ended
            .expect_err("ended")
            .detail
            .contains("Message Content Intent")
    );
}

#[tokio::test]
async fn a_gateway_that_asks_to_reconnect_is_resumed_where_it_left_off() {
    let (second, mut heard_again) = gateway(Vec::new(), None).await;
    let mut first_ready = ready();
    first_ready["d"]["resume_gateway_url"] = json!(second);
    let (first, _heard) = gateway(
        vec![
            first_ready,
            json!({ "op": 0, "s": 2, "t": "TYPING_START", "d": {} }),
            json!({ "op": 7, "d": null }),
        ],
        None,
    )
    .await;
    let (base, _requests) =
        mock::serve(vec![(200, leaked(format!(r#"{{"url":"{first}"}}"#)))]).await;
    let mut config = DiscordConfig::new("bot-token", OWNER);
    config.api_base = base;
    let channel = Arc::new(DiscordChannel::new(config).expect("built"));
    let (tx, _rx) = mpsc::unbounded_channel();
    let running = Arc::clone(&channel);
    tokio::spawn(async move {
        let _ = running.run(Arc::new(Collect(tx))).await;
    });

    let resumed = time::timeout(WAIT, heard_again.recv())
        .await
        .expect("in time")
        .expect("resume");
    assert_eq!(
        resumed,
        json!({ "op": 6, "d": { "token": "bot-token", "session_id": "s1", "seq": 2 } })
    );
}
