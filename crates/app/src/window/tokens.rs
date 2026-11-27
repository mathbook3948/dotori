//! The tokens of DESIGN.md that are not theme colors: the sizes of the windows and the pictures,
//! the fill of the picture tile and the colors of the greeting's features. A test reads
//! DESIGN.md, so a change there fails until it is made here too.

use gpui::Rgba;

use crate::window::theme::Theme;
use crate::window::theme::hex;

/// `size.window`: the width and height every window opens at.
pub(crate) const WINDOW: (f32, f32) = (880., 640.);
/// `size.window-min`: the smallest the chat window shrinks to.
pub(crate) const WINDOW_MIN: (f32, f32) = (720., 480.);
/// `size.column`: the widest the conversation and the composer grow in the chat window.
pub(crate) const COLUMN: f32 = 720.;
/// `size.walk`: the widest an onboarding step grows, and the greeting's column.
pub(crate) const WALK: f32 = 560.;
/// `size.greet-text`: the column of the greeting on the left of the first screen.
pub(crate) const GREET_TEXT: f32 = 380.;
/// `size.sidebar`: the width of the settings window's list of sections.
pub(crate) const SIDEBAR: f32 = 220.;
/// `size.picture`: the assistant's picture where a window shows it large.
pub(crate) const PICTURE: f32 = 56.;
/// `size.tile`: the picture tile of the greeting.
pub(crate) const TILE: f32 = 72.;
/// `size.avatar`: the picture tile of the name and theme step.
pub(crate) const AVATAR: f32 = 60.;

/// `tile`: the fill of the picture tile in the light look and in the dark one.
pub(crate) const TILE_FILL: (Rgba, Rgba) = (hex(0xFCE8CCFF), hex(0x30241AFF));

/// `feature`: the tiles of what the greeting says dotori can do, and the icon drawn on them.
pub(crate) struct Feature {
    pub(crate) time: Rgba,
    pub(crate) web: Rgba,
    pub(crate) files: Rgba,
    pub(crate) icon: Rgba,
}

pub(crate) const FEATURE: Feature = Feature {
    time: hex(0xD07A22FF),
    web: hex(0x4C8DF6FF),
    files: hex(0x2E9E6CFF),
    icon: hex(0xFFFFFFFF),
};

impl Theme {
    /// The fill of the picture tile in this look.
    pub(crate) fn tile(&self) -> Rgba {
        if self.is_dark() {
            TILE_FILL.1
        } else {
            TILE_FILL.0
        }
    }
}

#[cfg(test)]
#[path = "tokens.test.rs"]
mod tests;
