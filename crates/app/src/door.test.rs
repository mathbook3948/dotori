use super::*;

#[test]
fn a_door_written_is_read_back() {
    let path = std::env::temp_dir()
        .join(format!("dotori-door-{}", std::process::id()))
        .join("door.json");
    let door = Door::new(4242);

    door.write(&path).expect("written");

    assert_eq!(Door::read(&path).expect("read"), door);
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = fs::metadata(&path).expect("exists").permissions().mode();
        assert_eq!(mode & 0o777, 0o600);
    }
}

#[test]
fn tokens_are_long_and_differ() {
    let first = Door::new(1).token;
    let second = Door::new(1).token;

    assert_eq!(first.len(), 32);
    assert_ne!(first, second);
}

#[test]
fn a_missing_door_means_dotori_is_not_running() {
    let error = Door::read(Path::new("/nonexistent/dotori/door.json")).expect_err("missing");

    assert!(error.starts_with("dotori is not running"));
}
