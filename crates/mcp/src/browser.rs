use std::collections::HashMap;
use std::collections::HashSet;
use std::path::Path;
use std::path::PathBuf;
use std::process::Stdio;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use std::sync::atomic::AtomicU64;
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
use tokio::sync::oneshot;
use tokio::sync::watch;
use tokio::time;

use crate::Tool;
use crate::ToolFuture;
use crate::auth::Challenge;
use crate::auth::Challenges;
use crate::card;
use crate::card::Account;
use crate::card::Accounts;
use crate::card::Answer;
use crate::card::FillSpec;
use crate::card::Hidden;
use crate::card::Kept;
use crate::card::LogInSpec;
use crate::card::Role;
use crate::cdp::Cdp;
use crate::chrome;
use crate::fetch;
use crate::playwright::Driver;
use crate::playwright::key_events;
use crate::screen::Ask;
use crate::screen::Screen;
use crate::script;
use crate::script::run::Native;

const START_WAIT: Duration = Duration::from_secs(20);
const LOAD_WAIT: Duration = Duration::from_secs(20);
const SETTLE: Duration = Duration::from_millis(400);
const POLL: Duration = Duration::from_millis(200);
const SNAPSHOT: &str = include_str!("snapshot.js");
const FILL_JS: &str = include_str!("fill.js");
const SHOWN: usize = 20_000;
const CLOSE_WAIT: Duration = Duration::from_secs(10);
const RUN_WAIT: Duration = Duration::from_secs(100);
const WIDTH: u32 = 1280;
const HEIGHT: u32 = 800;
const WORLD: &str = "dotori";
const PIPED: &str = r#"exec "$0" "$@" 3<&0 4>&1 </dev/null >/dev/null"#;

/// The tool that runs Playwright-style code in dotori's browser.
pub const RUN: &str = "run_in_browser";
/// The tool that reads the page open in dotori's browser.
pub const READ: &str = "read_page";
/// The tool that asks the owner to take the browser over.
pub const ASK: &str = "ask_owner_in_browser";
/// The tool that shows the owner a card to fill the page's fields.
pub const FILL: &str = "ask_owner_to_fill";
/// The tool that logs in with an account the vault keeps.
pub const FILL_LOGIN: &str = "fill_login";
/// Every browser tool, which run on the PC that holds the browser.
pub const TOOLS: [&str; 5] = [RUN, READ, ASK, FILL, FILL_LOGIN];
/// The browser tools only the main session has, since they reach the owner in the chat window.
pub const OWNER_TOOLS: [&str; 3] = [ASK, FILL, FILL_LOGIN];
/// The browser tools that may wait for the owner's answer in the chat window.
pub const WAITS_FOR_OWNER: [&str; 2] = [FILL, FILL_LOGIN];
/// How long a card waits for the owner before it closes.
pub const CARD_WAIT: Duration = Duration::from_secs(240);
const AUTH_WAIT: Duration = Duration::from_secs(90);
const PRESS_PAUSE: Duration = Duration::from_millis(150);
const SEAL_WAIT: Duration = Duration::from_secs(2);
const TOUCHED: &str = "code ran on this page since it loaded, so dotori shows no card and fills \
                       nothing on it. Load it again with read_page and its `url`, then call this again \
                       without running code on it first.";
const CHANGED: &str = "the page changed since the card was shown, so dotori stopped; read the page \
                       and show the owner a new card";
const OTHER_WINDOW: &str = "another window of the browser opened since the card was shown, so \
                            dotori typed nothing; read the page and show the owner the card again";
const STILL_SEALED: &str = "dotori typed into this page, so no code of yours runs on it until it \
                            loads again; read_page with its `url` loads it again.";
const SEALED: &str = "dotori typed the owner's words into this page, so no code runs on it until it \
                      loads again; page.goto works, and read_page with its `url` loads it again";
const CARD_WAITING: &str = "a card waits for the owner in the chat window, so no code runs in the \
                            browser until they answer or it closes; read_page still works";
const NO_WINDOW: &str = "No dotori chat window is open on the PC that holds the browser, so the \
                         owner cannot see a card. Ask the owner to open the dotori window on their PC and to ask \
                         you there; a card never shows on Telegram, Slack or Discord.";
const LOGINS_EVERY: Duration = Duration::from_secs(60);

/// How the browser runs. The profile folder keeps logins between runs. Without an executable,
/// dotori's own Chrome for Testing in `downloads` is used, fetched there on first use. The
/// browser has no window of its own: the owner sees it on the chat window's screen.
#[derive(Clone, Debug)]
pub struct BrowserConfig {
    pub executable: Option<PathBuf>,
    pub downloads: PathBuf,
    pub profile: PathBuf,
    /// Variables the browser does not inherit from dotori, such as every secret.
    pub hidden_env: Vec<String>,
    /// Where the browser's logins go out to the owner's other PCs, and where theirs come in.
    pub logins: Option<Logins>,
    /// Where the accounts the owner linked are kept.
    pub accounts: Option<Arc<dyn Accounts>>,
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
    locked: std::sync::Mutex<HashSet<String>>,
    card: std::sync::Mutex<Option<Waiting>>,
    next_card: AtomicU64,
    hidden: std::sync::Mutex<Hidden>,
}

struct Waiting {
    id: u64,
    answer: oneshot::Sender<Answer>,
}

enum Waited {
    Answer(Answer),
    Lapsed,
    Replaced,
}

struct Checked {
    origin: String,
    host: String,
    path: String,
    ticked: Vec<(u64, bool)>,
    loader: String,
    roles: Vec<(u64, Role)>,
    nodes: Vec<(u64, i64)>,
}

enum Step {
    Type(u64, String, bool),
    Set(u64, bool),
    Click(u64),
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
    target: String,
    marks: Arc<std::sync::Mutex<Marks>>,
}

