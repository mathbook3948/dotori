use std::path::Path;
use std::path::PathBuf;
use std::process::Stdio;
use std::sync::Arc;
use std::time::Duration;

use reqwest::Url;
use serde_json::Value;
use serde_json::json;
use tokio::io::AsyncBufReadExt;
use tokio::io::BufReader;
use tokio::process::Child;
use tokio::process::Command;
use tokio::sync::Mutex;
use tokio::time;

use crate::Tool;
use crate::ToolFuture;
use crate::cdp::Cdp;
use crate::chrome;
use crate::fetch;

const START_WAIT: Duration = Duration::from_secs(20);
const LOAD_WAIT: Duration = Duration::from_secs(20);
const SETTLE: Duration = Duration::from_millis(400);
const POLL: Duration = Duration::from_millis(200);
const SNAPSHOT: &str = include_str!("snapshot.js");
const SHOWN: usize = 20_000;
const CLOSE_WAIT: Duration = Duration::from_secs(10);
const FIND: &str = "globalThis.__refs && globalThis.__refs.get(";
const WORLD: &str = "dotori";

/// How the browser runs. The profile folder keeps logins between runs. Without an executable,
/// dotori's own Chrome for Testing in `downloads` is used, fetched there on first use.
#[derive(Clone, Debug)]
pub struct BrowserConfig {
    pub executable: Option<PathBuf>,
    pub downloads: PathBuf,
    pub profile: PathBuf,
    pub headless: bool,
}

/// One browser shared by every browser tool. It starts on first use and starts again if it died.
#[derive(Debug)]
pub struct Browser {
    config: BrowserConfig,
    install: Arc<std::sync::Mutex<Install>>,
    running: Mutex<Option<Running>>,
    acting: Mutex<()>,
}

#[derive(Debug)]
enum Install {
    Fetching,
    Ready(PathBuf),
    Failed(String),
}

struct Running {
    child: Child,
    cdp: Arc<Cdp>,
    session: String,
    frame: String,
    world: Mutex<Option<i64>>,
    watcher: tokio::task::JoinHandle<()>,
}

impl Drop for Running {
    fn drop(&mut self) {
        self.watcher.abort();
    }
}

impl std::fmt::Debug for Running {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Running").finish_non_exhaustive()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Action {
    Open,
    Read,
    Click,
    Type,
}

/// One of the browser tools, all driving the same browser.
#[derive(Debug)]
pub struct BrowserTool {
    browser: Arc<Browser>,
    action: Action,
}

impl Browser {
    /// Starts fetching dotori's own browser in the background unless it is there or another one
    /// is set, so the first page does not wait for the download.
    pub fn new(config: BrowserConfig) -> Arc<Self> {
        let install = match config
            .executable
            .clone()
            .or_else(|| chrome::installed(&config.downloads))
        {
            Some(executable) => Install::Ready(executable),
            None => Install::Fetching,
        };
        let fetching = matches!(install, Install::Fetching);
        let install = Arc::new(std::sync::Mutex::new(install));
        if fetching {
            fetch(Arc::clone(&install), config.downloads.clone());
        }
        Arc::new(Self {
            config,
            install,
            running: Mutex::new(None),
            acting: Mutex::new(()),
        })
    }

    /// The tools that drive this browser.
    pub fn tools(self: &Arc<Self>) -> Vec<Box<dyn Tool>> {
        [Action::Open, Action::Read, Action::Click, Action::Type]
            .into_iter()
            .map(|action| {
                Box::new(BrowserTool {
                    browser: Arc::clone(self),
                    action,
                }) as Box<dyn Tool>
            })
            .collect()
    }

    fn executable(&self) -> Result<PathBuf, String> {
        let mut install = self.install.lock().expect("unpoisoned");
        match &*install {
            Install::Ready(executable) => Ok(executable.clone()),
            Install::Fetching => Err(String::from(
                "dotori is downloading its own browser for the first time; try again in a few minutes",
            )),
            Install::Failed(error) => {
                let error =
                    format!("dotori's browser could not be downloaded, trying again: {error}");
                *install = Install::Fetching;
                fetch(Arc::clone(&self.install), self.config.downloads.clone());
                Err(error)
            }
        }
    }

