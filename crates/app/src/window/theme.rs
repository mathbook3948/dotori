use gpui::Rgba;
use gpui::Window;
use gpui::WindowAppearance;
use gpui::rgb;
use gpui::rgba;

/// The colors every window draws with, one set for a light system look and one for a dark one.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Theme {
    pub(crate) canvas: Rgba,
    pub(crate) surface: Rgba,
    pub(crate) raised: Rgba,
    pub(crate) line: Rgba,
    pub(crate) text: Rgba,
    pub(crate) muted: Rgba,
    pub(crate) faint: Rgba,
    pub(crate) accent: Rgba,
    pub(crate) accent_hover: Rgba,
    pub(crate) accent_text: Rgba,
    pub(crate) selection: Rgba,
    pub(crate) other: Rgba,
    pub(crate) other_text: Rgba,
    pub(crate) ok: Rgba,
    pub(crate) ok_soft: Rgba,
    pub(crate) warn: Rgba,
    pub(crate) warn_soft: Rgba,
    pub(crate) danger: Rgba,
    pub(crate) danger_soft: Rgba,
}

const THEME_VARIABLE: &str = "DOTORI_THEME";

fn light() -> Theme {
    Theme {
        canvas: rgb(0xf7f3ee),
        surface: rgb(0xffffff),
        raised: rgb(0xfffdfa),
        line: rgb(0xe7dfd5),
        text: rgb(0x2a231d),
        muted: rgb(0x7c7067),
        faint: rgb(0xb3a79c),
        accent: rgb(0x8b5a2b),
        accent_hover: rgb(0x774b22),
        accent_text: rgb(0xffffff),
        selection: rgba(0x8b5a2b40),
        other: rgb(0xedf1f5),
        other_text: rgb(0x52606f),
        ok: rgb(0x3f9a5c),
        ok_soft: rgb(0xe3f2e7),
        warn: rgb(0xc9822b),
        warn_soft: rgb(0xf8ead6),
        danger: rgb(0xc0453a),
        danger_soft: rgb(0xf8deda),
    }
}

fn dark() -> Theme {
    Theme {
        canvas: rgb(0x1b1714),
        surface: rgb(0x26211d),
        raised: rgb(0x211c18),
        line: rgb(0x3a322c),
        text: rgb(0xf0e8e0),
        muted: rgb(0xa89c91),
        faint: rgb(0x6f655c),
        accent: rgb(0xc98a4b),
        accent_hover: rgb(0xd99a5b),
        accent_text: rgb(0x1b1714),
        selection: rgba(0xc98a4b55),
        other: rgb(0x232a33),
        other_text: rgb(0x9fb0c3),
        ok: rgb(0x5cbf7e),
        ok_soft: rgb(0x1f3326),
        warn: rgb(0xe0a458),
        warn_soft: rgb(0x3a2c1b),
        danger: rgb(0xe07064),
        danger_soft: rgb(0x3e2422),
    }
}

impl Theme {
    /// The theme that matches the system's light or dark look for `window`, unless
    /// `DOTORI_THEME` asks for `light` or `dark`.
    pub(crate) fn of(window: &Window) -> Self {
        match std::env::var(THEME_VARIABLE).as_deref() {
            Ok("dark") => return dark(),
            Ok("light") => return light(),
            _ => {}
        }
        match window.appearance() {
            WindowAppearance::Dark | WindowAppearance::VibrantDark => dark(),
            WindowAppearance::Light | WindowAppearance::VibrantLight => light(),
        }
    }
}
