use gpui::App;
use gpui::Bounds;
use gpui::Context;
use gpui::Entity;
use gpui::Focusable;
use gpui::SharedString;
use gpui::TitlebarOptions;
use gpui::Window;
use gpui::WindowBounds;
use gpui::WindowHandle;
use gpui::WindowOptions;
use gpui::div;
use gpui::prelude::*;
use gpui::px;
use gpui::size;

use crate::window::screen::ScreenView;
use crate::window::theme::Theme;

/// The window dotori's browser shows in when the owner has something to do there. It opens from
/// a card in the chat and closes when the owner is done.
pub(crate) struct BrowserWindow {
    screen: Entity<ScreenView>,
}

impl Render for BrowserWindow {
    fn render(&mut self, window: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        let theme = Theme::of(window);
        div()
            .size_full()
            .bg(theme.canvas)
            .text_color(theme.text)
            .text_sm()
            .child(self.screen.clone())
    }
}

/// Shows the browser window, opening it if it is not open, and hands the browser to the owner
/// when `take` is set. Closing the window gives the browser back to dotori.
pub(crate) fn show(
    open: Option<WindowHandle<BrowserWindow>>,
    screen: &Entity<ScreenView>,
    take: bool,
    cx: &mut App,
) -> Option<WindowHandle<BrowserWindow>> {
    if let Some(open) = open
        && open
            .update(cx, |_, window, cx| {
                window.activate_window();
                if take {
                    screen.update(cx, |screen, cx| screen.take_over(window, cx));
                }
            })
            .is_ok()
    {
        return Some(open);
    }
    let bounds = Bounds::centered(None, size(px(1040.), px(720.)), cx);
    let options = WindowOptions {
        window_bounds: Some(WindowBounds::Windowed(bounds)),
        window_min_size: Some(size(px(480.), px(360.))),
        titlebar: Some(TitlebarOptions {
            title: Some(SharedString::new_static("dotori 브라우저")),
            ..TitlebarOptions::default()
        }),
        app_id: Some(String::from("dotori")),
        ..WindowOptions::default()
    };
    let screen = screen.clone();
    cx.open_window(options, |window, cx| {
        let closing = screen.clone();
        window.on_window_should_close(cx, move |_, cx| {
            let closing = closing.clone();
            cx.spawn(async move |cx| {
                let _ = closing.update(cx, |screen, cx| screen.hand_back(cx));
            })
            .detach();
            true
        });
        if take {
            screen.update(cx, |screen, cx| screen.take_over(window, cx));
        }
        window.focus(&screen.focus_handle(cx));
        cx.new(|cx| {
            cx.observe_window_appearance(window, |_, _, cx| cx.notify())
                .detach();
            BrowserWindow { screen }
        })
    })
    .ok()
}
