use std::fs;
use std::future;
use std::num::NonZeroUsize;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::OnceLock;
use std::time::Duration;

use tokio::sync::mpsc;
use tokio::time;

use dotori_connectors::channel::app::AppChannel;
use dotori_connectors::peer::copies::Announced;
use dotori_connectors::peer::copies::Copies;
use dotori_connectors::peer::hub::Heard;
use dotori_connectors::peer::lease::Keeper;
use dotori_connectors::record::jsonl::JsonlStore;
use dotori_connectors::record::sqlite::SqliteStore;
use dotori_connectors::record::sqlite::read_legacy;
use dotori_connectors::sub_session::cli::CliRunner;
use dotori_connectors::sub_session::cli::CodingAgent;
use dotori_connectors::sub_session::cli::McpServer;
use dotori_connectors::sub_session::llm::LlmRunner;
use dotori_connectors::tool::basic::BasicTools;
use dotori_connectors::tool::basic::EDIT_FILE;
use dotori_connectors::tool::basic::Private;
use dotori_connectors::tool::basic::RUN_SHELL;
use dotori_connectors::tool::basic::WRITE_FILE;
use dotori_connectors::tool::basic::WithBasic;
use dotori_core::channel::Channel;
use dotori_core::channel::Inbox;
use dotori_core::llm::Model;
use dotori_core::record::RecordError;
use dotori_core::record::RecordStore;
use dotori_core::runtime::Agent;
use dotori_core::runtime::AgentConfig;
use dotori_core::runtime::Ports;
use dotori_core::sub_session::SubSessionRunner;
use dotori_core::tool::Toolbox;
use dotori_mcp::memory::Memory;
use dotori_mcp::memory::MemoryConfig;
use dotori_mcp::relay::relay;

use crate::browser_route::BROWSER_JOB;
use crate::browser_route::BrowserRoute;
use crate::confirm;
use crate::confirm::Asking;
use crate::confirm::DecisionChannel;
use crate::confirm::Guarded;
use crate::device;
use crate::door;
use crate::door::Door;
use crate::forget::FORGET;
use crate::forget::Forget;
use crate::forget::Previewed;
use crate::handoff;
use crate::handoff::HandedOff;
use crate::handoff::Handoff;
use crate::history;
use crate::home::Home;
use crate::icons;
use crate::idle::NoTools;
use crate::lock::DaemonLock;
use crate::logins;
use crate::logins::LoginFiles;
use crate::main_session;
use crate::main_session::AgentApp;
use crate::main_session::Fenced;
use crate::main_session::FencedTools;
use crate::main_session::Linked;
use crate::main_session::MAIN_JOB;
use crate::main_session::Route;
use crate::main_session::gained;
use crate::main_session::lost;
use crate::notes;
use crate::notes::MainTools;
use crate::notes::Noted;
use crate::notes::Notes;
use crate::peers;
use crate::routines;
use crate::routines::RoutineChannel;
use crate::routines::Routines;
use crate::settings::RunnerChoice;
use crate::settings::Settings;
use crate::settings_copies::SettingsCopies;
use crate::tools;
use crate::vault;
use crate::vault::Vault;
use crate::vector_copies;
use crate::vector_copies::VECTORS_JOB;
use crate::vector_copies::VectorCopies;

const QUEUE: NonZeroUsize = NonZeroUsize::new(256).expect("non-zero");
const SEEN: NonZeroUsize = NonZeroUsize::new(4096).expect("non-zero");
const TOOL_TIMEOUT: Duration = Duration::from_secs(120);
const SHELL_LIMIT: Duration = Duration::from_secs(110);
const SUB_SESSION_TURNS: usize = 50;
const SUB_SESSION_LIMIT: Duration = Duration::from_secs(60 * 60);
const SETTLE: Duration = Duration::from_secs(1);
const RELEASE_SENT: Duration = Duration::from_millis(300);
const FLUSH_EVERY: Duration = Duration::from_secs(2);
const LOOK_FOR_PCS: Duration = Duration::from_secs(2);

