use super::*;

#[test]
fn hex_values_become_colors() {
    let color = hex(0x2F6F_EB40);

    assert!((color.r - 47.0 / 255.0).abs() < f32::EPSILON);
    assert!((color.g - 111.0 / 255.0).abs() < f32::EPSILON);
    assert!((color.b - 235.0 / 255.0).abs() < f32::EPSILON);
    assert!((color.a - 64.0 / 255.0).abs() < f32::EPSILON);
}

#[test]
fn every_theme_of_the_design_is_known_by_name() {
    assert_eq!(
        names().collect::<Vec<_>>(),
        ["acorn", "graphite", "paper", "forest", "indigo"],
        "acorn comes first, so it is the default"
    );
    assert!(named("forest").is_some());
    assert!(named("purple").is_none());
}

#[test]
fn looks_read_back_as_they_are_written() {
    for look in [Look::System, Look::Light, Look::Dark] {
        assert_eq!(Look::named(look.name()), look);
    }
    assert_eq!(Look::named("sepia"), Look::System);
}
