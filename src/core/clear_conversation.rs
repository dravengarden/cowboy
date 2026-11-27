//! User-facing Clear conversation, durable first.
//!
//! The store deletes the transcript, forgets the native id and records the
//! restart fence ([`super::runtime_epoch_fence`]) in one acknowledged
//! transaction before the Hub changes anything. If that write fails, the
//! transcript, the native id and the current worker stay as they were: there
//! is no fence and no reset. Only after the commit does the Hub advance its
//! fence, clear its memory and mark the divider, and the caller replace the
//! worker.

use super::hub_managed_calls::StoreAckRx;
use super::{Hub, StoreWrite};

impl Hub {
    /// Enqueue the combined durable Clear and change nothing else except the
    /// pending latch: until [`Self::end_clear_conversation`], queued prompts
    /// wait (they would otherwise reach the worker being replaced) and Steer is
    /// refused. `Ok(None)`: no durable store is attached (memory-only mode),
    /// so there is nothing to wait for.
    ///
    /// # Errors
    /// When the store queue refuses the write; nothing changed.
    pub fn begin_clear_conversation(
        &self,
        session_id: &str,
    ) -> Result<Option<(StoreAckRx, ClearCommit)>, String> {
        let (native, turn_lost) = {
            let mut sessions = self.inner.sessions.lock();
            let session = sessions
                .get_mut(session_id)
                .ok_or("session no longer exists")?;
            if session.lineage_pins > 0 {
                return Err(
                    "A fork or handoff is capturing this session's history; retry Clear in a moment"
                        .into(),
                );
            }
            session.clear_pending = true;
            // Decided with the latch: owner prompts wait from here on.
            let turn_lost = Self::owner_turn_lost_of(session_id, session);
            (session.meta.agent_session_id.clone(), turn_lost)
        };
        let Some(tx) = self.inner.store_tx.as_ref() else {
            return Ok(None);
        };
        let fence = self.fence_discarding(session_id, native);
        let (ack_id, rx) = self.inner.store_acks.allocate();
        let sent = tx.send(StoreWrite::ClearEvents {
            session_id: session_id.to_owned(),
            calls_before_ms: chrono::Utc::now().timestamp_millis(),
            fence: Some(fence.clone()),
            ack_id: Some(ack_id),
            turn_lost: turn_lost.clone(),
        });
        if !sent {
            self.inner.store_acks.cancel(ack_id);
            self.end_clear_conversation(session_id);
            return Err("Clear was not saved: the durable store queue is unavailable".into());
        }
        Ok(Some((rx, ClearCommit { fence, turn_lost })))
    }

    /// Apply a committed Clear (or a memory-only one) to the Hub: advance the
    /// fence, forget usage and the native id, clear the transcript and drop
    /// the divider. A native id reported after the commit is fenced and
    /// persisted with its own transaction.
    pub fn finish_clear_conversation(&self, session_id: &str, commit: Option<ClearCommit>) {
        // Before the transcript goes, so no admission sees one without it.
        let turn_lost = match &commit {
            Some(commit) => commit.turn_lost.clone(),
            None => self.owner_turn_lost(session_id),
        };
        self.apply_owner_turn_lost(turn_lost);
        let (fence_key, fence) = self.reset_context(session_id, commit.is_none());
        if let Some(commit) = commit
            && commit.fence != (fence_key.clone(), fence.clone())
            && let Some(tx) = self.inner.store_tx.as_ref()
        {
            let _guard = self.runtime_projection_exclusive();
            self.advance_uncertain_effect_context_epoch(session_id);
            let _ = tx.send(StoreWrite::ClearAgentSession {
                session_id: session_id.to_owned(),
                fence_key,
                fence,
            });
        }
        if let Some(session) = self.inner.sessions.lock().get_mut(session_id) {
            session.unpersisted_min_seq = None;
        }
        self.clear_transcript_memory(session_id);
        self.mark_context_cleared(session_id);
    }
}

impl Hub {
    /// Lift the pending latch once the Clear failed, or committed and its
    /// worker replacement was requested, and drain what waited.
    pub fn end_clear_conversation(&self, session_id: &str) {
        if let Some(session) = self.inner.sessions.lock().get_mut(session_id) {
            session.clear_pending = false;
        }
        self.try_drain(session_id);
    }

