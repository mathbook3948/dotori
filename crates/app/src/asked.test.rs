use super::*;
use crate::words::words;

#[test]
fn dotori_s_own_tools_are_named_as_the_owner_would_say() {
    assert_eq!(own_tool("run_shell"), Some(words().chat.run_shell));
    assert_eq!(own_tool("erase_records"), Some(words().chat.erase_records));
    assert_eq!(own_tool("mail__send"), None);
    assert_eq!(field_label("command"), words().chat.command_field);
    assert_eq!(field_label("to"), "to");
}

#[test]
fn a_forget_card_shows_what_goes_and_not_the_numbers() {
    let input = r#"{"records":["019a"],"carriers":["019b"],"지울 기록":"2026-11-24 09:00 유노: 카드 번호"}"#;

    let (forget, _) = asked_fields("erase_records", input, false);
    let (other, _) = asked_fields("mail__send", input, false);

    let keys: Vec<&str> = forget.iter().map(|(key, _)| key.as_str()).collect();
    assert_eq!(keys, ["지울 기록"]);
    assert_eq!(other.len(), 3, "only a forget hides its numbers");
}

#[test]
fn a_long_value_is_cut_until_the_whole_is_asked_for() {
    let long = "가".repeat(SHOWN_VALUE + 10);
    let input = serde_json::json!({ "content": long }).to_string();

    let (cut, shortened) = asked_fields("write_file", &input, false);
    let (whole, _) = asked_fields("write_file", &input, true);

    assert!(shortened);
    assert!(
        cut[0]
            .1
            .ends_with(&(words().chat.cut_note)(SHOWN_VALUE + 10))
    );
    assert_eq!(whole[0].1.chars().count(), SHOWN_VALUE + 10);
}

#[test]
fn a_card_says_what_allowing_does() {
    assert_eq!(hint("run_shell", "별이"), (words().chat.run_hint)("별이"));
    assert_eq!(hint("erase_records", "dotori가"), words().chat.erase_hint);
    assert_eq!(hint("mail__send", "dotori가"), words().chat.allow_hint);
}
