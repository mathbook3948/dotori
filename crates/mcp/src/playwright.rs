//! What `page`, its locators, `page.keyboard` and `page.mouse` do when a script calls them. Each
//! action waits for its element the way Playwright does, and refuses a locator that matches more
//! than one element.

use std::time::Duration;
use std::time::Instant;

use reqwest::Url;
use serde_json::Value as Json;
use serde_json::json;
use tokio::time;

use crate::browser::Page;
use crate::fetch;
use crate::script::run::BoxFuture;
use crate::script::run::Host;
use crate::script::run::Native;
use crate::script::run::Value;
use crate::script::run::from_json;
use crate::script::run::function_source;
use crate::script::run::number;
use crate::script::run::string;
use crate::script::run::to_json;
use crate::script::run::truthy;

const LOCATOR: &str = include_str!("locator.js");
const WAIT: Duration = Duration::from_secs(5);
const LONGEST_WAIT: Duration = Duration::from_secs(30);
const POLL: Duration = Duration::from_millis(100);
const CONTENT: usize = 100_000;

pub(crate) struct Driver<'a> {
    pub(crate) page: Page<'a>,
}

fn native(kind: &'static str, data: Json) -> Value {
    Value::Native(Native { kind, data })
}

fn locator(chain: Json) -> Value {
    native("locator", chain)
}

fn chain_of(this: &Native) -> Vec<Json> {
    this.data.as_array().cloned().unwrap_or_default()
}

fn pattern(value: &Value) -> Json {
    match value {
        Value::Regex(regex) => json!({ "regex": regex.0, "flags": regex.1 }),
        other => json!(string(other)),
    }
}

fn option(options: Option<&Value>, key: &str) -> Option<Value> {
    match options {
        Some(Value::Object(fields)) => fields
            .lock()
            .expect("unpoisoned")
            .iter()
            .find(|(name, _)| name == key)
            .map(|(_, value)| value.clone()),
        _ => None,
    }
}

fn timeout(options: Option<&Value>) -> Duration {
    option(options, "timeout")
        .map(|ms| number(&ms))
        .filter(|ms| ms.is_finite() && *ms >= 0.0)
        .map_or(WAIT, |ms| {
            Duration::from_millis(ms as u64).min(LONGEST_WAIT)
        })
}

fn get_by(method: &str, args: &[Value]) -> Option<Json> {
    let first = args.first().map(pattern).unwrap_or(Json::Null);
    let options = args.get(1);
    let exact = option(options, "exact").is_some_and(|exact| truthy(&exact));
    Some(match method {
        "getByRole" => {
            let mut step = json!({ "role": string(args.first()?), "exact": exact });
            if let Some(name) = option(options, "name") {
                step["name"] = pattern(&name);
            }
            if let Some(level) = option(options, "level") {
                step["level"] = json!(number(&level));
            }
            if option(options, "includeHidden").is_some_and(|hidden| truthy(&hidden)) {
                step["includeHidden"] = json!(true);
            }
            step
        }
        "getByText" => json!({ "text": first, "exact": exact }),
        "getByLabel" => json!({ "label": first, "exact": exact }),
        "getByPlaceholder" => json!({ "placeholder": first, "exact": exact }),
        "getByTestId" => json!({ "testId": first, "exact": true }),
        "getByAltText" => json!({ "alt": first, "exact": exact }),
        "getByTitle" => json!({ "title": first, "exact": exact }),
        "locator" => json!({ "css": string(args.first()?) }),
        _ => return None,
    })
}

fn described(chain: &[Json]) -> String {
    let shown = |value: &Json| match value {
        Json::Object(regex) if regex.contains_key("regex") => format!(
            "/{}/{}",
            regex["regex"].as_str().unwrap_or_default(),
            regex["flags"].as_str().unwrap_or_default()
        ),
        other => other.to_string(),
    };
    let mut text = String::from("page");
    for step in chain {
        let Json::Object(step) = step else {
            continue;
        };
        let part = if let Some(css) = step.get("css") {
            format!("locator({})", shown(css).replace('"', "'"))
        } else if let Some(role) = step.get("role") {
            match step.get("name") {
                Some(name) => format!("getByRole({}, {{ name: {} }})", shown(role), shown(name)),
                None => format!("getByRole({})", shown(role)),
            }
        } else if let Some(nth) = step.get("nth") {
            format!("nth({nth})")
        } else if let Some((key, value)) = step.iter().find(|(key, _)| *key != "exact") {
            let call = match key.as_str() {
                "text" => "getByText",
                "label" => "getByLabel",
                "placeholder" => "getByPlaceholder",
                "testId" => "getByTestId",
                "alt" => "getByAltText",
                "title" => "getByTitle",
                "hasText" => "filter({ hasText })",
                other => other,
            };
            format!("{call}({})", shown(value))
        } else {
            continue;
        };
        text.push('.');
        text.push_str(&part);
    }
    text
}

