use super::*;

struct Echo;

impl Toolbox for Echo {
    fn specs(&self) -> Vec<ToolSpec> {
        Vec::new()
    }

    fn run(&self, call: ToolCall) -> BoxFuture<'_, Result<String, ToolError>> {
        Box::pin(async move { Ok(format!("here {}", call.name)) })
    }
}

fn call(name: &str) -> ToolCall {
    ToolCall {
        id: ToolCallId::new("1"),
        name: name.to_owned(),
        input: String::from("{}"),
    }
}

#[tokio::test]
async fn alone_every_tool_runs_here() {
    let route = BrowserRoute::new(Arc::new(Echo));

    assert_eq!(
        route.run(call("browser_run")).await.expect("ran"),
        "here browser_run"
    );
    assert_eq!(
        route.run(call("webfetch")).await.expect("ran"),
        "here webfetch"
    );
}

#[tokio::test]
async fn an_answer_from_the_browsers_pc_reaches_the_call_waiting_for_it() {
    let route = BrowserRoute::new(Arc::new(Echo));
    let (tx, rx) = oneshot::channel();
    route
        .waiting
        .lock()
        .expect("unpoisoned")
        .insert(7, (String::from("b"), tx));
    let from = |key: &str, message: Value| Heard {
        key: key.to_owned(),
        device: key.to_owned(),
        message,
    };
    let heard = |message: Value| from("b", message);

    assert!(route.hear(&from(
        "c",
        json!({ "kind": "tool_result", "id": 7, "result": "forged" })
    )));
    assert!(route.hear(&heard(
        json!({ "kind": "tool_result", "id": 7, "error": "no page" })
    )));
    assert!(!route.hear(&heard(json!({ "kind": "marks" }))));

    assert_eq!(rx.await.expect("answered"), Err(String::from("no page")));
}

mod linked {
    use std::path::PathBuf;

    use dotori_connectors::peer::Code;
    use dotori_connectors::peer::Identity;
    use dotori_connectors::peer::Invite;
    use dotori_connectors::peer::hub::Hub;
    use dotori_connectors::peer::hub::HubConfig;
    use dotori_connectors::peer::hub::join;
    use dotori_connectors::peer::lease::Keeper;
    use dotori_connectors::peer::now;
    use dotori_connectors::peer::wire::Hello;

    use super::*;

    struct Named(&'static str);

    impl Toolbox for Named {
        fn specs(&self) -> Vec<ToolSpec> {
            Vec::new()
        }

        fn run(&self, call: ToolCall) -> BoxFuture<'_, Result<String, ToolError>> {
            Box::pin(async move { Ok(format!("{} ran {} with {}", self.0, call.name, call.input)) })
        }
    }

