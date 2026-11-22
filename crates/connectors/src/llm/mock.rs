use tokio::io::AsyncReadExt;
use tokio::io::AsyncWriteExt;
use tokio::net::TcpListener;
use tokio::sync::mpsc;

/// One request as the mock server read it.
pub(crate) struct Captured {
    pub(crate) head: String,
    pub(crate) body: String,
}

/// Serves `replies` in order, one per connection, and reports each request it read.
pub(crate) async fn serve(
    replies: Vec<(u16, &'static str)>,
) -> (String, mpsc::UnboundedReceiver<Captured>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bound");
    let address = listener.local_addr().expect("has address");
    let (tx, rx) = mpsc::unbounded_channel();
    tokio::spawn(async move {
        for (status, body) in replies {
            let (mut stream, _) = listener.accept().await.expect("accepted");
            let mut read = Vec::new();
            let mut buffer = [0u8; 4096];
            let (head, length) = loop {
                let n = stream.read(&mut buffer).await.expect("readable");
                read.extend_from_slice(&buffer[..n]);
                if let Some(end) = read.windows(4).position(|w| w == b"\r\n\r\n") {
                    let head = String::from_utf8_lossy(&read[..end]).into_owned();
                    let length = head
                        .lines()
                        .find_map(|line| {
                            let (name, value) = line.split_once(':')?;
                            name.eq_ignore_ascii_case("content-length")
                                .then(|| value.trim().parse::<usize>().ok())?
                        })
                        .unwrap_or(0);
                    read.drain(..end + 4);
                    break (head, length);
                }
            };
            while read.len() < length {
                let n = stream.read(&mut buffer).await.expect("readable");
                read.extend_from_slice(&buffer[..n]);
            }
            let _ = tx.send(Captured {
                head,
                body: String::from_utf8_lossy(&read).into_owned(),
            });
            let response = format!(
                "HTTP/1.1 {status} Mock\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
                body.len()
            );
            stream
                .write_all(response.as_bytes())
                .await
                .expect("writable");
            let _ = stream.shutdown().await;
        }
    });
    (format!("http://{address}"), rx)
}
