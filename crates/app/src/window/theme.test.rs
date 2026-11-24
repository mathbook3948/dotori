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
        ["graphite", "paper", "forest", "indigo", "acorn"]
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

#[test]
fn the_glass_opacities_match_design_md() {
    let design = include_str!("../../../../DESIGN.md");
    assert!(design.contains(&format!(
        "glass:\n  window: {GLASS_WINDOW}\n  surface: {GLASS_SURFACE}\n"
    )));
}

#[test]
fn a_see_through_theme_lets_the_desktop_through_its_canvas_and_fills_only() {
    let (light, _) = named("graphite").expect("graphite");

    let glass = light.see_through();

    assert_eq!(glass.canvas.a, GLASS_WINDOW);
    assert_eq!(glass.surface.a, GLASS_SURFACE);
    assert_eq!(glass.warn_soft.a, GLASS_SURFACE);
    assert_eq!(glass.text, light.text, "text stays solid");
    assert_eq!(glass.primary, light.primary, "the main button stays solid");
}
