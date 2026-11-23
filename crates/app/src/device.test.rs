use std::sync::atomic::AtomicUsize;
use std::sync::atomic::Ordering;

use super::*;

static NEXT: AtomicUsize = AtomicUsize::new(0);

fn scratch() -> std::path::PathBuf {
    let n = NEXT.fetch_add(1, Ordering::SeqCst);
    let dir = std::env::temp_dir().join(format!("dotori-device-{}-{n}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    dir.join("device")
}

#[test]
fn a_pc_keeps_the_name_it_chose_first() {
    let path = scratch();

    let first = name(&path).expect("chosen");
    let again = name(&path).expect("read");

    assert_eq!(first, again);
    assert!(is_name(&first), "{first}");
    assert_ne!(name(&scratch()).expect("chosen"), first);
}

#[test]
fn a_damaged_name_is_refused_rather_than_replaced() {
    let path = scratch();
    fs::create_dir_all(path.parent().expect("has a folder")).expect("made");
    fs::write(&path, "not a name").expect("written");

    assert!(name(&path).is_err());
    assert_eq!(fs::read_to_string(&path).expect("kept"), "not a name");
}