#[derive(Default)]
struct Marks {
    touched: HashSet<String>,
    sealed: HashSet<String>,
    typed: HashMap<String, Vec<i64>>,
    scripting: bool,
    current: Option<String>,
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
    Fill,
    LogIn,
}

const ACTIONS: [Action; 5] = [
    Action::Run,
    Action::Read,
    Action::Ask,
    Action::Fill,
    Action::LogIn,
];

impl Action {
    fn owner_only(self) -> bool {
        match self {
            Self::Run | Self::Read => false,
            Self::Ask | Self::Fill | Self::LogIn => true,
        }
    }
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
            locked: std::sync::Mutex::default(),
            card: std::sync::Mutex::new(None),
            next_card: AtomicU64::new(card_start()),
            hidden: std::sync::Mutex::new(Hidden::default()),
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
                Ask::Input(input) if input.get("kind").and_then(Value::as_str) == Some("fill") => {
                    self.answer_card(&input);
                    Ok(())
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

    fn answer_card(&self, input: &Value) {
        let Some((id, answer)) = card::answer_of(input) else {
            return;
        };
        let waiting = {
            let mut card = self.card.lock().expect("unpoisoned");
            if card.as_ref().is_none_or(|waiting| waiting.id != id) {
                return;
            }
            card.take()
        };
        self.screen.update(json!({ "card": null }));
        if let Some(waiting) = waiting {
            let _ = waiting.answer.send(answer);
        }
    }

    fn card_seen(&self) -> Result<(), String> {
        if self.screen.watched() {
            Ok(())
        } else {
            Err(String::from(NO_WINDOW))
        }
    }

    async fn stored_secrets(&self, page: &Page<'_>) -> Vec<Kept> {
        let Some(accounts) = &self.config.accounts else {
            return Vec::new();
        };
        let Ok(origin) = page.evaluate("location.origin").await else {
            return Vec::new();
        };
        let Some(origin) = origin.as_str().filter(|origin| card::keepable(origin)) else {
            return Vec::new();
        };
        accounts
            .find(origin)
            .unwrap_or_default()
            .into_iter()
            .map(|account| Kept {
                value: account.password,
                origin: origin.to_owned(),
            })
            .collect()
    }

    fn released(&self, text: &str, stored: &[Kept]) -> Result<String, String> {
        let hidden = self.hidden.lock().expect("unpoisoned").with(stored);
        let origins = hidden.found_in(text);
        if origins.is_empty() {
            return Ok(text.to_owned());
        }
        self.locked
            .lock()
            .expect("unpoisoned")
            .extend(origins.iter().cloned());
        Err(format!(
            "dotori withheld this result whole because it held a password or code from the vault \
             or the owner. run_in_browser and read_page no longer work on {} until dotori's \
             browser starts again.",
            origins.join(", ")
        ))
    }

    async fn locked_here(&self, page: &Page<'_>) -> Result<(), String> {
        let origin = page.evaluate("location.origin").await?;
        let origin = origin.as_str().unwrap_or_default();
        if self.locked.lock().expect("unpoisoned").contains(origin) {
            return Err(format!(
                "a result on {origin} held a password or code, so dotori's browser takes no code \
                 and shows nothing there until it starts again"
            ));
        }
        Ok(())
    }

    async fn close_others(&self, page: &Page<'_>) -> Result<bool, String> {
        let others = other_pages(page).await?;
        for other in &others {
            page.running
                .cdp
                .send("Target.closeTarget", json!({ "targetId": other }), None)
                .await?;
        }
        let gone = async {
            while !other_pages(page).await?.is_empty() {
                time::sleep(POLL).await;
            }
            Ok::<(), String>(())
        };
        if !others.is_empty() {
            time::timeout(CLOSE_WAIT, gone)
                .await
                .map_err(|_| String::from("another window of the browser did not close"))??;
        }
        Ok(!others.is_empty())
    }

    fn card_waits(&self) -> bool {
        self.card.lock().expect("unpoisoned").is_some()
    }

    async fn check_card(&self, id: u64, roles: &[(u64, Role)]) -> Result<Checked, String> {
        if self.owner.lock().await.is_some() {
            return Err(String::from(
                "the owner is using the browser now; wait until they hand it back",
            ));
        }
        let _acting = self.acting.lock().await;
        let running = self.page().await?;
        let page = Page { running: &running };
        let (frame, loader) = page.document().await?;
        let (touched, sealed) = page.marked(&loader);
        if touched || sealed {
            self.close_others(&page).await?;
            return Err(String::from(TOUCHED));
        }
        let refs = page.world(false).await?;
        let mut nodes = Vec::new();
        for (at, _) in roles {
            nodes.push((*at, node_of(&page, refs, *at).await?));
        }
        let checked = check_in_world(&page, &frame, id, roles, &nodes).await?;
        let text = |key: &str| {
            checked
                .get(key)
                .and_then(Value::as_str)
                .map(str::to_owned)
                .ok_or_else(|| String::from("the page gave no origin"))
        };
        let origin = text("origin")?;
        if !card::keepable(&origin) {
            return Err(format!(
                "this page is at {origin}, and dotori shows cards and fills logins only on https pages. Nothing was shown to the owner."
            ));
        }
        let ticked = checked
            .get("checks")
            .and_then(Value::as_array)
            .map(Vec::as_slice)
            .unwrap_or_default()
            .iter()
            .filter_map(|check| Some((check.get("ref")?.as_u64()?, check.get("on")?.as_bool()?)))
            .collect();
        if self.close_others(&page).await? {
            page.mark_touched().await?;
            return Err(String::from(TOUCHED));
        }
        Ok(Checked {
            origin,
            host: text("host")?,
            path: text("path").unwrap_or_default(),
            ticked,
            loader,
            roles: roles.to_vec(),
            nodes,
        })
    }

    async fn wait_card(&self, id: u64, shown: Value) -> Waited {
        let (answer, answered) = oneshot::channel();
        *self.card.lock().expect("unpoisoned") = Some(Waiting { id, answer });
        self.screen.update(json!({ "card": shown }));
        let waited = match time::timeout(CARD_WAIT, answered).await {
            Ok(Ok(answer)) => Waited::Answer(answer),
            Ok(Err(_)) => Waited::Replaced,
            Err(_) => Waited::Lapsed,
        };
        let mut card = self.card.lock().expect("unpoisoned");
        if card.as_ref().is_some_and(|waiting| waiting.id == id) {
            *card = None;
            self.screen.update(json!({ "card": null }));
        }
        waited
    }

    async fn run_card(&self, card: &Checked, steps: Vec<Step>) -> Result<String, String> {
        if self.owner.lock().await.is_some() {
            return Err(String::from(
                "the owner is using the browser now, so dotori filled nothing; ask again once they hand it back",
            ));
        }
        let _acting = self.acting.lock().await;
        self.screen.update(json!({ "mode": "agent" }));
        let running = self.page().await?;
        let page = Page { running: &running };
        let mut typed = Vec::new();
        let done = self.steps(&page, card, steps, &mut typed).await;
        let cleared = if typed.is_empty() {
            Ok(())
        } else {
            if done.is_ok() {
                time::sleep(SEAL_WAIT).await;
            }
            clear_typed(&page, card, &typed).await
        };
        let shown = match (cleared, page.guard().await) {
            (Err(error), _) | (_, Err(error)) => Err(error),
            (Ok(()), Ok(())) => page.snapshot().await,
        };
        let still = !typed.is_empty()
            && page
                .document()
                .await
                .is_ok_and(|(_, loader)| loader == card.loader);
        let stored = self.stored_secrets(&page).await;
        drop(running);
        self.screen.update(json!({ "mode": "idle" }));
        done?;
        let shown = self.released(&shown?, &stored)?;
        let shown = fetch::window(&shown, 0, SHOWN);
        Ok(if still {
            format!("{shown}\n\n{STILL_SEALED}")
        } else {
            shown
        })
    }

    async fn steps(
        &self,
        page: &Page<'_>,
        card: &Checked,
        steps: Vec<Step>,
        typed: &mut Vec<i64>,
    ) -> Result<(), String> {
        let (frame, loader) = page.document().await?;
        if loader != card.loader {
            return Err(String::from(CHANGED));
        }
        if page.marked(&loader).0 {
            return Err(String::from(TOUCHED));
        }
        let again = check_in_world(page, &frame, 0, &card.roles, &card.nodes)
            .await
            .map_err(|error| format!("{error}. {CHANGED}"))?;
        if again.get("origin").and_then(Value::as_str) != Some(card.origin.as_str()) {
            return Err(String::from(CHANGED));
        }
        let world = fresh_world(page, &frame).await?;
        let mut plain = Vec::new();
        if steps.iter().any(|step| matches!(step, Step::Type(..))) {
            page.call_in(
                world,
                FILL_JS,
                vec![json!({ "value": "remember" }), json!({})],
            )
            .await?;
        }
        let node = |at: u64| {
            card.nodes
                .iter()
                .find(|(each, _)| *each == at)
                .map(|(_, node)| *node)
                .ok_or_else(|| String::from(CHANGED))
        };
        let on = |command: &'static str, at: u64, arg: Value| async move {
            let object = page
                .resolve(node(at)?, world)
                .await
                .map_err(|_| String::from(CHANGED))?;
            let done = page
                .call_in(
                    world,
                    FILL_JS,
                    vec![
                        json!({ "value": command }),
                        json!({ "value": arg }),
                        json!({ "objectId": object }),
                    ],
                )
                .await?;
            if done.get("error").is_some() {
                return Err(String::from(CHANGED));
            }
            Ok(done)
        };
        let click = |at: u64| async move {
            let point = on("at", at, Value::Null).await?;
            let x = point.get("x").and_then(Value::as_f64).unwrap_or(0.0);
            let y = point.get("y").and_then(Value::as_f64).unwrap_or(0.0);
            page.running.screen.pointer(x, y);
            page.send(
                "Input.dispatchMouseEvent",
                json!({ "type": "mouseMoved", "x": x, "y": y }),
            )
            .await?;
            for kind in ["mousePressed", "mouseReleased"] {
                page.send(
                    "Input.dispatchMouseEvent",
                    json!({ "type": kind, "x": x, "y": y, "button": "left", "clickCount": 1 }),
                )
                .await?;
            }
            Ok::<(), String>(())
        };
        for step in steps {
            match step {
                Step::Type(at, value, secret) => {
                    if value.is_empty() {
                        continue;
                    }
                    if !other_pages(page).await?.is_empty() {
                        return Err(String::from(OTHER_WINDOW));
                    }
                    let focused = on("focus", at, Value::Null).await?;
                    if focused.get("focused").and_then(Value::as_bool) != Some(true) {
                        return Err(format!(
                            "the cursor did not land in [{at}], so dotori typed nothing there and stopped"
                        ));
                    }
                    if secret {
                        self.hidden
                            .lock()
                            .expect("unpoisoned")
                            .add(&value, &card.origin);
                        let mut marks = page.running.marks.lock().expect("unpoisoned");
                        marks.sealed.insert(loader.clone());
                        marks
                            .typed
                            .entry(loader.clone())
                            .or_default()
                            .push(node(at)?);
                        drop(marks);
                        typed.push(node(at)?);
                    } else {
                        plain.push(node(at)?);
                    }
                    for event in key_events("Delete")? {
                        page.send("Input.dispatchKeyEvent", event).await?;
                    }
                    let still = on("focused", at, Value::Null).await?;
                    if still.get("focused").and_then(Value::as_bool) != Some(true)
                        || page.document().await?.1 != loader
                    {
                        return Err(format!(
                            "the cursor left [{at}] or the page moved, so dotori typed nothing there and stopped"
                        ));
                    }
                    page.send("Input.insertText", json!({ "text": value }))
                        .await?;
                    let landed = on("landed", at, json!(value)).await?;
                    if landed.get("landed").and_then(Value::as_bool) != Some(true) {
                        return Err(format!(
                            "the page did not take what was typed into [{at}], so dotori stopped"
                        ));
                    }
                }
                Step::Set(at, on_now) => {
                    let now = on("checked", at, Value::Null).await?;
                    if now.get("checked").and_then(Value::as_bool) != Some(on_now) {
                        click(at).await?;
                    }
                }
                Step::Click(at) => click(at).await?,
            }
        }
        time::sleep(PRESS_PAUSE).await;
        page.settle().await?;
        if !typed.is_empty() && page.document().await?.1 == loader {
            let changed = changed_nodes(page, world).await.unwrap_or_default();
            let mut marks = page.running.marks.lock().expect("unpoisoned");
            let kept = marks.typed.entry(loader.clone()).or_default();
            for node in changed.into_iter().filter(|node| !plain.contains(node)) {
                if !kept.contains(&node) {
                    kept.push(node);
                }
                if !typed.contains(&node) {
                    typed.push(node);
                }
            }
        }
        Ok(())
    }

