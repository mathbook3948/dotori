use super::*;
use crate::conversation::MessageRef;
use crate::conversation::ThreadRef;
use crate::id::ChannelId;
use crate::id::ExternalMessageId;
use crate::id::ExternalUserId;
use crate::message::Address;
use crate::message::Content;
use crate::message::Message;
use crate::message::Place;
use crate::message::Role;
use crate::message::Sender;
use crate::tool::ToolCallId;

fn heard(text: &str) -> Entry {
    Entry::Heard {
        thread: ThreadRef::new(0),
        message: Message {
            id: ExternalMessageId::new(text),
            address: Address {
                channel: ChannelId::parse("app").expect("valid"),
                place: Place::new("main"),
            },
            sender: Sender {
                id: ExternalUserId::new("me"),
                name: String::from("Me"),
                role: Role::Owner,
            },
            content: Content::parse(text).expect("non-empty"),
            audience: None,
            at: None,
        },
    }
}

fn said(answered: usize) -> Entry {
    Entry::Acted {
        actions: vec![Action::Say {
            thread: ThreadRef::new(0),
            text: String::from("ok"),
            privately: false,
        }],
        answered,
        trace: None,
    }
}

fn kept(id: &str) -> MessageRef {
    MessageRef {
        channel: ChannelId::parse("app").expect("valid"),
        id: ExternalMessageId::new(id),
    }
}

fn summary(id: &str, text: &str) -> Entry {
    Entry::Summarized {
        kept: kept(id),
        text: text.to_owned(),
    }
}

fn talk(conversation: &mut Vec<Entry>, count: u32) {
    let words = "가".repeat(9_000);
    for n in 0..count {
        conversation.push(heard(&format!("{n} {words}")));
        conversation.push(said(conversation.len()));
    }
}

fn long(count: u32) -> Vec<Entry> {
    let mut conversation = Vec::new();
    talk(&mut conversation, count);
    conversation
}

fn id_of(entry: &Entry) -> &str {
    match entry {
        Entry::Heard { message, .. } => message.id.as_str(),
        _ => panic!("expected a heard message"),
    }
}

#[test]
fn a_short_conversation_is_carried_whole() {
    let conversation = long(10);
    let carried = carried(&conversation);

    assert_eq!(
        carried,
        Carried {
            summary: None,
            start: 0,
            left_out: false
        }
    );
}

#[test]
fn a_long_conversation_leaves_out_its_start_at_a_heard_message_within_the_limit() {
    let conversation = long(60);

    let carried = carried(&conversation);

    assert!(carried.left_out);
    assert!(matches!(conversation[carried.start], Entry::Heard { .. }));
    let kept: usize = conversation[carried.start..].iter().map(weight).sum();
    assert!(kept <= KEPT);
}

#[test]
fn the_start_moves_in_steps_so_most_requests_share_their_beginning() {
    let mut conversation = long(60);
    let first = carried(&conversation).start;

    conversation.push(heard("짧은 말"));
    conversation.push(said(conversation.len()));
    assert_eq!(carried(&conversation).start, first);

    talk(&mut conversation, 12);
    assert!(carried(&conversation).start > first);
}

#[test]
fn the_newest_message_is_carried_however_long_it_is() {
    let mut conversation = long(3);
    conversation.push(heard(&"나".repeat(KEPT + STEP)));

    assert_eq!(carried(&conversation).start, conversation.len() - 1);
}

#[test]
fn without_a_message_after_the_cut_the_last_message_starts_it() {
    let mut conversation = long(40);
    let last = conversation.len();
    conversation.push(heard("마지막 말"));
    conversation.push(Entry::ToolReturned {
        call: ToolCallId::new("t1"),
        result: Ok("다".repeat(KEPT + STEP)),
    });

    assert_eq!(carried(&conversation).start, last);
}

#[test]
fn a_summary_starts_what_is_carried_at_the_message_it_names() {
    let mut conversation = long(10);
    let anchor = id_of(&conversation[8]).to_owned();
    conversation.push(summary(&anchor, "earlier things"));

    let carried = carried(&conversation);

    assert_eq!(carried.summary, Some("earlier things"));
    assert_eq!(carried.start, 8);
    assert!(!carried.left_out);
}

#[test]
fn a_summary_whose_message_is_missing_leaves_the_whole_conversation_to_cut() {
    let mut conversation = long(60);
    conversation.push(summary("erased", "earlier things"));

    let carried = carried(&conversation);

    assert_eq!(carried.summary, Some("earlier things"));
    assert!(carried.start > 0);
    assert!(carried.left_out);
    assert_eq!(carried.start, super::start(&conversation));
}

#[test]
fn a_long_stretch_after_the_summarys_message_is_cut_from_that_message() {
    let mut conversation = long(2);
    let anchor = id_of(&conversation[0]).to_owned();
    conversation.push(summary(&anchor, "earlier things"));
    talk(&mut conversation, 60);

    let carried = carried(&conversation);

    assert_eq!(carried.summary, Some("earlier things"));
    assert!(carried.start > 0);
    assert!(carried.left_out);
    let kept: usize = conversation[carried.start..].iter().map(weight).sum();
    assert!(kept <= KEPT);
}

