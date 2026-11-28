use std::time::Duration;

use dotori_core::conversation::Action;
use dotori_core::conversation::Entry;
use dotori_core::conversation::ThreadRef;
use dotori_core::id::ChannelId;
use dotori_core::id::ExternalMessageId;
use dotori_core::id::ExternalUserId;
use dotori_core::message::Address;
use dotori_core::message::Content;
use dotori_core::message::Message;
use dotori_core::message::Place;
use dotori_core::message::Role;
use dotori_core::message::Sender;
use dotori_core::tool::ToolCall;
use dotori_core::tool::ToolCallId;
use dotori_core::tool::ToolError;
use dotori_core::tool::ToolSpec;

use super::*;
use crate::llm::mock;

const TOOL_REPLY: &str = r#"{"id":"chatcmpl-1","object":"chat.completion","choices":[{"index":0,"message":{"role":"assistant","content":null,"refusal":null,"tool_calls":[{"id":"call_1","type":"function","function":{"name":"say","arguments":"{\"thread\":0,\"text\":\"hello\"}"}},{"id":"call_2","type":"function","function":{"name":"read_file","arguments":"{\"path\":\"a.txt\"}"}}]},"finish_reason":"tool_calls"}]}"#;

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

fn request(conversation: Vec<Entry>) -> ModelRequest {
    ModelRequest {
        conversation,
        tools: vec![ToolSpec {
            name: String::from("read_file"),
            description: String::from("Reads a file."),
            input_schema: String::from(r#"{"type":"object"}"#),
        }],
        context: None,
        time_zone: None,
    }
}

fn config() -> OpenAiConfig {
    OpenAiConfig::new("http://localhost:11434/v1/", "qwen")
}

fn sent(config: &OpenAiConfig, request: &ModelRequest) -> Value {
    serde_json::from_str(&body(config, request).expect("body")).expect("json")
}

#[test]
fn the_request_starts_with_the_system_prompt_and_lists_function_tools() {
    let body = sent(&config(), &request(vec![heard("hi")]));

    assert_eq!(body["model"], "qwen");
    assert!(body.get("max_tokens").is_none());
    assert_eq!(body["messages"][0]["role"], "system");
    assert_eq!(body["messages"][0]["content"], DEFAULT_SYSTEM);
    assert_eq!(body["messages"][1]["role"], "user");
    assert_eq!(body["tools"][0]["type"], "function");
    assert_eq!(body["tools"][0]["function"]["name"], "say");
    assert_eq!(body["tools"][2]["function"]["parameters"]["type"], "object");
}

#[test]
fn a_reply_is_replayed_with_its_tool_calls_and_answered_by_tool_messages() {
    let reply = parse(TOOL_REPLY).expect("parsed");
    match reply.actions.as_slice() {
        [Action::Say { text, .. }, Action::UseTool(call)] => {
            assert_eq!(text, "hello");
            assert_eq!(call.id, ToolCallId::new("call_2"));
            assert_eq!(call.input, r#"{"path":"a.txt"}"#);
        }
        other => panic!("unexpected actions {other:?}"),
    }
    let conversation = vec![
        heard("hi"),
        Entry::Acted {
            actions: reply.actions,
            answered: 1,
            trace: reply.trace,
        },
        Entry::ToolReturned {
            call: ToolCallId::new("call_2"),
            result: Ok(String::from("contents")),
        },
        heard("thanks"),
    ];

    let body = sent(&config(), &request(conversation));

    let messages = body["messages"].as_array().expect("messages");
    assert_eq!(messages.len(), 6);
    let original: Value = serde_json::from_str(TOOL_REPLY).expect("json");
    assert_eq!(messages[2]["role"], "assistant");
    assert_eq!(
        messages[2]["tool_calls"],
        original["choices"][0]["message"]["tool_calls"]
    );
    assert!(messages[2].get("refusal").is_none());
    assert_eq!(messages[3]["role"], "tool");
    assert_eq!(messages[3]["tool_call_id"], "call_1");
    assert_eq!(messages[4]["tool_call_id"], "call_2");
    assert_eq!(messages[4]["content"], "contents");
    assert_eq!(messages[5]["role"], "user");
}

#[test]
fn a_reply_from_another_provider_is_written_as_tool_calls() {
    let conversation = vec![
        heard("hi"),
        Entry::Acted {
            actions: vec![Action::Say {
                thread: ThreadRef::new(0),
                text: String::from("hello"),
                privately: false,
            }],
            answered: 1,
            trace: None,
        },
    ];

    let body = sent(&config(), &request(conversation));

    let call = &body["messages"][2]["tool_calls"][0];
    assert_eq!(call["id"], "call_1_0");
    assert_eq!(call["function"]["name"], "say");
    let arguments: Value =
        serde_json::from_str(call["function"]["arguments"].as_str().expect("text")).expect("json");
    assert_eq!(arguments["text"], "hello");
    assert_eq!(body["messages"][3]["role"], "tool");
}

#[test]
fn a_cut_off_or_filtered_reply_is_a_failure() {
    let cut = r#"{"choices":[{"message":{"role":"assistant","content":"and"},"finish_reason":"length"}]}"#;
    let filtered = r#"{"choices":[{"message":{"role":"assistant","content":null},"finish_reason":"content_filter"}]}"#;

    assert!(parse(cut).expect_err("cut").detail.contains("token limit"));
    assert!(
        parse(filtered)
            .expect_err("filtered")
            .detail
            .contains("declined")
    );
    assert!(
        parse(r#"{"choices":[]}"#)
            .expect_err("empty")
            .detail
            .contains("no choices")
    );
}

#[test]
fn a_failed_tool_says_so_in_its_message() {
    let conversation = vec![
        heard("hi"),
        Entry::Acted {
            actions: vec![Action::UseTool(ToolCall {
                id: ToolCallId::new("c"),
                name: String::from("read_file"),
                input: String::from("{}"),
            })],
            answered: 1,
            trace: None,
        },
        Entry::ToolReturned {
            call: ToolCallId::new("c"),
            result: Err(ToolError {
                detail: String::from("no such file"),
            }),
        },
    ];

    let body = sent(&config(), &request(conversation));

    assert_eq!(body["messages"][3]["content"], "Error: no such file");
}

#[tokio::test]
async fn complete_posts_to_chat_completions_with_the_bearer_key() {
    let (base_url, mut requests) = mock::serve(vec![(200, TOOL_REPLY)]).await;
    let mut config = OpenAiConfig::new(format!("{base_url}/v1"), "gpt");
    config.api_key = Some(String::from("sk-test"));
    let mut connector = OpenAi::new(config).expect("client");
    connector.retry = Retry {
        attempts: 1,
        first_wait: Duration::from_millis(10),
        max_wait: Duration::from_millis(10),
    };

    let reply = connector
        .complete(request(vec![heard("hi")]))
        .await
        .expect("reply");

    assert_eq!(reply.actions.len(), 2);
    let captured = requests.recv().await.expect("request");
    let head = captured.head.to_ascii_lowercase();
    assert!(head.starts_with("post /v1/chat/completions http/1.1"));
    assert!(head.contains("authorization: bearer sk-test"));
}

fn replayed(reply: &str) -> Value {
    let reply = parse(reply).expect("parsed");
    let conversation = vec![
        heard("hi"),
        Entry::Acted {
            actions: reply.actions,
            answered: 1,
            trace: reply.trace,
        },
        heard("next"),
    ];
    sent(&config(), &request(conversation))["messages"][2].clone()
}

#[test]
fn a_reply_with_no_content_and_no_calls_is_replayed_with_empty_content() {
    let message = replayed(
        r#"{"choices":[{"message":{"role":"assistant","content":null},"finish_reason":"stop"}]}"#,
    );

    assert_eq!(message["role"], "assistant");
    assert_eq!(message["content"], "");
    assert!(message.get("tool_calls").is_none());
}

#[test]
fn an_empty_tool_call_list_is_not_replayed() {
    let message = replayed(
        r#"{"choices":[{"message":{"role":"assistant","content":"ok","tool_calls":[]},"finish_reason":"stop"}]}"#,
    );

    assert_eq!(message["content"], "ok");
    assert!(message.get("tool_calls").is_none());
}

#[test]
fn arguments_given_as_an_object_are_accepted() {
    let reply = parse(r#"{"choices":[{"message":{"role":"assistant","content":null,"tool_calls":[{"id":"c","type":"function","function":{"name":"read_file","arguments":{"path":"a"}}}]},"finish_reason":"tool_calls"}]}"#).expect("parsed");

    match reply.actions.as_slice() {
        [Action::UseTool(call)] => assert_eq!(call.input, r#"{"path":"a"}"#),
        other => panic!("unexpected actions {other:?}"),
    }
}

#[test]
fn the_listed_models_come_back_sorted_by_name() {
    let names = model_names(
        r#"{"object":"list","data":[{"id":"qwen3","object":"model"},{"id":"gemma4","object":"model"},{"id":"gemma4"}]}"#,
    );

    assert_eq!(
        names,
        Some(vec![String::from("gemma4"), String::from("qwen3")])
    );
}

#[test]
fn a_list_that_does_not_read_gives_no_models() {
    assert_eq!(model_names("<html>not found</html>"), None);
}

const SUMMARY_REPLY: &str = r#"{"id":"chatcmpl-1","object":"chat.completion","choices":[{"index":0,"message":{"role":"assistant","content":"  Open requests\n- none  "},"finish_reason":"stop"}]}"#;

fn summary_request() -> SummaryRequest {
    SummaryRequest {
        previous: None,
        conversation: vec![heard("hi")],
        time_zone: None,
    }
}

#[test]
fn the_summary_request_is_a_system_prompt_and_one_user_message_without_tools() {
    let mut config = config();
    config.max_tokens = Some(900);

    let body: Value =
        serde_json::from_str(&summary_body(&config, &summary_request())).expect("json");

    let messages = body["messages"].as_array().expect("messages");
    assert_eq!(messages.len(), 2);
    assert_eq!(messages[0]["role"], "system");
    assert_eq!(messages[0]["content"], SUMMARY_SYSTEM);
    assert_eq!(messages[1]["role"], "user");
    assert!(messages[1]["content"].as_str().expect("text").starts_with(
        "<conversation>
"
    ));
    assert_eq!(body["max_tokens"], 900);
    assert!(body.get("tools").is_none());
}

#[test]
fn a_summary_is_the_message_content_trimmed() {
    assert_eq!(
        parse_summary(SUMMARY_REPLY).expect("summary"),
        "Open requests
- none"
    );
}

#[test]
fn a_cut_off_filtered_or_empty_summary_is_a_failure() {
    let cut = r#"{"choices":[{"message":{"role":"assistant","content":"Open"},"finish_reason":"length"}]}"#;
    let filtered = r#"{"choices":[{"message":{"role":"assistant","content":null},"finish_reason":"content_filter"}]}"#;
    let blank =
        r#"{"choices":[{"message":{"role":"assistant","content":" "},"finish_reason":"stop"}]}"#;
    let none =
        r#"{"choices":[{"message":{"role":"assistant","content":null},"finish_reason":"stop"}]}"#;

    assert!(
        parse_summary(cut)
            .expect_err("cut")
            .detail
            .contains("token limit")
    );
    assert!(
        parse_summary(filtered)
            .expect_err("filtered")
            .detail
            .contains("declined")
    );
    assert!(
        parse_summary(blank)
            .expect_err("blank")
            .detail
            .contains("empty summary")
    );
    assert!(
        parse_summary(none)
            .expect_err("none")
            .detail
            .contains("empty summary")
    );
}

#[tokio::test]
async fn summarize_posts_the_summary_request_to_chat_completions() {
    let (base_url, mut requests) = mock::serve(vec![(200, SUMMARY_REPLY)]).await;
    let mut connector =
        OpenAi::new(OpenAiConfig::new(format!("{base_url}/v1"), "gpt")).expect("client");
    connector.retry = Retry {
        attempts: 1,
        first_wait: Duration::from_millis(10),
        max_wait: Duration::from_millis(10),
    };

    let summary = connector
        .summarize(summary_request())
        .await
        .expect("summary");

    assert_eq!(
        summary,
        "Open requests
- none"
    );
    let captured = requests.recv().await.expect("request");
    assert!(
        captured
            .head
            .to_ascii_lowercase()
            .starts_with("post /v1/chat/completions http/1.1")
    );
}

fn twelve_long_messages() -> Vec<Entry> {
    (0..12)
        .map(|n| heard(&format!("m{n:02}{}", "가".repeat(20_000))))
        .collect()
}

#[test]
fn a_context_window_makes_the_request_carry_less() {
    let conversation = twelve_long_messages();
    let mut small = config();
    small.context_window = Some(130_000);

    let whole = sent(&config(), &request(conversation.clone())).to_string();
    let cut = sent(&small, &request(conversation)).to_string();

    assert!(whole.contains("m00"));
    assert!(!cut.contains("m00"));
    assert!(cut.contains("m11"));
}

#[test]
fn the_model_reports_the_room_its_context_window_leaves() {
    let mut config = config();
    assert_eq!(
        OpenAi::new(config.clone()).expect("client").room(),
        Room::DEFAULT
    );

    config.context_window = Some(400_000);
    assert_eq!(
        OpenAi::new(config.clone()).expect("client").room(),
        Room::for_window(400_000, 16_000)
    );

    config.max_tokens = Some(8_000);
    assert_eq!(
        OpenAi::new(config).expect("client").room(),
        Room::for_window(400_000, 8_000)
    );
}
