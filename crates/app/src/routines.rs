//! What the owner asks dotori to do at set times, such as a briefing every weekday morning. The
//! PC that runs the main session hands each routine to it when its time comes, once, and dotori
//! says the result in the routine's own thread, which the chat window shows.

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::Mutex;
use std::time::Duration;

use dotori_connectors::peer::hub::Heard;
use dotori_connectors::peer::hub::Hub;
use dotori_core::BoxFuture;
use dotori_core::channel::Channel;
use dotori_core::channel::ChannelError;
use dotori_core::channel::Inbox;
use dotori_core::id::ChannelId;
use dotori_core::id::ExternalMessageId;
use dotori_core::id::ExternalUserId;
use dotori_core::message::Audience;
use dotori_core::message::Content;
use dotori_core::message::Incoming;
use dotori_core::message::Place;
use jiff::Timestamp;
use jiff::civil::Date;
use jiff::civil::Weekday;
use jiff::tz::TimeZone;
use rusqlite::Connection;
use rusqlite::OptionalExtension;
use rusqlite::params;
use serde::Deserialize;
use serde::Serialize;
use serde_json::Value;
use serde_json::json;
use tokio::sync::Notify;
use tokio::time;

use crate::versioned;

/// The channel routines speak on.
pub(crate) const ROUTINE: &str = "routine";
pub(crate) const ADD: &str = "add_routine";
pub(crate) const REMOVE: &str = "remove_routine";
const MAX_ROUTINES: i64 = 30;
const MAX_NAME: usize = 60;
const MAX_TASK: usize = 1000;
const LOOK_EVERY: Duration = Duration::from_secs(20);
const LATE_MS: i64 = 30 * 60 * 1000;
const DAYS: [(&str, &str, Weekday); 7] = [
    ("mon", "월", Weekday::Monday),
    ("tue", "화", Weekday::Tuesday),
    ("wed", "수", Weekday::Wednesday),
    ("thu", "목", Weekday::Thursday),
    ("fri", "금", Weekday::Friday),
    ("sat", "토", Weekday::Saturday),
    ("sun", "일", Weekday::Sunday),
];
const WEEKDAYS: u8 = 0b001_1111;
const WEEKEND: u8 = 0b110_0000;
const VERSION: i64 = 2;
const SCHEMA: &str = "CREATE TABLE IF NOT EXISTS routines (\
    id INTEGER PRIMARY KEY AUTOINCREMENT, name TEXT NOT NULL, time TEXT NOT NULL, \
    days INTEGER NOT NULL, date TEXT, task TEXT NOT NULL, made_ms INTEGER NOT NULL, \
    fired_ms INTEGER NOT NULL); \
    CREATE TABLE IF NOT EXISTS meta (key TEXT PRIMARY KEY, value INTEGER NOT NULL); \
    CREATE TABLE IF NOT EXISTS options (key TEXT PRIMARY KEY, value TEXT NOT NULL)";
const ZONE: &str = "zone";

/// One routine. `days` has a bit for each weekday from Monday, and none means every day. A
/// routine with a `date` happens once, on that day, and is gone after.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct Routine {
    pub(crate) id: i64,
    pub(crate) name: String,
    pub(crate) time: String,
    pub(crate) days: u8,
    pub(crate) date: Option<String>,
    pub(crate) task: String,
    pub(crate) made_ms: i64,
    pub(crate) fired_ms: i64,
}

/// Every routine as one PC holds it, and when it last changed there. The copy changed last wins.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct RoutinesCopy {
    pub(crate) changed_ms: i64,
    pub(crate) routines: Vec<Routine>,
    #[serde(default)]
    pub(crate) zone: Option<String>,
}

impl Routine {
    /// The latest time it was due at or before `now`, or `None` when it never was.
    pub(crate) fn last_slot(&self, zone: &TimeZone, now: Timestamp) -> Option<i64> {
        let (hour, minute) = clock(&self.time)?;
        let at = |day: Date| {
            day.at(hour, minute, 0, 0)
                .to_zoned_datetime(zone.clone())
                .ok()
                .map(|moment| moment.timestamp().as_millisecond())
        };
        let now_ms = now.as_millisecond();
        if let Some(date) = &self.date {
            return at(date.parse().ok()?).filter(|slot| *slot <= now_ms);
        }
        let mut day = Some(now.to_zoned_datetime(zone.clone()).date());
        for _ in 0..=7 {
            let today = day?;
            if (self.days == 0 || self.days & bit(today.weekday()) != 0)
                && let Some(slot) = at(today).filter(|slot| *slot <= now_ms)
            {
                return Some(slot);
            }
            day = today.yesterday().ok();
        }
        None
    }

