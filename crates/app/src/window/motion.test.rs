use super::*;

const DESIGN: &str = include_str!("../../../../DESIGN.md");

fn token(name: &str) -> (String, String) {
    let mut lines = DESIGN.lines().skip_while(|line| *line != "motion:");
    let start = format!("  {name}:");
    let _ = lines.find(|line| *line == start);
    let mut field = |key: &str| {
        let line = lines.next().unwrap_or_default();
        let value = line
            .trim()
            .strip_prefix(key)
            .unwrap_or_else(|| panic!("motion.{name} has {key} next in DESIGN.md, found `{line}`"));
        value.trim().to_owned()
    };
    let duration = field("duration:");
    let curve = field("timingFunction:");
    (duration, curve)
}

fn named(curve: Curve) -> &'static str {
    match curve {
        Curve::EaseOut => "ease-out",
        Curve::EaseInOut => "ease-in-out",
    }
}

#[test]
fn the_motion_tokens_match_design_md() {
    for (name, motion) in [
        ("hover", HOVER),
        ("message-enter", MESSAGE_ENTER),
        ("follow", FOLLOW),
        ("working", WORKING),
        ("greet", GREET),
        ("step", STEP),
        ("switch", SWITCH),
        ("glide", GLIDE),
    ] {
        assert_eq!(
            token(name),
            (
                format!("{}ms", motion.duration.as_millis()),
                named(motion.curve).to_owned()
            ),
            "motion.{name}"
        );
        assert_eq!(motion.name, name, "a token goes by its name in DESIGN.md");
    }
    assert!(DESIGN.contains(&format!("  rise: {}px\n", RISE as u32)));
    assert!(DESIGN.contains(&format!("  slide: {}px\n", SLIDE as u32)));
}

#[test]
fn a_movement_starts_at_zero_ends_at_one_and_never_overshoots() {
    for curve in [Curve::EaseOut, Curve::EaseInOut] {
        assert_eq!(curve.ease(0.0), 0.0);
        assert_eq!(curve.ease(1.0), 1.0);
        assert_eq!(curve.ease(2.0), 1.0);
        let mut last = 0.0;
        for step in 1..=100 {
            let now = curve.ease(step as f32 / 100.0);
            assert!(now >= last && now <= 1.0, "{curve:?} at {step}");
            last = now;
        }
    }
    assert!(
        Curve::EaseOut.ease(0.5) > 0.5,
        "what arrives slows at the end"
    );
}

#[test]
fn a_movement_is_done_once_its_time_is_up() {
    assert_eq!(MESSAGE_ENTER.at(Duration::ZERO), 0.0);
    assert_eq!(MESSAGE_ENTER.at(Duration::from_millis(180)), 1.0);
    assert!(MESSAGE_ENTER.moving(Duration::from_millis(179)));
    assert!(!MESSAGE_ENTER.moving(Duration::from_millis(180)));
}

#[test]
fn a_breath_runs_into_the_next_without_a_jump() {
    assert_eq!(breath(0.0), 1.0);
    assert_eq!(breath(1.0), 1.0);
    assert!((breath(0.5) - 0.45).abs() < 1e-6);
    assert!((breath(0.25) - breath(0.75)).abs() < 1e-6);
}
