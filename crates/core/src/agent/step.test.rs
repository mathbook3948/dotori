use std::collections::HashMap;
use std::mem;
use std::num::NonZeroUsize;
use std::time::Duration;
use std::time::SystemTime;

use super::*;
use crate::carried::Room;
use crate::id::ExternalMessageId;
use crate::id::ExternalUserId;
use crate::message::Audience;
use crate::message::Content;
use crate::message::Place;
use crate::runtime::Wiring;
use crate::sub_session::SubSessionSpec;
use crate::tool::ToolCall;

struct Harness {
    state: AgentState,
    effects: Vec<Effect>,
    sent: u32,
}

impl Harness {
    fn new() -> Self {
        let capacity = NonZeroUsize::new(16).expect("non-zero");
        Self {
            state: AgentState::new(
                Wiring {
                    app: app(),
                    private: HashMap::from([
                        (app(), Place::new("main")),
                        (slack(), Place::new("D-owner")),
                    ]),
                },
                capacity,
            ),
            effects: Vec::new(),
            sent: 0,
        }
    }

    fn owner_says(&mut self, text: &str) {
        self.owner_types(text);
        self.gather_fires();
    }

    fn owner_says_on_slack(&mut self, place: &str, text: &str) {
        self.owner_types_on_slack(place, text);
        self.gather_fires();
    }

    fn owner_types(&mut self, text: &str) {
        let incoming = self.incoming("main", "owner", text);
        self.feed(Event::Received {
            channel: app(),
            incoming,
            at: SystemTime::UNIX_EPOCH,
        });
    }

    fn owner_types_on_slack(&mut self, place: &str, text: &str) {
        let mut incoming = self.incoming(place, "me", text);
        incoming.audience = Audience::People {
            some: vec![String::from("Mina")],
            total: 2,
        };
        self.feed(Event::Received {
            channel: slack(),
            incoming,
            at: SystemTime::UNIX_EPOCH,
        });
    }

    fn gather_fires(&mut self) {
        let Phase::Gathering { gather } = self.state.phase else {
            return;
        };
        let before = mem::take(&mut self.effects);
        self.feed(Event::Gathered(gather));
        self.effects.splice(0..0, before);
    }

    fn pending_gather(&self) -> GatherId {
        match self.state.phase {
            Phase::Gathering { gather } => gather,
            _ => panic!("not gathering"),
        }
    }

    fn delegated(&mut self) -> SubSessionId {
        self.owner_says("book a table");
        self.model_answers(vec![Action::Delegate(SubSessionSpec {
            title: String::from("booking"),
            brief: String::from("book a table for two"),
            folder: None,
        })]);
        self.effects
            .iter()
            .find_map(|effect| match effect {
                Effect::SpawnSubSession { sub_session, .. } => Some(*sub_session),
                _ => None,
            })
            .expect("sub-session spawned")
    }

    fn sub_session_finishes(&mut self, sub_session: SubSessionId) {
        self.feed(Event::SubSessionDone {
            sub_session,
            outcome: SubSessionOutcome::Finished(String::from("booked")),
        });
    }

    fn model_answers(&mut self, actions: Vec<Action>) {
        let call = self.call_in_flight();
        self.answer(
            call,
            Ok(ModelReply {
                actions,
                trace: None,
            }),
        );
    }

    fn tool_returns(&mut self, result: Result<String, ToolError>) {
        let run = self.running_tool();
        self.feed(Event::ToolDone { run, result });
    }

    fn shut_down(&mut self) {
        self.feed(Event::Shutdown { handing_off: false });
    }

    fn answer(&mut self, call: CallId, result: Result<ModelReply, ModelError>) {
        self.feed(Event::ModelDone { call, result });
    }

    fn feed(&mut self, event: Event) {
        self.effects = step(&mut self.state, event);
    }

    fn incoming(&mut self, place: &str, sender: &str, text: &str) -> Incoming {
        self.sent += 1;
        Incoming {
            id: ExternalMessageId::new(format!("m{}", self.sent)),
            place: Place::new(place),
            sender_id: ExternalUserId::new(sender),
            sender_name: sender.to_owned(),
            content: Content::parse(text).expect("non-empty"),
            audience: Audience::Owner,
            at: None,
        }
    }

    fn call_in_flight(&self) -> CallId {
        match self.state.phase {
            Phase::Thinking { call, .. } => call,
            _ => panic!("no model call in flight"),
        }
    }

