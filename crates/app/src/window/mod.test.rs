use super::*;

#[test]
fn a_window_closes_once_dotori_it_saw_running_stops() {
    let mut seen = false;

    assert!(!dotori_stopped(&mut seen, false), "never seen running");
    assert!(!dotori_stopped(&mut seen, true));
    assert!(!dotori_stopped(&mut seen, true));
    assert!(dotori_stopped(&mut seen, false));
}
