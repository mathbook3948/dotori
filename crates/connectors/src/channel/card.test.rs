use super::*;

fn card() -> Card {
    Card {
        token: String::from("00112233445566778899aabb"),
        title: String::from("dotori가 이 일을 해도 될지 물어요"),
        rows: vec![
            (String::from("도구"), String::from("명령 실행")),
            (String::from("명령"), String::from("ls\n-la")),
        ],
        cut: true,
        notes: vec![String::from("허락하면 바로 실행해요")],
        buttons: vec![
            Button {
                label: String::from("거절"),
                primary: false,
            },
            Button {
                label: String::from("허락"),
                primary: true,
            },
        ],
    }
}

#[test]
fn a_card_reads_as_its_title_rows_and_notes_and_says_when_it_was_shortened() {
    assert_eq!(
        card().text(None, 2000),
        "dotori가 이 일을 해도 될지 물어요\n\n도구: 명령 실행\n\n명령\nls\n-la\n\n전체 보기는 PC의 dotori 창에서 해요\n\n허락하면 바로 실행해요"
    );
}

#[test]
fn a_settled_card_keeps_what_it_said_within_the_limit() {
    let mut long = card();
    long.rows[1].1 = "가".repeat(3000);

    let text = long.text(Some("김민지 님이 허락했어요"), 2000);

    assert_eq!(text.chars().count(), 2000);
    assert!(text.ends_with("…\n\n김민지 님이 허락했어요"));
}

#[test]
fn a_button_carries_only_the_token_and_its_number() {
    let card = card();

    assert_eq!(card.value(1), "00112233445566778899aabb:1");
    assert_eq!(pressed(&card.value(1)), Some((card.token.clone(), 1)));
    assert_eq!(pressed("00112233445566778899aabb"), None);
    assert_eq!(pressed("not-a-token:1"), None);
    assert_eq!(pressed("00112233445566778899aabb:yes"), None);
}

#[test]
fn tokens_differ_and_fit_a_telegram_button() {
    let one = token().expect("random");
    let other = token().expect("random");

    assert_ne!(one, other);
    assert!(format!("{one}:99").len() <= 64);
    assert!(pressed(&format!("{one}:0")).is_some());
}

#[test]
fn a_long_label_is_cut() {
    let mut card = card();
    card.buttons[0].label = format!("{}로 로그인", "a".repeat(100));

    let labels: Vec<String> = card.labels().map(|(_, _, label)| label).collect();

    assert_eq!(labels[0].chars().count(), LABEL);
    assert!(labels[0].ends_with('…'));
    assert_eq!(labels[1], "허락");
}