    fn running_tool(&self) -> ToolRunId {
        match &self.state.phase {
            Phase::UsingTools { waiting } | Phase::Stopping { waiting } => {
                *waiting.keys().next().expect("a running tool")
            }
            _ => panic!("no tool running"),
        }
    }

    fn thread_of(&self, text: &str) -> ThreadRef {
        self.state
            .conversation
            .iter()
            .find_map(|entry| match entry {
                Entry::Heard { thread, message } if message.content.as_str() == text => {
                    Some(*thread)
                }
                _ => None,
            })
            .expect("message was heard")
    }

    fn model_calls(&self) -> Vec<&Vec<Entry>> {
        self.effects
            .iter()
            .filter_map(|effect| match effect {
                Effect::CallModel { conversation, .. } => Some(conversation),
                _ => None,
            })
            .collect()
    }

    fn the_model_call(&self) -> &Vec<Entry> {
        let calls = self.model_calls();
        assert_eq!(calls.len(), 1, "exactly one model call");
        calls[0]
    }

    fn cancelled(&self) -> Vec<CallId> {
        self.effects
            .iter()
            .filter_map(|effect| match effect {
                Effect::CancelModel(call) => Some(*call),
                _ => None,
            })
            .collect()
    }

    fn deliveries(&self) -> Vec<(&Address, &str)> {
        self.effects
            .iter()
            .filter_map(|effect| match effect {
                Effect::Deliver { to, text, .. } => Some((to, text.as_str())),
                _ => None,
            })
            .collect()
    }

    fn shown(&self) -> Vec<&Traffic> {
        self.effects
            .iter()
            .filter_map(|effect| match effect {
                Effect::Show(traffic) => Some(traffic),
                _ => None,
            })
            .collect()
    }

    fn long_talk(&mut self, count: usize) {
        for n in 0..count {
            self.owner_says(&long_text(n));
            self.model_answers(Vec::new());
        }
    }

    fn summaries(&self) -> Vec<(SummaryId, &SummaryRequest)> {
        self.effects
            .iter()
            .filter_map(|effect| match effect {
                Effect::Summarize { summary, request } => Some((*summary, request)),
                _ => None,
            })
            .collect()
    }

    fn the_summary(&self) -> (SummaryId, &SummaryRequest) {
        let summaries = self.summaries();
        assert_eq!(summaries.len(), 1, "exactly one summary");
        summaries[0]
    }

    fn summary_done(&mut self, summary: SummaryId, text: &str) {
        self.feed(Event::SummaryDone {
            summary,
            result: Ok(text.to_owned()),
        });
    }

    fn summarized(&self) -> Vec<(&str, &str)> {
        self.state
            .conversation
            .iter()
            .filter_map(|entry| match entry {
                Entry::Summarized { kept, text } => Some((kept.id.as_str(), text.as_str())),
                _ => None,
            })
            .collect()
    }

    fn stopped(&self) -> bool {
        self.effects
            .iter()
            .any(|effect| matches!(effect, Effect::Stop))
    }
}

fn long_text(n: usize) -> String {
    format!("{n}{}", "가".repeat(90_000))
}

fn app() -> ChannelId {
    ChannelId::parse("app").expect("valid id")
}

fn slack() -> ChannelId {
    ChannelId::parse("slack").expect("valid id")
}

fn heard(conversation: &[Entry]) -> Vec<&str> {
    conversation
        .iter()
        .filter_map(|entry| match entry {
            Entry::Heard { message, .. } => Some(message.content.as_str()),
            _ => None,
        })
        .collect()
}

fn say(thread: ThreadRef, text: &str) -> Action {
    Action::Say {
        thread,
        text: text.to_owned(),
        privately: false,
    }
}

fn use_tool(id: &str) -> Action {
    Action::UseTool(ToolCall {
        id: ToolCallId::new(id),
        name: String::from("read_file"),
        input: String::from("{}"),
    })
}

fn tool_results(conversation: &[Entry]) -> Vec<&str> {
    conversation
        .iter()
        .filter_map(|entry| match entry {
            Entry::ToolReturned { call, .. } => Some(call.as_str()),
            _ => None,
        })
        .collect()
}

