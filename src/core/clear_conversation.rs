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
    /// Enqueue the combined durable Clear and change nothing else. `Ok(None)`:
    /// no durable store is attached (memory-only mode), so there is nothing to
    /// wait for.
    ///
    /// # Errors
    /// When the store queue refuses the write; nothing changed.
    pub fn begin_clear_conversation(
        &self,
        session_id: &str,
    ) -> Result<Option<(StoreAckRx, ClearCommit)>, String> {
        let Some(tx) = self.inner.store_tx.as_ref() else {
            return Ok(None);
        };
        let native = self
            .inner
            .sessions
            .lock()
            .get(session_id)
            .ok_or("session no longer exists")?
            .meta
            .agent_session_id
            .clone();
        let fence = self.fence_discarding(session_id, native);
        let (ack_id, rx) = self.inner.store_acks.allocate();
        let sent = tx.send(StoreWrite::ClearEvents {
            session_id: session_id.to_owned(),
            calls_before_ms: chrono::Utc::now().timestamp_millis(),
            fence: Some(fence.clone()),
            ack_id: Some(ack_id),
        });
        if !sent {
            self.inner.store_acks.cancel(ack_id);
            return Err("Clear was not saved: the durable store queue is unavailable".into());
        }
        Ok(Some((rx, ClearCommit { fence })))
    }

    /// Apply a committed Clear (or a memory-only one) to the Hub: advance the
    /// fence, forget usage and the native id, clear the transcript and drop
    /// the divider. A native id reported after the commit is fenced and
    /// persisted with its own transaction.
    pub fn finish_clear_conversation(&self, session_id: &str, commit: Option<ClearCommit>) {
        let (fence_key, fence) = self.reset_context(session_id, commit.is_none());
        if let Some(commit) = commit
            && commit.fence != (fence_key.clone(), fence.clone())
            && let Some(tx) = self.inner.store_tx.as_ref()
        {
            let _ = tx.send(StoreWrite::ClearAgentSession {
                session_id: session_id.to_owned(),
                fence_key,
                fence,
            });
        }
        self.clear_transcript_memory(session_id);
        self.mark_context_cleared(session_id);
    }
}

/// The fence a durable Clear committed.
pub struct ClearCommit {
    fence: (String, serde_json::Value),
}

#[cfg(test)]
mod tests {
    use super::super::{Event, PersistenceHealth, SessionOrigin, StoreReceiver, StoreSink};
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
}
