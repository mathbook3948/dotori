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

/// A loaded record and whether this PC made it. Work started on another PC is that PC's to
/// close.
#[derive(Clone, Debug)]
pub struct Marked {
    pub record: Record,
    pub here: bool,
}

/// The append-only raw record. It is also where the main session resumes after a restart.
pub trait RecordStore: Send + Sync + 'static {
    /// Every record in the order it was appended.
    fn load(&self) -> BoxFuture<'_, Result<Vec<Record>, RecordError>>;

    /// Every record in order, each marked with whether this PC made it. A store that only one PC
    /// writes made them all.
    fn load_marked(&self) -> BoxFuture<'_, Result<Vec<Marked>, RecordError>> {
        Box::pin(async move {
            Ok(self
                .load()
                .await?
                .into_iter()
                .map(|record| Marked { record, here: true })
                .collect())
        })
    }

    /// Stores all records in order or none of them, so an error means nothing was stored.
    /// Core appends the batch again until it succeeds, and never edits or removes a record.
    fn append(&self, records: Vec<Record>) -> BoxFuture<'_, Result<(), RecordError>>;
}
