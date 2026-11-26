use super::*;

#[test]
fn korean_locales_speak_korean_and_the_rest_english() {
    assert_eq!(Language::of_locale("ko-KR"), Language::Korean);
    assert_eq!(Language::of_locale("ko_KR.UTF-8"), Language::Korean);
    assert_eq!(Language::of_locale("en-US"), Language::English);
    assert_eq!(Language::of_locale("ja-JP"), Language::English);
}

#[test]
fn choices_read_back_as_they_are_written() {
    for choice in Choice::ALL {
        assert_eq!(Choice::named(Some(choice.code())), choice);
    }
    assert_eq!(Choice::named(None), Choice::System);
    assert_eq!(Choice::named(Some("fr")), Choice::System);
}
