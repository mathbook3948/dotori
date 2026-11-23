//! dotori's own browser: the official Chrome for Testing build, fetched on first use and kept in
//! dotori's folder, so it never touches the browser the owner uses.

use std::io::Write;
use std::path::Path;
use std::path::PathBuf;
use std::time::Duration;

use md5::Digest;
use md5::Md5;
use serde_json::Value;

const VERSIONS: &str = "https://googlechromelabs.github.io/chrome-for-testing/last-known-good-versions-with-downloads.json";
const LIMIT: u64 = 1024 * 1024 * 1024;
const LIST_WAIT: Duration = Duration::from_secs(30);
const DOWNLOAD_WAIT: Duration = Duration::from_secs(60 * 60);
const CURRENT: &str = "current";

/// The Chrome for Testing build name of this computer, if there is one.
pub(crate) fn platform() -> Option<&'static str> {
    match (std::env::consts::OS, std::env::consts::ARCH) {
        ("linux", "x86_64") => Some("linux64"),
        ("macos", "aarch64") => Some("mac-arm64"),
        ("macos", "x86_64") => Some("mac-x64"),
        ("windows", "x86_64") => Some("win64"),
        _ => None,
    }
}

/// Where the program sits inside an unpacked build for `platform`.
pub(crate) fn program(platform: &str) -> PathBuf {
    let folder = PathBuf::from(format!("chrome-{platform}"));
    if platform.starts_with("mac") {
        folder
            .join("Google Chrome for Testing.app")
            .join("Contents")
            .join("MacOS")
            .join("Google Chrome for Testing")
    } else if platform.starts_with("win") {
        folder.join("chrome.exe")
    } else {
        folder.join("chrome")
    }
}

/// The browser already kept in `folder`, if a whole one is there.
pub(crate) fn installed(folder: &Path) -> Option<PathBuf> {
    let platform = platform()?;
    let version = std::fs::read_to_string(folder.join(CURRENT)).ok()?;
    let executable = folder.join(version.trim()).join(program(platform));
    executable.is_file().then_some(executable)
}

/// Returns the browser in `folder`, downloading the latest stable build first if none is there.
/// The download is checked against the MD5 that Google's storage reports for it.
pub(crate) async fn ensure(folder: &Path) -> Result<PathBuf, String> {
    if let Some(executable) = installed(folder) {
        return Ok(executable);
    }
    let platform = platform().ok_or("Chrome for Testing has no build for this computer")?;
    let (version, url) = latest(platform).await?;
    std::fs::create_dir_all(folder).map_err(|error| error.to_string())?;
    let archive = folder.join(format!("download.{}.zip", std::process::id()));
    let fetched = download(&url, &archive).await;
    let unpacked = match fetched {
        Ok(()) => {
            let folder = folder.to_owned();
            let archive = archive.clone();
            tokio::task::spawn_blocking(move || unpack(&archive, &folder, &version, platform))
                .await
                .map_err(|error| error.to_string())
                .and_then(|unpacked| unpacked)
        }
        Err(error) => Err(error),
    };
    let _ = std::fs::remove_file(&archive);
    unpacked
}

async fn latest(platform: &str) -> Result<(String, String), String> {
    let fail = |error: reqwest::Error| format!("cannot list Chrome for Testing builds: {error}");
    let text = reqwest::Client::builder()
        .timeout(LIST_WAIT)
        .build()
        .map_err(fail)?
        .get(VERSIONS)
        .send()
        .await
        .and_then(reqwest::Response::error_for_status)
        .map_err(fail)?
        .text()
        .await
        .map_err(fail)?;
    let list: Value = serde_json::from_str(&text)
        .map_err(|error| format!("cannot read the Chrome for Testing build list: {error}"))?;
    pick(&list, platform)
}

