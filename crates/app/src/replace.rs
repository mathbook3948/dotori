use std::fs;
use std::fs::OpenOptions;
use std::io::Write;
use std::path::Path;

/// Writes `bytes` to a draft beside `path` and renames it over `path`, so a crash never leaves
/// a half-written file there. `options` sets how the draft is created, such as its permissions.
pub(crate) fn replace(path: &Path, bytes: &[u8], mut options: OpenOptions) -> Result<(), String> {
    let fail = |error: std::io::Error| format!("{}: {error}", path.display());
    if let Some(folder) = path.parent() {
        fs::create_dir_all(folder).map_err(fail)?;
    }
    let mut draft = path.as_os_str().to_owned();
    draft.push(".new");
    let mut file = options
        .write(true)
        .create(true)
        .truncate(true)
        .open(&draft)
        .map_err(fail)?;
    file.write_all(bytes).map_err(fail)?;
    file.sync_all().map_err(fail)?;
    fs::rename(&draft, path).map_err(fail)
}
