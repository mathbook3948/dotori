use std::num::NonZeroUsize;
use std::sync::Arc;
use std::time::Duration;

use super::*;
use crate::channel::Channel;
use crate::id::ExternalUserId;
use crate::message::Role;
use crate::runtime::fakes::FakeChannel;
use crate::runtime::fakes::channel_id;
use crate::runtime::fakes::ports;

fn config(owners: Vec<ChannelUser>) -> AgentConfig {
    AgentConfig {
        owners,
        queue_capacity: NonZeroUsize::new(8).expect("non-zero"),
        seen_capacity: NonZeroUsize::new(8).expect("non-zero"),
        tool_timeout: Duration::from_secs(1),
    }
}

fn user(channel: &str, user: &str) -> ChannelUser {
    ChannelUser {
        channel: channel_id(channel),
        user: ExternalUserId::new(user),
    }
}

fn channel(id: &str) -> Arc<dyn Channel> {
    FakeChannel::new(id).0
}

#[test]
fn a_channel_reusing_the_app_id_is_rejected() {
    let ports = ports(FakeChannel::new("app").0, vec![channel("app")]);

    let result = validate(&config(Vec::new()), &ports);

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

    let result = validate(&config(Vec::new()), &ports);

    assert_eq!(
        result.err(),
        Some(ConfigError::DuplicateChannel(channel_id("slack")))
    );
}

#[test]
fn an_owner_on_an_unconnected_channel_is_rejected() {
    let ports = ports(FakeChannel::new("app").0, vec![channel("slack")]);

    let result = validate(&config(vec![user("telegram", "u1")]), &ports);

    assert_eq!(
        result.err(),
        Some(ConfigError::UnknownOwnerChannel(channel_id("telegram")))
    );
}

#[test]
fn valid_wiring_makes_configured_users_and_the_app_owners() {
    let ports = ports(FakeChannel::new("app").0, vec![channel("slack")]);

    let roster = validate(&config(vec![user("slack", "me")]), &ports).expect("valid");

    assert_eq!(roster.role_of(&user("slack", "me")), Role::Owner);
    assert_eq!(roster.role_of(&user("slack", "guest")), Role::Other);
    assert_eq!(roster.role_of(&user("app", "anyone")), Role::Owner);
}
