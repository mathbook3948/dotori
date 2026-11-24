use std::path::Path;
use std::path::PathBuf;
use std::process::Stdio;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use std::sync::atomic::Ordering;
use std::time::Duration;

use reqwest::Url;
use serde::Deserialize;
use serde::Serialize;
use serde_json::Value;
use serde_json::json;
use tokio::io::AsyncBufReadExt;
use tokio::io::BufReader;
use tokio::process::Child;
use tokio::process::Command;
use tokio::sync::Mutex;
use tokio::sync::mpsc;
use tokio::sync::watch;
use tokio::time;

use crate::Tool;
use crate::ToolFuture;
use crate::auth::Challenge;
use crate::auth::Challenges;
use crate::cdp::Cdp;
use crate::chrome;
use crate::fetch;
use crate::playwright::Driver;
use crate::screen::Ask;
use crate::screen::Screen;
use crate::script;
use crate::script::run::Native;

const START_WAIT: Duration = Duration::from_secs(20);
const LOAD_WAIT: Duration = Duration::from_secs(20);
const SETTLE: Duration = Duration::from_millis(400);
const POLL: Duration = Duration::from_millis(200);
const SNAPSHOT: &str = include_str!("snapshot.js");
const SHOWN: usize = 20_000;
const CLOSE_WAIT: Duration = Duration::from_secs(10);
const RUN_WAIT: Duration = Duration::from_secs(100);
const WIDTH: u32 = 1280;
const HEIGHT: u32 = 800;
const WORLD: &str = "dotori";
const AUTH_WAIT: Duration = Duration::from_secs(90);
const LOGINS_EVERY: Duration = Duration::from_secs(60);

/// How the browser runs. The profile folder keeps logins between runs. Without an executable,
/// dotori's own Chrome for Testing in `downloads` is used, fetched there on first use. The
/// browser has no window of its own: the owner sees it on the chat window's screen.
#[derive(Clone, Debug)]
pub struct BrowserConfig {
    pub executable: Option<PathBuf>,
    pub downloads: PathBuf,
    pub profile: PathBuf,
    /// Where the browser's logins go out to the owner's other PCs, and where theirs come in.
    pub logins: Option<Logins>,
}

/// Files that carry the browser's cookies between the owner's PCs. Only the user can read them.
/// `out` is written while this PC's browser is open; `into` is what another PC's browser had,
/// taken in when this browser starts if it is newer than what this browser last wrote.
#[derive(Clone, Debug)]
pub struct Logins {
    pub out: PathBuf,
    pub into: PathBuf,
}

/// A copy of a browser's cookies and when it was made, in milliseconds since the Unix epoch.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct LoginCopy {
    pub at: i64,
    pub cookies: Vec<Value>,
}

/// One browser shared by every browser tool. It starts on first use and starts again if it died.
pub struct Browser {
    config: BrowserConfig,
    install: Arc<std::sync::Mutex<Install>>,
    running: Mutex<Option<Running>>,
    acting: Arc<Mutex<()>>,
    screen: Arc<Screen>,
    shown: Arc<std::sync::Mutex<Option<Shown>>>,
    owner: Mutex<Option<tokio::sync::OwnedMutexGuard<()>>>,
    logins: SharedChallenges,
    taking: AtomicBool,
    gave: watch::Sender<u64>,
}

type SharedChallenges = Arc<std::sync::Mutex<Challenges<Arc<Cdp>>>>;

impl std::fmt::Debug for Browser {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Browser")
            .field("config", &self.config)
            .finish_non_exhaustive()
    }
}

#[derive(Clone)]
struct Shown {
    cdp: Arc<Cdp>,
    main: String,
    popup: Option<(String, String)>,
}

impl Shown {
    fn session(&self) -> &str {
        self.popup
            .as_ref()
            .map_or(&self.main, |(_, session)| session)
    }
}

#[derive(Debug)]
enum Install {
    Fetching,
    Ready(PathBuf),
    Failed(String),
}

pub(crate) struct Running {
    child: Child,
    cdp: Arc<Cdp>,
    session: String,
    frame: String,
    world: Mutex<Option<i64>>,
    watcher: tokio::task::JoinHandle<()>,
    pub(crate) screen: Arc<Screen>,
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
    Run,
    Read,
    Ask,
}

/// One of the browser tools, all driving the same browser.
#[derive(Debug)]
pub struct BrowserTool {
    browser: Arc<Browser>,
    action: Action,
}

impl Browser {
    /// Starts fetching dotori's own browser in the background unless it is there or another one
    /// is set, so the first page does not wait for the download. The browser shows itself on
    /// `screen` while a chat window watches.
    pub fn new(config: BrowserConfig) -> Arc<Self> {
        let (screen, asks) = Screen::new();
        Self::with_screen(config, screen, asks)
    }

