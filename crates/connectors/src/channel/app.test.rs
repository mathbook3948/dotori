use std::time::Duration;

use dotori_core::channel::InboxClosedError;
use dotori_core::message::Address;
use tokio::io::Lines;
use tokio::net::tcp::OwnedReadHalf;
use tokio::net::tcp::OwnedWriteHalf;
use tokio::sync::mpsc;
use tokio::time;

use super::*;

const WAIT: Duration = Duration::from_secs(5);

struct Collect(mpsc::UnboundedSender<Incoming>);

impl Inbox for Collect {
    fn push(&self, incoming: Incoming) -> BoxFuture<'_, Result<(), InboxClosedError>> {
        let _ = self.0.send(incoming);
        Box::pin(async { Ok(()) })
    }
}

async fn start() -> (
    Arc<AppChannel>,
    SocketAddr,
    mpsc::UnboundedReceiver<Incoming>,
) {
    let (channel, address) = AppChannel::bind("secret").expect("bound");
    let channel = Arc::new(channel);
    let (tx, rx) = mpsc::unbounded_channel();
    let running = Arc::clone(&channel);
    tokio::spawn(async move {
        let _ = running.run(Arc::new(Collect(tx))).await;
    });
    (channel, address, rx)
}

struct Window {
    lines: Lines<BufReader<OwnedReadHalf>>,
    write: OwnedWriteHalf,
}

impl Window {
    async fn open(address: SocketAddr, token: &str) -> Self {
        let stream = TcpStream::connect(address).await.expect("connected");
        let (read, write) = stream.into_split();
        let mut window = Self {
            lines: BufReader::new(read).lines(),
            write,
        };
        let hello = serde_json::to_string(&Hello {
            token: token.to_owned(),
        })
        .expect("json");
        send(&mut window.write, &hello).await.expect("sent");
        window
    }

    async fn say(&mut self, text: &str) {
        let line = serde_json::to_string(&FromWindow::Say {
            text: text.to_owned(),
        })
        .expect("json");
        send(&mut self.write, &line).await.expect("sent");
    }

    async fn next(&mut self) -> Option<ToWindow> {
        let line = time::timeout(WAIT, self.lines.next_line())
            .await
            .expect("in time")
            .ok()??;
        Some(serde_json::from_str(&line).expect("event"))
    }
}

#[tokio::test]
async fn what_the_owner_writes_reaches_the_inbox_and_every_window() {
    let (_channel, address, mut inbox) = start().await;
    let mut first = Window::open(address, "secret").await;
    let mut second = Window::open(address, "secret").await;
    time::sleep(Duration::from_millis(50)).await;

    first.say("hello").await;

    let incoming = time::timeout(WAIT, inbox.recv())
        .await
        .expect("in time")
        .expect("pushed");
    assert_eq!(incoming.content.as_str(), "hello");
    assert_eq!(incoming.place.as_str(), MAIN);
    assert_eq!(incoming.sender_id.as_str(), OWNER);
    let echo = ToWindow::Owner {
        text: String::from("hello"),
    };
    assert_eq!(first.next().await, Some(echo.clone()));
    assert_eq!(second.next().await, Some(echo));
}

#[tokio::test]
async fn replies_and_traffic_reach_windows_and_a_late_window_gets_the_backlog() {
    let (channel, address, _inbox) = start().await;

    channel
        .deliver(Place::new(MAIN), String::from("hi there"))
        .await
        .expect("delivered");
    channel
        .show(Traffic {
            address: Address {
                channel: ChannelId::parse("slack").expect("valid"),
                place: Place::new("C1"),
            },
            who: String::from("Alice"),
            text: String::from("ping"),
        })
        .await
        .expect("shown");
    let mut late = Window::open(address, "secret").await;

    assert_eq!(
        late.next().await,
        Some(ToWindow::Reply {
            text: String::from("hi there")
        })
    );
    assert_eq!(
        late.next().await,
        Some(ToWindow::Traffic {
            channel: String::from("slack"),
            place: String::from("C1"),
            who: String::from("Alice"),
            text: String::from("ping"),
        })
    );
}

#[tokio::test]
async fn a_window_with_the_wrong_token_is_hung_up_on() {
    let (channel, address, mut inbox) = start().await;
    channel
        .deliver(Place::new(MAIN), String::from("private"))
        .await
        .expect("delivered");
    let mut stranger = Window::open(address, "guess").await;

    stranger.say("let me in").await;

    assert_eq!(stranger.next().await, None);
    assert!(inbox.try_recv().is_err());
}

#[tokio::test]
async fn message_ids_never_repeat() {
    let (channel, _address, _inbox) = start().await;

    let first = channel.hub.message_id();
    let second = channel.hub.message_id();

    assert_ne!(first, second);
}
