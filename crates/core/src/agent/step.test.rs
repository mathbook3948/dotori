use std::collections::HashMap;
use std::num::NonZeroUsize;

use super::*;
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
        let incoming = self.incoming("main", "owner", text);
        self.feed(Event::Received {
            channel: app(),
            incoming,
        });
    }

    fn owner_says_on_slack(&mut self, place: &str, text: &str) {
        let mut incoming = self.incoming(place, "me", text);
        incoming.audience = Audience::People {
            some: vec![String::from("Mina")],
            total: 2,
        };
        self.feed(Event::Received {
            channel: slack(),
            incoming,
        });
    }

    fn delegated(&mut self) -> SubSessionId {
        self.owner_says("book a table");
        self.model_answers(vec![Action::Delegate(SubSessionSpec {
            title: String::from("booking"),
            brief: String::from("book a table for two"),
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
        self.feed(Event::Shutdown);
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

    fn stopped(&self) -> bool {
        self.effects
            .iter()
            .any(|effect| matches!(effect, Effect::Stop))
    }
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
fn owner_message_cancels_the_call_in_flight_and_calls_again() {
    let mut h = Harness::new();
    h.owner_says("find my flight");
    let first = h.call_in_flight();

    h.owner_says("the one on friday");

    assert_eq!(h.cancelled(), vec![first]);
    assert_eq!(
        heard(h.the_model_call()),
        vec!["find my flight", "the one on friday"]
    );
    assert_ne!(h.call_in_flight(), first);
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
    });
    h.feed(Event::Received {
        channel: slack(),
        incoming,
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
