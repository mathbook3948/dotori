use std::time::Duration;
use std::time::SystemTime;

use dotori_core::id::ChannelId;
use dotori_core::id::ExternalMessageId;
use dotori_core::id::ExternalUserId;
use dotori_core::message::Audience;
use dotori_core::message::Content;
use dotori_core::message::Message;
use dotori_core::message::Place;
use dotori_core::message::Role;
use dotori_core::message::Sender;

use super::*;

fn at(seconds: u64) -> SystemTime {
    UNIX_EPOCH + Duration::from_secs(seconds)
}

fn heard(seconds: u64, thread: u32, channel: &str, place: &str, who: &str, text: &str) -> Record {
    Record {
        at: at(seconds),
        entry: Entry::Heard {
            thread: ThreadRef::new(thread),
            message: Message {
                id: ExternalMessageId::new(format!("{seconds}")),
                address: Address {
                    channel: ChannelId::parse(channel).expect("channel"),
                    place: Place::new(place),
                },
                sender: Sender {
                    id: ExternalUserId::new(who),
                    name: who.to_owned(),
                    role: Role::Owner,
                },
                content: Content::parse(text.to_owned()).expect("text"),
                audience: Some(Audience::Owner),
                at: None,
            },
        },
    }
}

fn said(seconds: u64, says: &[(u32, &str, bool)]) -> Record {
    Record {
        at: at(seconds),
        entry: Entry::Acted {
            actions: says
                .iter()
                .map(|(thread, text, privately)| Action::Say {
                    thread: ThreadRef::new(*thread),
                    text: (*text).to_owned(),
                    privately: *privately,
                })
                .collect(),
            answered: 0,
            trace: None,
        },
    }
}

#[test]
fn the_window_gets_back_what_was_said_here_and_elsewhere_with_its_time() {
    let records = [
        heard(10, 0, "app", "main", "owner", "안녕"),
        said(11, &[(0, "안녕하세요", false)]),
        heard(20, 1, "slack", "C1", "Alice", "회의 언제야?"),
        said(21, &[(1, "3시예요", false), (1, "주인님께만", true)]),
    ];

    assert_eq!(
        lines(&records),
        [
            ToWindow::Owner {
                text: String::from("안녕"),
                at: 10_000,
            },
            ToWindow::Reply {
                text: String::from("안녕하세요"),
                at: 11_000,
            },
            ToWindow::Traffic {
                channel: String::from("slack"),
                place: String::from("C1"),
                who: String::from("Alice"),
                text: String::from("회의 언제야?"),
                at: 20_000,
            },
            ToWindow::Traffic {
                channel: String::from("slack"),
                place: String::from("C1"),
                who: String::from("dotori"),
                text: String::from("3시예요"),
                at: 21_000,
            },
            ToWindow::Traffic {
                channel: String::from("slack"),
                place: String::from("@owner"),
                who: String::from("dotori"),
                text: String::from("주인님께만"),
                at: 21_000,
            },
        ]
    );
}

fn failed(seconds: u64, thread: u32, text: &str) -> Record {
    Record {
        at: at(seconds),
        entry: Entry::DeliveryFailed {
            thread: ThreadRef::new(thread),
            text: text.to_owned(),
            detail: String::from("gone"),
        },
    }
}

fn replies(records: &[Record]) -> Vec<i64> {
    lines(records)
        .into_iter()
        .filter_map(|line| match line {
            ToWindow::Reply { at, .. } => Some(at),
            _ => None,
        })
        .collect()
}

#[test]
fn a_reply_that_never_went_out_is_the_last_one_like_it_before_the_failure() {
    let records = [
        heard(10, 0, "app", "main", "owner", "안녕"),
        said(11, &[(0, "ok", false)]),
        said(12, &[(0, "ok", false)]),
        failed(13, 0, "ok"),
        said(14, &[(0, "ok", false)]),
        said(15, &[(7, "모르는 곳", false)]),
    ];

    assert_eq!(replies(&records), [11_000, 14_000]);
}

#[test]
fn each_failure_takes_out_one_reply() {
    let records = [
        heard(10, 0, "app", "main", "owner", "안녕"),
        said(11, &[(0, "ok", false)]),
        said(12, &[(0, "ok", false)]),
        failed(13, 0, "ok"),
        failed(14, 0, "ok"),
        failed(15, 0, "ok"),
    ];

    assert!(replies(&records).is_empty());
}

#[test]
fn only_the_last_lines_come_back() {
    let records: Vec<Record> = (0..(SHOWN as u64 + 5))
        .map(|n| heard(n, 0, "app", "main", "owner", "말"))
        .collect();

    let shown = lines(&records);

    assert_eq!(shown.len(), SHOWN);
    assert_eq!(
        shown.first(),
        Some(&ToWindow::Owner {
            text: String::from("말"),
            at: 5_000,
        })
    );
}
