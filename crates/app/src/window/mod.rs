pub(crate) mod app_icon;
mod browser;
mod chat;
mod chrome;
pub(crate) mod identity;
mod input;
mod motion;
mod notes;
mod parts;
mod pcs;
mod rich;
mod routines;
mod sandbox;
mod screen;
mod select;
mod settings;
mod surface;
mod theme;
mod themes;
mod tokens;
mod when;

use std::borrow::Cow;
use std::cell::RefCell;
use std::fs;
use std::path::Path;
use std::path::PathBuf;
use std::rc::Rc;
use std::time::Duration;
use std::time::SystemTime;

use gpui::App;
use gpui::AppContext;
use gpui::Application;
use gpui::Bounds;
use gpui::Focusable;
use gpui::KeyBinding;
use gpui::SharedString;
use gpui::TitlebarOptions;
use gpui::Window;
use gpui::WindowBackgroundAppearance;
use gpui::WindowBounds;
use gpui::WindowDecorations;
use gpui::WindowHandle;
use gpui::WindowOptions;
use gpui::actions;
use gpui::point;
use gpui::px;
use gpui::size;

use crate::home::Home;
use crate::language;
use crate::link;
use crate::lock::DaemonLock;
use crate::screen_link;
use crate::window::chat::ChatView;
use crate::window::identity::Identity;
use crate::window::parts::Assets;
use crate::window::parts::FONT_FILES;
use crate::window::settings::SettingsView;
use crate::window::theme::Look;
use crate::window::tokens::WINDOW;
use crate::window::tokens::WINDOW_MIN;
use crate::words::words;

actions!(dotori, [Quit]);

const LOOK_EVERY: Duration = Duration::from_millis(500);
const ICON_EVERY: u32 = 4;

/// Which window the process opens. Each runs in a process of its own, so closing it frees
/// everything GPUI held.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Which {
    Chat,
    Settings,
    Onboarding,
}

pub(crate) fn run(home: Home, which: Which) -> Result<(), String> {
    let settings = match which {
        Which::Onboarding => Some(crate::settings::Settings::load(&home.settings())?),
        Which::Chat | Which::Settings => None,
    };
    let settings_path = home.settings();
    let home_lock = home.lock();
    let icon_folder = home.icons();
    let seen = stamp(&settings_path);
    apply_look(&settings_path);
    language::choose(language::saved(&settings_path));
    identity::choose(Identity::read(&settings_path, &icon_folder));
    let failed = Rc::new(RefCell::new(None));
    let failure = Rc::clone(&failed);
    Application::new()
        .with_assets(Assets)
        .run(move |cx: &mut App| {
            let failed = failure;
            let fonts = FONT_FILES.iter().map(|file| Cow::Borrowed(*file)).collect();
            if let Err(error) = cx.text_system().add_fonts(fonts) {
                eprintln!("dotori: the windows write in the system font: {error}");
            }
            cx.bind_keys(input::bindings());
            cx.bind_keys(chat::bindings());
            cx.bind_keys([KeyBinding::new("secondary-q", Quit, None)]);
            let title = match which {
                Which::Chat | Which::Settings => identity::name(),
                Which::Onboarding => (words().common.onboarding_title)(),
            };
            let (opens_at, smallest) = (WINDOW, WINDOW_MIN);
            let bounds = Bounds::centered(None, size(px(opens_at.0), px(opens_at.1)), cx);
            let options = WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(bounds)),
                window_min_size: Some(size(px(smallest.0), px(smallest.1))),
                is_resizable: true,
                titlebar: Some(TitlebarOptions {
                    title: Some(SharedString::from(title)),
                    appears_transparent: true,
                    traffic_light_position: Some(point(px(16.), px(12.))),
                }),
                window_background: WindowBackgroundAppearance::Opaque,
                window_decorations: Some(WindowDecorations::Client),
                app_id: Some(String::from("dotori")),
                ..WindowOptions::default()
            };
            let opened = match settings {
                None => cx
                    .open_window(options, |window, cx| {
                        let view = cx.new(|cx| {
                            cx.observe_window_appearance(window, |_, _, cx| cx.notify())
                                .detach();
                            ChatView::new(
                                link::open(home.clone()),
                                screen_link::open(home.clone()),
                                Some(home),
                                which == Which::Settings,
                                cx,
                            )
                        });
                        window.focus(&view.read(cx).focus_handle(cx));
                        app_icon::show_on(window);
                        view
                    })
                    .map(|_| ()),
                Some(settings) => cx
                    .open_window(options, |window, cx| {
                        let view = cx.new(|cx| {
                            cx.observe_window_appearance(window, |_, _, cx| cx.notify())
                                .detach();
                            match which {
                                Which::Onboarding => SettingsView::onboarding(home, settings, cx),
                                _ => SettingsView::new(home, settings, cx),
                            }
                        });
                        if which == Which::Onboarding {
                            window.focus(&view.read(cx).focus_handle(cx));
                        }
                        app_icon::show_on(window);
                        view
                    })
                    .map(|handle| close_with_dotori(home_lock.clone(), handle, cx)),
            };
            if let Err(error) = opened {
                *failed.borrow_mut() = Some(error.to_string());
                cx.quit();
                return;
            }
            follow_look(settings_path.clone(), icon_folder.clone(), seen, cx);
            cx.on_action(|_: &Quit, cx| cx.quit());
            cx.on_window_closed(|cx| {
                if cx.windows().is_empty() {
                    cx.quit();
                }
            })
            .detach();
            cx.activate(true);
        });
    match failed.take() {
        Some(error) => Err(format!("cannot open a window: {error}")),
        None => Ok(()),
    }
}

