use super::*;

#[test]
fn keys_become_down_and_up_events() {
    let enter = key_events("Enter").expect("known");
    assert_eq!(enter[0]["type"], "keyDown");
    assert_eq!(enter[0]["text"], "\r");
    assert_eq!(enter[0]["windowsVirtualKeyCode"], 13);
    assert_eq!(enter[1]["type"], "keyUp");

    let select = key_events("Control+A").expect("known");
    assert_eq!(select[0]["type"], "rawKeyDown");
    assert_eq!(select[0]["modifiers"], 2);
    assert_eq!(select[0]["commands"], json!(["selectAll"]));

    let letter = key_events("x").expect("known");
    assert_eq!(letter[0]["text"], "x");
    assert_eq!(letter[0]["windowsVirtualKeyCode"], 88);

    assert!(key_events("Hyper+X").is_err());
    assert!(key_events("NotAKey").is_err());
}

#[test]
fn get_by_calls_become_locator_steps() {
    let name = Value::Regex(std::sync::Arc::new((
        String::from("sign in"),
        String::from("i"),
    )));
    let options = crate::script::run::from_json(&json!({ "exact": true }));
    let step = get_by("getByRole", &[Value::Str("button".into()), options]).expect("a step");
    assert_eq!(step, json!({ "role": "button", "exact": true }));
    let step = get_by("getByText", &[name]).expect("a step");
    assert_eq!(
        step,
        json!({ "text": { "regex": "sign in", "flags": "i" }, "exact": false })
    );
    assert_eq!(get_by("click", &[]), None);
}

#[test]
fn url_globs_match_the_whole_address() {
    assert!(glob(
        "https://mail.example.com/inbox",
        "https://mail.example.com/inbox"
    ));
    assert!(glob("**/inbox/**", "https://mail.example.com/inbox/42"));
    assert!(!glob("**/inbox/**", "https://mail.example.com/login"));
    assert!(glob(
        "https://mail.example.com/*",
        "https://mail.example.com/login"
    ));
    assert!(!glob(
        "https://mail.example.com/*",
        "https://mail.example.com/a/b"
    ));
    assert!(glob("**", "https://anything.example/"));
    assert!(!glob("https://a.example/", "https://a.example/x"));
}