    pub fn with_screen(
        config: BrowserConfig,
        screen: Arc<Screen>,
        asks: mpsc::UnboundedReceiver<Ask>,
    ) -> Arc<Self> {
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
        let browser = Arc::new(Self {
            config,
            install,
            running: Mutex::new(None),
            acting: Arc::new(Mutex::new(())),
            screen,
            shown: Arc::new(std::sync::Mutex::new(None)),
            owner: Mutex::new(None),
            logins: Arc::default(),
            taking: AtomicBool::new(false),
            gave: watch::Sender::new(0),
        });
        if tokio::runtime::Handle::try_current().is_ok() {
            tokio::spawn(Arc::clone(&browser).listen(asks));
        }
        browser
    }

    async fn listen(self: Arc<Self>, mut asks: mpsc::UnboundedReceiver<Ask>) {
        while let Some(ask) = asks.recv().await {
            let done = match ask {
                Ask::Watch(on) => {
                    if !on {
                        self.gave.send_modify(|gave| *gave += 1);
                    }
                    if !on && self.owner.lock().await.take().is_some() {
                        self.screen.update(json!({ "mode": "idle" }));
                    }
                    self.cast(on).await
                }
                Ask::Take => {
                    if !self.taking.swap(true, Ordering::SeqCst) {
                        let browser = Arc::clone(&self);
                        let since = *self.gave.borrow();
                        tokio::spawn(async move {
                            if let Err(error) = browser.take(since).await {
                                browser.screen.update(json!({ "error": error }));
                            }
                            browser.taking.store(false, Ordering::SeqCst);
                        });
                    }
                    Ok(())
                }
                Ask::Give => {
                    self.gave.send_modify(|gave| *gave += 1);
                    self.owner.lock().await.take();
                    self.screen.update(json!({ "mode": "idle", "error": null }));
                    Ok(())
                }
                Ask::Input(input) if input.get("kind").and_then(Value::as_str) == Some("auth") => {
                    self.auth(&input).await
                }
                Ask::Input(input) => {
                    if self.owner.lock().await.is_some() {
                        self.owner_input(&input).await
                    } else {
                        Ok(())
                    }
                }
            };
            if let Err(error) = done {
                self.screen.update(json!({ "error": error }));
            }
        }
    }

    async fn cast(&self, on: bool) -> Result<(), String> {
        let shown = self.shown.lock().expect("unpoisoned").clone();
        let Some(shown) = shown else {
            return Ok(());
        };
        let (cdp, session) = (shown.cdp.clone(), shown.session().to_owned());
        let (method, params) = if on {
            ("Page.startScreencast", screencast())
        } else {
            ("Page.stopScreencast", json!({}))
        };
        cdp.send(method, params, Some(&session)).await.map(drop)
    }

    async fn take(&self, since: u64) -> Result<(), String> {
        let mut gave = self.gave.subscribe();
        if self.owner.lock().await.is_some() {
            self.screen.update(json!({ "mode": "owner" }));
            return Ok(());
        }
        self.screen.update(json!({ "mode": "waiting" }));
        let held = tokio::select! {
            held = Arc::clone(&self.acting).lock_owned() => held,
            _ = gave.wait_for(|now| *now != since) => {
                self.screen.update(json!({ "mode": "idle" }));
                return Ok(());
            }
        };
        if let Err(error) = self.page().await {
            drop(held);
            self.screen.update(json!({ "mode": "idle" }));
            return Err(error);
        }
        let mut owner = self.owner.lock().await;
        if *self.gave.borrow() != since {
            drop(held);
            self.screen.update(json!({ "mode": "idle" }));
            return Ok(());
        }
        *owner = Some(held);
        drop(owner);
        self.screen
            .update(json!({ "mode": "owner", "asked": null, "error": null }));
        Ok(())
    }

    async fn owner_input(&self, input: &Value) -> Result<(), String> {
        let shown = self.shown.lock().expect("unpoisoned").clone();
        let Some(shown) = shown else {
            return Ok(());
        };
        let (cdp, session) = (shown.cdp.clone(), shown.session().to_owned());
        if input.get("kind").and_then(Value::as_str) == Some("pick") {
            return self.pick(&cdp, &session, input).await;
        }
        if input.get("kind").and_then(Value::as_str) == Some("mouse")
            && input.get("event").and_then(Value::as_str) == Some("down")
        {
            let x = input.get("x").and_then(Value::as_f64).unwrap_or(0.0);
            let y = input.get("y").and_then(Value::as_f64).unwrap_or(0.0);
            let found = in_world(&cdp, &session, &format!("({PICKABLE})({x}, {y})")).await?;
            if let Some(picker) = found.as_object() {
                let mut picker = picker.clone();
                picker.insert(String::from("x"), json!(x));
                picker.insert(String::from("y"), json!(y));
                self.screen.update(json!({ "picker": picker }));
                return Ok(());
            }
        }
        if let Some((method, params)) = owner_event(input) {
            cdp.send(method, params, Some(&session)).await?;
        }
        Ok(())
    }

