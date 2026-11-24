//! When each line of the conversation was said, as the owner reads it: a line between days, and
//! the time under the last of the lines one speaker said in the same minute.

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
    /// The time, shown with the line when it ends what its speaker said in that minute.
    pub(crate) time: Option<String>,
}

#[derive(PartialEq)]
enum Speaker {
    Owner,
    Dotori,
    Elsewhere,
}

/// The marks for every line, in order, read in `zone` on the day `today`.
pub(crate) fn marks(lines: &[ToWindow], zone: &TimeZone, today: Date) -> Vec<Mark> {
    let said: Vec<Option<(Speaker, ZonedDateTime)>> = lines
        .iter()
        .map(|line| {
            let (speaker, at) = match line {
                ToWindow::Owner { at, .. } => (Speaker::Owner, *at),
                ToWindow::Reply { at, .. } => (Speaker::Dotori, *at),
                ToWindow::Traffic { at, .. } | ToWindow::Ask { at, .. } => {
                    (Speaker::Elsewhere, *at)
                }
                ToWindow::Answering { .. }
                | ToWindow::Alone
                | ToWindow::BrowserAt { .. }
                | ToWindow::Decided { .. } => {
                    return None;
                }
            };
            let local = local(at, zone)?;
            Some((speaker, local))
        })
        .collect();
    let mut last_day = None;
    said.iter()
        .enumerate()
        .map(|(n, now)| {
            let Some((speaker, at)) = now else {
                return Mark::default();
            };
            let date = at.date();
            let day = (last_day != Some(date)).then(|| day_name(date, today));
            last_day = Some(date);
            let continued = speaker != &Speaker::Elsewhere
                && said
                    .get(n + 1)
                    .and_then(Option::as_ref)
                    .is_some_and(|(next, later)| {
                        next == speaker
                            && later.date() == date
                            && (later.hour(), later.minute()) == (at.hour(), at.minute())
                    });
            Mark {
                day,
                time: (!continued).then(|| clock(at)),
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
