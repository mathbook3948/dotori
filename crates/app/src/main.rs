#![forbid(unsafe_code)]

mod browser_route;
mod chat;
mod confirm;
mod daemon;
mod device;
mod door;
mod forget;
mod handoff;
mod history;
mod home;
mod idle;
mod link;
mod lock;
mod logins;
mod main_session;
mod notes;
mod peers;
mod replace;
mod routines;
mod screen_link;
mod search_eval;
mod settings;
mod settings_copies;
mod tools;
mod tray;
mod vector_copies;
mod versioned;
mod window;

use std::env;
use std::process::ExitCode;

use crate::home::Home;

const USAGE: &str = "usage: dotori [daemon | chat | window | settings | search-eval <file>]";

#[tokio::main]
async fn main() -> ExitCode {
    let command = env::args().nth(1);
    let home = match Home::find() {
        Ok(home) => home,
        Err(error) => return fail(&error),
    };
    let result = match command.as_deref() {
        None if home::desktop() => tray::run(home).await,
        None | Some("daemon") => daemon::run(home).await,
        Some("chat") => chat::run(home).await,
        Some("mcp") => tools::serve(home).await,
        Some("window") => window::run(home, window::Which::Chat),
        Some("settings") => window::run(home, window::Which::Settings),
        Some("search-eval") => search_eval::run(home, env::args().nth(2)).await,
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
