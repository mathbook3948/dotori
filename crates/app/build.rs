//! Builds the icon of the Windows program from the acorn the windows draw, so the taskbar and the
//! file show it. Other systems take their icon from the desktop file or the app bundle.

use std::env;
use std::fs;
use std::path::PathBuf;

const ACORN: &str = include_str!("src/window/acorn.svg");
const CAP: &str = "#5B3A1E";
const NUT: &str = "#C98A4B";
const SIDES: [u32; 7] = [16, 24, 32, 48, 64, 128, 256];

fn main() {
    println!("cargo:rerun-if-changed=src/window/acorn.svg");
    println!("cargo:rerun-if-changed=build.rs");
    if env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return;
    }
    let out = PathBuf::from(env::var("OUT_DIR").expect("cargo names the output folder"));
    let icon = out.join("dotori.ico");
    fs::write(&icon, ico(&SIDES.map(png))).expect("the icon is written");
    let script = out.join("dotori.rc");
    let path = icon.display().to_string().replace('\\', "\\\\");
    fs::write(&script, format!("1 ICON \"{path}\"\n")).expect("the resource script is written");
    embed_resource::compile(&script, embed_resource::NONE)
        .manifest_optional()
        .expect("the icon is built into the program");
}

fn png(side: u32) -> (u32, Vec<u8>) {
    let svg = ACORN
        .replace("{width}", &format!("{}", side as f32 * 176. / 196.))
        .replace("{height}", &side.to_string())
        .replace("{cap}", CAP)
        .replace("{nut}", NUT);
    let tree = resvg::usvg::Tree::from_str(&svg, &Default::default()).expect("the acorn reads");
    let mut pixmap = resvg::tiny_skia::Pixmap::new(side, side).expect("a canvas");
    let size = tree.size();
    let shift = resvg::tiny_skia::Transform::from_translate(
        (side as f32 - size.width()) / 2.,
        (side as f32 - size.height()) / 2.,
    );
    resvg::render(&tree, shift, &mut pixmap.as_mut());
    (side, pixmap.encode_png().expect("the acorn encodes"))
}

/// An ICO file of PNG images, which Windows reads at every size since Vista.
fn ico(images: &[(u32, Vec<u8>)]) -> Vec<u8> {
    let count = images.len() as u16;
    let mut file = Vec::new();
    file.extend_from_slice(&0u16.to_le_bytes());
    file.extend_from_slice(&1u16.to_le_bytes());
    file.extend_from_slice(&count.to_le_bytes());
    let mut offset = 6 + 16 * images.len() as u32;
    for (side, data) in images {
        let edge = if *side >= 256 { 0 } else { *side as u8 };
        file.extend_from_slice(&[edge, edge, 0, 0]);
        file.extend_from_slice(&1u16.to_le_bytes());
        file.extend_from_slice(&32u16.to_le_bytes());
        file.extend_from_slice(&(data.len() as u32).to_le_bytes());
        file.extend_from_slice(&offset.to_le_bytes());
        offset += data.len() as u32;
    }
    for (_, data) in images {
        file.extend_from_slice(data);
    }
    file
}
