// SPDX-License-Identifier: GPL-3.0-or-later
//! Private record of each peer's `OpenBufferByPath` requests by message ID, so
//! a peer that lost the reply can learn the outcome from this same process
//! instead of replaying the open. Recording starts before the handler's first
//! await; requests on one connection are dispatched in order, so a request
//! this process never recorded was never received before the query. Evicted
//! outcomes are distinguished from requests that were never admitted.
//!
//! The same module answers which native buffer an open of a path would return
//! (`CowboyOpenTarget`) without opening anything, so the peer can confine its
//! admission fences to the buffers a new open could actually alias.
use super::*;
use parking_lot::Mutex;

/// Retained outcomes; the oldest settled one is forgotten first.
const MAX_ATTEMPTS: usize = 1_024;
const MAX_PEERS: usize = 1_024;

#[cfg(test)]
mod tests;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Outcome {
    Pending,
    /// The original native buffer, shared or about to be shared with the peer.
    Opened(u64),
    Failed,
    /// The result was evicted; absence cannot establish that no Open occurred.
    Forgotten,
}

#[derive(Default)]
struct PeerHistory {
    highest: u32,
    forgotten_through: u32,
}

#[derive(Default)]
struct Inner {
    attempts: HashMap<(proto::PeerId, u32), (u64, Outcome)>,
    sequence: u64,
    peers: HashMap<proto::PeerId, PeerHistory>,
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
    pub fn begin(&self, peer: proto::PeerId, message_id: u32) -> Result<Attempt> {
        let mut inner = self.0.lock();
        let key = (peer, message_id);
        anyhow::ensure!(message_id != 0, "invalid native Open message ID");
        if let Some(history) = inner.peers.get(&peer) {
            anyhow::ensure!(
                message_id > history.highest,
                "native Open message ID was already passed"
            );
        } else {
            anyhow::ensure!(
                inner.peers.len() < MAX_PEERS,
                "native Open peer history is full"
            );
        }
        // A refused request has no effect, but its ID is still consumed.
        // Reusing it after capacity becomes available cannot admit a late Open.
        inner.peers.entry(peer).or_default().highest = message_id;
        let sequence = inner
            .sequence
            .checked_add(1)
            .context("native Open sequence exhausted")?;
        if inner.attempts.len() >= MAX_ATTEMPTS {
            let oldest = inner
                .attempts
                .iter()
                .filter(|(_, (_, outcome))| *outcome != Outcome::Pending)
                .min_by_key(|(_, (sequence, _))| *sequence)
                .map(|(key, _)| *key)
                .context("native Open outcomes are all pending")?;
            inner.attempts.remove(&oldest);
            let history = inner
                .peers
                .get_mut(&oldest.0)
                .expect("recorded before admission");
            history.forgotten_through = history.forgotten_through.max(oldest.1);
        }
        inner.sequence = sequence;
        inner.attempts.insert(key, (sequence, Outcome::Pending));
        Ok(Attempt {
            state: self.clone(),
            key,
            settled: false,
        })
    }

    pub fn outcome(&self, peer: proto::PeerId, message_id: u32) -> Option<Outcome> {
        let inner = self.0.lock();
        inner
            .attempts
            .get(&(peer, message_id))
            .map(|(_, outcome)| *outcome)
            .or_else(|| {
                inner
                    .peers
                    .get(&peer)
                    .filter(|history| message_id <= history.forgotten_through)
                    .map(|_| Outcome::Forgotten)
            })
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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Target {
    /// An open would load a new buffer.
    NotOpen,
    /// An open would return this existing buffer.
    Open(u64),
    /// A load for this path is in flight; its buffer is not known yet.
    Loading,
}

impl BufferStore {
    /// What `open_buffer` would return for this path, without opening it.
    pub fn cowboy_open_target(&self, path: &ProjectPath) -> Target {
        if let Some(buffer) = self.path_to_buffer_id.get(path).copied()
            && self.get(buffer).is_some()
        {
            Target::Open(buffer.to_proto())
        } else if self.loading_buffers.contains_key(path) {
            Target::Loading
        } else {
            Target::NotOpen
        }
    }

    pub(super) async fn handle_cowboy_open_target(
        this: Entity<Self>,
        envelope: TypedEnvelope<proto::CowboyOpenTarget>,
        cx: AsyncApp,
    ) -> Result<proto::CowboyOpenTargetResponse> {
        use proto::cowboy_open_target_response::State as Reply;
        let request = envelope.payload;
        anyhow::ensure!(
            request.protocol == 1
                && request.project_id == proto::REMOTE_SERVER_PROJECT_ID
                && request.path.len() <= 4_096,
            "invalid private open target query"
        );
        let path = ProjectPath {
            worktree_id: WorktreeId::from_proto(request.worktree_id),
            path: RelPath::from_unix_str(&request.path)?.into(),
        };
        let target = this.read_with(&cx, |this, _| this.cowboy_open_target(&path));
        let (state, buffer_id) = match target {
            Target::NotOpen => (Reply::NotOpen, 0),
            Target::Open(id) => (Reply::Open, id),
            Target::Loading => (Reply::Loading, 0),
        };
        Ok(proto::CowboyOpenTargetResponse {
            protocol: 1,
            state: state as i32,
            buffer_id,
        })
    }

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
            Some(Outcome::Forgotten) => (Reply::Forgotten, 0),
        };
        Ok(proto::CowboyOpenAttemptResponse {
            protocol: 1,
            outcome: outcome as i32,
            buffer_id,
        })
    }
}
