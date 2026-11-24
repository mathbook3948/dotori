use std::sync::atomic::AtomicUsize;
use std::sync::atomic::Ordering;

use jiff::civil::date;
use jiff::tz::offset;

use super::*;

static NEXT: AtomicUsize = AtomicUsize::new(0);

fn routines() -> Arc<Routines> {
    let n = NEXT.fetch_add(1, Ordering::SeqCst);
    let folder = std::env::temp_dir().join(format!("dotori-routines-{}-{n}", std::process::id()));
    let _ = std::fs::remove_dir_all(&folder);
    Routines::new(folder.join("routines.sqlite3"))
}

fn seoul() -> TimeZone {
    TimeZone::fixed(offset(9))
}

fn at(day: Date, hour: i8, minute: i8) -> Timestamp {
    day.at(hour, minute, 0, 0)
        .to_zoned_datetime(seoul())
        .expect("a time")
        .timestamp()
}

fn routine(time: &str, days: u8, date: Option<&str>) -> Routine {
    Routine {
        id: 1,
        name: String::from("아침 브리핑"),
        time: time.to_owned(),
        days,
        date: date.map(str::to_owned),
        task: String::from("오늘 일정 알려줘"),
        made_ms: 0,
        fired_ms: 0,
    }
}

#[test]
fn a_daily_routine_is_due_once_in_the_half_hour_after_its_time() {
    let monday = date(2026, 11, 23);
    let mut every_day = routine("08:00", 0, None);
    let slot = at(monday, 8, 0).as_millisecond();

    assert_eq!(every_day.due(&seoul(), at(monday, 7, 59)), None);
    assert_eq!(every_day.due(&seoul(), at(monday, 8, 0)), Some(slot));
    assert_eq!(every_day.due(&seoul(), at(monday, 8, 29)), Some(slot));
    assert_eq!(every_day.due(&seoul(), at(monday, 8, 31)), None);

    every_day.fired_ms = slot;
    assert_eq!(every_day.due(&seoul(), at(monday, 8, 10)), None);
    assert_eq!(
        every_day.due(&seoul(), at(date(2026, 11, 24), 8, 5)),
        Some(at(date(2026, 11, 24), 8, 0).as_millisecond())
    );
}

#[test]
fn a_weekday_routine_skips_the_weekend() {
    let weekdays = routine("09:30", WEEKDAYS, None);
    let saturday = date(2026, 11, 28);
    let monday = date(2026, 11, 30);

    assert_eq!(weekdays.due(&seoul(), at(saturday, 9, 40)), None);
    assert_eq!(
        weekdays.last_slot(&seoul(), at(saturday, 9, 40)),
        Some(at(date(2026, 11, 27), 9, 30).as_millisecond())
    );
    assert!(weekdays.due(&seoul(), at(monday, 9, 31)).is_some());
}

#[test]
fn a_routine_made_after_its_time_waits_for_the_next_one() {
    let monday = date(2026, 11, 23);
    let mut late = routine("08:00", 0, None);
    late.made_ms = at(monday, 8, 5).as_millisecond();

    assert_eq!(late.due(&seoul(), at(monday, 8, 6)), None);
    assert!(late.due(&seoul(), at(date(2026, 11, 24), 8, 1)).is_some());
}

#[test]
fn a_one_time_routine_is_due_only_on_its_day() {
    let once = routine("14:00", 0, Some("2026-12-24"));

    assert_eq!(once.due(&seoul(), at(date(2026, 12, 23), 14, 5)), None);
    assert!(once.due(&seoul(), at(date(2026, 12, 24), 14, 5)).is_some());
    assert_eq!(once.due(&seoul(), at(date(2026, 12, 25), 14, 5)), None);
}

#[test]
fn the_schedule_reads_the_way_the_owner_would_say_it() {
    assert_eq!(routine("08:00", 0, None).when(), "매일");
    assert_eq!(routine("08:00", WEEKDAYS, None).when(), "평일");
    assert_eq!(routine("08:00", WEEKEND, None).when(), "주말");
    assert_eq!(routine("08:00", 0b000_0101, None).when(), "월, 수");
    assert_eq!(
        routine("14:00", 0, Some("2026-12-24")).when(),
        "2026년 12월 24일 한 번"
    );
}