    async fn page(&self) -> Result<tokio::sync::MappedMutexGuard<'_, Running>, String> {
        let mut running = self.running.lock().await;
        let alive = running.as_ref().is_some_and(|current| current.cdp.open());
        if !alive {
            if let Some(mut ended) = running.take() {
                let _ = ended.child.kill().await;
            }
            *running = Some(self.launch().await?);
        }
        tokio::sync::MutexGuard::try_map(running, Option::as_mut)
            .map_err(|_| String::from("the browser is not running"))
    }

    async fn forget(&self) {
        if let Some(mut ended) = self.running.lock().await.take() {
            let _ = ended.child.kill().await;
        }
    }

    async fn close_left_over(&self) {
        let Ok(port) = std::fs::read_to_string(self.config.profile.join("DevToolsActivePort"))
        else {
            return;
        };
        let mut lines = port.lines();
        let (Some(port), Some(path)) = (lines.next(), lines.next()) else {
            return;
        };
        let Ok(cdp) = Cdp::connect(&format!("ws://127.0.0.1:{}{}", port.trim(), path.trim())).await
        else {
            return;
        };
        let _ = cdp.send("Browser.close", json!({}), None).await;
        let _ = time::timeout(CLOSE_WAIT, async {
            while cdp.open() {
                time::sleep(POLL).await;
            }
        })
        .await;
        time::sleep(SETTLE).await;
    }

    async fn launch(&self) -> Result<Running, String> {
        let executable = self.executable()?;
        std::fs::create_dir_all(&self.config.profile).map_err(|error| error.to_string())?;
        self.close_left_over().await;
        forget_passwords(&self.config.profile)?;
        let mut command = Command::new(&executable);
        command
            .arg(format!("--user-data-dir={}", self.config.profile.display()))
            .arg("--remote-debugging-port=0")
            .arg("--no-first-run")
            .arg("--no-default-browser-check")
            .arg("--disable-blink-features=AutomationControlled")
            .arg("about:blank")
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .kill_on_drop(true);
        if self.config.headless {
            command.arg("--headless=new");
        }
        if root() {
            command.arg("--no-sandbox");
        }
        let mut child = command
            .spawn()
            .map_err(|error| format!("cannot start {}: {error}", executable.display()))?;
        let stderr = child.stderr.take().ok_or("the browser has no output")?;
        let mut lines = BufReader::new(stderr).lines();
        let url = time::timeout(START_WAIT, async {
            while let Ok(Some(line)) = lines.next_line().await {
                if let Some(url) = line.strip_prefix("DevTools listening on ") {
                    return Some(url.trim().to_owned());
                }
            }
            None
        })
        .await
        .map_err(|_| String::from("the browser did not start in time"))?
        .ok_or("the browser ended at once; is its profile open in another window?")?;
        tokio::spawn(async move { while let Ok(Some(_)) = lines.next_line().await {} });
        let cdp = Arc::new(Cdp::connect(&url).await?);
        let targets = cdp.send("Target.getTargets", json!({}), None).await?;
        let existing = targets
            .get("targetInfos")
            .and_then(Value::as_array)
            .and_then(|targets| {
                targets
                    .iter()
                    .find(|target| target.get("type").and_then(Value::as_str) == Some("page"))
            })
            .and_then(|target| target.get("targetId").and_then(Value::as_str))
            .map(str::to_owned);
        let target = match existing {
            Some(target) => target,
            None => text_at(
                &cdp.send("Target.createTarget", json!({ "url": "about:blank" }), None)
                    .await?,
                "targetId",
            )?,
        };
        let attached = cdp
            .send(
                "Target.attachToTarget",
                json!({ "targetId": target, "flatten": true }),
                None,
            )
            .await?;
        let session = text_at(&attached, "sessionId")?;
        let events = cdp
            .events()
            .ok_or("the browser's events are already taken")?;
        let watcher = tokio::spawn(watch(Arc::clone(&cdp), events));
        let page = |method: &'static str, params: Value| {
            let cdp = Arc::clone(&cdp);
            let session = session.clone();
            async move { cdp.send(method, params, Some(&session)).await }
        };
        page("Page.enable", json!({})).await?;
        page(
            "Fetch.enable",
            json!({ "patterns": [{ "resourceType": "Document", "requestStage": "Request" }] }),
        )
        .await?;
        let version = cdp.send("Browser.getVersion", json!({}), None).await?;
        if let Some(agent) = version.get("userAgent").and_then(Value::as_str)
            && agent.contains("HeadlessChrome")
        {
            page("Emulation.setUserAgentOverride", plain_agent(agent)).await?;
        }
        let tree = page("Page.getFrameTree", json!({})).await?;
        let frame = tree
            .pointer("/frameTree/frame/id")
            .and_then(Value::as_str)
            .ok_or("the page has no frame")?
            .to_owned();
        Ok(Running {
            child,
            cdp,
            session,
            frame,
            world: Mutex::new(None),
            watcher,
        })
    }
}

