//! Stands in for dotori's browser so an agent can see the windows ask the owner for it. Usage:
//! `cargo run -p dotori --example demo_screen -- <home> <what dotori asks>`. It writes
//! `<home>/screen.json` over the daemon's, sends one page asking the owner, and prints what the
//! windows send back, such as `{"kind":"take"}`, until it is stopped.

use std::io::BufRead;
use std::io::BufReader;
use std::io::Cursor;
use std::io::Write;
use std::net::TcpListener;
use std::path::PathBuf;
use std::thread;

use image::ImageFormat;
use image::Rgb;
use image::RgbImage;

const WIDE: u32 = 1280;
const TALL: u32 = 800;

fn main() {
    let mut args = std::env::args().skip(1);
    let home = PathBuf::from(args.next().expect("usage: demo_screen <home> <asked>"));
    let asked = args.next().expect("usage: demo_screen <home> <asked>");
    let listener = TcpListener::bind(("127.0.0.1", 0)).expect("bound");
    let port = listener.local_addr().expect("an address").port();
    let door = serde_json::json!({ "port": port, "token": "demo" });
    std::fs::write(home.join("screen.json"), door.to_string()).expect("door written");
    let page = page();
    for stream in listener.incoming().flatten() {
        let asked = asked.clone();
        let page = page.clone();
        thread::spawn(move || {
            let mut write = stream.try_clone().expect("a stream");
            let state = serde_json::json!({
                "kind": "state",
                "mode": "agent",
                "url": "https://shop.example.com/login",
                "asked": asked,
            });
            let frame = serde_json::json!({
                "kind": "frame",
                "jpeg": page,
                "width": WIDE,
                "height": TALL,
            });
            let _ = writeln!(write, "{state}");
            let _ = writeln!(write, "{frame}");
            for line in BufReader::new(stream).lines().map_while(Result::ok) {
                println!("{line}");
                if line.contains("\"take\"") {
                    let owner = serde_json::json!({
                        "kind": "state",
                        "mode": "owner",
                        "url": "https://shop.example.com/login",
                    });
                    let _ = writeln!(write, "{owner}");
                    let _ = writeln!(write, "{frame}");
                }
            }
        });
    }
}

fn page() -> String {
    let mut page = RgbImage::from_pixel(WIDE, TALL, Rgb([255, 255, 255]));
    let mut fill = |x: u32, y: u32, w: u32, h: u32, color: [u8; 3]| {
        for row in y..(y + h).min(TALL) {
            for column in x..(x + w).min(WIDE) {
                page.put_pixel(column, row, Rgb(color));
            }
        }
    };
    fill(0, 0, WIDE, 72, [244, 245, 247]);
    fill(48, 26, 180, 20, [26, 29, 33]);
    fill(440, 200, 400, 360, [244, 245, 247]);
    fill(480, 240, 160, 28, [26, 29, 33]);
    fill(480, 300, 320, 48, [255, 255, 255]);
    fill(480, 368, 320, 48, [255, 255, 255]);
    fill(480, 448, 320, 52, [47, 111, 235]);
    let mut jpeg = Vec::new();
    page.write_to(&mut Cursor::new(&mut jpeg), ImageFormat::Jpeg)
        .expect("encoded");
    base64(&jpeg)
}

fn base64(bytes: &[u8]) -> String {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let joined = chunk.iter().enumerate().fold(0u32, |joined, (n, byte)| {
            joined | u32::from(*byte) << (16 - 8 * n)
        });
        for n in 0..4 {
            if n <= chunk.len() {
                out.push(char::from(ALPHABET[(joined >> (18 - 6 * n) & 63) as usize]));
            } else {
                out.push('=');
            }
        }
    }
    out
}