    async fn auth(&self, input: &Value) -> Result<(), String> {
        let Some(id) = input.get("id").and_then(Value::as_u64) else {
            return Ok(());
        };
        let login = {
            let mut logins = self.logins.lock().expect("unpoisoned");
            let login = logins.take(id);
            self.screen.update(json!({ "login": logins.shown() }));
            login
        };
        let Some(login) = login else {
            return Ok(());
        };
        let given = match (
            input.get("username").and_then(Value::as_str),
            input.get("password").and_then(Value::as_str),
        ) {
            (Some(username), Some(password))
                if input.get("cancel").and_then(Value::as_bool) != Some(true) =>
            {
                Some((username, password))
            }
            _ => None,
        };
        answer_auth(&login, given).await
    }

    async fn pick(&self, cdp: &Cdp, session: &str, input: &Value) -> Result<(), String> {
        let picker = self.screen.state.lock().expect("unpoisoned")["picker"].clone();
        self.screen.update(json!({ "picker": null }));
        if input.get("cancel").and_then(Value::as_bool) == Some(true) || picker.is_null() {
            return Ok(());
        }
        match picker.get("kind").and_then(Value::as_str) {
            Some("files") => {
                let files = input.get("files").cloned().unwrap_or_else(|| json!([]));
                let node = picker.get("node").cloned().unwrap_or(Value::Null);
                let at = picker
                    .get("session")
                    .and_then(Value::as_str)
                    .unwrap_or(session);
                cdp.send(
                    "DOM.setFileInputFiles",
                    json!({ "files": files, "backendNodeId": node }),
                    Some(at),
                )
                .await
                .map(drop)
            }
            Some("select" | "value") => {
                let x = picker.get("x").and_then(Value::as_f64).unwrap_or(0.0);
                let y = picker.get("y").and_then(Value::as_f64).unwrap_or(0.0);
                let chosen = input.get("value").cloned().unwrap_or(Value::Null);
                in_world(cdp, session, &format!("({CHOOSE})({x}, {y}, {chosen})"))
                    .await
                    .map(drop)
            }
            _ => Ok(()),
        }
    }

    /// Writes the open browser's cookies for the owner's other PCs every so often, while this
    /// PC holds the browser. A browser that is not open is not started for it.
    pub async fn share_logins(self: Arc<Self>, holds: impl Fn() -> bool) {
        let Some(logins) = self.config.logins.clone() else {
            return;
        };
        let mut every = time::interval(LOGINS_EVERY);
        let mut written: Option<Vec<Value>> = None;
        loop {
            every.tick().await;
            if !holds() {
                continue;
            }
            let cdp = match self.running.lock().await.as_ref() {
                Some(running) if running.cdp.open() => Arc::clone(&running.cdp),
                _ => continue,
            };
            take_logins(&cdp, &logins).await;
            let Ok(reply) = cdp.send("Storage.getCookies", json!({}), None).await else {
                continue;
            };
            let cookies = reply
                .get("cookies")
                .and_then(Value::as_array)
                .cloned()
                .unwrap_or_default();
            if written.as_ref() == Some(&cookies) {
                continue;
            }
            let copy = LoginCopy {
                at: now_ms(),
                cookies: cookies.clone(),
            };
            match write_privately(&logins.out, &copy) {
                Ok(()) => written = Some(cookies),
                Err(error) => eprintln!("dotori: the browser's logins were not kept: {error}"),
            }
        }
    }

