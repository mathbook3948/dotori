use super::*;

const BOT: &str = "UBOT";

fn event(event: &str) -> String {
    format!(r#"{{"type":"events_api","envelope_id":"e1","payload":{{"event":{event}}}}}"#)
}

fn message(frame: &Frame) -> &Arrived {
    match &frame.kind {
        Kind::Message(arrived) => arrived,
        other => panic!("expected a message, got {other:?}"),
    }
}

#[test]
fn a_mention_in_a_channel_is_answered_in_its_thread() {
    let frame = read(
        &event(
            r#"{"type":"app_mention","channel":"C1","user":"U1","text":"<@UBOT> hi &amp; bye","ts":"10.5"}"#,
        ),
        BOT,
    );
    assert_eq!(frame.ack.as_deref(), Some("e1"));
    assert_eq!(
        message(&frame),
        &Arrived {
            id: String::from("C1/10.5"),
            place: String::from("C1/10.5"),
            user: String::from("U1"),
            text: String::from("hi & bye"),
        }
    );
}

#[test]
fn a_mention_inside_a_thread_stays_in_that_thread() {
    let frame = read(
        &event(
            r#"{"type":"app_mention","channel":"C1","user":"U1","text":"<@UBOT> more","ts":"11.0","thread_ts":"10.5"}"#,
        ),
        BOT,
    );
    assert_eq!(message(&frame).place, "C1/10.5");
    assert_eq!(message(&frame).id, "C1/11.0");
}

#[test]
fn a_direct_message_is_answered_in_the_conversation() {
    let frame = read(
        &event(
            r#"{"type":"message","channel_type":"im","channel":"D1","user":"U1","text":"안녕","ts":"3.0"}"#,
        ),
        BOT,
    );
    assert_eq!(message(&frame).place, "D1");
    assert_eq!(message(&frame).text, "안녕");
}

#[test]
fn messages_not_for_the_assistant_are_acknowledged_and_dropped() {
    let cases = [
        r#"{"type":"message","channel_type":"channel","channel":"C1","user":"U1","text":"hi","ts":"1.0"}"#,
        r#"{"type":"message","channel_type":"im","channel":"D1","user":"UBOT","text":"my own","ts":"1.0"}"#,
        r#"{"type":"message","channel_type":"im","channel":"D1","bot_id":"B1","user":"U2","text":"a bot","ts":"1.0"}"#,
        r#"{"type":"message","subtype":"message_changed","channel_type":"im","channel":"D1","user":"U1","text":"edit","ts":"1.0"}"#,
        r#"{"type":"app_mention","channel":"C1","user":"U1","text":"<@UBOT>","ts":"1.0"}"#,
    ];
    for case in cases {
        let frame = read(&event(case), BOT);
        assert_eq!(frame.ack.as_deref(), Some("e1"), "{case}");
        assert_eq!(frame.kind, Kind::Nothing, "{case}");
    }
}

#[test]
fn control_frames_are_read() {
    assert_eq!(read(r#"{"type":"hello"}"#, BOT).kind, Kind::Hello);
    assert_eq!(
        read(r#"{"type":"disconnect","reason":"refresh_requested"}"#, BOT).kind,
        Kind::Disconnect
    );
    assert_eq!(
        read("not json", BOT),
        Frame {
            ack: None,
            kind: Kind::Nothing
        }
    );
}

#[test]
fn a_place_splits_into_channel_and_thread() {
    assert_eq!(target("C1/10.5"), ("C1", Some("10.5")));
    assert_eq!(target("D1"), ("D1", None));
}

#[test]
fn slack_markup_becomes_plain_text() {
    let frame = read(
        &event(
            r#"{"type":"message","channel_type":"im","channel":"D1","user":"U1","text":"see <https://a.io/x|the doc> and <https://b.io> in <#C9|general>, ask <@U2> <!here> &lt;b&gt;","ts":"1.0"}"#,
        ),
        BOT,
    );
    assert_eq!(
        message(&frame).text,
        "see the doc (https://a.io/x) and https://b.io in #general, ask @U2 @here <b>"
    );
}

#[test]
fn a_shared_file_with_text_and_a_thread_broadcast_are_heard() {
    for subtype in ["file_share", "thread_broadcast"] {
        let frame = read(
            &event(&format!(
                r#"{{"type":"message","subtype":"{subtype}","channel_type":"im","channel":"D1","user":"U1","text":"look","ts":"1.0"}}"#
            )),
            BOT,
        );
        assert_eq!(message(&frame).text, "look", "{subtype}");
    }
}

#[test]
fn a_reply_cannot_ping_anyone() {
    assert_eq!(
        escaped("<!channel> & <@U1>"),
        "&lt;!channel&gt; &amp; &lt;@U1&gt;"
    );
}

#[test]
fn a_long_reply_splits_at_line_breaks() {
    let line = "가".repeat(100);
    let text = vec![line.as_str(); 50].join("\n");
    let split = pieces(&text);
    assert_eq!(split.len(), 2);
    assert!(split.iter().all(|piece| piece.chars().count() <= PIECE));
    assert!(split.iter().all(|piece| !piece.starts_with('\n')));
    assert_eq!(split.join("\n"), text);
    assert_eq!(pieces("짧다"), vec!["짧다"]);
    let unbroken = "a".repeat(PIECE + 10);
    assert_eq!(pieces(&unbroken).len(), 2);
}
