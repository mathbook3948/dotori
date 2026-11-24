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
        logins: None,
        executable: std::env::var_os("DOTORI_TEST_CHROME").map(PathBuf::from),
        downloads: base.join("chrome"),
        profile: base.join("profile"),
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
    assert_eq!(names, ["browser_run", "browser_read", "browser_ask_owner"]);
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

#[test]
fn the_owners_input_becomes_devtools_events() {
    let (method, click) = owner_event(
        &json!({ "kind": "mouse", "event": "down", "x": 5, "y": 6, "button": "left", "clicks": 1 }),
    )
    .expect("an event");
    assert_eq!(method, "Input.dispatchMouseEvent");
    assert_eq!(click["type"], "mousePressed");
    assert_eq!(click["clickCount"], 1.0);
    let (_, key) = owner_event(
        &json!({ "kind": "key", "event": "down", "key": "Enter", "code": 13, "text": "\r" }),
    )
    .expect("an event");
    assert_eq!(key["type"], "keyDown");
    assert_eq!(key["text"], "\r");
    let (_, arrow) =
        owner_event(&json!({ "kind": "key", "event": "down", "key": "ArrowLeft", "code": 37 }))
            .expect("an event");
    assert_eq!(arrow["type"], "rawKeyDown");
    let (method, compose) =
        owner_event(&json!({ "kind": "compose", "text": "한글" })).expect("an event");
    assert_eq!(method, "Input.imeSetComposition");
    assert_eq!(compose["selectionEnd"], 2);
    assert!(owner_event(&json!({ "kind": "mouse", "event": "fly" })).is_none());
    let (_, select) = owner_event(
        &json!({ "kind": "key", "event": "down", "key": "a", "code": 65, "modifiers": 2 }),
    )
    .expect("an event");
    assert_eq!(select["type"], "rawKeyDown");
    assert_eq!(select["commands"], json!(["selectAll"]));
    let (_, plain) = owner_event(
        &json!({ "kind": "key", "event": "down", "key": "a", "code": 65, "text": "a" }),
    )
    .expect("an event");
    assert!(plain.get("commands").is_none());
}

#[tokio::test]
async fn asking_the_owner_shows_on_the_screen_and_returns_at_once() {
    let browser = browser();
    let tools = browser.tools();
    assert!(tools[2].call(json!({ "reason": " " })).await.is_err());

    let said = tools[2]
        .call(json!({ "reason": "Log in to the bank" }))
        .await
        .expect("asked");

    assert!(said.contains("Asked the owner"), "{said}");
    assert_eq!(
        browser.screen.state.lock().expect("unpoisoned")["asked"],
        "Log in to the bank"
    );
}

const PICKS: &str = "data:text/html,<html><body style=\"margin:0\">\
<select id=s style=\"position:absolute;left:10px;top:10px;width:200px;height:30px\"><option>Apple</option><option>Banana</option></select>\
<input id=d type=date style=\"position:absolute;left:10px;top:60px;width:200px;height:30px\">\
<p id=said style=\"position:absolute;left:10px;top:120px\">none</p>\
<script>for (const id of ['s','d']) document.getElementById(id).addEventListener('change', (e) => { document.getElementById('said').textContent = id + '=' + e.target.value; });</script>\
</body></html>";

