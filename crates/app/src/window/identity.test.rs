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
    assert!(ends_closed("Jane"));
    assert!(ends_closed("Bob"));
    assert!(ends_closed("Robot"));
    assert!(!ends_closed("Kate"));
    assert!(ends_closed("별⭐"));
    assert!(ends_closed("R"));
    assert!(!ends_closed("K"));
}

#[test]
fn the_copula_follows_the_last_sound_of_the_name() {
    assert_eq!(followed("별", Particle::Is), "별이에요");
    assert_eq!(followed("도토리", Particle::Is), "도토리예요");
    assert_eq!(followed("dotori", Particle::Is), "dotori예요");
}
