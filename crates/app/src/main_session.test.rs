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
        at: None,
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
        .map(|(text, _, _)| text.clone())
        .collect();
    assert_eq!(waiting, ["먼저", "나중"]);
}

#[tokio::test]
async fn words_that_wait_keep_the_time_they_were_said() {
    let route = Route::new(None);
    let at = UNIX_EPOCH + Duration::from_secs(1_700_000_000);
    let mut early = said("먼저");
    early.at = Some(at);

    route.push(early).await.expect("kept");

    let kept: Vec<Option<SystemTime>> = route
        .waiting
        .lock()
        .expect("unpoisoned")
        .iter()
        .map(|(_, _, at)| *at)
        .collect();
    assert_eq!(kept, [Some(at)]);
}

#[tokio::test]
async fn alone_this_pc_always_runs_the_main_session() {
    gained(None).await;
    assert!(still_main(None).is_ok());
    let lost = tokio::time::timeout(std::time::Duration::from_millis(50), lost(None)).await;
    assert!(lost.is_err());
}

struct Echo;

impl Toolbox for Echo {
    fn specs(&self) -> Vec<ToolSpec> {
        Vec::new()
    }

    fn run(&self, call: ToolCall) -> BoxFuture<'_, Result<String, ToolError>> {
        Box::pin(async move { Ok(call.name) })
    }
}

fn call() -> ToolCall {
    ToolCall {
        id: dotori_core::tool::ToolCallId::new("c1"),
        name: String::from("mail__send"),
        input: String::from("{}"),
    }
}

#[tokio::test]
async fn a_pc_that_does_not_run_the_main_session_calls_no_tool() {
    let folder = std::env::temp_dir().join(format!("dotori-fence-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&folder);
    std::fs::create_dir_all(&folder).expect("folder");
    let config = dotori_connectors::peer::hub::HubConfig {
        identity: dotori_connectors::peer::Identity::load_or_make(&folder.join("peer.key"))
            .expect("key"),
        device: String::from("here"),
        name: String::from("here"),
        address: None,
        listen: None,
        book: folder.join("peers.json"),
        invite: folder.join("invite.json"),
        seen: folder.join("peers-seen.json"),
    };
    let (hub, _heard) = Hub::start(config).await.expect("started");
    let linked = Linked {
        keeper: Keeper::start(Arc::clone(&hub), String::from("here"), &[]),
        hub,
        own: String::from("here"),
    };

    let alone = FencedTools {
        tools: Arc::new(Echo),
        linked: None,
    };
    assert_eq!(alone.run(call()).await.expect("ran"), "mail__send");
    let fenced = FencedTools {
        tools: Arc::new(Echo),
        linked: Some(linked),
    };
    let refused = fenced.run(call()).await.expect_err("refused");
    assert!(refused.detail.contains("another PC"), "{}", refused.detail);

    let _ = std::fs::remove_dir_all(folder);
}