#[test]
fn owner_message_does_not_call_the_model_until_the_wait_ends() {
    let mut h = Harness::new();

    h.owner_types("find my flight");

    assert!(h.model_calls().is_empty());

    h.gather_fires();

    assert_eq!(heard(h.the_model_call()), vec!["find my flight"]);
}

#[test]
fn messages_in_quick_succession_make_one_call_that_heard_both() {
    let mut h = Harness::new();
    h.owner_types("find my flight");
    let first_wait = h.pending_gather();
    h.owner_types("the one on friday");

    h.feed(Event::Gathered(first_wait));

    assert!(h.effects.is_empty());
    assert!(matches!(h.state.phase, Phase::Gathering { .. }));

    h.gather_fires();

    assert_eq!(
        heard(h.the_model_call()),
        vec!["find my flight", "the one on friday"]
    );
}

#[test]
fn owner_message_cancels_the_call_in_flight_and_waits_before_calling_again() {
    let mut h = Harness::new();
    h.owner_says("find my flight");
    let first = h.call_in_flight();

    h.owner_types("the one on friday");

    assert_eq!(h.cancelled(), vec![first]);
    assert!(h.model_calls().is_empty());

    h.gather_fires();

    assert_eq!(
        heard(h.the_model_call()),
        vec!["find my flight", "the one on friday"]
    );
    assert_ne!(h.call_in_flight(), first);
}

#[test]
fn a_sub_session_result_while_gathering_waits_for_the_call_after_the_wait() {
    let mut h = Harness::new();
    let sub_session = h.delegated();
    h.owner_types("any news?");

    h.sub_session_finishes(sub_session);

    assert!(h.effects.is_empty());

    h.gather_fires();

    let conversation = h.the_model_call();
    assert_eq!(heard(conversation), vec!["book a table", "any news?"]);
    assert!(
        conversation
            .iter()
            .any(|entry| matches!(entry, Entry::SubSessionReturned { .. }))
    );
}

#[test]
fn shutdown_while_gathering_stops() {
    let mut h = Harness::new();
    h.owner_types("hello");
    let wait = h.pending_gather();

    h.shut_down();

    assert!(h.stopped());
    assert!(h.model_calls().is_empty());

    h.feed(Event::Gathered(wait));

    assert!(h.effects.is_empty());
}

#[test]
fn the_agent_is_working_while_it_gathers() {
    let mut h = Harness::new();
    assert!(!h.state.working());

    h.owner_types("hello");

    assert!(h.state.working());
}

#[test]
fn the_owner_on_slack_interrupts_like_in_the_app() {
    let mut h = Harness::new();
    h.owner_says("hello");
    let first = h.call_in_flight();

    h.owner_says_on_slack("general", "actually, never mind");

    assert_eq!(h.cancelled(), vec![first]);
    let conversation = h.the_model_call();
    assert_eq!(heard(conversation), vec!["hello", "actually, never mind"]);
    assert!(conversation.iter().all(|entry| match entry {
        Entry::Heard { message, .. } => message.sender.role == Role::Owner,
        _ => true,
    }));
}

#[test]
fn owner_message_waits_for_running_tools() {
    let mut h = Harness::new();
    h.owner_says("read the notes");
    h.model_answers(vec![use_tool("t1")]);

    h.owner_says("also the todo list");

    assert!(h.cancelled().is_empty());
    assert!(h.model_calls().is_empty());

    h.tool_returns(Ok(String::from("notes")));

    let conversation = h.the_model_call();
    assert_eq!(tool_results(conversation), vec!["t1"]);
    assert_eq!(
        heard(conversation),
        vec!["read the notes", "also the todo list"]
    );
}

#[test]
fn result_of_a_cancelled_call_is_ignored() {
    let mut h = Harness::new();
    h.owner_says("first");
    let cancelled = h.call_in_flight();
    h.owner_says("second");
    let thread = h.thread_of("first");

    h.answer(
        cancelled,
        Ok(ModelReply {
            actions: vec![say(thread, "stale")],
            trace: None,
        }),
    );

    assert!(h.effects.is_empty());
    assert!(
        !h.state
            .conversation
            .iter()
            .any(|entry| matches!(entry, Entry::Acted { .. }))
    );
}