    async fn fill(&self, arguments: Value) -> Result<String, String> {
        let spec = FillSpec::parse(&arguments)?;
        self.card_seen()?;
        if spec.keep && self.config.accounts.is_none() {
            return Err(String::from("this dotori keeps no accounts"));
        }
        let id = self.next_card.fetch_add(1, Ordering::SeqCst);
        let card = self.check_card(id, &spec.roles()).await?;
        if spec.keep
            && let Some(accounts) = &self.config.accounts
        {
            accounts.room(&card.origin)?;
        }
        let shown = spec.shown(id, &card.host, &card.ticked);
        let (at, values, checks) = match self.wait_card(id, shown).await {
            Waited::Answer(Answer::Press { at, values, checks }) => (at, values, checks),
            Waited::Answer(_) => return Ok(card::closed(&spec.title, false)),
            Waited::Lapsed => return Ok(card::closed(&spec.title, true)),
            Waited::Replaced => {
                return Err(String::from(
                    "a newer card took this card's place before the owner answered",
                ));
            }
        };
        let link = at != spec.submit.at;
        if link && !spec.links.iter().any(|named| named.at == at) {
            return Err(String::from(
                "the window pressed an element the card does not have, so dotori filled nothing",
            ));
        }
        if !link && let Some(refused) = spec.too_long(&values) {
            return Err(refused);
        }
        let typed = |at: u64| {
            values
                .iter()
                .find(|(field, _)| *field == at)
                .map(|(_, value)| value.clone())
                .unwrap_or_default()
        };
        let mut steps = Vec::new();
        let mut filled = Vec::new();
        let mut set = Vec::new();
        if !link {
            for field in &spec.fields {
                let value = typed(field.at);
                if !value.is_empty() {
                    filled.push(field.at);
                }
                steps.push(Step::Type(field.at, value, field.kind != card::Kind::Text));
            }
            for check in &spec.checks {
                if let Some((_, on)) = checks.iter().find(|(at, _)| *at == check.at) {
                    steps.push(Step::Set(check.at, *on));
                    set.push((check.at, *on));
                }
            }
        }
        steps.push(Step::Click(at));
        let account = (spec.keep && !link).then(|| account_of(&spec, &card, typed));
        let shown = self.run_card(&card, steps).await?;
        let kept = match (account, &self.config.accounts) {
            (Some(Ok(account)), Some(accounts)) => {
                accounts.keep(&account).map(|()| Some(account.site.clone()))
            }
            (Some(Err(error)), _) => Err(error),
            _ => Ok(None),
        };
        let told = card::pressed(
            &spec,
            at,
            &filled,
            &set,
            kept.as_ref().map(Option::as_deref).map_err(String::as_str),
        );
        Ok(format!("{told}\n\nThe page now:\n{shown}"))
    }

