//! Progress of one prompt another session queued, and the notice that tells
//! the sender its prompt's turn ended. See `docs/agent-api.md`
//! ("Session completion notices").

use super::*;

/// Where one queued prompt stands in its target session.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PromptProgress {
    /// The target is missing or closing.
    Gone,
    /// Still waiting in the target's queue.
    Queued,
    /// Handed to the target's worker; its turn has not ended.
    Running,
    /// Handed to the worker and the session no longer runs a turn. The
    /// status says how it ended: `Running` is idle, others are terminal.
    Settled(Status),
    /// Neither queued nor known to have run: removed, moved to drafts, or a
    /// dispatch so old that its evidence left the bounded hot window.
    Unknown,
}

impl PromptProgress {
    #[must_use]
    pub fn code(self) -> &'static str {
        match self {
            Self::Gone => "gone",
            Self::Queued => "queued",
            Self::Running => "running",
            Self::Settled(_) => "settled",
            Self::Unknown => "unknown",
        }
    }
}

impl Hub {
    /// Observe the prompt `cmid` queued into `session_id`. Reads only the
    /// in-memory queue, dispatch window and hot transcript; never wakes a worker.
    #[must_use]
    pub fn prompt_progress(&self, session_id: &str, cmid: &str) -> PromptProgress {
        let sessions = self.inner.sessions.lock();
        let Some(s) = sessions.get(session_id).filter(|s| !s.meta.closing) else {
            return PromptProgress::Gone;
        };
        if s.queue.iter().any(|m| m.cmid.as_deref() == Some(cmid)) {
            return PromptProgress::Queued;
        }
        let dispatched = s.dispatched_cmids.iter().any(|p| p.cmid == cmid)
            || s.log
                .iter()
                .rev()
                .any(|entry| entry.cmid.as_deref() == Some(cmid));
        if !dispatched {
            return PromptProgress::Unknown;
        }
        if s.in_flight || matches!(s.meta.status, Status::Busy | Status::Starting) {
            PromptProgress::Running
        } else {
            PromptProgress::Settled(s.meta.status)
        }
    }

    /// The sequence of `cmid`'s echoed user message, if the hot log holds it.
    #[must_use]
    pub fn prompt_echo_seq(&self, session_id: &str, cmid: &str) -> Option<u64> {
        let sessions = self.inner.sessions.lock();
        let s = sessions.get(session_id)?;
        s.log
            .iter()
            .find(|entry| entry.cmid.as_deref() == Some(cmid) && is_user_message_chunk(entry))
            .map(|entry| entry.seq)
    }

    /// Whether automatic prompts to this session park as drafts for a
    /// person (Antigravity runs human-paced only).
    #[must_use]
    pub fn is_human_paced(&self, session_id: &str) -> bool {
        self.human_paced(session_id)
    }

    /// Queue the sender's notice that the turn of a prompt it sent ended.
    /// The id makes a repeat a no-op. Returns true when the sender is
    /// human-paced (Antigravity) and the notice was parked as a draft, which
    /// a person must send.
    pub fn submit_peer_notice(&self, session_id: &str, key: &str, text: String) -> bool {
        let parked = self.is_human_paced(session_id);
        let cmid = format!("{}{key}", crate::agent_model::PEER_NOTICE_PREFIX);
        self.submit_automatic(session_id, text, cmid);
        parked
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hub_with(id: &str, provider: &str) -> Hub {
        let hub = Hub::new();
        hub.create_local_session(
            id.to_owned(),
            provider.to_owned(),
            "/tmp".to_owned(),
            "t".to_owned(),
            SessionOrigin::Web,
            false,
        );
        hub
    }

    #[tokio::test]
    async fn a_prompt_is_queued_then_running_then_settled_with_its_turn() {
        let hub = hub_with("peer", "codex");
        let (tx, mut rx) = tokio::sync::mpsc::channel(8);
        hub.set_dispatch_tx(tx);
        hub.set_status("peer", Status::Busy, None);
        hub.submit("peer", "land it".into(), vec![], Some("op-1".into()));
        assert_eq!(hub.prompt_progress("peer", "op-1"), PromptProgress::Queued);
        assert_eq!(
            hub.prompt_progress("peer", "other"),
            PromptProgress::Unknown
        );
        assert_eq!(hub.prompt_progress("gone", "op-1"), PromptProgress::Gone);
        // The earlier turn ends; ours is dispatched.
        hub.set_status("peer", Status::Running, None);
        assert!(rx.try_recv().is_ok());
        assert_eq!(hub.prompt_progress("peer", "op-1"), PromptProgress::Running);
        hub.set_status("peer", Status::Busy, None);
        assert_eq!(hub.prompt_progress("peer", "op-1"), PromptProgress::Running);
        hub.set_status("peer", Status::Running, None);
        assert_eq!(
            hub.prompt_progress("peer", "op-1"),
            PromptProgress::Settled(Status::Running)
        );
    }

    #[test]
    fn a_withdrawn_prompt_is_unknown() {
        let hub = hub_with("peer", "codex");
        hub.set_status("peer", Status::Busy, None);
        hub.submit("peer", "land it".into(), vec![], Some("op-1".into()));
        assert!(hub.remove_queued_by_cmid("peer", "op-1"));
        assert_eq!(hub.prompt_progress("peer", "op-1"), PromptProgress::Unknown);
    }

    #[tokio::test]
    async fn a_peer_notice_queues_once_and_parks_for_a_human_paced_sender() {
        let hub = hub_with("sender", "codex");
        hub.set_status("sender", Status::Busy, None);
        assert!(!hub.submit_peer_notice("sender", "peer:op-1", "done".into()));
        assert!(!hub.submit_peer_notice("sender", "peer:op-1", "done".into()));
        assert_eq!(hub.session_info("sender").unwrap().queue_count, 1);
        assert!(!Hub::is_authored_cmid("session-notice:peer:op-1"));

        let agy = hub_with("agy", "antigravity");
        let (tx, mut rx) = tokio::sync::mpsc::channel(8);
        agy.set_dispatch_tx(tx);
        agy.set_status("agy", Status::Running, None);
        assert!(agy.submit_peer_notice("agy", "peer:op-1", "done".into()));
        assert!(rx.try_recv().is_err(), "a person sends it");
        let (queue, drafts) = agy.queue_and_drafts("agy", |_| true).unwrap();
        assert!(queue.is_empty());
        assert_eq!(drafts.len(), 1);
    }
}
