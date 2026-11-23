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
    assert_eq!(names, ["browser_run", "browser_read"]);
    for tool in &tools {
        assert_eq!(tool.input_schema()["type"], "object", "{}", tool.name());
    }
}

#[tokio::test]
async fn code_that_does_not_parse_is_refused_before_the_browser_starts() {
    let tools = browser().tools();
    let refused = tools[0]
        .call(json!({ "code": "class Foo {}" }))
        .await
        .expect_err("refused");
    assert!(refused.contains("does not parse"), "{refused}");
    assert!(refused.contains("`class`"), "{refused}");
}

const SHOP: &str = "data:text/html,<html><head><title>Shop</title></head><body>\
<h1>Order</h1><p id=said>nothing yet</p>\
<label>Name <input id=q placeholder=Search></label><input type=password value=hunter2>\
<select id=fruit><option value=a>Apple</option><option value=b>Banana</option></select>\
<label><input type=checkbox id=agree> Agree</label>\
<button onclick=\"document.getElementById('said').textContent='got '+document.getElementById('q').value+' '+document.getElementById('fruit').value+' '+document.getElementById('agree').checked\">Go</button>\
<a href=\"https://example.com/\">Example</a><a href=\"https://example.org/\">Other</a>\
<div style=\"display:none\"><button>Hidden</button></div></body></html>";

#[tokio::test]
#[ignore = "starts a real Chrome; set DOTORI_TEST_CHROME or let it download one"]
async fn a_playwright_script_fills_clicks_and_reads_the_page() {
    let browser = browser();
    let running = browser.page().await.expect("started");
    let page = Page { running: &running };
    page.go(SHOP).await.expect("opened");
    let driver = Driver {
        page: Page { running: &running },
    };
    let body = script::parse::parse(
        "await page.getByLabel('Name').fill('도토리');\n\
         await page.locator('#fruit').selectOption('Banana');\n\
         await page.getByRole('checkbox', { name: 'Agree' }).check();\n\
         await page.getByRole('button', { name: 'Go' }).click();\n\
         const said = await page.locator('#said').textContent();\n\
         console.log('links', await page.getByRole('link').count());\n\
         try { await page.locator('input').fill('x'); } catch (e) { console.log(e.message); }\n\
         try { await page.getByText('Nowhere').click({ timeout: 300 }); } catch (e) { console.log(e.message); }\n\
         await page.locator('#q').press('Control+A');\n\
         await page.keyboard.press('Backspace');\n\
         const names = await page.getByRole('link').allInnerTexts();\n\
         return { said, left: await page.locator('#q').inputValue(), names, title: await page.evaluate(() => document.title) };",
    )
    .expect("parsed");

    let outcome = script::run::run(
        &body,
        &driver,
        Native {
            kind: "page",
            data: Value::Null,
        },
    )
    .await
    .expect("ran");

    assert_eq!(
        outcome.value.as_deref(),
        Some(
            r#"{"said":"got 도토리 b true","left":"","names":["Example","Other"],"title":"Shop"}"#
        )
    );
    assert_eq!(outcome.logs[0], "links 2");
    assert!(
        outcome.logs[1].contains("locator('input') matched 3 elements"),
        "{}",
        outcome.logs[1]
    );
    assert!(
        outcome.logs[2].contains("matches no element after 300 ms"),
        "{}",
        outcome.logs[2]
    );
    let snapshot = page.snapshot().await.expect("read");
    let marked = snapshot
        .lines()
        .find(|line| line.contains("link: Other"))
        .and_then(|line| line.strip_prefix('['))
        .and_then(|line| line.split(']').next())
        .expect("a numbered link")
        .to_owned();
    let picked = script::parse::parse(&format!(
        "return await page.locator('ref={marked}').getAttribute('href');"
    ))
    .expect("parsed");
    let found = script::run::run(
        &picked,
        &driver,
        Native {
            kind: "page",
            data: Value::Null,
        },
    )
    .await
    .expect("ran");
    assert_eq!(found.value.as_deref(), Some("https://example.org/"));
    drop(running);

    let tools = browser.tools();
    let result = tools[0]
        .call(json!({ "code": "console.log('hi'); return 1 + 1;" }))
        .await
        .expect("ran");
    assert!(
        result.starts_with("hi\nReturned: 2\n\n---\nTitle: Shop"),
        "{result}"
    );
    let failed = tools[0]
        .call(json!({ "code": "await page.goto('http://127.0.0.1:9/');" }))
        .await
        .expect_err("refused");
    assert!(
        failed.contains("not a public address") && failed.contains("The page now:"),
        "{failed}"
    );
    let _ = std::fs::remove_dir_all(&browser.config.profile);
}

#[tokio::test]
#[ignore = "starts a real Chrome; set DOTORI_TEST_CHROME or let it download one"]
async fn a_page_is_read_and_a_move_to_a_refused_address_is_undone() {
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
