use super::*;

#[test]
fn the_logo_comes_at_twice_the_size_it_is_drawn_at() {
    let asked = logo_named(56.);

    let svg = Assets.load(&asked).unwrap().unwrap();
    let svg = std::str::from_utf8(&svg).unwrap();

    assert_eq!(asked, "logo/light/112.svg");
    assert!(svg.contains(r#"height="56""#));
    assert!(svg.contains(&format!(r#"width="{}""#, 56. * ACORN_WIDTH / ACORN_HEIGHT)));
}

#[test]
fn a_picture_is_named_for_its_size_and_only_the_chosen_one_is_served() {
    assert_eq!(
        picture(Path::new("pictures/acorn.png"), 20.),
        SharedString::from("picture/40/acorn.png")
    );
    assert_eq!(
        sized("picture/40/acorn.png", PICTURE),
        Some((40, "acorn.png"))
    );
    assert_eq!(sized("logo/light.svg", LIGHT_LOGO), None);
    assert!(Assets.load("picture/40/not-chosen.png").unwrap().is_none());
}

fn logo_named(size: f32) -> String {
    format!("{LIGHT_LOGO}/{}.svg", side(size))
}

#[test]
fn indentation_and_runs_of_spaces_stay_with_the_words() {
    assert_eq!(
        words_keeping_spaces("    return x"),
        vec!["    return", "x"]
    );
    assert_eq!(words_keeping_spaces("a  b   c "), vec!["a", " b", "  c"]);
    assert_eq!(words_keeping_spaces("one two"), vec!["one", "two"]);
}
