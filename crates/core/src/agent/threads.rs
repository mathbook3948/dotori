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
        if let Some(thread) = self.refs.get(&address) {
            return *thread;
        }
        let index = u32::try_from(self.addresses.len()).expect("fewer than 2^32 threads");
        let thread = ThreadRef::new(index);
        self.addresses.push(address.clone());
        self.refs.insert(address, thread);
        thread
    }

    pub(crate) fn resolve(&self, thread: ThreadRef) -> Option<&Address> {
        self.addresses.get(thread.index())
    }
}
