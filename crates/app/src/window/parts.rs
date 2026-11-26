use std::borrow::Cow;
use std::io::Cursor;
use std::path::Path;
use std::sync::Arc;

use gpui::AssetSource;
use gpui::Div;
use gpui::Font;
use gpui::FontFeatures;
use gpui::FontStyle;
use gpui::FontWeight;
use gpui::Img;
use gpui::Rgba;
use gpui::SharedString;
use gpui::Svg;
use gpui::div;
use gpui::img;
use gpui::prelude::*;
use gpui::px;
use gpui::svg;
use image::ImageFormat;
use image::imageops::FilterType;

use crate::window::identity;
use crate::window::theme::Theme;
use crate::window::themes::LOGO;

/// The assistant's picture: the one the owner gave it, cut round, or else dotori's acorn in the
/// logo's colors for `theme`.
pub(crate) fn logo(theme: &Theme, size: f32) -> Img {
    match identity::icon() {
        Some(icon) => img(picture(&icon, size))
            .flex_none()
            .size(px(size))
            .rounded_full(),
        None => img(format!(
            "{}/{}.svg",
            if theme.is_dark() {
                DARK_LOGO
            } else {
                LIGHT_LOGO
            },
            side(size)
        ))
        .flex_none()
        .size(px(size)),
    }
}

/// The bolt that marks an account dotori logs in with without asking, in `primary`.
pub(crate) fn bolt(theme: &Theme, size: f32) -> Svg {
    svg()
        .path(BOLT_ICON)
        .flex_none()
        .size(px(size))
        .text_color(theme.primary)
}

/// The lock that marks what the vault keeps, drawn `size` pixels wide in `muted`.
pub(crate) fn lock(theme: &Theme, size: f32) -> Svg {
    svg()
        .path(LOCK_ICON)
        .flex_none()
        .size(px(size))
        .text_color(theme.muted)
}

/// The owner's picture at `path` as an image named for the size it is drawn at, `size` points a
/// side. GPUI shrinks an image to its box in one step and leaves jagged edges when it shrinks by
/// much, so the windows ask for each picture at twice the size it is drawn at.
pub(crate) fn picture(path: &Path, size: f32) -> SharedString {
    let name = path
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default();
    format!("{PICTURE}/{}/{name}", side(size)).into()
}

fn side(size: f32) -> u32 {
    (size * 2.).ceil().max(1.) as u32
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
        if let Some((side, name)) = sized(path, PICTURE) {
            return Ok(shrunk(side, name).map(Cow::Owned));
        }
        let (colors, side) = match (sized(path, LIGHT_LOGO), sized(path, DARK_LOGO)) {
            (Some((side, "")), _) => (LOGO.0, side),
            (_, Some((side, ""))) => (LOGO.1, side),
            _ => return Ok(None),
        };
        let height = side as f32 / 2.;
        let svg = ACORN
            .replace(
                "{width}",
                &format!("{}", height * ACORN_WIDTH / ACORN_HEIGHT),
            )
            .replace("{height}", &format!("{height}"))
            .replace("{cap}", &css(colors.cap))
            .replace("{nut}", &css(colors.nut));
        Ok(Some(Cow::Owned(svg.into_bytes())))
    }

    fn list(&self, _path: &str) -> gpui::Result<Vec<SharedString>> {
        Ok(vec![
            SharedString::new_static(SETTINGS_ICON),
            SharedString::new_static(CLOSE_ICON),
            SharedString::new_static(MINIMIZE_ICON),
            SharedString::new_static(SHIELD_ICON),
            SharedString::new_static(SHIELD_ON_ICON),
            SharedString::new_static(LOCK_ICON),
            SharedString::new_static(BOLT_ICON),
            SharedString::new_static(TRASH_ICON),
            SharedString::new_static(PENCIL_ICON),
            SharedString::new_static(CHECK_ICON),
            SharedString::new_static(CLOCK_ICON),
            SharedString::new_static(GLOBE_ICON),
            SharedString::new_static(FOLDER_ICON),
        ])
    }
}

const LIGHT_LOGO: &str = "logo/light";
const PICTURE: &str = "picture";

