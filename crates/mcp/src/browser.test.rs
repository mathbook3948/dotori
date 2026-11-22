use super::*;

const FORM: &str = "data:text/html,<html><head><title>Form</title><style>p{color:red}</style></head><body>\
<h1>Hello</h1><p>Some text</p><input id=q placeholder=Search>\
<button onclick=\"document.querySelector('p').textContent='got '+document.getElementById('q').value\">Go</button>\
<a href=\"https://example.com/\">Example</a><div style=\"display:none\"><button>Hidden</button></div></body></html>";

fn browser() -> Arc<Browser> {
    let profile = std::env::temp_dir().join(format!("dotori-browser-test-{}", std::process::id()));
    Browser::new(BrowserConfig {
        executable: None,
        profile,
        headless: true,
    })
}

#[test]
fn every_tool_has_a_distinct_name_and_an_object_schema() {
    let tools = browser().tools();
    let names: Vec<&str> = tools.iter().map(|tool| tool.name()).collect();
    assert_eq!(
        names,
        [
            "browser_open",
            "browser_read",
            "browser_click",
            "browser_type"
        ]
    );
    for tool in &tools {
        assert_eq!(tool.input_schema()["type"], "object", "{}", tool.name());
    }
}

#[tokio::test]
async fn opening_a_local_address_is_refused_before_the_browser_starts() {
    let tools = browser().tools();
    let refused = tools[0]
        .call(json!({ "url": "http://127.0.0.1:8080/" }))
        .await
        .expect_err("refused");
    assert!(refused.contains("not a public address"), "{refused}");
}

#[tokio::test]
#[ignore = "starts a real Chrome"]
async fn a_page_is_read_clicked_and_typed_into() {
    let browser = browser();
    let (cdp, session) = browser.page().await.expect("started");
    let page = Page {
        cdp: &cdp,
        session: &session,
    };
    page.go(FORM).await.expect("opened");
    let shown = page.snapshot().await.expect("read");
    assert!(shown.contains("Title: Form"), "{shown}");
    assert!(shown.contains("# Hello"), "{shown}");
    assert!(shown.contains("[1] input text: Search"), "{shown}");
    assert!(shown.contains("[2] button: Go"), "{shown}");
    assert!(
        shown.contains("[3] link: Example (https://example.com/)"),
        "{shown}"
    );
    let body = shown.split_once("\n\n").expect("a body").1;
    assert!(
        !body.contains("Hidden") && !body.contains("color"),
        "{body}"
    );

    page.type_into(1, "도토리", false).await.expect("typed");
    page.click(2).await.expect("clicked");
    let after = page.snapshot().await.expect("read again");
    assert!(after.contains("got 도토리"), "{after}");
    assert!(page.click(99).await.is_err());

    let tools = browser.tools();
    let read = tools[1].call(json!({})).await.expect("tool read");
    assert!(read.contains("got 도토리"), "{read}");
    let _ = std::fs::remove_dir_all(&browser.config.profile);
}
