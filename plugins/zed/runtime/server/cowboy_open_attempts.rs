// SPDX-License-Identifier: GPL-3.0-or-later
//! Private record of each peer's `OpenBufferByPath` requests by message ID, so
//! a peer that lost the reply can learn the outcome from this same process
//! instead of replaying the open. Recording starts before the handler's first
//! await; requests on one connection are dispatched in order, so a request
//! this process never recorded was never received before the query.
use super::*;
use parking_lot::Mutex;

/// Retained outcomes; the oldest settled one is forgotten first.
const MAX_ATTEMPTS: usize = 1_024;

#[cfg(test)]
mod tests;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Outcome {
    Pending,
    /// The original native buffer, shared or about to be shared with the peer.
    Opened(u64),
    Failed,
}

#[derive(Default)]
struct Inner {
    attempts: HashMap<(proto::PeerId, u32), (u64, Outcome)>,
    sequence: u64,
}

#[derive(Clone, Default)]
pub struct State(Arc<Mutex<Inner>>);

/// One recorded open. Dropping it without `opened` settles it as failed: the
/// handler returned an error or was dropped before obtaining a buffer.
#[must_use = "settle the attempt with `opened` once the buffer is known"]
pub struct Attempt {
    state: State,
    key: (proto::PeerId, u32),
    settled: bool,
}

impl State {
    pub fn begin(&self, peer: proto::PeerId, message_id: u32) -> Attempt {
        let mut inner = self.0.lock();
        inner.sequence += 1;
        let sequence = inner.sequence;
        let key = (peer, message_id);
        if inner.attempts.len() >= MAX_ATTEMPTS && !inner.attempts.contains_key(&key) {
            let oldest = inner
                .attempts
                .iter()
                .min_by_key(|(_, (sequence, outcome))| (*outcome == Outcome::Pending, *sequence))
                .map(|(key, _)| *key);
            if let Some(oldest) = oldest {
                inner.attempts.remove(&oldest);
            }
        }
        inner.attempts.insert(key, (sequence, Outcome::Pending));
        Attempt {
            state: self.clone(),
            key,
            settled: false,
        }
    }

    pub fn outcome(&self, peer: proto::PeerId, message_id: u32) -> Option<Outcome> {
        self.0
            .lock()
            .attempts
            .get(&(peer, message_id))
            .map(|(_, outcome)| *outcome)
    }

    fn settle(&self, key: (proto::PeerId, u32), outcome: Outcome) {
        if let Some(entry) = self.0.lock().attempts.get_mut(&key) {
            entry.1 = outcome;
        }
    }
}

impl Attempt {
    pub fn opened(mut self, buffer_id: u64) {
        self.settled = true;
        self.state.settle(self.key, Outcome::Opened(buffer_id));
    }
}

impl Drop for Attempt {
    fn drop(&mut self) {
        if !self.settled {
            self.state.settle(self.key, Outcome::Failed);
        }
    }
}

impl BufferStore {
    pub fn cowboy_open_attempts(&self) -> &State {
        &self.cowboy_open_attempts
    }

    pub(super) async fn handle_cowboy_open_attempt(
        this: Entity<Self>,
        envelope: TypedEnvelope<proto::CowboyOpenAttempt>,
        cx: AsyncApp,
    ) -> Result<proto::CowboyOpenAttemptResponse> {
        use proto::cowboy_open_attempt_response::Outcome as Reply;
        let request = envelope.payload;
        anyhow::ensure!(
            request.protocol == 1 && request.project_id == proto::REMOTE_SERVER_PROJECT_ID,
            "invalid private open attempt query"
        );
        let outcome = this.read_with(&cx, |this, _| {
            this.cowboy_open_attempts
                .outcome(envelope.sender_id, request.message_id)
        });
        let (outcome, buffer_id) = match outcome {
            None => (Reply::Unknown, 0),
            Some(Outcome::Pending) => (Reply::Pending, 0),
            Some(Outcome::Opened(id)) => (Reply::Opened, id),
            Some(Outcome::Failed) => (Reply::Failed, 0),
        };
        Ok(proto::CowboyOpenAttemptResponse {
            protocol: 1,
            outcome: outcome as i32,
            buffer_id,
        })
    }
}
