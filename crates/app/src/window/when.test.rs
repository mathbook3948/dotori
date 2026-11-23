use jiff::civil::date;
use jiff::tz::offset;

use super::*;

fn seoul() -> TimeZone {
    TimeZone::fixed(offset(9))
}

fn at(year: i16, month: i8, day: i8, hour: i8, minute: i8, second: i8) -> i64 {
    date(year, month, day)
        .at(hour, minute, second, 0)
        .to_zoned_datetime(seoul())
        .expect("a time")
        .timestamp()
        .as_millisecond()
}

fn owner(at: i64) -> ToWindow {
    ToWindow::Owner {
        text: String::from("hi"),
        at,
    }
}

fn reply(at: i64) -> ToWindow {
    ToWindow::Reply {
        text: String::from("hello"),
        at,
    }
}

fn shown(marks: &[Mark]) -> Vec<(Option<&str>, Option<&str>)> {
    marks
        .iter()
        .map(|mark| (mark.day.as_deref(), mark.time.as_deref()))
        .collect()
}

#[test]
fn the_time_goes_under_the_last_line_a_speaker_said_in_a_minute() {
    let today = date(2026, 11, 23);
    let lines = [
        owner(at(2026, 11, 23, 15, 7, 1)),
        owner(at(2026, 11, 23, 15, 7, 40)),
        reply(at(2026, 11, 23, 15, 7, 50)),
        reply(at(2026, 11, 23, 15, 8, 2)),
        owner(at(2026, 11, 23, 0, 5, 0)),
    ];

    assert_eq!(
        shown(&marks(&lines, &seoul(), today)),
        [
            (Some("오늘"), None),
            (None, Some("오후 3:07")),
            (None, Some("오후 3:07")),
            (None, Some("오후 3:08")),
            (None, Some("오전 12:05")),
        ]
    );
}

#[test]
fn a_new_day_gets_a_line_named_for_how_long_ago_it_was() {
    let today = date(2026, 11, 23);
    let lines = [
        owner(at(2025, 12, 31, 23, 59, 0)),
        owner(at(2026, 11, 2, 12, 0, 0)),
        owner(at(2026, 11, 22, 9, 0, 0)),
        owner(at(2026, 11, 23, 9, 0, 0)),
    ];

    let days: Vec<Option<String>> = marks(&lines, &seoul(), today)
        .into_iter()
        .map(|mark| mark.day)
        .collect();
    assert_eq!(
        days,
        [
            Some(String::from("2025년 12월 31일 수요일")),
            Some(String::from("11월 2일 월요일")),
            Some(String::from("어제")),
            Some(String::from("오늘")),
        ]
    );
}

#[test]
fn lines_elsewhere_each_keep_their_time_and_lines_without_one_show_none() {
    let today = date(2026, 11, 23);
    let elsewhere = |at| ToWindow::Traffic {
        channel: String::from("slack"),
        place: String::from("C1"),
        who: String::from("Alice"),
        text: String::from("ping"),
        at,
    };
    let lines = [
        elsewhere(at(2026, 11, 23, 9, 0, 0)),
        elsewhere(at(2026, 11, 23, 9, 0, 5)),
        owner(0),
        ToWindow::Answering {
            pc: String::from("B"),
            here: false,
        },
    ];

    assert_eq!(
        shown(&marks(&lines, &seoul(), today)),
        [
            (Some("오늘"), Some("오전 9:00")),
            (None, Some("오전 9:00")),
            (None, None),
            (None, None),
        ]
    );
}
