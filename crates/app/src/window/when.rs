//! When each line of the conversation was said, as the owner reads it: a line between days, and
//! the time of each line, which shows while the pointer rests on it.

use dotori_connectors::channel::app::ToWindow;
use jiff::Timestamp;
use jiff::ZonedDateTime;
use jiff::civil::Date;
use jiff::civil::Weekday;
use jiff::tz::TimeZone;

/// What to show around one line of the conversation.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct Mark {
    /// The day, shown above the line when it is the first of that day.
    pub(crate) day: Option<String>,
    /// The time the line was said, when it is known.
    pub(crate) time: Option<String>,
}

/// The marks for every line, in order, read in `zone` on the day `today`.
pub(crate) fn marks(lines: &[ToWindow], zone: &TimeZone, today: Date) -> Vec<Mark> {
    let mut last_day = None;
    lines
        .iter()
        .map(|line| {
            let at = match line {
                ToWindow::Owner { at, .. }
                | ToWindow::Reply { at, .. }
                | ToWindow::Traffic { at, .. }
                | ToWindow::Ask { at, .. } => *at,
                ToWindow::Answering { .. }
                | ToWindow::Alone
                | ToWindow::BrowserAt { .. }
                | ToWindow::Live
                | ToWindow::Decided { .. }
                | ToWindow::Lapsed { .. } => return Mark::default(),
            };
            let Some(at) = local(at, zone) else {
                return Mark::default();
            };
            let date = at.date();
            let day = (last_day != Some(date)).then(|| day_name(date, today));
            last_day = Some(date);
            Mark {
                day,
                time: Some(clock(&at)),
            }
        })
        .collect()
}

/// Today where this PC is.
pub(crate) fn today(zone: &TimeZone) -> Date {
    Timestamp::now().to_zoned_datetime(zone.clone()).date()
}

fn local(at: i64, zone: &TimeZone) -> Option<ZonedDateTime> {
    if at <= 0 {
        return None;
    }
    Some(
        Timestamp::from_millisecond(at)
            .ok()?
            .to_zoned_datetime(zone.clone()),
    )
}

fn clock(at: &ZonedDateTime) -> String {
    let hour = at.hour();
    let half = if hour < 12 { "오전" } else { "오후" };
    let shown = match hour % 12 {
        0 => 12,
        other => other,
    };
    format!("{half} {shown}:{:02}", at.minute())
}

fn day_name(date: Date, today: Date) -> String {
    if date == today {
        return String::from("오늘");
    }
    if today.yesterday().ok() == Some(date) {
        return String::from("어제");
    }
    let weekday = match date.weekday() {
        Weekday::Monday => "월요일",
        Weekday::Tuesday => "화요일",
        Weekday::Wednesday => "수요일",
        Weekday::Thursday => "목요일",
        Weekday::Friday => "금요일",
        Weekday::Saturday => "토요일",
        Weekday::Sunday => "일요일",
    };
    if date.year() == today.year() {
        format!("{}월 {}일 {weekday}", date.month(), date.day())
    } else {
        format!(
            "{}년 {}월 {}일 {weekday}",
            date.year(),
            date.month(),
            date.day()
        )
    }
}

#[cfg(test)]
#[path = "when.test.rs"]
mod tests;