/// Runs the agent until Ctrl-C or a terminate signal, with the app channel open for windows.
pub(crate) async fn run(home: Home) -> Result<(), String> {
    serve(home, || {}, future::pending()).await
}

/// Runs the agent like `run`. It calls `ready` once windows can connect, and also stops when
/// `quit` completes.
pub(crate) async fn serve(
    home: Home,
    ready: impl FnOnce(),
    quit: impl Future<Output = ()>,
) -> Result<(), String> {
    let _lock = DaemonLock::take(&home.lock())?;
    let settings = Settings::load(&home.settings())?;
    let vault = Arc::new(Vault::new(&home.root));
    let model = settings.model(vault::lookup(&vault))?;
    let shared = match tools::SharedTools::start().await {
        Ok(tools) => Some(Arc::new(tools)),
        Err(error) => {
            eprintln!("dotori: no shared tools: {error}");
            None
        }
    };
    let toolbox: Arc<dyn Toolbox> = match &shared {
        Some(tools) => Arc::clone(tools) as Arc<dyn Toolbox>,
        None => Arc::new(NoTools),
    };
    let handoff = Arc::new(Handoff::default());
    let sub_sessions: Arc<dyn SubSessionRunner> = Arc::new(HandedOff {
        handoff: Arc::clone(&handoff),
        inner: sub_session_runner(&home, &settings, &vault, &toolbox, shared)?,
    });
    let records = open_record(&home).await?;
    let notes = Notes::new(home.notes());
    let routines = Routines::new(home.routines());
    let model: Arc<dyn Model> = Arc::new(Noted {
        model,
        notes: Arc::clone(&notes),
        routines: Arc::clone(&routines),
    });
    let browser = BrowserRoute::new(toolbox);
    let token = door::token()?;
    let (app, address) = AppChannel::bind(token.clone()).map_err(|error| error.to_string())?;
    let app = Arc::new(app);
    let secrets = vault::secret_names(&settings, &vault);
    let (guarded, wrong) = settings.relays(vault::lookup(&vault), true, &secrets);
    let (mut guarded, failed) = relay(guarded).await;
    for problem in wrong.iter().chain(&failed) {
        eprintln!("dotori: an MCP server is left out: {problem}");
    }
    let mut asked = settings.asked();
    for name in asked.unmatched(guarded.iter().map(|tool| tool.name())) {
        eprintln!(
            "dotori: `confirm` names {name}, which no MCP server offers, so every tool of its server waits for the owner"
        );
        asked.close_server_of(&name);
    }
    let forget_copies: Arc<OnceLock<Arc<Copies>>> = Arc::default();
    let forgetting = MemoryConfig {
        record: home.record(),
        index: home.search_index(),
        dictionaries: home.dictionaries(),
        embedding: None,
        jobs: None,
    };
    guarded.push(Box::new(Forget {
        store: records.clone(),
        memory: forgetting.clone(),
        copies: Arc::clone(&forget_copies),
    }));
    for tool in [FORGET, RUN_SHELL, WRITE_FILE, EDIT_FILE] {
        asked.always(tool);
    }
    let asking = Arc::new(Asking::new(
        asked,
        Arc::new(WithBasic {
            basic: basic(&home, &secrets),
            rest: Arc::new(Guarded {
                tools: guarded,
                rest: Arc::clone(&browser) as Arc<dyn Toolbox>,
            }),
        }),
        Arc::clone(&app),
    ));
    let (outcomes, outcomes_heard) = mpsc::unbounded_channel();
    let decision = Arc::new(DecisionChannel::new(outcomes_heard));
    let toolbox: Arc<dyn Toolbox> = Arc::new(MainTools {
        notes: Arc::clone(&notes),
        routines: Arc::clone(&routines),
        shared: Arc::new(Previewed {
            inner: Arc::clone(&asking) as Arc<dyn Toolbox>,
            store: records.clone(),
        }),
    });
    match records.load().await {
        Ok(earlier) => app.show_earlier(&history::lines(&earlier)),
        Err(error) => eprintln!("dotori: the earlier conversation cannot be shown: {error}"),
    }
    let door = Door {
        port: address.port(),
        token,
    };
    let (records, linked, heard) = link(&home, &settings, records, &app).await;
    if let Some(linked) = &linked {
        browser.link(linked.clone());
        handoff.link(linked.clone(), Arc::clone(&records));
    }
    for job in [VECTORS_JOB, BROWSER_JOB] {
        vector_copies::write_job(&home.jobs(), job, linked.is_none());
    }
    vector_copies::write_job(&home.jobs(), "linked", linked.is_some());
    let login_files = LoginFiles {
        out: home.logins_out(),
        into: home.logins_in(),
    };
    if let Some(linked) = linked.clone() {
        tokio::spawn(logins::share(linked.clone(), login_files.clone()));
        let jobs = home.jobs();
        tokio::spawn(async move {
            let Some(mut holder) = linked.keeper.watch(BROWSER_JOB) else {
                return;
            };
            loop {
                let holds = holder.borrow_and_update().as_deref() == Some(linked.own.as_str());
                vector_copies::write_job(&jobs, BROWSER_JOB, holds);
                if holder.changed().await.is_err() {
                    return;
                }
            }
        });
    }
    let vectors = linked
        .clone()
        .map(|linked| VectorCopies::new(linked, home.search_index()));
    if let (Some(linked), Some(vectors)) = (linked.clone(), vectors.clone()) {
        let jobs = home.jobs();
        tokio::spawn(Arc::clone(&vectors).keep_asking());
        tokio::spawn(vectors.on_maker_change());
        tokio::spawn(async move {
            let Some(mut holder) = linked.keeper.watch(VECTORS_JOB) else {
                return;
            };
            loop {
                let makes = holder.borrow_and_update().as_deref() == Some(linked.own.as_str());
                vector_copies::write_job(&jobs, VECTORS_JOB, makes);
                if holder.changed().await.is_err() {
                    return;
                }
            }
        });
    }
    let route = Route::new(linked.clone());
    let (decide_tx, mut decisions) = mpsc::unbounded_channel::<((u64, bool), bool)>();
    {
        let mut clicked = app.decisions();
        let decide_tx = decide_tx.clone();
        tokio::spawn(async move {
            while let Some(decision) = clicked.recv().await {
                if decide_tx.send((decision, false)).is_err() {
                    return;
                }
            }
        });
    }
    {
        let asking = Arc::clone(&asking);
        let linked = linked.clone();
        tokio::spawn(async move {
            while let Some((decision, passed_on)) = decisions.recv().await {
                let asking = Arc::clone(&asking);
                let outcomes = outcomes.clone();
                let linked = linked.clone();
                tokio::spawn(async move {
                    confirm::decided(&asking, &outcomes, linked.as_ref(), decision, passed_on)
                        .await;
                });
            }
        });
    }
    if let Some((_, copies)) = &heard {
        let _ = forget_copies.set(Arc::clone(copies));
        let copies = Arc::clone(copies);
        tokio::spawn(async move {
            loop {
                copies.forgotten_elsewhere().await;
                let memory = forgetting.clone();
                match tokio::task::spawn_blocking(move || Memory::drop_forgotten(&memory)).await {
                    Ok(Ok(())) => {}
                    Ok(Err(error)) => eprintln!(
                        "dotori: the search index keeps what another PC forgot until the next search: {error}"
                    ),
                    Err(error) => eprintln!("dotori: {error}"),
                }
            }
        });
    }
    if let (Some(linked), Some((mut heard, copies))) = (linked.clone(), heard) {
        let app = Arc::clone(&app);
        let route = Arc::clone(&route);
        tokio::spawn(notes::share(Arc::clone(&notes), Arc::clone(&linked.hub)));
        tokio::spawn(routines::share(
            Arc::clone(&routines),
            Arc::clone(&linked.hub),
        ));
        let (words_tx, mut words) = mpsc::unbounded_channel::<Heard>();
        {
            let linked = linked.clone();
            let route = Arc::clone(&route);
            tokio::spawn(async move {
                while let Some(heard) = words.recv().await {
                    main_session::hear(&linked, &app, &route, &heard).await;
                }
            });
        }
        {
            let route = Arc::clone(&route);
            tokio::spawn(async move {
                let mut every = time::interval(FLUSH_EVERY);
                loop {
                    every.tick().await;
                    route.flush();
                }
            });
        }
        let settings_copies = Arc::new(SettingsCopies::new(home.settings(), home.settings_stamp()));
        {
            let settings_copies = Arc::clone(&settings_copies);
            let linked = linked.clone();
            tokio::spawn(async move { settings_copies.share(linked).await });
        }
        {
            let vault = Arc::clone(&vault);
            let linked = linked.clone();
            tokio::spawn(vault::share(vault, linked));
        }
        tokio::spawn(icons::share(home.icons(), home.settings(), linked.clone()));
        let (icon_folder, settings_path) = (home.icons(), home.settings());
        let vault = Arc::clone(&vault);
        let routines = Arc::clone(&routines);
        let handoff = Arc::clone(&handoff);
        tokio::spawn(async move {
            while let Some(heard) = heard.recv().await {
                linked.keeper.hear(&heard);
                if let Some(decision) = confirm::heard(&heard) {
                    let _ = decide_tx.send((decision, true));
                    continue;
                }
                if let Some((spec, outcome)) = handoff::heard(&heard.message) {
                    let handoff = Arc::clone(&handoff);
                    tokio::spawn(async move { handoff.show(spec, outcome).await });
                    continue;
                }
                if settings_copies.hear(&linked, &heard) {
                    continue;
                }
                if vault::hear(&vault, &linked, &heard) {
                    continue;
                }
                if icons::hear(&icon_folder, &settings_path, &linked, &heard) {
                    continue;
                }
                if vectors.as_ref().is_some_and(|vectors| vectors.hear(&heard)) {
                    continue;
                }
                if browser.hear(&heard) {
                    continue;
                }
                if logins::hear(&linked, &login_files, &heard) {
                    continue;
                }
                if notes::hear(&notes, &linked.hub, &heard) {
                    continue;
                }
                if routines::hear(&routines, &linked.hub, &heard) {
                    continue;
                }
                if copies.hear(&heard).await {
                    continue;
                }
                let _ = words_tx.send(heard);
            }
        });
    }
    {
        let app = Arc::clone(&app);
        let route = Arc::clone(&route) as Arc<dyn Inbox>;
        tokio::spawn(async move {
            if let Err(error) = app.run(route).await {
                eprintln!("dotori: windows cannot reach dotori: {error}");
            }
        });
    }
    if let Some(linked) = linked.clone()
        && let Some(mut holder) = linked.keeper.watch(MAIN_JOB)
    {
        let route = Arc::clone(&route);
        let app = Arc::clone(&app);
        tokio::spawn(async move {
            let mut shown = None;
            let mut every = time::interval(LOOK_FOR_PCS);
            loop {
                tokio::select! {
                    changed = holder.changed() => {
                        if changed.is_err() {
                            return;
                        }
                        route.flush();
                    }
                    _ = every.tick() => {}
                }
                let now = holder
                    .borrow_and_update()
                    .clone()
                    .filter(|_| !linked.hub.approved().is_empty())
                    .map(|key| (linked.hub.name_of(&key), key == linked.own));
                if now != shown {
                    app.answering(now.clone());
                    shown = now;
                }
            }
        });
    }
    if let Some(linked) = linked.clone() {
        let app = Arc::clone(&app);
        let mut holder = linked.keeper.watch(BROWSER_JOB);
        tokio::spawn(async move {
            while let Some(holder) = holder.as_mut() {
                if holder.changed().await.is_err() {
                    return;
                }
                let now = holder.borrow_and_update().clone();
                let key = now.unwrap_or_else(|| linked.own.clone());
                app.browser_at(linked.hub.name_of(&key), key == linked.own);
            }
        });
    }
    let config = AgentConfig {
        queue_capacity: QUEUE,
        seen_capacity: SEEN,
        tool_timeout: TOOL_TIMEOUT,
    };
    door.write(&home.door())?;
    eprintln!("dotori is running. Open a window with `dotori chat`.");
    ready();
    let mut stop = std::pin::pin!(stop_signal());
    let mut quit = std::pin::pin!(quit);
    let stopped = loop {
        tokio::select! {
            () = gained(linked.as_ref()) => {}
            stopped = &mut stop => break stopped,
            () = &mut quit => break Ok(()),
        }
        if let Some(linked) = &linked {
            time::sleep(SETTLE).await;
            if !linked.keeper.holds(MAIN_JOB) {
                continue;
            }
        }
        let (mut channels, off) = settings.channels(vault::lookup(&vault));
        for problem in off {
            eprintln!("dotori: {problem}");
        }
        channels.push(Arc::new(RoutineChannel::new(Arc::clone(&routines))) as Arc<dyn Channel>);
        channels.push(Arc::clone(&decision) as Arc<dyn Channel>);
        let channels = channels
            .into_iter()
            .map(|channel| {
                Arc::new(Fenced {
                    channel,
                    linked: linked.clone(),
                }) as Arc<dyn Channel>
            })
            .collect();
        let ports = Ports {
            app: Arc::new(AgentApp {
                app: Arc::clone(&app),
                route: Arc::clone(&route),
                linked: linked.clone(),
            }),
            channels,
            model: Arc::clone(&model),
            toolbox: Arc::new(FencedTools {
                tools: Arc::clone(&toolbox),
                linked: linked.clone(),
            }),
            sub_sessions: Arc::clone(&sub_sessions),
            records: Arc::clone(&records),
        };
        asking.lapse_waiting();
        let agent = Agent::spawn_continuing(config.clone(), ports, handoff.running())
            .await
            .map_err(|error| error.to_string())?;
        handoff.serve(Some(agent.returns()));
        if linked.is_some() {
            eprintln!("dotori: this PC runs the assistant now");
        }
        for _ in 0..50 {
            if route.runs_agent() {
                break;
            }
            time::sleep(Duration::from_millis(20)).await;
        }
        route.hand_waiting(&app).await;
        let ended = tokio::select! {
            () = lost(linked.as_ref()) => None,
            stopped = &mut stop => Some(stopped),
            () = &mut quit => Some(Ok(())),
        };
        route.elsewhere();
        handoff.serve(None);
        if ended.is_some() {
            handoff.quit();
            agent.shutdown().await;
            handoff.stop_all();
        } else {
            agent.hand_off().await;
        }
        asking.lapse_waiting();
        match ended {
            Some(stopped) => break stopped,
            None => eprintln!("dotori: another PC runs the assistant now"),
        }
    };
    let _ = fs::remove_file(home.door());
    if let Some(linked) = &linked {
        linked.keeper.release_all();
        time::sleep(RELEASE_SENT).await;
    }
    stopped
}

