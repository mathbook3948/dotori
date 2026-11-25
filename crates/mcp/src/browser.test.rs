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
        accounts: None,
        executable: std::env::var_os("DOTORI_TEST_CHROME").map(PathBuf::from),
        downloads: base.join("chrome"),
        profile: base.join("profile"),
        hidden_env: vec![String::from("CARGO_PKG_NAME")],
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
    let browser = browser();
    let mut tools = browser.tools();
    tools.extend(browser.owner_tools());
    let names: Vec<&str> = tools.iter().map(|tool| tool.name()).collect();
    assert_eq!(
        names,
        [
            "run_in_browser",
            "read_page",
            "ask_owner_in_browser",
            "ask_owner_to_fill",
            "fill_login"
        ]
    );
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
    assert_eq!(
        browser.config.profile.join("DevToolsActivePort").exists(),
        cfg!(windows),
        "only Windows opens a DevTools port another program could find"
    );
    if cfg!(target_os = "linux")
        && let Some(id) = running.child.id()
    {
        let environment = std::fs::read(format!("/proc/{id}/environ")).expect("environment");
        assert!(
            !String::from_utf8_lossy(&environment).contains("CARGO_PKG_NAME="),
            "the browser inherited a hidden variable"
        );
    }
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
    let file = format!(
        "file://{}",
        browser.config.profile.join("Local State").display()
    );
    page.send("Page.navigate", json!({ "url": file }))
        .await
        .expect("navigated");
    page.settle().await.expect("settled");
    let moved = page
        .guard()
        .await
        .expect_err("a file in dotori's folder is refused");
    assert!(moved.contains("refused"), "{moved}");
    let at = page.evaluate("location.href").await.expect("read");
    assert_eq!(at, json!("about:blank"));

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

    let config = browser.config.clone();
    browser.forget().await;
    let second = Browser::new(config.clone());
    drop(
        second
            .page()
            .await
            .expect("the browser of an owner that is gone made way"),
    );
    assert!(!cdp.open(), "the first browser is gone");
    let _ = std::fs::remove_dir_all(&config.profile);
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
    let tools = browser.owner_tools();
    assert!(tools[0].call(json!({ "reason": " " })).await.is_err());

    let said = tools[0]
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
            hidden_env: Vec::new(),
            logins: Some(logins),
            accounts: None,
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
    let login =
        |browser: &Browser| browser.screen.state.lock().expect("unpoisoned")["login"].clone();
    let waited = |browser: Arc<Browser>, after: u64| async move {
        for _ in 0..100 {
            let shown = login(&browser);
            if shown["id"].as_u64().is_some_and(|id| id > after) {
                return shown;
            }
            time::sleep(Duration::from_millis(100)).await;
        }
        panic!("no login asked");
    };
    let asked = waited(Arc::clone(&browser), 0).await;
    assert_eq!(asked["origin"], "https://httpbin.org", "{asked}");
    assert_eq!(asked["again"], false);
    let first = asked["id"].as_u64().expect("numbered");

    browser
        .auth(&json!({ "kind": "auth", "id": first + 100, "username": "owner", "password": "secret" }))
        .await
        .expect("ignored");
    assert_eq!(login(&browser)["id"], first);

    browser
        .auth(&json!({ "kind": "auth", "id": first, "username": "owner", "password": "wrong" }))
        .await
        .expect("answered");
    let again = waited(Arc::clone(&browser), first).await;
    assert_eq!(again["again"], true, "{again}");

    browser
        .auth(&json!({ "kind": "auth", "id": again["id"], "username": "owner", "password": "secret" }))
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
    assert!(login(&browser).is_null());
}

#[tokio::test]
#[ignore = "starts a real Chrome"]
async fn giving_the_browser_back_while_waiting_leaves_it_with_the_agent() {
    let n = NEXT.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    let base = std::env::temp_dir().join(format!("dotori-browser-test-{}-{n}", std::process::id()));
    let (screen, _) = Screen::new();
    let (asks, heard) = mpsc::unbounded_channel();
    let browser = Browser::with_screen(
        BrowserConfig {
            logins: None,
            accounts: None,
            executable: std::env::var_os("DOTORI_TEST_CHROME").map(PathBuf::from),
            downloads: base.join("chrome"),
            profile: base.join("profile"),
            hidden_env: Vec::new(),
        },
        screen,
        heard,
    );
    let mode = |browser: &Browser| browser.screen.state.lock().expect("unpoisoned")["mode"].clone();
    let acting = Arc::clone(&browser.acting).lock_owned().await;

    asks.send(Ask::Take).expect("sent");
    for _ in 0..50 {
        if mode(&browser) == "waiting" {
            break;
        }
        time::sleep(Duration::from_millis(20)).await;
    }
    assert_eq!(mode(&browser), "waiting");
    asks.send(Ask::Give).expect("sent");
    time::sleep(Duration::from_millis(200)).await;
    drop(acting);
    time::sleep(Duration::from_secs(3)).await;

    assert!(browser.owner.lock().await.is_none());
    assert_eq!(mode(&browser), "idle");
    assert!(browser.acting.try_lock().is_ok());

    asks.send(Ask::Take).expect("sent");
    for _ in 0..200 {
        if mode(&browser) == "owner" {
            break;
        }
        time::sleep(Duration::from_millis(50)).await;
    }
    assert_eq!(mode(&browser), "owner");
    assert!(browser.owner.lock().await.is_some());
}

