use dotori_core::conversation::Action;
use dotori_core::tool::ToolSpec;

use super::*;

fn call(id: &str, name: &str, input: &str) -> Use {
    Use {
        id: id.to_owned(),
        name: name.to_owned(),
        input: input.to_owned(),
    }
}

#[test]
fn say_becomes_a_reply_to_its_thread() {
    let decided = decide(vec![call("a", SAY, r#"{"thread":3,"text":"hi"}"#)]);

    assert_eq!(decided.ids, vec![String::from("a")]);
    assert!(decided.rejected.is_empty());
    match decided.actions.as_slice() {
        [Action::Say { thread, text, .. }] => {
            assert_eq!(thread.get(), 3);
            assert_eq!(text, "hi");
        }
        other => panic!("expected one say, got {other:?}"),
    }
}

#[test]
fn say_without_a_thread_or_with_blank_text_is_rejected() {
    let decided = decide(vec![
        call("a", SAY, r#"{"text":"hi"}"#),
        call("b", SAY, r#"{"thread":1,"text":"  "}"#),
        call("c", DELEGATE, r#"{"title":"t","brief":""}"#),
    ]);

    assert!(decided.actions.is_empty());
    let ids: Vec<&str> = decided.rejected.iter().map(|r| r.id.as_str()).collect();
    assert_eq!(ids, vec!["a", "b", "c"]);
}

#[test]
fn delegate_and_other_tools_keep_their_order_and_ids() {
    let decided = decide(vec![
        call("x", "read_file", r#"{"path":"a"}"#),
        call("y", DELEGATE, r#"{"title":"Build","brief":"Build it."}"#),
    ]);

    assert_eq!(decided.ids, vec![String::from("x"), String::from("y")]);
    match decided.actions.as_slice() {
        [Action::UseTool(tool), Action::Delegate(spec)] => {
            assert_eq!(tool.id.as_str(), "x");
            assert_eq!(tool.name, "read_file");
            assert_eq!(tool.input, r#"{"path":"a"}"#);
            assert_eq!(spec.title, "Build");
            assert_eq!(spec.brief, "Build it.");
        }
        other => panic!("unexpected actions {other:?}"),
    }
}

#[test]
fn built_in_tools_come_first_and_win_a_name_clash() {
    let toolbox = vec![
        ToolSpec {
            name: String::from(SAY),
            description: String::from("Impostor."),
            input_schema: String::from("{}"),
        },
        ToolSpec {
            name: String::from("read_file"),
            description: String::from("Reads."),
            input_schema: String::from("{}"),
        },
    ];

    let names: Vec<String> = tools(&toolbox).into_iter().map(|t| t.name).collect();

    assert_eq!(names, vec!["say", "delegate", "read_file"]);
    for spec in tools(&[]) {
        serde_json::from_str::<serde_json::Value>(&spec.input_schema).expect("valid schema");
    }
}

#[test]
fn a_blank_context_leaves_the_system_prompt_alone() {
    assert_eq!(system("base", None), "base");
    assert_eq!(system("base", Some("  ")), "base");
    assert_eq!(system("base", Some("more")), "base\n\nmore");
}

#[test]
fn say_can_be_sent_privately_and_is_public_by_default() {
    let decided = decide(vec![
        call("a", SAY, r#"{"thread":1,"text":"psst","privately":true}"#),
        call("b", SAY, r#"{"thread":1,"text":"hello"}"#),
    ]);

    let private: Vec<bool> = decided
        .actions
        .iter()
        .map(|action| match action {
            Action::Say { privately, .. } => *privately,
            other => panic!("expected say, got {other:?}"),
        })
        .collect();
    assert_eq!(private, vec![true, false]);
}
