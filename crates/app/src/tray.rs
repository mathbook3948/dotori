//! The tray icon that dotori runs with its daemon. Windows open as processes of their own, and
//! quitting from the tray stops the daemon.

use std::cell::RefCell;
use std::env;
use std::fs;
use std::panic;
use std::path::PathBuf;
use std::process;
use std::process::Command;
use std::rc::Rc;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use std::sync::atomic::Ordering;
use std::thread;
use std::time::Duration;

use gpui::App;
use gpui::Application;
use gpui::AsyncApp;
use tokio::sync::oneshot;
use tokio::task::JoinHandle;
use tray_icon::Icon;
use tray_icon::TrayIcon;
use tray_icon::TrayIconBuilder;
use tray_icon::menu::Menu;
use tray_icon::menu::MenuEvent;
use tray_icon::menu::MenuItem;

use crate::daemon;
use crate::home::Home;
use crate::language;
use crate::language::Language;
use crate::window::app_icon;
use crate::window::identity;
use crate::words::words;

const POLL: Duration = Duration::from_millis(100);
const ICON_SIZE: u32 = 32;

/// Runs the tray on this thread and the daemon on the runtime's workers. Either one ending ends
/// both. Without a display it does not start.
pub(crate) async fn run(home: Home) -> Result<(), String> {
    let Some(application) = application() else {
        return Err(String::from("dotori needs a desktop, and none was found"));
    };
    let (quit, quitting) = oneshot::channel::<()>();
    let ready = Arc::new(AtomicBool::new(false));
    let daemon_ready = Arc::clone(&ready);
    let onboarding = Arc::new(AtomicBool::new(false));
    let daemon_onboarding = Arc::clone(&onboarding);
    let door = home.door();
    let daemon = tokio::spawn(daemon::serve(
        home,
        Some(move || daemon_onboarding.store(true, Ordering::SeqCst)),
        move || daemon_ready.store(true, Ordering::SeqCst),
        async {
            let _ = quitting.await;
        },
    ));
    let quit = Rc::new(RefCell::new(Some(quit)));
    application.run(move |cx: &mut App| {
        let on_quit = Rc::clone(&quit);
        cx.on_app_quit(move |_| {
            if let Some(quit) = on_quit.borrow_mut().take() {
                let _ = quit.send(());
                let _ = fs::remove_file(&door);
            }
            async {}
        })
        .detach();
        cx.spawn(async move |cx| watch(daemon, ready, onboarding, quit, cx).await)
            .detach();
    });
    Ok(())
}

fn application() -> Option<Application> {
    #[cfg(target_os = "linux")]
    if gpui::guess_compositor() == "Headless" {
        return None;
    }
    let previous = panic::take_hook();
    panic::set_hook(Box::new(|_| {}));
    let started = panic::catch_unwind(Application::new);
    panic::set_hook(previous);
    started.ok()
}

struct Items {
    chat: MenuItem,
    settings: MenuItem,
    quit: MenuItem,
    shown: (String, Language),
    pictured: Option<PathBuf>,
}

impl Items {
    fn follow(&mut self, tray: &TrayIcon) {
        let picture = identity::icon();
        if picture != self.pictured {
            match tray_picture() {
                Ok(shown) => {
                    if let Err(error) = tray.set_icon(Some(shown)) {
                        eprintln!("dotori: the tray keeps its old picture: {error}");
                    }
                }
                Err(error) => eprintln!("dotori: the tray keeps its old picture: {error}"),
            }
            self.pictured = picture;
        }
        let now = (identity::name(), language::current());
        if now == self.shown {
            return;
        }
        let words = &words().common;
        if let Err(error) = tray.set_tooltip(Some(&now.0)) {
            eprintln!("dotori: the tray keeps its old name: {error}");
        }
        self.chat.set_text(words.open_chat);
        self.settings.set_text(words.settings);
        self.quit.set_text(words.quit);
        self.shown = now;
    }
}

async fn watch(
    daemon: JoinHandle<Result<(), String>>,
    ready: Arc<AtomicBool>,
    onboarding: Arc<AtomicBool>,
    quit: Rc<RefCell<Option<oneshot::Sender<()>>>>,
    cx: &mut AsyncApp,
) {
    let mut shown = None;
    let mut tried = false;
    loop {
        cx.background_executor().timer(POLL).await;
        if daemon.is_finished() {
            let stopped = match daemon.await {
                Ok(stopped) => stopped,
                Err(error) => Err(error.to_string()),
            };
            drop(shown);
            if let Err(error) = stopped {
                eprintln!("dotori: {error}");
                process::exit(1);
            }
            quit.borrow_mut().take();
            let _ = cx.update(|cx| cx.quit());
            return;
        }
        let walking = onboarding.load(Ordering::SeqCst);
        if !tried && (ready.load(Ordering::SeqCst) || walking) {
            tried = true;
            if walking {
                open("onboarding");
            }
            match tray() {
                Ok(made) => shown = Some(made),
                Err(error) => eprintln!("dotori: no tray icon: {error}"),
            }
        }
        let Some((tray, items)) = &mut shown else {
            continue;
        };
        items.follow(tray);
        while let Ok(event) = MenuEvent::receiver().try_recv() {
            if event.id == *items.chat.id() {
                open("window");
            } else if event.id == *items.settings.id() {
                open("settings");
            } else if event.id == *items.quit.id()
                && let Some(quit) = quit.borrow_mut().take()
            {
                items.quit.set_enabled(false);
                let _ = quit.send(());
            }
        }
    }
}

fn tray() -> Result<(TrayIcon, Items), String> {
    let words = &words().common;
    let chat = MenuItem::new(words.open_chat, true, None);
    let settings = MenuItem::new(words.settings, true, None);
    let quit = MenuItem::new(words.quit, true, None);
    let menu = Menu::new();
    menu.append_items(&[&chat, &settings, &quit])
        .map_err(|error| error.to_string())?;
    let icon = tray_picture()?;
    let name = identity::name();
    let tray = TrayIconBuilder::new()
        .with_menu(Box::new(menu))
        .with_tooltip(&name)
        .with_icon(icon)
        .build()
        .map_err(|error| error.to_string())?;
    let items = Items {
        chat,
        settings,
        quit,
        shown: (name, language::current()),
        pictured: identity::icon(),
    };
    Ok((tray, items))
}

/// Starts another dotori process that shows `command`, such as the settings window.
pub(crate) fn open(command: &str) {
    launch(&[command]);
}

/// Starts dotori with its tray, as a window does when it finds dotori not running.
pub(crate) fn start() {
    launch(&[]);
}

fn launch(arguments: &[&str]) {
    if cfg!(test) {
        return;
    }
    let started = env::current_exe().and_then(|exe| Command::new(exe).args(arguments).spawn());
    match started {
        Ok(mut child) => {
            thread::spawn(move || child.wait());
        }
        Err(error) => eprintln!("dotori: cannot start {arguments:?}: {error}"),
    }
}

/// The assistant's picture as the tray shows it: the owner's picture cut to a rounded square, or
/// the acorn.
pub(crate) fn tray_picture() -> Result<Icon, String> {
    Icon::from_rgba(app_icon::rgba(ICON_SIZE), ICON_SIZE, ICON_SIZE)
        .map_err(|error| error.to_string())
}

#[cfg(test)]
#[path = "tray.test.rs"]
mod tests;