const LOGIN: &str = "<html><head><title>Log in</title></head><body>\
<h1>Shop</h1><p id=said>nothing yet</p>\
<form id=f method=post action=\"/session\" onsubmit=\"event.preventDefault();document.getElementById('said').textContent='got '+document.getElementById('who').value+' '+document.getElementById('word').value.length+' '+document.getElementById('stay').checked\">\
<input type=email id=who aria-label=Email><input type=password id=word aria-label=Password>\
<label><input type=checkbox id=stay> Stay</label>\
<button>Log in</button></form>\
<a href=\"/forgot\" onclick=\"event.preventDefault();document.getElementById('said').textContent='forgot'\">Forgot</a></body></html>";

const ELSEWHERE: &str = "<html><body><form action=\"https://other.test/collect\">\
<input type=email aria-label=Email><input type=password aria-label=Password>\
<button>Log in</button></form></body></html>";

const SPLIT: &str = "<html><body><form><input type=email aria-label=Email></form>\
<form><input type=password aria-label=Password><button>Log in</button></form></body></html>";

const PLAIN: &str = "<html><body><form><input type=email aria-label=Email>\
<input type=text aria-label=Password><button>Log in</button></form></body></html>";

const SHOP_ORIGIN: &str = "https://shop.test";

#[derive(Debug, Default)]
struct KeptHere(std::sync::Mutex<Vec<card::Account>>);

impl Accounts for KeptHere {
    fn find(&self, origin: &str) -> Result<Vec<card::Account>, String> {
        Ok(self
            .0
            .lock()
            .expect("unpoisoned")
            .iter()
            .filter(|account| account.origin == origin)
            .cloned()
            .collect())
    }

    fn keep(&self, account: &card::Account) -> Result<(), String> {
        let mut kept = self.0.lock().expect("unpoisoned");
        kept.retain(|each| each.origin != account.origin || each.id != account.id);
        kept.push(account.clone());
        Ok(())
    }

    fn room(&self, _origin: &str) -> Result<(), String> {
        Ok(())
    }
}

fn account(id: &str, password: &str) -> card::Account {
    card::Account {
        site: String::from("Shop"),
        origin: String::from(SHOP_ORIGIN),
        id_label: Some(String::from("Email")),
        id: Some(id.to_owned()),
        password_label: String::from("Password"),
        password: password.to_owned(),
        trusted: false,
        page: Some(String::from("/login")),
        kept_ms: 0,
    }
}

struct Site {
    port: u16,
    folder: PathBuf,
}

async fn site(pages: &[(&'static str, &'static str)]) -> Site {
    use tokio::io::AsyncWriteExt;
    let n = NEXT.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    let folder = std::env::temp_dir().join(format!("dotori-https-test-{}-{n}", std::process::id()));
    std::fs::create_dir_all(&folder).expect("a folder");
    let made = std::process::Command::new("openssl")
        .args([
            "req", "-x509", "-newkey", "rsa:2048", "-nodes", "-days", "2",
        ])
        .args([
            "-subj",
            "/CN=shop.test",
            "-keyout",
            "key.pem",
            "-out",
            "cert.pem",
        ])
        .current_dir(&folder)
        .output()
        .expect("openssl ran");
    assert!(made.status.success(), "a certificate");
    let identity = native_tls::Identity::from_pkcs8(
        &std::fs::read(folder.join("cert.pem")).expect("cert"),
        &std::fs::read(folder.join("key.pem")).expect("key"),
    )
    .expect("an identity");
    let acceptor = tokio_native_tls::TlsAcceptor::from(
        native_tls::TlsAcceptor::new(identity).expect("an acceptor"),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bound");
    let port = listener.local_addr().expect("address").port();
    let pages: Vec<(String, String)> = pages
        .iter()
        .map(|(path, html)| ((*path).to_owned(), (*html).to_owned()))
        .collect();
    tokio::spawn(async move {
        while let Ok((stream, _)) = listener.accept().await {
            let acceptor = acceptor.clone();
            let pages = pages.clone();
            tokio::spawn(async move {
                let Ok(mut stream) = acceptor.accept(stream).await else {
                    return;
                };
                let mut head = Vec::new();
                let mut byte = [0_u8; 1];
                while !head.ends_with(b"\r\n\r\n") {
                    match tokio::io::AsyncReadExt::read(&mut stream, &mut byte).await {
                        Ok(1) => head.push(byte[0]),
                        _ => return,
                    }
                }
                let head = String::from_utf8_lossy(&head).into_owned();
                let path = head.split_whitespace().nth(1).unwrap_or("/").to_owned();
                let body = pages
                    .iter()
                    .find(|(page, _)| *page == path)
                    .map(|(_, html)| html.clone());
                let reply = match body {
                    Some(body) => format!(
                        "HTTP/1.1 200 OK\r\ncontent-type: text/html; charset=utf-8\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
                        body.len()
                    ),
                    None => String::from(
                        "HTTP/1.1 404 Not Found\r\ncontent-length: 0\r\nconnection: close\r\n\r\n",
                    ),
                };
                let _ = stream.write_all(reply.as_bytes()).await;
                let _ = stream.shutdown().await;
            });
        }
    });
    Site { port, folder }
}

impl Site {
    fn chrome(&self) -> PathBuf {
        let real = std::env::var("DOTORI_TEST_CHROME").expect("DOTORI_TEST_CHROME");
        let wrapper = self.folder.join("chrome.sh");
        std::fs::write(
            &wrapper,
            format!(
                "#!/bin/sh\nexec '{real}' --host-resolver-rules='MAP shop.test 127.0.0.1:{port}, MAP other.test 127.0.0.1:{port}' --ignore-certificate-errors --no-proxy-server \"$@\"\n",
                port = self.port
            ),
        )
        .expect("a wrapper");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&wrapper, std::fs::Permissions::from_mode(0o755))
                .expect("runnable");
        }
        wrapper
    }
}