fn chained(chain: &[Json], step: Json) -> Value {
    let mut longer = chain.to_vec();
    longer.push(step);
    locator(Json::Array(longer))
}

#[derive(Clone, Copy, PartialEq)]
enum Need {
    Attached,
    Visible,
    Enabled,
    Editable,
}

impl Driver<'_> {
    async fn pw(&self, call: &str) -> Result<Json, String> {
        self.page
            .evaluate(&format!("(globalThis.__pw ??= {LOCATOR}).{call}"))
            .await
    }

    async fn probe(&self, chain: &[Json], scroll: bool) -> Result<Json, String> {
        self.pw(&format!("probe({}, {scroll})", Json::Array(chain.to_vec())))
            .await
    }

    async fn ready(&self, chain: &[Json], need: Need, wait: Duration) -> Result<Json, String> {
        let started = Instant::now();
        loop {
            let probe = self.probe(chain, need != Need::Attached).await?;
            let count = probe.get("count").and_then(Json::as_u64).unwrap_or(0);
            let flag = |name: &str| probe.get(name).and_then(Json::as_bool).unwrap_or(false);
            let problem = match count {
                0 => Some(String::from("matches no element")),
                1 if need != Need::Attached && !flag("visible") => {
                    Some(String::from("is not visible"))
                }
                1 if matches!(need, Need::Enabled | Need::Editable) && !flag("enabled") => {
                    Some(String::from("is disabled"))
                }
                1 if need == Need::Editable && !flag("editable") => {
                    Some(String::from("does not take text"))
                }
                1 => None,
                many => {
                    return Err(format!(
                        "{} matched {many} elements; use .first(), .nth(i) or a narrower locator",
                        described(chain)
                    ));
                }
            };
            match problem {
                None => return Ok(probe),
                Some(problem) if started.elapsed() >= wait => {
                    return Err(format!(
                        "{} {problem} after {} ms",
                        described(chain),
                        wait.as_millis()
                    ));
                }
                Some(_) => time::sleep(POLL).await,
            }
        }
    }

    async fn mouse(&self, x: f64, y: f64, clicks: u32, button: &str) -> Result<(), String> {
        self.page
            .send(
                "Input.dispatchMouseEvent",
                json!({ "type": "mouseMoved", "x": x, "y": y }),
            )
            .await?;
        for count in 1..=clicks {
            for kind in ["mousePressed", "mouseReleased"] {
                self.page
                    .send(
                        "Input.dispatchMouseEvent",
                        json!({ "type": kind, "x": x, "y": y, "button": button, "clickCount": count }),
                    )
                    .await?;
            }
        }
        Ok(())
    }

    async fn after_action(&self) -> Result<(), String> {
        time::sleep(Duration::from_millis(150)).await;
        self.page.settle().await?;
        self.page.guard().await
    }

    async fn press(&self, combo: &str) -> Result<(), String> {
        for event in key_events(combo)? {
            self.page.send("Input.dispatchKeyEvent", event).await?;
        }
        Ok(())
    }

    async fn insert(&self, text: &str) -> Result<(), String> {
        if !text.is_empty() {
            self.page
                .send("Input.insertText", json!({ "text": text }))
                .await?;
        }
        Ok(())
    }

    async fn read(
        &self,
        chain: &[Json],
        what: &str,
        arg: &str,
        wait: Duration,
    ) -> Result<Value, String> {
        self.ready(chain, Need::Attached, wait).await?;
        let read = self
            .pw(&format!(
                "read({}, {}, {})",
                Json::Array(chain.to_vec()),
                json!(what),
                json!(arg)
            ))
            .await?;
        match read.get("value") {
            Some(value) => Ok(from_json(value)),
            None => Err(format!("{} is gone", described(chain))),
        }
    }

    async fn run_in_page(
        &self,
        chain: Option<&[Json]>,
        args: &[Value],
        every: bool,
    ) -> Result<Value, String> {
        let function = args.first().ok_or("pass a function")?;
        let source = match function_source(function) {
            Some(source) => source,
            None => format!("() => ({})", string(function)),
        };
        let arg = args.get(1).map_or(Json::Null, to_json);
        let expression = match chain {
            Some(chain) => format!(
                "(globalThis.__pw ??= {LOCATOR}).call({}, {source}, {arg}, {every})",
                Json::Array(chain.to_vec())
            ),
            None => format!("({source})({arg})"),
        };
        Ok(from_json(&self.page.evaluate(&expression).await?))
    }

    async fn locator_call(
        &self,
        chain: &[Json],
        method: &str,
        args: &[Value],
    ) -> Result<Value, String> {
        if let Some(step) = get_by(method, args) {
            return Ok(chained(chain, step));
        }
        let first = args.first().cloned().unwrap_or(Value::Undefined);
        let options = |at: usize| args.get(at);
        let list = Json::Array(chain.to_vec());
        Ok(match method {
            "first" => chained(chain, json!({ "nth": 0 })),
            "last" => chained(chain, json!({ "nth": -1 })),
            "nth" => chained(chain, json!({ "nth": number(&first) as i64 })),
            "filter" => {
                let mut longer = chain.to_vec();
                if let Some(text) = option(Some(&first), "hasText") {
                    longer.push(json!({ "hasText": pattern(&text) }));
                }
                if let Some(text) = option(Some(&first), "hasNotText") {
                    longer.push(json!({ "hasNotText": pattern(&text) }));
                }
                if let Some(visible) = option(Some(&first), "visible") {
                    longer.push(json!({ "visible": truthy(&visible) }));
                }
                locator(Json::Array(longer))
            }
            "count" => Value::Number(
                self.pw(&format!("count({list})"))
                    .await?
                    .as_f64()
                    .unwrap_or(0.0),
            ),
            "all" => {
                let count = self
                    .pw(&format!("count({list})"))
                    .await?
                    .as_u64()
                    .unwrap_or(0);
                let made: Vec<Value> = (0..count)
                    .map(|index| chained(chain, json!({ "nth": index })))
                    .collect();
                Value::Array(std::sync::Arc::new(std::sync::Mutex::new(made)))
            }
            "allTextContents" => {
                from_json(&self.pw(&format!("read({list}, 'all', 'text')")).await?)
            }
            "allInnerTexts" => from_json(&self.pw(&format!("read({list}, 'all', 'inner')")).await?),
            "click" | "dblclick" | "tap" => {
                let probe = self
                    .ready(chain, Need::Enabled, timeout(options(0)))
                    .await?;
                let at = |name: &str| probe.get(name).and_then(Json::as_f64).unwrap_or(0.0);
                let button = option(options(0), "button")
                    .map_or_else(|| String::from("left"), |b| string(&b));
                let clicks = if method == "dblclick" {
                    2
                } else {
                    option(options(0), "clickCount")
                        .map_or(1, |n| number(&n).clamp(1.0, 3.0) as u32)
                };
                self.mouse(at("x"), at("y"), clicks, &button).await?;
                self.after_action().await?;
                Value::Undefined
            }
            "hover" => {
                let probe = self
                    .ready(chain, Need::Visible, timeout(options(0)))
                    .await?;
                let at = |name: &str| probe.get(name).and_then(Json::as_f64).unwrap_or(0.0);
                self.page
                    .send(
                        "Input.dispatchMouseEvent",
                        json!({ "type": "mouseMoved", "x": at("x"), "y": at("y") }),
                    )
                    .await?;
                Value::Undefined
            }
            "fill" | "clear" => {
                let text = if method == "clear" {
                    String::new()
                } else {
                    string(&first)
                };
                self.ready(chain, Need::Editable, timeout(options(1)))
                    .await?;
                self.pw(&format!("focus({list}, true)")).await?;
                self.press("Delete").await?;
                self.insert(&text).await?;
                Value::Undefined
            }
            "press" => {
                self.ready(chain, Need::Attached, timeout(options(1)))
                    .await?;
                self.pw(&format!("focus({list}, false)")).await?;
                self.press(&string(&first)).await?;
                self.after_action().await?;
                Value::Undefined
            }
            "type" | "pressSequentially" => {
                self.ready(chain, Need::Editable, timeout(options(1)))
                    .await?;
                self.pw(&format!("focus({list}, false)")).await?;
                for c in string(&first).chars() {
                    self.insert(&c.to_string()).await?;
                }
                Value::Undefined
            }
            "focus" => {
                self.ready(chain, Need::Attached, timeout(options(0)))
                    .await?;
                self.pw(&format!("focus({list}, false)")).await?;
                Value::Undefined
            }
            "check" | "uncheck" | "setChecked" => {
                let wanted = match method {
                    "check" => true,
                    "uncheck" => false,
                    _ => truthy(&first),
                };
                let at = if method == "setChecked" { 1 } else { 0 };
                let probe = self
                    .ready(chain, Need::Enabled, timeout(options(at)))
                    .await?;
                let now = truthy(&self.read(chain, "checked", "", WAIT).await?);
                if now != wanted {
                    let point = |name: &str| probe.get(name).and_then(Json::as_f64).unwrap_or(0.0);
                    self.mouse(point("x"), point("y"), 1, "left").await?;
                    self.after_action().await?;
                }
                Value::Undefined
            }
            "selectOption" => {
                self.ready(chain, Need::Enabled, timeout(options(1)))
                    .await?;
                let wanted: Vec<String> = match &first {
                    Value::Array(items) => items
                        .lock()
                        .expect("unpoisoned")
                        .iter()
                        .map(string)
                        .collect(),
                    Value::Object(_) => option(Some(&first), "value")
                        .or_else(|| option(Some(&first), "label"))
                        .map(|v| vec![string(&v)])
                        .unwrap_or_default(),
                    other => vec![string(other)],
                };
                let chosen = self
                    .pw(&format!("select({list}, {})", json!(wanted)))
                    .await?;
                if let Some(error) = chosen.get("error").and_then(Json::as_str) {
                    return Err(format!("{}: {error}", described(chain)));
                }
                from_json(chosen.get("value").unwrap_or(&Json::Null))
            }
            "textContent" => self.read(chain, "text", "", timeout(options(0))).await?,
            "innerText" => self.read(chain, "inner", "", timeout(options(0))).await?,
            "innerHTML" => self.read(chain, "html", "", timeout(options(0))).await?,
            "inputValue" => self.read(chain, "value", "", timeout(options(0))).await?,
            "getAttribute" => {
                self.read(chain, "attribute", &string(&first), timeout(options(1)))
                    .await?
            }
            "isVisible" | "isHidden" => {
                let probe = self.probe(chain, false).await?;
                let visible = probe.get("count").and_then(Json::as_u64) == Some(1)
                    && probe.get("visible").and_then(Json::as_bool) == Some(true);
                Value::Bool(if method == "isVisible" {
                    visible
                } else {
                    !visible
                })
            }
            "isChecked" => self.read(chain, "checked", "", timeout(options(0))).await?,
            "isEnabled" | "isDisabled" => {
                let enabled = truthy(&self.read(chain, "enabled", "", timeout(options(0))).await?);
                Value::Bool(if method == "isEnabled" {
                    enabled
                } else {
                    !enabled
                })
            }
            "isEditable" => {
                self.read(chain, "editable", "", timeout(options(0)))
                    .await?
            }
            "waitFor" => {
                let state = option(options(0), "state")
                    .map_or_else(|| String::from("visible"), |s| string(&s));
                let wait = timeout(options(0));
                match state.as_str() {
                    "visible" => drop(self.ready(chain, Need::Visible, wait).await?),
                    "attached" => drop(self.ready(chain, Need::Attached, wait).await?),
                    "hidden" | "detached" => {
                        let started = Instant::now();
                        loop {
                            let probe = self.probe(chain, false).await?;
                            let count = probe.get("count").and_then(Json::as_u64).unwrap_or(0);
                            let gone = count == 0
                                || (state == "hidden"
                                    && probe.get("visible").and_then(Json::as_bool) == Some(false));
                            if gone {
                                break;
                            }
                            if started.elapsed() >= wait {
                                return Err(format!(
                                    "{} is still there after {} ms",
                                    described(chain),
                                    wait.as_millis()
                                ));
                            }
                            time::sleep(POLL).await;
                        }
                    }
                    other => return Err(format!("unknown state `{other}`")),
                }
                Value::Undefined
            }
            "scrollIntoViewIfNeeded" => {
                self.ready(chain, Need::Visible, timeout(options(0)))
                    .await?;
                Value::Undefined
            }
            "evaluate" => self.run_in_page(Some(chain), args, false).await?,
            "evaluateAll" => self.run_in_page(Some(chain), args, true).await?,
            "screenshot" => {
                return Err(String::from(
                    "screenshots are not available; read the page instead",
                ));
            }
            _ => return Err(format!("locators here have no `{method}`")),
        })
    }

    async fn page_call(&self, method: &str, args: &[Value]) -> Result<Value, String> {
        if let Some(step) = get_by(method, args) {
            return Ok(locator(Json::Array(vec![step])));
        }
        let first = args.first().cloned().unwrap_or(Value::Undefined);
        Ok(match method {
            "goto" => {
                let url =
                    Url::parse(&string(&first)).map_err(|error| format!("not a URL: {error}"))?;
                fetch::allowed(&url)?;
                self.page.go(url.as_str()).await?;
                self.page.guard().await?;
                Value::Undefined
            }
            "reload" => {
                self.page.send("Page.reload", json!({})).await?;
                self.after_action().await?;
                Value::Undefined
            }
            "goBack" | "goForward" => {
                let step = if method == "goBack" { -1 } else { 1 };
                self.page.evaluate(&format!("history.go({step})")).await?;
                self.after_action().await?;
                Value::Undefined
            }
            "url" => from_json(&self.page.evaluate("location.href").await?),
            "title" => from_json(&self.page.evaluate("document.title").await?),
            "content" => {
                let html = self
                    .page
                    .evaluate("document.documentElement.outerHTML")
                    .await?;
                let html = html.as_str().unwrap_or_default();
                let cut: String = html.chars().take(CONTENT).collect();
                Value::Str(cut.into())
            }
            "waitForTimeout" => {
                let ms = number(&first).clamp(0.0, LONGEST_WAIT.as_millis() as f64);
                time::sleep(Duration::from_millis(ms as u64)).await;
                Value::Undefined
            }
            "waitForLoadState" | "waitForNavigation" => {
                self.page.settle().await?;
                Value::Undefined
            }
            "waitForURL" => {
                let wait = timeout(args.get(1));
                let started = Instant::now();
                loop {
                    let href = string(&from_json(&self.page.evaluate("location.href").await?));
                    let reached = match &first {
                        Value::Regex(regex) => truthy(&from_json(
                            &self
                                .page
                                .evaluate(&format!(
                                    "new RegExp({}, {}).test({})",
                                    json!(regex.0),
                                    json!(regex.1),
                                    json!(href)
                                ))
                                .await?,
                        )),
                        other => {
                            let wanted = string(other);
                            href == wanted || href.contains(wanted.trim_matches('*'))
                        }
                    };
                    if reached {
                        break;
                    }
                    if started.elapsed() >= wait {
                        return Err(format!(
                            "the page is still at {href} after {} ms",
                            wait.as_millis()
                        ));
                    }
                    time::sleep(POLL).await;
                }
                self.page.settle().await?;
                Value::Undefined
            }
            "waitForSelector" => {
                let chain = vec![json!({ "css": string(&first) })];
                self.locator_call(&chain, "waitFor", &args[1..]).await?
            }
            "evaluate" => self.run_in_page(None, args, false).await?,
            "$eval" | "$$eval" => {
                let chain = vec![json!({ "css": string(&first) })];
                if method == "$eval" {
                    self.locator_call(&chain, "evaluate", &args[1..]).await?
                } else {
                    self.locator_call(&chain, "evaluateAll", &args[1..]).await?
                }
            }
            "screenshot" => {
                return Err(String::from(
                    "screenshots are not available; read the page instead",
                ));
            }
            "click" | "dblclick" | "fill" | "press" | "type" | "check" | "uncheck" | "hover"
            | "focus" | "selectOption" | "textContent" | "innerText" | "innerHTML"
            | "inputValue" | "getAttribute" | "isVisible" | "isHidden" | "isChecked"
            | "isEnabled" | "isDisabled" => {
                let chain = vec![json!({ "css": string(&first) })];
                self.locator_call(&chain, method, &args[1..]).await?
            }
            _ => return Err(format!("`page.{method}` is not supported here")),
        })
    }
}

