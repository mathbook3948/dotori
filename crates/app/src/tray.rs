//! The tray icon of a daemon started on a desktop. Windows open as processes of their own, and
//! quitting from the tray stops the daemon.

use std::cell::RefCell;
use std::env;
use std::fs;
use std::panic;
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
use tray_icon::menu::MenuId;
use tray_icon::menu::MenuItem;

use crate::daemon;
use crate::home::Home;

const POLL: Duration = Duration::from_millis(100);
const ICON_SIZE: u32 = 32;
const ACORN: [u8; 3] = [0x8a, 0x5a, 0x1c];

/// Runs the tray on this thread and the daemon on the runtime's workers. Either one ending ends
/// both. If GPUI cannot reach the display, the daemon runs without a tray.
pub(crate) async fn run(home: Home) -> Result<(), String> {
    let previous = panic::take_hook();
    panic::set_hook(Box::new(|_| {}));
    let started = panic::catch_unwind(Application::new);
    panic::set_hook(previous);
    let Ok(application) = started else {
        eprintln!("dotori: no display, so no tray icon");
        return daemon::run(home).await;
    };
    let (quit, quitting) = oneshot::channel::<()>();
    let ready = Arc::new(AtomicBool::new(false));
    let daemon_ready = Arc::clone(&ready);
    let door = home.door();
    let daemon = tokio::spawn(daemon::serve(
        home,
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
        cx.spawn(async move |cx| watch(daemon, ready, quit, cx).await)
            .detach();
    });
    Ok(())
}

struct Items {
    chat: MenuId,
    settings: MenuId,
    quit: MenuItem,
}

async fn watch(
    daemon: JoinHandle<Result<(), String>>,
    ready: Arc<AtomicBool>,
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
        if !tried && ready.load(Ordering::SeqCst) {
            tried = true;
            match tray() {
                Ok(made) => shown = Some(made),
                Err(error) => eprintln!("dotori: no tray icon: {error}"),
            }
        }
        let Some((_, items)) = &shown else {
            continue;
        };
        while let Ok(event) = MenuEvent::receiver().try_recv() {
            if event.id == items.chat {
                open("window");
            } else if event.id == items.settings {
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
    let chat = MenuItem::new("대화창 열기", true, None);
    let settings = MenuItem::new("설정", true, None);
    let quit = MenuItem::new("끝내기", true, None);
    let menu = Menu::new();
    menu.append_items(&[&chat, &settings, &quit])
        .map_err(|error| error.to_string())?;
    let icon = Icon::from_rgba(acorn(), ICON_SIZE, ICON_SIZE).map_err(|error| error.to_string())?;
    let tray = TrayIconBuilder::new()
        .with_menu(Box::new(menu))
        .with_tooltip("dotori")
        .with_icon(icon)
        .build()
        .map_err(|error| error.to_string())?;
    let items = Items {
        chat: chat.id().clone(),
        settings: settings.id().clone(),
        quit,
    };
    Ok((tray, items))
}

fn open(command: &str) {
    let started = env::current_exe().and_then(|exe| Command::new(exe).arg(command).spawn());
    match started {
        Ok(mut child) => {
            thread::spawn(move || child.wait());
        }
        Err(error) => eprintln!("dotori: cannot open {command}: {error}"),
    }
}

/// A filled circle in the acorn color, as RGBA rows.
pub(crate) fn acorn() -> Vec<u8> {
    let center = (ICON_SIZE as f32 - 1.0) / 2.0;
    let radius = ICON_SIZE as f32 / 2.0 - 1.0;
    let mut pixels = Vec::with_capacity((ICON_SIZE * ICON_SIZE * 4) as usize);
    for y in 0..ICON_SIZE {
        for x in 0..ICON_SIZE {
            let distance = ((x as f32 - center).powi(2) + (y as f32 - center).powi(2)).sqrt();
            let alpha = if distance <= radius { 0xff } else { 0 };
            pixels.extend_from_slice(&ACORN);
            pixels.push(alpha);
        }
    }
    pixels
}

#[cfg(test)]
#[path = "tray.test.rs"]
mod tests;
