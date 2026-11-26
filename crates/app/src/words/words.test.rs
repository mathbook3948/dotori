use jiff::civil::Weekday;
use jiff::civil::date;

use super::*;

fn hangul(text: &str) -> bool {
    text.chars().any(|c| ('\u{AC00}'..='\u{D7A3}').contains(&c))
}

const THEMES: [&str; 5] = ["acorn", "graphite", "paper", "forest", "indigo"];

fn functions_in(words: &Words) -> usize {
    let shown = format!("{words:?}");
    shown.matches("0x").count()
}

fn every_function(words: &Words) -> Vec<(&'static str, String)> {
    let (chat, common, onboarding, pcs, routines, screen, settings) = (
        &words.chat,
        &words.common,
        &words.onboarding,
        &words.pcs,
        &words.routines,
        &words.screen,
        &words.settings,
    );
    let mut said = vec![
        ("chat.run_hint", (chat.run_hint)("Dotori")),
        ("chat.write_hint", (chat.write_hint)("Dotori")),
        ("chat.asking", (chat.asking)("Dotori")),
        ("chat.in_browser_hint", (chat.in_browser_hint)()),
        ("chat.log_in_to", (chat.log_in_to)("example.com")),
        ("chat.answering_elsewhere", (chat.answering_elsewhere)("pc")),
        (
            "chat.say_to",
            (chat.say_to)().replace(&crate::window::identity::name(), ""),
        ),
        ("chat.command_exited", (chat.command_exited)(3)),
        ("chat.command_too_slow", (chat.command_too_slow)("110")),
        ("chat.cut_note", (chat.cut_note)(700)),
        ("chat.thread", (chat.thread)("general")),
        (
            "chat.decided_by",
            [true, false]
                .map(|allowed| (chat.decided_by)("Minji", allowed))
                .join(" "),
        ),
        (
            "chat.card_on_pc",
            [true, false]
                .map(|log_in| (chat.card_on_pc)("example.com", log_in))
                .join(" "),
        ),
        (
            "chat.log_in_with_kept",
            (chat.log_in_with_kept)("example.com", "Example"),
        ),
        (
            "chat.log_in_note",
            (chat.log_in_note)("Dotori", "example.com", "/login"),
        ),
        ("chat.log_in_as", (chat.log_in_as)("minji")),
        (
            "common.theme",
            THEMES.map(|name| (common.theme)(name)).join(" "),
        ),
        ("onboarding.hello", (onboarding.hello)()),
        ("onboarding.key_use", (onboarding.key_use)()),
        ("onboarding.elsewhere_hint", (onboarding.elsewhere_hint)()),
        ("pcs.about", (pcs.about)()),
        ("pcs.this_pc", (pcs.this_pc)("laptop")),
        ("pcs.reachable_at", (pcs.reachable_at)("10.0.0.2")),
        ("pcs.invite_hint", (pcs.invite_hint)(10)),
        ("pcs.joined", (pcs.joined)("laptop")),
        ("pcs.dropped", (pcs.dropped)(2)),
        ("routines.caption", (routines.caption)()),
        ("routines.zone_here", (routines.zone_here)("Asia/Seoul")),
        (
            "routines.weekday_short",
            [
                Weekday::Monday,
                Weekday::Tuesday,
                Weekday::Wednesday,
                Weekday::Thursday,
                Weekday::Friday,
                Weekday::Saturday,
                Weekday::Sunday,
            ]
            .map(|day| (routines.weekday_short)(day))
            .join(" "),
        ),
        ("routines.once", (routines.once)(date(2026, 12, 24))),
        ("routines.once_raw", (routines.once_raw)("12/24")),
        (
            "routines.day_this_year",
            (routines.day_this_year)(date(2026, 11, 2)),
        ),
        (
            "routines.day_other_year",
            (routines.day_other_year)(date(2025, 11, 2)),
        ),
        (
            "routines.clock",
            [(0, 5), (12, 0), (15, 7)]
                .map(|(hour, minute)| (routines.clock)(hour, minute))
                .join(" "),
        ),
        ("screen.agent", (screen.agent)()),
        ("screen.owner", (screen.owner)()),
        ("screen.idle", (screen.idle)()),
        ("screen.page_connecting", (screen.page_connecting)()),
        ("screen.page_empty", (screen.page_empty)()),
        ("screen.asks", (screen.asks)("a code")),
        ("settings.model_about", (settings.model_about)()),
        (
            "settings.key_hint",
            [Some("KEY"), None]
                .map(|name| (settings.key_hint)(name))
                .join(" "),
        ),
        ("settings.name_too_long", (settings.name_too_long)(20)),
        ("settings.slack_about", (settings.slack_about)()),
        (
            "settings.not_a_key_name",
            (settings.not_a_key_name)("xoxb-1"),
        ),
        ("settings.notes_about", (settings.notes_about)(3, 20)),
        (
            "settings.name_is_an_account",
            (settings.name_is_an_account)("ACCOUNT_1"),
        ),
        (
            "settings.read_from_environment",
            (settings.read_from_environment)("Slack"),
        ),
    ];
    said.push(("every string", format!("{words:?}")));
    said
}

#[test]
fn the_english_strings_carry_no_korean() {
    let english = of(Language::English);

    let said = every_function(english);

    for (field, text) in &said {
        assert!(!hangul(text), "{field}: {text}");
    }
}

#[test]
fn every_function_in_the_tables_is_called_by_the_check() {
    for language in [Language::Korean, Language::English] {
        let words = of(language);
        assert_eq!(
            every_function(words).len() - 1,
            functions_in(words),
            "a function added to the tables needs a line in every_function"
        );
    }
}

#[test]
fn every_theme_has_a_name_in_each_language() {
    for name in THEMES {
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

#[test]
fn a_test_can_speak_either_language_whatever_the_locale() {
    assert_eq!(speak(Language::English).chat.allow, "Allow");
    assert_eq!(words().chat.allow, "Allow");
    assert_eq!(speak(Language::Korean).chat.allow, "허락");
    assert_eq!(words().chat.allow, "허락");
}