async fn link(
    home: &Home,
    settings: &Settings,
    records: SqliteStore,
    app: &Arc<AppChannel>,
) -> (Arc<dyn RecordStore>, Option<Linked>, Option<Hearing>) {
    let Some((hub, heard)) = peers::start(home, settings).await else {
        return (Arc::new(records), None, None);
    };
    let own = hub.own().to_owned();
    let copies = Copies::start(Arc::clone(&hub), records.clone());
    let keeper = Keeper::start(
        Arc::clone(&hub),
        own.clone(),
        &[MAIN_JOB, VECTORS_JOB, BROWSER_JOB],
    );
    let linked = Linked {
        hub,
        keeper: Arc::clone(&keeper),
        own,
    };
    drop(keeper);
    tokio::spawn(main_session::mirror(linked.clone(), Arc::clone(app)));
    (
        Arc::new(Announced {
            store: records,
            copies: Arc::clone(&copies),
        }),
        Some(linked),
        Some((heard, copies)),
    )
}

type Hearing = (mpsc::UnboundedReceiver<Heard>, Arc<Copies>);

async fn open_record(home: &Home) -> Result<SqliteStore, String> {
    let device = device::name(&home.device())?;
    let store = SqliteStore::new(home.record(), device.as_str());
    let [single, lines] = home.old_records();
    let Some(old) = [single, lines].into_iter().find(|path| path.exists()) else {
        return Ok(store);
    };
    let fail = |error: RecordError| error.to_string();
    let io = |error: std::io::Error| error.to_string();
    if !store.is_empty().await.map_err(fail)? {
        eprintln!(
            "dotori: {} is left alone because the record folder already has records",
            old.display()
        );
        return Ok(store);
    }
    drop(store);
    let earlier = match old.extension().and_then(|extension| extension.to_str()) {
        Some("jsonl") => JsonlStore::new(old.clone()).load().await,
        _ => {
            let path = old.clone();
            tokio::task::spawn_blocking(move || read_legacy(&path))
                .await
                .map_err(|error| error.to_string())?
        }
    }
    .map_err(fail)?;
    let staging = home.record().with_extension("moving");
    if staging.exists() {
        fs::remove_dir_all(&staging).map_err(io)?;
    }
    SqliteStore::new(staging.clone(), device.as_str())
        .import(earlier)
        .await
        .map_err(fail)?;
    if home.record().exists() {
        fs::remove_dir_all(home.record()).map_err(io)?;
    }
    fs::rename(&staging, home.record()).map_err(io)?;
    let extension = old
        .extension()
        .and_then(|extension| extension.to_str())
        .unwrap_or_default()
        .to_owned();
    let mut moved = old.with_extension(format!("{extension}.moved"));
    let mut n = 1;
    while moved.exists() {
        moved = old.with_extension(format!("{extension}.moved.{n}"));
        n += 1;
    }
    fs::rename(&old, moved).map_err(io)?;
    Ok(SqliteStore::new(home.record(), device))
}

