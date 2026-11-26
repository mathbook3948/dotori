//! How things sit on a window: cards and raised controls are the `surface` color with a soft
//! shadow under them and no border, and the main button is the text color filled in, so it reads
//! as the one thing to press.

use gpui::BoxShadow;
use gpui::Div;
use gpui::Hsla;
use gpui::IntoElement;
use gpui::ParentElement;
use gpui::Rgba;
use gpui::Styled;
use gpui::Window;
use gpui::div;
use gpui::point;
use gpui::prelude::FluentBuilder;
use gpui::px;

use crate::window::theme::Theme;

fn shade(a: f32) -> Hsla {
    Rgba {
        r: 0.16,
        g: 0.12,
        b: 0.08,
        a,
    }
    .into()
}

fn soft(theme: &Theme, lift: f32) -> Vec<BoxShadow> {
    let (near, far) = if theme.is_dark() {
        (0.35, 0.25)
    } else {
        (0.05, 0.05)
    };
    vec![
        BoxShadow {
            color: shade(near),
            offset: point(px(0.), px(1.)),
            blur_radius: px(2.),
            spread_radius: px(0.),
        },
        BoxShadow {
            color: shade(far),
            offset: point(px(0.), px(4. * lift)),
            blur_radius: px(14. * lift),
            spread_radius: px(0.),
        },
    ]
}

/// The look of something placed on the canvas.
pub(crate) trait SurfaceExt: Styled + Sized {
    /// A card or a field: the surface with a soft shadow and, in the dark, a faint light edge
    /// along its top.
    fn card(self, theme: &Theme) -> Self {
        self.bg(theme.surface)
            .border_t_1()
            .border_color(theme.rim)
            .shadow(soft(theme, 1.))
    }

    /// A control raised a little above what holds it, such as the chosen segment of a switch.
    fn raised(self, theme: &Theme) -> Self {
        self.bg(theme.surface).shadow(soft(theme, 0.4))
    }

    /// The shadow of a card on a fill that carries a meaning, such as a card waiting on the owner.
    fn lifted(self, theme: &Theme) -> Self {
        self.shadow(soft(theme, 1.))
    }

    /// A small round mark whose fill can match what it sits on, such as a theme's canvas color:
    /// a soft shade all around it sets it apart. A line around something this small breaks into
    /// steps.
    fn set_apart(self, theme: &Theme) -> Self {
        self.shadow(vec![BoxShadow {
            color: shade(if theme.is_dark() { 0.6 } else { 0.22 }),
            offset: point(px(0.), px(0.)),
            blur_radius: px(2.),
            spread_radius: px(0.),
        }])
    }

    /// The main button: the text color filled in, with the canvas color for its words.
    fn main_button(self, theme: &Theme) -> Self {
        self.bg(theme.text).text_color(theme.canvas)
    }
}

impl<E: Styled> SurfaceExt for E {}

/// The width of a line that covers whole pixels of the display, about one point wide. A line a
/// point wide on a 150% display falls on half pixels, and GPUI then draws one side thicker.
pub(crate) fn hairline(window: &Window) -> f32 {
    let scale = window.scale_factor();
    scale.round().max(1.) / scale
}

/// `inner`, whose corners are `radius`, framed by a line of `color` `width` wide, such as a
/// focused field's. Without a color the frame is clear and keeps its place.
pub(crate) fn framed(color: Option<Hsla>, radius: f32, width: f32, inner: impl IntoElement) -> Div {
    div()
        .p(px(width))
        .rounded(px(radius + width))
        .when_some(color, |frame, color| frame.bg(color))
        .child(inner)
}
