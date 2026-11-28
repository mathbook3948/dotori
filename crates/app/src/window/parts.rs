use std::borrow::Cow;
use std::io::Cursor;
use std::path::Path;
use std::sync::Arc;

use gpui::AnimationExt;
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
use crate::window::motion::WORKING;
use crate::window::motion::breath;
use crate::window::select::Joint;
use crate::window::select::Pen;
use crate::window::surface::SurfaceExt;
use crate::window::theme::Logo;
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

/// The assistant's picture on its tile: the owner's picture filling it, or the acorn on the
/// `tile` fill, with the soft shadow of a card. Every window shows the assistant this way.
pub(crate) fn picture_tile(theme: &Theme, size: f32, radius: f32) -> Div {
    let shown: gpui::AnyElement = match identity::icon() {
        Some(icon) => img(picture(&icon, size))
            .size_full()
            .rounded(px(radius))
            .into_any_element(),
        None => logo(theme, size * 0.62).into_any_element(),
    };
    div()
        .flex()
        .flex_none()
        .items_center()
        .justify_center()
        .size(px(size))
        .rounded(px(radius))
        .bg(theme.tile())
        .lifted(theme)
        .child(shown)
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
        Ok(Some(Cow::Owned(
            acorn_in(colors, side as f32 / 2.).into_bytes(),
        )))
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
            SharedString::new_static(SLIDERS_ICON),
            SharedString::new_static(CHIP_ICON),
            SharedString::new_static(BUBBLE_ICON),
            SharedString::new_static(NOTEBOOK_ICON),
            SharedString::new_static(SCREEN_ICON),
            SharedString::new_static(CHEVRON_ICON),
            SharedString::new_static(SEARCH_ICON),
            SharedString::new_static(BACK_ICON),
            SharedString::new_static(MAXIMIZE_ICON),
            SharedString::new_static(RESTORE_ICON),
            SharedString::new_static(THINKING_ICON),
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
/// Sliders, for the settings that shape the assistant and the windows.
pub(crate) const SLIDERS_ICON: &str = "icons/sliders.svg";
/// A chip, for the model the assistant thinks with.
pub(crate) const CHIP_ICON: &str = "icons/chip.svg";
/// A speech bubble, for the other places to talk.
pub(crate) const BUBBLE_ICON: &str = "icons/bubble.svg";
/// A notebook, for what the assistant keeps in mind.
pub(crate) const NOTEBOOK_ICON: &str = "icons/notebook.svg";
/// A screen, for the owner's PCs.
pub(crate) const SCREEN_ICON: &str = "icons/screen.svg";
/// A chevron pointing down, on a field that opens a list of choices.
pub(crate) const CHEVRON_ICON: &str = "icons/chevron.svg";
/// A magnifying glass, before the search field of a long list.
pub(crate) const SEARCH_ICON: &str = "icons/search.svg";
/// An arrow pointing left, for going back to the conversation from the settings.
pub(crate) const BACK_ICON: &str = "icons/back.svg";
/// A square, for the button that fills the screen with the window.
pub(crate) const MAXIMIZE_ICON: &str = "icons/maximize.svg";
/// Two squares, for the button that brings a window that fills the screen back to its size.
pub(crate) const RESTORE_ICON: &str = "icons/restore.svg";
/// Three dots in a row, the assistant thinking.
pub(crate) const THINKING_ICON: &str = "icons/thinking.svg";
/// A cross in a circle, for a call the owner declined.
pub(crate) const DECLINED_ICON: &str = "icons/declined.svg";
/// Two sheets, one over the other, for copying what a message says.
pub(crate) const COPY_ICON: &str = "icons/copy.svg";
/// A small screen with a prompt, for what a call the owner allowed gave.
pub(crate) const RESULT_ICON: &str = "icons/result.svg";
const ICONS: [(&str, &str); 26] = [
    (
        COPY_ICON,
        r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="black" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><rect x="8.5" y="8.5" width="11" height="11" rx="2"/><path d="M15.5 8.5V6.5a2 2 0 0 0-2-2h-7a2 2 0 0 0-2 2v7a2 2 0 0 0 2 2h2"/></svg>"##,
    ),
    (
        DECLINED_ICON,
        r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="black" stroke-width="2" stroke-linecap="round"><circle cx="12" cy="12" r="8.5"/><path d="M9 9l6 6M15 9l-6 6"/></svg>"##,
    ),
    (
        RESULT_ICON,
        r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="black" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><rect x="3.5" y="4.5" width="17" height="15" rx="2.5"/><path d="M7.5 10l3 2-3 2M13 15h3.5"/></svg>"##,
    ),
    (
        THINKING_ICON,
        r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 28 6" fill="black"><circle cx="3" cy="3" r="3"/><circle cx="14" cy="3" r="3"/><circle cx="25" cy="3" r="3"/></svg>"#,
    ),
    (
        MAXIMIZE_ICON,
        r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="black" stroke-width="2" stroke-linejoin="round"><rect x="6" y="6" width="12" height="12" rx="2"/></svg>"#,
    ),
    (
        RESTORE_ICON,
        r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="black" stroke-width="2" stroke-linejoin="round"><rect x="5" y="9" width="10" height="10" rx="2"/><path d="M9 9V7a2 2 0 0 1 2-2h6a2 2 0 0 1 2 2v6a2 2 0 0 1-2 2h-2"/></svg>"#,
    ),
    (
        BACK_ICON,
        r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="black" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M19 12H5M11 6l-6 6 6 6"/></svg>"#,
    ),
    (
        SEARCH_ICON,
        r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="black" stroke-width="2" stroke-linecap="round"><circle cx="11" cy="11" r="6.5"/><path d="M16 16l4.5 4.5"/></svg>"#,
    ),
    (
        CHEVRON_ICON,
        r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="black" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M6 9l6 6 6-6"/></svg>"#,
    ),
    (
        SLIDERS_ICON,
        r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="black" stroke-width="1.8" stroke-linecap="round"><path d="M4 7h9M17 7h3M4 17h3M11 17h9"/><circle cx="15" cy="7" r="2"/><circle cx="9" cy="17" r="2"/></svg>"#,
    ),
    (
        CHIP_ICON,
        r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="black" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round"><rect x="6" y="6" width="12" height="12" rx="2"/><path d="M10 10h4v4h-4z"/><path d="M9.5 3v3M14.5 3v3M9.5 18v3M14.5 18v3M3 9.5h3M3 14.5h3M18 9.5h3M18 14.5h3"/></svg>"#,
    ),
    (
        BUBBLE_ICON,
        r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="black" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round"><path d="M5 5h14a1.5 1.5 0 0 1 1.5 1.5v9a1.5 1.5 0 0 1-1.5 1.5h-7l-4.5 3.5V17H5a1.5 1.5 0 0 1-1.5-1.5v-9A1.5 1.5 0 0 1 5 5z"/><path d="M8 10h8M8 13h5"/></svg>"#,
    ),
    (
        NOTEBOOK_ICON,
        r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="black" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round"><path d="M6.5 3.5h11a1.5 1.5 0 0 1 1.5 1.5v14a1.5 1.5 0 0 1-1.5 1.5h-11A1.5 1.5 0 0 1 5 19V5a1.5 1.5 0 0 1 1.5-1.5z"/><path d="M9 3.5v17M12 8h4M12 11.5h4"/></svg>"#,
    ),
    (
        SCREEN_ICON,
        r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="black" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round"><rect x="3.5" y="4.5" width="17" height="11.5" rx="1.5"/><path d="M9 20h6M12 16v4"/></svg>"#,
    ),
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

/// The acorn in its light colors, `height` pixels tall, as an SVG.
pub(crate) fn acorn_svg(height: f32) -> String {
    acorn_in(LOGO.0, height)
}

fn acorn_in(colors: Logo, height: f32) -> String {
    ACORN
        .replace(
            "{width}",
            &format!("{}", height * ACORN_WIDTH / ACORN_HEIGHT),
        )
        .replace("{height}", &format!("{height}"))
        .replace("{cap}", &css(colors.cap))
        .replace("{nut}", &css(colors.nut))
}

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
const ACORN: &str = include_str!("acorn.svg");

/// Text that wraps only between words. GPUI breaks Korean between any two syllables, so each
/// word is laid out on its own and lines break at the spaces. Runs of spaces and indentation keep
/// their width. A word wider than the line still wraps inside itself.
/// A `caption` line that leads somewhere else in the settings: the words before the link, the
/// link in `text` underlined, and the words after. `id` names the link for tests.
pub(crate) fn link_line(
    theme: &Theme,
    id: &'static str,
    words: [&'static str; 3],
    on_click: impl Fn(&gpui::ClickEvent, &mut gpui::Window, &mut gpui::App) + 'static,
) -> Div {
    let [before, link, after] = words;
    div()
        .flex()
        .flex_wrap()
        .gap_1()
        .text_xs()
        .text_color(theme.muted)
        .when(!before.is_empty(), |line| line.child(before))
        .child(
            div()
                .flex()
                .child(
                    div()
                        .id(id)
                        .debug_selector(move || id.to_owned())
                        .text_color(theme.text)
                        .font_weight(gpui::FontWeight::SEMIBOLD)
                        .underline()
                        .cursor_pointer()
                        .child(link)
                        .on_click(on_click),
                )
                .child(after),
        )
}

/// The assistant thinking before its first word: three `muted` dots breathing with
/// `motion.working`. It goes in the assistant's bubble wherever that bubble stands.
pub(crate) fn thinking(
    theme: &Theme,
    id: impl Into<gpui::ElementId>,
) -> gpui::AnimationElement<Div> {
    div()
        .flex()
        .items_center()
        .h(px(21.))
        .child(
            svg()
                .path(THINKING_ICON)
                .w(px(28.))
                .h(px(6.))
                .text_color(theme.muted),
        )
        .with_animation(
            id,
            WORKING.animation().repeat().with_easing(|t| t),
            |dots, progress| dots.opacity(breath(progress)),
        )
}

pub(crate) fn prose(text: &str) -> Div {
    prose_in(text, None)
}

/// [`prose`] whose words `pen` draws, so the owner can select them, when there is one.
pub(crate) fn prose_in(text: &str, pen: Option<&Pen>) -> Div {
    div()
        .flex()
        .flex_col()
        .children(text.split('\n').map(|paragraph| {
            let paragraph = paragraph.replace('\t', "    ");
            if paragraph.trim().is_empty() {
                return div().h(px(10.));
            }
            let words: Vec<&str> = paragraph
                .trim_end()
                .split(' ')
                .filter(|word| !word.is_empty())
                .collect();
            let last = words.len().saturating_sub(1);
            div()
                .flex()
                .flex_wrap()
                .when(pen.is_none(), |row| row.gap_x(px(SPACE)))
                .children(words.into_iter().enumerate().map(|(n, word)| match pen {
                    Some(pen) => {
                        let joint = if n == 0 { Joint::Newline } else { Joint::Space };
                        pen.word(word.to_owned(), joint, n < last)
                    }
                    None => div().min_w_0().child(SharedString::from(word.to_owned())),
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
