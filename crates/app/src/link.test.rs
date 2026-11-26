use super::*;

#[test]
fn starts_dotori_once_when_it_was_never_seen() {
    let (first, daemon) = step(Daemon::Unseen, true);
    assert_eq!(first, Step::Start);
    let (second, _) = step(daemon, true);
    assert_eq!(second, Step::Starting);
}

#[test]
fn never_starts_dotori_after_seeing_the_lock_held() {
    let (next, daemon) = step(Daemon::Seen, true);
    assert_eq!(next, Step::Gone);
    assert_eq!(daemon, Daemon::Seen);
}

#[test]
fn closes_when_dotori_stops_after_the_window_connected() {
    assert_eq!(step(Daemon::Reached, true).0, Step::Gone);
}

#[test]
fn a_held_lock_shows_the_link_failure_and_keeps_the_state() {
    for daemon in [
        Daemon::Unseen,
        Daemon::Started { free: 3 },
        Daemon::Seen,
        Daemon::Reached,
    ] {
        assert_eq!(step(daemon, false), (Step::Show, daemon));
    }
}

#[test]
fn gives_up_waiting_after_the_patience_runs_out() {
    let (_, mut daemon) = step(Daemon::Unseen, true);
    for _ in 1..START_RETRIES {
        let (next, after) = step(daemon, true);
        assert_eq!(next, Step::Starting);
        daemon = after;
    }
    assert_eq!(step(daemon, true).0, Step::CouldNotStart);
}