impl Host for Driver<'_> {
    fn call<'a>(
        &'a self,
        this: &'a Native,
        method: &'a str,
        args: Vec<Value>,
    ) -> BoxFuture<'a, Result<Value, String>> {
        Box::pin(async move {
            match this.kind {
                "page" => self.page_call(method, &args).await,
                "locator" => self.locator_call(&chain_of(this), method, &args).await,
                "keyboard" => {
                    let first = args.first().map(string).unwrap_or_default();
                    match method {
                        "press" => self.press(&first).await?,
                        "type" | "insertText" => self.insert(&first).await?,
                        "down" | "up" => return Err(String::from("use keyboard.press")),
                        _ => return Err(format!("`keyboard.{method}` is not supported here")),
                    }
                    if method == "press" {
                        self.after_action().await?;
                    }
                    Ok(Value::Undefined)
                }
                "mouse" => {
                    let at = |index: usize| args.get(index).map_or(0.0, number);
                    match method {
                        "click" => {
                            self.mouse(at(0), at(1), 1, "left").await?;
                            self.after_action().await?;
                        }
                        "move" => {
                            self.page
                                .send(
                                    "Input.dispatchMouseEvent",
                                    json!({ "type": "mouseMoved", "x": at(0), "y": at(1) }),
                                )
                                .await?;
                        }
                        "wheel" => {
                            self.page
                                .send(
                                    "Input.dispatchMouseEvent",
                                    json!({ "type": "mouseWheel", "x": 100, "y": 100, "deltaX": at(0), "deltaY": at(1) }),
                                )
                                .await?;
                            time::sleep(Duration::from_millis(200)).await;
                        }
                        _ => return Err(format!("`mouse.{method}` is not supported here")),
                    }
                    Ok(Value::Undefined)
                }
                other => Err(format!("{other} has no `{method}`")),
            }
        })
    }

    fn property(&self, this: &Native, name: &str) -> Option<Value> {
        match (this.kind, name) {
            ("page", "keyboard") => Some(native("keyboard", Json::Null)),
            ("page", "mouse") => Some(native("mouse", Json::Null)),
            _ => None,
        }
    }
}