#[test]
fn routines_are_added_checked_and_removed() {
    let routines = routines();

    let added = routines.add(
        &json!({
            "name": "아침 브리핑",
            "time": "08:00",
            "days": ["mon", "tue", "wed", "thu", "fri"],
            "task": "오늘 일정과 날씨 알려줘"
        }),
        10,
    );
    assert_eq!(added, Ok(String::from("Set up routine [1].")));
    assert!(
        routines
            .add(&json!({ "name": "x", "time": "25:00", "task": "y" }), 11)
            .is_err()
    );
    assert!(
        routines
            .add(
                &json!({ "name": "x", "time": "08:00", "days": ["someday"], "task": "y" }),
                11
            )
            .is_err()
    );
    let listed = routines.list().expect("listed");
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].days, WEEKDAYS);
    assert_eq!(listed[0].made_ms, 10);
    assert!(
        routines.context().expect("context").contains(
            "[1] 아침 브리핑, on mon, tue, wed, thu, fri at 08:00: 오늘 일정과 날씨 알려줘"
        )
    );

    assert!(routines.remove(1, 12).is_ok());
    assert!(routines.remove(1, 13).is_err());
    assert_eq!(routines.context(), None);
}

#[test]
fn a_run_is_noted_and_a_one_time_routine_goes_away() {
    let routines = routines();
    routines
        .add(&json!({ "name": "매일", "time": "08:00", "task": "a" }), 1)
        .expect("added");
    routines
        .add(
            &json!({ "name": "한 번", "time": "08:00", "date": "2026-12-24", "task": "b" }),
            1,
        )
        .expect("added");
    let listed = routines.list().expect("listed");

    routines.fired(&listed[0], 500, 600).expect("noted");
    routines.fired(&listed[1], 500, 600).expect("noted");

    let left = routines.list().expect("listed");
    assert_eq!(left.len(), 1);
    assert_eq!(left[0].fired_ms, 500);
    let incoming = incoming(&left[0], 500).expect("message");
    assert_eq!(incoming.id.as_str(), "routine-1-500");
    assert_eq!(incoming.place.as_str(), "매일");
    assert_eq!(incoming.content.as_str(), "a");
}

#[test]
fn the_copy_changed_last_wins() {
    let here = routines();
    let there = routines();
    here.add(
        &json!({ "name": "여기", "time": "08:00", "task": "a" }),
        100,
    )
    .expect("added");
    there
        .add(
            &json!({ "name": "저기", "time": "09:00", "task": "b" }),
            200,
        )
        .expect("added");

    assert_eq!(here.take(&there.copy().expect("copy")), Ok(true));
    assert_eq!(there.take(&here.copy().expect("copy")), Ok(false));

    let names: Vec<String> = here
        .list()
        .expect("listed")
        .into_iter()
        .map(|routine| routine.name)
        .collect();
    assert_eq!(names, ["저기"]);
}

#[test]
fn a_one_time_routine_whose_time_went_by_long_ago_is_over() {
    let once = routine("14:00", 0, Some("2026-12-24"));
    let daily = routine("14:00", 0, None);
    let day = date(2026, 12, 24);

    assert!(!once.over(&seoul(), at(day, 13, 0)));
    assert!(!once.over(&seoul(), at(day, 14, 20)));
    assert!(once.over(&seoul(), at(day, 14, 31)));
    assert!(!daily.over(&seoul(), at(day, 20, 0)));
}

#[test]
fn a_time_already_past_and_a_name_already_used_are_refused() {
    let routines = routines();
    let now = Timestamp::now().as_millisecond();

    let past = routines.add(
        &json!({ "name": "지난 일", "time": "08:00", "date": "2020-01-01", "task": "a" }),
        now,
    );
    assert!(past.expect_err("past").contains("already passed"));
    routines
        .add(
            &json!({ "name": "브리핑", "time": "08:00", "task": "a" }),
            now,
        )
        .expect("added");
    let again = routines.add(
        &json!({ "name": "브리핑", "time": "09:00", "task": "b" }),
        now,
    );
    assert!(
        again
            .expect_err("taken")
            .contains("already a routine named")
    );
}

