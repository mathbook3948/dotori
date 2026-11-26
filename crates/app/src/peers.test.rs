use super::*;
use crate::settings::PcSettings;
use crate::settings::PeerSettings;

fn named(pc: Option<&str>, peers: Option<&str>) -> Settings {
    Settings {
        pc: pc.map(|name| PcSettings {
            name: Some(name.to_owned()),
        }),
        peers: peers.map(|name| PeerSettings {
            name: Some(name.to_owned()),
            ..PeerSettings::default()
        }),
        ..Settings::default()
    }
}

#[test]
fn the_name_the_owner_gave_wins_over_the_one_kept_in_peers() {
    assert_eq!(name(&named(Some("서재"), Some("desk"))), "서재");
    assert_eq!(name(&named(None, Some("desk"))), "desk");
    assert_eq!(name(&named(Some("  "), Some("desk"))), "desk");
}

#[test]
fn a_pc_without_a_name_goes_by_the_computer_name() {
    let unnamed = name(&Settings::default());

    assert!(!unnamed.trim().is_empty());
    assert_eq!(
        unnamed,
        computer_name().unwrap_or_else(|| String::from("이 PC"))
    );
}
