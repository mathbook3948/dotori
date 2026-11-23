use super::*;

#[test]
fn gpui_keys_become_dom_keys() {
    assert_eq!(
        dom_key("enter"),
        Some((String::from("Enter"), 13, Some(String::from("\r"))))
    );
    assert_eq!(dom_key("left"), Some((String::from("ArrowLeft"), 37, None)));
    assert_eq!(
        dom_key("a"),
        Some((String::from("a"), 65, Some(String::from("a"))))
    );
    assert_eq!(dom_key("f13"), None);
}

#[test]
fn a_wide_page_is_fitted_and_centered_in_a_tall_area() {
    let area = Bounds::new(point(px(10.), px(20.)), gpui::size(px(400.), px(400.)));

    let shown = fitted(area, (1280.0, 800.0));

    assert_eq!(shown.size.width, px(400.));
    assert_eq!(shown.size.height, px(250.));
    assert_eq!(shown.origin, point(px(10.), px(95.)));
}

#[test]
fn modifiers_use_devtools_bits() {
    let held = Modifiers {
        control: true,
        shift: true,
        ..Modifiers::default()
    };
    assert_eq!(bits(held), 2 | 8);
}
