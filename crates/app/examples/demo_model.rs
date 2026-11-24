//! Serves the end-to-end test's fake model on a port, so an agent can drive the real windows with
//! the same fixed conversation. Usage: `cargo run -p dotori --example demo_model -- <port-file>`.
//! It writes the port it listens on to `<port-file>` and runs until it is stopped.

#[path = "../tests/support/model.rs"]
mod model;

#[tokio::main]
async fn main() {
    let port_file = std::env::args()
        .nth(1)
        .expect("usage: demo_model <port-file>");
    let address = model::start(model::Script {
        command: String::from("mv ~/Downloads/*.pdf ~/Documents/영수증/"),
    })
    .await;
    std::fs::write(&port_file, address.port().to_string()).expect("port written");
    std::future::pending::<()>().await;
}