    /// The tools that drive this browser.
    pub fn tools(self: &Arc<Self>) -> Vec<Box<dyn Tool>> {
        [Action::Run, Action::Read, Action::Ask]
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
        command
            .arg("--headless=new")
            .arg(format!("--window-size={WIDTH},{HEIGHT}"));
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
        self.logins.lock().expect("unpoisoned").clear();
        self.screen.update(json!({ "login": null }));
        let watcher = tokio::spawn(watch(
            Arc::clone(&cdp),
            events,
            Arc::clone(&self.screen),
            Arc::clone(&self.shown),
            Arc::clone(&self.logins),
            target.clone(),
        ));
        let page = |method: &'static str, params: Value| {
            let cdp = Arc::clone(&cdp);
            let session = session.clone();
            async move { cdp.send(method, params, Some(&session)).await }
        };
        page("Page.enable", json!({})).await?;
        page(
            "Fetch.enable",
            json!({
                "patterns": [{ "resourceType": "Document", "requestStage": "Request" }],
                "handleAuthRequests": true,
            }),
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
        page(
            "Page.setInterceptFileChooserDialog",
            json!({ "enabled": true }),
        )
        .await?;
        cdp.send(
            "Target.setDiscoverTargets",
            json!({ "discover": true }),
            None,
        )
        .await?;
        cdp.send(
            "Target.setAutoAttach",
            json!({ "autoAttach": true, "waitForDebuggerOnStart": true, "flatten": true }),
            None,
        )
        .await?;
        *self.shown.lock().expect("unpoisoned") = Some(Shown {
            cdp: Arc::clone(&cdp),
            main: session.clone(),
            popup: None,
        });
        if self.screen.watched() {
            page("Page.startScreencast", screencast()).await?;
        }
        if let Some(logins) = &self.config.logins {
            take_logins(&cdp, logins).await;
        }
        Ok(Running {
            child,
            cdp,
            session,
            frame,
            world: Mutex::new(None),
            watcher,
            screen: Arc::clone(&self.screen),
        })
    }
}

async fn take_logins(cdp: &Cdp, logins: &Logins) {
    let Some(theirs) = read_copy(&logins.into) else {
        return;
    };
    let taken_mark = logins.into.with_extension("taken");
    let taken: i64 = std::fs::read_to_string(&taken_mark)
        .ok()
        .and_then(|text| text.trim().parse().ok())
        .unwrap_or(0);
    let ours = read_copy(&logins.out).map_or(0, |ours| ours.at);
    if theirs.at <= ours.max(taken) {
        return;
    }
    if let Err(error) = cdp.send("Storage.clearCookies", json!({}), None).await {
        eprintln!("dotori: the logins from another PC were not taken: {error}");
        return;
    }
    let mut refused = 0;
    for cookie in theirs.cookies.iter().map(settable) {
        if cdp
            .send("Storage.setCookies", json!({ "cookies": [cookie] }), None)
            .await
            .is_err()
        {
            refused += 1;
        }
    }
    if refused > 0 {
        eprintln!("dotori: {refused} cookies from another PC did not fit this browser");
    }
    if let Err(error) = std::fs::write(&taken_mark, theirs.at.to_string()) {
        eprintln!("dotori: {}: {error}", taken_mark.display());
    }
    if read_copy(&logins.into).is_some_and(|now| now.at == theirs.at) {
        let _ = std::fs::remove_file(&logins.into);
    }
}

/// A cookie as `Storage.getCookies` gives it, cut to what `Storage.setCookies` takes.
pub(crate) fn settable(cookie: &Value) -> Value {
    const KEPT: [&str; 11] = [
        "name",
        "value",
        "domain",
        "path",
        "secure",
        "httpOnly",
        "sameSite",
        "expires",
        "priority",
        "sourceScheme",
        "partitionKey",
    ];
    let mut kept = serde_json::Map::new();
    for key in KEPT {
        if let Some(value) = cookie.get(key) {
            kept.insert(key.to_owned(), value.clone());
        }
    }
    if cookie.get("session").and_then(Value::as_bool) == Some(true) {
        kept.remove("expires");
    }
    Value::Object(kept)
}

fn read_copy(path: &std::path::Path) -> Option<LoginCopy> {
    serde_json::from_str(&std::fs::read_to_string(path).ok()?).ok()
}

fn write_privately(path: &std::path::Path, copy: &LoginCopy) -> Result<(), String> {
    use std::io::Write;
    let text = serde_json::to_vec(copy).map_err(|error| error.to_string())?;
    let draft = path.with_extension("json.new");
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options.open(&draft).map_err(|error| error.to_string())?;
    file.write_all(&text).map_err(|error| error.to_string())?;
    file.sync_all().map_err(|error| error.to_string())?;
    std::fs::rename(&draft, path).map_err(|error| error.to_string())
}

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|since| i64::try_from(since.as_millis()).unwrap_or(i64::MAX))
        .unwrap_or(0)
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

