//! A card that waits for the owner also goes to Slack, Discord or Telegram when the owner asked
//! there, with a button for each answer. The window's card stays. Whichever answer comes first
//! decides, through the same path a window's answer takes, and the card in the chat then says who
//! decided or that it can no longer be done. A card that takes typed values never goes to a chat:
//! the room only hears that the window shows it.

use std::collections::HashMap;
use std::collections::VecDeque;
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::atomic::AtomicU64;
use std::sync::atomic::Ordering;
use std::time::Duration;

use dotori_connectors::channel::app::AppChannel;
use dotori_connectors::channel::app::Published;
use dotori_connectors::channel::app::ToWindow;
use dotori_connectors::channel::card;
use dotori_connectors::channel::card::Button;
use dotori_connectors::channel::card::Card;
use dotori_connectors::channel::card::Cards;
use dotori_connectors::channel::card::Posted;
use dotori_connectors::channel::card::Press;
use dotori_core::message::Audience;
use dotori_core::message::Place;
use dotori_core::tool::Origin;
use dotori_mcp::card::Shown;
use serde_json::Value;
use serde_json::json;
use tokio::io::AsyncBufReadExt;
use tokio::io::AsyncReadExt;
use tokio::io::AsyncWriteExt;
use tokio::io::BufReader;
use tokio::net::TcpStream;
use tokio::sync::broadcast;
use tokio::sync::broadcast::error::RecvError;
use tokio::sync::mpsc;
use tokio::time;

use crate::asked;
use crate::confirm::Status;
use crate::door::Door;
use crate::home::Home;

const RETRY: Duration = Duration::from_secs(1);
const CONNECT_WAIT: Duration = Duration::from_secs(5);
const LINE: u64 = 256 * 1024;
const CLOSED_KEPT: usize = 256;
/// The most accounts a chat card offers as buttons; the window offers the rest.
pub(crate) const MOST_ACCOUNTS: usize = 8;
/// What a room that others can read hears in place of a card sent to the owner's direct place.
pub(crate) const POINTER: &str = "확인할 일을 DM으로 보냈어요.";
const REST_ON_PC: &str = "나머지는 PC의 dotori 창에서 골라 주세요";
const NO_ID: &str = "아이디 없이";

/// The cards posted in chats that still wait, and the chats they can go to.
pub(crate) struct RoomCards {
    subject: String,
    app: Arc<AppChannel>,
    rooms: Mutex<HashMap<String, Arc<dyn Cards>>>,
    desk: Mutex<Desk>,
    next: AtomicU64,
    screen: mpsc::UnboundedSender<Value>,
}

#[derive(Default)]
struct Desk {
    cards: HashMap<String, Open>,
    closed: VecDeque<String>,
    calls: HashMap<u64, BrowserCall>,
}

struct BrowserCall {
    origin: Origin,
    shown: Option<u64>,
}

