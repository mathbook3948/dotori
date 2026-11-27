use super::*;

#[test]
fn without_a_picture_the_acorn_fills_the_middle_and_leaves_the_corners_clear() {
    let side = 32;
    let pixels = acorn(side);

    assert_eq!(pixels.dimensions(), (side, side));
    assert_eq!(pixels.get_pixel(0, 0).0[3], 0, "a corner stays clear");
    assert!(
        pixels.pixels().filter(|pixel| pixel.0[3] > 200).count() > (side * side / 5) as usize,
        "the acorn covers the middle"
    );
}

#[test]
fn a_picture_is_cut_to_a_rounded_square() {
    let root = std::env::temp_dir().join(format!("dotori-app-icon-{}", std::process::id()));
    std::fs::create_dir_all(&root).expect("a folder");
    let path = root.join("blue.png");
    RgbaImage::from_pixel(80, 40, image::Rgba([40, 90, 200, 255]))
        .save(&path)
        .expect("saved");

    let tile = picture(&path, 32).expect("read");

    assert_eq!(tile.dimensions(), (32, 32));
    assert_eq!(tile.get_pixel(0, 0).0[3], 0, "the corners are cut away");
    assert_eq!(tile.get_pixel(16, 16).0, [40, 90, 200, 255]);
    assert_eq!(
        tile.get_pixel(16, 0).0[3],
        255,
        "the middle of an edge stays, as a circle would not keep it"
    );
    assert_eq!(
        tile.get_pixel(4, 4).0[3],
        255,
        "near a corner it is still filled, as a circle would not be"
    );
    let _ = std::fs::remove_dir_all(&root);
}
