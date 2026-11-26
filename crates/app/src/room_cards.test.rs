use std::future;
use std::sync::atomic::AtomicU64;

use dotori_core::BoxFuture;
use dotori_core::channel::Channel;
use dotori_core::channel::ChannelError;
use dotori_core::channel::Inbox;
use dotori_core::id::ChannelId;
use dotori_core::message::Address;
use dotori_core::message::Place;
use dotori_core::tool::ToolCall;
use dotori_core::tool::ToolCallId;
use dotori_core::tool::ToolError;
use dotori_core::tool::ToolSpec;
use dotori_core::tool::Toolbox;
use dotori_core::tool::asked_from;

use super::*;
use crate::language::Language;
use crate::words::speak;

const WAIT: Duration = Duration::from_secs(5);

struct Room {
    id: ChannelId,
    posts: mpsc::UnboundedSender<(Place, Card)>,
    settles: mpsc::UnboundedSender<(Posted, Option<Card>, String)>,
    presses: Mutex<Option<mpsc::UnboundedSender<Press>>>,
    next: AtomicU64,
    direct: Option<Place>,
}

struct Seen {
    posts: mpsc::UnboundedReceiver<(Place, Card)>,
    settles: mpsc::UnboundedReceiver<(Posted, Option<Card>, String)>,
}

impl Channel for Room {
    fn id(&self) -> &ChannelId {
        &self.id
    }

    fn run(&self, _inbox: Arc<dyn Inbox>) -> BoxFuture<'_, Result<(), ChannelError>> {
        Box::pin(future::pending())
    }

    fn deliver(&self, _place: Place, _text: String) -> BoxFuture<'_, Result<(), ChannelError>> {
        Box::pin(async { Ok(()) })
    }

    fn private_place(&self) -> Place {
        Place::new("@owner")
    }
}

impl Cards for Room {
    fn post(&self, place: Place, card: Card) -> BoxFuture<'_, Result<Posted, ChannelError>> {
        let message = self.next.fetch_add(1, Ordering::SeqCst).to_string();
        let _ = self.posts.send((place, card));
        Box::pin(async move {
            Ok(Posted {
                room: String::from("C1"),
                message,
            })
        })
    }

    fn settle(
        &self,
        posted: Posted,
        card: Option<Card>,
        said: String,
    ) -> BoxFuture<'_, Result<(), ChannelError>> {
        let _ = self.settles.send((posted, card, said));
        Box::pin(async { Ok(()) })
    }

    fn presses(&self) -> mpsc::UnboundedReceiver<Press> {
        let (presses, heard) = mpsc::unbounded_channel();
        *self.presses.lock().expect("unpoisoned") = Some(presses);
        heard
    }

    fn direct_place(&self) -> Option<Place> {
        self.direct.clone()
    }
}

struct Desk {
    cards: Arc<RoomCards>,
    app: Arc<AppChannel>,
    room: Arc<Room>,
    seen: Seen,
    screen: mpsc::UnboundedReceiver<Value>,
}

impl Desk {
    fn new() -> Self {
        Self::with(Some(Place::new("@owner")))
    }

    fn with(direct: Option<Place>) -> Self {
        speak(Language::Korean);
        let (app, _) = AppChannel::bind("token").expect("bound");
        let app = Arc::new(app);
        let (cards, screen) = RoomCards::new(String::from("dotori가"), Arc::clone(&app));
        let (posts, posts_seen) = mpsc::unbounded_channel();
        let (settles, settles_seen) = mpsc::unbounded_channel();
        let room = Arc::new(Room {
            id: ChannelId::parse("slack").expect("valid"),
            posts,
            settles,
            presses: Mutex::new(None),
            next: AtomicU64::new(1),
            direct,
        });
        cards.connect(&[Arc::clone(&room) as Arc<dyn Cards>]);
        Self {
            cards,
            app,
            room,
            seen: Seen {
                posts: posts_seen,
                settles: settles_seen,
            },
            screen,
        }
    }

    async fn posted(&mut self) -> (Place, Card) {
        time::timeout(WAIT, self.seen.posts.recv())
            .await
            .expect("in time")
            .expect("posted")
    }

    async fn settled(&mut self) -> (Posted, Option<Card>, String) {
        time::timeout(WAIT, self.seen.settles.recv())
            .await
            .expect("in time")
            .expect("settled")
    }

