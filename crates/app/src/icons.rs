//! Keeps the picture the owner gave the assistant, cut to a square and made small, and carries it
//! to the other linked PCs so every PC shows the assistant the same way.
//!
//! A picture is kept as a PNG named by random letters, and the settings name the one in use. The
//! settings travel with the other shared settings, and the picture follows on its own message.

use std::io::Cursor;
use std::path::Path;
use std::path::PathBuf;
use std::time::Duration;

use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use dotori_connectors::peer::hub::Heard;
use image::DynamicImage;
use image::ImageDecoder;
use image::ImageFormat;
use image::ImageReader;
use image::Limits;
use image::imageops::FilterType;
use serde_json::Value;
use serde_json::json;
use tokio::time;

use crate::main_session::Linked;
use crate::settings::Settings;

/// The side of a kept picture, in pixels.
pub(crate) const SIDE: u32 = 256;
const GIVEN_LONGEST: usize = 20 * 1024 * 1024;
const KEPT_LONGEST: usize = 1024 * 1024;
const NAME_LETTERS: usize = 16;
const GIVEN_SIDE_LONGEST: u32 = 12_000;
const KEPT_PICTURES: usize = 4;
const LOOK_EVERY: Duration = Duration::from_secs(5);

/// Cuts `picture`, in any common image format, to its middle square, makes it `SIDE` pixels on a
/// side, and keeps it as a PNG in `folder`. Gives the name it is kept under.
pub(crate) fn keep(folder: &Path, picture: &[u8]) -> Result<String, String> {
    if picture.len() > GIVEN_LONGEST {
        return Err(String::from("the picture is larger than 20 MB"));
    }
    let image = decode(picture, None, GIVEN_SIDE_LONGEST)
        .map_err(|error| format!("the picture does not read as an image: {error}"))?;
    let side = image.width().min(image.height());
    if side == 0 {
        return Err(String::from("the picture is empty"));
    }
    let square = image.crop_imm(
        (image.width() - side) / 2,
        (image.height() - side) / 2,
        side,
        side,
    );
    let small = DynamicImage::ImageRgba8(
        square
            .resize_exact(SIDE, SIDE, FilterType::Lanczos3)
            .to_rgba8(),
    );
    let mut png = Vec::new();
    small
        .write_to(&mut Cursor::new(&mut png), ImageFormat::Png)
        .map_err(|error| format!("the picture could not be made a PNG: {error}"))?;
    let mut letters = [0_u8; NAME_LETTERS / 2];
    getrandom::fill(&mut letters).map_err(|error| format!("no random source: {error}"))?;
    let name = format!(
        "{}.png",
        letters
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>()
    );
    write(folder, &name, &png)?;
    Ok(name)
}

/// The largest file a picture may be before it is read, in bytes.
pub(crate) fn given_longest() -> u64 {
    u64::try_from(GIVEN_LONGEST).unwrap_or(u64::MAX)
}

fn decode(
    bytes: &[u8],
    format: Option<ImageFormat>,
    side_longest: u32,
) -> image::ImageResult<DynamicImage> {
    let mut reader = ImageReader::new(Cursor::new(bytes));
    match format {
        Some(format) => reader.set_format(format),
        None => reader = reader.with_guessed_format()?,
    }
    let mut limits = Limits::default();
    limits.max_image_width = Some(side_longest);
    limits.max_image_height = Some(side_longest);
    reader.limits(limits);
    let mut decoder = reader.into_decoder()?;
    let orientation = decoder.orientation()?;
    let mut image = DynamicImage::from_decoder(decoder)?;
    image.apply_orientation(orientation);
    Ok(image)
}

/// Removes the kept pictures other than `keep` and the latest few.
pub(crate) fn prune(folder: &Path, keep: &[&str]) {
    let Ok(entries) = std::fs::read_dir(folder) else {
        return;
    };
    let mut kept: Vec<(std::time::SystemTime, PathBuf, bool)> = entries
        .filter_map(Result::ok)
        .filter_map(|entry| {
            let name = entry.file_name().into_string().ok()?;
            checked(&name)?;
            let modified = entry.metadata().ok()?.modified().ok()?;
            Some((modified, entry.path(), keep.contains(&name.as_str())))
        })
        .collect();
    kept.sort_by_key(|(modified, _, _)| std::cmp::Reverse(*modified));
    for (n, (_, path, wanted)) in kept.into_iter().enumerate() {
        if !wanted && n >= KEPT_PICTURES {
            let _ = std::fs::remove_file(path);
        }
    }
}