#[test]
fn redelivered_message_is_heard_once() {
    let mut h = Harness::new();
    let incoming = h.incoming("general", "me", "ping");
    h.feed(Event::Received {
        channel: slack(),
        incoming: incoming.clone(),
        at: SystemTime::UNIX_EPOCH,
    });
    h.feed(Event::Received {
        channel: slack(),
        incoming,
        at: SystemTime::UNIX_EPOCH,
    });

    assert!(h.effects.is_empty());
    assert_eq!(heard(&h.state.conversation), vec!["ping"]);
}

#[test]
fn model_failure_is_recorded_and_ends_the_turn() {
    let mut h = Harness::new();
    h.owner_says("hello");
    let call = h.call_in_flight();

    h.answer(
        call,
        Err(ModelError {
            detail: String::from("overloaded"),
        }),
    );

    assert!(h.effects.is_empty());
    assert!(matches!(
        h.state.conversation.last(),
        Some(Entry::ModelFailed { detail }) if detail == "overloaded"
    ));

    h.owner_says("try again");

    assert!(h.cancelled().is_empty());
    assert_eq!(h.model_calls().len(), 1);
}

#[test]
fn input_that_joined_a_failed_call_is_answered() {
    let mut h = Harness::new();
    let sub_session = h.delegated();
    h.owner_says("hello");
    let call = h.call_in_flight();
    h.sub_session_finishes(sub_session);

    h.answer(
        call,
        Err(ModelError {
            detail: String::from("overloaded"),
        }),
    );

    let conversation = h.the_model_call();
    assert_eq!(heard(conversation), vec!["book a table", "hello"]);
    assert!(
        conversation
            .iter()
            .any(|entry| matches!(entry, Entry::SubSessionReturned { .. }))
    );
}

#[test]
fn delivery_failure_is_recorded_without_calling_the_model() {
    let mut h = Harness::new();
    h.owner_says("hello");
    let thread = h.thread_of("hello");
    h.model_answers(vec![say(thread, "hi")]);

    h.feed(Event::DeliveryDone {
        thread,
        place: Place::new("main"),
        text: String::from("hi"),
        result: Err(ChannelError {
            detail: String::from("window closed"),
        }),
    });

    assert!(h.effects.is_empty());
    assert!(matches!(
        h.state.conversation.last(),
        Some(Entry::DeliveryFailed { text, .. }) if text == "hi"
    ));
}

#[test]
fn shutdown_waits_for_running_tools() {
    let mut h = Harness::new();
    h.owner_says("read the notes");
    h.model_answers(vec![use_tool("t1")]);

    h.shut_down();
    assert!(!h.stopped());

    h.owner_says("are you there");
    assert!(h.effects.is_empty());

    h.tool_returns(Ok(String::from("notes")));

    assert!(h.stopped());
    assert!(h.model_calls().is_empty());
    assert_eq!(tool_results(&h.state.conversation), vec!["t1"]);
    assert_eq!(heard(&h.state.conversation), vec!["read the notes"]);
}

#[test]
fn shutdown_cancels_the_call_in_flight() {
    let mut h = Harness::new();
    h.owner_says("hello");
    let call = h.call_in_flight();

    h.shut_down();

    assert_eq!(h.cancelled(), vec![call]);
    assert!(h.stopped());
}

#[test]
fn app_window_shows_traffic_of_other_channels() {
    let mut h = Harness::new();
    h.owner_says("hello");
    assert!(h.shown().is_empty());

    h.model_answers(Vec::new());
    h.owner_says_on_slack("general", "lunch at noon?");

    let shown = h.shown();
    assert_eq!(shown.len(), 1);
    assert_eq!(shown[0].address.channel, slack());
    assert_eq!(shown[0].address.place, Place::new("general"));
    assert_eq!(shown[0].who, "me");
    assert_eq!(shown[0].text, "lunch at noon?");

    let general = h.thread_of("lunch at noon?");
    h.model_answers(vec![say(general, "sure")]);
    h.feed(Event::DeliveryDone {
        thread: general,
        place: Place::new("general"),
        text: String::from("sure"),
        result: Ok(()),
    });

    let shown = h.shown();
    assert_eq!(shown.len(), 1);
    assert_eq!(shown[0].address.channel, slack());
    assert_eq!(shown[0].text, "sure");

    let owner = h.thread_of("hello");
    h.feed(Event::DeliveryDone {
        thread: owner,
        place: Place::new("main"),
        text: String::from("hi"),
        result: Ok(()),
    });
    assert!(h.shown().is_empty());
}

