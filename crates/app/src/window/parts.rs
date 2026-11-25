use std::borrow::Cow;

use gpui::AssetSource;
use gpui::Div;
use gpui::Img;
use gpui::Rgba;
use gpui::SharedString;
use gpui::div;
use gpui::img;
use gpui::prelude::*;
use gpui::px;

use crate::window::identity;
use crate::window::theme::Theme;
use crate::window::themes::LOGO;

/// The assistant's picture: the one the owner gave it, cut round, or else dotori's acorn in the
/// logo's colors for `theme`.
pub(crate) fn logo(theme: &Theme, size: f32) -> Img {
    match identity::icon() {
        Some(icon) => img(icon).flex_none().size(px(size)).rounded_full(),
        None => img(if theme.is_dark() {
            DARK_LOGO
        } else {
            LIGHT_LOGO
        })
        .flex_none()
        .size(px(size)),
    }
}

/// The windows' own images: the logo in a light and a dark look. GPUI loads an image named this
/// way with its colors in the order it draws, which an image made from bytes does not get.
pub(crate) struct Assets;

impl AssetSource for Assets {
    fn load(&self, path: &str) -> gpui::Result<Option<Cow<'static, [u8]>>> {
        if path == SETTINGS_ICON {
            return Ok(Some(Cow::Borrowed(GEAR.as_bytes())));
        }
        if let Some((_, svg)) = ICONS.iter().find(|(name, _)| *name == path) {
            return Ok(Some(Cow::Borrowed(svg.as_bytes())));
        }
        let colors = match path {
            LIGHT_LOGO => LOGO.0,
            DARK_LOGO => LOGO.1,
            _ => return Ok(None),
        };
        let svg = ACORN
            .replace("{cap}", &css(colors.cap))
            .replace("{nut}", &css(colors.nut));
        Ok(Some(Cow::Owned(svg.into_bytes())))
    }

    fn list(&self, _path: &str) -> gpui::Result<Vec<SharedString>> {
        Ok(vec![
            SharedString::new_static(LIGHT_LOGO),
            SharedString::new_static(DARK_LOGO),
            SharedString::new_static(SETTINGS_ICON),
            SharedString::new_static(CLOSE_ICON),
            SharedString::new_static(MINIMIZE_ICON),
            SharedString::new_static(MAXIMIZE_ICON),
            SharedString::new_static(SHIELD_ICON),
            SharedString::new_static(SHIELD_ON_ICON),
        ])
    }
}

const LIGHT_LOGO: &str = "logo/light.svg";
/// The gear the chat window opens the settings with, drawn in the color of its text.
pub(crate) const SETTINGS_ICON: &str = "icons/settings.svg";
/// The cross that closes a window that draws its own top, in the color of its text.
pub(crate) const CLOSE_ICON: &str = "icons/close.svg";
/// The bar that minimizes such a window.
pub(crate) const MINIMIZE_ICON: &str = "icons/minimize.svg";
/// The square that maximizes such a window, or puts it back.
pub(crate) const MAXIMIZE_ICON: &str = "icons/maximize.svg";
/// A shield, for what keeps the commands dotori's model runs away from its own files.
pub(crate) const SHIELD_ICON: &str = "icons/shield.svg";
/// The shield with a check, for when the sandbox is on.
pub(crate) const SHIELD_ON_ICON: &str = "icons/shield-on.svg";
const ICONS: [(&str, &str); 5] = [
    (
        SHIELD_ICON,
        r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="black" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round"><path d="M12 3l7 3v5c0 4.5-3 8.5-7 10-4-1.5-7-5.5-7-10V6l7-3z"/></svg>"#,
    ),
    (
        SHIELD_ON_ICON,
        r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="black" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round"><path d="M12 3l7 3v5c0 4.5-3 8.5-7 10-4-1.5-7-5.5-7-10V6l7-3z"/><path d="M9 12l2 2 4-4"/></svg>"#,
    ),
    (
        CLOSE_ICON,
        r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="black" stroke-width="2" stroke-linecap="round"><path d="M6 6l12 12M18 6L6 18"/></svg>"#,
    ),
    (
        MINIMIZE_ICON,
        r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="black" stroke-width="2" stroke-linecap="round"><path d="M6 12h12"/></svg>"#,
    ),
    (
        MAXIMIZE_ICON,
        r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="black" stroke-width="2" stroke-linejoin="round"><rect x="6.5" y="6.5" width="11" height="11" rx="1.5"/></svg>"#,
    ),
];
const GEAR: &str = r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="black" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round"><circle cx="12" cy="12" r="3"/><path d="M19.4 15a1.65 1.65 0 0 0 .33 1.82l.06.06a2 2 0 1 1-2.83 2.83l-.06-.06a1.65 1.65 0 0 0-1.82-.33 1.65 1.65 0 0 0-1 1.51V21a2 2 0 1 1-4 0v-.09a1.65 1.65 0 0 0-1-1.51 1.65 1.65 0 0 0-1.82.33l-.06.06a2 2 0 1 1-2.83-2.83l.06-.06a1.65 1.65 0 0 0 .33-1.82 1.65 1.65 0 0 0-1.51-1H3a2 2 0 1 1 0-4h.09a1.65 1.65 0 0 0 1.51-1 1.65 1.65 0 0 0-.33-1.82l-.06-.06a2 2 0 1 1 2.83-2.83l.06.06a1.65 1.65 0 0 0 1.82.33h.08a1.65 1.65 0 0 0 1-1.51V3a2 2 0 1 1 4 0v.09a1.65 1.65 0 0 0 1 1.51 1.65 1.65 0 0 0 1.82-.33l.06-.06a2 2 0 1 1 2.83 2.83l-.06.06a1.65 1.65 0 0 0-.33 1.82v.08a1.65 1.65 0 0 0 1.51 1H21a2 2 0 1 1 0 4h-.09a1.65 1.65 0 0 0-1.51 1z"/></svg>"#;
const DARK_LOGO: &str = "logo/dark.svg";

