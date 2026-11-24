use std::sync::atomic::AtomicUsize;
use std::sync::atomic::Ordering;

use image::Rgba;
use image::RgbaImage;

use super::*;

static NEXT: AtomicUsize = AtomicUsize::new(0);

fn folder() -> PathBuf {
    let n = NEXT.fetch_add(1, Ordering::SeqCst);
    let folder = std::env::temp_dir().join(format!("dotori-icons-{}-{n}", std::process::id()));
    let _ = std::fs::remove_dir_all(&folder);
    folder
}

fn picture(width: u32, height: u32, format: ImageFormat) -> Vec<u8> {
    let image = RgbaImage::from_fn(width, height, |x, _| {
        if x < width / 2 {
            Rgba([200, 40, 40, 255])
        } else {
            Rgba([40, 40, 200, 255])
        }
    });
    let mut bytes = Vec::new();
    DynamicImage::ImageRgba8(image)
        .write_to(&mut Cursor::new(&mut bytes), format)
        .expect("encoded");
    bytes
}

#[test]
fn a_wide_photo_is_kept_as_its_middle_square() {
    let folder = folder();

    let name = keep(&folder, &picture(900, 300, ImageFormat::Jpeg)).expect("kept");

    let kept = image::open(path(&folder, &name).expect("there")).expect("an image");
    assert_eq!((kept.width(), kept.height()), (SIDE, SIDE));
    let _ = std::fs::remove_dir_all(&folder);
}

#[test]
fn what_is_not_a_picture_is_refused() {
    let folder = folder();

    assert!(keep(&folder, b"not a picture").is_err());
    assert!(!folder.exists());
    let _ = std::fs::remove_dir_all(&folder);
}

#[test]
fn only_names_keep_makes_are_looked_up() {
    let folder = folder();
    let name = keep(&folder, &picture(64, 64, ImageFormat::Png)).expect("kept");

    assert!(path(&folder, &name).is_some());
    assert!(path(&folder, "../settings.toml").is_none());
    assert!(path(&folder, "0123456789abcdef.jpg").is_none());
    let _ = std::fs::remove_dir_all(&folder);
}

#[test]
fn a_picture_from_another_pc_is_kept_under_its_name() {
    let here = folder();
    let there = folder();
    let name = keep(&here, &picture(300, 300, ImageFormat::Png)).expect("kept");

    take(&there, &message(&here, &name).expect("a message")).expect("taken");

    assert_eq!(
        std::fs::read(path(&there, &name).expect("there")).expect("read"),
        std::fs::read(path(&here, &name).expect("here")).expect("read")
    );
    let _ = std::fs::remove_dir_all(&here);
    let _ = std::fs::remove_dir_all(&there);
}

#[test]
fn a_picture_from_another_pc_that_is_not_a_kept_icon_is_refused() {
    let folder = folder();
    let big = STANDARD.encode(picture(300, 300, ImageFormat::Png));

    let wrong_size = take(
        &folder,
        &json!({ "kind": "icon", "name": "0123456789abcdef.png", "png": big }),
    );
    let wrong_name = take(
        &folder,
        &json!({ "kind": "icon", "name": "../evil.png", "png": big }),
    );

    assert!(wrong_size.is_err());
    assert!(wrong_name.is_err());
    assert!(path(&folder, "0123456789abcdef.png").is_none());
    let _ = std::fs::remove_dir_all(&folder);
}

#[test]
fn only_the_picture_in_use_and_the_latest_few_are_kept() {
    let folder = folder();
    let first = keep(&folder, &picture(64, 64, ImageFormat::Png)).expect("kept");
    for _ in 0..6 {
        std::thread::sleep(std::time::Duration::from_millis(15));
        keep(&folder, &picture(64, 64, ImageFormat::Png)).expect("kept");
    }

    prune(&folder, &[&first]);

    let left = std::fs::read_dir(&folder).expect("listed").count();
    assert_eq!(left, KEPT_PICTURES + 1);
    assert!(path(&folder, &first).is_some(), "the one in use stays");
    let _ = std::fs::remove_dir_all(&folder);
}