#[test]
fn reply_to_an_unknown_thread_is_refused() {
    let mut h = Harness::new();
    h.owner_says("hello");

    h.model_answers(vec![say(ThreadRef::new(99), "hi")]);

    assert!(h.deliveries().is_empty());
    assert!(matches!(
        h.state.conversation.last(),
        Some(Entry::Refused(Action::Say { text, .. })) if text == "hi"
    ));
}

#[test]
fn sub_session_outcome_joins_the_next_call() {
    let mut h = Harness::new();
    h.owner_says("book a table");
    h.model_answers(vec![Action::Delegate(SubSessionSpec {
        title: String::from("booking"),
        brief: String::from("book a table for two"),
        folder: None,
    })]);
    let sub_session = h
        .effects
        .iter()
        .find_map(|effect| match effect {
            Effect::SpawnSubSession { sub_session, .. } => Some(*sub_session),
            _ => None,
        })
        .expect("sub-session spawned");
    assert!(h.model_calls().is_empty());

    h.owner_says("and remind me at six");
    h.feed(Event::SubSessionDone {
        sub_session,
        outcome: SubSessionOutcome::Finished(String::from("booked")),
    });

    assert!(h.cancelled().is_empty());
    assert!(h.model_calls().is_empty());

    h.model_answers(Vec::new());

    let conversation = h.the_model_call();
    assert!(conversation.iter().any(|entry| matches!(
        entry,
        Entry::SubSessionReturned { outcome: SubSessionOutcome::Finished(text), .. } if text == "booked"
    )));
}

#[test]
fn reply_records_how_much_of_the_conversation_it_answered() {
    let mut h = Harness::new();
    let sub_session = h.delegated();
    h.owner_says("hello");
    h.sub_session_finishes(sub_session);

    h.model_answers(Vec::new());

    let answered: Vec<usize> = h
        .state
        .conversation
        .iter()
        .filter_map(|entry| match entry {
            Entry::Acted { answered, .. } => Some(*answered),
            _ => None,
        })
        .collect();
    assert_eq!(answered, vec![1, 3]);
}

#[test]
fn a_heard_message_keeps_who_could_read_its_room() {
    let mut h = Harness::new();
    h.owner_says_on_slack("general", "what is on my calendar");

    let audience = h.state.conversation.iter().find_map(|entry| match entry {
        Entry::Heard { message, .. } => message.audience.clone(),
        _ => None,
    });
    assert_eq!(
        audience,
        Some(Audience::People {
            some: vec![String::from("Mina")],
            total: 2,
        })
    );
}

#[test]
fn a_private_reply_goes_to_the_owners_own_room_on_that_channel() {
    let mut h = Harness::new();
    h.owner_says_on_slack("general", "what is on my calendar");
    let general = h.thread_of("what is on my calendar");

    h.model_answers(vec![
        say(general, "I sent it to you directly"),
        Action::Say {
            thread: general,
            text: String::from("dentist at 3"),
            privately: true,
        },
    ]);

    let deliveries = h.deliveries();
    assert_eq!(deliveries.len(), 2);
    assert_eq!(deliveries[0].0.place, Place::new("general"));
    assert_eq!(deliveries[1].0.channel, slack());
    assert_eq!(deliveries[1].0.place, Place::new("D-owner"));
    assert_eq!(deliveries[1].1, "dentist at 3");
}

#[test]
fn a_private_reply_shows_on_the_app_where_it_went() {
    let mut h = Harness::new();
    h.owner_says_on_slack("general", "what is on my calendar");
    let general = h.thread_of("what is on my calendar");
    h.model_answers(vec![Action::Say {
        thread: general,
        text: String::from("dentist at 3"),
        privately: true,
    }]);

    h.feed(Event::DeliveryDone {
        thread: general,
        place: Place::new("D-owner"),
        text: String::from("dentist at 3"),
        result: Ok(()),
    });

    let shown = h.shown();
    assert_eq!(shown.len(), 1);
    assert_eq!(shown[0].address.channel, slack());
    assert_eq!(shown[0].address.place, Place::new("D-owner"));
}