    /// The slot to run now, if one came in the last half hour and has not run yet.
    pub(crate) fn due(&self, zone: &TimeZone, now: Timestamp) -> Option<i64> {
        self.last_slot(zone, now).filter(|slot| {
            *slot > self.fired_ms
                && *slot >= self.made_ms
                && now.as_millisecond() - *slot <= LATE_MS
        })
    }

    /// Whether it happens only once and its time went by too long ago to run.
    pub(crate) fn over(&self, zone: &TimeZone, now: Timestamp) -> bool {
        self.date.is_some()
            && self
                .last_slot(zone, now)
                .is_some_and(|slot| now.as_millisecond() - slot > LATE_MS)
    }

    /// Which days it happens, in the owner's words.
    pub(crate) fn when(&self) -> String {
        if let Some(date) = &self.date {
            return match date.parse::<Date>() {
                Ok(day) => format!("{}년 {}월 {}일 한 번", day.year(), day.month(), day.day()),
                Err(_) => format!("{date} 한 번"),
            };
        }
        match self.days {
            0 | 0b111_1111 => String::from("매일"),
            WEEKDAYS => String::from("평일"),
            WEEKEND => String::from("주말"),
            days => DAYS
                .iter()
                .filter(|(_, _, day)| days & bit(*day) != 0)
                .map(|(_, name, _)| *name)
                .collect::<Vec<_>>()
                .join(", "),
        }
    }

    fn when_for_model(&self) -> String {
        if let Some(date) = &self.date {
            return format!("once on {date} at {}", self.time);
        }
        if self.days == 0 {
            return format!("every day at {}", self.time);
        }
        let days: Vec<&str> = DAYS
            .iter()
            .filter(|(_, _, day)| self.days & bit(*day) != 0)
            .map(|(english, _, _)| *english)
            .collect();
        format!("on {} at {}", days.join(", "), self.time)
    }
}

/// The routines in their own SQLite file.
#[derive(Debug)]
pub(crate) struct Routines {
    path: PathBuf,
    connection: Mutex<Option<Connection>>,
    changes: Notify,
}

impl Routines {
    pub(crate) fn new(path: PathBuf) -> Arc<Self> {
        Arc::new(Self {
            path,
            connection: Mutex::new(None),
            changes: Notify::new(),
        })
    }

    fn with<T>(&self, work: impl FnOnce(&Connection) -> rusqlite::Result<T>) -> Result<T, String> {
        let mut held = self
            .connection
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if held.is_none() {
            *held = Some(versioned::open(&self.path, SCHEMA, VERSION)?);
        }
        let opened = held.as_ref().ok_or("no connection")?;
        let done = work(opened).map_err(|error| error.to_string());
        if done.is_err() {
            *held = None;
        }
        done
    }

    /// Every routine, oldest first.
    pub(crate) fn list(&self) -> Result<Vec<Routine>, String> {
        self.with(|connection| {
            connection
                .prepare(
                    "SELECT id, name, time, days, date, task, made_ms, fired_ms FROM routines \
                     ORDER BY id",
                )?
                .query_map([], |row| {
                    Ok(Routine {
                        id: row.get(0)?,
                        name: row.get(1)?,
                        time: row.get(2)?,
                        days: row.get(3)?,
                        date: row.get(4)?,
                        task: row.get(5)?,
                        made_ms: row.get(6)?,
                        fired_ms: row.get(7)?,
                    })
                })?
                .collect()
        })
    }

    /// What the model is told about the routines, or `None` when there are none.
    pub(crate) fn context(&self) -> Option<String> {
        let routines = self.list().ok()?;
        if routines.is_empty() {
            return None;
        }
        let lines: Vec<String> = routines
            .iter()
            .map(|routine| {
                format!(
                    "[{}] {}, {}: {}",
                    routine.id,
                    one_line(&routine.name),
                    routine.when_for_model(),
                    one_line(&routine.task)
                )
            })
            .collect();
        let zone = self.zone();
        let zone = zone.iana_name().unwrap_or("this PC's time zone");
        Some(format!(
            "Routines the owner set up, at times in {zone}. When one is due, its task arrives as a message on the \
             `routine` channel, in a thread named after the routine: do the task then and say \
             the result in that thread, which the owner reads in the chat window. Use \
             `remove_routine` with a routine's number when the owner asks to drop it.\n{}",
            lines.join("\n")
        ))
    }

