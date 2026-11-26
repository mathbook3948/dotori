use super::*;
use crate::language::Language;
use crate::words::of;
use crate::words::speak;

#[test]
fn dotori_s_own_tools_are_named_as_the_owner_would_say() {
    speak(Language::Korean);
    assert_eq!(own_tool("run_shell"), Some("명령 실행"));
    assert_eq!(own_tool("erase_records"), Some("기록 지우기"));
    assert_eq!(own_tool("mail__send"), None);
    assert_eq!(field_label("command"), "명령");
    assert_eq!(field_label("to"), "to");

    let english = speak(Language::English);
    assert_eq!(own_tool("run_shell"), Some(english.chat.run_shell));
    assert_eq!(field_label("command"), "Command");
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
    speak(Language::Korean);
    let long = "가".repeat(SHOWN_VALUE + 10);
    let input = serde_json::json!({ "content": long }).to_string();

    let (cut, shortened) = asked_fields("write_file", &input, false);
    let (whole, _) = asked_fields("write_file", &input, true);

    assert!(shortened);
    assert!(cut[0].1.ends_with("(전체 610자 중 앞부분)"));
    assert_eq!(whole[0].1.chars().count(), SHOWN_VALUE + 10);

    speak(Language::English);
    let (cut, _) = asked_fields("write_file", &input, false);
    assert!(cut[0].1.ends_with("(first part of 610 characters)"));
}

#[test]
fn a_card_says_what_allowing_does() {
    speak(Language::Korean);
    assert_eq!(
        hint("run_shell", "별이"),
        "허락하면 별이 이 명령을 바로 실행해요"
    );
    assert_eq!(
        hint("write_file", "dotori가"),
        "허락하면 dotori가 파일을 바로 바꿔요"
    );
    assert_eq!(
        hint("erase_records", "dotori가"),
        of(Language::Korean).chat.erase_hint
    );
    assert_eq!(hint("mail__send", "dotori가"), "허락하면 바로 실행해요");
    assert_eq!(title("별이"), "별이 이 일을 해도 될지 물어요");

    let english = speak(Language::English);
    assert_eq!(
        hint("run_shell", "별이"),
        "Allow and I'll run this command right away."
    );
    assert_eq!(hint("mail__send", "별이"), english.chat.allow_hint);
    assert_eq!(title("별이"), "Can I do this?");
}

#[test]
fn a_log_in_card_names_the_account_in_either_language() {
    speak(Language::Korean);
    assert_eq!(
        log_in_title("login.coupang.com", "쿠팡"),
        "금고의 login.coupang.com (쿠팡) 계정으로 로그인해도 될까요?"
    );
    assert_eq!(
        log_in_note("별이", "login.coupang.com", "/login"),
        "별이 login.coupang.com/login 로그인 칸에 직접 채워 넣어요. 대화하는 AI는 비밀번호를 보지 못해요."
    );

    speak(Language::English);
    assert_eq!(
        log_in_title("login.coupang.com", "Coupang"),
        "Can I log in with the login.coupang.com (Coupang) account from your vault?"
    );
}
