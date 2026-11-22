mod chat;
mod input;
mod settings;

use gpui::App;
use gpui::AppContext;
use gpui::Application;
use gpui::Bounds;
use gpui::Focusable;
use gpui::WindowBounds;
use gpui::WindowOptions;
use gpui::px;
use gpui::size;

use crate::home::Home;
use crate::link;
use crate::window::chat::ChatView;
use crate::window::settings::SettingsView;

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
    Application::new().run(move |cx: &mut App| {
        cx.bind_keys(input::bindings());
        let bounds = Bounds::centered(None, size(px(520.), px(640.)), cx);
        let options = WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(bounds)),
            ..WindowOptions::default()
        };
        let opened = match settings {
            None => cx
                .open_window(options, |window, cx| {
                    let view = cx.new(|cx| ChatView::new(link::open(home), cx));
                    window.focus(&view.read(cx).focus_handle(cx));
                    view
                })
                .map(|_| ()),
            Some(settings) => cx
                .open_window(options, |window, cx| {
                    let view = cx.new(|cx| SettingsView::new(home, settings, cx));
                    window.focus(&view.read(cx).focus_handle(cx));
                    view
                })
                .map(|_| ()),
        };
        if opened.is_err() {
            cx.quit();
            return;
        }
        cx.on_window_closed(|cx| {
            if cx.windows().is_empty() {
                cx.quit();
            }
        })
        .detach();
        cx.activate(true);
    });
    Ok(())
}