    /// A user Clear is committing for this session.
    #[must_use]
    pub fn clear_pending(&self, session_id: &str) -> bool {
        self.inner
            .sessions
            .lock()
            .get(session_id)
            .is_some_and(|session| session.clear_pending)
    }
}

/// The fence a durable Clear committed.
pub struct ClearCommit {
    fence: (String, serde_json::Value),
    turn_lost: super::owner_turn_loss::TurnLost,
}

#[cfg(test)]
mod tests {
    use super::super::{Event, PersistenceHealth, SessionOrigin, Status, StoreReceiver, StoreSink};
    use super::*;
    use std::sync::Arc;

    fn hub_with_writes() -> (Hub, StoreReceiver) {
        let (sink, mut rx) = StoreSink::channel(64, Arc::new(PersistenceHealth::default()));
        let hub = Hub::with_store(Some(sink));
        hub.create_local_session(
            "s".to_owned(),
            "codex".to_owned(),
            "/tmp".to_owned(),
            "test".to_owned(),
            SessionOrigin::Web,
            false,
        );
        hub.set_agent_session_id("s", "native-1".to_owned());
        hub.push(
            "s",
            Event::Update {
                update: serde_json::json!({"sessionUpdate": "plan", "entries": []}),
            },
        );
        assert!(hub.admit_runtime_epoch("s", "old", Some("native-1")));
        while rx.try_recv().is_ok() {}
        (hub, rx)
    }

    #[tokio::test]
    async fn a_failed_clear_write_changes_nothing() {
        let (hub, mut rx) = hub_with_writes();
        let before = hub.session_info("s").unwrap().event_count;
        let (ack, _commit) = hub.begin_clear_conversation("s").unwrap().unwrap();
        assert!(matches!(
            rx.try_recv(),
            Ok(StoreWrite::ClearEvents { fence: Some((_, fence)), ack_id: Some(_), .. })
                if fence == serde_json::json!(["native-1"])
        ));
        hub.store_ack_sink().fail_all("disk full");
        assert!(matches!(ack.await, Ok(Err(_))));
        // The caller stops here: no fence, no reset.
        let info = hub.session_info("s").unwrap();
        assert_eq!(info.event_count, before);
        assert_eq!(info.meta.agent_session_id.as_deref(), Some("native-1"));
        assert!(hub.admit_runtime_epoch("s", "old", Some("native-1")));
        assert!(rx.try_recv().is_err(), "nothing else was written");
    }

    #[tokio::test]
    async fn a_committed_clear_applies_without_a_second_native_write() {
        let (hub, mut rx) = hub_with_writes();
        let (_ack, commit) = hub.begin_clear_conversation("s").unwrap().unwrap();
        let _ = rx.try_recv();
        hub.finish_clear_conversation("s", Some(commit));
        let info = hub.session_info("s").unwrap();
        assert_eq!(info.meta.agent_session_id, None);
        assert!(!hub.admit_runtime_epoch("s", "old", Some("native-1")));
        while let Ok(write) = rx.try_recv() {
            assert!(
                !matches!(
                    write,
                    StoreWrite::ClearAgentSession { .. } | StoreWrite::ClearEvents { .. }
                ),
                "the committed Clear was written again"
            );
        }
    }

    /// A Clear of history the Hub never read for the owner's turn records
    /// that in its own transaction, and in memory before the transcript goes;
    /// one whose absence of an owner turn was confirmed records nothing.
    #[tokio::test]
    async fn a_clear_of_unread_history_records_the_lost_owner_turn() {
        let (hub, mut rx) = hub_with_writes();
        hub.inner
            .sessions
            .lock()
            .get_mut("s")
            .unwrap()
            .reached_start = false;
        let (_ack, commit) = hub.begin_clear_conversation("s").unwrap().unwrap();
        let key = format!(
            "{}s",
            super::super::settings_keys::SESSION_OWNER_TURN_LOST_PREFIX
        );
        let last = hub.last_event_seq("s").unwrap();
        assert!(matches!(
            rx.try_recv(),
            Ok(StoreWrite::ClearEvents { turn_lost: Some((written, seq)), .. })
                if written == key && seq == serde_json::json!(last)
        ));
        assert_eq!(hub.setting(&key), None, "nothing changes before the commit");
        hub.finish_clear_conversation("s", Some(commit));
        assert_eq!(hub.setting(&key), Some(serde_json::json!(last)));
        hub.end_clear_conversation("s");

        let (hub, mut rx) = hub_with_writes();
        hub.inner
            .sessions
            .lock()
            .get_mut("s")
            .unwrap()
            .reached_start = false;
        hub.confirm_no_owner_turn("s");
        let _ = hub.begin_clear_conversation("s").unwrap().unwrap();
        assert!(matches!(
            rx.try_recv(),
            Ok(StoreWrite::ClearEvents {
                turn_lost: None,
                ..
            })
        ));
    }