struct Open {
    key: Key,
    channel: String,
    card: Card,
    owner: String,
    answers: Vec<Answer>,
    posted: Option<Posted>,
    settled: Option<String>,
    answered: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Key {
    Call(u64),
    LogIn(u64),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Answer {
    Call {
        id: u64,
        ok: bool,
    },
    LogIn {
        card: u64,
        allow: bool,
        account: usize,
        trust: bool,
    },
}

type Face = (String, Vec<(String, String)>, bool, Vec<String>);

impl RoomCards {
    /// The cards for an assistant that `subject` names with its particle, such as "dotori가",
    /// and where the lines for the browser's screen come out.
    pub(crate) fn new(
        subject: String,
        app: Arc<AppChannel>,
    ) -> (Arc<Self>, mpsc::UnboundedReceiver<Value>) {
        let (screen, lines) = mpsc::unbounded_channel();
        let cards = Arc::new(Self {
            subject,
            app,
            rooms: Mutex::new(HashMap::new()),
            desk: Mutex::new(Desk::default()),
            next: AtomicU64::new(1),
            screen,
        });
        (cards, lines)
    }

    /// Posts cards through `chats` from now on, and takes the owner's presses there.
    pub(crate) fn connect(self: &Arc<Self>, chats: &[Arc<dyn Cards>]) {
        let mut rooms = self.rooms.lock().expect("unpoisoned");
        rooms.clear();
        for chat in chats {
            let channel = chat.id().as_str().to_owned();
            rooms.insert(channel.clone(), Arc::clone(chat));
            let mut presses = chat.presses();
            let cards = Arc::clone(self);
            tokio::spawn(async move {
                while let Some(press) = presses.recv().await {
                    cards.press(&channel, press).await;
                }
            });
        }
    }

    fn room(&self, origin: &Origin) -> Option<Arc<dyn Cards>> {
        self.rooms
            .lock()
            .expect("unpoisoned")
            .get(origin.address.channel.as_str())
            .cloned()
    }

    /// Posts the call `id` that waits for the owner where `origin` says they asked, if it was a
    /// chat. A room only the owner reads gets the card. Any other room gets only a pointer, and
    /// the card goes to the place in that chat only the owner reads, or nowhere if it has none.
    pub(crate) fn ask(
        self: &Arc<Self>,
        origin: Option<Origin>,
        id: u64,
        tool: &str,
        input: &str,
        sandboxed: bool,
    ) {
        let Some(origin) = origin else {
            return;
        };
        let Some(room) = self.room(&origin) else {
            return;
        };
        let (fields, cut) = asked::asked_fields(tool, input, false);
        let named = match asked::own_tool(tool) {
            Some(own) => own.to_owned(),
            None => tool.replacen("__", " ", 1),
        };
        let rows = std::iter::once((String::from("도구"), named))
            .chain(
                fields
                    .into_iter()
                    .map(|(key, value)| (asked::field_label(&key).to_owned(), value)),
            )
            .collect();
        let notes = sandboxed
            .then(|| String::from(asked::SANDBOXED))
            .into_iter()
            .chain(std::iter::once(asked::hint(tool, &self.subject)))
            .collect();
        let choices = [("거절", false), ("허락", true)]
            .into_iter()
            .map(|(label, ok)| (button(label, ok), Answer::Call { id, ok }))
            .collect();
        let face = (asked::title(&self.subject), rows, cut, notes);
        let key = Key::Call(id);
        if origin.audience == Some(Audience::Owner) {
            self.open(key, room, origin.address.place, origin.owner, face, choices);
            return;
        }
        let Some(direct) = room.direct_place() else {
            return;
        };
        self.open(
            key,
            Arc::clone(&room),
            direct,
            origin.owner.clone(),
            face,
            choices,
        );
        let pointer = (String::from(POINTER), Vec::new(), false, Vec::new());
        self.open(
            key,
            room,
            origin.address.place,
            origin.owner,
            pointer,
            Vec::new(),
        );
    }

    /// Says in every chat that the call `id` was decided, or with `None` that it no longer
    /// waits.
    pub(crate) fn decided(&self, id: u64, ok: Option<bool>) {
        self.settle(Key::Call(id), |owner| said(owner, ok));
    }

    /// Says in every chat that no call waits any more, as when this PC takes up the main session
    /// again or gives it up.
    pub(crate) fn lapse_calls(&self) {
        self.resync(|_| Status::Lapsed);
    }

    /// Settles the cards of every call `status` no longer counts as waiting.
    pub(crate) fn resync(&self, status: impl Fn(u64) -> Status) {
        let ids: Vec<u64> = self
            .desk
            .lock()
            .expect("unpoisoned")
            .cards
            .values()
            .filter_map(|open| match open.key {
                Key::Call(id) => Some(id),
                Key::LogIn(_) => None,
            })
            .collect();
        for id in ids {
            match status(id) {
                Status::Waiting => {}
                Status::Decided(ok) => self.decided(id, Some(ok)),
                Status::Lapsed => self.decided(id, None),
            }
        }
    }

    /// Starts a browser call that may show a card, asked for where `origin` says, and gives its
    /// ticket. The call carries the ticket to the browser, whose card carries it back.
    pub(crate) fn browser_asked(&self, origin: Option<Origin>) -> u64 {
        let ticket = self.next.fetch_add(1, Ordering::Relaxed);
        if let Some(origin) = origin.filter(|origin| self.room(origin).is_some()) {
            self.desk.lock().expect("unpoisoned").calls.insert(
                ticket,
                BrowserCall {
                    origin,
                    shown: None,
                },
            );
        }
        ticket
    }

    /// Takes the browser screen's state. A card the browser shows for a call asked for in a
    /// chat goes there: a log-in card with its buttons, and for a card that takes typed values
    /// only a note that the window shows it.
    pub(crate) fn browser_state(self: &Arc<Self>, state: &Value) {
        let Some(shown) = state.get("card").filter(|card| card.is_object()) else {
            return;
        };
        let (Some(id), Some(ticket)) = (
            shown.get("id").and_then(Value::as_u64),
            shown.get("ticket").and_then(Value::as_u64),
        ) else {
            return;
        };
        let origin = {
            let mut desk = self.desk.lock().expect("unpoisoned");
            match desk.calls.get_mut(&ticket) {
                Some(call) if call.shown != Some(id) => {
                    call.shown = Some(id);
                    call.origin.clone()
                }
                _ => return,
            }
        };
        let Some(room) = self.room(&origin) else {
            return;
        };
        let text = |key: &str| {
            shown
                .get(key)
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_owned()
        };
        let host = text("host");
        match shown.get("kind").and_then(Value::as_str) {
            Some("log_in") => {
                let ids: Vec<Option<String>> = shown
                    .get("ids")
                    .and_then(Value::as_array)
                    .map(Vec::as_slice)
                    .unwrap_or_default()
                    .iter()
                    .map(|id| id.as_str().map(str::to_owned))
                    .collect();
                let (site, path) = (text("site"), text("path"));
                let face = log_in(&self.subject, &host, &site, &path, &ids);
                let choices = log_in_choices(id, &ids);
                self.open(
                    Key::LogIn(ticket),
                    room,
                    origin.address.place,
                    origin.owner,
                    face,
                    choices,
                );
            }
            Some("fill") => {
                let note = if origin.audience == Some(Audience::Owner) {
                    format!(
                        "PC의 dotori 창에 {host} {} 카드를 띄웠어요.",
                        fill_kind(shown)
                    )
                } else {
                    String::from("PC의 dotori 창에 입력 카드를 띄웠어요.")
                };
                note_in(room, origin.address.place, note);
            }
            _ => {}
        }
    }

    /// Says in the chat what came of the browser call `ticket`, as `told` reads.
    pub(crate) fn browser_done(&self, ticket: u64, told: &str) {
        self.desk.lock().expect("unpoisoned").calls.remove(&ticket);
        let ok = match Shown::read(told) {
            Some((Shown::LoggedIn { .. }, _)) => Some(true),
            Some((Shown::Declined { .. }, _)) => Some(false),
            _ => None,
        };
        self.settle(Key::LogIn(ticket), |owner| said(owner, ok));
    }

    fn open(
        self: &Arc<Self>,
        key: Key,
        room: Arc<dyn Cards>,
        place: Place,
        owner: String,
        (title, rows, cut, notes): Face,
        choices: Vec<(Button, Answer)>,
    ) {
        let Ok(token) = card::token() else {
            eprintln!("dotori: no card goes to the chat, as no token could be made");
            return;
        };
        let (buttons, answers) = choices.into_iter().unzip();
        let card = Card {
            token: token.clone(),
            title,
            rows,
            cut,
            notes,
            buttons,
        };
        {
            let mut desk = self.desk.lock().expect("unpoisoned");
            if let Key::LogIn(ticket) = key
                && !desk.calls.contains_key(&ticket)
            {
                return;
            }
            desk.cards.insert(
                token.clone(),
                Open {
                    key,
                    channel: room.id().as_str().to_owned(),
                    card: card.clone(),
                    owner,
                    answers,
                    posted: None,
                    settled: None,
                    answered: false,
                },
            );
        }
        let cards = Arc::clone(self);
        tokio::spawn(async move {
            let posted = room.post(place, card.clone()).await;
            let settled = {
                let mut desk = cards.desk.lock().expect("unpoisoned");
                match (&posted, desk.cards.get_mut(&token)) {
                    (Ok(posted), Some(waiting)) => match waiting.settled.take() {
                        Some(said) => {
                            desk.close(&token);
                            Some((posted.clone(), said))
                        }
                        None => {
                            waiting.posted = Some(posted.clone());
                            None
                        }
                    },
                    _ => {
                        desk.cards.remove(&token);
                        None
                    }
                }
            };
            if let Err(error) = &posted {
                eprintln!(
                    "dotori: a card did not go to {}: {}",
                    room.id().as_str(),
                    error.detail
                );
            }
            if let Some((posted, said)) = settled {
                edit(room, posted, Some(card), said).await;
            }
        });
    }

    fn settle(&self, key: Key, said: impl Fn(&str) -> String) {
        let mut edits = Vec::new();
        {
            let mut desk = self.desk.lock().expect("unpoisoned");
            let tokens: Vec<String> = desk
                .cards
                .iter()
                .filter(|(_, open)| open.key == key)
                .map(|(token, _)| token.clone())
                .collect();
            for token in tokens {
                let Some(waiting) = desk.cards.get_mut(&token) else {
                    continue;
                };
                let told = said(&waiting.owner);
                if waiting.posted.is_none() {
                    waiting.settled = Some(told);
                    continue;
                }
                if let Some(done) = desk.close(&token)
                    && let Some(posted) = done.posted
                {
                    edits.push((done.channel, posted, done.card, told));
                }
            }
        }
        for (channel, posted, card, told) in edits {
            let room = self
                .rooms
                .lock()
                .expect("unpoisoned")
                .get(&channel)
                .cloned();
            if let Some(room) = room {
                tokio::spawn(edit(room, posted, Some(card), told));
            }
        }
    }

    /// Takes the owner's press of a card's button in `channel`. The first answer decides, as a
    /// window's would; a card that no longer waits says so.
    pub(crate) async fn press(&self, channel: &str, press: Press) {
        let answer = {
            let mut desk = self.desk.lock().expect("unpoisoned");
            let closed = desk.closed.contains(&press.token);
            match desk.cards.get_mut(&press.token) {
                Some(waiting) if waiting.channel == channel => {
                    if waiting.answered {
                        return;
                    }
                    let Some(answer) = waiting.answers.get(press.button).copied() else {
                        return;
                    };
                    waiting.answered = true;
                    Some(answer)
                }
                _ if closed => return,
                _ => None,
            }
        };
        match answer {
            Some(Answer::Call { id, ok }) => self.app.decide(id, ok),
            Some(Answer::LogIn {
                card,
                allow,
                account,
                trust,
            }) => {
                let _ = self.screen.send(json!({
                    "kind": "fill",
                    "id": card,
                    "allow": allow,
                    "trust": allow && trust,
                    "account": account,
                }));
            }
            None => {
                let room = self.rooms.lock().expect("unpoisoned").get(channel).cloned();
                if let Some(room) = room {
                    edit(room, press.posted, None, String::from(asked::lapsed())).await;
                }
            }
        }
    }
}

impl Desk {
    fn close(&mut self, token: &str) -> Option<Open> {
        if self.closed.len() == CLOSED_KEPT {
            self.closed.pop_front();
        }
        self.closed.push_back(token.to_owned());
        self.cards.remove(token)
    }
}

fn note_in(room: Arc<dyn Cards>, place: Place, note: String) {
    let Ok(token) = card::token() else {
        return;
    };
    let card = Card {
        token,
        title: note,
        rows: Vec::new(),
        cut: false,
        notes: Vec::new(),
        buttons: Vec::new(),
    };
    tokio::spawn(async move {
        if let Err(error) = room.post(place, card).await {
            eprintln!("dotori: a note did not go to the chat: {}", error.detail);
        }
    });
}

fn button(label: &str, primary: bool) -> Button {
    Button {
        label: label.to_owned(),
        primary,
    }
}

fn said(owner: &str, ok: Option<bool>) -> String {
    match ok {
        Some(true) => format!("{owner} 님이 허락했어요"),
        Some(false) => format!("{owner} 님이 거절했어요"),
        None => String::from(asked::lapsed()),
    }
}

fn fill_kind(shown: &Value) -> &'static str {
    let secret = shown
        .get("fields")
        .and_then(Value::as_array)
        .is_some_and(|fields| {
            fields
                .iter()
                .any(|field| field.get("kind").and_then(Value::as_str) == Some("secret"))
        });
    if secret && shown.get("keep").and_then(Value::as_bool) == Some(true) {
        "로그인"
    } else {
        "입력"
    }
}

fn log_in(subject: &str, host: &str, site: &str, path: &str, ids: &[Option<String>]) -> Face {
    let accounts: Vec<String> = ids
        .iter()
        .take(MOST_ACCOUNTS)
        .map(|id| asked::log_in_account(host, site, id.as_deref().unwrap_or(NO_ID)))
        .collect();
    let label = if ids.len() > 1 {
        "계정 고르기"
    } else {
        "계정"
    };
    let mut notes = vec![asked::log_in_note(subject, host, path)];
    if ids.len() > MOST_ACCOUNTS {
        notes.push(String::from(REST_ON_PC));
    }
    (
        asked::log_in_title(host, site),
        vec![(String::from(label), accounts.join("\n"))],
        false,
        notes,
    )
}

fn log_in_choices(card: u64, ids: &[Option<String>]) -> Vec<(Button, Answer)> {
    let answer = |allow: bool, account: usize, trust: bool| Answer::LogIn {
        card,
        allow,
        account,
        trust,
    };
    let decline = (button("거절", false), answer(false, 0, false));
    if ids.len() <= 1 {
        return vec![
            decline,
            (button("로그인", true), answer(true, 0, false)),
            (
                button("로그인, 다음부터 묻지 않기", false),
                answer(true, 0, true),
            ),
        ];
    }
    ids.iter()
        .take(MOST_ACCOUNTS)
        .enumerate()
        .map(|(n, id)| {
            let label = match id {
                Some(id) => format!("{id}로 로그인"),
                None => String::from("아이디 없는 계정으로 로그인"),
            };
            (button(&label, true), answer(true, n, false))
        })
        .chain(std::iter::once(decline))
        .collect()
}

async fn edit(room: Arc<dyn Cards>, posted: Posted, card: Option<Card>, said: String) {
    if let Err(error) = room.settle(posted, card, said).await {
        eprintln!(
            "dotori: a card in {} could not be updated: {}",
            room.id().as_str(),
            error.detail
        );
    }
}

/// Settles the chats' cards for calls as every window is told what came of them. When some of
/// those lines were missed, the cards are settled by what `status` says of each call instead.
pub(crate) async fn follow(
    cards: Arc<RoomCards>,
    mut published: broadcast::Receiver<Published>,
    status: impl Fn(u64) -> Status + Send,
) {
    loop {
        let line = match published.recv().await {
            Ok(published) => published.line,
            Err(RecvError::Lagged(_)) => {
                cards.resync(&status);
                continue;
            }
            Err(RecvError::Closed) => return,
        };
        match serde_json::from_str::<ToWindow>(&line) {
            Ok(ToWindow::Decided { id, ok }) => cards.decided(id, Some(ok)),
            Ok(ToWindow::Lapsed { id }) => cards.decided(id, None),
            _ => {}
        }
    }
}

/// Listens to the browser's screen on this PC as no window does, so the cards it shows can go
/// to a chat, and sends it `lines`, the owner's answers from the chats. It connects again
/// whenever the MCP server starts again.
pub(crate) async fn tap(
    home: Home,
    cards: Arc<RoomCards>,
    mut lines: mpsc::UnboundedReceiver<Value>,
) {
    loop {
        if listen(&home, &cards, &mut lines).await.is_none() {
            return;
        }
        time::sleep(RETRY).await;
    }
}

async fn listen(
    home: &Home,
    cards: &Arc<RoomCards>,
    lines: &mut mpsc::UnboundedReceiver<Value>,
) -> Option<()> {
    let Ok(door) = Door::read(&home.screen()) else {
        return Some(());
    };
    let Ok(Ok(stream)) =
        time::timeout(CONNECT_WAIT, TcpStream::connect(("127.0.0.1", door.port))).await
    else {
        return Some(());
    };
    let (read, mut write) = stream.into_split();
    let hello = json!({ "token": door.token, "quiet": true }).to_string();
    if write
        .write_all(format!("{hello}\n").as_bytes())
        .await
        .is_err()
    {
        return Some(());
    }
    let (states_tx, mut states) = mpsc::channel::<Value>(4);
    let reader = tokio::spawn(async move {
        let mut read = BufReader::new(read);
        loop {
            let mut bytes = Vec::new();
            match (&mut read).take(LINE).read_until(b'\n', &mut bytes).await {
                Ok(0) | Err(_) => return,
                Ok(_) => {
                    if let Ok(state) = serde_json::from_slice::<Value>(&bytes)
                        && states_tx.send(state).await.is_err()
                    {
                        return;
                    }
                }
            }
        }
    });
    let ended = loop {
        tokio::select! {
            state = states.recv() => {
                let Some(state) = state else { break Some(()) };
                cards.browser_state(&state);
            }
            line = lines.recv() => {
                let Some(line) = line else { break None };
                if write.write_all(format!("{line}\n").as_bytes()).await.is_err() {
                    break Some(());
                }
            }
        }
    };
    reader.abort();
    ended
}

#[cfg(test)]
#[path = "room_cards.test.rs"]
mod tests;
