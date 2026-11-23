use std::fs;
use std::fs::OpenOptions;
use std::io::ErrorKind;
use std::path::Path;

use crate::door;
use crate::replace::replace;

/// The name this PC writes into every record it makes. It is chosen at random the first time and
/// kept in `path` from then on.
pub(crate) fn name(path: &Path) -> Result<String, String> {
    match fs::read_to_string(path) {
        Ok(text) if is_name(text.trim()) => return Ok(text.trim().to_owned()),
        Ok(_) => return Err(format!("{} does not hold a PC name", path.display())),
        Err(error) if error.kind() == ErrorKind::NotFound => {}
        Err(error) => return Err(format!("{}: {error}", path.display())),
    }
    let chosen = door::token()?;
    replace(path, chosen.as_bytes(), OpenOptions::new())?;
    Ok(chosen)
}

fn is_name(text: &str) -> bool {
    text.len() == 32 && text.bytes().all(|byte| byte.is_ascii_hexdigit())
}

#[cfg(test)]
#[path = "device.test.rs"]
mod tests;
