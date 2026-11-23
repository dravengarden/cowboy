//! Hub state behind a context renewal (see [`crate::context_rollover`]).
use super::{Envelope, Event, Hub, SessionMeta, StoreWrite};
use crate::context_rollover::{Digest, MARKER};

/// Separate diagnostic sample. A restored unversioned usage value cannot prove
/// which native context generated it. The production field is still managed
/// by the original ACP usage path.
pub(super) struct UsageEvidence {
    native_id: String,
    provider_generation: String,
    used: u64,
}

/// Whether the agent reported usage while the current compaction ran. A
/// Provider that measures the compacted context (Codex recomputes it before
/// the compaction's completion, Claude Code reports the retained tokens at
/// its boundary) needs nothing more; one that reports nothing (`agy` runs
/// `/compact` as an ordinary prompt) leaves its pre-compaction reading, which
/// the completion then marks as history. Transient: a Controller restart
/// mid-compaction only marks a reading stale that a later report replaces.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum CompactionUsage {
    Idle,
    Running {
        reported: bool,
    },
    /// A compaction whose reports during it measure the history being
    /// summarized: Antigravity's proxy synthesizes these (`agy-compact-`
    /// IDs) around an ordinary prompt, and a proxy before
    /// `cowboy-antigravity-acp` 1.0.25 still relays that prompt's usage.
    Unmeasured,
}

/// The ID prefix of a compaction Antigravity's proxy synthesizes.
const PROMPT_COMPACTION_PREFIX: &str = "agy-compact-";

/// The lifecycle step a session event takes for [`CompactionUsage`].
enum CompactionStep {
    Started { unmeasured: bool },
    Completed,
    Ended,
}

fn compaction_step(event: &Event) -> Option<CompactionStep> {
    match event {
        Event::Update { update } if update["sessionUpdate"] == "compaction_update" => {
            match update["status"].as_str()? {
                "in_progress" => Some(CompactionStep::Started {
                    unmeasured: update["compactionId"]
                        .as_str()
                        .is_some_and(|id| id.starts_with(PROMPT_COMPACTION_PREFIX)),
                }),
                "completed" => Some(CompactionStep::Completed),
                _ => Some(CompactionStep::Ended),
            }
        }
        Event::TurnEnd { .. } => Some(CompactionStep::Ended),
        _ => None,
    }
}

#[derive(Clone, Copy)]
pub(super) enum HistoryBoundary {
    Unknown,
    Known(Option<u64>),
}

impl HistoryBoundary {
    pub(super) fn from_log(log: &[Envelope], reached_start: bool) -> Self {
        log.iter()
            .rev()
            .find_map(|entry| Self::from_event(&entry.event))
            .unwrap_or(if reached_start {
                Self::Known(None)
            } else {
                Self::Unknown
            })
    }

    pub(super) fn from_event(event: &Event) -> Option<Self> {
        let Event::Update { update } = event else {
            return None;
        };
        match update["sessionUpdate"].as_str()? {
            MARKER => Some(Self::Known(update["before"].as_u64())),
            "context_cleared" => Some(Self::Known(None)),
            _ => None,
        }
    }

    fn matches(self, boundary: u64) -> Option<bool> {
        match self {
            Self::Unknown => None,
            Self::Known(committed) => Some(committed == Some(boundary)),
        }
    }
}

impl Hub {
    /// Follow a compaction's lifecycle from the session's own events. A
    /// completion with no usage reported since the compaction began marks the
    /// reading stale (persisted, so a reload or restart shows the same): the
    /// meter shows it as unknown and no renewal decision reads it, until the
    /// agent's next real `usage_update` replaces it.
    pub(super) fn observe_compaction(&self, session_id: &str, event: &Event) {
        let Some(step) = compaction_step(event) else {
            return;
        };
        {
            let mut sessions = self.inner.sessions.lock();
            let Some(s) = sessions.get_mut(session_id) else {
                return;
            };
            let previous = std::mem::replace(&mut s.compaction_usage, CompactionUsage::Idle);
            match step {
                CompactionStep::Started { unmeasured } => {
                    s.compaction_usage = if unmeasured {
                        CompactionUsage::Unmeasured
                    } else {
                        CompactionUsage::Running { reported: false }
                    };
                    return;
                }
                CompactionStep::Ended => return,
                CompactionStep::Completed
                    if previous == (CompactionUsage::Running { reported: true }) =>
                {
                    return;
                }
                CompactionStep::Completed => {}
            }
            let Some(usage) = s
                .meta
                .usage
                .as_mut()
                .filter(|usage| !usage.stale_after_compaction())
            else {
                return;
            };
            usage.compacted_at_ms = Some(crate::logs::now_ms());
            s.usage_evidence = None;
            // Queued under the session lock, so a concurrent Clear's reset
            // cannot be overtaken in storage by this reading.
            if let Some(tx) = self.inner.store_tx.as_ref() {
                let _ = tx.send(StoreWrite::UpdateSessionUsage {
                    session_id: session_id.to_owned(),
                    usage: Some(usage.clone()),
                });
            }
        }
        self.broadcast_sessions();
    }

