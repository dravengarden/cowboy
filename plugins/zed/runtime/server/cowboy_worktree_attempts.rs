// SPDX-License-Identifier: GPL-3.0-or-later
//! Original worktree request outcomes, scoped to this process and peer.
//! Add completes only after registration, not when its early reply is sent.
//! Dropping a handler is uncertainty: it never establishes absence of effects.
use super::*;
use parking_lot::Mutex;
use proto::cowboy_worktree_attempt_response::Kind;

const CAPACITY: usize = 1_024;
const PEERS: usize = 1_024;

#[cfg(test)]
mod tests;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Outcome {
    Pending,
    Completed(u64),
    Uncertain,
}

#[derive(Default)]
struct History {
    highest: u32,
    forgotten_through: u32,
}

#[derive(Default)]
struct Inner {
    entries: HashMap<(proto::PeerId, u32), (u64, Kind, Outcome)>,
    peers: HashMap<proto::PeerId, History>,
    sequence: u64,
}

#[derive(Clone, Default)]
pub struct State(Arc<Mutex<Inner>>);

#[must_use = "complete only after the original worktree effect is observed"]
pub struct Attempt {
    state: State,
    key: (proto::PeerId, u32),
    completed: bool,
}

impl State {
    pub fn begin(&self, peer: proto::PeerId, message: u32, kind: Kind) -> Result<Attempt> {
        anyhow::ensure!(
            message != 0 && kind != Kind::Unspecified,
            "invalid worktree attempt"
        );
        let mut inner = self.0.lock();
        if let Some(history) = inner.peers.get(&peer) {
            anyhow::ensure!(
                message > history.highest,
                "worktree request ID was already passed"
            );
        } else {
            anyhow::ensure!(inner.peers.len() < PEERS, "worktree peer history is full");
        }
        // Consume refused IDs too: a late duplicate cannot become a new effect.
        inner.peers.entry(peer).or_default().highest = message;
        let sequence = inner
            .sequence
            .checked_add(1)
            .context("worktree sequence exhausted")?;
        if inner.entries.len() == CAPACITY {
            let oldest = inner
                .entries
                .iter()
                .filter(|(_, (_, _, outcome))| *outcome != Outcome::Pending)
                .min_by_key(|(_, (sequence, _, _))| *sequence)
                .map(|(key, _)| *key)
                .context("worktree outcomes are all pending")?;
            inner.entries.remove(&oldest);
            let history = inner.peers.get_mut(&oldest.0).expect("recorded peer");
            history.forgotten_through = history.forgotten_through.max(oldest.1);
        }
        inner.sequence = sequence;
        inner
            .entries
            .insert((peer, message), (sequence, kind, Outcome::Pending));
        Ok(Attempt {
            state: self.clone(),
            key: (peer, message),
            completed: false,
        })
    }

    fn result(&self, peer: proto::PeerId, message: u32) -> proto::CowboyWorktreeAttemptResponse {
        use proto::cowboy_worktree_attempt_response::Outcome as Reply;
        let inner = self.0.lock();
        let (kind, outcome, worktree_id) =
            if let Some((_, kind, outcome)) = inner.entries.get(&(peer, message)) {
                let (outcome, id) = match outcome {
                    Outcome::Pending => (Reply::Pending, 0),
                    Outcome::Completed(id) => (Reply::Completed, *id),
                    Outcome::Uncertain => (Reply::Uncertain, 0),
                };
                (*kind, outcome, id)
            } else {
                let forgotten = inner
                    .peers
                    .get(&peer)
                    .is_some_and(|h| message <= h.forgotten_through);
                (
                    Kind::Unspecified,
                    if forgotten {
                        Reply::Forgotten
                    } else {
                        Reply::Unknown
                    },
                    0,
                )
            };
        proto::CowboyWorktreeAttemptResponse {
            protocol: 1,
            kind: kind as i32,
            outcome: outcome as i32,
            worktree_id,
        }
    }

    fn settle(&self, key: (proto::PeerId, u32), outcome: Outcome) {
        if let Some(entry) = self.0.lock().entries.get_mut(&key) {
            entry.2 = outcome;
        }
    }
}

impl Attempt {
    pub fn complete(mut self, worktree_id: u64) {
        self.state.settle(self.key, Outcome::Completed(worktree_id));
        self.completed = true;
    }
}

impl Drop for Attempt {
    fn drop(&mut self) {
        if !self.completed {
            self.state.settle(self.key, Outcome::Uncertain);
        }
    }
}

impl BufferStore {
    pub fn cowboy_worktree_attempts(&self) -> &State {
        &self.cowboy_worktree_attempts
    }

    pub(super) async fn handle_cowboy_worktree_attempt(
        this: Entity<Self>,
        envelope: TypedEnvelope<proto::CowboyWorktreeAttempt>,
        cx: AsyncApp,
    ) -> Result<proto::CowboyWorktreeAttemptResponse> {
        let request = envelope.payload;
        anyhow::ensure!(
            request.protocol == 1 && request.project_id == proto::REMOTE_SERVER_PROJECT_ID,
            "invalid private worktree attempt query"
        );
        if request.message_id == 0 {
            return Ok(proto::CowboyWorktreeAttemptResponse {
                protocol: 1,
                kind: Kind::Unspecified as i32,
                outcome: proto::cowboy_worktree_attempt_response::Outcome::Supported as i32,
                worktree_id: 0,
            });
        }
        Ok(this.read_with(&cx, |this, _| {
            this.cowboy_worktree_attempts
                .result(envelope.sender_id, request.message_id)
        }))
    }
}
