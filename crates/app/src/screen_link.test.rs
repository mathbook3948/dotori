use super::*;

#[test]
fn base64_reads_back_what_was_written() {
    assert_eq!(unbase64("").expect("empty"), b"");
    assert_eq!(unbase64("Zg==").expect("one"), b"f");
    assert_eq!(unbase64("Zm9v").expect("three"), b"foo");
    assert_eq!(
        unbase64("aGVsbG8gd29ybGQ=").expect("eleven"),
        b"hello world"
    );
    assert!(unbase64("not*base64").is_none());
}

#[test]
fn a_frame_is_decoded_to_bgra_pixels() {
    let mut jpeg = Vec::new();
    let red = image::RgbImage::from_pixel(4, 2, image::Rgb([250, 10, 10]));
    image::DynamicImage::ImageRgb8(red)
        .write_to(
            &mut std::io::Cursor::new(&mut jpeg),
            image::ImageFormat::Jpeg,
        )
        .expect("encoded");
    let line = serde_json::json!({
        "kind": "frame",
        "jpeg": encode(&jpeg),
        "width": 1280,
        "height": 800
    })
    .to_string();

    let Some(Seen::Frame(picture)) = understood(&line) else {
        panic!("a frame");
    };

    assert_eq!((picture.width, picture.height), (4, 2));
    assert_eq!((picture.page_width, picture.page_height), (1280.0, 800.0));
    assert_eq!(picture.bgra.len(), 4 * 2 * 4);
    assert!(
        picture.bgra[2] > 200 && picture.bgra[0] < 60,
        "{:?}",
        &picture.bgra[..4]
    );
}

#[test]
fn state_and_pointer_lines_are_understood() {
    assert!(matches!(
        understood(r#"{"kind":"state","mode":"owner"}"#),
        Some(Seen::State(_))
    ));
    assert!(matches!(
        understood(r#"{"kind":"pointer","x":1,"y":2}"#),
        Some(Seen::Pointer { x, y }) if x == 1.0 && y == 2.0
    ));
    assert!(understood(r#"{"kind":"frame","jpeg":"AAAA"}"#).is_none());
    assert!(understood("nonsense").is_none());
}

fn encode(bytes: &[u8]) -> String {
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
