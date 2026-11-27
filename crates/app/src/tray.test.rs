use super::*;

#[test]
fn the_tray_takes_the_assistants_picture_at_its_size() {
    assert_eq!(
        app_icon::rgba(ICON_SIZE).len(),
        (ICON_SIZE * ICON_SIZE * 4) as usize
    );
    assert!(tray_picture().is_ok());
}
