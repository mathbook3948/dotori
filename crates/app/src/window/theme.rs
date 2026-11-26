use std::sync::RwLock;

use gpui::Rgba;
use gpui::Window;
use gpui::WindowAppearance;

use crate::window::themes::THEMES;

/// The colors every window draws with, named as the color tokens of DESIGN.md.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Theme {
    pub(crate) canvas: Rgba,
    pub(crate) surface: Rgba,
    pub(crate) raised: Rgba,
    pub(crate) line: Rgba,
    pub(crate) bubble: Rgba,
    pub(crate) text: Rgba,
    pub(crate) muted: Rgba,
    pub(crate) primary: Rgba,
    pub(crate) selection: Rgba,
    pub(crate) ok: Rgba,
    pub(crate) ok_soft: Rgba,
    pub(crate) warn: Rgba,
    pub(crate) warn_soft: Rgba,
    pub(crate) danger: Rgba,
    pub(crate) danger_soft: Rgba,
    pub(crate) rim: Rgba,
}

/// The two colors of dotori's acorn logo.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Logo {
    pub(crate) cap: Rgba,
    pub(crate) nut: Rgba,
}

/// Whether the windows follow the system's light or dark look, or keep to one.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum Look {
    #[default]
    System,
    Light,
    Dark,
}

impl Look {
    /// The look named in the settings, or the system's for any other word.
    pub(crate) fn named(name: &str) -> Self {
        match name {
            "light" => Self::Light,
            "dark" => Self::Dark,
            _ => Self::System,
        }
    }

    /// The word the settings keep for this look.
    pub(crate) fn name(self) -> &'static str {
        match self {
            Self::System => "system",
            Self::Light => "light",
            Self::Dark => "dark",
        }
    }
}

/// A color from its `0xRRGGBBAA` value.
pub(crate) const fn hex(value: u32) -> Rgba {
    Rgba {
        r: ((value >> 24) & 0xFF) as f32 / 255.0,
        g: ((value >> 16) & 0xFF) as f32 / 255.0,
        b: ((value >> 8) & 0xFF) as f32 / 255.0,
        a: (value & 0xFF) as f32 / 255.0,
    }
}

const THEME_VARIABLE: &str = "DOTORI_THEME";

static CHOSEN: RwLock<(usize, Look)> = RwLock::new((0, Look::System));

/// The names of the themes, the default first.
pub(crate) fn names() -> impl Iterator<Item = &'static str> {
    THEMES.iter().map(|(name, _, _)| *name)
}

/// The light and dark colors of the theme called `name`.
pub(crate) fn named(name: &str) -> Option<(Theme, Theme)> {
    THEMES
        .iter()
        .find(|(known, _, _)| *known == name)
        .map(|(_, light, dark)| (*light, *dark))
}

/// Draws every window of this process with the theme called `name` in `look` from the next
/// frame. An unknown name keeps the default theme.
pub(crate) fn choose(name: &str, look: Look) {
    let index = THEMES
        .iter()
        .position(|(known, _, _)| *known == name)
        .unwrap_or(0);
    *CHOSEN
        .write()
        .unwrap_or_else(|poisoned| poisoned.into_inner()) = (index, look);
}

/// The theme and look the owner chose in the settings, which the settings window shows.
pub(crate) fn saved() -> (usize, Look) {
    *CHOSEN
        .read()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// Whether `DOTORI_THEME` overrides what the owner chose.
pub(crate) fn overridden() -> bool {
    std::env::var_os(THEME_VARIABLE).is_some()
}

/// The theme and look in use, with `DOTORI_THEME` over the settings. The variable names a theme,
/// a look (`light` or `dark`), or both as `<theme>-<look>`.
pub(crate) fn chosen() -> (usize, Look) {
    let (mut index, mut look) = saved();
    if let Ok(asked) = std::env::var(THEME_VARIABLE) {
        for part in asked.split('-') {
            if let Some(at) = THEMES.iter().position(|(known, _, _)| *known == part) {
                index = at;
            } else if part == "light" || part == "dark" {
                look = Look::named(part);
            }
        }
    }
    (index, look)
}

fn dark(window: &Window, look: Look) -> bool {
    match look {
        Look::Light => false,
        Look::Dark => true,
        Look::System => matches!(
            window.appearance(),
            WindowAppearance::Dark | WindowAppearance::VibrantDark
        ),
    }
}

impl Theme {
    /// The colors to draw `window` with now: the chosen theme in the chosen look.
    pub(crate) fn of(window: &Window) -> Self {
        let (index, look) = chosen();
        let (_, light, dark_colors) = THEMES[index];
        if dark(window, look) {
            dark_colors
        } else {
            light
        }
    }
}

impl Theme {
    /// Whether these are the colors of a dark look, judged by the canvas.
    pub(crate) fn is_dark(&self) -> bool {
        0.2126 * self.canvas.r + 0.7152 * self.canvas.g + 0.0722 * self.canvas.b < 0.5
    }
}

#[cfg(test)]
#[path = "theme.test.rs"]
mod tests;
