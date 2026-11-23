//! Clear conversation fences the worker epochs of the context it discards.
//!
//! A dormant Clear recycles no worker, and a live one forgets the replaced
//! worker once its reset is acknowledged. Runtime frames carry their worker
//! epoch but no context identity, so without this fence a frame the old
//! worker emitted late (or a Machine re-reported on reconnect) repopulates the
//! fresh context: usage and its compaction mark, the native id that a later
//! wake would resume, configuration, transcript rows such as plans, and
//! pending permissions.
//!
//! Each epoch is bound to the context generation in which it was first
//! admitted; Clear advances the generation. An epoch first seen after Clear
//! belongs to the new context unless it reports a native id Clear discarded
//! (an old worker the Controller had not observed yet). The state is
//! Controller-local: after a Controller restart every epoch is admitted again,
//! as before, so the current context's replay is never refused.

use std::collections::{HashMap, VecDeque};

use super::Hub;

/// Bounds for state kept per session. Fenced epochs and discarded native ids
/// beyond these are old enough that their workers no longer emit frames.
const MAX_EPOCHS: usize = 16;
const MAX_CLEARED_NATIVE: usize = 8;

#[derive(Default)]
pub(super) struct EpochFence {
    generation: u64,
    epochs: HashMap<String, u64>,
    cleared_native: VecDeque<String>,
}

impl EpochFence {
    /// Start a new context: every epoch admitted so far is fenced.
    pub(super) fn advance(&mut self, discarded_native: Option<String>) {
        self.generation = self.generation.wrapping_add(1);
        if let Some(native) = discarded_native
            && !self.cleared_native.contains(&native)
        {
            if self.cleared_native.len() == MAX_CLEARED_NATIVE {
                self.cleared_native.pop_front();
            }
            self.cleared_native.push_back(native);
        }
    }

    fn admit(&mut self, epoch: &str, native: Option<&str>) -> bool {
        if let Some(generation) = self.epochs.get(epoch) {
            return *generation == self.generation;
        }
        let discarded =
            native.is_some_and(|native| self.cleared_native.iter().any(|id| id == native));
        let generation = if discarded {
            self.generation.wrapping_sub(1)
        } else {
            self.generation
        };
        self.bind(epoch, generation);
        !discarded
    }

    fn bind(&mut self, epoch: &str, generation: u64) {
        if self.epochs.len() >= MAX_EPOCHS
            && !self.epochs.contains_key(epoch)
            && let Some(oldest) = self
                .epochs
                .iter()
                .min_by_key(|(_, generation)| **generation)
                .map(|(epoch, _)| epoch.clone())
        {
            self.epochs.remove(&oldest);
        }
        self.epochs.insert(epoch.to_owned(), generation);
    }
}

/// The Machine broker's registry placeholder restates the Controller's own
/// declaration, which a reset rewrites; it is not a worker of either context.
fn placeholder(epoch: &str) -> bool {
    epoch.starts_with("broker-")
}

impl Hub {
    /// Whether a runtime frame or snapshot from `epoch` may project into the
    /// session's current context. `native` is the native session id the frame
    /// reports, if any. Unknown sessions are left to the other fences.
    pub fn admit_runtime_epoch(&self, session_id: &str, epoch: &str, native: Option<&str>) -> bool {
        if placeholder(epoch) {
            return true;
        }
        self.inner
            .sessions
            .lock()
            .get_mut(session_id)
            .is_none_or(|session| session.runtime_epochs.admit(epoch, native))
    }

    /// Re-admit the worker a cancelled reset keeps: it still runs, so its
    /// frames describe the session's only live context again.
    pub fn readmit_runtime_epoch(&self, session_id: &str, epoch: &str) {
        if placeholder(epoch) {
            return;
        }
        if let Some(session) = self.inner.sessions.lock().get_mut(session_id) {
            let generation = session.runtime_epochs.generation;
            session.runtime_epochs.bind(epoch, generation);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::EpochFence;

    #[test]
    fn an_epoch_keeps_the_generation_that_first_admitted_it() {
        let mut fence = EpochFence::default();
        assert!(fence.admit("old", Some("a")));
        fence.advance(Some("a".into()));
        assert!(!fence.admit("old", None));
        assert!(fence.admit("new", None));
        assert!(fence.admit("new", Some("b")));
        // An unseen worker still on the discarded native context is old.
        assert!(!fence.admit("unseen-old", Some("a")));
        assert!(!fence.admit("unseen-old", None));
    }

    #[test]
    fn the_bounded_map_forgets_the_oldest_generation_first() {
        let mut fence = EpochFence::default();
        assert!(fence.admit("first", None));
        fence.advance(None);
        for index in 0..super::MAX_EPOCHS - 1 {
            assert!(fence.admit(&format!("e{index}"), None));
        }
        assert!(fence.admit("one-more", None));
        assert!(fence.epochs.len() <= super::MAX_EPOCHS);
        assert!(!fence.epochs.contains_key("first"));
        assert!(fence.admit("e0", None));
    }
}