    pub(crate) fn add(&self, input: &Value, now_ms: i64) -> Result<String, String> {
        let text = |key: &str, max: usize| -> Result<String, String> {
            let value = input
                .get(key)
                .and_then(Value::as_str)
                .map(one_line)
                .filter(|value| !value.is_empty())
                .ok_or(format!("`{key}` is required"))?;
            if value.chars().count() > max {
                return Err(format!("`{key}` is at most {max} characters"));
            }
            Ok(value)
        };
        let name = text("name", MAX_NAME)?;
        let task = text("task", MAX_TASK)?;
        let time = input
            .get("time")
            .and_then(Value::as_str)
            .filter(|time| clock(time).is_some())
            .ok_or("`time` is required as HH:MM, from 00:00 to 23:59")?
            .to_owned();
        let date = match input.get("date").and_then(Value::as_str) {
            Some(date) => {
                let parsed: Date = date
                    .parse()
                    .map_err(|_| String::from("`date` is YYYY-MM-DD"))?;
                Some(parsed.to_string())
            }
            None => None,
        };
        let mut days = 0u8;
        for day in input
            .get("days")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
        {
            let name = day.as_str().unwrap_or_default().to_lowercase();
            let (_, _, weekday) = DAYS
                .iter()
                .find(|(english, _, _)| name.starts_with(english))
                .ok_or(format!(
                    "`{name}` is not a day; use mon, tue, wed, thu, fri, sat or sun"
                ))?;
            days |= bit(*weekday);
        }
        if days == 0b111_1111 {
            days = 0;
        }
        let wished = Routine {
            id: 0,
            name: name.clone(),
            time: time.clone(),
            days,
            date: date.clone(),
            task: task.clone(),
            made_ms: now_ms,
            fired_ms: 0,
        };
        if wished.date.is_some()
            && let Ok(now) = Timestamp::from_millisecond(now_ms)
            && wished.last_slot(&self.zone(), now).is_some()
        {
            return Err(String::from("that date and time has already passed"));
        }
        if self.list()?.iter().any(|routine| routine.name == name) {
            return Err(format!(
                "there is already a routine named \"{name}\"; pick another name"
            ));
        }
        let id = self.with(|connection| {
            let count: i64 =
                connection.query_row("SELECT count(*) FROM routines", [], |row| row.get(0))?;
            if count >= MAX_ROUTINES {
                return Ok(None);
            }
            connection.execute(
                "INSERT INTO routines (name, time, days, date, task, made_ms, fired_ms) \
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, 0)",
                params![name, time, days, date, task, now_ms],
            )?;
            Ok(Some(connection.last_insert_rowid()))
        })?;
        let Some(id) = id else {
            return Err(format!(
                "there are already {MAX_ROUTINES} routines; ask the owner which to remove"
            ));
        };
        self.touched(now_ms)?;
        Ok(format!("Set up routine [{id}]."))
    }

    pub(crate) fn remove(&self, id: i64, now_ms: i64) -> Result<String, String> {
        let removed =
            self.with(|connection| connection.execute("DELETE FROM routines WHERE id = ?1", [id]))?;
        if removed == 0 {
            return Err(format!("there is no routine [{id}]"));
        }
        self.touched(now_ms)?;
        Ok(format!("Removed routine [{id}]."))
    }

    fn fired(&self, routine: &Routine, slot: i64, now_ms: i64) -> Result<(), String> {
        self.with(|connection| {
            let both = connection.unchecked_transaction()?;
            if routine.date.is_some() {
                both.execute("DELETE FROM routines WHERE id = ?1", [routine.id])?;
            } else {
                both.execute(
                    "UPDATE routines SET fired_ms = ?2 WHERE id = ?1",
                    params![routine.id, slot],
                )?;
            }
            stamp(&both, now_ms)?;
            both.commit()
        })?;
        self.changes.notify_one();
        Ok(())
    }

    fn missed(&self, routine: &Routine, now_ms: i64) -> Result<(), String> {
        self.with(|connection| {
            let both = connection.unchecked_transaction()?;
            both.execute("DELETE FROM routines WHERE id = ?1", [routine.id])?;
            stamp(&both, now_ms)?;
            both.commit()
        })?;
        self.changes.notify_one();
        Ok(())
    }

    fn touched(&self, now_ms: i64) -> Result<(), String> {
        self.with(|connection| stamp(connection, now_ms))?;
        self.changes.notify_one();
        Ok(())
    }

