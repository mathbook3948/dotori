use std::error::Error;
use std::fmt;
use std::time::SystemTime;

use crate::BoxFuture;
use crate::conversation::Entry;

/// One line of the main session with the time core recorded it.
/// The entry carries the channel, place, sender, and role of what was heard.
#[derive(Clone, Debug)]
pub struct Record {
    pub at: SystemTime,
    pub entry: Entry,
}

#[derive(Clone, Debug)]
pub struct RecordError {
    pub detail: String,
}

impl fmt::Display for RecordError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "record store failed: {}", self.detail)
    }
}

impl Error for RecordError {}

/// The append-only raw record. It is also where the main session resumes after a restart.
pub trait RecordStore: Send + Sync + 'static {
    /// Every record in the order it was appended.
    fn load(&self) -> BoxFuture<'_, Result<Vec<Record>, RecordError>>;

    /// Stores all records in order or none of them, so an error means nothing was stored.
    /// Core appends the batch again until it succeeds, and never edits or removes a record.
    fn append(&self, records: Vec<Record>) -> BoxFuture<'_, Result<(), RecordError>>;
}
