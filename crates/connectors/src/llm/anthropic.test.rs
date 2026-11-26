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
use dotori_core::tool::ToolSpec;

use super::*;
use crate::llm::mock;

const TOOL_REPLY: &str = r#"{"id":"msg_1","type":"message","role":"assistant","model":"claude-sonnet-5-5","content":[{"type":"thinking","thinking":"They said hi.","signature":"sig"},{"type":"tool_use","id":"toolu_1","name":"say","input":{"thread":0,"text":"hello"}},{"type":"tool_use","id":"toolu_2","name":"read_file","input":{"path":"a.txt"}}],"stop_reason":"tool_use","stop_details":null,"usage":{"input_tokens":10,"output_tokens":5}}"#;

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
            input_schema: String::from(
                r#"{"type":"object","properties":{"path":{"type":"string"}}}"#,
            ),
        }],
        context: None,
    }
}

fn sent(config: &AnthropicConfig, request: &ModelRequest) -> Value {
    serde_json::from_str(&body(config, request).expect("body")).expect("json")
}

#[test]
fn the_request_asks_for_adaptive_thinking_auto_tools_and_default_fallbacks() {
    let config = AnthropicConfig::new("key");

    let body = sent(&config, &request(vec![heard("hi")]));

    assert_eq!(body["model"], "claude-sonnet-5-5");
    assert_eq!(body["max_tokens"], 16000);
    assert_eq!(body["system"][0]["text"], DEFAULT_SYSTEM);
    assert_eq!(body["system"][0]["cache_control"]["type"], "ephemeral");
    assert_eq!(body["thinking"]["type"], "adaptive");
    assert_eq!(
        body["thinking"]["block_binding"]["prefix_mismatch_behavior"],
        "drop_block"
    );
    assert_eq!(body["tool_choice"]["type"], "auto");
    assert_eq!(body["fallbacks"], "default");
    assert!(body.get("output_config").is_none());
    assert!(body.get("temperature").is_none());
    let names: Vec<&str> = body["tools"]
        .as_array()
        .expect("tools")
        .iter()
        .map(|t| t["name"].as_str().expect("name"))
        .collect();
    assert_eq!(names, vec!["say", "delegate", "read_file"]);
    assert_eq!(
        body["tools"][2]["input_schema"]["properties"]["path"]["type"],
        "string"
    );
    assert_eq!(body["messages"][0]["role"], "user");
    assert!(
        body["messages"][0]["content"][0]["text"]
            .as_str()
            .expect("text")
            .contains("hi")
    );
}

#[test]
fn effort_is_sent_only_when_set() {
    let mut config = AnthropicConfig::new("key");
    config.effort = Some(String::from("low"));

    let body = sent(&config, &request(vec![heard("hi")]));

    assert_eq!(body["output_config"]["effort"], "low");
}

#[test]
fn an_invalid_tool_schema_fails_the_request_before_sending() {
    let mut request = request(vec![heard("hi")]);
    request.tools[0].input_schema = String::from("not json");

    let error = body(&AnthropicConfig::new("key"), &request).expect_err("rejected");

    assert!(error.detail.contains("read_file"));
}

