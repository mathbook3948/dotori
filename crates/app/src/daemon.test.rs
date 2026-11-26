use std::future;
use std::sync::atomic::AtomicBool;
use std::sync::atomic::Ordering;

use tokio::sync::oneshot;

use super::*;

const NO_KEY_NEEDED: &str = "[model]\nprovider = \"openai-compatible\"\nbase_url = \"http://127.0.0.1:1/v1\"\nmodel = \"m\"\n";

fn home_needing_a_key(name: &str) -> Home {
    let root = std::env::temp_dir().join(format!("dotori-{name}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(&root).expect("a home");
    fs::write(
        root.join("settings.toml"),
        format!(
            "[model]\nprovider = \"anthropic\"\napi_key_env = \"DOTORI_TEST_ABSENT_KEY_{}\"\n",
            std::process::id()
        ),
    )
    .expect("settings");
    Home { root }
}

#[tokio::test]
async fn the_daemon_waits_for_a_key_and_starts_with_the_settings_saved_meanwhile() {
    let home = home_needing_a_key("waits");
    let vault = Vault::new(&home.root);
    let asked = AtomicBool::new(false);
    let mut stop = future::pending::<Result<(), String>>();
    let mut quit = future::pending::<()>();
    let mut waiting = std::pin::pin!(model_when_keyed(
        &home,
        &vault,
        || asked.store(true, Ordering::SeqCst),
        &mut stop,
        &mut quit,
    ));

    tokio::select! {
        _ = &mut waiting => panic!("returned without a key"),
        () = time::sleep(Duration::from_millis(200)) => {}
    }
    assert!(asked.load(Ordering::SeqCst));

    fs::write(home.settings(), NO_KEY_NEEDED).expect("settings");
    let started = time::timeout(Duration::from_secs(10), &mut waiting)
        .await
        .expect("started once the settings need no key")
        .expect("no error");

    assert!(matches!(started, Keyed::Ready(..)));
    let _ = fs::remove_dir_all(&home.root);
}

#[tokio::test]
async fn quitting_ends_the_wait_for_a_key() {
    let home = home_needing_a_key("quits");
    let vault = Vault::new(&home.root);
    let (quit_now, quitting) = oneshot::channel::<()>();
    let mut stop = future::pending::<Result<(), String>>();
    let mut quit = Box::pin(async {
        let _ = quitting.await;
    });

    let (ended, ()) = tokio::join!(
        time::timeout(
            Duration::from_secs(10),
            model_when_keyed(&home, &vault, || {}, &mut stop, &mut quit)
        ),
        async {
            time::sleep(Duration::from_millis(200)).await;
            let _ = quit_now.send(());
        }
    );

    assert!(matches!(ended, Ok(Ok(Keyed::Stopped(Ok(()))))));
    let _ = fs::remove_dir_all(&home.root);
}

#[tokio::test]
async fn a_stop_signal_ends_the_wait_with_what_it_reports() {
    let home = home_needing_a_key("stops");
    let vault = Vault::new(&home.root);
    let mut stop = Box::pin(async {
        time::sleep(Duration::from_millis(200)).await;
        Err(String::from("no signal handler"))
    });
    let mut quit = future::pending::<()>();

    let ended = time::timeout(
        Duration::from_secs(10),
        model_when_keyed(&home, &vault, || {}, &mut stop, &mut quit),
    )
    .await
    .expect("ended");

    assert!(matches!(ended, Ok(Keyed::Stopped(Err(message))) if message == "no signal handler"));
    let _ = fs::remove_dir_all(&home.root);
}

#[tokio::test]
async fn settings_that_cannot_be_read_end_the_wait_with_an_error() {
    let home = home_needing_a_key("broken");
    fs::write(home.settings(), "[model\n").expect("settings");
    let vault = Vault::new(&home.root);
    let mut stop = future::pending::<Result<(), String>>();
    let mut quit = future::pending::<()>();

    let ended = model_when_keyed(&home, &vault, || {}, &mut stop, &mut quit).await;

    assert!(ended.is_err());
    let _ = fs::remove_dir_all(&home.root);
}

#[cfg(target_os = "linux")]
#[test]
fn the_process_is_closed_to_other_programs_of_its_user() {
    use std::os::unix::fs::MetadataExt;

    use super::keep_to_itself;

    keep_to_itself();

    assert_eq!(
        rustix::process::dumpable_behavior().expect("read"),
        rustix::process::DumpableBehavior::NotDumpable
    );
    let environment = std::fs::metadata("/proc/self/environ").expect("there");
    assert_eq!(environment.uid(), 0, "only root may read its environment");
}