fn basic(home: &Home, secrets: &[String]) -> BasicTools {
    BasicTools {
        folder: home.work(),
        limit: SHELL_LIMIT,
        hidden_env: secrets.to_vec(),
        private: home.work().parent().map(|dotori| Private {
            folder: dotori.to_path_buf(),
            open: home.work(),
        }),
    }
}

fn sub_session_runner(
    home: &Home,
    settings: &Settings,
    vault: &Vault,
    toolbox: &Arc<dyn Toolbox>,
    shared: Option<Arc<tools::SharedTools>>,
) -> Result<Arc<dyn SubSessionRunner>, String> {
    let chosen = settings.sub_sessions.clone().unwrap_or_default();
    let agent = match chosen.runner.unwrap_or_default() {
        RunnerChoice::Llm => {
            return Ok(Arc::new(LlmRunner {
                model: settings.sub_session_model(vault::lookup(vault))?,
                basic: basic(home, &vault::secret_names(settings, vault)),
                rest: Arc::clone(toolbox),
                max_turns: SUB_SESSION_TURNS,
                tool_timeout: TOOL_TIMEOUT,
            }));
        }
        RunnerChoice::ClaudeCode => CodingAgent::ClaudeCode,
        RunnerChoice::Codex => CodingAgent::Codex,
    };
    Ok(Arc::new(CliRunner {
        agent,
        program: chosen
            .path
            .or_else(|| agent.find())
            .unwrap_or_else(|| PathBuf::from(agent.program())),
        folder: home.work(),
        mcp: shared.map(|shared| shared as Arc<dyn McpServer>),
        limit: SUB_SESSION_LIMIT,
        hidden_env: settings.guarded_secrets(),
    }))
}

#[cfg(unix)]
async fn stop_signal() -> Result<(), String> {
    use tokio::signal::unix::SignalKind;
    use tokio::signal::unix::signal;
    let mut terminate = signal(SignalKind::terminate()).map_err(|error| error.to_string())?;
    tokio::select! {
        stopped = tokio::signal::ctrl_c() => stopped.map_err(|error| error.to_string()),
        _ = terminate.recv() => Ok(()),
    }
}

#[cfg(not(unix))]
async fn stop_signal() -> Result<(), String> {
    tokio::signal::ctrl_c()
        .await
        .map_err(|error| error.to_string())
}