fn fetch(install: Arc<std::sync::Mutex<Install>>, downloads: PathBuf) {
    if tokio::runtime::Handle::try_current().is_err() {
        *install.lock().expect("unpoisoned") =
            Install::Failed(String::from("no runtime to download the browser on"));
        return;
    }
    tokio::spawn(async move {
        let done = match chrome::ensure(&downloads).await {
            Ok(executable) => Install::Ready(executable),
            Err(error) => {
                eprintln!("dotori: the browser could not be downloaded: {error}");
                Install::Failed(error)
            }
        };
        *install.lock().expect("unpoisoned") = done;
    });
}

fn forget_passwords(profile: &Path) -> Result<(), String> {
    let path = profile.join("Default").join("Preferences");
    let mut preferences = match std::fs::read_to_string(&path) {
        Ok(text) => serde_json::from_str::<Value>(&text).unwrap_or_else(|_| json!({})),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => json!({}),
        Err(error) => return Err(format!("{}: {error}", path.display())),
    };
    if !preferences.is_object() {
        preferences = json!({});
    }
    preferences["credentials_enable_service"] = json!(false);
    preferences["credentials_enable_autosignin"] = json!(false);
    if !preferences["profile"].is_object() {
        preferences["profile"] = json!({});
    }
    preferences["profile"]["password_manager_enabled"] = json!(false);
    if let Some(folder) = path.parent() {
        std::fs::create_dir_all(folder).map_err(|error| error.to_string())?;
    }
    let draft = path.with_extension("dotori-new");
    std::fs::write(&draft, preferences.to_string()).map_err(|error| error.to_string())?;
    std::fs::rename(&draft, &path).map_err(|error| error.to_string())
}

/// The user agent a windowed Chrome of the same version sends, and its client hints.
pub(crate) fn plain_agent(agent: &str) -> Value {
    let agent = agent.replace("HeadlessChrome", "Chrome");
    let version = agent
        .split("Chrome/")
        .nth(1)
        .and_then(|rest| rest.split_whitespace().next())
        .unwrap_or("0");
    let major = version.split('.').next().unwrap_or("0");
    let platform = if cfg!(target_os = "macos") {
        "macOS"
    } else if cfg!(windows) {
        "Windows"
    } else {
        "Linux"
    };
    json!({
        "userAgent": agent,
        "userAgentMetadata": {
            "brands": [
                { "brand": "Chromium", "version": major },
                { "brand": "Not_A Brand", "version": "24" }
            ],
            "fullVersionList": [
                { "brand": "Chromium", "version": version },
                { "brand": "Not_A Brand", "version": "24.0.0.0" }
            ],
            "fullVersion": version,
            "platform": platform,
            "platformVersion": "",
            "architecture": if cfg!(target_arch = "aarch64") { "arm" } else { "x86" },
            "model": "",
            "mobile": false
        }
    })
}

