use super::*;

const DESIGN: &str = include_str!("../../../../DESIGN.md");

fn token(section: &str, key: &str) -> String {
    let start = format!("{section}:");
    let field = format!("  {key}:");
    DESIGN
        .lines()
        .skip_while(|line| *line != start)
        .skip(1)
        .take_while(|line| line.starts_with("  "))
        .find_map(|line| line.strip_prefix(&field))
        .unwrap_or_else(|| panic!("{section}.{key} is in DESIGN.md"))
        .trim()
        .trim_matches('"')
        .to_owned()
}

fn shown(color: Rgba) -> String {
    let byte = |channel: f32| (channel * 255.).round() as u8;
    format!(
        "#{:02X}{:02X}{:02X}",
        byte(color.r),
        byte(color.g),
        byte(color.b)
    )
}

fn luminance(color: Rgba) -> f32 {
    let linear = |channel: f32| {
        if channel <= 0.04045 {
            channel / 12.92
        } else {
            ((channel + 0.055) / 1.055).powf(2.4)
        }
    };
    0.2126 * linear(color.r) + 0.7152 * linear(color.g) + 0.0722 * linear(color.b)
}

fn contrast(one: Rgba, other: Rgba) -> f32 {
    let (dark, light) = {
        let (one, other) = (luminance(one), luminance(other));
        (one.min(other), one.max(other))
    };
    (light + 0.05) / (dark + 0.05)
}

#[test]
fn the_sizes_match_design_md() {
    let pair = |(wide, tall): (f32, f32)| format!("{wide}x{tall}");
    assert_eq!(token("size", "window"), pair(WINDOW));
    assert_eq!(token("size", "window-min"), pair(WINDOW_MIN));
    assert!(DESIGN.contains(&format!("  greet-text: {}px\n", GREET_TEXT as u32)));
    for (key, size) in [
        ("picture", PICTURE),
        ("tile", TILE),
        ("avatar", AVATAR),
        ("sidebar", SIDEBAR),
    ] {
        assert_eq!(token("size", key), format!("{size}px"), "size.{key}");
    }
}

#[test]
fn the_tile_and_feature_colors_match_design_md() {
    assert_eq!(token("tile", "light"), shown(TILE_FILL.0));
    assert_eq!(token("tile", "dark"), shown(TILE_FILL.1));
    for (key, color) in [
        ("time", FEATURE.time),
        ("web", FEATURE.web),
        ("files", FEATURE.files),
        ("icon", FEATURE.icon),
    ] {
        assert_eq!(token("feature", key), shown(color), "feature.{key}");
    }
}

#[test]
fn a_feature_icon_stands_out_from_its_tile() {
    for tile in [FEATURE.time, FEATURE.web, FEATURE.files] {
        assert!(
            contrast(FEATURE.icon, tile) >= 3.,
            "{} on {}",
            shown(FEATURE.icon),
            shown(tile)
        );
    }
}
