//! Copies the raw record between linked PCs, record by record. When two PCs link they say how
//! far they hold each PC's records, and each sends the other what it lacks. After that every new
//! record, made here or copied in from a third PC, goes out at once.

use std::collections::BTreeMap;
use std::collections::HashMap;
use std::sync::Arc;
use std::sync::Mutex;

use dotori_core::BoxFuture;
use dotori_core::record::Record;
use dotori_core::record::RecordError;
use dotori_core::record::RecordStore;
use serde_json::Value;
use serde_json::json;
use tokio::sync::Notify;

use crate::peer::hub::Heard;
use crate::peer::hub::Hub;
use crate::record::sqlite::Copied;
use crate::record::sqlite::SqliteStore;

const BATCH_COUNT: usize = 500;
const BATCH_BYTES: usize = 4 * 1024 * 1024;

type Marks = BTreeMap<String, String>;

/// Keeps this PC's raw record and the other PCs' copies the same.
#[derive(Debug)]
pub struct Copies {
    hub: Arc<Hub>,
    store: SqliteStore,
    held: Mutex<HashMap<String, Marks>>,
    wake: Notify,
}

impl Copies {
    /// Starts sending records out. Pass every message the hub hears to `hear`.
    pub fn start(hub: Arc<Hub>, store: SqliteStore) -> Arc<Self> {
        let copies = Arc::new(Self {
            hub,
            store,
            held: Mutex::new(HashMap::new()),
            wake: Notify::new(),
        });
        tokio::spawn(Arc::clone(&copies).send_all());
        copies
    }

    /// Takes a message about the record from another PC. Says whether it was one.
    pub async fn hear(&self, heard: &Heard) -> bool {
        match heard.message.get("kind").and_then(Value::as_str) {
            Some("linked") => {
                match self.store.marks().await {
                    Ok(marks) => {
                        self.hub
                            .send(&heard.key, json!({ "kind": "marks", "marks": marks }));
                    }
                    Err(error) => eprintln!("dotori: cannot read the record to copy it: {error}"),
                }
                true
            }
            Some("marks") => {
                let marks: Marks = heard
                    .message
                    .get("marks")
                    .and_then(|marks| serde_json::from_value(marks.clone()).ok())
                    .unwrap_or_default();
                self.held
                    .lock()
                    .expect("unpoisoned")
                    .insert(heard.key.clone(), marks);
                self.wake.notify_one();
                true
            }
            Some("records") => {
                let Some(rows) = heard
                    .message
                    .get("rows")
                    .and_then(|rows| serde_json::from_value::<Vec<Copied>>(rows.clone()).ok())
                else {
                    return true;
                };
                let mut highest = Marks::new();
                for row in &rows {
                    let mark = highest.entry(row.device.clone()).or_default();
                    if row.id > *mark {
                        mark.clone_from(&row.id);
                    }
                }
                match self.store.copy_in(rows).await {
                    Ok(_) => {
                        if let Some(theirs) =
                            self.held.lock().expect("unpoisoned").get_mut(&heard.key)
                        {
                            raise(theirs, highest);
                        }
                        self.wake.notify_one();
                    }
                    Err(error) => {
                        eprintln!(
                            "dotori: records from {} were not copied: {error}",
                            heard.device
                        );
                    }
                }
                true
            }
            _ => false,
        }
    }

    /// Says that this PC made new records.
    pub fn made(&self) {
        self.wake.notify_one();
    }

    async fn send_all(self: Arc<Self>) {
        loop {
            self.wake.notified().await;
            while self.send_once().await {}
        }
    }

    async fn send_once(&self) -> bool {
        let Ok(ours) = self.store.marks().await else {
            return false;
        };
        let peers: Vec<(String, Marks)> = self
            .held
            .lock()
            .expect("unpoisoned")
            .iter()
            .map(|(key, marks)| (key.clone(), marks.clone()))
            .collect();
        let mut sent_any = false;
        for (key, theirs) in peers {
            for (device, mark) in &ours {
                let from = theirs.get(device).cloned().unwrap_or_default();
                if *mark <= from {
                    continue;
                }
                let Ok(rows) = self
                    .store
                    .after(device, &from, BATCH_COUNT, BATCH_BYTES)
                    .await
                else {
                    continue;
                };
                let Some(last) = rows.last().map(|row| row.id.clone()) else {
                    continue;
                };
                if !self
                    .hub
                    .send(&key, json!({ "kind": "records", "rows": rows }))
                {
                    self.held.lock().expect("unpoisoned").remove(&key);
                    break;
                }
                if let Some(theirs) = self.held.lock().expect("unpoisoned").get_mut(&key) {
                    raise(theirs, Marks::from([(device.clone(), last)]));
                }
                sent_any = true;
            }
        }
        sent_any
    }
}

fn raise(marks: &mut Marks, higher: Marks) {
    for (device, id) in higher {
        let mark = marks.entry(device).or_default();
        if id > *mark {
            *mark = id;
        }
    }
}

/// The raw record as the agent sees it, telling `copies` whenever it grows.
#[derive(Clone, Debug)]
pub struct Announced {
    pub store: SqliteStore,
    pub copies: Arc<Copies>,
}

impl RecordStore for Announced {
    fn load(&self) -> BoxFuture<'_, Result<Vec<Record>, RecordError>> {
        self.store.load()
    }

    fn append(&self, records: Vec<Record>) -> BoxFuture<'_, Result<(), RecordError>> {
        Box::pin(async move {
            self.store.append(records).await?;
            self.copies.made();
            Ok(())
        })
    }
}

#[cfg(test)]
#[path = "copies.test.rs"]
mod tests;