#[tokio::test]
#[ignore = "starts a real Chrome; set DOTORI_TEST_CHROME or let it download one"]
async fn a_select_or_date_under_the_owners_click_becomes_a_picker() {
    let browser = browser();
    let running = browser.page().await.expect("started");
    Page { running: &running }.go(PICKS).await.expect("opened");
    let (cdp, session) = (Arc::clone(&running.cdp), running.session.clone());
    drop(running);

    let select = in_world(&cdp, &session, &format!("({PICKABLE})(50, 25)"))
        .await
        .expect("probed");
    assert_eq!(select["kind"], "select");
    assert_eq!(select["options"][1]["label"], "Banana");
    let date = in_world(&cdp, &session, &format!("({PICKABLE})(50, 75)"))
        .await
        .expect("probed");
    assert_eq!(date["kind"], "value");
    assert_eq!(date["type"], "date");
    assert_eq!(
        in_world(&cdp, &session, &format!("({PICKABLE})(400, 400)"))
            .await
            .expect("probed"),
        Value::Null
    );

    in_world(&cdp, &session, &format!("({CHOOSE})(50, 25, 1)"))
        .await
        .expect("chosen");
    in_world(&cdp, &session, &format!("({CHOOSE})(50, 75, '2026-11-23')"))
        .await
        .expect("chosen");
    let said = in_world(
        &cdp,
        &session,
        "document.getElementById('said').textContent",
    )
    .await
    .expect("read");
    assert_eq!(said, "d=2026-11-23");
    let fruit = in_world(&cdp, &session, "document.getElementById('s').value")
        .await
        .expect("read");
    assert_eq!(fruit, "Banana");
    let _ = std::fs::remove_dir_all(&browser.config.profile);
}

#[tokio::test]
#[ignore = "starts a real Chrome; set DOTORI_TEST_CHROME or let it download one"]
async fn a_window_the_page_opens_is_followed_until_it_closes() {
    let browser = browser();
    let running = browser.page().await.expect("started");
    let (cdp, session) = (Arc::clone(&running.cdp), running.session.clone());
    drop(running);
    let popup = || {
        browser
            .shown
            .lock()
            .expect("unpoisoned")
            .as_ref()
            .and_then(|shown| shown.popup.clone())
    };

    Page { running: &browser.page().await.expect("running") }
        .go("data:text/html,<a href=\"about:blank\" target=\"_blank\" style=\"position:absolute;left:0;top:0;width:200px;height:50px;display:block\">log in</a>")
        .await
        .expect("opened");
    for kind in ["mousePressed", "mouseReleased"] {
        cdp.send(
            "Input.dispatchMouseEvent",
            json!({ "type": kind, "x": 20, "y": 20, "button": "left", "clickCount": 1 }),
            Some(&session),
        )
        .await
        .expect("clicked");
    }
    let mut followed = None;
    for _ in 0..50 {
        followed = popup();
        if followed.is_some() {
            break;
        }
        time::sleep(Duration::from_millis(100)).await;
    }
    let (target, _) = followed.expect("the popup is shown");
    assert_eq!(
        browser.screen.state.lock().expect("unpoisoned")["popup"],
        true
    );

    cdp.send("Target.closeTarget", json!({ "targetId": target }), None)
        .await
        .expect("closed");
    for _ in 0..50 {
        if popup().is_none() {
            break;
        }
        time::sleep(Duration::from_millis(100)).await;
    }
    assert!(popup().is_none(), "back on the page");
    assert_eq!(
        browser.screen.state.lock().expect("unpoisoned")["popup"],
        false
    );
    let _ = std::fs::remove_dir_all(&browser.config.profile);
}

