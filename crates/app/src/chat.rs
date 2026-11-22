use dotori_connectors::channel::app::ToWindow;
use tokio::io::AsyncBufReadExt;
use tokio::io::BufReader;

use crate::home::Home;
use crate::link;
use crate::link::Update;
use crate::lock::DaemonLock;

/// A window in the terminal: each line typed goes to dotori, and everything the app shows is printed.
pub(crate) async fn run(home: Home) -> Result<(), String> {
    if !DaemonLock::held(&home.lock())? {
        return Err(String::from("dotori is not running"));
    }
    let mut link = link::open(home);
    let printing = async {
        while let Some(update) = link.updates.recv().await {
            match update {
                Update::Connected => {}
                Update::Down(reason) => eprintln!("({reason}; reconnecting)"),
                Update::Event(event) => println!("{}", shown(&event)),
            }
        }
    };
    let typing = async {
        let mut input = BufReader::new(tokio::io::stdin()).lines();
        while let Ok(Some(text)) = input.next_line().await {
            if !text.trim().is_empty() && link.say.send(text).is_err() {
                break;
            }
        }
    };
    tokio::select! {
        () = printing => {}
        () = typing => {}
    }
    Ok(())
}

pub(crate) fn shown(event: &ToWindow) -> String {
    match event {
        ToWindow::Owner { text } => format!("you: {text}"),
        ToWindow::Reply { text } => format!("dotori: {text}"),
        ToWindow::Traffic {
            channel,
            place,
            who,
            text,
        } => format!("[{channel} {place}] {who}: {text}"),
    }
}