async fn watch(cdp: Arc<Cdp>, mut events: tokio::sync::mpsc::UnboundedReceiver<Value>) {
    while let Some(event) = events.recv().await {
        let session = event.get("sessionId").and_then(Value::as_str);
        let params = event.get("params").cloned().unwrap_or(Value::Null);
        let reply = match event.get("method").and_then(Value::as_str) {
            Some("Page.javascriptDialogOpening") => {
                let accept = matches!(
                    params.get("type").and_then(Value::as_str),
                    Some("alert" | "beforeunload")
                );
                Some(("Page.handleJavaScriptDialog", json!({ "accept": accept })))
            }
            Some("Fetch.requestPaused") => {
                let id = params.get("requestId").cloned().unwrap_or(Value::Null);
                let url = params
                    .pointer("/request/url")
                    .and_then(Value::as_str)
                    .unwrap_or_default();
                Some(match document_allowed(url) {
                    Ok(()) => ("Fetch.continueRequest", json!({ "requestId": id })),
                    Err(_) => (
                        "Fetch.failRequest",
                        json!({ "requestId": id, "errorReason": "BlockedByClient" }),
                    ),
                })
            }
            _ => None,
        };
        if let Some((method, params)) = reply {
            let _ = cdp.send(method, params, session).await;
        }
    }
}

/// Whether a page may be loaded from `url`, by the same rule as webfetch.
pub(crate) fn document_allowed(url: &str) -> Result<(), String> {
    let url = Url::parse(url).map_err(|error| error.to_string())?;
    fetch::allowed(&url)
}

impl BrowserTool {
    async fn act(&self, arguments: Value) -> Result<String, String> {
        let step = Step::read(self.action, &arguments)?;
        let start = match arguments.get("start_index") {
            None => 0,
            Some(start) => start
                .as_u64()
                .and_then(|start| usize::try_from(start).ok())
                .ok_or("`start_index` must be a whole number from 0")?,
        };
        let _acting = self.browser.acting.lock().await;
        let running = self.browser.page().await?;
        let page = Page { running: &running };
        let done = match step {
            Step::Open(url) => page.go(url.as_str()).await,
            Step::Read => Ok(()),
            Step::Click(element) => page.click(element).await,
            Step::Type {
                element,
                text,
                submit,
            } => page.type_into(element, &text, submit).await,
        };
        let shown = match done {
            Ok(()) => match page.guard().await {
                Ok(()) => page.snapshot().await,
                Err(error) => Err(error),
            },
            Err(error) => Err(error),
        };
        let broken = !running.cdp.open();
        drop(running);
        if let Err(error) = &shown
            && (broken || error.contains("not found"))
        {
            self.browser.forget().await;
        }
        Ok(fetch::window(&shown?, start, SHOWN))
    }
}

enum Step {
    Open(Url),
    Read,
    Click(u64),
    Type {
        element: u64,
        text: String,
        submit: bool,
    },
}

impl Step {
    fn read(action: Action, arguments: &Value) -> Result<Self, String> {
        Ok(match action {
            Action::Open => {
                let url = arguments
                    .get("url")
                    .and_then(Value::as_str)
                    .ok_or("`url` is required")?;
                let url = Url::parse(url).map_err(|error| format!("not a URL: {error}"))?;
                fetch::allowed(&url)?;
                Self::Open(url)
            }
            Action::Read => Self::Read,
            Action::Click => Self::Click(element(arguments)?),
            Action::Type => Self::Type {
                element: element(arguments)?,
                text: arguments
                    .get("text")
                    .and_then(Value::as_str)
                    .ok_or("`text` is required")?
                    .to_owned(),
                submit: arguments
                    .get("submit")
                    .and_then(Value::as_bool)
                    .unwrap_or(false),
            },
        })
    }
}

struct Page<'a> {
    running: &'a Running,
}