    async fn log_in(&self, arguments: Value) -> Result<String, String> {
        let spec = LogInSpec::parse(&arguments)?;
        let accounts = self
            .config
            .accounts
            .clone()
            .ok_or("this dotori keeps no accounts")?;
        let id = self.next_card.fetch_add(1, Ordering::SeqCst);
        let card = self.check_card(id, &spec.roles()).await?;
        let origin = card.origin.clone();
        let kept: Vec<Account> = accounts
            .find(&origin)?
            .into_iter()
            .filter(|account| account.origin == origin)
            .filter(|account| spec.id.is_none() || account.id.is_some())
            .filter(|account| {
                spec.account.is_none() || account.id.as_deref() == spec.account.as_deref()
            })
            .collect();
        if kept.is_empty() {
            return Err(format!(
                "the vault keeps no account for {origin} that fits these fields{}, so nothing was filled; to link one, show the owner a login card with ask_owner_to_fill and `keep`",
                spec.account
                    .as_deref()
                    .map(|id| format!(" with the id {id}"))
                    .unwrap_or_default()
            ));
        }
        let on_its_page = |account: &&Account| account.page.as_deref() == Some(card.path.as_str());
        let trusted: Vec<&Account> = kept.iter().filter(|account| account.trusted).collect();
        let (account, asked) = match trusted.as_slice() {
            [one] if on_its_page(one) => ((*one).clone(), false),
            _ => match self.pick_account(id, &card, &kept, &*accounts).await? {
                Ok(account) => (account, true),
                Err(told) => return Ok(told),
            },
        };
        let mut steps = Vec::new();
        if let (Some(at), Some(id)) = (spec.id, account.id.clone()) {
            steps.push(Step::Type(at, id, false));
        }
        steps.push(Step::Type(spec.password, account.password.clone(), true));
        steps.push(Step::Click(spec.submit));
        let shown = self.run_card(&card, steps).await?;
        Ok(format!(
            "{}\n\nThe page now:\n{shown}",
            card::logged_in(&account.site, account.id.as_deref(), asked)
        ))
    }

