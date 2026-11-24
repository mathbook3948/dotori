use gpui::Div;
use gpui::FontWeight;
use gpui::Rgba;
use gpui::SharedString;
use gpui::div;
use gpui::prelude::*;
use gpui::px;

use crate::window::theme::Theme;

/// The round acorn-brown badge with a `d`, dotori's mark in every window.
pub(crate) fn logo(theme: &Theme, size: f32) -> Div {
    div()
        .flex()
        .flex_none()
        .items_center()
        .justify_center()
        .size(px(size))
        .rounded_full()
        .bg(theme.accent)
        .text_color(theme.accent_text)
        .text_size(px(size * 0.5))
        .font_weight(FontWeight::BOLD)
        .child("d")
}

/// The bar at the top of a window: the mark, a title, and a line under it.
pub(crate) fn header(theme: &Theme, title: &'static str, under: impl IntoElement) -> Div {
    div()
        .flex()
        .flex_none()
        .items_center()
        .gap_3()
        .px_4()
        .h(px(60.))
        .bg(theme.raised)
        .border_b_1()
        .border_color(theme.line)
        .child(logo(theme, 34.))
        .child(
            div()
                .flex()
                .flex_1()
                .flex_col()
                .gap(px(2.))
                .child(
                    div()
                        .text_base()
                        .font_weight(FontWeight::SEMIBOLD)
                        .child(title),
                )
                .child(under),
        )
}

/// A small dot and a few words, such as whether the window reaches dotori.
pub(crate) fn status(color: Rgba, muted: Rgba, words: impl IntoElement) -> Div {
    div()
        .flex()
        .items_center()
        .gap_1p5()
        .text_xs()
        .text_color(muted)
        .child(div().flex_none().size(px(7.)).rounded_full().bg(color))
        .child(words)
}

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