    fn config(name: &str, listen: bool) -> HubConfig {
        let folder: PathBuf =
            std::env::temp_dir().join(format!("dotori-route-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&folder);
        std::fs::create_dir_all(&folder).expect("folder");
        HubConfig {
            identity: Identity::load_or_make(&folder.join("peer.key")).expect("key"),
            device: name.to_owned(),
            name: name.to_owned(),
            address: None,
            listen: listen.then(|| "127.0.0.1:0".parse().expect("address")),
            book: folder.join("peers.json"),
            invite: folder.join("invite.json"),
            seen: folder.join("peers-seen.json"),
        }
    }

    async fn start(
        config: HubConfig,
        wanted: &[&str],
        local: &'static str,
    ) -> (Arc<BrowserRoute>, Linked) {
        start_with(config, wanted, Arc::new(Named(local))).await
    }

    async fn start_with(
        config: HubConfig,
        wanted: &[&str],
        local: Arc<dyn Toolbox>,
    ) -> (Arc<BrowserRoute>, Linked) {
        let (hub, mut heard) = Hub::start(config).await.expect("started");
        let own = hub.own().to_owned();
        let keeper = Keeper::start(Arc::clone(&hub), own.clone(), wanted);
        let linked = Linked { hub, keeper, own };
        let route = BrowserRoute::new(local);
        route.link(linked.clone());
        {
            let linked = linked.clone();
            let route = Arc::clone(&route);
            tokio::spawn(async move {
                while let Some(heard) = heard.recv().await {
                    linked.keeper.hear(&heard);
                    route.hear(&heard);
                }
            });
        }
        (route, linked)
    }

    #[tokio::test]
    async fn a_browser_call_runs_on_the_pc_with_the_browser_and_other_tools_run_here() {
        let b_config = config("b", true);
        let a_config = config("a", false);
        let (b_route, b) = start(b_config.clone(), &[BROWSER_JOB], "B").await;
        let (_, secret) = Invite::make(&b_config.invite, now() + 600).expect("invite");
        let code = Code {
            key: b_config.identity.public,
            secret,
            address: None,
        };
        let hello = Hello {
            device: String::from("a"),
            name: String::from("a"),
            address: None,
        };
        let address = b.hub.listening().expect("listening").to_string();
        join(&a_config.identity, &hello, &code, &address, &a_config.book)
            .await
            .expect("joined");
        let (a_route, a) = start(a_config, &["main"], "A").await;
        let mut holder = a.keeper.watch(BROWSER_JOB).expect("watched");
        let b_key = b.own.clone();
        time::timeout(
            Duration::from_secs(30),
            holder.wait_for(|holder| holder.as_deref() == Some(b_key.as_str())),
        )
        .await
        .expect("a learns b holds the browser in time")
        .expect("watched");

        let mut browser = call("browser_run");
        browser.input = String::from(r#"{"code":"1"}"#);
        assert_eq!(
            a_route.run(browser).await.expect("ran"),
            r#"B ran browser_run with {"code":"1"}"#
        );
        assert_eq!(
            a_route.run(call("webfetch")).await.expect("ran"),
            "A ran webfetch with {}"
        );
        assert_eq!(
            b_route.run(call("browser_read")).await.expect("ran"),
            "B ran browser_read with {}"
        );
    }

    struct Slow(Arc<std::sync::atomic::AtomicBool>);

    impl Toolbox for Slow {
        fn specs(&self) -> Vec<ToolSpec> {
            Vec::new()
        }

        fn run(&self, _call: ToolCall) -> BoxFuture<'_, Result<String, ToolError>> {
            Box::pin(async move {
                time::sleep(Duration::from_secs(3)).await;
                self.0.store(true, std::sync::atomic::Ordering::SeqCst);
                Ok(String::from("acted"))
            })
        }
    }

    #[tokio::test]
    async fn a_call_given_up_on_another_pc_never_finishes_its_work() {
        let b_config = config("slow-b", true);
        let a_config = config("slow-a", false);
        let acted = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let (b_route, b) = start_with(
            b_config.clone(),
            &[BROWSER_JOB],
            Arc::new(Slow(Arc::clone(&acted))),
        )
        .await;
        let (_, secret) = Invite::make(&b_config.invite, now() + 600).expect("invite");
        let code = Code {
            key: b_config.identity.public,
            secret,
            address: None,
        };
        let hello = Hello {
            device: String::from("slow-a"),
            name: String::from("slow-a"),
            address: None,
        };
        let address = b.hub.listening().expect("listening").to_string();
        join(&a_config.identity, &hello, &code, &address, &a_config.book)
            .await
            .expect("joined");
        let (a_route, a) = start(a_config, &["main"], "A").await;
        let mut holder = a.keeper.watch(BROWSER_JOB).expect("watched");
        let b_key = b.own.clone();
        time::timeout(
            Duration::from_secs(30),
            holder.wait_for(|holder| holder.as_deref() == Some(b_key.as_str())),
        )
        .await
        .expect("a learns b holds the browser in time")
        .expect("watched");

        let gave_up = time::timeout(Duration::from_secs(1), a_route.run(call("browser_run"))).await;
        assert!(
            gave_up.is_err(),
            "the call is still running when a gives up"
        );
        time::sleep(Duration::from_secs(4)).await;

        assert!(
            !acted.load(std::sync::atomic::Ordering::SeqCst),
            "b stopped the call"
        );
        assert!(b_route.running.lock().expect("unpoisoned").is_empty());
    }
}

#[tokio::test]
async fn a_call_the_caller_gave_up_on_is_stopped_and_only_by_that_caller() {
    let route = BrowserRoute::new(Arc::new(Echo));
    let task = tokio::spawn(time::sleep(Duration::from_secs(60)));
    route
        .running
        .lock()
        .expect("unpoisoned")
        .insert((String::from("a"), 3), task.abort_handle());
    let from = |key: &str| Heard {
        key: key.to_owned(),
        device: key.to_owned(),
        message: json!({ "kind": "tool_cancel", "id": 3 }),
    };

    assert!(route.hear(&from("c")));
    assert!(!task.is_finished());
    assert!(route.hear(&from("a")));

    assert!(task.await.expect_err("stopped").is_cancelled());
}
