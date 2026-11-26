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
use gpui::WindowBackgroundAppearance;
use gpui::WindowBounds;
use gpui::WindowDecorations;
use gpui::WindowOptions;
use gpui::actions;
use gpui::point;
use gpui::px;
use gpui::size;

use crate::home::Home;
use crate::language;
use crate::link;
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
        Which::Settings | Which::Onboarding => {
            Some(crate::settings::Settings::load(&home.settings())?)
        }
        Which::Chat => None,
    };
    let settings_path = home.settings();
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
            cx.bind_keys([KeyBinding::new("secondary-q", Quit, None)]);
            let title = match which {
                Which::Chat => identity::name(),
                Which::Settings => (words().common.settings_title)(),
                Which::Onboarding => (words().common.onboarding_title)(),
            };
            let bounds = Bounds::centered(None, size(px(WINDOW.0), px(WINDOW.1)), cx);
            let options = WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(bounds)),
                window_min_size: Some(size(px(WINDOW_MIN.0), px(WINDOW_MIN.1))),
                is_resizable: !matches!(which, Which::Onboarding),
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
                            ChatView::new(link::open(home.clone()), screen_link::open(home), cx)
                        });
                        window.focus(&view.read(cx).focus_handle(cx));
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
                        window.focus(&view.read(cx).focus_handle(cx));
                        view
                    })
                    .map(|_| ()),
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

type Stamp = Option<(SystemTime, u64)>;

fn stamp(path: &Path) -> Stamp {
    let found = fs::metadata(path).ok()?;
    Some((found.modified().ok()?, found.len()))
}

fn follow_look(settings: PathBuf, icon_folder: PathBuf, mut seen: Stamp, cx: &mut App) {
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
            if now == seen && !renamed {
                continue;
            }
            seen = now;
            if let Some(((name, look), speaking)) = look {
                theme::choose(&name, look);
                language::choose(speaking);
            }
            if cx.update(|cx| cx.refresh_windows()).is_err() {
                return;
            }
        }
    })
    .detach();
}