impl Page<'_> {
    async fn send(&self, method: &str, params: Value) -> Result<Value, String> {
        self.running
            .cdp
            .send(method, params, Some(&self.running.session))
            .await
    }

    async fn world(&self, fresh: bool) -> Result<i64, String> {
        let mut world = self.running.world.lock().await;
        if let Some(context) = *world
            && !fresh
        {
            return Ok(context);
        }
        let made = self
            .send(
                "Page.createIsolatedWorld",
                json!({ "frameId": self.running.frame, "worldName": WORLD }),
            )
            .await?;
        let context = made
            .get("executionContextId")
            .and_then(Value::as_i64)
            .ok_or("the browser made no world for dotori's scripts")?;
        *world = Some(context);
        Ok(context)
    }

    async fn evaluate(&self, expression: &str) -> Result<Value, String> {
        let mut fresh = false;
        let reply = loop {
            let context = self.world(fresh).await?;
            let sent = self
                .send(
                    "Runtime.evaluate",
                    json!({
                        "expression": expression,
                        "returnByValue": true,
                        "awaitPromise": true,
                        "contextId": context
                    }),
                )
                .await;
            match sent {
                Err(error) if !fresh && error.contains("context") => fresh = true,
                sent => break sent?,
            }
        };
        if let Some(details) = reply.get("exceptionDetails") {
            let shown = details
                .pointer("/exception/description")
                .or_else(|| details.get("text"))
                .and_then(Value::as_str)
                .unwrap_or("the page script failed");
            return Err(shown.to_owned());
        }
        Ok(reply
            .pointer("/result/value")
            .cloned()
            .unwrap_or(Value::Null))
    }

    async fn go(&self, url: &str) -> Result<(), String> {
        let reply = self.send("Page.navigate", json!({ "url": url })).await?;
        if let Some(error) = reply.get("errorText").and_then(Value::as_str) {
            return Err(format!("could not open {url}: {error}"));
        }
        self.settle().await
    }

    async fn settle(&self) -> Result<(), String> {
        time::sleep(SETTLE).await;
        let ready = async {
            loop {
                if self
                    .evaluate("document.readyState")
                    .await
                    .ok()
                    .as_ref()
                    .and_then(Value::as_str)
                    == Some("complete")
                {
                    return;
                }
                time::sleep(POLL).await;
            }
        };
        let _ = time::timeout(LOAD_WAIT, ready).await;
        Ok(())
    }

    async fn guard(&self) -> Result<(), String> {
        let at = self.evaluate("location.href").await?;
        let at = at.as_str().unwrap_or_default();
        if at == "about:blank" || at.starts_with("data:") {
            return Ok(());
        }
        let refused = match Url::parse(at) {
            Ok(url) => fetch::allowed(&url).err(),
            Err(error) => Some(error.to_string()),
        };
        let Some(refused) = refused else {
            return Ok(());
        };
        let _ = self
            .send("Page.navigate", json!({ "url": "about:blank" }))
            .await;
        Err(format!(
            "the page went to {at}, which is refused: {refused}"
        ))
    }

    async fn center(&self, element: u64) -> Result<(f64, f64), String> {
        let found = self
            .evaluate(&format!(
                "(() => {{ const el = {FIND}{element}); \
                 if (!el || !el.isConnected) return null; \
                 el.scrollIntoView({{block: 'center', inline: 'center', behavior: 'instant'}}); \
                 const r = el.getClientRects()[0] || el.getBoundingClientRect(); \
                 return [r.left + r.width / 2, r.top + r.height / 2]; }})()"
            ))
            .await?;
        let point = found
            .as_array()
            .and_then(|point| Some((point.first()?.as_f64()?, point.get(1)?.as_f64()?)))
            .ok_or_else(|| format!("there is no element [{element}]; read the page again"))?;
        Ok(point)
    }

    async fn click(&self, element: u64) -> Result<(), String> {
        let (x, y) = self.center(element).await?;
        for kind in ["mouseMoved", "mousePressed", "mouseReleased"] {
            self.send(
                "Input.dispatchMouseEvent",
                json!({ "type": kind, "x": x, "y": y, "button": "left", "clickCount": 1 }),
            )
            .await?;
        }
        self.settle().await
    }

    async fn type_into(&self, element: u64, text: &str, submit: bool) -> Result<(), String> {
        let ready = self
            .evaluate(&format!(
                "(() => {{ const el = {FIND}{element}); \
                 if (!el || !el.isConnected) return 'gone'; \
                 const tag = el.tagName.toLowerCase(); \
                 const text = tag === 'textarea' || el.isContentEditable || (tag === 'input' && \
                   ['text', 'search', 'email', 'url', 'tel', 'password', 'number', ''].includes(el.type)); \
                 if (!text) return 'not text'; \
                 el.scrollIntoView({{block: 'center', behavior: 'instant'}}); el.focus(); \
                 if (el.isContentEditable) {{ document.execCommand('selectAll'); }} \
                 else {{ el.value = ''; el.dispatchEvent(new Event('input', {{bubbles: true}})); }} \
                 return 'ready'; }})()"
            ))
            .await?;
        match ready.as_str() {
            Some("ready") => {}
            Some("not text") => {
                return Err(format!(
                    "element [{element}] does not take text; use browser_click"
                ));
            }
            _ => {
                return Err(format!(
                    "there is no element [{element}]; read the page again"
                ));
            }
        }
        self.send("Input.insertText", json!({ "text": text }))
            .await?;
        if submit {
            for kind in ["keyDown", "keyUp"] {
                self.send(
                    "Input.dispatchKeyEvent",
                    json!({ "type": kind, "key": "Enter", "code": "Enter", "windowsVirtualKeyCode": 13, "text": "\r" }),
                )
                .await?;
            }
            self.settle().await?;
        }
        Ok(())
    }

    async fn snapshot(&self) -> Result<String, String> {
        let mut tries = 0;
        loop {
            match self.evaluate(SNAPSHOT).await {
                Ok(Value::String(text)) => return Ok(text),
                Ok(_) => return Err(String::from("the page could not be read")),
                Err(error) if tries < 2 && error.contains("context") => {
                    tries += 1;
                    self.settle().await?;
                }
                Err(error) => return Err(error),
            }
        }
    }
}