#[test]
fn only_the_latest_summary_counts() {
    let mut conversation = long(10);
    let first = id_of(&conversation[2]).to_owned();
    let second = id_of(&conversation[6]).to_owned();
    conversation.push(summary(&first, "old"));
    conversation.push(summary(&second, "new"));

    let carried = carried(&conversation);

    assert_eq!(carried.summary, Some("new"));
    assert_eq!(carried.start, 6);
}

#[test]
fn a_message_heard_again_after_the_summary_does_not_move_its_start() {
    let mut conversation = long(4);
    let anchor = id_of(&conversation[2]).to_owned();
    conversation.push(summary(&anchor, "earlier things"));
    conversation.push(conversation[2].clone());

    assert_eq!(carried(&conversation).start, 2);
}

#[test]
fn a_summary_weighs_nothing() {
    assert_eq!(weight(&summary("m", &"가".repeat(1_000))), 0);
}

fn kept_id(plan: &Plan) -> &str {
    plan.kept.id.as_str()
}

#[test]
fn a_conversation_under_the_threshold_needs_no_summary() {
    assert_eq!(plan(&long(20)), None);
}

#[test]
fn the_summary_names_the_first_message_that_leaves_at_most_the_tail_after_it() {
    let conversation = long(40);

    let plan = plan(&conversation).expect("a summary is due");

    assert_eq!(plan.covered.start, 0);
    assert!(matches!(
        conversation[plan.covered.end],
        Entry::Heard { .. }
    ));
    assert_eq!(kept_id(&plan), id_of(&conversation[plan.covered.end]));
    let after: usize = conversation[plan.covered.end..].iter().map(weight).sum();
    assert!(after <= TAIL);
    let one_earlier: usize = conversation[plan.covered.end - 2..]
        .iter()
        .map(weight)
        .sum();
    assert!(one_earlier > TAIL);
    assert_eq!(plan.weight, conversation.iter().map(weight).sum::<usize>());
    assert_eq!(plan.previous, None);
}

#[test]
fn what_one_summary_covers_is_capped_and_the_older_part_is_skipped() {
    let conversation = long(80);

    let plan = plan(&conversation).expect("a summary is due");

    assert!(plan.covered.start > 0);
    let covered: usize = conversation[plan.covered.clone()].iter().map(weight).sum();
    assert!(covered <= MOST);
    let one_more = covered + weight(&conversation[plan.covered.start - 1]);
    assert!(one_more > MOST);
}

#[test]
fn the_latest_summary_is_merged_and_only_what_follows_its_message_counts() {
    let mut conversation = long(2);
    let anchor = id_of(&conversation[0]).to_owned();
    conversation.push(summary(&anchor, "earlier things"));
    talk(&mut conversation, 36);

    let plan = plan(&conversation).expect("a summary is due");

    assert_eq!(plan.previous.as_deref(), Some("earlier things"));
    assert_eq!(plan.covered.start, 0);
    let after_anchor: usize = conversation[0..].iter().map(weight).sum();
    assert_eq!(plan.weight, after_anchor);
}

#[test]
fn the_weight_before_the_summarys_message_is_not_counted() {
    let mut conversation = long(40);
    let anchor = id_of(&conversation[60]).to_owned();
    conversation.push(summary(&anchor, "earlier things"));

    assert_eq!(plan(&conversation), None);
}

#[test]
fn a_summary_whose_message_is_gone_starts_the_count_at_the_summary() {
    let mut conversation = long(40);
    conversation.push(summary("erased", "earlier things"));
    let at = conversation.len() - 1;
    talk(&mut conversation, 10);
    assert_eq!(plan(&conversation), None);
    talk(&mut conversation, 30);

    let plan = plan(&conversation).expect("a summary is due");

    assert_eq!(plan.covered.start, at);
    assert_eq!(plan.previous.as_deref(), Some("earlier things"));
}

#[test]
fn without_a_message_leaving_little_after_it_the_last_message_is_named() {
    let mut conversation = long(30);
    let last = conversation.len();
    conversation.push(heard("마지막 말"));
    conversation.push(Entry::ToolReturned {
        call: ToolCallId::new("t1"),
        result: Ok("다".repeat(TAIL + STEP)),
    });

    let plan = plan(&conversation).expect("a summary is due");

    assert_eq!(plan.covered.end, last);
    assert_eq!(kept_id(&plan), "마지막 말");
}

#[test]
fn without_a_message_after_the_start_there_is_nothing_to_name() {
    let conversation = vec![
        heard("하나"),
        Entry::ToolReturned {
            call: ToolCallId::new("t1"),
            result: Ok("다".repeat(SUMMARIZE_AT)),
        },
    ];

    assert_eq!(plan(&conversation), None);
}
