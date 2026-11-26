//! When each line of the conversation was said, as the owner reads it: a line between days, and
//! the time of each line, which shows while the pointer rests on it.

use dotori_connectors::channel::app::ToWindow;
use jiff::Timestamp;
use jiff::ZonedDateTime;
use jiff::civil::Date;
use jiff::tz::TimeZone;

use crate::words::words;

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
                ToWindow::BrowserAt { .. }
                | ToWindow::Live
                | ToWindow::Working { .. }
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
    (words().routines.clock)(at.hour(), at.minute())
}

fn day_name(date: Date, today: Date) -> String {
    let strings = &words().routines;
    if date == today {
        return String::from(strings.today);
    }
    if today.yesterday().ok() == Some(date) {
        return String::from(strings.yesterday);
    }
    if date.year() == today.year() {
        (strings.day_this_year)(date)
    } else {
        (strings.day_other_year)(date)
    }
}

#[cfg(test)]
#[path = "when.test.rs"]
mod tests;
