//! Speaks to the running dotori as the owner does from a window, so an agent can drive the
//! demo without typing into a window or moving the mouse. Usage:
//! `cargo run -p dotori --example demo_owner -- <home> say <text>` sends a message, and
//! `... <home> allow` or `... <home> decline` answers the call that waits on the owner.

use std::io::BufRead;
use std::io::BufReader;
use std::io::Write;
use std::net::TcpStream;
use std::path::PathBuf;
use std::time::Duration;

use serde_json::Value;
use serde_json::json;

const QUIET: Duration = Duration::from_millis(700);

fn main() {
    let mut args = std::env::args().skip(1);
    let usage = "usage: demo_owner <home> say <text> | allow | decline";
    let home = PathBuf::from(args.next().expect(usage));
    let what = args.next().expect(usage);
    let door: Value = serde_json::from_str(
        &std::fs::read_to_string(home.join("door.json")).expect("dotori is running"),
    )
    .expect("a door");
    let port = door["port"].as_u64().expect("a port");
    let mut stream = TcpStream::connect(("127.0.0.1", port as u16)).expect("dotori is reachable");
    send(&mut stream, &json!({ "token": door["token"] }));
    let said = match what.as_str() {
        "say" => json!({ "kind": "say", "text": args.next().expect(usage) }),
        "allow" | "decline" => json!({
            "kind": "decide",
            "id": waiting(&stream).expect("a call waits on the owner"),
            "ok": what == "allow",
        }),
        _ => panic!("{usage}"),
    };
    send(&mut stream, &said);
    std::thread::sleep(QUIET);
}

fn send(stream: &mut TcpStream, value: &Value) {
    writeln!(stream, "{value}").expect("sent");
}

fn waiting(stream: &TcpStream) -> Option<u64> {
    stream.set_read_timeout(Some(QUIET)).expect("a timeout");
    let mut asked = Vec::new();
    let mut decided = Vec::new();
    for line in BufReader::new(stream).lines() {
        let Ok(line) = line else { break };
        let Ok(event) = serde_json::from_str::<Value>(&line) else {
            continue;
        };
        let id = event["id"].as_u64();
        match event["kind"].as_str() {
            Some("ask") => asked.extend(id),
            Some("decided" | "lapsed") => decided.extend(id),
            _ => {}
        }
    }
    asked.into_iter().rev().find(|id| !decided.contains(id))
}
