use std::collections::HashMap;

use crate::conversation::ThreadRef;
use crate::message::Address;

/// The two-way map between connector addresses and the handles the model sees.
#[derive(Debug, Default)]
pub(crate) struct Threads {
    addresses: Vec<Address>,
    refs: HashMap<Address, ThreadRef>,
}

impl Threads {
    pub(crate) fn intern(&mut self, address: Address) -> ThreadRef {
        todo!()
    }

    pub(crate) fn resolve(&self, thread: ThreadRef) -> Option<&Address> {
        todo!()
    }
}