    fn submit_during_clear(hub: &Hub) -> tokio::sync::mpsc::Receiver<super::super::DispatchReq> {
        let (tx, rx) = tokio::sync::mpsc::channel(8);
        hub.set_dispatch_tx(tx);
        hub.set_status("s", Status::Running, None);
        rx
    }

    #[tokio::test]
    async fn a_prompt_sent_during_a_clear_waits_for_the_new_context() {
        let (hub, mut writes) = hub_with_writes();
        let mut dispatched = submit_during_clear(&hub);
        let (_ack, commit) = hub.begin_clear_conversation("s").unwrap().unwrap();
        assert!(hub.clear_pending("s"));
        hub.submit(
            "s",
            "next".into(),
            vec![],
            Some("cmid-next".into()),
            &crate::prompt_origin::RequestCaller::Person,
        );
        assert!(
            dispatched.try_recv().is_err(),
            "dispatched into the old context"
        );
        let _ = writes.try_recv();
        hub.finish_clear_conversation("s", Some(commit));
        // Still held between the commit and the worker replacement.
        assert!(dispatched.try_recv().is_err());
        hub.end_clear_conversation("s");
        let request = dispatched.try_recv().expect("dispatched after the Clear");
        assert_eq!(request.cmid.as_deref(), Some("cmid-next"));
        assert_eq!(hub.session_info("s").unwrap().meta.agent_session_id, None);
        assert!(!hub.clear_pending("s"));
    }

    #[tokio::test]
    async fn a_prompt_sent_during_a_failed_clear_reaches_the_unchanged_session() {
        let (hub, _writes) = hub_with_writes();
        let mut dispatched = submit_during_clear(&hub);
        let (ack, _commit) = hub.begin_clear_conversation("s").unwrap().unwrap();
        hub.submit(
            "s",
            "next".into(),
            vec![],
            Some("cmid-next".into()),
            &crate::prompt_origin::RequestCaller::Person,
        );
        assert!(dispatched.try_recv().is_err());
        hub.store_ack_sink().fail_all("disk full");
        assert!(matches!(ack.await, Ok(Err(_))));
        hub.end_clear_conversation("s");
        let request = dispatched
            .try_recv()
            .expect("dispatched after the failed Clear");
        assert_eq!(request.cmid.as_deref(), Some("cmid-next"));
        assert_eq!(
            hub.session_info("s")
                .unwrap()
                .meta
                .agent_session_id
                .as_deref(),
            Some("native-1")
        );
    }

    #[tokio::test]
    async fn failed_clear_preserves_unpersisted_min_seq_until_committed_clear() {
        let (hub, mut writes) = hub_with_writes();
        hub.inner
            .sessions
            .lock()
            .get_mut("s")
            .unwrap()
            .unpersisted_min_seq = Some(0);
        let (ack, _commit) = hub.begin_clear_conversation("s").unwrap().unwrap();
        let _ = writes.try_recv();
        hub.store_ack_sink().fail_all("disk full");
        assert!(matches!(ack.await, Ok(Err(_))));
        hub.end_clear_conversation("s");
        assert_eq!(
            hub.inner
                .sessions
                .lock()
                .get("s")
                .unwrap()
                .unpersisted_min_seq,
            Some(0),
            "failed Clear must keep dropped-event marker so forks refuse incomplete prefix"
        );
        let (_ack2, commit2) = hub.begin_clear_conversation("s").unwrap().unwrap();
        let _ = writes.try_recv();
        hub.finish_clear_conversation("s", Some(commit2));
        hub.end_clear_conversation("s");
        assert_eq!(
            hub.inner
                .sessions
                .lock()
                .get("s")
                .unwrap()
                .unpersisted_min_seq,
            None,
            "committed Clear discards the old transcript and resets the dropped-event marker"
        );
    }
}
