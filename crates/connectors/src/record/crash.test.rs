use std::collections::BTreeMap;
use std::collections::BTreeSet;
use std::io::BufRead;
use std::io::BufReader;
use std::io::Write;
use std::path::Path;
use std::path::PathBuf;
use std::process::Child;
use std::process::ChildStdout;
use std::process::Command;
use std::process::Stdio;
use std::thread::sleep;
use std::time::Duration;
use std::time::UNIX_EPOCH;

use dotori_core::conversation::Entry;
use dotori_core::record::Record;
use dotori_core::record::RecordStore;

use super::sqlite::SqliteStore;

const FOLDER_ENV: &str = "DOTORI_CRASH_RECORD_FOLDER";
const WRITER_BATCH_CAP: u64 = 100_000;
const READY_LINE: &str = "ready";
const APPENDED_PREFIX: &str = "appended ";
const TRIALS: u64 = 12;
const JAN_1_2026: u64 = 1_767_225_600_000;

fn kill_point(trial: u64) -> (u64, u64) {
    (trial * 5 % 14, trial * 613 % 3_000)
}

fn batch_size(batch: u64) -> u64 {
    2 + batch % 4
}

fn batch(number: u64) -> Vec<Record> {
    let size = batch_size(number);
    (0..size)
        .map(|index| Record {
            at: UNIX_EPOCH + Duration::from_millis(JAN_1_2026 + number * 10 + index),
            entry: Entry::ModelFailed {
                detail: format!("{number}:{index}/{size}:{}", "x".repeat(300)),
            },
        })
        .collect()
}

fn runtime() -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("runtime")
}

fn marker(record: &Record) -> (u64, u64, u64) {
    let Entry::ModelFailed { detail } = &record.entry else {
        panic!("a record that no batch wrote: {record:?}");
    };
    let mut parts = detail.split(':');
    let number = parts.next().expect("batch number");
    let (index, size) = parts
        .next()
        .and_then(|rest| rest.split_once('/'))
        .expect("index and size");
    (
        number.parse().expect("batch number"),
        index.parse().expect("index"),
        size.parse().expect("size"),
    )
}

fn whole_batches(records: &[Record], reported: u64) -> Result<u64, String> {
    let mut seen: BTreeMap<u64, (u64, BTreeSet<u64>)> = BTreeMap::new();
    for record in records {
        let (number, index, size) = marker(record);
        let (expected_size, indexes) = seen.entry(number).or_insert((size, BTreeSet::new()));
        if *expected_size != size || !indexes.insert(index) {
            return Err(format!(
                "batch {number} holds a record twice or a stray one"
            ));
        }
    }
    for (number, (size, indexes)) in &seen {
        if indexes.len() as u64 != *size || *size != batch_size(*number) {
            return Err(format!(
                "batch {number} is partial: {} of {size} records",
                indexes.len()
            ));
        }
    }
    let present = seen.len() as u64;
    if seen.keys().copied().ne(0..present) {
        return Err(format!("batches are not 0..{present}: {:?}", seen.keys()));
    }
    if present < reported {
        return Err(format!(
            "{reported} batches were reported, {present} survived"
        ));
    }
    Ok(present)
}

fn recover(folder: &Path, reported: u64) -> Result<u64, String> {
    let records = runtime()
        .block_on(SqliteStore::new(folder, "crash").load())
        .map_err(|error| format!("the record did not open: {error}"))?;
    whole_batches(&records, reported)
}

