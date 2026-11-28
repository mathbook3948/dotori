//! Serves the end-to-end test's fake model on a port, so an agent can drive the real windows with
//! the same fixed conversation. Usage: `cargo run -p dotori --example demo_model -- <port-file>`.
//! It writes the port it listens on to `<port-file>` and runs until it is stopped. It thinks for
//! 1.5 seconds before each answer, so the window shows dotori at work. A second argument and a
//! third replace the command it asks to run and what that command does, so a run allowed on a real
//! PC can be harmless, such as `echo`.

#[path = "../tests/support/model.rs"]
mod model;

#[tokio::main]
async fn main() {
    let port_file = std::env::args()
        .nth(1)
        .expect("usage: demo_model <port-file>");
    let address = model::start(model::Script {
        command: std::env::args()
            .nth(2)
            .unwrap_or_else(|| String::from("mv ~/Downloads/*.pdf ~/Documents/영수증/")),
        purpose: std::env::args()
            .nth(3)
            .unwrap_or_else(|| String::from("다운로드 폴더의 PDF를 영수증 폴더로 옮겨요")),
        think: std::time::Duration::from_millis(1500),
        tools: std::sync::Arc::default(),
        results: std::sync::Arc::default(),
    })
    .await;
    std::fs::write(&port_file, address.port().to_string()).expect("port written");
    std::future::pending::<()>().await;
}