#[test]
fn a_tool_call_carries_the_room_the_owner_last_spoke_in() {
    let mut h = Harness::new();
    h.owner_says("hello");
    h.owner_says_on_slack("C1/10.5", "send it");
    h.model_answers(vec![use_tool("t1")]);

    let origins: Vec<&Option<Origin>> = h
        .effects
        .iter()
        .filter_map(|effect| match effect {
            Effect::RunTool { origin, .. } => Some(origin),
            _ => None,
        })
        .collect();

    assert_eq!(
        origins,
        [&Some(Origin {
            address: Address {
                channel: slack(),
                place: Place::new("C1/10.5"),
            },
            owner: String::from("me"),
            audience: Some(Audience::People {
                some: vec![String::from("Mina")],
                total: 2,
            }),
        })]
    );
}

#[test]
fn a_tool_call_after_a_sub_session_returned_has_no_room() {
    let mut h = Harness::new();
    h.owner_says_on_slack("C1/10.5", "book a table");
    h.model_answers(vec![Action::Delegate(SubSessionSpec {
        title: String::from("booking"),
        brief: String::from("book a table for two"),
        folder: None,
    })]);
    let sub_session = h
        .effects
        .iter()
        .find_map(|effect| match effect {
            Effect::SpawnSubSession { sub_session, .. } => Some(*sub_session),
            _ => None,
        })
        .expect("sub-session spawned");
    h.sub_session_finishes(sub_session);
    h.model_answers(vec![use_tool("t1")]);

    assert!(
        h.effects
            .iter()
            .any(|effect| matches!(effect, Effect::RunTool { origin: None, .. }))
    );
}

#[test]
fn a_heard_message_keeps_the_time_it_came_in() {
    let mut h = Harness::new();

    h.owner_says("hi");

    let at = h.state.conversation.iter().find_map(|entry| match entry {
        Entry::Heard { message, .. } => message.at,
        _ => None,
    });
    assert_eq!(at, Some(SystemTime::UNIX_EPOCH));
}

fn heard_at(h: &Harness, text: &str, at: Option<SystemTime>) -> Entry {
    let mut entry = h
        .state
        .conversation
        .iter()
        .find(|entry| matches!(entry, Entry::Heard { message, .. } if message.content.as_str() == text))
        .expect("message was heard")
        .clone();
    if let Entry::Heard { message, .. } = &mut entry {
        message.at = at;
    }
    entry
}

#[test]
fn a_forgotten_message_is_not_in_what_the_next_model_call_sees() {
    let mut h = Harness::new();
    h.owner_says("my card number is 1234");
    h.model_answers(Vec::new());
    let erased = heard_at(&h, "my card number is 1234", Some(SystemTime::UNIX_EPOCH));

    h.feed(Event::Forgotten {
        entries: vec![erased],
    });
    assert!(h.effects.is_empty(), "forgetting does nothing else");
    h.owner_says("what is my name?");

    assert_eq!(heard(h.the_model_call()), vec!["what is my name?"]);
}

#[test]
fn a_heard_message_is_forgotten_by_its_channel_and_id_whatever_time_it_carries() {
    let mut h = Harness::new();
    h.owner_says("hi");
    let later = SystemTime::UNIX_EPOCH + Duration::from_secs(60);
    let erased = heard_at(&h, "hi", Some(later));

    h.feed(Event::Forgotten {
        entries: vec![erased],
    });

    assert!(h.state.conversation.is_empty());
}

#[test]
fn a_reply_still_follows_the_message_it_answered_after_an_earlier_one_is_forgotten() {
    let mut h = Harness::new();
    h.owner_says("my card number is 1234");
    h.model_answers(Vec::new());
    h.owner_says("book a table");
    let thread = h.thread_of("book a table");
    h.model_answers(vec![say(thread, "booked")]);
    let erased = heard_at(&h, "my card number is 1234", None);

    h.feed(Event::Forgotten {
        entries: vec![erased],
    });

    let answered: Vec<usize> = h
        .state
        .conversation
        .iter()
        .filter_map(|entry| match entry {
            Entry::Acted { answered, .. } => Some(*answered),
            _ => None,
        })
        .collect();
    assert_eq!(answered, vec![0, 2]);
}