impl Tool for BrowserTool {
    fn name(&self) -> &str {
        match self.action {
            Action::Open => "browser_open",
            Action::Read => "browser_read",
            Action::Click => "browser_click",
            Action::Type => "browser_type",
        }
    }

    fn description(&self) -> &str {
        match self.action {
            Action::Open => {
                "Opens a URL in dotori's browser, which keeps the owner's logins, and returns the page. \
                 Interactive elements are shown as [n]; use n with browser_click and browser_type."
            }
            Action::Read => {
                "Returns the page now open in the browser, with interactive elements as [n]."
            }
            Action::Click => "Clicks element [n] of the last page read, then returns the page.",
            Action::Type => {
                "Types text into element [n], replacing what it held, and presses Enter if `submit` is true. \
                 Returns the page."
            }
        }
    }

    fn input_schema(&self) -> Value {
        let start = json!({ "type": "integer", "minimum": 0, "description": "Character of the page to start from." });
        let element =
            json!({ "type": "integer", "minimum": 1, "description": "The [n] of the element." });
        match self.action {
            Action::Open => json!({
                "type": "object",
                "properties": { "url": { "type": "string" }, "start_index": start },
                "required": ["url"]
            }),
            Action::Read => json!({ "type": "object", "properties": { "start_index": start } }),
            Action::Click => json!({
                "type": "object",
                "properties": { "element": element, "start_index": start },
                "required": ["element"]
            }),
            Action::Type => json!({
                "type": "object",
                "properties": {
                    "element": element,
                    "text": { "type": "string" },
                    "submit": { "type": "boolean" },
                    "start_index": start
                },
                "required": ["element", "text"]
            }),
        }
    }

    fn call(&self, arguments: Value) -> ToolFuture<'_> {
        Box::pin(self.act(arguments))
    }
}

fn element(arguments: &Value) -> Result<u64, String> {
    arguments
        .get("element")
        .and_then(Value::as_u64)
        .ok_or_else(|| String::from("`element` is required"))
}

fn text_at(value: &Value, key: &str) -> Result<String, String> {
    value
        .get(key)
        .and_then(Value::as_str)
        .map(str::to_owned)
        .ok_or_else(|| format!("the browser gave no {key}"))
}

fn root() -> bool {
    cfg!(target_os = "linux")
        && std::fs::read_to_string("/proc/self/status").is_ok_and(|status| {
            status
                .lines()
                .find_map(|line| line.strip_prefix("Uid:"))
                .and_then(|ids| ids.split_whitespace().next())
                == Some("0")
        })
}

#[cfg(test)]
#[path = "browser.test.rs"]
mod tests;
