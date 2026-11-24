use super::*;

#[test]
fn korean_names_take_the_particle_their_last_sound_asks_for() {
    assert!(ends_closed("별"));
    assert!(!ends_closed("도토리"));
    assert!(ends_closed("다람쥐봇"));
    assert!(!ends_closed("dotori"));
    assert!(ends_closed("Sam"));
    assert!(!ends_closed("Jarvis2"));
    assert!(ends_closed("Ring"));
    assert!(!ends_closed(""));
}
