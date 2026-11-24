use std::fs;
use std::fs::OpenOptions;
use std::io::Write;
use std::path::Path;
use std::sync::atomic::AtomicU64;
use std::sync::atomic::Ordering;

static DRAFTS: AtomicU64 = AtomicU64::new(0);

/// Writes `bytes` to a draft beside `path` and renames it over `path`, so a crash never leaves
/// a half-written file there. Each write has a draft of its own, so two processes writing at
/// once never mix their bytes. `options` sets how the draft is created, such as its permissions.
pub(crate) fn replace(path: &Path, bytes: &[u8], mut options: OpenOptions) -> Result<(), String> {
    let fail = |error: std::io::Error| format!("{}: {error}", path.display());
    if let Some(folder) = path.parent() {
        fs::create_dir_all(folder).map_err(fail)?;
    }
    let mut draft = path.as_os_str().to_owned();
    draft.push(format!(
        ".new-{}-{}",
        std::process::id(),
        DRAFTS.fetch_add(1, Ordering::Relaxed)
    ));
    let written = options
        .write(true)
        .create_new(true)
        .open(&draft)
        .and_then(|mut file| {
            file.write_all(bytes)?;
            file.sync_all()
        })
        .and_then(|()| fs::rename(&draft, path));
    if written.is_err() {
        let _ = fs::remove_file(&draft);
    }
    written.map_err(fail)
}
