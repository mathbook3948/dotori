use super::*;

const FORM: &str = "data:text/html,<html><head><title>Form</title><style>p{color:red}</style></head><body>\
<h1>Hello</h1><p>Some text</p><input id=q placeholder=Search><input type=password value=hunter2>\
<button onclick=\"document.querySelector('p').textContent='got '+document.getElementById('q').value\">Go</button>\
<a href=\"https://example.com/\">Example</a><div style=\"display:none\"><button>Hidden</button></div></body></html>";

static NEXT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

fn browser() -> Arc<Browser> {
    let n = NEXT.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    let base = std::env::temp_dir().join(format!("dotori-browser-test-{}-{n}", std::process::id()));
    Browser::new(BrowserConfig {
        executable: std::env::var_os("DOTORI_TEST_CHROME").map(PathBuf::from),
        downloads: base.join("chrome"),
        profile: base.join("profile"),
        headless: true,
    })
}

async fn in_page(running: &Running, expression: &str) -> Value {
    running
        .cdp
        .send(
            "Runtime.evaluate",
            json!({ "expression": expression, "returnByValue": true, "awaitPromise": true }),
            Some(&running.session),
        )
        .await
        .expect("evaluated")
        .pointer("/result/value")
        .cloned()
        .unwrap_or(Value::Null)
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
#[ignore = "starts a real Chrome; set DOTORI_TEST_CHROME or let it download one"]
async fn a_page_is_read_clicked_and_typed_into() {
    let browser = browser();
    let running = browser.page().await.expect("started");
    let cdp = Arc::clone(&running.cdp);
    let page = Page { running: &running };
    page.go(FORM).await.expect("opened");
    let shown = page.snapshot().await.expect("read");
    assert!(shown.contains("Title: Form"), "{shown}");
    assert!(shown.contains("# Hello"), "{shown}");
    assert!(shown.contains("[1] input text: Search"), "{shown}");
    assert!(shown.contains("[2] input password: "), "{shown}");
    assert!(shown.contains("[3] button: Go"), "{shown}");
    assert!(
        shown.contains("[4] link: Example (https://example.com/)"),
        "{shown}"
    );
    let body = shown.split_once("\n\n").expect("a body").1;
    assert!(
        !body.contains("Hidden") && !body.contains("color") && !body.contains("hunter2"),
        "{body}"
    );

    page.type_into(1, "도토리", false).await.expect("typed");
    page.click(3).await.expect("clicked");
    let after = page.snapshot().await.expect("read again");
    assert!(after.contains("got 도토리"), "{after}");
    assert!(page.click(99).await.is_err());
    let refused = page.type_into(3, "x", false).await.expect_err("a button");
    assert!(refused.contains("does not take text"), "{refused}");
    page.guard().await.expect("a data page is fine");

    page.evaluate("location.href = 'http://127.0.0.1:9/'")
        .await
        .ok();
    page.settle().await.expect("settled");
    let moved = page.guard().await.expect_err("a local address");
    assert!(moved.contains("refused"), "{moved}");
    drop(running);

    let tools = browser.tools();
    let read = tools[1].call(json!({})).await.expect("tool read");
    assert!(read.contains("Title: \nURL: about:blank"), "{read}");

    let second = Browser::new(browser.config.clone());
    drop(
        second
            .page()
            .await
            .expect("the left-over browser was closed"),
    );
    assert!(!cdp.open(), "the first browser is gone");
    let _ = std::fs::remove_dir_all(&browser.config.profile);
}

#[tokio::test]
#[ignore = "starts a real Chrome; set DOTORI_TEST_CHROME or let it download one"]
async fn a_page_sees_no_automation_and_no_trace_of_dotori() {
    let browser = browser();
    let running = browser.page().await.expect("started");
    let page = Page { running: &running };
    page.go(FORM).await.expect("opened");
    page.snapshot().await.expect("read");

    assert_eq!(in_page(&running, "navigator.webdriver").await, json!(false));
    assert_eq!(
        in_page(&running, "typeof globalThis.__refs").await,
        json!("undefined")
    );
    assert_eq!(
        in_page(&running, "alert.toString().includes('[native code]')").await,
        json!(true)
    );
    assert_eq!(
        in_page(&running, "navigator.userAgent.includes('Headless')").await,
        json!(false)
    );
    assert_eq!(
        in_page(
            &running,
            "(navigator.userAgentData?.brands ?? []).some(b => b.brand.includes('Headless'))"
        )
        .await,
        json!(false)
    );
    assert_eq!(
        in_page(
            &running,
            "new Promise(r => setTimeout(() => r(confirm('sure?')), 0))"
        )
        .await,
        json!(false)
    );
    assert_eq!(
        in_page(
            &running,
            "new Promise(r => setTimeout(() => { alert('hi'); r('went on'); }, 0))"
        )
        .await,
        json!("went on")
    );
    drop(running);
    let _ = std::fs::remove_dir_all(&browser.config.profile);
}

#[test]
fn a_headless_agent_reads_as_a_windowed_chrome() {
    let shown = plain_agent(
        "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 (KHTML, like Gecko) HeadlessChrome/158.0.8127.36 Safari/537.36",
    );
    let agent = shown["userAgent"].as_str().expect("an agent");
    assert!(agent.contains(" Chrome/158.0.8127.36 "), "{agent}");
    assert!(!agent.contains("Headless"), "{agent}");
    assert_eq!(shown["userAgentMetadata"]["brands"][0]["version"], "158");
    assert_eq!(shown["userAgentMetadata"]["fullVersion"], "158.0.8127.36");
}

#[test]
fn only_documents_from_public_addresses_may_load() {
    assert!(document_allowed("https://example.com/").is_ok());
    for refused in [
        "http://127.0.0.1:9/",
        "http://localhost/",
        "http://192.168.0.1/admin",
        "file:///etc/passwd",
        "not a url",
    ] {
        assert!(document_allowed(refused).is_err(), "{refused}");
    }
}

#[test]
fn passwords_are_turned_off_and_other_preferences_are_kept() {
    let profile = std::env::temp_dir().join(format!("dotori-browser-prefs-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&profile);
    std::fs::create_dir_all(profile.join("Default")).expect("made");
    std::fs::write(
        profile.join("Default").join("Preferences"),
        r#"{"intl":{"accept_languages":"ko"},"profile":{"name":"bot"}}"#,
    )
    .expect("written");

    forget_passwords(&profile).expect("written");

    let kept: Value = serde_json::from_str(
        &std::fs::read_to_string(profile.join("Default").join("Preferences")).expect("read"),
    )
    .expect("json");
    assert_eq!(kept["credentials_enable_service"], false);
    assert_eq!(kept["profile"]["password_manager_enabled"], false);
    assert_eq!(kept["profile"]["name"], "bot");
    assert_eq!(kept["intl"]["accept_languages"], "ko");
    let _ = std::fs::remove_dir_all(&profile);
}
