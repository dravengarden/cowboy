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
//! admitted; Clear advances the generation. Clear also records the native id
//! it discarded in a persisted session setting, so a worker still running it
//! is fenced whenever it reports that id: unseen by this Controller, first
//! reported without one, or re-reported after a Controller restart. The
//! generation map itself is Controller-local; after a restart every epoch that
//! does not report a discarded id is admitted, so the current context's replay
//! is never refused.
//!
//! Projection holds [`Hub::runtime_projection_guard`] from admission until it
//! has written, and Clear takes it exclusively while it advances the fence, so
//! a frame admitted before Clear cannot write after it.
//!
//! Durability: Clear deletes the transcript, forgets the native id and records
//! this fence in one store transaction (`StoreWrite::ClearEvents`); if it
//! fails, the store keeps both the transcript and the old context. Context
//! renewal and a fork reset persist the id and fence together without the
//! transcript (`StoreWrite::ClearAgentSession`).
//!
//! The fence is final. A Clear whose reset is cancelled (its replacement's
//! launch admission denied before the stop left) does not readmit the worker
//! it kept: the session fails closed as Crashed, and the next prompt or Retry
//! recycles it into a fresh worker.
//!
//! Known limit: an old epoch this Controller first sees after a restart, that
//! has not yet reported a discarded native id, can project frames that carry
//! no id until it does. Closing it needs a context identity on the wire.

use std::collections::HashMap;

use super::{Hub, settings_keys};

/// Fenced epochs kept per session. One beyond this is old enough that its
/// worker no longer emits frames; a discarded native id still fences it.
const MAX_EPOCHS: usize = 32;
/// Discarded native ids kept per session.
const MAX_CLEARED_NATIVE: usize = 8;

/// How the fence classifies one runtime frame's worker epoch.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RuntimeEpoch {
    Current,
    /// Bound to a context Clear discarded.
    Fenced,
    /// Admitted as current until this frame named a discarded native id: any
    /// ownership it took while unidentified is void.
    Reclassified,
}

#[derive(Default)]
pub(super) struct EpochFence {
    generation: u64,
    /// Epoch -> (generation that admitted it, binding order).
    epochs: HashMap<String, (u64, u64)>,
    bound: u64,
}

impl EpochFence {
    /// Start a new context: every epoch admitted so far is fenced.
    pub(super) fn advance(&mut self) {
        self.generation = self.generation.wrapping_add(1);
    }

    fn stale(&self) -> u64 {
        self.generation.wrapping_sub(1)
    }

    /// `discarded`: the frame reports a native id Clear discarded.
    fn classify(&mut self, epoch: &str, discarded: bool) -> RuntimeEpoch {
        let known = self.epochs.get(epoch).map(|(generation, _)| *generation);
        if discarded {
            // Also an epoch first admitted without its id: it runs the old context.
            self.bind(epoch, self.stale());
            return if known == Some(self.generation) {
                RuntimeEpoch::Reclassified
            } else {
                RuntimeEpoch::Fenced
            };
        }
        if let Some(generation) = known {
            return if generation == self.generation {
                RuntimeEpoch::Current
            } else {
                RuntimeEpoch::Fenced
            };
        }
        self.bind(epoch, self.generation);
        RuntimeEpoch::Current
    }

    #[cfg(test)]
    fn admit(&mut self, epoch: &str, discarded: bool) -> bool {
        self.classify(epoch, discarded) == RuntimeEpoch::Current
    }

    fn bind(&mut self, epoch: &str, generation: u64) {
        if self.epochs.len() >= MAX_EPOCHS && !self.epochs.contains_key(epoch) {
            // Forget an older current-generation epoch first (reloads leave
            // them behind; a later frame binds it to the same generation
            // again), never the newest one, which is the likely live owner.
            // Only then the oldest fenced one.
            let current = self.generation;
            let newest = self
                .epochs
                .values()
                .filter(|(bound, _)| *bound == current)
                .map(|(_, order)| *order)
                .max();
            let victim = self
                .epochs
                .iter()
                .filter(|(_, (_, order))| Some(*order) != newest)
                .min_by_key(|(_, (bound, order))| (*bound != current, *bound, *order))
                .map(|(epoch, _)| epoch.clone());
            if let Some(victim) = victim {
                self.epochs.remove(&victim);
            }
        }
        self.bound = self.bound.wrapping_add(1);
        self.epochs
            .insert(epoch.to_owned(), (generation, self.bound));
    }
}

/// The Machine broker's registry placeholder restates a declaration rather
/// than a worker of either context; only its native id can identify it.
fn placeholder(epoch: &str) -> bool {
    epoch.starts_with("broker-")
}

fn cleared_key(session_id: &str) -> String {
    format!(
        "{}{session_id}",
        settings_keys::SESSION_CLEARED_NATIVE_PREFIX
    )
}