fn scratch(name: &str) -> PathBuf {
    let folder = std::env::temp_dir().join(format!("dotori-crash-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&folder);
    folder
}

#[test]
#[ignore = "child process of the crash test, selected by FOLDER_ENV"]
fn crash_writer() {
    let Ok(folder) = std::env::var(FOLDER_ENV) else {
        return;
    };
    let store = SqliteStore::new(folder, "crash");
    let runtime = runtime();
    let mut stdout = std::io::stdout();
    writeln!(stdout, "{READY_LINE}").unwrap();
    stdout.flush().unwrap();
    for number in 0..WRITER_BATCH_CAP {
        runtime.block_on(store.append(batch(number))).unwrap();
        writeln!(stdout, "{APPENDED_PREFIX}{number}").unwrap();
        stdout.flush().unwrap();
    }
}

struct Writer {
    child: Child,
    stdout: BufReader<ChildStdout>,
}

impl Writer {
    fn spawn(folder: &Path) -> Self {
        let name = module_path!().split_once("::").expect("crate name").1;
        let mut child = Command::new(std::env::current_exe().unwrap())
            .args([&format!("{name}::crash_writer"), "--exact", "--ignored"])
            .arg("--nocapture")
            .env(FOLDER_ENV, folder)
            .stdout(Stdio::piped())
            .spawn()
            .unwrap();
        let stdout = BufReader::new(child.stdout.take().unwrap());
        Self { child, stdout }
    }

    fn next_line(&mut self) -> Option<String> {
        let mut line = String::new();
        let read = self.stdout.read_line(&mut line).unwrap_or(0);
        (read > 0 && line.ends_with('\n')).then(|| line.trim_end().to_owned())
    }

    fn next_appended(&mut self) -> Option<u64> {
        loop {
            let line = self.next_line()?;
            if let Some(number) = line.strip_prefix(APPENDED_PREFIX) {
                return Some(number.parse().unwrap());
            }
        }
    }

    fn wait_until_ready(&mut self) {
        while self.next_line().expect("writer exited before it was ready") != READY_LINE {}
    }

    fn kill_and_collect(mut self) -> Vec<u64> {
        self.child.kill().unwrap();
        self.child.wait().unwrap();
        std::iter::from_fn(|| self.next_appended()).collect()
    }
}

impl Drop for Writer {
    fn drop(&mut self) {
        self.child.kill().ok();
        self.child.wait().ok();
    }
}

#[test]
fn the_check_accepts_whole_batches_and_rejects_partial_or_missing_ones() {
    let whole: Vec<Record> = (0..3).flat_map(batch).collect();
    assert_eq!(whole_batches(&whole, 3), Ok(3));
    assert_eq!(whole_batches(&[], 0), Ok(0));
    assert!(whole_batches(&whole, 4).is_err());

    let mut partial = whole.clone();
    partial.pop();
    assert!(whole_batches(&partial, 0).is_err());

    let mut gap = whole.clone();
    gap.retain(|record| marker(record).0 != 1);
    assert!(whole_batches(&gap, 0).is_err());

    let mut twice = whole;
    twice.push(batch(2).remove(0));
    assert!(whole_batches(&twice, 0).is_err());
}

#[test]
fn a_record_survives_the_writer_being_killed_at_varied_points() {
    let root = scratch("kill");
    for trial in 0..TRIALS {
        let (batches_before_kill, delay_micros) = kill_point(trial);
        let folder = root.join(format!("trial-{trial}"));
        let mut writer = Writer::spawn(&folder);
        writer.wait_until_ready();
        let mut reported: Vec<u64> = (0..batches_before_kill)
            .map(|_| writer.next_appended().expect("writer exited early"))
            .collect();
        sleep(Duration::from_micros(delay_micros));
        reported.extend(writer.kill_and_collect());
        let in_order: Vec<u64> = (0..reported.len() as u64).collect();
        assert_eq!(reported, in_order, "trial {trial}: reports skipped");

        let survived = recover(&folder, reported.len() as u64)
            .unwrap_or_else(|reason| panic!("trial {trial}: {reason}"));
        assert!(survived <= reported.len() as u64 + 1, "trial {trial}");

        let again = recover(&folder, reported.len() as u64)
            .unwrap_or_else(|reason| panic!("trial {trial}: {reason}"));
        assert_eq!(again, survived, "trial {trial}: after reopening");
    }
    let _ = std::fs::remove_dir_all(&root);
}
