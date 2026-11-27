#![forbid(unsafe_code)]

mod accounts;
mod asked;
mod browser_route;
mod confirm;
mod daemon;
mod device;
mod door;
mod first_run;
mod forget;
mod greeting;
mod handoff;
mod history;
mod home;
mod icons;
mod idle;
mod language;
mod link;
mod live;
mod lock;
mod logins;
mod main_session;
mod notes;
mod peers;
mod replace;
mod room_cards;
mod routines;
mod sandbox;
mod screen_link;
mod search_eval;
mod settings;
mod settings_copies;
mod tools;
mod tray;
mod vault;
mod vector_copies;
mod versioned;
mod window;
mod words;

use std::env;
use std::process::ExitCode;

use crate::home::Home;

const USAGE: &str =
    "usage: dotori [window | settings | onboarding | search-eval <file> | sandbox install]";

#[tokio::main]
async fn main() -> ExitCode {
    let command = env::args().nth(1);
    if command.as_deref() == Some("sandbox") {
        return sandbox::run(env::args().nth(2).as_deref());
    }
    let home = match Home::find() {
        Ok(home) => home,
        Err(error) => return fail(&error),
    };
    let result = match command.as_deref() {
        #[cfg(debug_assertions)]
        None if env::var_os("DOTORI_TEST_WITHOUT_TRAY").is_some() => daemon::run(home).await,
        None => tray::run(home).await,
        Some("mcp") => tools::serve(home).await,
        Some("window") => {
            daemon::keep_to_itself();
            window::run(home, window::Which::Chat)
        }
        Some("settings") => {
            daemon::keep_to_itself();
            window::run(home, window::Which::Settings)
        }
        Some("onboarding") => {
            daemon::keep_to_itself();
            window::run(home, window::Which::Onboarding)
        }
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
