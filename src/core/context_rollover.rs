//! Hub state behind a context renewal (see [`crate::context_rollover`]).
use super::{Event, Hub};
use crate::context_rollover::{Digest, MARKER};

impl Hub {
    /// The digest of the session's current native context from the held log
    /// tail, or `None` for an unknown session.
    #[must_use]
    pub fn rollover_digest(&self, session_id: &str) -> Option<Digest> {
        let sessions = self.inner.sessions.lock();
        let session = sessions.get(session_id)?;
        Some(crate::context_rollover::digest(
            &session.log,
            session.reached_start,
        ))
    }

    /// Drop the renewal divider into the timeline and return its sequence,
    /// the exclusive boundary of the earlier history. Unlike Clear, every
    /// earlier event stays.
    pub fn mark_context_rolled_over(
        &self,
        session_id: &str,
        idle_ms: u64,
        context_used: u64,
    ) -> Option<u64> {
        let at = chrono::Utc::now().timestamp_millis();
        self.push(
            session_id,
            Event::Update {
                update: serde_json::json!({
                    "sessionUpdate": MARKER,
                    "at": at,
                    "idleMs": idle_ms,
                    "contextUsed": context_used,
                }),
            },
        );
        self.last_event_seq(session_id)
    }
}