async fn watch(
    cdp: Arc<Cdp>,
    mut events: tokio::sync::mpsc::UnboundedReceiver<Value>,
    screen: Arc<Screen>,
    shown: Arc<std::sync::Mutex<Option<Shown>>>,
    logins: SharedChallenges,
    main_target: String,
) {
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
            Some("Target.attachedToTarget") => {
                let info = params.get("targetInfo").cloned().unwrap_or(Value::Null);
                let attached = params
                    .get("sessionId")
                    .and_then(Value::as_str)
                    .map(str::to_owned);
                let target = info
                    .get("targetId")
                    .and_then(Value::as_str)
                    .map(str::to_owned);
                let opened_here = info.get("type").and_then(Value::as_str) == Some("page");
                let ours = target.as_deref() == Some(main_target.as_str());
                if let (false, Some(attached), Some(target)) = (ours, attached, target) {
                    let cdp = Arc::clone(&cdp);
                    let screen = Arc::clone(&screen);
                    let shown = Arc::clone(&shown);
                    tokio::spawn(async move {
                        if opened_here {
                            if let Err(error) =
                                follow_popup(&cdp, &screen, &shown, target, attached).await
                            {
                                screen.update(json!({ "error": error }));
                            }
                        } else {
                            let _ = cdp
                                .send(
                                    "Runtime.runIfWaitingForDebugger",
                                    json!({}),
                                    Some(&attached),
                                )
                                .await;
                            let _ = cdp
                                .send(
                                    "Target.detachFromTarget",
                                    json!({ "sessionId": attached }),
                                    None,
                                )
                                .await;
                        }
                    });
                }
                None
            }
            Some("Target.targetDestroyed") => {
                let gone = params
                    .get("targetId")
                    .and_then(Value::as_str)
                    .unwrap_or_default();
                let back = {
                    let mut held = shown.lock().expect("unpoisoned");
                    match held.as_mut() {
                        Some(current)
                            if current
                                .popup
                                .as_ref()
                                .is_some_and(|(target, _)| target == gone) =>
                        {
                            current.popup = None;
                            Some(current.main.clone())
                        }
                        _ => None,
                    }
                };
                if let Some(main) = back {
                    screen.update(json!({ "popup": false, "picker": null }));
                    if screen.watched() {
                        let _ = cdp
                            .send("Page.startScreencast", screencast(), Some(&main))
                            .await;
                    }
                }
                None
            }
            Some("Page.fileChooserOpened") => {
                screen.update(json!({ "picker": {
                    "kind": "files",
                    "multiple": params.get("mode").and_then(Value::as_str) == Some("selectMultiple"),
                    "node": params.get("backendNodeId").cloned().unwrap_or(Value::Null),
                    "session": session,
                } }));
                None
            }
            Some("Page.screencastFrame") => {
                let meta = |name: &str| {
                    params
                        .pointer(&format!("/metadata/{name}"))
                        .and_then(Value::as_f64)
                        .unwrap_or(0.0)
                };
                if let Some(jpeg) = params.get("data").and_then(Value::as_str) {
                    screen.frame(jpeg, meta("deviceWidth"), meta("deviceHeight"));
                }
                let id = params.get("sessionId").cloned().unwrap_or(Value::Null);
                Some(("Page.screencastFrameAck", json!({ "sessionId": id })))
            }
            Some("Page.frameNavigated") if params.pointer("/frame/parentId").is_none() => {
                if let Some(url) = params.pointer("/frame/url").and_then(Value::as_str) {
                    screen.update(json!({ "url": url }));
                }
                None
            }
            Some("Fetch.authRequired") => {
                let challenge = params.get("authChallenge").cloned().unwrap_or(Value::Null);
                let id = {
                    let mut waiting = logins.lock().expect("unpoisoned");
                    let id = waiting.ask(
                        Arc::clone(&cdp),
                        params.get("requestId").cloned().unwrap_or(Value::Null),
                        session.map(str::to_owned),
                        challenge
                            .get("origin")
                            .and_then(Value::as_str)
                            .unwrap_or_default()
                            .to_owned(),
                        challenge.get("realm").cloned().unwrap_or(Value::Null),
                    );
                    screen.update(json!({ "login": waiting.shown() }));
                    id
                };
                let screen = Arc::clone(&screen);
                let logins = Arc::clone(&logins);
                tokio::spawn(async move {
                    time::sleep(AUTH_WAIT).await;
                    let lapsed = {
                        let mut waiting = logins.lock().expect("unpoisoned");
                        let lapsed = waiting.take(id);
                        if lapsed.is_some() {
                            screen.update(json!({ "login": waiting.shown() }));
                        }
                        lapsed
                    };
                    if let Some(lapsed) = lapsed {
                        let _ = answer_auth(&lapsed, None).await;
                    }
                });
                None
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

async fn answer_auth(
    login: &Challenge<Arc<Cdp>>,
    given: Option<(&str, &str)>,
) -> Result<(), String> {
    let response = match given {
        Some((username, password)) => json!({
            "response": "ProvideCredentials",
            "username": username,
            "password": password,
        }),
        None => json!({ "response": "CancelAuth" }),
    };
    login
        .handle
        .send(
            "Fetch.continueWithAuth",
            json!({ "requestId": login.request, "authChallengeResponse": response }),
            login.session.as_deref(),
        )
        .await
        .map(drop)
}

/// Whether a page may be loaded from `url`, by the same rule as webfetch.
pub(crate) fn document_allowed(url: &str) -> Result<(), String> {
    let url = Url::parse(url).map_err(|error| error.to_string())?;
    fetch::allowed(&url)
}

impl BrowserTool {
    async fn act(&self, arguments: Value) -> Result<String, String> {
        if self.action == Action::Ask {
            let reason = arguments
                .get("reason")
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|reason| !reason.is_empty())
                .ok_or("`reason` is required")?;
            self.browser.screen.update(json!({ "asked": reason }));
            return Ok(String::from(
                "Asked the owner. They take the browser over from the chat window's browser tab and \
                 hand it back when done. Do not wait for them: tell the owner what you need, and \
                 read the page again later.",
            ));
        }
        let start = match arguments.get("start_index") {
            None => 0,
            Some(start) => start
                .as_u64()
                .and_then(|start| usize::try_from(start).ok())
                .ok_or("`start_index` must be a whole number from 0")?,
        };
        let script = match self.action {
            Action::Read | Action::Ask => None,
            Action::Run => {
                let code = arguments
                    .get("code")
                    .and_then(Value::as_str)
                    .ok_or("`code` is required")?;
                Some(
                    script::parse::parse(code)
                        .map_err(|error| format!("the code does not parse: {error}"))?,
                )
            }
        };
        if self.browser.owner.lock().await.is_some() {
            return Err(String::from(
                "the owner is using the browser now; wait until they hand it back",
            ));
        }
        let _acting = self.browser.acting.lock().await;
        self.browser.screen.update(json!({ "mode": "agent" }));
        let running = self.browser.page().await?;
        let page = Page { running: &running };
        let said = match &script {
            None => Ok(String::new()),
            Some(body) => {
                let driver = Driver {
                    page: Page { running: &running },
                };
                let home = Native {
                    kind: "page",
                    data: Value::Null,
                };
                match time::timeout(RUN_WAIT, script::run::run(body, &driver, home)).await {
                    Ok(Ok(outcome)) => Ok(said(&outcome)),
                    Ok(Err(error)) => Err(error),
                    Err(_) => Err(format!(
                        "the code did not finish within {} seconds",
                        RUN_WAIT.as_secs()
                    )),
                }
            }
        };
        let shown = match page.guard().await {
            Ok(()) => page.snapshot().await,
            Err(error) => Err(error),
        };
        let broken = !running.cdp.open();
        drop(running);
        self.browser.screen.update(json!({ "mode": "idle" }));
        if let Err(error) = &shown
            && (broken || error.contains("not found"))
        {
            self.browser.forget().await;
        }
        let shown = shown.map(|shown| fetch::window(&shown, start, SHOWN));
        match (said, shown) {
            (Ok(said), Ok(shown)) => Ok(format!("{said}{shown}")),
            (Err(error), Ok(shown)) => Err(format!("{error}\n\nThe page now:\n{shown}")),
            (Err(error), Err(page)) => Err(format!("{error}\n\nThe page cannot be shown: {page}")),
            (Ok(said), Err(page)) => Err(format!("{said}{page}")),
        }
    }
}

fn said(outcome: &script::run::Outcome) -> String {
    let mut said = String::new();
    for line in &outcome.logs {
        said.push_str(line);
        said.push('\n');
    }
    if let Some(value) = &outcome.value {
        said.push_str("Returned: ");
        said.push_str(value);
        said.push('\n');
    }
    if !said.is_empty() {
        said.push_str("\n---\n");
    }
    said
}

/// One page of the browser, for the tools and for scripts.
pub(crate) struct Page<'a> {
    pub(crate) running: &'a Running,
}

