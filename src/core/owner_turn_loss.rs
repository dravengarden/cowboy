//! The record a Clear leaves when it removes history the Hub never read for
//! the session's owner turn (docs/managed-agent-calls.md#call-budgets): the
//! call budget then refuses until a newer owner prompt or grant names a turn,
//! rather than count against an older one. It is written in the Clear's own
//! store transaction and set in memory before the transcript is.

use super::{Hub, Session, settings_keys};

/// A `(key, value)` setting written with a Clear.
pub type TurnLost = Option<(String, serde_json::Value)>;

impl Hub {
    /// The record a Clear of `session_id` would leave now, if any.
    pub(super) fn owner_turn_lost(&self, session_id: &str) -> TurnLost {
        let sessions = self.inner.sessions.lock();
        Self::owner_turn_lost_of(session_id, sessions.get(session_id)?)
    }

    /// The record for `session` (whose lock the caller holds): its history
    /// is not all in memory, no owner turn is known, and none was confirmed
    /// absent. The value is the newest event sequence: only a turn at least
    /// that new resumes calls.
    pub(super) fn owner_turn_lost_of(session_id: &str, session: &Session) -> TurnLost {
        (!session.reached_start && session.owner_turn_seq.is_none() && !session.owner_turn_absent)
            .then(|| {
                (
                    format!(
                        "{}{session_id}",
                        settings_keys::SESSION_OWNER_TURN_LOST_PREFIX
                    ),
                    serde_json::json!(session.next_seq.saturating_sub(1)),
                )
            })
    }

    /// Set the record in memory; the Clear's transaction persists it.
    pub(super) fn apply_owner_turn_lost(&self, turn_lost: TurnLost) {
        if let Some((key, value)) = turn_lost {
            self.with_settings_mut(|settings| Self::commit_setting_locked(settings, key, value));
        }
    }

    /// Durable history was read to its start and holds no owner turn: a
    /// later Clear loses nothing.
    pub fn confirm_no_owner_turn(&self, session_id: &str) {
        if let Some(session) = self.inner.sessions.lock().get_mut(session_id)
            && session.owner_turn_seq.is_none()
        {
            session.owner_turn_absent = true;
        }
    }
}