fn sized<'a>(path: &'a str, folder: &str) -> Option<(u32, &'a str)> {
    let rest = path.strip_prefix(folder)?.strip_prefix('/')?;
    match rest.split_once('/') {
        Some((side, name)) => Some((side.parse().ok()?, name)),
        None => Some((rest.strip_suffix(".svg")?.parse().ok()?, "")),
    }
}

fn shrunk(side: u32, name: &str) -> Option<Vec<u8>> {
    let path = identity::icon()?;
    if path.file_name()?.to_str()? != name {
        return None;
    }
    let bytes = std::fs::read(&path).ok()?;
    let image = image::load_from_memory(&bytes).ok()?;
    if image.width() <= side {
        return Some(bytes);
    }
    let mut png = Vec::new();
    image
        .resize_exact(side, side, FilterType::Lanczos3)
        .write_to(&mut Cursor::new(&mut png), ImageFormat::Png)
        .ok()?;
    Some(png)
}
/// The gear the chat window opens the settings with, drawn in the color of its text.
pub(crate) const SETTINGS_ICON: &str = "icons/settings.svg";
/// The cross that closes a window that draws its own top, in the color of its text.
pub(crate) const CLOSE_ICON: &str = "icons/close.svg";
/// The bar that minimizes such a window.
pub(crate) const MINIMIZE_ICON: &str = "icons/minimize.svg";
/// A shield, for what keeps the commands dotori's model runs away from its own files.
pub(crate) const SHIELD_ICON: &str = "icons/shield.svg";
/// The shield with a check, for when the sandbox is on.
pub(crate) const SHIELD_ON_ICON: &str = "icons/shield-on.svg";
/// The lock that marks what the vault keeps, in the color of its text.
pub(crate) const LOCK_ICON: &str = "icons/lock.svg";
/// The bolt that marks an account dotori logs in with without asking.
pub(crate) const BOLT_ICON: &str = "icons/bolt.svg";
/// The bin that removes a key from the vault.
pub(crate) const TRASH_ICON: &str = "icons/trash.svg";
/// The pencil over the assistant's picture that offers to change it.
pub(crate) const PENCIL_ICON: &str = "icons/pencil.svg";
/// The check by the chosen item of a list.
pub(crate) const CHECK_ICON: &str = "icons/check.svg";
/// What dotori can do at a set time.
pub(crate) const CLOCK_ICON: &str = "icons/clock.svg";
/// What dotori can do on the web.
pub(crate) const GLOBE_ICON: &str = "icons/globe.svg";
/// What dotori can do with the files on this PC.
pub(crate) const FOLDER_ICON: &str = "icons/folder.svg";
const ICONS: [(&str, &str); 12] = [
    (
        TRASH_ICON,
        r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="black" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round"><path d="M4 7h16"/><path d="M9 7V4h6v3"/><path d="M6 7l1 13h10l1-13"/><path d="M10 11v6M14 11v6"/></svg>"#,
    ),
    (
        BOLT_ICON,
        r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="black" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M13 2L4 14h7l-1 8 9-12h-7l1-8z"/></svg>"#,
    ),
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
        LOCK_ICON,
        r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="black" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><rect x="4" y="11" width="16" height="10" rx="2"/><path d="M8 11V7a4 4 0 0 1 8 0v4"/></svg>"#,
    ),
    (
        PENCIL_ICON,
        r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="black" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M15.5 4.5l4 4L8.5 19.5 4 20l.5-4.5 11-11z"/><path d="M13.5 6.5l4 4"/></svg>"#,
    ),
    (
        CHECK_ICON,
        r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 16 16" fill="none" stroke="black" stroke-width="2.2" stroke-linecap="round" stroke-linejoin="round"><path d="M3.5 8.5l3 3 6-7"/></svg>"#,
    ),
    (
        CLOCK_ICON,
        r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="black" stroke-width="2.2" stroke-linecap="round" stroke-linejoin="round"><circle cx="12" cy="13" r="7.5"/><path d="M12 9.5V13l2.5 2M5 5.5l2.5-2M19 5.5l-2.5-2"/></svg>"#,
    ),
    (
        GLOBE_ICON,
        r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="black" stroke-width="2"><circle cx="12" cy="12" r="8.5"/><path d="M3.5 12h17M12 3.5c2.6 2.4 3.8 5.2 3.8 8.5s-1.2 6.1-3.8 8.5c-2.6-2.4-3.8-5.2-3.8-8.5s1.2-6.1 3.8-8.5z"/></svg>"#,
    ),
    (
        FOLDER_ICON,
        r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="black" stroke-width="2.2" stroke-linejoin="round"><path d="M3.5 7.5a2 2 0 0 1 2-2h4l2 2.5h7a2 2 0 0 1 2 2v7.5a2 2 0 0 1-2 2h-13a2 2 0 0 1-2-2z"/></svg>"#,
    ),
];
const GEAR: &str = r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="black" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round"><circle cx="12" cy="12" r="3"/><path d="M19.4 15a1.65 1.65 0 0 0 .33 1.82l.06.06a2 2 0 1 1-2.83 2.83l-.06-.06a1.65 1.65 0 0 0-1.82-.33 1.65 1.65 0 0 0-1 1.51V21a2 2 0 1 1-4 0v-.09a1.65 1.65 0 0 0-1-1.51 1.65 1.65 0 0 0-1.82.33l-.06.06a2 2 0 1 1-2.83-2.83l.06-.06a1.65 1.65 0 0 0 .33-1.82 1.65 1.65 0 0 0-1.51-1H3a2 2 0 1 1 0-4h.09a1.65 1.65 0 0 0 1.51-1 1.65 1.65 0 0 0-.33-1.82l-.06-.06a2 2 0 1 1 2.83-2.83l.06.06a1.65 1.65 0 0 0 1.82.33h.08a1.65 1.65 0 0 0 1-1.51V3a2 2 0 1 1 4 0v.09a1.65 1.65 0 0 0 1 1.51 1.65 1.65 0 0 0 1.82-.33l.06-.06a2 2 0 1 1 2.83 2.83l-.06.06a1.65 1.65 0 0 0-.33 1.82v.08a1.65 1.65 0 0 0 1.51 1H21a2 2 0 1 1 0 4h-.09a1.65 1.65 0 0 0-1.51 1z"/></svg>"#;
const DARK_LOGO: &str = "logo/dark";