    /// The time zone every PC reads routine times in, as the owner set it, if they did.
    pub(crate) fn zone_name(&self) -> Result<Option<String>, String> {
        self.with(|connection| {
            connection
                .query_row("SELECT value FROM options WHERE key = ?1", [ZONE], |row| {
                    row.get(0)
                })
                .optional()
        })
    }

    /// The time zone routine times are read in: the one the owner set, or else this PC's.
    pub(crate) fn zone(&self) -> TimeZone {
        self.zone_name()
            .ok()
            .flatten()
            .and_then(|name| TimeZone::get(&name).ok())
            .unwrap_or_else(TimeZone::system)
    }

    /// Sets the time zone every PC reads routine times in, such as `Asia/Seoul`, or with `None`
    /// goes back to each PC's own. The other PCs get it with the routines.
    pub(crate) fn set_zone(&self, name: Option<&str>, now_ms: i64) -> Result<(), String> {
        if let Some(name) = name {
            TimeZone::get(name).map_err(|_| format!("`{name}` is not a time zone"))?;
        }
        self.with(|connection| write_zone(connection, name))?;
        self.touched(now_ms)
    }

    /// Every routine, for the other PCs.
    pub(crate) fn copy(&self) -> Result<RoutinesCopy, String> {
        let routines = self.list()?;
        let changed_ms = self.with(|connection| {
            Ok(connection
                .query_row(
                    "SELECT value FROM meta WHERE key = 'changed_ms'",
                    [],
                    |row| row.get(0),
                )
                .optional()?
                .unwrap_or(0))
        })?;
        Ok(RoutinesCopy {
            changed_ms,
            routines,
            zone: self.zone_name()?,
        })
    }

    /// Takes another PC's routines in place of these if they changed later. Says whether they
    /// did.
    ///
    /// The whole copy wins, not each change, on purpose. dotori serves one person, who changes
    /// these from one PC at a time, so two PCs changing them in the same moment does not happen
    /// in practice. Merging change by change would add a log and conflict rules for a case that
    /// does not occur.
    pub(crate) fn take(&self, copy: &RoutinesCopy) -> Result<bool, String> {
        let here = self.copy()?;
        if copy.changed_ms <= here.changed_ms {
            return Ok(false);
        }
        let fired_here = |routine: &Routine| {
            here.routines
                .iter()
                .find(|known| known.id == routine.id && known.made_ms == routine.made_ms)
                .map_or(0, |known| known.fired_ms)
        };
        self.with(|connection| {
            let swap = connection.unchecked_transaction()?;
            swap.execute("DELETE FROM routines", [])?;
            for routine in copy.routines.iter().take(MAX_ROUTINES as usize) {
                swap.execute(
                    "INSERT INTO routines (id, name, time, days, date, task, made_ms, fired_ms) \
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
                    params![
                        routine.id,
                        one_line(&routine.name),
                        routine.time,
                        routine.days,
                        routine.date,
                        one_line(&routine.task),
                        routine.made_ms,
                        routine.fired_ms.max(fired_here(routine))
                    ],
                )?;
            }
            swap.execute(
                "INSERT OR REPLACE INTO meta (key, value) VALUES ('changed_ms', ?1)",
                [copy.changed_ms],
            )?;
            write_zone(&swap, copy.zone.as_deref())?;
            swap.commit()?;
            Ok(true)
        })
    }
}

/// Hands each routine to the main session when it is due. It runs only with the main session,
/// so only one PC runs a routine.
pub(crate) struct RoutineChannel {
    id: ChannelId,
    routines: Arc<Routines>,
}

impl RoutineChannel {
    pub(crate) fn new(routines: Arc<Routines>) -> Self {
        Self {
            id: ChannelId::parse(ROUTINE).expect("a valid channel id"),
            routines,
        }
    }
}

impl Channel for RoutineChannel {
    fn id(&self) -> &ChannelId {
        &self.id
    }

