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

fn home(name: &str) -> std::path::PathBuf {
    let folder =
        std::env::temp_dir().join(format!("dotori-language-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&folder);
    std::fs::create_dir_all(&folder).expect("folder");
    folder
}

#[test]
fn settling_saves_a_language_where_none_is_kept_and_leaves_a_kept_one() {
    let folder = home("settle");
    let path = folder.join("settings.toml");
    std::fs::write(&path, "# 내 설정\n[memory]\n").expect("written");

    let settled = settle(&path).expect("settled").expect("one was saved");

    assert_eq!(saved(&path), settled);
    assert_eq!(
        Settings::load(&path).expect("loaded").language,
        Some(settled)
    );
    assert!(
        std::fs::read_to_string(&path)
            .expect("read")
            .contains("# 내 설정"),
        "the owner's comments stay"
    );
    assert_eq!(settle(&path).expect("settled"), None, "it settles once");

    let kept = home("kept").join("settings.toml");
    std::fs::write(&kept, "language = \"en\"\n").expect("written");
    assert_eq!(settle(&kept).expect("settled"), None);
    assert_eq!(saved(&kept), Language::English);
    assert_eq!(
        std::fs::read_to_string(&kept).expect("read"),
        "language = \"en\"\n"
    );
    let _ = std::fs::remove_dir_all(&folder);
    let _ = std::fs::remove_dir_all(kept.parent().expect("folder"));
}

#[test]
fn a_language_an_older_dotori_wrote_settles_to_a_concrete_one() {
    for (old, kept) in [
        ("system", None),
        ("fr", None),
        ("ko-KR", Some(Language::Korean)),
    ] {
        let folder = home("old");
        let path = folder.join("settings.toml");
        std::fs::write(&path, format!("language = \"{old}\"\n")).expect("written");

        assert_eq!(
            Settings::load(&path).expect("still loads").language,
            kept,
            "{old}"
        );
        let settled = settle(&path).expect("settled");
        assert_eq!(settled.is_some(), kept.is_none(), "{old}");
        let now = Settings::load(&path).expect("loaded").language;
        assert!(now.is_some(), "{old} became a language");
        assert_eq!(saved(&path), now.expect("some"));
        let _ = std::fs::remove_dir_all(&folder);
    }
}

#[test]
fn a_language_is_read_and_written_as_its_code() {
    for language in Language::ALL {
        let text = toml::to_string(&Settings {
            language: Some(language),
            ..Settings::default()
        })
        .expect("written");
        assert!(text.contains(&format!("language = \"{}\"", language.code())));
    }
    assert!(serde_json::from_str::<Language>("\"fr\"").is_err());
    assert_eq!(
        serde_json::from_str::<Language>("\"en\"").expect("read"),
        Language::English
    );
}
