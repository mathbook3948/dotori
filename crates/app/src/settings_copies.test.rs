use super::*;
use crate::settings::PeerSettings;

fn pc(name: &str) -> SettingsCopies {
    let folder =
        std::env::temp_dir().join(format!("dotori-settings-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&folder);
    std::fs::create_dir_all(&folder).expect("folder");
    SettingsCopies::new(folder.join("settings.toml"), folder.join("settings.stamp"))
}

fn save(copies: &SettingsCopies, settings: &Settings) {
    settings.save(&copies.settings).expect("saved");
}

fn slack(owner: &str) -> SlackSettings {
    SlackSettings {
        owner: Some(owner.to_owned()),
        ..SlackSettings::default()
    }
}

#[test]
fn a_new_pc_takes_a_configured_pcs_settings_and_one_part_changed_leaves_the_others() {
    let configured = pc("configured");
    let fresh = pc("fresh");
    save(
        &configured,
        &Settings {
            slack: Some(slack("U1")),
            memory: Some(MemorySettings::default()),
            peers: Some(PeerSettings {
                name: Some(String::from("데스크톱")),
                ..PeerSettings::default()
            }),
            ..Settings::default()
        },
    );
    save(&fresh, &Settings::default());
    let set = configured.current().expect("read");
    let empty = fresh.current().expect("read");
    assert_eq!((set.times.slack, empty.times.slack), (1, 0));

    assert!(
        !configured
            .take(empty.times, empty.shared.clone(), false)
            .expect("checked")
    );
    assert!(
        fresh
            .take(set.times, set.shared.clone(), false)
            .expect("taken")
    );
    assert_eq!(
        Settings::load(&fresh.settings).expect("loaded").slack,
        Some(slack("U1"))
    );

    let mut changed = Settings::load(&fresh.settings).expect("loaded");
    changed.model = ModelSettings::OpenaiCompatible {
        base_url: String::from("http://x"),
        model: String::from("m"),
        api_key_env: None,
        max_tokens: None,
    };
    save(&fresh, &changed);
    let edited = fresh.current().expect("read");
    assert!(edited.times.model > 1);
    assert_eq!(
        edited.times.slack, set.times.slack,
        "taking is not a change here"
    );

    assert!(
        configured
            .take(edited.times, edited.shared, false)
            .expect("taken")
    );
    let now = Settings::load(&configured.settings).expect("loaded");
    assert_eq!(now.model, changed.model);
    assert_eq!(now.slack, Some(slack("U1")));
    assert!(now.memory.is_some());
    assert_eq!(
        now.peers.and_then(|peers| peers.name).as_deref(),
        Some("데스크톱")
    );
}

#[test]
fn a_change_here_always_comes_after_the_time_it_replaces() {
    assert_eq!(next(100, true, 50), 101);
    assert_eq!(next(100, true, 500), 500);
    assert_eq!(next(100, false, 500), 100);
}

#[test]
fn a_pc_joining_in_first_run_takes_settings_as_old_as_its_own_and_a_set_up_pc_does_not() {
    let model = |url: &str| ModelSettings::OpenaiCompatible {
        base_url: url.to_owned(),
        model: String::from("m"),
        api_key_env: None,
        max_tokens: None,
    };
    let running = pc("running");
    let joining = pc("joining");
    let set_up = pc("set-up");
    save(
        &running,
        &Settings {
            model: model("http://running"),
            ..Settings::default()
        },
    );
    for copies in [&joining, &set_up] {
        save(
            copies,
            &Settings {
                model: model("http://here"),
                ..Settings::default()
            },
        );
    }
    let theirs = running.current().expect("read");
    assert_eq!(
        (
            theirs.times.model,
            joining.current().expect("read").times.model
        ),
        (1, 1),
        "both were set before either kept a time"
    );

    assert!(
        !set_up
            .take(theirs.times, theirs.shared.clone(), false)
            .expect("checked")
    );
    assert!(
        joining
            .take(theirs.times, theirs.shared, true)
            .expect("taken")
    );

    assert_eq!(
        Settings::load(&joining.settings).expect("loaded").model,
        model("http://running")
    );
    assert_eq!(
        Settings::load(&set_up.settings).expect("loaded").model,
        model("http://here")
    );
}
