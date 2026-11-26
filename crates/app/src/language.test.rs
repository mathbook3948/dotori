use super::*;

#[test]
fn korean_locales_speak_korean_and_the_rest_english() {
    assert_eq!(Language::of_locale("ko-KR"), Language::Korean);
    assert_eq!(Language::of_locale("ko_KR.UTF-8"), Language::Korean);
    assert_eq!(Language::of_locale("en-US"), Language::English);
    assert_eq!(Language::of_locale("ja-JP"), Language::English);
    assert_eq!(Language::of_locale("kok-IN"), Language::English);
}

#[test]
fn a_tag_is_read_by_its_primary_subtag_in_any_case() {
    for korean in ["ko", "KO", "Ko", "ko_KR", "ko-KR", "ko_KR.UTF-8"] {
        assert_eq!(Language::of_tag(korean), Some(Language::Korean), "{korean}");
    }
    for english in ["en", "EN", "en-US", "en_GB.UTF-8"] {
        assert_eq!(
            Language::of_tag(english),
            Some(Language::English),
            "{english}"
        );
    }
    for other in ["kok", "kor", "eng", "fr", "", "-ko"] {
        assert_eq!(Language::of_tag(other), None, "{other}");
    }
}

#[test]
fn choices_read_back_as_they_are_written() {
    for choice in Choice::ALL {
        assert_eq!(Choice::named(Some(choice.code())), choice);
    }
    assert_eq!(Choice::named(None), Choice::System);
    assert_eq!(Choice::named(Some("SYSTEM")), Choice::System);
    assert_eq!(Choice::named(Some("ko-KR")), Choice::Only(Language::Korean));
    assert_eq!(Choice::named(Some("EN")), Choice::Only(Language::English));
}

#[test]
fn an_unknown_choice_follows_the_system() {
    assert_eq!(Choice::named(Some("fr")), Choice::System);
    assert_eq!(Choice::named(Some("kok")), Choice::System);
}
