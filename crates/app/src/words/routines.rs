use jiff::civil::Date;
use jiff::civil::Weekday;

use crate::window::identity::Particle;
use crate::window::identity::called;

/// Strings of routines and the times they run.
#[derive(Debug)]
pub(crate) struct Routines {
    pub(crate) title: &'static str,
    /// What a routine is, under the title, which names the assistant.
    pub(crate) caption: fn() -> String,
    pub(crate) zone_label: &'static str,
    pub(crate) zone_placeholder: &'static str,
    pub(crate) zone_save: &'static str,
    pub(crate) zone_saved_shared: &'static str,
    pub(crate) zone_saved_each: &'static str,
    pub(crate) zone_shared: &'static str,
    pub(crate) zone_each: &'static str,
    /// The time zone this PC is in, by its name.
    pub(crate) zone_here: fn(&str) -> String,
    pub(crate) empty: &'static str,
    pub(crate) empty_ask: &'static str,
    pub(crate) remove: &'static str,
    pub(crate) every_day: &'static str,
    pub(crate) weekdays: &'static str,
    pub(crate) weekend: &'static str,
    /// The short name of a weekday, as a list of the days a routine runs shows it.
    pub(crate) weekday_short: fn(Weekday) -> &'static str,
    /// A routine that runs once, on that day.
    pub(crate) once: fn(Date) -> String,
    /// A routine that runs once, on a day kept in a form that does not read as a date.
    pub(crate) once_raw: fn(&str) -> String,
    pub(crate) today: &'static str,
    pub(crate) yesterday: &'static str,
    /// The line above the first message of a day in this year.
    pub(crate) day_this_year: fn(Date) -> String,
    /// The line above the first message of a day in another year.
    pub(crate) day_other_year: fn(Date) -> String,
    /// The time a message was said, from the hour from 0 to 23 and the minute.
    pub(crate) clock: fn(i8, i8) -> String,
}

fn twelve(hour: i8) -> i8 {
    match hour % 12 {
        0 => 12,
        other => other,
    }
}

fn korean_weekday(day: Weekday) -> &'static str {
    match day {
        Weekday::Monday => "월",
        Weekday::Tuesday => "화",
        Weekday::Wednesday => "수",
        Weekday::Thursday => "목",
        Weekday::Friday => "금",
        Weekday::Saturday => "토",
        Weekday::Sunday => "일",
    }
}

fn english_weekday(day: Weekday) -> &'static str {
    match day {
        Weekday::Monday => "Monday",
        Weekday::Tuesday => "Tuesday",
        Weekday::Wednesday => "Wednesday",
        Weekday::Thursday => "Thursday",
        Weekday::Friday => "Friday",
        Weekday::Saturday => "Saturday",
        Weekday::Sunday => "Sunday",
    }
}

fn english_month(month: i8) -> &'static str {
    match month {
        1 => "January",
        2 => "February",
        3 => "March",
        4 => "April",
        5 => "May",
        6 => "June",
        7 => "July",
        8 => "August",
        9 => "September",
        10 => "October",
        11 => "November",
        _ => "December",
    }
}

pub(crate) const KOREAN: Routines = Routines {
    title: "루틴",
    caption: || {
        format!(
            "정해 둔 시간에 {} 알아서 하는 일이에요. 결과는 채팅창에 보여요.",
            called(Particle::Subject)
        )
    },
    zone_label: "시간대",
    zone_placeholder: "이 PC의 시간대",
    zone_save: "바꾸기",
    zone_saved_shared: "바꿨어요. 다음 루틴부터 모든 PC가 이 시간대로 읽어요.",
    zone_saved_each: "바꿨어요. 다음 루틴부터 각 PC가 자기 시간대로 읽어요.",
    zone_shared: "모든 PC가 이 시간대로 루틴을 읽어요. 비우면 각 PC의 시간대를 써요.",
    zone_each: "지금은 각 PC의 시간대로 읽어요. Asia/Seoul처럼 적으면 모든 PC가 같은 시간대를 써요.",
    zone_here: |zone| format!("이 PC의 시간대: {zone}"),
    empty: "아직 루틴이 없어요",
    empty_ask: "채팅창에서 \"평일 아침 8시에 오늘 일정 알려줘\"처럼 부탁하면 돼요",
    remove: "지우기",
    every_day: "매일",
    weekdays: "평일",
    weekend: "주말",
    weekday_short: korean_weekday,
    once: |day| format!("{}년 {}월 {}일 한 번", day.year(), day.month(), day.day()),
    once_raw: |day| format!("{day} 한 번"),
    today: "오늘",
    yesterday: "어제",
    day_this_year: |day| {
        format!(
            "{}월 {}일 {}요일",
            day.month(),
            day.day(),
            korean_weekday(day.weekday())
        )
    },
    day_other_year: |day| {
        format!(
            "{}년 {}월 {}일 {}요일",
            day.year(),
            day.month(),
            day.day(),
            korean_weekday(day.weekday())
        )
    },
    clock: |hour, minute| {
        let half = if hour < 12 { "오전" } else { "오후" };
        format!("{half} {}:{minute:02}", twelve(hour))
    },
};

pub(crate) const ENGLISH: Routines = Routines {
    title: "Routines",
    caption: || {
        String::from("Things I do on my own at set times. You'll see the results in the chat.")
    },
    zone_label: "Time zone",
    zone_placeholder: "This PC's time zone",
    zone_save: "Change",
    zone_saved_shared: "Changed. From the next routine on, every PC reads times in this zone.",
    zone_saved_each: "Changed. From the next routine on, each PC reads times in its own zone.",
    zone_shared: "Every PC reads routine times in this zone. Clear it to use each PC's own.",
    zone_each: "Each PC reads routine times in its own zone now. Type one like Asia/Seoul to use it on every PC.",
    zone_here: |zone| format!("This PC is on {zone}."),
    empty: "No routines yet.",
    empty_ask: "Ask me in the chat, like \"Every weekday at 8 AM, tell me what's on today.\"",
    remove: "Remove",
    every_day: "Every day",
    weekdays: "Weekdays",
    weekend: "Weekends",
    weekday_short: |day| match day {
        Weekday::Monday => "Mon",
        Weekday::Tuesday => "Tue",
        Weekday::Wednesday => "Wed",
        Weekday::Thursday => "Thu",
        Weekday::Friday => "Fri",
        Weekday::Saturday => "Sat",
        Weekday::Sunday => "Sun",
    },
    once: |day| {
        format!(
            "Once on {} {}, {}",
            &english_month(day.month())[..3],
            day.day(),
            day.year()
        )
    },
    once_raw: |day| format!("Once on {day}"),
    today: "Today",
    yesterday: "Yesterday",
    day_this_year: |day| {
        format!(
            "{}, {} {}",
            english_weekday(day.weekday()),
            english_month(day.month()),
            day.day()
        )
    },
    day_other_year: |day| {
        format!(
            "{}, {} {}, {}",
            english_weekday(day.weekday()),
            english_month(day.month()),
            day.day(),
            day.year()
        )
    },
    clock: |hour, minute| {
        let half = if hour < 12 { "AM" } else { "PM" };
        format!("{}:{minute:02} {half}", twelve(hour))
    },
};
