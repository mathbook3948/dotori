use super::*;

const DESIGN_MD: &str = "---\nversion: alpha\nname: demo\ncolors:\n  plain:\n    light:\n      canvas: \"#F4F5F7\"\n      selection: \"#2F6FEB40\"\n    dark:\n      canvas: \"#0F1114\"\n      selection: \"#5B8DEF55\"\n  logo:\n    light:\n      cap: \"#5B3A1E\"\n    dark:\n      cap: \"#8B5A2B\"\ntypography:\n  body:\n    fontSize: 14px\n---\n\n# demo\n";

#[test]
fn the_themes_and_the_logo_are_read_from_the_front_matter() {
    let colors = colors(DESIGN_MD).expect("read");

    assert_eq!(colors.themes.len(), 1);
    let (name, light, dark) = &colors.themes[0];
    assert_eq!(name, "plain");
    assert_eq!(
        light.tokens,
        [
            (String::from("canvas"), String::from("#F4F5F7")),
            (String::from("selection"), String::from("#2F6FEB40")),
        ]
    );
    assert_eq!(
        dark.tokens[0],
        (String::from("canvas"), String::from("#0F1114"))
    );
    assert_eq!(
        colors.logo.0.tokens,
        [(String::from("cap"), String::from("#5B3A1E"))]
    );
}

#[test]
fn colors_become_rgba_values() {
    assert_eq!(hex("#F4F5F7"), Some(0xF4F5_F7FF));
    assert_eq!(hex("#2F6FEB40"), Some(0x2F6F_EB40));
    assert_eq!(hex("blue"), None);
}

#[test]
fn a_look_missing_a_token_or_a_bad_color_is_refused() {
    let uneven = DESIGN_MD.replace("      selection: \"#5B8DEF55\"\n", "");
    assert!(
        colors(&uneven)
            .expect_err("refused")
            .contains("different tokens")
    );

    let bad = DESIGN_MD.replace("#0F1114", "dark grey");
    assert!(
        colors(&bad)
            .expect_err("refused")
            .contains("is not #RRGGBB")
    );
}

#[test]
fn the_rendered_code_names_each_theme_and_token() {
    let rendered = render(&colors(DESIGN_MD).expect("read"));

    assert!(
        rendered
            .contains("(\"plain\", Theme { canvas: hex(0xF4F5F7FF),selection: hex(0x2F6FEB40), }")
    );
    assert!(rendered.contains("pub(crate) const THEMES: [(&str, Theme, Theme); 1]"));
    assert!(rendered.contains("Logo { cap: hex(0x5B3A1EFF), }"));
}
