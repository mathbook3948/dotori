//! Who does each one-PC-at-a-time job, such as running the main session. The PC that claims a
//! job first holds it and says so to the others every few seconds. When the others stop hearing
//! it, the first of them to claim takes over. No majority is asked: two PCs cut off from each
//! other can both hold a job until they link again, and then one steps down.

use std::collections::BTreeMap;
use std::collections::BTreeSet;
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::atomic::AtomicBool;
use std::sync::atomic::Ordering;
use std::time::Duration;
use std::time::Instant;
use std::time::SystemTime;

use serde_json::Value;
use serde_json::json;
use tokio::sync::watch;
use tokio::time;

use crate::peer::hub::Heard;
use crate::peer::hub::Hub;

/// How often a holder says it still holds its jobs.
pub const RENEW: Duration = Duration::from_secs(5);
/// How long a job stays with a PC no one has heard from.
pub const TTL: Duration = Duration::from_secs(20);

/// A moment, kept on two clocks: one that never goes back and one that keeps counting while the
/// PC sleeps. Time since a moment is the longer of the two, so a PC that slept knows it did.
#[derive(Clone, Copy, Debug)]
pub struct Moment {
    steady: Instant,
    wall: SystemTime,
}

impl Moment {
    pub fn now() -> Self {
        Self {
            steady: Instant::now(),
            wall: SystemTime::now(),
        }
    }

    pub fn since(&self, earlier: &Self) -> Duration {
        let steady = self.steady.saturating_duration_since(earlier.steady);
        let wall = self.wall.duration_since(earlier.wall).unwrap_or_default();
        steady.max(wall)
    }
}

#[derive(Clone, Debug)]
struct Holding {
    holder: String,
    term: u64,
    heard: Moment,
}

/// What this PC knows about who holds each job. It decides; the keeper below talks.
#[derive(Debug)]
pub struct Book {
    own: String,
    started: Moment,
    jobs: BTreeMap<String, Holding>,
    released: BTreeMap<String, u64>,
}

impl Book {
    pub fn new(own: impl Into<String>, now: Moment) -> Self {
        Self {
            own: own.into(),
            started: now,
            jobs: BTreeMap::new(),
            released: BTreeMap::new(),
        }
    }

    /// The PC holding `job` now, if anyone does as far as this PC knows.
    pub fn holder(&self, job: &str, now: Moment) -> Option<&str> {
        self.jobs
            .get(job)
            .filter(|holding| holding.holder == self.own || now.since(&holding.heard) < TTL)
            .map(|holding| holding.holder.as_str())
    }

    pub fn holds(&self, job: &str) -> bool {
        self.jobs
            .get(job)
            .is_some_and(|holding| holding.holder == self.own)
    }

    /// Takes in that `holder` says it holds `job` in `term`. A higher term wins, and between
    /// equal terms the smaller key does.
    pub fn heard(&mut self, job: &str, holder: &str, term: u64, now: Moment) {
        if self
            .released
            .get(job)
            .is_some_and(|released| *released >= term)
        {
            return;
        }
        let wins = match self.jobs.get(job) {
            None => true,
            Some(known) => {
                (term, std::cmp::Reverse(holder)) > (known.term, std::cmp::Reverse(&known.holder))
                    || (term == known.term && holder == known.holder)
                    || (known.holder != self.own && now.since(&known.heard) >= TTL)
            }
        };
        if wins {
            self.jobs.insert(
                job.to_owned(),
                Holding {
                    holder: holder.to_owned(),
                    term,
                    heard: now,
                },
            );
        }
    }

    /// Takes in that the holder of `job` let it go after `term`.
    pub fn released(&mut self, job: &str, term: u64) {
        if self
            .jobs
            .get(job)
            .is_some_and(|holding| holding.term <= term)
        {
            self.jobs.remove(job);
        }
        let mark = self.released.entry(job.to_owned()).or_default();
        *mark = (*mark).max(term);
    }