#[tokio::test]
#[ignore = "starts a real Chrome; set DOTORI_TEST_CHROME or let it download one"]
async fn a_popup_to_a_refused_address_never_loads_it() {
    let browser = browser();
    let running = browser.page().await.expect("started");
    let (cdp, session) = (Arc::clone(&running.cdp), running.session.clone());
    Page { running: &running }
        .go("data:text/html,<a href=\"http://127.0.0.1:9/secret\" target=\"_blank\" style=\"position:absolute;left:0;top:0;width:200px;height:50px;display:block\">open</a>")
        .await
        .expect("opened");
    drop(running);
    for kind in ["mousePressed", "mouseReleased"] {
        cdp.send(
            "Input.dispatchMouseEvent",
            json!({ "type": kind, "x": 20, "y": 20, "button": "left", "clickCount": 1 }),
            Some(&session),
        )
        .await
        .expect("clicked");
    }
    let mut popup = None;
    for _ in 0..50 {
        popup = browser
            .shown
            .lock()
            .expect("unpoisoned")
            .as_ref()
            .and_then(|shown| shown.popup.clone());
        if popup.is_some() {
            break;
        }
        time::sleep(Duration::from_millis(100)).await;
    }
    let state = browser.screen.state.lock().expect("unpoisoned").clone();
    let (_, popup_session) = popup.unwrap_or_else(|| panic!("the popup is followed: {state}"));
    time::sleep(Duration::from_millis(800)).await;

    let at = in_world(&cdp, &popup_session, "location.href")
        .await
        .expect("read");
    let text = in_world(
        &cdp,
        &popup_session,
        "document.body ? document.body.innerText : ''",
    )
    .await
    .expect("read");
    assert!(
        !text.as_str().unwrap_or_default().contains("secret"),
        "{text}"
    );
    assert_ne!(at, "http://127.0.0.1:9/secret", "{at}");
    let _ = std::fs::remove_dir_all(&browser.config.profile);
}