    fn press(&self, card: &Card, button: usize, message: &str) {
        self.room
            .presses
            .lock()
            .expect("unpoisoned")
            .as_ref()
            .expect("connected")
            .send(Press {
                token: card.token.clone(),
                button,
                posted: Posted {
                    room: String::from("C1"),
                    message: message.to_owned(),
                },
            })
            .expect("heard");
    }
}

fn from(channel: &str) -> Option<Origin> {
    Some(Origin {
        address: Address {
            channel: ChannelId::parse(channel).expect("valid"),
            place: Place::new("C1/10.5"),
        },
        owner: String::from("김민지"),
        audience: Some(Audience::Owner),
    })
}

fn shared(channel: &str) -> Option<Origin> {
    from(channel).map(|origin| Origin {
        audience: Some(Audience::Anyone(String::from("the Slack workspace"))),
        ..origin
    })
}

fn labels(card: &Card) -> Vec<&str> {
    card.buttons
        .iter()
        .map(|button| button.label.as_str())
        .collect()
}

fn told(shown: &Shown) -> String {
    format!(
        "{}\nwhat the model is told",
        serde_json::to_string(shown).expect("json")
    )
}

#[tokio::test]
async fn a_call_asked_in_a_chat_room_goes_there_as_the_window_shows_it() {
    let mut desk = Desk::new();

    desk.cards.ask(
        from("slack"),
        7,
        "run_shell",
        &json!({ "command": "ls -la" }).to_string(),
        true,
    );

    let (place, card) = desk.posted().await;
    assert_eq!(place, Place::new("C1/10.5"));
    assert_eq!(card.title, "dotori가 이 일을 해도 될지 물어요");
    assert_eq!(
        card.rows,
        [
            (String::from("도구"), String::from("명령 실행")),
            (String::from("명령"), String::from("ls -la")),
        ]
    );
    assert!(!card.cut);
    assert_eq!(
        card.notes,
        [
            String::from("샌드박스 안에서 실행해요. Dotori의 금고와 기록은 보이지 않아요."),
            String::from("허락하면 dotori가 이 명령을 바로 실행해요"),
        ]
    );
    assert_eq!(labels(&card), ["거절", "허락"]);
    assert!(card.buttons[1].primary);
}

#[tokio::test]
async fn a_card_in_a_chat_room_speaks_english_when_dotori_does() {
    let mut desk = Desk::new();
    let english = speak(Language::English);

    desk.cards.ask(
        from("slack"),
        7,
        "run_shell",
        &json!({ "command": "ls -la" }).to_string(),
        true,
    );

    let (_, card) = desk.posted().await;
    assert_eq!(card.title, "Can I do this?");
    assert_eq!(
        card.rows,
        [
            (String::from("Tool"), String::from("Run command")),
            (String::from("Command"), String::from("ls -la")),
        ]
    );
    assert_eq!(
        card.notes,
        [
            (english.chat.sandboxed)(),
            String::from("Allow and I'll run this command right away."),
        ]
    );
    assert_eq!(labels(&card), ["Decline", "Allow"]);
    desk.cards.decided(7, Some(true));
    assert_eq!(desk.settled().await.2, "김민지 allowed it");
    desk.cards.ask(from("slack"), 8, "mail__send", "{}", false);
    desk.posted().await;
    desk.cards.decided(8, None);
    assert_eq!(desk.settled().await.2, english.chat.expired);
}

#[tokio::test]
async fn a_call_from_the_window_or_a_routine_stays_in_the_window() {
    let mut desk = Desk::new();

    desk.cards.ask(from("app"), 1, "mail__send", "{}", false);
    desk.cards
        .ask(from("routine"), 2, "mail__send", "{}", false);
    desk.cards.ask(None, 3, "mail__send", "{}", false);
    desk.cards.ask(from("slack"), 4, "mail__send", "{}", false);

    let (_, card) = desk.posted().await;
    assert_eq!(
        card.rows[0],
        (String::from("도구"), String::from("mail send"))
    );
    assert!(desk.seen.posts.try_recv().is_err(), "only the Slack one");
}

#[tokio::test]
async fn a_long_value_is_shortened_and_the_card_says_the_window_shows_it_whole() {
    let mut desk = Desk::new();
    let long = "가".repeat(asked::SHOWN_VALUE + 1);

    desk.cards.ask(
        from("slack"),
        7,
        "write_file",
        &json!({ "content": long }).to_string(),
        false,
    );

    let (_, card) = desk.posted().await;
    assert!(card.cut);
    assert!(card.text(None, 4000).contains(card::WHOLE_ON_PC));
}

