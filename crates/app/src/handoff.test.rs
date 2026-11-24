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

struct Gated {
    gate: tokio::sync::Mutex<Option<oneshot::Receiver<()>>>,
    finished: Arc<AtomicBool>,
}

impl SubSessionRunner for Gated {
    fn run(&self, _spec: SubSessionSpec) -> BoxFuture<'_, SubSessionOutcome> {
        Box::pin(async move {
            if let Some(gate) = self.gate.lock().await.take() {
                let _ = gate.await;
            }
            self.finished.store(true, Ordering::SeqCst);
            SubSessionOutcome::Finished(String::from("done"))
        })
    }
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
async fn work_goes_on_after_the_agent_lets_go_and_stops_only_when_dotori_does() {
    let handoff = Arc::new(Handoff::default());
    let (open, gate) = oneshot::channel();
    let finished = Arc::new(AtomicBool::new(false));
    let runner = HandedOff {
        handoff: Arc::clone(&handoff),
        inner: Arc::new(Gated {
            gate: tokio::sync::Mutex::new(Some(gate)),
            finished: Arc::clone(&finished),
        }),
    };

    let waiting = runner.run(spec("long"));
    tokio::time::sleep(Duration::from_millis(50)).await;
    assert_eq!(handoff.running(), [spec("long")]);
    drop(waiting);
    let _ = open.send(());
    for _ in 0..50 {
        if finished.load(Ordering::SeqCst) {
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    assert!(
        finished.load(Ordering::SeqCst),
        "the work outlived the agent's wait"
    );
    assert!(handoff.running().is_empty());

    let (_open, gate) = oneshot::channel::<()>();
    let stopped = Arc::new(AtomicBool::new(false));
    let runner = HandedOff {
        handoff: Arc::clone(&handoff),
        inner: Arc::new(Gated {
            gate: tokio::sync::Mutex::new(Some(gate)),
            finished: Arc::clone(&stopped),
        }),
    };
    let waiting = runner.run(spec("cut"));
    tokio::time::sleep(Duration::from_millis(50)).await;
    handoff.stop_all();
    let outcome = waiting.await;
    assert!(matches!(outcome, SubSessionOutcome::Failed(_)));
    assert!(!stopped.load(Ordering::SeqCst));
}

#[tokio::test]
async fn a_result_the_agent_is_there_for_comes_back_as_usual() {
    let handoff = Arc::new(Handoff::default());
    let runner = HandedOff {
        handoff: Arc::clone(&handoff),
        inner: Arc::new(Gated {
            gate: tokio::sync::Mutex::new(None),
            finished: Arc::new(AtomicBool::new(false)),
        }),
    };

    let outcome = runner.run(spec("quick")).await;

    assert!(matches!(outcome, SubSessionOutcome::Finished(text) if text == "done"));
}
