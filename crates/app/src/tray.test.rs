use super::*;

#[test]
fn the_icon_is_a_circle_with_clear_corners() {
    let pixels = acorn();
    let size = ICON_SIZE as usize;
    assert_eq!(pixels.len(), size * size * 4);
    let alpha = |x: usize, y: usize| pixels[(y * size + x) * 4 + 3];
    assert_eq!(alpha(0, 0), 0);
    assert_eq!(alpha(size - 1, size - 1), 0);
    assert_eq!(alpha(size / 2, size / 2), 0xff);
    assert!(Icon::from_rgba(pixels.clone(), ICON_SIZE, ICON_SIZE).is_ok());
}