fn css(color: Rgba) -> String {
    let channel = |value: f32| (value.clamp(0.0, 1.0) * 255.0).round() as u8;
    format!(
        "#{:02X}{:02X}{:02X}",
        channel(color.r),
        channel(color.g),
        channel(color.b)
    )
}

const ACORN: &str = r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="40 34 176 196" width="176" height="196"><g transform="translate(128 128) scale(0.72) translate(-128 -128) rotate(15 128 140) translate(0 -4)"><path fill="{nut}" d="M 64 124 Q 128 152 192 124 C 202 154 196 190 162 214 C 146 226 134 231 128 236 C 122 231 110 226 94 214 C 60 190 54 154 64 124 Z"/><path fill="{cap}" d="M 56 112 Q 46 112 46 102 C 46 70 82 52 128 52 C 174 52 210 70 210 102 Q 210 112 200 112 Q 128 140 56 112 Z"/><path d="M128 54 Q128 36 112 26" fill="none" stroke="{cap}" stroke-width="16" stroke-linecap="round"/></g></svg>"#;

/// Text that wraps only between words. GPUI breaks Korean between any two syllables, so each
/// word is laid out on its own and lines break at the spaces. Runs of spaces and indentation keep
/// their width. A word wider than the line still wraps inside itself.
pub(crate) fn prose(text: &str) -> Div {
    div()
        .flex()
        .flex_col()
        .children(text.split('\n').map(|paragraph| {
            let paragraph = paragraph.replace('\t', "    ");
            if paragraph.trim().is_empty() {
                return div().h(px(10.));
            }
            div()
                .flex()
                .flex_wrap()
                .gap_x(px(SPACE))
                .children(paragraph.trim_end().split(' ').map(|word| {
                    if word.is_empty() {
                        div().w(px(0.))
                    } else {
                        div().min_w_0().child(SharedString::from(word.to_owned()))
                    }
                }))
        }))
}

/// The gap drawn for a space between words.
pub(crate) const SPACE: f32 = 4.;

/// The fixed-width font for code and commands. GPUI looks fonts up by family name, so a generic
/// name such as `monospace` falls back to the proportional font.
#[cfg(target_os = "macos")]
pub(crate) const MONO: &str = "Menlo";
/// The fixed-width font for code and commands. GPUI looks fonts up by family name, so a generic
/// name such as `monospace` falls back to the proportional font.
#[cfg(target_os = "windows")]
pub(crate) const MONO: &str = "Consolas";
/// The fixed-width font for code and commands. GPUI looks fonts up by family name, so a generic
/// name such as `monospace` falls back to the proportional font.
#[cfg(not(any(target_os = "macos", target_os = "windows")))]
pub(crate) const MONO: &str = "DejaVu Sans Mono";