fn saved_look(settings: &Path) -> Option<(String, Look)> {
    let chosen = crate::settings::Settings::load(settings)
        .ok()?
        .window
        .unwrap_or_default();
    Some((
        chosen.theme.unwrap_or_default(),
        Look::named(chosen.look.as_deref().unwrap_or_default()),
    ))
}

fn apply_look(settings: &Path) {
    if let Some((name, look)) = saved_look(settings) {
        theme::choose(&name, look);
    }
}

fn dotori_stopped(seen: &mut bool, held: bool) -> bool {
    if held {
        *seen = true;
        return false;
    }
    *seen
}

fn close_with_dotori(lock: PathBuf, handle: WindowHandle<SettingsView>, cx: &mut App) {
    cx.spawn(async move |cx| {
        let mut seen = false;
        loop {
            cx.background_executor().timer(LOOK_EVERY).await;
            let path = lock.clone();
            let held = cx
                .background_executor()
                .spawn(async move { DaemonLock::held(&path) })
                .await;
            let Ok(held) = held else {
                continue;
            };
            if dotori_stopped(&mut seen, held) {
                let _ = handle.update(cx, |view, _, cx| view.save_all(cx));
                let _ = cx.update(|cx| cx.quit());
                return;
            }
        }
    })
    .detach();
}

/// A window's title, which names the assistant in the language in use. The window sets it
/// again when either changes, since the title bar, the taskbar and Alt+Tab show it.
pub(crate) struct Title {
    says: fn() -> String,
    shown: String,
}

impl Title {
    /// A title that reads `says`, as the window opened with it.
    pub(crate) fn new(says: fn() -> String) -> Self {
        Self {
            shown: says(),
            says,
        }
    }

    /// Sets the title of `window` again if what it says changed.
    pub(crate) fn keep(&mut self, window: &mut Window) {
        let now = (self.says)();
        if now != self.shown {
            window.set_window_title(&now);
            self.shown = now;
        }
    }
}

/// When a file was last written and how long it is, to tell that it changed.
pub(crate) type Stamp = Option<(SystemTime, u64)>;

/// The stamp of the file at `path`, or none when it cannot be read.
pub(crate) fn stamp(path: &Path) -> Stamp {
    let found = fs::metadata(path).ok()?;
    Some((found.modified().ok()?, found.len()))
}

fn follow_look(settings: PathBuf, icon_folder: PathBuf, mut seen: Stamp, cx: &mut App) {
    let mut shown_icon = identity::icon();
    cx.spawn(async move |cx| {
        let mut ticks = 0_u32;
        loop {
            cx.background_executor().timer(LOOK_EVERY).await;
            ticks = ticks.wrapping_add(1);
            let path = settings.clone();
            let folder = icon_folder.clone();
            let last = seen;
            let look_again = ticks.is_multiple_of(ICON_EVERY);
            let (now, look, identity) = cx
                .background_executor()
                .spawn(async move {
                    let now = stamp(&path);
                    let changed = now != last;
                    let look = if changed {
                        saved_look(&path).map(|look| (look, language::saved(&path)))
                    } else {
                        None
                    };
                    let identity = (changed || look_again).then(|| Identity::read(&path, &folder));
                    (now, look, identity)
                })
                .await;
            let renamed = identity.is_some_and(identity::choose);
            let icon = identity::icon();
            let new_icon = icon != shown_icon;
            if now == seen && !renamed && !new_icon {
                continue;
            }
            shown_icon = icon;
            seen = now;
            if let Some(((name, look), speaking)) = look {
                theme::choose(&name, look);
                language::choose(speaking);
            }
            let refreshed = cx.update(|cx| {
                if new_icon {
                    for window in cx.windows() {
                        let _ = window.update(cx, |_, window, _| app_icon::show_on(window));
                    }
                }
                cx.refresh_windows();
            });
            if refreshed.is_err() {
                return;
            }
        }
    })
    .detach();
}

#[cfg(test)]
#[path = "mod.test.rs"]
mod tests;