    async fn pick_account(
        &self,
        id: u64,
        card: &Checked,
        kept: &[Account],
        accounts: &dyn Accounts,
    ) -> Result<Result<Account, String>, String> {
        self.card_seen()?;
        let site = kept
            .first()
            .map(|account| account.site.clone())
            .unwrap_or_default();
        let shown = json!({
            "id": id,
            "kind": "log_in",
            "site": site,
            "accounts": kept
                .iter()
                .map(|account| account.id.as_deref().map(card::shortened))
                .collect::<Vec<_>>(),
            "host": card.host,
            "path": card.path,
        });
        let (trust, picked) = match self.wait_card(id, shown).await {
            Waited::Answer(Answer::Allow { trust, account }) => (trust, account),
            Waited::Answer(_) => return Ok(Err(card::declined(&site))),
            Waited::Lapsed => return Ok(Err(card::closed(&format!("{site} 로그인"), true))),
            Waited::Replaced => {
                return Err(String::from(
                    "a newer card took this card's place before the owner answered",
                ));
            }
        };
        let mut account = kept
            .get(picked)
            .cloned()
            .ok_or("the window picked an account the card does not have")?;
        let now = accounts.find(&card.origin)?;
        if !now.contains(&account) {
            return Err(String::from(
                "the account changed or was unlinked while the owner was asked, so nothing was filled; ask again if it is still wanted",
            ));
        }
        if trust {
            for mut other in now
                .into_iter()
                .filter(|other| other.trusted && *other != account)
            {
                other.trusted = false;
                accounts.keep(&other)?;
            }
            account.trusted = true;
            account.page = Some(card.path.clone());
            accounts.keep(&account)?;
        }
        Ok(Ok(account))
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

    /// The tools that drive this browser, for every session and coding agent.
    pub fn tools(self: &Arc<Self>) -> Vec<Box<dyn Tool>> {
        self.tools_where(false)
    }

    /// The tools that reach the owner through the chat window, for the main session alone.
    pub fn owner_tools(self: &Arc<Self>) -> Vec<Box<dyn Tool>> {
        self.tools_where(true)
    }

    fn tools_where(self: &Arc<Self>, owner: bool) -> Vec<Box<dyn Tool>> {
        ACTIONS
            .into_iter()
            .filter(|action| action.owner_only() == owner)
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
        let trying = time::Instant::now();
        let (child, cdp, version) = loop {
            let mut command = devtools_command(&executable);
            command
                .arg(format!("--user-data-dir={}", self.config.profile.display()))
                .arg(format!(
                    "--disk-cache-dir={}",
                    self.config.profile.join("Cache").display()
                ))
                .arg("--no-first-run")
                .arg("--no-default-browser-check")
                .arg("--disable-blink-features=AutomationControlled")
                .arg("about:blank")
                .kill_on_drop(true);
            command
                .arg("--headless=new")
                .arg(format!("--window-size={WIDTH},{HEIGHT}"));
            if root() {
                command.arg("--no-sandbox");
            }
            for hidden in &self.config.hidden_env {
                command.env_remove(hidden);
            }
            let mut child = command
                .spawn()
                .map_err(|error| format!("cannot start {}: {error}", executable.display()))?;
            let cdp = Arc::new(devtools(&mut child).await?);
            let started =
                time::timeout(START_WAIT, cdp.send("Browser.getVersion", json!({}), None))
                    .await
                    .map_err(|_| String::from("the browser did not start in time"))?;
            match started {
                Ok(version) => break (child, cdp, version),
                Err(error) => {
                    if time::timeout(SETTLE, child.wait()).await.is_err() {
                        return Err(error);
                    }
                    if trying.elapsed() > CLOSE_WAIT {
                        return Err(String::from(
                            "the browser ended at once; is its profile open in another window?",
                        ));
                    }
                }
            }
        };
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
        self.locked.lock().expect("unpoisoned").clear();
        self.screen.update(json!({ "login": null }));
        let marks: Arc<std::sync::Mutex<Marks>> = Arc::default();
        let watcher = tokio::spawn(watch(
            Arc::clone(&cdp),
            events,
            Arc::clone(&self.screen),
            Arc::clone(&self.shown),
            Arc::clone(&self.logins),
            (target.clone(), session.clone(), Arc::clone(&marks)),
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
            target,
            marks,
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
    (main_target, main_session, marks): (String, String, Arc<std::sync::Mutex<Marks>>),
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
                if opened_here && !ours {
                    let mut marks = marks.lock().expect("unpoisoned");
                    if let Some(current) = marks.current.clone() {
                        marks.touched.insert(current);
                    }
                }
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
                if session == Some(main_session.as_str())
                    && let Some(loader) = params.pointer("/frame/loaderId").and_then(Value::as_str)
                {
                    let mut marks = marks.lock().expect("unpoisoned");
                    if marks.scripting {
                        marks.touched.insert(loader.to_owned());
                    }
                    marks.current = Some(loader.to_owned());
                }
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

async fn node_of(page: &Page<'_>, refs: i64, at: u64) -> Result<i64, String> {
    let found = page
        .send(
            "Runtime.evaluate",
            json!({
                "expression": format!("globalThis.__refs && globalThis.__refs.get({at})"),
                "contextId": refs,
                "returnByValue": false
            }),
        )
        .await?;
    let missing = || format!("[{at}] is not on the page now; read the page again");
    let object = found
        .pointer("/result/objectId")
        .and_then(Value::as_str)
        .ok_or_else(missing)?
        .to_owned();
    let described = page
        .send("DOM.describeNode", json!({ "objectId": object }))
        .await;
    let _ = page
        .send("Runtime.releaseObject", json!({ "objectId": object }))
        .await;
    described?
        .pointer("/node/backendNodeId")
        .and_then(Value::as_i64)
        .ok_or_else(missing)
}

async fn fresh_world(page: &Page<'_>, frame: &str) -> Result<i64, String> {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|since| since.as_nanos())
        .unwrap_or(0);
    let made = page
        .send(
            "Page.createIsolatedWorld",
            json!({ "frameId": frame, "worldName": format!("card-{nanos}") }),
        )
        .await?;
    made.get("executionContextId")
        .and_then(Value::as_i64)
        .ok_or_else(|| String::from("the browser made no world for the card"))
}

async fn check_in_world(
    page: &Page<'_>,
    frame: &str,
    id: u64,
    roles: &[(u64, Role)],
    nodes: &[(u64, i64)],
) -> Result<Value, String> {
    let world = fresh_world(page, frame).await?;
    let mut arguments = vec![
        json!({ "value": "check" }),
        json!({ "value": {
            "card": id,
            "roles": roles
                .iter()
                .map(|(at, role)| json!({ "ref": at, "role": role }))
                .collect::<Vec<_>>(),
        } }),
    ];
    for (at, node) in nodes {
        let object = page
            .resolve(*node, world)
            .await
            .map_err(|_| format!("[{at}] is not on the page now; read the page again"))?;
        arguments.push(json!({ "objectId": object }));
    }
    let checked = page.call_in(world, FILL_JS, arguments).await?;
    match checked.get("error").and_then(Value::as_str) {
        Some(error) => Err(format!("{error}. Nothing was shown to the owner.")),
        None => Ok(checked),
    }
}

async fn clear_typed(page: &Page<'_>, card: &Checked, typed: &[i64]) -> Result<(), String> {
    let (frame, loader) = page.document().await?;
    if loader != card.loader {
        return Ok(());
    }
    let cleared = async {
        let world = fresh_world(page, &frame).await?;
        for node in typed {
            if let Ok(object) = page.resolve(*node, world).await {
                page.call_in(
                    world,
                    FILL_JS,
                    vec![
                        json!({ "value": "clear" }),
                        json!({}),
                        json!({ "objectId": object }),
                    ],
                )
                .await?;
            }
        }
        Ok::<(), String>(())
    };
    if cleared.await.is_ok() {
        return Ok(());
    }
    let _ = page
        .send("Page.navigate", json!({ "url": "about:blank" }))
        .await;
    Err(String::from(
        "dotori could not clear what it typed, so it left the page or keeps it closed to code; open it again",
    ))
}

async fn other_pages(page: &Page<'_>) -> Result<Vec<String>, String> {
    let targets = page
        .running
        .cdp
        .send("Target.getTargets", json!({}), None)
        .await?;
    Ok(targets
        .get("targetInfos")
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or_default()
        .iter()
        .filter(|target| target.get("type").and_then(Value::as_str) == Some("page"))
        .filter_map(|target| target.get("targetId").and_then(Value::as_str))
        .filter(|target| *target != page.running.target)
        .map(str::to_owned)
        .collect())
}

async fn changed_nodes(page: &Page<'_>, world: i64) -> Result<Vec<i64>, String> {
    let reply = page
        .send(
            "Runtime.callFunctionOn",
            json!({
                "functionDeclaration": FILL_JS,
                "executionContextId": world,
                "arguments": [{ "value": "changed" }, {}],
                "returnByValue": false
            }),
        )
        .await?;
    let list = reply
        .pointer("/result/objectId")
        .and_then(Value::as_str)
        .ok_or("the page gave no list")?
        .to_owned();
    let properties = page
        .send(
            "Runtime.getProperties",
            json!({ "objectId": list, "ownProperties": true }),
        )
        .await?;
    let mut nodes = Vec::new();
    for property in properties
        .get("result")
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or_default()
    {
        let is_index = property
            .get("name")
            .and_then(Value::as_str)
            .is_some_and(|name| name.parse::<usize>().is_ok());
        let Some(object) = property.pointer("/value/objectId").and_then(Value::as_str) else {
            continue;
        };
        if !is_index {
            continue;
        }
        let described = page
            .send("DOM.describeNode", json!({ "objectId": object }))
            .await?;
        if let Some(node) = described
            .pointer("/node/backendNodeId")
            .and_then(Value::as_i64)
        {
            nodes.push(node);
        }
    }
    let _ = page
        .send("Runtime.releaseObject", json!({ "objectId": list }))
        .await;
    Ok(nodes)
}

fn account_of(
    spec: &FillSpec,
    card: &Checked,
    typed: impl Fn(u64) -> String,
) -> Result<card::Account, String> {
    let field = |kind: card::Kind| spec.fields.iter().find(|field| field.kind == kind);
    let secret = field(card::Kind::Secret).ok_or("the card has no password field")?;
    let password = typed(secret.at);
    if password.trim().is_empty() {
        return Err(String::from("the owner left the password empty"));
    }
    let id = field(card::Kind::Text).map(|field| (field.label.clone(), typed(field.at)));
    if id.as_ref().is_some_and(|(_, id)| id.trim().is_empty()) {
        return Err(String::from("the owner left the id empty"));
    }
    Ok(card::Account {
        site: spec.site.clone().unwrap_or_default(),
        origin: card.origin.clone(),
        id_label: id.as_ref().map(|(label, _)| label.clone()),
        id: id.map(|(_, id)| id.trim().to_owned()),
        password_label: secret.label.clone(),
        password,
        trusted: false,
        page: Some(card.path.clone()),
        kept_ms: 0,
    })
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

fn card_start() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|since| u64::try_from(since.as_millis()).unwrap_or(u64::MAX / 2))
        .unwrap_or(0)
        .saturating_mul(1000)
}

/// Whether a page may be loaded from `url`, by the same rule as fetch_url.
pub(crate) fn document_allowed(url: &str) -> Result<(), String> {
    let url = Url::parse(url).map_err(|error| error.to_string())?;
    fetch::allowed(&url)
}

impl BrowserTool {
    async fn act(&self, arguments: Value) -> Result<String, String> {
        let done = match self.action {
            Action::Fill => self.browser.fill(arguments).await,
            Action::LogIn => self.browser.log_in(arguments).await,
            Action::Run | Action::Read | Action::Ask => self.drive(arguments).await,
        };
        match done {
            Ok(text) => self.browser.released(&text, &[]),
            Err(error) => Err(self
                .browser
                .released(&error, &[])
                .unwrap_or_else(|withheld| withheld)),
        }
    }

    async fn drive(&self, arguments: Value) -> Result<String, String> {
        if self.action == Action::Ask {
            let reason = arguments
                .get("reason")
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|reason| !reason.is_empty())
                .ok_or("`reason` is required")?;
            self.browser.screen.update(json!({ "asked": reason }));
            return Ok(String::from(
                "Asked the owner. A card in the chat window shows them the page, they open the \
                 browser in its own window and hand it back when done. Do not wait for them: tell the owner what you need, and \
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
        let open = match (self.action, arguments.get("url")) {
            (Action::Read, Some(url)) => {
                let url = url.as_str().ok_or("`url` is an address")?;
                let url = Url::parse(url).map_err(|error| format!("not a URL: {error}"))?;
                fetch::allowed(&url)?;
                Some(url)
            }
            _ => None,
        };
        let script = match self.action {
            Action::Read | Action::Ask | Action::Fill | Action::LogIn => None,
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
        if script.is_some() && self.browser.card_waits() {
            return Err(String::from(CARD_WAITING));
        }
        let _acting = self.browser.acting.lock().await;
        let running = self.browser.page().await?;
        let page = Page { running: &running };
        if open.is_none() {
            self.browser.locked_here(&page).await?;
        }
        self.browser.screen.update(json!({ "mode": "agent" }));
        let mut stored = self.browser.stored_secrets(&page).await;
        let said = match (&script, &open) {
            (None, None) => Ok(String::new()),
            (None, Some(url)) => match self.browser.close_others(&page).await {
                Ok(_) => page.go(url.as_str()).await.map(|()| String::new()),
                Err(error) => Err(error),
            },
            (Some(body), _) => {
                let driver = Driver {
                    page: Page { running: &running },
                };
                let home = Native {
                    kind: "page",
                    data: Value::Null,
                };
                running.marks.lock().expect("unpoisoned").scripting = true;
                let ran = time::timeout(RUN_WAIT, script::run::run(body, &driver, home)).await;
                running.marks.lock().expect("unpoisoned").scripting = false;
                let touched = page.mark_touched().await;
                match (ran, touched) {
                    (Ok(Ok(outcome)), Ok(())) => Ok(said(&outcome)),
                    (Ok(Err(error)), _) | (Ok(Ok(_)), Err(error)) => Err(error),
                    (Err(_), _) => Err(format!(
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
        stored.extend(self.browser.stored_secrets(&page).await);
        let locked = self.browser.locked_here(&page).await;
        let broken = !running.cdp.open();
        drop(running);
        self.browser.screen.update(json!({ "mode": "idle" }));
        if let Err(error) = &shown
            && (broken || error.contains("not found"))
        {
            self.browser.forget().await;
        }
        locked?;
        let whole = [&said, &shown]
            .map(|part| part.as_ref().map_or_else(String::as_str, String::as_str))
            .concat();
        self.browser.released(&whole, &stored)?;
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
        self.evaluate_in(expression, true).await
    }

    /// Runs the model's code once in dotori's world, never again in a world made after the
    /// page moved on.
    pub(crate) async fn evaluate_once(&self, expression: &str) -> Result<Value, String> {
        self.evaluate_in(expression, false).await
    }

    async fn evaluate_in(&self, expression: &str, retry: bool) -> Result<Value, String> {
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
                Err(error) if retry && !fresh && error.contains("context") => fresh = true,
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

    /// The main frame and the loader id of the document it shows now.
    pub(crate) async fn document(&self) -> Result<(String, String), String> {
        let tree = self.send("Page.getFrameTree", json!({})).await?;
        let text = |key: &str| {
            tree.pointer(&format!("/frameTree/frame/{key}"))
                .and_then(Value::as_str)
                .map(str::to_owned)
                .ok_or_else(|| String::from("the page has no frame"))
        };
        Ok((text("id")?, text("loaderId")?))
    }

    /// Marks the document shown now as one the model's code ran in, before the model's code
    /// acts on it. A document dotori typed the owner's words into takes no model code at all.
    pub(crate) async fn touch(&self) -> Result<(), String> {
        let (_, loader) = self.document().await?;
        let mut marks = self.running.marks.lock().expect("unpoisoned");
        if marks.sealed.contains(&loader) {
            return Err(String::from(SEALED));
        }
        marks.touched.insert(loader);
        Ok(())
    }

    async fn mark_touched(&self) -> Result<(), String> {
        let (_, loader) = self.document().await?;
        self.running
            .marks
            .lock()
            .expect("unpoisoned")
            .touched
            .insert(loader);
        Ok(())
    }

    fn marked(&self, loader: &str) -> (bool, bool) {
        let marks = self.running.marks.lock().expect("unpoisoned");
        (
            marks.touched.contains(loader),
            marks.sealed.contains(loader),
        )
    }

    async fn resolve(&self, node: i64, context: i64) -> Result<String, String> {
        let resolved = self
            .send(
                "DOM.resolveNode",
                json!({ "backendNodeId": node, "executionContextId": context }),
            )
            .await?;
        resolved
            .pointer("/object/objectId")
            .and_then(Value::as_str)
            .map(str::to_owned)
            .ok_or_else(|| String::from("the element is gone"))
    }

    async fn call_in(
        &self,
        context: i64,
        function: &str,
        arguments: Vec<Value>,
    ) -> Result<Value, String> {
        let reply = self
            .send(
                "Runtime.callFunctionOn",
                json!({
                    "functionDeclaration": function,
                    "executionContextId": context,
                    "arguments": arguments,
                    "returnByValue": true,
                    "awaitPromise": true
                }),
            )
            .await?;
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

    async fn read(&self, fresh: bool) -> Result<Value, String> {
        let (_, loader) = self.document().await?;
        let typed = self
            .running
            .marks
            .lock()
            .expect("unpoisoned")
            .typed
            .get(&loader)
            .cloned()
            .unwrap_or_default();
        let context = self.world(fresh).await?;
        let mut arguments = Vec::new();
        for node in typed {
            if let Ok(object) = self.resolve(node, context).await {
                arguments.push(json!({ "objectId": object }));
            }
        }
        self.call_in(context, SNAPSHOT, arguments).await
    }

    pub(crate) async fn snapshot(&self) -> Result<String, String> {
        let mut tries = 0;
        loop {
            match self.read(tries > 0).await {
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
            Action::Run => RUN,
            Action::Read => READ,
            Action::Ask => ASK,
            Action::Fill => FILL,
            Action::LogIn => FILL_LOGIN,
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
                 then the page as read_page shows it."
            }
            Action::Read => {
                "Returns the page now open in the browser, with interactive elements as [n]. With \
                 `url`, opens that address first, the way to load a page that ask_owner_to_fill \
                 or fill_login will use, since a page opened by run_in_browser counts as one your \
                 code ran on."
            }
            Action::Ask => {
                "Asks the owner, through a card in the chat window, to take the browser over and do \
                 something themselves, such as passing a check dotori cannot. To log in, use fill_login \
                 with an account the vault keeps, or ask_owner_to_fill to show the owner the page's \
                 login fields. Returns at once; the owner hands the browser back when done."
            }
            Action::Fill => {
                "Shows the owner a card in the dotori chat window to fill fields of the page open in \
                 the browser, such as a login, a one-time code or a password reset, and to press its \
                 button or one of its links. Only on an https page that no code of yours ran on since \
                 it loaded: open it with read_page and its `url`, then call this. Name its elements by \
                 their [n]. A secret field must be a password field. \
                 dotori draws the card with the page's real host; you cannot put values in it, and \
                 what the owner types goes to the page, never to you. With `keep` and `site`, dotori \
                 seals the account (one secret field for the password, at most one text field for the \
                 id) in its vault for fill_login later. The owner answers in the dotori window on their \
                 PC; when they talk to you on Telegram, Slack or Discord, ask them to do it there. \
                 You learn which button or link they pressed, which fields were filled, and the page \
                 after."
            }
            Action::LogIn => {
                "Logs in to the page open in the browser with the account the vault keeps for its \
                 origin: dotori fills the id and password fields you name by their [n] and presses \
                 `submit`. Unless the owner said not to ask again for an account of this site, \
                 dotori first asks them in the chat window, where they also pick the account when \
                 the site has several. It fills only a page on the origin the account was kept for, \
                 and you never see the id or the password. Same page rules as ask_owner_to_fill. \
                 Fails when the vault keeps no account for the page."
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
            Action::Read => json!({
                "type": "object",
                "properties": {
                    "url": { "type": "string", "description": "An address to open first." },
                    "start_index": start
                }
            }),
            Action::Ask => json!({
                "type": "object",
                "properties": { "reason": { "type": "string", "description": "What the owner should do, such as `Pass the picture check`." } },
                "required": ["reason"]
            }),
            Action::Fill => {
                let named = json!({
                    "type": "object",
                    "properties": {
                        "ref": { "type": "integer", "description": "The element's [n] in read_page." },
                        "label": { "type": "string", "description": "What the card calls it, in Korean." }
                    },
                    "required": ["ref", "label"]
                });
                json!({
                    "type": "object",
                    "properties": {
                        "title": { "type": "string", "description": "The card's title, such as `쿠팡 로그인`." },
                        "hint": { "type": "string", "description": "One short line under the title, such as `휴대폰으로 받은 인증번호를 적어 주세요`." },
                        "site": { "type": "string", "description": "The name the owner knows the site by, such as `쿠팡`. Required with `keep`." },
                        "fields": {
                            "type": "array",
                            "items": {
                                "type": "object",
                                "properties": {
                                    "ref": { "type": "integer" },
                                    "label": { "type": "string" },
                                    "kind": { "type": "string", "enum": ["text", "secret", "code"], "description": "secret for a password, code for a one-time code dotori never keeps." }
                                },
                                "required": ["ref", "label", "kind"]
                            }
                        },
                        "checks": { "type": "array", "items": named, "description": "Checkboxes the owner may tick, such as `로그인 상태 유지`." },
                        "submit": named,
                        "links": { "type": "array", "items": named, "description": "Links the owner may press instead, such as `비밀번호 찾기`." },
                        "keep": { "type": "boolean", "description": "Keep the account in the vault for fill_login. Only for a login on an https page." }
                    },
                    "required": ["title", "fields", "submit"]
                })
            }
            Action::LogIn => json!({
                "type": "object",
                "properties": {
                    "id": { "type": "integer", "description": "The [n] of the id field, when the page shows one." },
                    "password": { "type": "integer", "description": "The [n] of the password field." },
                    "submit": { "type": "integer", "description": "The [n] of the button that logs in." },
                    "account": { "type": "string", "description": "The id of the kept account to log in with, when the site has several and the owner named one." }
                },
                "required": ["password", "submit"]
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

fn devtools_command(executable: &Path) -> Command {
    if cfg!(windows) {
        let mut command = Command::new(executable);
        command
            .arg("--remote-debugging-port=0")
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::piped());
        return command;
    }
    let mut command = Command::new("/bin/sh");
    command
        .arg("-c")
        .arg(PIPED)
        .arg(executable)
        .arg("--remote-debugging-pipe")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    command
}

async fn devtools(child: &mut Child) -> Result<Cdp, String> {
    if let (Some(to), Some(from)) = (child.stdin.take(), child.stdout.take()) {
        return Ok(Cdp::pipe(from, to));
    }
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
    Cdp::connect(&url).await
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
