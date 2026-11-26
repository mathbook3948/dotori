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
use crate::window::parts::MINIMIZE_ICON;
use crate::window::surface::SurfaceExt;
use crate::window::theme::Theme;

const RESIZE_MARGIN: f32 = 6.;
const TOP_LEFT: f32 = if cfg!(target_os = "macos") { 80. } else { 12. };
const TOP_HEIGHT: f32 = if cfg!(target_os = "macos") { 40. } else { 52. };

/// How the OS frames a window: whether dotori draws the frame itself, and which edges sit
/// against the screen or another window.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Frame {
    drawn: bool,
    tiling: Tiling,
    fixed: bool,
}

impl Frame {
    /// The frame `window` has now.
    pub(crate) fn of(window: &Window) -> Self {
        match window.window_decorations() {
            Decorations::Client { tiling } => Self {
                drawn: true,
                tiling,
                fixed: false,
            },
            Decorations::Server => Self {
                drawn: false,
                tiling: Tiling::default(),
                fixed: false,
            },
        }
    }

    /// The same frame for a window of one size: no resizing edges, and only the close button.
    pub(crate) fn fixed(self) -> Self {
        Self {
            fixed: true,
            ..self
        }
    }

    /// Lets the window be resized from its edges when dotori draws the frame.
    pub(crate) fn resizable(self, root: Div) -> Div {
        let tiling = self.tiling;
        root.when(self.drawn && !self.fixed, |root| {
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
    /// rest, and the window buttons on the right. Dragging it moves the window. A double click
    /// does nothing, since dotori's windows never fill the screen.
    pub(crate) fn strip(
        self,
        theme: &Theme,
        left: impl IntoElement,
        middle: impl IntoElement,
    ) -> Div {
        let (drawn, tiling, fixed) = (self.drawn, self.tiling, self.fixed);
        div()
            .flex_none()
            .h(px(TOP_HEIGHT))
            .when(!cfg!(target_os = "macos"), |top| top.pt(px(RESIZE_MARGIN)))
            .child(
                div()
                    .size_full()
                    .flex()
                    .items_center()
                    .gap_2()
                    .pl(px(TOP_LEFT))
                    .pr(px(12.))
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
                            .on_mouse_down(
                                MouseButton::Left,
                                move |event: &MouseDownEvent, window, cx| {
                                    if event.click_count > 1 {
                                        cx.stop_propagation();
                                        return;
                                    }
                                    let at = (event.position, window.viewport_size());
                                    if drawn && (fixed || resize_edge(at.0, at.1, tiling).is_none())
                                    {
                                        window.start_window_move();
                                    }
                                },
                            )
                            .child(left)
                            .child(div().flex_1().min_w_0().child(middle)),
                    )
                    .children(window_buttons(theme, drawn, fixed)),
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

fn window_buttons(theme: &Theme, drawn: bool, fixed: bool) -> Vec<AnyElement> {
    let buttons = [
        ("minimize", MINIMIZE_ICON, WindowControlArea::Min),
        ("close", CLOSE_ICON, WindowControlArea::Close),
    ]
    .into_iter()
    .filter(|(id, _, _)| !fixed || *id == "close");
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