    /// A committed boundary stays known when the hot tail rolls forward.
    /// A restored, truncated tail may need the newest durable marker.
    pub fn observes_rollover_boundary(&self, session_id: &str, boundary: u64) -> Option<bool> {
        self.inner
            .sessions
            .lock()
            .get(session_id)?
            .history_boundary
            .matches(boundary)
    }

    /// Fill an unknown restored slot. A Clear or newer marker received during
    /// the lookup always wins over the store read.
    pub fn restore_observed_rollover_boundary(
        &self,
        session_id: &str,
        committed: Option<u64>,
        boundary: u64,
    ) -> bool {
        let mut sessions = self.inner.sessions.lock();
        let Some(session) = sessions.get_mut(session_id) else {
            return false;
        };
        if let HistoryBoundary::Unknown = session.history_boundary {
            session.history_boundary = HistoryBoundary::Known(committed);
        }
        session.history_boundary.matches(boundary) == Some(true)
    }

    /// Record the agent-reported context-window usage (ACP `usage_update`):
    /// `used` tokens of a `size`-token window. Broadcast, and persisted as the
    /// session's last reading so a dormant session still shows it after a
    /// Controller restart. Deduped — the agent re-emits identical usage several times per
    /// turn, so we only broadcast when the numbers actually move, keeping the
    /// session-list churn (and mobile bandwidth) down.
    ///
    /// A `0`/`0` reading carries no observation: a freshly launched worker
    /// seeds its snapshot that way before the agent reports anything. Applying
    /// it after a resume (Provider update, reload, crash recovery) would erase
    /// the resumed context's real size, so idle hibernation and context
    /// rollover would treat a large expired context as empty. Only Clear and
    /// renewal, which discard the native context, reset the reading.
    pub fn set_session_usage(&self, session_id: &str, mut usage: crate::agent_model::SessionUsage) {
        if usage.used == 0 && usage.size == 0 {
            return;
        }
        let changed = {
            let mut sessions = self.inner.sessions.lock();
            match sessions.get_mut(session_id) {
                Some(s) => {
                    // Reports during a compaction that measures the history
                    // being summarized (an Antigravity proxy before
                    // `cowboy-antigravity-acp` 1.0.25 relays them) say nothing
                    // about the context: keep the stored reading and its mark.
                    if s.compaction_usage == CompactionUsage::Unmeasured {
                        return;
                    }
                    let stored = s.meta.usage.as_ref();
                    // Per-model accounting republished onto the last reading
                    // keeps that reading's observation time: it measures
                    // nothing new, so it keeps a stale mark and proves no
                    // report during a compaction.
                    let republished = stored.is_some_and(|stored| {
                        stored.observed_at_ms != 0
                            && stored.observed_at_ms == usage.observed_at_ms
                            && (stored.used, stored.size) == (usage.used, usage.size)
                    });
                    let stale_mark = stored.and_then(|stored| stored.compacted_at_ms);
                    // A counter-only snapshot (a worker reconnecting) repeats
                    // what the worker last saw, not a measurement of the
                    // compacted context; only a live report replaces a stale one.
                    let fresh = !usage.raw.is_null() && !republished;
                    if fresh && let CompactionUsage::Running { reported } = &mut s.compaction_usage {
                        *reported = true;
                    }
                    if !fresh && stale_mark.is_some() {
                        if !republished {
                            return;
                        }
                        usage.compacted_at_ms = stale_mark;
                    }
                    if !republished
                        && let Some(native_id) = s.meta.agent_session_id.as_ref()
                        && !s.meta.provider_generation_digest.is_empty()
                        && usage.size > 0
                    {
                        s.usage_evidence = Some(UsageEvidence {
                            native_id: native_id.clone(),
                            provider_generation: s.meta.provider_generation_digest.clone(),
                            used: usage.used,
                        });
                    }
                    // A counter-only snapshot repeating the stored counters is no
                    // newer observation; replacing the reading would drop its raw
                    // Provider fields (an Antigravity account plan, for one).
                    if usage.raw.is_null()
                        && s.meta.usage.as_ref().is_some_and(|stored| {
                            (stored.used, stored.size) == (usage.used, usage.size)
                        })
                    {
                        return;
                    }
                    if s.meta.usage.as_ref() == Some(&usage) {
                        return;
                    }
                    s.meta.context_used = usage.used;
                    s.meta.context_size = usage.size;
                    s.meta.usage = Some(usage.clone());
                    // Queued under the session lock, ordered with Clear's reset.
                    if let Some(tx) = self.inner.store_tx.as_ref() {
                        let _ = tx.send(StoreWrite::UpdateSessionUsage {
                            session_id: session_id.to_owned(),
                            usage: Some(usage),
                        });
                    }
                    true
                }
                _ => false,
            }
        };
        if changed {
            self.broadcast_sessions();
        }
    }

