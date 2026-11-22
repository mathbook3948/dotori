use super::*;

#[test]
fn a_download_with_the_wrong_digest_is_refused() {
    let folder = std::env::temp_dir().join(format!("dotori-dictionary-{}", std::process::id()));
    let refused = unpack(b"not the dictionary", &folder).expect_err("damaged");
    assert!(refused.contains("damaged"), "{refused}");
    assert!(!ready(&folder));
    let _ = std::fs::remove_dir_all(&folder);
}
