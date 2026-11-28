//! The frame dotori draws for its own windows where the OS lets it: a quiet strip at the top that
//! moves the window, round buttons, and edges that resize it.

use gpui::AnyElement;
use gpui::Decorations;
use gpui::Div;
use gpui::MouseButton;
use gpui::MouseDownEvent;
use gpui::Pixels;
use gpui::Point;
use gpui::ResizeEdge;
use gpui::Size;
use gpui::Stateful;
use gpui::Tiling;
use gpui::Window;
use gpui::WindowControlArea;
use gpui::div;
use gpui::prelude::*;
use gpui::px;
use gpui::svg;

use crate::window::parts::CLOSE_ICON;
use crate::window::parts::MAXIMIZE_ICON;
use crate::window::parts::MINIMIZE_ICON;
use crate::window::parts::RESTORE_ICON;
use crate::window::surface::SurfaceExt;
use crate::window::theme::Theme;

const RESIZE_MARGIN: f32 = 6.;
const TOP_LEFT: f32 = if cfg!(target_os = "macos") { 80. } else { 12. };
/// The height of the strip at the top of a window.
pub(crate) const TOP_HEIGHT: f32 = if cfg!(target_os = "macos") { 40. } else { 52. };

/// How the OS frames a window: whether dotori draws the frame itself, and which edges sit
/// against the screen or another window.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Frame {
    drawn: bool,
    tiling: Tiling,
    maximized: bool,
    bare: bool,
}

impl Frame {
    /// The frame `window` has now.
    pub(crate) fn of(window: &Window) -> Self {
        let maximized = window.is_maximized();
        match window.window_decorations() {
            Decorations::Client { tiling } => Self {
                drawn: true,
                tiling,
                maximized,
                bare: false,
            },
            Decorations::Server => Self {
                drawn: false,
                tiling: Tiling::default(),
                maximized,
                bare: false,
            },
        }
    }

    /// The same frame with no buttons in its strip, as while dotori starts: the strip still
    /// moves the window.
    pub(crate) fn bare(self) -> Self {
        Self { bare: true, ..self }
    }

    /// Lets the window be resized from its edges when dotori draws the frame.
    pub(crate) fn resizable(self, root: Div) -> Div {
        let tiling = self.tiling;
        root.when(self.drawn && !self.maximized, |root| {
            root.on_mouse_down(
                MouseButton::Left,
                move |event: &MouseDownEvent, window, cx| {
                    if let Some(edge) = resize_edge(event.position, window.viewport_size(), tiling)
                    {
                        window.start_window_resize(edge);
                        cx.stop_propagation();
                    }
                },
            )
        })
    }

    /// The strip at the top of a window: `left` after the system's buttons, `middle` taking the
    /// rest, and the window buttons on the right. Dragging it moves the window, and a double click
    /// fills the screen with it or brings it back. On Windows the system does both from the hit
    /// test, and only while no one in the window takes the press, so the strip leaves its presses
    /// alone there and nothing behind it takes them, which also keeps what scrolls under it out of
    /// reach. Its edges stay open for resizing.
    pub(crate) fn strip(
        self,
        theme: &Theme,
        left: impl IntoElement,
        middle: impl IntoElement,
    ) -> Div {
        let (drawn, tiling, maximized) = (self.drawn, self.tiling, self.maximized);
        let edge = if cfg!(target_os = "macos") {
            0.
        } else {
            RESIZE_MARGIN
        };
        div()
            .flex_none()
            .h(px(TOP_HEIGHT))
            .pt(px(edge))
            .px(px(edge))
            .child(
                div()
                    .size_full()
                    .block_mouse_except_scroll()
                    .flex()
                    .items_center()
                    .gap_2()
                    .pl(px(TOP_LEFT - edge))
                    .pr(px(12. - edge))
                    .child(
                        div()
                            .id("top")
                            .h_full()
                            .flex_1()
                            .min_w_0()
                            .flex()
                            .items_center()
                            .gap_2()
                            .window_control_area(WindowControlArea::Drag)
                            .when(!cfg!(target_os = "windows"), |top| {
                                top.on_mouse_down(
                                    MouseButton::Left,
                                    move |event: &MouseDownEvent, window, cx| {
                                        if event.click_count > 1 {
                                            window.zoom_window();
                                            cx.stop_propagation();
                                            return;
                                        }
                                        let at = (event.position, window.viewport_size());
                                        if drawn
                                            && (maximized
                                                || resize_edge(at.0, at.1, tiling).is_none())
                                        {
                                            window.start_window_move();
                                        }
                                    },
                                )
                            })
                            .child(left)
                            .child(div().flex_1().min_w_0().child(middle)),
                    )
                    .when(!self.bare, |strip| {
                        strip.children(window_buttons(theme, drawn, maximized))
                    }),
            )
    }
}

/// A round button of the top strip, raised a little off the canvas.
pub(crate) fn round_button(id: &'static str, theme: &Theme) -> Stateful<Div> {
    div()
        .id(id)
        .debug_selector(move || id.to_owned())
        .flex()
        .flex_none()
        .items_center()
        .justify_center()
        .size(px(36.))
        .rounded_full()
        .card(theme)
        .cursor_pointer()
        .hover(|button| button.bg(theme.bubble))
}

fn window_buttons(theme: &Theme, drawn: bool, maximized: bool) -> Vec<AnyElement> {
    let buttons = [
        ("minimize", MINIMIZE_ICON, WindowControlArea::Min),
        (
            "maximize",
            if maximized {
                RESTORE_ICON
            } else {
                MAXIMIZE_ICON
            },
            WindowControlArea::Max,
        ),
        ("close", CLOSE_ICON, WindowControlArea::Close),
    ]
    .into_iter();
    let native = cfg!(target_os = "windows");
    if !native && !drawn {
        return Vec::new();
    }
    buttons
        .map(|(id, icon, area)| {
            let button = round_button(id, theme)
                .child(svg().path(icon).size(px(16.)).text_color(theme.muted));
            if native {
                return button.window_control_area(area).into_any_element();
            }
            button
                .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                .on_click(move |_, window, _| match area {
                    WindowControlArea::Min => window.minimize_window(),
                    WindowControlArea::Max => window.zoom_window(),
                    _ => window.remove_window(),
                })
                .into_any_element()
        })
        .collect()
}

fn resize_edge(at: Point<Pixels>, size: Size<Pixels>, tiling: Tiling) -> Option<ResizeEdge> {
    let near = px(RESIZE_MARGIN);
    let left = !tiling.left && at.x < near;
    let right = !tiling.right && at.x > size.width - near;
    let top = !tiling.top && at.y < near;
    let bottom = !tiling.bottom && at.y > size.height - near;
    match (top, bottom, left, right) {
        (true, _, true, _) => Some(ResizeEdge::TopLeft),
        (true, _, _, true) => Some(ResizeEdge::TopRight),
        (_, true, true, _) => Some(ResizeEdge::BottomLeft),
        (_, true, _, true) => Some(ResizeEdge::BottomRight),
        (true, _, _, _) => Some(ResizeEdge::Top),
        (_, true, _, _) => Some(ResizeEdge::Bottom),
        (_, _, true, _) => Some(ResizeEdge::Left),
        (_, _, _, true) => Some(ResizeEdge::Right),
        _ => None,
    }
}

#[cfg(all(test, target_os = "windows"))]
#[path = "chrome.test.rs"]
mod tests;
