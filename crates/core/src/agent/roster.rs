use std::collections::HashSet;

use crate::id::ChannelId;
use crate::id::ChannelUser;
use crate::message::Role;

/// Who the owner is. Everyone on the app channel is the owner. Elsewhere only the configured users are.
pub(crate) struct Roster {
    app: ChannelId,
    owners: HashSet<ChannelUser>,
}

impl Roster {
    pub(crate) fn new(app: ChannelId, owners: HashSet<ChannelUser>) -> Self {
        Self { app, owners }
    }

    pub(crate) fn app(&self) -> &ChannelId {
        &self.app
    }

    pub(crate) fn role_of(&self, sender: &ChannelUser) -> Role {
        todo!()
    }
}