impl Page<'_> {
    pub(crate) async fn send(&self, method: &str, params: Value) -> Result<Value, String> {
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

    pub(crate) async fn evaluate(&self, expression: &str) -> Result<Value, String> {
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

    pub(crate) async fn go(&self, url: &str) -> Result<(), String> {
        let reply = self.send("Page.navigate", json!({ "url": url })).await?;
        if let Some(error) = reply.get("errorText").and_then(Value::as_str) {
            return Err(format!("could not open {url}: {error}"));
        }
        self.settle().await
    }

    pub(crate) async fn settle(&self) -> Result<(), String> {
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

    pub(crate) async fn guard(&self) -> Result<(), String> {
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

    pub(crate) async fn snapshot(&self) -> Result<String, String> {
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
            Action::Run => "browser_run",
            Action::Read => "browser_read",
            Action::Ask => "browser_ask_owner",
        }
    }

    fn description(&self) -> &str {
        match self.action {
            Action::Run => {
                "Runs Playwright-style JavaScript in dotori's browser, which keeps the owner's logins. \
                 `page` is ready to use: page.goto(url); locators from page.getByRole(role, { name }), \
                 getByText, getByLabel, getByPlaceholder, getByTestId, or page.locator(css), where \
                 `ref=N` picks element [N] of the last page read; locator.click(), fill(text), \
                 press(key), check(), selectOption(value), textContent(), innerText(), inputValue(), \
                 count(), all(), first(), nth(i), filter({ hasText }), waitFor(); page.keyboard.press(key); \
                 page.waitForURL(url); page.evaluate(fn, arg) runs fn inside the page. Each action waits \
                 for its element, and a locator that matches several elements is refused. Use await as in \
                 Playwright. What console.log prints and what the code returns come first in the result, \
                 then the page as browser_read shows it."
            }
            Action::Read => {
                "Returns the page now open in the browser, with interactive elements as [n]."
            }
            Action::Ask => {
                "Asks the owner to take the browser over in the chat window, such as to log in, \
                 since dotori never handles passwords. Returns at once; the owner hands it back \
                 when done."
            }
        }
    }

    fn input_schema(&self) -> Value {
        let start = json!({ "type": "integer", "minimum": 0, "description": "Character of the page to start from." });
        match self.action {
            Action::Run => json!({
                "type": "object",
                "properties": {
                    "code": { "type": "string", "description": "The JavaScript to run, such as `await page.goto('https://example.com'); return await page.title();`." },
                    "start_index": start
                },
                "required": ["code"]
            }),
            Action::Read => json!({ "type": "object", "properties": { "start_index": start } }),
            Action::Ask => json!({
                "type": "object",
                "properties": { "reason": { "type": "string", "description": "What the owner should do, such as `Log in to the bank`." } },
                "required": ["reason"]
            }),
        }
    }

    fn call(&self, arguments: Value) -> ToolFuture<'_> {
        Box::pin(self.act(arguments))
    }
}

async fn in_world(cdp: &Cdp, session: &str, expression: &str) -> Result<Value, String> {
    let tree = cdp
        .send("Page.getFrameTree", json!({}), Some(session))
        .await?;
    let frame = tree
        .pointer("/frameTree/frame/id")
        .and_then(Value::as_str)
        .ok_or("the page has no frame")?;
    let world = cdp
        .send(
            "Page.createIsolatedWorld",
            json!({ "frameId": frame, "worldName": WORLD }),
            Some(session),
        )
        .await?;
    let context = world
        .get("executionContextId")
        .and_then(Value::as_i64)
        .ok_or("the browser made no world for dotori's scripts")?;
    let reply = cdp
        .send(
            "Runtime.evaluate",
            json!({ "expression": expression, "returnByValue": true, "contextId": context }),
            Some(session),
        )
        .await?;
    Ok(reply
        .pointer("/result/value")
        .cloned()
        .unwrap_or(Value::Null))
}

async fn follow_popup(
    cdp: &Arc<Cdp>,
    screen: &Screen,
    shown: &std::sync::Mutex<Option<Shown>>,
    target: String,
    session: String,
) -> Result<(), String> {
    let page = |method: &'static str, params: Value| {
        let cdp = Arc::clone(cdp);
        let session = session.clone();
        async move { cdp.send(method, params, Some(&session)).await }
    };
    page(
        "Fetch.enable",
        json!({
            "patterns": [{ "resourceType": "Document", "requestStage": "Request" }],
            "handleAuthRequests": true,
        }),
    )
    .await?;
    page("Runtime.runIfWaitingForDebugger", json!({})).await?;
    let version = cdp.send("Browser.getVersion", json!({}), None).await?;
    if let Some(agent) = version.get("userAgent").and_then(Value::as_str)
        && agent.contains("HeadlessChrome")
    {
        page("Emulation.setUserAgentOverride", plain_agent(agent)).await?;
    }
    page("Page.enable", json!({})).await?;
    page(
        "Page.setInterceptFileChooserDialog",
        json!({ "enabled": true }),
    )
    .await?;
    let main = {
        let mut held = shown.lock().expect("unpoisoned");
        let Some(current) = held.as_mut() else {
            return Ok(());
        };
        current.popup = Some((target, session.clone()));
        current.main.clone()
    };
    screen.update(json!({ "popup": true, "picker": null }));
    if screen.watched() {
        let _ = cdp
            .send("Page.stopScreencast", json!({}), Some(&main))
            .await;
        page("Page.startScreencast", screencast()).await?;
    }
    Ok(())
}

