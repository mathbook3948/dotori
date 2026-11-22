use super::*;

#[test]
fn a_second_daemon_cannot_take_the_lock_until_the_first_lets_go() {
    let path = std::env::temp_dir()
        .join(format!("dotori-lock-{}", std::process::id()))
        .join("daemon.lock");
    let _ = fs::remove_file(&path);

    assert!(!DaemonLock::held(&path).expect("checked"));
    let first = DaemonLock::take(&path).expect("taken");
    assert!(DaemonLock::held(&path).expect("checked"));
    assert_eq!(
        DaemonLock::take(&path).expect_err("refused"),
        "dotori is already running"
    );

    drop(first);

    assert!(!DaemonLock::held(&path).expect("checked"));
    DaemonLock::take(&path).expect("taken again");
}