/// The stable version and its download address for `platform` in the list of builds.
pub(crate) fn pick(list: &Value, platform: &str) -> Result<(String, String), String> {
    let stable = list
        .pointer("/channels/Stable")
        .ok_or("the build list has no stable channel")?;
    let version = stable
        .get("version")
        .and_then(Value::as_str)
        .filter(|version| {
            !version.is_empty() && version.chars().all(|c| c.is_ascii_digit() || c == '.')
        })
        .ok_or("the build list has no usable stable version")?;
    let url = stable
        .pointer("/downloads/chrome")
        .and_then(Value::as_array)
        .and_then(|builds| {
            builds
                .iter()
                .find(|build| build.get("platform").and_then(Value::as_str) == Some(platform))
        })
        .and_then(|build| build.get("url"))
        .and_then(Value::as_str)
        .filter(|url| url.starts_with("https://storage.googleapis.com/"))
        .ok_or_else(|| format!("the stable build has no download for {platform}"))?;
    Ok((version.to_owned(), url.to_owned()))
}

async fn download(url: &str, archive: &Path) -> Result<(), String> {
    let fail = |error: reqwest::Error| format!("cannot download Chrome for Testing: {error}");
    let mut response = reqwest::Client::builder()
        .timeout(DOWNLOAD_WAIT)
        .build()
        .map_err(fail)?
        .get(url)
        .send()
        .await
        .and_then(reqwest::Response::error_for_status)
        .map_err(fail)?;
    let expected = reported_md5(response.headers())
        .ok_or("the download did not say its MD5, so it cannot be checked")?;
    let mut file = std::fs::File::create(archive).map_err(|error| error.to_string())?;
    let mut digest = Md5::new();
    let mut size = 0u64;
    while let Some(chunk) = response.chunk().await.map_err(fail)? {
        size = size.saturating_add(chunk.len() as u64);
        if size > LIMIT {
            return Err(String::from("the Chrome for Testing download is too large"));
        }
        digest.update(&chunk);
        file.write_all(&chunk).map_err(|error| error.to_string())?;
    }
    file.sync_all().map_err(|error| error.to_string())?;
    let found = base64(&digest.finalize());
    if found != expected {
        return Err(format!(
            "the Chrome for Testing download is damaged (md5 {found}, expected {expected})"
        ));
    }
    Ok(())
}

/// The base64 MD5 in Google storage's `x-goog-hash` headers.
pub(crate) fn reported_md5(headers: &reqwest::header::HeaderMap) -> Option<String> {
    headers
        .get_all("x-goog-hash")
        .iter()
        .filter_map(|value| value.to_str().ok())
        .flat_map(|value| value.split(','))
        .find_map(|part| part.trim().strip_prefix("md5="))
        .map(str::to_owned)
}

fn unpack(archive: &Path, folder: &Path, version: &str, platform: &str) -> Result<PathBuf, String> {
    let draft = folder.join(format!("{version}.new.{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&draft);
    std::fs::create_dir_all(&draft).map_err(|error| error.to_string())?;
    let file = std::fs::File::open(archive).map_err(|error| error.to_string())?;
    let mut zip = zip::ZipArchive::new(file).map_err(|error| error.to_string())?;
    zip.extract(&draft)
        .map_err(|error| format!("cannot unpack Chrome for Testing: {error}"))?;
    if !draft.join(program(platform)).is_file() {
        let _ = std::fs::remove_dir_all(&draft);
        return Err(String::from(
            "the Chrome for Testing archive has no browser in it",
        ));
    }
    let target = folder.join(version);
    let _ = std::fs::remove_dir_all(&target);
    std::fs::rename(&draft, &target).map_err(|error| error.to_string())?;
    let pointer = folder.join(format!("{CURRENT}.new"));
    std::fs::write(&pointer, version).map_err(|error| error.to_string())?;
    std::fs::rename(&pointer, folder.join(CURRENT)).map_err(|error| error.to_string())?;
    Ok(target.join(program(platform)))
}

pub(crate) fn base64(bytes: &[u8]) -> String {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::new();
    for chunk in bytes.chunks(3) {
        let joined = chunk.iter().enumerate().fold(0u32, |joined, (i, byte)| {
            joined | u32::from(*byte) << (16 - 8 * i)
        });
        for i in 0..4 {
            if i <= chunk.len() {
                out.push(char::from(ALPHABET[(joined >> (18 - 6 * i) & 63) as usize]));
            } else {
                out.push('=');
            }
        }
    }
    out
}

#[cfg(test)]
#[path = "chrome.test.rs"]
mod tests;