    fn run(&self, inbox: Arc<dyn Inbox>) -> BoxFuture<'_, Result<(), ChannelError>> {
        Box::pin(async move {
            let mut every = time::interval(LOOK_EVERY);
            loop {
                every.tick().await;
                let now = Timestamp::now();
                let zone = self.routines.zone();
                let routines = match self.routines.list() {
                    Ok(routines) => routines,
                    Err(error) => {
                        eprintln!("dotori: routines cannot be read: {error}");
                        continue;
                    }
                };
                for routine in routines {
                    if routine.over(&zone, now) {
                        if let Err(error) = self.routines.missed(&routine, now.as_millisecond()) {
                            eprintln!("dotori: routine [{}] stays: {error}", routine.id);
                        }
                        continue;
                    }
                    let Some(slot) = routine.due(&zone, now) else {
                        continue;
                    };
                    if let Err(error) = self.routines.fired(&routine, slot, now.as_millisecond()) {
                        eprintln!("dotori: routine [{}] was not run: {error}", routine.id);
                        continue;
                    }
                    let Some(incoming) = incoming(&routine, slot) else {
                        continue;
                    };
                    if inbox.push(incoming).await.is_err() {
                        eprintln!(
                            "dotori: routine [{}] could not reach the main session",
                            routine.id
                        );
                        return Ok(());
                    }
                }
            }
        })
    }

    fn deliver(&self, _place: Place, _text: String) -> BoxFuture<'_, Result<(), ChannelError>> {
        Box::pin(async { Ok(()) })
    }

    fn private_place(&self) -> Place {
        Place::new("owner")
    }
}

fn incoming(routine: &Routine, slot: i64) -> Option<Incoming> {
    let content = Content::parse(one_line(&routine.task)).ok()?;
    Some(Incoming {
        id: ExternalMessageId::new(format!("routine-{}-{slot}", routine.id)),
        place: Place::new(one_line(&routine.name)),
        sender_id: ExternalUserId::new(ROUTINE),
        sender_name: String::from("예약"),
        content,
        audience: Audience::Owner,
    })
}

/// Takes what another PC says about the routines. Says whether it was about them.
pub(crate) fn hear(routines: &Routines, hub: &Hub, heard: &Heard) -> bool {
    match heard.message.get("kind").and_then(Value::as_str) {
        Some("linked") => {
            if let Ok(copy) = routines.copy() {
                hub.send(&heard.key, json!({ "kind": "routines", "copy": copy }));
            }
            false
        }
        Some("routines") => {
            let taken = heard
                .message
                .get("copy")
                .and_then(|copy| serde_json::from_value::<RoutinesCopy>(copy.clone()).ok())
                .map(|copy| routines.take(&copy));
            if let Some(Err(error)) = taken {
                eprintln!(
                    "dotori: routines from {} were not kept: {error}",
                    heard.device
                );
            }
            true
        }
        _ => false,
    }
}

/// Sends the routines to every linked PC whenever they change here, also from the settings
/// window.
pub(crate) async fn share(routines: Arc<Routines>, hub: Arc<Hub>) {
    let mut sent = routines.copy().map(|copy| copy.changed_ms).unwrap_or(0);
    let mut every = time::interval(LOOK_EVERY);
    loop {
        tokio::select! {
            _ = every.tick() => {}
            () = routines.changes.notified() => {}
        }
        let Ok(copy) = routines.copy() else {
            continue;
        };
        if copy.changed_ms != sent {
            sent = copy.changed_ms;
            hub.broadcast(&json!({ "kind": "routines", "copy": copy }));
        }
    }
}

fn stamp(connection: &Connection, now_ms: i64) -> rusqlite::Result<()> {
    connection
        .execute(
            "INSERT OR REPLACE INTO meta (key, value) VALUES ('changed_ms', \
             max(?1, coalesce((SELECT value FROM meta WHERE key = 'changed_ms') + 1, 0)))",
            [now_ms],
        )
        .map(|_| ())
}

fn clock(time: &str) -> Option<(i8, i8)> {
    let (hour, minute) = time.split_once(':')?;
    if hour.len() != 2 || minute.len() != 2 {
        return None;
    }
    let hour: i8 = hour.parse().ok()?;
    let minute: i8 = minute.parse().ok()?;
    ((0..24).contains(&hour) && (0..60).contains(&minute)).then_some((hour, minute))
}

fn bit(day: Weekday) -> u8 {
    1 << day.to_monday_zero_offset()
}

fn one_line(text: &str) -> String {
    text.chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .collect::<String>()
        .trim()
        .to_owned()
}

fn write_zone(connection: &Connection, name: Option<&str>) -> rusqlite::Result<()> {
    match name {
        Some(name) => connection.execute(
            "INSERT OR REPLACE INTO options (key, value) VALUES (?1, ?2)",
            params![ZONE, name],
        )?,
        None => connection.execute("DELETE FROM options WHERE key = ?1", [ZONE])?,
    };
    Ok(())
}

#[cfg(test)]
#[path = "routines.test.rs"]
mod tests;