#[tokio::test]
#[ignore = "starts a real Chrome; set DOTORI_TEST_CHROME or let it download one"]
async fn logins_from_one_browser_are_taken_when_another_starts() {
    let base = std::env::temp_dir().join(format!("dotori-logins-real-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&base);
    let made = |name: &str, logins: Logins| {
        Browser::new(BrowserConfig {
            executable: std::env::var_os("DOTORI_TEST_CHROME").map(PathBuf::from),
            downloads: base.join("chrome"),
            profile: base.join(name).join("profile"),
            logins: Some(logins),
        })
    };
    std::fs::create_dir_all(base.join("a")).expect("folder");
    std::fs::create_dir_all(base.join("b")).expect("folder");
    let a = made(
        "a",
        Logins {
            out: base.join("a").join("logins-out.json"),
            into: base.join("a").join("logins-in.json"),
        },
    );
    {
        let running = a.page().await.expect("a started");
        running
            .cdp
            .send(
                "Storage.setCookies",
                json!({ "cookies": [{
                    "name": "sid", "value": "도토리-login", "domain": "example.com", "path": "/",
                    "secure": true, "httpOnly": true, "expires": 4_102_444_800_i64
                }] }),
                None,
            )
            .await
            .expect("cookie set");
    }
    tokio::spawn(Arc::clone(&a).share_logins(|| true));
    let out = base.join("a").join("logins-out.json");
    for _ in 0..100 {
        if out.exists() {
            break;
        }
        time::sleep(Duration::from_millis(100)).await;
    }
    let copied = read_copy(&out).expect("a wrote its logins");
    assert!(copied.cookies.iter().any(|cookie| cookie["name"] == "sid"));

    let into = base.join("b").join("logins-in.json");
    std::fs::copy(&out, &into).expect("carried");
    let b = made(
        "b",
        Logins {
            out: base.join("b").join("logins-out.json"),
            into: into.clone(),
        },
    );
    let running = b.page().await.expect("b started");
    let cookies = running
        .cdp
        .send("Storage.getCookies", json!({}), None)
        .await
        .expect("cookies");
    let sid = cookies["cookies"]
        .as_array()
        .and_then(|cookies| cookies.iter().find(|cookie| cookie["name"] == "sid"))
        .cloned()
        .expect("b has the login");
    assert_eq!(sid["value"], "도토리-login");
    assert_eq!(sid["httpOnly"], true);
    assert!(!into.exists(), "a copy is taken once");

    running
        .cdp
        .send(
            "Storage.setCookies",
            json!({ "cookies": [{ "name": "stale", "value": "x", "domain": "example.org", "path": "/" }] }),
            None,
        )
        .await
        .expect("stale set");
    let newer = LoginCopy {
        at: copied.at + 1,
        cookies: vec![json!({ "name": "fresh", "value": "y", "domain": "example.net", "path": "/", "partitionKey": "broken" })]
            .into_iter()
            .chain(copied.cookies.clone())
            .collect(),
    };
    write_privately(&into, &newer).expect("carried");
    let logins = Logins {
        out: base.join("b").join("logins-out.json"),
        into: into.clone(),
    };
    take_logins(&running.cdp, &logins).await;
    let names: Vec<String> = running
        .cdp
        .send("Storage.getCookies", json!({}), None)
        .await
        .expect("cookies")["cookies"]
        .as_array()
        .expect("list")
        .iter()
        .filter_map(|cookie| cookie["name"].as_str().map(str::to_owned))
        .collect();
    assert!(names.contains(&String::from("sid")), "{names:?}");
    assert!(
        !names.contains(&String::from("stale")),
        "a login the other PC no longer has is gone: {names:?}"
    );
    write_privately(&into, &newer).expect("carried again");
    take_logins(&running.cdp, &logins).await;
    assert!(into.exists(), "a copy already taken is not taken twice");
}

const FRAMED: &str = "data:text/html,<html><head><title>Framed</title></head><body><h1>Outer</h1>\
<iframe title=Inside srcdoc=\"<p>Read me</p><button>Pay</button>\"></iframe>\
<iframe title=Elsewhere src=\"data:text/html,<p>Secret</p>\"></iframe></body></html>";

#[tokio::test]
#[ignore = "starts a real Chrome; set DOTORI_TEST_CHROME or let it download one"]
async fn a_frame_from_the_same_page_is_read_and_one_from_elsewhere_is_named() {
    let browser = browser();
    let running = browser.page().await.expect("started");
    let page = Page { running: &running };
    page.go(FRAMED).await.expect("opened");
    page.settle().await.expect("settled");

    let shown = page.snapshot().await.expect("read");

    assert!(shown.contains("[frame: Inside]"), "{shown}");
    assert!(shown.contains("Read me"), "{shown}");
    assert!(shown.contains("- button: Pay"), "{shown}");
    assert!(shown.contains("[end of frame]"), "{shown}");
    assert!(shown.contains("[frame: Elsewhere, not read]"), "{shown}");
    let body = shown.split_once("\n\n").expect("a body").1;
    assert!(!body.contains("Secret"), "{body}");
}

#[tokio::test]
#[ignore = "starts a real Chrome and reaches httpbin.org"]
async fn a_page_that_asks_for_a_login_waits_for_the_owners_answer() {
    let browser = browser();
    let running = browser.page().await.expect("started");
    let cdp = Arc::clone(&running.cdp);
    let session = running.session.clone();
    let going = tokio::spawn(async move {
        cdp.send(
            "Page.navigate",
            json!({ "url": "https://httpbin.org/basic-auth/owner/secret" }),
            Some(&session),
        )
        .await
    });
    let mut asked = Value::Null;
    for _ in 0..100 {
        asked = browser.screen.state.lock().expect("unpoisoned")["picker"].clone();
        if asked.get("kind").and_then(Value::as_str) == Some("auth") {
            break;
        }
        time::sleep(Duration::from_millis(100)).await;
    }
    assert_eq!(asked["origin"], "https://httpbin.org", "{asked}");
    let notice = browser.screen.state.lock().expect("unpoisoned")["asked"].clone();
    assert!(
        notice
            .as_str()
            .is_some_and(|said| said.contains("httpbin.org"))
    );

    browser
        .auth(&json!({ "kind": "auth", "username": "owner", "password": "secret" }))
        .await
        .expect("answered");
    going.await.expect("joined").expect("navigated");
    let page = Page { running: &running };
    page.settle().await.expect("settled");

    let body = page
        .evaluate("document.body.innerText")
        .await
        .expect("read");
    assert!(
        body.as_str()
            .is_some_and(|text| text.contains("\"authenticated\": true")),
        "{body}"
    );
    assert!(browser.screen.state.lock().expect("unpoisoned")["picker"].is_null());
}
