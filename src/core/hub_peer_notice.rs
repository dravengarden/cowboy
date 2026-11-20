//! Progress of one prompt another session queued, and the notice that tells
//! the sender its prompt's turn ended. See `docs/agent-api.md`
//! ("Session completion notices").

use super::*;

/// Where one queued prompt stands in its target session. A result is bound
/// to the prompt's own turn: the first `TurnEnd` after its echoed user
/// message, never the session's later state.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PromptProgress {
    /// The target is missing.
    Gone,
    /// Still waiting in the target's queue.
    Queued,
    /// Handed to the target's worker; its turn has not ended.
    Running,
    /// The prompt's turn ended with this stop reason. `echo_seq` is the
    /// prompt's echoed user message, the start of that turn.
    Finished { stop_reason: String, echo_seq: u64 },
    /// The prompt ran but its turn never ended: the worker stopped (the
    /// status) or a later prompt started without a turn end.
    Stopped(Status),
    /// Neither queued nor provably run: removed, moved to drafts, a dispatch
    /// that never reached the worker, or evidence that left the bounded hot
    /// window. Only an observer that saw it queued can call it withdrawn.
    Unknown,
}

impl PromptProgress {
    #[must_use]
    pub fn code(&self) -> &'static str {
        match self {
            Self::Gone => "gone",
            Self::Queued => "queued",
            Self::Running => "running",
            Self::Finished { .. } => "finished",
            Self::Stopped(_) => "stopped",
            Self::Unknown => "unknown",
        }
    }

    /// Whether the prompt may still change state.
    #[must_use]
    pub fn pending(&self) -> bool {
        matches!(self, Self::Queued | Self::Running)
    }
}

impl Hub {
    /// Observe the prompt `cmid` queued into `session_id`. Reads only the
    /// in-memory queue, dispatch window and hot transcript; never wakes a worker.
    /// A closing session stays observable: a failed close keeps it.
    #[must_use]
    pub fn prompt_progress(&self, session_id: &str, cmid: &str) -> PromptProgress {
        let sessions = self.inner.sessions.lock();
        let Some(s) = sessions.get(session_id) else {
            return PromptProgress::Gone;
        };
        if s.queue.iter().any(|m| m.cmid.as_deref() == Some(cmid)) {
            return PromptProgress::Queued;
        }
        let busy = s.in_flight || matches!(s.meta.status, Status::Busy | Status::Starting);
        let echo = s
            .log
            .iter()
            .rposition(|entry| entry.cmid.as_deref() == Some(cmid) && is_user_message_chunk(entry));
        let Some(echo) = echo else {
            // Dispatched but not echoed: still starting, or the dispatch
            // never reached the worker (a preflight requeue or failure).
            let dispatched = s.dispatched_cmids.iter().any(|p| p.cmid == cmid);
            return if dispatched && busy {
                PromptProgress::Running
            } else {
                PromptProgress::Unknown
            };
        };
        let echo_seq = s.log[echo].seq;
        for entry in s.log.iter().skip(echo + 1) {
            match &entry.event {
                Event::TurnEnd { stop_reason, .. } => {
                    return PromptProgress::Finished {
                        stop_reason: stop_reason.clone(),
                        echo_seq,
                    };
                }
                _ if is_user_message_chunk(entry)
                    && entry.cmid.as_deref().is_some_and(|other| other != cmid) =>
                {
                    return PromptProgress::Stopped(Status::Interrupted);
                }
                _ => {}
            }
        }
        if busy {
            PromptProgress::Running
        } else {
            PromptProgress::Stopped(s.meta.status)
        }
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

    fn echo(hub: &Hub, session: &str, cmid: &str) {
        hub.push_tagged(
            session,
            Event::Update {
                update: serde_json::json!({
                    "sessionUpdate": "user_message_chunk",
                    "content": {"type": "text", "text": cmid}
                }),
            },
            Some(cmid.to_owned()),
        );
    }

    fn turn_end(hub: &Hub, session: &str, stop: &str) {
        hub.push(
            session,
            Event::TurnEnd {
                stop_reason: stop.to_owned(),
                usage: None,
            },
        );
    }

    #[tokio::test]
    async fn a_prompt_is_bound_to_its_own_turn() {
        let hub = hub_with("peer", "codex");
        let (tx, mut rx) = tokio::sync::mpsc::channel(8);
        hub.set_dispatch_tx(tx);
        hub.set_status("peer", Status::Busy, None);
        hub.submit("peer", "a".into(), vec![], Some("op-a".into()));
        hub.submit("peer", "b".into(), vec![], Some("op-b".into()));
        assert_eq!(hub.prompt_progress("peer", "op-a"), PromptProgress::Queued);
        assert_eq!(
            hub.prompt_progress("peer", "other"),
            PromptProgress::Unknown
        );
        assert_eq!(hub.prompt_progress("gone", "op-a"), PromptProgress::Gone);
        // The earlier turn ends; A is dispatched and runs.
        hub.set_status("peer", Status::Running, None);
        assert!(rx.try_recv().is_ok());
        assert_eq!(hub.prompt_progress("peer", "op-a"), PromptProgress::Running);
        hub.set_status("peer", Status::Busy, None);
        echo(&hub, "peer", "op-a");
        assert_eq!(hub.prompt_progress("peer", "op-a"), PromptProgress::Running);
        turn_end(&hub, "peer", "end_turn");
        hub.set_status("peer", Status::Running, None);
        let PromptProgress::Finished { stop_reason, .. } = hub.prompt_progress("peer", "op-a")
        else {
            panic!("A finished");
        };
        assert_eq!(stop_reason, "end_turn");
        // B drains at once and then crashes: A's result does not change.
        assert!(rx.try_recv().is_ok());
        hub.set_status("peer", Status::Busy, None);
        echo(&hub, "peer", "op-b");
        hub.set_status("peer", Status::Crashed, None);
        assert!(matches!(
            hub.prompt_progress("peer", "op-a"),
            PromptProgress::Finished { .. }
        ));
        assert_eq!(
            hub.prompt_progress("peer", "op-b"),
            PromptProgress::Stopped(Status::Crashed)
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
