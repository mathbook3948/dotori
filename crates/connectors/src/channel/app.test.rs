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
        loop {
            match self.next_raw().await? {
                ToWindow::Live | ToWindow::Working { .. } => continue,
                event => return Some(event),
            }
        }
    }

    async fn next_raw(&mut self) -> Option<ToWindow> {
        let line = time::timeout(WAIT, self.lines.next_line())
            .await
            .expect("in time")
            .ok()??;
        Some(serde_json::from_str(&line).expect("event"))
    }

    async fn next_untimed(&mut self) -> Option<ToWindow> {
        self.next().await.map(untimed)
    }
}

fn untimed(line: ToWindow) -> ToWindow {
    match line {
        ToWindow::Owner { text, .. } => ToWindow::Owner { text, at: 0 },
        ToWindow::Reply { text, .. } => ToWindow::Reply { text, at: 0 },
        ToWindow::Traffic {
            channel,
            place,
            who,
            text,
            ..
        } => ToWindow::Traffic {
            channel,
            place,
            who,
            text,
            at: 0,
        },
        other => other,
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
        at: 0,
    };
    assert_eq!(first.next_untimed().await, Some(echo.clone()));
    assert_eq!(second.next_untimed().await, Some(echo));
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
        late.next_raw().await.map(untimed),
        Some(ToWindow::Reply {
            text: String::from("hi there"),
            at: 0,
        })
    );
    assert_eq!(
        late.next_raw().await.map(untimed),
        Some(ToWindow::Traffic {
            channel: String::from("slack"),
            place: String::from("C1"),
            who: String::from("Alice"),
            text: String::from("ping"),
            at: 0,
        })
    );
    assert_eq!(
        late.next_raw().await,
        Some(ToWindow::Working { busy: false })
    );
    assert_eq!(late.next_raw().await, Some(ToWindow::Live));
}

#[tokio::test]
async fn windows_hear_when_dotori_starts_and_stops_working() {
    let (channel, address, _inbox) = start().await;
    channel.working(true);
    let mut window = Window::open(address, "secret").await;

    assert_eq!(
        window.next_raw().await,
        Some(ToWindow::Working { busy: true })
    );
    assert_eq!(window.next_raw().await, Some(ToWindow::Live));

    channel.working(false);
    assert_eq!(
        window.next_raw().await,
        Some(ToWindow::Working { busy: false })
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

#[tokio::test]
async fn an_endless_hello_is_hung_up_on() {
    let (_channel, address, _inbox) = start().await;
    let stream = TcpStream::connect(address).await.expect("connected");
    let (read, mut write) = stream.into_split();
    let flood = vec![b'x'; 300 * 1024];

    let _ = write.write_all(&flood).await;

    let mut lines = BufReader::new(read).lines();
    let ended = time::timeout(WAIT, lines.next_line())
        .await
        .expect("in time");
    assert!(matches!(ended, Ok(None) | Err(_)));
}

#[test]
fn the_token_never_shows_in_debug_output() {
    let (channel, _address) = AppChannel::bind("sk-window-token").expect("bound");

    assert!(!format!("{channel:?}").contains("sk-window-token"));
}

#[tokio::test]
async fn windows_learn_which_pc_answers_but_it_stays_out_of_the_conversation() {
    let (channel, address, _inbox) = start().await;
    channel.answering(Some((String::from("맥북"), false)));
    let mut early = Window::open(address, "secret").await;
    assert_eq!(
        early.next().await,
        Some(ToWindow::Answering {
            pc: String::from("맥북"),
            here: false
        })
    );

    channel.answering(Some((String::from("데스크톱"), true)));
    assert_eq!(
        early.next().await,
        Some(ToWindow::Answering {
            pc: String::from("데스크톱"),
            here: true
        })
    );
    let mut published = channel.published();
    channel.show_line(r#"{"kind":"answering","pc":"x","here":false}"#);
    channel
        .deliver(Place::new(MAIN), String::from("hi"))
        .await
        .expect("delivered");
    let line = published.recv().await.expect("published").line;
    assert_eq!(
        serde_json::from_str::<ToWindow>(&line)
            .map(untimed)
            .expect("line"),
        ToWindow::Reply {
            text: String::from("hi"),
            at: 0,
        }
    );
    let mut late = Window::open(address, "secret").await;
    assert_eq!(
        late.next_untimed().await,
        Some(ToWindow::Reply {
            text: String::from("hi"),
            at: 0,
        })
    );
    assert!(matches!(
        late.next().await,
        Some(ToWindow::Answering { here: true, .. })
    ));
}

#[tokio::test]
async fn a_line_keeps_the_time_it_was_first_shown_and_an_old_line_has_none() {
    let (channel, address, _inbox) = start().await;
    let before = now_ms();
    channel
        .deliver(Place::new(MAIN), String::from("now"))
        .await
        .expect("delivered");
    channel.show_line(r#"{"kind":"reply","text":"elsewhere","at":1700000000000}"#);
    channel.show_line(r#"{"kind":"owner","text":"from an older dotori"}"#);
    let mut window = Window::open(address, "secret").await;

    let Some(ToWindow::Reply { at, .. }) = window.next().await else {
        panic!("a reply first");
    };
    assert!((before..=now_ms()).contains(&at));
    assert_eq!(
        window.next().await,
        Some(ToWindow::Reply {
            text: String::from("elsewhere"),
            at: 1_700_000_000_000,
        })
    );
    assert_eq!(
        window.next().await,
        Some(ToWindow::Owner {
            text: String::from("from an older dotori"),
            at: 0,
        })
    );
}

#[tokio::test]
async fn lines_from_before_a_start_come_first_and_stay_on_this_pc() {
    let (channel, address, _inbox) = start().await;
    let mut published = channel.published();
    channel
        .deliver(Place::new(MAIN), String::from("now"))
        .await
        .expect("delivered");
    channel.show_earlier(&[
        ToWindow::Owner {
            text: String::from("yesterday"),
            at: 1,
        },
        ToWindow::Reply {
            text: String::from("answered"),
            at: 2,
        },
    ]);
    let mut window = Window::open(address, "secret").await;

    assert_eq!(
        window.next().await,
        Some(ToWindow::Owner {
            text: String::from("yesterday"),
            at: 1,
        })
    );
    assert_eq!(
        window.next().await,
        Some(ToWindow::Reply {
            text: String::from("answered"),
            at: 2,
        })
    );
    assert!(matches!(
        window.next_untimed().await,
        Some(ToWindow::Reply { text, .. }) if text == "now"
    ));
    assert!(
        published
            .recv()
            .await
            .expect("published")
            .line
            .contains("now")
    );
    assert!(published.try_recv().is_err());
}

#[tokio::test]
async fn connections_past_the_limit_are_hung_up_on_and_the_owner_still_gets_in_later() {
    let (_channel, address, mut inbox) = start().await;
    let mut held = Vec::new();
    for _ in 0..WINDOWS {
        held.push(TcpStream::connect(address).await.expect("connected"));
    }
    time::sleep(Duration::from_millis(200)).await;

    let mut turned_away = Window::open(address, "secret").await;
    turned_away.say("hello?").await;
    assert_eq!(turned_away.next().await, None);

    drop(held);
    time::sleep(Duration::from_millis(200)).await;
    let mut owner = Window::open(address, "secret").await;
    owner.say("now").await;
    let heard = time::timeout(WAIT, inbox.recv())
        .await
        .expect("in time")
        .expect("heard");
    assert_eq!(heard.content.as_str(), "now");
}
