use std::sync::Arc;

use super::*;
use crate::channel::Channel;
use crate::message::Place;
use crate::runtime::fakes::FakeChannel;
use crate::runtime::fakes::channel_id;
use crate::runtime::fakes::ports;

fn channel(id: &str) -> Arc<dyn Channel> {
    FakeChannel::new(id).0
}

#[test]
fn a_channel_reusing_the_app_id_is_rejected() {
    let ports = ports(FakeChannel::new("app").0, vec![channel("app")]);

    let result = validate(&ports);

    assert_eq!(
        result.err(),
        Some(ConfigError::DuplicateChannel(channel_id("app")))
    );
}

#[test]
fn two_channels_with_one_id_are_rejected() {
    let ports = ports(
        FakeChannel::new("app").0,
        vec![channel("slack"), channel("slack")],
    );

    let result = validate(&ports);

    assert_eq!(
        result.err(),
        Some(ConfigError::DuplicateChannel(channel_id("slack")))
    );
}

#[test]
fn valid_wiring_names_the_app_window() {
    let ports = ports(FakeChannel::new("app").0, vec![channel("slack")]);

    let wiring = validate(&ports).expect("valid");

    assert_eq!(wiring.app, channel_id("app"));
    assert_eq!(
        wiring.private.get(&channel_id("slack")),
        Some(&Place::new("private"))
    );
    assert_eq!(
        wiring.private.get(&channel_id("app")),
        Some(&Place::new("private"))
    );
}
