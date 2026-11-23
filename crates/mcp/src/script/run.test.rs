use std::sync::Mutex;

use super::*;
use crate::script::parse::parse;

#[derive(Default)]
struct FakePage {
    calls: Mutex<Vec<String>>,
}

impl Host for FakePage {
    fn call<'a>(
        &'a self,
        this: &'a Native,
        method: &'a str,
        args: Vec<Value>,
    ) -> BoxFuture<'a, Result<Value, String>> {
        Box::pin(async move {
            let shown_args: Vec<String> = args.iter().map(shown).collect();
            self.calls.lock().expect("unpoisoned").push(format!(
                "{}.{method}({})",
                this.kind,
                shown_args.join(", ")
            ));
            match (this.kind, method) {
                ("page", "title") => Ok(text("도토리")),
                ("page", "locator") => Ok(Value::Native(Native {
                    kind: "locator",
                    data: json!(string(&args[0])),
                })),
                ("locator", "textContent") => Ok(text(&format!("text of {}", this.data))),
                ("locator", "count") => Ok(Value::Number(2.0)),
                ("page", "fail") => Err(String::from("no such element")),
                ("page", "evaluate") => Ok(text(&function_source(&args[0]).unwrap_or_default())),
                _ => Ok(Value::Undefined),
            }
        })
    }

    fn property(&self, this: &Native, name: &str) -> Option<Value> {
        (this.kind == "page" && name == "keyboard").then(|| {
            Value::Native(Native {
                kind: "keyboard",
                data: Json::Null,
            })
        })
    }
}

async fn outcome(source: &str) -> (Result<Outcome, String>, Vec<String>) {
    let host = FakePage::default();
    let body = parse(source).expect("parsed");
    let page = Native {
        kind: "page",
        data: Json::Null,
    };
    let done = run(&body, &host, page).await;
    (done, host.calls.into_inner().expect("unpoisoned"))
}

async fn value(source: &str) -> String {
    outcome(source)
        .await
        .0
        .expect("ran")
        .value
        .unwrap_or_default()
}

#[tokio::test]
async fn page_calls_reach_the_host_in_order() {
    let (done, calls) = outcome(
        "await page.goto('https://example.com');\n\
         const heading = page.locator('h1');\n\
         await page.keyboard.press('Enter');\n\
         return `${await page.title()}: ${await heading.textContent()}`;",
    )
    .await;

    assert_eq!(
        done.expect("ran").value.as_deref(),
        Some("도토리: text of \"h1\"")
    );
    assert_eq!(
        calls,
        vec![
            "page.goto(https://example.com)",
            "page.locator(h1)",
            "keyboard.press(Enter)",
            "page.title()",
            "locator.textContent()"
        ]
    );
}

#[tokio::test]
async fn ordinary_javascript_works() {
    assert_eq!(
        value("const xs = [3, 1, 2]; return xs.map(x => x * 2).filter(x => x > 2).join('-');")
            .await,
        "6-4"
    );
    assert_eq!(
        value("let total = 0; for (let i = 1; i <= 4; i++) { if (i === 3) continue; total += i; } return total;").await,
        "7"
    );
    assert_eq!(
        value("const { a, b: renamed } = { a: 1, b: 9 }; return a + renamed;").await,
        "10"
    );
    assert_eq!(
        value("const o = { name: '민지', tags: ['x'] }; o.age = 3; return JSON.stringify(o);")
            .await,
        r#"{"name":"민지","tags":["x"],"age":3}"#
    );
    assert_eq!(
        value("return [1, 2, 3].reduce((s, x) => s + x, 0) / 4;").await,
        "1.5"
    );
    assert_eq!(
        value("return ' A b '.trim().toLowerCase().split(' ');").await,
        r#"["a","b"]"#
    );
    assert_eq!(
        value("function twice(x) { return x * 2; } return twice(21);").await,
        "42"
    );
    assert_eq!(
        value("return (typeof undefined === 'undefined' && null) ?? 'x';").await,
        "x"
    );
    assert_eq!(
        value("return [5, 10, 1].sort((a, b) => a - b);").await,
        "[1,5,10]"
    );
}

#[tokio::test]
async fn logs_are_kept_and_errors_can_be_caught() {
    let (done, _) = outcome(
        "console.log('start', { n: 1 });\n\
         try { await page.fail(); } catch (e) { console.log('caught', e.message); }\n\
         return 'done';",
    )
    .await;
    let done = done.expect("ran");
    assert_eq!(
        done.logs,
        vec![r#"start {"n":1}"#, "caught no such element"]
    );
    assert_eq!(done.value.as_deref(), Some("done"));
}

#[tokio::test]
async fn an_uncaught_error_names_itself_after_the_logs() {
    let (done, _) =
        outcome("console.log('before'); await page.fail(); console.log('after');").await;
    let error = done.expect_err("thrown");
    assert_eq!(error, "before\n\nUncaught Error: no such element");
    let (done, _) = outcome("throw new Error('stop');").await;
    assert_eq!(done.expect_err("thrown"), "Uncaught Error: stop");
    let (done, _) = outcome("return missing + 1;").await;
    assert!(done.expect_err("thrown").contains("missing is not defined"));
}

#[tokio::test]
async fn an_endless_loop_is_stopped() {
    let (done, _) = outcome("while (true) {}").await;
    assert!(done.expect_err("stopped").contains("too many steps"));
    let (done, _) = outcome("try { while (true) {} } catch (e) { return 'caught'; }").await;
    assert!(done.expect_err("not catchable").contains("too many steps"));
}

#[tokio::test]
async fn a_function_for_the_page_keeps_its_source() {
    assert_eq!(
        value("return await page.evaluate(() => document.title);").await,
        "() => document.title"
    );
}

#[tokio::test]
async fn constants_cannot_change() {
    let (done, _) = outcome("const a = 1; a = 2;").await;
    assert!(done.expect_err("thrown").contains("constant"));
}
