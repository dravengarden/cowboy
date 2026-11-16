//! Hub state behind a context renewal (see [`crate::context_rollover`]).
use super::{Event, Hub};
use crate::context_rollover::{Digest, MARKER};

impl Hub {
    /// The digest of the session's current native context from the held log
    /// tail, or `None` for an unknown session, with the durable cursor to read
    /// its opening request from when the tail no longer holds it.
    #[must_use]
    pub fn rollover_digest(&self, session_id: &str) -> Option<(Digest, Option<u64>)> {
        let sessions = self.inner.sessions.lock();
        let session = sessions.get(session_id)?;
        Some((
            crate::context_rollover::digest(&session.log, session.reached_start),
            crate::context_rollover::opening_cursor(&session.log, session.reached_start),
        ))
    }

    /// Whether the renewal bounded by `before` still awaits its first answer:
    /// the newest context boundary is its marker and no agent output follows.
    #[must_use]
    pub fn renewal_pending(&self, session_id: &str, before: u64) -> bool {
        let sessions = self.inner.sessions.lock();
        let Some(session) = sessions.get(session_id) else {
            return false;
        };
        for envelope in session.log.iter().rev() {
            let Event::Update { update } = &envelope.event else {
                continue;
            };
            match update
                .get("sessionUpdate")
                .and_then(serde_json::Value::as_str)
            {
                Some(MARKER) => {
                    return update.get("before").and_then(serde_json::Value::as_u64)
                        == Some(before);
                }
                Some(
                    "context_cleared" | "agent_message_chunk" | "agent_thought_chunk" | "tool_call",
                ) => return false,
                _ => {}
            }
        }
        // The marker fell out of the held tail with no answer after it.
        !session.reached_start
    }

    /// Whether the current native context has received a user turn; a
    /// renewal or Clear starts a new one.
    #[must_use]
    pub fn context_has_user_message(&self, session_id: &str) -> bool {
        self.inner
            .sessions
            .lock()
            .get(session_id)
            .is_some_and(super::current_context_has_user_message)
    }

    /// Drop the renewal divider into the timeline and return its sequence,
    /// the exclusive boundary of the earlier history. Unlike Clear, every
    /// earlier event stays.
    pub fn mark_context_rolled_over(
        &self,
        session_id: &str,
        before: u64,
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
                    "before": before,
                    "idleMs": idle_ms,
                    "contextUsed": context_used,
                }),
            },
        );
        self.last_event_seq(session_id)
    }
}