    /// Claims `job` if no one holds it and this PC has waited long enough to hear any holder,
    /// or every other PC it knows has already told it what it holds. Gives the term claimed.
    pub fn claim(&mut self, job: &str, now: Moment, everyone_told: bool) -> Option<u64> {
        if self.holder(job, now).is_some() {
            return None;
        }
        if !everyone_told && now.since(&self.started) < TTL {
            return None;
        }
        let term = self
            .jobs
            .get(job)
            .map(|holding| holding.term)
            .into_iter()
            .chain(self.released.get(job).copied())
            .max()
            .unwrap_or(0)
            .saturating_add(1);
        self.jobs.insert(
            job.to_owned(),
            Holding {
                holder: self.own.clone(),
                term,
                heard: now,
            },
        );
        Some(term)
    }

    /// Notes that this PC told the others it still holds `job`.
    pub fn renewed(&mut self, job: &str, now: Moment) {
        if let Some(holding) = self.jobs.get_mut(job)
            && holding.holder == self.own
        {
            holding.heard = now;
        }
    }

    /// Lets go of every job this PC held but could not renew in time, as after a sleep. Gives
    /// the jobs let go.
    pub fn lapse(&mut self, now: Moment) -> Vec<String> {
        let lapsed: Vec<String> = self
            .jobs
            .iter()
            .filter(|(_, holding)| holding.holder == self.own && now.since(&holding.heard) >= TTL)
            .map(|(job, _)| job.clone())
            .collect();
        for job in &lapsed {
            if let Some(holding) = self.jobs.remove(job) {
                let mark = self.released.entry(job.clone()).or_default();
                *mark = (*mark).max(holding.term);
            }
        }
        if !lapsed.is_empty() {
            self.started = now;
        }
        lapsed
    }

    /// The jobs this PC holds, with their terms.
    pub fn held(&self) -> Vec<(String, u64)> {
        self.jobs
            .iter()
            .filter(|(_, holding)| holding.holder == self.own)
            .map(|(job, holding)| (job.clone(), holding.term))
            .collect()
    }

    /// Every job someone holds as far as this PC knows, with holder and term.
    pub fn known(&self, now: Moment) -> Vec<(String, String, u64)> {
        self.jobs
            .iter()
            .filter(|(_, holding)| holding.holder == self.own || now.since(&holding.heard) < TTL)
            .map(|(job, holding)| (job.clone(), holding.holder.clone(), holding.term))
            .collect()
    }
}

/// Talks for this PC's lease book: claims the jobs it wants, renews what it holds, and tells
/// each job's watchers who holds it now.
#[derive(Debug)]
pub struct Keeper {
    hub: Arc<Hub>,
    book: Mutex<Book>,
    wanted: BTreeSet<String>,
    told: Mutex<BTreeSet<String>>,
    stopping: AtomicBool,
    holders: BTreeMap<String, watch::Sender<Option<String>>>,
}

impl Keeper {
    /// Starts keeping the jobs in `wanted` and watching who holds them.
    pub fn start(hub: Arc<Hub>, own: String, wanted: &[&str]) -> Arc<Self> {
        let holders = wanted
            .iter()
            .map(|job| ((*job).to_owned(), watch::channel(None).0))
            .collect();
        let keeper = Arc::new(Self {
            hub,
            book: Mutex::new(Book::new(own, Moment::now())),
            wanted: wanted.iter().map(|job| (*job).to_owned()).collect(),
            told: Mutex::new(BTreeSet::new()),
            stopping: AtomicBool::new(false),
            holders,
        });
        tokio::spawn(Arc::clone(&keeper).keep());
        keeper
    }

    /// Who holds `job`, now and as it changes.
    pub fn watch(&self, job: &str) -> Option<watch::Receiver<Option<String>>> {
        self.holders.get(job).map(watch::Sender::subscribe)
    }

    /// Whether this PC still holds `job`. Ask before acting for the job toward the outside.
    pub fn holds(&self, job: &str) -> bool {
        let lapsed = !self
            .book
            .lock()
            .expect("unpoisoned")
            .lapse(Moment::now())
            .is_empty();
        if lapsed {
            self.after_lapse();
        }
        self.book.lock().expect("unpoisoned").holds(job)
    }

