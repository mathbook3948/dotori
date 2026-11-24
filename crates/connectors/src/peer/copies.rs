//! Copies the raw record between linked PCs, record by record. When two PCs link they say how
//! far they hold each PC's records, and each sends the other what it lacks. After that every new
//! record, made here or copied in from a third PC, goes out at once. What the owner asked to
//! forget goes to every PC too, on link and as soon as it is forgotten.

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
use crate::record::sqlite::Forgotten;
use crate::record::sqlite::SqliteStore;

const BATCH_COUNT: usize = 500;
const BATCH_BYTES: usize = 4 * 1024 * 1024;

type Marks = BTreeMap<String, String>;

#[derive(Debug, Default)]
struct Other {
    marks: Marks,
    waiting: bool,
    stuck: bool,
}

/// Keeps this PC's raw record and the other PCs' copies the same. One batch at a time goes to
/// each PC, and the next waits for that PC to say how far it holds the record now, so a batch
/// it could not store is not passed over and a large record is not read into memory at once.
#[derive(Debug)]
pub struct Copies {
    hub: Arc<Hub>,
    store: SqliteStore,
    others: Mutex<HashMap<String, Other>>,
    wake: Notify,
    forgot: Notify,
}

impl Copies {
    /// Starts sending records out. Pass every message the hub hears to `hear`.
    pub fn start(hub: Arc<Hub>, store: SqliteStore) -> Arc<Self> {
        let copies = Arc::new(Self {
            hub,
            store,
            others: Mutex::new(HashMap::new()),
            wake: Notify::new(),
            forgot: Notify::new(),
        });
        tokio::spawn(Arc::clone(&copies).send_all());
        copies
    }

    /// Takes a message about the record from another PC. Says whether it was one.
    pub async fn hear(&self, heard: &Heard) -> bool {
        match heard.message.get("kind").and_then(Value::as_str) {
            Some("linked") => {
                match self.store.forgotten().await {
                    Ok(list) if !list.is_empty() => {
                        self.hub
                            .send(&heard.key, json!({ "kind": "forgotten", "list": list }));
                    }
                    Ok(_) => {}
                    Err(error) => eprintln!("dotori: cannot read what was forgotten: {error}"),
                }
                self.tell_marks(&heard.key, false).await;
                true
            }
            Some("forgotten") => {
                let list = heard
                    .message
                    .get("list")
                    .and_then(|list| serde_json::from_value::<Vec<Forgotten>>(list.clone()).ok());
                match list {
                    Some(list) => match self.store.take_forgotten(list.clone()).await {
                        Ok(0) => {}
                        Ok(_) => {
                            self.forgot.notify_one();
                            self.hub
                                .broadcast(&json!({ "kind": "forgotten", "list": list }));
                            self.tell_marks(&heard.key, false).await;
                        }
                        Err(error) => eprintln!(
                            "dotori: what {} forgot was not forgotten here: {error}",
                            heard.device
                        ),
                    },
                    None => eprintln!(
                        "dotori: {} sent a list to forget that does not read",
                        heard.device
                    ),
                }
                true
            }
            Some("marks") => {
                let marks: Marks = heard
                    .message
                    .get("marks")
                    .and_then(|marks| serde_json::from_value(marks.clone()).ok())
                    .unwrap_or_default();
                let stuck = heard.message.get("stuck").and_then(Value::as_bool) == Some(true);
                if stuck {
                    eprintln!(
                        "dotori: {} could not store records from this PC; trying again when it links again",
                        heard.device
                    );
                }
                self.others.lock().expect("unpoisoned").insert(
                    heard.key.clone(),
                    Other {
                        marks,
                        waiting: false,
                        stuck,
                    },
                );
                self.wake.notify_one();
                true
            }
            Some("records") => {
                let rows = heard
                    .message
                    .get("rows")
                    .and_then(|rows| serde_json::from_value::<Vec<Copied>>(rows.clone()).ok());
                let mut highest = Marks::new();
                for row in rows.iter().flatten() {
                    let mark = highest.entry(row.device.clone()).or_default();
                    if row.id > *mark {
                        mark.clone_from(&row.id);
                    }
                }
                let stored = match rows {
                    Some(rows) => self.store.copy_in(rows).await.map(|_| ()),
                    None => Err(RecordError {
                        detail: String::from("records that do not read"),
                    }),
                };
                let stuck = stored.is_err();
                match stored {
                    Ok(()) => {
                        if let Some(other) =
                            self.others.lock().expect("unpoisoned").get_mut(&heard.key)
                        {
                            raise(&mut other.marks, highest);
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
                self.tell_marks(&heard.key, stuck).await;
                true
            }
            _ => false,
        }
    }

    /// Tells every linked PC to forget what the owner asked this PC to forget.
    pub fn forget(&self, list: &[Forgotten]) {
        self.hub
            .broadcast(&json!({ "kind": "forgotten", "list": list }));
    }

    /// Waits until another PC made this one forget records, so what was made from them can go too.
    pub async fn forgotten_elsewhere(&self) {
        self.forgot.notified().await;
    }

    /// Says that this PC made new records.
    pub fn made(&self) {
        self.wake.notify_one();
    }

    async fn tell_marks(&self, key: &str, stuck: bool) {
        match self.store.marks().await {
            Ok(marks) => {
                self.hub.send(
                    key,
                    json!({ "kind": "marks", "marks": marks, "stuck": stuck }),
                );
            }
            Err(error) => eprintln!("dotori: cannot read the record to copy it: {error}"),
        }
    }

    async fn send_all(self: Arc<Self>) {
        loop {
            self.wake.notified().await;
            self.send_once().await;
        }
    }

    async fn send_once(&self) {
        let Ok(ours) = self.store.marks().await else {
            return;
        };
        let ready: Vec<(String, Marks)> = self
            .others
            .lock()
            .expect("unpoisoned")
            .iter()
            .filter(|(_, other)| !other.waiting && !other.stuck)
            .map(|(key, other)| (key.clone(), other.marks.clone()))
            .collect();
        for (key, theirs) in ready {
            let behind: Vec<(String, String)> = ours
                .iter()
                .filter_map(|(device, mark)| {
                    let from = theirs.get(device).cloned().unwrap_or_default();
                    (*mark > from).then(|| (device.clone(), from))
                })
                .collect();
            let mut rows = Vec::new();
            for (device, from) in behind {
                if let Ok(found) = self
                    .store
                    .after(&device, &from, BATCH_COUNT, BATCH_BYTES)
                    .await
                    && !found.is_empty()
                {
                    rows = found;
                    break;
                }
            }
            if rows.is_empty() {
                continue;
            }
            if self
                .hub
                .send(&key, json!({ "kind": "records", "rows": rows }))
            {
                if let Some(other) = self.others.lock().expect("unpoisoned").get_mut(&key) {
                    other.waiting = true;
                }
            } else {
                self.others.lock().expect("unpoisoned").remove(&key);
            }
        }
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