#[test]
fn a_newer_copy_never_takes_back_a_run() {
    let here = routines();
    let there = routines();
    here.add(
        &json!({ "name": "매일", "time": "08:00", "task": "a" }),
        100,
    )
    .expect("added");
    there.take(&here.copy().expect("copy")).expect("taken");
    let routine = here.list().expect("listed").remove(0);
    here.fired(&routine, 5_000, 200).expect("ran");
    there
        .add(
            &json!({ "name": "다른 것", "time": "09:00", "task": "b" }),
            300,
        )
        .expect("added");

    assert_eq!(here.take(&there.copy().expect("copy")), Ok(true));

    let after = here.list().expect("listed");
    assert_eq!(after.len(), 2);
    assert_eq!(after[0].fired_ms, 5_000);
}

#[test]
fn the_owner_sets_one_time_zone_for_every_pc() {
    let here = routines();
    let there = routines();
    assert_eq!(
        here.zone_name().expect("read"),
        None,
        "each PC's own at first"
    );

    here.set_zone(Some("Asia/Seoul"), 10).expect("set");

    assert_eq!(here.zone().iana_name(), Some("Asia/Seoul"));
    assert!(there.take(&here.copy().expect("copy")).expect("took"));
    assert_eq!(
        there.zone_name().expect("read").as_deref(),
        Some("Asia/Seoul")
    );

    here.set_zone(None, 20).expect("cleared");
    assert!(there.take(&here.copy().expect("copy")).expect("took"));
    assert_eq!(there.zone_name().expect("read"), None);
}

#[test]
fn a_name_that_is_no_time_zone_is_refused() {
    let routines = routines();

    let refused = routines.set_zone(Some("Mars/Olympus"), 10);

    assert!(refused.expect_err("refused").contains("not a time zone"));
    assert_eq!(routines.zone_name().expect("read"), None);
}

#[test]
fn a_copy_from_a_pc_that_knows_no_time_zone_keeps_the_one_here() {
    let routines = routines();
    routines.set_zone(Some("Asia/Seoul"), 10).expect("set");
    let older: RoutinesCopy =
        serde_json::from_str(r#"{"changed_ms":50,"routines":[]}"#).expect("an older copy");

    assert!(routines.take(&older).expect("took"));

    assert_eq!(
        routines.zone_name().expect("read").as_deref(),
        Some("Asia/Seoul")
    );
}

#[test]
fn a_time_zone_is_kept_by_its_own_name() {
    let routines = routines();

    routines.set_zone(Some(" asia/seoul "), 10).expect("set");

    assert_eq!(
        routines.zone_name().expect("read").as_deref(),
        Some("Asia/Seoul")
    );
}

#[test]
fn a_file_from_before_time_zones_opens_and_takes_one() {
    let folder = std::env::temp_dir().join(format!(
        "dotori-routines-v1-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::SeqCst)
    ));
    let _ = std::fs::remove_dir_all(&folder);
    let path = folder.join("routines.sqlite3");
    let first = Routines::new(path.clone());
    first
        .add(
            &json!({ "name": "아침 브리핑", "time": "08:00", "task": "오늘 일정 알려줘" }),
            10,
        )
        .expect("added");
    drop(first);
    let old = Connection::open(&path).expect("opened");
    old.execute_batch("DROP TABLE options; PRAGMA user_version = 1;")
        .expect("made it a first-version file");
    drop(old);

    let routines = Routines::new(path.clone());
    routines.set_zone(Some("Asia/Seoul"), 20).expect("set");

    assert_eq!(routines.list().expect("listed").len(), 1);
    assert_eq!(
        routines.zone_name().expect("read").as_deref(),
        Some("Asia/Seoul")
    );
    let version: i64 = Connection::open(&path)
        .expect("opened")
        .query_row("PRAGMA user_version", [], |row| row.get(0))
        .expect("read");
    assert_eq!(version, VERSION);
    let _ = std::fs::remove_dir_all(&folder);
}
