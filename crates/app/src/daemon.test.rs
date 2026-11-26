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
async fn a_daemon_whose_key_went_missing_waits_and_starts_with_the_settings_saved_meanwhile() {
    let home = home_needing_a_key("waits");
    first_run::write(&home.first_run(), FirstRun::Done).expect("first run");
    let vault = Vault::new(&home.root);
    let mut stop = future::pending::<Result<(), String>>();
    let mut quit = future::pending::<()>();
    let mut waiting = std::pin::pin!(model_when_set_up(
        &home,
        &vault,
        None::<fn()>,
        &mut stop,
        &mut quit,
    ));

    tokio::select! {
        _ = &mut waiting => panic!("returned without a key"),
        () = time::sleep(Duration::from_millis(200)) => {}
    }

    fs::write(home.settings(), NO_KEY_NEEDED).expect("settings");
    let started = time::timeout(Duration::from_secs(10), &mut waiting)
        .await
        .expect("started once the settings need no key")
        .expect("no error");

    assert!(matches!(started, Keyed::Ready(..)));
    let _ = fs::remove_dir_all(&home.root);
}

fn home_set_up_to(name: &str, settings: &str, first_run: FirstRun) -> Home {
    let root = std::env::temp_dir().join(format!("dotori-{name}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(&root).expect("a home");
    fs::write(root.join("settings.toml"), settings).expect("settings");
    let home = Home { root };
    first_run::write(&home.first_run(), first_run).expect("first run");
    home
}

#[tokio::test]
async fn the_daemon_does_not_start_after_the_key_step_alone() {
    let home = home_set_up_to("key-alone", NO_KEY_NEEDED, FirstRun::At(Step::Look));
    let vault = Vault::new(&home.root);
    let asked = AtomicBool::new(false);
    let mut stop = future::pending::<Result<(), String>>();
    let mut quit = future::pending::<()>();
    let mut waiting = std::pin::pin!(model_when_set_up(
        &home,
        &vault,
        Some(|| asked.store(true, Ordering::SeqCst)),
        &mut stop,
        &mut quit,
    ));

    tokio::select! {
        _ = &mut waiting => panic!("started before first run was done"),
        () = time::sleep(Duration::from_millis(200)) => {}
    }

    assert!(
        asked.load(Ordering::SeqCst),
        "a window is asked to walk the owner through"
    );
    let _ = fs::remove_dir_all(&home.root);
}

#[tokio::test]
async fn the_daemon_starts_once_first_run_is_done_with_the_name_and_channels_chosen() {
    let home = home_set_up_to("walked", NO_KEY_NEEDED, FirstRun::At(Step::Look));
    let vault = Vault::new(&home.root);
    let mut stop = future::pending::<Result<(), String>>();
    let mut quit = future::pending::<()>();
    let mut waiting = std::pin::pin!(model_when_set_up(
        &home,
        &vault,
        Some(|| {}),
        &mut stop,
        &mut quit,
    ));
    tokio::select! {
        _ = &mut waiting => panic!("started before first run was done"),
        () = time::sleep(Duration::from_millis(200)) => {}
    }

    fs::write(
        home.settings(),
        format!("{NO_KEY_NEEDED}[telegram]\nowner = 12345\n\n[assistant]\nname = \"별\"\n"),
    )
    .expect("settings");
    first_run::write(&home.first_run(), FirstRun::Done).expect("done");
    let started = time::timeout(Duration::from_secs(10), &mut waiting)
        .await
        .expect("started once first run was done")
        .expect("no error");

    let Keyed::Ready(settings, _) = started else {
        panic!("stopped instead of starting");
    };
    assert_eq!(
        settings
            .assistant
            .and_then(|assistant| assistant.name)
            .as_deref(),
        Some("별")
    );
    assert_eq!(
        settings.telegram.and_then(|telegram| telegram.owner),
        Some(12345)
    );
    let _ = fs::remove_dir_all(&home.root);
}

#[tokio::test]
async fn an_install_from_before_first_run_with_a_key_starts_at_once() {
    let home = home_set_up_to("upgraded", NO_KEY_NEEDED, FirstRun::Unbegun);
    let vault = Vault::new(&home.root);
    let asked = AtomicBool::new(false);
    let mut stop = future::pending::<Result<(), String>>();
    let mut quit = future::pending::<()>();

    let started = time::timeout(
        Duration::from_secs(1),
        model_when_set_up(
            &home,
            &vault,
            Some(|| asked.store(true, Ordering::SeqCst)),
            &mut stop,
            &mut quit,
        ),
    )
    .await
    .expect("started at once")
    .expect("no error");

    assert!(matches!(started, Keyed::Ready(..)));
    assert!(
        !asked.load(Ordering::SeqCst),
        "no window walks an install already set up"
    );
    assert_eq!(first_run::read(&home.first_run()), FirstRun::Done);
    let _ = fs::remove_dir_all(&home.root);
}

#[tokio::test]
async fn a_first_run_without_a_key_is_begun_at_the_greeting() {
    let home = home_needing_a_key("begun");
    let vault = Vault::new(&home.root);
    let mut stop = future::pending::<Result<(), String>>();
    let mut quit = future::pending::<()>();
    let waiting = model_when_set_up(&home, &vault, Some(|| {}), &mut stop, &mut quit);

    let _ = time::timeout(Duration::from_millis(200), waiting).await;

    assert_eq!(
        first_run::read(&home.first_run()),
        FirstRun::At(Step::Welcome)
    );
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
            model_when_set_up(&home, &vault, None::<fn()>, &mut stop, &mut quit)
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
        model_when_set_up(&home, &vault, None::<fn()>, &mut stop, &mut quit),
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

    let ended = model_when_set_up(&home, &vault, None::<fn()>, &mut stop, &mut quit).await;

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
