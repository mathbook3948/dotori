use std::sync::atomic::AtomicUsize;
use std::sync::atomic::Ordering;

use serde_json::json;

use super::*;

static NEXT: AtomicUsize = AtomicUsize::new(0);

fn scratch() -> PathBuf {
    let n = NEXT.fetch_add(1, Ordering::SeqCst);
    let dir = std::env::temp_dir().join(format!("dotori-chrome-{}-{n}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    dir
}

fn list() -> Value {
    json!({
        "channels": {
            "Beta": { "version": "159.0.1.0", "downloads": { "chrome": [] } },
            "Stable": {
                "version": "158.0.8127.36",
                "downloads": {
                    "chrome": [
                        { "platform": "linux64", "url": "https://storage.googleapis.com/chrome-for-testing-public/158.0.8127.36/linux64/chrome-linux64.zip" },
                        { "platform": "win64", "url": "https://elsewhere.example/chrome-win64.zip" }
                    ]
                }
            }
        }
    })
}

#[test]
fn the_stable_build_for_this_platform_is_picked() {
    let (version, url) = pick(&list(), "linux64").expect("picked");

    assert_eq!(version, "158.0.8127.36");
    assert!(url.ends_with("/linux64/chrome-linux64.zip"), "{url}");
}

#[test]
fn a_build_off_google_storage_or_missing_is_refused() {
    assert!(pick(&list(), "win64").is_err());
    assert!(pick(&list(), "mac-arm64").is_err());
    let mut odd = list();
    odd["channels"]["Stable"]["version"] = json!("../158");
    assert!(pick(&odd, "linux64").is_err());
}

#[test]
fn the_md5_is_read_from_googles_hash_headers() {
    let mut headers = reqwest::header::HeaderMap::new();
    headers.append("x-goog-hash", "crc32c=SyYyfg==".parse().expect("header"));
    headers.append(
        "x-goog-hash",
        "md5=xmTYpjL3Of9XM+V8v3+UHw==".parse().expect("header"),
    );

    assert_eq!(
        reported_md5(&headers).as_deref(),
        Some("xmTYpjL3Of9XM+V8v3+UHw==")
    );
    assert_eq!(reported_md5(&reqwest::header::HeaderMap::new()), None);
}

#[test]
fn base64_matches_the_standard_alphabet_and_padding() {
    assert_eq!(base64(b""), "");
    assert_eq!(base64(b"f"), "Zg==");
    assert_eq!(base64(b"fo"), "Zm8=");
    assert_eq!(base64(b"foo"), "Zm9v");
    assert_eq!(base64(&Md5::digest(b"")), "1B2M2Y8AsgTpgAmY7PhCfg==");
}

#[test]
fn an_unpacked_build_is_found_only_when_whole() {
    let Some(platform) = platform() else {
        return;
    };
    let folder = scratch();
    assert_eq!(installed(&folder), None);

    let archive = folder.join("build.zip");
    std::fs::create_dir_all(&folder).expect("made");
    let mut zip = zip::ZipWriter::new(std::fs::File::create(&archive).expect("created"));
    zip.start_file(
        program(platform).to_string_lossy().replace('\\', "/"),
        zip::write::SimpleFileOptions::default(),
    )
    .expect("started");
    zip.write_all(b"#!/bin/sh\n").expect("written");
    zip.finish().expect("finished");

    let executable = unpack(&archive, &folder, "158.0.1.2", platform).expect("unpacked");

    assert_eq!(installed(&folder), Some(executable));
    std::fs::write(folder.join("current"), "157.0.0.0").expect("pointed elsewhere");
    assert_eq!(installed(&folder), None);
    let bad = folder.join("empty.zip");
    zip::ZipWriter::new(std::fs::File::create(&bad).expect("created"))
        .finish()
        .expect("finished");
    assert!(unpack(&bad, &folder, "158.0.1.3", platform).is_err());
}

#[tokio::test]
#[ignore = "downloads about 200 MB from Google"]
async fn the_latest_stable_build_downloads_checks_out_and_runs() {
    let folder = scratch();

    let executable = ensure(&folder).await.expect("downloaded");
    let again = ensure(&folder).await.expect("kept");

    assert_eq!(executable, again);
    let version = std::process::Command::new(&executable)
        .arg("--version")
        .output()
        .expect("ran");
    let shown = String::from_utf8_lossy(&version.stdout);
    assert!(shown.contains("Chrome for Testing"), "{shown}");
    let _ = std::fs::remove_dir_all(&folder);
}