fn css(color: Rgba) -> String {
    let channel = |value: f32| (value.clamp(0.0, 1.0) * 255.0).round() as u8;
    format!(
        "#{:02X}{:02X}{:02X}",
        channel(color.r),
        channel(color.g),
        channel(color.b)
    )
}

const ACORN_WIDTH: f32 = 176.;
const ACORN_HEIGHT: f32 = 196.;
const ACORN: &str = r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="40 34 176 196" width="{width}" height="{height}"><g transform="translate(128 128) scale(0.72) translate(-128 -128) rotate(15 128 140) translate(0 -4)"><path fill="{nut}" d="M 64 124 Q 128 152 192 124 C 202 154 196 190 162 214 C 146 226 134 231 128 236 C 122 231 110 226 94 214 C 60 190 54 154 64 124 Z"/><path fill="{cap}" d="M 56 112 Q 46 112 46 102 C 46 70 82 52 128 52 C 174 52 210 70 210 102 Q 210 112 200 112 Q 128 140 56 112 Z"/><path d="M128 54 Q128 36 112 26" fill="none" stroke="{cap}" stroke-width="16" stroke-linecap="round"/></g></svg>"#;

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

/// The font the windows write in. It has a regular and a bold weight, so semibold text draws
/// bold and medium text draws regular.
pub(crate) const FONT: &str = "Kakao Small Sans";

/// The files of [`FONT`], which the windows load before they open.
pub(crate) const FONT_FILES: [&[u8]; 2] = [
    include_bytes!("../../fonts/KakaoSmallSans-Regular.ttf"),
    include_bytes!("../../fonts/KakaoSmallSans-Bold.ttf"),
];

/// [`FONT`] with its ligatures off, so `->` and `:-D` stay as typed.
pub(crate) fn typeface() -> Font {
    Font {
        family: FONT.into(),
        features: FontFeatures(Arc::new(vec![
            (String::from("liga"), 0),
            (String::from("calt"), 0),
        ])),
        fallbacks: None,
        weight: FontWeight::NORMAL,
        style: FontStyle::Normal,
    }
}

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

#[cfg(test)]
#[path = "parts.test.rs"]
mod tests;
