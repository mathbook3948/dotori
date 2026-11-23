use std::fs;
use std::future;
use std::num::NonZeroUsize;
use std::path::PathBuf;
use std::sync::Arc;
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

use crate::browser_route::BROWSER_JOB;
use crate::browser_route::BrowserRoute;
use crate::device;
use crate::door;
use crate::door::Door;
use crate::home::Home;
use crate::idle::NoTools;
use crate::lock::DaemonLock;
use crate::main_session;
use crate::main_session::AgentApp;
use crate::main_session::Fenced;
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
use crate::settings::RunnerChoice;
use crate::settings::Settings;
use crate::settings::environment;
use crate::settings_copies::SettingsCopies;
use crate::tools;
use crate::vector_copies;
use crate::vector_copies::VECTORS_JOB;
use crate::vector_copies::VectorCopies;

const QUEUE: NonZeroUsize = NonZeroUsize::new(256).expect("non-zero");
const SEEN: NonZeroUsize = NonZeroUsize::new(4096).expect("non-zero");
const TOOL_TIMEOUT: Duration = Duration::from_secs(120);
const SUB_SESSION_TURNS: usize = 50;
const SUB_SESSION_LIMIT: Duration = Duration::from_secs(60 * 60);
const SETTLE: Duration = Duration::from_secs(1);
const RELEASE_SENT: Duration = Duration::from_millis(300);
const FLUSH_EVERY: Duration = Duration::from_secs(2);

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
    let model = settings.model(environment)?;
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
    let sub_sessions = sub_session_runner(&home, &settings, &toolbox, shared)?;
    let records = open_record(&home).await?;
    let notes = Notes::new(home.notes());
    let model: Arc<dyn Model> = Arc::new(Noted {
        model,
        notes: Arc::clone(&notes),
    });
    let browser = BrowserRoute::new(toolbox);
    let toolbox: Arc<dyn Toolbox> = Arc::new(MainTools {
        notes: Arc::clone(&notes),
        shared: Arc::clone(&browser) as Arc<dyn Toolbox>,
    });
    let token = door::token()?;
    let (app, address) = AppChannel::bind(token.clone()).map_err(|error| error.to_string())?;
    let app = Arc::new(app);
    let door = Door {
        port: address.port(),
        token,
    };
    let (records, linked, heard) = link(&home, &settings, records, &app).await;
    if let Some(linked) = &linked {
        browser.link(linked.clone());
    }
    vector_copies::write_jobs(&home.jobs(), linked.is_none());
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
                vector_copies::write_jobs(&jobs, makes);
                if holder.changed().await.is_err() {
                    return;
                }
            }
        });
    }
    let route = Route::new(linked.clone());
    if let (Some(linked), Some((mut heard, copies))) = (linked.clone(), heard) {
        let app = Arc::clone(&app);
        let route = Arc::clone(&route);
        tokio::spawn(notes::share(Arc::clone(&notes), Arc::clone(&linked.hub)));
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
        tokio::spawn(async move {
            while let Some(heard) = heard.recv().await {
                linked.keeper.hear(&heard);
                if settings_copies.hear(&linked, &heard) {
                    continue;
                }
                if vectors.as_ref().is_some_and(|vectors| vectors.hear(&heard)) {
                    continue;
                }
                if browser.hear(&heard) {
                    continue;
                }
                if notes::hear(&notes, &linked.hub, &heard) {
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
    if let Some(linked) = linked.clone() {
        let route = Arc::clone(&route);
        let app = Arc::clone(&app);
        let mut holder = linked.keeper.watch(MAIN_JOB);
        tokio::spawn(async move {
            while let Some(holder) = holder.as_mut() {
                if holder.changed().await.is_err() {
                    return;
                }
                let now = holder.borrow_and_update().clone();
                if let Some(key) = now {
                    app.answering(linked.hub.name_of(&key), key == linked.own);
                }
                route.flush();
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
                if let Some(key) = now {
                    app.browser_at(linked.hub.name_of(&key), key == linked.own);
                }
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
        let channels = settings
            .channels(environment)
            .unwrap_or_else(|error| {
                eprintln!("dotori: Slack is off: {error}");
                Vec::new()
            })
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
            toolbox: Arc::clone(&toolbox),
            sub_sessions: Arc::clone(&sub_sessions),
            records: Arc::clone(&records),
        };
        let agent = Agent::spawn(config.clone(), ports)
            .await
            .map_err(|error| error.to_string())?;
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
        agent.shutdown().await;
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

fn sub_session_runner(
    home: &Home,
    settings: &Settings,
    toolbox: &Arc<dyn Toolbox>,
    shared: Option<Arc<tools::SharedTools>>,
) -> Result<Arc<dyn SubSessionRunner>, String> {
    let chosen = settings.sub_sessions.clone().unwrap_or_default();
    let agent = match chosen.runner.unwrap_or_default() {
        RunnerChoice::Llm => {
            return Ok(Arc::new(LlmRunner {
                model: settings.sub_session_model(environment)?,
                toolbox: Arc::clone(toolbox),
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
