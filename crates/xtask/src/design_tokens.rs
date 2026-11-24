use std::error::Error;
use std::io::Write;
use std::path::Path;
use std::process::Command;
use std::process::Stdio;

const DESIGN: &str = "DESIGN.md";
const THEMES: &str = "crates/app/src/window/themes.rs";
const LOGO: &str = "logo";

#[derive(Debug, PartialEq, Eq)]
pub struct Palette {
    pub tokens: Vec<(String, String)>,
}

#[derive(Debug, PartialEq, Eq)]
pub struct Colors {
    pub themes: Vec<(String, Palette, Palette)>,
    pub logo: (Palette, Palette),
}

pub fn leaves(design: &str) -> Result<Vec<(Vec<String>, String)>, String> {
    let mut parts = design.splitn(3, "---\n");
    let (Some(""), Some(front)) = (parts.next(), parts.next()) else {
        return Err(format!("{DESIGN} starts without front matter"));
    };
    let mut path: Vec<String> = Vec::new();
    let mut found = Vec::new();
    for line in front.lines().filter(|line| !line.trim().is_empty()) {
        let indent = line.len() - line.trim_start().len();
        if indent % 2 != 0 {
            return Err(format!("{DESIGN}: odd indentation in `{line}`"));
        }
        let Some((key, value)) = line.trim().split_once(':') else {
            return Err(format!("{DESIGN}: no key in `{line}`"));
        };
        path.truncate(indent / 2);
        path.push(key.trim().to_owned());
        let value = value.trim().trim_matches('"');
        if !value.is_empty() {
            found.push((path.clone(), value.to_owned()));
        }
    }
    Ok(found)
}

fn empty() -> Palette {
    Palette { tokens: Vec::new() }
}

pub fn colors(design: &str) -> Result<Colors, String> {
    let mut themes: Vec<(String, Palette, Palette)> = Vec::new();
    let mut logo = (empty(), empty());
    for (path, value) in leaves(design)? {
        let [group, name, look, token] = path.as_slice() else {
            continue;
        };
        if group != "colors" {
            continue;
        }
        let light = match look.as_str() {
            "light" => true,
            "dark" => false,
            _ => return Err(format!("colors.{name}.{look}: a look is light or dark")),
        };
        if hex(&value).is_none() {
            return Err(format!(
                "colors.{name}.{look}.{token}: `{value}` is not #RRGGBB or #RRGGBBAA"
            ));
        }
        let palette = if name == LOGO {
            if light { &mut logo.0 } else { &mut logo.1 }
        } else {
            let at = match themes.iter().position(|(known, _, _)| known == name) {
                Some(at) => at,
                None => {
                    themes.push((name.clone(), empty(), empty()));
                    themes.len() - 1
                }
            };
            let (_, light_set, dark_set) = &mut themes[at];
            if light { light_set } else { dark_set }
        };
        palette.tokens.push((token.replace('-', "_"), value));
    }
    let keys = |palette: &Palette| {
        palette
            .tokens
            .iter()
            .map(|(key, _)| key.clone())
            .collect::<Vec<_>>()
    };
    let Some((first, first_light, _)) = themes.first() else {
        return Err(format!("{DESIGN} has no color themes"));
    };
    for (name, light, dark) in &themes {
        if keys(light) != keys(dark) {
            return Err(format!(
                "colors.{name}: the light and dark looks name different tokens"
            ));
        }
        if keys(light) != keys(first_light) {
            return Err(format!(
                "colors.{name}: names different tokens than colors.{first}"
            ));
        }
    }
    if keys(&logo.0) != keys(&logo.1) || logo.0.tokens.is_empty() {
        return Err(format!(
            "colors.{LOGO}: needs the same tokens in its light and dark looks"
        ));
    }
    Ok(Colors { themes, logo })
}

pub fn hex(value: &str) -> Option<u32> {
    let digits = value.strip_prefix('#')?;
    let digits = match digits.len() {
        6 => format!("{digits}FF"),
        8 => digits.to_owned(),
        _ => return None,
    };
    u32::from_str_radix(&digits, 16).ok()
}

pub fn render(colors: &Colors) -> String {
    let palette = |type_name: &str, palette: &Palette| {
        let fields: String = palette
            .tokens
            .iter()
            .map(|(token, value)| format!("{token}: hex(0x{:08X}),", hex(value).unwrap_or(0)))
            .collect();
        format!("{type_name} {{ {fields} }}")
    };
    let themes: String = colors
        .themes
        .iter()
        .map(|(name, light, dark)| {
            format!(
                "(\"{name}\", {}, {}),",
                palette("Theme", light),
                palette("Theme", dark)
            )
        })
        .collect();
    format!(
        "//! The colors of DESIGN.md as code. `mise run design` writes this file from DESIGN.md and\n\
         //! `mise run check` fails when it falls behind, so change the colors there.\n\
         \n\
         use crate::window::theme::Logo;\n\
         use crate::window::theme::Theme;\n\
         use crate::window::theme::hex;\n\
         \n\
         /// Every theme of DESIGN.md: its name, its light colors and its dark colors.\n\
         pub(crate) const THEMES: [(&str, Theme, Theme); {}] = [{themes}];\n\
         \n\
         /// The logo's colors on a light theme and on a dark one.\n\
         pub(crate) const LOGO: (Logo, Logo) = ({}, {});\n",
        colors.themes.len(),
        palette("Logo", &colors.logo.0),
        palette("Logo", &colors.logo.1),
    )
}

fn formatted(workspace_root: &Path, source: &str) -> Result<String, Box<dyn Error>> {
    let mut rustfmt = Command::new("rustfmt")
        .current_dir(workspace_root)
        .args(["--edition", "2024", "--emit", "stdout"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()?;
    rustfmt
        .stdin
        .take()
        .ok_or("no stdin for rustfmt")?
        .write_all(source.as_bytes())?;
    let output = rustfmt.wait_with_output()?;
    if !output.status.success() {
        return Err("rustfmt could not format the themes".into());
    }
    Ok(String::from_utf8(output.stdout)?)
}

fn wanted(workspace_root: &Path) -> Result<String, Box<dyn Error>> {
    let design = std::fs::read_to_string(workspace_root.join(DESIGN))?;
    let colors = colors(&design)?;
    formatted(workspace_root, &render(&colors))
}

pub fn check(workspace_root: &Path) -> Result<Vec<String>, Box<dyn Error>> {
    let wanted = wanted(workspace_root)?;
    let written = std::fs::read_to_string(workspace_root.join(THEMES)).unwrap_or_default();
    Ok(if written == wanted {
        Vec::new()
    } else {
        vec![format!(
            "{THEMES} does not match the colors of {DESIGN}; run `mise run design`"
        )]
    })
}

pub fn write(workspace_root: &Path) -> Result<Vec<String>, Box<dyn Error>> {
    std::fs::write(workspace_root.join(THEMES), wanted(workspace_root)?)?;
    Ok(Vec::new())
}

#[cfg(test)]
#[path = "design_tokens.test.rs"]
mod tests;