impl Hub {
    /// Held by runtime projection from its admission check until its writes
    /// land; Clear advances the fence under the exclusive side.
    pub fn runtime_projection_guard(&self) -> parking_lot::RwLockReadGuard<'_, ()> {
        self.inner.runtime_projection.read_recursive()
    }

    pub(super) fn runtime_projection_exclusive(&self) -> parking_lot::RwLockWriteGuard<'_, ()> {
        self.inner.runtime_projection.write()
    }

    fn cleared_native(&self, session_id: &str) -> Vec<String> {
        self.setting(&cleared_key(session_id))
            .and_then(|value| serde_json::from_value(value).ok())
            .unwrap_or_default()
    }

    /// Record a native id Clear discarded (bounded, newest last) in memory and
    /// return the fence setting, which Clear persists in the same transaction
    /// that forgets the session's native id.
    pub(super) fn record_cleared_native(
        &self,
        session_id: &str,
        native: Option<String>,
    ) -> (String, serde_json::Value) {
        let (key, fence) = self.fence_discarding(session_id, native);
        self.with_settings_mut(|settings| {
            Self::commit_setting_locked(settings, key.clone(), fence.clone())
        });
        (key, fence)
    }

    /// The fence that discarding `native` would record; changes nothing.
    pub(super) fn fence_discarding(
        &self,
        session_id: &str,
        native: Option<String>,
    ) -> (String, serde_json::Value) {
        let mut cleared = self.cleared_native(session_id);
        if let Some(native) = native
            && !cleared.contains(&native)
        {
            cleared.push(native);
            let excess = cleared.len().saturating_sub(MAX_CLEARED_NATIVE);
            cleared.drain(..excess);
        }
        (cleared_key(session_id), serde_json::json!(cleared))
    }

    /// The fence setting as Clear persists it with the transcript deletion.
    pub(super) fn cleared_fence(&self, session_id: &str) -> (String, serde_json::Value) {
        (
            cleared_key(session_id),
            serde_json::json!(self.cleared_native(session_id)),
        )
    }

    /// Whether a runtime frame or snapshot from `epoch` may project into the
    /// session's current context. `native` is the native session id the frame
    /// reports, if any. Unknown sessions are left to the other fences.
    pub fn admit_runtime_epoch(&self, session_id: &str, epoch: &str, native: Option<&str>) -> bool {
        self.classify_runtime_epoch(session_id, epoch, native) == RuntimeEpoch::Current
    }

    /// [`Self::admit_runtime_epoch`] with the reason for a refusal.
    pub fn classify_runtime_epoch(
        &self,
        session_id: &str,
        epoch: &str,
        native: Option<&str>,
    ) -> RuntimeEpoch {
        let cleared = native.map(|_| self.cleared_native(session_id));
        let mut sessions = self.inner.sessions.lock();
        let Some(session) = sessions.get_mut(session_id) else {
            return RuntimeEpoch::Current;
        };
        let discarded = native.is_some_and(|native| {
            cleared
                .as_ref()
                .is_some_and(|ids| ids.iter().any(|id| id == native))
        });
        if placeholder(epoch) {
            return if discarded {
                RuntimeEpoch::Fenced
            } else {
                RuntimeEpoch::Current
            };
        }
        session.runtime_epochs.classify(epoch, discarded)
    }
}

#[cfg(test)]
mod tests {
    use super::{EpochFence, MAX_EPOCHS};
    use crate::core::Hub;

    #[test]
    fn an_epoch_keeps_the_generation_that_first_admitted_it() {
        let mut fence = EpochFence::default();
        assert!(fence.admit("old", false));
        fence.advance();
        assert!(!fence.admit("old", false));
        assert!(fence.admit("new", false));
        // A worker first seen without its id, later reporting a discarded one.
        assert!(fence.admit("late-id", false));
        assert!(!fence.admit("late-id", true));
        assert!(!fence.admit("late-id", false));
    }

    #[test]
    fn eviction_forgets_current_epochs_before_fenced_ones() {
        let mut fence = EpochFence::default();
        assert!(fence.admit("fenced", false));
        fence.advance();
        for index in 0..MAX_EPOCHS * 2 {
            assert!(fence.admit(&format!("e{index}"), false));
        }
        assert!(fence.epochs.len() <= MAX_EPOCHS);
        assert!(!fence.admit("fenced", false));
        // The newest current epoch survives eviction, so the next Clear fences it.
        let newest = format!("e{}", MAX_EPOCHS * 2 - 1);
        fence.bind("fenced-again", 0);
        assert!(fence.epochs.contains_key(&newest));
        fence.advance();
        assert!(!fence.admit(&newest, false));
    }

    #[test]
    fn a_discarded_native_id_survives_a_controller_restart() {
        let hub = Hub::new();
        hub.create_local_session(
            "s".to_owned(),
            "codex".to_owned(),
            "/tmp".to_owned(),
            "test".to_owned(),
            crate::model::SessionOrigin::Web,
            false,
        );
        hub.set_agent_session_id("s", "agent-1".to_owned());
        hub.prepare_context_reset("s");
        // A fresh generation map, as after a restart, still fences the id.
        hub.inner
            .sessions
            .lock()
            .get_mut("s")
            .unwrap()
            .runtime_epochs = EpochFence::default();
        assert!(!hub.admit_runtime_epoch("s", "old", Some("agent-1")));
        assert!(!hub.admit_runtime_epoch("s", "broker-s", Some("agent-1")));
        assert!(hub.admit_runtime_epoch("s", "new", Some("agent-2")));
        assert!(hub.admit_runtime_epoch("s", "broker-s", None));
        // Fenced for good: a later Clear keeps the id discarded.
        hub.prepare_context_reset("s");
        assert!(!hub.admit_runtime_epoch("s", "old", Some("agent-1")));
    }
}
