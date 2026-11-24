//! Challenges a page asks for over HTTP authentication. Each waits for the owner's answer in the chat
//! window under a number of its own, so an answer only ever reaches the request it was typed for.

use serde_json::Value;
use serde_json::json;

const ANSWERED_KEPT: usize = 16;

/// A login a page asked for. `handle` is what reaches the browser that asked.
#[derive(Clone, Debug)]
pub(crate) struct Challenge<T> {
    pub(crate) id: u64,
    pub(crate) handle: T,
    pub(crate) request: Value,
    pub(crate) session: Option<String>,
    pub(crate) origin: String,
    pub(crate) realm: Value,
    pub(crate) again: bool,
}

/// The logins waiting for the owner, oldest first.
#[derive(Debug)]
pub(crate) struct Challenges<T> {
    next: u64,
    waiting: Vec<Challenge<T>>,
    answered: Vec<Value>,
}

impl<T> Default for Challenges<T> {
    fn default() -> Self {
        Self {
            next: 1,
            waiting: Vec::new(),
            answered: Vec::new(),
        }
    }
}

impl<T> Challenges<T> {
    /// Adds a login the page asked for and gives its number. It is marked `again` when the same
    /// request asks after an answer, as a page does when the name or password was wrong.
    pub(crate) fn ask(
        &mut self,
        handle: T,
        request: Value,
        session: Option<String>,
        origin: String,
        realm: Value,
    ) -> u64 {
        let id = self.next;
        self.next += 1;
        let again = self.answered.contains(&request);
        self.waiting.push(Challenge {
            id,
            handle,
            request,
            session,
            origin,
            realm,
            again,
        });
        id
    }

    /// Takes the login numbered `id` out of the waiting ones, if it still waits.
    pub(crate) fn take(&mut self, id: u64) -> Option<Challenge<T>> {
        let at = self.waiting.iter().position(|login| login.id == id)?;
        let login = self.waiting.remove(at);
        if self.answered.len() >= ANSWERED_KEPT {
            self.answered.remove(0);
        }
        self.answered.push(login.request.clone());
        Some(login)
    }

    /// Drops every waiting login, as when the browser that asked is gone.
    pub(crate) fn clear(&mut self) {
        self.waiting.clear();
        self.answered.clear();
    }

    /// What the chat window shows: the oldest waiting login, without anything secret, or null.
    pub(crate) fn shown(&self) -> Value {
        self.waiting.first().map_or(Value::Null, |login| {
            json!({
                "id": login.id,
                "origin": login.origin,
                "realm": login.realm,
                "again": login.again,
            })
        })
    }
}

#[cfg(test)]
#[path = "auth.test.rs"]
mod tests;
