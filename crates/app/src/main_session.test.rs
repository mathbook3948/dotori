use std::sync::Mutex as StdMutex;

use dotori_core::id::ExternalMessageId;
use dotori_core::id::ExternalUserId;
use dotori_core::message::Audience;
use dotori_core::message::Content;

use super::*;

#[derive(Default)]
struct Kept(StdMutex<Vec<String>>);

impl Inbox for Kept {
    fn push(&self, incoming: Incoming) -> BoxFuture<'_, Result<(), InboxClosedError>> {
        self.0
            .lock()
            .expect("unpoisoned")
            .push(incoming.content.as_str().to_owned());
        Box::pin(async { Ok(()) })
    }
}

fn said(text: &str) -> Incoming {
    Incoming {
        id: ExternalMessageId::new("1"),
        place: Place::new("main"),
        sender_id: ExternalUserId::new("owner"),
        sender_name: String::from("owner"),
        content: Content::parse(text.to_owned()).expect("content"),
        audience: Audience::Owner,
    }
}

#[tokio::test]
async fn words_reach_the_agent_while_it_runs_here_and_wait_otherwise() {
    let route = Route::new(None);
    let agent = Arc::new(Kept::default());

    route.push(said("먼저")).await.expect("kept");
    route.to_agent(Arc::clone(&agent) as Arc<dyn Inbox>);
    route.push(said("다음")).await.expect("pushed");
    route.elsewhere();
    route.push(said("나중")).await.expect("kept");

    assert_eq!(*agent.0.lock().expect("unpoisoned"), ["다음"]);
    let waiting: Vec<String> = route
        .waiting
        .lock()
        .expect("unpoisoned")
        .iter()
        .map(|(text, _)| text.clone())
        .collect();
    assert_eq!(waiting, ["먼저", "나중"]);
}

#[tokio::test]
async fn alone_this_pc_always_runs_the_main_session() {
    gained(None).await;
    assert!(still_main(None).is_ok());
    let lost = tokio::time::timeout(std::time::Duration::from_millis(50), lost(None)).await;
    assert!(lost.is_err());
}