#[tokio::test]
async fn the_owners_press_decides_as_a_window_would_and_the_card_says_who() {
    let mut desk = Desk::new();
    let mut decisions = desk.app.decisions();
    tokio::spawn(follow(
        Arc::clone(&desk.cards),
        desk.app.published(),
        |_| Status::Waiting,
    ));
    desk.cards.ask(from("slack"), 7, "mail__send", "{}", false);
    let (_, card) = desk.posted().await;

    desk.press(&card, 1, "1");
    let decided = time::timeout(WAIT, decisions.recv())
        .await
        .expect("in time");
    assert_eq!(decided, Some((7, true)));
    desk.app.decided(7, true);

    let (posted, kept, said) = desk.settled().await;
    assert_eq!(posted.message, "1");
    assert_eq!(kept, Some(card));
    assert_eq!(said, "김민지 님이 허락했어요");
}

#[tokio::test]
async fn the_first_answer_wins_and_a_second_press_is_ignored() {
    let mut desk = Desk::new();
    let mut decisions = desk.app.decisions();
    tokio::spawn(follow(
        Arc::clone(&desk.cards),
        desk.app.published(),
        |_| Status::Waiting,
    ));
    desk.cards.ask(from("slack"), 7, "mail__send", "{}", false);
    desk.cards.ask(from("slack"), 8, "mail__send", "{}", false);
    let (_, first) = desk.posted().await;
    let (_, second) = desk.posted().await;

    desk.press(&first, 0, "1");
    desk.press(&first, 1, "1");
    assert_eq!(
        time::timeout(WAIT, decisions.recv())
            .await
            .expect("in time"),
        Some((7, false))
    );

    desk.app.decided(8, false);
    let (_, _, said) = desk.settled().await;
    assert_eq!(said, "김민지 님이 거절했어요");
    desk.press(&second, 1, "2");
    desk.app.decided(7, false);
    let (posted, _, said) = desk.settled().await;
    assert_eq!(
        (posted.message.as_str(), said.as_str()),
        ("1", "김민지 님이 거절했어요")
    );

    time::sleep(Duration::from_millis(50)).await;
    assert!(decisions.try_recv().is_err(), "nothing more was decided");
    assert!(
        desk.seen.settles.try_recv().is_err(),
        "no card was edited again"
    );
}

#[tokio::test]
async fn a_call_that_no_longer_waits_says_it_can_no_longer_be_done() {
    let mut desk = Desk::new();
    tokio::spawn(follow(
        Arc::clone(&desk.cards),
        desk.app.published(),
        |_| Status::Waiting,
    ));
    desk.cards.ask(from("slack"), 7, "mail__send", "{}", false);
    desk.cards.ask(from("slack"), 8, "mail__send", "{}", false);
    desk.posted().await;
    desk.posted().await;

    desk.app.lapsed(7);
    assert_eq!(
        desk.settled().await.2,
        "이제 할 수 없어요. 다시 부탁해 주세요"
    );
    desk.cards.lapse_calls();
    let (posted, _, said) = desk.settled().await;
    assert_eq!(
        (posted.message.as_str(), said.as_str()),
        ("2", "이제 할 수 없어요. 다시 부탁해 주세요")
    );
}

#[tokio::test]
async fn a_call_decided_before_its_card_was_posted_is_settled_once_it_is() {
    let mut desk = Desk::new();

    desk.cards.ask(from("slack"), 7, "mail__send", "{}", false);
    desk.cards.decided(7, Some(true));

    desk.posted().await;
    assert_eq!(desk.settled().await.2, "김민지 님이 허락했어요");
}

#[tokio::test]
async fn a_press_on_a_card_from_before_dotori_started_says_it_can_no_longer_be_done() {
    let mut desk = Desk::new();
    let stale = Card {
        token: card::token().expect("random"),
        title: String::new(),
        rows: Vec::new(),
        cut: false,
        notes: Vec::new(),
        buttons: Vec::new(),
    };

    desk.press(&stale, 1, "9");

    let (posted, card, said) = desk.settled().await;
    assert_eq!(posted.message, "9");
    assert_eq!(card, None);
    assert_eq!(said, "이제 할 수 없어요. 다시 부탁해 주세요");
}

struct Ran;

impl Toolbox for Ran {
    fn specs(&self) -> Vec<ToolSpec> {
        Vec::new()
    }

    fn run(&self, _call: ToolCall) -> BoxFuture<'_, Result<String, ToolError>> {
        Box::pin(async { Ok(String::from("sent")) })
    }
}