#[test]
fn a_reply_is_forgotten_as_the_record_keeps_it_after_an_earlier_forget_moved_it() {
    let mut h = Harness::new();
    h.owner_says("my card number is 1234");
    h.model_answers(Vec::new());
    h.owner_says("book a table");
    let thread = h.thread_of("book a table");
    h.model_answers(vec![say(thread, "booked")]);
    let recorded = h
        .state
        .conversation
        .iter()
        .rfind(|entry| matches!(entry, Entry::Acted { .. }))
        .expect("the reply was recorded")
        .clone();
    let erased = heard_at(&h, "my card number is 1234", None);
    h.feed(Event::Forgotten {
        entries: vec![erased],
    });

    h.feed(Event::Forgotten {
        entries: vec![recorded],
    });

    assert!(
        !h.state
            .conversation
            .iter()
            .any(|entry| matches!(entry, Entry::Acted { actions, .. } if !actions.is_empty()))
    );
}

#[test]
fn a_model_call_in_flight_still_answers_the_right_entries_after_a_forget() {
    let mut h = Harness::new();
    h.owner_says("my card number is 1234");
    h.model_answers(Vec::new());
    h.owner_says("book a table");
    let thread = h.thread_of("book a table");
    let erased = heard_at(&h, "my card number is 1234", None);

    h.feed(Event::Forgotten {
        entries: vec![erased],
    });
    h.model_answers(vec![say(thread, "booked")]);

    assert_eq!(h.deliveries().len(), 1);
    assert!(matches!(
        h.state.conversation.last(),
        Some(Entry::Acted { answered: 2, .. })
    ));
}

#[test]
fn forgetting_an_entry_that_is_not_in_the_conversation_changes_nothing() {
    let mut h = Harness::new();
    h.owner_says("hi");
    let before = h.state.conversation.clone();
    let stranger = Entry::ModelFailed {
        detail: String::from("never happened"),
    };

    h.feed(Event::Forgotten {
        entries: vec![stranger],
    });

    assert_eq!(h.state.conversation, before);
}

#[test]
fn a_long_conversation_starts_a_summary_when_a_reply_finishes() {
    let mut h = Harness::new();
    h.long_talk(2);
    assert!(h.summaries().is_empty());
    h.owner_says(&long_text(2));
    assert!(h.summaries().is_empty());

    h.model_answers(Vec::new());

    let (_, request) = h.the_summary();
    assert_eq!(request.previous, None);
    assert_eq!(request.conversation.len(), 4);
    assert_eq!(request.time_zone, None);
    match &h.state.summarizing {
        Summarizing::Running { kept, .. } => assert_eq!(kept.id.as_str(), "m3"),
        _ => panic!("a summary is running"),
    }
    assert!(!h.state.working());
}

#[test]
fn a_failed_reply_that_ends_the_turn_starts_a_summary_too() {
    let mut h = Harness::new();
    h.long_talk(2);
    h.owner_says(&long_text(2));
    let call = h.call_in_flight();

    h.answer(
        call,
        Err(ModelError {
            detail: String::from("overloaded"),
        }),
    );

    h.the_summary();
}

#[test]
fn no_summary_starts_while_the_model_is_thinking() {
    let mut h = Harness::new();
    h.long_talk(2);

    h.owner_says(&long_text(2));

    assert!(h.summaries().is_empty());
    assert!(matches!(h.state.summarizing, Summarizing::Idle));
}

#[test]
fn no_summary_starts_while_tools_run_but_one_starts_when_the_turn_ends() {
    let mut h = Harness::new();
    h.long_talk(2);
    h.owner_says(&long_text(2));

    h.model_answers(vec![use_tool("t1")]);
    assert!(h.summaries().is_empty());
    h.tool_returns(Ok(String::from("done")));
    assert!(h.summaries().is_empty());
    h.model_answers(Vec::new());

    h.the_summary();
}

#[test]
fn the_result_is_recorded_and_the_next_call_carries_it() {
    let mut h = Harness::new();
    h.long_talk(3);
    let (summary, _) = h.the_summary();

    h.summary_done(summary, "earlier things");

    assert!(h.effects.is_empty());
    assert_eq!(h.summarized(), vec![("m3", "earlier things")]);
    assert!(matches!(h.state.summarizing, Summarizing::Idle));
    h.owner_says("hello");
    let call = h.the_model_call();
    assert!(
        call.iter()
            .any(|entry| matches!(entry, Entry::Summarized { .. }))
    );
}

