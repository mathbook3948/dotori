mod chat;
mod daemon;
mod door;
mod home;
mod idle;
mod link;
mod lock;
mod settings;
#[cfg(feature = "window")]
mod tray;
#[cfg(feature = "window")]
mod window;

use std::env;
use std::process::ExitCode;

use crate::home::Home;

#[cfg(feature = "window")]
const USAGE: &str = "usage: dotori [daemon | chat | window | settings]";
#[cfg(not(feature = "window"))]
const USAGE: &str = "usage: dotori [daemon | chat]";

#[tokio::main]
async fn main() -> ExitCode {
    let command = env::args().nth(1);
    let home = match Home::find() {
        Ok(home) => home,
        Err(error) => return fail(&error),
    };
    let result = match command.as_deref() {
        #[cfg(feature = "window")]
        None | Some("daemon") if tray::desktop() => tray::run(home).await,
        None | Some("daemon") => daemon::run(home).await,
        Some("chat") => chat::run(home).await,
        #[cfg(feature = "window")]
        Some("window") => window::run(home, window::Which::Chat),
        #[cfg(feature = "window")]
        Some("settings") => window::run(home, window::Which::Settings),
        Some(_) => return fail(USAGE),
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => fail(&error),
    }
}

fn fail(error: &str) -> ExitCode {
    eprintln!("dotori: {error}");
    ExitCode::FAILURE
}
