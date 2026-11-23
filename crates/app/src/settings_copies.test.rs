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

#[test]
fn settings_changed_later_are_taken_but_what_belongs_to_one_pc_stays() {
    let here = pc("here");
    let there = pc("there");
    save(
        &here,
        &Settings {
            peers: Some(PeerSettings {
                name: Some(String::from("데스크톱")),
                ..PeerSettings::default()
            }),
            ..Settings::default()
        },
    );
    let mine = here.current().expect("read");
    assert_eq!(
        mine.changed_ms, 0,
        "settings no one changed since they were shared are old"
    );

    let mut changed = Settings {
        memory: Some(MemorySettings::default()),
        ..Settings::default()
    };
    save(&there, &changed);
    there.current().expect("read");
    std::thread::sleep(Duration::from_millis(5));
    changed.slack = Some(SlackSettings::default());
    save(&there, &changed);
    let theirs = there.current().expect("read");
    assert!(theirs.changed_ms > 0);

    assert!(
        here.take(theirs.changed_ms, theirs.shared.clone())
            .expect("taken")
    );
    let now = Settings::load(&here.settings).expect("loaded");
    assert_eq!(now.slack, changed.slack);
    assert_eq!(now.memory, changed.memory);
    assert_eq!(
        now.peers.and_then(|peers| peers.name).as_deref(),
        Some("데스크톱")
    );
    assert!(
        !here
            .take(theirs.changed_ms, theirs.shared.clone())
            .expect("checked")
    );
    assert!(!there.take(mine.changed_ms, mine.shared).expect("older"));
    assert_eq!(
        here.current().expect("read").changed_ms,
        theirs.changed_ms,
        "taking is not a new change"
    );
}