    fn after_lapse(&self) {
        self.told.lock().expect("unpoisoned").clear();
        self.step();
    }

    /// Takes a message about leases from another PC. Says whether it was one.
    pub fn hear(&self, heard: &Heard) -> bool {
        let now = Moment::now();
        match heard.message.get("kind").and_then(Value::as_str) {
            Some("linked") => {
                let known = self.book.lock().expect("unpoisoned").known(now);
                let leases: Vec<Value> = known
                    .into_iter()
                    .map(
                        |(job, holder, term)| json!({ "job": job, "holder": holder, "term": term }),
                    )
                    .collect();
                self.hub
                    .send(&heard.key, json!({ "kind": "leases", "leases": leases }));
                false
            }
            Some("leases") => {
                {
                    let mut book = self.book.lock().expect("unpoisoned");
                    for lease in heard
                        .message
                        .get("leases")
                        .and_then(Value::as_array)
                        .into_iter()
                        .flatten()
                    {
                        if let (Some(job), Some(holder), Some(term)) = (
                            lease.get("job").and_then(Value::as_str),
                            lease.get("holder").and_then(Value::as_str),
                            lease.get("term").and_then(Value::as_u64),
                        ) {
                            book.heard(job, holder, term, now);
                        }
                    }
                }
                self.told
                    .lock()
                    .expect("unpoisoned")
                    .insert(heard.key.clone());
                self.tell();
                true
            }
            Some("released") => {
                if let (Some(job), Some(term)) = (
                    heard.message.get("job").and_then(Value::as_str),
                    heard.message.get("term").and_then(Value::as_u64),
                ) {
                    self.book.lock().expect("unpoisoned").released(job, term);
                    self.step();
                }
                true
            }
            _ => false,
        }
    }

    /// Lets go of every job this PC holds and tells the others, so one of them takes over at once.
    pub fn release_all(&self) {
        self.stopping.store(true, Ordering::SeqCst);
        let held = self.book.lock().expect("unpoisoned").held();
        for (job, term) in held {
            self.book.lock().expect("unpoisoned").released(&job, term);
            self.hub
                .broadcast(&json!({ "kind": "released", "job": job, "term": term }));
        }
        self.tell();
    }

    async fn keep(self: Arc<Self>) {
        let mut every = time::interval(RENEW);
        loop {
            every.tick().await;
            self.step();
        }
    }

    fn step(&self) {
        let now = Moment::now();
        let linked: BTreeSet<String> = self.hub.linked().into_iter().map(|(key, _)| key).collect();
        let everyone_told = {
            let mut told = self.told.lock().expect("unpoisoned");
            told.retain(|key| linked.contains(key));
            self.hub.approved().iter().all(|key| told.contains(key))
        };
        let mut said = Vec::new();
        {
            let mut book = self.book.lock().expect("unpoisoned");
            let everyone_told = if book.lapse(now).is_empty() {
                everyone_told
            } else {
                self.told.lock().expect("unpoisoned").clear();
                self.hub.approved().is_empty()
            };
            if !self.stopping.load(Ordering::SeqCst) {
                for job in &self.wanted {
                    book.claim(job, now, everyone_told);
                }
            }
            for (job, term) in book.held() {
                book.renewed(&job, now);
                said.push(json!({ "job": job, "holder": book.own, "term": term }));
            }
        }
        if !said.is_empty() {
            self.hub
                .broadcast(&json!({ "kind": "leases", "leases": said }));
        }
        self.tell();
    }

    fn tell(&self) {
        let now = Moment::now();
        let book = self.book.lock().expect("unpoisoned");
        for (job, holders) in &self.holders {
            let holder = book.holder(job, now).map(str::to_owned);
            holders.send_if_modified(|known| {
                let changed = *known != holder;
                *known = holder.clone();
                changed
            });
        }
    }
}

#[cfg(test)]
#[path = "lease.test.rs"]
mod tests;