struct Watched {
    browser: Arc<Browser>,
    lines: BufReader<tokio::net::tcp::OwnedReadHalf>,
    write: tokio::net::tcp::OwnedWriteHalf,
    _site: Site,
}

async fn watched(
    pages: &[(&'static str, &'static str)],
    accounts: Option<Arc<dyn Accounts>>,
) -> Watched {
    use tokio::io::AsyncWriteExt;
    let site = site(pages).await;
    let base = site.folder.clone();
    let (screen, asks) = Screen::new();
    let browser = Browser::with_screen(
        BrowserConfig {
            logins: None,
            accounts,
            hidden_env: Vec::new(),
            executable: Some(site.chrome()),
            downloads: base.join("chrome"),
            profile: base.join("profile"),
        },
        Arc::clone(&screen),
        asks,
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bound");
    let port = listener.local_addr().expect("address").port();
    tokio::spawn(screen.serve(listener, String::from("token")));
    let stream = tokio::net::TcpStream::connect(("127.0.0.1", port))
        .await
        .expect("connected");
    let (read, mut write) = stream.into_split();
    write
        .write_all(b"{\"token\":\"token\"}\n")
        .await
        .expect("sent");
    let mut lines = BufReader::new(read);
    let mut first = String::new();
    lines.read_line(&mut first).await.expect("the state");
    for _ in 0..50 {
        if browser.screen.watched() {
            break;
        }
        time::sleep(Duration::from_millis(20)).await;
    }
    Watched {
        browser,
        lines,
        write,
        _site: site,
    }
}

impl Watched {
    async fn card(&mut self) -> Value {
        loop {
            let mut line = String::new();
            time::timeout(Duration::from_secs(30), self.lines.read_line(&mut line))
                .await
                .expect("a card in time")
                .expect("read");
            let state: Value = serde_json::from_str(line.trim()).unwrap_or(Value::Null);
            if state["kind"] == "state" && state["card"].is_object() {
                return state["card"].clone();
            }
        }
    }

    async fn answer(&mut self, answer: Value) {
        use tokio::io::AsyncWriteExt;
        self.write
            .write_all(format!("{answer}\n").as_bytes())
            .await
            .expect("answered");
    }

    async fn open(&self, url: &str) {
        let running = self.browser.page().await.expect("started");
        Page { running: &running }.go(url).await.expect("opened");
    }

    async fn read(&self) -> String {
        self.call(READ, json!({}))
            .await
            .expect("joined")
            .expect("read")
    }

    async fn in_page(&self, expression: &str) -> Value {
        let running = self.browser.page().await.expect("running");
        in_page(&running, expression).await
    }

    fn call(
        &self,
        tool: &'static str,
        input: Value,
    ) -> tokio::task::JoinHandle<Result<String, String>> {
        let mut tools = self.browser.tools();
        tools.extend(self.browser.owner_tools());
        tokio::spawn(async move {
            let tool = tools
                .iter()
                .find(|each| each.name() == tool)
                .expect("a tool");
            tool.call(input).await
        })
    }
}

fn login_card(keep: bool) -> Value {
    json!({
        "title": "Shop",
        "site": "Shop",
        "fields": [
            { "ref": 1, "label": "Email", "kind": "text" },
            { "ref": 2, "label": "Password", "kind": "secret" }
        ],
        "checks": [{ "ref": 3, "label": "Stay" }],
        "submit": { "ref": 4, "label": "Log in" },
        "links": [{ "ref": 5, "label": "Forgot" }],
        "keep": keep
    })
}

fn bare_card() -> Value {
    json!({
        "title": "Shop",
        "fields": [
            { "ref": 1, "label": "Email", "kind": "text" },
            { "ref": 2, "label": "Password", "kind": "secret" }
        ],
        "submit": { "ref": 3, "label": "Log in" }
    })
}

#[test]
fn the_tools_that_reach_the_owner_are_kept_apart_from_the_shared_ones() {
    let browser = browser();
    let shared: Vec<String> = browser
        .tools()
        .iter()
        .map(|tool| tool.name().to_owned())
        .collect();
    let owner: Vec<String> = browser
        .owner_tools()
        .iter()
        .map(|tool| tool.name().to_owned())
        .collect();

    assert_eq!(shared, ["run_in_browser", "read_page"]);
    assert_eq!(owner, OWNER_TOOLS);
}

#[tokio::test]
async fn a_card_with_no_window_to_show_it_is_refused_before_the_browser_starts() {
    let tools = browser().owner_tools();
    let refused = tools[1].call(login_card(false)).await.expect_err("refused");

    assert!(refused.contains("No dotori chat window"), "{refused}");
}

#[tokio::test]
#[ignore = "starts a real Chrome; set DOTORI_TEST_CHROME"]
async fn the_owner_fills_a_card_and_the_model_never_sees_what_they_typed() {
    let mut window = watched(&[("/login", LOGIN)], None).await;
    window.open("https://shop.test/login").await;
    let read = window.read().await;
    assert!(read.contains("[1] input email"), "{read}");
    assert!(read.contains("[4] button: Log in"), "{read}");

    let filling = window.call(FILL, login_card(false));
    let card = window.card().await;
    assert_eq!(card["kind"], "fill");
    assert_eq!(card["host"], "shop.test");
    window
        .answer(json!({
            "kind": "fill", "id": card["id"], "press": 4,
            "values": [{ "ref": 1, "value": "me@example.com" }, { "ref": 2, "value": "hunter22!" }],
            "checks": [{ "ref": 3, "on": true }]
        }))
        .await;
    let told = filling.await.expect("joined").expect("filled");

    assert_eq!(
        window
            .in_page("document.getElementById('said').textContent")
            .await,
        "got me@example.com 9 true"
    );
    assert_eq!(
        window
            .in_page("document.getElementById('word').value")
            .await,
        "",
        "what dotori typed is cleared once the page took it"
    );
    assert!(told.contains("주인이 'Log in'을(를) 눌렀어요."), "{told}");
    assert!(told.contains("채운 칸: Email, Password"), "{told}");
    assert!(!told.contains("hunter22"), "{told}");
    let again = window.read().await;
    assert!(!again.contains("hunter22"), "{again}");
    assert!(window.browser.screen.state.lock().expect("unpoisoned")["card"].is_null());
}

#[tokio::test]
#[ignore = "starts a real Chrome; set DOTORI_TEST_CHROME"]
async fn a_link_on_the_card_is_clicked_and_nothing_is_filled() {
    let mut window = watched(&[("/login", LOGIN)], None).await;
    window.open("https://shop.test/login").await;
    window.read().await;

    let filling = window.call(FILL, login_card(false));
    let card = window.card().await;
    window
        .answer(json!({
            "kind": "fill", "id": card["id"], "press": 5,
            "values": [{ "ref": 1, "value": "me@example.com" }]
        }))
        .await;
    let told = filling.await.expect("joined").expect("pressed");

    assert_eq!(
        window
            .in_page("document.getElementById('said').textContent")
            .await,
        "forgot"
    );
    assert_eq!(
        window.in_page("document.getElementById('who').value").await,
        ""
    );
    assert!(told.contains("주인이 'Forgot'을(를) 눌렀어요."), "{told}");
}

#[tokio::test]
#[ignore = "starts a real Chrome; set DOTORI_TEST_CHROME"]
async fn a_card_on_elements_that_are_missing_or_of_the_wrong_kind_is_refused() {
    let window = watched(&[("/login", LOGIN)], None).await;
    window.open("https://shop.test/login").await;
    window.read().await;
    let refused = |change: fn(&mut Value)| {
        let mut asked = login_card(false);
        change(&mut asked);
        window.call(FILL, asked)
    };

    let button = refused(|asked| {
        asked["fields"][0]["ref"] = json!(4);
        asked["submit"]["ref"] = json!(5);
        asked["links"] = json!([]);
    })
    .await
    .expect("joined")
    .expect_err("refused");
    let missing = refused(|asked| asked["fields"][0]["ref"] = json!(99))
        .await
        .expect("joined")
        .expect_err("refused");
    let password = refused(|asked| {
        asked["fields"][0]["ref"] = json!(2);
        asked["fields"][1]["ref"] = json!(1);
    })
    .await
    .expect("joined")
    .expect_err("refused");
    let unchecked = refused(|asked| {
        asked["checks"][0]["ref"] = json!(4);
        asked["submit"]["ref"] = json!(5);
        asked["links"] = json!([]);
    })
    .await
    .expect("joined")
    .expect_err("refused");

    assert!(
        button.contains("[4] is button, not a text field"),
        "{button}"
    );
    assert!(missing.contains("[99] is not on the page"), "{missing}");
    assert!(password.contains("[2] is a password field"), "{password}");
    assert!(unchecked.contains("not a checkbox"), "{unchecked}");
    assert!(window.browser.screen.state.lock().expect("unpoisoned")["card"].is_null());
}

#[tokio::test]
#[ignore = "starts a real Chrome; set DOTORI_TEST_CHROME"]
async fn a_secret_that_is_not_a_password_field_is_refused() {
    let window = watched(&[("/plain", PLAIN)], None).await;
    window.open("https://shop.test/plain").await;
    window.read().await;

    let refused = window
        .call(FILL, bare_card())
        .await
        .expect("joined")
        .expect_err("refused");

    assert!(
        refused.contains("[2] is input text, not a password field"),
        "{refused}"
    );
}

#[tokio::test]
#[ignore = "starts a real Chrome; set DOTORI_TEST_CHROME"]
async fn fields_in_two_forms_or_a_form_that_sends_elsewhere_are_refused() {
    let window = watched(&[("/split", SPLIT), ("/elsewhere", ELSEWHERE)], None).await;
    window.open("https://shop.test/split").await;
    window.read().await;
    let split = window
        .call(FILL, bare_card())
        .await
        .expect("joined")
        .expect_err("refused");
    window.open("https://shop.test/elsewhere").await;
    window.read().await;
    let elsewhere = window
        .call(FILL, bare_card())
        .await
        .expect("joined")
        .expect_err("refused");

    assert!(split.contains("not in one form"), "{split}");
    assert!(elsewhere.contains("another site"), "{elsewhere}");
}

#[tokio::test]
#[ignore = "starts a real Chrome; set DOTORI_TEST_CHROME"]
async fn a_page_not_sent_over_https_gets_no_card() {
    let window = watched(&[], None).await;
    window
        .open(&format!("data:text/html,{}", LOGIN.replace('#', "")))
        .await;
    window.read().await;

    let refused = window
        .call(FILL, login_card(false))
        .await
        .expect("joined")
        .expect_err("refused");

    assert!(refused.contains("only on https pages"), "{refused}");
}

#[tokio::test]
#[ignore = "starts a real Chrome; set DOTORI_TEST_CHROME"]
async fn a_page_the_model_ran_code_on_gets_no_card_until_it_loads_again() {
    let window = watched(&[("/login", LOGIN)], None).await;
    window.open("https://shop.test/login").await;
    window.read().await;
    window
        .call(
            RUN,
            json!({ "code": "await page.evaluate(() => document.title);" }),
        )
        .await
        .expect("joined")
        .expect("ran");
    window.read().await;

    let refused = window
        .call(FILL, login_card(false))
        .await
        .expect("joined")
        .expect_err("refused");
    let moved = window
        .call(
            RUN,
            json!({ "code": "await page.goto('https://shop.test/login');" }),
        )
        .await
        .expect("joined")
        .expect("moved");
    let still = window
        .call(FILL, login_card(false))
        .await
        .expect("joined")
        .expect_err("refused");
    window
        .call(READ, json!({ "url": "https://shop.test/login" }))
        .await
        .expect("joined")
        .expect("loaded again");
    let shown = window.call(FILL, login_card(false));
    let mut window = window;
    let card = window.card().await;

    assert!(refused.contains("code ran on this page"), "{refused}");
    assert!(moved.contains("[4] button: Log in"), "{moved}");
    assert!(
        still.contains("code ran on this page"),
        "a page the model's code opened is touched too: {still}"
    );
    assert_eq!(card["kind"], "fill");
    window
        .answer(json!({ "kind": "fill", "id": card["id"], "later": true }))
        .await;
    shown.await.expect("joined").expect("closed");
}

#[tokio::test]
#[ignore = "starts a real Chrome; set DOTORI_TEST_CHROME"]
async fn no_code_runs_while_a_card_waits_or_on_a_page_dotori_typed_into() {
    let mut window = watched(&[("/login", LOGIN)], None).await;
    window.open("https://shop.test/login").await;
    window.read().await;
    let shown = window.call(FILL, login_card(false));
    let card = window.card().await;

    let waiting = window
        .call(RUN, json!({ "code": "return 1;" }))
        .await
        .expect("joined")
        .expect_err("refused");
    window
        .answer(json!({ "kind": "fill", "id": card["id"], "later": true }))
        .await;
    shown.await.expect("joined").expect("closed");
    {
        let running = window.browser.page().await.expect("running");
        let page = Page { running: &running };
        let (_, loader) = page.document().await.expect("a document");
        running
            .marks
            .lock()
            .expect("unpoisoned")
            .sealed
            .insert(loader);
    }
    let sealed = window
        .call(
            RUN,
            json!({ "code": "return await page.locator('#word').inputValue();" }),
        )
        .await
        .expect("joined")
        .expect_err("refused");

    assert!(waiting.contains("a card waits for the owner"), "{waiting}");
    assert!(sealed.contains("no code runs on it"), "{sealed}");
}

#[tokio::test]
#[ignore = "starts a real Chrome; set DOTORI_TEST_CHROME"]
async fn a_page_that_changed_since_the_card_was_shown_is_not_filled() {
    let mut window = watched(&[("/login", LOGIN), ("/other", LOGIN)], None).await;
    window.open("https://shop.test/login").await;
    window.read().await;

    let filling = window.call(FILL, login_card(false));
    let card = window.card().await;
    window.open("https://shop.test/other").await;
    window
        .answer(json!({
            "kind": "fill", "id": card["id"], "press": 4,
            "values": [{ "ref": 1, "value": "me@example.com" }, { "ref": 2, "value": "hunter22!" }]
        }))
        .await;
    let refused = filling.await.expect("joined").expect_err("refused");

    assert!(
        refused.contains("changed since the card was shown"),
        "{refused}"
    );
    assert_eq!(
        window.in_page("document.getElementById('who').value").await,
        ""
    );
}

#[tokio::test]
#[ignore = "starts a real Chrome; set DOTORI_TEST_CHROME"]
async fn a_kept_account_is_never_filled_into_a_page_of_another_origin() {
    let accounts = Arc::new(KeptHere::default());
    accounts
        .keep(&card::Account {
            origin: String::from("https://other.test"),
            trusted: true,
            ..account("me@example.com", "hunter22!")
        })
        .expect("kept");
    let window = watched(&[("/login", LOGIN)], Some(accounts)).await;
    window.open("https://shop.test/login").await;
    window.read().await;

    let refused = window
        .call(FILL_LOGIN, json!({ "id": 1, "password": 2, "submit": 4 }))
        .await
        .expect("joined")
        .expect_err("refused");

    assert!(
        refused.contains("keeps no account for https://shop.test"),
        "{refused}"
    );
    assert_eq!(
        window
            .in_page("document.getElementById('word').value")
            .await,
        ""
    );
}

#[tokio::test]
#[ignore = "starts a real Chrome; set DOTORI_TEST_CHROME"]
async fn the_owner_picks_one_of_several_accounts_and_trust_moves_to_it() {
    let accounts = Arc::new(KeptHere::default());
    accounts
        .keep(&account("me@example.com", "first-pass"))
        .expect("kept");
    accounts
        .keep(&account("family@example.com", "second-pass"))
        .expect("kept");
    let mut window = watched(
        &[("/login", LOGIN)],
        Some(Arc::clone(&accounts) as Arc<dyn Accounts>),
    )
    .await;
    window.open("https://shop.test/login").await;
    window.read().await;

    let logging = window.call(FILL_LOGIN, json!({ "id": 1, "password": 2, "submit": 4 }));
    let card = window.card().await;
    assert_eq!(card["kind"], "log_in");
    assert_eq!(card["accounts"], json!(["me", "family"]));
    assert_eq!(card["ids"], json!(["me@example.com", "family@example.com"]));
    accounts
        .keep(&card::Account {
            trusted: true,
            ..account("me@example.com", "first-pass")
        })
        .expect("trusted elsewhere");
    window
        .answer(
            json!({ "kind": "fill", "id": card["id"], "allow": true, "trust": true, "account": 1 }),
        )
        .await;
    let told = logging.await.expect("joined").expect("logged in");

    assert_eq!(
        window
            .in_page("document.getElementById('said').textContent")
            .await,
        "got family@example.com 11 false"
    );
    assert!(
        told.contains("family@example.com") && !told.contains("second-pass"),
        "{told}"
    );
    let trusted: Vec<Option<String>> = accounts
        .find(SHOP_ORIGIN)
        .expect("read")
        .into_iter()
        .filter(|account| account.trusted)
        .map(|account| account.id)
        .collect();
    assert_eq!(trusted, [Some(String::from("family@example.com"))]);

    window.open("https://shop.test/login").await;
    window.read().await;
    window
        .call(FILL_LOGIN, json!({ "id": 1, "password": 2, "submit": 4 }))
        .await
        .expect("joined")
        .expect("logged in without asking");
    assert_eq!(
        window
            .in_page("document.getElementById('said').textContent")
            .await,
        "got family@example.com 11 false"
    );

    window
        .call(READ, json!({ "url": "https://shop.test/login" }))
        .await
        .expect("joined")
        .expect("read");
    let named = window.call(
        FILL_LOGIN,
        json!({ "id": 1, "password": 2, "submit": 4, "account": "me@example.com" }),
    );
    let asked = window.card().await;
    window
        .answer(json!({ "kind": "fill", "id": asked["id"], "allow": false }))
        .await;
    named.await.expect("joined").expect("declined");
    assert_eq!(
        asked["accounts"],
        json!(["me"]),
        "the account the model named, which is not trusted, is asked about alone"
    );
    let textarea = window
        .call(FILL_LOGIN, json!({ "id": 2, "password": 2, "submit": 4 }))
        .await
        .expect("joined")
        .expect_err("refused");
    assert!(textarea.contains("named once"), "{textarea}");
}

const TEXTAREA_ID: &str = "<html><body><form method=post action=\"/session\">\
<textarea aria-label=Email></textarea><input type=password aria-label=Password>\
<button>Log in</button></form></body></html>";

#[tokio::test]
#[ignore = "starts a real Chrome; set DOTORI_TEST_CHROME"]
async fn a_login_whose_id_field_is_not_a_text_input_is_refused() {
    let accounts = Arc::new(KeptHere::default());
    accounts
        .keep(&account("me@example.com", "first-pass"))
        .expect("kept");
    let window = watched(
        &[("/login", TEXTAREA_ID)],
        Some(Arc::clone(&accounts) as Arc<dyn Accounts>),
    )
    .await;
    window.open("https://shop.test/login").await;
    window.read().await;

    let refused = window
        .call(FILL_LOGIN, json!({ "id": 1, "password": 2, "submit": 3 }))
        .await
        .expect("joined")
        .expect_err("refused");

    assert!(refused.contains("not a text input"), "{refused}");
}

#[tokio::test]
#[ignore = "starts a real Chrome; set DOTORI_TEST_CHROME"]
async fn an_account_that_changed_while_the_owner_was_asked_is_not_filled() {
    let accounts = Arc::new(KeptHere::default());
    accounts
        .keep(&account("me@example.com", "first-pass"))
        .expect("kept");
    let mut window = watched(
        &[("/login", LOGIN)],
        Some(Arc::clone(&accounts) as Arc<dyn Accounts>),
    )
    .await;
    window.open("https://shop.test/login").await;
    window.read().await;

    let logging = window.call(FILL_LOGIN, json!({ "id": 1, "password": 2, "submit": 4 }));
    let card = window.card().await;
    accounts
        .keep(&account("me@example.com", "changed-pass"))
        .expect("changed elsewhere");
    window
        .answer(json!({ "kind": "fill", "id": card["id"], "allow": true, "account": 0 }))
        .await;
    let refused = logging.await.expect("joined").expect_err("refused");

    assert!(refused.contains("changed or was unlinked"), "{refused}");
    assert_eq!(
        window
            .in_page("document.getElementById('word').value")
            .await,
        ""
    );
}

const GET_FORM: &str = "<html><body><form method=get action=\"/session\">\
<input type=email aria-label=Email><input type=password aria-label=Password>\
<button>Log in</button></form></body></html>";

const CLOBBERED: &str = "<html><body><form method=post action=\"https://other.test/collect\">\
<input name=action type=hidden value=\"/session\"><input name=method type=hidden value=post>\
<input type=email aria-label=Email><input type=password aria-label=Password>\
<button>Log in</button></form></body></html>";

const FORMACTION: &str = "<html><body><form method=post action=\"/session\">\
<input type=email aria-label=Email><input type=password aria-label=Password>\
<button formaction=\"https://other.test/collect\">Log in</button></form></body></html>";

const COPYING: &str = "<html><body><form id=f method=post action=\"/session\" onsubmit=\"event.preventDefault();\
setTimeout(() => { document.getElementById('mirror').value = 'MIRROR' + document.getElementById('saved').textContent; }, 3000)\">\
<input type=email id=who aria-label=Email><input type=password id=word aria-label=Password>\
<input type=text id=mirror aria-label=Mirror><button>Log in</button></form>\
<input type=text id=copy aria-label=Copy><p id=saved hidden></p>\
<script>document.getElementById('word').addEventListener('beforeinput', (e) => { \
const shout = (e.data || '').toUpperCase(); document.getElementById('copy').value = shout; \
document.getElementById('saved').textContent = shout; });</script></body></html>";

#[tokio::test]
#[ignore = "starts a real Chrome; set DOTORI_TEST_CHROME"]
async fn a_page_the_model_s_code_moved_to_counts_as_touched_and_is_never_run_again() {
    let window = watched(&[("/login", LOGIN), ("/two", LOGIN)], None).await;
    window.open("https://shop.test/login").await;
    window.read().await;

    let moved = window
        .call(
            RUN,
            json!({ "code": "await page.evaluate(() => { location.href = 'https://shop.test/two'; return new Promise(() => {}); });" }),
        )
        .await
        .expect("joined");
    window.read().await;
    let refused = window
        .call(FILL, login_card(false))
        .await
        .expect("joined")
        .expect_err("refused");

    assert!(
        moved.is_err(),
        "the code is not run again in the new page: {moved:?}"
    );
    assert!(
        window
            .in_page("location.pathname")
            .await
            .as_str()
            .is_some_and(|path| path == "/two"),
        "the page moved"
    );
    assert!(refused.contains("code ran on this page"), "{refused}");
}

#[tokio::test]
#[ignore = "starts a real Chrome; set DOTORI_TEST_CHROME"]
async fn another_window_refuses_a_card_until_read_page_reloads_and_one_opened_during_it_stops_the_fill()
 {
    let mut window = watched(&[("/login", LOGIN)], None).await;
    window.open("https://shop.test/login").await;
    window.read().await;
    let open_another = |browser: Arc<Browser>| async move {
        let running = browser.page().await.expect("running");
        running
            .cdp
            .send("Target.createTarget", json!({ "url": "about:blank" }), None)
            .await
            .expect("another window");
    };
    let pages = |browser: Arc<Browser>| async move {
        let running = browser.page().await.expect("running");
        other_pages(&Page { running: &running })
            .await
            .expect("pages")
            .len()
    };
    open_another(Arc::clone(&window.browser)).await;
    assert_eq!(pages(Arc::clone(&window.browser)).await, 1);

    let refused = window
        .call(FILL, login_card(false))
        .await
        .expect("joined")
        .expect_err("refused");
    assert!(refused.contains("code ran on this page"), "{refused}");
    assert_eq!(pages(Arc::clone(&window.browser)).await, 0, "closed");
    open_another(Arc::clone(&window.browser)).await;
    window
        .call(READ, json!({ "url": "https://shop.test/login" }))
        .await
        .expect("joined")
        .expect("loaded again");
    assert_eq!(
        pages(Arc::clone(&window.browser)).await,
        0,
        "read_page with a url closes the other windows first"
    );
    let filling = window.call(FILL, login_card(false));
    let card = window.card().await;
    open_another(Arc::clone(&window.browser)).await;
    window
        .answer(json!({
            "kind": "fill", "id": card["id"], "press": 4,
            "values": [{ "ref": 1, "value": "me@example.com" }, { "ref": 2, "value": "hunter22!" }]
        }))
        .await;
    let stopped = filling.await.expect("joined").expect_err("refused");

    assert!(
        stopped.contains("code ran on this page") || stopped.contains("another window"),
        "a window that opened during the card stops the fill: {stopped}"
    );
    assert_eq!(
        window
            .in_page("document.getElementById('word').value")
            .await,
        ""
    );
}

#[tokio::test]
#[ignore = "starts a real Chrome; set DOTORI_TEST_CHROME"]
async fn a_result_holding_a_kept_password_is_withheld_and_the_site_closes() {
    let accounts = Arc::new(KeptHere::default());
    accounts
        .keep(&account("me@example.com", "first-pass"))
        .expect("kept");
    let window = watched(
        &[("/login", LOGIN)],
        Some(Arc::clone(&accounts) as Arc<dyn Accounts>),
    )
    .await;
    window.open("https://shop.test/login").await;
    let read = window.read().await;

    let guessed = window
        .call(RUN, json!({ "code": "return 'first' + '-pass';" }))
        .await
        .expect("joined")
        .expect_err("withheld");
    let again = window
        .call(READ, json!({}))
        .await
        .expect("joined")
        .expect_err("closed");
    let id = window
        .call(RUN, json!({ "code": "return 'me@example.com';" }))
        .await
        .expect("joined");

    assert!(read.contains("[1] input email"), "{read}");
    assert!(guessed.contains("withheld"), "{guessed}");
    assert!(!guessed.contains("first-pass"), "{guessed}");
    assert!(again.contains("until it starts again"), "{again}");
    assert!(id.is_err(), "the site stays closed: {id:?}");
}

#[tokio::test]
#[ignore = "starts a real Chrome; set DOTORI_TEST_CHROME"]
async fn a_form_that_sends_by_get_or_elsewhere_however_it_is_named_is_refused() {
    let window = watched(
        &[
            ("/get", GET_FORM),
            ("/clobbered", CLOBBERED),
            ("/formaction", FORMACTION),
        ],
        None,
    )
    .await;
    let mut refused = Vec::new();
    for path in ["/get", "/clobbered", "/formaction"] {
        window.open(&format!("https://shop.test{path}")).await;
        let read = window.read().await;
        let button = read
            .lines()
            .find(|line| line.contains("button: Log in"))
            .and_then(|line| line.strip_prefix('['))
            .and_then(|line| line.split(']').next())
            .and_then(|at| at.parse::<u64>().ok())
            .expect("the button");
        let mut asked = bare_card();
        asked["fields"][0]["ref"] = json!(button - 2);
        asked["fields"][1]["ref"] = json!(button - 1);
        asked["submit"]["ref"] = json!(button);
        refused.push(
            window
                .call(FILL, asked)
                .await
                .expect("joined")
                .expect_err("refused"),
        );
    }

    assert!(refused[0].contains("in the address"), "{}", refused[0]);
    assert!(refused[1].contains("another site"), "{}", refused[1]);
    assert!(refused[2].contains("another site"), "{}", refused[2]);
}

#[tokio::test]
#[ignore = "starts a real Chrome; set DOTORI_TEST_CHROME"]
async fn a_page_that_copies_what_was_typed_shows_none_of_it_and_stays_closed_to_code() {
    let mut window = watched(&[("/copy", COPYING)], None).await;
    window.open("https://shop.test/copy").await;
    window.read().await;

    let mut asked = bare_card();
    asked["submit"]["ref"] = json!(4);
    let filling = window.call(FILL, asked);
    let card = window.card().await;
    window
        .answer(json!({
            "kind": "fill", "id": card["id"], "press": 4,
            "values": [{ "ref": 1, "value": "me@example.com" }, { "ref": 2, "value": "hunter22!" }]
        }))
        .await;
    let told = filling.await.expect("joined").expect("filled");
    time::sleep(Duration::from_secs(2)).await;
    let read = window.read().await;
    let refused = window
        .call(
            RUN,
            json!({ "code": "return await page.locator('#mirror').inputValue();" }),
        )
        .await
        .expect("joined")
        .expect_err("refused");
    let went = window
        .call(
            RUN,
            json!({ "code": "await page.goto('https://shop.test/copy');" }),
        )
        .await
        .expect("joined");

    assert!(!told.contains("HUNTER22"), "{told}");
    assert!(
        !read.contains("HUNTER22"),
        "a field of the same form is masked: {read}"
    );
    assert!(told.contains("loads again"), "{told}");
    assert!(refused.contains("no code runs on it"), "{refused}");
    assert!(went.is_ok(), "page.goto still works: {went:?}");
}

#[tokio::test]
#[ignore = "starts a real Chrome; set DOTORI_TEST_CHROME"]
async fn a_copy_outside_the_form_is_cleared_with_what_was_typed() {
    let mut window = watched(&[("/copy", COPYING)], None).await;
    window.open("https://shop.test/copy").await;
    window.read().await;

    let mut asked = bare_card();
    asked["submit"]["ref"] = json!(4);
    let filling = window.call(FILL, asked);
    let card = window.card().await;
    window
        .answer(json!({
            "kind": "fill", "id": card["id"], "press": 4,
            "values": [{ "ref": 1, "value": "me@example.com" }, { "ref": 2, "value": "hunter22!" }]
        }))
        .await;
    filling.await.expect("joined").expect("filled");

    assert_eq!(
        window
            .in_page("document.getElementById('saved').textContent")
            .await,
        "HUNTER22!",
        "the page copied what was typed"
    );
    assert_eq!(
        window
            .in_page("document.getElementById('copy').value")
            .await,
        "",
        "a field the fill changed is cleared with the typed ones"
    );
}

#[tokio::test]
#[ignore = "starts a real Chrome; set DOTORI_TEST_CHROME"]
async fn a_trusted_account_asks_on_another_page_and_when_two_are_trusted() {
    let accounts = Arc::new(KeptHere::default());
    accounts
        .keep(&card::Account {
            trusted: true,
            ..account("me@example.com", "first-pass")
        })
        .expect("kept");
    let mut window = watched(
        &[("/login", LOGIN), ("/other", LOGIN)],
        Some(Arc::clone(&accounts) as Arc<dyn Accounts>),
    )
    .await;
    window.open("https://shop.test/other").await;
    window.read().await;

    let elsewhere = window.call(FILL_LOGIN, json!({ "id": 1, "password": 2, "submit": 4 }));
    let card = window.card().await;
    window
        .answer(json!({ "kind": "fill", "id": card["id"], "allow": false }))
        .await;
    let declined = elsewhere.await.expect("joined").expect("declined");
    accounts
        .keep(&card::Account {
            trusted: true,
            ..account("family@example.com", "second-pass")
        })
        .expect("kept");
    window
        .call(READ, json!({ "url": "https://shop.test/login" }))
        .await
        .expect("joined")
        .expect("read");
    let both = window.call(FILL_LOGIN, json!({ "id": 1, "password": 2, "submit": 4 }));
    let asked = window.card().await;
    window
        .answer(json!({ "kind": "fill", "id": asked["id"], "allow": false }))
        .await;
    both.await.expect("joined").expect("declined");

    assert_eq!(
        card["kind"], "log_in",
        "a trusted account asks off its own page"
    );
    assert!(declined.contains("거절"), "{declined}");
    assert_eq!(
        asked["accounts"],
        json!(["me", "family"]),
        "two trusted accounts ask"
    );
}
