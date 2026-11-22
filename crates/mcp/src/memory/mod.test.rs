use std::sync::atomic::AtomicUsize;
use std::sync::atomic::Ordering;

use super::*;

static NEXT: AtomicUsize = AtomicUsize::new(0);

fn config() -> MemoryConfig {
    let n = NEXT.fetch_add(1, Ordering::SeqCst);
    let folder = std::env::temp_dir().join(format!("dotori-memory-{}-{n}", std::process::id()));
    let _ = std::fs::remove_dir_all(&folder);
    std::fs::create_dir_all(&folder).expect("made");
    MemoryConfig {
        record: folder.join("record.sqlite3"),
        index: folder.join("search.sqlite3"),
        dictionaries: folder.join("dictionaries"),
    }
}

fn record(config: &MemoryConfig, rows: &[(&str, Option<&str>, Option<&str>)]) {
    let record = Connection::open(&config.record).expect("opened");
    record
        .execute_batch(
            "CREATE TABLE IF NOT EXISTS records (seq INTEGER PRIMARY KEY AUTOINCREMENT, \
             at_ms INTEGER NOT NULL, kind TEXT NOT NULL, channel TEXT, place TEXT, sender_id TEXT, \
             sender_name TEXT, role TEXT, text TEXT, line TEXT NOT NULL)",
        )
        .expect("made");
    for (kind, sender, text) in rows {
        record
            .execute(
                "INSERT INTO records (at_ms, kind, channel, place, sender_name, role, text, line) \
                 VALUES (1700000000123, ?1, 'slack', 'C1', ?2, ?3, ?4, '{}')",
                params![kind, sender, sender.map(|_| "other"), text],
            )
            .expect("inserted");
    }
}

fn memory(config: MemoryConfig) -> Arc<Memory> {
    Arc::new(Memory {
        config,
        state: Mutex::new(State {
            index: None,
            analyzer: Analyzer::Plain,
        }),
    })
}

#[test]
fn a_search_finds_records_by_their_words_and_names_them() {
    let config = config();
    record(
        &config,
        &[
            ("heard", Some("민지"), Some("내일 회의는 3시에 해요")),
            ("tool_returned", None, None),
            ("heard", Some("유노"), Some("점심 메뉴 추천해줘")),
        ],
    );
    let memory = memory(config.clone());

    let found = memory.search("회의 언제", 10).expect("searched");
    assert_eq!(
        found,
        "[record 1] 2023-11-14 22:13 UTC in slack C1, 민지 (other): 내일 회의는 3시에 해요"
    );
    assert_eq!(memory.search("민지가", 10).expect("by sender"), found);
    assert_eq!(
        memory.search("저녁", 10).expect("searched"),
        "Nothing in memory matches."
    );

    record(&config, &[("acted", None, Some("회의 장소는 2층입니다"))]);
    let later = memory.search("회의", 10).expect("caught up");
    assert!(
        later.contains("[record 4]") && later.contains("dotori: 회의 장소"),
        "{later}"
    );
    let _ = std::fs::remove_dir_all(config.record.parent().expect("a folder"));
}

#[test]
fn nothing_recorded_yet_is_not_an_error() {
    let memory = memory(config());
    assert_eq!(
        memory.search("아무거나", 10).expect("searched"),
        "Nothing is recorded yet."
    );
}

#[test]
fn a_query_of_only_symbols_is_refused() {
    let config = config();
    record(&config, &[("heard", Some("민지"), Some("안녕"))]);
    assert!(memory(config).search("?!", 10).is_err());
}

#[test]
fn a_change_of_analyzer_rebuilds_the_index() {
    let config = config();
    record(&config, &[("heard", Some("민지"), Some("회의"))]);
    let mut index = open_index(&config.index).expect("index");
    let source = open_record(&config.record).expect("read").expect("there");
    catch_up(&mut index, &source, &Analyzer::Plain).expect("caught up");
    index
        .execute("UPDATE meta SET value = 'other' WHERE key = 'analyzer'", [])
        .expect("changed");
    catch_up(&mut index, &source, &Analyzer::Plain).expect("rebuilt");
    let count: i64 = index
        .query_row("SELECT count(*) FROM terms", [], |row| row.get(0))
        .expect("counted");
    assert_eq!(count, 1);
}

#[test]
fn times_are_shown_in_utc() {
    assert_eq!(when(0), "1970-01-01 00:00 UTC");
    assert_eq!(when(951_825_600_000), "2000-02-29 12:00 UTC");
    assert_eq!(when(1_767_225_599_000), "2025-12-31 23:59 UTC");
}

#[test]
#[ignore = "needs the Korean dictionary in DOTORI_KO_DIC"]
fn the_korean_dictionary_finds_inflected_words() {
    let path = PathBuf::from(std::env::var("DOTORI_KO_DIC").expect("set"));
    let korean = Analyzer::korean(&path).expect("loaded");
    let terms = korean.terms("내일 회의는 민지네 사무실에서 했었다");
    assert!(terms.contains(&String::from("회의")), "{terms:?}");
    assert!(terms.contains(&String::from("사무실")), "{terms:?}");
    assert!(terms.contains(&String::from("하")), "{terms:?}");
    assert!(
        !terms.iter().any(|term| term == "는" || term == "에서"),
        "{terms:?}"
    );
}
