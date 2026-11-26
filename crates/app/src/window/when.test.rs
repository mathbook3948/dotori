use jiff::civil::date;
use jiff::tz::offset;

use super::*;
use crate::language::Language;
use crate::words::of;

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

fn clock(hour: i8, minute: i8) -> String {
    (words().routines.clock)(hour, minute)
}

#[test]
fn every_line_keeps_the_time_it_was_said() {
    let today = date(2026, 11, 23);
    let lines = [
        owner(at(2026, 11, 23, 15, 7, 1)),
        owner(at(2026, 11, 23, 15, 7, 40)),
        reply(at(2026, 11, 23, 15, 7, 50)),
        reply(at(2026, 11, 23, 15, 8, 2)),
        owner(at(2026, 11, 23, 0, 5, 0)),
    ];

    let today_line = words().routines.today;
    let (seven, eight, five) = (clock(15, 7), clock(15, 8), clock(0, 5));
    assert_eq!(
        shown(&marks(&lines, &seoul(), today)),
        [
            (Some(today_line), Some(seven.as_str())),
            (None, Some(seven.as_str())),
            (None, Some(seven.as_str())),
            (None, Some(eight.as_str())),
            (None, Some(five.as_str())),
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
    let strings = &words().routines;
    assert_eq!(
        days,
        [
            Some((strings.day_other_year)(date(2025, 12, 31))),
            Some((strings.day_this_year)(date(2026, 11, 2))),
            Some(String::from(strings.yesterday)),
            Some(String::from(strings.today)),
        ]
    );
}

#[test]
fn lines_without_a_time_show_none() {
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

    let nine = clock(9, 0);
    assert_eq!(
        shown(&marks(&lines, &seoul(), today)),
        [
            (Some(words().routines.today), Some(nine.as_str())),
            (None, Some(nine.as_str())),
            (None, None),
            (None, None),
        ]
    );
}

#[test]
fn each_language_names_the_day_and_the_time_its_own_way() {
    let korean = &of(Language::Korean).routines;
    let english = &of(Language::English).routines;

    assert_eq!(
        (korean.day_other_year)(date(2025, 12, 31)),
        "2025년 12월 31일 수요일"
    );
    assert_eq!((korean.day_this_year)(date(2026, 11, 2)), "11월 2일 월요일");
    assert_eq!((korean.clock)(0, 5), "오전 12:05");
    assert_eq!(
        (english.day_other_year)(date(2025, 12, 31)),
        "Wednesday, December 31, 2025"
    );
    assert_eq!(
        (english.day_this_year)(date(2026, 11, 2)),
        "Monday, November 2"
    );
    assert_eq!((english.clock)(15, 7), "3:07 PM");
}
