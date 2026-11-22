use dotori_connectors::channel::app::FromWindow;
use dotori_connectors::channel::app::Hello;
use dotori_connectors::channel::app::ToWindow;
use tokio::io::AsyncBufReadExt;
use tokio::io::AsyncWriteExt;
use tokio::io::BufReader;
use tokio::net::TcpStream;

use crate::door::Door;
use crate::home::Home;

/// A window in the terminal: each line typed goes to dotori, and everything the app shows is printed.
pub(crate) async fn run(home: Home) -> Result<(), String> {
    let door = Door::read(&home.door())?;
    let stream = TcpStream::connect(("127.0.0.1", door.port))
        .await
        .map_err(|error| format!("dotori is not reachable: {error}"))?;
    let (read, mut write) = stream.into_split();
    let hello =
        serde_json::to_string(&Hello { token: door.token }).map_err(|error| error.to_string())?;
    line(&mut write, &hello).await?;
    let printing = async {
        let mut events = BufReader::new(read).lines();
        while let Ok(Some(text)) = events.next_line().await {
            match serde_json::from_str::<ToWindow>(&text) {
                Ok(ToWindow::Owner { text }) => println!("you: {text}"),
                Ok(ToWindow::Reply { text }) => println!("dotori: {text}"),
                Ok(ToWindow::Traffic {
                    channel,
                    place,
                    who,
                    text,
                }) => println!("[{channel} {place}] {who}: {text}"),
                Err(_) => {}
            }
        }
    };
    let typing = async {
        let mut input = BufReader::new(tokio::io::stdin()).lines();
        while let Ok(Some(text)) = input.next_line().await {
            if text.trim().is_empty() {
                continue;
            }
            let said = serde_json::to_string(&FromWindow::Say { text })
                .map_err(|error| error.to_string())?;
            line(&mut write, &said).await?;
        }
        Ok::<(), String>(())
    };
    tokio::select! {
        () = printing => Err(String::from("dotori closed the connection")),
        typed = typing => typed,
    }
}

async fn line(write: &mut tokio::net::tcp::OwnedWriteHalf, text: &str) -> Result<(), String> {
    let sent = async {
        write.write_all(text.as_bytes()).await?;
        write.write_all(b"\n").await
    };
    sent.await
        .map_err(|error| format!("dotori is not reachable: {error}"))
}