    /// Only return a reading from the same observed native context and Provider
    /// generation as the decision snapshot. Includes a real zero reading.
    pub fn verified_context_usage(&self, expected: &SessionMeta) -> Option<u64> {
        let sessions = self.inner.sessions.lock();
        let session = sessions.get(&expected.id)?;
        let evidence = session.usage_evidence.as_ref()?;
        let native_id = expected.agent_session_id.as_deref()?;
        (evidence.native_id == native_id
            && session.meta.agent_session_id.as_deref() == Some(native_id)
            && evidence.provider_generation == expected.provider_generation_digest
            && session.meta.provider_generation_digest == evidence.provider_generation)
            .then_some(evidence.used)
    }

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

impl Hub {
    /// A parent turn end after the task's anchor, with no inference from worker
    /// exit or idleness. When the anchor or intervening history is gone, leave
    /// the observation unknown.
    pub fn parent_ended_during_wait(&self, id: &str, task_key: &str) -> Option<bool> {
        let sessions = self.inner.sessions.lock();
        let session = sessions.get(id)?;
        let mut ended = false;
        let mut previous = session.log.last()?.seq.saturating_add(1);
        let mut gap = false;
        for envelope in session.log.iter().rev() {
            gap |= envelope.seq.saturating_add(1) != previous;
            previous = envelope.seq;
            match &envelope.event {
                Event::TurnEnd { stop_reason, .. } => {
                    // These are the successful native PromptResponse outcomes,
                    // excluding fabricated cancellation and failed dispatch.
                    ended |= matches!(
                        stop_reason.as_str(),
                        "EndTurn" | "MaxTokens" | "MaxTurnRequests"
                    );
                }
                Event::Update { update }
                    if crate::transcript_anchors::anchor_key(update).as_deref()
                        == Some(task_key) =>
                {
                    if update
                        .pointer("/origin/kind")
                        .and_then(serde_json::Value::as_str)
                        == Some("repaired")
                    {
                        return None;
                    }
                    return (ended || !gap).then_some(ended);
                }
                _ => {}
            }
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::runtime_reconciliation_tests::restored_busy;

    #[test]
    fn committed_boundary_outlives_hot_tail_and_a_clear_fences_durable_lookup() {
        let hub = Hub::new();
        let mut restored = restored_busy("session");
        restored.reached_start = false;
        hub.restore_reconciling_runtime(vec![restored]);
        assert_eq!(hub.observes_rollover_boundary("session", 12), None);
        assert!(hub.restore_observed_rollover_boundary("session", Some(12), 12));
        assert_eq!(hub.observes_rollover_boundary("session", 13), Some(false));
        hub.mark_context_rolled_over("session", 13, 1, 2).unwrap();
        hub.inner
            .sessions
            .lock()
            .get_mut("session")
            .unwrap()
            .log
            .clear();
        assert_eq!(hub.observes_rollover_boundary("session", 13), Some(true));
        hub.clear_transcript("session");
        assert!(!hub.restore_observed_rollover_boundary("session", Some(13), 13));
        assert_eq!(hub.observes_rollover_boundary("session", 13), Some(false));
    }
}
