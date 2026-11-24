use std::sync::atomic::AtomicBool;
use std::sync::atomic::Ordering;

use super::*;

fn spec(title: &str) -> SubSessionSpec {
    SubSessionSpec {
        title: title.to_owned(),
        brief: String::from("do it"),
        folder: Some(String::from("/home/me/project")),
    }
}

struct Quick(Arc<AtomicBool>);

impl SubSessionRunner for Quick {
    fn run(&self, _spec: SubSessionSpec) -> BoxFuture<'_, SubSessionOutcome> {
        Box::pin(async move {
            self.0.store(true, Ordering::SeqCst);
            SubSessionOutcome::Finished(String::from("done"))
        })
    }
}

struct Broken;

impl SubSessionRunner for Broken {
    fn run(&self, _spec: SubSessionSpec) -> BoxFuture<'_, SubSessionOutcome> {
        Box::pin(async move { panic!("the runner broke") })
    }
}

async fn until(what: &str, mut done: impl FnMut() -> bool) {
    for _ in 0..100 {
        if done() {
            return;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    panic!("{what} did not happen in time");
}

#[test]
fn a_result_reads_back_the_same_on_the_other_pc() {
    let sent = message(
        &spec("build"),
        &SubSessionOutcome::Failed(String::from("no disk")),
    );

    let (heard_spec, heard_outcome) = heard(&sent).expect("a result");

    assert_eq!(heard_spec, spec("build"));
    assert!(matches!(heard_outcome, SubSessionOutcome::Failed(text) if text == "no disk"));
    assert!(heard(&json!({ "kind": "linked" })).is_none());
}

#[tokio::test]
async fn a_finished_result_stays_counted_as_running_until_someone_takes_it() {
    let handoff = Arc::new(Handoff::default());
    let finished = Arc::new(AtomicBool::new(false));
    let runner = HandedOff {
        handoff: Arc::clone(&handoff),
        inner: Arc::new(Quick(Arc::clone(&finished))),
    };

    let waiting = runner.run(spec("quick"));
    until("the work", || finished.load(Ordering::SeqCst)).await;
    tokio::time::sleep(Duration::from_millis(50)).await;

    assert_eq!(
        handoff.running(),
        [spec("quick")],
        "with no agent to take it, a new agent here must leave it open"
    );
    let ended = tokio::time::timeout(Duration::from_millis(50), waiting).await;
    assert!(ended.is_err(), "the agent hears of it only through Returns");
    handoff.quit();
    handoff.stop_all();
    assert!(handoff.running().is_empty());
}

#[tokio::test]
async fn a_runner_that_breaks_still_ends_with_a_result() {
    let handoff = Arc::new(Handoff::default());
    let runner = HandedOff {
        handoff: Arc::clone(&handoff),
        inner: Arc::new(Broken),
    };

    let _waiting = runner.run(spec("broken"));

    until("the broken run to settle", || !handoff.running().is_empty()).await;
    tokio::time::sleep(Duration::from_millis(50)).await;
    assert_eq!(handoff.running(), [spec("broken")]);
    handoff.quit();
    handoff.stop_all();
}
