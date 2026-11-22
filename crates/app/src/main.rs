mod chat;
mod daemon;
mod door;
mod home;
mod idle;
mod settings;

use std::env;
use std::process::ExitCode;

use crate::home::Home;

const USAGE: &str = "usage: dotori [daemon | chat]";

#[tokio::main]
async fn main() -> ExitCode {
    let command = env::args().nth(1);
    let home = match Home::find() {
        Ok(home) => home,
        Err(error) => return fail(&error),
    };
    let result = match command.as_deref() {
        None | Some("daemon") => daemon::run(home).await,
        Some("chat") => chat::run(home).await,
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
