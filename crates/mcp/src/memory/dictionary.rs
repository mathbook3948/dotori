use std::io::Cursor;
use std::path::Path;
use std::path::PathBuf;
use std::time::Duration;

use sha2::Digest;
use sha2::Sha256;

const URL: &str =
    "https://github.com/lindera/lindera/releases/download/v7.0.0/lindera-ko-dic-7.0.0.zip";
const DIGEST: &str = "0cdf9c4b8ebcf549147979d18e517d0323a6b37790c1add8cb0525fcd5315f26";
const NAME: &str = "lindera-ko-dic";
const LIMIT: usize = 64 * 1024 * 1024;
const WAIT: Duration = Duration::from_secs(600);

/// Where the Korean dictionary lives once it is in `folder`.
pub(crate) fn path(folder: &Path) -> PathBuf {
    folder.join(NAME)
}

pub(crate) fn ready(folder: &Path) -> bool {
    path(folder).join("metadata.json").is_file()
}

/// Downloads the Korean dictionary into `folder` unless it is there, and checks its digest.
pub(crate) async fn fetch(folder: &Path) -> Result<PathBuf, String> {
    if ready(folder) {
        return Ok(path(folder));
    }
    let client = reqwest::Client::builder()
        .timeout(WAIT)
        .build()
        .map_err(|error| error.to_string())?;
    let mut response = client
        .get(URL)
        .send()
        .await
        .and_then(reqwest::Response::error_for_status)
        .map_err(|error| format!("cannot download the dictionary: {error}"))?;
    let mut bytes = Vec::new();
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|error| format!("cannot download the dictionary: {error}"))?
    {
        bytes.extend_from_slice(&chunk);
        if bytes.len() > LIMIT {
            return Err(String::from("the dictionary download is too large"));
        }
    }
    let folder = folder.to_owned();
    tokio::task::spawn_blocking(move || unpack(&bytes, &folder))
        .await
        .map_err(|error| error.to_string())?
}

fn unpack(bytes: &[u8], folder: &Path) -> Result<PathBuf, String> {
    let digest: String = Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect();
    if digest != DIGEST {
        return Err(format!(
            "the dictionary download is damaged (sha-256 {digest})"
        ));
    }
    let draft = folder.join(format!("{NAME}.new"));
    let _ = std::fs::remove_dir_all(&draft);
    std::fs::create_dir_all(&draft).map_err(|error| error.to_string())?;
    let mut archive =
        zip::ZipArchive::new(Cursor::new(bytes)).map_err(|error| error.to_string())?;
    archive
        .extract(&draft)
        .map_err(|error| format!("cannot unpack the dictionary: {error}"))?;
    let unpacked = draft.join(NAME);
    if !unpacked.join("metadata.json").is_file() {
        return Err(String::from("the dictionary archive has no metadata.json"));
    }
    let target = path(folder);
    let _ = std::fs::remove_dir_all(&target);
    std::fs::rename(&unpacked, &target).map_err(|error| error.to_string())?;
    let _ = std::fs::remove_dir_all(&draft);
    Ok(target)
}

#[cfg(test)]
#[path = "dictionary.test.rs"]
mod tests;
