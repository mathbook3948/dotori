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
        ("chat.asking", (chat.asking)("Dotori")),
        ("chat.in_browser_hint", (chat.in_browser_hint)()),
        ("chat.closed", (chat.closed)()),
        ("chat.unreachable", (chat.unreachable)()),
        ("chat.not_connected", (chat.not_connected)()),
        ("chat.input_card_on_pc", (chat.input_card_on_pc)()),
        ("chat.sandboxed", (chat.sandboxed)()),
        ("chat.rest_on_pc", (chat.rest_on_pc)()),
        ("chat.log_in_to", (chat.log_in_to)("example.com")),
        ("chat.say_to", (chat.say_to)()),
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
        ("common.onboarding_title", (common.onboarding_title)()),
        ("common.browser_title", (common.browser_title)()),
        (
            "common.theme",
            THEMES.map(|name| (common.theme)(name)).join(" "),
        ),
        ("onboarding.hello", (onboarding.hello)()),
        ("onboarding.key_use", (onboarding.key_use)()),
        ("onboarding.elsewhere_hint", (onboarding.elsewhere_hint)()),
        ("onboarding.join_failed", (onboarding.join_failed)()),
        ("onboarding.joined", (onboarding.joined)("laptop")),
        ("onboarding.received", (onboarding.received)("laptop")),
        (
            "onboarding.join_here_failed",
            (onboarding.join_here_failed)("locked"),
        ),
        ("pcs.about", (pcs.about)()),
        ("pcs.this_pc", (pcs.this_pc)("laptop")),
        ("pcs.reachable_at", (pcs.reachable_at)("10.0.0.2")),
        ("pcs.invite_hint", (pcs.invite_hint)(10)),
        ("pcs.joined", (pcs.joined)("laptop")),
        ("pcs.dropped", (pcs.dropped)(2)),
        ("routines.caption", (routines.caption)()),
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
        ("settings.name_too_long", (settings.name_too_long)(20)),
        ("settings.slack_about", (settings.slack_about)()),
        ("settings.telegram_about", (settings.telegram_about)()),
        ("settings.discord_about", (settings.discord_about)()),
        (
            "settings.not_a_key_name",
            (settings.not_a_key_name)("xoxb-1"),
        ),
        ("settings.notes_about", (settings.notes_about)(3, 20)),
        (
            "settings.channel_missing",
            (settings.channel_missing)("Slack", &[settings.missing_bot_token]),
        ),
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
    let name = crate::window::identity::name();

    for (field, text) in every_function(english) {
        assert!(!hangul(&text.replace(&name, "")), "{field}: {text}");
    }
}

#[test]
fn no_string_calls_the_assistant_dotori_in_lowercase() {
    for language in [Language::Korean, Language::English] {
        let words = of(language);
        let fixed = format!("{words:?}");
        assert!(
            !fixed.contains("dotori"),
            "a fixed string says dotori: {fixed}"
        );
        for (field, text) in every_function(words) {
            assert!(!text.contains("dotori"), "{field}: {text}");
        }
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

#[test]
fn a_channel_names_every_missing_field_with_the_right_particles() {
    let said = &crate::words::settings::KOREAN;
    assert_eq!(
        (said.channel_missing)("Slack", &[said.missing_app_token, said.missing_member_id]),
        "Slack을 쓰려면 앱 토큰, 내 멤버 ID를 넣어 주세요."
    );
    assert_eq!(
        (said.channel_missing)("Discord", &[said.missing_bot_token]),
        "Discord를 쓰려면 봇 토큰을 넣어 주세요."
    );
    let said = &crate::words::settings::ENGLISH;
    assert_eq!(
        (said.channel_missing)(
            "Slack",
            &[
                said.missing_app_token,
                said.missing_bot_token,
                said.missing_member_id
            ]
        ),
        "To use Slack, enter the app token, the bot token and your member ID."
    );
}