/// The key events for a key such as `Enter`, `a` or `Control+A`, down then up.
pub(crate) fn key_events(combo: &str) -> Result<Vec<Json>, String> {
    let parts: Vec<&str> = if combo == "+" {
        vec!["+"]
    } else {
        combo.split('+').collect()
    };
    let (key, held) = parts.split_last().ok_or("an empty key")?;
    let mut modifiers = 0;
    for name in held {
        modifiers |= match *name {
            "Alt" => 1,
            "Control" => 2,
            "Meta" => 4,
            "Shift" => 8,
            "ControlOrMeta" => {
                if cfg!(target_os = "macos") {
                    4
                } else {
                    2
                }
            }
            other => return Err(format!("unknown modifier `{other}`")),
        };
    }
    let named: Option<(&str, i64, &str)> = match *key {
        "Enter" => Some(("Enter", 13, "\r")),
        "Tab" => Some(("Tab", 9, "")),
        "Escape" => Some(("Escape", 27, "")),
        "Backspace" => Some(("Backspace", 8, "")),
        "Delete" => Some(("Delete", 46, "")),
        "ArrowUp" => Some(("ArrowUp", 38, "")),
        "ArrowDown" => Some(("ArrowDown", 40, "")),
        "ArrowLeft" => Some(("ArrowLeft", 37, "")),
        "ArrowRight" => Some(("ArrowRight", 39, "")),
        "Home" => Some(("Home", 36, "")),
        "End" => Some(("End", 35, "")),
        "PageUp" => Some(("PageUp", 33, "")),
        "PageDown" => Some(("PageDown", 34, "")),
        "Space" | " " => Some((" ", 32, " ")),
        _ => None,
    };
    let (key, code, typed) = match named {
        Some((key, code, typed)) => (key.to_owned(), code, typed.to_owned()),
        None => {
            let mut chars = key.chars();
            let (Some(c), None) = (chars.next(), chars.next()) else {
                return Err(format!("unknown key `{key}`"));
            };
            let code = i64::from(u32::from(c.to_ascii_uppercase()));
            (c.to_string(), code, c.to_string())
        }
    };
    let typed = if modifiers & (1 | 2 | 4) != 0 {
        String::new()
    } else {
        typed
    };
    let mut down = json!({ "type": if typed.is_empty() { "rawKeyDown" } else { "keyDown" }, "key": key, "windowsVirtualKeyCode": code, "modifiers": modifiers });
    if !typed.is_empty() {
        down["text"] = json!(typed);
    }
    if key.len() == 1 && modifiers & 2 != 0 && key.eq_ignore_ascii_case("a") {
        down["commands"] = json!(["selectAll"]);
    }
    let up = json!({ "type": "keyUp", "key": key, "windowsVirtualKeyCode": code, "modifiers": modifiers });
    Ok(vec![down, up])
}

#[cfg(test)]
#[path = "playwright.test.rs"]
mod tests;