#[tokio::test]
async fn a_window_answer_first_closes_the_rooms_card_and_a_later_press_changes_nothing() {
    let mut desk = Desk::new();
    let mut asked = crate::settings::Asked::default();
    asked.always("mail__send");
    let asking = crate::confirm::Asking::new(
        asked,
        None,
        Arc::new(Ran),
        Arc::clone(&desk.app),
        Arc::clone(&desk.cards),
    );
    let mut decisions = desk.app.decisions();
    let mut published = desk.app.published();
    tokio::spawn(follow(
        Arc::clone(&desk.cards),
        desk.app.published(),
        |_| Status::Waiting,
    ));
    let call = ToolCall {
        id: ToolCallId::new("c"),
        name: String::from("mail__send"),
        input: String::from(r#"{"to":"boss@example.com"}"#),
    };

    asked_from(from("slack"), async { asking.run(call).await })
        .await
        .expect("waits");
    let (_, card) = desk.posted().await;
    let Ok(ToWindow::Ask { id, .. }) =
        serde_json::from_str(&published.recv().await.expect("asked").line)
    else {
        panic!("the window is asked too");
    };
    assert!(
        asking.decide(id, true).await.is_some(),
        "the window decides"
    );

    assert_eq!(desk.settled().await.2, "김민지 님이 허락했어요");
    desk.press(&card, 0, "1");
    time::sleep(Duration::from_millis(50)).await;
    assert!(decisions.try_recv().is_err(), "the press came second");
    assert!(desk.seen.settles.try_recv().is_err());
    assert_eq!(asking.decide(id, false).await, None);
}

fn log_in_state(id: u64, ticket: u64, ids: Value) -> Value {
    json!({
        "kind": "state",
        "card": {
            "id": id,
            "ticket": ticket,
            "kind": "log_in",
            "site": "쿠팡",
            "accounts": ["minji"],
            "ids": ids,
            "host": "login.coupang.com",
            "path": "/login",
        },
    })
}

#[tokio::test]
async fn a_log_in_card_offers_to_stop_asking_and_answers_the_browser_as_the_window_does() {
    let mut desk = Desk::new();
    let ticket = desk.cards.browser_asked(from("slack"));

    desk.cards
        .browser_state(&log_in_state(40, ticket, json!(["minji@example.com"])));
    desk.cards
        .browser_state(&log_in_state(40, ticket, json!(["minji@example.com"])));

    let (_, card) = desk.posted().await;
    assert!(desk.seen.posts.try_recv().is_err(), "posted once");
    assert_eq!(
        card.title,
        "금고의 login.coupang.com (쿠팡) 계정으로 로그인해도 될까요?"
    );
    assert_eq!(
        card.rows,
        [(
            String::from("계정"),
            String::from("login.coupang.com (쿠팡), minji@example.com")
        )]
    );
    assert_eq!(
        card.notes,
        [String::from(
            "dotori가 login.coupang.com/login 로그인 칸에 직접 채워 넣어요. 대화하는 AI는 비밀번호를 보지 못해요."
        )]
    );
    assert_eq!(
        labels(&card),
        ["거절", "로그인", "로그인, 다음부터 묻지 않기"]
    );

    desk.press(&card, 2, "1");
    let line = time::timeout(WAIT, desk.screen.recv())
        .await
        .expect("in time");
    assert_eq!(
        line,
        Some(json!({ "kind": "fill", "id": 40, "allow": true, "trust": true, "account": 0 }))
    );
    desk.cards.browser_done(
        ticket,
        &told(&Shown::LoggedIn {
            site: String::from("쿠팡"),
        }),
    );
    assert_eq!(desk.settled().await.2, "김민지 님이 허락했어요");
}

#[tokio::test]
async fn several_accounts_get_a_button_each_with_the_whole_id_and_no_trust() {
    let mut desk = Desk::new();
    let ticket = desk.cards.browser_asked(shared("slack"));

    desk.cards.browser_state(&log_in_state(
        41,
        ticket,
        json!(["minji.example@example.com", null]),
    ));

    let (place, card) = desk.posted().await;
    assert_eq!(
        place,
        Place::new("C1/10.5"),
        "a log-in card stays where it was asked"
    );
    assert_eq!(card.rows[0].0, "계정 고르기");
    assert_eq!(
        labels(&card),
        [
            "minji.example@example.com 계정으로 로그인",
            "아이디 없는 계정으로 로그인",
            "거절"
        ]
    );
    desk.press(&card, 1, "1");
    assert_eq!(
        time::timeout(WAIT, desk.screen.recv())
            .await
            .expect("in time"),
        Some(json!({ "kind": "fill", "id": 41, "allow": true, "trust": false, "account": 1 }))
    );
}

#[tokio::test]
async fn a_log_in_card_that_waited_too_long_says_it_can_no_longer_be_done() {
    let mut desk = Desk::new();
    let telegram = Arc::new(Room {
        id: ChannelId::parse("telegram").expect("valid"),
        posts: desk.room.posts.clone(),
        settles: desk.room.settles.clone(),
        presses: Mutex::new(None),
        next: AtomicU64::new(1),
        direct: None,
    });
    desk.cards.connect(&[telegram as Arc<dyn Cards>]);
    let ticket = desk.cards.browser_asked(from("telegram"));

    desk.cards
        .browser_state(&log_in_state(42, ticket, json!(["minji@example.com"])));
    desk.posted().await;
    desk.cards.browser_done(
        ticket,
        &told(&Shown::Closed {
            title: String::from("쿠팡 로그인"),
            lapsed: true,
        }),
    );

    assert_eq!(
        desk.settled().await.2,
        "이제 할 수 없어요. 다시 부탁해 주세요"
    );
}

fn fill_state(id: u64, ticket: u64, keep: bool) -> Value {
    json!({
        "kind": "state",
        "card": {
            "id": id,
            "ticket": ticket,
            "kind": "fill",
            "title": "은행 로그인",
            "host": "bank.example.com",
            "fields": [
                { "ref": 3, "label": "아이디", "kind": "text" },
                { "ref": 4, "label": "비밀번호", "kind": "secret" },
            ],
            "submit": { "ref": 5, "label": "로그인" },
            "note": "대화하는 AI는 비밀번호를 보지 못해요.",
            "keep": keep,
        },
    })
}

#[tokio::test]
async fn a_card_that_takes_typed_values_only_tells_the_room_the_window_shows_it() {
    let mut desk = Desk::new();
    let alone = desk.cards.browser_asked(from("slack"));
    desk.cards.browser_state(&fill_state(51, alone, false));
    let (_, note) = desk.posted().await;
    let kept = desk.cards.browser_asked(from("slack"));
    desk.cards.browser_state(&fill_state(52, kept, true));
    let (_, log_in) = desk.posted().await;
    let others = desk.cards.browser_asked(shared("slack"));
    desk.cards.browser_state(&fill_state(53, others, true));
    let (_, plain) = desk.posted().await;

    assert_eq!(
        note.title,
        "PC의 Dotori 창에 bank.example.com 입력 카드를 띄웠어요."
    );
    assert_eq!(
        log_in.title,
        "PC의 Dotori 창에 bank.example.com 로그인 카드를 띄웠어요."
    );
    assert_eq!(plain.title, "PC의 Dotori 창에 입력 카드를 띄웠어요.");
    for card in [note, log_in, plain] {
        assert!(card.rows.is_empty() && card.notes.is_empty() && card.buttons.is_empty());
        let text = card.text(None, 4000);
        assert!(
            !text.contains("아이디") && !text.contains("비밀번호"),
            "{text}"
        );
    }
}

#[tokio::test]
async fn a_shared_room_gets_only_a_pointer_and_the_card_goes_to_the_owners_direct_place() {
    let mut desk = Desk::new();
    let mut decisions = desk.app.decisions();
    tokio::spawn(follow(
        Arc::clone(&desk.cards),
        desk.app.published(),
        |_| Status::Waiting,
    ));

    desk.cards.ask(
        shared("slack"),
        7,
        "run_shell",
        &json!({ "command": "cat ~/secret" }).to_string(),
        false,
    );

    let (direct, card) = desk.posted().await;
    let (room, pointer) = desk.posted().await;
    assert_eq!(direct, Place::new("@owner"));
    assert_eq!(card.rows[1].1, "cat ~/secret");
    assert_eq!(room, Place::new("C1/10.5"));
    assert_eq!(pointer.title, "확인할 일을 DM으로 보냈어요.");
    assert!(pointer.rows.is_empty() && pointer.notes.is_empty() && pointer.buttons.is_empty());

    desk.press(&card, 1, "1");
    assert_eq!(
        time::timeout(WAIT, decisions.recv())
            .await
            .expect("in time"),
        Some((7, true))
    );
    desk.app.decided(7, true);
    let mut settled = [desk.settled().await, desk.settled().await];
    settled.sort_by(|one, other| one.0.message.cmp(&other.0.message));
    assert_eq!(settled[0].1.as_ref(), Some(&card));
    assert_eq!(settled[1].1.as_ref(), Some(&pointer));
    assert!(
        settled
            .iter()
            .all(|(_, _, said)| said == "김민지 님이 허락했어요")
    );
}

#[tokio::test]
async fn a_shared_room_in_a_chat_with_no_direct_place_gets_nothing() {
    let mut desk = Desk::with(None);

    desk.cards
        .ask(shared("slack"), 7, "mail__send", "{}", false);
    desk.cards.ask(from("slack"), 8, "mail__send", "{}", false);

    let (place, card) = desk.posted().await;
    assert_eq!(place, Place::new("C1/10.5"));
    assert_eq!(card.buttons.len(), 2);
    time::sleep(Duration::from_millis(50)).await;
    assert!(
        desk.seen.posts.try_recv().is_err(),
        "only the owner-only room's card"
    );
}

#[tokio::test]
async fn a_log_in_card_goes_to_the_call_its_ticket_names() {
    let mut desk = Desk::new();
    let first = desk.cards.browser_asked(from("slack"));
    let second = desk.cards.browser_asked(None);

    desk.cards
        .browser_state(&log_in_state(60, second, json!(["minji@example.com"])));
    time::sleep(Duration::from_millis(20)).await;
    assert!(
        desk.seen.posts.try_recv().is_err(),
        "the window's call stays there"
    );
    desk.cards
        .browser_state(&log_in_state(61, first, json!(["minji@example.com"])));
    desk.posted().await;
    desk.cards.browser_done(
        first,
        &told(&Shown::Declined {
            site: String::from("쿠팡"),
        }),
    );
    assert_eq!(desk.settled().await.2, "김민지 님이 거절했어요");

    desk.cards
        .browser_state(&log_in_state(62, first, json!(["minji@example.com"])));
    time::sleep(Duration::from_millis(20)).await;
    assert!(
        desk.seen.posts.try_recv().is_err(),
        "a call that ended posts no card that nothing would settle"
    );
}

#[tokio::test]
async fn thirty_accounts_show_a_few_and_leave_the_rest_to_the_window() {
    let mut desk = Desk::new();
    let ticket = desk.cards.browser_asked(from("slack"));
    let ids: Vec<String> = (0..30).map(|n| format!("user{n}@example.com")).collect();

    desk.cards
        .browser_state(&log_in_state(70, ticket, json!(ids)));

    let (_, card) = desk.posted().await;
    assert_eq!(card.buttons.len(), MOST_ACCOUNTS + 1);
    assert_eq!(card.buttons[MOST_ACCOUNTS].label, "거절");
    assert_eq!(card.rows[0].1.lines().count(), MOST_ACCOUNTS);
    assert!(
        card.notes
            .contains(&String::from("나머지는 PC의 Dotori 창에서 골라 주세요"))
    );
}

#[tokio::test]
async fn missed_lines_settle_the_cards_by_what_became_of_each_call() {
    let mut desk = Desk::new();
    let (published, heard) = broadcast::channel(1);
    desk.cards.ask(from("slack"), 7, "mail__send", "{}", false);
    desk.cards.ask(from("slack"), 8, "mail__send", "{}", false);
    desk.cards.ask(from("slack"), 9, "mail__send", "{}", false);
    for _ in 0..3 {
        desk.posted().await;
    }
    time::sleep(Duration::from_millis(20)).await;
    for _ in 0..3 {
        let _ = published.send(Published {
            line: String::from("{}"),
            origin: None,
        });
    }

    tokio::spawn(follow(Arc::clone(&desk.cards), heard, |id| match id {
        7 => Status::Decided(true),
        8 => Status::Lapsed,
        _ => Status::Waiting,
    }));

    let mut settled = [desk.settled().await, desk.settled().await];
    settled.sort_by(|one, other| one.0.message.cmp(&other.0.message));
    assert_eq!(settled[0].2, "김민지 님이 허락했어요");
    assert_eq!(settled[1].2, "이제 할 수 없어요. 다시 부탁해 주세요");
    time::sleep(Duration::from_millis(50)).await;
    assert!(
        desk.seen.settles.try_recv().is_err(),
        "the call still waiting keeps its card"
    );
    drop(published);
}
