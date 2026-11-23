use super::*;

fn files(name: &str) -> LoginFiles {
    let folder = std::env::temp_dir().join(format!("dotori-logins-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&folder);
    std::fs::create_dir_all(&folder).expect("folder");
    LoginFiles {
        out: folder.join("logins-out.json"),
        into: folder.join("logins-in.json"),
    }
}

fn copy(at: i64) -> LoginCopy {
    LoginCopy {
        at,
        cookies: vec![json!({ "name": "sid", "value": format!("v{at}"), "domain": "example.com" })],
    }
}

#[test]
fn only_a_copy_newer_than_any_here_is_kept_and_only_the_user_can_read_it() {
    let files = files("keep");
    write_privately(&files.out, &copy(10)).expect("written");

    assert!(
        !files.keep(&copy(5)).expect("checked"),
        "older than this browser's own"
    );
    assert!(files.keep(&copy(20)).expect("kept"));
    assert!(!files.keep(&copy(20)).expect("checked"));
    assert_eq!(read(&files.into), Some(copy(20)));
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(&files.into)
            .expect("metadata")
            .permissions()
            .mode();
        assert_eq!(mode & 0o777, 0o600);
    }
}