const PICKABLE: &str = "(x, y) => {
  const el = document.elementFromPoint(x, y);
  const select = el && el.closest('select');
  if (select && !select.disabled) {
    select.focus();
    return { kind: 'select', multiple: select.multiple, selected: select.selectedIndex,
      options: Array.from(select.options).map((o) => ({ label: o.label || o.text, disabled: o.disabled })) };
  }
  const input = el && el.closest('input');
  if (input && !input.disabled && ['date', 'time', 'datetime-local', 'month', 'week', 'color'].includes(input.type)) {
    input.focus();
    return { kind: 'value', type: input.type, value: input.value };
  }
  return null;
}";

const CHOOSE: &str = "(x, y, chosen) => {
  const el = document.elementFromPoint(x, y);
  const select = el && el.closest('select');
  const input = el && el.closest('input');
  if (select && typeof chosen === 'number') select.selectedIndex = chosen;
  else if (input && typeof chosen === 'string') input.value = chosen;
  else return false;
  const target = select || input;
  target.dispatchEvent(new Event('input', { bubbles: true }));
  target.dispatchEvent(new Event('change', { bubbles: true }));
  return true;
}";

fn screencast() -> Value {
    json!({ "format": "jpeg", "quality": 70, "maxWidth": WIDTH, "maxHeight": HEIGHT, "everyNthFrame": 1 })
}

