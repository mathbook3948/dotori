use tokio::io::AsyncBufReadExt;
use tokio::io::AsyncWriteExt;
use tokio::io::BufReader;

use super::*;

#[tokio::test]
async fn a_pipe_carries_commands_answers_and_events_each_ended_by_a_nul() {
    let (ours, browser) = tokio::io::duplex(1024);
    let (from_browser, to_browser) = tokio::io::split(ours);
    let (browser_reads, mut browser_writes) = tokio::io::split(browser);
    let cdp = Cdp::pipe(from_browser, to_browser);
    let mut events = cdp.events().expect("events");
    let browser = tokio::spawn(async move {
        let mut asked = Vec::new();
        BufReader::new(browser_reads)
            .read_until(0, &mut asked)
            .await
            .expect("read");
        assert_eq!(asked.pop(), Some(0));
        let asked: Value = serde_json::from_slice(&asked).expect("JSON");
        assert_eq!(asked["method"], "Browser.getVersion");
        let event = json!({ "method": "Page.loadEventFired", "params": {} });
        let answer = json!({ "id": asked["id"], "result": { "product": "Chrome" } });
        browser_writes
            .write_all(format!("{event}\0{answer}\0").as_bytes())
            .await
            .expect("written");
        browser_writes
    });

    let answer = cdp
        .send("Browser.getVersion", json!({}), None)
        .await
        .expect("answered");

    assert_eq!(answer["product"], "Chrome");
    let event = events.recv().await.expect("an event");
    assert_eq!(event["method"], "Page.loadEventFired");
    assert!(cdp.open());
    drop(browser.await.expect("browser"));
    time::timeout(ANSWER_WAIT, async {
        while cdp.open() {
            time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("the pipe closing ends the connection");
    let closed = cdp.send("Browser.close", json!({}), None).await;
    assert_eq!(closed, Err(String::from("the browser connection closed")));
}
