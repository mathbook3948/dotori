use super::*;

#[test]
fn console_text_reads_the_code_page_of_this_pc_where_windows_has_one() {
    let ascii = console_text(b"dir C:\\");
    if cfg!(target_os = "windows") {
        assert_eq!(
            ascii.as_deref(),
            Some("dir C:\\"),
            "plain letters read the same"
        );
        assert_eq!(console_text(b""), None);
    } else {
        assert_eq!(ascii, None);
    }
}

#[test]
fn a_handle_that_names_no_window_is_refused_where_windows_has_windows() {
    let png = b"\x89PNG\r\n\x1a\nnot really";
    let set = set_window_icon(0, (png, 32), (png, 64));
    if cfg!(target_os = "windows") {
        assert!(set.is_err(), "no window has the handle 0");
    } else {
        assert!(set.is_ok(), "nothing to do here");
    }
}