#[test]
fn a_reply_becomes_actions_and_is_replayed_unchanged() {
    let reply = parse(TOOL_REPLY).expect("parsed");

    match reply.actions.as_slice() {
        [Action::Say { thread, text, .. }, Action::UseTool(call)] => {
            assert_eq!(thread.get(), 0);
            assert_eq!(text, "hello");
            assert_eq!(call.id.as_str(), "toolu_2");
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
    ];

    let body = sent(&AnthropicConfig::new("key"), &request(conversation));

    let original: Value = serde_json::from_str(TOOL_REPLY).expect("json");
    assert_eq!(body["messages"][1]["role"], "assistant");
    assert_eq!(body["messages"][1]["content"], original["content"]);
    let results = body["messages"][2]["content"].as_array().expect("content");
    assert_eq!(results[0]["tool_use_id"], "toolu_1");
    assert_eq!(results[1]["tool_use_id"], "toolu_2");
    assert_eq!(results[1]["is_error"], true);
}

#[test]
fn a_refusal_and_a_cut_off_reply_are_failures() {
    let refused = r#"{"content":[],"stop_reason":"refusal","stop_details":{"type":"refusal","category":"cyber"}}"#;
    let cut = r#"{"content":[{"type":"text","text":"and then"}],"stop_reason":"max_tokens"}"#;

    assert!(
        parse(refused)
            .expect_err("refused")
            .detail
            .contains("cyber")
    );
    assert!(parse(cut).expect_err("cut").detail.contains("token limit"));
}

#[test]
fn an_empty_reply_keeps_no_trace() {
    let reply = parse(r#"{"content":[],"stop_reason":"end_turn"}"#).expect("parsed");

    assert!(reply.actions.is_empty());
    assert!(reply.trace.is_none());
}

#[test]
fn foreign_tool_ids_are_rewritten_into_what_claude_accepts() {
    assert_eq!(id_claude_accepts("call_ab.c:1"), "call_ab_c_1");
    assert_eq!(id_claude_accepts("toolu_01-x"), "toolu_01-x");
}

#[test]
fn the_api_key_never_shows_in_debug_output() {
    let text = format!("{:?}", AnthropicConfig::new("sk-secret"));

    assert!(!text.contains("sk-secret"));
}

fn connector(base_url: String) -> Anthropic {
    let mut config = AnthropicConfig::new("sk-test");
    config.base_url = base_url;
    let mut connector = Anthropic::new(config).expect("client");
    connector.retry = Retry {
        attempts: 3,
        first_wait: Duration::from_millis(10),
        max_wait: Duration::from_millis(10),
    };
    connector
}

#[tokio::test]
async fn complete_posts_to_the_messages_api_with_its_headers() {
    let (base_url, mut requests) = mock::serve(vec![(200, TOOL_REPLY)]).await;

    let reply = connector(base_url)
        .complete(request(vec![heard("hi")]))
        .await
        .expect("reply");

    assert_eq!(reply.actions.len(), 2);
    let captured = requests.recv().await.expect("request");
    let head = captured.head.to_ascii_lowercase();
    assert!(head.starts_with("post /v1/messages http/1.1"));
    assert!(head.contains("x-api-key: sk-test"));
    assert!(head.contains("anthropic-version: 2026-10-21"));
    assert!(head.contains(
        "anthropic-beta: server-side-fallback-2026-07-01,thinking-binding-controls-2026-08-01"
    ));
    let body: Value = serde_json::from_str(&captured.body).expect("json body");
    assert_eq!(body["model"], "claude-sonnet-5-5");
}

#[tokio::test]
async fn an_overloaded_api_is_tried_again() {
    let overloaded =
        r#"{"type":"error","error":{"type":"overloaded_error","message":"Overloaded"}}"#;
    let (base_url, _requests) = mock::serve(vec![(529, overloaded), (200, TOOL_REPLY)]).await;

    let reply = connector(base_url)
        .complete(request(vec![heard("hi")]))
        .await;

    assert_eq!(reply.expect("second try").actions.len(), 2);
}

#[tokio::test]
async fn a_bad_request_fails_at_once_with_the_api_message() {
    let bad = r#"{"type":"error","error":{"type":"invalid_request_error","message":"messages: roles must alternate"}}"#;
    let (base_url, _requests) = mock::serve(vec![(400, bad), (200, TOOL_REPLY)]).await;

    let error = connector(base_url)
        .complete(request(vec![heard("hi")]))
        .await
        .expect_err("failed");

    assert!(error.detail.contains("400"));
    assert!(
        error
            .detail
            .contains("invalid_request_error messages: roles must alternate")
    );
}

#[tokio::test]
async fn retries_stop_after_the_last_attempt() {
    let busy = r#"{"type":"error","error":{"type":"rate_limit_error","message":"slow down"}}"#;
    let (base_url, _requests) = mock::serve(vec![
        (429, busy),
        (429, busy),
        (429, busy),
        (200, TOOL_REPLY),
    ])
    .await;

    let error = connector(base_url)
        .complete(request(vec![heard("hi")]))
        .await
        .expect_err("gave up");

    assert!(error.detail.contains("after 3 attempts"));
}

#[test]
fn a_reply_with_a_blank_text_block_is_not_replayed_raw() {
    let reply = parse(r#"{"content":[{"type":"text","text":""},{"type":"tool_use","id":"toolu_1","name":"say","input":{"thread":0,"text":"hi"}}],"stop_reason":"tool_use"}"#).expect("parsed");

    assert_eq!(reply.actions.len(), 1);
    assert!(reply.trace.is_none());
}

#[test]
fn the_app_context_follows_the_system_prompt() {
    let config = AnthropicConfig::new("key");
    let mut asked = request(vec![heard("hi")]);
    asked.context = Some(String::from("The owner likes tea."));

    let body = sent(&config, &asked);

    assert_eq!(
        body["system"][0]["text"],
        format!("{DEFAULT_SYSTEM}\n\nThe owner likes tea.")
    );
}

#[test]
fn the_last_two_user_turns_end_where_the_next_request_can_read_the_cache() {
    let config = AnthropicConfig::new("key");
    let said = |answered| Entry::Acted {
        actions: vec![Action::Say {
            thread: ThreadRef::new(0),
            text: String::from("ok"),
            privately: false,
        }],
        answered,
        trace: None,
    };
    let conversation = vec![heard("one"), said(1), heard("two"), said(3), heard("three")];

    let body = sent(&config, &request(conversation));

    let marked: Vec<(String, bool)> = body["messages"]
        .as_array()
        .expect("messages")
        .iter()
        .flat_map(|message| {
            let role = message["role"].as_str().expect("role").to_owned();
            message["content"]
                .as_array()
                .expect("content")
                .iter()
                .map(move |block| (role.clone(), block.get("cache_control").is_some()))
                .collect::<Vec<_>>()
        })
        .filter(|(_, marked)| *marked)
        .collect();
    assert_eq!(
        marked,
        [(String::from("user"), true), (String::from("user"), true)]
    );
    let last = body["messages"].as_array().expect("messages").len() - 1;
    let last_block = body["messages"][last]["content"]
        .as_array()
        .expect("content")
        .len()
        - 1;
    assert_eq!(
        body["messages"][last]["content"][last_block]["cache_control"]["type"],
        "ephemeral"
    );
    assert!(
        body["messages"][0]["content"][0]
            .get("cache_control")
            .is_none()
    );
}

#[test]
fn a_user_turn_that_ends_in_a_tool_result_is_marked_on_the_result() {
    let config = AnthropicConfig::new("key");
    let conversation = vec![
        heard("read it"),
        Entry::Acted {
            actions: vec![Action::UseTool(ToolCall {
                id: ToolCallId::new("toolu_1"),
                name: String::from("read_file"),
                input: String::from(r#"{"path":"a.txt"}"#),
            })],
            answered: 1,
            trace: None,
        },
        Entry::ToolReturned {
            call: ToolCallId::new("toolu_1"),
            result: Ok(String::from("contents")),
        },
    ];

    let body = sent(&config, &request(conversation));

    let last = &body["messages"][2]["content"][0];
    assert_eq!(last["type"], "tool_result");
    assert_eq!(last["cache_control"]["type"], "ephemeral");
    assert_eq!(
        body["messages"][0]["content"][0]["cache_control"]["type"],
        "ephemeral"
    );
}