/// Where the picture kept as `name` is, when the name is one `keep` makes and the file is there.
pub(crate) fn path(folder: &Path, name: &str) -> Option<PathBuf> {
    let path = folder.join(checked(name)?);
    path.is_file().then_some(path)
}

fn checked(name: &str) -> Option<&str> {
    let stem = name.strip_suffix(".png")?;
    (stem.len() == NAME_LETTERS && stem.bytes().all(|byte| byte.is_ascii_hexdigit()))
        .then_some(name)
}

fn write(folder: &Path, name: &str, png: &[u8]) -> Result<(), String> {
    std::fs::create_dir_all(folder).map_err(|error| format!("{}: {error}", folder.display()))?;
    let path = folder.join(name);
    let draft = folder.join(format!("{name}.new"));
    let fail = |error: std::io::Error| format!("{}: {error}", path.display());
    std::fs::write(&draft, png).map_err(fail)?;
    std::fs::rename(&draft, &path).map_err(fail)
}

fn in_use(settings: &Path) -> Option<String> {
    Settings::load(settings)
        .ok()?
        .assistant?
        .icon
        .filter(|name| checked(name).is_some())
}

fn message(folder: &Path, name: &str) -> Option<Value> {
    let png = std::fs::read(path(folder, name)?).ok()?;
    (png.len() <= KEPT_LONGEST)
        .then(|| json!({ "kind": "icon", "name": name, "png": STANDARD.encode(png) }))
}

/// Takes a picture another PC sent and keeps it under the name it came with, when it is a small
/// PNG. Says whether the message was about the picture.
pub(crate) fn hear(folder: &Path, settings: &Path, linked: &Linked, heard: &Heard) -> bool {
    match heard.message.get("kind").and_then(Value::as_str) {
        Some("linked") => {
            if let Some(message) = in_use(settings).and_then(|name| message(folder, &name)) {
                linked.hub.send(&heard.key, message);
            }
            false
        }
        Some("icon") => {
            let (folder, settings) = (folder.to_path_buf(), settings.to_path_buf());
            let (message, device) = (heard.message.clone(), heard.device.clone());
            tokio::task::spawn_blocking(move || match take(&folder, &message) {
                Ok(name) => {
                    let in_use = in_use(&settings);
                    let keep: Vec<&str> = [Some(name.as_str()), in_use.as_deref()]
                        .into_iter()
                        .flatten()
                        .collect();
                    prune(&folder, &keep);
                }
                Err(error) => {
                    eprintln!("dotori: the picture from {device} was not kept: {error}");
                }
            });
            true
        }
        _ => false,
    }
}

fn take(folder: &Path, message: &Value) -> Result<String, String> {
    let name = message
        .get("name")
        .and_then(Value::as_str)
        .and_then(checked)
        .ok_or("it has no name a picture is kept under")?;
    if path(folder, name).is_some() {
        return Ok(name.to_owned());
    }
    let encoded = message
        .get("png")
        .and_then(Value::as_str)
        .ok_or("it has no picture")?;
    if encoded.len() > KEPT_LONGEST * 4 / 3 + 4 {
        return Err(String::from("it is too large"));
    }
    let png = STANDARD
        .decode(encoded)
        .map_err(|error| format!("it does not decode: {error}"))?;
    let image = decode(&png, Some(ImageFormat::Png), SIDE)
        .map_err(|error| format!("it is not a PNG of at most {SIDE} pixels a side: {error}"))?;
    if image.width() != SIDE || image.height() != SIDE {
        return Err(format!("it is not {SIDE} pixels on a side"));
    }
    write(folder, name, &png)?;
    Ok(name.to_owned())
}

/// Looks at the picture in use every few seconds and sends it to the other PCs when it changed.
pub(crate) async fn share(folder: PathBuf, settings: PathBuf, linked: Linked) {
    let mut sent = in_use(&settings);
    let mut every = time::interval(LOOK_EVERY);
    loop {
        every.tick().await;
        let now = in_use(&settings);
        if now == sent {
            continue;
        }
        if let Some(message) = now.as_deref().and_then(|name| message(&folder, name)) {
            linked.hub.broadcast(&message);
        }
        sent = now;
    }
}

#[cfg(test)]
#[path = "icons.test.rs"]
mod tests;