/// The DevTools input event for what a chat window sent while the owner holds the browser.
pub(crate) fn owner_event(input: &Value) -> Option<(&'static str, Value)> {
    let number = |name: &str| input.get(name).and_then(Value::as_f64).unwrap_or(0.0);
    let modifiers = input.get("modifiers").and_then(Value::as_i64).unwrap_or(0);
    match input.get("kind").and_then(Value::as_str)? {
        "mouse" => {
            let kind = match input.get("event").and_then(Value::as_str)? {
                "move" => "mouseMoved",
                "down" => "mousePressed",
                "up" => "mouseReleased",
                _ => return None,
            };
            let button = input
                .get("button")
                .and_then(Value::as_str)
                .unwrap_or("none");
            Some((
                "Input.dispatchMouseEvent",
                json!({
                    "type": kind,
                    "x": number("x"),
                    "y": number("y"),
                    "button": button,
                    "clickCount": number("clicks").max(if kind == "mouseMoved" { 0.0 } else { 1.0 }),
                    "modifiers": modifiers
                }),
            ))
        }
        "wheel" => Some((
            "Input.dispatchMouseEvent",
            json!({
                "type": "mouseWheel",
                "x": number("x"),
                "y": number("y"),
                "deltaX": number("dx"),
                "deltaY": number("dy"),
                "modifiers": modifiers
            }),
        )),
        "key" => {
            let key = input.get("key").and_then(Value::as_str)?;
            let text = input
                .get("text")
                .and_then(Value::as_str)
                .unwrap_or_default();
            let kind = match (input.get("event").and_then(Value::as_str)?, text.is_empty()) {
                ("down", true) => "rawKeyDown",
                ("down", false) => "keyDown",
                ("up", _) => "keyUp",
                _ => return None,
            };
            let mut event = json!({
                "type": kind,
                "key": key,
                "windowsVirtualKeyCode": number("code"),
                "modifiers": modifiers
            });
            if kind == "keyDown" {
                event["text"] = json!(text);
            }
            let command = match key.to_ascii_lowercase().as_str() {
                "a" => Some("selectAll"),
                "c" => Some("copy"),
                "x" => Some("cut"),
                "z" if modifiers & 8 != 0 => Some("redo"),
                "z" => Some("undo"),
                "y" => Some("redo"),
                _ => None,
            };
            if kind == "rawKeyDown"
                && modifiers & (2 | 4) != 0
                && let Some(command) = command
            {
                event["commands"] = json!([command]);
            }
            Some(("Input.dispatchKeyEvent", event))
        }
        "text" => Some((
            "Input.insertText",
            json!({ "text": input.get("text").and_then(Value::as_str)? }),
        )),
        "compose" => {
            let text = input.get("text").and_then(Value::as_str)?;
            let end = text.encode_utf16().count();
            Some((
                "Input.imeSetComposition",
                json!({ "text": text, "selectionStart": end, "selectionEnd": end }),
            ))
        }
        _ => None,
    }
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
