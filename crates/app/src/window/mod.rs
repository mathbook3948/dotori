mod chat;
mod input;
mod notes;
mod parts;
mod pcs;
mod rich;
mod routines;
mod screen;
mod settings;
mod theme;
mod themes;
mod when;

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
use gpui::WindowBounds;
use gpui::WindowOptions;
use gpui::actions;
use gpui::px;
use gpui::size;

use crate::home::Home;
use crate::link;
use crate::screen_link;
use crate::window::chat::ChatView;
use crate::window::parts::Assets;
use crate::window::settings::SettingsView;
use crate::window::theme::Look;

actions!(dotori, [Quit]);

const LOOK_EVERY: Duration = Duration::from_millis(500);

/// Which window the process opens. Each runs in a process of its own, so closing it frees
/// everything GPUI held.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Which {
    Chat,
    Settings,
}

pub(crate) fn run(home: Home, which: Which) -> Result<(), String> {
    let settings = match which {
        Which::Settings => Some(crate::settings::Settings::load(&home.settings())?),
        Which::Chat => None,
    };
    let settings_path = home.settings();
    let seen = stamp(&settings_path);
    apply_look(&settings_path);
    let failed = Rc::new(RefCell::new(None));
    let failure = Rc::clone(&failed);
    Application::new()
        .with_assets(Assets)
        .run(move |cx: &mut App| {
            let failed = failure;
            cx.bind_keys(input::bindings());
            cx.bind_keys([KeyBinding::new("secondary-q", Quit, None)]);
            let (title, wide, tall) = match which {
                Which::Chat => ("dotori", 560., 760.),
                Which::Settings => ("dotori 설정", 520., 640.),
            };
            let bounds = Bounds::centered(None, size(px(wide), px(tall)), cx);
            let options = WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(bounds)),
                window_min_size: Some(size(px(360.), px(420.))),
                titlebar: Some(TitlebarOptions {
                    title: Some(SharedString::new_static(title)),
                    ..TitlebarOptions::default()
                }),
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
                            SettingsView::new(home, settings, cx)
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
            follow_look(settings_path.clone(), seen, cx);
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

fn follow_look(settings: PathBuf, mut seen: Stamp, cx: &mut App) {
    cx.spawn(async move |cx| {
        loop {
            cx.background_executor().timer(LOOK_EVERY).await;
            let path = settings.clone();
            let last = seen;
            let (now, look) = cx
                .background_executor()
                .spawn(async move {
                    let now = stamp(&path);
                    let look = if now == last { None } else { saved_look(&path) };
                    (now, look)
                })
                .await;
            if now == seen {
                continue;
            }
            seen = now;
            if let Some((name, look)) = look {
                theme::choose(&name, look);
            }
            if cx.update(|cx| cx.refresh_windows()).is_err() {
                return;
            }
        }
    })
    .detach();
}