#[test]
fn a_summary_done_twice_or_with_an_unknown_id_changes_nothing() {
    let mut h = Harness::new();
    h.long_talk(3);
    let (summary, _) = h.the_summary();
    let unknown = h.state.ids.next_summary();

    h.summary_done(unknown, "from nowhere");
    assert!(h.summarized().is_empty());
    assert!(matches!(h.state.summarizing, Summarizing::Running { .. }));
    h.summary_done(summary, "earlier things");
    h.summary_done(summary, "again");

    assert_eq!(h.summarized(), vec![("m3", "earlier things")]);
}

#[test]
fn an_owner_message_during_a_summary_is_answered_as_usual() {
    let mut h = Harness::new();
    h.long_talk(3);
    let (summary, _) = h.the_summary();

    h.owner_says("what time is it?");

    assert!(h.cancelled().is_empty());
    h.the_model_call();
    let thread = h.thread_of("what time is it?");
    h.summary_done(summary, "earlier things");
    h.model_answers(vec![say(thread, "noon")]);
    assert_eq!(h.deliveries().len(), 1);
    assert_eq!(h.summarized(), vec![("m3", "earlier things")]);
    assert!(matches!(
        h.state.conversation.last(),
        Some(Entry::Acted { .. })
    ));
}

#[test]
fn a_smaller_room_starts_a_summary_sooner() {
    let mut h = Harness::new();
    h.state.room = Room::for_window(80_000, 10_000);
    h.owner_says(&"가".repeat(20_000));
    h.model_answers(Vec::new());
    assert!(h.summaries().is_empty());

    h.owner_says(&"나".repeat(20_000));
    h.model_answers(Vec::new());

    let (_, request) = h.the_summary();
    assert_eq!(heard(&request.conversation), vec!["가".repeat(20_000)]);
}

#[test]
fn a_failed_summary_records_nothing_and_waits_for_a_step_more_before_trying_again() {
    let mut h = Harness::new();
    h.long_talk(3);
    let (summary, _) = h.the_summary();

    h.feed(Event::SummaryDone {
        summary,
        result: Err(ModelError {
            detail: String::from("overloaded"),
        }),
    });
    assert!(h.summarized().is_empty());
    assert!(matches!(h.state.summarizing, Summarizing::Failed { .. }));
    h.owner_says(&"나".repeat(Room::DEFAULT.step() / 2));
    h.model_answers(Vec::new());
    assert!(h.summaries().is_empty());
    h.owner_says(&"다".repeat(Room::DEFAULT.step() / 2));
    h.model_answers(Vec::new());

    h.the_summary();
}

#[test]
fn erasing_during_a_summary_drops_its_result() {
    let mut h = Harness::new();
    h.long_talk(3);
    let (summary, _) = h.the_summary();
    let erased = heard_at(&h, &long_text(0), None);

    h.feed(Event::Forgotten {
        entries: vec![erased],
    });
    assert!(matches!(h.state.summarizing, Summarizing::Idle));
    h.summary_done(summary, "earlier things");

    assert!(h.summarized().is_empty());
}

#[test]
fn erasing_removes_the_summaries_after_the_erased_record() {
    let mut h = Harness::new();
    h.long_talk(3);
    let (summary, _) = h.the_summary();
    h.summary_done(summary, "earlier things");
    h.owner_says("hello");
    h.model_answers(Vec::new());
    let erased = heard_at(&h, &long_text(2), None);

    h.feed(Event::Forgotten {
        entries: vec![erased],
    });

    assert!(h.summarized().is_empty());
}

#[test]
fn erasing_after_a_summary_keeps_it() {
    let mut h = Harness::new();
    h.long_talk(3);
    let (summary, _) = h.the_summary();
    h.summary_done(summary, "earlier things");
    h.owner_says("hello");
    h.model_answers(Vec::new());
    let erased = heard_at(&h, "hello", None);

    h.feed(Event::Forgotten {
        entries: vec![erased],
    });

    assert_eq!(h.summarized(), vec![("m3", "earlier things")]);
}

#[test]
fn no_summary_starts_while_stopping() {
    let mut h = Harness::new();
    h.long_talk(2);
    h.owner_says(&long_text(2));
    let call = h.call_in_flight();

    h.shut_down();
    assert!(h.summaries().is_empty());
    h.answer(
        call,
        Ok(ModelReply {
            actions: Vec::new(),
            trace: None,
        }),
    );

    assert!(h.summaries().is_empty());
    assert!(matches!(h.state.summarizing, Summarizing::Idle));
}
