use super::*;

fn hangul(text: &str) -> bool {
    text.chars().any(|c| ('\u{AC00}'..='\u{D7A3}').contains(&c))
}

#[test]
fn the_english_strings_carry_no_korean() {
    let english = format!("{:?}", of(Language::English));

    assert!(!hangul(&english), "{english}");
}

#[test]
fn every_theme_has_a_name_in_each_language() {
    for name in ["acorn", "graphite", "paper", "forest", "indigo"] {
        assert_ne!(
            (KOREAN.common.theme)(name),
            (KOREAN.common.theme)("unknown")
        );
        assert_ne!(
            (ENGLISH.common.theme)(name),
            (ENGLISH.common.theme)("unknown")
        );
    }
}
