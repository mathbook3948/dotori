//! The picture the OS shows for dotori: the assistant's picture the owner chose, cut to the
//! rounded square the windows draw it in, or else the acorn. The tray shows it, and on Windows
//! each window's taskbar button too.

use image::RgbaImage;
use image::imageops::FilterType;

use crate::window::identity;
use crate::window::parts::acorn_svg;
use crate::window::tokens::AVATAR;

/// The picture as rows of RGBA, `side` pixels square.
pub(crate) fn rgba(side: u32) -> Vec<u8> {
    identity::icon()
        .and_then(|path| picture(&path, side))
        .unwrap_or_else(|| acorn(side))
        .into_raw()
}

fn picture(path: &std::path::Path, side: u32) -> Option<RgbaImage> {
    let image = image::open(path).ok()?.to_rgba8();
    let edge = image.width().min(image.height());
    let square = image::imageops::crop_imm(
        &image,
        (image.width() - edge) / 2,
        (image.height() - edge) / 2,
        edge,
        edge,
    )
    .to_image();
    let mut tile = image::imageops::resize(&square, side, side, FilterType::Lanczos3);
    let half = side as f32 / 2.;
    let radius = side as f32 * CORNER;
    let straight = half - radius;
    for (x, y, pixel) in tile.enumerate_pixels_mut() {
        let dx = ((x as f32 + 0.5 - half).abs() - straight).max(0.);
        let dy = ((y as f32 + 0.5 - half).abs() - straight).max(0.);
        let cover = (radius - (dx * dx + dy * dy).sqrt() + 0.5).clamp(0., 1.);
        pixel.0[3] = (f32::from(pixel.0[3]) * cover) as u8;
    }
    Some(tile)
}

const CORNER: f32 = 17. / AVATAR;

fn acorn(side: u32) -> RgbaImage {
    let empty = || RgbaImage::new(side, side);
    let Ok(tree) = resvg::usvg::Tree::from_str(&acorn_svg(side as f32), &Default::default()) else {
        return empty();
    };
    let Some(mut pixmap) = resvg::tiny_skia::Pixmap::new(side, side) else {
        return empty();
    };
    let size = tree.size();
    let dx = (side as f32 - size.width()) / 2.;
    let dy = (side as f32 - size.height()) / 2.;
    resvg::render(
        &tree,
        resvg::tiny_skia::Transform::from_translate(dx, dy),
        &mut pixmap.as_mut(),
    );
    let mut image = empty();
    for (pixel, drawn) in image.pixels_mut().zip(pixmap.pixels()) {
        let color = drawn.demultiply();
        pixel.0 = [color.red(), color.green(), color.blue(), color.alpha()];
    }
    image
}

/// Shows the picture on the window's taskbar button and in its corner, on Windows. Elsewhere the
/// desktop takes the app's own icon, and asking GPUI for an X11 window's handle panics.
pub(crate) fn show_on(window: &gpui::Window) {
    if !cfg!(target_os = "windows") {
        return;
    }
    use raw_window_handle::HasWindowHandle;
    use raw_window_handle::RawWindowHandle;
    let Ok(handle) = HasWindowHandle::window_handle(window) else {
        return;
    };
    let RawWindowHandle::Win32(handle) = handle.as_raw() else {
        return;
    };
    let (Some(small), Some(big)) = (png(32), png(64)) else {
        return;
    };
    if let Err(error) = dotori_native::set_window_icon(handle.hwnd.get(), (&small, 32), (&big, 64))
    {
        eprintln!("dotori: the window keeps its old icon: {error}");
    }
}

fn png(side: u32) -> Option<Vec<u8>> {
    let image = RgbaImage::from_raw(side, side, rgba(side))?;
    let mut png = Vec::new();
    image
        .write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)
        .ok()?;
    Some(png)
}

#[cfg(test)]
#[path = "app_icon.test.rs"]
mod tests;
